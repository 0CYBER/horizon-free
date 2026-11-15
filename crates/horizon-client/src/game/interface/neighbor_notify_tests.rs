//! `neighbor_notify` reaches mods from the server's neighbour updates.
use horizon_core::{geometry::Direction, position::BlockPos};
use mod_api::{Flow, Hooks, LevelHost, NeighborNotifyEvent, Registrar};
use std::sync::Mutex;

static HEARD: Mutex<Vec<(i32, i32, i32, Option<Direction>)>> = Mutex::new(Vec::new());

/// Cancels the updates from x 0 and logs what it is asked.
struct Hush;
#[mod_api::__subscriptions]
impl Hooks for Hush {
    fn register(&self, _: &mut dyn Registrar, _: &mod_api::RegistrationContext) {}
    fn neighbor_notify(&self, _: &dyn LevelHost, event: &mut NeighborNotifyEvent) -> Flow {
        let BlockPos { x, y, z } = event.pos;
        HEARD.lock().unwrap().push((x, y, z, event.skipped_side));
        if x == 0 { Flow::Cancel } else { Flow::Continue }
    }
}

#[test]
#[ignore = "daily"]
fn a_cancelled_update_leaves_the_lamp_unlit() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("hush", Box::new(Hush))].into();
    mod_platform::testing::with_mods(mods, || {
        super::native_guest_turn_tests::with_guests(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            let registry = &game.pack.assets.registry;
            let power = registry.default_state("minecraft:redstone_block").unwrap();
            let lamp = registry.default_state("minecraft:redstone_lamp").unwrap();
            // Two redstone blocks set without updates, each with an unlit
            // lamp east of it.
            for x in [0, 5] {
                game.world.set(x, 64, 0, power).unwrap();
                game.world.set(x + 1, 64, 0, lamp).unwrap();
            }
            game.with_level(|behaviours, level, ticks| {
                let block = behaviours.block(power);
                for x in [0, 5] {
                    behaviours.update_neighbors_at(level, ticks, BlockPos::new(x, 64, 0), block);
                }
            });
            assert_eq!(game.world.block(1, 64, 0), lamp, "the cancelled update");
            assert_ne!(
                game.world.block(6, 64, 0),
                lamp,
                "the lamp that heard lights"
            );
            let heard = HEARD.lock().unwrap();
            assert_eq!(heard[..2], [(0, 64, 0, None), (5, 64, 0, None)]);
        });
    });
}
