//! `player.place_block` against real LAN guests, through the public player handle, with a
//! literal test for each refusal.

use super::native_entity_refs::tests::with_client_game;
use super::native_guest_turn_tests::with_guests;
use mod_api::{EntityRef, PlayerWorldRefusal as Refusal, WorldEpoch};

/// The API's value for a block state of the world.
fn api_state(game: &super::Game<'_>, name: &str) -> mod_api::BlockState {
    use super::api_values::TryIntoApi;
    game.pack
        .assets
        .registry
        .default_state(name)
        .unwrap()
        .try_into_api()
        .unwrap()
}

fn loaded(game: &mut super::Game<'_>) {
    // The write is `level_set_block`'s, which an authoritative server only takes.
    game.options.dedicated = Some(Vec::new());
    game.world.chunks.insert(
        (0, 0),
        std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
    );
}

/// A bed written for a player gets its head (`setPlacedBy`).
#[test]
#[ignore = "daily"]
fn a_block_placed_as_a_player_runs_set_placed_by() {
    with_guests(|game| {
        loaded(game);
        let b = game.player(13).unwrap();
        let level = game.level(game.dim.id()).unwrap();
        let bed = game
            .pack
            .assets
            .registry
            .default_state("minecraft:red_bed")
            .unwrap();
        let api_bed = api_state(game, "minecraft:red_bed");
        assert_eq!(
            b.place_block(game, level, [2, 70, 2], api_bed, true),
            Ok(())
        );
        assert_eq!(game.world.block(2, 70, 2), bed);
        let head = game.world.block(2, 70, 1);
        let facts = game.res.facts();
        assert_eq!(
            facts.get(head).name,
            facts.get(bed).name,
            "the head was placed"
        );
    });
}

/// Each refusal is its own name and writes nothing.
#[test]
#[ignore = "daily"]
fn each_place_block_refusal_has_its_own_name_and_writes_nothing() {
    with_guests(|game| {
        loaded(game);
        let b = game.player(13).unwrap();
        let level = game.level(game.dim.id()).unwrap();
        let stone = api_state(game, "minecraft:stone");
        let empty = game.world.block(3, 70, 3);
        let at = [2, 70, 2];
        assert_eq!(b.place_block(game, level, at, stone, false), Ok(()));
        assert_eq!(
            b.place_block(game, level, at, stone, false),
            Err(Refusal::Unchanged),
            "stone already stands there"
        );
        for pos in [[2, 400, 2], [2, -100, 2], [30_000_000, 70, 2]] {
            assert_eq!(
                b.place_block(game, level, pos, stone, true),
                Err(Refusal::OutOfWorld { pos })
            );
        }
        let far = [1600, 70, 1600];
        assert_eq!(
            b.place_block(game, level, far, stone, true),
            Err(Refusal::NotLoaded { pos: far })
        );
        // A mod cannot make a state the world lacks; the game's own check names one.
        let nothing = horizon_world::registry::BlockStateId(u32::MAX);
        assert_eq!(
            game.player_place_block(b, level, [3, 70, 3], nothing, true),
            Err(Refusal::UnknownState)
        );
        let old =
            mod_api::LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
        assert_eq!(
            b.place_block(game, old, [3, 70, 3], stone, true),
            Err(Refusal::StaleLevel)
        );
        let gone = mod_api::__private::game::player_ref(
            EntityRef::new(
                WorldEpoch::new(b.entity().epoch().get() + 1),
                b.entity().id(),
            ),
            b.uuid(),
        );
        assert_eq!(
            gone.place_block(game, level, [3, 70, 3], stone, true),
            Err(Refusal::Stale)
        );
        assert_eq!(game.world.block(3, 70, 3), empty, "nothing written");
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(13, |g| g.dead = true)
            .unwrap();
        assert_eq!(
            b.place_block(game, level, [3, 70, 3], stone, true),
            Err(Refusal::Dead)
        );
        assert_eq!(game.world.block(3, 70, 3), empty, "nothing written");
    });
}

/// A level that is open but is not the one the player is in is `OtherLevel`.
#[test]
#[ignore = "daily"]
fn another_level_is_not_where_the_player_is() {
    with_guests(|game| {
        loaded(game);
        let b = game.player(13).unwrap();
        let overworld = game.level("minecraft:overworld").unwrap();
        let stone = api_state(game, "minecraft:stone");
        game.enter_level(super::Dim::Nether).unwrap();
        assert_eq!(
            b.place_block(game, overworld, [2, 70, 2], stone, true),
            Err(Refusal::OtherLevel)
        );
    });
}

/// On a client it is `Client`.
#[test]
#[ignore = "daily"]
fn a_client_cannot_place_a_block_as_a_player() {
    with_client_game(|game| {
        let level = mod_api::LevelRef::new(WorldEpoch::new(0), game.dim.key());
        let ghost = mod_api::__private::game::player_ref(EntityRef::new(WorldEpoch::new(0), 1), 1);
        let stone = api_state(game, "minecraft:stone");
        assert_eq!(
            ghost.place_block(game, level, [2, 70, 2], stone, true),
            Err(Refusal::Client)
        );
    });
}
