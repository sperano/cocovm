import unittest

import report


class ReportTests(unittest.TestCase):
    def test_disabled_native_telemetry_uses_field_accounting_and_shows_unknowns(self):
        metrics = {"stages": {}, "enabled": False, "measurement_duration_seconds": 2,
                   "scenario": {"fields_run": 120}}
        cells = [cell.strip() for cell in report.row("basic-idle", [(metrics, {})]).strip("|").split("|")]
        self.assertEqual(cells[3], "unavailable")
        self.assertEqual(cells[4], "60.0 (60.0–60.0)")
        self.assertTrue(all(cell == "unavailable" for cell in cells[5:]))

    def test_disabled_core_allocations_are_unavailable(self):
        metrics = {"elapsed_secs": 2, "fields_per_sec": 60, "allocation_tracking": False,
                   "allocations": 0, "allocated_bytes": 0}
        cells = [cell.strip() for cell in report.row("dac", [(metrics, {})]).strip("|").split("|")]
        self.assertEqual(cells[4], "60.0 (60.0–60.0)")
        self.assertEqual(cells[5:7], ["unavailable", "unavailable"])


if __name__ == "__main__":
    unittest.main()
