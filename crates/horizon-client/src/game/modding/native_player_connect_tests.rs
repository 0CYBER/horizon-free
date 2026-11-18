//! `player.connect_address` against real LAN guests.

use super::native_guest_turn_tests::with_guests;
use mod_api::{ConnectAddress, EntityRef, WorldEpoch};

/// A guest's address is the host and port its connection carries; one that typed nothing, a
/// stale handle and a fake player have none.
#[test]
#[ignore = "daily"]
fn a_player_has_the_address_their_client_typed() {
    with_guests(|game| {
        let (a, b) = (game.player(12).unwrap(), game.player(13).unwrap());
        assert_eq!(
            b.connect_address(game),
            None,
            "a connection that typed nothing"
        );
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update_connection(13, |g| {
                g.sender.connect = Some(("play.example.com".into(), 25570))
            });
        assert_eq!(
            b.connect_address(game),
            Some(ConnectAddress::new("play.example.com", 25570))
        );
        assert_eq!(a.connect_address(game), None, "the other player's own");
        let entity = b.entity();
        let stale = mod_api::__private::game::player_ref(
            EntityRef::new(WorldEpoch::new(entity.epoch().get() + 1), entity.id()),
            b.uuid(),
        );
        assert_eq!(stale.connect_address(game), None);
        game.next_entity = 100;
        let level = game.level(game.dim.id()).unwrap();
        let machine = game.fake_player(level, 1234, "machine").unwrap();
        assert_eq!(machine.connect_address(game), None);
    });
}
