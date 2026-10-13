"""Drop modifiers must come from physical Ctrl, not the sprint key mapping."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class DropModifierPolicyTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "rustc is needed for the drop-policy probe")
    def test_playing_and_inventory_drop_ignore_latched_sprint(self):
        source = (ROOT / "crates/horizon-client/src/game.rs").read_text()
        inventory = source.split("fn inventory_hotkey(", 1)[1]
        playing = source.split("KeyCode::KeyQ if self.screen == Screen::Playing =>", 1)[1]
        expressions = [section.split("let all = ", 1)[1].split(";", 1)[0]
                       for section in (inventory, playing)]
        program = """
use std::collections::HashSet;
#[derive(PartialEq, Eq, Hash)] enum KeyCode { ControlLeft, ControlRight }
struct Game { keys: HashSet<KeyCode>, physical_keys: HashSet<KeyCode> }
impl Game {
    fn inventory_drop_all(&self) -> bool { INVENTORY }
    fn playing_drop_all(&self) -> bool { PLAYING }
}
#[test]
fn sprint_and_control_are_independent() {
    for sprint in [false, true] {
        for held in 0..4 {
            let ctrl = held != 0;
            let mut game = Game { keys: HashSet::new(), physical_keys: HashSet::new() };
            if held & 1 != 0 { game.physical_keys.insert(KeyCode::ControlLeft); }
            if held & 2 != 0 { game.physical_keys.insert(KeyCode::ControlRight); }
            if sprint { game.keys.insert(KeyCode::ControlLeft); }
            assert_eq!(game.inventory_drop_all(), ctrl, "inventory: sprint={sprint}, ctrl={ctrl}");
            assert_eq!(game.playing_drop_all(), ctrl, "playing: sprint={sprint}, ctrl={ctrl}");
        }
    }
}
""".replace("INVENTORY", expressions[0]).replace("PLAYING", expressions[1])
        with tempfile.TemporaryDirectory(prefix="horizon-drop-policy-") as directory:
            path = Path(directory)
            (path / "probe.rs").write_text(program)
            compiled = subprocess.run(
                ["rustc", "--edition=2024", "--test", str(path / "probe.rs"), "-o", str(path / "probe")],
                capture_output=True, text=True, check=False,
            )
            self.assertEqual(compiled.returncode, 0, compiled.stderr)
            result = subprocess.run([str(path / "probe")], capture_output=True, text=True, check=False)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
