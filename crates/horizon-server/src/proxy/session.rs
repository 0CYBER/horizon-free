//! One player through the proxy: logged in with the client, logged in to the game, then every packet relayed
//! both ways, as it is, while the client's own settings are kept for a later login to the game (replay.rs).
//!
//! When the game goes away (a restart, a crash) the client is not let go: it is sent StartConfiguration, which
//! a client takes as "the world is being set up again" (a loading screen), and held in that phase on the same
//! connection, kept alive by the proxy. When the game is back the proxy logs into it again as the player,
//! tells it what the client said, and passes its configuration to the client, which then plays on.

use super::{
    Deadline, Inner,
    auth::Profile,
    backend::{self, Backend, Identity},
    replay::{Phase, Replay, body},
    wire::{Closer, ConnReader, ConnWriter, Connection, Writer, ids, login, translated},
};
use std::{
    net::IpAddr,
    sync::{Arc, Condvar, Mutex},
    thread,
    time::{Duration, Instant},
};

/// How long either side may be silent: the game and the client both keep each other alive every 15 s.
const SILENCE: Duration = Duration::from_secs(30);
/// How often a held client is sent a keep-alive, so that it never times out.
const KEEP_ALIVE: Duration = if cfg!(test) {
    Duration::from_millis(200)
} else {
    Duration::from_secs(5)
};
/// The proxy's own keep-alive ids: a number no game sends (a game's are times).
static NEXT_KEEP_ALIVE: std::sync::atomic::AtomicI64 =
    std::sync::atomic::AtomicI64::new(0x5052_4f58_0000_0000);
/// How long to wait before logging into a game that was not ready for it again.
const RETRY: Duration = if cfg!(test) {
    Duration::from_millis(50)
} else {
    Duration::from_millis(500)
};
/// How long a client may take to answanswer StartConfiguration.
const ACK_WAIT: Duration = Duration::from_secs(10);
/// How long one packet may take to reach the client. The socket's write timeout is per `write`, so a client
/// that drains a few bytes at a time would otherwise keep a send (and the writer lock) going for minutes.
const SEND_DEADLINE: Duration = Duration::from_secs(10);

/// The client's writer, which notes when a send began: the keep-alive thread can then cut a send that
/// takes too long without waiting for the writer's lock.
struct ClientWriter {
    inner: ConnWriter,
    since: Arc<Mutex<Option<Instant>>>,
}

impl ClientWriter {
    fn send(&mut self, body: &[u8]) -> Result<(), super::wire::Error> {
        *self.since.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
        let sent = self.inner.send(body);
        *self.since.lock().unwrap_or_else(|e| e.into_inner()) = None;
        sent
    }
}

pub struct Session {
    inner: Arc<Inner>,
    name: String,
    identity: (IpAddr, u128, Vec<login::Property>),
    /// The intent of the client's handshake (login or transfer), passed to the game each time.
    intent: i32,
    /// Shuts the client's socket without taking the writer's lock, which a stuck send may hold.
    client_closer: Closer,
    /// Every write to the client goes through this lock, and a packet from a game is written only while
    /// its generation is the current one: nothing from the old game follows StartConfiguration.
    client: Mutex<ClientWriter>,
    /// When the send in progress began, if any: see [`Session::cut_slow_send`].
    sending: Arc<Mutex<Option<Instant>>>,
    state: Mutex<State>,
    wake: Condvar,
    /// A write to the client is under way on a thread of its own ([`Session::off_the_loop`]).
    working: std::sync::atomic::AtomicBool,
}

struct State {
    /// Held: the client waits in configuration for a game.
    held: bool,
    /// The state the client is in, from its own packets.
    phase: Phase,
    /// StartConfiguration is sent and the client has not answered yet.
    pending_ack: bool,
    /// FinishConfiguration was sent and the proxy is still awaiting the client's transition to play.
    finish_sent: bool,
    game: Option<Arc<Mutex<ConnWriter>>>,
    /// Shuts the game's socket the same way.
    game_closer: Option<Closer>,
    /// Which game's packets are passed on: bumped each time a game goes or comes.
    generation: u64,
    /// Which engine process owns the installed backend connection.
    engine_generation: Option<u64>,
    replay: Replay,
    /// A game just logged into again: tell it the chat session when it starts play.
    replay_play: bool,
    held_since: Option<Instant>,
    attaching: bool,
    /// The keep-alive sent to the held client and not answered: (id, when).
    keep_alive: Option<(i64, Instant)>,
    last_keep_alive: Instant,
    ended: bool,
}

/// Takes a client whose name is verified through the rest of its login and its whole play.
pub(super) fn run(
    inner: &Arc<Inner>,
    mut client: Connection,
    who: Profile,
    peer: IpAddr,
    intent: i32,
    deadline: Deadline,
) {
    let identity = Identity {
        address: peer,
        uuid: who.uuid,
        name: &who.name,
        intent,
        properties: &who.properties,
    };
    // A game that is up is logged into first, so that its refusal (a ban, a full server) is what the player
    // sees, as without the proxy. A game that is down (or going down) holds the player at once.
    let game = if let Some(engine) = inner.up_generation() {
        match backend::login(inner.game, &inner.secret, &identity) {
            Ok(game) => Some((engine, game)),
            Err(backend::Failure::Refused(json)) if inner.up_generation() == Some(engine) => {
                return refuse(&mut client, json);
            }
            Err(e) => {
                inner.note(
                    "DEBUG",
                    &format!("{} is held: no login to the game ({e})", who.name),
                );
                None
            }
        }
    } else {
        None
    };
    if finish_login(inner, &mut client, &who).is_err() {
        return;
    }
    // The client acknowledged login. The relay now has its own silence timeout.
    drop(deadline);
    let (client_reader, client_writer) = client.split();
    let _ = client_reader.set_read_timeout(Some(SILENCE));
    let Ok(client_closer) = client_reader.closer() else {
        return;
    };
    let sending: Arc<Mutex<Option<Instant>>> = Arc::default();
    let session = Arc::new(Session {
        inner: inner.clone(),
        name: who.name.clone(),
        identity: (peer, who.uuid, who.properties.clone()),
        intent,
        client_closer,
        client: Mutex::new(ClientWriter {
            inner: client_writer,
            since: sending.clone(),
        }),
        sending,
        state: Mutex::new(State {
            held: true,
            phase: Phase::Config,
            pending_ack: false,
            finish_sent: false,
            game: None,
            game_closer: None,
            generation: 0,
            engine_generation: None,
            replay: Replay::default(),
            replay_play: false,
            held_since: Some(Instant::now()),
            attaching: game.is_some(),
            keep_alive: None,
            last_keep_alive: Instant::now(),
            ended: false,
        }),
        wake: Condvar::new(),
        working: std::sync::atomic::AtomicBool::new(false),
    });
    inner
        .sessions
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(session.clone());
    let attached = game.is_some_and(|(engine, game)| session.attach_game(game, engine, 0, false));
    session.lock().attaching = false;
    if attached {
        inner.note(
            "INFO",
            &format!(
                "{} joined through the proxy (UUID {:032x})",
                who.name, who.uuid
            ),
        );
    } else {
        inner.note(
            "INFO",
            &format!("Holding {} until the game accepts them", who.name),
        );
        // Readiness can change between the initial login and registration.
        session.resume();
    }
    session.read_client(client_reader);
    session.end();
    inner.note("INFO", &format!("{} left through the proxy", who.name));
}

/// Compression, LoginFinished and the client's LoginAcknowledged: the client is in configuration.
fn finish_login(
    inner: &Inner,
    client: &mut Connection,
    who: &Profile,
) -> Result<(), super::wire::Error> {
    if inner.compression >= 0 {
        client.send(
            &login::Clientbound::Compression(login::Compression {
                threshold: inner.compression,
            })
            .encode(),
        )?;
        client.set_compression(inner.compression);
    }
    client.send(
        &login::Clientbound::LoginFinished(login::LoginFinished {
            profile: login::GameProfile {
                uuid: who.uuid,
                name: who.name.clone(),
                properties: who.properties.clone(),
            },
            session: who.uuid,
        })
        .encode(),
    )?;
    let (id, payload) = client.recv()?;
    match login::Serverbound::decode(id, &payload)? {
        login::Serverbound::LoginAcknowledged(_) => Ok(()),
        _ => Err(super::wire::Error::new("unexpected packet during login")),
    }
}

fn refuse(client: &mut Connection, reason_json: String) {
    let _ =
        client.send(&login::Clientbound::Disconnect(login::Disconnect { reason_json }).encode());
}

impl Session {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn client(&self) -> std::sync::MutexGuard<'_, ClientWriter> {
        self.client.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Ends the session: both connections close, whoever waits is woken.
    pub fn end(&self) {
        let game = {
            let mut st = self.lock();
            let Some(game) = Self::mark_ended(&mut st) else {
                return;
            };
            game
        };
        self.close_ended(game);
    }

    fn mark_ended(st: &mut State) -> Option<Option<Closer>> {
        if st.ended {
            return None;
        }
        st.ended = true;
        st.game = None;
        Some(st.game_closer.take())
    }

    fn close_ended(&self, game: Option<Closer>) {
        self.client_closer.close();
        if let Some(game) = game {
            game.close();
        }
        self.wake.notify_all();
        let me: *const Session = self;
        self.inner
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|s| !std::ptr::eq(Arc::as_ptr(s), me));
    }

    /// Installs the backend only while its engine and this hold are still current.
    fn attach_game(self: &Arc<Self>, game: Backend, engine: u64, hold: u64, replay: bool) -> bool {
        let Backend { reader, writer } = game;
        let Ok(closer) = reader.closer() else {
            return false;
        };
        let writer = Arc::new(Mutex::new(writer));
        let mut output = writer.lock().unwrap_or_else(|e| e.into_inner());
        let (generation, packets) = {
            let gate = self.inner.gate();
            let mut st = self.lock();
            if gate.phase != super::Gate::Up
                || gate.generation != engine
                || st.ended
                || !st.held
                || st.generation != hold
                || st.phase != Phase::Config
                || st.pending_ack
                || st.finish_sent
                || st
                    .held_since
                    .is_none_or(|since| since.elapsed() >= self.inner.hold)
            {
                return false;
            }
            let packets = if replay {
                st.replay.config_packets()
            } else {
                Vec::new()
            };
            st.generation += 1;
            st.engine_generation = Some(engine);
            st.game = Some(writer.clone());
            st.game_closer = Some(closer);
            st.held = false;
            st.held_since = None;
            st.replay_play = replay;
            (st.generation, packets)
        };
        // The writer is locked before publication. Live packets arriving during replay follow it.
        let sent = backend::acknowledge(&mut output)
            .and_then(|()| packets.iter().try_for_each(|packet| output.send(packet)));
        drop(output);
        if sent.is_err() {
            self.game_lost(generation);
            return false;
        }
        let session = self.clone();
        let spawned = thread::Builder::new()
            .name("Proxy game reader".into())
            .spawn(move || session.read_game(generation, reader, &writer));
        if spawned.is_err() {
            self.end();
            return false;
        }
        true
    }

    /// The game's packets to the client, until this game goes.
    fn read_game(
        self: &Arc<Self>,
        generation: u64,
        mut reader: ConnReader,
        writer: &Arc<Mutex<ConnWriter>>,
    ) {
        let _ = reader.set_read_timeout(Some(SILENCE));
        loop {
            let packet = reader.recv();
            let Ok((id, payload)) = packet else {
                // The game's connection ended without a word: it died, or was stopped.
                return self.game_lost(generation);
            };
            let mut client = self.client();
            let (phase, replay_play) = {
                let mut st = self.lock();
                if st.generation != generation || st.held || st.ended {
                    return;
                }
                if st.phase == Phase::Config
                    && id == ids::configuration::clientbound::FINISH_CONFIGURATION
                {
                    st.finish_sent = true;
                } else if st.phase == Phase::Play
                    && id == ids::play::clientbound::START_CONFIGURATION
                {
                    st.pending_ack = true;
                }
                let replay_play = st.replay_play
                    && st.phase == Phase::Play
                    && id == ids::play::clientbound::LOGIN;
                if replay_play {
                    st.replay_play = false;
                }
                (st.phase, replay_play)
            };
            // The new game's Play Login is followed by the chat session, before anything else the client
            // sends: the game's writer is held from before the Login reaches the client until the replay
            // is sent, so a message or a refreshed key the client answers with waits behind it.
            let mut replaying =
                replay_play.then(|| writer.lock().unwrap_or_else(|e| e.into_inner()));
            if is_shutdown_kick(phase, id, &payload) && self.inner.hold_on_shutdown() {
                drop(replaying);
                drop(client);
                return self.game_lost(generation);
            }
            if client.send(&body(id, &payload)).is_err() {
                drop(replaying);
                drop(client);
                return self.end();
            }
            drop(client);
            if is_disconnect(phase, id) {
                return self.end();
            }
            if let Some(game) = replaying.as_mut() {
                // The new game has no chat session for the player yet: the one the client made.
                let packets = self.lock().replay.play_packets();
                for packet in packets {
                    let _ = game.send(&packet);
                }
            }
        }
    }

    /// The game this session was logged into is gone: unless this is a stop for good, hold the player.
    fn game_lost(self: &Arc<Self>, generation: u64) {
        if self.inner.closing() {
            let current = {
                let st = self.lock();
                !st.ended && st.generation == generation
            };
            if current {
                self.end();
            }
        } else {
            self.hold_if(|st| st.generation == generation);
        }
    }

    /// The client's packets: noted for a later replay, then to the game, or swallowed while held.
    fn read_client(self: &Arc<Self>, mut reader: ConnReader) {
        while let Ok((id, payload)) = reader.recv() {
            let (game, start) = {
                let client = self.client();
                let mut st = self.lock();
                if st.ended {
                    return;
                }
                let before = st.phase;
                st.replay.note(before, id, &payload);
                if before == Phase::Config
                    && id == ids::configuration::serverbound::KEEP_ALIVE
                    && let Some((expected, _)) = st.keep_alive
                    && payload.len() == 8
                    && payload[..] == expected.to_be_bytes()
                {
                    // The answer to the proxy's own keep-alive: the game never sent it.
                    st.keep_alive = None;
                    continue;
                }
                // StartConfiguration for a held client, sent below with the state released and the
                // writer still held: `pending_ack` is set first, as the client may answer at once.
                let mut start = false;
                if before == Phase::Config
                    && id == ids::configuration::serverbound::FINISH_CONFIGURATION
                {
                    if !st.finish_sent {
                        continue;
                    }
                    st.finish_sent = false;
                    if st.held && !st.pending_ack {
                        st.pending_ack = true;
                        start = true;
                    }
                    self.wake.notify_all();
                }
                st.phase = next_phase(before, id);
                if before == Phase::Play
                    && id == ids::play::serverbound::CONFIGURATION_ACKNOWLEDGED
                    && st.pending_ack
                {
                    st.pending_ack = false;
                    self.wake.notify_all();
                }
                let game = if st.held { None } else { st.game.clone() };
                (game, start.then_some(client))
            };
            if let Some(mut client) = start
                && client
                    .send(&body(ids::play::clientbound::START_CONFIGURATION, &[]))
                    .is_err()
            {
                drop(client);
                return self.end();
            }
            if let Some(game) = game {
                // A failure is the game going away; its reader hears of it.
                let _ = game
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .send(&body(id, &payload));
            }
        }
    }

    /// Holds the player: the game it was relayed to is gone or going.
    pub(super) fn hold_game(self: &Arc<Self>, engine: u64) {
        self.hold_if(|st| st.engine_generation == Some(engine));
    }

    fn hold_if(self: &Arc<Self>, current: impl Fn(&State) -> bool) {
        // Nothing to hold (or send) for a session already held or gone: do not wait for its writer.
        {
            let st = self.lock();
            if st.held || st.ended || !current(&st) {
                return;
            }
        }
        let old = {
            let mut client = self.client();
            let mut st = self.lock();
            if st.held || st.ended || !current(&st) {
                return;
            }
            st.held = true;
            st.held_since = Some(Instant::now());
            st.generation += 1;
            st.engine_generation = None;
            st.replay_play = false;
            st.last_keep_alive = Instant::now();
            // The old backend leaves in the same transition that holds the session, and its socket
            // is closed after the state is released: a backend attached meanwhile is never the one
            // closed.
            let old = {
                st.game = None;
                st.game_closer.take()
            };
            // The send happens without the state: a client that stalls on it must not block the
            // keep-alive thread, which takes the state of every session. `pending_ack` is set first, as
            // the client may answer as soon as the packet is out.
            let send = st.phase == Phase::Play && !st.pending_ack;
            if send {
                st.pending_ack = true;
            }
            drop(st);
            if send
                && client
                    .send(&body(ids::play::clientbound::START_CONFIGURATION, &[]))
                    .is_err()
            {
                drop(client);
                if let Some(game) = old {
                    game.close();
                }
                return self.end();
            }
            old
        };
        if let Some(game) = old {
            game.close();
        }
        self.inner.note(
            "INFO",
            &format!("Holding {} while the game restarts", self.name),
        );
    }

    fn same_hold(st: &State, hold: u64) -> bool {
        st.held && !st.ended && st.generation == hold
    }

    /// The game is back: logs into it again as this player, tells it what the client said, and lets its
    /// configuration through to the client.
    pub fn resume(self: &Arc<Self>) {
        let Some(engine) = self.inner.up_generation() else {
            return;
        };
        let hold = {
            let mut st = self.lock();
            if !st.held || st.attaching || st.ended {
                return;
            }
            st.attaching = true;
            st.generation
        };
        let session = self.clone();
        let spawned = thread::Builder::new()
            .name("Proxy resume".into())
            .spawn(move || {
                let result = session.log_in_again(engine, hold);
                if let Err(reason) = result {
                    session.inner.note(
                        "WARN",
                        &format!("{} can't be put back in the game: {reason}", session.name),
                    );
                    session.disconnect_if(
                        &translated(
                            "horizon.proxy.unavailable",
                            "The server is not available right now. Try again in a moment.",
                        ),
                        Some(engine),
                        |st| Self::same_hold(st, hold),
                    );
                }
                session.lock().attaching = false;
            });
        if spawned.is_err() {
            self.lock().attaching = false;
        }
    }

    fn log_in_again(self: &Arc<Self>, engine: u64, hold: u64) -> Result<(), String> {
        // The client has to be in configuration before the game's configuration reaches it.
        let deadline = Instant::now() + ACK_WAIT;
        {
            let mut st = self.lock();
            while (st.pending_ack || st.finish_sent) && Self::same_hold(&st, hold) {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Err("the client did not enter configuration".into());
                }
                st = self
                    .wake
                    .wait_timeout(st, left)
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
            if !Self::same_hold(&st, hold) {
                return Ok(());
            }
        }
        let (address, uuid, properties) = &self.identity;
        let who = Identity {
            address: *address,
            uuid: *uuid,
            name: &self.name,
            intent: self.intent,
            properties,
        };
        // The port may accept before the game is ready to log anyone in: try again until the hold runs out,
        // and give up at once only when the game turns the player away.
        let give_up = self
            .lock()
            .held_since
            .map_or_else(Instant::now, |since| since + self.inner.hold);
        let game = loop {
            if self.inner.up_generation() != Some(engine) || !Self::same_hold(&self.lock(), hold) {
                return Ok(());
            }
            match backend::login(self.inner.game, &self.inner.secret, &who) {
                Ok(game) => break game,
                Err(backend::Failure::Refused(json)) => {
                    self.disconnect_if(
                        &super::wire::component_from_json(&json),
                        Some(engine),
                        |st| Self::same_hold(st, hold),
                    );
                    return Ok(());
                }
                Err(other) => {
                    if Instant::now() + RETRY >= give_up {
                        return Err(other.to_string());
                    }
                    thread::sleep(RETRY);
                }
            }
        };
        if self.attach_game(game, engine, hold, true) {
            self.inner
                .note("INFO", &format!("{} is back in the game", self.name));
        }
        Ok(())
    }

    /// Keeps a held client alive and gives up on one held too long. Every second. The writes to the
    /// client are made on a thread of their own, never here: this is the one thread that looks after
    /// every session, and a client that stops reading must not stall the others.
    pub fn tick(self: &Arc<Self>, now: Instant, hold: Duration) {
        // Cuts a send that has run past its deadline: it only closes the socket, and waits for nothing.
        self.cut_slow_send(now, SEND_DEADLINE);
        let (expired, keep_alive_due) = {
            let st = self.lock();
            if !st.held || st.ended {
                return;
            }
            let expired = st
                .held_since
                .is_some_and(|since| now.duration_since(since) > hold);
            let due = !expired && Self::keep_alive_due(&st, now);
            (expired.then_some(st.generation), due)
        };
        if let Some(generation) = expired {
            self.off_the_loop(move |session| {
                let reason = translated(
                    "horizon.proxy.restart_timed_out",
                    "The server did not come back in time",
                );
                if session.disconnect_if(&reason, None, |st| Self::same_hold(st, generation)) {
                    session.inner.note(
                        "WARN",
                        &format!("The game did not come back for {}", session.name),
                    );
                }
            });
            return;
        }
        if keep_alive_due {
            self.off_the_loop(move |session| session.send_keep_alive(now));
        }
        // Also retries if a resume thread could not be started on the previous tick.
        self.resume();
    }

    fn keep_alive_due(st: &State, now: Instant) -> bool {
        st.held
            && !st.ended
            && st.phase == Phase::Config
            && !st.pending_ack
            && !st.finish_sent
            && now.duration_since(st.last_keep_alive) >= KEEP_ALIVE
    }

    /// The held client's keep-alive, if it is still due once the writer is this thread's. Waits for the
    /// writer, as it runs off the keep-alive thread; the state is not held while it writes.
    fn send_keep_alive(&self, now: Instant) {
        let mut client = self.client();
        let id = {
            let mut st = self.lock();
            if !Self::keep_alive_due(&st, now) {
                return;
            }
            st.last_keep_alive = now;
            let id = NEXT_KEEP_ALIVE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            st.keep_alive = Some((id, now));
            id
        };
        let mut w = Writer::new();
        w.i64(id);
        if client
            .send(&body(ids::configuration::clientbound::KEEP_ALIVE, &w.data))
            .is_err()
        {
            drop(client);
            self.end();
        }
    }

    /// Runs `work` on a thread of its own, one at a time per session: a tick that finds the last one
    /// still running does nothing, and the next tick asks again. A stalled write ends at its deadline
    /// ([`Self::cut_slow_send`]), which lets the thread go.
    fn off_the_loop(self: &Arc<Self>, work: impl FnOnce(&Arc<Self>) + Send + 'static) {
        use std::sync::atomic::Ordering;
        if self.working.swap(true, Ordering::AcqRel) {
            return;
        }
        // Clears the flag however the work ends, a panic included.
        struct Done(Arc<Session>);
        impl Drop for Done {
            fn drop(&mut self) {
                self.0.working.store(false, Ordering::Release);
            }
        }
        let done = Done(self.clone());
        let session = self.clone();
        let _ = std::thread::Builder::new()
            .name("proxy-client-send".into())
            .spawn(move || {
                let _done = done;
                work(&session);
            });
        // A thread that could not be started drops `done` with the closure, which clears the flag.
    }

    /// Ends the session if a send to the client has been under way for longer than `limit`: a client that
    /// takes a packet a few bytes at a time is not worth the writer lock the others wait for.
    #[cfg(test)]
    fn sending_longer_than(&self, limit: Duration) -> bool {
        let since = *self.sending.lock().unwrap_or_else(|e| e.into_inner());
        since.is_some_and(|since| since.elapsed() > limit)
    }

    fn cut_slow_send(&self, now: Instant, limit: Duration) {
        let since = *self.sending.lock().unwrap_or_else(|e| e.into_inner());
        if since.is_some_and(|since| now.duration_since(since) > limit) {
            // Close the socket first and on its own: the session may already be marked ended (a
            // disconnect in progress), and `end` then returns before it closes anything. Closing
            // frees the stalled write, and with it the locks the send holds.
            self.client_closer.close();
            self.end();
        }
    }

    /// Sends the client away with `reason`, in the state it is in, and ends the session.
    pub fn disconnect(&self, reason: &super::wire::Tag) {
        self.disconnect_if(reason, None, |_| true);
    }

    fn disconnect_if(
        &self,
        reason: &super::wire::Tag,
        engine: Option<u64>,
        allowed: impl FnOnce(&State) -> bool,
    ) -> bool {
        let mut client = self.client();
        let (phase, game) = {
            let gate = self.inner.gate();
            if engine.is_some_and(|epoch| gate.phase != super::Gate::Up || gate.generation != epoch)
            {
                return false;
            }
            let mut st = self.lock();
            if !allowed(&st) {
                return false;
            }
            let Some(game) = Self::mark_ended(&mut st) else {
                return false;
            };
            // After StartConfiguration the client already reads Configuration packets, though `phase`
            // stays Play until it acknowledges.
            let phase = if st.pending_ack {
                Phase::Config
            } else {
                st.phase
            };
            (phase, game)
        };
        let id = match phase {
            Phase::Config => ids::configuration::clientbound::DISCONNECT,
            Phase::Play => ids::play::clientbound::DISCONNECT,
        };
        let mut w = Writer::new();
        w.nbt(reason);
        let _ = client.send(&body(id, &w.data));
        drop(client);
        self.close_ended(game);
        true
    }
}

/// Whether a packet from the game is the kick a stopping game sends everyone (`server_shutdown`), in the
/// disconnect packet of the state the client is in.
fn is_shutdown_kick(phase: Phase, id: i32, payload: &[u8]) -> bool {
    if !is_disconnect(phase, id) {
        return false;
    }
    let Ok(tag) = super::wire::Reader::new(payload).nbt() else {
        return false;
    };
    matches!(tag.get("translate"), Some(super::wire::Tag::String(key))
        if *key == super::wire::JavaString::from(SHUTDOWN_KEY))
}

fn is_disconnect(phase: Phase, id: i32) -> bool {
    id == match phase {
        Phase::Config => ids::configuration::clientbound::DISCONNECT,
        Phase::Play => ids::play::clientbound::DISCONNECT,
    }
}

const SHUTDOWN_KEY: &str = "multiplayer.disconnect.server_shutdown";

/// The state the client is in after it sent packet `id`: FinishConfiguration starts play, and
/// ConfigurationAcknowledged (the answer to StartConfiguration) a new configuration.
pub fn next_phase(phase: Phase, id: i32) -> Phase {
    match phase {
        Phase::Config if id == ids::configuration::serverbound::FINISH_CONFIGURATION => Phase::Play,
        Phase::Play if id == ids::play::serverbound::CONFIGURATION_ACKNOWLEDGED => Phase::Config,
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_client_follows_the_game_through_configuration_and_back() {
        use ids::{configuration::serverbound as cfg, play::serverbound as play};
        let mut phase = Phase::Config;
        for (id, expected) in [
            (cfg::CLIENT_INFORMATION, Phase::Config),
            (cfg::FINISH_CONFIGURATION, Phase::Play),
            (play::KEEP_ALIVE, Phase::Play),
            // 3 means FinishConfiguration only in configuration: in play it is another packet.
            (cfg::FINISH_CONFIGURATION, Phase::Play),
            (play::CONFIGURATION_ACKNOWLEDGED, Phase::Config),
            // 16 is ConfigurationAcknowledged only in play.
            (play::CONFIGURATION_ACKNOWLEDGED, Phase::Config),
        ] {
            phase = next_phase(phase, id);
            assert_eq!(phase, expected, "after packet {id}");
        }
    }

    #[test]
    fn only_a_shutdown_kick_in_the_clients_own_state_is_a_stop() {
        let kick = translated_payload("multiplayer.disconnect.server_shutdown");
        assert!(is_shutdown_kick(
            Phase::Play,
            ids::play::clientbound::DISCONNECT,
            &kick
        ));
        assert!(is_shutdown_kick(
            Phase::Config,
            ids::configuration::clientbound::DISCONNECT,
            &kick
        ));
        // A ban is a kick the player must see.
        let ban = translated_payload("multiplayer.disconnect.banned");
        assert!(!is_shutdown_kick(
            Phase::Play,
            ids::play::clientbound::DISCONNECT,
            &ban
        ));
        // The same words in another packet (chat) are not a kick.
        assert!(!is_shutdown_kick(
            Phase::Play,
            ids::play::clientbound::SYSTEM_CHAT,
            &kick
        ));
        let mut literal = Writer::new();
        literal.nbt(&super::super::wire::Tag::String(SHUTDOWN_KEY.into()));
        assert!(!is_shutdown_kick(
            Phase::Play,
            ids::play::clientbound::DISCONNECT,
            &literal.data
        ));
    }

    fn session_named(proxy: &super::super::Proxy, name: &str) -> Arc<Session> {
        proxy
            .inner
            .snapshot()
            .into_iter()
            .find(|s| s.name == name)
            .unwrap()
    }

    #[test]
    fn a_busy_writer_does_not_stop_the_keep_alives_of_the_other_sessions() {
        use super::super::join_tests::{FakeGame, join, proxy_for, temp_dir};
        let dir = temp_dir("busy-writer");
        let secret = super::super::secret(&dir).unwrap();
        let game = FakeGame::start(&secret);
        let (proxy, port) = proxy_for(&game, &dir);
        proxy.game_up();
        let mut first = join(port, "Slow", &[]);
        let mut second = join(port, "Other", &[]);
        let (slow, other) = (
            session_named(&proxy, "Slow"),
            session_named(&proxy, "Other"),
        );
        proxy.game_stopping();
        for s in [&slow, &other] {
            while !s.lock().held {
                thread::sleep(Duration::from_millis(10));
            }
        }
        // Both players answer StartConfiguration, so that they are kept alive.
        for (client, session) in [(&mut first, &slow), (&mut second, &other)] {
            client
                .reader
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            while client.recv().unwrap().0 != ids::play::clientbound::START_CONFIGURATION {}
            client
                .send(&body(
                    ids::play::serverbound::CONFIGURATION_ACKNOWLEDGED,
                    &[],
                ))
                .unwrap();
            while session.lock().pending_ack {
                thread::sleep(Duration::from_millis(10));
            }
        }
        // A send to the first player is under way.
        let busy = slow.client();
        let (done, finished) = std::sync::mpsc::channel();
        let (ticking, other_ticking) = (slow.clone(), other.clone());
        thread::spawn(move || {
            let later = Instant::now() + Duration::from_secs(60);
            ticking.tick(later, Duration::from_secs(3600));
            other_ticking.tick(later, Duration::from_secs(3600));
            let _ = done.send(());
        });
        let ticked = finished.recv_timeout(Duration::from_secs(2));
        assert!(ticked.is_ok(), "tick waited for a writer that is busy");
        // The other player still hears the keep-alive, though the first one's writer is held.
        loop {
            let (id, _) = second.recv().unwrap();
            if id == ids::configuration::clientbound::KEEP_ALIVE {
                break;
            }
        }
        // The first player's keep-alive waits on its own thread and goes out once the writer is free.
        drop(busy);
        loop {
            let (id, _) = first.recv().unwrap();
            if id == ids::configuration::clientbound::KEEP_ALIVE {
                break;
            }
        }
    }

    /// Sends packets to `session` from another thread until one fails. A client that reads nothing
    /// fills the socket's buffers, and the send under way then stalls. The packets stay well under the
    /// frame limit, and are random bytes: the connection compresses, and a repetitive payload would
    /// shrink to nothing.
    fn send_until_stalled(
        session: &Arc<Session>,
    ) -> thread::JoinHandle<Result<(), super::super::wire::Error>> {
        use super::super::join_tests::ECHOED;
        let mut x = 1u32;
        let payload: Vec<u8> = (0..256 * 1024)
            .map(|_| {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (x >> 24) as u8
            })
            .collect();
        let sender = session.clone();
        thread::spawn(move || {
            for _ in 0..2_000 {
                sender.client().send(&body(ECHOED, &payload))?;
            }
            Ok(())
        })
    }

    /// Cuts the stalled send of `sent` and returns how long it took to fail, given what the socket's own
    /// write timeout (2 s) would take.
    fn cut_and_time(
        session: &Arc<Session>,
        sent: thread::JoinHandle<Result<(), super::super::wire::Error>>,
    ) -> Duration {
        let started = Instant::now();
        // The deadline is well under the socket's write timeout, so the cut comes first.
        while !session.sending_longer_than(Duration::from_millis(200)) {
            assert!(
                started.elapsed() < Duration::from_secs(60),
                "the send never stalled"
            );
            thread::sleep(Duration::from_millis(20));
        }
        let cut = Instant::now();
        session.cut_slow_send(Instant::now(), Duration::from_millis(200));
        while !sent.is_finished() {
            assert!(
                cut.elapsed() < Duration::from_secs(10),
                "the send was not cut"
            );
            thread::sleep(Duration::from_millis(10));
        }
        let took = cut.elapsed();
        assert!(sent.join().unwrap().is_err());
        took
    }

    #[test]
    fn a_send_the_client_does_not_take_is_cut_at_the_deadline() {
        use super::super::join_tests::{FakeGame, join, proxy_for, temp_dir};
        let dir = temp_dir("slow-reader");
        let secret = super::super::secret(&dir).unwrap();
        let game = FakeGame::start(&secret);
        let (proxy, port) = proxy_for(&game, &dir);
        proxy.game_up();
        // Kept open and never read: the socket's buffers fill and the send stalls. A reader that
        // drips keeps each write under the socket's own timeout the same way; only the deadline
        // on the whole send ends either.
        let client = join(port, "Drip", &[]);
        let session = session_named(&proxy, "Drip");
        let sent = send_until_stalled(&session);
        let took = cut_and_time(&session, sent);
        assert!(took < Duration::from_millis(1500), "cut after {took:?}");
        assert!(session.lock().ended);
        drop(client);
    }

    #[test]
    fn a_stalled_disconnect_is_cut_though_the_session_is_already_marked_ended() {
        use super::super::join_tests::{FakeGame, join, proxy_for, temp_dir};
        let dir = temp_dir("slow-disconnect");
        let secret = super::super::secret(&dir).unwrap();
        let game = FakeGame::start(&secret);
        let (proxy, port) = proxy_for(&game, &dir);
        proxy.game_up();
        let client = join(port, "Stall", &[]);
        let session = session_named(&proxy, "Stall");
        // `disconnect_if` marks the session ended before it sends the kick.
        Session::mark_ended(&mut session.lock());
        let sent = send_until_stalled(&session);
        let took = cut_and_time(&session, sent);
        assert!(took < Duration::from_millis(1500), "cut after {took:?}");
        drop(client);
    }

    fn translated_payload(key: &str) -> Vec<u8> {
        let mut w = Writer::new();
        w.nbt(&super::super::wire::Tag::compound(vec![(
            "translate",
            super::super::wire::Tag::String(super::super::wire::JavaString::from(key)),
        )]));
        w.data
    }
}
