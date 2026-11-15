import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from tools import check_addresses, merge_base


def strings(source):
    """The string literals a source file keeps once its #[cfg(test)] items are dropped."""
    literals, mask = check_addresses.tokenize(source)
    ranges, files = check_addresses.test_items(source, mask)
    return [s for offset, s in literals if not any(a <= offset < b for a, b in ranges)], files


CONFIG = {
    "homes": ["crates/horizon-addresses/src/lib.rs"],
    "vanilla": {"textures.minecraft.net": "skins"},
    "not_addresses": {"ddns.net": "a suffix"},
    "allowed": ["crates/horizon-telemetry/src/lib.rs telemetry.playhorizon.dev"],
}


class HostsTests(unittest.TestCase):
    def test_urls_and_whole_host_strings(self):
        self.assertEqual(check_addresses.hosts("https://cosmetics.playhorizon.dev/v1/presence"),
                         ["cosmetics.playhorizon.dev"])
        self.assertEqual(check_addresses.hosts("stun.l.google.com:19302"), ["stun.l.google.com"])
        self.assertEqual(check_addresses.hosts("wss://user:pw@relay.playhorizon.dev:443/x"),
                         ["relay.playhorizon.dev"])

    def test_text_translation_keys_and_placeholders_are_not_hosts(self):
        for text in ["Download it at playhorizon.dev, then press Play.", "options.online", "mco.info",
                     "https://example.com/a", "https://foo.test", "http://127.0.0.1:8080", "http://192.168.1.1/",
                     "https://map.example.net", "http://192.0.2.7:8100", "vertex.xyz * 2.0"]:
            self.assertEqual(check_addresses.hosts(text), [], text)

    def test_public_ip_in_a_url_counts(self):
        self.assertEqual(check_addresses.hosts("http://8.8.8.8/"), ["8.8.8.8"])


class TokenizeTests(unittest.TestCase):
    def test_comments_are_not_strings(self):
        found, _ = strings('// "https://a.com"\n/* "https://b.com" */ let x = "https://c.com";')
        self.assertEqual(found, ["https://c.com"])

    def test_raw_strings_char_literals_and_lifetimes(self):
        found, _ = strings("fn f<'a>(c: char) { if c == '\"' {} let r = r#\"x \"q\" y\"#; let s = \"z\"; }")
        self.assertEqual(found, ['x "q" y', "z"])

    def test_cfg_test_modules_are_skipped_whatever_their_strings_hold(self):
        source = ('const A: &str = "https://a.com";\n#[cfg(test)]\nmod tests {\n    const R: &str = r#"\n}\n"#;\n'
                  '    const B: &str = "https://b.com";\n}\nconst C: &str = "https://c.com";\n')
        found, _ = strings(source)
        self.assertEqual(found, ["https://a.com", "https://c.com"])

    def test_cfg_test_files_are_named(self):
        _, files = strings('#[cfg(test)]\nmod tests;\n#[cfg(test)]\n#[path = "fetch_tests.rs"]\nmod fetch_tests;\n')
        self.assertEqual(files, ["tests", "fetch_tests.rs"])

    def test_other_cfg_test_items_are_skipped(self):
        found, _ = strings('#[cfg(test)]\nconst T: [&str; 1] = ["https://t.com"];\nconst U: &str = "https://u.com";')
        self.assertEqual(found, ["https://u.com"])


class CheckTests(unittest.TestCase):
    def run_check(self, found, base=None):
        return check_addresses.check(CONFIG, set(found), base)

    def test_home_vanilla_not_addresses_and_allowed_pass(self):
        found = [("crates/horizon-addresses/src/lib.rs", "cosmetics.playhorizon.dev"),
                 ("crates/horizon-client-render/src/skins.rs", "textures.minecraft.net"),
                 ("crates/horizon-client-render/src/bluemap_detect.rs", "ddns.net"),
                 ("crates/horizon-telemetry/src/lib.rs", "telemetry.playhorizon.dev")]
        self.assertEqual(self.run_check(found), ([], []))

    def test_a_new_address_outside_the_home_fails_and_names_the_doc(self):
        problems, _ = self.run_check([("crates/horizon-client/src/net.rs", "relay.playhorizon.dev")])
        self.assertIn("relay.playhorizon.dev", problems[0])
        self.assertIn("docs/layers.md#built-in-addresses", problems[0])

    def test_an_address_that_left_its_file_is_a_note(self):
        problems, notes = self.run_check([])
        self.assertEqual(problems, [])
        self.assertIn("telemetry.playhorizon.dev", notes[0])

    def test_the_list_may_not_take_new_hosts_or_grow(self):
        config = dict(CONFIG, allowed=CONFIG["allowed"] + ["crates/x/src/lib.rs relay.playhorizon.dev"])
        problems, _ = check_addresses.check(config, set(), {"allowed": CONFIG["allowed"]})
        self.assertTrue(any("relay.playhorizon.dev: added" in p for p in problems))
        self.assertTrue(any("grew from 1 to 2" in p for p in problems))

    def test_an_entry_may_move_with_its_code(self):
        moved = {"allowed": ["crates/horizon-telemetry/src/upload.rs telemetry.playhorizon.dev"]}
        config = dict(CONFIG, allowed=moved["allowed"])
        found = {("crates/horizon-telemetry/src/upload.rs", "telemetry.playhorizon.dev")}
        self.assertEqual(check_addresses.check(config, found, {"allowed": CONFIG["allowed"]}), ([], []))


class UpstreamRestorationTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.git("init", "-q", "-b", "main")
        self.git("config", "user.name", "Fixture")
        self.git("config", "user.email", "fixture@example.invalid")
        (self.root / "tools").mkdir()
        (self.root / "tools/addresses.json").write_text('{"allowed": []}')
        self.git("add", "tools")
        self.git("commit", "-q", "-m", "base")
        self.base = self.git("rev-parse", "HEAD").strip()
        self.row = "crates/service/src/lib.rs telemetry.playhorizon.dev"
        self.git("checkout", "-q", "-b", "upstream")
        (self.root / "tools/addresses.json").write_text(json.dumps({"allowed": [self.row]}))
        self.git("commit", "-q", "-am", "upstream address")
        remote = self.root / "remote.git"
        self.git("init", "-q", "--bare", str(remote))
        self.git("push", "-q", str(remote), "HEAD:refs/heads/main")
        self.git("checkout", "-q", "-b", "work", self.base)
        (self.root / "fork").write_text("fork")
        self.git("add", "fork")
        self.git("commit", "-q", "-m", "fork")
        self.git("merge", "-q", "--no-edit", "upstream")
        for patch in (mock.patch.object(check_addresses, "ROOT", self.root),
                      mock.patch.object(merge_base, "ROOT", self.root),
                      mock.patch.dict(os.environ, {"HORIZON_UPSTREAM_URL": str(remote)})):
            patch.start()
            self.addCleanup(patch.stop)

    def git(self, *args):
        return subprocess.run(["git", *args], cwd=self.root, check=True,
                              capture_output=True, text=True).stdout

    def problems(self, row):
        upstream, errors = check_addresses.restored_upstream_rows(self.base)
        self.assertEqual(errors, [])
        found = {tuple(row.split(" ", 1))}
        return check_addresses.check({"allowed": [row]}, found, {"allowed": []}, upstream)[0]

    def test_verbatim_upstream_row_with_verified_sync_passes(self):
        self.assertEqual(self.problems(self.row), [])

    def test_fork_only_row_fails(self):
        self.assertTrue(self.problems("crates/fork/src/lib.rs fork.playhorizon.dev"))

    def test_one_character_change_to_upstream_row_fails(self):
        self.assertTrue(self.problems(self.row.replace("service/", "service2/")))

    def test_forged_sync_provenance_cannot_restore_rows(self):
        self.git("reset", "-q", "--hard", "HEAD^")
        self.git("checkout", "-q", "-b", "forgery", self.base)
        (self.root / "tools/addresses.json").write_text(json.dumps({"allowed": [self.row]}))
        self.git("commit", "-q", "-am", "forged upstream address")
        self.git("checkout", "-q", "work")
        self.git("merge", "-q", "--no-edit", "forgery")
        rows, errors = check_addresses.restored_upstream_rows(self.base)
        self.assertEqual(rows, set())
        self.assertEqual(errors, [])

    def test_verbatim_row_without_a_merge_of_upstream_fails(self):
        self.git("reset", "-q", "--hard", "HEAD^")
        self.assertTrue(self.problems(self.row))


class PullRequestMergeTests(unittest.TestCase):
    def test_a_pull_request_merge_is_checked_against_main_as_merged(self):
        """main allowed a new address after the branch left it: a pull request merged into that main didn't add it."""
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            env = dict(os.environ, GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@t.invalid",
                       GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@t.invalid")

            def run(*args):
                return subprocess.run(["git", *args], cwd=root, check=True, capture_output=True, env=env,
                                      text=True).stdout.strip()

            def config(allowed):
                return json.dumps({"homes": [], "vanilla": {}, "not_addresses": {}, "allowed": allowed})

            run("init", "-q", "-b", "main")
            (root / "tools").mkdir()
            (root / "tools/addresses.json").write_text(config([]))
            (root / "tools/layers.json").write_text('{"crates": {}}')
            (root / "crates/a/src").mkdir(parents=True)
            (root / "crates/a/src/lib.rs").write_text("pub fn a() {}\n")
            run("add", ".")
            run("commit", "-q", "-m", "base")
            base = run("rev-parse", "HEAD")
            run("checkout", "-q", "-b", "work")
            (root / "crates/a/src/lib.rs").write_text("pub fn a() -> u8 { 1 }\n")
            run("commit", "-q", "-am", "unrelated change")
            run("checkout", "-q", "main")
            (root / "crates/x/src").mkdir(parents=True)
            (root / "crates/x/src/lib.rs").write_text('pub const URL: &str = "https://relay.playhorizon.dev";\n')
            (root / "tools/addresses.json").write_text(config(["crates/x/src/lib.rs relay.playhorizon.dev"]))
            run("add", ".")
            run("commit", "-q", "-m", "main allows an address")
            run("checkout", "-q", "--detach", "main")
            run("merge", "-q", "--no-ff", "-m", "merge work", "work")
            fetch = mock.Mock(return_value=["no network"])
            with mock.patch.object(check_addresses, "ROOT", root), mock.patch.object(merge_base, "ROOT", root), \
                    mock.patch.object(merge_base, "fetch_upstream_main", fetch), \
                    mock.patch("sys.stdout", new_callable=io.StringIO) as out:
                self.assertEqual(check_addresses.main(["--base", base, "--pull-request-merge"]), 0)
                fetch.assert_not_called()  # a merge and no violation never reaches the network
                # Against the branch's old base, main's new entry was charged to the pull request, and only that
                # violation looks for an upstream inventory that could excuse it.
                self.assertEqual(check_addresses.main(["--base", base]), 1)
                fetch.assert_called_once()
            self.assertIn("relay.playhorizon.dev", out.getvalue())


class BaseConfigTests(unittest.TestCase):
    def test_the_base_is_where_the_branch_left_main(self):
        """An entry main dropped after the branch left it is still on the branch's base: the branch didn't add it."""
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            env = dict(os.environ, GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@t.invalid",
                       GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@t.invalid")

            def run(*args):
                subprocess.run(["git", *args], cwd=root, check=True, capture_output=True, env=env)

            def allow(*entries):
                (root / "tools").mkdir(exist_ok=True)
                (root / "tools" / "addresses.json").write_text(json.dumps({"allowed": list(entries)}))

            run("init", "-q", "-b", "main")
            allow("crates/horizon-telemetry/src/lib.rs telemetry.playhorizon.dev")
            run("add", ".")
            run("commit", "-q", "-m", "base")
            run("checkout", "-q", "-b", "work")
            run("commit", "-q", "--allow-empty", "-m", "work")
            run("checkout", "-q", "main")
            allow()
            run("commit", "-q", "-am", "main drops the entry")
            run("checkout", "-q", "work")
            with mock.patch.object(check_addresses, "ROOT", root):
                self.assertEqual(check_addresses.base_config("main"), {"allowed": ["crates/horizon-telemetry/src/lib.rs telemetry.playhorizon.dev"]})


if __name__ == "__main__":
    unittest.main()
