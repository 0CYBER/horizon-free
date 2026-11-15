"""Execute the launcher publishing verifier's machine-consumed argument contract."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class LauncherVerification(unittest.TestCase):
    @unittest.skipUnless(os.name == "posix" and shutil.which("bash"), "Linux Bash publishing step")
    def test_verification_uses_the_publishing_repository(self):
        workflow = (ROOT / ".github/workflows/launcher.yml").read_text()
        step = workflow.split("      - name: The signature, checked as launchers check it\n", 1)[1]
        shell = step.split("        run: |\n", 1)[1]
        # Read only this run block, not following steps or workflow metadata.
        lines = []
        for line in shell.splitlines():
            if line and not line.startswith("          "):
                break
            lines.append(line[10:])
        shell = "\n".join(lines)
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            # Capture the real Bash invocation at the Cargo boundary; no build/signing/network.
            cargo = root / "cargo"
            cargo.write_text("#!/usr/bin/env python3\nimport json, sys\nprint(json.dumps(sys.argv[1:]))\n")
            cargo.chmod(0o755)
            (root / "dist").mkdir()
            (root / "dist/ephemeral.pub").write_text("dry-run-public-key\n")
            for repository in ("horizon-rust/horizon", "another/fork"):
                for ephemeral in ("", "yes"):
                    with self.subTest(repository=repository, ephemeral=ephemeral):
                        result = subprocess.run(
                            ["bash", "-e", "-c", shell], cwd=root,
                            env={**os.environ, "PATH": str(root) + os.pathsep + os.environ["PATH"],
                                 "GITHUB_REPOSITORY": repository, "TAG": "launcher@0.2.7",
                                 "EPHEMERAL": ephemeral},
                            capture_output=True, text=True, check=True,
                        )
                        expected = ["run", "--locked", "--quiet", "-p", "horizon-release-sign",
                                    "--", "verify", "--repository", repository,
                                    "--tag", "launcher@0.2.7", "--publishing"]
                        if ephemeral:
                            expected += ["--key", "dry-run-public-key"]
                        self.assertEqual(json.loads(result.stdout), expected + ["dist"])


if __name__ == "__main__":
    unittest.main()
