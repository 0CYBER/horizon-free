"""Build parallelism: a self-hosted runner's CPU quota within its memory, fixed elsewhere."""
import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]

spec = importlib.util.spec_from_file_location("cargo_jobs", ROOT / "tools/ci/cargo_jobs.py")
cargo_jobs = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cargo_jobs)
GIB = 1 << 30


class CargoJobsTests(unittest.TestCase):
    def test_cpu_quota_bounded_by_memory_with_a_floor_of_four(self):
        cases = [
            ((8, 16 * GIB, 3072), 5),  # an 8-CPU, 16 GiB runner: memory, not its CPUs, binds
            ((8, 16 * GIB, 4096), 4),
            ((16, 64 * GIB, 3072), 16),  # CPUs bind
            ((2, 4 * GIB, 3072), 4),  # never below the old fixed cap
        ]
        for args, expected in cases:
            with self.subTest(args=args):
                self.assertEqual(cargo_jobs.jobs(*args), expected)

    @unittest.skipUnless(sys.platform == "linux", "limits() reads a Linux runner's cgroup and CPU affinity")
    def test_limits_read_the_runner_cgroup(self):
        machine_cpus = len(os.sched_getaffinity(0))
        machine_memory = os.sysconf("SC_PHYS_PAGES") * os.sysconf("SC_PAGE_SIZE")
        small = min(machine_memory, 16 * GIB)
        cases = [
            # (files under the cgroup root, expected (cpus, memory))
            ({"cpu.max": "750000 100000\n", "memory.max": f"{small}\n"}, (7, small)),  # v2 with a quota
            ({"cpu.max": "max 100000\n", "memory.max": "max\n"}, (machine_cpus, machine_memory)),  # v2, no limit
            ({"cpu/cpu.cfs_quota_us": "200000\n", "cpu/cpu.cfs_period_us": "100000\n",
              "memory/memory.limit_in_bytes": f"{small}\n"}, (2, small)),  # v1 with a quota
            ({"cpu/cpu.cfs_quota_us": "-1\n", "cpu/cpu.cfs_period_us": "100000\n",
              "memory/memory.limit_in_bytes": "9223372036854771712\n"}, (machine_cpus, machine_memory)),  # v1, none
            ({}, (machine_cpus, machine_memory)),  # no cgroup files: a GitHub-hosted job's root cgroup
        ]
        for files, expected in cases:
            with self.subTest(files=sorted(files)), tempfile.TemporaryDirectory() as directory:
                cgroup = Path(directory)
                for name, text in files.items():
                    (cgroup / name).parent.mkdir(parents=True, exist_ok=True)
                    (cgroup / name).write_text(text)
                self.assertEqual(cargo_jobs.limits(cgroup), expected)
                result = subprocess.run([sys.executable, ROOT / "tools/ci/cargo_jobs.py", "--job-mib", "3072",
                                         "--cgroup", cgroup], capture_output=True, text=True, check=True)
                self.assertEqual(int(result.stdout), cargo_jobs.jobs(*expected, 3072))

    def test_cap_bounds_every_cargo_run_and_test_harness_of_the_job(self):
        with tempfile.TemporaryDirectory() as directory:
            workspace = Path(directory) / "work/repo"
            workspace.mkdir(parents=True)
            env = Path(directory) / "env"
            env.write_text("")
            cargo_jobs.cap(5, 8, workspace, env)
            config = (workspace.parent / ".cargo/config.toml").read_text()
            self.assertIn("[build]\njobs = 5\n", config)
            self.assertEqual(env.read_text(), "RUST_TEST_THREADS=8\nCI_JOBS=5\nCI_CPUS=8\n")


if __name__ == "__main__":
    unittest.main()
