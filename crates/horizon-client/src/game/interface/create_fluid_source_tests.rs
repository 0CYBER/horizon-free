//! `create_fluid_source` reaches mods from the server's fluid ticks.
use horizon_core::position::BlockPos;
use mod_api::{CreateFluidSourceEvent, Hooks, LevelHost, Registrar};
use std::sync::Mutex;

static HEARD: Mutex<Vec<(i32, i32, i32, bool)>> = Mutex::new(Vec::new());

/// Refuses the water source at x 0 and logs what it is asked.
struct Dry;
#[mod_api::__subscriptions]
impl Hooks for Dry {
    fn register(&self, _: &mut dyn Registrar, _: &mod_api::RegistrationContext) {}
    fn create_fluid_source(&self, _: &dyn LevelHost, event: &mut CreateFluidSourceEvent) {
        let BlockPos { x, y, z } = event.pos;
        HEARD.lock().unwrap().push((x, y, z, event.can_convert));
        if x == 0 {
            event.can_convert = false;
        }
    }
}

#[test]
#[ignore = "daily"]
fn a_mod_keeps_one_source_from_counting() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("dry", Box::new(Dry))].into();
    mod_platform::testing::with_mods(mods, || {
        super::native_guest_turn_tests::with_guests(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            let registry = &game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let source = registry.default_state("minecraft:water").unwrap();
            let flowing = registry
                .resolve(&horizon_world::chunk::BlockState {
                    name: "minecraft:water".into(),
                    properties: [("level".to_string(), "1".to_string())].into(),
                })
                .unwrap();
            // Two pools: sources at x 0 and 2 around flowing water at 1, and
            // at x 10 and 12 around 11.
            for x in 0..=12 {
                game.world.set(x, 63, 0, stone).unwrap();
            }
            for base in [0, 10] {
                game.world.set(base, 64, 0, source).unwrap();
                game.world.set(base + 1, 64, 0, flowing).unwrap();
                game.world.set(base + 2, 64, 0, source).unwrap();
            }
            game.with_level(|behaviours, level, ticks| {
                for x in [1, 11] {
                    behaviours.scheduled_fluid_tick(level, ticks, BlockPos::new(x, 64, 0));
                }
            });
            assert_ne!(game.world.block(1, 64, 0), source, "one counted source");
            assert_eq!(game.world.block(11, 64, 0), source, "two counted sources");
            let heard = HEARD.lock().unwrap();
            for asked in [(0, 64, 0, true), (2, 64, 0, true), (10, 64, 0, true)] {
                assert!(heard.contains(&asked), "{asked:?} in {heard:?}");
            }
        });
    });
}
