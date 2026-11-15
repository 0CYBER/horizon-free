//! Typed payloads (arismoko/horizon#813 step 4b-1) on the real routes: the integrated server over memory and a
//! published server over TCP. The values are decoded by the typed adapter and encoded by the
//! token sends; the expected bodies are written out by hand.
use horizon_protocol::packets::{configuration as cfg, play};
use mod_api::{
    Hooks, IPayloadContext, ModEntry, PayloadAccepted, PayloadCodec, PayloadDecodeError, PayloadId,
    PayloadReader, PayloadSendError, PayloadWriter, PlayerRef, Registrar, ToClient, ToServer,
};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

const NAME: &str = "game::payload_transport::typed_tests::typed_payloads_follow_the_raw_routes";
const UP: &str = "typed_probe:up";
const OTHER: &str = "typed_probe:other";
const DOWN: &str = "typed_probe:down";
const STAGED: f32 = 3.0;

static ENCODES: AtomicUsize = AtomicUsize::new(0);
static DECODES: AtomicUsize = AtomicUsize::new(0);

/// A VarInt that counts its codec calls.
#[derive(Debug, PartialEq)]
struct Counted(i32);
impl PayloadCodec for Counted {
    fn encode(&self, out: &mut PayloadWriter) {
        ENCODES.fetch_add(1, Ordering::SeqCst);
        out.var_int(self.0);
    }
    fn decode(input: &mut PayloadReader<'_>) -> Result<Self, PayloadDecodeError> {
        DECODES.fetch_add(1, Ordering::SeqCst);
        input.var_int().map(Self)
    }
    fn schema(_: &mut Vec<std::any::TypeId>) -> u64 {
        mod_api::payload_schema_leaf("var_int")
    }
}

#[derive(Debug, PartialEq, PayloadCodec)]
struct Probe {
    counter: Counted,
    labels: Vec<String>,
    chain: Chain,
}
#[derive(Debug, PartialEq, PayloadCodec)]
struct Chain(Option<Box<Chain>>);

/// The same ids' value with another shape.
#[derive(Debug, PartialEq, PayloadCodec)]
struct Other(i32);

fn probe(counter: i32) -> Probe {
    Probe {
        counter: Counted(counter),
        labels: Vec::new(),
        chain: Chain(None),
    }
}

struct Tokens {
    up: ToServer<Probe>,
    other: ToServer<Probe>,
    down: ToClient<Probe>,
}
static TOKENS: Mutex<Option<Tokens>> = Mutex::new(None);
/// What the server handlers saw: channel, counter and the first peer's health then.
static SERVER: Mutex<Vec<(&str, i32, Option<f32>)>> = Mutex::new(Vec::new());
static CLIENT: Mutex<Vec<i32>> = Mutex::new(Vec::new());
/// The first player to send 7, whose health the failing handler stages a write to.
static WITNESS: Mutex<Option<PlayerRef>> = Mutex::new(None);

fn down() -> ToClient<Probe> {
    TOKENS.lock().unwrap().as_ref().unwrap().down.clone()
}

fn witness_health(context: &mut dyn IPayloadContext) -> Option<f32> {
    let witness = (*WITNESS.lock().unwrap())?;
    let host = context.server_host().ok()?;
    host.living(witness.entity())?.health(host)
}

fn up(value: Probe, context: &mut dyn IPayloadContext) -> Result<(), String> {
    assert_eq!(std::thread::current().name(), Some("Server thread"));
    let counter = value.counter.0;
    if counter == 7 {
        let player = context.player().map_err(|e| format!("{e:?}"))?;
        WITNESS.lock().unwrap().get_or_insert(player);
    }
    let health = witness_health(context);
    SERVER.lock().unwrap().push((UP, counter, health));
    match counter {
        7 => down()
            .reply(context, &probe(8))
            .map(drop)
            .map_err(|e| format!("reply: {e:?}")),
        13 => {
            let witness = WITNESS.lock().unwrap().expect("a witness sent 7 first");
            let host = context.server_host().map_err(|e| format!("{e:?}"))?;
            let living = host.living(witness.entity()).ok_or("no witness")?;
            assert!(living.set_health(host, STAGED));
            assert_eq!(
                down().reply(context, &probe(14)),
                Ok(PayloadAccepted::Queued)
            );
            panic!("typed handler panics after a staged write and a staged reply");
        }
        _ => Ok(()),
    }
}

/// A healthy observer on its own callback: records the witness's health and echoes the counter.
fn other(value: Probe, context: &mut dyn IPayloadContext) -> Result<(), String> {
    let counter = value.counter.0;
    let health = witness_health(context);
    SERVER.lock().unwrap().push((OTHER, counter, health));
    down()
        .reply(context, &probe(counter))
        .map(drop)
        .map_err(|e| format!("reply: {e:?}"))
}

fn client(value: Probe, _: &mut dyn IPayloadContext) -> Result<(), String> {
    CLIENT.lock().unwrap().push(value.counter.0);
    Ok(())
}

struct Registration;
#[mod_api::__subscriptions]
impl Hooks for Registration {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let mut payloads = registrar.payload_handlers();
        let mut view = payloads.registrar("1");
        let tokens = Tokens {
            up: view.play_to_server_typed(PayloadId(UP.into()), up),
            other: view.play_to_server_typed(PayloadId(OTHER.into()), other),
            down: view.play_to_client_typed(PayloadId(DOWN.into()), client),
        };
        *TOKENS.lock().unwrap() = Some(tokens);
    }
}

#[test]
#[ignore = "daily"]
fn typed_payloads_follow_the_raw_routes() {
    super::tests::require_resources();
    if super::super::native_entity_refs::tests::run_fresh_cases(
        NAME,
        "HORIZON_TYPED_TRANSPORT_CHILD",
        &["integrated", "lan"],
    ) {
        return;
    }
    let case = std::env::var("HORIZON_TYPED_TRANSPORT_CHILD").unwrap();
    let mods = vec![ModEntry::new("typed_probe", Box::new(Registration))].into();
    mod_platform::testing::with_mods(mods, || {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            game.settings.render_distance = 2;
            game.settings.simulation_distance = 2;
            let options = horizon_client::title::WorldOptions {
                world_type: horizon_client::save::WorldPreset::Flat,
                ..Default::default()
            };
            game.open_integrated("typed", Some("Typed"), Some(1), true, Some(&options))
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(60);
            while !game
                .remote
                .as_ref()
                .is_some_and(|r| r.logged_in && r.joined)
            {
                game.poll_remote().unwrap();
                assert!(Instant::now() < deadline, "integrated play not ready");
                std::thread::yield_now();
            }
            match case.as_str() {
                "integrated" => integrated(game),
                "lan" => lan(game),
                other => panic!("unknown case {other}"),
            }
            game.leave_server(None).unwrap();
        });
    });
}

fn integrated(game: &mut super::Game<'_>) {
    // A declared typed payload costs nothing while no message is sent.
    let start = game.world.game_time;
    let deadline = Instant::now() + Duration::from_secs(60);
    while game.world.game_time < start + 100 {
        game.poll_remote().unwrap();
        assert!(Instant::now() < deadline, "100 server ticks");
        std::thread::yield_now();
    }
    assert_eq!(
        (
            ENCODES.load(Ordering::SeqCst),
            DECODES.load(Ordering::SeqCst)
        ),
        (0, 0)
    );
    let (up, other) = {
        let tokens = TOKENS.lock().unwrap();
        let tokens = tokens.as_ref().unwrap();
        (tokens.up.clone(), tokens.other.clone())
    };
    // Forged tokens: another slot, and the same claim read as another value type.
    let mut out = mod_api::wire::Writer::new();
    mod_api::wire::Wire::put(up.token(), &mut out);
    let bytes = out.into_bytes();
    let mut forged = mod_api::wire::Reader::decode::<mod_api::PayloadToken>(&bytes).unwrap();
    forged.slot = other.token().slot;
    let mut out = mod_api::wire::Writer::new();
    mod_api::wire::Wire::put(&forged, &mut out);
    let forged: ToServer<Probe> = mod_api::wire::Reader::decode(&out.into_bytes()).unwrap();
    let retyped: ToServer<Other> = mod_api::wire::Reader::decode(&bytes).unwrap();
    let host: &mut dyn mod_api::ClientHost = game;
    let id = PayloadId(UP.into());
    assert_eq!(
        forged.send(host, &probe(5)),
        Err(PayloadSendError::StaleToken(id.clone()))
    );
    assert_eq!(
        retyped.send(host, &Other(5)),
        Err(PayloadSendError::StaleToken(id))
    );
    assert_eq!(
        ENCODES.load(Ordering::SeqCst),
        0,
        "a stale token encodes nothing"
    );
    for (token, counter) in [(&up, 1), (&other, 2), (&up, 3), (&up, 7)] {
        assert_eq!(
            token.send(host, &probe(counter)),
            Ok(PayloadAccepted::Queued)
        );
    }
    let deadline = Instant::now() + Duration::from_secs(60);
    while CLIENT.lock().unwrap().len() < 2 {
        game.poll_remote().unwrap();
        assert!(Instant::now() < deadline, "typed reply");
        std::thread::yield_now();
    }
    let seen: Vec<_> = SERVER
        .lock()
        .unwrap()
        .iter()
        .map(|(id, counter, _)| (*id, *counter))
        .collect();
    assert_eq!(seen, [(UP, 1), (OTHER, 2), (UP, 3), (UP, 7)]);
    assert_eq!(*CLIENT.lock().unwrap(), [2, 8]);
}

fn lan(game: &mut super::Game<'_>) {
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
    let nested = |levels: usize| {
        let mut body = vec![7, 0];
        body.extend(std::iter::repeat_n(1, levels));
        body.push(0);
        body
    };
    // Each malformed body ends its own session by name, with no handler call and no reply.
    for (index, body) in [
        vec![7, 0xff, 0xff, 0xff, 0xff, 0x0f],
        vec![7, 0x81, 0x20],
        nested(33),
        vec![7, 0, 0, 0xaa],
    ]
    .into_iter()
    .enumerate()
    {
        let mut peer = super::sender_tests::connect_peer(port, &format!("Malformed{index}"));
        send(&mut peer, UP, body);
        let reason = await_disconnect(&mut peer, DOWN);
        assert!(reason.contains("typed_probe"), "{reason}");
    }
    // A clientbound-only channel sent by a client: a forged direction, no handler runs.
    let mut forged = super::sender_tests::connect_peer(port, "ForgedDirection");
    send(&mut forged, DOWN, vec![7, 0, 0]);
    await_disconnect(&mut forged, DOWN);
    assert!(SERVER.lock().unwrap().is_empty());
    assert!(CLIENT.lock().unwrap().is_empty());
    // An independent session is accepted: 7, then the order A=1, B=2, A=3, then 7 again,
    // whose reply orders them before any other connection's message.
    let mut witness = super::sender_tests::connect_peer(port, "TypedWitness");
    send(&mut witness, UP, vec![7, 0, 0]);
    assert_eq!(await_reply(&mut witness, DOWN), [8, 0, 0]);
    let full = SERVER.lock().unwrap()[0].2.expect("the witness's health");
    send(&mut witness, UP, vec![1, 0, 0]);
    send(&mut witness, OTHER, vec![2, 0, 0]);
    send(&mut witness, UP, vec![3, 0, 0]);
    send(&mut witness, UP, vec![7, 0, 0]);
    assert_eq!(await_reply(&mut witness, DOWN), [2, 0, 0]);
    assert_eq!(await_reply(&mut witness, DOWN), [8, 0, 0]);
    // A reply staged before a panic is never sent, and the session closes.
    let mut failing = super::sender_tests::connect_peer(port, "TypedPanic");
    send(&mut failing, UP, vec![13, 0, 0]);
    let reason = await_disconnect(&mut failing, DOWN);
    assert!(reason.contains("typed_probe"), "{reason}");
    // The staged write to the witness was rolled back, as a healthy callback sees it.
    send(&mut witness, OTHER, vec![4, 0, 0]);
    assert_eq!(await_reply(&mut witness, DOWN), [4, 0, 0]);
    let seen: Vec<_> = SERVER
        .lock()
        .unwrap()
        .iter()
        .map(|(id, counter, _)| (*id, *counter))
        .collect();
    assert_eq!(
        seen,
        [
            (UP, 7),
            (UP, 1),
            (OTHER, 2),
            (UP, 3),
            (UP, 7),
            (UP, 13),
            (OTHER, 4)
        ]
    );
    let last = SERVER.lock().unwrap().last().unwrap().2;
    assert_eq!(
        last,
        Some(full),
        "the failed handler's write was rolled back"
    );
    // The panicked callback is disabled: a later message on it is refused at ingress, as for
    // a disabled owner, and the session ends without running it.
    let mut later = super::sender_tests::connect_peer(port, "TypedLater");
    send(&mut later, UP, vec![7, 0, 0]);
    await_disconnect(&mut later, DOWN);
    assert_eq!(SERVER.lock().unwrap().len(), seen.len());
}

/// Send `data` on `channel` from a TCP peer in play.
pub(super) fn send(
    peer: &mut horizon_protocol::connection::Connection,
    channel: &str,
    data: Vec<u8>,
) {
    peer.send(
        &play::Serverbound::CustomPayload(cfg::CustomPayload {
            channel: channel.into(),
            data,
        })
        .encode(),
    )
    .unwrap();
}

/// The next payload on `channel`, answering keep-alives on the way.
pub(super) fn await_reply(
    peer: &mut horizon_protocol::connection::Connection,
    channel: &str,
) -> Vec<u8> {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        assert!(Instant::now() < deadline, "typed reply deadline");
        let (id, body) = peer.recv().unwrap();
        match play::Clientbound::decode(id, &body).unwrap() {
            play::Clientbound::CustomPayload(p) if p.channel == channel => return p.data,
            play::Clientbound::KeepAlive(p) => peer
                .send(&play::Serverbound::KeepAlive(p).encode())
                .unwrap(),
            play::Clientbound::Disconnect(p) => panic!("disconnected: {p:?}"),
            _ => {}
        }
    }
}

/// The session's disconnect reason, or how its stream closed; a payload on `channel` before
/// it fails the test.
pub(super) fn await_disconnect(
    peer: &mut horizon_protocol::connection::Connection,
    channel: &str,
) -> String {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        assert!(Instant::now() < deadline, "disconnect deadline");
        let (id, body) = match peer.recv() {
            Ok(packet) => packet,
            // A protocol refusal closes the stream without a reason packet.
            Err(error) => return format!("closed: {error}"),
        };
        match play::Clientbound::decode(id, &body).unwrap() {
            play::Clientbound::CustomPayload(p) if p.channel == channel => {
                panic!("a reply was sent before the refusal: {:?}", p.data)
            }
            play::Clientbound::KeepAlive(p) => peer
                .send(&play::Serverbound::KeepAlive(p).encode())
                .unwrap(),
            play::Clientbound::Disconnect(p) => {
                let lang = [("disconnect.genericReason".to_owned(), "%s".to_owned())].into();
                return horizon_protocol::text::plain(&p.reason, &lang);
            }
            _ => {}
        }
    }
}
