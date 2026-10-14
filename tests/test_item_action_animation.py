"""Compile the actual drop and ignition branches with world effects stubbed."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


def block(source, start):
    opening = source.index("{", start)
    depth, end = 1, opening + 1
    while depth:
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    return source[start:end]


class ItemActionAnimationTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "rustc is required for action-policy tests")
    def test_successful_drop_and_ignition_start_a_swing(self):
        source = (ROOT / "crates/horizon-client/src/game.rs").read_text()
        drop = block(source, source.index("fn drop_held("))
        ignition = block(source, source.index("if self.can_place_used_fire(target)"))
        caller_start = source.index("        let swings = self.swings_started;", source.index("fn use_item_as_player("))
        caller = source[caller_start:source.index("        if self.use_potion_item", caller_start)]
        program = """
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
enum UseResult { Pass, Fail, Success { swing: bool } }
#[derive(Clone)] struct ItemStack { count: u32, item: String }
struct Hit { pos: [i32; 3], face: [i32; 3] }
struct Facts; impl Facts { fn is_air(&self, state:u8)->bool { state == 0 } }
struct Res { facts:Facts }
struct Random; impl Random { fn next_float(&mut self)->f32 { 0.5 } }
struct World { air:bool, path:bool, random:Random }
impl World { fn block(&self,_:i32,_:i32,_:i32)->u8 { if self.air {0} else {1} } }
struct Audio { sounds:u32 } impl Audio { fn play(&mut self,_:&str,_:Option<[f64;3]>,_:f32,_:f32) {self.sounds+=1;} }
struct Game { selected:usize, inventory:[Option<ItemStack>;1], alive:bool, swings:u32, drops:u32, res:Res, world:World, audio:Audio }
impl Game {
 DROP
 fn ignite(&mut self,item:&str)->Result<()> {
   let stack=ItemStack {count:1,item:item.into()};
   CALLER
   Ok(())
 }
 fn use_tool(&mut self,stack:&ItemStack)->Result<UseResult> {
   let item=stack.item.as_str();
   let hit = Hit { pos:[0;3], face:[0,1,0] };
   let at = |p:[i32;3]| p.map(f64::from);
   let target = [0,1,0];
   IGNITION
   Ok(UseResult::Fail)
 }
 fn eye_position(&self)->Option<[f64;3]> { self.alive.then_some([0.0;3]) }
 fn player_drop(&mut self,stack:ItemStack) { self.drops += stack.count; }
 fn swing_arm(&mut self) { self.swings += 1; }
 fn try_light_portal(&mut self,_:[i32;3])->Result<bool> {Ok(false)}
 fn may_build(&self)->bool {true}
 fn can_place_used_fire(&self,_:[i32;3])->bool {self.world.air&&!self.world.path}
 fn place_used_fire(&mut self,_:[i32;3])->Result<()> {Ok(())}
 fn consume_held(&mut self) {} fn damage_held(&mut self,_:u32) {}
}
fn game()->Game { Game { selected:0, inventory:[Some(ItemStack { count:4,item:"minecraft:stone".into() })], alive:true, swings:0, drops:0, res:Res {facts:Facts}, world:World {air:true,path:false,random:Random}, audio:Audio {sounds:0} } }
#[test] fn dropping_one_and_all_swings_only_on_success() {
 let mut g=game();g.drop_held(false);assert_eq!((g.drops,g.swings),(1,1));
 g.drop_held(true);assert_eq!((g.drops,g.swings),(4,2));
 g.drop_held(false);assert_eq!(g.swings,2);
 let mut g=game();g.alive=false;g.drop_held(false);assert_eq!(g.swings,0);
}
#[test] fn successful_ignition_swings_but_blocked_ignition_does_not() {
 for item in ["flint_and_steel","fire_charge"] {
  let mut g=game();g.ignite(item).unwrap();assert_eq!((g.swings,g.audio.sounds),(1,1));
  let mut g=game();g.world.air=false;g.ignite(item).unwrap();assert_eq!((g.swings,g.audio.sounds),(0,0));
  let mut g=game();g.world.path=true;g.ignite(item).unwrap();assert_eq!((g.swings,g.audio.sounds),(0,0));
 }
}
""".replace("DROP", drop).replace("IGNITION", ignition).replace("CALLER", caller)
        program = program.replace("self.swings_started", "self.swings")
        with tempfile.TemporaryDirectory(prefix="horizon-item-action-") as directory:
            path = Path(directory)
            (path / "probe.rs").write_text(program)
            compiled = subprocess.run(
                ["rustc", "--edition=2024", "--test", str(path / "probe.rs"), "-o", str(path / "probe")],
                capture_output=True, text=True, check=False,
            )
            self.assertEqual(compiled.returncode, 0, compiled.stderr)
            tested = subprocess.run([str(path / "probe")], capture_output=True, text=True, check=False)
            self.assertEqual(tested.returncode, 0, tested.stdout + tested.stderr)


if __name__ == "__main__":
    unittest.main()
