//! A fake player's kick has the same leave boundary and notifications as a closed guest.

use super::native_guest_turn_tests::with_guests;
use horizon_client::lan::{Out, PlayerSender};
use horizon_core::nbt::Tag;
use horizon_protocol::{buf::Reader, packets::play::Clientbound};
use mod_api::{
    Hooks, ModEntry, Mods, PlayerKickedEvent, PlayerLoggedOutEvent, PlayerRef, ServerHost, Text,
};
use std::sync::{Arc, Mutex, mpsc::Receiver};

#[derive(Default)]
struct Notices {
    player: Mutex<Option<PlayerRef>>,
    kicked: Mutex<Vec<String>>,
    logged_out: Mutex<Vec<u128>>,
}

struct Owner(Arc<Notices>);
#[mod_api::__subscriptions]
impl Hooks for Owner {
    fn player_kicked(&self, host: &mut dyn ServerHost, event: &mut PlayerKickedEvent<'_>) {
        let player = *self.0.player.lock().unwrap();
        let Some(player) = player.filter(|player| player.uuid() == event.player) else {
            return;
        };
        self.0.kicked.lock().unwrap().push(event.reason.to_owned());
        assert!(
            !player
                .disconnect(host, Text::literal("Replacement reason"))
                .unwrap()
        );
    }

    fn player_logged_out(&self, _: &mut dyn ServerHost, event: &mut PlayerLoggedOutEvent<'_>) {
        if event.player == 100 {
            self.0.logged_out.lock().unwrap().push(event.player);
        }
    }
}

fn listen(game: &mut super::Game<'_>, conn: u32) -> Receiver<Out> {
    let (tx, rx) = std::sync::mpsc::channel();
    game.lan.as_mut().unwrap().hub.update_guest(conn, |guest| {
        guest.sender = PlayerSender::new(tx, Arc::default())
    });
    rx
}

fn leave_lines(rx: &Receiver<Out>) -> Vec<String> {
    let lang = [("multiplayer.player.left".into(), "%s left the game".into())].into();
    rx.try_iter()
        .flat_map(|out| match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            // /kick @a also disconnects the network guests; their channels do not close.
            Out::Disconnect(_) => Vec::new(),
        })
        .filter_map(|body| {
            let mut reader = Reader::new(&body);
            let id = reader.varint().unwrap();
            let Clientbound::SystemChat(chat) = Clientbound::decode(id, reader.rest()).ok()? else {
                return None;
            };
            let Tag::Compound(component) = &chat.content else {
                return None;
            };
            let Some(Tag::String(key)) = component.get(&"translate".into()) else {
                return None;
            };
            (key.to_string_lossy() == "multiplayer.player.left")
                .then(|| horizon_protocol::text::plain(&chat.content, &lang))
        })
        .collect()
}

#[test]
#[ignore = "daily"]
fn fake_kicks_leave_once_with_the_first_reason() {
    for path in ["mod", "acting", "name", "selector"] {
        let notices = Arc::new(Notices::default());
        let mods: Mods = vec![ModEntry::new(
            "fake_owner",
            Box::new(Owner(notices.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_guests(|game| {
                let level = game.level("minecraft:overworld").unwrap();
                let fake = game.fake_player(level, 100, "robot").unwrap();
                *notices.player.lock().unwrap() = Some(fake);
                let conn = game
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guests
                    .iter()
                    .find(|guest| guest.uuid == 100)
                    .unwrap()
                    .conn;
                let observers = [listen(game, 12), listen(game, 13)];
                match path {
                    "mod" => assert!(
                        fake.disconnect(game, Text::literal("First reason"))
                            .unwrap()
                    ),
                    "acting" => assert!(
                        game.as_guest(conn, |game| {
                            fake.disconnect(game, Text::literal("First reason"))
                                .unwrap()
                        })
                        .unwrap()
                    ),
                    "name" => assert!(game.native_run_command("kick robot First reason")),
                    "selector" => assert!(game.native_run_command("kick @a First reason")),
                    _ => unreachable!(),
                }
                assert!(
                    !fake
                        .disconnect(game, Text::literal("Second reason"))
                        .unwrap()
                );
                let guest = game.lan.as_ref().unwrap().hub.guest(conn).unwrap();
                assert!(guest.disconnect_pending);
                assert_eq!(guest.left_reason, "First reason");
                assert_eq!(game.player_is_fake(fake), Some(true));
                assert!(notices.kicked.lock().unwrap().is_empty());
                assert!(notices.logged_out.lock().unwrap().is_empty());
                for observer in &observers {
                    assert!(leave_lines(observer).is_empty());
                }

                game.level_pass = Default::default();
                game.tick_lan_packets();
                // Packet draining holds sends until the full tick's flush boundary.
                horizon_client::lan::flush_all(
                    game.lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guests
                        .iter()
                        .map(|g| &g.sender),
                );
                let host = game.lan.as_ref().unwrap();
                assert!(host.hub.guest(conn).is_none());
                assert!(host.hub.guests.iter().all(|guest| guest.uuid != 100));
                assert!(!host.play.contains_key(&conn));
                assert_eq!(game.player_is_fake(fake), None);
                assert_eq!(*notices.kicked.lock().unwrap(), ["First reason"]);
                assert_eq!(*notices.logged_out.lock().unwrap(), [100]);
                for observer in &observers {
                    let lines = leave_lines(observer);
                    assert_eq!(lines.len(), 1, "{path}: {lines:?}");
                    assert_eq!(lines[0], "robot left the game");
                }

                assert!(
                    !fake
                        .disconnect(game, Text::literal("After removal"))
                        .unwrap()
                );
                game.level_pass = Default::default();
                game.tick_lan_packets();
                // Packet draining holds sends until the full tick's flush boundary.
                horizon_client::lan::flush_all(
                    game.lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guests
                        .iter()
                        .map(|g| &g.sender),
                );
                assert_eq!(*notices.kicked.lock().unwrap(), ["First reason"]);
                assert_eq!(*notices.logged_out.lock().unwrap(), [100]);
                for observer in &observers {
                    assert!(leave_lines(observer).is_empty());
                }
            });
        });
    }
}
