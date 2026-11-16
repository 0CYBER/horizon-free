"""Execute release shell steps with optional archives present and absent."""

import base64
import hashlib
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
REQUIRED = {
    "horizon-linux-x86_64.tar.gz", "horizon-windows-x64.zip", "horizon-macos-arm64.zip",
    "horizon-macos-x64.zip", "horizon-server-linux-x86_64.tar.gz", "horizon-source.tar.gz",
}
OPTIONAL = ("horizon-windows-arm64.zip", "horizon-linux-musl-x86_64.tar.gz")
# The bash on PATH, as in test_pr_ready: Windows' CreateProcess looks in System32 before PATH, so a bare "bash"
# there is WSL's, which gets none of the steps' env and none of the host's paths. WSL from the Microsoft Store puts
# its bash on PATH as an app alias in WindowsApps instead (#16860).
BASH = shutil.which("bash")
POSIX_BASH = BASH is not None and not (
    os.name == "nt" and os.path.basename(os.path.dirname(os.path.normcase(BASH))) in ("system32", "windowsapps"))


def run_step(containing):
    workflow = (ROOT / ".github/workflows/nightly-build.yml").read_text()
    for match in re.finditer(r"^        run: \|\n((?:          .*\n|\n)+)", workflow, re.MULTILINE):
        script = "\n".join(line[10:] for line in match[1].splitlines())
        if containing in script:
            return script
    raise AssertionError(f"no workflow step containing {containing}")


@unittest.skipUnless(POSIX_BASH and shutil.which("openssl") and shutil.which("sha256sum"),
                     "release shell checks require a bash that is not WSL's, OpenSSL and sha256sum")
class OptionalReleaseAssetsTests(unittest.TestCase):
    def test_sign_and_publish_with_each_optional_archive_combination(self):
        for mask in range(4):
            with self.subTest(mask=mask), tempfile.TemporaryDirectory() as folder:
                root = Path(folder)
                dist = root / "dist"
                dist.mkdir()
                present = REQUIRED | {name for i, name in enumerate(OPTIONAL) if mask & (1 << i)}
                for name in present:
                    (dist / name).write_text(name)
                script_dir = root / "tools/ci"
                script_dir.mkdir(parents=True)
                shutil.copy(ROOT / "tools/ci/sign_release.sh", script_dir)
                # The steps' own env from nightly-build.yml: SHORT is the commit the title and
                # SHA256SUMS.v2.sig name (ALSO_FOR, nightly-<date>-<short>).
                env = dict(os.environ, EPHEMERAL="yes", SHA="0123456789abcdef", SHORT="0123456", DATE="2026-11-04",
                           ALSO_FOR="nightly-2026-11-04-0123456", SOURCE_TAG="nightly-test",
                           RELEASE_SIGNING_KEY="", RELEASE_SIGNING_KEY_NEXT="")
                signed = subprocess.run([BASH, "-e", "-c", run_step("sign_release.sh nightly")],
                                        cwd=root, env=env, text=True, capture_output=True)
                self.assertEqual(signed.returncode, 0, signed.stderr)
                # `name` or `*name`: sha256sum on Windows (Git Bash) marks binary mode with `*`.
                sums = {name.removeprefix("*"): digest for digest, name in
                        (line.split() for line in (dist / "SHA256SUMS").read_text().splitlines())}
                self.assertEqual(sums, {name: hashlib.sha256(name.encode()).hexdigest() for name in present})
                self.assertTrue((dist / "SHA256SUMS.sig").stat().st_size)
                self.assertTrue((dist / "SHA256SUMS.v2.sig").stat().st_size)
                (dist / "smoke-test.json").write_text("{}")
                (dist / "nightly-notes.md").write_text("test")
                # Isolate GitHub and tag rotation; validate the exact attachment arguments
                # produced by the real workflow, including paths with display labels.
                stubs = '''
python3() { :; }
gh() {
  for arg in "$@"; do
    case "$arg" in dist/*)
      file=${arg%%#*}
      test -f "$file" || return 1
      printf '%s\\n' "$file" >> published
    esac
  done
}
'''
                published = subprocess.run([BASH, "-e", "-c", stubs + run_step("gh release create nightly")],
                                           cwd=root, env=env, text=True, capture_output=True)
                self.assertEqual(published.returncode, 0, published.stderr)
                files = {Path(name).name for name in (root / "published").read_text().splitlines()}
                self.assertEqual(files, present | {"SHA256SUMS", "SHA256SUMS.sig", "SHA256SUMS.v2.sig", "smoke-test.json",
                                                   "nightly-notes.md"})
                for name in set(OPTIONAL) - present:
                    self.assertIn(f"::warning::{name} is missing", signed.stdout)
                    self.assertIn(f"::warning::{name} is missing", published.stdout)

    def test_sign_release_covers_sdk_archive_bytes_and_index(self):
        # SDK publishing workflows are deferred, but the signing helper accepts SDK assets
        # explicitly, just like game archives. Check that API without assuming a Nightly SDK job.
        build = "c" * 40
        for tag, also_for in ((f"sdk-{build}", ""), ("nightly", "nightly-2026-11-04-0123456")):
            with self.subTest(tag=tag), tempfile.TemporaryDirectory() as folder:
                root = Path(folder)
                assets = {"sdk.json": b'{"build":"' + build.encode() + b'"}\n',
                          "runtime-x86_64-unknown-linux-gnu.tar.xz": bytes(range(256)),
                          "build-data.tar.xz": b"\x00binary build data\xff"}
                for name, data in assets.items():
                    (root / name).write_bytes(data)
                result = subprocess.run(
                    [BASH, str(ROOT / "tools/ci/sign_release.sh"), tag,
                     *(str(root / name) for name in assets)],
                    env={**os.environ, "EPHEMERAL": "yes", "ALSO_FOR": also_for,
                         "RELEASE_SIGNING_KEY": "", "RELEASE_SIGNING_KEY_NEXT": ""},
                    capture_output=True, text=True,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                sums = (root / "SHA256SUMS").read_bytes()
                self.assertEqual(
                    {name.removeprefix("*"): digest for digest, name in
                     (line.split() for line in sums.decode().splitlines())},
                    {name: hashlib.sha256(data).hexdigest() for name, data in assets.items()},
                )
                # Ed25519 SubjectPublicKeyInfo: the helper emits its raw 32-byte public key.
                public = root / "public.der"
                public.write_bytes(bytes.fromhex("302a300506032b6570032100") +
                                   base64.b64decode((root / "ephemeral.pub").read_bytes()))
                signatures = [(tag, "SHA256SUMS.sig")]
                if also_for:
                    signatures.append((also_for, "SHA256SUMS.v2.sig"))
                for signed_tag, filename in signatures:
                    message, signature = root / "message", root / "signature"
                    message.write_bytes(f"horizon release {signed_tag}\n".encode() + sums)
                    signature.write_bytes(base64.b64decode((root / filename).read_bytes()))
                    verified = subprocess.run(
                        ["openssl", "pkeyutl", "-verify", "-rawin", "-pubin", "-keyform", "DER",
                         "-inkey", str(public), "-in", str(message), "-sigfile", str(signature)],
                        capture_output=True, text=True,
                    )
                    self.assertEqual(verified.returncode, 0, verified.stderr)
