import importlib.util
from pathlib import Path
import sys
import unittest

TOOLS = Path(__file__).resolve().parents[1] / "tools"


def _load_harness():
    spec = importlib.util.spec_from_file_location(
        "_remote_fireball_parity_harness", TOOLS / "test_remote_fireball_parity.py"
    )
    module = importlib.util.module_from_spec(spec)
    sys.path.append(str(TOOLS))
    try:
        spec.loader.exec_module(module)
    finally:
        sys.path.remove(str(TOOLS))
    return module


harness = _load_harness()


class FakeGame:
    def __init__(self, fail_script=False):
        self.commands = []
        self.scripts = []
        self.fail_script = fail_script
        self._kinds = iter(["Egg", "EnderPearl", "WindCharge"])

    def __call__(self, action, **kwargs):
        if action in ("state", "status"):
            return {"player": {"position": [0.0, 64.0, 0.0]}}
        if action == "command":
            self.commands.append(kwargs["command"])
        return {}

    def raw(self, *args):
        self.commands.append(" ".join(args))
        return {}

    def wait(self, ticks):
        self.commands.append(f"wait {ticks}")

    def script(self, lines, every=False):
        self.scripts.append((list(lines), every))
        if self.fail_script:
            raise RuntimeError("script failed")
        kind = next(self._kinds)
        replies = [{}]
        for i in range(6):
            replies.append({"projectiles": [{"kind": kind, "position": [1.0 + i, 65.0, 0.5]}]})
            replies.append({"ticks": i + 1})
        return replies


class RemoteFireballThrownTests(unittest.TestCase):
    def test_check_thrown_samples_at_quarter_tick_rate_and_restores_20(self):
        host = FakeGame()
        guest = FakeGame()
        results = harness.check_thrown(host, guest, Path("unused"))

        self.assertEqual(set(results), {"minecraft:egg", "minecraft:ender_pearl", "minecraft:wind_charge"})
        tick_commands = [c for c in host.commands if c.startswith("tick rate ")]
        self.assertEqual(tick_commands, ["tick rate 5", "tick rate 20"] * 3)
        self.assertEqual(len(guest.scripts), 3)
        for lines, every in guest.scripts:
            self.assertTrue(every)
            self.assertEqual(lines, ["use-item", *(["status", "wait 2"] * 6)])

    def test_check_thrown_restores_tick_rate_on_failure(self):
        host = FakeGame()
        guest = FakeGame(fail_script=True)
        with self.assertRaises(RuntimeError):
            harness.check_thrown(host, guest, Path("unused"))
        tick_commands = [c for c in host.commands if c.startswith("tick rate ")]
        self.assertEqual(tick_commands, ["tick rate 5", "tick rate 20"])


if __name__ == "__main__":
    unittest.main()
