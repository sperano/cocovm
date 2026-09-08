import unittest

import host


class HostTests(unittest.TestCase):
    def test_device_identifiers_are_removed_from_nested_metadata(self):
        value = {"displays": [{"name": "Monitor", "refresh_rate": 60,
                               "_spdisplays_display-serial-number": "private",
                               "details": {"DeviceUUID": "private", "width": 1920}}]}
        self.assertEqual(host.strip_device_identifiers(value),
                         {"displays": [{"name": "Monitor", "refresh_rate": 60,
                                        "details": {"width": 1920}}]})

    def test_cpu_time_accepts_ps_fractional_and_day_formats(self):
        for text, seconds in (("0:01.25", 1.25), ("02:03:04.5", 7384.5),
                              ("2-03:04:05", 183845), ("00:00.00", 0)):
            with self.subTest(text=text):
                self.assertEqual(host.cpu_seconds(text), seconds)
        with self.assertRaises(ValueError):
            host.cpu_seconds("1:2:3:4:5")

    def test_summary_excludes_startup_and_straddling_observations(self):
        samples = [dict(unix_seconds=start, completed_unix_seconds=start + 0.5,
                        monotonic=start, cpu_seconds=cpu, rss_bytes=rss,
                        threads=threads, file_descriptors=None, package_idle_wakeups=wakeups)
                   for start, cpu, rss, threads, wakeups in
                   ((0, 50, 9000, 9, 100), (3, 51, 100, 2, 102),
                    (5, 52, 200, 3, 105), (5.8, 53, 8000, 8, 110))]
        selected = host.interval_samples(samples, 2, 6)
        report = host.summarize(selected)
        self.assertEqual(report["sample_count"], 2)
        self.assertEqual(report["cpu_percent_one_core"], 50)
        self.assertEqual(report["peak_sampled_rss_bytes"], 200)
        self.assertEqual(report["package_idle_wakeups_delta"], 3)
        self.assertEqual(report["threads_min"], 2)
        self.assertIsNone(report["fds_max"])
        self.assertIsNone(report["interrupt_wakeups_delta"])
        self.assertEqual(host.interval_samples(samples, None, 6), [])
        self.assertIn("unavailable", host.summarize([]))


if __name__ == "__main__":
    unittest.main()
