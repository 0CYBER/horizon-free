//! `player_disconnect` from a play payload handler, over real TCP: the reason reaches the
//! right client once, a second call (from `player_kicked`) is refused, and the hub's poll
//! tells `player_logged_out` exactly once per player.
use super::sender_tests::connect_peer;
use horizon_protocol::{
    connection::Connection,
    packets::{configuration as cfg, play},
};
use mod_api::{
    HandlerThread, Hooks, IPayloadContext, IPayloadHandler, ModEntry, PayloadBytes, PayloadId,
    PlayerKickedEvent, PlayerLoggedOutEvent, Registrar, ServerHost, Text, text::TextFormError,
};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const NAME: &str = "game::payload_transport::disconnect_tests::a_payload_handler_disconnects_once_and_the_poll_logs_out_once";
const CHANNEL: &str = "disconnect_probe:kick";
const PLAYERS: [&str; 2] = ["SenderOne", "SenderTwo"];
const REASONS: [&str; 2] = [
    "SenderOne left by its own payload",
    "SenderTwo kicked by SenderOne",
];

/// One handled payload: (sender, acting player, target, answer). A main-thread payload
/// handler runs outside any guest turn: no acting player, both reached as connected records.
type Handled = (u128, Option<u128>, u128, Result<bool, TextFormError>);

#[derive(Default)]
struct Probe {
    handled: Mutex<Vec<Handled>>,
    kicked: Mutex<Vec<(u128, String)>>,
    second: Mutex<Vec<Option<Result<bool, TextFormError>>>>,
    logged_out: Mutex<Vec<u128>>,
}
impl IPayloadHandler for Probe {
    /// The payload's one byte names the player to disconnect.
    fn handle(
        &self,
        bytes: PayloadBytes,
        context: &mut dyn IPayloadContext,
    ) -> Result<(), mod_api::PayloadHandleError> {
        let sender = context.player().unwrap();
        let host = context.server_host().unwrap();
        let index = usize::from(bytes.0[0]);
        let target = host.players().player_by_name(PLAYERS[index]).unwrap();
        let acting = host.players().acting_player().map(|p| p.uuid());
        let answer = target.disconnect(host, Text::literal(REASONS[index]));
        self.handled
            .lock()
            .unwrap()
            .push((sender.uuid(), acting, target.uuid(), answer));
        Ok(())
    }
}
struct Registration(Arc<Probe>);
#[mod_api::__subscriptions]
impl Hooks for Registration {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        registrar
            .payload_handlers()
            .registrar("1")
            .executes_on(HandlerThread::Main)
            .play_bidirectional(
                PayloadId(CHANNEL.into()),
                self.0.clone(),
                Some(self.0.clone()),
            );
    }
    fn player_kicked(&self, host: &mut dyn ServerHost, event: &mut PlayerKickedEvent<'_>) {
        let second = host
            .player(event.player)
            .map(|player| player.disconnect(host, Text::literal("Second reason")));
        self.0.second.lock().unwrap().push(second);
        self.0
            .kicked
            .lock()
            .unwrap()
            .push((event.player, event.reason.to_string()));
    }
    fn player_logged_out(&self, _: &mut dyn ServerHost, event: &mut PlayerLoggedOutEvent<'_>) {
        self.0.logged_out.lock().unwrap().push(event.player);
    }
}

/// The disconnect reasons a peer reads (as plain text) until its connection closes.
fn reasons_until_closed(c: &mut Connection) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut reasons = Vec::new();
    while let Ok((id, body)) = c.recv() {
        assert!(Instant::now() < deadline, "the server never closed it");
        match play::Clientbound::decode(id, &body) {
            Ok(play::Clientbound::Disconnect(p)) => reasons.push(horizon_protocol::text::plain(
                &p.reason,
                &Default::default(),
            )),
            Ok(play::Clientbound::KeepAlive(p)) => {
                drop(c.send(&play::Serverbound::KeepAlive(p).encode()))
            }
            _ => {}
        }
    }
    reasons
}

fn wait_for(probe: &Probe, logged_out: usize) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while probe.logged_out.lock().unwrap().len() < logged_out {
        assert!(Instant::now() < deadline, "player_logged_out never came");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn send(c: &mut Connection, target: u8) {
    c.send(
        &play::Serverbound::CustomPayload(cfg::CustomPayload {
            channel: CHANNEL.into(),
            data: vec![target],
        })
        .encode(),
    )
    .unwrap();
}

#[test]
#[ignore = "needs HORIZON_TEST_RESOURCES; starts an integrated server and two physical TCP clients"]
fn a_payload_handler_disconnects_once_and_the_poll_logs_out_once() {
    super::tests::require_resources();
    if super::super::native_entity_refs::tests::run_fresh_cases(
        NAME,
        "HORIZON_DISCONNECT_TRANSPORT_CHILD",
        &["two_players"],
    ) {
        return;
    }
    let probe = Arc::new(Probe::default());
    let mods = vec![ModEntry::new(
        "disconnect_probe",
        Box::new(Registration(probe.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            game.settings.render_distance = 2;
            game.settings.simulation_distance = 2;
            let options = horizon_client::title::WorldOptions {
                world_type: horizon_client::save::WorldPreset::Flat,
                ..Default::default()
            };
            game.open_integrated(
                "disconnect_transport",
                Some("Disconnect transport"),
                Some(1),
                true,
                Some(&options),
            )
            .unwrap();
            let deadline = Instant::now() + Duration::from_secs(60);
            while !game
                .remote
                .as_ref()
                .is_some_and(|r| r.logged_in && r.joined)
            {
                game.poll_remote().unwrap();
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            }
            let port = game
                .integrated
                .as_ref()
                .unwrap()
                .call(|server| {
                    server.publish(super::super::lan::LanSettings {
                        port: 0,
                        guest_commands: false,
                        force_game_mode: false,
                    })
                })
                .expect("integrated server answered publication request")
                .unwrap();
            let [mut one, mut two] = PLAYERS.map(|name| connect_peer(port, name));
            let [uuid_one, uuid_two] = PLAYERS.map(horizon_protocol::crypto::offline_uuid);

            // SenderOne's payload disconnects SenderTwo.
            send(&mut one, 1);
            assert_eq!(reasons_until_closed(&mut two), [REASONS[1]]);
            wait_for(&probe, 1);
            // Then SenderOne's own payload disconnects SenderOne.
            send(&mut one, 0);
            assert_eq!(reasons_until_closed(&mut one), [REASONS[0]]);
            wait_for(&probe, 2);
            // Another second of server ticks: nothing more is heard.
            std::thread::sleep(Duration::from_secs(1));

            assert_eq!(
                *probe.handled.lock().unwrap(),
                [
                    (uuid_one, None, uuid_two, Ok(true)),
                    (uuid_one, None, uuid_one, Ok(true)),
                ]
            );
            assert_eq!(
                *probe.kicked.lock().unwrap(),
                [
                    (uuid_two, REASONS[1].to_owned()),
                    (uuid_one, REASONS[0].to_owned()),
                ]
            );
            assert_eq!(
                *probe.second.lock().unwrap(),
                [Some(Ok(false)), Some(Ok(false))]
            );
            assert_eq!(*probe.logged_out.lock().unwrap(), [uuid_two, uuid_one]);
            game.leave_server(None).unwrap();
        });
    });
}
