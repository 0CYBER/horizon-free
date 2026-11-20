//! An anvil a mod opens with no block (game/modding/native_stations.rs) wears no block.
use crate::game::native_entity_refs::tests::with_game;
use crate::game::native_stations_tests::{menu_key, stick_guest};
use horizon_core::random::RandomSource;

/// A take on an anvil opened with no block rolls no wear and leaves an anvil block where the
/// player stands alone; the same take on that block's own menu rolls the block's wear
/// (`AnvilMenu.onTake`).
#[test]
#[ignore = "daily"]
fn an_anvil_with_no_block_wears_no_block() {
    with_game(|game| {
        let anvil = game
            .pack
            .assets
            .registry
            .default_state("minecraft:anvil")
            .unwrap();
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
        );
        game.world.set(6, 200, 10, anvil).unwrap();
        let _rx = stick_guest(game);
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |g| g.position = [6.5, 200.0, 10.5]);
        let player = game.player(123).unwrap();
        let stick = mod_api::__private::item_key("minecraft:stick").unwrap();
        let window = game
            .player_open_station(player, menu_key("minecraft:anvil"), Vec::new(), vec![stick])
            .unwrap()
            .unwrap();
        let mut gs = game
            .lan
            .as_mut()
            .unwrap()
            .menus
            .get_mut(&1)
            .unwrap()
            .station
            .take()
            .unwrap();
        // The level random's next draw: `AnvilMenu.onTake` rolls the block's wear from it.
        let next = |game: &super::Game<'_>| game.world.random.clone().next_long();
        let before = next(game);
        gs.menu.combined = true;
        game.lan_station_combined(1, &mut gs, None);
        assert_eq!(next(game), before, "no wear rolled");
        assert_eq!(game.world.block(6, 200, 10), anvil);

        // The block's own anvil menu at the same place.
        assert!(game.portable_station_closed(1, window));
        gs.menu.combined = true;
        game.lan_station_combined(1, &mut gs, None);
        assert_ne!(next(game), before, "the block's wear rolled");
    });
}
