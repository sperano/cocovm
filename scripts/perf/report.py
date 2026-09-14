#!/usr/bin/env python3
"""Print per-scenario medians and run-to-run ranges from baseline.py output."""
import argparse
import json
from pathlib import Path
import statistics

MEBIBYTE = 1024 * 1024
NANOSECONDS_PER_MILLISECOND = 1_000_000
MILLISECONDS_PER_SECOND = 1_000


def value_range(values, digits=2):
    values = [v for v in values if v is not None]
    if not values:
        return "unavailable"
    median = statistics.median(values)
    return f"{median:.{digits}f} ({min(values):.{digits}f}–{max(values):.{digits}f})"


def measurements(directory):
    groups = {}
    for path in sorted(directory.glob("*/metrics.json")):
        metrics = json.loads(path.read_text())
        resources = json.loads(path.with_name("resources.json").read_text())
        name = metrics["scenario"]
        if isinstance(name, dict):
            name = name["name"]
        groups.setdefault(name, []).append((metrics, resources))
    return groups


def row(name, runs):
    native = "stages" in runs[0][0]
    cpu = [r.get("cpu_percent_one_core") for _, r in runs]
    rss = [r["peak_sampled_rss_bytes"] / MEBIBYTE
           if r.get("peak_sampled_rss_bytes") is not None else None for _, r in runs]
    rates, allocations, bytes_allocated, uploads, p99, missing = [], [], [], [], [], []
    for metrics, _ in runs:
        if native:
            duration = metrics["measurement_duration_seconds"]
            stages = metrics["stages"]
            if not metrics.get("enabled", True):
                fields = metrics.get("scenario", {}).get("fields_run")
                rates.append(fields / duration if fields is not None else None)
                continue
            rates.append(stages["field_execution"]["count"] / duration)
            allocations.append(metrics["allocations"]["count"] / duration)
            bytes_allocated.append(metrics["allocations"]["requested_bytes"] / duration / MEBIBYTE)
            uploads.append(metrics["texture_enqueue_cpu"]["bytes"] / duration / MEBIBYTE)
            p99.append(stages["vm_ui_update"]["p99_ns"] / NANOSECONDS_PER_MILLISECOND
                       if stages["vm_ui_update"]["count"] else None)
            missing.append(metrics["audio"]["missing_frames"]
                           if metrics["audio"]["callbacks"] else None)
        else:
            duration = metrics["elapsed_secs"]
            rates.append(metrics["fields_per_sec"])
            if metrics.get("allocation_tracking", True):
                allocations.append(metrics["allocations"] / duration)
                bytes_allocated.append(metrics["allocated_bytes"] / duration / MEBIBYTE)
    return (f"| {name} | {len(runs)} | {value_range(cpu)} | {value_range(rss)} | "
            f"{value_range(rates, 1)} | {value_range(allocations, 0)} | "
            f"{value_range(bytes_allocated)} | {value_range(uploads)} | "
            f"{value_range(p99, 3)} | {value_range(missing, 0)} |")


def operation_measurements(runs):
    groups = {}
    for metrics, _ in runs:
        scenario = metrics.get("scenario", {})
        if not isinstance(scenario, dict):
            continue
        for event in scenario.get("operation_events", []):
            groups.setdefault(event["name"], []).append(event)
    return groups


def print_operation_table(groups):
    if not groups:
        return
    print("\nOperation timings include work performed synchronously by each request.")
    print("\n| Operation | Attempts | Failures | Duration ms |")
    print("|---|---:|---:|---:|")
    for name, events in groups.items():
        durations = [event["duration_seconds"] * MILLISECONDS_PER_SECOND
                     for event in events]
        failures = sum(not event.get("success", True) for event in events)
        print(f"| {name} | {len(events)} | {failures} | {value_range(durations, 3)} |")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    print("Values are median (minimum–maximum) across runs. CPU uses one core = 100%.")
    print("\n| Scenario | Runs | CPU % | RSS MiB | Fields/s | Allocations/s | Allocated MiB/s | Enqueued MiB/s | VM UI p99 ms | Missing audio frames |")
    print("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
    for name, runs in measurements(args.directory).items():
        print(row(name, runs))
        print_operation_table(operation_measurements(runs))


if __name__ == "__main__":
    main()
