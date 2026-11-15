//! Multiplayer session: a network thread that connects to a server, logs in,
//! runs the configuration phase and then forwards decoded play packets to the
//! game thread. Keep-alives are answered on the network thread; pings wait for
//! the game thread, like ClientCommonPacketListenerImpl.handlePing, so a pong
//! cannot overtake an earlier world update or teleport acknowledgement.
//! Online-mode servers need the Microsoft account the launcher signed in
//! (crate::auth): the login joins the server through the session service and
//! encrypts the connection (ClientHandshakePacketListenerImpl.handleHello).
use crate::auth::AuthSession;
pub mod ingress;
mod known_packs;
use known_packs::{fill_known_entries, select_known_packs};
#[cfg(test)]
#[path = "net/mod_packet_tests.rs"]
mod mod_packet_tests;
pub(crate) mod queued_writer;
use horizon_protocol::{
    PROTOCOL_VERSION,
    connection::{ConnReader, Connection, Counters},
    crypto::{generate_secret, offline_uuid, server_hash},
    packets::{Empty, configuration as cfg, handshake, login, play},
    registry::Dynamic,
    rsa::RsaPublicKey,
    servers::parse_address,
    text::{Language, plain, translatable},
};
use queued_writer::QueuedWriter;
pub use queued_writer::{PreparedPacket, PreparedPayload};
#[cfg(test)]
use std::sync::mpsc::channel;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};
use {
    horizon_core::nbt::{JavaString, Tag},
    horizon_world::item_defaults::ItemData,
};

use horizon_hooks::tunnel;

/// `CommonListenerCookie.serverCookies`: cookies the server asked the client
/// to store (`ClientboundStoreCookiePacket`), answered to
/// `ClientboundCookieRequestPacket`. A `transfer` carries them to the next
/// connection (`TransferState`), so a proxy's backend switch keeps them.
pub type Cookies = Arc<Mutex<HashMap<String, Vec<u8>>>>;

/// A recording of the session's packets: the tap a replay recorder plugs into. Replay recording is a
/// platform feature (docs/layers.md, "Controllers, splitscreen, replays and UI pages"); the engine only
/// hands it the packets: the network thread what it receives, the game thread what the client makes for
/// the recording itself (its own movement, pose and swings).
pub trait PacketRecorder: Send {
    /// Whether it still takes packets (false once stopped or failed).
    fn is_active(&self) -> bool;
    /// A play packet the network thread received, decoded, with its id and payload as they came.
    /// The recorder decides what it keeps.
    fn received(&self, packet: &play::Clientbound, id: i32, payload: &[u8]);
    /// A packet the client made for the recording, encoded, stamped now.
    fn record_packet(&self, id: i32, payload: &[u8]);
    fn status(&self) -> RecorderStatus;
    /// Finishes the recording; the file it wrote.
    fn stop(self: Box<Self>) -> std::io::Result<PathBuf>;
}

/// Where a recording stands.
#[derive(Clone, Debug, PartialEq)]
pub struct RecorderStatus {
    pub active: bool,
    pub path: PathBuf,
    pub elapsed: Duration,
    pub packet_count: u64,
    pub bytes_written: u64,
    pub error: Option<String>,
}

/// The session's recorder, shared with its network thread.
type RecorderSlot = Arc<Mutex<Option<Box<dyn PacketRecorder>>>>;

/// What the network thread reports to the game.
pub enum Event {
    /// Progress text for the connecting screen.
    Status(String),
    /// The server's `minecraft:brand` custom payload.
    Brand(String),
    /// The server runs mods' quiet commands (`horizon_protocol::quiet`).
    QuietCommands,
    /// The server supports private custom recipe-book synchronization.
    CustomBooks,
    /// The server samples far terrain for this client (`lod_net`).
    FarTerrain(crate::lod_net::Offer),
    /// ClientConfigurationPacketListenerImpl.handleEnabledFeatures.
    Features(Vec<String>),
    /// Configuration finished: the play state begins with these registries.
    Joined {
        registries: Arc<Dynamic>,
        uuid: u128,
        name: String,
    },
    Packet(Box<play::Clientbound>),
    /// Registered application bytes, owned until the receiving root transaction ends.
    RawPayload(Box<crate::raw_payloads::Incoming>),
    /// A packet that failed to decode (skipped, like vanilla's skippable packets).
    DecodeError(String),
    /// The server sent us back to configuration (`ClientboundStartConfigurationPacket`):
    /// the game shows `ServerReconfigScreen` until the server logs it in again.
    Reconfiguring,
    /// The server sent us back to configuration and finished it again.
    Reconfigured(Arc<Dynamic>),
    /// ClientboundResourcePackPushPacket, in configuration or in play.
    PackPush(cfg::ResourcePackPush),
    /// ClientboundResourcePackPopPacket: one pack, or every pack.
    PackPop(Option<u128>),
    /// `show_dialog` in configuration: the dialog itself.
    ShowDialog(horizon_core::nbt::Tag),
    /// `clear_dialog` in configuration.
    ClearDialog,
    /// ClientboundCodeOfConductPacket: the game shows it and answers with
    /// `Session::accept_code_of_conduct` or a disconnect; until then the
    /// server waits in configuration.
    CodeOfConduct(String),
    /// `ClientboundTransferPacket`: the server sent us to another address.
    /// The connection ends right after; the game reconnects there with the
    /// stored cookies.
    Transfer {
        host: String,
        port: i32,
    },
    /// `ClientboundServerLinksPacket`: the links the pause menu's "Server
    /// Links..." lists (ClientCommonPacketListenerImpl.handleServerLinks).
    ServerLinks(Vec<cfg::ServerLink>),
    /// The connection ended. `from_server`: the server gave the reason (or
    /// the connection was refused for good), rather than the connection
    /// failing (auto-reconnect retries only failures and shutdowns).
    Disconnected {
        reason: String,
        /// The reason's translation key, when it had one
        /// (`reconnect::allowed` classifies by it).
        key: Option<String>,
        from_server: bool,
    },
}

fn event_channel() -> (ingress::Producer<Event>, ingress::Consumer<Event>) {
    ingress::channel(96 << 20, Event::ingress_charge).expect("positive incoming byte budget")
}
impl Event {
    fn ingress_charge(&self) -> usize {
        use horizon_protocol::retained::OwnedHeap;
        use std::mem::{align_of, size_of};
        let dynamic = |value: &Dynamic| {
            size_of::<Dynamic>() + 2 * size_of::<usize>() + align_of::<Dynamic>() - 1
                + value.fresh_owned_heap_bytes()
        };
        let owned = match self {
            Self::Status(s) | Self::Brand(s) | Self::DecodeError(s) | Self::CodeOfConduct(s) => {
                s.capacity()
            }
            Self::QuietCommands
            | Self::CustomBooks
            | Self::Reconfiguring
            | Self::ClearDialog
            | Self::PackPop(_) => 0,
            Self::FarTerrain(crate::lod_net::Offer {
                distance: _,
                in_flight: _,
                per_second: _,
            }) => 0,
            Self::Features(values) => values.owned_heap_bytes(),
            // configure only inserts/replaces before freezing these snapshots.
            Self::Joined {
                registries,
                uuid: _,
                name,
            } => dynamic(registries).saturating_add(name.capacity()),
            Self::Reconfigured(registries) => dynamic(registries),
            Self::Packet(packet) => {
                size_of::<play::Clientbound>().saturating_add(packet.as_ref().owned_heap_bytes())
            }
            Self::RawPayload(incoming) => size_of::<crate::raw_payloads::Incoming>()
                .saturating_add(incoming.owned_heap_bytes()),
            Self::PackPush(packet) => packet.owned_heap_bytes(),
            Self::ShowDialog(tag) => tag.owned_heap_bytes(),
            Self::Transfer { host, port: _ } => host.capacity(),
            Self::ServerLinks(links) => links.owned_heap_bytes(),
            Self::Disconnected {
                reason,
                key,
                from_server: _,
            } => reason.capacity().saturating_add(key.owned_heap_bytes()),
        };
        // Layout-derived allowance for a frozen receiving vector, actor backlog,
        // decoder marker and retained receipt, including vector spare capacity.
        // Raw application buffers themselves are never multiplied.
        owned.saturating_add(8 * size_of::<(Self, ingress::ReceiveLease<Self>, usize, usize)>())
    }
}

pub struct Session {
    events: ingress::Consumer<Event>,
    payload_events: ingress::Producer<Event>,
    payloads: Arc<crate::raw_payloads::Transport>,
    writer: Arc<Mutex<Option<QueuedWriter>>>,
    /// The connection is in configuration (else in play). Changed only
    /// with `writer` locked, as the packet that switches is sent.
    configuring: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    pub address: String,
    pub name: String,
    pub counters: Arc<Mutex<Option<Arc<Counters>>>>,
    /// The cookies this server stored on us (`ClientboundStoreCookiePacket`).
    pub cookies: Cookies,
    /// HORIZON_NET_DEBUG=1 logs every packet sent from the game thread.
    debug: bool,
    /// The far terrain requests waiting for the server (`lod_net`).
    far: Arc<crate::lod_net::Inbox>,
    /// The recording running, if any.
    recorder: RecorderSlot,
    /// This connection's number, new for each one ([`Session::serial`]).
    serial: u64,
}
/// The next [`Session::serial`].
static NEXT_SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
fn next_serial() -> u64 {
    NEXT_SERIAL.fetch_add(1, Ordering::Relaxed)
}
use std::path::PathBuf;

impl Session {
    /// A number of its own for each connection, never 0: entity ids are per connection, so what
    /// is kept by id goes when it changes (`horizon_hooks::players::PlayerModels::connection`).
    pub fn serial(&self) -> u64 {
        self.serial
    }
    /// The negotiated wire release, once the connection has been established.
    pub fn server_version(&self) -> horizon_protocol::adapter::ServerVersion {
        self.writer.lock().unwrap().as_ref().map_or(
            horizon_protocol::adapter::ServerVersion::Current,
            QueuedWriter::server_version,
        )
    }

    /// Secure-chat enforcement synchronized later than login on older servers.
    pub fn legacy_secure_chat_enforcement(&self) -> Option<bool> {
        self.writer
            .lock()
            .unwrap()
            .as_ref()
            .and_then(QueuedWriter::legacy_secure_chat_enforcement)
    }

    /// Start connecting in the background, as `auth`'s account when there is one.
    pub fn connect(
        address: &str,
        name: &str,
        auth: Option<Arc<AuthSession>>,
        info: cfg::ClientInformation,
        lang: Arc<Language>,
        resources: Arc<ItemData>,
    ) -> Self {
        Self::start(
            address, name, auth, info, lang, None, None, resources, false,
        )
    }
    /// `ClientHandshakePacketListenerImpl.handleHello`: LAN entries can continue
    /// after session authentication fails; the server still decides whether to admit them.
    /// Only for entries selected from LAN discovery, not addresses classified by IP.
    pub fn connect_lan(
        address: &str,
        name: &str,
        auth: Option<Arc<AuthSession>>,
        info: cfg::ClientInformation,
        lang: Arc<Language>,
        resources: Arc<ItemData>,
    ) -> Self {
        Self::start(address, name, auth, info, lang, None, None, resources, true)
    }
    /// `ConnectScreen.startConnecting` for a `transfer`: the cookies the
    /// server stored carry to the new connection (`TransferState`).
    pub fn connect_transferred(
        address: &str,
        name: &str,
        auth: Option<Arc<AuthSession>>,
        info: cfg::ClientInformation,
        lang: Arc<Language>,
        cookies: Cookies,
        resources: Arc<ItemData>,
    ) -> Self {
        Self::start(
            address,
            name,
            auth,
            info,
            lang,
            None,
            Some(cookies),
            resources,
            false,
        )
    }
    /// Join an integrated server through memory (`Connection.connectToLocalServer`). The
    /// hello still names the signed-in account (`Minecraft.handleClientLogin`'s
    /// `ServerboundHelloPacket(getUser().getName(), getUser().getProfileId())`), so the
    /// world's `players/data/<uuid>.dat` and `singleplayer_uuid` are the account's.
    pub fn connect_local(
        pipe: horizon_protocol::connection::Pipe,
        name: &str,
        auth: Option<Arc<AuthSession>>,
        info: cfg::ClientInformation,
        lang: Arc<Language>,
        resources: Arc<ItemData>,
    ) -> Self {
        Self::start(
            "local",
            name,
            auth,
            info,
            lang,
            Some(pipe),
            None,
            resources,
            false,
        )
    }
    /// A lightweight session for studio replay playback without background network IO.
    pub fn replay(name: &str) -> Self {
        let (_send, events) = event_channel();
        Self {
            payload_events: _send,
            events,
            writer: Arc::new(Mutex::new(None)),
            configuring: Arc::new(AtomicBool::new(false)),
            thread: None,
            payloads: crate::raw_payloads::Transport::empty(),
            address: "studio-replay".to_owned(),
            name: name.to_owned(),
            counters: Arc::new(Mutex::new(None)),
            cookies: Default::default(),
            debug: false,
            far: Arc::new(crate::lod_net::Inbox::default()),
            recorder: Arc::new(Mutex::new(None)),
            serial: next_serial(),
        }
    }
    fn start(
        address: &str,
        name: &str,
        auth: Option<Arc<AuthSession>>,
        info: cfg::ClientInformation,
        lang: Arc<Language>,
        local: Option<horizon_protocol::connection::Pipe>,
        cookies: Option<Cookies>,
        resources: Arc<ItemData>,
        lan: bool,
    ) -> Self {
        let (send, events) = event_channel();
        let payload_events = send.clone();
        let captured_payloads =
            crate::raw_payloads::Transport::capture(crate::raw_payloads::Side::Client);
        let payloads = captured_payloads
            .as_ref()
            .cloned()
            .unwrap_or_else(|_| crate::raw_payloads::Transport::empty());
        let thread_payloads = payloads.clone();
        let writer = Arc::new(Mutex::new(None));
        let configuring = Arc::new(AtomicBool::new(true));
        let counters = Arc::new(Mutex::new(None));
        // Connection.initiateServerboundPlayConnection: a transfer's
        // handshake says so (ClientIntent.TRANSFER).
        let transferred = cookies.is_some();
        let cookies = cookies.unwrap_or_default();
        let (thread_writer, thread_counters, thread_cookies) =
            (writer.clone(), counters.clone(), cookies.clone());
        let thread_configuring = configuring.clone();
        let far = Arc::new(crate::lod_net::Inbox::default());
        let thread_far = far.clone();
        let recorder = Arc::new(Mutex::new(None));
        let thread_recorder = recorder.clone();
        let (address_owned, name_owned) = (address.to_owned(), name.to_owned());
        // Payload handlers of wasm mods run on this thread (the client's network root).
        let thread = horizon_guest_stack::spawn("horizon-net".into(), move || {
            horizon_threads::favour_current();
            let result = captured_payloads.map_err(Reason::plain).and_then(|_| {
                run(
                    local,
                    &address_owned,
                    &name_owned,
                    auth.as_deref(),
                    info,
                    &lang,
                    &send,
                    &thread_writer,
                    &thread_configuring,
                    &thread_counters,
                    &thread_cookies,
                    &thread_far,
                    transferred,
                    &resources,
                    lan,
                    &thread_recorder,
                    &thread_payloads,
                )
            });
            thread_payloads.close();
            thread_far.cancel();
            let from_server = result.is_ok();
            let why = result.unwrap_or_else(|e| e);
            if let Some(w) = thread_writer.lock().unwrap().take() {
                w.shutdown();
            }
            let _ = send.send(Event::Disconnected {
                reason: why.text,
                key: why.key,
                from_server,
            });
        })
        .inspect_err(|error| {
            // No reader runs to report the end: say it here, or the session waits forever.
            let _ = payload_events.send(Event::Disconnected {
                reason: format!("network thread: {error}"),
                key: None,
                from_server: false,
            });
        })
        .ok();
        let debug = std::env::var_os("HORIZON_NET_DEBUG").is_some();
        Self {
            events,
            payload_events,
            payloads,
            writer,
            configuring,
            thread,
            address: address.to_owned(),
            name: name.to_owned(),
            counters,
            cookies,
            debug,
            far,
            recorder,
            serial: next_serial(),
        }
    }
    /// Validate an untrusted mod packet once, reserve its physical writer credit,
    /// and return an invisible root-publication record. This is not Session::send:
    /// registered/engine channels must use their authority-aware service API.
    pub fn prepare_mod_packet(
        &self,
        packet: mod_api::ServerboundPlayPacket,
    ) -> std::result::Result<PreparedPacket, mod_api::SendPacketError> {
        use horizon_protocol::{
            buf::{Codec, Ident, Reader, Writer},
            packets::custom_payload,
        };
        use mod_api::SendPacketError;
        let id = match packet.id.namespace() {
            "minecraft" => horizon_protocol::ids::play::serverbound::NAMES
                .iter()
                .position(|name| *name == packet.id.path()),
            _ => None,
        }
        .ok_or(SendPacketError::UnknownPacket { id: packet.id })? as i32;
        if id == horizon_protocol::ids::play::serverbound::CUSTOM_PAYLOAD {
            // Check ownership before copying a potentially large application body.
            // Ident is the same identifier boundary used by the packet codec.
            let mut reader = Reader::new(&packet.body);
            let channel = Ident::read(&mut reader).map_err(|_| SendPacketError::InvalidBody)?;
            if self.payloads.local.owns(&channel) || custom_payload::is_reserved_channel(&channel) {
                return Err(SendPacketError::ProtectedPayload {
                    id: mod_api::PayloadId(channel),
                });
            }
            if reader.remaining() > custom_payload::SERVERBOUND_LIMIT {
                return Err(SendPacketError::PayloadTooLarge {
                    size: reader.remaining() as u64,
                    limit: custom_payload::SERVERBOUND_LIMIT as u64,
                });
            }
        } else {
            play::Serverbound::decode(id, &packet.body)
                .map_err(|_| SendPacketError::InvalidBody)?;
        }
        let mut encoded = Writer::new();
        encoded.varint(id);
        encoded.bytes(&packet.body);
        let writer = self
            .writer
            .lock()
            .unwrap()
            .as_ref()
            .cloned()
            .ok_or(SendPacketError::NotConnected)?;
        writer.prepare_packet(encoded.data)
    }
    /// The frozen receiving turn, retaining every input credit through its actor.
    #[horizon_mod_api_macros::replaceable]
    pub fn poll(&self) -> Vec<ingress::OwnedReceipt<Event>> {
        self.events.take_turn()
    }
    /// Starts recording into `recorder`. `initialize` runs first, while the network thread
    /// can't record, so what it writes (a snapshot of the world) comes before any live packet.
    pub fn start_recording<R: PacketRecorder + 'static>(
        &self,
        recorder: R,
        initialize: impl FnOnce(&R),
    ) {
        let mut slot = self.recorder.lock().unwrap();
        initialize(&recorder);
        *slot = Some(Box::new(recorder));
    }
    /// Stop recording and finalize the `.mcpr` archive.
    pub fn stop_recording(&self) -> Option<std::io::Result<PathBuf>> {
        let rec = self.recorder.lock().unwrap().take()?;
        Some(rec.stop())
    }
    /// Whether recording is currently active.
    pub fn is_recording(&self) -> bool {
        self.recorder
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|r| r.is_active())
    }
    /// Status of the active recording, if any.
    pub fn recorder_status(&self) -> Option<RecorderStatus> {
        self.recorder.lock().unwrap().as_ref().map(|r| r.status())
    }
    /// Record an encoded Clientbound packet into the active recording session at current elapsed time.
    pub fn record_clientbound(&self, packet: &horizon_protocol::packets::play::Clientbound) {
        if self.is_recording() {
            let encoded = packet.encode();
            let mut reader = horizon_protocol::buf::Reader::new(&encoded);
            if let Ok(id) = reader.varint()
                && let Some(rec) = self.recorder.lock().unwrap().as_ref()
            {
                rec.record_packet(id, reader.rest());
            }
        }
    }
    /// Physical reply target, preserving this session's ordered event stream.
    pub fn payload_endpoint(&self) -> crate::raw_payloads::Endpoint {
        crate::raw_payloads::Endpoint::client(
            self.writer.clone(),
            self.configuring.clone(),
            self.payloads.clone(),
            self.payload_events.clone(),
        )
    }
    /// This physical session's frozen declarations and current epoch.
    pub fn payload_transport(&self) -> &Arc<crate::raw_payloads::Transport> {
        &self.payloads
    }
    /// Prepare a registered payload without publishing it before root commit.
    /// The caller resolves its channel against the frozen peer table first.
    pub fn prepare_payload(
        &self,
        phase: horizon_protocol::payload_negotiation::Phase,
        body: Vec<u8>,
        id: &mod_api::PayloadId,
    ) -> Result<PreparedPayload, mod_api::PayloadSendError> {
        let writer = self.writer.lock().unwrap();
        if self.configuring.load(Ordering::Relaxed)
            != (phase == horizon_protocol::payload_negotiation::Phase::Configuration)
        {
            return Err(mod_api::PayloadSendError::WrongProtocol(id.clone()));
        }
        writer
            .as_ref()
            .ok_or(mod_api::PayloadSendError::Disconnected)?
            .prepare_payload(body, id)
    }
    /// Send a play packet; silently dropped once the connection is gone,
    /// and while the server has the connection back in configuration (a
    /// proxy switching servers). Vanilla's `handleConfigurationStart` clears
    /// the level before it acknowledges, so no play packet follows the
    /// acknowledgement; one that did would reach the server as a
    /// configuration packet, and Velocity drops such a client.
    pub fn send(&self, packet: play::Serverbound) {
        if self.debug
            && !matches!(
                packet,
                play::Serverbound::ClientTickEnd(_)
                    | play::Serverbound::MovePlayerPos(_)
                    | play::Serverbound::MovePlayerPosRot(_)
            )
        {
            eprintln!("[net] -> {packet:?}");
        }
        let body = match packet.try_encode() {
            Ok(body) => body,
            Err(error) => {
                eprintln!("[net] send refused: {error}");
                return;
            }
        };
        let mut writer = self.writer.lock().unwrap();
        if self.configuring.load(Ordering::Relaxed) {
            return;
        }
        if let Some(w) = writer.as_mut()
            && w.send(&body).is_err()
        {
            w.shutdown();
        }
    }
    /// Far terrain the server samples, as it offered (`Event::FarTerrain`):
    /// biomes named by this client's indices, block states below `states`.
    pub fn far_terrain(
        &self,
        offer: crate::lod_net::Offer,
        biomes: HashMap<String, u16>,
        states: u32,
    ) -> Arc<crate::lod_net::RemoteSource> {
        crate::lod_net::RemoteSource::new(self.far.clone(), offer, biomes, states)
    }
    /// The far terrain requests its samplers queued go to the server, from
    /// the game thread, among its other packets.
    pub fn send_far_terrain(&self) {
        for data in self.far.requests() {
            self.send(play::Serverbound::CustomPayload(cfg::CustomPayload {
                channel: crate::lod_net::CHANNEL.into(),
                data,
            }));
        }
    }
    /// The far terrain made with [`Self::far_terrain`] ends: its samplers
    /// stop waiting for the server.
    pub fn cancel_far_terrain(&self) {
        self.far.cancel();
    }
    /// The connection is configured again (a proxy's next server): requests
    /// the old server held never come back, so their slots are freed.
    pub fn reset_far_terrain(&self) {
        self.far.reset();
    }
    /// ServerboundResourcePackPacket, in whichever phase the connection is.
    pub fn send_pack_status(&self, id: u128, action: i32) {
        let status = cfg::ResourcePackResponse { id, action };
        let mut writer = self.writer.lock().unwrap();
        let body = if self.configuring.load(Ordering::Relaxed) {
            cfg::Serverbound::ResourcePack(status).encode()
        } else {
            play::Serverbound::ResourcePack(status).encode()
        };
        if let Some(w) = writer.as_mut()
            && w.send(&body).is_err()
        {
            w.shutdown();
        }
    }
    /// ServerboundCustomClickActionPacket (a dialog's answer), in whichever
    /// phase the connection is.
    pub fn send_custom_click(&self, id: &str, payload: Option<horizon_core::nbt::Tag>) {
        let click = cfg::CustomClickAction {
            id: id.to_owned(),
            payload,
        };
        let mut writer = self.writer.lock().unwrap();
        let body = if self.configuring.load(Ordering::Relaxed) {
            cfg::Serverbound::CustomClickAction(click).encode()
        } else {
            play::Serverbound::CustomClickAction(click).encode()
        };
        if let Some(w) = writer.as_mut()
            && w.send(&body).is_err()
        {
            w.shutdown();
        }
    }
    /// ServerboundAcceptCodeOfConductPacket, sent in configuration.
    pub fn accept_code_of_conduct(&self) {
        let mut writer = self.writer.lock().unwrap();
        if let Some(w) = writer.as_mut()
            && w.send(&cfg::Serverbound::AcceptCodeOfConduct(Empty).encode())
                .is_err()
        {
            w.shutdown();
        }
    }
    /// `Connection.disconnect`: the socket closes and the game goes on at once.
    /// The reader thread is left to end on its own: on Windows a shutdown
    /// doesn't wake a read already waiting, so joining it held the window until
    /// the server dropped the connection, many seconds later.
    pub fn disconnect(&mut self) {
        self.payloads.close();
        if let Some(w) = self.writer.lock().unwrap().take() {
            w.shutdown();
        }
        drop(self.thread.take());
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.disconnect();
    }
}

/// An address of the form `friend:K7QM-3XPD` joins a friend's world through
/// its invite code (Play with Friends, through a hook's [`tunnel::Resolver`]):
/// the session goes through a tunnel the client reaches on a loopback port, so
/// everything after is a plain join.
pub const FRIEND: &str = tunnel::PREFIX;

/// Whether `code` is an invite code; with `full`, only a full one (a short
/// relay code could also be a host name). Without a resolver nothing is one.
pub fn is_invite(code: &str, full: bool) -> bool {
    horizon_hooks::engine::tunnel().is_some_and(|r| r.is_code(code, full))
}

/// Whether an invite code can be joined: a resolver is registered (horizon-official registers
/// horizon-friends'). The title screen's Friends icon, which opens Join a Friend, is inactive
/// without one, as the pause menu's is without a page to share from.
pub fn joins_friends() -> bool {
    #[cfg(test)]
    if JOINS_FRIENDS_IN_TEST.get() {
        return true;
    }
    horizon_hooks::engine::tunnel().is_some()
}
#[cfg(test)]
thread_local! {
    /// A test's stand-in for a registered resolver: the registry is the process's and sealed.
    pub(crate) static JOINS_FRIENDS_IN_TEST: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// An address as players and logs see it: a friend's invite code stays
/// out of it (its secret is in there).
pub fn shown_address(address: &str) -> String {
    match address.strip_prefix(FRIEND) {
        Some(code) => match horizon_hooks::engine::tunnel().and_then(|r| r.nameplate(code)) {
            Some(nameplate) => format!("Friend's world ({nameplate}-****)"),
            None => "Friend's world".to_owned(),
        },
        None => address.to_owned(),
    }
}

/// The relay the player set (`friendsRelay` in horizon.txt, beside
/// options.txt), else the resolver's (`HORIZON_RELAY_URL`). None by default.
pub fn friends_relay_setting() -> Option<String> {
    let dir = std::env::current_dir().ok()?;
    crate::ui_theme::preference(&dir, "friendsRelay")
        .filter(|r| !r.is_empty())
        .or_else(|| horizon_hooks::engine::tunnel().and_then(|r| r.relay_from_env()))
}
/// Remember the relay typed on Join a friend (empty forgets it).
pub fn save_friends_relay(relay: &str) {
    if let Ok(dir) = std::env::current_dir() {
        crate::ui_theme::set_preference(&dir, "friendsRelay", (!relay.is_empty()).then_some(relay));
    }
}

/// Reach the friend's world from the code (directly, or through a relay)
/// and open the tunnel: the loopback address to connect to, and the token to send on it first.
/// Without a resolver (a build without Play with Friends) the address is refused, never retried.
fn join_friend(
    code: &str,
    lang: &Language,
    send: &ingress::Producer<Event>,
) -> Result<(String, u16, Vec<u8>), Reason> {
    let Some(resolver) = horizon_hooks::engine::tunnel() else {
        return Err(Reason::keyed_arg(
            "horizon.multiplayer.invalid_address",
            lang,
            &shown_address(&format!("{FRIEND}{code}")),
        ));
    };
    let mut progress = |p: tunnel::Progress| {
        let key = match p {
            tunnel::Progress::Finding => "horizon.multiplayer.finding_friend",
            tunnel::Progress::Connected { direct: true } => {
                "horizon.multiplayer.connected_friend_direct"
            }
            tunnel::Progress::Connected { direct: false } => {
                "horizon.multiplayer.connected_friend_relay"
            }
        };
        status(send, &tr(lang, key));
    };
    let opened = resolver
        .open(code, friends_relay_setting(), &mut progress)
        .map_err(Reason::plain)?;
    Ok(("127.0.0.1".to_owned(), opened.port, opened.token))
}

fn status(send: &ingress::Producer<Event>, text: &str) {
    let _ = send.send(Event::Status(text.to_owned()));
}

/// Shown when an online-mode server asks a client without an account to authenticate.
pub const NOT_SIGNED_IN: &str = "This server is in online mode. Sign in with your Microsoft account in the Horizon launcher to join it.";

/// A translation, or the key itself (Language.getOrDefault).
fn tr(lang: &Language, key: &str) -> String {
    lang.get(key).cloned().unwrap_or_else(|| key.to_owned())
}

/// Why a connection ended: the shown text and, when the reason was a
/// translatable component or a known Horizon error, its translation key —
/// the auto-reconnect classifies by key (`reconnect::allowed`), the
/// English keyword scan of the text is the fallback for plain reasons.
#[derive(Clone)]
pub struct Reason {
    pub text: String,
    pub key: Option<String>,
}
impl Reason {
    fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            key: None,
        }
    }
    /// A vanilla key's text in the session's language, with the key.
    fn keyed(key: &str, lang: &Language) -> Self {
        Self {
            text: tr(lang, key),
            key: Some(key.to_owned()),
        }
    }
    /// A Horizon key's text with a `%s` argument filled.
    fn keyed_arg(key: &str, lang: &Language, arg: &str) -> Self {
        let mut r = Self::keyed(key, lang);
        r.text = r.text.replacen("%s", arg, 1);
        r
    }
    /// A server's disconnect component: the resolved text and the
    /// `translate` key when the component named one (servers' literal
    /// kicks have none; their text is the server's own language).
    fn component(reason_json: &str, lang: &Language) -> Self {
        match serde_json::from_str::<serde_json::Value>(reason_json) {
            Ok(v) => Self {
                text: json_text(&v, lang),
                key: v["translate"].as_str().map(str::to_owned),
            },
            Err(_) => Self::plain(reason_json),
        }
    }
    /// The play/configuration phases' disconnect: the reason arrives as
    /// an NBT component.
    fn tag(tag: &Tag, lang: &Language) -> Self {
        let key = match tag {
            Tag::Compound(map) => map
                .get(&JavaString::from("translate"))
                .and_then(|t| match t {
                    Tag::String(s) => s.to_string().ok(),
                    _ => None,
                }),
            _ => None,
        };
        Self {
            text: plain(tag, lang),
            key,
        }
    }
}

/// `ConnectScreen.connect`'s log line, as vanilla writes it from its "Server Connector #N" thread:
/// the address as the player gave it, and the port (25565 when none was given), before any SRV
/// lookup. Other launchers read the last one from `latest.log` to offer the server again.
fn connecting_line(host: &str, port: u16) -> String {
    format!("Connecting to {host}, {port}")
}
fn log_connecting(host: &str, port: u16) {
    // ConnectScreen.UNIQUE_THREAD_ID: the first connector is #1.
    static CONNECTORS: AtomicU32 = AtomicU32::new(0);
    let n = CONNECTORS.fetch_add(1, Ordering::Relaxed) + 1;
    crate::logging::write_as(
        &format!("Server Connector #{n}"),
        log::Level::Info,
        &connecting_line(host, port),
    );
}

/// `Connection$1.initChannel`: a client socket's pipeline starts with
/// `ReadTimeoutHandler(30)`. A server sends a keep-alive every 15 s, so one
/// that sends nothing for 30 s has frozen or dropped off the network.
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// `ConnectScreen.startConnecting`: the socket to a server, with `read_timeout`
/// ([`READ_TIMEOUT`]) on every read. The friends relay supplies a concrete
/// endpoint, not a Minecraft hostname, so it skips the SRV lookup. Returns the
/// resolved host and port for `ConnectScreen`'s login handshake.
fn connect_tcp(
    host: &str,
    port: u16,
    relayed: bool,
    read_timeout: Duration,
) -> Result<(Connection, String, u16), String> {
    let (c, host, port) = if relayed {
        Connection::connect(host, port, Duration::from_secs(10)).map(|c| (c, host.to_owned(), port))
    } else {
        Connection::connect_server_with_address(host, port, Duration::from_secs(10))
    }
    .map_err(|e| e.to_string())?;
    c.reader
        .set_read_timeout(Some(read_timeout))
        .map_err(|e| e.to_string())?;
    Ok((c, host, port))
}

/// Whether joining servers of older releases is on (roadmap `older-servers`): off by default while the
/// adapter is incomplete; `HORIZON_OLDER_SERVERS=1` turns it on.
pub fn older_servers_enabled() -> bool {
    std::env::var_os("HORIZON_OLDER_SERVERS").is_some_and(|v| v == "1")
}

/// The older release a server speaks, by its status ping, when Horizon can translate it; `None` for the
/// pinned version, an unknown one, a failed ping, or with the feature off.
fn older_server(address: &str) -> Option<horizon_protocol::adapter::ServerVersion> {
    use horizon_protocol::adapter::ServerVersion;
    if !older_servers_enabled() {
        return None;
    }
    let (json, _) = ping(address, Duration::from_secs(5)).ok()?;
    let protocol = i32::try_from(json["version"]["protocol"].as_i64()?).ok()?;
    ServerVersion::from_protocol(protocol).filter(|v| *v != ServerVersion::Current)
}

/// `Connection.exceptionCaught`: a read that timed out ends with
/// `disconnect.timeout`, any other failure with `disconnect.genericReason`
/// ("Internal Exception: ..."); a stream the server closed ends with
/// `disconnect.endOfStream` (`Connection.channelInactive`).
fn connection_lost(e: horizon_protocol::Error, lang: &Language) -> String {
    if e.is_timed_out() {
        tr(lang, "disconnect.timeout")
    } else if e.is_end_of_stream() {
        tr(lang, "disconnect.endOfStream")
    } else {
        plain(
            &translatable(
                "disconnect.genericReason",
                &[&format!("Internal Exception: {e}")],
            ),
            lang,
        )
    }
}

/// `ClientHandshakePacketListenerImpl.handleHello`: the whole connection.
/// Ok: why it ended for good (the server's reason, or
/// a refusal no retry fixes); Err: the connection failed.
fn run(
    local: Option<horizon_protocol::connection::Pipe>,
    address: &str,
    name: &str,
    auth: Option<&AuthSession>,
    info: cfg::ClientInformation,
    lang: &Language,
    send: &ingress::Producer<Event>,
    shared_writer: &Arc<Mutex<Option<QueuedWriter>>>,
    configuring: &Arc<AtomicBool>,
    shared_counters: &Mutex<Option<Arc<Counters>>>,
    cookies: &Cookies,
    far: &crate::lod_net::Inbox,
    transferred: bool,
    resources: &ItemData,
    lan: bool,
    recorder: &RecorderSlot,
    payloads: &Arc<crate::raw_payloads::Transport>,
) -> Result<Reason, Reason> {
    let (host, port, mut c) = if let Some(pipe) = local {
        (address.to_owned(), 0, Connection::local(pipe))
    } else {
        let (host, port, token) = match address.strip_prefix(FRIEND) {
            Some(code) => join_friend(code, lang, send)?,
            None => {
                let Some(target) = parse_address(address) else {
                    // A refusal: never retried (`horizon.multiplayer
                    // .invalid_address` is in reconnect's key list).
                    return Err(Reason::keyed_arg(
                        "horizon.multiplayer.invalid_address",
                        lang,
                        address,
                    ));
                };
                status(send, &tr(lang, "connect.connecting"));
                log_connecting(&target.0, target.1);
                (target.0, target.1, Vec::new())
            }
        };
        let (mut c, host, port) =
            connect_tcp(&host, port, address.starts_with(FRIEND), READ_TIMEOUT)
                .map_err(Reason::plain)?;
        // The tunnel's port takes only the client that sends its token first.
        if !token.is_empty() {
            c.writer
                .write_raw(&token)
                .map_err(|e| Reason::plain(e.to_string()))?;
        }
        if !address.starts_with(FRIEND)
            && let Some(version) = older_server(address)
        {
            log::info!("Joining {address} as a {} server", version.name());
            c.adapt(version);
        }
        (host, port, c)
    };
    *shared_counters.lock().unwrap() = Some(c.reader.counters.clone());
    let e = |e: horizon_protocol::Error| Reason::plain(connection_lost(e, lang));
    c.send(
        &handshake::Serverbound::Intention(handshake::Intention {
            protocol: PROTOCOL_VERSION,
            host: host.clone(),
            port,
            intent: if transferred {
                handshake::TRANSFER
            } else {
                handshake::LOGIN
            },
        })
        .encode(),
    )
    .map_err(e)?;
    // Vanilla's login sends the account's profile id, or the offline one.
    let uuid = auth.map_or_else(|| offline_uuid(name), |a| a.uuid);
    c.send(
        &login::Serverbound::Hello(login::ServerHello {
            name: name.to_owned(),
            uuid,
        })
        .encode(),
    )
    .map_err(e)?;
    let profile = loop {
        let (id, payload) = c.recv().map_err(e)?;
        match login::Clientbound::decode(id, &payload).map_err(e)? {
            login::Clientbound::Compression(p) => c.set_compression(p.threshold),
            login::Clientbound::LoginFinished(p) => {
                c.send(&login::Serverbound::LoginAcknowledged(Empty).encode())
                    .map_err(e)?;
                break p.profile;
            }
            login::Clientbound::Disconnect(p) => {
                return Ok(Reason::component(&p.reason_json, lang));
            }
            login::Clientbound::Hello(hello) => {
                status(send, &tr(lang, "connect.authorizing"));
                let key = RsaPublicKey::from_der(&hello.public_key)
                    .map_err(|err| Reason::plain(format!("Protocol error: {err}")))?;
                let secret = generate_secret().map_err(e)?;
                if hello.should_authenticate {
                    let signed_in = auth.is_some();
                    let authenticated = auth
                        .ok_or_else(|| Reason::keyed("horizon.multiplayer.not_signed_in", lang))
                        .and_then(|account| {
                            let hash = server_hash(&hello.server_id, &secret, &hello.public_key);
                            // The session service's refusal: the wrapped
                            // `disconnect.loginFailedInfo` text is shown, and
                            // the inner key (the reason's own) classifies
                            // (`reconnect::allowed`).
                            crate::auth::join_server(account, &hash).map_err(|err| {
                                let (reason, key) = match err.reason() {
                                    Ok(key) => (tr(lang, key), Some(key.to_owned())),
                                    Err(message) => (message.to_owned(), None),
                                };
                                Reason {
                                    text: plain(
                                        &translatable("disconnect.loginFailedInfo", &[&reason]),
                                        lang,
                                    ),
                                    key,
                                }
                            })
                        });
                    if let Err(reason) = authenticated {
                        if !continues_after_failed_join(lan, signed_in) {
                            return Err(reason);
                        }
                        log::warn!("Authentication failed: {}", reason.text);
                    }
                }
                status(send, &tr(lang, "connect.encrypting"));
                c.send(
                    &login::Serverbound::Key(login::Key {
                        key: key.encrypt(&secret).map_err(e)?,
                        challenge: key.encrypt(&hello.challenge).map_err(e)?,
                    })
                    .encode(),
                )
                .map_err(e)?;
                c.enable_encryption(&secret);
            }
            login::Clientbound::CustomQuery(q) => {
                c.send(
                    &login::Serverbound::CustomQueryAnswer(login::CustomQueryAnswer {
                        transaction: q.transaction,
                        payload: None,
                    })
                    .encode(),
                )
                .map_err(e)?;
            }
            // ClientHandshakePacketListenerImpl.handleRequestCookie: the
            // cookies a transfer carried over (none on a plain join).
            login::Clientbound::CookieRequest(q) => {
                let payload = cookies.lock().unwrap().get(&q.key).cloned();
                c.send(
                    &login::Serverbound::CookieResponse(login::CookieResponse {
                        key: q.key,
                        payload,
                    })
                    .encode(),
                )
                .map_err(e)?;
            }
            login::Clientbound::Unknown(..) => {}
        }
    };
    status(send, &tr(lang, "connect.joining"));
    // ClientHandshakePacketListenerImpl.handleLoginFinished: the brand, then
    // the client information.
    c.send(
        &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
            channel: "minecraft:brand".into(),
            data: brand(),
        })
        .try_encode()
        .map_err(e)?,
    )
    .map_err(e)?;
    c.send(&cfg::Serverbound::ClientInformation(info).encode())
        .map_err(e)?;
    let (mut reader, writer) = c.split();
    *shared_writer.lock().unwrap() =
        Some(QueuedWriter::new(writer).map_err(|error| Reason::plain(error.to_string()))?);
    let registries = configure(
        &mut reader,
        shared_writer,
        configuring,
        lang,
        send,
        cookies,
        resources,
        payloads,
    )?;
    let _ = send.send(Event::Joined {
        registries: Arc::new(registries),
        uuid: profile.uuid,
        name: profile.name,
    });
    receive_play(
        &mut reader,
        shared_writer,
        configuring,
        lang,
        send,
        cookies,
        far,
        resources,
        recorder,
        payloads,
    )
}

fn receive_play(
    reader: &mut ConnReader,
    shared_writer: &Arc<Mutex<Option<QueuedWriter>>>,
    configuring: &Arc<AtomicBool>,
    lang: &Language,
    send: &ingress::Producer<Event>,
    cookies: &Cookies,
    far: &crate::lod_net::Inbox,
    resources: &ItemData,
    recorder: &RecorderSlot,
    payloads: &Arc<crate::raw_payloads::Transport>,
) -> Result<Reason, Reason> {
    loop {
        let (id, payload) = reader
            .recv()
            .map_err(|e| Reason::plain(connection_lost(e, lang)))?;
        let packet = match play::Clientbound::decode_with_payload_codecs(
            id,
            &payload,
            &payloads.local.codecs,
        ) {
            Ok(p) => p,
            Err(err) => {
                if id == horizon_protocol::ids::play::clientbound::CUSTOM_PAYLOAD {
                    return Err(Reason::plain(format!("custom play payload refused: {err}")));
                }
                let name = horizon_protocol::ids::name(
                    horizon_protocol::State::Play,
                    horizon_protocol::Direction::Clientbound,
                    id,
                )
                .unwrap_or("?");
                let _ = send.send(Event::DecodeError(format!("{name}: {err}")));
                continue;
            }
        };
        if let Some(rec) = recorder.lock().unwrap().as_ref()
            && rec.is_active()
        {
            rec.received(&packet, id, &payload);
        }
        let reply = |p: play::Serverbound| {
            if let Some(w) = shared_writer.lock().unwrap().as_mut() {
                let _ = w.send(&p.encode());
            }
        };
        match packet {
            play::Clientbound::KeepAlive(k) => reply(play::Serverbound::KeepAlive(k)),
            play::Clientbound::BundleDelimiter(_) => {}
            play::Clientbound::ResourcePackPush(p) => {
                let _ = send.send(Event::PackPush(p));
            }
            play::Clientbound::ResourcePackPop(p) => {
                let _ = send.send(Event::PackPop(p.id));
            }
            play::Clientbound::StartConfiguration(_) => {
                payloads.begin_configuration();
                configuring.store(true, Ordering::Release);
                let _ = send.send(Event::Reconfiguring);
                if let Some(w) = shared_writer.lock().unwrap().as_mut() {
                    w.transition_protocol(
                        horizon_protocol::payload_negotiation::Phase::Configuration,
                        &play::Serverbound::ConfigurationAcknowledged(Empty).encode(),
                    )
                    .map_err(|error| Reason::plain(connection_lost(error, lang)))?;
                    configuring.store(true, Ordering::Relaxed);
                }
                let registries = configure(
                    reader,
                    shared_writer,
                    configuring,
                    lang,
                    send,
                    cookies,
                    resources,
                    payloads,
                )?;
                let _ = send.send(Event::Reconfigured(Arc::new(registries)));
            }
            // ClientCommonPacketListenerImpl.handleRequestCookie: the stored
            // cookie (or none) goes straight back.
            play::Clientbound::CookieRequest(q) => {
                let payload = cookies.lock().unwrap().get(&q.key).cloned();
                reply(play::Serverbound::CookieResponse(cfg::CookieResponse {
                    key: q.key,
                    payload,
                }));
            }
            play::Clientbound::StoreCookie(p) => {
                cookies.lock().unwrap().insert(p.key, p.payload);
            }
            play::Clientbound::ServerLinks(p) => {
                let _ = send.send(Event::ServerLinks(p.links));
            }
            // ClientCommonPacketListenerImpl.handleTransfer: the connection
            // ends and the game connects to the other address.
            play::Clientbound::Transfer(p) => {
                let _ = send.send(Event::Transfer {
                    host: p.host,
                    port: p.port,
                });
                return Ok(Reason::keyed("disconnect.transfer", lang));
            }
            play::Clientbound::Disconnect(d) => return Ok(Reason::tag(&d.reason, lang)),
            play::Clientbound::CustomPayload(p) if p.channel == "minecraft:brand" => {
                receive_brand(&p, send);
            }
            // A far tile, straight to the sampler waiting for it.
            play::Clientbound::CustomPayload(p) if p.channel == crate::lod_net::CHANNEL => {
                far.deliver(p.data);
            }
            play::Clientbound::CustomPayload(payload) => {
                let endpoint = crate::raw_payloads::Endpoint::client(
                    shared_writer.clone(),
                    configuring.clone(),
                    payloads.clone(),
                    send.clone(),
                );
                if let Some(payload) = receive_application(
                    &endpoint,
                    horizon_protocol::payload_negotiation::Phase::Play,
                    payload,
                    send,
                )? {
                    send.send(Event::Packet(Box::new(play::Clientbound::CustomPayload(
                        payload,
                    ))))
                    .map_err(|_| Reason::plain("Disconnected"))?;
                }
            }
            other => {
                if send.send(Event::Packet(Box::new(other))).is_err() {
                    return Ok(Reason::plain("Disconnected"));
                }
            }
        }
    }
}

fn receive_application(
    endpoint: &crate::raw_payloads::Endpoint,
    phase: horizon_protocol::payload_negotiation::Phase,
    payload: cfg::CustomPayload,
    send: &ingress::Producer<Event>,
) -> Result<Option<cfg::CustomPayload>, Reason> {
    match crate::raw_payloads::Incoming::classify(endpoint.transport(), phase, payload)
        .map_err(|error| Reason::plain(error.to_string()))?
    {
        crate::raw_payloads::Classified::Other(payload) => Ok(Some(payload)),
        crate::raw_payloads::Classified::Application(incoming) => {
            crate::raw_payloads::dispatch(
                incoming,
                endpoint.clone(),
                crate::raw_payloads::MainQueue::Client(send.clone()),
            );
            Ok(None)
        }
        // Raw channels are server-side: a client's table has none.
        crate::raw_payloads::Classified::Raw(_) => Ok(None),
    }
}

/// The configuration phase: registries, tags and known packs until the
/// server finishes it (ClientConfigurationPacketListenerImpl).
fn configure(
    reader: &mut ConnReader,
    writer: &Arc<Mutex<Option<QueuedWriter>>>,
    configuring: &Arc<AtomicBool>,
    lang: &Language,
    send: &ingress::Producer<Event>,
    cookies: &Cookies,
    resources: &ItemData,
    payloads: &Arc<crate::raw_payloads::Transport>,
) -> Result<Dynamic, Reason> {
    let mut registries = Dynamic::default();
    let mut content_verified = false;
    payloads.begin_configuration();
    let mut negotiation =
        horizon_protocol::payload_negotiation::Negotiation::new(payloads.local.manifest.clone());
    crate::datapack_net::clear_received();
    let reply = |p: cfg::Serverbound| -> Result<(), Reason> {
        match writer.lock().unwrap().as_mut() {
            Some(w) => p
                .try_encode_with_payload_codecs(&payloads.local.codecs)
                .and_then(|body| w.send(&body))
                .map_err(|e| Reason::plain(connection_lost(e, lang))),
            None => Err(Reason::plain("Disconnected")),
        }
    };
    if let Some(control) = negotiation.initial() {
        reply(cfg::Serverbound::CustomPayload(cfg::CustomPayload {
            channel: horizon_protocol::payload_negotiation::CHANNEL.into(),
            data: control
                .encode()
                .map_err(|error| Reason::plain(error.to_string()))?,
        }))?;
    }
    loop {
        let (id, payload) = reader
            .recv()
            .map_err(|e| Reason::plain(connection_lost(e, lang)))?;
        match cfg::Clientbound::decode_with_payload_codecs(id, &payload, &payloads.local.codecs)
            .map_err(|e| Reason::plain(connection_lost(e, lang)))?
        {
            cfg::Clientbound::KeepAlive(k) => reply(cfg::Serverbound::KeepAlive(k))?,
            cfg::Clientbound::CustomPayload(p)
                if p.channel == horizon_protocol::payload_negotiation::CHANNEL =>
            {
                let control = horizon_protocol::payload_negotiation::Control::decode(&p.data)
                    .map_err(|error| Reason::plain(error.to_string()))?;
                if let Some(control) = negotiation
                    .receive(control)
                    .map_err(|error| Reason::plain(error.to_string()))?
                {
                    reply(cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                        channel: horizon_protocol::payload_negotiation::CHANNEL.into(),
                        data: control
                            .encode()
                            .map_err(|error| Reason::plain(error.to_string()))?,
                    }))?;
                }
                if let Some(accepted) = negotiation.accepted() {
                    payloads.accept(accepted).map_err(Reason::plain)?;
                }
            }
            cfg::Clientbound::CustomPayload(p) if p.channel == crate::content_identity::CHANNEL => {
                let hash =
                    crate::content_identity::identity(resources.root(), resources.defaults())
                        .map_err(|_| {
                            Reason::plain(crate::content_identity::MISMATCH.to_string())
                        })?;
                content_verified = crate::content_identity::decode_payload(&p.data)
                    .map_err(|error| Reason::plain(error.to_string()))?
                    == Some(hash);
                reply(cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                    channel: crate::content_identity::CHANNEL.into(),
                    data: hash.to_vec(),
                }))?;
            }
            cfg::Clientbound::CustomPayload(p) if crate::viewed_levels_net::asked(&p) => {
                reply(crate::viewed_levels_net::answer(&p.data).map_err(Reason::plain)?)?
            }
            cfg::Clientbound::CustomPayload(p) if crate::menu_extra_net::asked(&p) => {
                reply(crate::menu_extra_net::answer(&p.data).map_err(Reason::plain)?)?;
                registries.menu_extra_revision = horizon_protocol::menu_extra::REVISION;
            }
            cfg::Clientbound::CustomPayload(p) if p.channel == crate::datapack_net::CHANNEL => {
                crate::datapack_net::accept_chunk(&p.data).map_err(Reason::plain)?;
            }
            cfg::Clientbound::CustomPayload(p) => {
                let endpoint = crate::raw_payloads::Endpoint::client(
                    writer.clone(),
                    configuring.clone(),
                    payloads.clone(),
                    send.clone(),
                );
                let Some(p) = receive_application(
                    &endpoint,
                    horizon_protocol::payload_negotiation::Phase::Configuration,
                    p,
                    send,
                )?
                else {
                    continue;
                };
                // Horizon servers use the client brand to negotiate their extensions.
                // Announce those only after the server identifies itself as Horizon.
                if p.channel == "minecraft:brand"
                    && horizon_protocol::buf::Reader::new(&p.data)
                        .string(32767)
                        .ok()
                        .as_deref()
                        == Some("horizon")
                {
                    if let Some(control) = negotiation.require_protocol() {
                        reply(cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                            channel: horizon_protocol::payload_negotiation::CHANNEL.into(),
                            data: control
                                .encode()
                                .map_err(|error| Reason::plain(error.to_string()))?,
                        }))?;
                    }
                    let mut w = horizon_protocol::buf::Writer::new();
                    w.string("horizon");
                    reply(cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                        channel: "minecraft:brand".into(),
                        data: w.data,
                    }))?;
                    // Announce before replying to known packs, even when the
                    // server has no mods. Never disclose this to vanilla servers.
                    reply(cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                        channel: horizon_protocol::mod_list::CHANNEL.into(),
                        data: horizon_protocol::mod_list::encode(
                            &horizon_protocol::mod_list::local(),
                        ),
                    }))?;
                    // And the mods' datapack registries it has (#811).
                    if let Some(data) = crate::datapack_net::offer() {
                        reply(cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                            channel: crate::datapack_net::OFFER_CHANNEL.into(),
                            data,
                        }))?;
                    }
                }
                receive_brand(&p, send);
            }
            cfg::Clientbound::Ping(p) => reply(cfg::Serverbound::Pong(p))?,
            cfg::Clientbound::SelectKnownPacks(p) => {
                reply(cfg::Serverbound::SelectKnownPacks(cfg::SelectKnownPacks {
                    packs: select_known_packs(p.packs),
                }))?
            }
            cfg::Clientbound::UpdateEnabledFeatures(p) => {
                let _ = send.send(Event::Features(p.features));
            }
            cfg::Clientbound::RegistryData(mut r) => {
                fill_known_entries(&mut r, resources.root());
                registries.add(r);
            }
            cfg::Clientbound::UpdateTags(t) => registries.add_tags(&t),
            cfg::Clientbound::CodeOfConduct(c) => {
                let _ = send.send(Event::CodeOfConduct(c.text));
            }
            // The game answers: it may prompt the player first.
            cfg::Clientbound::ResourcePackPush(p) => {
                let _ = send.send(Event::PackPush(p));
            }
            cfg::Clientbound::ResourcePackPop(p) => {
                let _ = send.send(Event::PackPop(p.id));
            }
            // The game shows it; the answer goes back from there.
            cfg::Clientbound::ShowDialog(d) => {
                let _ = send.send(Event::ShowDialog(d.dialog));
            }
            cfg::Clientbound::ClearDialog(_) => {
                let _ = send.send(Event::ClearDialog);
            }
            cfg::Clientbound::CookieRequest(q) => {
                let payload = cookies.lock().unwrap().get(&q.key).cloned();
                reply(cfg::Serverbound::CookieResponse(cfg::CookieResponse {
                    key: q.key,
                    payload,
                }))?
            }
            cfg::Clientbound::StoreCookie(p) => {
                cookies.lock().unwrap().insert(p.key, p.payload);
            }
            cfg::Clientbound::ServerLinks(p) => {
                let _ = send.send(Event::ServerLinks(p.links));
            }
            cfg::Clientbound::Transfer(p) => {
                let _ = send.send(Event::Transfer {
                    host: p.host,
                    port: p.port,
                });
                return Err(Reason::keyed("disconnect.transfer", lang));
            }
            cfg::Clientbound::Disconnect(d) => return Err(Reason::tag(&d.reason, lang)),
            cfg::Clientbound::FinishConfiguration(_) => {
                negotiation
                    .finish_without_offer()
                    .map_err(|error| Reason::plain(error.to_string()))?;
                if payloads.epoch().is_none() {
                    payloads
                        .accept(
                            negotiation
                                .accepted()
                                .expect("configuration barrier freezes negotiation"),
                        )
                        .map_err(Reason::plain)?;
                }
                crate::datapack_net::finish_transfer().map_err(Reason::plain)?;
                registries.custom_components = content_verified;
                payloads
                    .finish_configuration_with(|| {
                        let mut w = writer.lock().unwrap_or_else(|e| e.into_inner());
                        let w = w.as_mut().ok_or_else(|| "Disconnected".to_owned())?;
                        w.transition_protocol(
                            horizon_protocol::payload_negotiation::Phase::Play,
                            &cfg::Serverbound::FinishConfiguration(Empty).encode(),
                        )
                        .map_err(|e| connection_lost(e, lang))?;
                        configuring.store(false, Ordering::Release);
                        Ok(())
                    })
                    .map_err(Reason::plain)?;
                return Ok(registries);
            }
            _ => {}
        }
    }
}

#[horizon_mod_api_macros::replaceable]
fn receive_brand(payload: &cfg::CustomPayload, send: &ingress::Producer<Event>) {
    if payload.channel == "minecraft:brand"
        && let Ok(brand) = horizon_protocol::buf::Reader::new(&payload.data).string(32767)
    {
        let _ = send.send(Event::Brand(brand));
    } else if payload.channel == horizon_protocol::quiet::CHANNEL {
        let _ = send.send(Event::QuietCommands);
    } else if payload.channel == horizon_protocol::custom_book::CHANNEL {
        let _ = send.send(Event::CustomBooks);
    } else if payload.channel == crate::lod_net::CHANNEL
        && let Some(offer) = crate::lod_net::Offer::decode(&payload.data)
    {
        let _ = send.send(Event::FarTerrain(offer));
    }
}

/// `ClientBrandRetriever.getClientModName`: vanilla's default client brand.
fn brand() -> Vec<u8> {
    let mut w = horizon_protocol::buf::Writer::new();
    w.string("vanilla");
    w.data
}

/// Flatten a JSON chat component (login disconnects are JSON).
/// At most `horizon_protocol::text::MAX_TEXT` bytes, as `plain`: a login-phase reason from a
/// server is any JSON it likes.
pub fn json_text(v: &serde_json::Value, lang: &Language) -> String {
    use horizon_protocol::text::{MAX_TEXT, push_capped};
    match v {
        serde_json::Value::String(s) => {
            let mut out = String::new();
            push_capped(&mut out, s);
            out
        }
        serde_json::Value::Array(a) => {
            let mut out = String::new();
            for e in a {
                if out.len() >= MAX_TEXT {
                    break;
                }
                push_capped(&mut out, &json_text(e, lang));
            }
            out
        }
        serde_json::Value::Object(o) => {
            let mut out = if let Some(t) = o.get("text").and_then(|t| t.as_str()) {
                let mut out = String::new();
                push_capped(&mut out, t);
                out
            } else if let Some(key) = o.get("translate").and_then(|t| t.as_str()) {
                let args: Vec<String> = o
                    .get("with")
                    .and_then(|w| w.as_array())
                    .map(|a| a.iter().map(|e| json_text(e, lang)).collect())
                    .unwrap_or_default();
                let pattern = lang
                    .get(key)
                    .cloned()
                    .or_else(|| {
                        o.get("fallback")
                            .and_then(|f| f.as_str())
                            .map(str::to_owned)
                    })
                    .unwrap_or_else(|| key.to_owned());
                horizon_protocol::text::format_java(&pattern, &args)
            } else {
                String::new()
            };
            if let Some(extra) = o.get("extra").and_then(|e| e.as_array()) {
                for e in extra {
                    if out.len() >= MAX_TEXT {
                        break;
                    }
                    push_capped(&mut out, &json_text(e, lang));
                }
            }
            out
        }
        other => other.to_string(),
    }
}

/// The language a session resolves server-sent components with: the selected
/// one as `lang` loaded it (`en_us` under it, packs included), so a menu title
/// the server sends as a key follows the language screen. A process with no
/// language set (a dedicated server's log) reads en_us.json.
pub fn load_language(resources: &std::path::Path) -> Language {
    if let Some(lang) = crate::lang::current() {
        return lang.map().clone();
    }
    // No language screen ran (a dedicated server's log): en_us, with
    // Horizon's own strings included — the net thread resolves keys with
    // this map (`Reason::keyed`).
    horizon_client_core::lang::Language::load(&[&resources.to_path_buf()], "en_us")
        .map()
        .clone()
}

/// Ask a server for its status (ServerStatusPinger): MOTD, players and the
/// round trip time in milliseconds.
pub fn ping(address: &str, timeout: Duration) -> Result<(serde_json::Value, u128), String> {
    use horizon_protocol::packets::status;
    let (host, port) = parse_address(address).ok_or("Invalid address")?;
    let mut c = Connection::connect_server(&host, port, timeout).map_err(|e| e.to_string())?;
    c.reader
        .set_read_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
    let e = |e: horizon_protocol::Error| e.to_string();
    c.send(
        &handshake::Serverbound::Intention(handshake::Intention {
            protocol: PROTOCOL_VERSION,
            host: host.clone(),
            port,
            intent: handshake::STATUS,
        })
        .encode(),
    )
    .map_err(e)?;
    c.send(&status::Serverbound::StatusRequest(Empty).encode())
        .map_err(e)?;
    let (id, payload) = c.recv().map_err(e)?;
    let json = match status::Clientbound::decode(id, &payload).map_err(e)? {
        status::Clientbound::StatusResponse(s) => {
            serde_json::from_str(&s.json).map_err(|e| e.to_string())?
        }
        _ => return Err("unexpected status reply".into()),
    };
    let start = std::time::Instant::now();
    c.send(&status::Serverbound::PingRequest(status::Ping { time: 1 }).encode())
        .map_err(e)?;
    let _ = c.recv().map_err(e)?;
    Ok((json, start.elapsed().as_millis()))
}

/// What a server that answers only the legacy ping reports
/// (`LegacyServerPinger.Output.handleResponse`).
#[derive(Clone, Debug, PartialEq)]
pub struct LegacyStatus {
    pub protocol: i32,
    pub version: String,
    pub motd: String,
    pub players: i32,
    pub max_players: i32,
}

/// `Mth.getInt(str, default)`.
fn legacy_int(s: &str, default: i32) -> i32 {
    s.parse().unwrap_or(default)
}

/// `LegacyServerPinger.channelRead0` on the string after the 0xFF byte: split
/// on NUL (at most 6 parts) and the first part must be `§1`.
fn parse_legacy_status(text: &str) -> Option<LegacyStatus> {
    let parts: Vec<&str> = text.splitn(6, '\0').collect();
    if parts.first() != Some(&"\u{a7}1") || parts.len() < 6 {
        return None;
    }
    Some(LegacyStatus {
        protocol: legacy_int(parts[1], 0),
        version: parts[2].to_owned(),
        motd: parts[3].to_owned(),
        players: legacy_int(parts[4], -1),
        max_players: legacy_int(parts[5], -1),
    })
}

/// `LegacyProtocolUtils.writeLegacyString`: the UTF-16 length, then UTF-16BE.
fn write_legacy_string(out: &mut Vec<u8>, s: &str) {
    let units: Vec<u16> = s.encode_utf16().collect();
    out.extend_from_slice(&(units.len() as u16).to_be_bytes());
    for unit in units {
        out.extend_from_slice(&unit.to_be_bytes());
    }
}

/// `LegacyServerPinger.channelActive`: the 1.6 `0xFE 0x01 0xFA MC|PingHost` request.
fn legacy_ping_request(host: &str, port: u16) -> Vec<u8> {
    let mut out = vec![254, 1, 250];
    write_legacy_string(&mut out, "MC|PingHost");
    let mut payload = vec![127];
    write_legacy_string(&mut payload, host);
    payload.extend_from_slice(&i32::from(port).to_be_bytes());
    out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    out.extend_from_slice(&payload);
    out
}

/// `ServerStatusPinger.pingLegacyServer`: ask a server that does not answer
/// the status handshake with the pre-1.7 ping.
pub fn ping_legacy(address: &str, timeout: Duration) -> Result<LegacyStatus, String> {
    let (host, port) = parse_address(address).ok_or("Invalid address")?;
    let mut c = Connection::connect_server(&host, port, timeout).map_err(|e| e.to_string())?;
    c.reader
        .set_read_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
    let e = |e: horizon_protocol::Error| e.to_string();
    c.writer
        .write_raw(&legacy_ping_request(&host, port))
        .map_err(e)?;
    let head = c.reader.read_raw(3).map_err(e)?;
    if head[0] != 255 {
        return Err("unexpected legacy reply".into());
    }
    let units = usize::from(u16::from_be_bytes([head[1], head[2]]));
    let bytes = c.reader.read_raw(units * 2).map_err(e)?;
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| u16::from_be_bytes(*p))
        .collect();
    parse_legacy_status(&String::from_utf16_lossy(&units))
        .ok_or_else(|| "unexpected legacy reply".to_owned())
}

/// `ClientHandshakePacketListenerImpl.handleHello`: a failed session join goes
/// on to encryption only for an entry picked from LAN discovery
/// (`ServerData.isLan`), and the host decides. A vanilla client always has an
/// account; Horizon without one never joins anything (Mojang's rules: no play
/// without an account that owns the game), LAN included.
fn continues_after_failed_join(lan: bool, signed_in: bool) -> bool {
    lan && signed_in
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_world::native_load_context::DataLoadOrigin;

    fn empty_item_data(name: &str) -> (PathBuf, Arc<ItemData>) {
        let root =
            std::env::temp_dir().join(format!("horizon-net-data-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("items.json"), "{}").unwrap();
        let data = crate::test_resources::item_data(&root, DataLoadOrigin::ClientPreload).unwrap();
        (root, data)
    }

    #[test]
    fn horizon_server_without_raw_protocol_is_refused_at_finish() {
        for wrong_version in [false, true] {
            let (root, data) = empty_item_data("raw-protocol-refusal");
            let (client, server) = horizon_protocol::connection::Pipe::pair();
            let (mut reader, writer) = Connection::local(client).split();
            reader
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let thread = std::thread::spawn(move || {
                let (send, _events) = event_channel();
                configure(
                    &mut reader,
                    &Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap()))),
                    &Arc::new(AtomicBool::new(true)),
                    &Language::new(),
                    &send,
                    &Cookies::default(),
                    &data,
                    &crate::raw_payloads::Transport::empty(),
                )
            });
            let mut server = Connection::local(server);
            let mut brand = horizon_protocol::buf::Writer::new();
            brand.string("horizon");
            server
                .send(
                    &cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                        channel: "minecraft:brand".into(),
                        data: brand.data,
                    })
                    .encode(),
                )
                .unwrap();
            if wrong_version {
                server
                    .send(
                        &cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                            channel: horizon_protocol::payload_negotiation::CHANNEL.into(),
                            data: vec![0, horizon_protocol::payload_negotiation::RAW_PROTOCOL + 1],
                        })
                        .encode(),
                    )
                    .unwrap();
            } else {
                server
                    .send(&cfg::Clientbound::FinishConfiguration(Empty).encode())
                    .unwrap();
            }
            let Err(error) = thread.join().unwrap() else {
                panic!("old Horizon server admitted");
            };
            assert!(error.text.contains(if wrong_version {
                "protocol version"
            } else {
                "missing response"
            }));
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn configuration_keeps_mod_lists_private_until_the_server_identifies_as_horizon() {
        for server_brand in ["vanilla", "horizon"] {
            let (root, data) = empty_item_data(&format!("mod-list-{server_brand}"));
            let (client, server) = horizon_protocol::connection::Pipe::pair();
            let (mut reader, writer) = Connection::local(client).split();
            reader
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            // configure queues its final ACK asynchronously. Keep its writer alive
            // until the peer has received that ACK, just as the real connection
            // retains it for play; dropping the last owner closes the pipe.
            let shared_writer = Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap())));
            let client_writer = shared_writer.clone();
            let thread = std::thread::spawn(move || {
                let (send, _events) = event_channel();
                configure(
                    &mut reader,
                    &client_writer,
                    &Arc::new(AtomicBool::new(true)),
                    &Language::new(),
                    &send,
                    &Cookies::default(),
                    &data,
                    &crate::raw_payloads::Transport::empty(),
                )
            });
            let mut server = Connection::local(server);
            server
                .reader
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            // A payload alone must not solicit private identities.
            server
                .send(
                    &cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                        channel: horizon_protocol::mod_list::CHANNEL.into(),
                        data: horizon_protocol::mod_list::encode(&[]),
                    })
                    .encode(),
                )
                .unwrap();
            server
                .send(&cfg::Clientbound::Ping(cfg::Ping { id: 17 }).encode())
                .unwrap();
            let (id, body) = server.recv().unwrap();
            assert_eq!(
                cfg::Serverbound::decode(id, &body).unwrap(),
                cfg::Serverbound::Pong(cfg::Ping { id: 17 }),
                "an unidentified server received a mod announcement"
            );
            let mut brand = horizon_protocol::buf::Writer::new();
            brand.string(server_brand);
            server
                .send(
                    &cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                        channel: "minecraft:brand".into(),
                        data: brand.data,
                    })
                    .encode(),
                )
                .unwrap();
            if server_brand == "horizon" {
                use horizon_protocol::payload_negotiation::{CHANNEL, Control};
                server
                    .send(
                        &cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                            channel: CHANNEL.into(),
                            data: Control::Query.encode().unwrap(),
                        })
                        .encode(),
                    )
                    .unwrap();
            }
            server
                .send(
                    &cfg::Clientbound::SelectKnownPacks(cfg::SelectKnownPacks { packs: vec![] })
                        .encode(),
                )
                .unwrap();
            let mut declared_mods = false;
            let mut saw_offer = false;
            loop {
                let (id, body) = server.recv().unwrap();
                match cfg::Serverbound::decode(id, &body).unwrap() {
                    cfg::Serverbound::CustomPayload(p)
                        if p.channel == horizon_protocol::payload_negotiation::CHANNEL =>
                    {
                        use horizon_protocol::payload_negotiation::{CHANNEL, Control, Manifest};
                        if Control::decode(&p.data).unwrap() == Control::Query {
                            server
                                .send(
                                    &cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                                        channel: CHANNEL.into(),
                                        data: Control::Offer(Manifest::default()).encode().unwrap(),
                                    })
                                    .encode(),
                                )
                                .unwrap();
                        } else {
                            saw_offer = true;
                        }
                    }
                    cfg::Serverbound::CustomPayload(p)
                        if p.channel == horizon_protocol::mod_list::CHANNEL =>
                    {
                        declared_mods = true;
                    }
                    cfg::Serverbound::SelectKnownPacks(_) => break,
                    _ => {}
                }
            }
            assert_eq!(declared_mods, server_brand == "horizon");
            assert_eq!(saw_offer, server_brand == "horizon");
            server
                .send(&cfg::Clientbound::FinishConfiguration(Empty).encode())
                .unwrap();
            let (id, body) = server.recv().unwrap();
            assert_eq!(
                cfg::Serverbound::decode(id, &body).unwrap(),
                cfg::Serverbound::FinishConfiguration(Empty)
            );
            thread
                .join()
                .unwrap()
                .unwrap_or_else(|reason| panic!("configuration failed: {}", reason.text));
            drop(shared_writer);
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn a_login_reason_repeating_its_argument_stays_bounded() {
        let pattern = "%1$s".repeat(16_000);
        let mut v = serde_json::json!({ "text": "A" });
        for _ in 0..3 {
            v = serde_json::json!({ "translate": "x.unknown", "fallback": pattern, "with": [v] });
        }
        let v = serde_json::json!({ "text": "", "extra": [v.clone(), v] });
        let text = json_text(&v, &Language::default());
        assert!(text.len() <= horizon_protocol::text::MAX_TEXT);
    }

    fn pack(namespace: &str, id: &str, version: &str) -> cfg::KnownPack {
        cfg::KnownPack {
            namespace: namespace.into(),
            id: id.into(),
            version: version.into(),
        }
    }

    #[test]
    fn the_client_knows_vanilla_core_pack_of_its_own_version_only() {
        let core = pack("minecraft", "core", horizon_protocol::GAME_VERSION);
        let offered = vec![
            core.clone(),
            pack(
                "minecraft",
                "trade_rebalance",
                horizon_protocol::GAME_VERSION,
            ),
            pack("example", "server_pack", "1"),
        ];
        assert_eq!(select_known_packs(offered), vec![core]);
        assert_eq!(
            select_known_packs(vec![pack("minecraft", "core", "1.21.11")]),
            vec![]
        );
        assert_eq!(select_known_packs(vec![]), vec![]);
    }

    #[test]
    fn a_known_entry_takes_its_data_from_the_clients_pack() {
        let root = std::env::temp_dir().join(format!("hz-known-entries-{}", std::process::id()));
        let dir = root.join("data/minecraft/damage_type");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("in_fire.json"),
            br#"{"message_id": "inFire", "exhaustion": 0.1, "scaling": "when_caused_by_living_non_player"}"#,
        )
        .unwrap();
        let sent = Tag::compound(vec![(
            "message_id",
            Tag::String(JavaString::from("custom")),
        )]);
        let mut r = cfg::RegistryData {
            registry: "minecraft:damage_type".into(),
            entries: vec![
                cfg::RegistryEntry {
                    id: "minecraft:in_fire".into(),
                    data: None,
                },
                cfg::RegistryEntry {
                    id: "minecraft:custom".into(),
                    data: Some(sent.clone()),
                },
                cfg::RegistryEntry {
                    id: "minecraft:missing".into(),
                    data: None,
                },
            ],
        };
        fill_known_entries(&mut r, &root);
        std::fs::remove_dir_all(&root).unwrap();
        let Some(Tag::Compound(filled)) = &r.entries[0].data else {
            panic!("in_fire was not filled: {:?}", r.entries[0].data);
        };
        assert_eq!(
            filled.get(&JavaString::from("message_id")),
            Some(&Tag::String(JavaString::from("inFire")))
        );
        // Data the server sent stays; an entry the client has no file for stays empty.
        assert_eq!(r.entries[1].data, Some(sent));
        assert_eq!(r.entries[2].data, None);
    }

    /// Launchers find the last server joined in `latest.log` by vanilla's line.
    #[test]
    fn the_connecting_line_is_vanillas() {
        let line = |address: &str| {
            let (host, port) = parse_address(address).unwrap();
            connecting_line(&host, port)
        };
        assert_eq!(
            line("play.example.net"),
            "Connecting to play.example.net, 25565"
        );
        assert_eq!(
            line("play.example.net:25577"),
            "Connecting to play.example.net, 25577"
        );
        assert_eq!(line("[::1]:25566"), "Connecting to ::1, 25566");
    }

    #[test]
    #[ignore = "daily"]
    fn direct_login_preserves_hostname_and_negotiates_brand() {
        let (root, data) = empty_item_data("brand");
        for server_brand in [
            b"\x07vanilla".as_slice(),
            b"\x07horizon",
            b"\x05Paper",
            b"\xff",
        ] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let session = Session::connect(
                &format!("localhost:{port}"),
                "Probe",
                None,
                cfg::ClientInformation::new(8),
                Arc::new(Language::new()),
                Arc::clone(&data),
            );
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut server = Connection::from_stream(stream).unwrap();
            let (id, body) = server.recv().unwrap();
            let handshake::Serverbound::Intention(packet) =
                handshake::Serverbound::decode(id, &body).unwrap()
            else {
                panic!("expected login handshake")
            };
            assert_eq!((packet.host.as_str(), packet.port), ("localhost", port));
            assert_eq!(packet.intent, handshake::LOGIN);
            server.recv().unwrap(); // login profile
            server
                .send(
                    &login::Clientbound::LoginFinished(login::LoginFinished {
                        profile: login::GameProfile {
                            uuid: 1,
                            name: "Probe".into(),
                            properties: vec![],
                        },
                        session: 1,
                    })
                    .encode(),
                )
                .unwrap();
            let (id, body) = server.recv().unwrap();
            assert!(matches!(
                login::Serverbound::decode(id, &body).unwrap(),
                login::Serverbound::LoginAcknowledged(_)
            ));
            // handleLoginFinished: the brand first, then the information.
            let (id, body) = server.recv().unwrap();
            assert_eq!(
                cfg::Serverbound::decode(id, &body).unwrap(),
                cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                    channel: "minecraft:brand".into(),
                    data: b"\x07vanilla".to_vec(),
                })
            );
            let (id, body) = server.recv().unwrap();
            assert!(matches!(
                cfg::Serverbound::decode(id, &body).unwrap(),
                cfg::Serverbound::ClientInformation(_)
            ));
            server
                .send(
                    &cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                        channel: "minecraft:brand".into(),
                        data: server_brand.to_vec(),
                    })
                    .encode(),
                )
                .unwrap();
            if server_brand == b"\xff" {
                // A registered brand uses its own string codec: malformed
                // bytes fail the session rather than continuing negotiation.
                assert!(server.recv().unwrap_err().is_end_of_stream());
                drop(server);
                drop(session);
                continue;
            }
            if server_brand == b"\x07horizon" {
                use horizon_protocol::payload_negotiation::{CHANNEL, Control};
                server
                    .send(
                        &cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                            channel: CHANNEL.into(),
                            data: Control::Query.encode().unwrap(),
                        })
                        .encode(),
                    )
                    .unwrap();
            }
            server
                .send(
                    &cfg::Clientbound::SelectKnownPacks(cfg::SelectKnownPacks { packs: vec![] })
                        .encode(),
                )
                .unwrap();
            if server_brand == b"\x07horizon" {
                use horizon_protocol::payload_negotiation::{CHANNEL, Control, Manifest};
                let (id, body) = server.recv().unwrap();
                assert_eq!(
                    cfg::Serverbound::decode(id, &body).unwrap(),
                    cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                        channel: CHANNEL.into(),
                        data: Control::Query.encode().unwrap(),
                    })
                );
                server
                    .send(
                        &cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                            channel: CHANNEL.into(),
                            data: Control::Offer(Manifest::default()).encode().unwrap(),
                        })
                        .encode(),
                    )
                    .unwrap();
                let (id, body) = server.recv().unwrap();
                assert_eq!(
                    cfg::Serverbound::decode(id, &body).unwrap(),
                    cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                        channel: "minecraft:brand".into(),
                        data: b"\x07horizon".to_vec(),
                    })
                );
            }
            // Wait for the pack reply without pinning unrelated extension payloads.
            // The negotiated brand must not be announced a second time.
            let mut saw_protocol_offer = false;
            loop {
                let (id, body) = server.recv().unwrap();
                match cfg::Serverbound::decode(id, &body).unwrap() {
                    cfg::Serverbound::SelectKnownPacks(_) => break,
                    cfg::Serverbound::CustomPayload(p) => {
                        assert_ne!(p.channel, "minecraft:brand");
                        if p.channel == horizon_protocol::payload_negotiation::CHANNEL {
                            use horizon_protocol::payload_negotiation::{Control, Manifest};
                            assert_eq!(
                                Control::decode(&p.data).unwrap(),
                                Control::Offer(Manifest::default())
                            );
                            assert!(!saw_protocol_offer, "duplicate protocol offer");
                            saw_protocol_offer = true;
                        }
                    }
                    packet => panic!("unexpected configuration reply: {packet:?}"),
                }
            }
            assert_eq!(saw_protocol_offer, server_brand == b"\x07horizon");
            drop(server);
            drop(session);
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_ping_request_is_the_1_6_ping_host_packet() {
        let bytes = legacy_ping_request("a.b", 25565);
        assert_eq!(&bytes[..3], &[254, 1, 250]);
        // "MC|PingHost": 11 chars.
        assert_eq!(&bytes[3..5], &[0, 11]);
        let after = 5 + 22;
        // payload: 127, string (2 + 6 bytes), port.
        assert_eq!(&bytes[after..after + 2], &[0, 1 + 2 + 6 + 4]);
        assert_eq!(bytes[after + 2], 127);
        assert_eq!(&bytes[after + 3..after + 5], &[0, 3]);
        assert_eq!(&bytes[bytes.len() - 4..], &25565i32.to_be_bytes());
    }

    #[test]
    fn legacy_status_is_split_on_nul_after_the_section_one_marker() {
        let status =
            parse_legacy_status("\u{a7}1\x00127\x001.6.4\x00A \u{a7}aserver\x003\x0020").unwrap();
        assert_eq!(
            status,
            LegacyStatus {
                protocol: 127,
                version: "1.6.4".into(),
                motd: "A \u{a7}aserver".into(),
                players: 3,
                max_players: 20,
            }
        );
        // Unparsable counts are -1, a wrong marker is no status.
        let status = parse_legacy_status("\u{a7}1\x00x\x00v\x00m\x00?\x00?").unwrap();
        assert_eq!(
            (status.protocol, status.players, status.max_players),
            (0, -1, -1)
        );
        assert!(parse_legacy_status("kick\x00127\x00v\x00m\x001\x002").is_none());
    }

    /// The en_us text of the disconnect reasons.
    fn disconnect_language() -> Language {
        [
            ("disconnect.timeout", "Timed out"),
            ("disconnect.endOfStream", "End of stream"),
            ("disconnect.genericReason", "%s"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
    }

    #[test]
    #[ignore = "daily"]
    fn a_server_that_stops_answering_times_out() {
        // The server accepts and then sends nothing, like a frozen server
        // or a network gone without a reset.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let held = std::thread::spawn(move || listener.accept().map(|(stream, _)| stream));
        let (c, _, _) = connect_tcp("127.0.0.1", port, true, Duration::from_millis(300)).unwrap();
        let _server = held.join().unwrap().unwrap();
        let (mut reader, writer) = c.split();
        let (root, data) = empty_item_data("timeout");
        let (done, result) = channel();
        std::thread::spawn(move || {
            let (send, _events) = event_channel();
            let _ = done.send(receive_play(
                &mut reader,
                &Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap()))),
                &Arc::new(AtomicBool::new(false)),
                &disconnect_language(),
                &send,
                &Cookies::default(),
                &crate::lod_net::Inbox::default(),
                &data,
                &Arc::new(Mutex::new(None)),
                &crate::raw_payloads::Transport::empty(),
            ));
        });
        let ended = result
            .recv_timeout(Duration::from_secs(10))
            .expect("the connection never timed out");
        assert_eq!(
            ended.map(|r| r.text).map_err(|r| r.text),
            Err("Timed out".to_owned())
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lost_connections_read_like_vanillas_disconnect_reasons() {
        let lang = disconnect_language();
        let lost = |e| connection_lost(e, &lang);
        assert_eq!(lost(horizon_protocol::Error::timed_out()), "Timed out");
        assert_eq!(
            lost(horizon_protocol::Error::end_of_stream()),
            "End of stream"
        );
        assert_eq!(
            lost(horizon_protocol::Error::new("Bad VarInt")),
            "Internal Exception: Bad VarInt"
        );
    }

    /// #3716: a client asked about viewed levels answers in configuration, before its
    /// acknowledgement, that it views two levels besides its own.
    #[test]
    fn configuration_answers_the_viewed_levels_query_with_its_cap() {
        let (root, data) = empty_item_data("viewed-levels");
        let (client, server) = horizon_protocol::connection::Pipe::pair();
        let (mut reader, writer) = Connection::local(client).split();
        let writer = Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap())));
        let client_writer = writer.clone();
        let thread = std::thread::spawn(move || {
            let (send, _events) = event_channel();
            configure(
                &mut reader,
                &client_writer,
                &Arc::new(AtomicBool::new(true)),
                &Language::new(),
                &send,
                &Cookies::default(),
                &data,
                &crate::raw_payloads::Transport::empty(),
            )
        });
        let mut server = Connection::local(server);
        for packet in [
            cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                channel: "horizon:levels".into(),
                data: horizon_protocol::viewed_levels::query(),
            }),
            cfg::Clientbound::FinishConfiguration(Empty),
        ] {
            server.send(&packet.encode()).unwrap();
        }
        for expected in [
            cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                channel: "horizon:levels".into(),
                data: horizon_protocol::viewed_levels::answer(2),
            }),
            cfg::Serverbound::FinishConfiguration(Empty),
        ] {
            let (id, body) = server.recv().unwrap();
            assert_eq!(cfg::Serverbound::decode(id, &body).unwrap(), expected);
        }
        assert!(thread.join().unwrap().is_ok());
        drop(writer);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// #3384: a client asked which `horizon:menu_extra` layout it reads answers its revision
    /// in configuration, and keeps it for play (a server that never asked serves the legacy layout).
    #[test]
    fn configuration_answers_the_menu_extra_revision_query() {
        let (root, data) = empty_item_data("menu-extra-revision");
        let (client, server) = horizon_protocol::connection::Pipe::pair();
        let (mut reader, writer) = Connection::local(client).split();
        let writer = Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap())));
        let client_writer = writer.clone();
        let thread = std::thread::spawn(move || {
            let (send, _events) = event_channel();
            configure(
                &mut reader,
                &client_writer,
                &Arc::new(AtomicBool::new(true)),
                &Language::new(),
                &send,
                &Cookies::default(),
                &data,
                &crate::raw_payloads::Transport::empty(),
            )
        });
        let mut server = Connection::local(server);
        for packet in [
            cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                channel: "horizon:menu_extra".into(),
                data: horizon_protocol::menu_extra::query(),
            }),
            cfg::Clientbound::FinishConfiguration(Empty),
        ] {
            server.send(&packet.encode()).unwrap();
        }
        for expected in [
            cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                channel: "horizon:menu_extra".into(),
                data: horizon_protocol::menu_extra::answer(),
            }),
            cfg::Serverbound::FinishConfiguration(Empty),
        ] {
            let (id, body) = server.recv().unwrap();
            assert_eq!(cfg::Serverbound::decode(id, &body).unwrap(), expected);
        }
        // A failure reads as 0 and fails the assertion below.
        let revision = thread
            .join()
            .unwrap()
            .map(|registries| registries.menu_extra_revision)
            .unwrap_or(0);
        drop(writer);
        std::fs::remove_dir_all(root).unwrap();
        assert_eq!(revision, horizon_protocol::menu_extra::REVISION);
    }

    #[test]
    fn configuration_answers_the_content_identity_challenge() {
        let root =
            std::env::temp_dir().join(format!("horizon-content-identity-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("facts.json"),
            br#"{"states":[["minecraft:stone",false,false,"Block{minecraft:stone}",{"id":"minecraft:stone"}]]}"#,
        )
        .unwrap();
        std::fs::write(root.join("items.json"), "{}").unwrap();
        let data = crate::test_resources::item_data(&root, DataLoadOrigin::ClientPreload).unwrap();
        let expected = crate::content_identity::identity(&root, data.defaults()).unwrap();
        let (client, server) = horizon_protocol::connection::Pipe::pair();
        let (mut reader, writer) = Connection::local(client).split();
        let writer = Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap())));
        // Keep the connection owner alive until the peer receives the queued
        // finish ACK, as a real Session does when configure returns to play.
        let client_writer = writer.clone();
        let thread = std::thread::spawn(move || {
            let (send, _events) = event_channel();
            configure(
                &mut reader,
                &client_writer,
                &Arc::new(AtomicBool::new(true)),
                &Language::new(),
                &send,
                &Cookies::default(),
                &data,
                &crate::raw_payloads::Transport::empty(),
            )
        });
        let mut server = Connection::local(server);
        server
            .send(
                &cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                    channel: crate::content_identity::CHANNEL.into(),
                    data: vec![],
                })
                .encode(),
            )
            .unwrap();
        server
            .send(&cfg::Clientbound::Ping(cfg::Ping { id: 17 }).encode())
            .unwrap();
        let (id, body) = server.recv().unwrap();
        assert_eq!(
            cfg::Serverbound::decode(id, &body).unwrap(),
            cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                channel: crate::content_identity::CHANNEL.into(),
                data: expected.to_vec(),
            })
        );
        let (id, body) = server.recv().unwrap();
        assert_eq!(
            cfg::Serverbound::decode(id, &body).unwrap(),
            cfg::Serverbound::Pong(cfg::Ping { id: 17 })
        );
        server
            .send(&cfg::Clientbound::FinishConfiguration(Empty).encode())
            .unwrap();
        let (id, body) = server.recv().unwrap();
        assert_eq!(
            cfg::Serverbound::decode(id, &body).unwrap(),
            cfg::Serverbound::FinishConfiguration(Empty)
        );
        assert!(thread.join().unwrap().is_ok());
        drop(writer);
        std::fs::remove_dir_all(
            std::env::temp_dir().join(format!("horizon-content-identity-{}", std::process::id())),
        )
        .unwrap();
    }

    #[test]
    fn disconnect_and_drop_do_not_wait_for_a_stalled_reader() {
        // A reader can remain stalled after Windows socket shutdown. It must
        // not delay either an explicit leave or Session's Drop on the UI thread.
        for explicit_disconnect in [true, false] {
            let (client, _server) = horizon_protocol::connection::Pipe::pair();
            let (_reader, writer) = Connection::local(client).split();
            let writer = Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap())));
            let (_send, events) = event_channel();
            let (release, stalled) = channel();
            let (finished, completed) = channel();
            let thread = std::thread::spawn(move || {
                let _ = stalled.recv_timeout(std::time::Duration::from_secs(3));
                let _ = finished.send(());
            });
            let mut session = Session {
                events,
                payload_events: event_channel().0,
                payloads: crate::raw_payloads::Transport::empty(),
                writer: writer.clone(),
                configuring: Arc::new(AtomicBool::new(false)),
                thread: Some(thread),
                address: "local".into(),
                name: "TestPlayer".into(),
                counters: Arc::new(Mutex::new(None)),
                cookies: Cookies::default(),
                debug: false,
                far: Arc::default(),
                recorder: Arc::new(Mutex::new(None)),
                serial: next_serial(),
            };
            let started = std::time::Instant::now();
            if explicit_disconnect {
                session.disconnect();
                session.disconnect(); // Repeated cleanup is harmless.
            }
            drop(session);
            let elapsed = started.elapsed();
            let reader_still_pending = completed.try_recv().is_err();
            // Always release the synthetic reader before asserting, even if
            // a future regression restores the blocking join.
            let _ = release.send(());
            if reader_still_pending {
                completed
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .unwrap();
            }
            assert!(
                writer.lock().unwrap().is_none(),
                "connection was not closed"
            );
            assert!(
                reader_still_pending,
                "disconnect waited for the stalled reader"
            );
            assert!(
                elapsed < std::time::Duration::from_secs(1),
                "disconnect took {elapsed:?}"
            );
        }
    }

    #[test]
    fn pings_wait_behind_teleports_but_keepalives_do_not_wait_for_the_game() {
        let (root, data) = empty_item_data("pings");
        let (client, server) = horizon_protocol::connection::Pipe::pair();
        let (mut reader, writer) = Connection::local(client).split();
        let writer = Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap())));
        let (send, events) = event_channel();
        let thread_writer = writer.clone();
        let thread = std::thread::spawn(move || {
            receive_play(
                &mut reader,
                &thread_writer,
                &Arc::new(AtomicBool::new(false)),
                &Language::new(),
                &send,
                &Cookies::default(),
                &crate::lod_net::Inbox::default(),
                &data,
                &Arc::new(Mutex::new(None)),
                &crate::raw_payloads::Transport::empty(),
            )
        });
        let mut server = Connection::local(server);
        let teleport = play::Clientbound::PlayerPosition(play::PlayerPosition {
            teleport: 42,
            position: [10.0, 64.0, 10.0],
            delta: [0.0; 3],
            yaw: 90.0,
            pitch: 0.0,
            relatives: 0,
        });
        server.send(&teleport.encode()).unwrap();
        server
            .send(&play::Clientbound::Ping(cfg::Ping { id: 7 }).encode())
            .unwrap();
        // A reply to this proves the receive loop has passed the ping. The
        // old implementation instead replies with Pong before this keepalive.
        server
            .send(&play::Clientbound::KeepAlive(cfg::KeepAlive { id: 9 }).encode())
            .unwrap();
        let (id, body) = server.recv().unwrap();
        assert_eq!(
            play::Serverbound::decode(id, &body).unwrap(),
            play::Serverbound::KeepAlive(cfg::KeepAlive { id: 9 })
        );
        let timeout = Duration::from_secs(5);
        let (event, _teleport_credit) = events.recv_timeout_owned(timeout).unwrap().into_parts();
        assert!(matches!(event, Event::Packet(p) if *p == teleport));
        let (event, _ping_credit) = events.recv_timeout_owned(timeout).unwrap().into_parts();
        assert!(
            matches!(event, Event::Packet(p) if *p == play::Clientbound::Ping(cfg::Ping { id: 7 }))
        );
        writer.lock().unwrap().take().unwrap().shutdown();
        assert!(thread.join().unwrap().is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn local_send_rejects_unknown_oversize_payload_before_writing() {
        let (client, server) = horizon_protocol::connection::Pipe::pair();
        let (_, writer) = Connection::local(client).split();
        let (_, events) = event_channel();
        let session = Session {
            events,
            payload_events: event_channel().0,
            payloads: crate::raw_payloads::Transport::empty(),
            writer: Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap()))),
            configuring: Arc::new(AtomicBool::new(false)),
            thread: None,
            address: "local".into(),
            name: "Player".into(),
            counters: Arc::new(Mutex::new(None)),
            cookies: Cookies::default(),
            debug: false,
            far: Arc::default(),
            recorder: Arc::new(Mutex::new(None)),
            serial: next_serial(),
        };
        let mut server = Connection::local(server);
        let payload = |size| {
            play::Serverbound::CustomPayload(cfg::CustomPayload {
                channel: "test:opaque".into(),
                data: vec![0x83; size],
            })
        };
        session.send(payload(32_767));
        let (id, bytes) = server.recv().unwrap();
        assert_eq!(
            play::Serverbound::decode(id, &bytes).unwrap(),
            payload(32_767)
        );
        session.send(payload(32_768));
        session.send(play::Serverbound::ClientTickEnd(Empty));
        let (id, bytes) = server.recv().unwrap();
        assert_eq!(
            play::Serverbound::decode(id, &bytes).unwrap(),
            play::Serverbound::ClientTickEnd(Empty)
        );
    }

    #[test]
    fn play_packets_wait_out_a_reconfiguration() {
        let (root, data) = empty_item_data("reconfiguration");
        let (client, server) = horizon_protocol::connection::Pipe::pair();
        let (mut reader, writer) = Connection::local(client).split();
        let (send, events) = event_channel();
        let session = Session {
            events,
            payload_events: event_channel().0,
            payloads: crate::raw_payloads::Transport::empty(),
            writer: Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap()))),
            configuring: Arc::new(AtomicBool::new(false)),
            thread: None,
            address: "local".into(),
            name: "Player".into(),
            counters: Arc::new(Mutex::new(None)),
            cookies: Cookies::default(),
            debug: false,
            far: Arc::default(),
            recorder: Arc::new(Mutex::new(None)),
            serial: next_serial(),
        };
        let (thread_writer, thread_configuring, thread_recorder) = (
            session.writer.clone(),
            session.configuring.clone(),
            session.recorder.clone(),
        );
        let thread = std::thread::spawn(move || {
            receive_play(
                &mut reader,
                &thread_writer,
                &thread_configuring,
                &Language::new(),
                &send,
                &Cookies::default(),
                &crate::lod_net::Inbox::default(),
                &data,
                &thread_recorder,
                &crate::raw_payloads::Transport::empty(),
            )
        });
        let mut server = Connection::local(server);
        let tick_end = play::Serverbound::ClientTickEnd(Empty);
        server
            .send(&play::Clientbound::StartConfiguration(Empty).encode())
            .unwrap();
        let (id, body) = server.recv().unwrap();
        assert_eq!(
            play::Serverbound::decode(id, &body).unwrap(),
            play::Serverbound::ConfigurationAcknowledged(Empty)
        );
        // The game thread still ticks the old level: nothing of it goes out.
        session.send(tick_end.clone());
        server
            .send(&cfg::Clientbound::FinishConfiguration(Empty).encode())
            .unwrap();
        let (id, body) = server.recv().unwrap();
        assert_eq!(
            cfg::Serverbound::decode(id, &body).unwrap(),
            cfg::Serverbound::FinishConfiguration(Empty)
        );
        // Back in play, the game's packets flow again.
        session.send(tick_end.clone());
        let (id, body) = server.recv().unwrap();
        assert_eq!(play::Serverbound::decode(id, &body).unwrap(), tick_end);
        drop(session);
        assert!(thread.join().unwrap().is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn play_answers_cookie_requests_from_stored_cookies_and_follows_transfers() {
        let (root, data) = empty_item_data("cookies");
        let (client, server) = horizon_protocol::connection::Pipe::pair();
        let (mut reader, writer) = Connection::local(client).split();
        let writer = Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap())));
        let (send, events) = event_channel();
        let cookies = Cookies::default();
        let (thread_writer, thread_cookies) = (writer.clone(), cookies.clone());
        let thread = std::thread::spawn(move || {
            receive_play(
                &mut reader,
                &thread_writer,
                &Arc::new(AtomicBool::new(false)),
                &Language::new(),
                &send,
                &thread_cookies,
                &crate::lod_net::Inbox::default(),
                &data,
                &Arc::new(Mutex::new(None)),
                &crate::raw_payloads::Transport::empty(),
            )
        });
        let mut server = Connection::local(server);
        let ask = |server: &mut Connection, key: &str| {
            let request = play::Clientbound::CookieRequest(cfg::CookieRequest { key: key.into() });
            server.send(&request.encode()).unwrap();
            let (id, body) = server.recv().unwrap();
            play::Serverbound::decode(id, &body).unwrap()
        };
        // handleRequestCookie answers an unknown key with no payload.
        assert_eq!(
            ask(&mut server, "proxy:session"),
            play::Serverbound::CookieResponse(cfg::CookieResponse {
                key: "proxy:session".into(),
                payload: None,
            })
        );
        let store = play::Clientbound::StoreCookie(cfg::StoreCookie {
            key: "proxy:session".into(),
            payload: vec![1, 2, 3],
        });
        server.send(&store.encode()).unwrap();
        assert_eq!(
            ask(&mut server, "proxy:session"),
            play::Serverbound::CookieResponse(cfg::CookieResponse {
                key: "proxy:session".into(),
                payload: Some(vec![1, 2, 3]),
            })
        );
        let transfer = play::Clientbound::Transfer(cfg::Transfer {
            host: "example.net".into(),
            port: 25566,
        });
        server.send(&transfer.encode()).unwrap();
        assert!(thread.join().unwrap().is_ok());
        let turn: Vec<_> = events
            .take_turn()
            .into_iter()
            .map(ingress::OwnedReceipt::into_parts)
            .collect();
        assert!(matches!(
            turn.last(),
            Some((Event::Transfer { host, port: 25566 }, _credit)) if host == "example.net"
        ));
        // The stored cookie stays for the next connection (TransferState).
        assert_eq!(
            cookies.lock().unwrap().get("proxy:session"),
            Some(&vec![1, 2, 3])
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn server_brand_payload_is_decoded_and_invalid_or_unrelated_payloads_are_ignored() {
        let (send, events) = event_channel();
        let mut writer = horizon_protocol::buf::Writer::new();
        writer.string("vanilla");
        let mut payload = cfg::CustomPayload {
            channel: "minecraft:brand".into(),
            data: writer.data,
        };
        receive_brand(&payload, &send);
        let (event, _credit) = events.try_recv_owned().unwrap().into_parts();
        assert!(matches!(event, Event::Brand(brand) if brand == "vanilla"));
        payload.channel = "example:other".into();
        receive_brand(&payload, &send);
        assert!(events.try_recv_owned().is_err());
        payload.channel = "minecraft:brand".into();
        payload.data = vec![0xff];
        receive_brand(&payload, &send);
        assert!(events.try_recv_owned().is_err());
    }
    /// No account never joins; the signed-in rule is compared with the real
    /// client login listener by test_lan_auth_parity.py.
    #[test]
    #[ignore = "daily"]
    fn lan_login_continues_after_authentication_failure() {
        let (root, data) = empty_item_data("lan-auth");
        for lan in [true, false] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let worker = std::thread::spawn(move || {
                let (stream, _) = listener.accept().unwrap();
                // Generous: a debug build's key generation on a busy CI runner takes seconds. A pass
                // never waits for these.
                stream
                    .set_read_timeout(Some(Duration::from_secs(60)))
                    .unwrap();
                let mut server = Connection::from_stream(stream).unwrap();
                server.recv().unwrap(); // intention
                server.recv().unwrap(); // profile
                let key = horizon_protocol::rsa::RsaPrivateKey::generate(1024).unwrap();
                let challenge = vec![1, 2, 3, 4];
                server
                    .send(
                        &login::Clientbound::Hello(login::Hello {
                            server_id: String::new(),
                            public_key: key.public_der(),
                            challenge: challenge.clone(),
                            should_authenticate: true,
                        })
                        .encode(),
                    )
                    .unwrap();
                let Ok((id, body)) = server.recv() else {
                    return false;
                };
                let login::Serverbound::Key(reply) = login::Serverbound::decode(id, &body).unwrap()
                else {
                    panic!("client did not send encryption reply")
                };
                assert_eq!(key.decrypt(&reply.challenge).unwrap(), challenge);
                let secret: [u8; 16] = key.decrypt(&reply.key).unwrap().try_into().unwrap();
                server.enable_encryption(&secret);
                server
                    .send(
                        &login::Clientbound::Disconnect(login::Disconnect {
                            reason_json: r#"{"text":"LAN fixture accepted"}"#.into(),
                        })
                        .encode(),
                    )
                    .unwrap();
                true
            });
            let address = format!("127.0.0.1:{port}");
            let connect = if lan {
                Session::connect_lan
            } else {
                Session::connect
            };
            // The session's language carries Horizon's bundled strings
            // (`load_language`), so the keyed refusal resolves.
            let lang: Language = [(
                "horizon.multiplayer.not_signed_in".to_owned(),
                NOT_SIGNED_IN.to_owned(),
            )]
            .into_iter()
            .collect();
            let session = connect(
                &address,
                "Tester",
                None,
                cfg::ClientInformation::new(8),
                Arc::new(lang),
                Arc::clone(&data),
            );
            let (reason, _credit) = loop {
                let (event, credit) = session
                    .events
                    .recv_timeout_owned(Duration::from_secs(60))
                    .unwrap()
                    .into_parts();
                if let Event::Disconnected { reason, .. } = event {
                    break (reason, credit);
                }
            };
            // Without an account nothing joins, LAN or not.
            let sent_key = worker.join().unwrap();
            assert!(!sent_key, "lan {lan}: joined without an account");
            assert_eq!(reason, NOT_SIGNED_IN);
            // A signed-in account whose join the session service rejects:
            // the case vanilla's listener is run with.
            let goes_on = continues_after_failed_join(lan, true);
            assert_eq!(goes_on, lan);
            println!(
                "lan_auth={}:{}:{}",
                if lan { "LAN" } else { "OTHER" },
                goes_on,
                !goes_on
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A recorder that keeps what reaches it, as (id, payload).
    #[derive(Clone, Default)]
    struct TestRecorder(Arc<Mutex<Vec<(i32, Vec<u8>)>>>);
    impl PacketRecorder for TestRecorder {
        fn is_active(&self) -> bool {
            true
        }
        fn received(&self, _packet: &play::Clientbound, id: i32, payload: &[u8]) {
            self.0.lock().unwrap().push((id, payload.to_vec()));
        }
        fn record_packet(&self, id: i32, payload: &[u8]) {
            self.0.lock().unwrap().push((id, payload.to_vec()));
        }
        fn status(&self) -> RecorderStatus {
            RecorderStatus {
                active: true,
                path: PathBuf::from("test.mcpr"),
                elapsed: Duration::ZERO,
                packet_count: self.0.lock().unwrap().len() as u64,
                bytes_written: 0,
                error: None,
            }
        }
        fn stop(self: Box<Self>) -> std::io::Result<PathBuf> {
            Ok(PathBuf::from("test.mcpr"))
        }
    }
    impl TestRecorder {
        fn decoded(&self) -> Vec<play::Clientbound> {
            let packets = self.0.lock().unwrap();
            packets
                .iter()
                .map(|(id, payload)| play::Clientbound::decode(*id, payload).unwrap())
                .collect()
        }
    }

    #[test]
    fn session_recorder_gets_the_play_packets_the_network_thread_receives() {
        let (root, data) = empty_item_data("studio_recording");
        let (client, server) = horizon_protocol::connection::Pipe::pair();
        let (mut reader, writer) = Connection::local(client).split();
        let (send, events) = event_channel();
        let session = Session {
            events,
            payload_events: event_channel().0,
            payloads: crate::raw_payloads::Transport::empty(),
            writer: Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap()))),
            configuring: Arc::new(AtomicBool::new(false)),
            thread: None,
            address: "local".into(),
            name: "StudioTester".into(),
            counters: Arc::new(Mutex::new(None)),
            cookies: Cookies::default(),
            debug: false,
            far: Arc::default(),
            recorder: Arc::new(Mutex::new(None)),
            serial: next_serial(),
        };
        let recorder = TestRecorder::default();
        session.start_recording(recorder.clone(), |_| {});
        assert!(session.is_recording());

        let (thread_writer, thread_configuring, thread_recorder) = (
            session.writer.clone(),
            session.configuring.clone(),
            session.recorder.clone(),
        );
        let thread = std::thread::spawn(move || {
            receive_play(
                &mut reader,
                &thread_writer,
                &thread_configuring,
                &Language::new(),
                &send,
                &Cookies::default(),
                &crate::lod_net::Inbox::default(),
                &data,
                &thread_recorder,
                &crate::raw_payloads::Transport::empty(),
            )
        });

        let mut server = Connection::local(server);
        let pos_pkt = play::Clientbound::PlayerPosition(play::PlayerPosition {
            teleport: 1,
            position: [100.0, 64.0, -200.0],
            delta: [0.0; 3],
            yaw: 45.0,
            pitch: 10.0,
            relatives: 0,
        });
        server.send(&pos_pkt.encode()).unwrap();
        // Disconnect ends receive_play's loop.
        let disc_pkt = play::Clientbound::Disconnect(cfg::Disconnect {
            reason: Tag::String("Replay tap test completed".into()),
        });
        server.send(&disc_pkt.encode()).unwrap();
        let _ = thread.join().unwrap();

        assert_eq!(
            session.stop_recording().unwrap().unwrap(),
            PathBuf::from("test.mcpr")
        );
        assert!(!session.is_recording());
        // The recorder is handed both; what a recording keeps is the recorder's choice.
        assert_eq!(recorder.decoded(), [pos_pkt, disc_pkt]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn session_recorder_gets_the_packets_the_client_makes() {
        use horizon_protocol::entity_data::{Value, serializer};

        let session = Session::replay("StudioTester");
        session.record_clientbound(&play::Clientbound::SwingAnimation(play::SwingAnimation {
            id: 1,
            hand: 0,
            kind: 0,
            duration: 6,
        }));
        let recorder = TestRecorder::default();
        // What `initialize` writes comes first.
        let snapshot = play::Clientbound::PlayerPosition(play::PlayerPosition {
            teleport: 0,
            position: [12.5, 64.0, -88.5],
            delta: [0.0; 3],
            yaw: 90.0,
            pitch: 15.0,
            relatives: 0,
        });
        session.start_recording(recorder.clone(), |rec| {
            let encoded = snapshot.encode();
            let mut reader = horizon_protocol::buf::Reader::new(&encoded);
            let id = reader.varint().unwrap();
            rec.record_packet(id, reader.rest());
        });
        let swing_pkt = play::Clientbound::SwingAnimation(play::SwingAnimation {
            id: 42,
            hand: 0,
            kind: 1,
            duration: 6,
        });
        session.record_clientbound(&swing_pkt);
        let data_pkt = play::Clientbound::SetEntityData(play::SetEntityData {
            id: 42,
            values: vec![
                (0, serializer::BYTE, Value::Byte(0x02)),
                (6, serializer::POSE, Value::Id(5)),
            ],
        });
        session.record_clientbound(&data_pkt);
        assert_eq!(session.recorder_status().unwrap().packet_count, 3);
        session.stop_recording().unwrap().unwrap();
        // Nothing reaches a stopped recording.
        session.record_clientbound(&swing_pkt);

        assert_eq!(recorder.decoded(), [snapshot, swing_pkt, data_pkt]);
    }

    #[test]
    fn fill_known_entries_rejects_path_traversal_and_reads_valid_entries() {
        let temp_dir =
            std::env::temp_dir().join(format!("horizon-net-reg-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        let resources = temp_dir.join("res");
        let valid_dir = resources.join("data/minecraft/worldgen/biome");
        std::fs::create_dir_all(&valid_dir).unwrap();
        std::fs::write(
            valid_dir.join("plains.json"),
            br#"{"has_precipitation":true}"#,
        )
        .unwrap();
        std::fs::write(temp_dir.join("secret.json"), br#"{"leaked":true}"#).unwrap();

        let mut reg = cfg::RegistryData {
            registry: "minecraft:worldgen/biome".into(),
            entries: vec![
                cfg::RegistryEntry {
                    id: "minecraft:plains".into(),
                    data: None,
                },
                cfg::RegistryEntry {
                    id: "minecraft:../../../../secret".into(),
                    data: None,
                },
                cfg::RegistryEntry {
                    id: "..:plains".into(),
                    data: None,
                },
            ],
        };
        fill_known_entries(&mut reg, &resources);
        assert!(reg.entries[0].data.is_some());
        assert!(reg.entries[1].data.is_none());
        assert!(reg.entries[2].data.is_none());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
