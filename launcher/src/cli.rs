//! `horizon-launcher <subcommand>`: the same steps as the window, from a terminal. Every
//! command goes through `backend`; this file only parses arguments and prints.
use clap::{Parser, Subcommand};
use horizon_launcher::account::{self, Accounts};
use horizon_launcher::backend::{self, Release};
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::Command;

#[derive(Parser)]
#[command(
    name = "horizon-launcher",
    version = backend::VERSION,
    about = "Installs, prepares and starts Horizon. Without a subcommand, opens the window."
)]
struct Cli {
    /// Print JSON instead of text, where a command has something to report
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum ServersCmd {
    /// The servers, their folder and whether they run
    List,
    /// Create a server; with no options it is a new world, survival, normal, 10 players, friends only
    Create {
        #[arg(long, default_value = "My server")]
        name: String,
        /// The new world's seed (default: random)
        #[arg(long, conflicts_with = "copy_world")]
        seed: Option<String>,
        /// Start from a copy of this world folder (one of your saves)
        #[arg(long)]
        copy_world: Option<PathBuf>,
        #[arg(long, default_value = "survival", value_parser = ["survival", "creative", "adventure", "spectator"])]
        mode: String,
        #[arg(long, default_value = "normal", value_parser = ["peaceful", "easy", "normal", "hard"])]
        difficulty: String,
        #[arg(long, default_value_t = 10)]
        max_players: u32,
        /// Let anyone in (no whitelist)
        #[arg(long)]
        open: bool,
        /// Check players' Minecraft accounts (they must sign in with Microsoft)
        #[arg(long)]
        check_accounts: bool,
        /// Who may join (repeatable); the first becomes an operator
        #[arg(long = "player")]
        players: Vec<String>,
        /// The installed version whose programs and game files servers use, if none are set up yet
        #[arg(long)]
        version: Option<String>,
        /// Agree to the Minecraft EULA (https://aka.ms/MinecraftEULA)
        #[arg(long)]
        accept_eula: bool,
    },
    /// Copy an installed, prepared version's programs and game files for the servers (again)
    SetUp {
        #[arg(long)]
        version: Option<String>,
    },
    /// Run a horizon-server command on a server, for example: servers run "My server" -- status
    Run {
        /// The server's name or folder
        server: String,
        #[arg(last = true, required = true)]
        args: Vec<String>,
    },
    /// Take a server off the list (its folder stays)
    Forget { server: String },
}

#[derive(Subcommand)]
enum AccountCmd {
    /// The signed-in accounts and which one plays
    List,
    /// Sign in with a Microsoft account in your browser (device code)
    SignIn,
    /// Sign out: the account's tokens leave the keychain
    SignOut {
        /// Its name or UUID
        who: String,
    },
    /// Play as this account
    Select { who: String },
    /// Play offline, without an account
    Offline,
}

#[derive(Subcommand)]
enum DataCmd {
    /// What is allowed, the install ID and how many items wait (the default)
    Status,
    /// Turn kinds on: performance, crashes, bugs, benchmarks, or all
    Allow {
        #[arg(required = true)]
        kinds: Vec<String>,
    },
    /// Turn kinds off: performance, crashes, bugs, benchmarks, or all
    Deny {
        #[arg(required = true)]
        kinds: Vec<String>,
    },
    /// Print exactly what would be sent
    Show,
    /// Delete everything sent from here, and what waits, and take a new install ID
    Delete,
    /// Send what is waiting now
    Send,
    /// Send a bug report you write; it is printed first, as it would leave
    Bug {
        text: String,
        /// Add the end of the game's log (scrubbed, without chat)
        #[arg(long)]
        log: bool,
        /// Send without asking
        #[arg(long)]
        yes: bool,
    },
    /// Upload a benchmark report (docs/benchmark-report.md); it is printed first
    Benchmark {
        file: PathBuf,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum ModpackCmd {
    /// The imported packs and their coverage (the default)
    List,
    /// Import a .mrpack, a CurseForge zip or an instance folder, and write its coverage report
    Import { path: PathBuf },
    /// Get the jars of what this computer can port and carry them over
    Port {
        /// The pack's folder name, as list shows it
        pack: String,
        /// A folder with your own jars (an installed instance, a downloads folder)
        #[arg(long)]
        from: Option<PathBuf>,
        /// The installed version the ports are for (default: the selected one)
        #[arg(long)]
        version: Option<String>,
    },
    /// Play a ported pack as its own profile
    Play {
        pack: String,
        #[arg(long)]
        version: Option<String>,
    },
    /// Remove an imported pack: its jars, ports and the profile's saves
    Remove { pack: String },
}

#[derive(Subcommand)]
enum ModsCmd {
    /// What the browser installed (the default)
    List,
    /// Search for packs Horizon loads
    Search {
        text: Vec<String>,
        #[arg(long, value_enum, default_value_t = KindArg::Resource)]
        kind: KindArg,
        /// Also what was made for other versions or runs only partly, each marked
        #[arg(long)]
        everything: bool,
        #[arg(long, value_enum, default_value_t = SourceArg::Modrinth)]
        source: SourceArg,
    },
    /// Say how well a Java mod runs in Horizon: its jar is downloaded (checksum checked),
    /// carried over and read; the answer is kept
    Check {
        project: String,
        /// Allow a version made for another Minecraft version (1.21 and later)
        #[arg(long)]
        everything: bool,
        #[arg(long, value_enum, default_value_t = SourceArg::Modrinth)]
        source: SourceArg,
    },
    /// Install a pack and what it needs, by its id or slug (checksums checked); a Java mod is
    /// ported into horizon-mods and turned on; a modpack is imported
    Install {
        project: String,
        /// Allow a version made for another Minecraft version
        #[arg(long)]
        everything: bool,
        #[arg(long, value_enum, default_value_t = SourceArg::Modrinth)]
        source: SourceArg,
    },
    /// Update what the browser installed (or one pack)
    Update {
        project: Option<String>,
        /// Only say what is newer
        #[arg(long)]
        check: bool,
    },
    /// Remove a pack the browser installed
    Remove { project: String },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum KindArg {
    Mod,
    Resource,
    Modpack,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum SourceArg {
    Modrinth,
    /// CurseForge support is coming soon
    Curseforge,
}

impl SourceArg {
    fn get(self) -> Result<Box<dyn horizon_launcher::browser::Source>, String> {
        use horizon_launcher::browser::{self, SourceId};
        let id = match self {
            SourceArg::Modrinth => SourceId::Modrinth,
            SourceArg::Curseforge => SourceId::CurseForge,
        };
        browser::source(id).ok_or_else(|| browser::CURSEFORGE_SOON.into())
    }
}

#[derive(Subcommand)]
enum Cmd {
    /// List Horizon releases, or the installed versions
    Versions {
        /// Only the versions installed here (no network)
        #[arg(long)]
        installed: bool,
    },
    /// Download and install a release (a tag such as v2026.10.13, or "latest")
    Install { tag: String },
    /// Remove an installed version
    Remove { tag: String },
    /// Prepare a version's resources from your own Minecraft install
    Prepare {
        /// The game folder of the official launcher, Prism Launcher, MultiMC, the Modrinth App,
        /// ATLauncher or GDLauncher (default: the launcher's, then every launcher's default)
        #[arg(long)]
        minecraft: Option<PathBuf>,
        /// The installed version (default: the selected one)
        #[arg(long)]
        version: Option<String>,
    },
    /// Start the client
    Play {
        #[arg(long)]
        version: Option<String>,
        /// Open or create this world (a folder under saves) at once
        #[arg(long)]
        world: Option<String>,
        /// Creative mode for a new world
        #[arg(long)]
        creative: bool,
        /// Join a friend's world with its invite code
        #[arg(long)]
        join: Option<String>,
        /// An extra argument for the game, passed through as-is (repeatable), for example
        /// --server-thread
        #[arg(long = "arg", value_name = "ARG", allow_hyphen_values = true)]
        arg: Vec<String>,
    },
    /// Run a dedicated server here; type commands into its console
    Server {
        /// The server's folder (default: the one the launcher uses)
        #[arg(long)]
        dir: Option<PathBuf>,
        #[arg(long)]
        version: Option<String>,
        /// Agree to the Minecraft EULA (https://aka.ms/MinecraftEULA)
        #[arg(long)]
        accept_eula: bool,
    },
    /// Show a release's patch notes (default: the newest)
    Notes { tag: Option<String> },
    /// Show the Discord, sponsor and website links
    Links,
    /// Microsoft accounts: list them (the default), sign in, sign out, or pick the one that plays
    Account {
        #[command(subcommand)]
        action: Option<AccountCmd>,
    },
    /// Show the launcher's settings, or set one (the Settings page): selected-version TAG,
    /// minecraft-dir PATH, server-dir PATH ("default" resets a folder), install-new-versions
    /// on|off, opening-animation on|off
    Settings {
        #[arg(value_parser = SETTING_KEYS)]
        key: Option<String>,
        /// The new value (with KEY)
        #[arg(requires = "key")]
        value: Option<String>,
    },
    /// Show or set the extra arguments the game starts with (for example --server-thread);
    /// quotes group a word with spaces in it
    GameArgs {
        /// Omit to show the current arguments
        #[arg(allow_hyphen_values = true)]
        args: Option<String>,
    },
    /// Print the end of the game's log (logs/latest.log in the game folder), or its newest crash
    /// report
    Logs {
        /// How many lines of latest.log
        #[arg(long, default_value_t = 100)]
        lines: usize,
        /// The newest crash report instead
        #[arg(long)]
        crash: bool,
    },
    /// Look for a newer Horizon release and launcher, and install them
    Update {
        /// Only report what is available
        #[arg(long)]
        check: bool,
    },
    /// Linux: put Horizon in the applications menu (an entry and the icon, under
    /// $XDG_DATA_HOME), or take it out with --remove
    DesktopEntry {
        /// Remove the entry and the icon, and stop adding them back
        #[arg(long)]
        remove: bool,
    },
    /// Opt-in data collection (docs/telemetry.md): what is allowed, what would be sent, and
    /// deleting it; all off until you allow a kind
    Data {
        #[command(subcommand)]
        action: Option<DataCmd>,
    },
    /// Modpacks: import a Java pack, see its coverage report, port what this computer can and
    /// play it as its own profile (docs/modpacks.md)
    Modpack {
        #[command(subcommand)]
        action: Option<ModpackCmd>,
    },
    /// The mod browser: Java mods, resource packs and modpacks from Modrinth
    /// (CurseForge support is coming soon), each file checked against its checksum
    /// (docs/mod-browser.md)
    Mods {
        #[command(subcommand)]
        action: Option<ModsCmd>,
    },
    /// Servers hosted with horizon-server (the Servers page): list them, create one, or run any
    /// horizon-server command on one
    Servers {
        #[command(subcommand)]
        action: Option<ServersCmd>,
    },
    /// Serve the Model Context Protocol on stdin and stdout, for AI agents (refused while No AI
    /// is on)
    Mcp,
    /// How to connect an AI agent (Claude Code, Codex, Cursor, any MCP client) to `mcp`: the
    /// config to paste and the command that adds it (refused while No AI is on)
    AgentConfig {
        /// Only this client
        #[arg(long, value_parser = ["claude-code", "codex", "cursor", "generic"])]
        client: Option<String>,
        /// Start the MCP server as a client would and say whether it answered
        #[arg(long)]
        test: bool,
    },
    /// Show or set No AI: on hides and turns off every AI feature (the companion, agent control,
    /// horizon-ctl and both MCP servers); off, the default, makes them available
    NoAi {
        /// Omit to show the current state
        #[arg(value_parser = ["on", "off"])]
        state: Option<String>,
    },
    /// Measure the launcher: starts (the first is the cold one), page switching, idle CPU and
    /// memory, the release list fetch; writes a benchmark report to benchmarks/ in its data folder
    Benchmark {
        /// How many times to start it
        #[arg(long, default_value_t = 5)]
        repeats: usize,
    },
}

/// The CLI's definition, for the MCP tools generated from it (cli_tools.rs).
pub fn command() -> clap::Command {
    <Cli as clap::CommandFactory>::command()
}

/// The commands that exist for AI agents: hidden and refused while No AI is on. Every other
/// command stays: the launcher, packagers and scripts need them, and none drives the game.
pub const AI_COMMANDS: [&str; 2] = ["mcp", "agent-config"];

/// The keys `settings` sets.
const SETTING_KEYS: [&str; 5] = [
    "selected-version",
    "minecraft-dir",
    "server-dir",
    "install-new-versions",
    "opening-animation",
];

fn on_off(value: &str) -> Result<bool, String> {
    match value {
        "on" | "true" => Ok(true),
        "off" | "false" => Ok(false),
        _ => Err(format!("{value}: say on or off")),
    }
}

fn set_setting(s: &mut backend::Settings, key: &str, value: &str) -> Result<(), String> {
    let folder = |value: &str| (value != "default").then(|| PathBuf::from(value));
    match key {
        "selected-version" => s.selected = Some(backend::resolve_version(Some(value))?),
        "minecraft-dir" => s.minecraft_dir = folder(value),
        "server-dir" => s.server_dir = folder(value),
        "install-new-versions" => s.install_new_versions = Some(on_off(value)?),
        "opening-animation" => s.skip_opening = !on_off(value)?,
        _ => return Err(format!("No setting {key}.")),
    }
    Ok(())
}

/// Every launcher setting, as `settings` shows them.
fn settings_json(s: &backend::Settings) -> serde_json::Value {
    serde_json::json!({
        "selected_version": backend::resolve_version(None).ok(),
        "minecraft_dir": s.minecraft_dir,
        "server_dir": backend::server_dir(None),
        "install_new_versions": s.install_new_versions.unwrap_or(true),
        "opening_animation": !s.skip_opening,
        "game_args": s.game_args,
        "desktop_entry": s.desktop_entry.unwrap_or(true),
        "no_ai": backend::no_ai(),
    })
}

/// Runs a subcommand and returns the exit code, or None when the window should open.
pub fn run() -> Option<i32> {
    use clap::FromArgMatches;
    let no_ai = backend::no_ai();
    let mut definition = command();
    if no_ai {
        // Hidden from the help too; a call still gets the refusal below.
        for name in AI_COMMANDS {
            definition = definition.mut_subcommand(name, |c| c.hide(true));
        }
    }
    let cli = Cli::from_arg_matches(&definition.get_matches()).unwrap_or_else(|e| e.exit());
    let command = cli.command?;
    let result = match command {
        Cmd::Mcp | Cmd::AgentConfig { .. } if no_ai => Err(horizon_core::no_ai::REFUSAL.into()),
        Cmd::Mcp => crate::mcp::serve(),
        command => execute(command, cli.json),
    };
    Some(match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    })
}

/// Download progress on one terminal line, only when stderr is a terminal and the percentage moves.
fn progress(label: String) -> impl Fn(f32) {
    let last = std::cell::Cell::new(-1);
    let tty = std::io::stderr().is_terminal();
    move |p| {
        let pct = (p * 100.) as i32;
        if !tty || pct == last.replace(pct) {
            return;
        }
        eprint!("\r{label} {pct:>3}%");
        let _ = std::io::stderr().flush();
        if p >= 1.0 {
            eprintln!();
        }
    }
}

fn json(value: serde_json::Value) {
    println!("{}", serde_json::to_string_pretty(&value).unwrap());
}

/// Runs a game process in this terminal and reports how it ended.
fn attached(mut cmd: Command) -> Result<(), String> {
    let status = cmd
        .status()
        .map_err(|e| format!("Could not start {:?}: {e}", cmd.get_program()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Exited with {status}"))
    }
}

fn release_json(r: &Release) -> serde_json::Value {
    serde_json::json!({
        "tag": r.tag, "name": r.name, "date": r.date, "prerelease": r.prerelease,
        "available_here": r.asset.is_some(), "installed": backend::installed().contains(&r.tag),
    })
}

fn execute(command: Cmd, as_json: bool) -> Result<(), String> {
    if matches!(command, Cmd::Versions { .. } | Cmd::Update { .. }) {
        for (tag, from) in backend::reuse_prepared_everywhere() {
            if !as_json {
                eprintln!("Prepared {tag} with the resources of {from}.");
            }
        }
    }
    match command {
        Cmd::Versions { installed: true } => {
            let installed = backend::installed();
            let selected = backend::resolve_version(None).ok();
            if as_json {
                json(serde_json::json!({ "installed": installed, "selected": selected }));
            } else if installed.is_empty() {
                println!("No version is installed.");
            }
            for tag in installed.iter().filter(|_| !as_json) {
                let mark = if selected.as_ref() == Some(tag) {
                    "*"
                } else {
                    " "
                };
                let state = if !backend::is_prepared(tag) {
                    "not prepared"
                } else if backend::has_sounds(tag) {
                    "prepared"
                } else {
                    "prepared, no sounds (prepare again)"
                };
                println!("{mark} {tag}  {state}");
            }
        }
        Cmd::Versions { installed: false } => {
            let releases = backend::releases()?;
            if as_json {
                json(releases.iter().map(release_json).collect());
                return Ok(());
            }
            let installed = backend::installed();
            for r in &releases {
                let state = match (installed.contains(&r.tag), r.asset.is_some()) {
                    (true, _) => "installed",
                    (false, true) => "",
                    (false, false) => "no build for this platform",
                };
                println!("{:<15} {}  {}", r.tag, r.date, state);
            }
        }
        Cmd::Install { tag } => {
            let label = format!("Installing {tag}");
            let tag = backend::install_tag(&tag, false, progress(label))?;
            if as_json {
                json(
                    serde_json::json!({ "installed": tag, "prepared": backend::is_prepared(&tag) }),
                );
            } else if backend::is_prepared(&tag) {
                println!(
                    "Installed {tag}. It is prepared already, from another installed version."
                );
            } else {
                println!(
                    "Installed {tag}. Prepare it next: horizon-launcher prepare --version {tag}"
                );
            }
        }
        Cmd::Remove { tag } => {
            backend::remove_version(&tag)?;
            println!("Removed {tag}.");
        }
        Cmd::Prepare { minecraft, version } => {
            let tag = backend::resolve_version(version.as_deref())?;
            let minecraft = backend::minecraft_dir(minecraft)?;
            let (preparer, minecraft) =
                backend::preparer(&tag, &minecraft, &|line| eprintln!("{line}"))?;
            backend::sounds(&tag, &minecraft, &|line| println!("{line}"));
            match preparer {
                backend::Preparer::Process(cmd) => attached(cmd)?,
                backend::Preparer::InProcess(options) => {
                    backend::prepare_in_process(&options, &|line| println!("{line}"))?
                }
            }
            println!("{tag} is prepared.");
        }
        Cmd::Play {
            version,
            world,
            creative,
            join,
            arg,
        } => {
            let mut cmd = backend::client(&backend::resolve_version(version.as_deref())?)?;
            if let Some(code) = join {
                backend::join_friend(&mut cmd, &code)?;
            }
            if let Some(world) = world {
                cmd.arg("--world").arg(world);
            }
            if creative {
                cmd.arg("--creative");
            }
            cmd.args(arg);
            attached(cmd)?
        }
        Cmd::Server {
            dir,
            version,
            accept_eula,
        } => {
            let tag = backend::resolve_version(version.as_deref())?;
            let dir = backend::server_dir(dir);
            // The server inherits this terminal, so typed lines reach its console.
            attached(backend::server(&tag, &dir, accept_eula)?)?
        }
        Cmd::Notes { tag } => {
            let r = backend::release_notes(tag.as_deref())?;
            if as_json {
                json(
                    serde_json::json!({ "tag": r.tag, "name": r.name, "date": r.date, "notes": r.body }),
                );
            } else {
                let title = if r.name.is_empty() { &r.tag } else { &r.name };
                println!(
                    "{title}\n{}  {}\n\n{}",
                    r.tag,
                    r.date,
                    backend::markdown_to_text(&r.body)
                );
            }
        }
        Cmd::Account { action } => account_command(action.unwrap_or(AccountCmd::List), as_json)?,
        Cmd::Benchmark { repeats } => {
            let path = crate::perf::benchmark(repeats.max(1))?;
            if as_json {
                json(serde_json::json!({ "report": path }));
            } else {
                println!("{}", path.display());
            }
        }
        Cmd::Links => {
            let links = backend::links();
            if as_json {
                json(serde_json::to_value(&links).unwrap());
            } else {
                for (name, url) in [
                    ("Discord", &links.discord),
                    ("Sponsor", &links.sponsor),
                    ("Website", &links.website),
                ] {
                    println!("{name:<8} {}", if url.is_empty() { "(none)" } else { url });
                }
            }
        }
        Cmd::NoAi { state } => {
            if let Some(state) = state {
                backend::set_no_ai(state == "on")?;
            }
            let on = backend::no_ai();
            if as_json {
                json(serde_json::json!({ "no_ai": on }));
            } else if on {
                println!("No AI is on: every AI feature is hidden and off.");
            } else {
                println!("No AI is off: AI features are available.");
            }
        }
        Cmd::AgentConfig { test: true, .. } => {
            let tools = horizon_launcher::agents::self_test(backend::exe_path())?;
            if as_json {
                json(serde_json::json!({ "answered": true, "tools": tools }));
            } else {
                println!("The MCP server answered with {tools} tools.");
            }
        }
        Cmd::AgentConfig { client, .. } => {
            use horizon_launcher::agents;
            let exe = backend::exe_path();
            if as_json {
                let mut all = agents::json(exe);
                if let Some(id) = &client {
                    all["clients"] = all["clients"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|c| c["id"] == id.as_str())
                        .cloned()
                        .collect();
                }
                json(all);
                return Ok(());
            }
            for c in agents::clients(exe)
                .iter()
                .filter(|c| client.as_ref().is_none_or(|id| id == c.id))
            {
                println!("{}", c.name);
                if let Some(line) = c.add_line() {
                    println!("  Add it with:  {line}");
                }
                println!("  Or put this in {}:", c.file);
                for line in c.config.lines() {
                    println!("    {line}");
                }
                println!();
            }
            println!("Then ask your agent to say hello: it can call the links tool, or play.");
        }
        Cmd::Settings { key, value } => {
            let mut settings = backend::Settings::load();
            if let Some(key) = key {
                let value = value.ok_or("settings KEY VALUE: the value is missing")?;
                set_setting(&mut settings, &key, &value)?;
                settings.save();
            }
            let all = settings_json(&settings);
            if as_json {
                json(all);
            } else {
                for (key, value) in all.as_object().into_iter().flatten() {
                    println!("{key}: {value}");
                }
            }
        }
        Cmd::GameArgs { args } => {
            let mut settings = backend::Settings::load();
            if let Some(line) = args {
                settings.game_args = line;
                settings.save();
            }
            let line = settings.game_args;
            if as_json {
                json(serde_json::json!({
                    "game_args": line,
                    "parsed": backend::split_args(&line),
                }));
            } else if line.is_empty() {
                println!("No extra game arguments.");
            } else {
                println!("Game arguments: {line}");
            }
        }
        Cmd::Logs { lines, crash } => {
            let (path, text) = backend::read_logs(lines, crash)?;
            if as_json {
                json(serde_json::json!({ "path": path, "text": text }));
            } else {
                print!("{text}");
            }
        }
        Cmd::Update { check } => update(check, as_json)?,
        Cmd::Data { action } => data(action.unwrap_or(DataCmd::Status), as_json)?,
        Cmd::DesktopEntry { remove } => {
            let mut settings = backend::Settings::load();
            settings.desktop_entry = Some(!remove);
            settings.save();
            if remove {
                horizon_launcher::desktop::remove()?;
                println!("Removed Horizon from the applications menu.");
            } else {
                horizon_launcher::desktop::install(backend::exe_path())?;
                println!("Horizon is in the applications menu.");
            }
        }
        Cmd::Modpack { action } => modpack(action.unwrap_or(ModpackCmd::List), as_json)?,
        Cmd::Mods { action } => mods(action.unwrap_or(ModsCmd::List), as_json)?,
        Cmd::Servers { action } => servers(action.unwrap_or(ServersCmd::List), as_json)?,
        Cmd::Mcp => unreachable!(),
    }
    Ok(())
}

fn update(check: bool, as_json: bool) -> Result<(), String> {
    let updates = backend::check_updates()?;
    if as_json && check {
        json(serde_json::json!({
            "horizon": updates.horizon.as_ref().map(|r| &r.tag),
            "launcher": updates.launcher.as_ref().and_then(|r| backend::launcher_version(&r.tag)),
            "launcher_current": backend::VERSION,
        }));
        return Ok(());
    }
    match &updates.horizon {
        None => println!("Horizon: up to date."),
        Some(r) if check => println!("Horizon: {} is available.", r.tag),
        Some(r) => {
            let tag =
                backend::install_tag(&r.tag, true, progress(format!("Installing {}", r.tag)))?;
            println!("Horizon: installed {tag}.");
        }
    }
    match &updates.launcher {
        None => println!("Launcher: {} is up to date.", backend::VERSION),
        Some(r) => {
            let version = backend::launcher_version(&r.tag).unwrap_or_default();
            if check {
                println!(
                    "Launcher: {version} is available (this is {}).",
                    backend::VERSION
                );
            } else if !backend::self_update_allowed() {
                println!("Launcher: {version} is available; this build does not replace itself.");
            } else {
                backend::self_update(r, progress(format!("Updating the launcher to {version}")))?;
                println!("Launcher: updated to {version}. Restart the launcher to use it.");
            }
        }
    }
    Ok(())
}

/// `account …`: names and UUIDs only; tokens are never printed.
fn account_command(action: AccountCmd, as_json: bool) -> Result<(), String> {
    let mut accounts = Accounts::load();
    let find = |accounts: &Accounts, who: &str| {
        accounts
            .find(who)
            .cloned()
            .ok_or_else(|| format!("No signed-in account {who}. See `horizon-launcher account`."))
    };
    match action {
        AccountCmd::List => {
            if as_json {
                json(serde_json::json!({
                    "accounts": accounts.accounts.iter().map(|a| serde_json::json!({
                        "name": a.name, "uuid": a.uuid,
                        "selected": accounts.selected.as_ref() == Some(&a.uuid),
                    })).collect::<Vec<_>>(),
                    "sign_in_configured": account::client_id().is_some(),
                }));
            } else if accounts.accounts.is_empty() {
                println!("No Microsoft account signed in: the game plays offline.");
            } else {
                for a in &accounts.accounts {
                    let mark = if accounts.selected.as_ref() == Some(&a.uuid) {
                        "*"
                    } else {
                        " "
                    };
                    println!("{mark} {}  {}", a.name, a.uuid);
                }
                if accounts.selected().is_none() {
                    println!("Playing offline (`account select NAME` to play as one).");
                }
            }
        }
        AccountCmd::SignIn => {
            let client_id = account::client_id().ok_or(account::NOT_CONFIGURED)?;
            let ep = account::Endpoints::live();
            let code = account::device_code(&ep, &client_id)?;
            eprintln!(
                "To sign in, open {} in a browser and enter the code {}",
                code.verification_uri, code.user_code
            );
            let ms = account::wait_for_sign_in(&ep, &client_id, &code, &|| false)?;
            let (signed_in, secrets) = account::finish_sign_in(&ep, ms)?;
            accounts.add(account::store(), signed_in.clone(), &secrets)?;
            accounts.save()?;
            println!("Signed in as {}.", signed_in.name);
            if !account::store().persistent() {
                eprintln!("{}", account::NO_KEYCHAIN);
            }
        }
        AccountCmd::SignOut { who } => {
            let a = find(&accounts, &who)?;
            accounts.remove(account::store(), &a.uuid)?;
            accounts.save()?;
            println!("Signed out {}.", a.name);
        }
        AccountCmd::Select { who } => {
            let a = find(&accounts, &who)?;
            accounts.selected = Some(a.uuid);
            accounts.save()?;
            println!("The game plays as {}.", a.name);
        }
        AccountCmd::Offline => {
            accounts.selected = None;
            accounts.save()?;
            println!("The game plays offline.");
        }
    }
    Ok(())
}

/// `data`: the same consent as Settings → Data collection.
fn data(action: DataCmd, as_json: bool) -> Result<(), String> {
    use horizon_launcher::telemetry;
    let review = |item: &horizon_telemetry::Item, yes: bool| -> Result<bool, String> {
        println!(
            "{}",
            serde_json::to_string_pretty(item).map_err(|e| e.to_string())?
        );
        if yes {
            return Ok(true);
        }
        if !std::io::stdin().is_terminal() {
            return Err("Add --yes to send it without asking.".into());
        }
        eprint!("Send this? [y/N] ");
        let mut line = String::new();
        std::io::stdin()
            .read_line(&mut line)
            .map_err(|e| e.to_string())?;
        Ok(line.trim().eq_ignore_ascii_case("y"))
    };
    match action {
        DataCmd::Status => {
            let status = telemetry::status_json();
            if as_json {
                json(status);
                return Ok(());
            }
            let consent = telemetry::consent();
            if consent.needs_asking() {
                println!("Not answered yet: nothing is collected.");
            }
            for (kind, name, _) in telemetry::SWITCHES {
                let on = consent.allows(kind);
                println!("{name}: {}", if on { "on" } else { "off" });
            }
            println!("Waiting to be sent: {}", status["waiting"]);
            match horizon_telemetry::endpoint() {
                Some(url) => println!("Server: {url}"),
                None => println!("Server: turned off (HORIZON_TELEMETRY_URL=off), nothing is sent"),
            }
        }
        DataCmd::Allow { kinds } => {
            telemetry::set(&telemetry::parse_kinds(&kinds)?, true)?;
            return data(DataCmd::Status, as_json);
        }
        DataCmd::Deny { kinds } => {
            telemetry::set(&telemetry::parse_kinds(&kinds)?, false)?;
            return data(DataCmd::Status, as_json);
        }
        DataCmd::Show => {
            let (what, text) = telemetry::preview();
            eprintln!("{what}");
            println!("{text}");
        }
        DataCmd::Delete => println!("{}", telemetry::delete_my_data()?),
        DataCmd::Send => println!("{:?}", telemetry::send()),
        DataCmd::Bug { text, log, yes } => {
            telemetry::ensure_allowed(horizon_telemetry::Kind::Bug)?;
            let item = telemetry::bug_report(&text, log)?;
            if review(&item, yes)? {
                println!("{}", telemetry::submit(&item)?);
            }
        }
        DataCmd::Benchmark { file, yes } => {
            telemetry::ensure_allowed(horizon_telemetry::Kind::Benchmark)?;
            let item = telemetry::benchmark_report(&file)?;
            if review(&item, yes)? {
                println!("{}", telemetry::submit(&item)?);
            }
        }
    }
    Ok(())
}

fn servers(action: ServersCmd, as_json: bool) -> Result<(), String> {
    use horizon_launcher::servers::{self, NewServer, World};
    let find = |which: &str| {
        servers::list()
            .into_iter()
            .find(|e| e.name == which || e.dir == std::path::Path::new(which))
            .ok_or_else(|| {
                format!("No server called {which}: `horizon-launcher servers` lists them")
            })
    };
    match action {
        ServersCmd::List => {
            let list = servers::list();
            if as_json {
                json(serde_json::Value::Array(
                    list.iter()
                        .map(|e| {
                            serde_json::json!({
                                "name": e.name, "dir": e.dir,
                                "phase": servers::status(&e.dir).phase,
                            })
                        })
                        .collect(),
                ));
            } else if list.is_empty() {
                println!("No server yet: horizon-launcher servers create --accept-eula");
            }
            for e in list.iter().filter(|_| !as_json) {
                println!(
                    "{}  ({}): {}",
                    e.name,
                    e.dir.display(),
                    servers::status(&e.dir).phase
                );
            }
        }
        ServersCmd::Create {
            name,
            seed,
            copy_world,
            mode,
            difficulty,
            max_players,
            open,
            check_accounts,
            players,
            version,
            accept_eula,
        } => {
            let spec = NewServer {
                name,
                world: match copy_world {
                    Some(dir) => World::Copy(dir),
                    None => World::New {
                        seed: seed.unwrap_or_default(),
                    },
                },
                game_mode: mode,
                difficulty,
                max_players,
                friends_only: !open,
                check_accounts,
                players,
                eula: accept_eula,
            };
            let tag = backend::resolve_version(version.as_deref()).ok();
            let entry = servers::create(&spec, tag.as_deref(), &|l| eprintln!("{l}"))
                .map_err(|p| p.message)?;
            if as_json {
                json(serde_json::json!({ "name": entry.name, "dir": entry.dir }));
            } else {
                println!("Created {} in {}", entry.name, entry.dir.display());
            }
        }
        ServersCmd::SetUp { version } => {
            let tag = backend::resolve_version(version.as_deref())?;
            servers::set_up(&tag)?;
            println!(
                "Servers run Horizon {}",
                servers::runtime_release().unwrap_or(tag)
            );
        }
        ServersCmd::Run { server, args } => {
            let entry = find(&server)?;
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let status = servers::command(&entry.dir, &args)?
                .status()
                .map_err(|e| e.to_string())?;
            if !status.success() {
                return Err(format!("horizon-server exited with {status}"));
            }
        }
        ServersCmd::Forget { server } => {
            let entry = find(&server)?;
            servers::forget(&entry.dir)?;
            println!(
                "{} is off the list; its folder stays in {}",
                entry.name,
                entry.dir.display()
            );
        }
    }
    Ok(())
}

fn mods(action: ModsCmd, as_json: bool) -> Result<(), String> {
    use horizon_launcher::browser::{installed, Kind, Query};
    use horizon_launcher::modpacks;
    let home = backend::game_home();
    let entry_json = |e: &installed::Entry| serde_json::to_value(e).unwrap_or_default();
    let line = |label: &str, f: f32| progress(label.to_string())(f);
    match action {
        ModsCmd::List => {
            let list = installed::Installed::load(&home).entries;
            if as_json {
                json(list.iter().map(entry_json).collect());
            } else if list.is_empty() {
                println!(
                    "Nothing installed from the browser yet: horizon-launcher mods search <text>"
                );
            }
            for e in list.iter().filter(|_| !as_json) {
                println!(
                    "{}  {}  ({}, {})\n  {}  {}",
                    e.title,
                    e.number,
                    e.source.label(),
                    e.slug,
                    e.file,
                    e.checksum
                );
            }
        }
        ModsCmd::Search {
            text,
            kind,
            everything,
            source,
        } => {
            let kind = match kind {
                KindArg::Resource => Kind::ResourcePack,
                KindArg::Modpack => Kind::Modpack,
                KindArg::Mod => Kind::Mod,
            };
            let query = Query {
                everything,
                ..Query::new(&text.join(" "), kind)
            };
            let found = source.get()?.search(&query)?;
            if as_json {
                json(found.iter().map(|p| {
                    let support = p.support();
                    serde_json::json!({
                        "id": p.id, "slug": p.slug, "title": p.title, "summary": p.summary,
                        "author": p.author, "downloads": p.downloads, "page": p.page,
                        "license": p.license, "support": support.label(), "reason": support.reason(),
                    })
                }).collect());
            } else {
                for p in &found {
                    let support = p.support();
                    let unchecked = p.kind == Kind::Mod && support.installable();
                    println!(
                        "{}  ({})  {} · {} downloads\n  {}",
                        p.title,
                        p.slug,
                        if unchecked {
                            "Not checked"
                        } else {
                            support.label()
                        },
                        p.downloads,
                        p.summary
                    );
                    if unchecked {
                        println!("  horizon-launcher mods check {}", p.slug);
                    } else if let Some(why) = support.reason() {
                        println!("  {why}");
                    }
                }
            }
        }
        ModsCmd::Check {
            project,
            everything,
            source,
        } => {
            let source = source.get()?;
            let p = source.project(&project)?;
            if p.kind != Kind::Mod {
                return Err(format!("{} isn't a Java mod.", p.title));
            }
            let (version, c) = horizon_launcher::browser::java::check_project(
                &home,
                &*source,
                &p,
                everything,
                &progress(p.title.clone()),
            )?;
            if as_json {
                let mut v = serde_json::to_value(&c).unwrap_or_default();
                v["version"] = version.number.clone().into();
                v["label"] = c.verdict.label().into();
                println!("{v}");
                return Ok(());
            }
            println!(
                "{} {} ({} {}): {}. {}",
                p.title,
                version.number,
                c.loader,
                c.mod_id,
                c.verdict.label(),
                c.summary
            );
            for line in &c.ports {
                println!("  ports: {line}");
            }
            for line in &c.skipped {
                println!("  doesn't: {line}");
            }
        }
        ModsCmd::Install {
            project,
            everything,
            source,
        } => {
            let source = source.get()?;
            let p = source.project(&project)?;
            if p.kind == Kind::Modpack {
                let folder = modpacks::ports_dir().join("downloads");
                let file =
                    installed::fetch_modpack(&*source, &p, &folder, &progress(p.title.clone()))?;
                let log = &|line: String| eprintln!("{line}");
                let resources = backend::resolve_version(None)
                    .ok()
                    .filter(|t| backend::is_prepared(t))
                    .map(|t| backend::resources(&t));
                let pack = modpacks::import_and_port(&file, resources.as_deref(), log)?;
                println!("{} ({})", modpacks::outcome(&pack), pack.summary());
                println!("Play it: horizon-launcher modpack play {}", pack.slug);
                return Ok(());
            }
            let done = installed::install(&*source, &p, everything, &home, &line)?;
            if as_json {
                json(done.iter().map(entry_json).collect());
            }
            for e in done.iter().filter(|_| !as_json) {
                println!("{} {}: {} ({})", e.title, e.number, e.file, e.checksum);
                for s in &e.skipped {
                    println!("  not ported: {s}");
                }
            }
        }
        ModsCmd::Update { project, check } => {
            let list = installed::Installed::load(&home).entries;
            let chosen: Vec<_> = list
                .iter()
                .filter(|e| {
                    project
                        .as_deref()
                        .is_none_or(|n| e.project == n || e.slug == n)
                })
                .collect();
            if chosen.is_empty() {
                return Err(match project {
                    Some(n) => format!("{n} wasn't installed by the browser."),
                    None => "Nothing installed from the browser yet.".into(),
                });
            }
            for e in chosen {
                let Some(source) = horizon_launcher::browser::source(e.source) else {
                    println!("{}: {} isn't available here.", e.title, e.source.label());
                    continue;
                };
                match installed::update_for(&*source, e)? {
                    None => println!("{} {}: up to date.", e.title, e.number),
                    Some(v) if check => {
                        println!("{} {}: {} is available.", e.title, e.number, v.number)
                    }
                    Some(_) => {
                        let p = source.project(&e.project)?;
                        for done in installed::install(&*source, &p, true, &home, &line)? {
                            println!("{} {}: {}", done.title, done.number, done.file);
                        }
                    }
                }
            }
        }
        ModsCmd::Remove { project } => {
            let e = installed::remove(&home, &project)?;
            println!("Removed {} ({}).", e.title, e.file);
        }
    }
    Ok(())
}

fn modpack(action: ModpackCmd, as_json: bool) -> Result<(), String> {
    use horizon_launcher::modpacks::{self, Imported};
    use horizon_port_pack::report::Status;
    let log = &|line: String| eprintln!("{line}");
    let work = |pack: &str| {
        let work = modpacks::ports_dir().join(pack);
        modpacks::load(&work).map(|_| work)
    };
    let show = |p: &Imported| {
        let counts: Vec<String> = Status::ALL
            .iter()
            .map(|s| format!("{} {}", p.count(*s), s.label()))
            .collect();
        if as_json {
            json(serde_json::json!({
                "pack": p.slug, "name": p.title(), "summary": p.summary(),
                "counts": Status::ALL.iter().map(|s| (s.label(), p.count(*s))).collect::<std::collections::BTreeMap<_, _>>(),
                "portable": p.portable, "ported_mods": p.mods.len(), "jars": p.jars(),
                "report": p.work.join("coverage.md"),
            }));
        } else {
            println!(
                "{}  ({})\n  {}\n  {}",
                p.title(),
                p.slug,
                p.summary(),
                counts.join(", ")
            );
            println!("  {} portable, {} carried over", p.portable, p.mods.len());
            if let Some(jars) = p.jars() {
                println!("  {jars}");
            }
            println!("  {}", p.work.join("coverage.md").display());
        }
    };
    match action {
        ModpackCmd::List => {
            let packs = modpacks::list();
            if packs.is_empty() && !as_json {
                println!("No modpack imported yet: horizon-launcher modpack import <pack>");
            }
            packs.iter().for_each(show);
        }
        ModpackCmd::Import { path } => show(&modpacks::import(&path, log)?),
        ModpackCmd::Port {
            pack,
            from,
            version,
        } => {
            let tag = backend::resolve_version(version.as_deref()).ok();
            let resources = tag
                .filter(|t| backend::is_prepared(t))
                .map(|t| backend::resources(&t));
            show(&modpacks::port(
                &work(&pack)?,
                resources.as_deref(),
                from.as_deref(),
                log,
            )?)
        }
        ModpackCmd::Play { pack, version } => {
            let mut cmd = backend::client(&backend::resolve_version(version.as_deref())?)?;
            modpacks::apply_profile(&mut cmd, &work(&pack)?)?;
            attached(cmd)?
        }
        ModpackCmd::Remove { pack } => modpacks::remove(&work(&pack)?)?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn play_takes_extra_arguments_that_start_with_hyphens() {
        let cli = Cli::try_parse_from([
            "horizon-launcher",
            "play",
            "--arg",
            "--server-thread",
            "--arg",
            "40",
        ])
        .unwrap();
        match cli.command {
            Some(Cmd::Play { arg, .. }) => assert_eq!(arg, ["--server-thread", "40"]),
            _ => panic!("expected play"),
        }
    }

    #[test]
    fn game_args_takes_one_line_that_starts_with_hyphens() {
        let cli = Cli::try_parse_from([
            "horizon-launcher",
            "game-args",
            "--server-thread --render-distance 40",
        ])
        .unwrap();
        match cli.command {
            Some(Cmd::GameArgs { args }) => {
                assert_eq!(
                    args.as_deref(),
                    Some("--server-thread --render-distance 40")
                );
            }
            _ => panic!("expected game-args"),
        }
    }
}
