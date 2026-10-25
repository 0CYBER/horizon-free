//! Loading and calling mods with wasmtime's component model.
use crate::manifest::{Manifest, Package, Side};
use server_bindings::{
    HorizonServerMod, exports::horizon::mods::server_events::Registration as ServerRegistration,
    horizon::mods::players,
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};
use wasmtime::{
    Config, Engine, Store, StoreLimits, StoreLimitsBuilder, Trap,
    component::{Component, HasSelf, Linker},
};

mod bindings {
    wasmtime::component::bindgen!({
        path: "../horizon-mod-sdk/wit",
        world: "horizon-mod",
    });
}
/// The server mods' world: `log` and `storage` are the client world's.
mod server_bindings {
    wasmtime::component::bindgen!({
        path: "../horizon-mod-sdk/wit",
        world: "horizon-server-mod",
        with: {
            "horizon:mods/log": super::bindings::horizon::mods::log,
            "horizon:mods/storage": super::bindings::horizon::mods::storage,
        },
    });
}
pub use bindings::horizon::mods::{
    game::{Clock, Player},
    hud::{Corner, Line},
};
use bindings::{
    HorizonMod,
    exports::horizon::mods::events::Registration,
    horizon::mods::{chat, game, hud, log as mod_log, storage},
};
pub use server_bindings::{
    exports::horizon::mods::server_events::BlockPos,
    horizon::mods::players::{GameMode, Player as ServerPlayer},
};
type ClientEvents = bindings::exports::horizon::mods::events::Guest;
type ServerEvents = server_bindings::exports::horizon::mods::server_events::Guest;

/// A mod's linear memory cap.
const MEMORY: usize = 64 << 20;
/// Fuel (about one unit per WebAssembly instruction) per call: a bound on
/// runaway loops, far above the hooks' time budgets.
const FUEL_INIT: u64 = 2_000_000_000;
const FUEL_TICK: u64 = 50_000_000;
const FUEL_FRAME: u64 = 10_000_000;
const FUEL_EVENT: u64 = 100_000_000;
/// Chat lines a mod may show per call, HUD lines and their length.
const CHAT_PER_CALL: usize = 8;
const HUD_LINES: usize = 16;
const LINE_CHARS: usize = 256;
/// Total bytes a mod's `storage` may hold on disk, across all its keys.
const STORAGE_QUOTA: u64 = 4 << 20;
/// Distinct keys a mod's `storage` may hold: `STORAGE_QUOTA` alone doesn't
/// bound a flood of zero- or near-zero-byte files (each still a file name,
/// a directory entry and an inode).
const STORAGE_MAX_KEYS: usize = 64;
/// Bytes `storage::set` may write across one hook call: fuel bounds a
/// mod's CPU time per call, not the time a synchronous disk write takes, so
/// a cheap-in-fuel loop replacing one key with large payloads needs its own
/// cap, separate from the on-disk quota it would otherwise stay under.
const STORAGE_WRITE_BUDGET: u64 = 1 << 20;

/// What mods may read of the game, copied once per tick by the game.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub player: Option<Player>,
    pub clock: Option<Clock>,
    /// The players on the server, for server mods.
    pub players: Vec<ServerPlayer>,
}

/// Which mods a game starts: the window's game the client mods, the game
/// simulating the world the server mods, single player in the frame both.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Runs {
    Client,
    Server,
    Both,
}
impl Runs {
    fn starts(self, side: Side) -> bool {
        match self {
            Runs::Both => true,
            Runs::Client => side != Side::Server,
            Runs::Server => side == Side::Server,
        }
    }
}

/// What mods asked the game to show since the last [`Mods::drain`].
#[derive(Clone, Debug, PartialEq)]
pub enum Output {
    /// `chat.show`: a line after the mod's name.
    Chat { name: String, text: String },
    /// A mod command's feedback, red when `error`.
    Feedback { text: String, error: bool },
    /// The mod was turned off: it trapped, overran or failed to start.
    Stopped { name: String, reason: String },
    /// `players.tell`: a line after the mod's name, to the player with this id.
    Tell {
        name: String,
        player: String,
        text: String,
    },
    /// `players.broadcast`: a line after the mod's name, to every player.
    Broadcast { name: String, text: String },
    /// A server mod command's feedback, to the player who typed it.
    Reply {
        player: String,
        text: String,
        error: bool,
    },
}

/// A mod's text panel, drawn every frame until it changes.
#[derive(Clone, Debug)]
pub struct Panel {
    pub corner: Corner,
    pub lines: Vec<Line>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Installed and turned off by the player.
    Off,
    Running,
    /// Turned on, and run by another game: a server mod in the window while
    /// the world runs on its server thread.
    Elsewhere,
    /// Could not load or start, or stopped while running.
    Failed(String),
}

/// One installed mod, for the Mods page.
pub struct Entry {
    pub path: PathBuf,
    /// None when the manifest could not be read (`status` says why).
    pub manifest: Option<Manifest>,
    pub status: Status,
    /// The mod's average time per call of its busiest hook.
    pub cost: Duration,
    run: Option<Running>,
    modified: Option<SystemTime>,
}

struct Running {
    store: Store<State>,
    hooks: Hooks,
}

/// A running mod's exports and the hooks its `init` asked for.
enum Hooks {
    Client(HorizonMod, Registration),
    Server(HorizonServerMod, ServerRegistration),
}
impl Hooks {
    fn client(&self) -> Option<&Registration> {
        match self {
            Hooks::Client(_, r) => Some(r),
            Hooks::Server(..) => None,
        }
    }
    fn server(&self) -> Option<&ServerRegistration> {
        match self {
            Hooks::Server(_, r) => Some(r),
            Hooks::Client(..) => None,
        }
    }
    fn commands(&self) -> &[String] {
        match self {
            Hooks::Client(_, r) => &r.commands,
            Hooks::Server(_, r) => &r.commands,
        }
    }
}

struct State {
    name: String,
    id: String,
    snapshot: Arc<Snapshot>,
    out: Vec<Output>,
    panel: Option<Panel>,
    /// Capability `storage`'s folder; made on first `set`.
    data_dir: PathBuf,
    /// Bytes `storage::set` may still write before this hook call ends
    /// (reset per call in [`Mods::call`]): fuel bounds a mod's CPU time but
    /// not disk I/O time, so a loop of small-fuel, large-payload `set`s
    /// needs its own budget.
    storage_budget: u64,
    limits: StoreLimits,
}

/// The installed mods and the engine running them. `Mods::default()` is
/// the state with none: no engine, and every hook a no-op.
#[derive(Default)]
pub struct Mods {
    engine: Option<(Engine, Linker<State>)>,
    entries: Vec<Entry>,
    snapshot: Arc<Snapshot>,
    out: Vec<Output>,
}

impl Mods {
    /// Reads every mod at `paths` and starts those `enabled` says are on,
    /// client and server mods alike. A command that `reserved` (vanilla's
    /// commands) or an earlier mod already uses is refused.
    pub fn load(
        paths: &[PathBuf],
        enabled: impl Fn(&str) -> bool,
        reserved: impl Fn(&str) -> bool,
    ) -> Mods {
        Mods::load_for(Runs::Both, paths, enabled, reserved)
    }
    /// [`Mods::load`], starting only the mods `runs` says this game runs; the
    /// others that are on are [`Status::Elsewhere`].
    pub fn load_for(
        runs: Runs,
        paths: &[PathBuf],
        enabled: impl Fn(&str) -> bool,
        reserved: impl Fn(&str) -> bool,
    ) -> Mods {
        let mut mods = Mods::default();
        for path in paths {
            let modified = modified(path);
            let mut entry = Entry {
                path: path.clone(),
                manifest: None,
                status: Status::Off,
                cost: Duration::ZERO,
                run: None,
                modified,
            };
            match Package::read(path) {
                Err(e) => entry.status = Status::Failed(e),
                Ok(package) => {
                    let on = enabled(&package.manifest.id);
                    entry.manifest = Some(package.manifest.clone());
                    if on && !runs.starts(package.manifest.side) {
                        entry.status = Status::Elsewhere;
                    } else if on {
                        // Content is read before the world loads; a mod whose
                        // content is refused stays off, with the reason.
                        let content = crate::Content::read(path, &package.manifest);
                        entry.status =
                            match content.and_then(|_| mods.start(path, &package, &reserved)) {
                                Ok(run) => {
                                    entry.run = run;
                                    Status::Running
                                }
                                Err(e) => Status::Failed(e),
                            };
                    }
                }
            }
            if let (Status::Failed(reason), Some(m)) = (&entry.status, &entry.manifest) {
                log::warn!("Horizon: mod {} did not start: {reason}", m.id);
            }
            mods.entries.push(entry);
        }
        mods
    }

    fn engine(&mut self) -> Result<&(Engine, Linker<State>), String> {
        if self.engine.is_none() {
            let mut config = Config::new();
            config.wasm_component_model(true).consume_fuel(true);
            let engine = Engine::new(&config).map_err(|e| e.to_string())?;
            let mut linker = Linker::new(&engine);
            HorizonMod::add_to_linker::<State, HasSelf<State>>(&mut linker, |s| s)
                .map_err(|e| e.to_string())?;
            players::add_to_linker::<State, HasSelf<State>>(&mut linker, |s| s)
                .map_err(|e| e.to_string())?;
            self.engine = Some((engine, linker));
        }
        Ok(self.engine.as_ref().expect("made above"))
    }

    /// Starts a mod's code; `Ok(None)` for a content-only mod, which has
    /// nothing to run (the game reads its content before the world loads).
    /// `path` is the mod's own `.hmod` or folder, for capability `storage`'s
    /// per-mod directory.
    fn start(
        &mut self,
        path: &Path,
        package: &Package,
        reserved: &dyn Fn(&str) -> bool,
    ) -> Result<Option<Running>, String> {
        let m = &package.manifest;
        let Some(wasm) = &package.wasm else {
            return Ok(None);
        };
        // Two running mods with the same id would share one `data_dir`
        // (capability `storage`): the first loaded keeps the id.
        if self
            .entries
            .iter()
            .any(|e| e.run.is_some() && e.manifest.as_ref().is_some_and(|other| other.id == m.id))
        {
            return Err(format!("another running mod already uses id {}", m.id));
        }
        let server = m.side == Side::Server;
        let taken: Vec<String> = self
            .entries
            .iter()
            .filter_map(|e| e.run.as_ref())
            .filter(|r| r.hooks.server().is_some() == server)
            .flat_map(|r| r.hooks.commands().iter().cloned())
            .collect();
        let snapshot = self.snapshot.clone();
        let (engine, linker) = self.engine()?;
        let component =
            Component::new(engine, wasm).map_err(|e| format!("not a component: {e}"))?;
        let ty = component.component_type();
        let imports: Vec<String> = ty
            .imports(engine)
            .map(|(name, _)| name.to_string())
            .collect();
        m.check_imports(imports.iter().map(String::as_str))?;
        let data_dir = path
            .parent()
            .unwrap_or(Path::new("."))
            .join(crate::manifest::STORAGE_DIR)
            .join(&m.id);
        let state = State {
            name: m.name.clone(),
            id: m.id.clone(),
            snapshot,
            out: Vec::new(),
            panel: None,
            data_dir,
            storage_budget: STORAGE_WRITE_BUDGET,
            limits: StoreLimitsBuilder::new()
                .memory_size(MEMORY)
                .instances(16)
                .build(),
        };
        let mut store = Store::new(engine, state);
        store.limiter(|s| &mut s.limits);
        store.set_fuel(FUEL_INIT).map_err(|e| e.to_string())?;
        let mut hooks = if server {
            let exports = HorizonServerMod::instantiate(&mut store, &component, linker)
                .map_err(|e| reason(&e))?;
            let r = exports
                .horizon_mods_server_events()
                .call_init(&mut store)
                .map_err(|e| reason(&e))??;
            Hooks::Server(exports, r)
        } else {
            let exports =
                HorizonMod::instantiate(&mut store, &component, linker).map_err(|e| reason(&e))?;
            let r = exports
                .horizon_mods_events()
                .call_init(&mut store)
                .map_err(|e| reason(&e))??;
            Hooks::Client(exports, r)
        };
        let commands = match &mut hooks {
            Hooks::Client(_, r) => &mut r.commands,
            Hooks::Server(_, r) => &mut r.commands,
        };
        commands.retain(|c| {
            let free = !reserved(c) && !taken.contains(c);
            if !free {
                log::warn!("Horizon: mod {} cannot take /{c}: already a command", m.id);
            }
            free
        });
        self.out.append(&mut store.data_mut().out);
        Ok(Some(Running { store, hooks }))
    }

    /// True when at least one mod runs: the game skips its mod work otherwise.
    pub fn is_running(&self) -> bool {
        self.entries.iter().any(|e| e.run.is_some())
    }
    /// True when a server mod runs: the server skips its mod work otherwise.
    pub fn runs_server(&self) -> bool {
        self.entries
            .iter()
            .any(|e| e.run.as_ref().is_some_and(|r| r.hooks.server().is_some()))
    }
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
    /// What mods read until the next snapshot.
    pub fn set_snapshot(&mut self, snapshot: Snapshot) {
        self.snapshot = Arc::new(snapshot);
    }

    /// Calls `f` on every running mod whose hooks `wants`, with `fuel`.
    fn each(
        &mut self,
        fuel: u64,
        wants: impl Fn(&Hooks) -> bool,
        mut f: impl FnMut(&Hooks, &mut Store<State>) -> wasmtime::Result<()>,
    ) {
        for i in 0..self.entries.len() {
            if self.entries[i]
                .run
                .as_ref()
                .is_some_and(|r| wants(&r.hooks))
            {
                self.call(i, fuel, &mut f);
            }
        }
    }
    /// [`Mods::each`] over the client mods.
    fn each_client(
        &mut self,
        fuel: u64,
        wants: impl Fn(&Registration) -> bool,
        mut f: impl FnMut(&ClientEvents, &mut Store<State>) -> wasmtime::Result<()>,
    ) {
        self.each(
            fuel,
            |h| h.client().is_some_and(&wants),
            |h, s| match h {
                Hooks::Client(m, _) => f(m.horizon_mods_events(), s),
                Hooks::Server(..) => Ok(()),
            },
        );
    }
    /// [`Mods::each`] over the server mods.
    fn each_server(
        &mut self,
        fuel: u64,
        wants: impl Fn(&ServerRegistration) -> bool,
        mut f: impl FnMut(&ServerEvents, &mut Store<State>) -> wasmtime::Result<()>,
    ) {
        self.each(
            fuel,
            |h| h.server().is_some_and(&wants),
            |h, s| match h {
                Hooks::Server(m, _) => f(m.horizon_mods_server_events(), s),
                Hooks::Client(..) => Ok(()),
            },
        );
    }

    /// One call into mod `i`; a trap or an overrun turns the mod off.
    fn call<R>(
        &mut self,
        i: usize,
        fuel: u64,
        f: impl FnOnce(&Hooks, &mut Store<State>) -> wasmtime::Result<R>,
    ) -> Option<R> {
        let entry = &mut self.entries[i];
        let run = entry.run.as_mut()?;
        run.store.data_mut().snapshot = self.snapshot.clone();
        run.store.data_mut().storage_budget = STORAGE_WRITE_BUDGET;
        let started = Instant::now();
        let result = run
            .store
            .set_fuel(fuel)
            .and_then(|()| f(&run.hooks, &mut run.store));
        // An average over about the last 64 calls.
        entry.cost = (entry.cost * 63 + started.elapsed()) / 64;
        self.out.append(&mut run.store.data_mut().out);
        match result {
            Ok(r) => Some(r),
            Err(e) => {
                let reason = reason(&e);
                log::warn!("Horizon: mod {} stopped: {reason}", run.store.data().id);
                let name = run.store.data().name.clone();
                self.out.push(Output::Stopped {
                    name,
                    reason: reason.clone(),
                });
                entry.status = Status::Failed(reason);
                entry.run = None;
                None
            }
        }
    }

    /// A game tick.
    pub fn tick(&mut self, tick: u64) {
        self.each_client(FUEL_TICK, |h| h.tick, |m, s| m.call_tick(s, tick));
    }
    /// A rendered frame, `seconds` after the last.
    pub fn frame(&mut self, seconds: f32) {
        self.each_client(FUEL_FRAME, |h| h.frame, |m, s| m.call_frame(s, seconds));
    }
    /// A chat line the player received, as plain text.
    pub fn chat(&mut self, text: &str) {
        self.each_client(FUEL_EVENT, |h| h.chat, |m, s| m.call_chat(s, text));
    }
    /// Whether a running client mod owns `/name`.
    pub fn has_command(&self, name: &str) -> bool {
        self.command_owner(false, name).is_some()
    }
    fn command_owner(&self, server: bool, name: &str) -> Option<usize> {
        self.entries.iter().position(|e| {
            e.run.as_ref().is_some_and(|r| {
                r.hooks.server().is_some() == server && r.hooks.commands().iter().any(|c| c == name)
            })
        })
    }
    /// Runs `line` (without its slash) if a client mod owns its command;
    /// false when none does.
    pub fn command(&mut self, line: &str) -> bool {
        let (name, args) = line.split_once(' ').unwrap_or((line, ""));
        let Some(i) = self.command_owner(false, name) else {
            return false;
        };
        let result = self.call(i, FUEL_EVENT, |h, s| match h {
            Hooks::Client(m, _) => m.horizon_mods_events().call_command(s, name, args),
            Hooks::Server(..) => Ok(Ok(None)),
        });
        match result {
            Some(Ok(Some(text))) => self.out.push(Output::Feedback { text, error: false }),
            Some(Err(text)) => self.out.push(Output::Feedback { text, error: true }),
            Some(Ok(None)) | None => {}
        }
        true
    }
    /// The commands running client mods own, for suggestions.
    pub fn commands(&self) -> impl Iterator<Item = &str> {
        self.owned_commands(false)
    }
    fn owned_commands(&self, server: bool) -> impl Iterator<Item = &str> {
        self.entries
            .iter()
            .filter_map(|e| e.run.as_ref())
            .filter(move |r| r.hooks.server().is_some() == server)
            .flat_map(|r| r.hooks.commands().iter().map(String::as_str))
    }

    /// A server tick.
    pub fn server_tick(&mut self, tick: u64) {
        self.each_server(FUEL_TICK, |h| h.tick, |m, s| m.call_tick(s, tick));
    }
    /// `player` joined the server (they are in the snapshot's players).
    pub fn player_joined(&mut self, player: &ServerPlayer) {
        self.each_server(
            FUEL_EVENT,
            |h| h.joins,
            |m, s| m.call_player_joined(s, player),
        );
    }
    /// `player` left the server (they are no longer in the snapshot's players).
    pub fn player_left(&mut self, player: &ServerPlayer) {
        self.each_server(
            FUEL_EVENT,
            |h| h.joins,
            |m, s| m.call_player_left(s, player),
        );
    }
    /// A chat message `player` sent, as plain text.
    pub fn player_chat(&mut self, player: &ServerPlayer, text: &str) {
        self.each_server(FUEL_EVENT, |h| h.chat, |m, s| m.call_chat(s, player, text));
    }
    /// `player` broke `state` at `pos`.
    pub fn block_broken(&mut self, player: &ServerPlayer, pos: BlockPos, state: &str) {
        self.each_server(
            FUEL_EVENT,
            |h| h.blocks,
            |m, s| m.call_block_broken(s, player, pos, state),
        );
    }
    /// `player` placed `state` at `pos`.
    pub fn block_placed(&mut self, player: &ServerPlayer, pos: BlockPos, state: &str) {
        self.each_server(
            FUEL_EVENT,
            |h| h.blocks,
            |m, s| m.call_block_placed(s, player, pos, state),
        );
    }
    /// The commands running server mods own, for the server's command tree.
    pub fn server_commands(&self) -> impl Iterator<Item = &str> {
        self.owned_commands(true)
    }
    /// Runs `line` (without its slash), typed by `player`, if a server mod
    /// owns its command; its feedback is an [`Output::Reply`]. False when
    /// none does.
    pub fn server_command(&mut self, player: &ServerPlayer, line: &str) -> bool {
        let (name, args) = line.split_once(' ').unwrap_or((line, ""));
        let Some(i) = self.command_owner(true, name) else {
            return false;
        };
        let result = self.call(i, FUEL_EVENT, |h, s| match h {
            Hooks::Server(m, _) => m
                .horizon_mods_server_events()
                .call_command(s, player, name, args),
            Hooks::Client(..) => Ok(Ok(None)),
        });
        let reply = |text, error| Output::Reply {
            player: player.id.clone(),
            text,
            error,
        };
        match result {
            Some(Ok(Some(text))) => self.out.push(reply(text, false)),
            Some(Err(text)) => self.out.push(reply(text, true)),
            Some(Ok(None)) | None => {}
        }
        true
    }
    /// The running mods' HUD panels.
    pub fn panels(&self) -> impl Iterator<Item = &Panel> {
        self.entries
            .iter()
            .filter_map(|e| e.run.as_ref()?.store.data().panel.as_ref())
    }
    /// What mods asked to show since the last call.
    pub fn drain(&mut self) -> Vec<Output> {
        std::mem::take(&mut self.out)
    }

    /// Restarts the mods whose file changed since they loaded (mod
    /// development: rebuild, and the game picks it up). True when any did.
    pub fn reload_changed(&mut self, reserved: impl Fn(&str) -> bool) -> bool {
        let mut any = false;
        for i in 0..self.entries.len() {
            let entry = &self.entries[i];
            let now = modified(&entry.path);
            let was_on = !matches!(entry.status, Status::Off | Status::Elsewhere);
            if now == entry.modified || !was_on {
                continue;
            }
            any = true;
            self.entries[i].modified = now;
            self.entries[i].run = None;
            let path = self.entries[i].path.clone();
            let started = Package::read(&path).and_then(|p| {
                self.entries[i].manifest = Some(p.manifest.clone());
                self.start(&path, &p, &reserved)
            });
            let entry = &mut self.entries[i];
            let id = entry.manifest.as_ref().map_or("?", |m| m.id.as_str());
            match started {
                Ok(run) => {
                    log::info!("Horizon: mod {id} reloaded");
                    entry.run = run;
                    entry.status = Status::Running;
                }
                Err(e) => {
                    log::warn!("Horizon: mod {id} did not reload: {e}");
                    entry.status = Status::Failed(e);
                }
            }
        }
        any
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.is_dir() {
        // A folder mod changes when its component does.
        return std::fs::metadata(path.join(crate::manifest::WASM))
            .ok()?
            .modified()
            .ok();
    }
    meta.modified().ok()
}

/// A trap or error, as the player reads it.
fn reason(e: &wasmtime::Error) -> String {
    match e.downcast_ref::<Trap>() {
        Some(Trap::OutOfFuel) => "it took too long (an endless loop?)".into(),
        Some(Trap::UnreachableCodeReached) => "it crashed (a panic)".into(),
        Some(trap) => format!("it crashed: {trap}"),
        None => format!("{e:#}"),
    }
}

fn clip(text: String) -> String {
    match text.char_indices().nth(LINE_CHARS) {
        Some((at, _)) => text[..at].to_string(),
        None => text,
    }
}

impl mod_log::Host for State {
    fn write(&mut self, level: mod_log::Level, message: String) {
        let level = match level {
            mod_log::Level::Debug => log::Level::Debug,
            mod_log::Level::Info => log::Level::Info,
            mod_log::Level::Warn => log::Level::Warn,
            mod_log::Level::Error => log::Level::Error,
        };
        log::log!(level, "[mod {}] {}", self.id, clip(message));
    }
}
impl game::Host for State {
    fn get_player(&mut self) -> Option<Player> {
        self.snapshot.player.clone()
    }
    fn get_clock(&mut self) -> Option<Clock> {
        self.snapshot.clock
    }
}
impl State {
    /// Queues a chat line unless the call already sent [`CHAT_PER_CALL`].
    fn say(&mut self, line: impl FnOnce(String) -> Output) -> bool {
        let shown = self
            .out
            .iter()
            .filter(|o| {
                matches!(
                    o,
                    Output::Chat { .. } | Output::Tell { .. } | Output::Broadcast { .. }
                )
            })
            .count();
        if shown < CHAT_PER_CALL {
            let name = self.name.clone();
            self.out.push(line(name));
        }
        shown < CHAT_PER_CALL
    }
}
impl chat::Host for State {
    fn show(&mut self, text: String) {
        self.say(|name| Output::Chat {
            name,
            text: clip(text),
        });
    }
}
impl players::Host for State {
    fn online(&mut self) -> Vec<ServerPlayer> {
        self.snapshot.players.clone()
    }
    fn tell(&mut self, player: String, text: String) -> bool {
        self.snapshot.players.iter().any(|p| p.id == player)
            && self.say(|name| Output::Tell {
                name,
                player,
                text: clip(text),
            })
    }
    fn broadcast(&mut self, text: String) {
        self.say(|name| Output::Broadcast {
            name,
            text: clip(text),
        });
    }
}
impl hud::Host for State {
    fn set_panel(&mut self, corner: Corner, lines: Vec<Line>) {
        let lines = lines
            .into_iter()
            .take(HUD_LINES)
            .map(|l| Line {
                text: clip(l.text),
                color: l.color & 0xFF_FFFF,
            })
            .collect();
        self.panel = Some(Panel { corner, lines });
    }
    fn clear(&mut self) {
        self.panel = None;
    }
}
impl storage::Host for State {
    fn get(&mut self, key: String) -> Option<Vec<u8>> {
        storage_key_ok(&key)
            .then(|| std::fs::read(self.data_dir.join(&key)).ok())
            .flatten()
    }
    fn set(&mut self, key: String, value: Vec<u8>) -> Result<(), String> {
        if !storage_key_ok(&key) {
            return Err(format!("{key:?} is not [a-z0-9_.-]{{1,64}}"));
        }
        let len = value.len() as u64;
        if len > self.storage_budget {
            return Err("too much storage traffic in one call".into());
        }
        // Everything already on disk but the file this call replaces.
        let existing: Vec<std::fs::DirEntry> = std::fs::read_dir(&self.data_dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy() != key)
            .collect();
        if existing.len() >= STORAGE_MAX_KEYS {
            return Err(format!("would hold over {STORAGE_MAX_KEYS} keys"));
        }
        let used: u64 = existing
            .iter()
            .filter_map(|e| e.metadata().ok())
            .map(|m| m.len())
            .sum();
        if used + len > STORAGE_QUOTA {
            return Err(format!(
                "would use over {} MiB of storage",
                STORAGE_QUOTA >> 20
            ));
        }
        std::fs::create_dir_all(&self.data_dir).map_err(|e| e.to_string())?;
        std::fs::write(self.data_dir.join(&key), value).map_err(|e| e.to_string())?;
        self.storage_budget -= len;
        Ok(())
    }
    fn delete(&mut self, key: String) {
        if storage_key_ok(&key) {
            let _ = std::fs::remove_file(self.data_dir.join(&key));
        }
    }
}

/// A storage key: one path segment of `[a-z0-9_.-]{1,64}`, not `.` or `..`.
/// No `/`, so a key can never leave the mod's own directory.
fn storage_key_ok(key: &str) -> bool {
    let chars_ok = (1..=64).contains(&key.len())
        && key
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b));
    chars_ok && key != "." && key != ".."
}
