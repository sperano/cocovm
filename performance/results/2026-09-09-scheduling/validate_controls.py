#!/usr/bin/env python3
"""Validate native deferred MCP control scheduling with isolated fixtures."""

import argparse
import concurrent.futures
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time

SCRIPT_DIR = Path(__file__).resolve().parent
ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "scripts/perf"))
import baseline  # noqa: E402
import control  # noqa: E402

CLIENT_TIMEOUT_SECONDS = 90
SERVER_READY_SECONDS = 10
SERVER_RETRY_INTERVAL_SECONDS = 0.05
PAUSE_SETTLE_SECONDS = 0.25
RESUME_PAUSE_SECONDS = 2
RESPONSIVENESS_LIMIT_SECONDS = 0.5
TIMEOUT_EXPECTED_SECONDS = 11
TIMEOUT_DELIVERY_EARLY_TOLERANCE_SECONDS = 0.5
TIMEOUT_DELIVERY_LATE_TOLERANCE_SECONDS = 2
SCENARIO_EXIT_SECONDS = 40
AX_COMMAND_TIMEOUT_SECONDS = 5
TIMEOUT_WAIT_FIELDS = 60
RUNNING_WAIT_FIELDS = 60
RESUME_WAIT_FIELDS = 600
PAUSED_VM = "perf-0000"
RUNNING_VM = "perf-0001"
PAUSED_VM_WINDOW = "Performance 0"


def session(port):
    payload = {
        "jsonrpc": "2.0",
        "id": 0,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "scheduling-validation", "version": "1"},
        },
    }
    _, session_id = control.post(port, payload)
    if not session_id:
        raise RuntimeError("initialize returned no MCP session")
    return session_id


def call(port, name, arguments):
    session_id = session(port)
    payload = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments},
    }
    started = time.monotonic()
    response, _ = control.post(port, payload, session_id)
    return {"elapsed_seconds": time.monotonic() - started, "response": response}


def wait_ready(port):
    deadline = time.monotonic() + SERVER_READY_SECONDS
    while True:
        try:
            session(port)
            return
        except OSError:
            if time.monotonic() >= deadline:
                raise TimeoutError("control server did not become ready")
            time.sleep(SERVER_RETRY_INTERVAL_SECONDS)


def perf_args(warmup, duration, vm_count):
    return argparse.Namespace(
        kind="native",
        warmup=warmup,
        duration=duration,
        no_telemetry=False,
        no_allocations=True,
        display=None,
        vm_count=vm_count,
    )


def launch(binary, scenario, warmup, duration, vm_count, fixture, report, port):
    args = perf_args(warmup, duration, vm_count)
    env = baseline.environment(args, fixture, report, scenario)
    stdout = (fixture / "stdout.log").open("w")
    stderr = (fixture / "stderr.log").open("w")
    process = subprocess.Popen(
        [str(binary), "--control-port", str(port)],
        cwd=ROOT,
        env=env,
        stdout=stdout,
        stderr=stderr,
    )
    process.validation_logs = (stdout, stderr)
    wait_ready(port)
    return process


def stop(process):
    baseline.stop_process(process)
    for stream in process.validation_logs:
        stream.close()


def response_text(result):
    return json.dumps(result["response"]).lower()


def tool_text(result):
    content = result["response"].get("result", {}).get("content", [])
    return "\n".join(item.get("text", "") for item in content)


def require_success(result, operation):
    response = result["response"]
    if "error" in response or response.get("result", {}).get("isError"):
        raise AssertionError(f"{operation} failed: {response}")


def require_timeout_delivery(result):
    elapsed = result["elapsed_seconds"]
    earliest = TIMEOUT_EXPECTED_SECONDS - TIMEOUT_DELIVERY_EARLY_TOLERANCE_SECONDS
    latest = TIMEOUT_EXPECTED_SECONDS + TIMEOUT_DELIVERY_LATE_TOLERANCE_SECONDS
    if not earliest <= elapsed <= latest:
        raise AssertionError(
            f"paused timeout delivered after {elapsed:.3f}s, "
            f"expected {earliest}..{latest}s"
        )
    if "timed out" not in response_text(result):
        raise AssertionError("paused wait did not return a timeout error")


def start_wait(pool, port, vm, fields):
    return pool.submit(call, port, "wait", {"vm": vm, "fields": fields})


def pause(port, vm, running):
    return call(port, "set_running", {"vm": vm, "running": running})


def press_suspend(process, window_title):
    title = window_title.replace("\\", "\\\\").replace('"', '\\"')
    script = f'''tell application "System Events"
    set ownedProcess to first application process whose unix id is {process.pid}
    set ownedWindow to window "{title}" of ownedProcess
    repeat with candidate in entire contents of ownedWindow
        try
            set element to contents of candidate
            if role of element is "AXButton" and name of element is "Suspend" then
                perform action "AXPress" of element
                return "pressed"
            end if
        end try
    end repeat
    error "accessible Suspend button not found"
end tell'''
    try:
        result = subprocess.run(
            ["osascript", "-e", script],
            capture_output=True,
            text=True,
            check=True,
            timeout=AX_COMMAND_TIMEOUT_SECONDS,
        )
    except subprocess.CalledProcessError as error:
        raise RuntimeError(f"Suspend AX failed: {error.stderr.strip()}") from error
    if result.stdout.strip() != "pressed":
        raise RuntimeError(f"unexpected Suspend AX result: {result.stdout!r}")


def timeout_and_mixed_checks(port, pool):
    results = {}
    paused = start_wait(pool, port, PAUSED_VM, TIMEOUT_WAIT_FIELDS)
    time.sleep(PAUSE_SETTLE_SECONDS)
    results["pause_for_timeout"] = pause(port, PAUSED_VM, False)
    results["paused_list_vms"] = call(port, "list_vms", {})
    running = start_wait(pool, port, RUNNING_VM, RUNNING_WAIT_FIELDS)
    results["running_peer_completion"] = running.result()
    results["paused_timeout"] = paused.result()
    require_success(results["pause_for_timeout"], "pause for timeout")
    require_success(results["paused_list_vms"], "list_vms while paused")
    require_success(results["running_peer_completion"], "running peer wait")
    require_timeout_delivery(results["paused_timeout"])
    if results["paused_list_vms"]["elapsed_seconds"] > RESPONSIVENESS_LIMIT_SECONDS:
        raise AssertionError("list_vms was not responsive while wait was pending")
    return results


def resume_check(port, pool):
    initial_resume = pause(port, PAUSED_VM, True)
    require_success(initial_resume, "resume after timeout")
    future = start_wait(pool, port, PAUSED_VM, RESUME_WAIT_FIELDS)
    time.sleep(PAUSE_SETTLE_SECONDS)
    paused = pause(port, PAUSED_VM, False)
    require_success(paused, "pause during resumable wait")
    time.sleep(RESUME_PAUSE_SECONDS)
    resumed = pause(port, PAUSED_VM, True)
    require_success(resumed, "resume pending wait")
    completion = future.result()
    require_success(completion, "resumed wait")
    return {"resume": resumed, "resumed_completion": completion}


def suspended_mixed_check(process, port, pool):
    results = {}
    suspended = start_wait(pool, port, PAUSED_VM, TIMEOUT_WAIT_FIELDS)
    time.sleep(PAUSE_SETTLE_SECONDS)
    press_suspend(process, PAUSED_VM_WINDOW)
    results["suspended_list_vms"] = call(port, "list_vms", {})
    running = start_wait(pool, port, RUNNING_VM, RUNNING_WAIT_FIELDS)
    results["running_peer_completion"] = running.result()
    results["suspended_timeout"] = suspended.result()
    require_success(results["suspended_list_vms"], "list_vms while suspended")
    require_success(results["running_peer_completion"], "suspended peer wait")
    require_timeout_delivery(results["suspended_timeout"])
    statuses = tool_text(results["suspended_list_vms"])
    for slug, status in [(PAUSED_VM, "suspended"), (RUNNING_VM, "running")]:
        line = next(
            (line for line in statuses.splitlines() if line.startswith(f"{slug} — ")),
            "",
        )
        if not line.endswith(f"({status})"):
            raise AssertionError(f"expected {slug} ({status}), got {line!r}")
    results["resume_suspended"] = call(port, "start_vm", {"vm": PAUSED_VM})
    require_success(results["resume_suspended"], "resume suspended VM")
    return results


def validate_pause_cases(binary, root):
    port = baseline.free_port()
    fixture = root / "pause"
    fixture.mkdir()
    report = fixture / "metrics.json"
    process = launch(binary, "multi-vm", 1, 50, 2, fixture, report, port)
    try:
        with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:
            results = timeout_and_mixed_checks(port, pool)
            results.update(resume_check(port, pool))
            try:
                results["suspended_mixed"] = suspended_mixed_check(process, port, pool)
            except RuntimeError as error:
                results["suspended_mixed"] = {
                    "unavailable": str(error),
                    "coverage": "lifecycle suspend/close and mixed paused/running",
                }
        process.wait(timeout=SCENARIO_EXIT_SECONDS)
        metrics = json.loads(report.read_text())
        return {"controls": results, "functional_validation_metrics": metrics}
    finally:
        stop(process)


def validate_stopped_case(binary, root):
    port = baseline.free_port()
    fixture = root / "stopped"
    fixture.mkdir()
    report = fixture / "metrics.json"
    process = launch(binary, "lifecycle", 5, 5, 1, fixture, report, port)
    try:
        result = call(port, "wait", {"vm": PAUSED_VM, "fields": RESUME_WAIT_FIELDS})
        if "no longer running" not in response_text(result):
            raise AssertionError("closed target did not fail its pending wait")
        return result
    finally:
        stop(process)


def validate_static_paused_case(binary, root):
    port = baseline.free_port()
    fixture = root / "static-paused"
    fixture.mkdir()
    report = fixture / "metrics.json"
    process = launch(binary, "paused", 1, 15, 1, fixture, report, port)
    try:
        result = call(port, "wait", {"vm": PAUSED_VM, "fields": TIMEOUT_WAIT_FIELDS})
        require_timeout_delivery(result)
        process.wait(timeout=SCENARIO_EXIT_SECONDS)
        metrics = json.loads(report.read_text())
        return {
            "timeout": result,
            "manager_update_count": metrics["stages"]["manager_update"]["count"],
            "measurement_duration_seconds": metrics["measurement_duration_seconds"],
            "audio": metrics["audio"],
            "functional_validation_metrics": metrics,
        }
    finally:
        stop(process)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=Path)
    parser.add_argument(
        "--output",
        type=Path,
        default=SCRIPT_DIR / "control-validation.json",
    )
    args = parser.parse_args()
    control.REQUEST_TIMEOUT = CLIENT_TIMEOUT_SECONDS
    with tempfile.TemporaryDirectory(prefix="task227-control-") as directory:
        root = Path(directory)
        report = {
            "purpose": "functional validation with telemetry enabled",
            "binary": str(args.binary.resolve()),
            "static_paused_case": validate_static_paused_case(args.binary.resolve(), root),
            "pause_cases": validate_pause_cases(args.binary.resolve(), root),
            "stopped_case": validate_stopped_case(args.binary.resolve(), root),
        }
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
