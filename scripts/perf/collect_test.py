import json
from pathlib import Path
import tempfile
import unittest

import collect


class CollectTests(unittest.TestCase):
    def test_includes_lifecycle_correlation_artifact(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run = root / "lifecycle-0"
            run.mkdir()
            (root / "metadata.json").write_text("{}")
            for name in ("metrics", "resources", "samples"):
                (run / f"{name}.json").write_text("{}" if name != "samples" else "[]")
            expected = {"plateau": {"completed_cycles": [{"cycle": 0}]}}
            (run / "lifecycle.json").write_text(json.dumps(expected))

            result = collect.collect(root)

            self.assertEqual(result["runs"][0]["lifecycle"], expected)


if __name__ == "__main__":
    unittest.main()
