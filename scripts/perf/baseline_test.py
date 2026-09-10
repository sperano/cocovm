import os
import io
import json
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import baseline

SHORT_TERMINATE_GRACE = 0.05


class BaselineTests(unittest.TestCase):
    def test_focus_vm_requires_keep_foreground(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "output"
            argv = ["baseline.py", "native", "--output", str(output), "--focus-vm"]
            with patch("sys.argv", argv), patch("sys.stderr", io.StringIO()):
                with self.assertRaises(SystemExit) as raised:
                    baseline.arguments()
            self.assertEqual(raised.exception.code, 2)
            self.assertFalse(output.exists())

    def test_window_limit_includes_boundary_and_rejects_nonfinite_values(self):
        self.assertTrue(baseline.valid_window_seconds(baseline.MAX_WINDOW_SECONDS))
        for value in (0, -1, float("nan"), float("inf"), baseline.MAX_WINDOW_SECONDS + 1):
            with self.subTest(value=value):
                self.assertFalse(baseline.valid_window_seconds(value))

    def test_excessive_windows_are_rejected_before_output_creation(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "output"
            for flag in ("--warmup", "--duration"):
                argv = ["baseline.py", "core", "--output", str(output), flag,
                        str(baseline.MAX_WINDOW_SECONDS + 1)]
                with self.subTest(flag=flag), patch("sys.argv", argv), patch("sys.stderr", io.StringIO()):
                    with self.assertRaises(SystemExit) as raised:
                        baseline.arguments()
                    self.assertEqual(raised.exception.code, 2)
                    self.assertFalse(output.exists())

    def test_control_load_must_complete_at_least_one_request(self):
        with tempfile.TemporaryDirectory() as directory:
            run_dir = Path(directory)
            with self.assertRaisesRegex(RuntimeError, "no client report"):
                baseline.validate_control_load(run_dir)
            path = run_dir / "control.json"
            path.write_text(json.dumps([{"successes": 0, "failures": 100}]))
            with self.assertRaisesRegex(RuntimeError, "no successful MCP"):
                baseline.validate_control_load(run_dir)
            path.write_text(json.dumps([{"successes": 0}, {"successes": 1}]))
            baseline.validate_control_load(run_dir)

    def test_uncooperative_child_is_killed_and_reaped(self):
        child = subprocess.Popen([sys.executable, "-c",
            "import signal,time; signal.signal(signal.SIGTERM, signal.SIG_IGN); "
            "print('ready',flush=True); time.sleep(60)"], stdout=subprocess.PIPE, text=True)
        try:
            self.assertEqual(child.stdout.readline().strip(), "ready")
            with patch.object(baseline, "TERMINATE_GRACE", SHORT_TERMINATE_GRACE):
                baseline.stop_process(child)
            self.assertEqual(child.returncode, -signal.SIGKILL)
        finally:
            baseline.stop_process(child)
            child.stdout.close()

    def test_sampler_failure_reaps_child_and_preserves_resource_artifacts(self):
        args = SimpleNamespace(kind="core", warmup=1, duration=1, no_allocations=False,
                               sample_profile=False, no_telemetry=False, keep_foreground=False)
        observed = []

        def fail_sample(pid):
            observed.append(pid)
            raise RuntimeError("sampler failed")

        with tempfile.TemporaryDirectory() as directory:
            run_dir = Path(directory) / "run"
            run_dir.mkdir()
            with patch.object(baseline.host, "sample", side_effect=fail_sample):
                with self.assertRaisesRegex(RuntimeError, "sampler failed"):
                    baseline.execute(args, "dac", run_dir,
                                     [sys.executable, "-c", "import time; time.sleep(60)"],
                                     0, directory)
            self.assertTrue((run_dir / "resources.json").exists())
            self.assertTrue((run_dir / "samples.json").exists())
            with self.assertRaises(ProcessLookupError):
                os.kill(observed[0], 0)


if __name__ == "__main__":
    unittest.main()
