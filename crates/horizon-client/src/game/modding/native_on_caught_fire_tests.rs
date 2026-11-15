//! `on_caught_fire` heard by a block Part (arismoko/horizon#3185): the listener gets the burned block's old state
//! and the face toward the fire, and what it writes through its level is in the world.
use super::native_entity_refs::tests::with_game;
use horizon_core::{geometry::Direction, position::BlockPos};
use horizon_world::level::LevelAccess;
use mod_api::{
    self, BlockPart, BlockUpdateFlags, FireOddsEvent, Hooks, LevelHost, LevelWriteHost,
    OnCaughtFireEvent, Registrar,
};

/// Wool that, burned from its west face, leaves its old state on the cell above.
struct Ember(std::sync::Arc<std::sync::atomic::AtomicUsize>);

#[mod_api::horizon_part]
impl BlockPart for Ember {
    fn fire_odds(&self, _: &dyn LevelHost, _: &mut FireOddsEvent) -> Option<(i32, i32)> {
        Some((0, 300))
    }
    fn on_caught_fire(&self, level: &mut dyn LevelWriteHost, event: &mut OnCaughtFireEvent) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            level.block_state([event.pos.x, event.pos.y, event.pos.z]),
            Some(event.state)
        );
        if event.face == Direction::West {
            let above = [event.pos.x, event.pos.y + 1, event.pos.z];
            level.set_block(above, event.state, BlockUpdateFlags::NONE);
        }
    }
}

struct EmberMod(std::sync::Arc<std::sync::atomic::AtomicUsize>);

#[mod_api::__subscriptions]
impl Hooks for EmberMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let wool = r.block_key("minecraft:white_wool");
        r.block_part(wool, Ember(self.0.clone()));
    }
}

#[test]
#[ignore = "daily"]
fn a_block_part_hears_it_burned_and_writes_the_level() {
    use horizon_client::chunk::Chunk;
    use std::sync::Arc;

    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = Arc::new(AtomicUsize::new(0));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "ember",
        Box::new(EmberMod(calls.clone())),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(horizon_core::position::ChunkPos::new(0, 0))
                .unwrap();
            let registry = game.pack.assets.registry;
            let wool = registry.default_state("minecraft:white_wool").unwrap();
            let air = registry.default_state("minecraft:air").unwrap();
            game.world.set(1, 64, 0, wool).unwrap();
            // The wool is still present when the notice is dispatched.
            let burn = |game: &mut super::Game<'_>, face: Direction| {
                game.with_level(|_, level, ticks| {
                    level.on_caught_fire(ticks, wool, BlockPos::new(1, 64, 0), face);
                });
                game.world.block(1, 65, 0)
            };
            assert_eq!(burn(game, Direction::North), air, "the Part writes nothing");
            assert_eq!(
                burn(game, Direction::West),
                wool,
                "its write is in the level"
            );
            // Now burn the neighbour through the real simulation and dispatch.
            calls.store(0, Ordering::SeqCst);
            let fire = registry.default_state("minecraft:fire").unwrap();
            let rack = registry.default_state("minecraft:netherrack").unwrap();
            game.world.set(0, 63, 0, rack).unwrap();
            game.world.set(0, 64, 0, fire).unwrap();
            game.world.set(1, 65, 0, air).unwrap();
            game.with_level(|b, level, ticks| {
                level.rules.fire_spread_radius_around_player = -1;
                b.scheduled_block_tick(level, ticks, BlockPos::new(0, 64, 0));
            });
            assert_ne!(game.world.block(1, 64, 0), wool);
            assert_eq!(game.world.block(1, 65, 0), wool);
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            game.world.set(1, 64, 0, wool).unwrap();
            game.world.set(1, 65, 0, air).unwrap();
            game.options.dedicated = None;
            assert_eq!(
                burn(game, Direction::West),
                air,
                "a client dispatches no notice"
            );
        })
    });
}
