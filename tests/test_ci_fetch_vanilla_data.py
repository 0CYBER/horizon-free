"""tools/ci/fetch-vanilla-data.sh with VANILLA_DATA_CACHE: a restored folder of the dataset's encrypted chunks is used
only when it is complete, of plain files, and matches the MANIFEST; anything else (an incomplete or tampered
restore) downloads the dataset instead, never fails the step. git, ssh-keygen and zstd are stand-ins; openssl and tar
are real."""

import hashlib
import io
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "tools/ci/fetch-vanilla-data.sh"
DATASET = "26.3-linux-x64"
STUBS = {
    "ssh-keygen": "exit 0\n",
    "zstd": "exec cat\n",
    # clone copies the fake repository; show reads a file from it; sparse-checkout does nothing.
    "git": 'if [ "$1" = clone ]; then eval "last=\\${$#}"; cp -R "$FAKE_REPO" "$last"; exit 0; fi\n'
           'repo=$2; shift 2\n'
           'case "$1" in show) cat "$repo/${2#HEAD:}" ;; *) : ;; esac\n',
}


@unittest.skipUnless(sys.platform == "linux" and all(shutil.which(tool) for tool in ("bash", "openssl", "tar")),
                     "needs Linux with bash, openssl, tar")
class FetchFromCacheTests(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.root)
        stubs = self.root / "stubs"
        stubs.mkdir()
        for name, body in STUBS.items():
            (stubs / name).write_text("#!/bin/sh\n" + body)
            (stubs / name).chmod(0o755)
        buffer = io.BytesIO()
        with tarfile.open(fileobj=buffer, mode="w") as archive:
            info = tarfile.TarInfo("minecraft/version.json")
            info.size = 3
            archive.addfile(info, io.BytesIO(b"26\n"))
        encrypted = subprocess.run(["openssl", "enc", "-e", "-aes-256-cbc", "-pbkdf2", "-iter", "600000", "-md",
                                    "sha256", "-pass", "pass:pass phrase"], input=buffer.getvalue(),
                                   capture_output=True, check=True).stdout
        self.repo = self.root / "repo"
        (self.repo / DATASET).mkdir(parents=True)
        half = len(encrypted) // 2
        (self.repo / DATASET / "chunk-0").write_bytes(encrypted[:half])
        (self.repo / DATASET / "chunk-1").write_bytes(encrypted[half:])
        (self.repo / DATASET / "MANIFEST").write_text(
            f"sha256 {hashlib.sha256(encrypted).hexdigest()}\nchunk-0\nchunk-1\n")
        self.cache, self.temp, self.workspace = self.root / "cache", self.root / "temp", self.root / "work"
        self.temp.mkdir()
        self.workspace.mkdir()
        self.env = {**os.environ, "PATH": f"{stubs}:{os.environ['PATH']}", "FAKE_REPO": str(self.repo),
                    "VANILLA_DATA_DEPLOY_KEY": "key", "VANILLA_DATA_PASSPHRASE": "pass phrase",
                    "VANILLA_DATA_CACHE": str(self.cache), "GITHUB_WORKSPACE": str(self.workspace),
                    "RUNNER_TEMP": str(self.temp)}

    def fetch(self):
        destination = self.root / "data"
        shutil.rmtree(destination, ignore_errors=True)
        destination.mkdir()
        output = self.root / "output"
        output.write_text("")
        result = subprocess.run(["bash", str(SCRIPT), str(destination), DATASET],
                                env={**self.env, "GITHUB_OUTPUT": str(output)}, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual((destination / "minecraft/version.json").read_bytes(), b"26\n")
        return dict(line.split("=", 1) for line in output.read_text().splitlines())["cache"]

    def test_a_complete_cache_is_used_and_an_incomplete_one_is_downloaded_again(self):
        self.assertEqual(self.fetch(), "refreshed")
        self.assertEqual(self.fetch(), "hit")
        chunk = self.cache / DATASET / "chunk-1"
        elsewhere = self.root / "elsewhere"
        cases = {
            "a missing chunk": lambda: chunk.unlink(),
            "a truncated chunk": lambda: chunk.write_bytes(chunk.read_bytes()[:5]),
            "a missing MANIFEST": lambda: (self.cache / DATASET / "MANIFEST").unlink(),
            "a chunk that is a link": lambda: (shutil.copy(chunk, elsewhere), chunk.unlink(),
                                               chunk.symlink_to(elsewhere)),
            "a dataset folder that is a link": lambda: (shutil.move(self.cache / DATASET, elsewhere),
                                                        (self.cache / DATASET).symlink_to(elsewhere)),
        }
        for case, damage in cases.items():
            with self.subTest(case):
                damage()
                self.assertEqual(self.fetch(), "refreshed")
                self.assertFalse((self.cache / DATASET).is_symlink() or chunk.is_symlink())
                self.assertEqual(self.fetch(), "hit")
                shutil.rmtree(elsewhere, ignore_errors=True)
                elsewhere.unlink(missing_ok=True)


if __name__ == "__main__":
    unittest.main()
