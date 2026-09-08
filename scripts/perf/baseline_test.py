import os
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
                               sample_profile=False, no_telemetry=False)
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
