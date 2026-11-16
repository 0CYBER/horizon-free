"""Exercise the hand renderer's actual dispatch (`render_hand`) with a fake mod."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class RenderHandHookTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_each_hand_can_be_cancelled_posed_or_drawn_in(self):
        source = next(p.read_text() for p in [ROOT / "crates/horizon-client/src/game.rs", *sorted((ROOT / "crates/horizon-client/src/game").rglob("*.rs"))] if "fn hand_geometry(" in p.read_text())
        start = source.index("            let snapshot =", source.index("fn hand_geometry("))
        end = source.index("            // No mod moved the hand", start)
        dispatch = source[start:end]
        pitch_start = source.index("        let interpolated_pitch =", source.index("fn hand_geometry("))
        pitch_end = source.index("        // FirstPersonHandsAndItemsRenderer", pitch_start)
        pitch = source[pitch_start:pitch_end]
        self.assertLess(source.index("if !shown[i]", source.index("fn hand_geometry(")), start)
        self.assertLess(end, source.index("let Some(stack) = item", end))
        self.assertLess(end, source.index("if let Some(id) = maps::map_id", end))
        fixture = r"""
mod mod_api {
    pub enum HookId { RenderHand }
    pub mod __private {
        std::thread_local! { pub static LISTENED: std::cell::Cell<bool> = const { std::cell::Cell::new(true) }; pub static CONVERSIONS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) }; }
        pub fn listened(_: super::HookId) -> bool { LISTENED.get() }
    }
    pub struct ItemStack { pub value: i32 }
    pub struct HandRender<'a> {
        pub player: u128, pub player_entity_id: i32, pub hand: usize,
        pub stack: Option<&'a ItemStack>, pub partial: f32, pub interpolated_pitch: f32,
        pub swing: f32, pub equip: f32, pub packed_light: u32,
        pub translation: [f32; 3], pub rotation: [f32; 3], pub scale: f32,
    }
}
struct EngineStack { count: u32, value: i32 }
trait TryIntoApi { fn try_into_api(&self) -> Result<Option<mod_api::ItemStack>, ()>; }
impl TryIntoApi for Option<&EngineStack> {
    fn try_into_api(&self) -> Result<Option<mod_api::ItemStack>, ()> {
        mod_api::__private::CONVERSIONS.set(mod_api::__private::CONVERSIONS.get() + 1);
        Ok(self.map(|stack| mod_api::ItemStack { value: stack.value }))
    }
}
struct Model { previous_head_pitch: f32 }
struct Game { cancel: Option<usize>, calls: Vec<(usize, Option<i32>, f32, f32, f32)>, runs: Vec<(u32, i32)>, pitch: f32, player_model: Option<Model> }
impl Game {
    fn own_uuid(&self) -> u128 { 7 }
    fn own_entity_id(&self) -> i32 { 9 }
    // The real one hands the mods a draw at `to_world`, appends what they drew and says whether they cancelled.
    fn render_hand_cancelled(&mut self, render: &mut mod_api::HandRender<'_>, to_world: f32, light: u32, out: &mut Vec<usize>, runs: &mut Vec<(u32, i32)>) -> bool {
        assert_eq!((to_world, light), (1.5, 77));
        assert_eq!((render.player, render.player_entity_id, render.interpolated_pitch, render.packed_light), (7, 9, 2.0, 3 << 4 | 10 << 20));
        self.calls.push((render.hand, render.stack.map(|stack| stack.value), render.partial, render.swing, render.equip));
        render.translation[0] += 2.0;
        render.rotation[1] += 15.0;
        render.scale *= 2.0;
        runs.push((out.len() as u32, 0));
        out.push(render.hand);
        self.cancel == Some(render.hand)
    }
    fn draw(&mut self) -> Vec<(usize, [f32; 3], [f32; 3], f32)> {
        let mut rendered = Vec::new();
        let (to_world, packed) = (1.5_f32, 77_u32);
        let (mut out, mut runs) = (Vec::new(), Vec::new());
        for (hand, item) in [Some(EngineStack { count: 1, value: 42 }), None].into_iter().enumerate() {
            let (a, attack, inverse_height, light) = (0.5, 0.25, 0.75, 0xA3u8);
            PITCH
            DISPATCH
            rendered.push((hand, render.translation, render.rotation, render.scale));
        }
        self.runs = runs;
        rendered
    }
}
#[test]
fn cancellation_and_pose_share_one_dispatch_for_each_hand() {
    for (cancel, hands) in [(None, vec![0, 1]), (Some(0), vec![1]), (Some(1), vec![0])] {
        let mut game = Game { cancel, calls: Vec::new(), runs: Vec::new(), pitch: 3.0, player_model: Some(Model { previous_head_pitch: 1.0 }) };
        let expected = hands.into_iter().map(|h| (h, [2.0, 0.0, 0.0], [0.0, 15.0, 0.0], 2.0)).collect::<Vec<_>>();
        mod_api::__private::CONVERSIONS.set(0);
        assert_eq!(game.draw(), expected);
        assert_eq!(mod_api::__private::CONVERSIONS.get(), 2);
        assert_eq!(game.calls, vec![(0, Some(42), 0.5, 0.25, 0.75), (1, None, 0.5, 0.25, 0.75)]);
        assert_eq!(game.runs, vec![(0, 0), (1, 0)], "what a mod drew is in the hand's vertices, cancelled or not");
    }
    mod_api::__private::LISTENED.set(false);
    mod_api::__private::CONVERSIONS.set(0);
    let mut game = Game { cancel: Some(0), calls: Vec::new(), runs: Vec::new(), pitch: 3.0, player_model: Some(Model { previous_head_pitch: 1.0 }) };
    assert_eq!(game.draw(), vec![(0, [0.0; 3], [0.0; 3], 1.0), (1, [0.0; 3], [0.0; 3], 1.0)]);
    assert!(game.calls.is_empty());
    assert!(game.runs.is_empty());
    assert_eq!(mod_api::__private::CONVERSIONS.get(), 0);
}
""".replace("DISPATCH", dispatch).replace("PITCH", pitch)
        target = ROOT / "target"
        target.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=target) as folder:
            src, exe = Path(folder) / "hand.rs", Path(folder) / "hand.exe"
            src.write_text(fixture)
            subprocess.run(["rustc", "--edition=2024", "--test", str(src), "-o", str(exe)], check=True)
            subprocess.run([str(exe)], check=True)
