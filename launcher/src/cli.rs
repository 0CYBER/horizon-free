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
    /// Install a pack and what it needs, by its id or slug (checksums checked); a modpack is
    /// imported on the Modpacks page
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
    Resource,
    Shader,
    Modpack,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum SourceArg {
    Modrinth,
    Curseforge,
}

impl SourceArg {
    fn get(self) -> Result<Box<dyn horizon_launcher::browser::Source>, String> {
        use horizon_launcher::browser::{self, SourceId};
        let id = match self {
            SourceArg::Modrinth => SourceId::Modrinth,
            SourceArg::Curseforge => SourceId::CurseForge,
        };
        browser::source(id).ok_or_else(|| {
            "CurseForge needs an API key: HORIZON_CURSEFORGE_API_KEY (docs/mod-browser.md).".into()
        })
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
    /// Show or set the game's menu style (classic is vanilla's own screens)
    MenuStyle {
        /// Omit to show the current style
        #[arg(value_parser = ["horizon", "classic"])]
        style: Option<String>,
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
    /// The mod browser: resource packs, shader packs and modpacks from Modrinth (and CurseForge
    /// with a key), each file checked against its checksum (docs/mod-browser.md)
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
    /// Serve the Model Context Protocol on stdin and stdout, for AI agents
    Mcp,
    /// Measure the launcher: starts (the first is the cold one), page switching, idle CPU and
    /// memory, the release list fetch; writes a benchmark report to benchmarks/ in its data folder
    Benchmark {
        /// How many times to start it
        #[arg(long, default_value_t = 5)]
        repeats: usize,
    },
}

/// Runs a subcommand and returns the exit code, or None when the window should open.
pub fn run() -> Option<i32> {
    let cli = Cli::parse();
    let command = cli.command?;
    let result = match command {
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
        "tag": r.tag, "name": r.name, "date": r.date, "channel": if r.beta { "beta" } else { "stable" },
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
                let channel = if r.beta { "beta" } else { "stable" };
                println!("{:<15} {}  {:<6}  {}", r.tag, r.date, channel, state);
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
        Cmd::Play { version, join, arg } => {
            let mut cmd = backend::client(&backend::resolve_version(version.as_deref())?)?;
            if let Some(code) = join {
                backend::join_friend(&mut cmd, &code)?;
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
        Cmd::MenuStyle { style } => {
            if let Some(style) = style.as_deref().and_then(backend::MenuStyle::parse) {
                backend::set_menu_style(style)?;
            }
            let style = backend::menu_style().name();
            if as_json {
                json(serde_json::json!({ "menu_style": style }));
            } else {
                println!("Menu style: {style}");
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
                None => println!("Server: none yet, nothing is sent"),
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
                KindArg::Shader => Kind::ShaderPack,
                KindArg::Modpack => Kind::Modpack,
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
                    println!(
                        "{}  ({})  {} · {} downloads\n  {}",
                        p.title,
                        p.slug,
                        support.label(),
                        p.downloads,
                        p.summary
                    );
                    if let Some(why) = support.reason() {
                        println!("  {why}");
                    }
                }
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
                let pack = modpacks::import(&file, log)?;
                println!(
                    "Imported {} ({}): horizon-launcher modpack port {}",
                    pack.title(),
                    pack.summary(),
                    pack.slug
                );
                return Ok(());
            }
            let done = installed::install(&*source, &p, everything, &home, &line)?;
            if as_json {
                json(done.iter().map(entry_json).collect());
            }
            for e in done.iter().filter(|_| !as_json) {
                println!("{} {}: {} ({})", e.title, e.number, e.file, e.checksum);
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
