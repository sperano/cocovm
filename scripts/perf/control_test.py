import threading
import unittest
from unittest.mock import patch

import control


class ControlTests(unittest.TestCase):
    def test_percentiles_use_nearest_rank_with_tail_and_empty_input(self):
        histogram = [0] * 100
        histogram[1], histogram[20], histogram[99] = 94, 5, 1
        self.assertEqual(control.percentile(histogram, 95), 20)
        self.assertEqual(control.percentile(histogram, 99), 20)
        self.assertEqual(control.percentile(histogram, 100), 99)
        self.assertIsNone(control.percentile([0], 95))

    def test_failed_request_is_counted_and_stopping_exits_client(self):
        stop = threading.Event()

        def failed_request(*_args):
            stop.set()
            raise OSError("connection refused")

        with patch.object(control, "post", side_effect=failed_request):
            result = control.client(1, stop)
        self.assertEqual(result["failures"], 1)
        self.assertEqual(result["successes"], 0)
        self.assertIsNone(result["p99_ms_bucket"])
        self.assertEqual(sum(result["latency_ms_histogram"]), 0)


if __name__ == "__main__":
    unittest.main()
