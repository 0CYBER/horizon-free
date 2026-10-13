"""Compile the real frame hand-selection expression without opening a window."""
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class HandFramePolicyTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "rustc is needed for the frame-policy probe")
    def test_inventory_screen_does_not_suppress_first_person_geometry(self):
        source = (ROOT / "crates/horizon-client/src/game.rs").read_text()
        match = re.search(r"let hand = (if .*?);\s*let screen =", source, re.S)
        self.assertIsNotNone(match, "frame hand-selection seam changed")
        selection = match.group(1)
        # Only geometry generation is stubbed; execute the actual frame guard.
        program = """
#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen { Playing, Creative, Inventory }
struct Game { screen: Screen, scoping: bool }
impl Game {
    fn scoping(&self) -> bool { self.scoping }
    fn hand_geometry(&mut self, _: f64) -> Vec<u8> { vec![1] }
    fn hand_len(&mut self, title: bool) -> usize {
        let alpha = 0.0;
        let hand = SELECTION;
        hand.len()
    }
}
#[test]
fn creative_inventory_and_return_keep_the_hand() {
    let mut game = Game { screen: Screen::Playing, scoping: false };
    for screen in [Screen::Playing, Screen::Creative, Screen::Inventory, Screen::Playing] {
        game.screen = screen;
        assert_eq!(game.hand_len(false), 1);
    }
}
#[test]
fn title_and_spyglass_still_hide_the_hand() {
    let mut game = Game { screen: Screen::Inventory, scoping: false };
    assert_eq!(game.hand_len(true), 0);
    game.scoping = true;
    assert_eq!(game.hand_len(false), 0);
}
""".replace("SELECTION", selection)
        with tempfile.TemporaryDirectory(prefix="horizon-hand-policy-") as directory:
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
