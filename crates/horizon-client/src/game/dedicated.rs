//! The dedicated server (net.minecraft.server.Main and DedicatedServer):
//! `horizon-client --dedicated [--nogui] [--port N] [--world NAME]
//! [--universe DIR] [--initSettings]`. It reads server.properties and
//! eula.txt from the working directory, opens `<universe>/<level-name>` and
//! runs the same game as single player, without a local player: the world,
//! its simulation and the play-state hub that LAN hosting uses, configured
//! from server.properties, with the operator, whitelist and ban lists and a
//! console on standard input.
use super::{Game, Options, lan::LanSettings};
use horizon_client::{
    Result,
    lan::{LoginGate, NetConfig, hub::HubConfig},
    server::{
        eula,
        java_properties::now_millis,
        properties::{GAME_MODES, ServerProperties, SystemEntropy},
        user_lists::{
            BanEntry, BanList, IpBanList, NameAndId, OpEntry, OpList, WhiteEntry, WhiteList,
            format_date,
        },
    },
    text::Text,
};
use horizon_protocol::text::translatable;
use horizon_world::nbt::{JavaString, Tag};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock, mpsc},
    time::{Duration, Instant},
};

/// Double.toString for the magnitudes positions have (no exponent form
/// between 1e-3 and 1e7).
pub(super) fn java_double(v: f64) -> String {
    if v == 0.0 || (1e-3..1e7).contains(&v.abs()) {
        format!("{v:?}")
    } else {
        let s = format!("{v:E}");
        let (m, e) = s.split_once('E').unwrap_or((&s, "0"));
        let m = if m.contains('.') {
            m.to_owned()
        } else {
            format!("{m}.0")
        };
        format!("{m}E{e}")
    }
}

/// What the startup read before the game's resources were loaded.
struct Setup {
    props: ServerProperties,
    universe: PathBuf,
    level_name: String,
    port: Option<u16>,
    started: Instant,
}
static SETUP: OnceLock<Mutex<Option<Setup>>> = OnceLock::new();

/// Log4j's console pattern: `[HH:mm:ss] [thread/LEVEL]: message`.
pub(super) fn log(level: &str, message: &str) {
    let file_level = match level {
        "WARN" => ::log::Level::Warn,
        "ERROR" => ::log::Level::Error,
        _ => ::log::Level::Info,
    };
    horizon_client::logging::write_only(file_level, message);
    let secs = now_millis().div_euclid(1000).rem_euclid(86_400);
    println!(
        "[{:02}:{:02}:{:02}] [Server thread/{level}]: {message}",
        secs / 3600,
        secs / 60 % 60,
        secs % 60
    );
}

/// Main.main up to the server start: arguments, server.properties (stored
/// back at once), eula.txt. False when the process should stop here.
pub(super) fn preflight(options: &mut Options) -> Result<bool> {
    let args = options.dedicated.clone().unwrap_or_default();
    let mut universe = PathBuf::from(".");
    let mut world = None;
    let mut port = None;
    let mut init_settings = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let name = arg.trim_start_matches('-');
        match name {
            "nogui"
            | "demo"
            | "bonusChest"
            | "forceUpgrade"
            | "eraseCache"
            | "recreateRegionFiles"
            | "safeMode"
            | "jfrProfile" => {}
            "initSettings" => init_settings = true,
            "help" => {
                eprintln!(
                    "Option                Description\n------                -----------\n--help\n--initSettings        Initializes 'server.properties' and 'eula.txt', then quits\n--nogui\n--port <Integer>      (default: -1)\n--universe <String>   (default: .)\n--world <String>"
                );
                return Ok(false);
            }
            "universe" => universe = PathBuf::from(it.next().ok_or("--universe needs a value")?),
            "world" => world = Some(it.next().ok_or("--world needs a value")?.clone()),
            "port" => {
                let v: i32 = it.next().ok_or("--port needs a value")?.parse()?;
                port = u16::try_from(v).ok();
            }
            "serverId" | "pidFile" => {
                let value = it.next().ok_or("missing value")?;
                if name == "pidFile" {
                    std::fs::write(value, std::process::id().to_string())?;
                }
            }
            _ => return Err(format!("{arg} is not a recognized option").into()),
        }
    }
    let started = Instant::now();
    let settings = PathBuf::from("server.properties");
    let props = ServerProperties::from_file(&settings, &mut SystemEntropy::default())?;
    for w in &props.warnings {
        log("WARN", w);
    }
    props.store(&settings)?;
    let eula_file = PathBuf::from("eula.txt");
    let agreed = match eula::agreed(&eula_file) {
        Ok(a) => a,
        Err(w) => {
            log("WARN", &w);
            false
        }
    };
    if init_settings {
        let abs = |p: &PathBuf| std::env::current_dir().map_or(p.clone(), |d| d.join(p));
        log(
            "INFO",
            &format!(
                "Initialized '{}' and '{}'",
                abs(&settings).display(),
                abs(&eula_file).display()
            ),
        );
        return Ok(false);
    }
    if !agreed {
        log(
            "INFO",
            "You need to agree to the EULA in order to run the server. Go to eula.txt for more info.",
        );
        return Ok(false);
    }
    let level_name = world.unwrap_or_else(|| props.level_name.clone());
    SETUP
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap()
        .replace(Setup {
            props,
            universe,
            level_name,
            port,
            started,
        });
    Ok(true)
}

/// The operator, whitelist and ban lists (DedicatedPlayerList), shared with
/// the connection threads that check logins.
pub(super) struct Lists {
    pub ops: OpList,
    pub white: WhiteList,
    pub bans: BanList,
    pub ip_bans: IpBanList,
    pub use_whitelist: bool,
    pub enforce_whitelist: bool,
}
impl Lists {
    fn load() -> Self {
        let mut l = Self {
            ops: OpList::new("ops.json"),
            white: WhiteList::new("whitelist.json"),
            bans: BanList::new("banned-players.json"),
            ip_bans: IpBanList::new("banned-ips.json"),
            use_whitelist: false,
            enforce_whitelist: false,
        };
        let warn = |what: &str, r: std::result::Result<(), String>| {
            if let Err(e) = r {
                log("WARN", &format!("Failed to load {what}: {e}"));
            }
        };
        // DedicatedPlayerList's constructor order.
        warn("user banlist", l.bans.load());
        let _ = l.bans.save();
        warn("ip banlist", l.ip_bans.load());
        let _ = l.ip_bans.save();
        warn("operators list", l.ops.load());
        warn("white-list", l.white.load());
        let _ = l.ops.save();
        if !l.white.file.exists() {
            let _ = l.white.save();
        }
        l
    }
    /// PlayerList.isWhiteListed.
    fn whitelisted(&self, user: &NameAndId) -> bool {
        !self.use_whitelist || self.ops.contains(user) || self.white.contains(user)
    }
    /// PlayerList.canPlayerLogin before the player count.
    fn can_login(&mut self, address: std::net::SocketAddr, user: &NameAndId) -> Option<Tag> {
        if self.bans.contains(user)
            && let Some(ban) = self.bans.get(user).cloned()
        {
            let mut reason = translatable_tag(
                "multiplayer.disconnect.banned.reason",
                vec![ban_reason(&ban.reason)],
            );
            if let Some(expires) = ban.expires {
                append(
                    &mut reason,
                    translatable_tag(
                        "multiplayer.disconnect.banned.expiration",
                        vec![Tag::String(JavaString::from(ban_date(expires).as_str()))],
                    ),
                );
            }
            return Some(reason);
        }
        if !self.whitelisted(user) {
            return Some(translatable("multiplayer.disconnect.not_whitelisted", &[]));
        }
        let ip = address.ip().to_string();
        if self.ip_bans.contains(&ip)
            && let Some(ban) = self.ip_bans.get(&ip).cloned()
        {
            let mut reason = translatable_tag(
                "multiplayer.disconnect.banned_ip.reason",
                vec![ban_reason(&ban.reason)],
            );
            if let Some(expires) = ban.expires {
                append(
                    &mut reason,
                    translatable_tag(
                        "multiplayer.disconnect.banned_ip.expiration",
                        vec![Tag::String(JavaString::from(ban_date(expires).as_str()))],
                    ),
                );
            }
            return Some(reason);
        }
        None
    }
}

/// PlayerList.BAN_DATE_FORMAT ("yyyy-MM-dd 'at' HH:mm:ss z"), in UTC.
fn ban_date(millis: i64) -> String {
    let s = format_date(millis);
    format!("{} at {} UTC", &s[..10], &s[11..19])
}
/// BanListEntry.getReasonMessage.
fn ban_reason(reason: &Option<String>) -> Tag {
    match reason {
        Some(r) => Tag::String(JavaString::from(r.as_str())),
        None => translatable("multiplayer.disconnect.banned.reason.default", &[]),
    }
}
fn translatable_tag(key: &str, args: Vec<Tag>) -> Tag {
    Tag::compound([
        ("translate", Tag::String(JavaString::from(key))),
        ("with", Tag::List(args)),
    ])
}
/// MutableComponent.append.
fn append(to: &mut Tag, sibling: Tag) {
    if let Tag::Compound(map) = to {
        let extra = map
            .entry(JavaString::from("extra"))
            .or_insert_with(|| Tag::List(Vec::new()));
        if let Tag::List(items) = extra {
            items.push(sibling);
        }
    }
}

/// What the server loop keeps between ticks.
struct ServerState {
    lists: Arc<Mutex<Lists>>,
    console: mpsc::Receiver<String>,
    stop: bool,
    autosave: bool,
    empty_ticks: i32,
}

impl Game<'_> {
    /// DedicatedServer.initServer and MinecraftServer.runServer.
    pub(super) fn run_dedicated(&mut self) -> Result<()> {
        let Setup {
            props,
            universe,
            level_name,
            port,
            started,
        } = SETUP
            .get()
            .and_then(|m| m.lock().unwrap().take())
            .ok_or("dedicated server not prepared")?;
        // The console thread starts first, as in initServer.
        let (tx, console) = mpsc::channel();
        std::thread::Builder::new()
            .name("Server console handler".into())
            .spawn(move || {
                let stdin = std::io::stdin();
                let mut line = String::new();
                while stdin.read_line(&mut line).is_ok_and(|n| n > 0) {
                    if tx
                        .send(line.trim_end_matches(['\r', '\n']).to_owned())
                        .is_err()
                    {
                        break;
                    }
                    line.clear();
                }
            })?;
        log("INFO", "Starting minecraft server version 26.3");
        log("INFO", "Loading properties");
        let game_mode = props.game_mode;
        log(
            "INFO",
            &format!(
                "Default game type: {}",
                GAME_MODES[game_mode as usize].to_uppercase()
            ),
        );
        let port = port.unwrap_or(props.server_port as u16);
        log(
            "INFO",
            &format!(
                "Starting Minecraft server on {}:{port}",
                if props.server_ip.is_empty() {
                    "*"
                } else {
                    &props.server_ip
                }
            ),
        );
        if props.online_mode {
            // Authentication needs Mojang's session servers (an HTTPS client
            // and RSA), which Horizon does not have yet: offline mode.
            log(
                "WARN",
                "online-mode=true is not supported by Horizon yet: running in offline mode",
            );
        }
        log("WARN", "**** SERVER IS RUNNING IN OFFLINE/INSECURE MODE!");
        log(
            "WARN",
            "The server will make no attempt to authenticate usernames. Beware.",
        );
        log(
            "WARN",
            "While this makes the game possible to play without internet access, it also opens up the ability for hackers to connect with any username they choose.",
        );
        log(
            "WARN",
            "To change this, set \"online-mode\" to \"true\" in the server.properties file.",
        );
        let mut lists = Lists::load();
        lists.use_whitelist = props.white_list;
        lists.enforce_whitelist = props.enforce_whitelist;
        let lists = Arc::new(Mutex::new(lists));
        log("INFO", &format!("Preparing level \"{level_name}\""));
        let world_dir = std::env::current_dir()?.join(&universe).join(&level_name);
        let folder = world_dir.to_string_lossy().into_owned();
        let new_world = !world_dir.join("level.dat").exists();
        self.open_world(
            &folder,
            Some(&level_name),
            Some(props.seed),
            game_mode == 1,
            None,
        )?;
        if new_world {
            self.level.hardcore = props.hardcore;
        }
        // The single-player host's state is not used: players have their own files.
        self.level.position = None;
        // DedicatedServer.initServer: worldData.setGameType; forceDifficulty.
        self.set_default_game_mode(game_mode);
        self.ui.difficulty = props.difficulty as u8;
        self.settings.render_distance = props.view_distance.clamp(2, 32);
        self.settings.simulation_distance = props.simulation_distance.clamp(2, 32);
        self.streaming.radius = self.settings.render_distance;
        self.settle_spawn()?;
        let gate_lists = lists.clone();
        let gate: LoginGate = Arc::new(move |address, uuid, name| {
            let user = NameAndId {
                id: uuid,
                name: name.to_owned(),
            };
            gate_lists.lock().unwrap().can_login(address, &user)
        });
        let bypass_lists = lists.clone();
        let favicon = std::fs::read("server-icon.png")
            .ok()
            .filter(|b| png_size(b) == Some((64, 64)));
        let net = NetConfig {
            bind: (!props.server_ip.is_empty())
                .then(|| props.server_ip.parse().ok())
                .flatten(),
            port,
            motd: props.motd.clone(),
            max_players: props.max_players.max(0) as usize,
            compression_threshold: props.network_compression_threshold,
            enable_status: props.enable_status,
            hide_online_players: props.hide_online_players,
            favicon,
            // DedicatedServer.enforceSecureProfile needs online mode.
            enforces_secure_chat: false,
            accepts_transfers: props.accepts_transfers,
            announce: false,
            bypasses_limit: Some(Arc::new(move |uuid| {
                bypass_lists
                    .lock()
                    .unwrap()
                    .ops
                    .entries()
                    .iter()
                    .any(|e| e.user.id == uuid && e.bypasses_player_limit)
            })),
            gate: Some(gate),
        };
        let config = HubConfig {
            default_game_mode: game_mode,
            guest_commands: true,
            force_game_mode: props.force_game_mode,
            hardcore: self.level.hardcore,
            view_distance: props.view_distance.clamp(2, 32),
            simulation_distance: props.simulation_distance.clamp(2, 32),
            dimension: self.dim.id().to_owned(),
            seed: self.level.seed,
            max_players: props.max_players.max(0) as usize,
            dedicated: true,
        };
        let settings = LanSettings {
            port,
            guest_commands: true,
            force_game_mode: props.force_game_mode,
        };
        if let Err(e) = self.lan_host(settings, net, config, None) {
            log("WARN", "**** FAILED TO BIND TO PORT!");
            log("WARN", &format!("The exception was: {e}"));
            log("WARN", "Perhaps a server is already running on that port?");
            return Ok(());
        }
        self.sync_ops(&lists);
        log(
            "INFO",
            &format!(
                "Done ({:.3}s)! For help, type \"help\"",
                started.elapsed().as_secs_f64()
            ),
        );
        // DedicatedServer.initServer ends with saveEverything(false, true, true).
        self.save_game()?;
        self.log_saved_levels();
        let mut state = ServerState {
            lists,
            console,
            stop: false,
            autosave: true,
            empty_ticks: 0,
        };
        let pause_after = props.pause_when_empty_seconds * 20;
        let idle_timeout = props.player_idle_timeout;
        let mut next = Instant::now();
        while !state.stop {
            while let Ok(line) = state.console.try_recv() {
                self.console_command(&mut state, &line);
            }
            let players = self.lan.as_ref().map_or(0, |h| h.hub.guests.len());
            if pause_after > 0 {
                if players == 0 {
                    state.empty_ticks += 1;
                } else {
                    state.empty_ticks = 0;
                }
            }
            let paused = pause_after > 0 && state.empty_ticks >= pause_after;
            if paused && state.empty_ticks == pause_after {
                log(
                    "INFO",
                    &format!(
                        "Server empty for {} seconds, pausing",
                        props.pause_when_empty_seconds
                    ),
                );
                self.save_game()?;
            }
            self.update_dedicated_streaming();
            self.update_world();
            if paused {
                // MinecraftServer.tickServer: only the connections run.
                self.tick_lan();
            } else {
                if !state.autosave {
                    // `save-off`: the periodic save inside the tick is skipped.
                    self.ticks = self.ticks.max(1);
                }
                self.tick()?;
            }
            if idle_timeout > 0 {
                self.kick_idle(idle_timeout);
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
        self.stop_dedicated()
    }

    /// Load the spawn area and find the world spawn as single player does,
    /// without keeping a player there.
    fn settle_spawn(&mut self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(600);
        while self.player.is_none() {
            self.update_world();
            self.spawn_player()?;
            if Instant::now() > deadline {
                return Err("the spawn area did not load".into());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        self.player = None;
        Ok(())
    }

    /// The chunks to keep loaded: the players' (lan_update_streaming), or
    /// the spawn chunk while nobody is online.
    fn update_dedicated_streaming(&mut self) {
        let center = self
            .lan
            .as_ref()
            .and_then(|h| h.hub.guests.iter().find(|g| g.placed).map(|g| g.chunk()))
            .or_else(|| {
                self.spawn
                    .map(|s| ((s.x.floor() as i32) >> 4, (s.z.floor() as i32) >> 4))
            });
        if let Some(c) = center {
            self.streaming.center = c;
        }
    }

    /// ServerGamePacketListenerImpl.tick: players idle longer than the
    /// timeout are kicked.
    fn kick_idle(&mut self, minutes: i32) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let limit = u64::try_from(minutes).unwrap_or(0) * 1200;
        let idle: Vec<u32> = host
            .hub
            .guests
            .iter()
            .filter(|g| host.hub.ticks.saturating_sub(g.last_action_tick) > limit)
            .map(|g| g.conn)
            .collect();
        for conn in idle {
            host.hub
                .kick(conn, translatable("multiplayer.disconnect.idling", &[]));
        }
    }

    /// Ops' permission levels for the hub (PlayerList.getProfilePermissions).
    fn sync_ops(&mut self, lists: &Arc<Mutex<Lists>>) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        host.hub.ops = lists
            .lock()
            .unwrap()
            .ops
            .entries()
            .iter()
            .map(|e| (e.user.id, e.level as u8))
            .collect();
        host.hub.resend_commands();
    }

    /// A known player by name: online first, else the offline profile.
    fn resolve(&self, name: &str) -> NameAndId {
        self.lan
            .as_ref()
            .and_then(|h| {
                h.hub
                    .guests
                    .iter()
                    .find(|g| g.name.eq_ignore_ascii_case(name))
            })
            .map_or_else(
                || NameAndId::offline(name),
                |g| NameAndId {
                    id: g.uuid,
                    name: g.name.clone(),
                },
            )
    }

    /// MinecraftServer.sendSystemMessage: what the console is told.
    fn console_reply(&self, text: Text) {
        log("INFO", &format!("System chat: {}", text.plain()));
    }

    /// DedicatedServer.handleConsoleInputs: one console line, run as the
    /// server (permission level 4). The dedicated-only commands are here;
    /// everything else goes to the game's dispatcher.
    fn console_command(&mut self, state: &mut ServerState, line: &str) {
        let line = line.strip_prefix('/').unwrap_or(line);
        let mut words = line.split_whitespace();
        let Some(command) = words.next() else {
            return;
        };
        let rest: Vec<&str> = words.collect();
        let lit = |s: &str| Text::literal(s.to_owned());
        match command {
            "stop" => {
                self.console_reply(Text::translate("commands.stop.stopping", &[]));
                state.stop = true;
            }
            "save-all" => {
                self.console_reply(Text::translate("commands.save.saving", &[]));
                match self.save_game() {
                    Ok(()) => self.console_reply(Text::translate("commands.save.success", &[])),
                    Err(e) => log("ERROR", &format!("{e}")),
                }
            }
            "save-off" => {
                if state.autosave {
                    state.autosave = false;
                    self.console_reply(Text::translate("commands.save.disabled", &[]));
                } else {
                    self.console_reply(Text::translate("commands.save.alreadyOff", &[]));
                }
            }
            "save-on" => {
                if state.autosave {
                    self.console_reply(Text::translate("commands.save.alreadyOn", &[]));
                } else {
                    state.autosave = true;
                    self.console_reply(Text::translate("commands.save.enabled", &[]));
                }
            }
            "list" => {
                let guests: Vec<(String, u128)> = self
                    .lan
                    .as_ref()
                    .map(|h| {
                        h.hub
                            .guests
                            .iter()
                            .map(|g| (g.name.clone(), g.uuid))
                            .collect()
                    })
                    .unwrap_or_default();
                let max = self
                    .lan
                    .as_ref()
                    .map_or(0, |h| h.hub.lan.shared.max_players());
                let names: Vec<String> = if rest.first() == Some(&"uuids") {
                    guests
                        .iter()
                        .map(|(n, u)| {
                            format!("{n} ({})", horizon_protocol::crypto::uuid_string(*u))
                        })
                        .collect()
                } else {
                    guests.iter().map(|(n, _)| n.clone()).collect()
                };
                self.console_reply(Text::translate(
                    "commands.list.players",
                    &[
                        lit(&guests.len().to_string()),
                        lit(&max.to_string()),
                        lit(&names.join(", ")),
                    ],
                ));
            }
            "op" | "deop" if !rest.is_empty() => {
                let user = self.resolve(rest[0]);
                let level = 4;
                let changed = {
                    let mut l = state.lists.lock().unwrap();
                    if command == "op" {
                        if l.ops.contains(&user) {
                            false
                        } else {
                            l.ops.add(OpEntry {
                                user: user.clone(),
                                level,
                                bypasses_player_limit: false,
                            })
                        }
                    } else {
                        l.ops.remove(&user)
                    }
                };
                let key = match (command, changed) {
                    ("op", true) => "commands.op.success",
                    ("op", false) => "commands.op.failed",
                    (_, true) => "commands.deop.success",
                    _ => "commands.deop.failed",
                };
                self.console_reply(Text::translate(key, &[lit(&user.name)]));
                let lists = state.lists.clone();
                self.sync_ops(&lists);
            }
            "whitelist" => self.console_whitelist(state, &rest),
            "ban" if !rest.is_empty() => {
                let user = self.resolve(rest[0]);
                let reason = (rest.len() > 1).then(|| rest[1..].join(" "));
                let added = state.lists.lock().unwrap().bans.add(BanEntry::new(
                    user.clone(),
                    Some("Server".into()),
                    None,
                    reason.clone(),
                ));
                if added {
                    self.console_reply(Text::translate(
                        "commands.ban.success",
                        &[
                            lit(&user.name),
                            reason.as_deref().map_or_else(
                                || {
                                    Text::translate(
                                        "multiplayer.disconnect.banned.reason.default",
                                        &[],
                                    )
                                },
                                lit,
                            ),
                        ],
                    ));
                    self.kick_uuid(user.id, translatable("multiplayer.disconnect.banned", &[]));
                } else {
                    self.console_reply(Text::translate("commands.ban.failed", &[]));
                }
            }
            "ban-ip" if !rest.is_empty() => {
                let target = rest[0];
                let ip = if target.parse::<std::net::IpAddr>().is_ok() {
                    Some(target.to_owned())
                } else {
                    self.lan.as_ref().and_then(|h| {
                        h.hub
                            .guests
                            .iter()
                            .find(|g| g.name.eq_ignore_ascii_case(target))
                            .map(|g| g.address.ip().to_string())
                    })
                };
                let Some(ip) = ip else {
                    self.console_reply(Text::translate("commands.banip.invalid", &[]));
                    return;
                };
                let reason = (rest.len() > 1).then(|| rest[1..].join(" "));
                let added = state.lists.lock().unwrap().ip_bans.add(BanEntry::new(
                    ip.clone(),
                    Some("Server".into()),
                    None,
                    reason.clone(),
                ));
                if !added {
                    self.console_reply(Text::translate("commands.banip.failed", &[]));
                    return;
                }
                self.console_reply(Text::translate(
                    "commands.banip.success",
                    &[
                        lit(&ip),
                        reason.as_deref().map_or_else(
                            || Text::translate("multiplayer.disconnect.banned.reason.default", &[]),
                            lit,
                        ),
                    ],
                ));
                let banned: Vec<u128> = self
                    .lan
                    .as_ref()
                    .map(|h| {
                        h.hub
                            .guests
                            .iter()
                            .filter(|g| g.address.ip().to_string() == ip)
                            .map(|g| g.uuid)
                            .collect()
                    })
                    .unwrap_or_default();
                if !banned.is_empty() {
                    self.console_reply(Text::translate(
                        "commands.banip.info",
                        &[lit(&banned.len().to_string()), lit("")],
                    ));
                }
                for uuid in banned {
                    self.kick_uuid(uuid, translatable("multiplayer.disconnect.ip_banned", &[]));
                }
            }
            "pardon" if !rest.is_empty() => {
                let user = self.resolve(rest[0]);
                let removed = state.lists.lock().unwrap().bans.remove(&user);
                let key = if removed {
                    "commands.pardon.success"
                } else {
                    "commands.pardon.failed"
                };
                self.console_reply(Text::translate(key, &[lit(&user.name)]));
            }
            "pardon-ip" if !rest.is_empty() => {
                let ip = rest[0].to_owned();
                if ip.parse::<std::net::IpAddr>().is_err() {
                    self.console_reply(Text::translate("commands.pardonip.invalid", &[]));
                    return;
                }
                let removed = state.lists.lock().unwrap().ip_bans.remove(&ip);
                let key = if removed {
                    "commands.pardonip.success"
                } else {
                    "commands.pardonip.failed"
                };
                self.console_reply(Text::translate(key, &[lit(&ip)]));
            }
            "banlist" => {
                let l = state.lists.lock().unwrap();
                let mut lines = Vec::new();
                let which = rest.first().copied().unwrap_or("");
                if which != "ips" {
                    for b in l.bans.entries() {
                        lines.push((b.user.name.clone(), b.source.clone(), b.reason.clone()));
                    }
                }
                if which != "players" {
                    for b in l.ip_bans.entries() {
                        lines.push((b.user.clone(), b.source.clone(), b.reason.clone()));
                    }
                }
                drop(l);
                if lines.is_empty() {
                    self.console_reply(Text::translate("commands.banlist.none", &[]));
                } else {
                    self.console_reply(Text::translate(
                        "commands.banlist.list",
                        &[lit(&lines.len().to_string())],
                    ));
                    for (name, source, reason) in lines {
                        self.console_reply(Text::translate(
                            "commands.banlist.entry",
                            &[
                                lit(&name),
                                lit(&source),
                                reason.as_deref().map_or_else(
                                    || {
                                        Text::translate(
                                            "multiplayer.disconnect.banned.reason.default",
                                            &[],
                                        )
                                    },
                                    lit,
                                ),
                            ],
                        ));
                    }
                }
            }
            "kick" if !rest.is_empty() => {
                let user = self.resolve(rest[0]);
                let online = self
                    .lan
                    .as_ref()
                    .is_some_and(|h| h.hub.guests.iter().any(|g| g.uuid == user.id));
                if !online {
                    self.console_reply(Text::translate("argument.entity.notfound.player", &[]));
                    return;
                }
                let reason = if rest.len() > 1 {
                    Tag::String(JavaString::from(rest[1..].join(" ").as_str()))
                } else {
                    translatable("multiplayer.disconnect.kicked", &[])
                };
                self.kick_uuid(user.id, reason);
                self.console_reply(Text::translate(
                    "commands.kick.success",
                    &[
                        lit(&user.name),
                        if rest.len() > 1 {
                            lit(&rest[1..].join(" "))
                        } else {
                            Text::translate("multiplayer.disconnect.kicked", &[])
                        },
                    ],
                ));
            }
            "say" if !rest.is_empty() => {
                // SayCommand from the console: an unsigned system chat
                // message of type say_command, logged as not secure.
                let message = rest.join(" ");
                log("INFO", &format!("[Not Secure] [Server] {message}"));
                if let Some(h) = &self.lan {
                    h.hub.say("Server", &message);
                }
            }
            _ => {
                // The game's dispatcher, captured for the console.
                self.chat_capture = Some(Vec::new());
                self.run_command(line);
                for text in self.chat_capture.take().unwrap_or_default() {
                    self.console_reply(text);
                }
            }
        }
    }

    fn console_whitelist(&mut self, state: &mut ServerState, rest: &[&str]) {
        let lit = |s: &str| Text::literal(s.to_owned());
        match rest {
            ["on"] | ["off"] => {
                let on = rest[0] == "on";
                let mut l = state.lists.lock().unwrap();
                let key = match (on, l.use_whitelist) {
                    (true, true) => "commands.whitelist.alreadyOn",
                    (true, false) => "commands.whitelist.enabled",
                    (false, false) => "commands.whitelist.alreadyOff",
                    (false, true) => "commands.whitelist.disabled",
                };
                l.use_whitelist = on;
                drop(l);
                self.store_property("white-list", on.to_string());
                self.console_reply(Text::translate(key, &[]));
            }
            ["list"] => {
                let names: Vec<String> = state
                    .lists
                    .lock()
                    .unwrap()
                    .white
                    .entries()
                    .iter()
                    .map(|e| e.user.name.clone())
                    .collect();
                if names.is_empty() {
                    self.console_reply(Text::translate("commands.whitelist.none", &[]));
                } else {
                    self.console_reply(Text::translate(
                        "commands.whitelist.list",
                        &[lit(&names.len().to_string()), lit(&names.join(", "))],
                    ));
                }
            }
            ["reload"] => {
                let _ = state.lists.lock().unwrap().white.load();
                self.console_reply(Text::translate("commands.whitelist.reloaded", &[]));
            }
            ["add", name] | ["remove", name] => {
                let user = self.resolve(name);
                let mut l = state.lists.lock().unwrap();
                let (ok, key) = if rest[0] == "add" {
                    if l.white.contains(&user) {
                        (false, "commands.whitelist.add.failed")
                    } else {
                        l.white.add(WhiteEntry { user: user.clone() });
                        (true, "commands.whitelist.add.success")
                    }
                } else if l.white.remove(&user) {
                    (true, "commands.whitelist.remove.success")
                } else {
                    (false, "commands.whitelist.remove.failed")
                };
                drop(l);
                let args = if ok {
                    vec![lit(&user.name)]
                } else {
                    Vec::new()
                };
                self.console_reply(Text::translate(key, &args));
            }
            _ => self.console_reply(Text::translate("command.unknown.argument", &[])),
        }
    }

    /// DedicatedServerSettings.update: change one key and store the file.
    fn store_property(&mut self, key: &str, value: String) {
        let path = PathBuf::from("server.properties");
        if let Ok(p) = ServerProperties::from_file(&path, &mut SystemEntropy::default())
            && let Ok(p) = p.update(key, value)
        {
            let _ = p.store(&path);
        }
    }

    fn kick_uuid(&mut self, uuid: u128, reason: Tag) {
        if let Some(h) = self.lan.as_mut() {
            let conns: Vec<u32> = h
                .hub
                .guests
                .iter()
                .filter(|g| g.uuid == uuid)
                .map(|g| g.conn)
                .collect();
            for conn in conns {
                h.hub.kick(conn, reason.clone());
            }
        }
    }

    /// MinecraftServer.stopServer: players saved and sent away, worlds saved.
    /// MinecraftServer.saveAllChunks, not silent: one line per level, in
    /// the server's level order.
    fn log_saved_levels(&self) {
        for dimension in [
            "minecraft:overworld",
            "minecraft:the_nether",
            "minecraft:the_end",
        ] {
            log(
                "INFO",
                &format!(
                    "Saving chunks for level 'ServerLevel[{}]'/{dimension}",
                    self.level.name
                ),
            );
        }
    }

    fn stop_dedicated(&mut self) -> Result<()> {
        log("INFO", "Stopping server");
        log("INFO", "Saving players");
        if let Some(h) = self.lan.as_mut() {
            for g in &h.hub.guests {
                horizon_client::lan::playerdata::save(g, &self.world_root);
            }
            let conns: Vec<u32> = h.hub.guests.iter().map(|g| g.conn).collect();
            for conn in conns {
                h.hub.kick(
                    conn,
                    translatable("multiplayer.disconnect.server_shutdown", &[]),
                );
            }
        }
        std::thread::sleep(Duration::from_millis(100));
        log("INFO", "Saving worlds");
        self.save_game()?;
        self.log_saved_levels();
        self.lan_close(false);
        Ok(())
    }
}

/// The width and height of a PNG (its IHDR chunk).
fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some((w, h))
}
