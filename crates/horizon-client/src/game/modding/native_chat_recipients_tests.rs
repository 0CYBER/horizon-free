//! `chat_recipient`: asked once for each placed receiver before a public line is sent, and a
//! receiver it hides is sent nothing while the others still get the line.
use super::native_entity_refs::tests::with_game;
use horizon_client::lan::hub::chat::ChatMessage;
use horizon_client::lan::{Joined, NetConfig, Out, PlayerSender, guest::Guest};
use horizon_client::text::Text;
use horizon_protocol::packets::{configuration::ClientInformation, play};
use mod_api::{ChatRecipientEvent, Flow, Hooks, ServerHost};
use std::sync::{Arc, Mutex, atomic::AtomicI32, mpsc};

type Asked = Arc<Mutex<Vec<(u128, String, String, u128)>>>;

/// Hides every line that says "secret" from guest 13, and records what it is asked.
struct Ignorer(Asked);

#[mod_api::__subscriptions]
impl Hooks for Ignorer {
    fn chat_recipient(
        &self,
        _host: &mut dyn ServerHost,
        event: &mut ChatRecipientEvent<'_>,
    ) -> Flow {
        self.0.lock().unwrap().push((
            event.sender,
            event.sender_name.to_owned(),
            event.message.to_owned(),
            event.recipient,
        ));
        if event.message.contains("secret") && event.recipient == 13 {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

/// Guests 12 (the sender) and 13 on a LAN host, with what each is sent.
fn two_guests(game: &mut super::Game<'_>) -> (mpsc::Receiver<Out>, mpsc::Receiver<Out>) {
    let net = NetConfig {
        listen: false,
        announce: false,
        lod_distance: 0,
        ..NetConfig::lan(0, String::new(), 3)
    };
    let config = horizon_client::lan::hub::HubConfig {
        default_game_mode: 0,
        guest_commands: true,
        force_game_mode: false,
        hardcore: false,
        view_distance: 2,
        simulation_distance: 2,
        dimension: game.dim.id().to_owned(),
        allow_flight: true,
        seed: 0,
        max_players: 3,
        integrated: true,
        levels: game.level_shapes,
        level_seeds: game.levels.runtime.seeds(),
    };
    let settings = super::lan::LanSettings {
        port: 0,
        guest_commands: true,
        force_game_mode: false,
    };
    game.lan_host(settings, net, config).unwrap();
    let mut join = |conn: u32| {
        let (tx, rx) = mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn,
                uuid: u128::from(conn),
                name: format!("guest{conn}"),
                properties: Vec::new(),
                info: ClientInformation::new(8),
                brand: String::new(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:1".parse().unwrap(),
                expected: false,
            },
            conn as i32,
            [0.5, 64.0, 0.5],
            2,
        );
        guest.placed = true;
        guest.permission = 4;
        game.lan.as_mut().unwrap().hub.guests.push(guest);
        rx
    };
    (join(12), join(13))
}

/// How many public chat lines (signed or disguised) a guest was sent since the last call.
fn lines(rx: &mpsc::Receiver<Out>) -> usize {
    rx.try_iter()
        .flat_map(|out| match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            Out::Disconnect(_) => Vec::new(),
        })
        .filter(|body| {
            let mut reader = horizon_protocol::buf::Reader::new(body);
            let id = reader.varint().unwrap();
            matches!(
                play::Clientbound::decode(id, reader.rest()),
                Ok(play::Clientbound::PlayerChat(_) | play::Clientbound::DisguisedChat(_))
            )
        })
        .count()
}

fn signed(content: &str) -> ChatMessage {
    ChatMessage {
        sender: 12,
        index: 0,
        signature: None,
        content: content.to_owned(),
        timestamp: 0,
        salt: 0,
        last_seen: Vec::new(),
        unsigned_content: None,
    }
}

/// A player's line: asked once per receiver, the sender included, with the text players see; the
/// hidden receiver is sent nothing and the others still are; a line nobody hides reaches all.
#[test]
#[ignore = "daily"]
fn a_chat_line_hidden_from_one_receiver_reaches_the_others() {
    let asked = Asked::default();
    let mods = vec![mod_api::ModEntry::new(
        "ignorer",
        Box::new(Ignorer(asked.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let (sender, other) = two_guests(game);
            lines(&sender);
            lines(&other);
            let bound = play::ChatTypeBound {
                chat_type: 0,
                name: horizon_client::lan::data::component_nbt(
                    &Text::literal("guest12").component(),
                ),
                target: None,
            };
            let say = |game: &mut super::Game<'_>, content: &str| {
                let line = format!("<guest12> {content}");
                game.lan_broadcast_chat(&signed(content), &bound, line, "guest12", content)
                    .unwrap();
            };
            say(game, "a secret");
            assert_eq!(lines(&sender), 1, "the sender still sees it");
            assert_eq!(lines(&other), 0, "the hidden receiver is not sent it");
            assert_eq!(
                std::mem::take(&mut *asked.lock().unwrap()),
                [
                    (12, "guest12".to_owned(), "a secret".to_owned(), 12),
                    (12, "guest12".to_owned(), "a secret".to_owned(), 13),
                ],
                "asked once per receiver, the sender included"
            );
            say(game, "hello");
            assert_eq!(lines(&sender), 1);
            assert_eq!(lines(&other), 1, "a line no mod hides reaches everyone");
        });
    });
}

/// `/say` and `/me` are public too, signed or not: the hidden receiver is not sent them either,
/// and `/msg` is not asked about at all.
#[test]
#[ignore = "daily"]
fn say_and_me_are_asked_per_receiver_and_a_whisper_is_not() {
    let asked = Asked::default();
    let mods = vec![mod_api::ModEntry::new(
        "ignorer",
        Box::new(Ignorer(asked.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let (sender, other) = two_guests(game);
            lines(&sender);
            lines(&other);
            assert!(game.guest_message_command(12, "say a secret", None));
            assert_eq!(lines(&sender), 1);
            assert_eq!(lines(&other), 0, "/say, unsigned: hidden from guest 13");
            assert!(game.guest_message_command(12, "me waves", None));
            assert_eq!(lines(&sender), 1);
            assert_eq!(lines(&other), 1, "/me: nothing hides it");
            assert_eq!(asked.lock().unwrap().len(), 4, "two lines, two receivers");
            asked.lock().unwrap().clear();
            assert!(game.guest_message_command(12, "msg guest13 a secret", None));
            assert!(asked.lock().unwrap().is_empty(), "a whisper is not public");
            assert_eq!(lines(&other), 1, "and it reaches its target");
        });
    });
}
