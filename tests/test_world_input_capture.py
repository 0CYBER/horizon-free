"""Exercise the client's capture methods without opening or focusing an OS window.

The fixture substitutes only winit and mouse state. Method bodies come from the
client, so the assertions observe grab attempts, visibility and stored capture
state. Native Windows delivery remains a manual check. Vanilla 26.3 reference:
LevelLoadingScreen.tick/onClose, Gui.setScreen, MouseHandler.grabMouse/releaseMouse.
"""
from pathlib import Path
import os
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]


def method(source, name):
    start = source.index(f"    fn {name}(")
    opening = source.index("{", start)
    depth = 1
    end = opening + 1
    while depth:
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    return source[start:end]


class WorldInputCaptureTests(unittest.TestCase):
    def test_loading_completion_wiring(self):
        game = (ROOT / "crates/horizon-client/src/game.rs").read_text(encoding="utf-8")
        remote = (ROOT / "crates/horizon-client/src/game/remote.rs").read_text(encoding="utf-8")
        spawn = method(game, "spawn_player")
        self.assertLess(spawn.index("self.player = Some(player)"),
                        spawn.index("self.capture_loaded_world()"))
        tick = remote[remote.index("    pub(super) fn tick_remote("):]
        ready = tick[tick.index("if self.remote.as_ref().is_some_and(|r| !r.loaded_sent)"):]
        self.assertLess(ready.index("r.loaded_sent = true"),
                        ready.index("self.capture_loaded_world()"))
        self.assertLess(ready.index("self.capture_loaded_world()"),
                        ready.index("let stick = self.pad_move()"))

    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_capture_lifecycle(self):
        source = (ROOT / "crates/horizon-client/src/game.rs").read_text(encoding="utf-8")
        fixture = r'''
use std::cell::Cell;
#[derive(Clone, Copy, PartialEq)]
enum Screen { Playing, Paused, Inventory }
enum CursorGrabMode { Locked, Confined, None }
#[derive(Default)]
struct Window { grabs: Cell<u32>, visible: Cell<bool>, fail: bool }
impl Window {
    fn set_cursor_grab(&self, mode: CursorGrabMode) -> Result<(), ()> {
        if !matches!(mode, CursorGrabMode::None) {
            self.grabs.set(self.grabs.get() + 1);
        }
        if self.fail { Err(()) } else { Ok(()) }
    }
    fn set_cursor_visible(&self, visible: bool) { self.visible.set(visible); }
}
#[derive(Default)]
struct Mouse { ignore_first: bool }
impl Mouse { fn set_ignore_first_move(&mut self) { self.ignore_first = true; } }
struct Remote { chat_input: Option<()> }
struct Game {
    focused: bool, agent_pointer: bool, window: Option<Window>, mouse: Mouse,
    captured: bool, title: Option<()>, screen: Screen, remote: Option<Remote>,
    mining: bool,
}
impl Default for Game {
    fn default() -> Self {
        Self { focused: true, agent_pointer: false, window: Some(Window::default()),
            mouse: Mouse::default(), captured: false, title: None,
            screen: Screen::Playing, remote: None, mining: false }
    }
}
impl Game {
    fn stop_mining(&mut self) { self.mining = false; }
METHODS
}
fn main() {
    // New and existing worlds, returning to title, and reopening a world.
    for remote in [false, true] {
        let mut game = Game::default();
        if remote { game.remote = Some(Remote { chat_input: None }); }
        for _ in 0..3 {
            game.title = Some(());
            game.capture(false);
            game.capture_loaded_world();
            assert!(!game.captured);
            game.title = None;
            game.capture_loaded_world();
            assert!(game.captured);
            assert!(game.mouse.ignore_first);
            assert!(!game.window.as_ref().unwrap().visible.get());
            for screen in [Screen::Paused, Screen::Inventory] {
                game.screen = screen;
                game.mining = true;
                game.capture(false);
                assert!(!game.mining);
                game.capture_loaded_world();
                assert!(!game.captured);
                assert!(game.window.as_ref().unwrap().visible.get());
                game.screen = Screen::Playing;
                game.capture(true);
                assert!(game.captured);
            }
        }
        // Alt+Tab during loading: no native grab or agent-pointer reset.
        game.focused = false;
        game.mining = true;
        game.capture(false);
        assert!(!game.mining);
        assert!(!game.captured);
        assert!(game.window.as_ref().unwrap().visible.get());
        game.agent_pointer = true;
        let grabs = game.window.as_ref().unwrap().grabs.get();
        game.capture_loaded_world();
        game.capture(true);
        assert!(!game.captured);
        assert!(game.agent_pointer);
        assert_eq!(game.window.as_ref().unwrap().grabs.get(), grabs);
        game.focused = true;
        assert!(!game.captured); // Focus alone does not resume gameplay.
        game.capture(true); // Click or resume action.
        assert!(game.captured);
    }
    let mut game = Game::default();
    game.remote = Some(Remote { chat_input: Some(()) });
    game.capture_loaded_world();
    assert!(!game.captured);
    assert_eq!(game.window.as_ref().unwrap().grabs.get(), 0);
    game.remote = None;
    game.window.as_mut().unwrap().fail = true;
    game.capture_loaded_world();
    assert!(!game.captured);
    assert!(game.window.as_ref().unwrap().visible.get());
}
'''
        fixture = fixture.replace("METHODS", method(source, "capture") + "\n" +
                                  method(source, "capture_loaded_world"))
        with tempfile.TemporaryDirectory(prefix="horizon-input-") as folder:
            folder = Path(folder)
            rust = folder / "capture.rs"
            binary = folder / ("capture.exe" if os.name == "nt" else "capture")
            rust.write_text(fixture, encoding="utf-8")
            subprocess.run(["rustc", "--edition=2024", str(rust), "-o", str(binary)],
                           check=True, capture_output=True, text=True, timeout=60)
            subprocess.run([str(binary)], check=True, capture_output=True, text=True, timeout=10)


if __name__ == "__main__":
    unittest.main()
