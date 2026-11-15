"""Exercise CI apt recovery without installing packages or sleeping."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class AptRetryTests(unittest.TestCase):
    def run_install(self, failed_command, failures):
        """Return exit status and calls from isolated apt, sudo and timer stubs."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            stubs = {
                "sudo": '#!/bin/bash\nexec "$@"\n',
                "sleep": '#!/bin/bash\necho "sleep $*" >> "$LOG"\n',
                "timeout": '''#!/bin/bash
[[ "$1 $2" == '--kill-after=10s 120s' ]] || exit 2
shift 2
[[ " $* " != *" --no-download "* ]] || exit 2
[[ "$FAILED_COMMAND" != timeout ]] || exit 124
exec "$@"
''',
                "apt-get": '''#!/bin/bash
echo "apt $*" >> "$LOG"
if [[ " $* " == *" update "* ]]; then
  # Simulate an index failure that apt normally reports as success.
  [[ " $* " == *" APT::Update::Error-Mode=any "* ]] || exit 0
  command=update
elif [[ " $* " == *" --download-only "* ]]; then
  command=download
else
  [[ " $* " == *" --no-download "* ]] || exit 2
  command=install
fi
n=$(cat "$STATE-$command" 2>/dev/null || echo 0)
echo $((n + 1)) > "$STATE-$command"
if [[ "$command" == "$FAILED_COMMAND" && "$n" -lt "$FAILURES" ]]; then
  echo 'Temporary failure resolving security.ubuntu.com' >&2
  exit 100
fi
''',
            }
            for name, content in stubs.items():
                path = root / name
                path.write_text(content)
                path.chmod(0o755)
            env = dict(os.environ, PATH=f"{root}:{os.environ['PATH']}",
                       LOG=str(root / "log"), STATE=str(root / "state"),
                       FAILED_COMMAND=failed_command, FAILURES=str(failures),
                       APT_RETRY_TRIES="4", APT_RETRY_PAUSE="5")
            result = subprocess.run(
                ["bash", ROOT / "tools/ci/install_launcher_deps.sh", "libx11-xcb-dev", "libudev-dev"],
                env=env, text=True, capture_output=True)
            return result, (root / "log").read_text().splitlines()

    def test_timed_out_attempts_stop_at_the_bound(self):
        """A network timeout exhausts retries before package installation."""
        result, calls = self.run_install("timeout", 99)
        self.assertEqual(result.returncode, 124, result.stderr)
        self.assertEqual(calls, ["sleep 5", "sleep 10", "sleep 15"])
        self.assertIn("failed after 4 tries", result.stderr)

    def test_transient_index_and_download_failures_recover(self):
        """Recover index/download errors, then install once without a timer."""
        for command in ("update", "download"):
            with self.subTest(command=command):
                result, calls = self.run_install(command, 2)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual([c for c in calls if c.startswith("sleep")], ["sleep 5", "sleep 10"])
                self.assertEqual(len([c for c in calls if c.startswith("apt")]), 5)
                self.assertEqual(calls[-1], "apt install -y --no-download --no-install-recommends libx11-xcb-dev libudev-dev")

    def test_install_failure_is_not_retried(self):
        """A dpkg-phase error is returned without retrying mutation."""
        result, calls = self.run_install("install", 99)
        self.assertEqual(result.returncode, 100, result.stderr)
        self.assertEqual(len(calls), 3)
        self.assertIn("--no-download", calls[-1])

    def test_persistent_errors_fail_closed_at_the_bound(self):
        """Persistent network errors prevent the mutating install phase."""
        for command in ("update", "download"):
            with self.subTest(command=command):
                result, calls = self.run_install(command, 99)
                self.assertEqual(result.returncode, 100, result.stderr)
                self.assertEqual([c for c in calls if c.startswith("sleep")],
                                 ["sleep 5", "sleep 10", "sleep 15"])
                updates = [c for c in calls if c.endswith(" update")]
                installs = [c for c in calls if c.startswith("apt install")]
                self.assertEqual(len(updates), 4 if command == "update" else 1)
                self.assertEqual(len(installs), 0 if command == "update" else 4)


if __name__ == "__main__":
    unittest.main()
