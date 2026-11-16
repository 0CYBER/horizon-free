"""Retained Horizon CI queue policy, with the shared offline YAML parser."""
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
# Use the same offline parser as the repository guard, not a runner package.
sys.path.insert(0, str(ROOT / "tools/ci/vendor/pyyaml-6.0.3.zip"))
import yaml


class ConcurrencyTests(unittest.TestCase):
    def test_checks_replace_pull_requests_but_finish_the_running_main_check(self):
        workflow = yaml.safe_load((ROOT / ".github/workflows/ci.yml").read_text())
        self.assertEqual(workflow["concurrency"], {
            "group": "checks-${{ github.event.pull_request.number || "
                     "(github.event_name == 'push' && 'main') || github.sha }}",
            "cancel-in-progress": "${{ github.event_name == 'pull_request' }}",
        })


if __name__ == "__main__":
    unittest.main()
