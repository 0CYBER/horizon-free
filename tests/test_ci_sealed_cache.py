"""tools/ci/sealed-cache.sh: what a cache entry holds of the vanilla data is encrypted and authenticated for its cache
key, and an entry that fails its check, or holds a member that may not be unpacked, is discarded with a warning (the
run goes on cold), never decrypted or unpacked, and never a failure."""
import hashlib
import hmac
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
SCRIPT = ROOT / "tools/ci/sealed-cache.sh"
NEEDED = ("bash", "openssl", "zstd", "tar")
KEY = "parity-oracle-v2-0123456789abcdef-java-rust-folders-other-abc123-1"
PREFIX = "parity-oracle-v2-0123456789abcdef-java-rust-folders-other-"


@unittest.skipUnless(sys.platform == "linux" and Path("/usr/bin/python3").exists()
                     and all(shutil.which(tool) for tool in NEEDED), "needs Linux with " + ", ".join(NEEDED))
class SealedCacheTests(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.root)
        self.workspace, self.temp = self.root / "work", self.root / "temp"
        self.folder = self.workspace / ".local/parity-runner"
        (self.folder / "oracle-cache").mkdir(parents=True)
        self.temp.mkdir()
        self.file = self.folder / "oracle-cache/out.txt"
        self.file.write_bytes(b"oracle output\n")
        # A nanosecond date: the oracle shim replays a run only while its outputs keep theirs.
        os.utime(self.file, ns=(1_700_000_000_123_456_789, 1_700_000_000_123_456_789))
        self.archive = self.temp / "sealed/oracle/archive"

    def run_script(self, *arguments, passphrase="correct horse"):
        env = {**os.environ, "GITHUB_WORKSPACE": str(self.workspace), "RUNNER_TEMP": str(self.temp),
               "VANILLA_DATA_PASSPHRASE": passphrase}
        return subprocess.run(["bash", str(SCRIPT), *map(str, arguments)], env=env, capture_output=True, text=True)

    def seal(self, key=KEY):
        result = self.run_script("seal", "oracle", key, self.archive, self.folder)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def unseal(self, label="oracle", passphrase="correct horse", prefix=PREFIX):
        stage = self.temp / "stage"
        shutil.rmtree(stage, ignore_errors=True)
        result = self.run_script("unseal", label, prefix, self.archive, stage, passphrase=passphrase)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return stage, result.stdout

    def forge(self, members, passphrase="correct horse", key=KEY, padding=0):
        """An entry sealed with the right passphrase and key around a tar of `members` ((TarInfo, bytes or None)
        pairs), followed by `padding` zero bytes: what only main's sealing step could write, so only the member
        check stands between it and the stage."""
        buffer = io.BytesIO()
        with tarfile.open(fileobj=buffer, mode="w", format=tarfile.PAX_FORMAT) as archive:
            for info, data in members:
                archive.addfile(info, io.BytesIO(data) if data is not None else None)
        packed = subprocess.run(["zstd", "-q", "-T0", "-3"], input=buffer.getvalue() + bytes(padding),
                                capture_output=True, check=True)
        secret = self.root / "passphrase"
        secret.write_text(passphrase)
        self.archive.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(["openssl", "enc", "-e", "-aes-256-cbc", "-pbkdf2", "-iter", "600000", "-md", "sha256", "-pass",
                        f"file:{secret}", "-out", str(self.archive)], input=packed.stdout, check=True)
        mac_key = hashlib.pbkdf2_hmac("sha256", passphrase.encode(), b"horizon-rust/horizon sealed-cache MAC key v1",
                                      600000)
        message = b"horizon sealed-cache v2\0oracle\0" + key.encode() + b"\0" + self.archive.read_bytes()
        Path(f"{self.archive}.mac").write_text(f"{key}\n{hmac.new(mac_key, message, 'sha256').hexdigest()}\n")

    @staticmethod
    def member(name, kind=tarfile.REGTYPE, link="", data=b"x"):
        info = tarfile.TarInfo(name)
        info.type, info.linkname = kind, link
        if kind == tarfile.DIRTYPE:
            info.mode = 0o755
        if kind == tarfile.REGTYPE:
            info.size = len(data)
            return info, data
        return info, None

    def inside(self, path):
        return str(self.workspace / path).lstrip("/")

    def assert_discarded(self, why, **unseal):
        stage, out = self.unseal(**unseal)
        self.assertFalse(stage.exists())
        self.assertIn(f"::warning::oracle cache entry discarded ({why})", out)

    def test_a_sealed_folder_unseals_into_the_stage_with_its_dates(self):
        self.seal()
        self.assertNotIn(b"oracle output", self.archive.read_bytes())
        stage, out = self.unseal()
        copy = stage / str(self.file).lstrip("/")
        self.assertEqual(copy.read_bytes(), b"oracle output\n")
        self.assertEqual(copy.stat().st_mtime_ns, self.file.stat().st_mtime_ns)
        self.assertIn("unsealed oracle", out)
        self.assertNotIn(str(self.file), out)

    def test_a_changed_byte_is_discarded_before_decrypting(self):
        self.seal()
        data = bytearray(self.archive.read_bytes())
        data[len(data) // 2] ^= 1
        self.archive.write_bytes(bytes(data))
        self.assert_discarded("its authentication tag does not match")

    def test_an_entry_sealed_under_another_label_is_discarded(self):
        self.seal()
        stage = self.temp / "stage"
        result = self.run_script("unseal", "prepared", PREFIX, self.archive, stage)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(stage.exists())
        self.assertIn("::warning::prepared cache entry discarded (its authentication tag does not match)", result.stdout)

    def test_an_entry_without_its_tag_is_discarded(self):
        self.seal()
        Path(f"{self.archive}.mac").unlink()
        self.assert_discarded("no authentication tag")

    def test_another_passphrase_is_discarded(self):
        self.seal()
        self.assert_discarded("its authentication tag does not match", passphrase="another")

    def test_no_entry_is_a_cold_start(self):
        stage, out = self.unseal()
        self.assertFalse(stage.exists())
        self.assertIn("no oracle cache entry", out)

    def test_an_excluded_folder_is_never_sealed_nor_placed(self):
        # The oracle shim's runtime memo: JDK file hashes guarded by stat data of one machine (tools/parity_shim.py).
        memo = self.folder / "runtime-memo"
        (memo / "jdk").mkdir(parents=True)
        (memo / "jdk/hashes.json").write_bytes(b"{}")
        result = self.run_script("seal", "oracle", KEY, self.archive, "--exclude", memo, self.folder)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        stage, _ = self.unseal()
        self.assertTrue((stage / str(self.file).lstrip("/")).exists())
        self.assertFalse((stage / str(memo).lstrip("/")).exists())
        self.assertTrue(memo.exists())

    def test_an_entry_holding_an_excluded_path_has_it_removed_on_unsealing(self):
        memo = self.folder / "runtime-memo"
        memo.mkdir()
        (memo / "x.json").write_bytes(b"{}")
        self.seal()
        stage = self.temp / "stage"
        result = self.run_script("unseal", "oracle", PREFIX, self.archive, "--exclude", memo, stage)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((stage / str(self.file).lstrip("/")).exists())
        self.assertFalse((stage / str(memo).lstrip("/")).exists())
        self.assertIn("held an excluded path", result.stdout)

    def test_only_folders_under_the_checkout_or_runner_temp_are_sealed(self):
        outside = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, outside)
        result = self.run_script("seal", "oracle", KEY, self.archive, outside)
        self.assertEqual(result.returncode, 2)
        self.assertFalse(self.archive.exists())

    def test_an_entry_sealed_for_another_key_is_discarded(self):
        # An older dataset's entry, valid under its own key, put under the key of a newer one.
        self.seal(key="parity-oracle-v2-fedcba9876543210-java-rust-folders-other-abc123-1")
        self.assert_discarded("it was sealed for another key")

    def test_the_key_beside_the_tag_is_authenticated(self):
        self.seal(key="parity-oracle-v2-fedcba9876543210-java-rust-folders-other-abc123-1")
        tags = Path(f"{self.archive}.mac")
        tags.write_text(KEY + "\n" + tags.read_text().splitlines()[1] + "\n")
        self.assert_discarded("its authentication tag does not match")

    def test_an_entry_of_the_same_prefix_unseals(self):
        # Restored by prefix: another commit's or shard's entry of the same dataset, toolchains and group.
        self.seal(key=PREFIX + "def456-2")
        stage, _ = self.unseal()
        self.assertTrue((stage / str(self.file).lstrip("/")).exists())

    def test_members_that_may_not_be_unpacked_discard_the_entry(self):
        local = self.inside(".local")
        cases = {
            "an absolute name": [self.member("/" + local + "/x")],
            "a name outside both folders": [self.member(str(self.root).lstrip("/") + "/elsewhere/x")],
            "a name with ..": [self.member(local + "/../../escape")],
            "a name twice": [self.member(local + "/x", data=b"1"), self.member(local + "/x", data=b"2")],
            # No symbolic link at all, inside or out: a target is never resolved, so none can lead through another.
            "a symbolic link inside": [self.member(local + "/b", tarfile.DIRTYPE),
                                       self.member(local + "/a", tarfile.SYMTYPE, "b")],
            "a link through a link": [self.member(local + "/b", tarfile.DIRTYPE),
                                      self.member(local + "/a", tarfile.SYMTYPE, "b"),
                                      self.member(local + "/c", tarfile.SYMTYPE, "a/../" + "../" * 12 + "etc")],
            "a link listed after a member under it": [self.member(local + "/a/x"),
                                                      self.member(local + "/a", tarfile.SYMTYPE, "/etc")],
            "a symbolic link out": [self.member(local + "/link", tarfile.SYMTYPE, "/etc")],
            "a hard link out": [self.member(local + "/hard", tarfile.LNKTYPE, "etc/passwd")],
            "a hard link before its file": [self.member(local + "/hard", tarfile.LNKTYPE, local + "/probe"),
                                            self.member(local + "/probe", data=b"binary")],
            "a hard link to a folder": [self.member(local + "/d", tarfile.DIRTYPE),
                                        self.member(local + "/hard", tarfile.LNKTYPE, local + "/d")],
            "a hard link to a hard link": [self.member(local + "/probe", data=b"binary"),
                                           self.member(local + "/one", tarfile.LNKTYPE, local + "/probe"),
                                           self.member(local + "/two", tarfile.LNKTYPE, local + "/one")],
            "a hard link to a name not quite the file's": [
                self.member(local + "/probe", data=b"binary"),
                self.member(local + "/hard", tarfile.LNKTYPE, local + "/./probe")],
            "a pipe": [self.member(local + "/pipe", tarfile.FIFOTYPE)],
            "a device": [self.member(local + "/null", tarfile.CHRTYPE)],
        }
        for case, members in cases.items():
            with self.subTest(case):
                self.forge(members)
                self.assert_discarded("it holds a member that may not be unpacked")

    def test_a_hard_link_to_a_file_listed_before_it_is_kept(self):
        # Cargo's binaries: each in deps/ and, linked, in the profile folder.
        local = self.inside(".local")
        self.forge([self.member(local + "/d", tarfile.DIRTYPE),
                    self.member(local + "/d/probe-0123", data=b"binary"),
                    self.member(local + "/probe", tarfile.LNKTYPE, local + "/d/probe-0123")])
        stage, _ = self.unseal()
        self.assertEqual((stage / local / "probe").read_bytes(), b"binary")
        self.assertEqual((stage / local / "probe").stat().st_ino, (stage / local / "d/probe-0123").stat().st_ino)

    def test_sealing_leaves_symbolic_links_out_and_keeps_hard_links(self):
        # As the prepared resources' links to the data and the shim's java link, which each run makes again.
        (self.folder / "minecraft").symlink_to(self.temp, target_is_directory=True)
        (self.folder / "java").symlink_to(self.file)
        os.link(self.file, self.folder / "linked.txt")
        result = self.run_script("seal", "oracle", KEY, self.archive, self.folder)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("left out 2 links and special files", result.stdout)
        stage, _ = self.unseal()
        copy = stage / str(self.folder).lstrip("/")
        self.assertFalse((copy / "minecraft").is_symlink() or (copy / "java").is_symlink())
        self.assertEqual((copy / "linked.txt").stat().st_ino, (copy / "oracle-cache/out.txt").stat().st_ino)
        self.assertEqual((copy / "linked.txt").read_bytes(), b"oracle output\n")

    def test_a_folder_given_as_a_symbolic_link_is_left_out_not_followed(self):
        # Followed, it would be sealed under the name it leads to, and the name given would be missing on restore.
        real = self.workspace / "real"
        real.mkdir()
        (real / "inside.txt").write_bytes(b"x")
        linked = self.workspace / ".local/linked"
        linked.symlink_to(real, target_is_directory=True)
        for given in (linked, f"{linked}/", f"{linked}//", f"{linked}///"):
            with self.subTest(given=str(given)):
                result = self.run_script("seal", "oracle", KEY, self.archive, given, self.folder)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertIn("::warning::a oracle folder to seal is a symbolic link: left out", result.stdout)
                self.assertIn("left out 1 links and special files", result.stdout)
                stage, _ = self.unseal()
                self.assertTrue((stage / str(self.file).lstrip("/")).exists())
                self.assertFalse((stage / str(real).lstrip("/")).exists())
                self.assertFalse((stage / str(linked).lstrip("/")).exists())

    def test_an_unpacking_that_fails_leaves_no_stage_even_with_read_only_folders(self):
        # A file under a file fails tar halfway, after a folder without write access was made: nothing reaches the
        # stage, and the private folder goes too.
        local = self.inside(".local")
        locked, _ = self.member(local + "/locked", tarfile.DIRTYPE)
        locked.mode = 0o555
        self.forge([(locked, None), self.member(local + "/locked/y"), self.member(local + "/a"),
                    self.member(local + "/a/x")])
        self.assert_discarded("it did not unpack")
        self.assertEqual([path.name for path in self.temp.iterdir() if path.name.startswith("sealed-cache.")], [])

    def test_padding_after_the_end_of_the_archive_is_read_and_ignored(self):
        # tar pads to whole records; a check that stopped at the end marker would leave zstd writing into a closed pipe.
        local = self.inside(".local")
        self.forge([self.member(local + "/probe", data=b"binary")], padding=4 << 20)
        stage, _ = self.unseal()
        self.assertEqual((stage / local / "probe").read_bytes(), b"binary")


if __name__ == "__main__":
    unittest.main()
