//! `player_disconnect` against real LAN guests: the reason reaches the right one only.

use super::native_guest_turn_tests::with_guests;
use horizon_client::lan::{Out, PlayerSender};
use horizon_core::nbt::Tag;
use horizon_protocol::text::translatable;
use mod_api::{PlayerRef, Text, prelude::*};
use std::sync::{Arc, mpsc::Receiver};

fn left_reason(game: &super::Game<'_>, conn: u32) -> String {
    let hub = &game.lan.as_ref().unwrap().hub;
    hub.guest(conn).unwrap().left_reason.clone()
}

/// Gives guest `conn` a test channel, to read what its connection is sent.
fn listen(game: &mut super::Game<'_>, conn: u32) -> Receiver<Out> {
    let (tx, rx) = std::sync::mpsc::channel();
    let hub = &mut game.lan.as_mut().unwrap().hub;
    hub.update_guest(conn, |g| g.sender = PlayerSender::new(tx, Arc::default()));
    rx
}

/// The disconnect reasons a guest's connection was sent.
fn disconnects(rx: &Receiver<Out>) -> Vec<Tag> {
    rx.try_iter()
        .filter_map(|out| match out {
            Out::Disconnect(reason) => Some(reason),
            _ => None,
        })
        .collect()
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn disconnect_sends_the_reason_to_that_guest_only() {
    with_guests(|game| {
        let players = game.players().server_players();
        let (a, b) = (players[0], players[1]);
        let (to_a, to_b) = (listen(game, 12), listen(game, 13));
        assert!(
            a.disconnect(game, Text::literal("Server restarting"))
                .unwrap()
        );
        assert_eq!(left_reason(game, 12), "Server restarting");
        assert_eq!(disconnects(&to_a).len(), 1);
        assert_eq!(left_reason(game, 13), "");
        assert!(disconnects(&to_b).is_empty());
        assert!(
            b.disconnect(
                game,
                Text::translate("multiplayer.disconnect.server_shutdown", vec![])
            )
            .unwrap()
        );
        // The client is sent the key to translate; with no server language the log gives it.
        assert_eq!(
            disconnects(&to_b),
            [translatable("multiplayer.disconnect.server_shutdown", &[])]
        );
        assert_eq!(
            left_reason(game, 13),
            "multiplayer.disconnect.server_shutdown"
        );
        assert_eq!(left_reason(game, 12), "Server restarting");
        assert!(disconnects(&to_a).is_empty());
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn disconnect_reaches_acting_and_displaced_players() {
    with_guests(|game| {
        let players = game.players().server_players();
        let (a, b) = (players[0], players[1]);
        let (to_a, to_b) = (listen(game, 12), listen(game, 13));
        // In guest12's turn, guest13 is displaced: their record is reached, not their body.
        let kicked = game
            .as_guest(12, |game| {
                assert_eq!(game.players().acting_player(), Some(a));
                b.disconnect(game, Text::literal("Displaced by guest12"))
                    .unwrap()
            })
            .unwrap();
        assert!(kicked);
        assert_eq!(left_reason(game, 13), "Displaced by guest12");
        assert_eq!(disconnects(&to_b).len(), 1);
        assert_eq!(left_reason(game, 12), "");
        // The acting player, inside their own turn.
        let kicked = game
            .as_guest(12, |game| {
                a.disconnect(game, Text::literal("Bye from your own hook"))
                    .unwrap()
            })
            .unwrap();
        assert!(kicked);
        assert_eq!(left_reason(game, 12), "Bye from your own hook");
        assert_eq!(disconnects(&to_a).len(), 1);
    });
}

/// A disconnect is pending until the connection is removed: a second call answers false,
/// sends nothing and keeps the first reason.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_second_disconnect_while_pending_is_refused() {
    with_guests(|game| {
        let a = game.players().server_players()[0];
        let to_a = listen(game, 12);
        assert!(a.disconnect(game, Text::literal("First reason")).unwrap());
        assert!(!a.disconnect(game, Text::literal("Second reason")).unwrap());
        let refused = game
            .as_guest(12, |game| {
                a.disconnect(game, Text::literal("Third reason")).unwrap()
            })
            .unwrap();
        assert!(!refused);
        assert_eq!(left_reason(game, 12), "First reason");
        let hub = &game.lan.as_ref().unwrap().hub;
        assert!(hub.guest(12).unwrap().disconnect_pending);
        assert!(!hub.guest(13).unwrap().disconnect_pending);
        let sent = disconnects(&to_a);
        assert_eq!(sent.len(), 1);
        assert_eq!(
            horizon_protocol::text::plain(&sent[0], &Default::default()),
            "First reason"
        );
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn disconnect_refuses_the_owner_stale_and_offline_players() {
    with_guests(|game| {
        let players = game.players().server_players();
        let (a, b) = (players[0], players[1]);
        game.lan.as_mut().unwrap().hub.singleplayer_owner = Some(12);
        assert!(!a.disconnect(game, Text::literal("owner")).unwrap());
        game.lan.as_mut().unwrap().hub.singleplayer_owner = None;

        let stale = mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(
                mod_api::WorldEpoch::new(a.entity().epoch().get() + 1),
                a.entity().id(),
            ),
            a.uuid(),
        );
        let wrong_uuid = mod_api::__private::game::player_ref(a.entity(), 99);
        let wrong_entity = mod_api::__private::game::player_ref(b.entity(), a.uuid());
        for invalid in [stale, wrong_uuid, wrong_entity] {
            assert!(!invalid.disconnect(game, Text::literal("stale")).unwrap());
        }

        let mut too_deep = Text::literal("leaf");
        for _ in 1..513 {
            too_deep = Text::literal("parent").append(too_deep);
        }
        assert!(matches!(
            a.disconnect(game, too_deep),
            Err(mod_api::text::TextFormError::TooDeep)
        ));
        assert_eq!(left_reason(game, 12), "");
        assert_eq!(left_reason(game, 13), "");

        game.lan.as_mut().unwrap().hub.guests.clear();
        assert!(!a.disconnect(game, Text::literal("offline")).unwrap());
    });
}

/// A wasm listener reaches the same method through its generated forwarder: the reason
/// crosses the wire and the answers come back encoded (checked inside the guest, which
/// traps on a mismatch and then leaves the tick phase unanswered).
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn wasm_disconnect_crosses_the_generated_forwarder() {
    crate::game::native_entity_refs::tests::run_on_guest_stack(|| {
        with_guests(|game| {
            use mod_api::wire::{Reader, Writer};
            let players = game.players().server_players();
            let a = players[0];
            let stale = mod_api::__private::game::player_ref(
                mod_api::EntityRef::new(
                    mod_api::WorldEpoch::new(a.entity().epoch().get() + 1),
                    a.entity().id(),
                ),
                a.uuid(),
            );
            let input = |player: PlayerRef, reason: &str| {
                let mut w = Writer::new();
                w.put(&player);
                w.put(&Text::literal(reason));
                w
            };
            let answer = |kicked: bool| {
                let mut w = Writer::new();
                w.put(&Ok::<bool, mod_api::text::TextFormError>(kicked));
                w
            };
            let escape = |bytes: &[u8]| {
                bytes
                    .iter()
                    .map(|b| format!("\\{b:02x}"))
                    .collect::<String>()
            };
            let check = |expected: Writer, at: usize| {
                expected
                    .bytes()
                    .iter()
                    .enumerate()
                    .map(|(i, byte)| {
                        format!(
                            "i32.const {} i32.load8_u i32.const {byte} i32.ne if unreachable end ",
                            at + i
                        )
                    })
                    .collect::<String>()
            };
            let id =
                horizon_mod_wasm::fixture::import_id("BackendMut", "player_disconnect").unwrap();
            let (kick, refused) = (input(a, "Kicked by wasm"), input(stale, "stale"));
            let mut reply = Writer::new();
            reply.put(&mod_api::ServerTickPhase::ServerTickEnd {
                world_running: true,
            });
            let bytes = wat::parse_str(format!(r#"(module
              (@custom "dylink.0" "\01\04\00\00\00\00")
              (import "env" "memory" (memory 1))
              (import "env" "__indirect_function_table" (table 1 funcref))
              (import "horizon" "host" (func $host (param i32 i32 i32 i32 i32) (result i32)))
              (import "horizon" "host_reply" (func (param i32)))
              (func (export "horizon_layout") (result i64) i64.const {layout})
              (data (i32.const 4096) "{kick}")
              (data (i32.const 8192) "{refused}")
              (data (i32.const 16384) "{reply}")
              (func (export "horizon_in") (param i32) (result i32) i32.const 512)
              (func (export "horizon_hook") (param i32) (result i64)
                i32.const {id} i32.const 4096 i32.const {kick_len} i32.const 32768 i32.const 4096 call $host drop
                {kicked}
                i32.const {id} i32.const 8192 i32.const {refused_len} i32.const 36864 i32.const 4096 call $host drop
                {not_kicked}
                i64.const {reply_at}))"#,
                layout = *mod_api::wire::LAYOUT,
                kick = escape(kick.bytes()),
                refused = escape(refused.bytes()),
                reply = escape(reply.bytes()),
                kick_len = kick.bytes().len(),
                refused_len = refused.bytes().len(),
                kicked = check(answer(true), 32768),
                not_kicked = check(answer(false), 36864),
                reply_at = (16384u64 << 32) | reply.bytes().len() as u64,
            ))
            .unwrap();
            let module = horizon_mod_wasm::fixture::module(&bytes).unwrap();
            let mut context = Writer::new();
            context.put(&mod_api::Authority::DedicatedServer);
            context.put(&a.entity().epoch());
            context.put(&mod_api::TickId::new(1));
            let context: mod_api::LifecycleContext = Reader::new(context.bytes()).take().unwrap();
            let mut tick = mod_api::ServerTickEvent {
                context: &context,
                phase: mod_api::ServerTickPhase::ServerTickStart {
                    world_running: true,
                },
            };
            mod_api::Hooks::server_tick(&module, game, &mut tick);
            assert!(matches!(
                tick.phase,
                mod_api::ServerTickPhase::ServerTickEnd {
                    world_running: true
                }
            ));
            assert_eq!(left_reason(game, 12), "Kicked by wasm");
            assert_eq!(left_reason(game, 13), "");
        });
    });
}
