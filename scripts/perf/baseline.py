#!/usr/bin/env python3
"""Repeat isolated native/core performance scenarios; never modifies user machines."""
import argparse
import json
import math
import os
from pathlib import Path
import platform
import socket
import subprocess
import tempfile
import time

import assets
import control
import foreground
import host

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_WARMUP = 3.0
DEFAULT_DURATION = 10.0
DEFAULT_REPEATS = 3
MAX_WINDOW_SECONDS = 3600.0
EXIT_GRACE = 30.0
TERMINATE_GRACE = 2.0
PROFILE_SECONDS = 2
PROFILE_INTERVAL_MS = 1
FOCUS_WARMUP_FRACTION = 0.5
PERF_VM_WINDOW_TITLE = "Performance 0"
NATIVE_SCENARIOS = ("manager-idle", "basic-idle", "graphics", "paused", "suspended",
                    "background", "multi-vm", "tv", "dac", "cartridge", "saved-previews",
                    "printer", "snapshot", "lifecycle", "control-load")
CORE_SCENARIOS = ("basic-idle", "graphics", "dac", "cartridge")


def valid_window_seconds(value):
    return math.isfinite(value) and 0 < value <= MAX_WINDOW_SECONDS


def arguments():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kind", choices=("core", "native"))
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--scenario", action="append")
    parser.add_argument("--warmup", type=float, default=DEFAULT_WARMUP)
    parser.add_argument("--duration", type=float, default=DEFAULT_DURATION)
    parser.add_argument("--repeats", type=int, default=DEFAULT_REPEATS)
    parser.add_argument("--profile", choices=("release", "dev"), default="release")
    parser.add_argument("--checkout", type=Path, default=ROOT,
                        help="checkout that owns the measured binary and revision metadata")
    parser.add_argument("--display", choices=("rgb", "cmp", "tv", "tv-bw"))
    parser.add_argument("--vm-count", type=int)
    parser.add_argument("--sample-profile", action="store_true", help="capture macOS sample; adds overhead")
    parser.add_argument("--no-allocations", action="store_true", help="core allocator-overhead comparison")
    parser.add_argument("--no-telemetry", action="store_true", help="native instrumentation-overhead comparison")
    parser.add_argument("--keep-foreground", action="store_true",
                        help="macOS native: maintain owned process focus; adds automation overhead")
    parser.add_argument("--focus-vm", action="store_true",
                        help="macOS native: make the first VM viewport the main window")
    parser.add_argument("--focus-after-warmup", action="store_true",
                        help="macOS native: move VM focus at the measurement boundary")
    parser.add_argument("--vm-position", nargs=2, type=int, metavar=("X", "Y"),
                        help="macOS native: position the focused first VM viewport")
    args = parser.parse_args()
    if not all(valid_window_seconds(value) for value in (args.warmup, args.duration)):
        parser.error(f"warmup and duration must be finite, positive, and at most {MAX_WINDOW_SECONDS:g} seconds")
    if args.repeats <= 0:
        parser.error("repeats must be positive")
    allowed = NATIVE_SCENARIOS if args.kind == "native" else CORE_SCENARIOS
    if args.scenario and any(scenario not in allowed for scenario in args.scenario):
        parser.error(f"scenario must be one of {allowed}")
    if args.vm_count is not None and args.vm_count <= 0:
        parser.error("vm-count must be positive")
    if args.keep_foreground and (args.kind != "native" or platform.system() != "Darwin"):
        parser.error("keep-foreground requires a native macOS run")
    if args.focus_vm and not args.keep_foreground:
        parser.error("focus-vm requires keep-foreground")
    if args.vm_position and not args.focus_vm:
        parser.error("vm-position requires focus-vm")
    if args.focus_after_warmup and not args.focus_vm:
        parser.error("focus-after-warmup requires focus-vm")
    selected_scenarios = args.scenario or allowed
    if args.focus_vm and "background" in selected_scenarios:
        parser.error("focus-vm cannot be combined with the intentionally background scenario")
    args.output = args.output.resolve()
    args.checkout = args.checkout.resolve()
    args.output.mkdir(parents=True, exist_ok=False)
    return args


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def environment(args, fixture, report, scenario):
    env = os.environ.copy()
    env.update(COCOVM_PERF_WARMUP_SECS=str(args.warmup), COCOVM_PERF_DURATION_SECS=str(args.duration),
               COCOVM_PERF_REPEATS="1", COCOVM_PERF_OUTPUT=str(report),
               COCOVM_PERF_DISABLE_METRICS="1" if args.no_telemetry else "0", COCOVM_PERF_ALLOCATIONS="0" if args.no_allocations else "1")
    assets.prepare(args.kind, env, fixture, report)
    if args.kind == "native":
        config = fixture / "config"
        config.mkdir()
        env.update(XDG_CONFIG_HOME=str(config), XDG_DATA_HOME=str(fixture / "data"),
                   COCOVM_PERF_SCENARIO=scenario, COCOVM_PERF_OUTPUT=str(report))
        if args.display:
            env["COCOVM_PERF_DISPLAY"] = args.display
        if args.vm_count:
            env["COCOVM_PERF_VM_COUNT"] = str(args.vm_count)
    return env


def marker(run_dir, suffix):
    try:
        return json.loads((run_dir / f"metrics.json.{suffix}").read_text())["unix_seconds"]
    except (FileNotFoundError, ValueError, KeyError):
        return None


def stop_process(process):
    """Reap only the owned child, with bounded TERM then KILL waits."""
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=TERMINATE_GRACE)
        except subprocess.TimeoutExpired:
            process.kill()
    process.wait(timeout=EXIT_GRACE)


def start_profiler(process, run_dir):
    return subprocess.Popen(["sample", str(process.pid), str(PROFILE_SECONDS),
                             str(PROFILE_INTERVAL_MS), "-file", str(run_dir / "profile.txt")],
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def update_focus(process, args, scenario, elapsed, focus, run_dir):
    if not args.keep_foreground or scenario == "background" or elapsed < args.warmup * FOCUS_WARMUP_FRACTION:
        return True
    if args.focus_after_warmup and marker(run_dir, "started") is None:
        return True
    focus["checks"] += 1
    try:
        changed = foreground.ensure_window(process.pid, PERF_VM_WINDOW_TITLE, args.vm_position) \
            if args.focus_vm else foreground.ensure(process.pid)
        if changed is None:
            focus["missing"] += 1
        else:
            focus["verified"] += 1
            focus["changes"] += int(changed)
    except RuntimeError:
        if process.poll() is None and marker(run_dir, "finished") is None:
            raise
        return False
    return True


def finish_observation(load, profiler, run_dir, focus, args):
    try:
        if load:
            (run_dir / "control.json").write_text(json.dumps(control.finish(load), indent=2))
    finally:
        try:
            if profiler:
                stop_process(profiler)
        finally:
            if args.keep_foreground:
                (run_dir / "foreground.json").write_text(json.dumps(focus, indent=2))
                if args.focus_vm and focus["verified"] == 0:
                    raise RuntimeError(f"never verified focused window {PERF_VM_WINDOW_TITLE!r}")


def observe(process, args, scenario, run_dir, port, samples):
    started = time.monotonic()
    load, profiler = None, None
    backgrounded = False
    focus = {"checks": 0, "changes": 0, "missing": 0, "verified": 0}
    try:
        while process.poll() is None:
            elapsed = time.monotonic() - started
            if elapsed > args.warmup + args.duration + EXIT_GRACE:
                raise TimeoutError("scenario did not exit by its deadline")
            if scenario == "background" and elapsed >= args.warmup * FOCUS_WARMUP_FRACTION and not backgrounded:
                subprocess.run(["osascript", "-e", 'tell application "Finder" to activate'],
                               check=True, timeout=host.COMMAND_TIMEOUT)
                backgrounded = True
            measuring = marker(run_dir, "started") is not None
            finished = marker(run_dir, "finished") is not None
            if measuring and not finished:
                if scenario == "control-load" and load is None:
                    load = control.start(port)
                if args.sample_profile and profiler is None:
                    profiler = start_profiler(process, run_dir)
            if finished:
                break
            if not update_focus(process, args, scenario, elapsed, focus, run_dir):
                break
            measurement = host.sample(process.pid)
            if measurement:
                samples.append(measurement)
            time.sleep(host.SAMPLE_INTERVAL)
    finally:
        finish_observation(load, profiler, run_dir, focus, args)
    process.wait(timeout=EXIT_GRACE)


def write_resources(run_dir, process, command, samples):
    started, finished = marker(run_dir, "started"), marker(run_dir, "finished")
    selected = host.interval_samples(samples, started, finished)
    summary = host.summarize(selected)
    summary.update(exit_code=process.returncode, command=command,
                   measurement_started_unix_seconds=started, measurement_finished_unix_seconds=finished,
                   total_observations=len(samples), observations_inside_window=len(selected))
    (run_dir / "resources.json").write_text(json.dumps(summary, indent=2))
    (run_dir / "samples.json").write_text(json.dumps(samples, indent=2))


def validate_control_load(run_dir):
    path = run_dir / "control.json"
    if not path.exists():
        raise RuntimeError("control-load produced no client report")
    clients = json.loads(path.read_text())
    if sum(client.get("successes", 0) for client in clients) <= 0:
        raise RuntimeError("control-load completed no successful MCP requests")


def execute(args, scenario, run_dir, command, port, directory):
    env = environment(args, Path(directory), run_dir / "metrics.json", scenario)
    samples = []
    with (run_dir / "stdout.log").open("w") as stdout, (run_dir / "stderr.log").open("w") as stderr:
        process = subprocess.Popen(command, cwd=directory, env=env, stdout=stdout, stderr=stderr)
        try:
            observe(process, args, scenario, run_dir, port, samples)
        finally:
            try:
                stop_process(process)
            finally:
                write_resources(run_dir, process, command, samples)
    if process.returncode:
        raise RuntimeError(f"{scenario} exited {process.returncode}; see {run_dir / 'stderr.log'}")
    if args.kind == "core":
        (run_dir / "metrics.json").write_text((run_dir / "stdout.log").read_text())
    if not (run_dir / "metrics.json").exists():
        raise RuntimeError(f"{scenario} produced no report")
    json.loads((run_dir / "metrics.json").read_text())
    if scenario == "control-load":
        validate_control_load(run_dir)


def run(args, scenario, repeat):
    run_dir = args.output / f"{scenario}-{repeat}"
    run_dir.mkdir()
    profile = "debug" if args.profile == "dev" else "release"
    target = Path(os.environ.get("CARGO_TARGET_DIR", args.checkout / "target")).resolve() / profile
    binary = target / ("cocovm" if args.kind == "native" else "examples/perf_baseline")
    port = free_port() if scenario == "control-load" else 0
    command = [str(binary), "--control-port", str(port)] if args.kind == "native" else [str(binary), scenario]
    try:
        with tempfile.TemporaryDirectory(prefix="cocovm-perf-") as directory:
            execute(args, scenario, run_dir, command, port, directory)
    except BaseException as error:
        (run_dir / "failure.json").write_text(json.dumps({"error": str(error), "type": type(error).__name__}))
        raise
    print(f"{scenario} repeat {repeat}: complete", flush=True)


def main():
    args = arguments()
    metadata = host.metadata(args.checkout)
    metadata["settings"] = {key: str(value) if isinstance(value, Path) else value
                            for key, value in vars(args).items()}
    (args.output / "metadata.json").write_text(json.dumps(metadata, indent=2))
    scenarios = args.scenario or (NATIVE_SCENARIOS if args.kind == "native" else CORE_SCENARIOS)
    for scenario in scenarios:
        for repeat in range(args.repeats):
            run(args, scenario, repeat)


if __name__ == "__main__":
    main()
