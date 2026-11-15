"""The downloaded runtime must reach the directory each real packer reads."""
import re
import unittest

from tests.test_ci_concurrency import ROOT, expression_value, yaml


def resolve(expression, matrix, env):
    # Reuse the workflow expression evaluator; this workflow also uses format.
    expression = expression.removeprefix("${{").removesuffix("}}")
    expression = re.sub(r"format\('([^']*)', matrix\.target\)",
                        lambda m: repr(m[1].format(matrix.get("target", ""))), expression)
    expression = expression.replace("matrix.", "github.matrix.").replace("env.", "github.env.")
    return expression_value(expression, {"matrix": matrix, "env": env})


class WasmStdStagingTests(unittest.TestCase):
    def test_build_prerequisites_cover_wasm_and_hosted_matrix_legs(self):
        workflow = yaml.safe_load((ROOT / ".github/workflows/game-build.yml").read_text())
        exporters = workflow["jobs"]["exporters"]["steps"]
        toolchain = next(step for step in exporters if step.get("uses", "").startswith("dtolnay/rust-toolchain@"))
        components = {component.strip() for component in toolchain["with"]["components"].split(",")}
        self.assertTrue({"rust-src", "rustfmt"}.issubset(components))
        build = workflow["jobs"]["build"]
        tools = next(step for step in build["steps"] if step.get("name") == "Workflow-owned validation tools")
        self.assertIn("tools/ci/rust-version", tools["with"]["sparse-checkout"].splitlines())
        for matrix in build["strategy"]["matrix"]["include"]:
            with self.subTest(platform=matrix["name"]):
                runner = {"os": "macOS" if matrix.get("mac") else "Windows" if matrix["name"].startswith("windows") else "Linux",
                          "environment": "github-hosted"}
                condition = tools.get("if", "true").replace("runner.", "github.runner.")
                condition = condition.replace("!matrix.musl", "matrix.musl == false")
                self.assertTrue(expression_value(condition, {"runner": runner}, matrix={"musl": False, **matrix}))

    def test_every_matrix_packer_reads_the_download_directory(self):
        job = yaml.safe_load((ROOT / ".github/workflows/game-build.yml").read_text())["jobs"]["build"]
        download, = [step for step in job["steps"]
                     if step.get("uses", "").startswith("actions/download-artifact@")
                     and step.get("with", {}).get("name") == "wasm-std"]
        alpine = (ROOT / "tools/ci/build-musl.sh").read_text()
        target = re.search(r"^target=(\S+)$", alpine, re.M)[1]
        musl_bin = re.search(r'^bin="([^"]+)"$', alpine, re.M)[1].replace("$target", target)
        self.assertRegex(alpine, r'package\s+\\\s*"\$bin/horizon-client"')
        for matrix in job["strategy"]["matrix"]["include"]:
            with self.subTest(target=matrix["name"]):
                env = {"BIN": resolve(job["env"]["BIN"], matrix, {})}
                destination = resolve(download["with"]["path"], matrix, env)
                if matrix.get("musl"):
                    self.assertEqual(destination, musl_bin)
                else:
                    self.assertEqual(destination, env["BIN"])
                    package = next(step for step in job["steps"] if step.get("name") == "Package")
                    self.assertIn('package "$BIN/horizon-client${{ matrix.exe }}"', package["run"])
                if matrix.get("server_asset"):
                    server = next(step for step in job["steps"] if step.get("if") == "matrix.server_asset")
                    self.assertIn('package-server "$BIN/horizon-client"', server["run"])
                    self.assertEqual(destination, env["BIN"])
