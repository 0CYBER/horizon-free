"""Exercise workflow routing expressions and smoke commands with actual matrix entries."""
import itertools
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import tempfile
import textwrap
import unittest

from tests.test_ci_runners import jobs, runs_on

ROOT = Path(__file__).resolve().parents[1]
GAME = ROOT / ".github/workflows/game-build.yml"
NIGHTLY = ROOT / ".github/workflows/nightly-build.yml"
SMOKE = ROOT / "tools/ci/game_smoke.sh"


class NightlyRoutingTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("node"), "Node evaluates GitHub's JavaScript-style expressions")
    def test_real_matrix_routes_both_trust_levels(self):
        text = GAME.read_text()
        workflow_jobs = dict(jobs(text))
        guard = workflow_jobs["runner-configuration"]
        guard_if = re.search(r"^\s*if:\s*(.*)$", guard, re.M).group(1)
        self.assertEqual(runs_on(guard), "ubuntu-latest")
        self.assertIn("needs: runner-configuration", workflow_jobs["exporters"])
        self.assertIn("needs: [runner-configuration, exporters]", workflow_jobs["build"])
        expressions = re.findall(r"^\s*runs-on:\s*\$\{\{(.*?)\}\}\s*$", text, re.M)
        self.assertEqual(len(expressions), 2)
        block = text.split("        include:\n", 1)[1].split("    runs-on:", 1)[0]
        rows = []
        for line in block.splitlines():
            found = re.match(r"\s*(- )?(\w+): ([^#]+)", line)
            if not found:
                continue
            new, key, value = found.groups()
            if new:
                rows.append({})
            if rows:
                value = value.strip()
                rows[-1][key] = {"true": True, "false": False}.get(value, value)
        self.assertEqual({row["name"] for row in rows}, {"linux", "linux-musl", "windows", "windows-arm64", "macos", "macos-x64"})
        script = """const fs = require('fs');
const state = JSON.parse(fs.readFileSync(0, 'utf8'));
// Actions returns an empty string for absent context properties, not undefined.
const context = values => new Proxy(values, {get: (values, key) => values[key] ?? ''});
const matrix = context(state.matrix), vars = context(state.vars), inputs = state.inputs, github = context(state.github);
const expression = state.expression, fromJSON = JSON.parse, toJSON = JSON.stringify;
console.log(JSON.stringify(eval(expression)));
"""
        def evaluate(expression, matrix, trusted, repository="arismoko/horizon"):
            result = subprocess.run(["node", "-e", script], input=json.dumps({"expression": expression, "matrix": matrix, "inputs": {"trusted": trusted}, "vars": variables, "github": {"repository": repository}}), text=True, capture_output=True, check=True)
            return json.loads(result.stdout)
        default_mac = ["self-hosted", "macOS", "horizon-nightly"]
        bookkeeping = re.search(r"^\s*runs-on:\s*\$\{\{(.*?)\}\}\s*$", NIGHTLY.read_text(), re.M).group(1)
        for release_runner, disabled, trusted in itertools.product([False, True], [False, True], [False, True]):
            variables = {"SELF_HOSTED_CI": "off" if disabled else "all"}
            if release_runner:
                variables["MAC_RUNNER"] = "release-mac"
            # An empty runs-on is rejected by Actions; never choose a shared runner instead.
            dedicated = "release-mac" if release_runner else ""
            expected_exporter = dedicated if trusted else ("ubuntu-latest" if disabled else default_mac)
            with self.subTest(release_runner=release_runner, disabled=disabled, trusted=trusted):
                self.assertEqual(evaluate(guard_if, {}, trusted), trusted and not release_runner)
                self.assertEqual(evaluate(expressions[0], {}, trusted), expected_exporter)
                expected_bookkeeping = "ubuntu-latest" if disabled else ["self-hosted", "linux", "horizon-light"]
                self.assertEqual(evaluate(bookkeeping, {}, trusted), expected_bookkeeping)
                for row in rows:
                    if row["name"] in {"linux", "linux-musl"}:
                        self.assertEqual(row["os"], "ubuntu-22.04", "shipped Linux binaries keep the documented libc baseline")
                    if row.get("mac"):
                        expected = dedicated if trusted else ("macos-15" if disabled else default_mac)
                    else:
                        expected = row["os"]
                    with self.subTest(entry=row["name"]):
                        self.assertEqual(evaluate(expressions[1], row, trusted), expected)

        timeout = re.search(r"^\s*timeout-minutes:\s*\$\{\{(.*?)\}\}", workflow_jobs["build"], re.M).group(1)
        for trusted, row in itertools.product([False, True], rows):
            with self.subTest(trusted=trusted, entry=row["name"]):
                self.assertEqual(evaluate(timeout, row, trusted), 60 if row.get("mac") else 40)

    def test_every_shipped_glibc_program_is_checked_by_workflow_owned_tools(self):
        text = GAME.read_text()
        commands = re.findall(r"check_glibc.py ([^\n]+)", text.replace("\\\n", " "))
        checked = [{Path(argument).name for argument in shlex.split(command)} for command in commands]
        self.assertEqual(checked, [
            {"horizon-client", "horizon-ctl", "horizon-prepare", "horizon-server"},
            {"horizon-engine", "horizon-server", "horizon-prepare"},
        ])
        tools = [step for step in text.split("\n      - ") if "path: .workflow-tools" in step]
        self.assertTrue(tools)
        for checkout in tools:
            self.assertIn("ref: ${{ github.workflow_sha }}", checkout,
                          "older build candidates may lack the new workflow's validator/smoke helpers")

    def test_hosted_runners_never_use_machine_specific_tool_setup(self):
        steps = [block for block in GAME.read_text().split("\n      - ") if "bash tools/ci/runner-env.sh" in block]
        self.assertEqual(len(steps), 2)
        for step in steps:
            condition = re.search(r"^\s*if:\s*(.*)$", step, re.M).group(1)
            for environment in ["github-hosted", "self-hosted"]:
                expression = condition.replace("runner.environment", repr(environment))
                self.assertEqual(eval(expression, {"__builtins__": {}}), environment == "self-hosted")

    @unittest.skipUnless(shutil.which("bash"), "The runner guard executes in Bash")
    def test_missing_release_runner_guard_fails_clearly(self):
        guard = dict(jobs(GAME.read_text()))["runner-configuration"]
        shell = textwrap.dedent(guard.split("        run: |\n", 1)[1])
        result = subprocess.run(["bash", "-c", shell], text=True, capture_output=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("::error::", result.stdout)
        self.assertIn("MAC_RUNNER", result.stdout)

    @unittest.skipUnless(os.name == "posix", "The smoke command runs on Linux and macOS")
    def test_smoke_runs_from_relative_or_absolute_binary_directory(self):
        command = next(line.strip() for line in SMOKE.read_text().splitlines() if line.strip().startswith("client="))
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            binary_dir = root / "target tree/release"
            binary_dir.mkdir(parents=True)
            binary = binary_dir / "horizon-client"
            binary.write_text("#!/bin/sh\nprintf 'smoke ran'\n")
            binary.chmod(0o755)
            for directory in [str(binary_dir), str(binary_dir.relative_to(root))]:
                with self.subTest(directory=directory):
                    result = subprocess.run(["bash", "-c", command + '\n"$client"'], cwd=root, env={**os.environ, "BIN": directory}, text=True, capture_output=True)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(result.stdout, "smoke ran")

    @unittest.skipUnless(shutil.which("Xvfb"), "Xvfb is required for the Linux smoke display")
    def test_linux_smoke_gets_a_ready_display(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            binary = root / "horizon-client"
            # A controlled game fixture isolates the real smoke orchestration and Xvfb.
            binary.write_text(textwrap.dedent('''\
                #!/usr/bin/env python3
                import json, os, pathlib, sys
                args = sys.argv[1:]
                report = {"frames": int(args[args.index("--world-frames") + 1]),
                          "chunks": 1, "meshed_sections": 1, "player": [0, 64, 0],
                          "display": os.environ.get("DISPLAY"), "wayland": os.environ.get("WAYLAND_DISPLAY")}
                pathlib.Path(args[args.index("--smoke-report") + 1]).write_text(json.dumps(report))
            '''))
            binary.chmod(0o755)
            result = subprocess.run(["bash", str(SMOKE)], cwd=ROOT, env={**os.environ, "BIN": folder, "RUNNER_TEMP": folder, "RUNNER_OS": "Linux", "DISPLAY": ":invalid", "WAYLAND_DISPLAY": "invalid"}, text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            report = json.loads((root / "smoke-test.json").read_text())
            self.assertEqual(report["frames"], 1500)
            self.assertRegex(report["display"], r"^:[0-9]+$")
            self.assertIsNone(report["wayland"])


if __name__ == "__main__":
    unittest.main()
