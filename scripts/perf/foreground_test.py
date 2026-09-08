import subprocess
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import foreground

BENCHMARK_PID = 12345


class ForegroundTests(unittest.TestCase):
    def test_owned_pid_is_targeted_and_already_focused_is_unchanged(self):
        with patch.object(foreground.subprocess, "run", return_value=SimpleNamespace(stdout="unchanged\n")) as run:
            self.assertFalse(foreground.ensure(BENCHMARK_PID))
        script = run.call_args.args[0][-1]
        self.assertIn(f"whose unix id is {BENCHMARK_PID}", script)
        self.assertIn("if frontmost of benchmarkProcess", script)
        self.assertNotIn("activate", script)

    def test_automation_denial_is_an_explicit_failure(self):
        error = subprocess.CalledProcessError(1, ["osascript"], stderr="Not authorized")
        with patch.object(foreground.subprocess, "run", side_effect=error):
            with self.assertRaisesRegex(RuntimeError, "Not authorized"):
                foreground.ensure(BENCHMARK_PID)


if __name__ == "__main__":
    unittest.main()
