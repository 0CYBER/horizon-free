"""The dragon-contacts harness's 240 s limit covers its Rust cases' run, not the test binary's compile."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

from tools import test_dragon_contacts_parity as harness


class RustTimeLimitTests(unittest.TestCase):
    def test_the_test_binary_is_built_first_without_the_cases_limit(self):
        cases = [{"case": f"case{index}", "health": 20.0, "revenge": None, "velocity": [0.0, 0.0, 0.0]}
                 for index in range(7)]
        calls = []

        def run(command, **kwargs):
            calls.append((command, kwargs.get("timeout")))
            if "dev.horizonlab.parity.VanillaCommandExecOracle" in command:
                Path(command[-2]).write_text(json.dumps(cases))
            elif command[:2] == ["cargo", "test"] and "--no-run" not in command:
                Path(kwargs["env"]["HORIZON_DRAGON_CONTACT_OUTPUT"]).write_text(json.dumps(cases))
            return subprocess.CompletedProcess(command, 0)

        with tempfile.TemporaryDirectory() as directory, \
                mock.patch.object(harness.lab, "LOCAL", Path(directory)), \
                mock.patch.object(harness.subprocess, "run", side_effect=run), \
                mock.patch.object(harness, "game_environment", side_effect=lambda base, root: dict(base)), \
                mock.patch.object(sys, "argv", ["harness", "--classpath", "server.jar"]), \
                mock.patch("builtins.print"):
            harness.main()
        cargo = [(command, timeout) for command, timeout in calls if command[:2] == ["cargo", "test"]]
        self.assertEqual(cargo, [
            (["cargo", "test", "--locked", "-p", "horizon-client-lib", "--lib", "--no-run"], None),
            (["cargo", "test", "--locked", "-p", "horizon-client-lib", "--lib", "mobs::dragon_contacts::", "--",
              "--include-ignored"], 240)])
        # Everything else keeps its 240 s.
        self.assertEqual({timeout for command, timeout in calls if command[:2] != ["cargo", "test"]}, {240})


if __name__ == "__main__":
    unittest.main()
