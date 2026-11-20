//! `player.sees`, `player.ack_block_changes` and `player.show_block` against real LAN guests,
//! through the public player handle, with a literal test for each refusal.

use super::native_entity_refs::tests::with_client_game;
use super::native_guest_turn_tests::with_guests;
use horizon_protocol::packets::play::Clientbound as C;
use mod_api::{
    EntityRef, EntityVisibilityRefusal as Visibility, PlayerRef, PlayerWorldRefusal as Refusal,
    WorldEpoch,
};

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

/// The handle of a player of an earlier open of the world.
fn stale(player: PlayerRef) -> PlayerRef {
    let entity = player.entity();
    mod_api::__private::game::player_ref(
        EntityRef::new(WorldEpoch::new(entity.epoch().get() + 1), entity.id()),
        player.uuid(),
    )
}

/// A fake player: no connection, not in the server's list.
fn fake(game: &mut super::Game<'_>) -> PlayerRef {
    game.next_entity = 100;
    let level = game.level(game.dim.id()).unwrap();
    game.fake_player(level, 1234, "machine").unwrap()
}

fn kept(game: &super::Game<'_>) -> i32 {
    game.lan
        .as_ref()
        .unwrap()
        .hub
        .guest(13)
        .unwrap()
        .ack_up_to
        .get()
}

/// The highest sequence of the tick waits as the player's ack, in their turn or out of it.
#[test]
#[ignore = "daily"]
fn a_mod_acknowledges_block_changes() {
    with_guests(|game| {
        let b = game.player(13).unwrap();
        assert_eq!(b.ack_block_changes(game, 7), Ok(()));
        assert_eq!(b.ack_block_changes(game, 3), Ok(()));
        assert_eq!(kept(game), 7);
        game.as_guest(13, |game| assert_eq!(b.ack_block_changes(game, 9), Ok(())))
            .unwrap();
        assert_eq!(kept(game), 9);
    });
}

/// Each refusal of the acknowledgement is its own name and leaves the pending ack alone.
#[test]
#[ignore = "daily"]
fn each_block_ack_refusal_has_its_own_name() {
    with_guests(|game| {
        let b = game.player(13).unwrap();
        assert_eq!(
            b.ack_block_changes(game, -1),
            Err(Refusal::NegativeSequence { sequence: -1 })
        );
        assert_eq!(
            b.ack_block_changes(game, 0),
            Ok(()),
            "0 is the first sequence"
        );
        assert_eq!(kept(game), 0);
        assert_eq!(stale(b).ack_block_changes(game, 5), Err(Refusal::Stale));
        let machine = fake(game);
        assert_eq!(
            machine.ack_block_changes(game, 5),
            Err(Refusal::NoConnection)
        );
        assert_eq!(kept(game), 0);
    });
}

/// A block shown to one player goes to their client alone and changes nothing in the world.
#[test]
#[ignore = "daily"]
fn a_block_is_shown_to_one_player() {
    with_guests(|game| {
        let b = game.player(13).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update_connection(13, |g| {
                g.sender = horizon_client::lan::PlayerSender::new(tx, Default::default());
            });
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        let before = game.world.block(1, 70, 1);
        let shown_state = api_state(game, "minecraft:stone");
        assert_eq!(b.show_block(game, [1, 70, 1], shown_state), Ok(()));
        assert_eq!(game.world.block(1, 70, 1), before);
        let mut shown = Vec::new();
        while let Ok(horizon_client::lan::Out::Packet(body)) = rx.try_recv() {
            let mut r = horizon_protocol::buf::Reader::new(&body);
            let id = r.varint().unwrap();
            if let Ok(C::BlockUpdate(u)) = C::decode(id, r.rest()) {
                shown.push((u.pos, u.state));
            }
        }
        assert_eq!(shown, [([1, 70, 1], stone.0 as i32)]);
    });
}

/// Each refusal of `show_block` is its own name and sends nothing; the edges of the packet's
/// position are taken.
#[test]
#[ignore = "daily"]
fn each_show_block_refusal_has_its_own_name() {
    with_guests(|game| {
        let b = game.player(13).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update_connection(13, |g| {
                g.sender = horizon_client::lan::PlayerSender::new(tx, Default::default());
            });
        let stone = api_state(game, "minecraft:stone");
        for pos in [
            [1, 2048, 1],
            [1, -2049, 1],
            [1 << 25, 70, 1],
            [1, 70, -(1 << 25) - 1],
        ] {
            assert_eq!(
                b.show_block(game, pos, stone),
                Err(Refusal::PositionNotSendable { pos })
            );
        }
        for pos in [
            [(1 << 25) - 1, 2047, -(1 << 25)],
            [-(1 << 25), -2048, (1 << 25) - 1],
        ] {
            assert_eq!(
                b.show_block(game, pos, stone),
                Ok(()),
                "{pos:?} fits the packet"
            );
        }
        // A mod cannot make a state the world lacks; the game's own check names one.
        assert_eq!(
            game.player_show_block(
                b,
                [1, 70, 1],
                horizon_world::registry::BlockStateId(u32::MAX)
            ),
            Err(Refusal::UnknownState)
        );
        assert_eq!(
            stale(b).show_block(game, [1, 70, 1], stone),
            Err(Refusal::Stale)
        );
        let machine = fake(game);
        assert_eq!(
            machine.show_block(game, [1, 70, 1], stone),
            Err(Refusal::NoConnection)
        );
        let mut sent = 0;
        while let Ok(horizon_client::lan::Out::Packet(body)) = rx.try_recv() {
            let mut r = horizon_protocol::buf::Reader::new(&body);
            let id = r.varint().unwrap();
            sent += matches!(C::decode(id, r.rest()), Ok(C::BlockUpdate(_))) as usize;
        }
        assert_eq!(sent, 2, "only the two positions at the edge went out");
    });
}

/// A player is seen unless a mod hid them below the viewer's level, or they spectate and the
/// viewer doesn't.
#[test]
#[ignore = "daily"]
fn player_sees_follows_the_trackers_rules() {
    with_guests(|game| {
        let (a, b) = (game.player(12).unwrap(), game.player(13).unwrap());
        assert_eq!(a.sees(game, b), Ok(true));
        assert_eq!(a.sees(game, a), Ok(true));
        assert!(game.player_hide(b, Some(3)));
        assert_eq!(a.sees(game, b), Ok(false));
        assert_eq!(b.sees(game, a), Ok(true));
        let guests = &mut game.lan.as_mut().unwrap().hub.guests;
        guests.update_connection(12, |g| g.permission = 3);
        assert_eq!(a.sees(game, b), Ok(true));
        assert!(game.player_hide(b, None));
        let guests = &mut game.lan.as_mut().unwrap().hub.guests;
        guests.update_connection(13, |g| g.game_mode = 3);
        assert_eq!(a.sees(game, b), Ok(false), "a spectator");
        assert_eq!(b.sees(game, a), Ok(true));
        // A player being sent again is out of the tracker's world and the Tab list: not seen meanwhile,
        // whatever the viewer's level, and seen again (at the level a mod hid them at) when it ends.
        let guests = &mut game.lan.as_mut().unwrap().hub.guests;
        guests.update_connection(13, |g| g.game_mode = 0);
        assert_eq!(a.sees(game, b), Ok(true));
        assert!(game.player_refresh(b));
        assert_eq!(a.sees(game, b), Ok(false), "refreshing");
        assert_eq!(b.sees(game, a), Ok(true), "the others are unaffected");
        let like_the_list = game.lan.as_ref().unwrap().hub.sees(3, 13);
        assert_eq!(a.sees(game, b), Ok(like_the_list));
        let hub = &mut game.lan.as_mut().unwrap().hub;
        hub.ticks += 2;
        hub.end_refreshes();
        assert_eq!(a.sees(game, b), Ok(true));
    });
}

/// An entity a mod hid from one viewer is hidden from that viewer alone, reads back, makes
/// `sees` answer false for a player and is shown again; the viewer's own player is refused.
#[test]
#[ignore = "daily"]
fn an_entity_is_hidden_from_one_viewer_alone() {
    with_guests(|game| {
        let (a, b) = (game.player(12).unwrap(), game.player(13).unwrap());
        assert_eq!(b.entity().visible_to(game, a), Ok(true));
        assert_eq!(b.entity().set_visible_to(game, a, false), Ok(true));
        assert_eq!(
            b.entity().set_visible_to(game, a, false),
            Ok(false),
            "no change"
        );
        assert_eq!(b.entity().visible_to(game, a), Ok(false));
        assert_eq!(
            a.entity().visible_to(game, b),
            Ok(true),
            "the other way is untouched"
        );
        assert_eq!(a.sees(game, b), Ok(false));
        assert_eq!(b.sees(game, a), Ok(true));
        assert_eq!(
            a.entity().set_visible_to(game, a, false),
            Err(Visibility::StaleEntity),
            "a viewer's own player is never tracked to it"
        );
        assert_eq!(b.entity().set_visible_to(game, a, true), Ok(true));
        assert_eq!(b.entity().visible_to(game, a), Ok(true));
        assert_eq!(a.sees(game, b), Ok(true));
        assert_eq!(
            b.entity().set_visible_to(game, stale(a), false),
            Err(Visibility::Stale)
        );
        let machine = fake(game);
        assert_eq!(
            b.entity().set_visible_to(game, machine, false),
            Err(Visibility::NoConnection)
        );
    });
}

/// `sees` names a stale handle of either player and a fake player, and a client.
#[test]
#[ignore = "daily"]
fn each_sees_refusal_has_its_own_name() {
    with_guests(|game| {
        let (a, b) = (game.player(12).unwrap(), game.player(13).unwrap());
        assert_eq!(a.sees(game, stale(b)), Err(Refusal::Stale));
        assert_eq!(stale(a).sees(game, b), Err(Refusal::Stale));
        let machine = fake(game);
        assert_eq!(a.sees(game, machine), Err(Refusal::NoConnection));
        assert_eq!(machine.sees(game, a), Err(Refusal::NoConnection));
    });
}

/// On a client all three answer `Client`.
#[test]
#[ignore = "daily"]
fn a_client_cannot_ack_show_or_ask_who_sees() {
    with_client_game(|game| {
        let ghost = mod_api::__private::game::player_ref(EntityRef::new(WorldEpoch::new(0), 1), 1);
        let stone = api_state(game, "minecraft:stone");
        assert_eq!(ghost.ack_block_changes(game, 1), Err(Refusal::Client));
        assert_eq!(
            ghost.show_block(game, [1, 70, 1], stone),
            Err(Refusal::Client)
        );
        assert_eq!(ghost.sees(game, ghost), Err(Refusal::Client));
    });
}
