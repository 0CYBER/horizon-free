"""The persistent preparation cache (HORIZON_PERSISTENT_CACHE) reuses keys and recovers its lock after a killed job."""
import os
from pathlib import Path
import signal
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "tools/ci/vanilla-data.sh"


class PreparedCacheTest(unittest.TestCase):
    def test_fresh_stale_and_dead_lock(self):
        (ROOT / "target").mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=ROOT / "target") as directory:
            root = Path(directory)
            source, cache, work, tmp = [root / x for x in ("source", "cache", "work", "temp")]
            for path in (source / "minecraft/26.3", source / "horizon-client/26.3", source / "tools", cache, work, tmp):
                path.mkdir(parents=True)
            (source / "minecraft/26.3/prepared.json").write_text("{}")
            (source / "horizon-client/26.3/extra").write_text("shared")
            (source / "horizon-client/26.3/generated").write_text("old")
            sounds = source / "horizon-client/26.3/assets/minecraft/sounds"
            sounds.mkdir(parents=True)
            (sounds / "fixture.ogg").write_bytes(b"sound fixture")
            (source / "horizon-client/26.3/horizon-data.bin").write_bytes(b"stale runtime cache")
            (work / "research-lock.json").write_text('{"version": "26.3"}')
            (work / "tools/ci").mkdir(parents=True)
            shutil.copyfile(ROOT / "tools/prepare_output.py", work / "tools/prepare_output.py")
            shutil.copyfile(ROOT / "tools/ci/seed_prepared_extras.py", work / "tools/ci/seed_prepared_extras.py")
            shutil.copyfile(ROOT / "tools/ci/preparation_step.py", work / "tools/ci/preparation_step.py")
            target = work / "target/release"
            target.mkdir(parents=True)
            prepare = target / "horizon-prepare"
            prepare.write_text('#!/bin/sh\nmkdir -p .local/horizon-client/26.3 .local/datagen\nprintf new > .local/horizon-client/26.3/generated\nprintf "{}" > .local/horizon-client/26.3/horizon-resources.json\n')
            prepare.chmod(0o755)
            binary = work / "bin"
            binary.mkdir()
            cargo = binary / "cargo"
            cargo.write_text('#!/bin/sh\nexit 0\n')
            cargo.chmod(0o755)
            env = dict(os.environ, PATH=f"{binary}:{os.environ['PATH']}", HORIZON_PERSISTENT_CACHE="true",
                       HORIZON_PREPARED_CACHE=str(cache), HORIZON_SHARED_SOURCE=str(source),
                       HORIZON_PREPARE_KEY="fixture", HORIZON_PREPARE_EXPORT_DIGEST=str(tmp / "export.digest"),
                       RUNNER_TEMP=str(tmp), CARGO_TARGET_DIR=str(work / "target"))
            def run():
                return subprocess.run(["bash", SCRIPT], cwd=work, env=env, text=True,
                                      capture_output=True, check=True).stdout
            self.assertIn("prepared key fixture stale", run())
            self.assertFalse((work / ".local/horizon-client/26.3/extra").exists())
            self.assertFalse((work / ".local/horizon-client/26.3/horizon-data.bin").exists())
            self.assertEqual((work / ".local/horizon-client/26.3/assets/minecraft/sounds/fixture.ogg").read_bytes(), b"sound fixture")
            self.assertEqual((work / ".local/horizon-client/26.3/generated").read_text(), "new")
            seeded = subprocess.check_output(["python3", ROOT / "tools/prepare_output.py", "digest",
                                              work / ".local/horizon-client/26.3"], text=True).strip()
            self.assertEqual((tmp / "export.digest").read_text().strip(), seeded)
            self.assertTrue(run().startswith("prepared key fixture fresh (manifest "))
            lock = cache / "prepared/.lock-file"
            holder = subprocess.Popen(["bash", "-c", 'exec 9>"$1"; flock 9; echo locked; exec sleep 60', "holder", str(lock)],
                                      stdout=subprocess.PIPE, text=True)
            self.assertEqual(holder.stdout.readline(), "locked\n")
            waiter = subprocess.Popen(["bash", SCRIPT], cwd=work, env=env, text=True,
                                      stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            time.sleep(0.2)
            self.assertIsNone(waiter.poll())
            holder.send_signal(signal.SIGKILL)
            holder.wait(timeout=5)
            holder.stdout.close()
            out, err = waiter.communicate(timeout=5)
            self.assertEqual(waiter.returncode, 0, err)
            self.assertTrue(out.startswith("prepared key fixture fresh (manifest "))
            # A job dying during seed leaves no completion marker; the next job rebuilds.
            other = dict(env, HORIZON_PREPARE_KEY="interrupted", SEED_STARTED=str(root / "seed-started"),
                         REAL_PYTHON=sys.executable)
            python = binary / "python3"
            python.write_text('#!/bin/sh\nif [ "$1" = -m ]; then touch "$SEED_STARTED"; exec sleep 60; fi\nexec "$REAL_PYTHON" "$@"\n')
            python.chmod(0o755)
            process = subprocess.Popen(["bash", SCRIPT], cwd=work, env=other, start_new_session=True,
                                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            for _ in range(100):
                if (root / "seed-started").exists(): break
                time.sleep(0.05)
            self.assertTrue((root / "seed-started").exists())
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=5)
            self.assertFalse((cache / "prepared/interrupted/.entries").exists())
            python.unlink()
            retry = subprocess.run(["bash", SCRIPT], cwd=work, env=other, text=True,
                                   capture_output=True, check=True)
            self.assertIn("prepared key interrupted stale", retry.stdout)
            self.assertTrue((cache / "prepared/interrupted/.entries").exists())
            self.assertTrue((cache / "prepared/fixture/.entries").exists())


if __name__ == "__main__":
    unittest.main()
