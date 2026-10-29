"""Regression for menus left open when a server replaces the local player."""
from pathlib import Path
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


class RemoteMenuTransferTests(unittest.TestCase):
    def test_login_and_respawn_reset_before_replacing_world(self):
        source = (ROOT / "crates/horizon-client/src/game/remote.rs").read_text(encoding="utf-8")
        enter = method(source, "enter_remote_dimension")
        self.assertLess(enter.index("self.remote_close_local()"), enter.index("self.world = World::new"))
        self.assertIn("self.screen = Screen::Playing", enter)
        handler = method(source, "handle_packet")
        login = handler[handler.index("C::Login(p) =>"):handler.index("C::Respawn(p) =>")]
        respawn = handler[handler.index("C::Respawn(p) =>"):handler.index("C::PlayerPosition(p) =>")]
        self.assertIn("self.enter_remote_dimension(&p.spawn, false)", login)
        self.assertIn("self.enter_remote_dimension(&p.spawn, true)", respawn)

    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_local_close_clears_stale_menu_and_restores_inventory(self):
        source = (ROOT / "crates/horizon-client/src/game/remote.rs").read_text(encoding="utf-8")
        fixture = r'''
use std::collections::HashMap;
#[derive(Default)]
struct Remote { menu: Option<(i32, String)>, menu_title: Option<String> }
#[derive(Default)]
struct RecipeUi { resets: usize }
impl RecipeUi { fn reset_screen(&mut self) { self.resets += 1; } }
struct Entity { slots: Vec<Option<i32>> }
impl Entity { fn slots_mut(&mut self) -> &mut Vec<Option<i32>> { &mut self.slots } }
#[derive(Default)]
struct World { block_entities: HashMap<(i32, i32, i32), Entity> }
struct Game {
    remote: Option<Remote>, recipe_ui: RecipeUi, mount: bool, station: Option<i32>,
    craft_grid: Vec<Option<i32>>, carried: Option<i32>, craft_size: usize,
    container: Option<(i32, i32, i32)>, world: World, inventory: Vec<Option<i32>>,
}
impl Game {
    fn mount_menu_reset(&mut self) { self.mount = false; }
    __CLOSE__
}
fn main() {
    for old_world_survives in [true, false] {
        let key = (1, 2, 3);
        let mut world = World::default();
        world.block_entities.insert(key, Entity { slots: vec![Some(9); 45] });
        let mut game = Game {
            remote: Some(Remote { menu: Some((7, "minecraft:generic_9x5".into())),
                                  menu_title: Some("Lobby".into()) }),
            recipe_ui: RecipeUi::default(), mount: true, station: Some(4),
            craft_grid: vec![Some(3); 9], carried: Some(5), craft_size: 3,
            container: Some(key), world, inventory: vec![Some(42); 36],
        };
        game.remote_close_local();
        if !old_world_survives { game.world = World::default(); }
        let remote = game.remote.unwrap();
        assert!(remote.menu.is_none() && remote.menu_title.is_none());
        assert!(game.container.is_none() && game.station.is_none() && !game.mount);
        assert!(game.carried.is_none() && game.craft_grid.iter().all(Option::is_none));
        assert_eq!(game.craft_size, 2);
        assert_eq!(game.inventory, vec![Some(42); 36]);
        assert_eq!(game.recipe_ui.resets, 1);
        if old_world_survives {
            assert!(game.world.block_entities[&key].slots.iter().all(Option::is_none));
        }
    }
}
'''.replace("__CLOSE__", method(source, "remote_close_local"))
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "fixture.rs"
            binary = Path(tmp) / "fixture"
            path.write_text(fixture, encoding="utf-8")
            subprocess.run(["rustc", "--edition=2024", str(path), "-o", str(binary)], check=True)
            subprocess.run([str(binary)], check=True)


if __name__ == "__main__":
    unittest.main()
