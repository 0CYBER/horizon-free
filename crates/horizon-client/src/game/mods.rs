//! Horizon mods in the game (`docs/mods.md`): loading the ones turned on,
//! their tick, frame, chat and command hooks, their HUD panels and what
//! they print. With no mod on, `horizon_mods::Mods` holds no engine and
//! every hook here returns after one check.
//!
//! Code mods run only in builds with the `mods` feature (release builds):
//! without it the game links no wasmtime, and every hook does nothing.
//! Content mods (`content_root`) load in every build.
use super::Game;
use horizon_client::{
    command::{Dispatcher, vanilla},
    extras_hud::Corners,
    hud::HudBatch,
    mod_list,
};
use std::{path::PathBuf, time::Duration};
#[cfg(feature = "mods")]
use {
    super::Screen,
    super::{execution_role::ExecutionRole, lan::LanHost, server_mods::ModEvent},
    horizon_client::{
        extras::Corner,
        extras_hud,
        mod_list::Installed,
        text::{self, Text},
    },
    horizon_mods::{Clock, Mods, Output, Player, Runs, ServerPlayer, Snapshot, Status},
    std::time::Instant,
};

#[derive(Default)]
pub(super) struct ModsState {
    #[cfg(feature = "mods")]
    pub(super) mods: Mods,
    /// `--dev-mod PATH`: always on, reloaded when rebuilt.
    pub(super) dev: Vec<PathBuf>,
    /// When dev mods were last checked for a rebuild.
    #[cfg(feature = "mods")]
    checked: Option<Instant>,
    /// The chat's command tree while a running mod owns commands.
    #[cfg(feature = "mods")]
    tree: Option<Dispatcher>,
    /// A server mod runs here: what players do is queued for it.
    #[cfg(feature = "mods")]
    pub(super) server: bool,
    /// The players server mods saw last tick, for joins and leaves.
    #[cfg(feature = "mods")]
    pub(super) players: Vec<ServerPlayer>,
    /// What players did this tick, for server mods at its end.
    #[cfg(feature = "mods")]
    pub(super) events: Vec<ModEvent>,
    /// A running server mod has capability `world`: its calls get the
    /// levels' chunks to read.
    #[cfg(feature = "mods")]
    pub(super) world: bool,
    /// Blocks server mods set, by dimension, made before that level's next
    /// tick.
    #[cfg(feature = "mods")]
    pub(super) block_writes: Vec<(String, horizon_mods::BlockChange)>,
}
impl ModsState {
    /// The chat's command tree: vanilla's, plus the running mods' commands.
    pub(super) fn dispatcher(&self) -> &Dispatcher {
        #[cfg(feature = "mods")]
        if let Some(tree) = &self.tree {
            return tree;
        }
        vanilla::game_dispatcher()
    }
    /// Whether a mod runs: the chat keeps its lines for them.
    pub(super) fn running(&self) -> bool {
        #[cfg(feature = "mods")]
        if self.mods.is_running() {
            return true;
        }
        false
    }
    /// Whether a mod draws a HUD panel.
    pub(super) fn has_panels(&self) -> bool {
        #[cfg(feature = "mods")]
        if self.mods.panels().next().is_some() {
            return true;
        }
        false
    }
}
/// How often dev mods are checked for a rebuild.
#[cfg(feature = "mods")]
const RELOAD_CHECK: Duration = Duration::from_secs(1);

/// A vanilla command name: mods may not take it.
#[cfg(feature = "mods")]
fn vanilla_command(name: &str) -> bool {
    let d = vanilla::game_dispatcher();
    d.nodes[0]
        .children
        .iter()
        .any(|&c| d.nodes[c].name() == name)
}

/// `root` with the content mods that are on layered over it
/// (docs/modpacks.md): their blocks, items, data and assets. Read once at
/// start, before the resources load; a mod whose content is refused is left
/// out here and its row on the Mods page says why.
pub(super) fn content_root(
    root: &std::path::Path,
    enabled: &std::collections::BTreeSet<String>,
    dev: &[std::path::PathBuf],
) -> std::path::PathBuf {
    use horizon_client::mod_content::{self, ContentMod};
    use horizon_mods::{Capability, Content, Package};
    let here = std::env::current_dir().unwrap_or_default();
    let mut paths = horizon_mods::find(&here.join(mod_list::DIR));
    paths.retain(|p| !dev.contains(p));
    paths.extend(dev.iter().cloned());
    let mut mods: Vec<ContentMod> = Vec::new();
    for path in &paths {
        let Ok(package) = Package::read(path) else {
            continue;
        };
        let m = &package.manifest;
        let on = enabled.contains(&m.id) || dev.contains(path);
        if !on
            || !m.capabilities.contains(&Capability::Registries)
            || mods.iter().any(|c| c.id == m.id)
        {
            continue;
        }
        let content = Content::read(path, m).and_then(|content| {
            // The first of two mods owning one namespace stays on.
            let own = (m.id.as_str(), content.namespaces.as_slice());
            match mods
                .iter()
                .find_map(|c| horizon_mods::content::clash((&c.id, &c.content.namespaces), own))
            {
                Some(e) => Err(e),
                None => Ok(content),
            }
        });
        match content {
            Ok(content) => mods.push(ContentMod {
                id: m.id.clone(),
                content,
            }),
            Err(e) => log::warn!("Horizon: mod {} content not loaded: {e}", m.id),
        }
    }
    match mod_content::resource_root(root, &mods, &here.join(".horizon-cache")) {
        Ok(out) => {
            if out != root {
                log::info!(
                    "Horizon: content from {} mod(s) in {}",
                    mods.len(),
                    out.display()
                );
            }
            out
        }
        Err(e) => {
            log::warn!("Horizon: content mods not loaded: {e}");
            root.to_owned()
        }
    }
}

/// A build without the `mods` feature: no mod runs.
#[cfg(not(feature = "mods"))]
impl Game<'_> {
    /// Says why the installed and dev mods don't run.
    pub(super) fn start_mods(&mut self) {
        let dir = Self::options_path().with_file_name(mod_list::DIR);
        let found = horizon_mods::find(&dir).len() + self.mods.dev.len();
        if found > 0 {
            log::warn!(
                "Horizon: {found} mod(s) not started: this build has no mod runtime (build with --features mods)"
            );
        }
    }
    pub(super) fn sync_mods(&mut self) {}
    pub(super) fn tick_mods(&mut self) {}
    pub(super) fn mods_frame(&mut self, _elapsed: Duration) {}
    pub(super) fn mod_command(&mut self, _command: &str) -> bool {
        false
    }
    pub(super) fn mods_chat(&mut self, _line: &str) {}
    pub(super) fn draw_mod_panels(&self, _batch: &mut HudBatch, _corners: &mut Corners) {}
}

#[cfg(feature = "mods")]
impl Game<'_> {
    /// Reads `horizon-mods/` and the dev mods, and starts the ones turned on.
    pub(super) fn start_mods(&mut self) {
        let dir = Self::options_path().with_file_name(mod_list::DIR);
        let dev = &self.mods.dev;
        let mut paths = horizon_mods::find(&dir);
        paths.retain(|p| !dev.contains(p));
        paths.extend(dev.iter().cloned());
        // A dev mod is on whatever the list says.
        let dev_ids: Vec<String> = dev
            .iter()
            .filter_map(|p| horizon_mods::Package::read(p).ok())
            .map(|p| p.manifest.id)
            .collect();
        let enabled = &self.settings.mods.enabled;
        let on = |id: &str| enabled.contains(id) || dev_ids.iter().any(|d| d == id);
        let mods = Mods::load_for(self.mod_runs(), &paths, on, vanilla_command);
        self.mods.mods = mods;
        self.list_mods();
        self.sync_mod_commands();
        self.show_mod_output();
    }
    /// Which mods this game runs: a server the server mods, the window
    /// client mods, and both when it simulates the world itself (in-frame).
    fn mod_runs(&self) -> Runs {
        match self.options.execution_role {
            ExecutionRole::IntegratedServer | ExecutionRole::DedicatedServer => Runs::Server,
            ExecutionRole::Client if self.options.server_thread => Runs::Client,
            ExecutionRole::Client => Runs::Both,
        }
    }
    /// The Mods page's rows, from what loaded.
    fn list_mods(&mut self) {
        self.mods.server = self.mods.mods.runs_server();
        self.mods.world = self.mods.mods.wants_world();
        let dev = &self.mods.dev;
        self.settings.mods.installed = self
            .mods
            .mods
            .entries()
            .iter()
            .map(|e| {
                let file = e.path.file_name().unwrap_or_default().to_string_lossy();
                let problem = match &e.status {
                    Status::Failed(reason) => Some(format!("Stopped: {reason}")),
                    _ => None,
                };
                let Some(m) = &e.manifest else {
                    return Installed {
                        id: String::new(),
                        title: file.into_owned(),
                        description: String::new(),
                        problem,
                        dev: false,
                    };
                };
                // What it may do first: the row cuts long descriptions.
                let may: Vec<&str> = m.capabilities.iter().map(|c| c.describe()).collect();
                let may = if may.is_empty() {
                    "Asks for nothing.".to_owned()
                } else {
                    format!("May {}.", may.join(", "))
                };
                Installed {
                    id: m.id.clone(),
                    title: format!("{} {}", m.name, m.version),
                    description: format!("{may} {}", m.description).trim().to_owned(),
                    problem,
                    dev: dev.contains(&e.path),
                }
            })
            .collect();
    }
    /// The chat's tree after the running mods changed.
    fn sync_mod_commands(&mut self) {
        let mut names = self.mods.mods.commands().peekable();
        self.mods.tree = names
            .peek()
            .is_some()
            .then(|| vanilla::game_dispatcher().with_mod_commands(names));
    }
    /// After the options changed: restart the mods if the list of the ones
    /// turned on did.
    pub(super) fn sync_mods(&mut self) {
        let running: Vec<&str> = self
            .mods
            .mods
            .entries()
            .iter()
            .filter(|e| e.status != Status::Off)
            .filter_map(|e| Some(e.manifest.as_ref()?.id.as_str()))
            .collect();
        let wanted: Vec<&str> = self
            .settings
            .mods
            .installed
            .iter()
            .enumerate()
            .filter(|&(i, _)| self.settings.mods.is_on(i))
            .map(|(_, m)| m.id.as_str())
            .collect();
        if running != wanted {
            self.start_mods();
        }
    }
    /// What mods read this tick: the server's players as they were at the
    /// last tick's end.
    fn mod_snapshot(&self) -> Snapshot {
        let players = if self.mods.server {
            self.mods.players.clone()
        } else {
            Vec::new()
        };
        self.mod_snapshot_with(players)
    }
    /// What mods read, with these players on the server.
    pub(super) fn mod_snapshot_with(&self, players: Vec<ServerPlayer>) -> Snapshot {
        let player = self.player.as_ref().map(|p| Player {
            x: p.position.x,
            y: p.position.y,
            z: p.position.z,
            yaw: self.yaw,
            pitch: self.pitch,
            health: p.health.health(),
            food: p.health.food.food.max(0) as u32,
            dimension: self.dim.id().to_owned(),
        });
        let clock = player.is_some().then(|| Clock {
            game_time: self.world.game_time.max(0) as u64,
            day_time: self.world.day_time.max(0) as u64,
        });
        Snapshot {
            player,
            clock,
            players,
        }
    }
    /// A game tick.
    pub(super) fn tick_mods(&mut self) {
        if !self.mods.mods.is_running() {
            return;
        }
        let snapshot = self.mod_snapshot();
        self.mods.mods.set_snapshot(snapshot);
        self.mods.mods.tick(self.ticks);
        self.show_mod_output();
    }
    /// Restarts the dev mods rebuilt since the last check, once a second.
    pub(super) fn reload_dev_mods(&mut self) {
        if self.mods.dev.is_empty()
            || self
                .mods
                .checked
                .is_some_and(|t| t.elapsed() < RELOAD_CHECK)
        {
            return;
        }
        self.mods.checked = Some(Instant::now());
        if self.mods.mods.reload_changed(vanilla_command) {
            self.list_mods();
            self.sync_mod_commands();
            if self.options.execution_role.is_client() {
                self.system_message(Text::literal("Mods reloaded").with_color(text::GRAY));
            } else {
                log::info!("Horizon: mods reloaded");
            }
        }
    }
    /// Once per frame: the frame hook, and dev mods' rebuilds.
    pub(super) fn mods_frame(&mut self, elapsed: Duration) {
        self.reload_dev_mods();
        if !self.mods.mods.is_running() {
            return;
        }
        self.mods.mods.frame(elapsed.as_secs_f32());
        self.show_mod_output();
    }
    /// `/command` typed in chat: true when a mod owns it.
    pub(super) fn mod_command(&mut self, command: &str) -> bool {
        if !self.mods.mods.is_running() {
            return false;
        }
        // The command may follow a teleport in the same tick.
        let snapshot = self.mod_snapshot();
        self.mods.mods.set_snapshot(snapshot);
        if !self.mods.mods.command(command) {
            return false;
        }
        self.show_mod_output();
        true
    }
    /// A chat line the player received.
    pub(super) fn mods_chat(&mut self, line: &str) {
        self.mods.mods.chat(line);
        self.show_mod_output();
    }
    /// What mods showed: client mods' lines into the chat, server mods'
    /// to their players.
    pub(super) fn show_mod_output(&mut self) {
        self.route_mod_output(None);
    }
    /// [`Game::show_mod_output`], with the hub when it is out of `self.lan`.
    pub(super) fn route_mod_output(&mut self, mut host: Option<&mut LanHost>) {
        for output in self.mods.mods.drain() {
            let line = match output {
                Output::Chat { name, text } => {
                    let mut line = Text::literal(format!("[{name}] ")).with_color(text::GRAY);
                    line.push(Text::literal(text));
                    line
                }
                Output::Feedback { text, error: false } => Text::literal(text),
                Output::Feedback { text, error: true } => Text::literal(text).with_color(text::RED),
                Output::Stopped { name, reason } => {
                    self.list_mods();
                    self.sync_mod_commands();
                    let line = format!("Mod {name} stopped: {reason}");
                    if self.options.dedicated.is_some() {
                        super::dedicated::log("WARN", &line);
                    }
                    Text::literal(line).with_color(text::RED)
                }
                server @ (Output::Tell { .. }
                | Output::Broadcast { .. }
                | Output::Reply { .. }
                | Output::SetBlocks { .. }) => {
                    self.route_server_output(host.as_deref_mut(), server);
                    continue;
                }
            };
            self.add_mod_chat_line(line);
        }
    }
    /// A mod's line in the window's chat; not through system_message: a mod
    /// must not hear its own lines.
    pub(super) fn add_mod_chat_line(&mut self, line: Text) {
        let focused = self.screen == Screen::Chat;
        self.ui
            .chat
            .add(&self.gui.atlas, line, None, self.ticks, focused);
    }
    /// The mods' HUD panels, stacked with the extras in their corners.
    pub(super) fn draw_mod_panels(&self, batch: &mut HudBatch, corners: &mut Corners) {
        for panel in self.mods.mods.panels() {
            let corner = match panel.corner {
                horizon_mods::Corner::TopLeft => Corner::TopLeft,
                horizon_mods::Corner::TopRight => Corner::TopRight,
                horizon_mods::Corner::BottomLeft => Corner::BottomLeft,
                horizon_mods::Corner::BottomRight => Corner::BottomRight,
            };
            // F3 takes the top left, as it does from the extras.
            if self.debug && corner == Corner::TopLeft {
                continue;
            }
            let lines: Vec<(&str, u32)> = panel
                .lines
                .iter()
                .map(|l| (l.text.as_str(), l.color))
                .collect();
            extras_hud::draw_lines(batch, &self.gui.atlas, corners, corner, &lines);
        }
    }
}
