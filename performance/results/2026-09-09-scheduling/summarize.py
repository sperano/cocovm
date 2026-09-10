#!/usr/bin/env python3
"""Summarize repaint scheduling captures as Markdown tables."""
import json
from pathlib import Path
import statistics

ROOT = Path(__file__).parent
CAPTURES = (
    "before-rgb-clean", "after-rgb-final", "before-tv", "after-tv-final",
    "before-active", "after-active-final", "before-misc", "after-misc-final",
    "before-four-background-probe", "after-four-background-final-probe",
)
NANOSECONDS_PER_MILLISECOND = 1_000_000


def rate(count, duration):
    return count / duration if count is not None and duration else None


def percentile(histograms, percentile):
    histogram = [sum(values) for values in zip(*histograms)]
    target = sum(histogram) * percentile
    observed = 0
    for latency, frequency in enumerate(histogram):
        observed += frequency
        if observed >= target:
            return latency
    return None


def milliseconds(nanoseconds):
    if nanoseconds is None:
        return None
    return nanoseconds / NANOSECONDS_PER_MILLISECOND


def run_values(run):
    metrics = run["metrics"]
    resources = run["resources"]
    duration = metrics["measurement_duration_seconds"]
    stages = metrics["stages"]
    audio = metrics["audio"]
    control = run.get("control", [])
    control_histograms = [client["latency_ms_histogram"] for client in control]
    manager = stages["manager_update"]
    vm = stages["vm_ui_update"]
    return {
        "cpu": resources.get("cpu_percent_one_core"),
        "idle_wakeups": rate(resources.get("package_idle_wakeups_delta"),
                             resources.get("sample_span_seconds")),
        "interrupt_wakeups": rate(resources.get("interrupt_wakeups_delta"),
                                  resources.get("sample_span_seconds")),
        "manager_updates": rate(manager["count"], duration),
        "vm_updates": rate(vm["count"], duration),
        "fields": rate(metrics["scenario"]["fields_run"], duration),
        "uploads": rate(stages["texture_enqueue_cpu"]["count"], duration),
        "manager_p95": milliseconds(manager.get("p95_ns")),
        "manager_p99": milliseconds(manager.get("p99_ns")),
        "vm_p95": milliseconds(vm.get("p95_ns")),
        "vm_p99": milliseconds(vm.get("p99_ns")),
        "missing_audio": audio["missing_frames"],
        "overflow_audio": audio["overflow_frames"],
        "queue_min": audio.get("queue_min_frames"),
        "queue_max": audio.get("queue_max_frames"),
        "control_p95": percentile(control_histograms, 0.95) if control else None,
        "control_p99": percentile(control_histograms, 0.99) if control else None,
        "control_successes": sum(client["successes"] for client in control),
        "control_failures": sum(client["failures"] for client in control),
    }


def value_range(values, digits=2):
    present = [value for value in values if value is not None]
    if not present:
        return "unavailable"
    return (f"{statistics.median(present):.{digits}f} "
            f"[{min(present):.{digits}f}–{max(present):.{digits}f}]")


def groups(capture):
    grouped = {}
    for run in json.loads((ROOT / f"{capture}.json").read_text())["runs"]:
        scenario = run["metrics"]["scenario"]["name"]
        grouped.setdefault(scenario, []).append(run_values(run))
    return grouped


def print_rows(grouped, keys):
    for scenario, runs in sorted(grouped.items()):
        columns = [scenario]
        for key in keys:
            columns.append(value_range([run[key] for run in runs]))
        print("| " + " | ".join(columns) + " |")
    print()


def print_table(capture):
    print(f"## {capture}\n")
    grouped = groups(capture)
    run_count = min(len(runs) for runs in grouped.values())
    print(f"Values are medians with minimum–maximum ranges across {run_count} "
          "runs per scenario.\n")
    print("### Resource and cadence metrics\n")
    print("| Scenario | CPU % | Idle wakeups/s | Interrupt wakeups/s | Manager UI/s | VM UI/s | Fields/s |")
    print("|---|---:|---:|---:|---:|---:|---:|")
    print_rows(grouped, ("cpu", "idle_wakeups", "interrupt_wakeups",
                         "manager_updates", "vm_updates", "fields"))
    print("### Presentation and UI duration metrics\n")
    print("| Scenario | Uploads/s | Manager p95 ms | Manager p99 ms | VM p95 ms | VM p99 ms |")
    print("|---|---:|---:|---:|---:|---:|")
    print_rows(grouped, ("uploads", "manager_p95", "manager_p99", "vm_p95",
                         "vm_p99"))
    print("### Audio metrics\n")
    print("| Scenario | Missing frames | Overflow frames | Queue minimum frames | Queue maximum frames |")
    print("|---|---:|---:|---:|---:|")
    print_rows(grouped, ("missing_audio", "overflow_audio", "queue_min",
                         "queue_max"))
    control_grouped = {
        scenario: runs for scenario, runs in grouped.items()
        if any(run["control_p95"] is not None for run in runs)
    }
    if control_grouped:
        print("### Control metrics\n")
        print("| Scenario | p95 ms | p99 ms | Successes | Failures |")
        print("|---|---:|---:|---:|---:|")
        print_rows(control_grouped,
                   ("control_p95", "control_p99", "control_successes",
                    "control_failures"))


def print_control_validation():
    path = ROOT / "control-validation.json"
    if not path.exists():
        return
    data = json.loads(path.read_text())
    static = data["static_paused_case"]
    controls = data["pause_cases"]["controls"]
    rows = (
        ("Static paused timeout", static["timeout"]["elapsed_seconds"]),
        ("Mixed paused timeout", controls["paused_timeout"]["elapsed_seconds"]),
        ("List VMs during wait", controls["paused_list_vms"]["elapsed_seconds"]),
        ("Running peer, 60 fields", controls["running_peer_completion"]["elapsed_seconds"]),
        ("Resumed target, 600 fields", controls["resumed_completion"]["elapsed_seconds"]),
        ("Closed target failure", data["stopped_case"]["elapsed_seconds"]),
    )
    print("## Native deferred-control validation\n")
    print("| Outcome | Elapsed seconds |")
    print("|---|---:|")
    for label, elapsed in rows:
        print(f"| {label} | {elapsed:.3f} |")
    print()
    print(f"The static paused capture records {static['manager_update_count']} "
          f"manager callbacks over {static['measurement_duration_seconds']:.3f} s, "
          "with zero fields, display conversions, texture uploads, and audio pushes.")


def main():
    print("# Detailed scheduling measurements\n")
    print("Rates use each run's measured interval. UI callbacks and texture "
          "uploads are application counters, not native presents or GPU "
          "submissions. Timing percentiles come from fixed histograms.\n")
    for capture in CAPTURES:
        if (ROOT / f"{capture}.json").exists():
            print_table(capture)
    print_control_validation()


if __name__ == "__main__":
    main()
