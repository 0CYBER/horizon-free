//! `PlayerRef::respawn` through the public handle: the Respawn button for a dead player, and
//! each refusal by name.
use super::native_guest_turn_tests::with_guests;
use mod_api::{Hooks, PlayerRespawnedEvent, RespawnRefusal, ServerHost, prelude::*};
use std::sync::{Arc, Mutex};

fn set_dead(game: &mut super::Game<'_>, conn: u32, dead: bool) {
    game.lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .update_connection(conn, |g| {
            g.dead = dead;
            g.health = if dead { 0.0 } else { 20.0 };
        });
}

fn is_dead(game: &super::Game<'_>, conn: u32) -> bool {
    game.lan.as_ref().unwrap().hub.guest(conn).unwrap().dead
}

/// Records each `player_respawned`.
struct Heard(Arc<Mutex<Vec<(u128, String)>>>);

#[mod_api::__subscriptions]
impl Hooks for Heard {
    fn player_respawned(&self, _: &mut dyn ServerHost, event: &mut PlayerRespawnedEvent<'_>) {
        self.0
            .lock()
            .unwrap()
            .push((event.player, event.name.to_owned()));
    }
}

/// A dead player is respawned as their client's button does, `player_respawned` is heard once,
/// and the same handle is then refused as alive.
#[test]
#[ignore = "daily"]
fn a_dead_player_is_respawned_once() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "heard",
        Box::new(Heard(heard.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_guests(|game| {
            let b = game.players().server_players()[1];
            set_dead(game, 13, true);
            assert_eq!(b.respawn(game), Ok(()));
            assert!(!is_dead(game, 13));
            assert_eq!(b, game.player(b.uuid()).unwrap());
            assert_eq!(*heard.lock().unwrap(), [(13, "guest13".to_owned())]);
            assert_eq!(b.respawn(game), Err(RespawnRefusal::Alive));
            assert_eq!(heard.lock().unwrap().len(), 1, "not respawned twice");
        });
    });
}

/// `Alive` is health above 0 and the game not won: a player who has won the game is respawned
/// on the button even alive (the request is taken on the next server tick, as the client's own
/// is), and a dead one is still respawned at once.
#[test]
#[ignore = "daily"]
fn a_player_who_won_the_game_is_respawned_alive_as_the_button_does() {
    with_guests(|game| {
        let b = game.players().server_players()[1];
        game.lan
            .as_mut()
            .unwrap()
            .play
            .entry(13)
            .or_default()
            .won_game = true;
        assert!(!is_dead(game, 13));
        assert_eq!(b.respawn(game), Ok(()));
        assert!(
            game.lan.as_ref().unwrap().play[&13].respawn_requested,
            "the poem's exit is requested, as the packet does"
        );
        assert!(!is_dead(game, 13), "still alive, not respawned in place");
        game.lan
            .as_mut()
            .unwrap()
            .play
            .get_mut(&13)
            .unwrap()
            .won_game = false;
        game.lan
            .as_mut()
            .unwrap()
            .play
            .get_mut(&13)
            .unwrap()
            .respawn_requested = false;
        assert_eq!(b.respawn(game), Err(RespawnRefusal::Alive));
    });
}

/// Each refusal has its own name and changes nothing: a living player, a call inside another
/// player's hook, a handle whose player has left, a fake player, the world's own player, and a
/// client.
#[test]
#[ignore = "daily"]
fn each_respawn_refusal_has_its_own_name_and_changes_nothing() {
    use horizon_client::motion::PlayerMotion;
    use horizon_core::geometry::Vec3;
    with_guests(|game| {
        let (a, b) = {
            let players = game.players().server_players();
            (players[0], players[1])
        };
        // Alive.
        assert_eq!(b.respawn(game), Err(RespawnRefusal::Alive));
        assert!(!is_dead(game, 13));

        // Inside another player's hook (their state swapped in), though b is dead.
        set_dead(game, 13, true);
        game.as_guest(12, |game| {
            assert_eq!(b.respawn(game), Err(RespawnRefusal::InPlayerHook));
        })
        .unwrap();
        assert!(is_dead(game, 13), "still dead");

        // A fake player has no connection.
        let level = game.level("minecraft:overworld").unwrap();
        let fake = game.fake_player(level, 100, "robot").unwrap();
        assert_eq!(fake.respawn(game), Err(RespawnRefusal::NoConnection));

        // The world's own player has none either.
        {
            let _serving = super::integrated::tests::install_serving(game, false);
            game.level.player_uuid = Some([0, 0, 0, 99]);
            let mut body = PlayerMotion::new(game.motion, Vec3::new(0.0, 64.0, 0.0)).unwrap();
            body.entity_id = 99;
            game.player = Some(body);
            let owner = game.player(99).unwrap();
            assert_eq!(owner.respawn(game), Err(RespawnRefusal::NoConnection));
            game.player = None;
            game.level.player_uuid = None;
        }

        // A handle of another world epoch, and one whose player has left.
        let stale = mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(
                mod_api::WorldEpoch::new(b.entity().epoch().get() + 1),
                b.entity().id(),
            ),
            b.uuid(),
        );
        assert_eq!(stale.respawn(game), Err(RespawnRefusal::Stale));
        let hub = &mut game.lan.as_mut().unwrap().hub;
        let index = hub.guests.connection_index(13).unwrap();
        hub.guests.remove(index);
        assert_eq!(b.respawn(game), Err(RespawnRefusal::Stale));

        // A client.
        game.options.execution_role = crate::game::execution_role::ExecutionRole::Client;
        let (client, _server) = horizon_protocol::connection::Pipe::pair();
        game.connect_local(client, "RespawnRefusalTest").unwrap();
        assert_eq!(a.respawn(game), Err(RespawnRefusal::Client));
    });
}
