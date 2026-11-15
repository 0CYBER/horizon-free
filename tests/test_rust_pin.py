"""CI, the launcher's native-mod toolchain and the wasm mods' toolchain use one Rust: tools/ci/rust-version.

A workflow names no floating channel (`stable`, `beta`, `nightly`) and no release of its own. The allowed forms:
- `bash tools/ci/rustup_install.sh [rustup toolchain install options]`: installs the pin and makes it the job's
  toolchain, so `cargo`, `rustc` and `rustfmt` run bare;
- `dtolnay/rust-toolchain@master` with `toolchain: ${{ steps.<id>.outputs.version }}[-<target>]`, the output of a
  step that reads the pin file;
- `"$(cat …tools/ci/rust-version)"` (or a shell variable holding it) wherever a command needs the release spelled out.
"""

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORKFLOWS = ROOT / ".github/workflows"
# Upstream's workflows, disabled on this fork and kept as upstream writes them for syncs: nothing runs them.
DISABLED = {"ci.yml", "launcher.yml", "mod-registry.yml", "nightly.yml", "pack-bench.yml", "parity-main.yml"}
RELEASE = r"(?:stable|beta|nightly|\d+\.\d+(?:\.\d+)?)\b"
FORBIDDEN = [
    (re.compile(rf"rustup_install\.sh\s+{RELEASE}"), "rustup_install.sh takes the pin itself"),
    (re.compile(rf"(?:cargo|rustc|rustfmt|rustdoc)\s+\+{RELEASE}"), "a toolchain override"),
    (re.compile(rf"rustup\s+(?:toolchain\s+install|default|run|which\s+--toolchain|\S+\s+--toolchain)\s+{RELEASE}"),
     "a toolchain spelled out"),
    (re.compile(r"^\s*RUSTUP_TOOLCHAIN\s*:"), "a step's RUSTUP_TOOLCHAIN overrides the job's pin"),
    (re.compile(r"dtolnay/rust-toolchain@(?!master\b)"), "the action's channel ref"),
    (re.compile(rf"^\s*toolchain:\s*{RELEASE}"), "a toolchain spelled out"),
]


def violations(name: str, text: str) -> list[str]:
    found = []
    lines = text.splitlines()
    for number, line in enumerate(lines, 1):
        code = line.split(" #", 1)[0] if not line.lstrip().startswith("#") else ""
        for pattern, why in FORBIDDEN:
            if pattern.search(code):
                found.append(f"{name}:{number}: {why}: {line.strip()}")
        if "dtolnay/rust-toolchain@master" in code:
            following = "\n".join(lines[number : number + 4])
            if not re.search(r"toolchain:\s*\$\{\{\s*steps\.\w+\.outputs\.version\s*\}\}", following):
                found.append(f"{name}:{number}: the action takes the pin from a step's output")
    return found


class RustPinTests(unittest.TestCase):
    def test_the_pin_is_one_exact_release(self):
        self.assertRegex((ROOT / "tools/ci/rust-version").read_text(), r"^\d+\.\d+\.\d+\n?$")

    def test_the_launcher_and_wasm_toolchains_read_the_pin(self):
        for path, include in (("launcher/src/rust_toolchain.rs", '"../../tools/ci/rust-version"'),
                              ("crates/cargo-horizon-mod/src/wasm.rs", '"../../../tools/ci/rust-version"')):
            with self.subTest(path=path):
                self.assertIn(f"include_str!({include}).trim_ascii()", (ROOT / path).read_text())

    def test_active_workflows_use_the_pin(self):
        found = []
        for path in sorted(WORKFLOWS.glob("*.yml")):
            if path.name not in DISABLED:
                found += violations(path.name, path.read_text())
        self.assertEqual(found, [])

    def test_the_check_catches_each_floating_form(self):
        for line in ("bash tools/ci/rustup_install.sh stable --profile minimal", "cargo +stable test",
                     "rustup run stable rustfmt x.rs", "rustup toolchain install 1.99.1 --profile minimal",
                     "    RUSTUP_TOOLCHAIN: stable", "- uses: dtolnay/rust-toolchain@stable",
                     "rustup which --toolchain 1.99.1 cargo", "  toolchain: 1.99.1"):
            with self.subTest(line=line):
                self.assertTrue(violations("x.yml", line))
        allowed = ('bash tools/ci/rustup_install.sh --profile minimal\n'
                   'rustup toolchain install "$(cat tools/ci/rust-version)" --profile minimal\n'
                   '- uses: dtolnay/rust-toolchain@master\n  with:\n    toolchain: ${{ steps.rust.outputs.version }}\n'
                   '# cargo +stable in a comment\n')
        self.assertEqual(violations("x.yml", allowed), [])


if __name__ == "__main__":
    unittest.main()
