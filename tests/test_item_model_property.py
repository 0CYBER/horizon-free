"""Exercise the real ItemEnv bridge without a game window or prepared assets."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from test_world_input_capture import method

ROOT = Path(__file__).resolve().parents[1]


class ItemModelPropertyTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_unlistened_fast_path_and_owned_stack_dispatch(self):
        source = (next(ROOT.glob('crates/horizon-client/src/game/*/item_draw.rs'))).read_text(encoding="utf-8")
        actual = method(source, "item_model_property")
        fixture = r'''
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
static LOADED: AtomicBool = AtomicBool::new(false);
static SNAPSHOTS: AtomicUsize = AtomicUsize::new(0);
mod mod_api {
    pub enum HookId { ItemModelProperty }
    pub mod __private {
        pub fn listened(hook: super::HookId) -> bool {
            matches!(hook, super::HookId::ItemModelProperty) && crate::LOADED.load(crate::Ordering::Relaxed)
        }
    }
}
struct StackView { count: i32, bundle_selected: i32 }
struct ItemStack { count: u32 }
impl ItemStack { fn count(&self) -> u32 { self.count } }
impl StackView {
    fn try_into_api(&self) -> Result<ItemStack, ()> {
        SNAPSHOTS.fetch_add(1, Ordering::Relaxed);
        let count = u32::try_from(self.count).map_err(|_| ())?;
        if count == 0 { return Err(()); }
        Ok(ItemStack { count })
    }
}
struct Game { calls: Cell<usize> }
impl Game {
    fn native_item_model_property(&self, property: &str, stack: &ItemStack, bundle_selected: i32, value: &mut bool) {
        assert_eq!(stack.count(), 2);
        assert_eq!(bundle_selected, 3);
        self.calls.set(self.calls.get() + 1);
        if property == "example:charged" { *value = stack.count() > 1; }
    }
}
struct Env<'a> { game: &'a Game }
impl Env<'_> { METHOD }
#[test]
fn bridge() {
    let stack = StackView { count: 2, bundle_selected: 3 };
    let game = Game { calls: Cell::new(0) };
    let env = Env { game: &game };
    assert!(!env.item_model_property("example:charged", &stack));
    assert_eq!(game.calls.get(), 0);
    assert_eq!(SNAPSHOTS.load(Ordering::Relaxed), 0);
    LOADED.store(true, Ordering::Relaxed);
    assert!(env.item_model_property("example:charged", &stack));
    assert!(!env.item_model_property("other:property", &stack));
    assert_eq!(game.calls.get(), 2);
    assert_eq!(SNAPSHOTS.load(Ordering::Relaxed), 2);
    let empty = StackView { count: 0, bundle_selected: -1 };
    assert!(!env.item_model_property("example:charged", &empty));
    assert_eq!(game.calls.get(), 2);
}
'''.replace("METHOD", actual)
        (ROOT / "target").mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=ROOT / "target", prefix="item-model-bridge-") as folder:
            src, exe = Path(folder) / "property.rs", Path(folder) / "property.exe"
            src.write_text(fixture, encoding="utf-8")
            subprocess.run(["rustc", "--edition=2024", "--test", str(src), "-o", str(exe)], check=True)
            subprocess.run([str(exe)], check=True)
