import json
from pathlib import Path
import tempfile
import unittest

import lifecycle

TEST_SAMPLE_INTERVAL = 0.2


def sample(start, end, rss, threads, descriptors):
    return {
        "unix_seconds": start,
        "completed_unix_seconds": end,
        "rss_bytes": rss,
        "physical_footprint_bytes": rss * 2,
        "threads": threads,
        "file_descriptors": descriptors,
    }


class LifecycleTests(unittest.TestCase):
    def test_correlates_complete_samples_around_transition(self):
        events = [{"name": "resume-cold-vm", "unix_seconds": 10.0,
                   "cycle": 0, "cycle_step": 4}]
        samples = [sample(8.5, 9.0, 100, 4, 5),
                   sample(10.5, 11.0, 140, 5, 7)]

        transition = lifecycle.correlate(events, samples)[0]

        self.assertEqual(transition["sample_before"]["rss_bytes"], 100)
        self.assertEqual(transition["sample_after"]["rss_bytes"], 140)
        self.assertEqual(transition["resource_delta"]["rss_bytes"], 40)
        self.assertEqual(transition["resource_delta"]["threads"], 1)

    def test_rejects_samples_outside_bounded_gap(self):
        events = [{"name": "stop-vm", "unix_seconds": 10.0,
                   "cycle": 0, "cycle_step": 5}]
        samples = [sample(1.0, 2.0, 100, 4, 5),
                   sample(18.0, 19.0, 100, 4, 5)]

        transition = lifecycle.correlate(events, samples)[0]

        self.assertIsNone(transition["sample_before"])
        self.assertIsNone(transition["sample_after"])
        self.assertIsNone(transition["resource_delta"])

    def test_plateau_compares_completed_cycle_boundaries(self):
        correlated = [
            {"name": lifecycle.PLATEAU_TRANSITION, "cycle": 0,
             "sample_after": lifecycle._resources(sample(1, 2, 100, 4, 5))},
            {"name": lifecycle.PLATEAU_TRANSITION, "cycle": 1,
             "sample_after": lifecycle._resources(sample(3, 4, 110, 4, 5))},
        ]

        result = lifecycle.plateau(correlated)

        self.assertEqual(result["first_to_last_growth"]["rss_bytes"], 10)
        self.assertEqual(result["first_to_last_growth"]["threads"], 0)

    def test_write_creates_lifecycle_report_only_for_lifecycle_scenario(self):
        event = {"name": "stop-vm", "unix_seconds": 10.0,
                 "cycle": 0, "cycle_step": 5}
        with tempfile.TemporaryDirectory() as directory:
            run_dir = Path(directory)
            metrics = {"scenario": {"name": "lifecycle", "operation_events": [event]}}
            (run_dir / "metrics.json").write_text(json.dumps(metrics))

            lifecycle.write(run_dir, [], TEST_SAMPLE_INTERVAL)

            result = json.loads((run_dir / "lifecycle.json").read_text())
            self.assertEqual(result["scenario"], "lifecycle")
            self.assertEqual(result["sample_interval_target_seconds"], TEST_SAMPLE_INTERVAL)


if __name__ == "__main__":
    unittest.main()
