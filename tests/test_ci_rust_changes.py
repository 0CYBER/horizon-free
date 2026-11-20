import os
from pathlib import Path
import posixpath
import re
import subprocess
import tempfile
import unittest

from tests.symlinks import symlink_or_skip
from tools.ci.rust_changes import (
    EMBEDDED, needs_rust_alone, UNKNOWN_CHANGE, changed_entries, changed_paths, mentions_replaceable, needs_launcher, needs_mod_points, needs_rust, needs_server_build, needs_tooling,
    selections,
)


class RustChangesTests(unittest.TestCase):
    def test_only_content_changes_skip_rust(self):
        self.assertFalse(needs_rust([]))
        self.assertFalse(needs_rust([
            "docs/setup.md", "docs/media/banner.webp", "README.md",
        ]))
        # The Python tools: verify's Python steps run their tests, and no Rust build or test reads them.
        self.assertFalse(needs_rust([
            "tools/parity.py", "tools/test_tick_parity.py", "tests/test_ci_runners.py", "docs/setup.md",
        ]))
        for path in [
            "crates/horizon-client/src/game.rs", "xtask/src/main.rs", "Cargo.lock",
            ".cargo/config.toml", ".github/workflows/ci.yml", "tools/ci/rust_changes.py",
            "tools/release_train.py", "tools/ci/sign_release.sh", "tools/native-hook-fragments/a.json",
            "tools/parity_hosts.json", "mods/horizon/src/x.py",
            "research-lock.json", "crates/horizon-theme/tokens.css", "unknown-file",
        ]:
            with self.subTest(path=path):
                self.assertTrue(needs_rust(["README.md", path]))

    def test_launcher_checks_follow_the_crates_it_builds(self):
        for path in ["crates/horizon-client/src/game.rs", "launcher/links.json", "launcher/docs/a.md",
                     "launcher/README.md", "website/src/index.html", "tools/pr_ready.py"]:
            with self.subTest(path=path):
                self.assertFalse(needs_launcher([path]))
        for path in ["launcher/src/main.rs", "launcher/Cargo.lock", "crates/horizon-port-pack/src/net.rs",
                     "crates/horizon-mcp/src/lib.rs", "crates/horizon-mod-api-gen/src/injection.rs",
                     "crates/horizon-theme/src/lib.rs", "Cargo.toml", "Cargo.lock",
                     ".github/workflows/ci.yml"]:
            with self.subTest(path=path):
                self.assertTrue(needs_launcher(["README.md", path]))

    def test_the_rust_parts_setup_script_runs_every_part_that_calls_it(self):
        # verify's launcher, lint, server build and Rust tests set up with it; a change to it alone runs them all.
        ci = (Path(__file__).resolve().parents[1] / ".github/workflows/ci.yml").read_text()
        self.assertIn("tools/ci/install_rust.sh", ci)
        self.assertTrue(needs_launcher(["tools/ci/install_rust.sh"]))
        self.assertTrue(needs_rust(["tools/ci/install_rust.sh"]))
        self.assertEqual(selections([("M", "tools/ci/install_rust.sh")])["rust_required"], True)

    def test_the_server_build_clippy_follows_the_client_crates_and_manifests(self):
        for path in ["crates/horizon-world/src/lib.rs", "crates/horizon-perf/src/lib.rs", "tools/parity.py",
                     "docs/setup.md", "launcher/src/main.rs"]:
            with self.subTest(path=path):
                self.assertFalse(needs_server_build([path]))
        for path in ["crates/horizon-client/src/game.rs", "crates/horizon-client-render/src/hud.rs",
                     "crates/horizon-client-core/src/lang.rs", "crates/horizon-world/Cargo.toml", "Cargo.lock",
                     "Cargo.toml", ".cargo/config.toml", ".github/workflows/ci.yml", "tools/ci/rust_changes.py"]:
            with self.subTest(path=path):
                self.assertTrue(needs_server_build(["docs/setup.md", path]))

    def test_moving_source_into_docs_still_requires_rust(self):
        with tempfile.TemporaryDirectory() as directory:
            def git(*args):
                return subprocess.check_output(["git", "-C", directory, *args])

            git("init", "--quiet")
            source = Path(directory, "source.rs")
            source.write_text("fn main() {}\n")
            git("add", ".")
            git("-c", "user.name=CI test", "-c", "user.email=ci@example.invalid",
                "-c", "commit.gpgsign=false", "commit", "--quiet", "-m", "base")
            base = git("rev-parse", "HEAD").decode().strip()
            Path(directory, "docs").mkdir()
            source.rename(Path(directory, "docs/example.rs"))
            git("add", "-A")
            git("-c", "user.name=CI test", "-c", "user.email=ci@example.invalid",
                "-c", "commit.gpgsign=false", "commit", "--quiet", "-m", "move")
            # Exercise the same diff command as CI in the temporary repository.
            previous = Path.cwd()
            try:
                os.chdir(directory)
                paths = changed_paths(base, "HEAD")
            finally:
                os.chdir(previous)
            self.assertIn("source.rs", paths)
            self.assertTrue(needs_rust(paths))

    def test_tooling_keeps_watched_markdown_and_non_documentation(self):
        for path in [
            "tools/README.md", "tests/fixtures/example.md", ".github/pull_request_template.md",
            "docs/modding/src/introduction.md", "roadmap/journal/example.md",
            "README.md", "ROADMAP.md", "docs/README.md",
            "crates/horizon-client/src/game.rs", "packaging/server/start.sh", "research-lock.json",
            "unknown-file",
        ]:
            with self.subTest(path=path):
                self.assertTrue(needs_tooling([("M", path)]))
                self.assertTrue(needs_tooling([("M", "docs/setup.md"), ("M", path)]))
        self.assertFalse(needs_tooling([("M", "docs/setup.md")]))
        self.assertFalse(needs_tooling([]))

    def test_documentation_membership_changes_keep_link_checks(self):
        for status in ["A", "D", "T", "R100", "?"]:
            with self.subTest(status=status):
                self.assertTrue(needs_tooling([(status, "docs/setup.md")]))
        self.assertTrue(needs_tooling([("D", "docs/old.md"), ("A", "docs/new.md")]))

    def test_boundary_uses_fail_closed_rust_selection(self):
        # The lint boundary step consumes this same central decision.
        for path in ["Cargo.toml", "crates/example/Cargo.toml", "launcher/Cargo.toml",
                     "crates/example/src/lib.rs", UNKNOWN_CHANGE]:
            with self.subTest(path=path):
                self.assertTrue(selections([("M", path)])["rust_required"])
        self.assertFalse(selections([("M", "docs/setup.md")])["rust_required"])

    def test_shared_selections_preserve_launcher_requirements(self):
        self.assertEqual(selections([("M", "docs/setup.md")]), {
            "rust_required": False, "launcher_required": False, "tooling_required": False,
        })
        for path in ["Cargo.lock", "tools/ci/rust_changes.py", ".github/workflows/ci.yml"]:
            selected = selections([("M", path)])
            self.assertTrue(selected["rust_required"])
            self.assertTrue(selected["launcher_required"])
            self.assertTrue(selected["tooling_required"])
        # A documentation change in a mod API crate selects no Rust checks.
        self.assertFalse(selections([("M", "crates/horizon-mod-api/README.md")])["rust_required"])

    def test_git_status_and_cli_selections_for_actual_documentation_changes(self):
        selector = Path(__file__).resolve().parents[1] / "tools/ci/rust_changes.py"
        with tempfile.TemporaryDirectory() as directory:
            def git(*args):
                return subprocess.check_output(["git", "-C", directory, *args])

            git("init", "--quiet")
            docs = Path(directory, "docs")
            docs.mkdir()
            for name in ["modified.md", "deleted.md", "old.md", "type.md"]:
                (docs / name).write_text("old\n")
            git("add", ".")
            commit = ["-c", "user.name=CI test", "-c", "user.email=ci@example.invalid",
                      "-c", "commit.gpgsign=false", "commit", "--quiet", "-m", "fixture"]
            git(*commit)
            base = git("rev-parse", "HEAD").decode().strip()
            (docs / "modified.md").write_text("new\n")
            (docs / "deleted.md").unlink()
            (docs / "old.md").rename(docs / "renamed.md")
            (docs / "added.md").write_text("added\n")
            (docs / "type.md").unlink()
            symlink_or_skip(docs / "type.md", "modified.md")
            git("add", "-A")
            git(*commit)
            previous = Path.cwd()
            try:
                os.chdir(directory)
                entries = changed_entries(base, "HEAD")
            finally:
                os.chdir(previous)
            self.assertEqual(set(entries), {
                ("M", "docs/modified.md"), ("D", "docs/deleted.md"), ("D", "docs/old.md"),
                ("A", "docs/renamed.md"), ("A", "docs/added.md"), ("T", "docs/type.md"),
            })
            import sys
            output = subprocess.check_output(
                [sys.executable, str(selector), "--all", base, "HEAD"], cwd=directory, text=True,
            )
            self.assertEqual(dict(line.split("=") for line in output.splitlines()), {
                "rust_required": "false", "launcher_required": "false",
                "tooling_required": "true",
            })

    def test_a_base_missing_from_the_checkout_means_everything_changed(self):
        with tempfile.TemporaryDirectory() as directory:
            subprocess.check_call(["git", "-C", directory, "init", "--quiet"])
            subprocess.check_call([
                "git", "-C", directory, "-c", "user.name=CI test", "-c", "user.email=ci@example.invalid",
                "-c", "commit.gpgsign=false", "commit", "--quiet", "--allow-empty", "-m", "head",
            ])
            previous = Path.cwd()
            try:
                os.chdir(directory)
                # A base newer than the checked-out history, as when main moved after the PR event.
                paths = changed_paths("a" * 40, "HEAD")
            finally:
                os.chdir(previous)
            self.assertEqual(paths, [UNKNOWN_CHANGE])
            self.assertTrue(needs_rust(paths))
            self.assertTrue(needs_launcher(paths))


ROOT = Path(__file__).resolve().parents[1]
# include_str!/include_bytes!/include! of a literal path, beside the file or under CARGO_MANIFEST_DIR.
# Any of a macro's delimiters: include_str!("a"), include_str!["a"], include_str! {"a"}.
INCLUDE = re.compile(
    r'include(?:_str|_bytes)?!\s*[(\[{]\s*'
    r'(concat!\s*[(\[{]\s*env!\s*[(\[{]\s*"CARGO_MANIFEST_DIR"\s*[)\]}]\s*,\s*)?"([^"]*)"')


def rust_embeds():
    """Every tracked file a tracked Rust file embeds: {target: {embedding files}}."""
    tracked = set(subprocess.check_output(["git", "-C", str(ROOT), "ls-files"], text=True, encoding="utf-8").splitlines())
    embeds = {}
    for source in sorted(path for path in tracked if path.endswith(".rs")):
        text = (ROOT / source).read_text(encoding="utf-8", errors="replace")
        for match in INCLUDE.finditer(text):
            base = posixpath.dirname(source)
            if match.group(1):
                while base and f"{base}/Cargo.toml" not in tracked:
                    base = posixpath.dirname(base)
            path = match.group(2).lstrip("/") if match.group(1) else match.group(2)
            target = posixpath.normpath(posixpath.join(base, path))
            if target in tracked:
                embeds.setdefault(target, set()).add(source)
    return embeds


class EmbeddedFileTests(unittest.TestCase):
    def test_files_rust_embeds_select_the_rust_checks(self):
        for path in ["docs/modpacks/atm10-mods.csv", "docs/parity/random-chance-seeded.jsonl",
                     "crates/horizon-mod-api-gen/coverage-notes.md"]:
            with self.subTest(path=path):
                self.assertTrue(needs_rust([path]))
        self.assertTrue(needs_launcher(["docs/media/village.webp"]))
        self.assertFalse(needs_rust(["docs/setup.md", "docs/modpacks.md", "README.md"]))

    def test_the_embedded_list_is_every_embed_the_rules_would_skip(self):
        embeds = rust_embeds()
        # The scan finds the embeds at all: one the list has always had.
        self.assertIn("crates/horizon-port-pack/src/classify.rs", embeds["docs/modpacks/atm10-mods.csv"])
        skipped = {
            target: sources for target, sources in embeds.items()
            if not needs_rust_alone([target]) and any(needs_rust_alone([source]) for source in sources)
        }
        self.assertEqual({target: set(sources) for target, sources in EMBEDDED.items()}, skipped)


class ModPointSelectionTests(unittest.TestCase):
    def test_the_game_the_generator_and_the_check_select_it(self):
        for path in ["crates/horizon-client/src/game/tick.rs", "crates/horizon-client/src/game.rs",
                     "crates/horizon-mod-api-gen/src/points.rs", "crates/horizon-mod-api-macros/src/lib.rs",
                     "tools/check_mod_points_unchanged.py", "crates/horizon-world/Cargo.toml", "Cargo.lock",
                     UNKNOWN_CHANGE, ".github/workflows/ci.yml", "tools/ci/surface_report.py",
                     "tools/ci/surface.toml", "crates/horizon-mod-vocab/changes/a-note.toml",
                     "crates/horizon-core/src/nbt.rs"]:
            with self.subTest(path=path):
                self.assertTrue(needs_mod_points(["README.md", path]))

    def test_documentation_and_unrelated_changes_do_not(self):
        for path in ["docs/layers.md", "crates/horizon-mod-api/README.md", "tools/parity.py", "launcher/src/main.rs",
                     "crates/horizon-world/README.md"]:
            with self.subTest(path=path):
                self.assertFalse(needs_mod_points([path]))
        self.assertFalse(needs_mod_points([]))

    def test_a_lower_crate_file_counts_only_when_it_mentions_replaceable(self):
        self.assertFalse(needs_mod_points(["crates/horizon-world/src/lib.rs"]))
        self.assertTrue(needs_mod_points(["crates/horizon-world/src/lib.rs"], lambda files: True))
        self.assertFalse(needs_mod_points(["docs/a.md"], lambda files: True))

    def test_replaceable_is_looked_for_before_and_after_and_fails_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            def git(*args):
                return subprocess.check_output(["git", "-C", directory, *args]).decode().strip()

            git("init", "--quiet")
            Path(directory, "a.rs").write_text("#[horizon_mod_api_macros::replaceable]\npub fn a() {}\n")
            Path(directory, "b.rs").write_text("pub fn b() {}\n")
            git("add", ".")
            commit = ["-c", "user.name=CI test", "-c", "user.email=ci@example.invalid",
                      "-c", "commit.gpgsign=false", "commit", "--quiet", "-m"]
            git(*commit, "base")
            base = git("rev-parse", "HEAD")
            Path(directory, "a.rs").write_text("pub fn a() {}\n")
            git("add", ".")
            git(*commit, "head")
            previous = Path.cwd()
            try:
                os.chdir(directory)
                # The attribute was removed: only the base has the word.
                self.assertTrue(mentions_replaceable(base, "HEAD", ["a.rs"]))
                self.assertFalse(mentions_replaceable(base, "HEAD", ["b.rs"]))
                self.assertTrue(mentions_replaceable("no-such-revision", "HEAD", ["b.rs"]))
            finally:
                os.chdir(previous)


if __name__ == "__main__":
    unittest.main()
