//! `player_persistent_data` and `player_set_persistent_data` against real LAN guests, through the
//! public player handle.

use super::native_guest_turn_tests::with_guests;
use horizon_core::nbt::{Compound, Tag};
use mod_api::{PlayerDataRefusal, prelude::*};

fn saved_data(game: &super::Game<'_>, conn: u32) -> Option<Tag> {
    let hub = &game.lan.as_ref().unwrap().hub;
    hub.guest(conn)?
        .saved
        .as_ref()?
        .get("NeoForgeData")
        .cloned()
}

/// A chain of compounds `levels` deep, each holding the next under `k`.
fn nested(levels: usize) -> Compound {
    let mut data = Compound::new();
    for _ in 1..levels {
        let mut outer = Compound::new();
        outer.insert("k".into(), Tag::Compound(data));
        data = outer;
    }
    data
}

/// The mods' compound goes into the player's own saved record, in their turn or out of it,
/// and an empty one takes the key away.
#[test]
#[ignore = "daily"]
fn persistent_data_is_kept_in_the_players_record() {
    with_guests(|game| {
        let players = game.players().server_players();
        let (a, b) = (players[0], players[1]);
        assert_eq!(b.persistent_data(game), Some(Compound::new()));
        let mut data = Compound::new();
        data.insert("husksync:key".into(), Tag::Int(7));
        assert_eq!(b.set_persistent_data(game, data.clone()), Ok(()));
        assert_eq!(saved_data(game, 13), Some(Tag::Compound(data.clone())));
        assert_eq!(saved_data(game, 12), None);
        game.as_guest(12, |game| {
            assert_eq!(b.persistent_data(game), Some(data.clone()));
            assert_eq!(a.set_persistent_data(game, data.clone()), Ok(()));
            assert_eq!(a.persistent_data(game), Some(data.clone()));
        })
        .unwrap();
        assert_eq!(saved_data(game, 12), Some(Tag::Compound(data)));
        assert_eq!(b.set_persistent_data(game, Compound::new()), Ok(()));
        assert_eq!(saved_data(game, 13), None);
        assert_eq!(b.persistent_data(game), Some(Compound::new()));
    });
}

/// This API refuses with its own names, writing nothing: data 512 levels deep or more in the
/// player's file (511 are written), a dead player, and a handle whose player has left.
#[test]
#[ignore = "daily"]
fn each_persistent_data_refusal_has_its_own_name_and_writes_nothing() {
    with_guests(|game| {
        let b = game.players().server_players()[1];
        let mut kept = Compound::new();
        kept.insert("kept".into(), Tag::Byte(1));
        assert_eq!(b.set_persistent_data(game, kept.clone()), Ok(()));
        assert_eq!(b.set_persistent_data(game, nested(511)), Ok(()));
        assert_eq!(b.persistent_data(game), Some(nested(511)));
        assert_eq!(b.set_persistent_data(game, kept.clone()), Ok(()));
        assert_eq!(
            b.set_persistent_data(game, nested(512)),
            Err(PlayerDataRefusal::TooDeep)
        );
        assert_eq!(
            b.persistent_data(game),
            Some(kept.clone()),
            "nothing was written"
        );
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update_connection(13, |g| g.dead = true);
        assert_eq!(
            b.set_persistent_data(game, Compound::new()),
            Err(PlayerDataRefusal::Dead)
        );
        assert_eq!(
            b.persistent_data(game),
            Some(kept),
            "a dead player is read, not written"
        );
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update_connection(13, |g| g.dead = false);
        let index = game
            .lan
            .as_ref()
            .unwrap()
            .hub
            .guests
            .connection_index(13)
            .unwrap();
        game.lan.as_mut().unwrap().hub.guests.remove(index);
        assert_eq!(
            b.set_persistent_data(game, Compound::new()),
            Err(PlayerDataRefusal::Stale)
        );
        assert_eq!(b.persistent_data(game), None);
        game.options.execution_role = crate::game::execution_role::ExecutionRole::Client;
        let (client, _server) = horizon_protocol::connection::Pipe::pair();
        game.connect_local(client, "PlayerDataRefusalTest").unwrap();
        assert_eq!(
            b.set_persistent_data(game, Compound::new()),
            Err(PlayerDataRefusal::Client)
        );
    });
}

/// A death respawn carries only `PlayerPersisted` of the mods' compound: a key outside it is
/// lost, a key inside it is read back.
#[test]
#[ignore = "daily"]
fn a_respawn_keeps_only_player_persisted() {
    with_guests(|game| {
        let b = game.players().server_players()[1];
        let mut inside = Compound::new();
        inside.insert("inside".into(), Tag::Int(2));
        let mut data = Compound::new();
        data.insert("outside".into(), Tag::Int(1));
        data.insert("PlayerPersisted".into(), Tag::Compound(inside.clone()));
        assert_eq!(b.set_persistent_data(game, data.clone()), Ok(()));
        assert_eq!(b.persistent_data(game), Some(data), "kept while alive");
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update_connection(13, |g| g.dead = true);
        game.guest_respawn(13);
        let mut after = Compound::new();
        after.insert("PlayerPersisted".into(), Tag::Compound(inside));
        assert_eq!(b.persistent_data(game), Some(after));
        let outside = b
            .persistent_data(game)
            .unwrap()
            .get(&"outside".into())
            .cloned();
        assert_eq!(outside, None, "a key outside PlayerPersisted is lost");
    });
}
