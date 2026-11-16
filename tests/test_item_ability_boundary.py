"""Execute the engine's item-scoped ability boundary with counted conversion."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from rust_items import rust_fn

class AbilityBoundary(unittest.TestCase):
    def test_unrelated_item_skips_conversion(self):
        compiler = shutil.which("rustc")
        if compiler is None:
            self.skipTest("requires Rust to execute the production boundary fixture")
        method = rust_fn('stack_can_perform')
        fixture = r'''
use std::cell::Cell;
struct ItemStack { item: String }
mod mod_api {
 pub enum HookId { CanPerformAction }
 pub mod __private {
 pub fn item_listens(_: super::HookId, item: &str)->bool { item == "subscribed" }
 }
}
struct Game { conversions: Cell<u32> }
impl Game {
 fn native_can_perform_action(&self,stack:&ItemStack,_:&str,out:&mut bool) { self.conversions.set(self.conversions.get()+1); if stack.item == "subscribed" { *out=true; } }
 METHOD
}
#[test] fn conversion_is_scoped() {
 let g=Game { conversions:Cell::new(0) };
 let unrelated=ItemStack{item:"unrelated".into()};
 assert!(!g.stack_can_perform(&unrelated,"test:ability",false));
 assert!(g.stack_can_perform(&unrelated,"test:ability",true));
 assert_eq!(g.conversions.get(),0);
 assert!(g.stack_can_perform(&ItemStack{item:"subscribed".into()},"test:ability",false));
 assert_eq!(g.conversions.get(),1);
}
'''.replace('METHOD', method)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            (path / 'test.rs').write_text(fixture)
            subprocess.run([compiler, '--edition=2024', '--test', str(path / 'test.rs'), '-o', str(path / 'test')], check=True)
            subprocess.run([str(path / 'test')], check=True)
