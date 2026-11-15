//! `smoke_source` answered by a block Part, as Farmer's Delight's head inject on
//! `CampfireBlock.isSmokeSource` needs (arismoko/horizon#1027): the Part's answer replaces vanilla's both ways,
//! and the campfire above takes it as the block under it changes.
use super::native_entity_refs::tests::with_game;
use mod_api::{self, BlockPart, Hooks, LevelHost, Registrar, SmokeSourceEvent};

/// Answers `smoke_source` for its block.
struct Smoky(bool);

#[mod_api::horizon_part]
impl BlockPart for Smoky {
    fn smoke_source(&self, _: &dyn LevelHost, _: &mut SmokeSourceEvent) -> Option<bool> {
        Some(self.0)
    }
}

/// Stone smokes, which vanilla's doesn't; a hay bale doesn't, which vanilla's does.
struct SmokeMod;

#[mod_api::__subscriptions]
impl Hooks for SmokeMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let stone = r.block_key("minecraft:stone");
        r.block_part(stone, Smoky(true));
        let hay = r.block_key("minecraft:hay_block");
        r.block_part(hay, Smoky(false));
    }
}

/// Whether the campfire at (0, 65, 0) is a signal fire.
fn signal_fire(game: &super::Game<'_>) -> bool {
    let state = game
        .pack
        .assets
        .registry
        .state(game.world.block(0, 65, 0))
        .unwrap();
    assert_eq!(&*state.name, "minecraft:campfire");
    state.properties.get("signal_fire").map(String::as_str) == Some("true")
}

#[test]
#[ignore = "daily"]
fn a_block_part_decides_whether_the_campfire_above_is_a_signal_fire() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("smoke", Box::new(SmokeMod))].into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            // An authoritative server asks the Parts; a client keeps vanilla's answer.
            game.options.dedicated = Some(Vec::new());
            game.build_registries();
            game.set_allow_commands(true);
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            game.block_ticks
                .add_empty_chunk(horizon_core::position::ChunkPos::new(0, 0))
                .unwrap();
            assert!(game.run_command("setblock 0 64 0 dirt"));
            assert!(game.run_command("setblock 0 65 0 campfire"));
            assert!(!signal_fire(game), "no Part answers for dirt: vanilla's no");

            // The block under the campfire changes: its shape update asks again.
            assert!(game.run_command("setblock 0 64 0 stone"));
            assert!(signal_fire(game), "the stone Part answers yes");

            assert!(game.run_command("setblock 0 64 0 hay_block"));
            assert!(
                !signal_fire(game),
                "the hay Part's no replaces vanilla's yes"
            );

            assert!(game.run_command("setblock 0 64 0 dirt"));
            assert!(!signal_fire(game));
        })
    });
}
