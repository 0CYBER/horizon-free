"""Exercise the real creative-tab selection branch without a desktop window."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class CreativeInventoryTransitionTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "rustc is needed for the tab-selection probe")
    def test_inventory_tab_stays_in_the_creative_screen(self):
        source = (ROOT / "crates/horizon-client/src/game.rs").read_text()
        click = source.split("fn click_creative(&mut self, right: bool)", 1)[1]
        branch = click.split("if cx >= ox + tx", 1)[1]
        branch = branch.split("{", 1)[1].split("\n            }\n", 1)[0]
        program = """
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Screen { Creative, Inventory }
struct Tab { inventory: bool }
struct Creative { selected: usize, tabs: [Tab; 2] }
impl Creative { fn select(&mut self, i: usize) { self.selected = i; } }
struct Game { screen: Screen, creative_screen: Creative, inv_drag: Option<u8>, last_inv_click: Option<u8> }
impl Game {
    fn click_tab(&mut self, i: usize) {
        let t = &self.creative_screen.tabs[i];
        SELECTION
    }
}
#[test]
fn inventory_and_back_preserve_the_creative_screen() {
    let mut game = Game { screen: Screen::Creative,
        creative_screen: Creative { selected: 0, tabs: [Tab { inventory: false }, Tab { inventory: true }] },
        inv_drag: Some(1), last_inv_click: Some(2) };
    game.click_tab(1);
    assert_eq!(game.screen, Screen::Creative);
    assert_eq!(game.creative_screen.selected, 1);
    assert!(game.inv_drag.is_none() && game.last_inv_click.is_none());
    game.click_tab(0);
    assert_eq!(game.screen, Screen::Creative);
    assert_eq!(game.creative_screen.selected, 0);
}
""".replace("SELECTION", branch)
        with tempfile.TemporaryDirectory(prefix="horizon-creative-tab-") as directory:
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
