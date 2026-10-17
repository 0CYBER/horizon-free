//! The integrated server (`IntegratedServer`): single player's world runs on
//! its own "Server thread", as vanilla's does, and the window joins it as a
//! player through memory, so a world tick never holds up a frame.
//!
//! Opt-in for now (`--server-thread` or `HORIZON_SERVER_THREAD=1`): the
//! player joins as a network player, and the server side of a network player
//! does not do everything single player does yet. Worlds whose player is
//! outside the Overworld open the usual way, since network players stay in
//! the Overworld so far.
use super::{
    Dim, Game, Options, PackResources, Resources,
    lan::{LanSettings, Owner},
};
use horizon_client::{
    Result,
    audio::Audio,
    hud::HudAtlas,
    lan::{MAX_PLAYERS, NetConfig, hub::HubConfig},
    save::Level,
    settings::Settings,
    title::WorldOptions,
};
use horizon_protocol::connection::Pipe;
use horizon_protocol::{crypto::offline_uuid, text::translatable};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc::{Sender, channel},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

/// What the window tells its server thread.
#[derive(Default)]
struct Flags {
    /// `Minecraft.pause`: a single-player pause screen is open.
    paused: AtomicBool,
    stop: AtomicBool,
    smoothed_tick_ms: AtomicU32,
}

/// The window's handle on its server thread.
pub(super) struct Integrated {
    flags: Arc<Flags>,
    thread: Option<JoinHandle<()>>,
}
impl Integrated {
    pub fn smoothed_tick_ms(&self) -> f32 {
        f32::from_bits(self.flags.smoothed_tick_ms.load(Ordering::Relaxed))
    }
    /// `IntegratedServer.tickServer` skips the world while the game is paused.
    pub fn set_paused(&self, paused: bool) {
        self.flags.paused.store(paused, Ordering::Relaxed);
    }
    /// `IntegratedServer.halt(true)`: returns once the world is saved and let go.
    pub fn stop(mut self) {
        self.halt();
    }
    fn halt(&mut self) {
        self.flags.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
impl Drop for Integrated {
    fn drop(&mut self) {
        self.halt();
    }
}

/// The world to open and the player who owns it.
struct Launch {
    folder: String,
    name: Option<String>,
    seed: Option<i64>,
    creative: bool,
    create: Option<WorldOptions>,
    /// The window's profile: the only one let in, and the world's player.
    owner: u128,
}

/// Whether this world opens on a server thread: asked for, and its player
/// (if any yet) in the Overworld.
pub(super) fn wanted(options: &Options, world: &Path) -> bool {
    options.server_thread
        && options.dedicated.is_none()
        && Level::read(world).is_none_or(|l| Dim::from_id(&l.dimension) == Dim::Overworld)
}

impl Game<'_> {
    /// `Minecraft.doWorldLoad`: start the server thread on the world, wait
    /// until it can be joined, then join it through memory.
    pub(super) fn open_integrated(
        &mut self,
        folder: &str,
        name: Option<&str>,
        seed: Option<i64>,
        creative: bool,
        create: Option<&WorldOptions>,
    ) -> Result<()> {
        let auth = horizon_client::auth::current();
        let player = auth
            .as_ref()
            .map_or_else(|| self.options.player_name(), |a| a.name.clone());
        let launch = Launch {
            folder: folder.to_owned(),
            name: name.map(str::to_owned),
            seed,
            creative,
            create: create.cloned(),
            // An offline-mode server names its players by their offline profile.
            owner: offline_uuid(&player),
        };
        let mut options = self.options.clone();
        // A server without a console: no local player, no window, no control socket.
        options.dedicated = Some(Vec::new());
        options.server_thread = false;
        options.integrated = true;
        options.control = None;
        options.benchmark = None;
        options.frames = None;
        options.world_frames = None;
        options.report = None;
        options.server = None;
        options.world = None;
        options.dev_mods.clear();
        let (res, pack, settings) = (self.res.clone(), self.pack.clone(), self.settings.clone());
        let flags = Arc::new(Flags::default());
        let (ready_tx, ready) = channel();
        let server_flags = flags.clone();
        let thread = std::thread::Builder::new()
            .name("Server thread".into())
            .spawn(move || {
                let result = serve(
                    &res,
                    pack,
                    options,
                    settings,
                    launch,
                    &server_flags,
                    &ready_tx,
                );
                if let Err(e) = result {
                    log::error!("Horizon: integrated server: {e}");
                    let _ = ready_tx.send(Err(e.to_string()));
                }
            })?;
        let handle = Integrated {
            flags,
            thread: Some(thread),
        };
        let pipe = match ready.recv() {
            Ok(Ok(pipe)) => pipe,
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => return Err("the integrated server stopped".into()),
        };
        self.connect_local(pipe, &player)?;
        self.integrated = Some(handle);
        Ok(())
    }

    /// The server thread's loop (`MinecraftServer.runServer`): 20 ticks a
    /// second, the world held while the window is paused.
    fn run_integrated(
        &mut self,
        launch: Launch,
        flags: &Flags,
        ready: &Sender<std::result::Result<Pipe, String>>,
    ) -> Result<()> {
        self.open_world(
            &launch.folder,
            launch.name.as_deref(),
            launch.seed,
            launch.creative,
            launch.create.as_ref(),
        )?;
        let saved = Box::new(self.level.clone());
        let view_distance = self.settings.render_distance.clamp(2, 32);
        let config = HubConfig {
            default_game_mode: self.level.mode(),
            guest_commands: true,
            force_game_mode: false,
            hardcore: self.level.hardcore,
            view_distance,
            simulation_distance: self.settings.simulation_distance.clamp(2, 32),
            dimension: self.dim.id().to_owned(),
            seed: self.level.seed,
            max_players: MAX_PLAYERS,
            // No player of its own: the hub runs without a host.
            dedicated: true,
        };
        let owner = launch.owner;
        let net = NetConfig {
            // No network until it is opened to the LAN.
            listen: false,
            // A memory connection is not compressed (`Connection.isMemoryConnection`).
            compression_threshold: -1,
            enable_status: false,
            announce: false,
            gate: Some(Arc::new(move |_, uuid, _| {
                (uuid != owner).then(|| translatable("multiplayer.disconnect.not_whitelisted", &[]))
            })),
            ..NetConfig::lan(0, self.level.name.clone(), MAX_PLAYERS)
        };
        let settings = LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        };
        self.lan_host(settings, net, config, None)?;
        let commands = self.allow_commands();
        let pipe = {
            let host = self
                .lan
                .as_mut()
                .ok_or("the integrated server did not start")?;
            // The owner is an operator when the world allows commands
            // (`IntegratedPlayerList`, `MinecraftServer.getProfilePermissions`).
            host.hub.ops.insert(owner, if commands { 4 } else { 0 });
            host.owner = Some(Owner {
                uuid: owner,
                saved: Some(saved),
            });
            host.hub.lan.connect_local()?
        };
        let _ = ready.send(Ok(pipe));
        self.settle_spawn()?;
        let mut next = Instant::now();
        while !flags.stop.load(Ordering::Relaxed) {
            self.update_dedicated_streaming();
            self.update_world();
            if flags.paused.load(Ordering::Relaxed) {
                // Only the connections run.
                self.tick_lan();
            } else {
                let started = Instant::now();
                self.tick()?;
                self.debug_charts.log_tick(started, started.elapsed());
                flags.smoothed_tick_ms.store(
                    self.debug_charts.smoothed_tick_ms.to_bits(),
                    Ordering::Relaxed,
                );
            }
            let rate = self.clock.rate().max(1.0);
            next += Duration::from_secs_f64(1.0 / f64::from(rate));
            let now = Instant::now();
            if next > now {
                std::thread::sleep(next - now);
            } else if now - next > Duration::from_secs(2) {
                next = now;
            }
        }
        self.stop_integrated()
    }

    /// `MinecraftServer.stopServer`: the players are saved and sent away,
    /// then the world is saved.
    fn stop_integrated(&mut self) -> Result<()> {
        if let Some(mut host) = self.lan.take() {
            for g in &host.hub.guests {
                self.save_guest(&host, g);
            }
            let conns: Vec<u32> = host.hub.guests.iter().map(|g| g.conn).collect();
            for conn in conns {
                host.hub.kick(
                    conn,
                    translatable("multiplayer.disconnect.server_shutdown", &[]),
                );
            }
            self.lan = Some(host);
        }
        std::thread::sleep(Duration::from_millis(100));
        self.save_game()?;
        self.lan_close(false);
        self.save.flush();
        self.session_lock = None;
        Ok(())
    }
}

/// The server thread: a game of its own over the shared resources.
fn serve(
    res: &Arc<Resources>,
    pack: PackResources,
    options: Options,
    settings: Settings,
    launch: Launch,
    flags: &Flags,
    ready: &Sender<std::result::Result<Pipe, String>>,
) -> Result<()> {
    // It draws nothing: only the font, which books and chat measure with.
    let atlas = HudAtlas::fonts(&options.resources)?;
    let audio = Audio::silent(&options.resources);
    let mut server = Game::new(res, pack, atlas, options, settings, audio)?;
    server.run_integrated(launch, flags, ready)
}
