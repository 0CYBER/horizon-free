"""Exercise the canonical screen wheel gate from the extracted window handler."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


def block(source, start_text):
    """Extract a Rust block, including its signature or condition."""
    start = source.index(start_text)
    opening = source.index("{", start)
    depth, end = 1, opening + 1
    while depth:
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    return source[start:end]


class ScreenMouseScrolledTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_pre_cancellation_and_post_scheduling(self):
        source = (ROOT / "crates/horizon-client/src/game.rs").read_text(encoding="utf-8")
        dispatch = block(source, "if !self.pad_synthetic_input()\n                        && let Some(screen)")
        hooks = (ROOT / "crates/horizon-client/src/game/native_hooks/base.rs").read_text(encoding="utf-8")
        pre = block(hooks, "    pub(in crate::game) fn native_screen_mouse_scrolled_pre(")
        pre = pre.replace("pub(in crate::game) ", "")
        fixture = r"""
use std::cell::RefCell;
fn glfw_modifiers(bits: u32) -> u32 { bits }
struct ScreenView<'a> { id: &'a str, mouse: [f32; 2] }
mod native_hooks {
    #[derive(Debug, PartialEq)]
    pub enum ScreenMousePost { Scroll([f64; 2]) }
}
struct Game { modifiers: u32, synthetic: bool, open: bool, cancel: bool,
    calls: RefCell<Vec<&'static str>>, scrolled: usize }
impl Game {
    fn pad_synthetic_input(&self) -> bool { self.synthetic }
    fn screen_view_now(&self) -> Option<ScreenView<'_>> {
        self.open.then_some(ScreenView { id: "inventory", mouse: [7.0, 9.0] })
    }
    PRE
    fn fire_screen_mouse_scrolled_pre(&self, screen: &ScreenView<'_>, scroll: [f64; 2], modifiers: u32, canceled: &mut bool) {
        assert_eq!((screen.id, screen.mouse, scroll, modifiers),
            ("inventory", [7.0, 9.0], [3.0, -2.0], 5));
        assert!(!*canceled);
        self.calls.borrow_mut().push("pre");
        *canceled = self.cancel;
    }
    fn wheel(&mut self) -> Result<(), ()> {
        let (raw_x, raw_y) = (1.5, -1.0);
        let scaled = |v: f64| v * 2.0;
        let mut mouse_post = None;
        DISPATCH
        // Stand in for the screen's scroll handler. The extracted gate must
        // stop both screen handling and Post scheduling when Pre cancels.
        self.calls.borrow_mut().push("screen");
        self.scrolled += 1;
        assert_eq!(mouse_post, if self.open && !self.synthetic {
            Some(native_hooks::ScreenMousePost::Scroll([3.0, -2.0]))
        } else { None });
        Ok(())
    }
}
#[test]
fn cancellation_and_input_gates() {
    for cancel in [false, true] {
        for synthetic in [false, true] {
            for open in [false, true] {
                let mut g = Game { modifiers: 5, synthetic, open, cancel,
                    calls: RefCell::new(vec![]), scrolled: 0 };
                g.wheel().unwrap();
                let expected = match (open && !synthetic, cancel) {
                    (true, true) => vec!["pre"],
                    (true, false) => vec!["pre", "screen"],
                    (false, _) => vec!["screen"],
                };
                assert_eq!(*g.calls.borrow(), expected, "{cancel} {synthetic} {open}");
                assert_eq!(g.scrolled, usize::from(!(open && !synthetic && cancel)));
            }
        }
    }
}
""".replace("DISPATCH", dispatch).replace("PRE", pre)
        with tempfile.TemporaryDirectory() as folder:
            src, exe = Path(folder) / "scroll.rs", Path(folder) / "scroll.exe"
            src.write_text(fixture, encoding="utf-8")
            subprocess.run(["rustc", "--edition=2024", "--test", str(src), "-o", str(exe)], check=True)
            subprocess.run([str(exe)], check=True)


if __name__ == "__main__":
    unittest.main()
