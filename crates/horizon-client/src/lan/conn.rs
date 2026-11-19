//! One accepted connection: handshake, status ping, login (offline, or online
//! with encryption and the session service's verdict) and the
//! configuration phase (ServerStatusPacketListenerImpl,
//! ServerLoginPacketListenerImpl, ServerConfigurationPacketListenerImpl). A
//! player that completes configuration continues in the play state on the
//! network threads ([`io_pool`]): they forward decoded packets to the game
//! thread, frame, compress and encrypt what the game queues, and keep the
//! keep-alive schedule on their own so a long tick never times a client out.
mod payloads;
use super::config_payloads::ConfigIo;
use super::{
    Body, Joined, NetEvent, Out, Shared,
    data::{CORE_PACK, SyncData},
    forwarding,
    io_pool::{self, Session as _},
};
use crate::raw_payloads::{Side, Transport};
use horizon_protocol::{
    PROTOCOL_VERSION,
    connection::{ConnWriter, Connection, Counters},
    crypto::{offline_uuid, server_hash, uuid_string},
    packets::{
        Empty,
        configuration::{self as cfg, ClientInformation, KnownPack},
        handshake, login, play, status,
    },
    payload_negotiation::Phase,
    text::translatable,
};
use mod_api::{PayloadFrameStage, PayloadId, PayloadSendError};
use std::{
    cell::Cell,
    net::TcpStream,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicI32, AtomicI64, AtomicU32, AtomicUsize, Ordering},
        mpsc::{RecvTimeoutError, Sender, channel},
    },
    time::{Duration, Instant},
};

/// ServerCommonPacketListenerImpl.keepConnectionAlive: a keep-alive every 15 s,
/// and a client that has not answered the last one by the next is dropped.
const KEEP_ALIVE_EVERY: Duration = Duration::from_secs(15);
/// ReadTimeoutHandler(30) in ServerConnectionListener: a connection that sends
/// nothing for 30 s.
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// How long the handshake, a status request and the login may wait for the client's next packet,
/// where vanilla waits 30 s: a client answers these at once, and each connection holds a thread
/// and one of the places the server keeps for connections, which a crowd of silent or slow
/// sockets would otherwise take for 30 s each. Configuration and play keep [`READ_TIMEOUT`] (a
/// resource pack downloads in configuration).
const LOGIN_TIMEOUT: Duration = Duration::from_secs(10);
/// Vanilla's server-side disconnect during login is a JSON component.
fn login_disconnect(c: &mut Connection, reason: &horizon_core::nbt::Tag) {
    let json = component_json(reason);
    let _ =
        c.send(&login::Clientbound::Disconnect(login::Disconnect { reason_json: json }).encode());
}

/// A component as ComponentSerialization.CODEC writes it with JsonOps (and
/// Gson without HTML escaping): plain text collapses to a string, the
/// content fields come first, then `extra`, then the style.
pub fn component_json(tag: &horizon_core::nbt::Tag) -> String {
    component_value(tag).compact()
}
fn component_value(tag: &horizon_core::nbt::Tag) -> crate::server::json::Json {
    use crate::server::json::Json;
    use horizon_core::nbt::Tag;
    const ORDER: &[&str] = &[
        "type",
        "text",
        "translate",
        "fallback",
        "with",
        "score",
        "selector",
        "separator",
        "keybind",
        "nbt",
        "interpret",
        "block",
        "entity",
        "storage",
        "source",
        "object",
        "sprite",
        "atlas",
        "player",
        "hat",
        "extra",
        "color",
        "shadow_color",
        "bold",
        "italic",
        "underlined",
        "strikethrough",
        "obfuscated",
        "click_event",
        "hover_event",
        "insertion",
        "font",
    ];
    match tag {
        Tag::String(s) => Json::String(s.to_string_lossy()),
        Tag::List(items) => Json::Array(items.iter().map(component_value).collect()),
        Tag::Compound(map) => {
            if map.len() == 1
                && let Some(Tag::String(t)) = map.get(&horizon_core::nbt::JavaString::from("text"))
            {
                return Json::String(t.to_string_lossy());
            }
            let mut keys: Vec<String> = map.keys().map(|k| k.to_string_lossy()).collect();
            keys.sort_by_key(|k| ORDER.iter().position(|o| o == k).unwrap_or(ORDER.len()));
            Json::Object(
                keys.into_iter()
                    .map(|k| {
                        let v = &map[&horizon_core::nbt::JavaString::from(k.as_str())];
                        let value = match v {
                            Tag::Byte(b) => Json::Bool(*b != 0),
                            Tag::Int(i) => Json::Number(i.to_string()),
                            other => component_value(other),
                        };
                        (k, value)
                    })
                    .collect(),
            )
        }
        Tag::Byte(b) => Json::Bool(*b != 0),
        Tag::Int(i) => Json::Number(i.to_string()),
        Tag::Long(i) => Json::Number(i.to_string()),
        _ => Json::Null,
    }
}

/// The inverse of [`component_value`]: a JSON component (a server.properties
/// prompt) as ComponentSerialization.CODEC writes it with NbtOps. An array is
/// its first component with the rest appended (`createFromList`); a list of
/// components mixing plain strings and objects has the strings as `{text}`.
pub fn component_tag(v: &crate::server::json::Json) -> horizon_core::nbt::Tag {
    use crate::server::json::Json;
    use horizon_core::nbt::{JavaString, Tag};
    fn list(items: &[Json]) -> Tag {
        same_kind(items.iter().map(component_tag).collect())
    }
    fn same_kind(mut tags: Vec<Tag>) -> Tag {
        if tags.iter().any(|t| matches!(t, Tag::Compound(_))) {
            for t in &mut tags {
                if let Tag::String(s) = t {
                    *t = Tag::compound([("text", Tag::String(s.clone()))]);
                }
            }
        }
        Tag::List(tags)
    }
    fn value(v: &Json) -> Tag {
        match v {
            Json::Bool(b) => Tag::Byte(i8::from(*b)),
            Json::Number(n) => n.parse().map_or_else(
                |_| Tag::Double(n.parse::<f64>().unwrap_or(0.0).to_bits()),
                Tag::Int,
            ),
            Json::String(s) => Tag::String(JavaString::from(s.as_str())),
            Json::Array(items) => list(items),
            Json::Object(members) => Tag::compound(
                members
                    .iter()
                    .map(|(k, v)| (k.as_str(), value(v)))
                    .collect::<Vec<_>>(),
            ),
            Json::Null => Tag::String(JavaString::from("")),
        }
    }
    match v {
        Json::Array(items) if !items.is_empty() => {
            let mut first = component_tag(&items[0]);
            if let Tag::String(s) = &first {
                first = Tag::compound([("text", Tag::String(s.clone()))]);
            }
            if let Tag::Compound(map) = &mut first {
                let mut extra = match map.remove(&JavaString::from("extra")) {
                    Some(Tag::List(e)) => e,
                    _ => Vec::new(),
                };
                extra.extend(items[1..].iter().map(component_tag));
                map.insert(JavaString::from("extra"), same_kind(extra));
            }
            first
        }
        Json::Object(members) => {
            if let [(key, Json::String(text))] = members.as_slice()
                && key == "text"
            {
                return Tag::String(JavaString::from(text.as_str()));
            }
            Tag::compound(
                members
                    .iter()
                    .map(|(k, v)| {
                        let tag = match k.as_str() {
                            "extra" | "with" => match v {
                                Json::Array(items) => list(items),
                                other => value(other),
                            },
                            _ => value(v),
                        };
                        (k.as_str(), tag)
                    })
                    .collect::<Vec<_>>(),
            )
        }
        Json::String(s) | Json::Number(s) => Tag::String(JavaString::from(s.as_str())),
        Json::Bool(b) => Tag::String(JavaString::from(b.to_string().as_str())),
        _ => Tag::String(JavaString::from("")),
    }
}

/// StringUtil.isValidPlayerName.
pub fn valid_name(name: &str) -> bool {
    (1..=16).contains(&name.len()) && name.chars().all(|c| c as u32 > 32 && (c as u32) < 127)
}

thread_local! {
    static HOLDING: Cell<bool> = const { Cell::new(false) };
}

/// While one is alive, the packets this thread sends to each player are held
/// and handed to the player's network thread together by
/// [`PlayerSender::flush`] or [`flush_all`]: one hand-over per player per tick
/// rather than one per packet. The bytes and their order don't change.
pub struct Holding(bool);
impl Holding {
    pub fn start() -> Self {
        Self(HOLDING.replace(true))
    }
}
impl Drop for Holding {
    fn drop(&mut self) {
        HOLDING.set(self.0);
    }
}

/// The packets (and their bytes) a connection's network thread has handed the hub and the hub hasn't
/// taken from its channel yet. The hub's inbox caps apply only as it drains that channel, at the start
/// of a tick: while a tick runs long (an autosave, chunk generation) a client could otherwise queue
/// tiny packets without bound, each a boxed `Serverbound` many times its size on the wire.
#[derive(Debug, Default)]
pub struct Undrained {
    packets: AtomicUsize,
    bytes: AtomicUsize,
}
impl Undrained {
    /// Twice the hub's inbox caps: a client within them never comes near.
    pub const MAX_PACKETS: usize = 2 * super::hub::INBOX_LIMIT;
    pub const MAX_BYTES: usize = 2 * super::hub::INBOX_BYTES;
    /// Count a packet of `bytes` handed to the hub, or `false` (counting nothing) when that would
    /// pass a cap: the network thread disconnects the client then.
    pub fn handed(&self, bytes: usize) -> bool {
        let packets = self.packets.fetch_add(1, Ordering::AcqRel);
        let total = self.bytes.fetch_add(bytes, Ordering::AcqRel);
        if packets >= Self::MAX_PACKETS || total.saturating_add(bytes) > Self::MAX_BYTES {
            self.taken(bytes);
            return false;
        }
        true
    }
    /// The hub took a packet of `bytes` from its channel.
    pub fn taken(&self, bytes: usize) {
        self.packets.fetch_sub(1, Ordering::AcqRel);
        self.bytes.fetch_sub(bytes, Ordering::AcqRel);
    }
}

/// The sending half given to the game thread.
#[derive(Clone)]
pub struct PlayerSender {
    payload_transport: Arc<Transport>,
    /// What the player's network thread has handed the hub and the hub hasn't taken yet.
    pub undrained: Arc<Undrained>,
    /// The bytes of the bodies sent and not framed yet.
    pub pending: Arc<AtomicUsize>,
    held: Arc<Mutex<Vec<Body>>>,
    framing: horizon_protocol::connection::Framing,
    to: To,
    /// The connection's bytes on the wire (after compression and encryption), counted by its
    /// network thread: `horizon-report`'s network use per player. None for a test's channel.
    wire: Option<Arc<Counters>>,
    /// Registry negotiation belongs to this live connection, never a process-wide UUID cache.
    pub datapack_offer: Option<Arc<[u8]>>,
    /// The plugin channels the client announced, in configuration and play.
    pub announced: Arc<super::announced_channels::AnnouncedChannels>,
    /// Its `horizon:levels` answer: its revision and the levels besides its own it can view.
    pub viewed_levels: horizon_protocol::viewed_levels::Answer,
    /// The `horizon:menu_extra` payload layout the client reads: its revision answer,
    /// 0 for one that did not answer (the legacy layout, a menu of a block only).
    pub menu_extra_rev: u8,
    /// The host and port the client typed in its handshake ([`typed_address`]), None for a
    /// connection that sent none.
    pub connect: Option<(Arc<str>, u16)>,
}
#[derive(Clone)]
enum To {
    /// A connectionless actor: nothing is ever delivered.
    Disabled,
    /// A test's channel.
    Channel(Sender<Out>),
    /// A connection on the network threads, and its keep-alive clock.
    Pool(Arc<io_pool::Outbox>, Arc<KeepAliveWatch>),
}
impl PlayerSender {
    #[cfg(any(test, feature = "test-payload-probe"))]
    pub(crate) fn payload_diagnostic_observer(&self) -> Box<dyn Fn() -> String + Send + Sync> {
        let pending = Arc::downgrade(&self.pending);
        let held = Arc::downgrade(&self.held);
        let (outbox, kind) = match &self.to {
            To::Pool(outbox, _) => (Some(Arc::downgrade(outbox)), "pool"),
            To::Disabled => (None, "disabled"),
            To::Channel(_) => (None, "test channel"),
        };
        Box::new(move || {
            let pending = pending.upgrade().map(|p| p.load(Ordering::Acquire));
            let window = pending.map(|p| io_pool::MAX_PENDING.saturating_sub(p));
            let held = match held.upgrade() {
                Some(held) => match held.try_lock() {
                    Ok(h) => format!(
                        "items={} bytes={}",
                        h.len(),
                        h.iter().map(|body| body.len()).sum::<usize>()
                    ),
                    Err(_) => "busy/poisoned".into(),
                },
                None => "owner dropped".into(),
            };
            let outbox = match &outbox {
                Some(outbox) => outbox
                    .upgrade()
                    .map(|o| o.diagnostics())
                    .unwrap_or_else(|| "outbox owner dropped".into()),
                None => kind.into(),
            };
            format!(
                "server window={window:?} capacity={} pending_charge={pending:?} (None=owner dropped) held={held} {outbox}",
                io_pool::MAX_PENDING
            )
        })
    }
    /// The immutable codec snapshot and live epochs for this connection.
    pub fn payload_transport(&self) -> &Arc<Transport> {
        &self.payload_transport
    }
    /// Frozen physical framing, used to reject before copying a large raw body.
    pub(crate) fn payload_framing(&self) -> horizon_protocol::connection::Framing {
        self.framing
    }
    pub(crate) fn payload_capacity(&self) -> usize {
        io_pool::MAX_PENDING
    }
    /// A sender whose packets go to `tx` (a test's).
    pub fn new(tx: Sender<Out>, pending: Arc<AtomicUsize>) -> Self {
        Self {
            undrained: Arc::default(),
            pending,
            held: Arc::default(),
            framing: horizon_protocol::connection::Framing::Network(None),
            to: To::Channel(tx),
            wire: None,
            payload_transport: Transport::empty(),
            datapack_offer: None,
            announced: Arc::default(),
            viewed_levels: Default::default(),
            menu_extra_rev: 0,
            connect: None,
        }
    }
    #[cfg(test)]
    pub(crate) fn new_with_payload_transport(
        tx: Sender<Out>,
        pending: Arc<AtomicUsize>,
        transport: Arc<Transport>,
    ) -> Self {
        let mut sender = Self::new(tx, pending);
        sender.payload_transport = transport;
        sender
    }
    /// Real bounded output queue without socket adoption, for executor tests.
    #[cfg(test)]
    pub(crate) fn payload_executor_test_sender(
        transport: Arc<Transport>,
    ) -> (Self, impl Fn() -> Vec<Vec<u8>>) {
        let pending = Arc::default();
        let outbox = io_pool::Outbox::new(Arc::clone(&pending), false).unwrap();
        let inspect = outbox.clone();
        let sender = Self {
            payload_transport: transport,
            undrained: Arc::default(),
            pending,
            held: Arc::default(),
            framing: horizon_protocol::connection::Framing::Memory,
            to: To::Pool(
                outbox,
                Arc::new(KeepAliveWatch {
                    start: Instant::now(),
                    alive: Arc::new(AtomicI64::new(-1)),
                }),
            ),
            wire: None,
            datapack_offer: None,
            announced: Arc::default(),
            viewed_levels: Default::default(),
            menu_extra_rev: 0,
            connect: None,
        };
        (sender, move || inspect.queued_contents())
    }
    /// A connectionless actor has no writer, queue or packet delivery.
    pub fn disabled() -> Self {
        Self {
            undrained: Arc::default(),
            pending: Arc::default(),
            held: Arc::default(),
            framing: horizon_protocol::connection::Framing::Network(None),
            to: To::Disabled,
            wire: None,
            payload_transport: Transport::empty(),
            datapack_offer: None,
            announced: Arc::default(),
            viewed_levels: Default::default(),
            menu_extra_rev: 0,
            connect: None,
        }
    }
    /// The bytes received from and sent to the client on the wire so far.
    pub fn wire_bytes(&self) -> Option<(u64, u64)> {
        self.wire.as_ref().map(|c| {
            (
                c.bytes_in.load(Ordering::Relaxed).max(0) as u64,
                c.bytes_out.load(Ordering::Relaxed).max(0) as u64,
            )
        })
    }
    /// Whether the client has answered no keep-alive for too long.
    pub fn timed_out(&self) -> bool {
        matches!(&self.to, To::Pool(_, watch) if watch.timed_out())
    }
    /// Close the connection now, dropping what is still queued for it.
    pub fn close(&self) {
        self.payload_transport.close();
        if let To::Pool(outbox, _) = &self.to {
            outbox.send(Vec::new(), Some(io_pool::End::Close));
        }
    }
    fn held(&self) -> std::sync::MutexGuard<'_, Vec<Body>> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }
    fn reserve(&self, cost: usize) -> bool {
        self.pending
            .try_update(Ordering::AcqRel, Ordering::Relaxed, |n| {
                n.checked_add(cost).filter(|n| *n <= io_pool::MAX_PENDING)
            })
            .is_ok()
    }
    /// [`Self::reserve`] for a body to send, which a failed reservation closes the connection
    /// for. A memory connection (the integrated server's own player, a splitscreen player) is not
    /// bounded, as its outbox isn't (`Outbox::new`'s `capped`): it reads what it is sent as it
    /// comes, and its first view arrives in one burst that can pass MAX_PENDING (a reopened
    /// world's chunks all load from disk at once). Bounding it closed the connection with no
    /// reason, "End of stream": #12849, then again #18313. Keep this exemption.
    fn reserve_body(&self, cost: usize) -> bool {
        if matches!(self.framing, horizon_protocol::connection::Framing::Memory) {
            self.pending.fetch_add(cost, Ordering::AcqRel);
            return true;
        }
        self.reserve(cost)
    }
    /// Prepare a whole encoded packet without publishing it. The channel is diagnostic only;
    /// its wire identifier must already be included in `body`.
    pub fn prepare_payload(
        &self,
        body: Vec<u8>,
        id: &PayloadId,
    ) -> Result<PreparedPayload, PayloadSendError> {
        let To::Pool(outbox, _) = &self.to else {
            return Err(PayloadSendError::Disconnected);
        };
        if !outbox.alive() {
            return Err(PayloadSendError::Disconnected);
        }
        let bytes = self
            .framing
            .contents_owned(body)
            .map_err(|e| match e.frame_limit() {
                Some(d) => PayloadSendError::FrameTooLarge {
                    id: id.clone(),
                    stage: match d.stage {
                        horizon_protocol::frame::FrameSize::Encoded => PayloadFrameStage::Encoded,
                        horizon_protocol::frame::FrameSize::Uncompressed => {
                            PayloadFrameStage::Uncompressed
                        }
                    },
                    size: d.size as u64,
                    limit: d.limit as u64,
                },
                None => PayloadSendError::EncodeFailed(e.to_string()),
            })?;
        // Bookkeeping for PreparedPayload and the transaction's deferred journal record,
        // including geometric journal growth (records are at most 256 bytes). This is
        // retained-memory accounting, not a packet count or application-body cap.
        let cost = io_pool::charge(bytes.capacity()).saturating_add(512);
        if !self.reserve(cost) {
            return Err(PayloadSendError::Backpressure);
        }
        Ok(PreparedPayload {
            sender: self.clone(),
            bytes: Some(bytes),
            cost,
        })
    }
    pub fn send(&self, body: Vec<u8>) {
        self.send_body(body.into());
    }
    fn send_body(&self, body: Body) {
        if matches!(self.to, To::Disabled) {
            return;
        }
        if !self.reserve_body(io_pool::charge(body.retained())) {
            self.close();
            return;
        }
        if HOLDING.get() {
            self.held().push(body);
        } else {
            self.flush();
            match &self.to {
                To::Disabled => {}
                To::Channel(tx) => drop(tx.send(Out::Packet(body.into_vec()))),
                To::Pool(outbox, _) => outbox.send(vec![body], None),
            }
        }
    }
    /// Packets queued for the network thread or held by the tick, and their bytes.
    pub fn queued(&self) -> (usize, usize) {
        if matches!(self.to, To::Disabled) {
            return (0, 0);
        }
        let waiting = match &self.to {
            To::Pool(outbox, _) => outbox.queued(),
            To::Channel(_) | To::Disabled => 0,
        };
        (
            waiting + self.held().len(),
            self.pending.load(Ordering::Relaxed),
        )
    }
    /// Several packets in order, as [`Self::send`] each: under one lock
    /// while the tick holds them. Shared bodies stay shared up to the wire.
    pub fn send_many<B: Into<Body>>(&self, bodies: impl IntoIterator<Item = B>) {
        if matches!(self.to, To::Disabled) {
            return;
        }
        if !HOLDING.get() {
            bodies
                .into_iter()
                .for_each(|body| self.send_body(body.into()));
            return;
        }
        let mut held = self.held();
        for body in bodies {
            let body = body.into();
            if !self.reserve_body(io_pool::charge(body.retained())) {
                self.close();
                return;
            }
            held.push(body);
        }
    }
    /// Hand the held packets to the network thread.
    pub fn flush(&self) {
        let held = std::mem::take(&mut *self.held());
        if !held.is_empty() {
            match &self.to {
                To::Disabled => {}
                To::Channel(tx) => drop(tx.send(unshared(held))),
                To::Pool(outbox, _) => outbox.send(held, None),
            }
        }
    }
    /// Send a play disconnect with this reason after what was sent before,
    /// then close.
    pub fn disconnect(&self, reason: horizon_core::nbt::Tag) {
        self.payload_transport.close();
        self.flush();
        match &self.to {
            To::Disabled => {}
            To::Channel(tx) => drop(tx.send(Out::Disconnect(reason))),
            To::Pool(outbox, _) => {
                let body = play::Clientbound::Disconnect(cfg::Disconnect { reason }).encode();
                let body = Body::from(body);
                if !self.reserve_body(io_pool::charge(body.retained())) {
                    self.close();
                    return;
                }
                outbox.send(vec![body], Some(io_pool::End::Disconnect));
            }
        }
    }
}

/// Held bodies as a test's channel gives them: each its own bytes.
fn unshared(held: Vec<Body>) -> Out {
    Out::Packets(held.into_iter().map(Body::into_vec).collect())
}

/// A transaction-owned packet. Dropping it cancels its retained-byte reservation.
pub struct PreparedPayload {
    sender: PlayerSender,
    bytes: Option<Vec<u8>>,
    cost: usize,
}
impl PreparedPayload {
    /// Publish exactly the bytes prepared earlier, after the transaction commits.
    pub fn publish(mut self) -> Result<(), PayloadSendError> {
        self.sender.flush();
        let bytes = self.bytes.take().expect("prepared bytes");
        let To::Pool(outbox, _) = &self.sender.to else {
            unreachable!()
        };
        if outbox.publish(bytes, self.cost) {
            Ok(())
        } else {
            Err(PayloadSendError::Disconnected)
        }
    }
}
impl Drop for PreparedPayload {
    fn drop(&mut self) {
        if self.bytes.is_some() {
            self.sender.pending.fetch_sub(self.cost, Ordering::Relaxed);
        }
    }
}

/// Hand every player's held packets to their network threads, waking each
/// thread once: a wake-up costs a system call, and on a busy machine often
/// the rest of the core's time slice. Each player's packets keep their order,
/// also with what is sent to it next.
pub fn flush_all<'a>(senders: impl IntoIterator<Item = &'a PlayerSender>) {
    let mut wake: Vec<&Arc<io_pool::Worker>> = Vec::new();
    for s in senders {
        let held = std::mem::take(&mut *s.held());
        if held.is_empty() {
            continue;
        }
        match &s.to {
            To::Disabled => {}
            To::Channel(tx) => drop(tx.send(unshared(held))),
            To::Pool(outbox, _) => {
                if let Some(worker) = outbox.push(held, None)
                    && !wake.iter().any(|w| Arc::ptr_eq(w, worker))
                {
                    wake.push(worker);
                }
            }
        }
    }
    wake.into_iter().for_each(|w| w.wake());
}

/// LegacyQueryHandler: the pre-1.7 server list ping. What the first read
/// holds decides the reply (1.3 and older: no more bytes; 1.4 and 1.5: 0x01;
/// 1.6: 0x01 then the MC|PingHost payload); anything else gets no reply.
fn serve_legacy(mut stream: TcpStream, shared: &Shared) -> Result<(), String> {
    use std::io::{Read, Write};
    let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
    let mut data = Vec::new();
    let mut chunk = [0u8; 1024];
    // The bytes of the first read: what arrived together.
    if let Ok(n) = stream.read(&mut chunk) {
        data.extend_from_slice(&chunk[..n]);
    }
    let in_string = |d: &[u8], at: &mut usize| -> Option<String> {
        let len = usize::from(u16::from_be_bytes([*d.get(*at)?, *d.get(*at + 1)?]));
        let bytes = d.get(*at + 2..*at + 2 + len * 2)?;
        *at += 2 + len * 2;
        Some(String::from_utf16_lossy(
            &bytes
                .chunks(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect::<Vec<_>>(),
        ))
    };
    let motd = shared.motd();
    let (online, max) = (shared.online(), shared.max_players());
    let body = if data.len() == 1 {
        format!("{motd}\u{a7}{online}\u{a7}{max}")
    } else {
        if data.get(1) != Some(&1) {
            return Ok(());
        }
        if data.len() > 2 {
            // readCustomPayloadPacket.
            let mut at = 3;
            let valid = data[2] == 0xFA
                && in_string(&data, &mut at).as_deref() == Some("MC|PingHost")
                && data
                    .get(at..at + 2)
                    .map(|b| usize::from(u16::from_be_bytes([b[0], b[1]])))
                    == Some(data.len().saturating_sub(at + 2))
                && {
                    at += 2;
                    data.get(at).is_some_and(|v| *v >= 73)
                }
                && {
                    at += 1;
                    in_string(&data, &mut at).is_some()
                }
                && data
                    .get(at..at + 4)
                    .is_some_and(|b| i32::from_be_bytes([b[0], b[1], b[2], b[3]]) <= 65535);
            if !valid {
                return Ok(());
            }
        }
        format!(
            "\u{a7}1\0127\0{}\0{motd}\0{online}\0{max}",
            horizon_core::VANILLA_VERSION
        )
    };
    let units: Vec<u16> = body.encode_utf16().collect();
    let mut out = vec![0xFF];
    out.extend_from_slice(&(units.len() as u16).to_be_bytes());
    for u in units {
        out.extend_from_slice(&u.to_be_bytes());
    }
    stream.write_all(&out).map_err(|e| e.to_string())?;
    let _ = stream.shutdown(std::net::Shutdown::Both);
    Ok(())
}

/// Serve an accepted connection, which holds `counted`'s place among the
/// server's until it closes.
pub(super) fn serve(
    stream: TcpStream,
    address: std::net::SocketAddr,
    shared: Arc<Shared>,
    counted: super::Counted,
) -> Result<(), String> {
    let e = |e: horizon_protocol::Error| e.to_string();
    // LegacyQueryHandler, ahead of the frame decoder while status is on.
    let mut first = [0u8; 1];
    if shared.enable_status() && stream.peek(&mut first).is_ok_and(|n| n == 1) && first[0] == 0xFE {
        return serve_legacy(stream, &shared);
    }
    // A peer that stops reading (a frozen game, a dropped network) would otherwise hold
    // its writer in a full send buffer for good, or until TCP gives up minutes later, and
    // its thread and its place among the server's connections with it.
    let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
    let c = Connection::from_stream(stream).map_err(e)?;
    serve_connection(c, address, shared, Some(counted))
}

/// How long one write may wait on a peer that reads nothing: as long as a read may wait.
const WRITE_TIMEOUT: Duration = READ_TIMEOUT;

/// The address a memory connection logs in from: no network peer has port 0.
pub const MEMORY_ADDRESS: std::net::SocketAddr = std::net::SocketAddr::V4(
    std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 0),
);

/// `ServerConnectionListener.startMemoryChannel`: an integrated server's
/// own player, over one end of an in-memory pipe.
pub fn serve_local(
    pipe: horizon_protocol::connection::Pipe,
    shared: Arc<Shared>,
) -> Result<(), String> {
    serve_connection(Connection::local(pipe), MEMORY_ADDRESS, shared, None)
}

fn serve_connection(
    mut c: Connection,
    address: std::net::SocketAddr,
    shared: Arc<Shared>,
    counted: Option<super::Counted>,
) -> Result<(), String> {
    let e = |e: horizon_protocol::Error| e.to_string();
    c.reader.set_read_timeout(Some(LOGIN_TIMEOUT)).map_err(e)?;
    let (id, payload) = c.recv().map_err(e)?;
    // Proxy forwarding carries data after the host (for example Floodgate's).
    let forwarding = shared.config.forwarding_secret.is_some()
        && shared.config.online.is_none()
        && !c.is_memory();
    let intention = if forwarding {
        handshake::decode_intention(id, &payload, handshake::PROXY_HOST_LIMIT).map_err(e)?
    } else {
        match handshake::Serverbound::decode(id, &payload).map_err(e)? {
            handshake::Serverbound::Intention(i) => i,
            _ => return Err("expected a handshake".into()),
        }
    };
    match intention.intent {
        // ServerHandshakePacketListenerImpl: a server that does not reply
        // to status requests closes the connection.
        handshake::STATUS if !shared.enable_status() => Ok(()),
        handshake::STATUS => serve_status(&mut c, &shared, intention.protocol),
        handshake::TRANSFER if !shared.accepts_transfers() => {
            login_disconnect(
                &mut c,
                &translatable("multiplayer.disconnect.transfers_disabled", &[]),
            );
            Ok(())
        }
        handshake::LOGIN | handshake::TRANSFER => {
            if let Some(version) = older_client(&shared, intention.protocol) {
                c.serve_older(version);
            } else if intention.protocol != PROTOCOL_VERSION {
                let key = if intention.protocol < 754 {
                    "multiplayer.disconnect.outdated_client"
                } else {
                    "multiplayer.disconnect.incompatible"
                };
                let json = format!(
                    "{{\"translate\":\"{key}\",\"with\":[\"{}\"]}}",
                    horizon_protocol::GAME_VERSION
                );
                let _ = c.send(
                    &login::Clientbound::Disconnect(login::Disconnect { reason_json: json })
                        .encode(),
                );
                return Ok(());
            }
            let typed = typed_address(&intention, c.is_memory());
            serve_login(c, address, typed, &shared, counted)
        }
        other => Err(format!("unknown intent {other}")),
    }
}

/// The host and port a client typed (`ClientIntentionPacket`), the host cut at its first NUL,
/// where a proxy appends its own data (Floodgate's, Forge's marker). None for a memory
/// connection, whose client typed nothing, and for an empty host.
fn typed_address(intention: &handshake::Intention, memory: bool) -> Option<(Arc<str>, u16)> {
    let host = intention.host.split('\0').next().unwrap_or_default();
    (!memory && !host.is_empty()).then(|| (host.into(), intention.port))
}

/// The older release a client's handshake states, when this server lets it join
/// (`horizon-accept-older-clients`): 26.2 down to 1.20.5. The client-direction adapter
/// also knows older releases whose server-direction layouts are not implemented.
fn older_client(
    shared: &Shared,
    protocol: i32,
) -> Option<horizon_protocol::adapter::ServerVersion> {
    use horizon_protocol::adapter::ServerVersion;
    let version = ServerVersion::from_protocol(protocol)?;
    (shared.config.accept_older_clients
        && version.at_most(ServerVersion::V26_2)
        && !version.at_most(ServerVersion::V1_20_3))
    .then_some(version)
}

fn serve_status(c: &mut Connection, shared: &Shared, protocol: i32) -> Result<(), String> {
    let e = |e: horizon_protocol::Error| e.to_string();
    let mut requested = false;
    loop {
        let (id, payload) = c.recv().map_err(e)?;
        match status::Serverbound::decode(id, &payload).map_err(e)? {
            // ServerStatusPacketListenerImpl: a second request disconnects.
            status::Serverbound::StatusRequest(_) if requested => return Ok(()),
            status::Serverbound::StatusRequest(_) => {
                requested = true;
                // A client this server lets join is told its own protocol, so its list shows it joinable.
                let json = match older_client(shared, protocol) {
                    Some(_) => shared.status_json_for(protocol),
                    None => shared.status_json(),
                };
                c.send(
                    &status::Clientbound::StatusResponse(status::StatusResponse { json }).encode(),
                )
                .map_err(e)?;
            }
            status::Serverbound::PingRequest(p) => {
                c.send(&status::Clientbound::PongResponse(p).encode())
                    .map_err(e)?;
                return Ok(());
            }
            status::Serverbound::Unknown(..) => return Ok(()),
        }
    }
}

/// ServerLoginPacketListenerImpl.getUserName: the name and the address
/// (Connection.getLoggableAddress, `/ip:port` as InetSocketAddress prints it).
fn user_name(name: &str, address: std::net::SocketAddr, log: &super::NetLog) -> String {
    if log.log_ips {
        format!("{name} (/{address})")
    } else {
        format!("{name} (<ip address withheld>)")
    }
}

/// ServerLoginPacketListenerImpl.disconnect: log the reason's text and send
/// it. Returns the text, for the "lost connection" line that follows.
fn refuse(
    c: &mut Connection,
    shared: &Shared,
    thread: &str,
    user: &str,
    reason: &horizon_core::nbt::Tag,
) -> String {
    let text = log_disconnect(shared, thread, user, reason);
    login_disconnect(c, reason);
    text
}

/// The "Disconnecting" log line of a refusal; returns the reason's text.
fn log_disconnect(
    shared: &Shared,
    thread: &str,
    user: &str,
    reason: &horizon_core::nbt::Tag,
) -> String {
    let text = shared
        .config
        .log
        .as_ref()
        .map(|log| horizon_protocol::text::plain(reason, &log.lang))
        .unwrap_or_default();
    if let Some(log) = &shared.config.log {
        (log.write)(thread, "INFO", &format!("Disconnecting {user}: {text}"));
    }
    text
}

/// ServerCommonPacketListenerImpl.handleCookieResponse in the configuration state: this server
/// sends no cookie request there, so an answer is unexpected (DISCONNECT_UNEXPECTED_QUERY) and
/// ends the connection with a configuration disconnect.
fn refuse_cookie_in_configuration(
    c: &mut ConfigIo<'_>,
    shared: &Shared,
    user: &str,
) -> Result<Option<Parts>, String> {
    let reason = translatable("multiplayer.disconnect.unexpected_query_response", &[]);
    let text = log_disconnect(shared, "Server thread", user, &reason);
    // Through ConfigIo: with a payload manifest the queued writer owns the socket.
    c.disconnect(reason);
    lost(shared, user, &text);
    Ok(None)
}

/// ServerLoginPacketListenerImpl.onDisconnect, once the server thread sees
/// the connection closed: by a refusal, or by the client ("Disconnected").
fn lost(shared: &Shared, user: &str, text: &str) {
    if let Some(log) = &shared.config.log {
        (log.write)(
            "Server thread",
            "INFO",
            &format!("{user} lost connection: {text}"),
        );
    }
}

/// UNIQUE_THREAD_ID: the User Authenticator threads are numbered from 1.
static AUTHENTICATORS: AtomicU32 = AtomicU32::new(0);

/// ServerLoginPacketListenerImpl.handleKey and the User Authenticator
/// thread: check the client's answer to the encryption request, encrypt the
/// connection, and ask the session service whether the player joined with
/// this server's hash. The profile, or the text of the refusal.
fn authenticate(
    c: &mut Connection,
    shared: &Shared,
    online: &super::OnlineMode,
    name: &str,
    address: std::net::SocketAddr,
    user: &str,
) -> Result<Result<(u128, Vec<login::Property>), String>, String> {
    let e = |e: horizon_protocol::Error| e.to_string();
    let mut challenge = [0u8; 4];
    horizon_protocol::crypto::random_bytes(&mut challenge).map_err(e)?;
    c.send(
        &login::Clientbound::Hello(login::Hello {
            server_id: String::new(),
            public_key: online.public_der.clone(),
            challenge: challenge.to_vec(),
            should_authenticate: true,
        })
        .encode(),
    )
    .map_err(e)?;
    let (id, payload) = c.recv().map_err(e)?;
    let key = match login::Serverbound::decode(id, &payload).map_err(e)? {
        login::Serverbound::Key(k) => k,
        _ => return Err("Unexpected packet during login".into()),
    };
    // ServerboundKeyPacket.isChallengeValid and getSecretKey.
    let protocol_error = || "Protocol error".to_owned();
    if online.key.decrypt(&key.challenge).ok().as_deref() != Some(&challenge[..]) {
        return Err(protocol_error());
    }
    let secret: [u8; 16] = online
        .key
        .decrypt(&key.key)
        .ok()
        .and_then(|s| s.try_into().ok())
        .ok_or_else(protocol_error)?;
    let digest = server_hash("", &secret, &online.public_der);
    c.enable_encryption(&secret);
    let thread = format!(
        "User Authenticator #{}",
        AUTHENTICATORS.fetch_add(1, Ordering::Relaxed) + 1
    );
    let log = |level: &str, line: &str| {
        if let Some(log) = &shared.config.log {
            (log.write)(&thread, level, line);
        }
    };
    let ip = online.prevent_proxy.then(|| address.ip());
    match crate::auth::has_joined(name, &digest, ip) {
        Ok(Some(profile)) => {
            log(
                "INFO",
                &format!("UUID of player {name} is {}", uuid_string(profile.uuid)),
            );
            Ok(Ok((profile.uuid, profile.properties)))
        }
        Ok(None) => {
            let text = refuse(
                c,
                shared,
                &thread,
                user,
                &translatable("multiplayer.disconnect.unverified_username", &[]),
            );
            log(
                "ERROR",
                &format!("Username '{name}' tried to join with an invalid session"),
            );
            Ok(Err(text))
        }
        Err(_) => {
            let text = refuse(
                c,
                shared,
                &thread,
                user,
                &translatable("multiplayer.disconnect.authservers_down", &[]),
            );
            log(
                "ERROR",
                "Couldn't verify username because servers are unavailable",
            );
            Ok(Err(text))
        }
    }
}

fn serve_login(
    mut c: Connection,
    address: std::net::SocketAddr,
    typed: Option<(Arc<str>, u16)>,
    shared: &Arc<Shared>,
    counted: Option<super::Counted>,
) -> Result<(), String> {
    let e = |e: horizon_protocol::Error| e.to_string();
    let (id, payload) = c.recv().map_err(e)?;
    let hello = match login::Serverbound::decode(id, &payload).map_err(e)? {
        login::Serverbound::Hello(h) => h,
        _ => return Err("expected a login hello".into()),
    };
    if !valid_name(&hello.name) {
        // handleHello's validState throws; Connection.exceptionCaught sends the
        // exception as the login disconnect.
        login_disconnect(
            &mut c,
            &translatable(
                "disconnect.genericReason",
                &[
                    "Internal Exception: java.lang.IllegalStateException: Invalid characters in username",
                ],
            ),
        );
        return Ok(());
    }
    let (mut name, mut address) = (hello.name.clone(), address);
    let user_of = |name: &str, address| {
        shared
            .config
            .log
            .as_ref()
            .map(|log| user_name(name, address, log))
            .unwrap_or_default()
    };
    let mut user = user_of(&name, address);
    // handleHello: online mode authenticates first; offline mode, and the
    // integrated server's own player through memory, derive the profile
    // from the name. Behind a Velocity proxy (forwarding.secret) the proxy has
    // authenticated the player and the profile is the one it forwards.
    let online = shared.config.online.as_ref().filter(|_| !c.is_memory());
    let forwarding = shared
        .config
        .forwarding_secret
        .as_deref()
        .filter(|_| online.is_none() && !c.is_memory());
    // `ServerLoginPacketListenerImpl.handleHello`: the integrated server's own player
    // (the name matches `getSingleplayerProfile`) logs in as that profile — the account's
    // UUID — never as an offline profile made from the name. Every other memory player
    // (a splitscreen seat) stays offline; a LAN guest could not own the name in online
    // mode, and this keeps an offline host's name from being taken over a memory room.
    let singleplayer = shared
        .config
        .singleplayer_profile
        .as_ref()
        .filter(|(owner_name, _)| c.is_memory() && hello.name.eq_ignore_ascii_case(owner_name))
        .map(|(owner_name, uuid)| (owner_name.clone(), *uuid));
    let (uuid, properties) = match (online, forwarding) {
        (Some(online), _) => {
            match authenticate(&mut c, shared, online, &hello.name, address, &user) {
                Ok(Ok(profile)) => profile,
                Ok(Err(refused)) => {
                    lost(shared, &user, &refused);
                    return Ok(());
                }
                Err(err) => {
                    lost(shared, &user, "Disconnected");
                    return Err(err);
                }
            }
        }
        (None, Some(secret)) => match forward(&mut c, secret) {
            Ok(Ok(forwarded)) => {
                (name, address) = (
                    forwarded.name,
                    std::net::SocketAddr::new(forwarded.address, address.port()),
                );
                user = user_of(&name, address);
                (forwarded.uuid, forwarded.properties)
            }
            Ok(Err(reason)) => {
                let text = "This server is behind a proxy: join through the proxy.";
                let json = format!("{{\"text\":\"{text}\"}}");
                let _ = c.send(
                    &login::Clientbound::Disconnect(login::Disconnect { reason_json: json })
                        .encode(),
                );
                if let Some(log) = &shared.config.log {
                    (log.write)(
                        "Server thread",
                        "WARN",
                        &format!("Refused {user}: {reason}"),
                    );
                }
                lost(shared, &user, text);
                return Ok(());
            }
            Err(err) => {
                lost(shared, &user, "Disconnected");
                return Err(err);
            }
        },
        (None, None) => match singleplayer {
            Some((owner_name, owner_uuid)) => {
                name = owner_name;
                let properties = shared
                    .config
                    .owner_profile
                    .as_ref()
                    .map(|profile| profile.properties().to_vec())
                    .unwrap_or_default();
                (owner_uuid, properties)
            }
            None => (offline_uuid(&hello.name), Vec::new()),
        },
    };
    // verifyLoginAndFinishConnectionSetup: PlayerList.canPlayerLogin, then
    // duplicates and a full server.
    if let Some(reason) = shared.try_admit(address, uuid, &name) {
        let text = refuse(&mut c, shared, "Server thread", &user, &reason);
        lost(shared, &user, &text);
        return Ok(());
    }
    // A player a proxy said would arrive (`player-expect`) uses the expectation up
    // here, as the login starts: one that ran out while the player was still in
    // configuration (a resource pack prompt) would otherwise make it an ordinary
    // login from this server's own file.
    let expected = shared.config.arrivals.as_ref().and_then(|table| {
        table
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take_until(uuid, std::time::Instant::now())
    });
    // A login that does not reach play (it failed in configuration) gives the
    // expectation back, so the proxy's next attempt is still expected.
    let put_back = || {
        if let (Some(deadline), Some(table)) = (expected, shared.config.arrivals.as_ref()) {
            table.lock().unwrap_or_else(|e| e.into_inner()).put_back(
                uuid,
                deadline,
                std::time::Instant::now(),
            );
        }
    };
    let result = login_and_configure(&mut c, shared, uuid, &name, &user, properties);
    match result {
        Ok(Some(parts)) => {
            play(
                c,
                shared,
                parts,
                address,
                typed,
                counted,
                expected.is_some(),
            );
            Ok(())
        }
        Ok(None) => {
            put_back();
            shared.release(uuid);
            Ok(())
        }
        Err(err) => {
            put_back();
            shared.release(uuid);
            // ServerConfigurationPacketListenerImpl.onDisconnect.
            let profile = format!("{name} ({})", uuid_string(uuid));
            lost(shared, &profile, "Disconnected");
            Err(err)
        }
    }
}

/// Asks the proxy for the player's forwarded information (login plugin channel `velocity:player_info`) and
/// checks it against the shared secret. The outer error is the connection failing; the inner one is a refusal
/// (the client isn't a proxy, or signed with another secret), with the reason for the log.
fn forward(
    c: &mut Connection,
    secret: &[u8],
) -> Result<Result<forwarding::Forwarded, String>, String> {
    const TRANSACTION: i32 = 1;
    let e = |e: horizon_protocol::Error| e.to_string();
    c.send(
        &login::Clientbound::CustomQuery(login::CustomQuery {
            transaction: TRANSACTION,
            channel: forwarding::CHANNEL.into(),
            data: forwarding::request(),
        })
        .encode(),
    )
    .map_err(e)?;
    let (id, payload) = c.recv().map_err(e)?;
    match login::Serverbound::decode(id, &payload).map_err(e)? {
        login::Serverbound::CustomQueryAnswer(answer) if answer.transaction == TRANSACTION => {
            Ok(match answer.payload {
                Some(data) => forwarding::verify(secret, &data),
                None => Err("the client didn't answer the forwarding request".into()),
            })
        }
        _ => Err("unexpected packet during login".into()),
    }
}

struct Parts {
    payload_transport: Arc<Transport>,
    uuid: u128,
    name: String,
    properties: Vec<login::Property>,
    info: ClientInformation,
    brand: String,
    datapack_offer: Option<Vec<u8>>,
    channels: super::config_payloads::EngineChannels,
}

fn login_and_configure(
    c: &mut Connection,
    shared: &Arc<Shared>,
    uuid: u128,
    name: &str,
    user: &str,
    properties: Vec<login::Property>,
) -> Result<Option<Parts>, String> {
    let e = |e: horizon_protocol::Error| e.to_string();
    // ServerLoginPacketListenerImpl: memory connections are not compressed.
    let threshold = shared.config.compression_threshold;
    if threshold >= 0 && !c.is_memory() {
        c.send(&login::Clientbound::Compression(login::Compression { threshold }).encode())
            .map_err(e)?;
        c.set_compression(threshold);
    }
    c.send(
        &login::Clientbound::LoginFinished(login::LoginFinished {
            profile: login::GameProfile {
                uuid,
                name: name.to_owned(),
                properties: properties.clone(),
            },
            session: uuid,
        })
        .encode(),
    )
    .map_err(e)?;
    let (id, payload) = c.recv().map_err(e)?;
    match login::Serverbound::decode(id, &payload).map_err(e)? {
        login::Serverbound::LoginAcknowledged(_) => {}
        // handleCustomQueryPacket and handleCookieResponse: this server sends no query and no
        // cookie request here, so an answer is unexpected (DISCONNECT_UNEXPECTED_QUERY).
        login::Serverbound::CustomQueryAnswer(_) | login::Serverbound::CookieResponse(_) => {
            let reason = translatable("multiplayer.disconnect.unexpected_query_response", &[]);
            let text = refuse(c, shared, "Server thread", user, &reason);
            lost(shared, user, &text);
            return Ok(None);
        }
        _ => return Err("unexpected packet during login".into()),
    }
    c.reader.set_read_timeout(Some(READ_TIMEOUT)).map_err(e)?;
    // Configuration.
    let payload_transport = Transport::new(shared.payload_local.clone(), Side::Server);
    let mut config_io =
        ConfigIo::new(c, payload_transport.clone(), shared.events.clone()).map_err(e)?;
    let c = &mut config_io;
    let sync_data = shared.sync();
    let sync: &SyncData = &sync_data;
    let send = |c: &mut ConfigIo<'_>, p: cfg::Clientbound| {
        if let cfg::Clientbound::Disconnect(packet) = p {
            c.disconnect(packet.reason);
            return Ok(());
        }
        p.try_encode().and_then(|body| c.send(&body)).map_err(e)
    };
    send(
        c,
        cfg::Clientbound::CustomPayload(cfg::CustomPayload {
            channel: "minecraft:brand".into(),
            data: {
                let mut w = horizon_protocol::buf::Writer::new();
                // MinecraftServer.getServerModName: "vanilla" there; a modded
                // server names itself, and Horizon's says "horizon", as its client does.
                w.string("horizon");
                w.data
            },
        }),
    )?;
    // ServerConfigurationPacketListenerImpl.startConfiguration: links, only
    // when there are any (MinecraftServer.serverLinks is empty elsewhere).
    if !shared.config.server_links.is_empty() {
        send(
            c,
            cfg::Clientbound::ServerLinks(cfg::ServerLinks {
                links: shared.config.server_links.clone(),
            }),
        )?;
    }
    send(
        c,
        cfg::Clientbound::UpdateEnabledFeatures(cfg::UpdateEnabledFeatures {
            features: sync.features.clone(),
        }),
    )?;
    let mut payload_negotiation = horizon_protocol::payload_negotiation::Negotiation::new(
        payload_transport.local.manifest.clone(),
    );
    // This build's name goes first, so a Horizon client can name it if the negotiation fails
    // (crate::build_mismatch). A vanilla client discards it, as it discards the Query.
    send(
        c,
        cfg::Clientbound::CustomPayload(cfg::CustomPayload {
            channel: crate::build_mismatch::CHANNEL.into(),
            data: crate::build_mismatch::encode(&crate::build_mismatch::this_build()),
        }),
    )?;
    // Query must precede known packs even for empty installations: a client
    // answers it before its pack reply, so the barrier cannot overtake its Offer.
    send_payload_control(c, horizon_protocol::payload_negotiation::Control::Query)?;
    // Horizon clients announce their list after recognizing our brand, before
    // answering known packs. That reply orders the announcement without a ping.
    let server_mods = horizon_protocol::mod_list::local();
    let mut client_mods = Vec::new();
    let mut datapack_offer = None;
    let core = KnownPack {
        namespace: CORE_PACK.0.into(),
        id: CORE_PACK.1.into(),
        version: CORE_PACK.2.into(),
    };
    send(
        c,
        cfg::Clientbound::SelectKnownPacks(cfg::SelectKnownPacks {
            packs: vec![core.clone()],
        }),
    )?;
    let mut info = ClientInformation::new(10);
    let mut brand = String::from("vanilla");
    let known_pack = loop {
        let (id, payload) = c.recv().map_err(e)?;
        let Some(packet) = receive_configuration(
            c,
            id,
            &payload,
            &payload_transport,
            &mut payload_negotiation,
        )?
        else {
            continue;
        };
        match packet {
            cfg::Serverbound::ClientInformation(i) => info = i,
            cfg::Serverbound::CustomPayload(p) if p.channel == "minecraft:brand" => {
                brand = horizon_protocol::buf::Reader::new(&p.data)
                    .string(32767)
                    .unwrap_or(brand);
                if brand == "horizon" {
                    // The query was already sent before known packs.
                    let _ = payload_negotiation.require_protocol();
                    send(
                        c,
                        cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                            channel: horizon_protocol::mod_list::CHANNEL.into(),
                            data: horizon_protocol::mod_list::encode(&server_mods),
                        }),
                    )?;
                }
            }
            cfg::Serverbound::CustomPayload(p)
                if p.channel == horizon_protocol::mod_list::CHANNEL =>
            {
                match horizon_protocol::mod_list::decode(&p.data) {
                    Ok(list) => client_mods = list,
                    Err(error) => {
                        let text = mod_list_unreadable(&error);
                        let reason = super::data::component_nbt(&text.component());
                        let _ = send(c, cfg::Clientbound::Disconnect(cfg::Disconnect { reason }));
                        lost(
                            shared,
                            &format!("{name} ({})", uuid_string(uuid)),
                            &text.plain(),
                        );
                        return Ok(None);
                    }
                }
            }
            cfg::Serverbound::CustomPayload(p)
                if p.channel == crate::datapack_net::OFFER_CHANNEL =>
            {
                datapack_offer = Some(p.data);
            }
            cfg::Serverbound::SelectKnownPacks(p) => {
                // A Horizon client that never answered predates the negotiation.
                if let Err(error) = payload_negotiation.finish_without_offer() {
                    return Err(negotiation_disconnect(c, error));
                }
                if payload_transport.epoch().is_none()
                    && let Err(error) =
                        payload_transport.accept(payload_negotiation.accepted().unwrap_or_default())
                {
                    return Err(payload_disconnect(c, error));
                }
                break p.packs == vec![core.clone()];
            }
            cfg::Serverbound::CookieResponse(_) => {
                return refuse_cookie_in_configuration(c, shared, user);
            }
            _ => {}
        }
    };
    // An older client's packet ids are translated, not yet what is inside them (#10404): the registry data,
    // tags and play packets would reach it in 26.3's ids and layouts, so it is turned away before them.
    if let Some(version) = c.older_client()
        && !shared.config.older_clients_play
    {
        let text = super::OLDER_CLIENTS_UNFINISHED;
        let reason = horizon_core::nbt::Tag::String(horizon_core::nbt::JavaString::from(text));
        let _ = send(c, cfg::Clientbound::Disconnect(cfg::Disconnect { reason }));
        if let Some(log) = &shared.config.log {
            let line = format!(
                "Disconnecting {name}: its client is on protocol {} ({text})",
                version.protocol()
            );
            (log.write)("Server thread", "INFO", &line);
        }
        lost(shared, &format!("{name} ({})", uuid_string(uuid)), text);
        return Ok(None);
    }
    // A Horizon client hears that this server runs its mods' quiet commands
    // (horizon_protocol::quiet); a vanilla one is told nothing.
    if brand == "horizon" {
        send(
            c,
            cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                channel: horizon_protocol::quiet::CHANNEL.into(),
                data: Vec::new(),
            }),
        )?;
        // Advertise private custom-book synchronization independently of mod registration.
        send(
            c,
            cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                channel: horizon_protocol::custom_book::CHANNEL.into(),
                data: horizon_protocol::custom_book::CAPABILITY.to_vec(),
            }),
        )?;
        c.ask_viewed_levels().map_err(e)?;
        c.ask_menu_extra_rev().map_err(e)?;
    }
    // And how far it samples far terrain for it (crate::lod_net).
    if brand == "horizon" && shared.config.lod_distance > 0 {
        send(
            c,
            cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                channel: crate::lod_net::CHANNEL.into(),
                data: crate::lod_net::Offer::new(shared.config.lod_distance).encode(),
            }),
        )?;
    }
    let difference = horizon_protocol::mod_list::compare(&server_mods, &client_mods);
    if !difference.is_empty() {
        let text = mod_mismatch(&difference);
        let reason = super::data::component_nbt(&text.component());
        let _ = send(c, cfg::Clientbound::Disconnect(cfg::Disconnect { reason }));
        lost(
            shared,
            &format!("{name} ({})", uuid_string(uuid)),
            &text.plain(),
        );
        return Ok(None);
    }
    // Custom component and statistic IDs need Horizon's registry check.
    // Operators may require Horizon clients without either extension.
    if brand != "horizon"
        && (shared.config.horizon_clients_only || crate::content_identity::requires_horizon())
    {
        let text = if shared.config.horizon_clients_only {
            super::HORIZON_CLIENTS_ONLY_REASON
        } else {
            crate::content_identity::MISMATCH
        };
        let reason = horizon_core::nbt::Tag::String(horizon_core::nbt::JavaString::from(text));
        let _ = send(c, cfg::Clientbound::Disconnect(cfg::Disconnect { reason }));
        if let Some(log) = &shared.config.log {
            let line =
                format!("Disconnecting {name}: its client is \"{brand}\", not Horizon ({text})");
            (log.write)("Server thread", "INFO", &line);
        }
        lost(shared, &format!("{name} ({})", uuid_string(uuid)), text);
        return Ok(None);
    }
    if brand == "horizon"
        && let Some(hash) = sync.content_identity
    {
        send(
            c,
            cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                channel: crate::content_identity::CHANNEL.into(),
                data: hash.to_vec(),
            }),
        )?;
        send(c, cfg::Clientbound::Ping(cfg::Ping { id: 0 }))?;
        let mut content_match = false;
        loop {
            let (id, payload) = c.recv().map_err(e)?;
            let Some(packet) = receive_configuration(
                c,
                id,
                &payload,
                &payload_transport,
                &mut payload_negotiation,
            )?
            else {
                continue;
            };
            match packet {
                cfg::Serverbound::CustomPayload(p)
                    if p.channel == crate::content_identity::CHANNEL =>
                {
                    content_match = crate::content_identity::decode_payload(&p.data)
                        .map_err(|error| error.to_string())?
                        == Some(hash);
                }
                cfg::Serverbound::Pong(p) if p.id == 0 => break,
                cfg::Serverbound::ClientInformation(i) => info = i,
                cfg::Serverbound::CookieResponse(_) => {
                    return refuse_cookie_in_configuration(c, shared, user);
                }
                _ => {}
            }
        }
        if !content_match {
            let reason = horizon_core::nbt::Tag::String(horizon_core::nbt::JavaString::from(
                crate::content_identity::MISMATCH,
            ));
            let _ = send(c, cfg::Clientbound::Disconnect(cfg::Disconnect { reason }));
            lost(
                shared,
                &format!("{name} ({})", uuid_string(uuid)),
                crate::content_identity::MISMATCH,
            );
            return Ok(None);
        }
    }
    // The mods' datapack registries this client has, or the reason it cannot play (arismoko/horizon#811).
    match crate::datapack_net::server_step(brand == "horizon", datapack_offer.as_deref()) {
        crate::datapack_net::Step::Nothing => {}
        crate::datapack_net::Step::Send(data) => {
            for data in crate::datapack_net::chunks(&data)? {
                send(
                    c,
                    cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                        channel: crate::datapack_net::CHANNEL.into(),
                        data,
                    }),
                )?;
            }
        }
        crate::datapack_net::Step::Refuse(text) => {
            let reason =
                horizon_core::nbt::Tag::String(horizon_core::nbt::JavaString::from(text.as_str()));
            let _ = send(c, cfg::Clientbound::Disconnect(cfg::Disconnect { reason }));
            lost(shared, &format!("{name} ({})", uuid_string(uuid)), &text);
            return Ok(None);
        }
    }
    for packet in sync.registry_packets(known_pack, &shared.config.levels) {
        send(c, cfg::Clientbound::RegistryData(packet))?;
    }
    send(c, cfg::Clientbound::UpdateTags(sync.tags.clone()))?;
    // addOptionalTasks: the code of conduct task comes first.
    if let Some(text) =
        super::code_of_conduct::pick(&shared.config.code_of_conducts, &info.language)
    {
        accept_code_of_conduct(
            c,
            text,
            &mut info,
            &payload_transport,
            &mut payload_negotiation,
        )?;
    }
    if let Some(pack) = &shared.config.resource_pack
        && let Some(rejected) = offer_pack(
            c,
            pack,
            &mut info,
            &payload_transport,
            &mut payload_negotiation,
        )?
    {
        let (reason, text) = pack_rejected(shared, name, rejected);
        let _ = send(c, cfg::Clientbound::Disconnect(cfg::Disconnect { reason }));
        lost(shared, &format!("{name} ({})", uuid_string(uuid)), &text);
        return Ok(None);
    }
    if let Some(epoch) = payload_transport.epoch()
        && !epoch.wait_main()
    {
        return Err("configuration payload epoch ended".into());
    }
    send(c, cfg::Clientbound::FinishConfiguration(Empty))?;
    loop {
        let (id, payload) = c.recv().map_err(e)?;
        let Some(packet) = receive_configuration(
            c,
            id,
            &payload,
            &payload_transport,
            &mut payload_negotiation,
        )?
        else {
            continue;
        };
        match packet {
            cfg::Serverbound::FinishConfiguration(_) => break,
            cfg::Serverbound::ClientInformation(i) => info = i,
            cfg::Serverbound::CookieResponse(_) => {
                return refuse_cookie_in_configuration(c, shared, user);
            }
            _ => {}
        }
    }
    payload_transport.finish_configuration()?;
    let channels = std::mem::take(&mut c.channels);
    c.finish()?;
    Ok(Some(Parts {
        channels,
        payload_transport,
        uuid,
        name: name.to_owned(),
        properties,
        info,
        brand,
        datapack_offer,
    }))
}

fn send_payload_control(
    c: &mut ConfigIo<'_>,
    control: horizon_protocol::payload_negotiation::Control,
) -> Result<(), String> {
    c.send(
        &cfg::Clientbound::CustomPayload(cfg::CustomPayload {
            channel: horizon_protocol::payload_negotiation::CHANNEL.into(),
            data: control.encode().map_err(|e| e.to_string())?,
        })
        .encode(),
    )
    .map_err(|e| e.to_string())
}
fn payload_disconnect(c: &mut ConfigIo<'_>, error: String) -> String {
    let reason = horizon_core::nbt::Tag::String(error.as_str().into());
    c.disconnect(reason);
    error
}
/// [`payload_disconnect`] for a failed `horizon:payloads` negotiation: the reason names this
/// build and the client's when they differ (crate::build_mismatch).
fn negotiation_disconnect(
    c: &mut ConfigIo<'_>,
    error: horizon_protocol::payload_negotiation::Error,
) -> String {
    let error = error.to_string();
    let this = crate::build_mismatch::this_build();
    match crate::build_mismatch::refusal(Some(&this), c.client_build.as_deref(), &error) {
        Some(text) => {
            c.disconnect(super::data::component_nbt(&text.component()));
            text.plain()
        }
        None => payload_disconnect(c, error),
    }
}

/// Consume transport metadata in every configuration receive loop. Application
/// payloads stay ordered for the configuration dispatcher (not implemented here).
fn receive_configuration(
    c: &mut ConfigIo<'_>,
    id: i32,
    bytes: &[u8],
    transport: &Transport,
    negotiation: &mut horizon_protocol::payload_negotiation::Negotiation,
) -> Result<Option<cfg::Serverbound>, String> {
    let packet = cfg::Serverbound::decode_with_payload_codecs(id, bytes, &transport.local.codecs)
        .map_err(|e| payload_disconnect(c, e.to_string()))?;
    if let cfg::Serverbound::CustomPayload(payload) = &packet
        && payload.channel == crate::build_mismatch::CHANNEL
    {
        c.client_build = crate::build_mismatch::decode(&payload.data);
        return Ok(None);
    }
    if let cfg::Serverbound::CustomPayload(payload) = &packet
        && payload.channel == horizon_protocol::payload_negotiation::CHANNEL
    {
        let result = horizon_protocol::payload_negotiation::Control::decode(&payload.data)
            .and_then(|control| negotiation.receive(control));
        let reply = result.map_err(|e| negotiation_disconnect(c, e))?;
        if let Some(reply) = reply {
            send_payload_control(c, reply)?;
        }
        if let Some(accepted) = negotiation.accepted() {
            transport
                .accept(accepted)
                .map_err(|e| payload_disconnect(c, e))?;
        }
        return Ok(None);
    }
    match packet {
        cfg::Serverbound::CustomPayload(payload) => c
            .application(payload)
            .map(|payload| payload.map(cfg::Serverbound::CustomPayload)),
        packet => Ok(Some(packet)),
    }
}

/// ServerboundResourcePackPacket.Action.DECLINED.
const PACK_DECLINED: i32 = 1;

fn mod_mismatch(difference: &horizon_protocol::mod_list::Difference) -> crate::text::Text {
    use crate::{lang::Language, text::Text};
    // Vanilla has no Horizon keys. Keep English fallbacks on the wire, while a
    // Horizon client resolves each component in its own selected language.
    let english = Language::load(&[], crate::lang::DEFAULT);
    let translated = |key: &str, args: &[Text]| {
        Text::translate_with_fallback(key, english.get(key).unwrap(), args)
    };
    let mut text = translated("horizon.mods.mismatch", &[]);
    for (key, names) in [
        ("horizon.mods.missing", &difference.missing),
        ("horizon.mods.extra", &difference.extra),
    ] {
        if !names.is_empty() {
            text = text
                .append("\n")
                .append(translated(key, &[Text::literal(names.join("\n"))]));
        }
    }
    for changed in &difference.changed {
        text = text.append("\n").append(translated(
            "horizon.mods.version",
            &[
                Text::literal(&changed.id),
                Text::literal(&changed.server),
                Text::literal(&changed.client),
            ],
        ));
    }
    text
}

/// A mod list that breaks a cap is refused whole, never compared in part or
/// read as empty: like a mismatch, the reason reaches the client as a
/// configuration disconnect naming the decode error, and the join never
/// reaches play.
fn mod_list_unreadable(error: &horizon_protocol::Error) -> crate::text::Text {
    use crate::{lang::Language, text::Text};
    let english = Language::load(&[], crate::lang::DEFAULT);
    Text::translate_with_fallback(
        "horizon.mods.unreadable",
        english.get("horizon.mods.unreadable").unwrap(),
        &[Text::literal(error.to_string())],
    )
}

/// ServerCommonPacketListenerImpl.handleResourcePackResponse: a player who
/// declined the required pack is disconnected. The reason and its text.
fn pack_rejected(shared: &Shared, name: &str, pack: u128) -> (horizon_core::nbt::Tag, String) {
    let reason = translatable("multiplayer.requiredTexturePrompt.disconnect", &[]);
    let Some(log) = &shared.config.log else {
        return (reason, String::new());
    };
    let line = format!(
        "Disconnecting {name} due to resource pack {} rejection",
        uuid_string(pack)
    );
    (log.write)("Server thread", "INFO", &line);
    let text = horizon_protocol::text::plain(&reason, &log.lang);
    (reason, text)
}

/// RateKickingConnection: a player whose packets a second passed rate-limit
/// is disconnected ("disconnect.exceeded_packet_rate") with a WARN, as
/// vanilla's kick logs it.
/// The kick for a client whose packets wait past [`Undrained`]'s caps: the same message as the
/// hub's own inbox check, and a line in the log.
fn undrained_exceeded(shared: &Shared, name: &str) -> (horizon_core::nbt::Tag, String) {
    let reason = translatable("disconnect.exceeded_packet_rate", &[]);
    let Some(log) = &shared.config.log else {
        return (reason, String::new());
    };
    let line =
        format!("{name} sent more packets than the server handles before its tick took them");
    (log.write)("Server thread", "WARN", &line);
    let text = horizon_protocol::text::plain(&reason, &log.lang);
    (reason, text)
}

fn rate_exceeded(shared: &Shared, average: f32) -> (horizon_core::nbt::Tag, String) {
    let reason = translatable("disconnect.exceeded_packet_rate", &[]);
    let Some(log) = &shared.config.log else {
        return (reason, String::new());
    };
    let line = format!(
        "Player exceeded rate-limit (sent {} packets per second)",
        horizon_core::java_number::float_to_string(average)
    );
    (log.write)("Server thread", "WARN", &line);
    let text = horizon_protocol::text::plain(&reason, &log.lang);
    (reason, text)
}

/// ServerResourcePackConfigurationTask: offer the server's pack, then wait
/// for the client's final answer (`Action.isTerminal`: anything but
/// ACCEPTED or DOWNLOADED). The player may take a while to answer the
/// prompt, so the connection is kept alive meanwhile, as
/// ServerConfigurationPacketListenerImpl.tick does. The id of a required
/// pack the client declined, which disconnects it.
fn offer_pack(
    c: &mut ConfigIo<'_>,
    pack: &crate::server::properties::ResourcePackInfo,
    info: &mut ClientInformation,
    transport: &Transport,
    negotiation: &mut horizon_protocol::payload_negotiation::Negotiation,
) -> Result<Option<u128>, String> {
    let e = |e: horizon_protocol::Error| e.to_string();
    let push = cfg::ResourcePackPush {
        id: pack.id,
        url: pack.url.clone(),
        hash: pack.hash.clone(),
        required: pack.required,
        prompt: pack.prompt.as_ref().map(component_tag),
    };
    c.send(&cfg::Clientbound::ResourcePackPush(push).encode())
        .map_err(e)?;
    wait_alive(c, transport, negotiation, |packet| match packet {
        Ok(cfg::Serverbound::ResourcePack(r)) if !matches!(r.action, 3 | 4) => {
            let rejected = r.action == PACK_DECLINED && pack.required;
            Some(Ok(rejected.then_some(r.id)))
        }
        Ok(cfg::Serverbound::ClientInformation(i)) => {
            *info = i;
            None
        }
        Ok(_) => None,
        Err(err) => Some(Err(err)),
    })
}

/// ServerCodeOfConductConfigurationTask: send the text, then wait for
/// ServerboundAcceptCodeOfConductPacket (`handleAcceptCodeOfConduct`),
/// keeping the connection alive while the player reads. Finishing the
/// configuration first is `finishCurrentTask` of the wrong task type, an
/// error that drops the connection.
fn accept_code_of_conduct(
    c: &mut ConfigIo<'_>,
    text: &str,
    info: &mut ClientInformation,
    transport: &Transport,
    negotiation: &mut horizon_protocol::payload_negotiation::Negotiation,
) -> Result<(), String> {
    let e = |e: horizon_protocol::Error| e.to_string();
    c.send(&cfg::Clientbound::CodeOfConduct(cfg::CodeOfConduct { text: text.into() }).encode())
        .map_err(e)?;
    wait_alive(c, transport, negotiation, |packet| match packet {
        Ok(cfg::Serverbound::AcceptCodeOfConduct(_)) => Some(Ok(())),
        Ok(cfg::Serverbound::FinishConfiguration(_)) => {
            Some(Err("Unexpected configuration task".to_owned()))
        }
        Ok(cfg::Serverbound::ClientInformation(i)) => {
            *info = i;
            None
        }
        Ok(_) => None,
        Err(err) => Some(Err(err)),
    })
}

/// Read configuration packets until `answer` has a result, sending a
/// keep-alive every `KEEP_ALIVE_EVERY` meanwhile
/// (ServerConfigurationPacketListenerImpl.tick).
fn wait_alive<T>(
    c: &mut ConfigIo<'_>,
    transport: &Transport,
    negotiation: &mut horizon_protocol::payload_negotiation::Negotiation,
    mut answer: impl FnMut(Result<cfg::Serverbound, String>) -> Option<Result<T, String>>,
) -> Result<T, String> {
    if let Some(sender) = c.sender.clone() {
        return std::thread::scope(|scope| {
            let (stop, stopped) = channel::<()>();
            scope.spawn(move || {
                let start = Instant::now();
                while stopped.recv_timeout(KEEP_ALIVE_EVERY) == Err(RecvTimeoutError::Timeout) {
                    let id = i64::try_from(start.elapsed().as_millis()).unwrap_or(0);
                    if sender
                        .send_packet(&cfg::Clientbound::KeepAlive(cfg::KeepAlive { id }).encode())
                        .is_err()
                    {
                        break;
                    }
                }
            });
            let result = loop {
                let (id, payload) = match c.recv() {
                    Ok(packet) => packet,
                    Err(e) => break Err(e.to_string()),
                };
                let packet = match receive_configuration(c, id, &payload, transport, negotiation) {
                    Ok(Some(packet)) => Ok(packet),
                    Ok(None) => continue,
                    Err(e) => Err(e),
                };
                if let Some(result) = answer(packet) {
                    break result;
                }
            };
            drop(stop);
            result
        });
    }
    // The engine's channels live on ConfigIo, which the wait below borrows as a plain connection.
    let mut channels = std::mem::take(&mut c.channels);
    let result = {
        let connection = &mut **c;
        let writer = Mutex::new(&mut connection.writer);
        std::thread::scope(|scope| {
            let (stop, stopped) = channel::<()>();
            let keep_writer = &writer;
            scope.spawn(move || {
                let start = Instant::now();
                while stopped.recv_timeout(KEEP_ALIVE_EVERY) == Err(RecvTimeoutError::Timeout) {
                    let id = i64::try_from(start.elapsed().as_millis()).unwrap_or(0);
                    let keep_alive = cfg::Clientbound::KeepAlive(cfg::KeepAlive { id });
                    if keep_writer
                        .lock()
                        .unwrap()
                        .send(&keep_alive.encode())
                        .is_err()
                    {
                        break;
                    }
                }
            });
            let result = loop {
                let packet = match connection.reader.recv() {
                    Ok((id, payload)) => cfg::Serverbound::decode_with_payload_codecs(
                        id,
                        &payload,
                        &transport.local.codecs,
                    )
                    .map_err(|e| e.to_string()),
                    Err(err) => break Err(err.to_string()),
                };
                if let Ok(cfg::Serverbound::CustomPayload(payload)) = &packet
                    && payload.channel == horizon_protocol::payload_negotiation::CHANNEL
                {
                    let result = (|| {
                        let control =
                            horizon_protocol::payload_negotiation::Control::decode(&payload.data)
                                .map_err(|e| e.to_string())?;
                        if let Some(reply) =
                            negotiation.receive(control).map_err(|e| e.to_string())?
                        {
                            let body = cfg::Clientbound::CustomPayload(
                                horizon_protocol::packets::custom_payload::CustomPayload {
                                    channel: horizon_protocol::payload_negotiation::CHANNEL.into(),
                                    data: reply.encode().map_err(|e| e.to_string())?,
                                },
                            )
                            .encode();
                            writer
                                .lock()
                                .unwrap()
                                .send(&body)
                                .map_err(|e| e.to_string())?;
                        }
                        if let Some(accepted) = negotiation.accepted() {
                            transport.accept(accepted)?;
                        }
                        Ok::<(), String>(())
                    })();
                    if let Err(error) = result {
                        break Err(error);
                    }
                    continue;
                }
                // As receive_configuration does: the engine's channels are read here too.
                if let Ok(cfg::Serverbound::CustomPayload(payload)) = &packet {
                    match channels.receive(payload) {
                        Ok(true) => continue,
                        Ok(false) => {}
                        Err(error) => break Err(error),
                    }
                }
                if let Some(result) = answer(packet) {
                    break result;
                }
            };
            drop(stop);
            result
        })
    };
    c.channels = channels;
    result.map_err(|error| payload_disconnect(c, error))
}

fn play(
    c: Connection,
    shared: &Arc<Shared>,
    parts: Parts,
    address: std::net::SocketAddr,
    typed: Option<(Arc<str>, u16)>,
    counted: Option<super::Counted>,
    expected: bool,
) {
    let Some(conn) = shared.next_id() else {
        shared.release(parts.uuid);
        return;
    };
    shared.entered_play();
    let pending = Arc::new(AtomicUsize::new(0));
    let outbox = match io_pool::Outbox::new(pending.clone(), !c.is_memory()) {
        Ok(outbox) => outbox,
        Err(e) => {
            log::warn!("[lan] {e}");
            shared.release(parts.uuid);
            return;
        }
    };
    let latency = Arc::new(AtomicI32::new(0));
    let watch = Arc::new(KeepAliveWatch {
        start: Instant::now(),
        alive: Arc::new(AtomicI64::new(-1)),
    });
    let undrained = Arc::<Undrained>::default();
    let sender = PlayerSender {
        undrained: undrained.clone(),
        pending,
        held: Arc::default(),
        framing: c.writer.framing(),
        to: To::Pool(outbox.clone(), watch.clone()),
        wire: Some(c.reader.counters.clone()),
        payload_transport: parts.payload_transport.clone(),
        datapack_offer: parts.datapack_offer.map(Into::into),
        announced: parts.channels.announced.clone().unwrap_or_default(),
        viewed_levels: parts.channels.viewed_levels.answer(),
        menu_extra_rev: parts.channels.menu_extra_rev.revision(),
        connect: typed,
    };
    let required_pack = shared
        .config
        .resource_pack
        .as_ref()
        .is_some_and(|p| p.required);
    let session = PlayConn {
        payload_sender: sender.clone(),
        payload_transport: parts.payload_transport,
        conn,
        shared: shared.clone(),
        uuid: parts.uuid,
        name: parts.name.clone(),
        required_pack,
        rate: PacketRate::new(shared.config.rate_limit, Instant::now()),
        latency: latency.clone(),
        sent_at: None,
        next_keep_alive: watch.start + KEEP_ALIVE_EVERY,
        watch,
        counted,
        undrained,
    };
    // The ServerPlayer is made with the configuration's client information (updateOptions).
    shared.set_listed(parts.uuid, parts.info.allows_listing);
    let joined = Joined {
        conn,
        uuid: parts.uuid,
        name: parts.name,
        properties: parts.properties,
        info: parts.info,
        brand: parts.brand,
        sender,
        latency,
        address,
        expected,
    };
    let mut session = Box::new(session);
    if shared
        .events
        .send(NetEvent::Joined(Box::new(joined)))
        .is_err()
    {
        session.left("server closed".to_owned());
        session.closed();
        return;
    }
    let (socket, reader, writer) = c.detach();
    io_pool::adopt(socket, (reader, writer), outbox, session, READ_TIMEOUT);
}

/// A player's play state on its network thread.
struct PlayConn {
    payload_sender: PlayerSender,
    payload_transport: Arc<Transport>,
    conn: u32,
    shared: Arc<Shared>,
    uuid: u128,
    name: String,
    required_pack: bool,
    rate: Option<PacketRate>,
    watch: Arc<KeepAliveWatch>,
    /// Smoothed keep-alive round trip in milliseconds.
    latency: Arc<AtomicI32>,
    /// The keep-alive waiting for its answer, and when it was sent.
    sent_at: Option<(i64, Instant)>,
    next_keep_alive: Instant,
    /// The connection's place among the server's, given back once closed.
    counted: Option<super::Counted>,
    /// Shared with the player's [`PlayerSender`]: what the hub hasn't taken yet.
    undrained: Arc<Undrained>,
}
impl PlayConn {
    /// `IntegratedServer.isSingleplayerOwner`: the integrated server's own player,
    /// by name (ignoring case).
    fn is_owner(&self) -> bool {
        self.shared
            .config
            .singleplayer_profile
            .as_ref()
            .is_some_and(|(owner, _)| owner.eq_ignore_ascii_case(&self.name))
    }
}
impl io_pool::Session for PlayConn {
    /// Forward the player's packet, or say why it ends the connection: a
    /// required resource pack declined, a rate-limit excess, a cookie response
    /// nobody asked for, a malformed packet.
    fn packet(&mut self, id: i32, payload: Vec<u8>) -> io_pool::Verdict {
        use io_pool::Verdict;
        let kick = |(reason, text): (horizon_core::nbt::Tag, String)| {
            let body = play::Clientbound::Disconnect(cfg::Disconnect { reason }).encode();
            Verdict::Kick(text, body)
        };
        if self.payload_transport.closed() {
            return Verdict::Go;
        }
        let now = Instant::now();
        if let Some(average) = self.rate.as_mut().and_then(|r| r.received(now)) {
            return kick(rate_exceeded(&self.shared, average));
        }
        match play::Serverbound::decode_with_payload_codecs(
            id,
            &payload,
            &self.payload_transport.local.codecs,
        )
        .and_then(checked_play)
        {
            // ServerCommonPacketListenerImpl.handleKeepAlive: only the pending one counts.
            Ok(play::Serverbound::KeepAlive(k)) => {
                if let Some((sent, at)) = self.sent_at
                    && k.id == sent
                {
                    self.watch.alive.store(k.id, Ordering::Relaxed);
                    let ms = i32::try_from(at.elapsed().as_millis()).unwrap_or(i32::MAX);
                    let old = self.latency.load(Ordering::Relaxed);
                    self.latency.store((old * 3 + ms) / 4, Ordering::Relaxed);
                    self.sent_at = None;
                    Verdict::Go
                } else if self.is_owner() {
                    Verdict::Go
                } else {
                    kick((
                        translatable("disconnect.timeout", &[]),
                        "Timed out".to_owned(),
                    ))
                }
            }
            // handleUseItemOn and handleUseItem: `ackBlockChangesUpTo` throws on a negative sequence
            // before anything is done, and the server only logs it ("Failed to handle packet ...,
            // suppressing error"): such a packet does nothing, and no player is disconnected for it.
            Ok(
                play::Serverbound::UseItem(play::UseItem { sequence, .. })
                | play::Serverbound::UseItemOn(play::UseItemOn { sequence, .. }),
            ) if sequence < 0 => Verdict::Go,
            Ok(play::Serverbound::ResourcePack(r))
                if r.action == PACK_DECLINED && self.required_pack =>
            {
                kick(pack_rejected(&self.shared, &self.name, r.id))
            }
            // ServerCommonPacketListenerImpl.handleCookieResponse: the server asks for no cookie, so
            // a response is always unexpected, for the singleplayer owner too.
            Ok(play::Serverbound::CookieResponse(_)) => kick((
                translatable("multiplayer.disconnect.unexpected_query_response", &[]),
                "Unexpected custom data from client".to_owned(),
            )),
            Ok(p) => {
                let p = match p {
                    play::Serverbound::CustomPayload(payload) => {
                        match crate::raw_payloads::Incoming::classify(
                            &self.payload_transport,
                            Phase::Play,
                            payload,
                        ) {
                            Ok(crate::raw_payloads::Classified::Application(incoming)) => {
                                let endpoint = crate::raw_payloads::Endpoint::server(
                                    self.payload_sender.clone(),
                                );
                                crate::raw_payloads::dispatch(
                                    incoming,
                                    endpoint.clone(),
                                    crate::raw_payloads::MainQueue::Server {
                                        events: self.shared.events.clone(),
                                        endpoint,
                                        conn: Some(self.conn),
                                    },
                                );
                                return Verdict::Go;
                            }
                            Ok(crate::raw_payloads::Classified::Raw(deliveries)) => {
                                for incoming in deliveries {
                                    let endpoint = crate::raw_payloads::Endpoint::server(
                                        self.payload_sender.clone(),
                                    );
                                    crate::raw_payloads::dispatch(
                                        incoming,
                                        endpoint.clone(),
                                        crate::raw_payloads::MainQueue::Server {
                                            events: self.shared.events.clone(),
                                            endpoint,
                                            conn: Some(self.conn),
                                        },
                                    );
                                }
                                return Verdict::Go;
                            }
                            Ok(crate::raw_payloads::Classified::Other(payload))
                                if self
                                    .payload_sender
                                    .announced
                                    .apply(&payload.channel, &payload.data) =>
                            {
                                return Verdict::Go;
                            }
                            Ok(crate::raw_payloads::Classified::Other(payload)) => {
                                play::Serverbound::CustomPayload(payload)
                            }
                            Err(error) => return Verdict::Close(error.to_string()),
                        }
                    }
                    packet => packet,
                };
                // Bounded here, where packets are made, not only where the hub drains them.
                if !self.undrained.handed(payload.len()) {
                    return kick(undrained_exceeded(&self.shared, &self.name));
                }
                let event = NetEvent::Packet(self.conn, Box::new(p), payload.len());
                match self.shared.events.send(event) {
                    Ok(()) => Verdict::Go,
                    Err(_) => Verdict::Close("server closed".to_owned()),
                }
            }
            // A malformed packet disconnects like vanilla's decoder exception.
            Err(e) => Verdict::Close(format!("Invalid packet {id}: {e}")),
        }
    }
    /// ServerCommonPacketListenerImpl.keepConnectionAlive: a keep-alive 15 s
    /// after the last was answered. The game thread times a client out
    /// ([`PlayerSender::timed_out`]).
    fn tick(&mut self, now: Instant) -> Option<Vec<u8>> {
        self.payload_tick(now)
    }
    fn left(&mut self, reason: String) {
        let _ = self.shared.events.send(NetEvent::Left(self.conn, reason));
        self.shared.closing.fetch_add(1, Ordering::AcqRel);
    }
    fn closed(&mut self) {
        self.payload_transport.close();
        self.shared.closing.fetch_sub(1, Ordering::AcqRel);
        self.shared.release(self.uuid);
        self.counted = None;
    }
}

/// RateKickingConnection: the packets a second a connection averages, as
/// Connection.tickSecond keeps it, against server.properties' rate-limit.
struct PacketRate {
    limit: f32,
    received: u32,
    average: f32,
    next: Instant,
}
impl PacketRate {
    /// The tracker of a connection, or none when the limit is off (0 or less).
    fn new(limit: i32, now: Instant) -> Option<Self> {
        (limit > 0).then(|| Self {
            limit: limit as f32,
            received: 0,
            average: 0.0,
            next: now + Duration::from_secs(1),
        })
    }
    /// A packet arrived. Every whole second the average updates
    /// (Mth.lerp(0.75F, received, average)): over the limit, Some of it.
    fn received(&mut self, now: Instant) -> Option<f32> {
        let exceeded = if now >= self.next {
            // One tick with the count, then a packetless tick's 0.75 decay
            // for each further whole second already gone.
            let first = self.received as f32 + 0.75 * (self.average - self.received as f32);
            self.average = first;
            self.received = 0;
            let elapsed = (now - self.next).as_secs() + 1;
            if elapsed > 1 {
                self.average *= 0.75f32.powi((elapsed - 1) as i32);
            }
            self.next += Duration::from_secs(elapsed);
            (first > self.limit).then_some(first)
        } else {
            None
        };
        self.received = self.received.saturating_add(1);
        exceeded
    }
}

const COMMAND_SUGGESTION: i32 = horizon_protocol::ids::play::serverbound::COMMAND_SUGGESTION;

/// A play packet as vanilla's decoders read it.
fn checked_play(packet: play::Serverbound) -> horizon_protocol::Result<play::Serverbound> {
    match &packet {
        // ServerboundCommandSuggestionPacket: a VarInt id, then the command, at most
        // 32500 chars. The game reads it later; its codec's limits apply here.
        play::Serverbound::Unknown(COMMAND_SUGGESTION, body) => {
            let mut r = horizon_protocol::buf::Reader::new(body);
            r.varint()?;
            r.string(32500)?;
            Ok(packet)
        }
        _ => Ok(packet),
    }
}

/// Packet bodies at least this large are compressed once for every
/// connection: chunk columns, mostly, which every player near them receives.
const SHARED_FRAME_MIN: usize = 8 << 10;
/// The compressed frames kept, in bytes; the oldest go first.
const SHARED_FRAME_BYTES: usize = 32 << 20;

/// Compressed frame contents by the body they were made from: its length,
/// one 128-bit hash of it keyed by a random secret, and the threshold.
struct SharedFrames {
    /// XXH3's secret, random for each run, so that no body can be made to
    /// share its hash with another.
    secret: Box<[u8]>,
    frames: Mutex<FrameCache>,
    /// [`SHARED_FRAME_BYTES`].
    limit: usize,
}
type FrameKey = (usize, u128, usize);
/// A frame's contents, made once: whoever asks while another thread
/// compresses the same body waits for it rather than compressing it again.
type Frame = Arc<std::sync::OnceLock<Result<Arc<[u8]>, String>>>;
#[derive(Default)]
struct FrameCache {
    frames: crate::fast_hash::FxHashMap<FrameKey, Frame>,
    order: std::collections::VecDeque<FrameKey>,
    /// The bytes of the frames made and still here.
    bytes: usize,
}
impl SharedFrames {
    fn get() -> &'static Self {
        static FRAMES: std::sync::OnceLock<SharedFrames> = std::sync::OnceLock::new();
        FRAMES.get_or_init(|| Self::new(SHARED_FRAME_BYTES))
    }
    fn new(limit: usize) -> Self {
        use std::hash::{BuildHasher, Hasher};
        // 192 bytes, as XXH3's default secret.
        let secret = (0..24)
            .flat_map(|i: usize| {
                let mut h = std::hash::RandomState::new().build_hasher();
                h.write_usize(i);
                h.finish().to_le_bytes()
            })
            .collect();
        Self {
            secret,
            frames: Mutex::default(),
            limit,
        }
    }
    fn cache(&self) -> std::sync::MutexGuard<'_, FrameCache> {
        self.frames.lock().unwrap_or_else(PoisonError::into_inner)
    }
    /// The frames kept and the bytes of their contents.
    fn census(&self) -> (usize, usize) {
        let cache = self.cache();
        (cache.frames.len(), cache.bytes)
    }
    /// `frame::contents(body, Some(threshold))`, compressed only when no
    /// connection has sent the same body lately or compresses it now.
    fn contents(&self, body: &[u8], threshold: usize) -> horizon_protocol::Result<Arc<[u8]>> {
        let hash = xxhash_rust::xxh3::xxh3_128_with_secret(body, &self.secret);
        let key = (body.len(), hash, threshold);
        let (frame, made_here) = {
            let mut cache = self.cache();
            match cache.frames.get(&key) {
                Some(frame) => (frame.clone(), false),
                None => {
                    let frame = Frame::default();
                    cache.frames.insert(key, frame.clone());
                    cache.order.push_back(key);
                    (frame, true)
                }
            }
        };
        let contents = frame
            .get_or_init(|| {
                horizon_protocol::frame::contents(body, Some(threshold))
                    .map(Arc::from)
                    .map_err(|e| e.to_string())
            })
            .clone()
            .map_err(horizon_protocol::Error::new)?;
        if made_here {
            let mut cache = self.cache();
            // Unless pushed out while it was being made.
            if cache
                .frames
                .get(&key)
                .is_some_and(|f| Arc::ptr_eq(f, &frame))
            {
                cache.bytes += contents.len();
            }
            while cache.bytes > self.limit
                && let Some(old) = cache.order.pop_front()
            {
                let made = cache.frames.remove(&old);
                cache.bytes -= made
                    .and_then(|f| f.get()?.as_ref().ok().map(|c| c.len()))
                    .unwrap_or(0);
            }
        }
        Ok(contents)
    }
}

/// The compressed frames shared by every connection: how many and their bytes.
pub fn shared_frame_census() -> (usize, usize) {
    SharedFrames::get().census()
}

/// Drop the compressed frames kept for other connections, with their table's
/// memory: nobody is on to send them to. A frame being made is still handed to
/// whoever waits for it.
pub fn forget_shared_frames() {
    *SharedFrames::get().cache() = FrameCache::default();
}

/// Frame one packet body into `out`: a large one compressed through
/// [`SharedFrames`], so a chunk column sent to fifty players is compressed
/// once, not fifty times. The bytes on the wire are the same.
pub(super) fn frame_body(
    writer: &mut ConnWriter,
    body: &[u8],
    out: &mut Vec<u8>,
) -> horizon_protocol::Result<()> {
    // Renumbered first for a client of an older release; one with no such packet gets nothing.
    let Some(body) = writer.for_client(body)? else {
        return Ok(());
    };
    let body = &body[..];
    match writer.framing() {
        horizon_protocol::connection::Framing::Network(Some(t))
            if body.len() >= t.max(SHARED_FRAME_MIN) =>
        {
            writer.frame_into(&SharedFrames::get().contents(body, t)?, out);
        }
        _ => writer.frame_into(&writer.contents(body)?, out),
    }
    Ok(())
}

/// The keep-alive clock of a play connection, shared by its network thread
/// (which sends the keep-alives) and the game thread (which watches them).
struct KeepAliveWatch {
    /// What keep-alive ids count from: an id is its sending time in ms.
    start: Instant,
    /// The id of the last keep-alive the client answered (-1: none yet).
    alive: Arc<AtomicI64>,
}
impl KeepAliveWatch {
    /// keepConnectionAlive sends a keep-alive 15 s after the last and drops
    /// a client that has not answered it 15 s later: nothing answered of
    /// what was sent in the last 30 s.
    fn timed_out(&self) -> bool {
        let answered = u64::try_from(self.alive.load(Ordering::Relaxed)).unwrap_or(0);
        self.start.elapsed() > Duration::from_millis(answered) + 2 * KEEP_ALIVE_EVERY
    }
}

#[cfg(test)]
mod tests {
    include!("profession_join_tests.rs");

    /// Bodies shared between players reach each of them as the same bytes, in the order
    /// they were sent among the player's own.
    #[test]
    fn shared_bodies_reach_every_player_in_order() {
        let shared: Vec<Body> = [vec![1, 2, 3], vec![4]]
            .into_iter()
            .map(Body::shared)
            .collect();
        let players: Vec<_> = (0..2)
            .map(|_| {
                let (tx, rx) = std::sync::mpsc::channel();
                (PlayerSender::new(tx, Arc::default()), rx)
            })
            .collect();
        {
            let _holding = Holding::start();
            for (i, (sender, _)) in players.iter().enumerate() {
                sender.send(vec![9, i as u8]);
                sender.send_many(shared.iter().cloned());
                sender.send_many([vec![8]]);
            }
            flush_all(players.iter().map(|(sender, _)| sender));
        }
        for (i, (_, rx)) in players.iter().enumerate() {
            let Ok(Out::Packets(bodies)) = rx.try_recv() else {
                panic!("expected the held packets");
            };
            assert_eq!(bodies, [vec![9, i as u8], vec![1, 2, 3], vec![4], vec![8]]);
            assert!(rx.try_recv().is_err());
        }
    }

    #[test]
    fn undrained_packets_stop_at_twice_the_inbox() {
        let u = Undrained::default();
        // Up to the cap, counted; one more is refused and counts nothing.
        for _ in 0..Undrained::MAX_PACKETS {
            assert!(u.handed(3));
        }
        assert!(!u.handed(3));
        // The hub taking one gives its place back.
        u.taken(3);
        assert!(u.handed(3));
        // Bytes too: a few large packets reach the byte cap long before the count.
        let big = Undrained::default();
        let size = Undrained::MAX_BYTES / 4;
        for _ in 0..4 {
            assert!(big.handed(size));
        }
        assert!(!big.handed(1));
        big.taken(size);
        assert!(big.handed(size));
    }
    use super::*;

    /// arismoko/horizon#3209: a client may announce its channels while the server waits on a configuration
    /// task (here the code of conduct). With no typed declaration the wait reads packets
    /// itself; the list must still be kept.
    #[test]
    fn channels_announced_during_a_configuration_wait_are_kept() {
        use horizon_protocol::connection::Pipe;
        use horizon_protocol::packets::custom_payload::CustomPayload;
        let (client, server) = Pipe::pair();
        let mut client = Connection::local(client);
        let mut server = Connection::local(server);
        for packet in [
            cfg::Serverbound::CustomPayload(CustomPayload {
                channel: "minecraft:register".into(),
                data: b"voicechat:secret\0bungeecord:main".to_vec(),
            }),
            cfg::Serverbound::CustomPayload(CustomPayload {
                channel: "minecraft:unregister".into(),
                data: b"bungeecord:main".to_vec(),
            }),
            cfg::Serverbound::AcceptCodeOfConduct(horizon_protocol::packets::Empty),
        ] {
            client.send(&packet.encode()).unwrap();
        }
        let transport = Transport::empty();
        let (events, _receiver) = super::super::event_channel_with_capacity(1 << 20);
        let mut io = ConfigIo::new(&mut server, transport.clone(), events).unwrap();
        assert!(io.sender.is_none(), "the wait reads packets itself");
        let mut negotiation = horizon_protocol::payload_negotiation::Negotiation::new(
            transport.local.manifest.clone(),
        );
        let mut info = ClientInformation::new(10);
        accept_code_of_conduct(&mut io, "Be kind.", &mut info, &transport, &mut negotiation)
            .unwrap();
        let announced = io.channels.announced.as_ref().map(|list| list.list());
        assert_eq!(announced, Some(vec!["voicechat:secret".to_owned()]));
    }

    #[test]
    fn mod_refusal_translates_on_the_client_and_names_every_difference() {
        use horizon_protocol::mod_list::{Difference, VersionDifference};
        let difference = Difference {
            missing: (0..9).map(|i| format!("missing_{i} 1.0")).collect(),
            extra: vec!["extra 2.0".into()],
            changed: vec![VersionDifference {
                id: "changed".into(),
                server: "3.0".into(),
                client: "2.9".into(),
            }],
        };
        let reason = crate::lan::data::component_nbt(&mod_mismatch(&difference).component());
        let french = crate::lang::Language::load(&[], "fr_fr");
        let text = horizon_protocol::text::plain(&reason, french.map());
        assert!(text.starts_with("Vos mods ne correspondent"));
        assert!(text.contains("changed (serveur 3.0, votre version 2.9)"));
        assert!(text.contains("extra 2.0"));
        let vanilla = horizon_protocol::text::plain(&reason, &Default::default());
        assert!(vanilla.starts_with("Your mods don't match"));
        for name in &difference.missing {
            assert!(text.contains(name));
            assert!(vanilla.contains(name));
        }
    }

    /// Having tables for Horizon to join an old server does not make the reverse direction supported.
    #[test]
    fn nbt_item_clients_are_refused_even_when_older_clients_are_enabled() {
        let config = crate::lan::NetConfig {
            announce: false,
            listen: false,
            accept_older_clients: true,
            ..crate::lan::NetConfig::loopback(0, "Older clients".into(), 8)
        };
        let lan =
            crate::lan::Lan::start_with(Arc::new(crate::lan::data::SyncData::empty()), config)
                .unwrap();
        for protocol in [764, 765] {
            let mut c = Connection::local(lan.connect_local().unwrap());
            c.send(
                &handshake::Serverbound::Intention(handshake::Intention {
                    protocol,
                    host: "127.0.0.1".into(),
                    port: 0,
                    intent: handshake::LOGIN,
                })
                .encode(),
            )
            .unwrap();
            // If this regresses to accepting an unsupported version, its hello produces a login
            // response instead of leaving the test waiting for one the server has not sent yet.
            let _ = c.send(
                &login::Serverbound::Hello(login::ServerHello {
                    name: "Old".into(),
                    uuid: offline_uuid("Old"),
                })
                .encode(),
            );
            let (id, payload) = c.recv().unwrap();
            let login::Clientbound::Disconnect(reason) =
                login::Clientbound::decode(id, &payload).unwrap()
            else {
                panic!("protocol {protocol} was allowed into the reverse adapter")
            };
            assert!(
                reason
                    .reason_json
                    .contains("multiplayer.disconnect.incompatible"),
                "protocol {protocol}: {}",
                reason.reason_json
            );
        }
    }

    /// Logs in through `c` and answers the `LoginFinished` profile.
    fn login_profile(c: &mut Connection, name: &str) -> login::GameProfile {
        c.send(
            &handshake::Serverbound::Intention(handshake::Intention {
                protocol: horizon_protocol::PROTOCOL_VERSION,
                host: "127.0.0.1".into(),
                port: 0,
                intent: handshake::LOGIN,
            })
            .encode(),
        )
        .unwrap();
        c.send(
            &login::Serverbound::Hello(login::ServerHello {
                name: name.into(),
                uuid: offline_uuid(name),
            })
            .encode(),
        )
        .unwrap();
        loop {
            let (id, payload) = c.recv().unwrap();
            match login::Clientbound::decode(id, &payload).unwrap() {
                login::Clientbound::Compression(p) => c.set_compression(p.threshold),
                login::Clientbound::LoginFinished(p) => return p.profile,
                other => panic!("unexpected login packet {other:?}"),
            }
        }
    }

    /// A login by transfer: `Ok` once the profile is told, the reason's JSON when it is turned away.
    fn transfer_login(mut c: Connection, name: &str) -> Result<(), String> {
        c.send(
            &handshake::Serverbound::Intention(handshake::Intention {
                protocol: horizon_protocol::PROTOCOL_VERSION,
                host: "127.0.0.1".into(),
                port: 0,
                intent: handshake::TRANSFER,
            })
            .encode(),
        )
        .unwrap();
        // A server that does not accept transfers turns one away at the handshake and closes
        // (ServerHandshakePacketListenerImpl.handleIntention), so the hello may find the connection
        // gone: its disconnect is still there to read.
        let _ = c.send(
            &login::Serverbound::Hello(login::ServerHello {
                name: name.into(),
                uuid: offline_uuid(name),
            })
            .encode(),
        );
        loop {
            let (id, payload) = c.recv().unwrap();
            match login::Clientbound::decode(id, &payload).unwrap() {
                login::Clientbound::Compression(p) => c.set_compression(p.threshold),
                login::Clientbound::LoginFinished(_) => return Ok(()),
                login::Clientbound::Disconnect(d) => return Err(d.reason_json),
                other => panic!("unexpected login packet {other:?}"),
            }
        }
    }

    /// `serversettings/accept_transfers/set` changes what the next transfer finds, with no restart:
    /// a server that does not accept transfers turns one away, and takes it once it does.
    #[test]
    fn accepting_transfers_can_be_turned_on_and_off_live() {
        let config = crate::lan::NetConfig {
            announce: false,
            listen: false,
            accepts_transfers: false,
            ..crate::lan::NetConfig::loopback(0, "Transfers".into(), 8)
        };
        let lan =
            crate::lan::Lan::start_with(Arc::new(crate::lan::data::SyncData::empty()), config)
                .unwrap();
        let refused = transfer_login(Connection::local(lan.connect_local().unwrap()), "Alex")
            .expect_err("a transfer is turned away while transfers are off");
        assert!(
            refused.contains("multiplayer.disconnect.transfers_disabled"),
            "{refused}"
        );
        lan.shared.set_accepts_transfers(true);
        transfer_login(Connection::local(lan.connect_local().unwrap()), "Alex")
            .expect("a transfer is taken once transfers are on");
        lan.shared.set_accepts_transfers(false);
        // The player of the last login is still here: a second profile tries.
        let again = transfer_login(Connection::local(lan.connect_local().unwrap()), "Steve")
            .expect_err("and turned away again once they are off");
        assert!(
            again.contains("multiplayer.disconnect.transfers_disabled"),
            "{again}"
        );
    }

    #[test]
    fn an_empty_server_checks_a_horizon_clients_extra_mods() {
        use horizon_protocol::mod_list::{self, Entry, Side};
        let config = crate::lan::NetConfig {
            announce: false,
            listen: false,
            ..crate::lan::NetConfig::lan(0, "Owner - World".into(), 8)
        };
        let lan = crate::lan::Lan::start_with(Arc::new(SyncData::empty()), config).unwrap();
        let mut c = Connection::local(lan.connect_local().unwrap());
        c.reader
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        login_profile(&mut c, "Owner");
        c.send(&login::Serverbound::LoginAcknowledged(horizon_protocol::packets::Empty).encode())
            .unwrap();
        let mut brand = horizon_protocol::buf::Writer::new();
        brand.string("horizon");
        c.send(
            &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                channel: "minecraft:brand".into(),
                data: brand.data,
            })
            .encode(),
        )
        .unwrap();
        c.send(
            &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                channel: mod_list::CHANNEL.into(),
                data: mod_list::encode(&[Entry {
                    id: "shared_extra".into(),
                    version: "2.0".into(),
                    side: Side::Both,
                }]),
            })
            .encode(),
        )
        .unwrap();
        loop {
            let (id, payload) = c.recv().unwrap();
            match cfg::Clientbound::decode(id, &payload).unwrap() {
                cfg::Clientbound::CustomPayload(p)
                    if p.channel == horizon_protocol::payload_negotiation::CHANNEL =>
                {
                    use horizon_protocol::payload_negotiation::{CHANNEL, Control, Manifest};
                    if Control::decode(&p.data).unwrap() == Control::Query {
                        c.send(
                            &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                                channel: CHANNEL.into(),
                                data: Control::Offer(Manifest::default()).encode().unwrap(),
                            })
                            .encode(),
                        )
                        .unwrap();
                    }
                }
                cfg::Clientbound::SelectKnownPacks(p) => c
                    .send(&cfg::Serverbound::SelectKnownPacks(p).encode())
                    .unwrap(),
                cfg::Clientbound::Disconnect(d) => {
                    assert!(
                        horizon_protocol::text::plain(&d.reason, &Default::default())
                            .contains("shared_extra 2.0")
                    );
                    break;
                }
                cfg::Clientbound::FinishConfiguration(_) => panic!("extra mod was accepted"),
                _ => {}
            }
        }
    }

    #[test]
    fn horizon_client_raw_protocol_admission_at_known_packs() {
        for mode in [0, 1, 2] {
            let wrong_version = mode == 1;
            let compatible = mode == 2;
            let config = crate::lan::NetConfig {
                announce: false,
                listen: false,
                ..crate::lan::NetConfig::lan(0, "test".into(), 8)
            };
            let lan = crate::lan::Lan::start_with(Arc::new(SyncData::empty()), config).unwrap();
            let mut c = Connection::local(lan.connect_local().unwrap());
            c.reader
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            login_profile(&mut c, "Owner");
            c.send(
                &login::Serverbound::LoginAcknowledged(horizon_protocol::packets::Empty).encode(),
            )
            .unwrap();
            let mut brand = horizon_protocol::buf::Writer::new();
            brand.string("horizon");
            c.send(
                &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                    channel: "minecraft:brand".into(),
                    data: brand.data,
                })
                .encode(),
            )
            .unwrap();
            // A client that negotiates names its build first; one from before never does.
            let client_build = "Nightly 2000-01-01 · 0000000";
            if wrong_version {
                for (channel, data) in [
                    (
                        crate::build_mismatch::CHANNEL,
                        crate::build_mismatch::encode(client_build),
                    ),
                    (
                        horizon_protocol::payload_negotiation::CHANNEL,
                        vec![0, horizon_protocol::payload_negotiation::RAW_PROTOCOL + 1],
                    ),
                ] {
                    c.send(
                        &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                            channel: channel.into(),
                            data,
                        })
                        .encode(),
                    )
                    .unwrap();
                }
            }
            loop {
                let (id, payload) = c.recv().unwrap();
                match cfg::Clientbound::decode(id, &payload).unwrap() {
                    cfg::Clientbound::CustomPayload(p)
                        if compatible
                            && p.channel == horizon_protocol::payload_negotiation::CHANNEL =>
                    {
                        use horizon_protocol::payload_negotiation::{CHANNEL, Control, Manifest};
                        if Control::decode(&p.data).unwrap() == Control::Query {
                            c.send(
                                &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                                    channel: CHANNEL.into(),
                                    data: Control::Offer(Manifest::default()).encode().unwrap(),
                                })
                                .encode(),
                            )
                            .unwrap();
                        }
                    }
                    cfg::Clientbound::SelectKnownPacks(p) => {
                        let _ = c.send(&cfg::Serverbound::SelectKnownPacks(p).encode());
                    }
                    cfg::Clientbound::Disconnect(d) => {
                        assert!(!compatible, "compatible empty Horizon peer refused");
                        let text = horizon_protocol::text::plain(&d.reason, &Default::default());
                        assert!(text.contains(if wrong_version {
                            "protocol version"
                        } else {
                            "missing response"
                        }));
                        // The reason says which builds met, and that an older client updates.
                        let server = crate::build_mismatch::this_build();
                        assert!(
                            text.starts_with(&format!("This server runs Horizon {server}; ")),
                            "{text}"
                        );
                        if wrong_version {
                            assert!(
                                text.contains(&format!("your game is Horizon {client_build}."))
                            );
                        } else {
                            assert!(text.contains(
                                "your game is an older Horizon build. Update your game to join."
                            ));
                        }
                        break;
                    }
                    cfg::Clientbound::FinishConfiguration(_) => {
                        assert!(compatible, "old Horizon client admitted");
                        c.send(
                            &cfg::Serverbound::FinishConfiguration(
                                horizon_protocol::packets::Empty,
                            )
                            .encode(),
                        )
                        .unwrap();
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    #[test]
    fn raw_payload_peer_admission_over_memory_configuration() {
        use horizon_protocol::payload_negotiation::{
            CHANNEL, Control, Flow, Manifest, RegistrationPhase,
        };
        for optional in [true, false] {
            let config = crate::lan::NetConfig {
                announce: false,
                listen: false,
                ..crate::lan::NetConfig::lan(0, "test".into(), 8)
            };
            let lan = crate::lan::Lan::start_with(Arc::new(SyncData::empty()), config).unwrap();
            let mut c = Connection::local(lan.connect_local().unwrap());
            c.reader
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            login_profile(&mut c, "Owner");
            c.send(
                &login::Serverbound::LoginAcknowledged(horizon_protocol::packets::Empty).encode(),
            )
            .unwrap();
            let mut builder = Manifest::builder();
            for n in 0..500 {
                builder
                    .register(
                        RegistrationPhase::Play,
                        &format!("example:payload_{n:04}_with_long_name"),
                        Flow::ToServer,
                        "version_long_enough_for_large_offer",
                        optional,
                    )
                    .unwrap();
            }
            let offer = Control::Offer(builder.finish().unwrap()).encode().unwrap();
            assert!(offer.len() > 32_767);
            // A query followed by an offer crosses the server's known-pack request.
            for data in [Control::Query.encode().unwrap(), offer] {
                c.send(
                    &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                        channel: CHANNEL.into(),
                        data,
                    })
                    .encode(),
                )
                .unwrap();
            }
            let mut saw_offer = false;
            loop {
                let (id, payload) = c.recv().unwrap();
                match cfg::Clientbound::decode_with_payload_codecs(
                    id,
                    &payload,
                    &lan.shared.payload_local.codecs,
                )
                .unwrap()
                {
                    cfg::Clientbound::CustomPayload(p) if p.channel == CHANNEL => {
                        if matches!(Control::decode(&p.data).unwrap(), Control::Offer(_)) {
                            saw_offer = true;
                        }
                    }
                    cfg::Clientbound::SelectKnownPacks(p) => {
                        // A required unmatched Offer is refused as soon as it is read.
                        // The disconnect can close the peer before this crossed ACK;
                        // still read and assert that named refusal below.
                        let sent = c.send(&cfg::Serverbound::SelectKnownPacks(p).encode());
                        if optional {
                            sent.unwrap();
                        }
                    }
                    cfg::Clientbound::Disconnect(d) => {
                        assert!(!optional);
                        assert!(
                            horizon_protocol::text::plain(&d.reason, &Default::default())
                                .contains("example:payload_")
                        );
                        break;
                    }
                    cfg::Clientbound::FinishConfiguration(_) => {
                        assert!(optional && saw_offer);
                        c.send(
                            &cfg::Serverbound::FinishConfiguration(
                                horizon_protocol::packets::Empty,
                            )
                            .encode(),
                        )
                        .unwrap();
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    #[test]
    fn vanilla_configuration_does_not_disclose_horizon_mods() {
        let config = crate::lan::NetConfig {
            announce: false,
            listen: false,
            ..crate::lan::NetConfig::lan(0, "Owner - World".into(), 8)
        };
        let lan = crate::lan::Lan::start_with(Arc::new(SyncData::empty()), config).unwrap();
        let mut c = Connection::local(lan.connect_local().unwrap());
        c.reader
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        login_profile(&mut c, "Owner");
        c.send(&login::Serverbound::LoginAcknowledged(horizon_protocol::packets::Empty).encode())
            .unwrap();
        let mut data = horizon_protocol::buf::Writer::new();
        data.string("vanilla");
        c.send(
            &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                channel: "minecraft:brand".into(),
                data: data.data,
            })
            .encode(),
        )
        .unwrap();
        loop {
            let (id, payload) = c.recv().unwrap();
            match cfg::Clientbound::decode(id, &payload).unwrap() {
                cfg::Clientbound::CustomPayload(p) => {
                    assert_ne!(p.channel, horizon_protocol::mod_list::CHANNEL);
                }
                cfg::Clientbound::SelectKnownPacks(p) => {
                    c.send(&cfg::Serverbound::SelectKnownPacks(p).encode())
                        .unwrap();
                }
                cfg::Clientbound::Disconnect(d) => panic!("unexpected refusal: {:?}", d.reason),
                cfg::Clientbound::FinishConfiguration(_) => {
                    c.send(
                        &cfg::Serverbound::FinishConfiguration(horizon_protocol::packets::Empty)
                            .encode(),
                    )
                    .unwrap();
                    break;
                }
                _ => {}
            }
        }
    }

    #[test]
    fn a_malformed_mod_list_gets_a_disconnect_reason() {
        use horizon_protocol::mod_list;
        let config = crate::lan::NetConfig {
            announce: false,
            listen: false,
            ..crate::lan::NetConfig::lan(0, "Owner - World".into(), 8)
        };
        let lan = crate::lan::Lan::start_with(Arc::new(SyncData::empty()), config).unwrap();
        let mut c = Connection::local(lan.connect_local().unwrap());
        c.reader
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        login_profile(&mut c, "Owner");
        c.send(&login::Serverbound::LoginAcknowledged(horizon_protocol::packets::Empty).encode())
            .unwrap();
        let mut brand = horizon_protocol::buf::Writer::new();
        brand.string("horizon");
        c.send(
            &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                channel: "minecraft:brand".into(),
                data: brand.data,
            })
            .encode(),
        )
        .unwrap();
        let mut malformed = mod_list::encode(&[]);
        malformed.push(0); // Trailing bytes: the list is refused whole, not read as empty.
        c.send(
            &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                channel: mod_list::CHANNEL.into(),
                data: malformed,
            })
            .encode(),
        )
        .unwrap();
        loop {
            let (id, payload) = c.recv().unwrap();
            match cfg::Clientbound::decode(id, &payload).unwrap() {
                // The server may refuse the list and close before these replies land, so a
                // failed reply send is not the test's concern: the Disconnect below is.
                cfg::Clientbound::SelectKnownPacks(p) => {
                    let _ = c.send(&cfg::Serverbound::SelectKnownPacks(p).encode());
                }
                cfg::Clientbound::Disconnect(d) => {
                    let text = horizon_protocol::text::plain(&d.reason, &Default::default());
                    assert!(text.contains("couldn't be read"));
                    assert!(text.contains("trailing bytes"));
                    break;
                }
                cfg::Clientbound::FinishConfiguration(_) => panic!("malformed list was accepted"),
                _ => {}
            }
        }
    }

    #[test]
    fn payloads_and_suggestions_past_vanillas_limits_are_malformed() {
        let payload = |len: usize| {
            play::Serverbound::CustomPayload(cfg::CustomPayload {
                channel: "a:b".into(),
                data: vec![0; len],
            })
        };
        let decode = |packet: play::Serverbound| {
            let encoded = packet.encode();
            play::Serverbound::decode(
                horizon_protocol::ids::play::serverbound::CUSTOM_PAYLOAD,
                &encoded[1..],
            )
        };
        assert!(decode(payload(32_767)).is_ok());
        assert!(decode(payload(32_768)).is_err());
        let config = |len: usize| {
            cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                channel: "a:b".into(),
                data: vec![0; len],
            })
            .encode()
        };
        // The body after the packet's one-byte VarInt id.
        let id = horizon_protocol::ids::configuration::serverbound::CUSTOM_PAYLOAD;
        assert!(cfg::Serverbound::decode(id, &config(32_767)[1..]).is_ok());
        assert!(cfg::Serverbound::decode(id, &config(32_768)[1..]).is_err());
        let suggestion = |len: usize| {
            let mut w = horizon_protocol::buf::Writer::new();
            w.varint(7);
            w.string(&"a".repeat(len));
            play::Serverbound::Unknown(COMMAND_SUGGESTION, w.data)
        };
        assert!(checked_play(suggestion(32500)).is_ok());
        assert!(checked_play(suggestion(32501)).is_err());
    }

    #[test]
    fn prompts_become_the_components_vanilla_sends() {
        let tag = |json: &str| {
            let tag = component_tag(&crate::server::json::parse(json, true).unwrap());
            component_json(&tag)
        };
        assert_eq!(tag(r#"{"text":"Hi"}"#), r#""Hi""#);
        assert_eq!(tag(r#""Hi""#), r#""Hi""#);
        assert_eq!(
            tag(r#"["Hi ",{"text":"there","bold":true}]"#),
            r#"{"text":"Hi ","extra":[{"text":"there","bold":true}]}"#
        );
        assert_eq!(
            tag(r#"{"translate":"a","with":["b",{"text":"c","color":"red"}]}"#),
            r#"{"translate":"a","with":["b",{"text":"c","color":"red"}]}"#
        );
    }

    /// The owner's login never waits on the profile fetch, even one that
    /// never answers, and carries no properties.
    #[test]
    fn the_owners_login_never_waits_on_the_profile_fetch() {
        let (_hold, never) = channel::<()>();
        let config = crate::lan::NetConfig {
            announce: false,
            listen: false,
            owner_profile: Some(Arc::new(crate::lan::OwnerProfile::new(move || {
                let _ = never.recv();
                Vec::new()
            }))),
            ..crate::lan::NetConfig::loopback(0, "Owner - World".into(), 8)
        };
        let lan =
            crate::lan::Lan::start_with(Arc::new(crate::lan::data::SyncData::empty()), config)
                .unwrap();
        let start = Instant::now();
        let owner = login_profile(
            &mut Connection::local(lan.connect_local().unwrap()),
            "Owner",
        );
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "{:?}",
            start.elapsed()
        );
        assert_eq!(owner.uuid, offline_uuid("Owner"));
        assert!(owner.properties.is_empty());
    }

    /// `ServerLoginPacketListenerImpl.handleHello`: the integrated server's own player
    /// logs in as `getSingleplayerProfile`'s UUID, not the offline one its name makes;
    /// anyone else through memory keeps the offline profile.
    #[test]
    fn the_integrated_owners_login_is_the_singleplayer_profile() {
        let owner_uuid = 0x0123_4567_89ab_cdef_0123_4567_89ab_cdefu128;
        let config = crate::lan::NetConfig {
            announce: false,
            listen: false,
            singleplayer_profile: Some(("Owner".into(), owner_uuid)),
            ..crate::lan::NetConfig::loopback(0, "Owner - World".into(), 8)
        };
        let lan =
            crate::lan::Lan::start_with(Arc::new(crate::lan::data::SyncData::empty()), config)
                .unwrap();
        let owner = login_profile(
            &mut Connection::local(lan.connect_local().unwrap()),
            "Owner",
        );
        assert_eq!(owner.uuid, owner_uuid);
        let guest = login_profile(&mut Connection::local(lan.connect_local().unwrap()), "Alex");
        assert_eq!(guest.uuid, offline_uuid("Alex"));
    }

    /// Logs "Alex" in over memory up to `LoginFinished`; returns the server (kept alive by the
    /// caller), the connection and the lines the server's log writes (en_us's text for the one key
    /// the refusals use).
    fn alex_logged_in() -> (crate::lan::Lan, Connection, Arc<Mutex<Vec<String>>>) {
        let lines = Arc::new(Mutex::new(Vec::<String>::new()));
        let into = lines.clone();
        let mut lang = horizon_protocol::text::Language::new();
        lang.insert(
            "multiplayer.disconnect.unexpected_query_response".into(),
            "Unexpected custom data from client".into(),
        );
        let config = crate::lan::NetConfig {
            announce: false,
            listen: false,
            log: Some(crate::lan::NetLog {
                write: Arc::new(move |_, _, line| into.lock().unwrap().push(line.to_owned())),
                lang: Arc::new(lang),
                log_ips: false,
            }),
            ..crate::lan::NetConfig::loopback(0, "Stray packets".into(), 8)
        };
        let lan =
            crate::lan::Lan::start_with(Arc::new(crate::lan::data::SyncData::empty()), config)
                .unwrap();
        let mut c = Connection::local(lan.connect_local().unwrap());
        c.send(
            &handshake::Serverbound::Intention(handshake::Intention {
                protocol: horizon_protocol::PROTOCOL_VERSION,
                host: "127.0.0.1".into(),
                port: 0,
                intent: handshake::LOGIN,
            })
            .encode(),
        )
        .unwrap();
        c.send(
            &login::Serverbound::Hello(login::ServerHello {
                name: "Alex".into(),
                uuid: offline_uuid("Alex"),
            })
            .encode(),
        )
        .unwrap();
        loop {
            let (id, payload) = c.recv().unwrap();
            match login::Clientbound::decode(id, &payload).unwrap() {
                login::Clientbound::Compression(p) => c.set_compression(p.threshold),
                login::Clientbound::LoginFinished(_) => break,
                other => panic!("unexpected login packet {other:?}"),
            }
        }
        (lan, c, lines)
    }

    /// `alex_logged_in`, then sends `stray` if there is one and `LoginAcknowledged` (so a server
    /// that lets the stray packet pass goes on to configuration instead of waiting for the next
    /// packet), and returns the server's next packet with the lines its log has written so far.
    fn login_with_stray_packet(
        stray: Option<login::Serverbound>,
    ) -> ((i32, Vec<u8>), Arc<Mutex<Vec<String>>>) {
        let (_lan, mut c, lines) = alex_logged_in();
        if let Some(stray) = stray {
            c.send(&stray.encode()).unwrap();
        }
        // A server that has already refused the login may have closed the connection.
        let _ = c.send(&login::Serverbound::LoginAcknowledged(Empty).encode());
        (c.recv().unwrap(), lines)
    }

    /// `ServerCommonPacketListenerImpl.handleCookieResponse` in the configuration state: this
    /// server sends no cookie request there, so an answer is unexpected and ends the connection
    /// with a configuration disconnect, before the player joins. `after` is the clientbound
    /// packet the client waits for before it sends the cookie response (it answers the known packs
    /// on the way).
    fn assert_cookie_refused_in_configuration(after: i32) {
        use horizon_protocol::ids::configuration::clientbound::{DISCONNECT, SELECT_KNOWN_PACKS};
        let (_lan, mut c, lines) = alex_logged_in();
        c.send(&login::Serverbound::LoginAcknowledged(Empty).encode())
            .unwrap();
        loop {
            let (id, payload) = c.recv().unwrap();
            if id == after {
                break;
            }
            assert_ne!(id, DISCONNECT, "refused before the cookie response");
            // Like a vanilla client: the known packs offered are the ones it knows.
            if id == SELECT_KNOWN_PACKS {
                let cfg::Clientbound::SelectKnownPacks(offer) =
                    cfg::Clientbound::decode(id, &payload).unwrap()
                else {
                    panic!("not the known packs")
                };
                c.send(&cfg::Serverbound::SelectKnownPacks(offer).encode())
                    .unwrap();
            }
        }
        c.send(
            &cfg::Serverbound::CookieResponse(cfg::CookieResponse {
                key: "horizon:test".into(),
                payload: None,
            })
            .encode(),
        )
        .unwrap();
        loop {
            let (id, payload) = c.recv().expect("the server closed without a disconnect");
            if id != DISCONNECT {
                continue;
            }
            let cfg::Clientbound::Disconnect(reason) =
                cfg::Clientbound::decode(id, &payload).unwrap()
            else {
                panic!("not a configuration disconnect")
            };
            assert_eq!(
                format!("{:?}", reason.reason),
                format!(
                    "{:?}",
                    translatable("multiplayer.disconnect.unexpected_query_response", &[])
                ),
            );
            break;
        }
        let user = "Alex (<ip address withheld>)";
        let expected = [
            format!("Disconnecting {user}: Unexpected custom data from client"),
            format!("{user} lost connection: Unexpected custom data from client"),
        ];
        let until = Instant::now() + Duration::from_secs(10);
        while lines.lock().unwrap().len() < expected.len() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(*lines.lock().unwrap(), expected);
    }

    #[test]
    fn a_configuration_cookie_response_before_the_known_packs_disconnects_the_client() {
        use horizon_protocol::ids::configuration::clientbound::SELECT_KNOWN_PACKS;
        assert_cookie_refused_in_configuration(SELECT_KNOWN_PACKS);
    }

    #[test]
    fn a_configuration_cookie_response_while_it_finishes_disconnects_the_client() {
        use horizon_protocol::ids::configuration::clientbound::FINISH_CONFIGURATION;
        assert_cookie_refused_in_configuration(FINISH_CONFIGURATION);
    }

    /// `ServerLoginPacketListenerImpl.handleCustomQueryPacket` and `handleCookieResponse`: this
    /// server sends no login query and no cookie request, so an answer to either is unexpected and
    /// ends the login with `ServerCommonPacketListenerImpl.DISCONNECT_UNEXPECTED_QUERY`, before
    /// configuration starts.
    fn assert_unexpected_query_refused(stray: login::Serverbound) {
        use horizon_protocol::ids::login::clientbound::LOGIN_DISCONNECT;
        let ((id, payload), lines) = login_with_stray_packet(Some(stray));
        assert_eq!(
            id, LOGIN_DISCONNECT,
            "the login went on: the server sent packet {id}"
        );
        let login::Clientbound::Disconnect(reason) =
            login::Clientbound::decode(id, &payload).unwrap()
        else {
            panic!("not a login disconnect")
        };
        assert_eq!(
            reason.reason_json,
            r#"{"translate":"multiplayer.disconnect.unexpected_query_response"}"#
        );
        // The refusal is logged as vanilla's `disconnect` does, then the loss of the connection.
        let user = "Alex (<ip address withheld>)";
        let expected = [
            format!("Disconnecting {user}: Unexpected custom data from client"),
            format!("{user} lost connection: Unexpected custom data from client"),
        ];
        let until = Instant::now() + Duration::from_secs(10);
        while lines.lock().unwrap().len() < expected.len() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(*lines.lock().unwrap(), expected);
    }

    #[test]
    fn a_login_query_answer_nobody_asked_for_disconnects_the_client() {
        assert_unexpected_query_refused(login::Serverbound::CustomQueryAnswer(
            login::CustomQueryAnswer {
                transaction: 1,
                payload: Some(vec![1, 2, 3]),
            },
        ));
    }

    #[test]
    fn a_login_cookie_response_nobody_asked_for_disconnects_the_client() {
        assert_unexpected_query_refused(login::Serverbound::CookieResponse(
            login::CookieResponse {
                key: "horizon:test".into(),
                payload: None,
            },
        ));
    }

    /// The same login with nothing stray goes on to configuration: its first packet is the brand.
    #[test]
    fn a_login_with_nothing_stray_reaches_configuration() {
        use horizon_protocol::ids::configuration::clientbound::CUSTOM_PAYLOAD;
        let ((id, _), _) = login_with_stray_packet(None);
        assert_eq!(id, CUSTOM_PAYLOAD);
    }

    fn unpack(out: Out) -> Vec<Vec<u8>> {
        match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            Out::Disconnect(_) => vec![b"disconnect".to_vec()],
        }
    }

    fn bodies(rx: &std::sync::mpsc::Receiver<Out>) -> Vec<Vec<Vec<u8>>> {
        rx.try_iter().map(unpack).collect()
    }

    #[test]
    fn packets_sent_after_a_posted_tick_follow_it() {
        let (tx, rx) = channel();
        let (tx2, rx2) = channel();
        let sender = PlayerSender::new(tx, Arc::default());
        let other = PlayerSender::new(tx2, Arc::default());
        for round in 0..50u8 {
            let holding = Holding::start();
            sender.send(vec![round, 1]);
            sender.send(vec![round, 2]);
            other.send(vec![round]);
            drop(holding);
            flush_all([&sender, &other]);
            sender.send(vec![round, 3]);
            let next = || unpack(rx.recv_timeout(Duration::from_secs(10)).unwrap());
            assert_eq!(next(), [vec![round, 1], vec![round, 2]]);
            assert_eq!(next(), [vec![round, 3]]);
            let got = unpack(rx2.recv_timeout(Duration::from_secs(10)).unwrap());
            assert_eq!(got, [vec![round]]);
        }
        let holding = Holding::start();
        sender.send(vec![9]);
        drop(holding);
        flush_all([&sender]);
        sender.disconnect(horizon_core::nbt::Tag::Byte(0));
        let next = || unpack(rx.recv_timeout(Duration::from_secs(10)).unwrap());
        assert_eq!(next(), [vec![9]]);
        assert_eq!(next(), [b"disconnect".to_vec()]);
        // Nothing sent twice.
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn shared_frames_are_the_frames_compressed_here() {
        use horizon_protocol::frame;
        let frames = SharedFrames::new(usize::MAX);
        let a: Vec<u8> = (0..40_000u32).map(|i| (i * 7 % 13) as u8).collect();
        let mut b = a.clone();
        b[20_000] ^= 1;
        for (body, threshold) in [(&a, 256), (&b, 256), (&a, 256), (&a, 64), (&b, 256)] {
            let shared = frames.contents(body, threshold).unwrap();
            assert_eq!(*shared, *frame::contents(body, Some(threshold)).unwrap());
        }
        // One frame per body and threshold.
        assert_eq!(frames.frames.lock().unwrap().frames.len(), 3);
    }

    /// Senders asking for the same body at once share one compression: they
    /// all get the frame the first made.
    #[test]
    fn a_body_compressed_now_is_not_compressed_again() {
        let frames = SharedFrames::new(usize::MAX);
        let body: Vec<u8> = (0..400_000u32).map(|i| (i * 7 % 251) as u8).collect();
        let start = std::sync::Barrier::new(8);
        let made: Vec<Arc<[u8]>> = std::thread::scope(|scope| {
            let threads: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        start.wait();
                        frames.contents(&body, 256).unwrap()
                    })
                })
                .collect();
            threads.into_iter().map(|t| t.join().unwrap()).collect()
        });
        assert!(made.iter().all(|m| Arc::ptr_eq(m, &made[0])));
        let cache = frames.frames.lock().unwrap();
        assert_eq!(cache.frames.len(), 1);
        assert_eq!(cache.bytes, made[0].len());
    }

    #[test]
    fn shared_frames_keep_the_newest_within_their_limit() {
        let first = SharedFrames::new(0)
            .contents(&[1; 9000], 256)
            .unwrap()
            .len();
        let frames = SharedFrames::new(first * 2);
        for n in 0..5u8 {
            frames.contents(&[n; 9000], 256).unwrap();
        }
        let cache = frames.frames.lock().unwrap();
        assert!((1..=2).contains(&cache.frames.len()));
        assert_eq!(cache.order.len(), cache.frames.len());
        assert_eq!(
            cache.bytes,
            cache
                .frames
                .values()
                .map(|f| f.get().unwrap().as_ref().unwrap().len())
                .sum::<usize>()
        );
        assert!(cache.bytes <= first * 2);
    }

    #[test]
    fn held_packets_go_together_and_in_order() {
        let (tx, rx) = channel();
        let sender = PlayerSender::new(tx, Arc::default());
        sender.send(vec![1]);
        assert_eq!(bodies(&rx), [vec![vec![1]]]);
        let holding = Holding::start();
        sender.send(vec![2]);
        sender.send(vec![3]);
        assert!(bodies(&rx).is_empty());
        drop(holding);
        // A packet sent after the tick still follows the held ones.
        sender.send(vec![4]);
        assert_eq!(bodies(&rx), [vec![vec![2], vec![3]], vec![vec![4]]]);
        let holding = Holding::start();
        sender.send(vec![5]);
        sender.disconnect(horizon_core::nbt::Tag::Byte(0));
        drop(holding);
        sender.flush();
        assert_eq!(bodies(&rx), [vec![vec![5]], vec![b"disconnect".to_vec()]]);
        assert_eq!(
            sender.pending.load(Ordering::Relaxed),
            5 + 10 * std::mem::size_of::<io_pool::Queued>()
        );
    }

    /// `handleUseItemOn` and `handleUseItem`: `ackBlockChangesUpTo(sequence)` throws on a negative
    /// sequence before the handler does anything, and the server only logs it, so such a packet
    /// does nothing and its sender stays. One with a sequence of 0 or more goes on to the hub.
    #[test]
    fn a_use_with_a_negative_sequence_is_dropped_before_the_hub() {
        use super::io_pool::Verdict;
        use horizon_protocol::ids::play::serverbound::{USE_ITEM, USE_ITEM_ON};
        let config = crate::lan::NetConfig {
            announce: false,
            listen: false,
            ..crate::lan::NetConfig::loopback(0, "Sequence".into(), 8)
        };
        let lan = crate::lan::Lan::start_with(Arc::new(SyncData::empty()), config).unwrap();
        let mut c = PlayConn {
            payload_sender: PlayerSender::disabled(),
            payload_transport: Transport::empty(),
            conn: 1,
            shared: lan.shared.clone(),
            uuid: 2,
            name: "p".to_owned(),
            required_pack: false,
            rate: None,
            watch: Arc::new(KeepAliveWatch {
                start: Instant::now(),
                alive: Arc::new(AtomicI64::new(-1)),
            }),
            latency: Arc::new(AtomicI32::new(0)),
            sent_at: None,
            next_keep_alive: Instant::now(),
            counted: None,
            undrained: Arc::default(),
        };
        // The bodies after the packets' one-byte VarInt ids.
        let packets = |sequence: i32| {
            [
                (
                    USE_ITEM,
                    play::Serverbound::UseItem(play::UseItem {
                        hand: 0,
                        sequence,
                        yaw: 0.0,
                        pitch: 0.0,
                    })
                    .encode(),
                ),
                (
                    USE_ITEM_ON,
                    play::Serverbound::UseItemOn(play::UseItemOn {
                        hand: 0,
                        pos: [0, 64, 0],
                        face: 1,
                        cursor: [0.5, 1.0, 0.5],
                        inside: false,
                        world_border: false,
                        sequence,
                    })
                    .encode(),
                ),
            ]
        };
        let forwarded = |lan: &crate::lan::Lan| {
            let events = lan.poll();
            events
                .into_iter()
                .map(|receipt| receipt.into_parts().0)
                .filter(|e| matches!(e, crate::lan::NetEvent::Packet(..)))
                .count()
        };
        for sequence in [-1, i32::MIN] {
            for (id, packet) in packets(sequence) {
                assert!(matches!(c.packet(id, packet[1..].to_vec()), Verdict::Go));
            }
        }
        assert_eq!(forwarded(&lan), 0, "a negative sequence reached the hub");
        for sequence in [0, 7] {
            for (id, packet) in packets(sequence) {
                assert!(matches!(c.packet(id, packet[1..].to_vec()), Verdict::Go));
            }
        }
        assert_eq!(forwarded(&lan), 4);
    }

    /// A client that answers no keep-alive is timed out 30 s after the last
    /// one it answered was sent (or after joining).
    #[test]
    fn a_silent_client_times_out() {
        let ago = |s| Instant::now().checked_sub(Duration::from_secs(s)).unwrap();
        let watch = |start, answered| KeepAliveWatch {
            start,
            alive: Arc::new(AtomicI64::new(answered)),
        };
        assert!(!watch(ago(29), -1).timed_out());
        assert!(watch(ago(31), -1).timed_out());
        // Answered the keep-alive sent 15 s in: fine until 45 s.
        assert!(!watch(ago(44), 15_000).timed_out());
        assert!(watch(ago(46), 15_000).timed_out());
    }

    #[test]
    fn a_rate_over_the_limit_kicks() {
        let t0 = Instant::now();
        let second = Duration::from_secs(1);
        // Off at 0 (and below).
        assert!(PacketRate::new(0, t0).is_none());
        assert!(PacketRate::new(-1, t0).is_none());
        let mut rate = PacketRate::new(10, t0).unwrap();
        // 15 packets inside the first second average 3.75 at the tick.
        for _ in 0..15 {
            assert_eq!(rate.received(t0), None);
        }
        assert_eq!(rate.received(t0 + second), None);
        // 50 a second averages 12.5: over a limit of 10 at the first tick.
        let mut rate = PacketRate::new(10, t0).unwrap();
        for _ in 0..50 {
            rate.received(t0);
        }
        assert_eq!(rate.received(t0 + second), Some(12.5));
    }

    #[test]
    fn an_idle_gap_decays_without_a_loop() {
        let t0 = Instant::now();
        let mut rate = PacketRate {
            limit: 100.0,
            received: 0,
            average: 40.0,
            next: t0,
        };
        // Ten seconds later: one tick (0 packets: 40 * 0.75 = 30), then ten
        // more packetless decays — ~1.69 — with `next` past now.
        assert_eq!(rate.received(t0 + Duration::from_secs(10)), None);
        assert!(rate.average > 1.5 && rate.average < 2.0, "{}", rate.average);
        assert!(rate.next > t0 + Duration::from_secs(10));
    }

    /// `ServerCommonPacketListenerImpl.handleKeepAlive`: an answer to the
    /// pending keep-alive with its id counts (and times the round trip); any
    /// other answer, or one with nothing pending, disconnects the player with
    /// `disconnect.timeout`, unless they are the singleplayer owner. A refused
    /// answer must not count as "alive" either.
    #[test]
    fn a_keep_alive_that_is_not_the_pending_one_disconnects_all_but_the_owner() {
        use super::io_pool::Verdict;
        use horizon_protocol::ids::play::serverbound::KEEP_ALIVE;
        let config = crate::lan::NetConfig {
            announce: false,
            listen: false,
            singleplayer_profile: Some(("Owner".to_owned(), 1)),
            ..crate::lan::NetConfig::loopback(0, "KeepAlive".into(), 8)
        };
        let lan = crate::lan::Lan::start_with(Arc::new(SyncData::empty()), config).unwrap();
        let conn = |uuid: u128, pending: Option<i64>| PlayConn {
            payload_sender: PlayerSender::disabled(),
            payload_transport: Transport::empty(),
            conn: 1,
            shared: lan.shared.clone(),
            uuid,
            name: if uuid == 1 { "Owner" } else { "p" }.to_owned(),
            required_pack: false,
            rate: None,
            watch: Arc::new(KeepAliveWatch {
                start: Instant::now(),
                alive: Arc::new(AtomicI64::new(-1)),
            }),
            latency: Arc::new(AtomicI32::new(0)),
            sent_at: pending.map(|id| (id, Instant::now())),
            next_keep_alive: Instant::now(),
            counted: None,
            undrained: Arc::default(),
        };
        let answer = |c: &mut PlayConn, id: i64| c.packet(KEEP_ALIVE, id.to_be_bytes().to_vec());
        // The pending one: accepted.
        let mut c = conn(2, Some(7));
        assert!(matches!(answer(&mut c, 7), Verdict::Go));
        assert!(c.sent_at.is_none());
        assert_eq!(c.watch.alive.load(Ordering::Relaxed), 7);
        // Another id while one is pending: kicked, and not counted as alive.
        let mut c = conn(2, Some(7));
        assert!(matches!(answer(&mut c, 8), Verdict::Kick(..)));
        assert_eq!(c.watch.alive.load(Ordering::Relaxed), -1);
        assert!(c.sent_at.is_some());
        // Nothing pending: kicked.
        let mut c = conn(2, None);
        assert!(matches!(answer(&mut c, 7), Verdict::Kick(..)));
        // The singleplayer owner is never disconnected for it, nor counted alive.
        let mut c = conn(1, Some(7));
        assert!(matches!(answer(&mut c, 8), Verdict::Go));
        assert_eq!(c.watch.alive.load(Ordering::Relaxed), -1);
    }

    /// `ServerCommonPacketListenerImpl.handleCookieResponse`: the server asks for no cookie, so a
    /// response is always unexpected and disconnects with `DISCONNECT_UNEXPECTED_QUERY`, for the
    /// singleplayer owner too (unlike a keep-alive).
    #[test]
    fn a_cookie_response_in_play_disconnects_the_player() {
        use super::io_pool::Verdict;
        use horizon_protocol::ids::play::serverbound::{CLIENT_TICK_END, COOKIE_RESPONSE};
        let config = crate::lan::NetConfig {
            announce: false,
            listen: false,
            singleplayer_profile: Some(("Owner".to_owned(), 1)),
            ..crate::lan::NetConfig::loopback(0, "Cookie".into(), 8)
        };
        let lan = crate::lan::Lan::start_with(Arc::new(SyncData::empty()), config).unwrap();
        let conn = |uuid: u128| PlayConn {
            payload_sender: PlayerSender::disabled(),
            payload_transport: Transport::empty(),
            conn: 1,
            shared: lan.shared.clone(),
            uuid,
            name: if uuid == 1 { "Owner" } else { "p" }.to_owned(),
            required_pack: false,
            rate: None,
            watch: Arc::new(KeepAliveWatch {
                start: Instant::now(),
                alive: Arc::new(AtomicI64::new(-1)),
            }),
            latency: Arc::new(AtomicI32::new(0)),
            sent_at: None,
            next_keep_alive: Instant::now(),
            counted: None,
            undrained: Arc::default(),
        };
        // The body after the packet's one-byte VarInt id.
        let response = play::Serverbound::CookieResponse(cfg::CookieResponse {
            key: "minecraft:test".into(),
            payload: None,
        })
        .encode();
        let expected = play::Clientbound::Disconnect(cfg::Disconnect {
            reason: translatable("multiplayer.disconnect.unexpected_query_response", &[]),
        })
        .encode();
        for uuid in [2, 1] {
            match conn(uuid).packet(COOKIE_RESPONSE, response[1..].to_vec()) {
                Verdict::Kick(text, body) => {
                    assert_eq!(text, "Unexpected custom data from client");
                    assert_eq!(body, expected);
                }
                _ => panic!("the cookie response of player {uuid} did not disconnect them"),
            }
        }
        // A packet the player may send still goes through.
        assert!(matches!(
            conn(2).packet(CLIENT_TICK_END, Vec::new()),
            Verdict::Go
        ));
    }
}

#[cfg(test)]
mod typed_address_tests {
    use super::*;
    fn typed(host: &str, port: u16, memory: bool) -> Option<(String, u16)> {
        let intention = handshake::Intention {
            protocol: 0,
            host: host.into(),
            port,
            intent: handshake::LOGIN,
        };
        typed_address(&intention, memory).map(|(host, port)| (host.to_string(), port))
    }
    /// The host and port as typed, the host cut where a proxy puts its own data.
    #[test]
    fn the_host_is_the_text_before_a_nul() {
        let play = Some(("play.example.com".to_owned(), 25570));
        assert_eq!(typed("play.example.com", 25570, false), play);
        assert_eq!(
            typed("play.example.com\x00198.51.100.7\x00uuid", 25570, false),
            play
        );
        assert_eq!(typed("", 25565, false), None);
        assert_eq!(typed("\0data", 25565, false), None);
        assert_eq!(typed("localhost", 25565, true), None);
    }
}

#[cfg(test)]
mod outgoing_budget_tests {
    use super::*;
    use horizon_protocol::connection::Framing;
    fn sender(framing: Framing) -> PlayerSender {
        let pending = Arc::default();
        let outbox =
            io_pool::Outbox::new(Arc::clone(&pending), !matches!(framing, Framing::Memory))
                .unwrap();
        PlayerSender {
            payload_transport: Transport::empty(),
            undrained: Arc::default(),
            pending,
            held: Arc::default(),
            framing,
            to: To::Pool(
                outbox,
                Arc::new(KeepAliveWatch {
                    start: Instant::now(),
                    alive: Arc::new(AtomicI64::new(-1)),
                }),
            ),
            wire: None,
            datapack_offer: None,
            announced: Arc::default(),
            viewed_levels: Default::default(),
            menu_extra_rev: 0,
            connect: None,
        }
    }
    #[test]
    fn empty_prepared_journal_records_are_bounded_and_fully_refunded() {
        let s = sender(Framing::Memory);
        let id = PayloadId("test:raw".into());
        let cost = io_pool::charge(0) + 512;
        let mut permits = Vec::new();
        for _ in 0..io_pool::MAX_PENDING / cost {
            permits.push(s.prepare_payload(Vec::new(), &id).unwrap());
        }
        assert_eq!(s.pending.load(Ordering::Relaxed), permits.len() * cost);
        assert!(s.pending.load(Ordering::Relaxed) <= io_pool::MAX_PENDING);
        assert!(matches!(
            s.prepare_payload(Vec::new(), &id),
            Err(PayloadSendError::Backpressure)
        ));
        drop(permits.pop());
        assert_eq!(s.pending.load(Ordering::Relaxed), permits.len() * cost);
        let replacement = s.prepare_payload(Vec::new(), &id).unwrap();
        drop(replacement);
        drop(permits);
        assert_eq!(s.pending.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn prepared_reservations_refund_and_share_capacity() {
        let s = sender(Framing::Memory);
        let id = PayloadId("test:raw".into());
        let body = Vec::with_capacity(io_pool::MAX_PENDING - 4096);
        let p = s.prepare_payload(body, &id).unwrap();
        assert!(s.pending.load(Ordering::Relaxed) > io_pool::MAX_PENDING - 4096);
        assert!(matches!(
            s.prepare_payload(vec![0; 8192], &id),
            Err(PayloadSendError::Backpressure)
        ));
        drop(p);
        assert_eq!(s.pending.load(Ordering::Relaxed), 0);
        let p = s.prepare_payload(vec![0; 9 << 20], &id).unwrap();
        s.close();
        assert!(matches!(p.publish(), Err(PayloadSendError::Disconnected)));
        assert_eq!(s.pending.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn prepare_does_not_deliver_and_publish_reuses_compressed_contents() {
        let s = sender(Framing::Network(Some(32)));
        let id = PayloadId("test:raw".into());
        let body = vec![42; 10000];
        let expected = Framing::Network(Some(32)).contents(&body).unwrap();
        let p = s.prepare_payload(body, &id).unwrap();
        let To::Pool(outbox, _) = &s.to else {
            unreachable!()
        };
        assert!(outbox.queued_contents().is_empty());
        assert_eq!(p.bytes.as_ref().unwrap(), &expected);
        p.publish().unwrap();
        assert_eq!(outbox.queued_contents(), [expected]);
        let before = s.pending.load(Ordering::Relaxed);
        let hold = Holding::start();
        s.send(Vec::with_capacity(20000));
        assert_eq!(
            s.pending.load(Ordering::Relaxed),
            before + io_pool::charge(20000)
        );
        drop(hold);
        s.flush();
        let reserved = s.prepare_payload(vec![1; 400], &id).unwrap();
        assert!(s.pending.load(Ordering::Relaxed) > before + 20000);
        drop(reserved);
    }
    #[test]
    fn network_refuses_before_queueing_and_empty_messages_cost_space() {
        let s = sender(Framing::Network(None));
        assert!(matches!(
            s.prepare_payload(vec![0; 1 << 21], &PayloadId("test:raw".into())),
            Err(PayloadSendError::FrameTooLarge { .. })
        ));
        assert_eq!(s.pending.load(Ordering::Relaxed), 0);
        let _hold = Holding::start();
        for _ in 0..(io_pool::MAX_PENDING / io_pool::charge(0) + 10) {
            s.send(Vec::new());
        }
        assert!(s.pending.load(Ordering::Relaxed) <= io_pool::MAX_PENDING);
        assert!(s.held().len() < io_pool::MAX_PENDING / std::mem::size_of::<Vec<u8>>());
    }
    fn outbox(s: &PlayerSender) -> &Arc<io_pool::Outbox> {
        let To::Pool(outbox, _) = &s.to else {
            unreachable!()
        };
        outbox
    }
    // The integrated server's own player: a reopened world sends its whole first view at once,
    // more than the network budget, and the player must stay (#12849, #18313).
    #[test]
    fn a_memory_connection_stays_open_past_the_budget_on_send() {
        let s = sender(Framing::Memory);
        s.send(Vec::with_capacity(io_pool::MAX_PENDING));
        s.send(vec![1, 2, 3]);
        assert!(outbox(&s).alive(), "send closed the connection");
        assert_eq!(outbox(&s).queued(), 2);
    }
    #[test]
    fn a_memory_connection_stays_open_past_the_budget_on_send_many() {
        let s = sender(Framing::Memory);
        let half = io_pool::MAX_PENDING / 2;
        s.send_many([Vec::with_capacity(half), Vec::with_capacity(half)]);
        assert!(outbox(&s).alive(), "send_many closed the connection");
        {
            let _hold = Holding::start();
            s.send_many([Vec::with_capacity(half), vec![1, 2, 3]]);
        }
        s.flush();
        assert!(outbox(&s).alive(), "a held send_many closed the connection");
        assert_eq!(outbox(&s).queued(), 4);
    }
    #[test]
    fn a_memory_connection_past_the_budget_still_gets_its_disconnect() {
        let s = sender(Framing::Memory);
        // A first view's burst already queued.
        s.pending.store(io_pool::MAX_PENDING, Ordering::Relaxed);
        let reason = translatable("disconnect.closed", &[]);
        s.disconnect(reason.clone());
        let expected = play::Clientbound::Disconnect(cfg::Disconnect { reason }).encode();
        assert_eq!(
            outbox(&s).queued_contents().last(),
            Some(&expected),
            "the disconnect was dropped"
        );
    }
    /// A network client that reads slowly keeps the budget: past it, it is closed.
    #[test]
    fn a_network_connection_still_closes_past_the_budget() {
        let s = sender(Framing::Network(None));
        s.send(Vec::with_capacity(io_pool::MAX_PENDING));
        assert!(!outbox(&s).alive());
        assert_eq!(outbox(&s).queued(), 0);
    }
}

#[cfg(test)]
#[path = "datapack_offer_tests.rs"]
mod datapack_offer_tests;
