"""Correlate named lifecycle transitions with externally sampled resources."""

import json

MAX_CORRELATION_GAP_SECONDS = 1.5
RESOURCE_KEYS = (
    "rss_bytes",
    "physical_footprint_bytes",
    "threads",
    "file_descriptors",
)
PLATEAU_TRANSITION = "restart-after-window-close"


def _distance_before(sample, event_time):
    return event_time - sample["completed_unix_seconds"]


def _distance_after(sample, event_time):
    return sample["unix_seconds"] - event_time


def _nearest_before(samples, event_time):
    eligible = [sample for sample in samples
                if sample["completed_unix_seconds"] <= event_time]
    if not eligible:
        return None
    sample = max(eligible, key=lambda item: item["completed_unix_seconds"])
    return sample if _distance_before(sample, event_time) <= MAX_CORRELATION_GAP_SECONDS else None


def _nearest_after(samples, event_time):
    eligible = [sample for sample in samples if sample["unix_seconds"] >= event_time]
    if not eligible:
        return None
    sample = min(eligible, key=lambda item: item["unix_seconds"])
    return sample if _distance_after(sample, event_time) <= MAX_CORRELATION_GAP_SECONDS else None


def _resources(sample):
    if sample is None:
        return None
    return {key: sample.get(key) for key in RESOURCE_KEYS}


def _delta(before, after):
    if before is None or after is None:
        return None
    return {
        key: after.get(key) - before.get(key)
        if before.get(key) is not None and after.get(key) is not None else None
        for key in RESOURCE_KEYS
    }


def correlate(events, samples):
    """Attach nearest complete before/after resource observations to events."""
    correlated = []
    for event in events:
        event_time = event["unix_seconds"]
        before = _nearest_before(samples, event_time)
        after = _nearest_after(samples, event_time)
        correlated.append({
            **event,
            "sample_before": _resources(before),
            "sample_after": _resources(after),
            "resource_delta": _delta(before, after),
        })
    return correlated


def plateau(correlated):
    """Report post-cycle resource snapshots and first-to-last growth."""
    cycles = [{"cycle": event["cycle"], **event["sample_after"]}
              for event in correlated
              if event["name"] == PLATEAU_TRANSITION and event["sample_after"] is not None]
    growth = _delta(cycles[0], cycles[-1]) if len(cycles) >= 2 else None
    return {
        "boundary": f"sample after {PLATEAU_TRANSITION}",
        "completed_cycles": cycles,
        "first_to_last_growth": growth,
        "interpretation": "Sampling can miss brief peaks; inspect multiple fresh-process repeats and ranges before concluding that resources plateau.",
    }


def report(metrics, samples):
    scenario = metrics.get("scenario", {})
    events = scenario.get("operation_events", [])
    correlated = correlate(events, samples)
    return {
        "scenario": scenario.get("name"),
        "sample_interval_target_seconds": None,
        "max_correlation_gap_seconds": MAX_CORRELATION_GAP_SECONDS,
        "transitions": correlated,
        "plateau": plateau(correlated),
    }


def write(run_dir, samples, sample_interval):
    metrics_path = run_dir / "metrics.json"
    if not metrics_path.exists():
        return
    metrics = json.loads(metrics_path.read_text())
    if metrics.get("scenario", {}).get("name") != "lifecycle":
        return
    result = report(metrics, samples)
    result["sample_interval_target_seconds"] = sample_interval
    (run_dir / "lifecycle.json").write_text(json.dumps(result, indent=2))
