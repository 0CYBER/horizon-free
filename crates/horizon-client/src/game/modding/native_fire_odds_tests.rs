//! `fire_odds` answered by a block Part with the asked block's position and face (arismoko/horizon#3185): the
//! Part's answer for one face makes the fire beside that face valid, and the other faces keep
//! vanilla's table.
use super::native_entity_refs::tests::with_game;
use horizon_core::{geometry::Direction, position::BlockPos};
use mod_api::{self, BlockPart, FireOddsEvent, Hooks, LevelHost, Registrar};

/// Glass that ignites from its west face only.
struct WestFace;

#[mod_api::horizon_part]
impl BlockPart for WestFace {
    fn fire_odds(&self, _: &dyn LevelHost, event: &mut FireOddsEvent) -> Option<(i32, i32)> {
        (event.face == Direction::West).then_some((5, 0))
    }
}

struct FireOddsMod;

#[mod_api::__subscriptions]
impl Hooks for FireOddsMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let glass = r.block_key("minecraft:glass");
        r.block_part(glass, WestFace);
    }
}

#[test]
#[ignore = "daily"]
fn a_block_part_answers_fire_odds_for_one_face() {
    use horizon_client::chunk::Chunk;
    use std::sync::Arc;

    let mods: mod_api::Mods =
        vec![mod_api::ModEntry::new("fire_odds", Box::new(FireOddsMod))].into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let glass = registry.default_state("minecraft:glass").unwrap();
            let fire = registry.default_state("minecraft:fire").unwrap();
            game.world.set(1, 65, 0, glass).unwrap();
            // Fire in the air at `x`, beside the glass, with air below it: it
            // stands only beside a flammable face.
            let stands = |game: &mut super::Game<'_>, x: i32| {
                game.with_level(|b, level, ticks| {
                    b.can_survive(level, ticks, BlockPos::new(x, 65, 0), fire)
                })
            };
            assert!(stands(game, 0), "the Part makes the west face flammable");
            assert!(!stands(game, 2), "the east face keeps vanilla's never");
            // A client's copy of the level asks no Part.
            game.options.dedicated = None;
            assert!(!stands(game, 0));
        })
    });
}
