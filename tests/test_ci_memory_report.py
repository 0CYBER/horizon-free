"""Exercise lint/verify cgroup diagnostics with readable and unavailable counters."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

from tests.test_ci_concurrency import ROOT, yaml


@unittest.skipIf(os.name == "nt", "executes the POSIX cgroup-report commands")
class MemoryReportTests(unittest.TestCase):
    def test_peak_anon_and_oom_counters_and_unavailable_fallback(self):
        workflow = yaml.load((ROOT / ".github/workflows/dyna-ci.yml").read_text(), Loader=yaml.BaseLoader)
        for job, name in (("lint", "Report lint compiler cache"), ("verify", "Report test compiler cache")):
            step = next(s for s in workflow["jobs"][job]["steps"] if s.get("name") == name)
            commands = "\n".join(line for line in step["run"].splitlines() if "/sys/fs/cgroup/memory." in line)
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                (root / "memory.peak").write_text("17179869184\n")
                (root / "memory.events").write_text("low 0\noom 3\noom_kill 2\noom_group_kill 0\n")
                (root / "memory.stat").write_text("file 12000000000\nanon 3456789\nanon_thp 0\n")
                script = commands.replace("/sys/fs/cgroup", directory)
                with self.subTest(job=job, available=True):
                    result = subprocess.run(["bash", "-e", "-o", "pipefail", "-c", script],
                                            capture_output=True, text=True, check=True)
                    self.assertEqual(result.stdout.splitlines(),
                                     ["memory.peak 17179869184", "oom_kill 2", "anon at job end 3456789"])
                for file in root.iterdir():
                    file.unlink()
                with self.subTest(job=job, available=False):
                    result = subprocess.run(["bash", "-e", "-o", "pipefail", "-c", script],
                                            capture_output=True, text=True, check=True)
                    self.assertEqual(result.stdout.splitlines(),
                                     ["memory.peak unavailable", "oom_kill unavailable", "anon at job end unavailable"])
                    self.assertEqual(result.stderr, "")
