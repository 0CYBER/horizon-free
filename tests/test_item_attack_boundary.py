"""Run the production item and packet dispatch with counted boundary doubles."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]

class ItemAttackBoundary(unittest.TestCase):
    def test_scoped_conversion_and_packet_veto(self):
        compiler = shutil.which("rustc")
        if compiler is None:
            self.skipTest("requires Rust to execute the production boundary fixture")
        source = (ROOT / 'crates/horizon-client/src/game/native_hooks/item_attack.rs').read_text()
        methods = source[source.index('    pub(in crate::game) fn native_left_click_entity_event'):]
        methods = methods.replace('pub(in crate::game)', 'pub')
        remote = (ROOT / 'crates/horizon-client/src/game/network/swing_packet.rs').read_text()
        start = remote.index('        if ')
        packet = remote[start:remote.rfind('    }')].replace('horizon_protocol::packets::play::Serverbound', 'S').replace('horizon_protocol::packets::Empty', 'Empty')
        fixture = r'''
use std::cell::Cell;
thread_local! { static CLONES: Cell<u32> = const { Cell::new(0) }; }
struct Stack { item: String }
impl Clone for Stack { fn clone(&self) -> Self { CLONES.set(CLONES.get()+1); Self { item:self.item.clone() } } }
#[derive(Clone,Copy)] enum Hand { Main, Off }
struct Player { entity_id: i32 }
struct SwingAnimation; impl SwingAnimation { const DEFAULT: Self = Self; }
struct Empty; enum S { Punch(Empty) }
mod mod_api {
 pub enum HookId { OnLeftClickEntity, OnEntitySwing }
 pub mod __private {
 pub fn has_part_hook(_: super::HookId) -> bool { true }
 pub fn item_listens(_: super::HookId, item: &str) -> bool { item == "subscribed" }
 }
}
struct Game { hand: Hand, offhand: Option<Stack>, player: Option<Player>, stack: Stack, calls: u32, packets: u32, canceled: bool }
impl Game {
 fn held(&self)->Option<&Stack> { Some(&self.stack) }
 fn native_acting_id(&self)->u128 { 1 }
 fn native_on_left_click_entity(&mut self,_:u128,_:&Stack,_:i32,_:&str,c:&mut bool) { self.calls+=1; *c=self.canceled; }
 fn native_on_entity_swing(&mut self,_:&Stack,_:Option<i32>,_:Hand,c:&mut bool) { self.calls+=1; *c=self.canceled; }
 fn try_swing_hand(&mut self,h:Hand,_:SwingAnimation)->bool { !self.native_entity_swing_event(h) }
 fn send(&mut self,_:S) { self.packets+=1; }
 fn attack_packet(&mut self) { PACKET }
 METHODS
#[test] fn unrelated_item_never_converts() {
 let mut g=game("unrelated",false);
 assert!(!g.native_left_click_entity_event(1,"pig"));
 assert!(!g.native_entity_swing_event(Hand::Main));
 assert_eq!(CLONES.get(),0); assert_eq!(g.calls,0);
 g.stack.item="subscribed".into();
 g.native_left_click_entity_event(1,"pig"); g.native_entity_swing_event(Hand::Main);
 assert_eq!(CLONES.get(),2); assert_eq!(g.calls,2);
}
#[test] fn canceled_swing_sends_no_punch() {
 let mut g=game("subscribed",true); g.attack_packet(); assert_eq!(g.packets,0);
 g.canceled=false; g.attack_packet(); assert_eq!(g.packets,1);
}
fn game(item:&str,canceled:bool)->Game { Game { hand:Hand::Main, offhand:None,player:None,stack:Stack{item:item.into()}, calls:0,packets:0,canceled } }
'''.replace('METHODS', methods).replace('PACKET', packet)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            (path / 'test.rs').write_text(fixture)
            subprocess.run([compiler, '--edition=2024', '--test', str(path / 'test.rs'), '-o', str(path / 'test')], check=True)
            subprocess.run([str(path / 'test')], check=True)
