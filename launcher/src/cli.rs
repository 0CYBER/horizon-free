//! `horizon-launcher <subcommand>`: the same steps as the window, from a terminal. Every
//! command goes through `backend`; this file only parses arguments and prints.
use clap::{Parser, Subcommand};
use horizon_launcher::account::{self, Accounts};
use horizon_launcher::backend::{self, Release};
use horizon_launcher::github_mods;
use horizon_launcher::i18n::{tr, trf};
use horizon_launcher::instances;
use horizon_launcher::loaders;
use horizon_launcher::native_mods;
use horizon_launcher::other_launchers;
use horizon_launcher::server_list;
use horizon_launcher::storage::{Category, Storage};
#[derive(Clone, Debug)]
enum ClearStorage {
    All,
    Row(Category),
}
fn parse_storage_row(value: &str) -> Result<ClearStorage, String> {
    use clap::ValueEnum;
    if value == "all" {
        Ok(ClearStorage::All)
    } else {
        Category::ALL
            .into_iter()
            .find(|category| category.to_possible_value().unwrap().get_name() == value)
            .map(ClearStorage::Row)
            .ok_or_else(|| tr("Unknown").into())
    }
}
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
#[derive(Parser)]
#[command(
    name = crate::identity::EXECUTABLE,
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
enum JavaCmd {
    /// The Java runtimes Horizon downloaded from Mojang and the ones installed on this computer
    List,
    /// Download Mojang's Java runtime: a major (8, 16, 17, 21, 25) or a component name from a
    /// version's JSON (java-runtime-delta)
    Install { java: String },
}
#[derive(Subcommand)]
enum RustCmd {
    /// Whether it is installed, its version and whether this computer can link with it
    Status,
    /// Download the pinned toolchain from static.rust-lang.org (once), checked by SHA-256
    Install,
    /// Delete the toolchain and the native builds' Cargo home
    Remove,
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
        /// Do not check players' Minecraft accounts (offline mode: anyone can use any name)
        #[arg(long)]
        offline: bool,
        /// Kept for old scripts: account checking is now the default
        #[arg(long, hide = true, conflicts_with = "offline")]
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
    /// Sign in with a Microsoft account in your browser (device code). Play needs one that owns
    /// Minecraft: Java Edition
    SignIn {
        /// Sign this account (name or UUID) in again: another Microsoft account is refused and
        /// who plays stays the same
        #[arg(long = "for")]
        target: Option<String>,
    },
    /// Renew an account's session now, even when its access token is still fresh
    Refresh { who: String },
    /// Renew an account's session for a game already running: prints the read-once session file
    /// it hands over (`--json`: {"session_file"} or {"authentication"} when it can't)
    Session { who: String },
    /// Sign out: the account's tokens leave the keychain (and the secrets file, where it kept them)
    SignOut {
        /// Its name or UUID
        who: String,
    },
    /// Play as this account
    Select { who: String },
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
        /// Allow missing original Java asset/data jars to be downloaded from Modrinth
        #[arg(long)]
        allow_jar_downloads: bool,
        #[arg(long)]
        version: Option<String>,
    },
    /// Remove an imported pack: its jars, ports and the profile's saves
    Remove { pack: String },
}

#[derive(Subcommand)]
enum InstanceCmd {
    /// The instances, each with its game folder (* = the one Play starts)
    List,
    /// Make an instance: empty but for your options and name, or a copy of another one's
    /// worlds, options and packs
    Create {
        name: String,
        /// The instance to copy (its name or id)
        #[arg(long)]
        copy: Option<String>,
        /// Make a Minecraft: Java Edition instance of this version (1.21.4, latest or
        /// latest-snapshot) instead of a Horizon one
        #[arg(long, conflicts_with = "copy")]
        minecraft: Option<String>,
        /// With this mod loader: fabric, quilt, forge or neoforge
        #[arg(long, requires = "minecraft")]
        loader: Option<String>,
        /// The loader's version (default: its newest stable one)
        #[arg(long, requires = "loader")]
        loader_version: Option<String>,
    },
    /// The versions of Minecraft: Java Edition a Java instance can play, the newest first
    Versions {
        /// Snapshots too
        #[arg(long)]
        snapshots: bool,
        /// Old betas and alphas too
        #[arg(long)]
        old: bool,
    },
    /// A Java instance's Java: its own Java home or the automatic pick, memory and JVM arguments
    Java {
        instance: String,
        /// A Java home to run it on
        #[arg(long, conflicts_with = "auto")]
        home: Option<PathBuf>,
        /// Go back to the Java the version needs, picked or downloaded by the launcher
        #[arg(long)]
        auto: bool,
        /// The most memory the game may use, in MiB
        #[arg(long)]
        memory: Option<u32>,
        /// Extra JVM arguments (one string; "" clears them)
        #[arg(long, allow_hyphen_values = true)]
        args: Option<String>,
        /// The game window's size, as 1280x720; "default" goes back to the game's own
        #[arg(long)]
        window: Option<String>,
    },
    /// A Java instance's mod loader: fabric, quilt, forge or neoforge, or none. --list shows its versions
    Loader {
        instance: String,
        loader: String,
        /// The loader's version (default: its newest stable one)
        #[arg(long)]
        version: Option<String>,
        /// List the loader's versions for the instance's Minecraft instead
        #[arg(long, conflicts_with = "version")]
        list: bool,
    },
    /// Rename an instance (its folder stays)
    Rename { instance: String, name: String },
    /// Delete an instance: its folder (worlds, options, packs) goes to the Trash, where you can
    /// get it back. Horizon's own instance stays
    Delete { instance: String },
    /// Set an instance's icon: a block or item name from your prepared files (grass_block),
    /// a colour (#3fa7d6), or none
    Icon { instance: String, icon: String },
    /// Put an instance in a sidebar group, or take it out with no group
    Group {
        instance: String,
        group: Option<String>,
    },
    /// Make Play start this instance
    Select { instance: String },
    /// Keep an instance on one installed Horizon version, or "latest" to follow updates again
    Version { instance: String, version: String },
    /// Export an instance (or a port) to share: its settings, packs, Horizon mods and the worlds
    /// named, as a .mrpack (resource packs from Modrinth listed by download) or a zip. Never
    /// logs, screenshots or account data; worlds go without their per-player files
    Export {
        /// The instance (default: the selected one)
        instance: Option<String>,
        #[arg(long, value_enum, default_value_t = ExportFormat::Mrpack)]
        format: ExportFormat,
        /// A world to put in it (its name or folder); repeat for more
        #[arg(long)]
        world: Vec<String>,
        /// Where to write it (default: "<name>.mrpack" in the current folder)
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Show or set what runs around an instance's game: a shell command before it (the game
    /// starts only if it succeeds), a wrapper it runs through (`nice -n 5`), environment
    /// variables ("NAME=value NAME2=value") and a shell command after it exits. An empty value
    /// clears one. The game and the commands also get INST_NAME, INST_ID, INST_DIR,
    /// INST_MC_DIR and, for a Java instance, INST_JAVA. On Linux, also GameMode, MangoHud and
    /// the dedicated graphics card, when the computer has them
    Launch {
        instance: String,
        #[arg(long)]
        pre_launch: Option<String>,
        #[arg(long)]
        wrapper: Option<String>,
        #[arg(long)]
        post_exit: Option<String>,
        #[arg(long)]
        env: Option<String>,
        /// Linux: run it through GameMode (gamemoderun), on or off
        #[arg(long)]
        gamemode: Option<String>,
        /// Linux: MangoHud's overlay, on or off
        #[arg(long)]
        mangohud: Option<String>,
        /// Linux: the dedicated graphics card (DRI_PRIME, or NVIDIA's offload), on or off
        #[arg(long)]
        dedicated_gpu: Option<String>,
    },
    /// Put a shortcut on the desktop that plays an instance (`play --instance`): a .desktop
    /// file (also in the applications menu) on Linux, an app on macOS, a .lnk on Windows
    Shortcut {
        /// The instance (default: the selected one)
        instance: Option<String>,
        /// Move the instance's shortcuts to the trash instead
        #[arg(long)]
        remove: bool,
    },
    /// Import what a file is, as dropping it on the window does: a shared instance (a .mrpack
    /// without mod loaders, such as `instance export` writes, or a game folder zip) becomes a
    /// new instance; a Java modpack goes to the Modpacks import; a resource or shader pack into
    /// the instance (default: the selected one)
    Import {
        file: PathBuf,
        /// The instance a pack goes into
        #[arg(long)]
        instance: Option<String>,
        /// A .mrpack becomes a Minecraft: Java Edition instance with its Minecraft version, mod
        /// loader, mods and overrides, instead of a Horizon instance or a Modpacks import
        #[arg(long)]
        java: bool,
    },
    /// List the instances other launchers keep (Prism Launcher, MultiMC, the official
    /// launcher, the CurseForge app, the Modrinth App), read without changing anything there
    Others {
        /// Look in this folder instead: a launcher's folder, a folder of instances (its
        /// instances/, the CurseForge app's Instances) or one instance
        #[arg(long)]
        folder: Option<PathBuf>,
    },
    /// Copy another launcher's instance into a new Java instance: its Minecraft version and its
    /// game folder (worlds, mods, packs, options); the other launcher's copy stays as it is
    CopyFrom {
        /// Its name, as `instance others` lists it
        name: String,
        /// Look in this folder instead: a launcher's folder, a folder of instances (its
        /// instances/, the CurseForge app's Instances) or one instance
        #[arg(long)]
        folder: Option<PathBuf>,
        /// The new instance's name (default: the same)
        #[arg(long = "as")]
        new_name: Option<String>,
        /// The Minecraft version it plays, when its launcher doesn't say (the CurseForge app,
        /// the Modrinth App)
        #[arg(long)]
        minecraft: Option<String>,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum ExportFormat {
    /// Modrinth's modpack format
    Mrpack,
    /// The game folder's layout, zipped
    Zip,
}

#[derive(Subcommand)]
enum WorldsCmd {
    /// The worlds: name, mode, version, last played and size (the default)
    List {
        /// The instance (default: the selected one)
        #[arg(long)]
        instance: Option<String>,
    },
    /// Zip a world into the instance's backups/ folder, as the game's Make Backup does
    Backup {
        /// Its folder or name
        world: String,
        #[arg(long)]
        instance: Option<String>,
    },
    /// Copy a world beside it, as "<name> (1)"
    Copy {
        world: String,
        #[arg(long)]
        instance: Option<String>,
    },
    /// Move a world to the system's trash
    Delete {
        world: String,
        #[arg(long)]
        instance: Option<String>,
    },
    /// A world's data packs, and which are on
    DatapackList {
        world: String,
        #[arg(long)]
        instance: Option<String>,
    },
    /// Copy a data pack zip into a world (the game turns it on when the world opens)
    DatapackAdd {
        world: String,
        file: PathBuf,
        #[arg(long)]
        instance: Option<String>,
    },
    /// Move a world's data pack to the system's trash
    DatapackRemove {
        world: String,
        /// Its file name, with or without .zip
        pack: String,
        #[arg(long)]
        instance: Option<String>,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum PackKindArg {
    Resource,
    Shader,
}

impl From<PackKindArg> for horizon_launcher::packs::Kind {
    fn from(k: PackKindArg) -> Self {
        match k {
            PackKindArg::Resource => Self::Resource,
            PackKindArg::Shader => Self::Shader,
        }
    }
}

#[derive(Subcommand)]
enum PacksCmd {
    /// The resource and shader packs, and which are on (the default)
    List {
        /// Only this kind
        #[arg(long, value_enum)]
        kind: Option<PackKindArg>,
        /// The instance (default: the selected one)
        #[arg(long)]
        instance: Option<String>,
    },
    /// Copy a pack file in: a resource pack .zip, a shader pack .zip or a .hshader
    Add {
        file: PathBuf,
        #[arg(long)]
        instance: Option<String>,
    },
    /// Turn a pack on (a resource pack goes on top; a shader pack replaces the one that was on)
    On {
        /// Its file name or title
        pack: String,
        #[arg(long, value_enum, default_value_t = PackKindArg::Resource)]
        kind: PackKindArg,
        #[arg(long)]
        instance: Option<String>,
    },
    /// Turn a pack off
    Off {
        pack: String,
        #[arg(long, value_enum, default_value_t = PackKindArg::Resource)]
        kind: PackKindArg,
        #[arg(long)]
        instance: Option<String>,
    },
    /// Turn a pack off and move it to the system's trash
    Remove {
        pack: String,
        #[arg(long, value_enum, default_value_t = PackKindArg::Resource)]
        kind: PackKindArg,
        #[arg(long)]
        instance: Option<String>,
    },
}

#[derive(Subcommand)]
enum ScreenshotsCmd {
    /// The screenshots, the newest first, with their files (the default)
    List {
        /// The instance (default: the selected one)
        #[arg(long)]
        instance: Option<String>,
    },
    /// Move a screenshot to the system's trash
    Delete {
        /// Its file name, with or without .png
        screenshot: String,
        #[arg(long)]
        instance: Option<String>,
    },
}

#[derive(Subcommand)]
enum MultiplayerCmd {
    /// The servers of the game's Multiplayer screen (the default)
    List {
        /// The instance (default: the selected one)
        #[arg(long)]
        instance: Option<String>,
    },
    /// Add a server to the list
    Add {
        /// Its address, as in the game's Add Server (host or host:port)
        address: String,
        /// Its name in the list (default: Minecraft Server)
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        instance: Option<String>,
    },
    /// Remove a server from the list: its position from 1, name or address
    Remove {
        server: String,
        #[arg(long)]
        instance: Option<String>,
    },
}

#[derive(Subcommand)]
enum ModsCmd {
    /// What the browser installed in an instance (the default)
    List {
        /// The instance (default: the selected one)
        #[arg(long)]
        instance: Option<String>,
    },
    /// Search for packs Horizon loads, or for what a Java instance runs (made for its
    /// Minecraft version and loader)
    Search {
        text: Vec<String>,
        #[arg(long, value_enum, default_value_t = KindArg::Resource)]
        kind: KindArg,
        /// Also what was made for other versions or runs only partly, each marked
        #[arg(long)]
        everything: bool,
        #[arg(long, value_enum, default_value_t = SourceArg::Modrinth)]
        source: SourceArg,
        /// The instance (default: the selected one)
        #[arg(long)]
        instance: Option<String>,
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
    /// Install a pack and what it needs into an instance, by its id or slug (checksums
    /// checked); a Java mod is ported into horizon-mods and turned on (a Java instance gets the
    /// jar made for its version and loader in mods/); a modpack is imported as an instance of
    /// its own; a data pack goes into the world `--world` names
    Install {
        project: String,
        /// Install the project's data pack into this world of the instance (its name or folder)
        #[arg(long)]
        world: Option<String>,
        /// The instance (default: the selected one)
        #[arg(long)]
        instance: Option<String>,
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
        /// The instance (default: the selected one)
        #[arg(long)]
        instance: Option<String>,
    },
    /// Install a native mod from a GitHub link (docs/modding/src/how-to/native-mods.md, "Install from GitHub"):
    /// https://github.com/OWNER/REPO (its latest release, else its default branch),
    /// .../releases/tag/TAG, .../tree/REF or .../commit/SHA. Resolved to an exact release file or
    /// commit, checked like a registry mod. NOT reviewed by the Horizon registry
    AddGithub {
        url: String,
        /// The mod to install when the link holds several
        #[arg(long = "mod")]
        mod_id: Option<String>,
        /// Install every mod the link holds
        #[arg(long)]
        all: bool,
        /// The instance (default: the selected one)
        #[arg(long)]
        instance: Option<String>,
        /// Allow native code for this install (remembered; revoke in Settings, Storage)
        #[arg(long)]
        trust_native_code: bool,
    },
    /// Look for newer versions of the native mods installed from GitHub (the latest release, or the
    /// default branch's head for a mod installed from a commit), and install them
    UpdateGithub {
        /// One mod (default: all of them)
        id: Option<String>,
        /// Only say what is newer
        #[arg(long)]
        check: bool,
        /// The instance (default: the selected one)
        #[arg(long)]
        instance: Option<String>,
        /// Allow native code for this install (remembered; revoke in Settings, Storage)
        #[arg(long)]
        trust_native_code: bool,
    },
    /// Remove a pack the browser installed
    Remove {
        project: String,
        /// The instance (default: the selected one)
        #[arg(long)]
        instance: Option<String>,
    },
    /// Turn on a mod the browser installed (horizon-mods.txt; the game's Mods screen has no
    /// switch; in a Java instance, its file loses `.disabled`)
    Enable {
        project: String,
        /// The instance (default: the selected one)
        #[arg(long)]
        instance: Option<String>,
    },
    /// Turn off a mod the browser installed; it stays installed (in a Java instance, as
    /// `<file>.disabled`)
    Disable {
        project: String,
        /// The instance (default: the selected one)
        #[arg(long)]
        instance: Option<String>,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum KindArg {
    Mod,
    Resource,
    Modpack,
    /// Horizon's own shader packs (Horizon's registry only)
    Shader,
    /// Data packs (Modrinth): install one into a world with `mods install <pack> --world <name>`
    Datapack,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum SourceArg {
    Modrinth,
    /// CurseForge support is coming soon
    Curseforge,
    /// Horizon's own registry: mods, modpacks and resource packs made for Horizon, reviewed
    Horizon,
}

impl SourceArg {
    fn get(self) -> Result<Box<dyn horizon_launcher::browser::Source>, String> {
        use horizon_launcher::browser::{self, SourceId};
        let id = match self {
            SourceArg::Modrinth => SourceId::Modrinth,
            SourceArg::Curseforge => SourceId::CurseForge,
            SourceArg::Horizon => SourceId::Horizon,
        };
        browser::source(id).ok_or_else(|| browser::CURSEFORGE_SOON.into())
    }
}

#[derive(Subcommand)]
enum Cmd {
    /// Write a shell completion script to stdout
    Completions { shell: clap_complete::Shell },
    /// List Horizon releases, or the installed versions
    Versions {
        /// Only the versions installed here (no network)
        #[arg(long)]
        installed: bool,
    },
    /// Download and install a release (a tag such as v2026.10.13, or "latest")
    Install { tag: String },
    /// Install a local unsigned developer build from a trusted checkout
    #[command(about = tr("Install a local unsigned developer build from a trusted checkout"), after_help = tr("Example: horizon-launcher developer-install CHECKOUT --program CHECKOUT/target/debug/horizon-client --trust-unsigned\nThis executes the checkout's build code and an unsigned game with your rights. Trust both before continuing."), long_about = tr("Install a local unsigned developer build. This runs the checkout's build code through cargo run -p xtask -- package. Commit tracked changes and rebuild first. Build horizon-ctl, horizon-prepare and horizon-server beside the game, generate horizon-wasm-std.wasm beside it with cargo run --locked -p cargo-horizon-mod -- wasm-std PATH, and compile exporters into the checkout's exporters folder. Native builds use the saved local source commit, including unpublished commits. Select this base explicitly; automatic updates never replace it."))]
    DeveloperInstall(developer_builds::DeveloperInstall),
    /// Remove an installed version
    Remove { tag: String },
    /// Install a game program you have, such as a horizon-client built on your fork's Actions,
    /// as its own version on an installed base's game files, and select it. A local unsigned
    /// developer base is allowed only when explicitly selected or named with --version.
    /// Nothing signed it: it runs with your rights, and updates never replace it
    #[command(
        after_help = "The program must match this computer's executable format and carry the exact `horizon-game-executable;`\nidentity signature embedded by ordinary and launcher-generated games. It carries no mod API level or build id,\nand is not a signature of authenticity. Unmarked programs are refused."
    )]
    ImportBuild {
        /// The program (horizon-client, horizon-client.exe on Windows), or a zip holding it
        path: PathBuf,
        /// The installed base whose game files it plays with (default: the selected usable
        /// base, else the newest release or Nightly; developer bases need explicit selection)
        #[arg(long)]
        version: Option<String>,
        /// You picked this build yourself and trust it with full access to this computer
        #[arg(long)]
        trust_unsigned: bool,
    },
    /// Prepare a version's resources from your own Minecraft install, or from Mojang
    Prepare {
        /// The game folder of the official launcher, Prism Launcher, MultiMC, the Modrinth App,
        /// ATLauncher or GDLauncher (default: the launcher's, then every launcher's default)
        #[arg(long, conflicts_with = "download")]
        minecraft: Option<PathBuf>,
        /// Download Minecraft from Mojang for the signed-in account that owns it, instead of
        /// using an install
        #[arg(long)]
        download: bool,
        /// The installed version (default: the selected one)
        #[arg(long)]
        version: Option<String>,
    },
    /// Start the client (needs a signed-in Microsoft account that owns Minecraft: Java Edition)
    Play {
        /// Allow missing original Java asset/data jars to be downloaded from Modrinth
        #[arg(long)]
        allow_jar_downloads: bool,
        #[arg(long, help = "Installed version to play (default: instance version)")]
        version: Option<String>,
        /// The instance to play (its name or id; default: the selected one). A Java instance
        /// starts its Minecraft: Java Edition, downloaded with its Java the first time
        #[arg(long)]
        instance: Option<String>,
        /// Open or create this world (a folder under saves) at once
        #[arg(long)]
        world: Option<String>,
        /// Creative mode for a new world
        #[arg(long)]
        creative: bool,
        /// Join a friend's world with its invite code
        #[arg(long)]
        join: Option<String>,
        /// Join this server at once: an address, or a server of the list by name or position
        #[arg(long, conflicts_with_all = ["world", "join"])]
        server: Option<String>,
        /// An extra argument for the game, passed through as-is (repeatable), for example
        /// --server-thread
        #[arg(long = "arg", value_name = "ARG", allow_hyphen_values = true)]
        arg: Vec<String>,
        /// Play this launch as this signed-in account (its name or UUID), without changing
        /// the one `account select` picked: several games can play as several accounts at once
        #[arg(long, value_name = "WHO")]
        account: Option<String>,
    },
    /// Run a dedicated server here; type commands into its console
    Server {
        /// Allow missing original Java asset/data jars to be downloaded from Modrinth
        #[arg(long)]
        allow_jar_downloads: bool,
        /// The server's folder (default: the one the launcher uses)
        #[arg(long)]
        dir: Option<PathBuf>,
        #[arg(long)]
        version: Option<String>,
        /// Agree to the Minecraft EULA (https://aka.ms/MinecraftEULA)
        #[arg(long)]
        accept_eula: bool,
    },
    /// Show disposable storage by category as JSON. Builds must be idle
    ///
    /// Rust/Cargo/temp build-tool caches remain clearable. Game-accessible caches, installed-version
    /// removal and automatic runtime eviction retain files while any launch path lacks enforceable
    /// lifetime exclusion; an empty process scan is not authorization. Read-only reports stay available.
    /// Private launch records and raw captures are collected together only after all participating owners
    /// exit, under the exclusive permanent namespace gate. Published unit keys persist until exclusive
    /// unit removal. Registry entry counts are logged; lock entries are not age-collected.
    Storage {
        /// Request cleanup of build-files, prebuilt-games, spare-game-builds, rust-compiler, other-caches or all rows; game-accessible caches refuse cleanup without enforceable lifetime exclusion
        #[arg(long, value_parser = parse_storage_row)]
        clear: Option<ClearStorage>,
    },
    /// Generate the registry-only Cargo.lock for one native mod, without executing mod code
    NativeLock {
        folder: PathBuf,
        /// Explicitly replace an existing lock with newly resolved registry pins
        #[arg(long)]
        update_lock: bool,
    },
    /// Native mods (docs/modding/src/how-to/native-mods.md): compile flat Rust mod projects against the instance's exact
    /// Horizon source revision with the launcher's own Rust, as the version the instance then
    /// plays. Not sandboxed: the mods' code runs with your rights
    NativeBuild {
        /// Discover internal uses through compiler diagnostics. Prints resolved declarations
        /// as JSON for cargo horizon-mod internals sync; never installs or selects a game
        #[arg(long, conflicts_with_all = ["server_output", "instance", "version", "update", "profile"], requires = "folder")]
        sync_internals: bool,
        #[arg(long, requires = "folder", conflicts_with_all = ["sync_internals", "server_output", "instance", "version", "update", "profile"])]
        catalog: bool,
        /// Copy discovery catalogs into this caller-owned empty directory
        #[arg(long, requires = "catalog")]
        catalog_output: Option<PathBuf>,
        /// A mod folder (horizon-mod.toml with [native], Cargo.toml and src/), or a folder of them
        /// (default: the native mods installed and turned on in the instance). Server exports
        /// also read wasm internals declarations from .hmod packages directly in this folder
        folder: Option<PathBuf>,
        /// Export mode: build a standalone dedicated-server engine into this folder instead of
        /// an instance's build. Needs a mod FOLDER; no instance is read or changed, and no
        /// launcher config is written
        #[arg(
            long = "server-output",
            value_name = "DIR",
            conflicts_with = "instance",
            requires = "folder"
        )]
        server_output: Option<PathBuf>,
        /// Allow exact-hash jar downloads into the local cache for the server export;
        /// jar bytes are never included in the exported folder
        #[arg(long, requires = "server_output")]
        allow_jar_downloads: bool,
        /// The instance whose installed native mods to build, and which then plays the build
        /// (default: the selected one)
        #[arg(long)]
        instance: Option<String>,
        /// The installed version whose game files the build reuses, and whose commit it builds on
        /// when no mod pins one (default: the one the instance plays, so a rebuild stays on its
        /// Horizon)
        #[arg(long)]
        version: Option<String>,
        /// Move to the newest installed release or Nightly instead of the Horizon the instance
        /// plays (rebuilds the mods for that pinned SDK)
        #[arg(long, conflicts_with = "version")]
        update: bool,
        /// Allow native code for this install (remembered; revoke in Settings, Storage)
        #[arg(long)]
        trust_native_code: bool,
        /// Native mods use the pinned SDK's optimized compilation settings.
        /// The game is never rebuilt by a mod build.
        #[arg(long, value_enum)]
        profile: Option<native_mods::Profile>,
        /// Build the horizon-mixins tool from its pinned source instead of downloading the
        /// prebuilt one: for CI, which keeps that path proven
        #[arg(long, hide = true)]
        mixins_from_source: bool,
        /// When a mod this build can't use was saved into the instance's worlds, back those
        /// worlds up (into the game folder's backups/) and build without it. Without this the
        /// build stops and names the worlds, leaving the instance as it is
        #[arg(long, conflicts_with = "server_output")]
        back_up_worlds: bool,
    },
    /// Show a release's patch notes (default: the newest)
    Notes { tag: Option<String> },
    /// Show the website, roadmap and documentation links
    Links,
    /// Microsoft accounts: list them (the default), sign in, sign out, or pick the one that plays
    Account {
        #[command(subcommand)]
        action: Option<AccountCmd>,
    },
    /// Show the launcher's settings, or set one (the Settings page): selected-version TAG,
    /// game-architecture auto|x64|arm64,
    /// minecraft-dir PATH, server-dir PATH ("default" resets a folder), install-new-versions
    /// on|off, prune-old-versions on|off (delete old versions after an install, keeping the
    /// selected one, the ones instances use and the newest few), keep-versions N (how many of the
    /// newest to keep; "default" is 2), opening-animation on|off, proxy http://host:port|socks5://host:port
    /// ("default" removes it), while-playing stay|minimize|hide (what the window does while the game runs), language
    /// system|en|fr|ru|de|zh|es|uk|pl (the window's language; system follows the computer's), accent
    /// violet|blue|teal|green|amber|red|pink|#rrggbb|DEGREES (a named accent, any colour, or any
    /// hue), background PATH (a picture behind Home; "default"
    /// shows Horizon's landscape), cache-limit-gib N (managed cache limit
    /// in GiB: 10, 25 (default), 50, 100; 0 disables automatic cleanup)
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
    /// Upload an instance's latest.log to mclo.gs (a paste service for Minecraft logs) and print
    /// its link, for a bug report: only when the player asks. The home folder, the user name and
    /// the account names are taken out first; the link expires after 90 days unread
    ShareLog {
        /// The instance (default: the selected one)
        #[arg(long)]
        instance: Option<String>,
    },
    /// Delete a log shared with share-log: the "<id>:<token>" it printed
    UnshareLog { shared: String },
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
    /// Write a bug report to a file you attach to the issue by hand; nothing is sent
    /// (docs/telemetry.md)
    BugReport {
        text: String,
        /// Add the end of the game's log (names and worlds hidden, without chat)
        #[arg(long)]
        log: bool,
    },
    /// Opt-in data collection (docs/telemetry.md): what is allowed, what would be sent, and
    /// deleting it; all off until you allow a kind
    Data {
        #[command(subcommand)]
        action: Option<DataCmd>,
    },
    /// Instances: separate game folders, each with its own worlds, options and packs
    Instance {
        #[command(subcommand)]
        action: Option<InstanceCmd>,
    },
    /// An instance's single-player worlds: list them, back one up, copy or delete it
    Worlds {
        #[command(subcommand)]
        action: Option<WorldsCmd>,
    },
    /// An instance's resource and shader packs: list, add, turn on or off, remove
    Packs {
        #[command(subcommand)]
        action: Option<PacksCmd>,
    },
    /// Java for Minecraft: Java Edition instances: list the runtimes, download Mojang's
    Java {
        #[command(subcommand)]
        action: Option<JavaCmd>,
    },
    /// The screenshots F2 took in an instance: list, delete
    Screenshots {
        #[command(subcommand)]
        action: Option<ScreenshotsCmd>,
    },
    /// The servers in an instance's Multiplayer list (servers.dat): list, add, remove
    Multiplayer {
        #[command(subcommand)]
        action: Option<MultiplayerCmd>,
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
    /// The portable Rust toolchain native mods compile with (docs/modding/src/how-to/native-mods.md): the launcher's
    /// own, in its data folder, never installed on the system
    Rust {
        #[command(subcommand)]
        action: Option<RustCmd>,
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
const SETTING_KEYS: [&str; 14] = [
    "selected-version",
    "minecraft-dir",
    "server-dir",
    "install-new-versions",
    "prune-old-versions",
    "keep-versions",
    "opening-animation",
    "game-architecture",
    "proxy",
    "while-playing",
    "language",
    "accent",
    "background",
    "cache-limit-gib",
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
        "prune-old-versions" => s.prune_old_versions = Some(on_off(value)?),
        "keep-versions" => {
            s.keep_versions = match value {
                "default" => None,
                n => Some(
                    n.parse::<usize>()
                        .ok()
                        .filter(|n| *n > 0)
                        .ok_or_else(|| format!("{value}: say a number from 1, or default"))?,
                ),
            }
        }
        "opening-animation" => s.skip_opening = !on_off(value)?,
        "game-architecture" => {
            let choice = backend::GameArchitecture::parse(value).ok_or_else(|| {
                tr("Say auto, x64 or arm64 for the game architecture.").to_string()
            })?;
            if choice != backend::GameArchitecture::Auto && !backend::can_choose_game_architecture()
            {
                return Err(
                    tr("Manual game architectures are available on Windows ARM only.").into(),
                );
            }
            s.game_architecture = choice;
        }
        "while-playing" => {
            s.while_playing = backend::WhilePlaying::parse(value)
                .ok_or_else(|| format!("{value}: say stay, minimize or hide"))?
        }
        "accent" => {
            use horizon_launcher::themes::Accent;
            s.accent = Accent::parse(value).ok_or_else(|| {
                let names: Vec<_> = Accent::ALL.iter().map(|a| a.name()).collect();
                format!(
                    "{value}: say {}, a colour like #2563eb or a hue in degrees (0 to 360)",
                    names.join(", ")
                )
            })?
        }
        "background" => {
            s.background = match value.trim() {
                "default" | "" => None,
                file => Some(horizon_launcher::themes::keep_background(
                    &backend::data_dir(),
                    std::path::Path::new(file),
                )?),
            }
        }
        "language" => {
            use horizon_launcher::i18n::Language;
            s.language = match value.trim() {
                "system" | "default" => None,
                code => Some(Language::parse(code).ok_or_else(|| {
                    format!("{value}: say system, en, fr, ru, de, zh, es, uk, pl or pt-br")
                })?),
            }
        }
        "cache-limit-gib" => {
            s.cache_limit_gib = match value {
                "default" => None,
                n => Some(
                    n.parse::<u64>()
                        .ok()
                        .filter(|n| matches!(n, 0 | 10 | 25 | 50 | 100))
                        .ok_or_else(|| {
                            tr("Enter a cache limit in GiB; 0 disables automatic cleanup.")
                                .to_string()
                        })?,
                ),
            };
        }
        "proxy" => {
            if value == "default" || value.is_empty() {
                s.proxy = None;
            } else if let Some(why) = horizon_launcher::proxy::refuse(value) {
                return Err(why);
            } else {
                s.proxy = Some(value.trim().to_string());
            }
        }
        _ => return Err(format!("No setting {key}.")),
    }
    Ok(())
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
        Cmd::Mcp | Cmd::AgentConfig { .. } if no_ai => Err(horizon_platform::no_ai::REFUSAL.into()),
        Cmd::Mcp => crate::mcp::serve().map(|()| 0),
        command => {
            let result = execute(command, cli.json);
            // A game it started has ended (attached) or none did (an error): a session file it
            // didn't read goes now.
            account::discard_unread_sessions();
            result
        }
    };
    Some(match result {
        Ok(code) => code,
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

/// Keep detailed terminal updates, but only the first and completed download updates in logs.
fn native_progress(tty: bool) -> impl Fn(native_mods::Step) -> Option<String> {
    use native_mods::Step;
    let previous = std::cell::RefCell::new(None);
    move |step| {
        let show = tty
            || match (&step, previous.borrow().as_ref()) {
                (Step::Rust(done), Some(Step::Rust(last))) => *done >= 1.0 && *last < 1.0,
                (
                    Step::Tools { build, done, total },
                    Some(Step::Tools {
                        build: last_build,
                        done: last_done,
                        total: last_total,
                    }),
                ) if build == last_build => {
                    *total > 0 && done >= total && (*last_total == 0 || last_done < last_total)
                }
                _ => true,
            };
        let label = show.then(|| step.label());
        *previous.borrow_mut() = Some(step);
        label
    }
}

fn json(value: serde_json::Value) {
    println!("{}", serde_json::to_string_pretty(&value).unwrap());
}

/// Runs a game process in this terminal and reports how it ended.
fn attached(cmd: Command) -> Result<i32, String> {
    let (mut cmd, _runtime) = backend::prepare_command(cmd)?;
    let status = backend::spawn_attached(&mut cmd)
        .and_then(|mut child| child.wait())
        .map_err(|e| format!("Could not start {:?}: {e}", cmd.get_program()))?;
    #[cfg(unix)]
    {
        horizon_launcher::process::supervisor::exit_code(status).map_err(|error| error.to_string())
    }
    #[cfg(not(unix))]
    {
        status.code().ok_or_else(|| format!("Exited with {status}"))
    }
}

fn release_json(r: &Release) -> serde_json::Value {
    serde_json::json!({
        "tag": r.tag, "name": r.name, "date": r.date, "prerelease": r.prerelease,
        "available_here": r.asset.is_some(), "installed": backend::installed().contains(&r.tag),
    })
}

fn execute(command: Cmd, as_json: bool) -> Result<i32, String> {
    if matches!(command, Cmd::Versions { .. } | Cmd::Update { .. }) {
        for (tag, from) in backend::reuse_prepared_everywhere() {
            if !as_json {
                eprintln!("Prepared {tag} with the resources of {from}.");
            }
        }
    }
    match command {
        Cmd::Completions { shell } => {
            clap_complete::generate(
                shell,
                &mut self::command(),
                "horizon-launcher",
                &mut std::io::stdout(),
            );
        }
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
                let state = if !backend::check_prepared(tag) {
                    "not prepared"
                } else if backend::has_sounds(tag) {
                    "prepared"
                } else {
                    "prepared, no sounds (prepare again)"
                };
                let label =
                    horizon_launcher::developer_install::label(tag).unwrap_or_else(|| tag.clone());
                println!("{mark} {label}  {state}");
            }
        }
        Cmd::Versions { installed: false } => {
            let releases = backend::releases()?;
            if as_json {
                json(releases.iter().map(release_json).collect());
                return Ok(0);
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
                    serde_json::json!({ "installed": tag, "prepared": backend::check_prepared(&tag) }),
                );
            } else if backend::check_prepared(&tag) {
                println!(
                    "Installed {tag}. It is prepared already, from another installed version."
                );
            } else {
                println!(
                    "Installed {tag}. Prepare it next: horizon-launcher prepare --version {tag}"
                );
            }
        }
        Cmd::Storage { clear } => {
            let storage = Storage::open().map_err(native_mods::cache_error)?;
            let (row, all) = match clear {
                Some(ClearStorage::Row(row)) => (Some(row), false),
                Some(ClearStorage::All) => (None, true),
                None => (None, false),
            };
            let report = storage.report(row, all).map_err(native_mods::cache_error)?;
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
            if row.is_some() || all {
                if let Some(kept) = report.rows.iter().flat_map(|row| &row.kept).next() {
                    return Err(kept.reason.clone());
                }
            }
        }
        Cmd::NativeLock {
            folder,
            update_lock,
        } => native_mods::registry_lock(&folder, update_lock)?,
        Cmd::NativeBuild {
            sync_internals,
            catalog,
            catalog_output,
            folder,
            server_output,
            allow_jar_downloads,
            instance,
            version,
            update,
            trust_native_code,
            profile,
            mixins_from_source,
            back_up_worlds,
        } => {
            native_permission(trust_native_code)?;
            native_mods::mixins_from_source(mixins_from_source);
            if catalog {
                let discovered =
                    native_mods::pinned_catalog_with_modules(&folder.expect("required by clap"))?;
                let destination = catalog_output.ok_or("--catalog requires --catalog-output")?;
                let destination = discovered.snapshot(&destination)?;
                println!("{}", destination.display());
                return Ok(0);
            }
            if sync_internals {
                let entries = native_mods::sync_internals(&folder.expect("required by clap"))?;
                println!(
                    "{}",
                    serde_json::to_string(&entries).map_err(|error| error.to_string())?
                );
                return Ok(0);
            }
            let progress = native_progress(std::io::stderr().is_terminal());
            let steps = |step| {
                if let Some(label) = progress(step) {
                    eprintln!("{label}");
                }
            };
            if let Some(output) = server_output {
                // Export mode: a standalone server engine in `output`. No instance is read
                // or changed, and no launcher config is written; `folder` is required (clap).
                let folder = folder.unwrap();
                let base = match &version {
                    Some(v) => backend::resolve_version(Some(v))?,
                    // The instance's version is never looked at here: the newest installed
                    // release or Nightly, or that one with `--update` (never a native build).
                    None => native_mods::base_for(None, &backend::installed(), update)
                        .ok_or_else(backend::no_version_installed)?,
                };
                let built = native_mods::build_server(
                    &base,
                    &folder,
                    &output,
                    profile,
                    allow_jar_downloads,
                    steps,
                    registry_consent,
                )?;
                let built = std::path::absolute(&built).unwrap_or(built);
                let path = built.display().to_string();
                println!(
                    "{}",
                    trf("Built the server engine: {path}", &[("path", &path)])
                );
                println!(
                    "{}",
                    tr(
                        "Point the server at it: set engine.path to this file in the server's \
                        server.toml (docs/hosting.md); the launcher writes nothing itself."
                    )
                );
                return Ok(0);
            }
            let target = match &instance {
                Some(name) => instances::find(name)?,
                None => instances::selected(),
            };
            let target = match profile {
                Some(profile) => {
                    instances::edit(&target.id, |meta| meta.native_profile = Some(profile))?
                }
                None => target,
            };
            let base = match version {
                Some(v) => backend::resolve_version(Some(&v))?,
                None => {
                    let plays = target.resolve_version(None).ok();
                    native_mods::base_for(plays.as_deref(), &backend::installed(), update)
                        .ok_or_else(backend::no_version_installed)?
                }
            };
            let explicit = folder.is_some() || profile.is_some();
            let folder = match folder {
                Some(folder) => folder,
                None => native_mods::installed_mods(&target.home)?,
            };
            let profile = explicit.then_some(target.native_profile);
            let built =
                native_mods::build(&base, &folder, &target, profile, back_up_worlds, steps, registry_consent).map_err(|error| {
                    if let native_mods::Failed::Worlds(stake) = &error {
                        return trf(
                            "{error}\n{stay}: nothing changed. Or run again with --back-up-worlds to back them up into {backups} and build without the mods named.",
                            &[
                                ("error", &stake.text),
                                ("stay", &native_mods::fit::stay(&stake.stay)),
                                ("backups", &stake.backups.display().to_string()),
                            ],
                        );
                    }
                    let error = String::from(error);
                    if error.starts_with(native_mods::MISMATCH) {
                        trf("{error} Run `cargo horizon-mod update --horizon {commit}` in each mod's folder to pin it to this game, then build again.", &[("error", &error), ("commit", &base)])
                    } else { error }
                })?;
            println!(
                "{}",
                trf(
                    "{build} built and selected for its instance.",
                    &[("build", &built)]
                )
            );
        }
        Cmd::DeveloperInstall(args) => developer_builds::install(args, as_json)?,
        Cmd::Remove { tag } => {
            backend::remove_version(&tag)?;
            println!("Removed {tag}.");
        }
        Cmd::ImportBuild {
            path,
            version,
            trust_unsigned,
        } => developer_builds::import(&path, version.as_deref(), trust_unsigned, as_json)?,
        Cmd::Prepare {
            minecraft,
            download,
            version,
        } => {
            let tag = backend::resolve_version(version.as_deref())?;
            let minecraft = backend::minecraft_dir(minecraft)?;
            let from = if download {
                backend::MinecraftFrom::Mojang
            } else {
                backend::MinecraftFrom::Install
            };
            let (preparer, minecraft) =
                backend::preparer(&tag, &minecraft, from, &|line| eprintln!("{line}"))?;
            backend::sounds(&tag, &minecraft, &|line| println!("{line}"));
            match preparer {
                backend::Preparer::Process(cmd) => {
                    let code = attached(cmd)?;
                    if code != 0 {
                        return Ok(code);
                    }
                }
                backend::Preparer::InProcess(options) => {
                    backend::prepare_in_process(&options, &|line| println!("{line}"))?
                }
            }
            println!("{tag} is prepared.");
        }
        Cmd::Play {
            allow_jar_downloads,
            version,
            instance,
            world,
            creative,
            join,
            server,
            arg,
            account,
        } => {
            let instance = match instance {
                Some(name) => instances::find(&name)?,
                None => instances::selected(),
            };
            if instance.java.is_some() {
                if version.is_some()
                    || world.is_some()
                    || join.is_some()
                    || creative
                    || account.is_some()
                {
                    return Err(format!(
                        "{} plays Minecraft: Java Edition: --version, --world, --creative, \
                         --join and --account are for Horizon instances.",
                        instance.name
                    ));
                }
                let server = server.map(|server| {
                    let list = server_list::List::load(&instance.home);
                    server_list::find(&list, &server).map_or(server, |(_, s)| s.ip)
                });
                let mut cmd =
                    backend::java_game(&instance, server.as_deref(), &|line| eprintln!("{line}"))?;
                if let Some(e) = account::take_unsaved() {
                    eprintln!("{e}");
                }
                cmd.args(arg);
                let vars = instance.vars(cmd.get_program());
                cmd.envs(vars.iter().map(|(k, v)| (k, v)));
                let started = std::time::Instant::now();
                let played = attached(horizon_launcher::launch::wrap(cmd, &instance.launch));
                if let Err(e) = instances::record_play(&instance.id, started.elapsed()) {
                    eprintln!("Play time not saved: {e}");
                }
                after_exit(&instance, &vars);
                return played;
            }
            // Who plays this launch only.
            let who = match account {
                Some(who) => Some(
                    Accounts::load()
                        .find(&who)
                        .ok_or_else(|| {
                            format!("No signed-in account {who}. See `horizon-launcher account`.")
                        })?
                        .uuid
                        .clone(),
                ),
                None => None,
            };
            let mut cmd = backend::client_as(
                &instance.resolve_version(version.as_deref())?,
                who.as_deref(),
            )?;
            instances::apply(&mut cmd, &instance)?;
            if let Some(code) = join {
                backend::join_friend(&mut cmd, &code)?;
            }
            if let Some(world) = world {
                cmd.arg("--world").arg(world);
            }
            if let Some(server) = server {
                let list = server_list::List::load(&instance.home);
                let ip = server_list::find(&list, &server).map_or(server, |(_, s)| s.ip);
                cmd.arg("--server").arg(ip);
            }
            if creative {
                cmd.arg("--creative");
            }
            cmd.args(arg);
            let started = std::time::Instant::now();
            let vars = instance.vars(cmd.get_program());
            cmd.envs(vars.iter().map(|(k, v)| (k, v)));
            horizon_launcher::jars::prepare_launch(
                &mut cmd,
                &instance.launch,
                horizon_launcher::jars::Permission::headless(allow_jar_downloads),
            )
            .map_err(|error| error.to_string())?;
            let played = attached(horizon_launcher::launch::wrap(cmd, &instance.launch));
            if let Err(e) = instances::record_play(&instance.id, started.elapsed()) {
                eprintln!("Play time not saved: {e}");
            }
            after_exit(&instance, &vars);
            return played;
        }
        Cmd::Server {
            dir,
            version,
            accept_eula,
            allow_jar_downloads,
        } => {
            let tag = backend::resolve_version(version.as_deref())?;
            let dir = backend::server_dir(dir);
            // The server inherits this terminal, so typed lines reach its console.
            let mut cmd = backend::server(&tag, &dir, accept_eula)?;
            horizon_launcher::jars::prepare_launch(
                &mut cmd,
                &Default::default(),
                horizon_launcher::jars::Permission::headless(allow_jar_downloads),
            )
            .map_err(|error| error.to_string())?;
            return attached(cmd);
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
        Cmd::Data { action } => data(action.unwrap_or(DataCmd::Status), as_json)?,
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
                    ("Website", &links.website),
                    ("Roadmap", &links.roadmap),
                    ("Documentation", &links.docs),
                    ("Discord", &links.discord),
                    ("Sponsor", &links.sponsor),
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
            let tools = horizon_launcher::agents::self_test(&backend::update_target())?;
            if as_json {
                json(serde_json::json!({ "answered": true, "tools": tools }));
            } else {
                println!("The MCP server answered with {tools} tools.");
            }
        }
        Cmd::AgentConfig { client, .. } => {
            use horizon_launcher::agents;
            let exe = &backend::update_target();
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
                return Ok(0);
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
                settings.save()?;
                horizon_launcher::proxy::announce(settings.proxy.clone());
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
                settings.save()?;
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
        Cmd::ShareLog { instance } => {
            use horizon_launcher::game_log;
            let i = match instance {
                Some(name) => instances::find(&name)?,
                None => instances::selected(),
            };
            let path = i.home.join("logs/latest.log");
            let text =
                std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let names = game_log::names(&[&i.home, &backend::game_home()]);
            let text = game_log::private(&text, dirs::home_dir().as_deref(), &names);
            let shared = game_log::share(&text)?;
            if as_json {
                json(serde_json::json!({
                    "url": shared.url, "delete": format!("{}:{}", shared.id, shared.token),
                }));
            } else {
                println!("{}", shared.url);
                println!(
                    "Delete it: horizon-launcher unshare-log {}:{}",
                    shared.id, shared.token
                );
            }
        }
        Cmd::UnshareLog { shared } => {
            let (id, token) = shared
                .split_once(':')
                .ok_or("Give the \"<id>:<token>\" share-log printed.")?;
            horizon_launcher::game_log::unshare(&horizon_launcher::game_log::Shared {
                url: String::new(),
                id: id.into(),
                token: token.into(),
            })?;
            println!("Deleted the shared log {id}.");
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
        Cmd::BugReport { text, log } => {
            let path = horizon_launcher::bug_report::write(&text, log)?;
            if as_json {
                json(serde_json::json!({ "path": path }));
            } else {
                println!(
                    "Wrote {}: read it, then attach it to your issue.",
                    path.display()
                );
            }
        }
        Cmd::DesktopEntry { remove } => {
            let mut settings = backend::Settings::load();
            settings.desktop_entry = Some(!remove);
            settings.save()?;
            if remove {
                horizon_launcher::desktop::remove()?;
                println!("Removed Horizon from the applications menu.");
            } else {
                horizon_launcher::desktop::install(backend::exe_path())?;
                println!("Horizon is in the applications menu.");
            }
        }
        Cmd::Worlds { action } => worlds(
            action.unwrap_or(WorldsCmd::List { instance: None }),
            as_json,
        )?,
        Cmd::Packs { action } => game_packs(
            action.unwrap_or(PacksCmd::List {
                kind: None,
                instance: None,
            }),
            as_json,
        )?,
        Cmd::Screenshots { action } => screenshots(
            action.unwrap_or(ScreenshotsCmd::List { instance: None }),
            as_json,
        )?,
        Cmd::Multiplayer { action } => multiplayer(
            action.unwrap_or(MultiplayerCmd::List { instance: None }),
            as_json,
        )?,
        Cmd::Instance { action } => instance(action.unwrap_or(InstanceCmd::List), as_json)?,
        Cmd::Modpack { action } => return modpack(action.unwrap_or(ModpackCmd::List), as_json),
        Cmd::Mods { action } => mods(action.unwrap_or(ModsCmd::List { instance: None }), as_json)?,
        Cmd::Servers { action } => servers(action.unwrap_or(ServersCmd::List), as_json)?,
        Cmd::Rust { action } => rust(action.unwrap_or(RustCmd::Status), as_json)?,
        Cmd::Java { action } => java(action.unwrap_or(JavaCmd::List), as_json)?,
        Cmd::Mcp => unreachable!(),
    }
    Ok(0)
}
fn java(action: JavaCmd, as_json: bool) -> Result<(), String> {
    use horizon_launcher::java;
    match action {
        JavaCmd::List => {
            let mut all = java::mojang_runtimes(&java::runtimes_dir());
            all.extend(java::installed());
            if as_json {
                json(serde_json::json!(all));
            } else if all.is_empty() {
                println!("No Java yet. Horizon downloads Mojang's when an instance needs it.");
            } else {
                for r in &all {
                    let from = match r.source {
                        java::Source::Mojang => "Mojang's, downloaded by Horizon",
                        java::Source::Installed => "installed",
                    };
                    println!("Java {}  ({from})\n  {}", r.version, r.home.display());
                }
            }
        }
        JavaCmd::Install { java: which } => {
            let need = java::Need::named(&which)?;
            let runtime = java::install(&need, &|line| eprintln!("{line}"))?;
            if as_json {
                json(serde_json::json!(runtime));
            } else {
                println!(
                    "Java {} is ready: {}",
                    runtime.version,
                    runtime.home.display()
                );
            }
        }
    }
    Ok(())
}
fn rust(action: RustCmd, as_json: bool) -> Result<(), String> {
    use horizon_launcher::rust_toolchain as rust;
    match action {
        RustCmd::Status => {
            let installed = rust::installed();
            let version = installed.as_ref().map(|t| t.version()).transpose()?;
            let linker = rust::linker().err();
            if as_json {
                json(serde_json::json!({
                    "pinned": rust::VERSION,
                    "host": rust::host().map(|h| h.target),
                    "installed": installed.as_ref().map(|t| t.dir.display().to_string()),
                    "version": version,
                    "linker": linker,
                }));
            } else {
                match (&installed, &version) {
                    (Some(t), Some(v)) => println!("{v} in {}", t.dir.display()),
                    _ => println!(
                        "Rust {} is not installed (`horizon-launcher rust install`).",
                        rust::VERSION
                    ),
                }
                println!(
                    "{}",
                    linker.unwrap_or_else(|| "This computer can link the game.".into())
                );
            }
        }
        RustCmd::Install => {
            let shown = std::cell::Cell::new(0);
            let toolchain = rust::install(|f| {
                let percent = (f * 100.0) as u32;
                if percent >= shown.get() + 10 {
                    shown.set(percent);
                    eprintln!("{percent}%");
                }
            })?;
            println!("{} in {}", toolchain.version()?, toolchain.dir.display());
        }
        RustCmd::Remove => {
            rust::remove()?;
            println!("Removed the launcher's Rust toolchain and Cargo home.");
        }
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
                        "authentication": account::session::status(account::store(), &a.uuid),
                    })).collect::<Vec<_>>(),
                    "sign_in_configured": account::client_id().is_some(),
                    "owner": accounts.has_owner(account::store()),
                }));
            } else if accounts.accounts.is_empty() {
                println!(
                    "No Microsoft account signed in. Play needs one that owns Minecraft: Java \
                     Edition: horizon-launcher account sign-in"
                );
            } else {
                for a in &accounts.accounts {
                    let mark = if accounts.selected.as_ref() == Some(&a.uuid) {
                        "*"
                    } else {
                        " "
                    };
                    let status = account::session::status(account::store(), &a.uuid);
                    // Why it couldn't be checked (a keyring that didn't answer, #12179).
                    let why = match status.state {
                        account::session::State::Retry => format!(": {}", status.message),
                        _ => String::new(),
                    };
                    println!("{mark} {}  {}  {}{why}", a.name, a.uuid, status.label());
                }
                if !accounts.has_owner(account::store()) {
                    println!(
                        "{} {}",
                        account::NoOwner::default(),
                        tr(backend::SIGN_IN_HINT)
                    );
                }
            }
        }
        AccountCmd::Refresh { who } => {
            let status = account::session::refresh(&who)?;
            if as_json {
                json(serde_json::to_value(&status).map_err(|e| e.to_string())?);
            } else if status.state != account::session::State::Ready {
                return Err(format!("{}: {}", status.label(), status.message));
            } else {
                println!("{}: {}", status.label(), status.message);
            }
        }
        AccountCmd::Session { who } => {
            let dir = backend::game_home().join("sessions");
            match (account::session::renew_for_game(&dir, &who), as_json) {
                (Ok(file), true) => json(serde_json::json!({ "session_file": file })),
                (Ok(file), false) => println!("{}", file.display()),
                (Err(status), true) => json(serde_json::json!({ "authentication": status })),
                (Err(status), false) => return Err(status.message),
            }
        }
        AccountCmd::SignIn { target: Some(who) } => {
            let a = find(&accounts, &who)?;
            let sign_in = account::SignIn::start_for(a.clone());
            let mut shown = false;
            let signed_in = loop {
                match sign_in.state() {
                    account::SignInState::Code(code) if !shown => {
                        shown = true;
                        if as_json {
                            json(serde_json::json!({
                                "link": code.verification_uri,
                                "code": code.user_code,
                            }));
                        } else {
                            eprintln!(
                                "To sign {} in again, open {} in a browser and enter the code {}",
                                a.name, code.verification_uri, code.user_code
                            );
                        }
                    }
                    account::SignInState::Done(result) => break result?,
                    _ => {}
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            };
            if as_json {
                json(serde_json::json!({ "signed_in": signed_in.uuid }));
            } else {
                println!("Signed in as {} again.", signed_in.name);
            }
        }
        AccountCmd::SignIn { target: None } => {
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
            if let Some(said) = account::no_keychain() {
                eprintln!("{said}");
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
            accounts.play_as(&a.uuid)?;
            accounts.save()?;
            println!("The game plays as {}.", a.name);
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
            offline,
            check_accounts: _,
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
                check_accounts: !offline,
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
/// Installs the native mods of `found` (a GitHub link, resolved) into the game folder `home`: the
/// warning and the exact source link first, and nothing downloaded without `trust`.
fn mods(action: ModsCmd, as_json: bool) -> Result<(), String> {
    use horizon_launcher::browser::{Kind, Query, SourceId, horizon, installed, java_instance};
    use horizon_launcher::modpacks;
    // What the browser installs goes into an instance's game folder; Java mods' checks are kept
    // in the default one, shared by all.
    let instance = match &action {
        ModsCmd::List { instance }
        | ModsCmd::Search { instance, .. }
        | ModsCmd::Install { instance, .. }
        | ModsCmd::Update { instance, .. }
        | ModsCmd::Remove { instance, .. }
        | ModsCmd::AddGithub { instance, .. }
        | ModsCmd::UpdateGithub { instance, .. }
        | ModsCmd::Enable { instance, .. }
        | ModsCmd::Disable { instance, .. } => match instance {
            Some(name) => Some(instances::find(name)?),
            None => Some(instances::selected()),
        },
        ModsCmd::Check { .. } => None,
    };
    // A Java instance gets the source's own files, for its version and loader.
    let java = instance
        .as_ref()
        .and_then(|i| i.java.as_ref())
        .map(|j| j.target());
    let home = match (&action, &instance) {
        (ModsCmd::Search { .. }, _) | (_, None) => backend::game_home(),
        (_, Some(i)) => i.home.clone(),
    };
    let entry_json = |e: &installed::Entry| serde_json::to_value(e).unwrap_or_default();
    let line = |label: &str, f: f32| progress(label.to_string())(f);
    let from_registry = installed::Installed::load(&home)
        .entries
        .iter()
        .any(|e| e.source == SourceId::Horizon);
    if from_registry {
        if let Ok(index) = horizon::index() {
            for said in horizon::turn_off_revoked(&index, &home) {
                eprintln!("{said}");
            }
        }
    }
    let on = matches!(action, ModsCmd::Enable { .. });
    match action {
        ModsCmd::List { .. } => {
            let list = installed::Installed::load(&home).entries;
            if as_json {
                json(list.iter().map(entry_json).collect());
            } else if list.is_empty() && github_mods::origins(&home).is_empty() {
                println!(
                    "Nothing installed from the browser yet: horizon-launcher mods search <text>"
                );
            }
            for o in horizon_launcher::github_mods::origins(&home)
                .iter()
                .filter(|_| !as_json && java.is_none())
            {
                println!(
                    "{} {}  (native mod, from GitHub: {})\n  {}",
                    o.id,
                    o.version,
                    o.label(),
                    o.source_url()
                );
            }
            for e in list.iter().filter(|_| !as_json) {
                let off = java.is_some() && !java_instance::is_on(&home, e);
                println!(
                    "{}{}  {}  ({}, {})\n  {}  {}",
                    e.title,
                    if off { " (off)" } else { "" },
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
            ..
        } => {
            let kind = match kind {
                KindArg::Resource => Kind::ResourcePack,
                KindArg::Modpack => Kind::Modpack,
                KindArg::Mod => Kind::Mod,
                KindArg::Shader => Kind::Shader,
                KindArg::Datapack => Kind::DataPack,
            };
            if let Some(target) = &java {
                let found = source.get()?.search(&target.query(&text.join(" "), kind))?;
                if as_json {
                    json(found.iter().map(|p| serde_json::json!({
                        "id": p.id, "slug": p.slug, "title": p.title, "summary": p.summary,
                        "author": p.author, "downloads": p.downloads, "page": p.page,
                        "license": p.license,
                    })).collect());
                }
                for p in found.iter().filter(|_| !as_json) {
                    println!(
                        "{}  ({})  {} downloads\n  {}",
                        p.title, p.slug, p.downloads, p.summary
                    );
                }
                return Ok(());
            }
            let query = Query {
                everything,
                ..Query::new(&text.join(" "), kind)
            };
            let found = source.get()?.search(&query)?;
            let datapack = kind == Kind::DataPack;
            if as_json {
                json(found.iter().map(|p| {
                    let support = p.support();
                    serde_json::json!({
                        "id": p.id, "slug": p.slug, "title": p.title, "summary": p.summary,
                        "author": p.author, "downloads": p.downloads, "page": p.page,
                        "license": p.license, "support": support.label(), "reason": support.reason(),
                        "about": if p.source == SourceId::Horizon { horizon::trust_and_capabilities(&p.id, instance.as_ref()) } else { Vec::new() },
                    })
                }).collect());
            } else {
                if datapack && !found.is_empty() {
                    println!("{}\n", horizon_launcher::browser::datapack_support());
                }
                for p in &found {
                    let support = p.support();
                    if p.source == SourceId::Horizon {
                        println!("{}  ({})  {}\n  {}", p.title, p.slug, p.author, p.summary);
                        for line in horizon::trust_and_capabilities(&p.id, instance.as_ref()) {
                            println!("  {line}");
                        }
                        continue;
                    }
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
                    } else if datapack {
                        println!("  horizon-launcher mods install {} --world <world>", p.slug);
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
            world,
            ..
        } => {
            let source = source.get()?;
            let p = source.project(&project)?;
            if let Some(world) = world {
                let world = horizon_launcher::worlds::find(&home, &world)?;
                let done = installed::install_datapack(
                    &*source,
                    &p,
                    everything,
                    &world.path,
                    &progress(p.title.clone()),
                )?;
                if as_json {
                    json(serde_json::json!({
                        "world": world.folder, "file": done.pack.file, "version": done.number,
                        "runs": done.coverage.read, "not_read": done.coverage.unread,
                    }));
                } else {
                    println!(
                        "{} {} is in {}'s data packs as {}: the game turns it on when the world \
                         opens. {}",
                        p.title,
                        done.number,
                        world.name,
                        done.pack.file,
                        done.coverage.summary()
                    );
                }
                return Ok(());
            }
            if p.kind == Kind::DataPack {
                return Err(format!(
                    "{} is a data pack: say which world with --world.",
                    p.title
                ));
            }
            if p.kind == Kind::Modpack && p.source == SourceId::Horizon {
                let pack = horizon::pack::install(&p.id, &line)?;
                if as_json {
                    json(
                        serde_json::json!({"pack": pack.id, "version": pack.version, "mods": pack.mods, "home": horizon::pack::home(&pack.work)}),
                    );
                } else {
                    println!(
                        "{} {} is installed as its own profile, with {}.",
                        pack.name,
                        pack.version,
                        pack.mods.join(", ")
                    );
                    println!("Play it: horizon-launcher modpack play {}", pack.id);
                }
                return Ok(());
            }
            if p.kind == Kind::Modpack {
                let folder = modpacks::ports_dir().join("downloads");
                let file =
                    installed::fetch_modpack(&*source, &p, &folder, &progress(p.title.clone()))?;
                let log = &|line: String| eprintln!("{line}");
                let resources = backend::resolve_version(None)
                    .ok()
                    .filter(|t| backend::check_prepared(t))
                    .map(|t| backend::resources(&t));
                let pack = modpacks::import_and_port(&file, resources.as_deref(), log)?;
                println!("{} ({})", modpacks::outcome(&pack), pack.summary());
                println!("Play it: horizon-launcher modpack play {}", pack.slug);
                return Ok(());
            }
            let done = match &java {
                Some(target) => java_instance::install(&*source, target, &p, &home, &line)?,
                None => installed::install(&*source, &p, everything, &home, &line)?,
            };
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
        ModsCmd::Update { project, check, .. } => {
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
                let newer = match &java {
                    Some(target) => java_instance::update_for(&*source, target, e)?,
                    None => installed::update_for(&*source, e)?,
                };
                match newer {
                    None => println!("{} {}: up to date.", e.title, e.number),
                    Some(v) if check => {
                        println!("{} {}: {} is available.", e.title, e.number, v.number)
                    }
                    Some(_) => {
                        let p = source.project(&e.project)?;
                        let done = match &java {
                            Some(target) => {
                                java_instance::install(&*source, target, &p, &home, &line)?
                            }
                            None => installed::install(&*source, &p, true, &home, &line)?,
                        };
                        for done in done {
                            println!("{} {}: {}", done.title, done.number, done.file);
                        }
                    }
                }
            }
        }
        ModsCmd::AddGithub {
            url,
            mod_id,
            all,
            trust_native_code,
            ..
        } => {
            let link = github_mods::parse(&url)?;
            let found = github_mods::resolve(&link, &github_mods::api)?;
            github_native(
                &home,
                java.is_some(),
                &found,
                mod_id.as_deref(),
                all,
                trust_native_code,
            )?;
        }
        ModsCmd::UpdateGithub {
            id,
            check,
            trust_native_code,
            ..
        } => {
            if java.is_some() {
                return Err("Native mods from GitHub are for Horizon instances.".into());
            }
            let mut origins = github_mods::origins(&home);
            origins.retain(|o| id.as_deref().is_none_or(|i| o.id == i));
            if origins.is_empty() {
                return Err(match id {
                    Some(i) => format!("{i} wasn't installed from GitHub."),
                    None => "No native mod was installed from GitHub: horizon-launcher mods add-github <url>".into(),
                });
            }
            for o in origins {
                let latest = github_mods::resolve(&o.follows(), &github_mods::api)?;
                if !github_mods::is_update(&o, &latest) {
                    println!("{} {}: up to date ({}).", o.id, o.version, o.label());
                    continue;
                }
                println!(
                    "{}: {} is available (now {}).",
                    o.id,
                    latest.label(),
                    o.label()
                );
                if !check {
                    github_native(&home, false, &latest, Some(&o.id), false, trust_native_code)?;
                }
            }
        }
        ModsCmd::Remove { project, .. }
            if java.is_none() && github_mods::origin(&home, &project).is_some() =>
        {
            github_mods::remove(&home, &project)?;
            println!("Removed {project} (installed from GitHub).");
        }
        ModsCmd::Remove { project, .. } => {
            let e = match java {
                Some(_) => java_instance::remove(&home, &project)?,
                None => installed::remove(&home, &project)?,
            };
            println!("Removed {} ({}).", e.title, e.file);
        }
        ModsCmd::Enable { project, .. } | ModsCmd::Disable { project, .. } if java.is_some() => {
            let e = installed::Installed::load(&home)
                .lookup(&project)
                .cloned()
                .ok_or_else(|| format!("{project} wasn't installed by the browser."))?;
            java_instance::set_on(&home, &e, on)?;
            println!("{} is {}.", e.title, if on { "on" } else { "off" });
        }
        ModsCmd::Enable { project, .. } | ModsCmd::Disable { project, .. } => {
            let e = installed::Installed::load(&home)
                .lookup(&project)
                .cloned()
                .ok_or_else(|| format!("{project} wasn't installed by the browser."))?;
            let id = horizon::mod_id(&e)
                .filter(|_| e.kind == Kind::Mod)
                .ok_or_else(|| format!("{} isn't a mod: only mods turn on and off.", e.title))?;
            if on {
                if let Some(r) = horizon::index()
                    .ok()
                    .filter(|_| e.source == SourceId::Horizon)
                    .and_then(|i| {
                        i.revoked(&e.project, e.checksum.trim_start_matches("SHA-256:"))
                            .cloned()
                    })
                {
                    return Err(format!("{} was taken down: {}", e.title, r.reason));
                }
            }
            horizon_launcher::browser::java::set_enabled(&home, id, on)?;
            println!("{} is {}.", e.title, if on { "on" } else { "off" });
        }
    }
    Ok(())
}
/// The other launchers' instances in `folder`, else in their default folders.
fn other_instances(folder: Option<&Path>) -> Result<Vec<other_launchers::Found>, String> {
    use horizon_launcher::sources::{Folders, Os};
    Ok(match folder {
        Some(dir) if !dir.is_dir() => return Err(format!("{} isn't a folder.", dir.display())),
        Some(dir) => other_launchers::in_folder(dir, None),
        None => {
            let folders = Folders::current().ok_or("No home folder.")?;
            other_launchers::scan(Os::current(), &folders)
        }
    })
}
/// Where `instance others` looked when it found nothing, and what it looks for.
fn no_other_instances(folder: Option<&Path>) -> String {
    use horizon_launcher::sources::{Folders, Os};
    match folder {
        Some(dir) => format!(
            "No other launcher's instance found in {}. Name a launcher's own folder (the \
             official launcher's .minecraft, Prism Launcher, MultiMC, ATLauncher, the Modrinth \
             App, the CurseForge app's minecraft folder), a folder of instances (instances, or \
             the CurseForge app's Instances), or one instance's folder.",
            dir.display()
        ),
        None => {
            let looked: String = Folders::current()
                .map(|f| other_launchers::scanned_folders(Os::current(), &f))
                .unwrap_or_default()
                .iter()
                .map(|d| format!("\n    {}", d.display()))
                .collect();
            format!(
                "No other launcher's instance found. Looked in:{looked}\nKeep yours somewhere \
                 else? Name that folder with --folder."
            )
        }
    }
}
fn other_plays(f: &other_launchers::Found) -> String {
    match (&f.minecraft, &f.loader) {
        (None, _) => "Minecraft version not recorded: copy-from --minecraft <version>".into(),
        (Some(v), Some(l)) => format!("Minecraft {v} with {} {}", l.name, l.version),
        (Some(v), None) => format!("Minecraft {v}"),
    }
}
fn other_json(f: &other_launchers::Found) -> serde_json::Value {
    serde_json::json!({
        "launcher": f.launcher, "name": f.name, "minecraft": f.minecraft,
        "loader": f.loader.as_ref().map(|l| l.name),
        "loader_version": f.loader.as_ref().map(|l| &l.version),
        "game": f.game, "shared": f.shared,
    })
}
/// Runs the instance's post-exit command in this terminal, if it has one, and waits for it.
fn after_exit(instance: &instances::Instance, vars: &[(&'static str, std::ffi::OsString)]) {
    let Some(mut cmd) =
        horizon_launcher::launch::after_exit(&instance.launch, &instance.home, vars)
    else {
        return;
    };
    match cmd.status() {
        Ok(s) if s.success() => {}
        Ok(s) => eprintln!("The post-exit command failed ({s})."),
        Err(e) => eprintln!("The post-exit command didn't run: {e}"),
    }
}

/// `1280x720` (or `1280×720`), or `default` for none.
fn parse_window(text: &str) -> Result<Option<(u32, u32)>, String> {
    let text = text.trim();
    if text.eq_ignore_ascii_case("default") {
        return Ok(None);
    }
    let size = text
        .split_once(['x', 'X', '×'])
        .and_then(|(w, h)| Some((w.trim().parse().ok()?, h.trim().parse().ok()?)))
        .filter(|&(w, h): &(u32, u32)| (1..=16384).contains(&w) && (1..=16384).contains(&h));
    match size {
        Some(size) => Ok(Some(size)),
        None => Err(format!(
            "{text:?} isn't a window size: 1280x720, or default."
        )),
    }
}

fn instance(action: InstanceCmd, as_json: bool) -> Result<(), String> {
    use instances::Icon;
    let value = |i: &instances::Instance, selected: bool| {
        serde_json::json!({
            "id": i.id, "name": i.name, "kind": format!("{:?}", i.kind).to_lowercase(),
            "home": i.home, "icon": i.icon, "group": i.group, "selected": selected,
            "version": i.version, "version_label": i.version.as_deref().and_then(horizon_launcher::developer_install::label), "java": i.java, "played_secs": i.played.as_secs(),
            "last_played_ms": i.last_played.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64),
        })
    };
    let line = |i: &instances::Instance, selected: bool| {
        let mark = if selected { "*" } else { " " };
        let group = i
            .group
            .as_ref()
            .map(|g| format!("  [{g}]"))
            .unwrap_or_default();
        let pinned = match (&i.java, &i.version) {
            (Some(java), _) => match &java.loader {
                Some(l) => format!(
                    "  Minecraft {} (Java Edition) with {} {}",
                    java.minecraft,
                    l.kind.name(),
                    l.version
                ),
                None => format!("  Minecraft {} (Java Edition)", java.minecraft),
            },
            (None, Some(v)) => format!(
                "  {}",
                horizon_launcher::developer_install::label(v).unwrap_or_else(|| v.clone())
            ),
            (None, None) => String::new(),
        };
        let played = match i.last_played {
            Some(_) => format!("  {} played", instances::play_time(i.played)),
            None => String::new(),
        };
        println!(
            "{mark} {} ({}){pinned}{group}{played}\n    {}",
            i.name,
            i.id,
            i.home.display()
        );
    };
    let changed = |i: instances::Instance| {
        let selected = i.id == instances::selected().id;
        match as_json {
            true => json(value(&i, selected)),
            false => line(&i, selected),
        }
    };
    match action {
        InstanceCmd::List => {
            let selected = instances::selected().id;
            let all = instances::list();
            if as_json {
                json(all.iter().map(|i| value(i, i.id == selected)).collect());
            } else {
                all.iter().for_each(|i| line(i, i.id == selected));
            }
        }
        InstanceCmd::Create {
            name,
            copy,
            minecraft: None,
            ..
        } => changed(instances::create(&name, copy.as_deref())?),
        InstanceCmd::Create {
            name,
            minecraft: Some(version),
            loader,
            loader_version,
            ..
        } => {
            let versions = horizon_launcher::minecraft_versions::load()?;
            let id = &versions.find(&version)?.id;
            // The loader first: a version it doesn't have makes no instance.
            let loader = loader
                .map(|kind| loaders::choose(&kind, id, loader_version.as_deref()))
                .transpose()?;
            let made = instances::create_java(&name, id)?;
            match loader {
                Some(loader) => changed(instances::edit(&made.id, |m| {
                    m.java.get_or_insert_with(Default::default).loader = Some(loader)
                })?),
                None => changed(made),
            }
        }
        InstanceCmd::Loader {
            instance,
            loader,
            version,
            list,
        } => {
            let Some(java) = instances::find(&instance)?.java else {
                return Err(format!(
                    "{instance} is a Horizon instance: mod loaders are for Java instances."
                ));
            };
            if list {
                let kind = loaders::Kind::parse(&loader)?;
                let listed = loaders::versions(kind, &java.minecraft, &backend::get_bytes)?;
                if as_json {
                    json(serde_json::json!(listed));
                } else {
                    for l in listed {
                        println!("{}{}", l.version, if l.stable { "" } else { "  (beta)" });
                    }
                }
                return Ok(());
            }
            let loader = match loader.trim() {
                "none" => None,
                kind => Some(loaders::choose(kind, &java.minecraft, version.as_deref())?),
            };
            changed(instances::edit(&instance, |m| {
                m.java.get_or_insert_with(Default::default).loader = loader
            })?)
        }
        InstanceCmd::Versions { snapshots, old } => {
            let versions = horizon_launcher::minecraft_versions::load()?;
            let shown: Vec<_> = versions.of(snapshots, old).collect();
            if as_json {
                json(serde_json::json!(shown));
            } else {
                for v in shown {
                    let date = v.released.get(..10).unwrap_or("");
                    println!("{}  {date}  {:?}", v.id, v.kind);
                }
            }
        }
        InstanceCmd::Java {
            instance,
            home,
            auto,
            memory,
            args,
            window,
        } => {
            let window = window.as_deref().map(parse_window).transpose()?;
            if instances::find(&instance)?.java.is_none() {
                return Err(format!(
                    "{instance} is a Horizon instance: Horizon is not a Java program."
                ));
            }
            changed(instances::edit(&instance, |m| {
                let java = m.java.get_or_insert_with(Default::default);
                if auto {
                    java.home = None;
                }
                java.home = home.or(java.home.take());
                java.memory_mb = memory.or(java.memory_mb);
                if let Some(args) = args {
                    java.jvm_args = args.trim().to_string();
                }
                if let Some(window) = window {
                    java.window = window;
                }
            })?)
        }
        InstanceCmd::Delete { instance } => {
            let gone = instances::delete(&instance)?;
            match as_json {
                true => json(serde_json::json!({"deleted": gone.id, "trashed": gone.work})),
                false => println!("{} is in the Trash.", gone.name),
            }
        }
        InstanceCmd::Rename { instance, name } => {
            changed(instances::edit(&instance, |m| m.name = Some(name))?)
        }
        InstanceCmd::Icon { instance, icon } => {
            let icon = (icon != "none").then(|| Icon::parse(&icon)).transpose()?;
            changed(instances::edit(&instance, |m| m.icon = icon)?)
        }
        InstanceCmd::Group { instance, group } => {
            changed(instances::edit(&instance, |m| m.group = group)?)
        }
        InstanceCmd::Select { instance } => changed(instances::select(&instance)?),
        InstanceCmd::Version { instance, version } => {
            let pin = match version.as_str() {
                "latest" => None,
                tag => Some(backend::resolve_version(Some(tag))?),
            };
            changed(instances::edit(&instance, |m| m.version = pin)?)
        }
        InstanceCmd::Launch {
            instance,
            pre_launch,
            wrapper,
            env,
            gamemode,
            mangohud,
            dedicated_gpu,
            post_exit,
        } => {
            use horizon_launcher::launch;
            let env = env.as_deref().map(launch::parse_env).transpose()?;
            let [gamemode, mangohud, dedicated_gpu] =
                [gamemode, mangohud, dedicated_gpu].map(|v| v.as_deref().map(on_off));
            let [gamemode, mangohud, dedicated_gpu] = [
                gamemode.transpose()?,
                mangohud.transpose()?,
                dedicated_gpu.transpose()?,
            ];
            let found = launch::Found::here();
            for (on, has, name) in [
                (gamemode, found.gamemode, "GameMode (gamemoderun)"),
                (mangohud, found.mangohud, "MangoHud"),
                (dedicated_gpu, found.two_gpus, "A second graphics card"),
            ] {
                if on == Some(true) && !has {
                    eprintln!(
                        "{name} isn't on this computer: the switch does nothing until it is."
                    );
                }
            }
            let changing = pre_launch.is_some()
                || wrapper.is_some()
                || post_exit.is_some()
                || env.is_some()
                || gamemode.is_some()
                || mangohud.is_some()
                || dedicated_gpu.is_some();
            let i = match changing {
                false => instances::find(&instance)?,
                true => instances::edit(&instance, |m| {
                    if let Some(p) = pre_launch {
                        m.launch.pre_launch = p.trim().to_string();
                    }
                    if let Some(w) = wrapper {
                        m.launch.wrapper = w.trim().to_string();
                    }
                    if let Some(p) = post_exit {
                        m.launch.post_exit = p.trim().to_string();
                    }
                    if let Some(e) = env {
                        m.launch.env = e;
                    }
                    if let Some(on) = gamemode {
                        m.launch.gamemode = on;
                    }
                    if let Some(on) = mangohud {
                        m.launch.mangohud = on;
                    }
                    if let Some(on) = dedicated_gpu {
                        m.launch.dedicated_gpu = on;
                    }
                })?,
            };
            let l = &i.launch;
            if as_json {
                json(serde_json::json!({
                    "instance": i.id, "pre_launch": l.pre_launch, "wrapper": l.wrapper, "env": l.env,
                    "post_exit": l.post_exit,
                    "gamemode": l.gamemode, "mangohud": l.mangohud, "dedicated_gpu": l.dedicated_gpu,
                    "found": {
                        "gamemode": found.gamemode, "mangohud": found.mangohud,
                        "two_gpus": found.two_gpus, "nvidia": found.nvidia,
                    },
                }));
            } else {
                println!(
                    "{}: {}",
                    i.name,
                    l.summary().unwrap_or("nothing around the game".into())
                );
                println!("  pre-launch: {}", l.pre_launch);
                println!("  wrapper:    {}", l.wrapper);
                println!("  env:        {}", launch::env_line(&l.env));
                println!("  post-exit:  {}", l.post_exit);
                let yes = |b: bool| if b { "on" } else { "off" };
                for (name, on, has) in [
                    ("gamemode:", l.gamemode, found.gamemode),
                    ("mangohud:", l.mangohud, found.mangohud),
                    ("gpu:", l.dedicated_gpu, found.two_gpus),
                ] {
                    let note = if has { "" } else { " (not on this computer)" };
                    println!("  {name:<11} {}{note}", yes(on));
                }
            }
        }
        InstanceCmd::Shortcut { instance, remove } => {
            use horizon_launcher::shortcut;
            let i = match instance {
                Some(name) => instances::find(&name)?,
                None => instances::selected(),
            };
            let paths = match remove {
                false => shortcut::make(&backend::update_target(), &i.id, &i.name)?,
                true => shortcut::remove(&i.id)?,
            };
            if as_json {
                json(serde_json::json!({ "instance": i.id, "paths": paths }));
            } else if remove && paths.is_empty() {
                println!("{} has no shortcut.", i.name);
            } else {
                let did = if remove { "Moved to the trash" } else { "Made" };
                for path in paths {
                    println!("{did}: {}", path.display());
                }
            }
        }
        InstanceCmd::Others { folder } => {
            let found = other_instances(folder.as_deref())?;
            if as_json {
                json(found.iter().map(other_json).collect());
            } else if found.is_empty() {
                println!("{}", no_other_instances(folder.as_deref()));
            }
            for f in found.iter().filter(|_| !as_json) {
                println!(
                    "{}  ({}, {})\n    {}",
                    f.name,
                    f.launcher,
                    other_plays(f),
                    f.game.display()
                );
            }
        }
        InstanceCmd::CopyFrom {
            name,
            folder,
            new_name,
            minecraft,
        } => {
            let found = other_instances(folder.as_deref())?;
            let matching: Vec<_> = found.iter().filter(|f| f.name == name).collect();
            let f = match matching[..] {
                [one] => one,
                [] => {
                    return Err(format!(
                        "No other launcher has an instance called {name}: \
                         horizon-launcher instance others"
                    ));
                }
                _ => {
                    return Err(format!(
                        "More than one launcher has an instance called {name}: say which \
                         with --folder."
                    ));
                }
            };
            let target = new_name.as_deref().unwrap_or(&f.name);
            let made = other_launchers::import(f, target, minecraft.as_deref()).map_err(|e| {
                if e == instances::name_taken(target.trim()) {
                    instances::name_taken_with_flag(target.trim())
                } else {
                    e
                }
            })?;
            let unsupported = other_launchers::loader(f).is_none();
            if let Some(l) = f.loader.as_ref().filter(|_| unsupported && !as_json) {
                println!(
                    "{} {} isn't installed for it yet: its mods are copied and wait for it.",
                    l.name, l.version
                );
            }
            changed(made);
        }
        InstanceCmd::Import {
            file, java: true, ..
        } => {
            let i =
                horizon_launcher::instances_mrpack::import(&file, &|line, _| eprintln!("{line}"))?;
            if as_json {
                json(value(&i, false));
            } else {
                println!(
                    "Made the Java instance {} in {}: horizon-launcher play --instance \"{}\"",
                    i.name,
                    i.home.display(),
                    i.name
                );
            }
        }
        InstanceCmd::Import { file, instance, .. } => {
            use horizon_launcher::import::{self, Dropped};
            use horizon_launcher::modpacks;
            match import::what(&file)? {
                Dropped::Instance => {
                    let i = import::import_instance(&file, &|line, _| eprintln!("{line}"))?;
                    if as_json {
                        json(value(&i, false));
                    } else {
                        println!(
                            "Made the instance {} in {}: horizon-launcher instance select \"{}\"",
                            i.name,
                            i.home.display(),
                            i.name
                        );
                    }
                }
                Dropped::Modpack => {
                    let pack = modpacks::import(&file, &|line: String| eprintln!("{line}"))?;
                    println!("{} ({})", modpacks::outcome(&pack), pack.summary());
                    println!(
                        "Port and play it: horizon-launcher modpack port {}",
                        pack.slug
                    );
                }
                // A game build is its own command: it installs a version
                // rather than touching an instance, and takes a `--version` to
                // say which release's game files it plays with.
                Dropped::Build => {
                    let tag = horizon_launcher::build_import::import(&file, None)?;
                    if as_json {
                        json(serde_json::json!({ "version": tag }));
                    } else {
                        println!("Imported the build as {tag}, and selected it.");
                    }
                }
                Dropped::Pack(_) => {
                    let i = match instance {
                        Some(name) => instances::find(&name)?,
                        None => instances::selected(),
                    };
                    let p = horizon_launcher::packs::add(&i.home, &file)?;
                    println!(
                        "Added {} to {}: turn it on with horizon-launcher packs on",
                        p.title, i.name
                    );
                }
                Dropped::DataPack => {
                    return Err(format!(
                        "{} is a data pack: put it into a world with horizon-launcher worlds \
                         datapack-add <world> <file>",
                        file.display()
                    ));
                }
            }
        }
        InstanceCmd::Export {
            instance,
            format,
            world,
            out,
        } => {
            use horizon_launcher::export::{self, Format};
            let i = match instance {
                Some(name) => instances::find(&name)?,
                None => instances::selected(),
            };
            let format = match format {
                ExportFormat::Mrpack => Format::Mrpack,
                ExportFormat::Zip => Format::Zip,
            };
            let worlds = world
                .iter()
                .map(|w| horizon_launcher::worlds::find(&i.home, w).map(|w| w.folder))
                .collect::<Result<Vec<_>, _>>()?;
            let linked = match format {
                Format::Mrpack => export::linkable(&i.home, &export::from_modrinth),
                Format::Zip => Vec::new(),
            };
            let out = match out {
                Some(out) => out,
                None => export::free_path(
                    &std::env::current_dir().map_err(|e| e.to_string())?,
                    &i.name,
                    format,
                ),
            };
            let done = export::export(&i.home, &i.name, format, &worlds, &linked, &out)?;
            if as_json {
                json(serde_json::json!({
                    "path": done.path, "copied": done.copied, "linked": done.linked,
                    "worlds": done.worlds,
                }));
            } else {
                println!(
                    "Exported {} to {}: {} files, {} listed by their Modrinth download{}.",
                    i.name,
                    done.path.display(),
                    done.copied,
                    done.linked,
                    match done.worlds.is_empty() {
                        true => String::new(),
                        false => format!(", worlds: {}", done.worlds.join(", ")),
                    }
                );
            }
        }
    }
    Ok(())
}

fn game_packs(action: PacksCmd, as_json: bool) -> Result<(), String> {
    use horizon_launcher::packs::{self, Kind};
    let home = |instance: Option<String>| -> Result<PathBuf, String> {
        Ok(match instance {
            Some(name) => instances::find(&name)?.home,
            None => instances::selected().home,
        })
    };
    let value = |p: &packs::Pack| {
        serde_json::json!({
            "kind": format!("{:?}", p.kind).to_lowercase(), "file": p.file, "title": p.title,
            "description": p.description, "enabled": p.enabled, "path": p.path,
        })
    };
    match action {
        PacksCmd::List { kind, instance } => {
            let home = home(instance)?;
            let kinds = match kind {
                Some(k) => vec![Kind::from(k)],
                None => vec![Kind::Resource, Kind::Shader],
            };
            let all: Vec<packs::Pack> = kinds.iter().flat_map(|k| packs::list(&home, *k)).collect();
            if as_json {
                json(all.iter().map(value).collect());
            }
            for k in kinds.iter().filter(|_| !as_json) {
                println!("{}:", k.folder());
                let these: Vec<_> = all.iter().filter(|p| p.kind == *k).collect();
                if these.is_empty() {
                    println!("    none");
                }
                for p in these {
                    let on = if p.enabled { "on " } else { "off" };
                    match p.title == p.file {
                        true => println!("  {on} {}", p.file),
                        false => println!("  {on} {} ({})", p.title, p.file),
                    }
                }
            }
        }
        PacksCmd::Add { file, instance } => {
            let pack = packs::add(&home(instance)?, &file)?;
            match as_json {
                true => json(value(&pack)),
                false => println!("Added {} to {} (off).", pack.file, pack.kind.folder()),
            }
        }
        PacksCmd::On {
            pack,
            kind,
            instance,
        } => set_pack(&home(instance)?, kind.into(), &pack, true)?,
        PacksCmd::Off {
            pack,
            kind,
            instance,
        } => set_pack(&home(instance)?, kind.into(), &pack, false)?,
        PacksCmd::Remove {
            pack,
            kind,
            instance,
        } => {
            let home = home(instance)?;
            let p = packs::find(&home, kind.into(), &pack)?;
            packs::remove(&home, &p)?;
            println!("{} is in the trash.", p.file);
        }
    }
    Ok(())
}

fn set_pack(
    home: &std::path::Path,
    kind: horizon_launcher::packs::Kind,
    query: &str,
    on: bool,
) -> Result<(), String> {
    use horizon_launcher::packs;
    let pack = packs::find(home, kind, query)?;
    packs::set_enabled(home, &pack, on)?;
    println!("{} is {}.", pack.title, if on { "on" } else { "off" });
    Ok(())
}

fn screenshots(action: ScreenshotsCmd, as_json: bool) -> Result<(), String> {
    use horizon_launcher::screenshots;
    let home = |instance: Option<String>| -> Result<PathBuf, String> {
        Ok(match instance {
            Some(name) => instances::find(&name)?.home,
            None => instances::selected().home,
        })
    };
    match action {
        ScreenshotsCmd::List { instance } => {
            let home = home(instance)?;
            let shots = screenshots::list(&home);
            if as_json {
                json(
                    shots
                        .iter()
                        .map(|s| {
                            let millis = s
                                .taken
                                .duration_since(std::time::UNIX_EPOCH)
                                .map_or(0, |d| d.as_millis() as u64);
                            serde_json::json!({ "name": s.name, "path": s.path, "taken_ms": millis, "size": s.size })
                        })
                        .collect(),
                );
            } else if shots.is_empty() {
                println!("No screenshots in {}", screenshots::folder(&home).display());
            }
            for s in shots.iter().filter(|_| !as_json) {
                println!("{}", s.path.display());
            }
        }
        ScreenshotsCmd::Delete {
            screenshot,
            instance,
        } => {
            let shot = screenshots::find(&home(instance)?, &screenshot)?;
            screenshots::delete(&shot)?;
            println!("{} is in the trash.", shot.name);
        }
    }
    Ok(())
}

fn multiplayer(action: MultiplayerCmd, as_json: bool) -> Result<(), String> {
    let home = |instance: Option<String>| -> Result<PathBuf, String> {
        Ok(match instance {
            Some(name) => instances::find(&name)?.home,
            None => instances::selected().home,
        })
    };
    match action {
        MultiplayerCmd::List { instance } => {
            let home = home(instance)?;
            let servers = server_list::List::load(&home).servers();
            if as_json {
                json(
                    servers
                        .iter()
                        .map(|s| serde_json::json!({ "name": s.name, "address": s.ip, "icon": s.icon.is_some() }))
                        .collect(),
                );
            } else if servers.is_empty() {
                println!("No servers in {}", home.join("servers.dat").display());
            }
            for (i, s) in servers.iter().enumerate().filter(|_| !as_json) {
                println!("{}. {} ({})", i + 1, s.name, s.ip);
            }
        }
        MultiplayerCmd::Add {
            address,
            name,
            instance,
        } => {
            let home = home(instance)?;
            let mut list = server_list::List::load(&home);
            list.add(name.as_deref().unwrap_or(""), &address)?;
            list.save(&home)?;
            println!("Added {}.", address.trim());
        }
        MultiplayerCmd::Remove { server, instance } => {
            let home = home(instance)?;
            let mut list = server_list::List::load(&home);
            let (i, _) = server_list::find(&list, &server)?;
            let gone = list.remove(i)?;
            list.save(&home)?;
            println!("Removed {} ({}).", gone.name, gone.ip);
        }
    }
    Ok(())
}

fn worlds(action: WorldsCmd, as_json: bool) -> Result<(), String> {
    use horizon_launcher::datapacks;
    use horizon_launcher::worlds::{self, World};
    let home = |instance: Option<String>| -> Result<PathBuf, String> {
        Ok(match instance {
            Some(name) => instances::find(&name)?.home,
            None => instances::selected().home,
        })
    };
    let value = |w: &World| {
        let millis = w
            .last_played
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        serde_json::json!({
            "folder": w.folder, "name": w.name, "mode": w.mode, "version": w.version,
            "last_played_ms": millis, "size": w.size, "path": w.path, "icon": w.icon,
        })
    };
    match action {
        WorldsCmd::List { instance } => {
            let home = home(instance)?;
            let all = worlds::list(&home);
            if as_json {
                json(all.iter().map(value).collect());
            } else if all.is_empty() {
                println!("No worlds in {}", home.join("saves").display());
            }
            for w in all.iter().filter(|_| !as_json) {
                let version = w.version.as_deref().unwrap_or("unknown version");
                let mb = w.size as f64 / 1_000_000.0;
                println!(
                    "{} ({})\n    {}, {version}, {mb:.1} MB",
                    w.name, w.folder, w.mode
                );
            }
        }
        WorldsCmd::Backup { world, instance } => {
            let home = home(instance)?;
            let zip = worlds::backup(&home, &worlds::find(&home, &world)?)?;
            match as_json {
                true => json(serde_json::json!({ "backup": zip })),
                false => println!("{}", zip.display()),
            }
        }
        WorldsCmd::Copy { world, instance } => {
            let home = home(instance)?;
            let copy = worlds::copy(&worlds::find(&home, &world)?)?;
            match as_json {
                true => json(value(&copy)),
                false => println!("Copied to {} ({})", copy.name, copy.folder),
            }
        }
        WorldsCmd::Delete { world, instance } => {
            let home = home(instance)?;
            let w = worlds::find(&home, &world)?;
            worlds::delete(&w)?;
            println!("{} is in the trash.", w.name);
        }
        WorldsCmd::DatapackList { world, instance } => {
            let w = worlds::find(&home(instance)?, &world)?;
            let packs = datapacks::list(&w.path);
            if as_json {
                json(
                    packs
                        .iter()
                        .map(|p| serde_json::json!({ "file": p.file, "description": p.description, "enabled": p.enabled, "path": p.path }))
                        .collect(),
                );
            } else if packs.is_empty() {
                println!("No data packs in {}", w.name);
            }
            for p in packs.iter().filter(|_| !as_json) {
                let on = if p.enabled { "on " } else { "off" };
                println!("  {on} {}  {}", p.file, p.description);
            }
        }
        WorldsCmd::DatapackAdd {
            world,
            file,
            instance,
        } => {
            let w = worlds::find(&home(instance)?, &world)?;
            let p = datapacks::add(&w.path, &file)?;
            println!(
                "Added {} to {}: the game turns it on when the world opens.",
                p.file, w.name
            );
        }
        WorldsCmd::DatapackRemove {
            world,
            pack,
            instance,
        } => {
            let w = worlds::find(&home(instance)?, &world)?;
            let p = datapacks::find(&w.path, &pack)?;
            datapacks::remove(&w.path, &p)?;
            println!("{} is in the trash.", p.file);
        }
    }
    Ok(())
}

fn modpack(action: ModpackCmd, as_json: bool) -> Result<i32, String> {
    use horizon_launcher::browser::horizon;
    use horizon_launcher::modpacks::{self, Imported};
    use horizon_port_pack::report::Status;
    let log = &|line: String| eprintln!("{line}");
    let work = |pack: &str| {
        let registry = horizon::pack::packs_dir().join(pack);
        if horizon::pack::is_pack(&registry) {
            return Ok(registry);
        }
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
            println!(
                "  {} portable, {} carried over, {} data packs for new worlds",
                p.portable,
                p.mods.len(),
                p.datapacks
            );
            if let Some(jars) = p.jars() {
                println!("  {jars}");
            }
            println!("  {}", p.work.join("coverage.md").display());
        }
    };
    match action {
        ModpackCmd::List => {
            let packs = modpacks::list();
            let registry = horizon::pack::list();
            if packs.is_empty() && registry.is_empty() && !as_json {
                println!("No modpack imported yet: horizon-launcher modpack import <pack>");
            }
            packs.iter().for_each(show);
            for p in &registry {
                if as_json {
                    json(
                        serde_json::json!({"pack": p.id, "name": p.name, "version": p.version, "source": "horizon", "mods": p.mods}),
                    );
                } else {
                    println!(
                        "{} {}  ({}, from Horizon's registry)\n  {}",
                        p.name,
                        p.version,
                        p.id,
                        p.mods.join(", ")
                    );
                }
            }
        }
        ModpackCmd::Import { path } => show(&modpacks::import(&path, log)?),
        ModpackCmd::Port {
            pack,
            from,
            version,
        } => {
            let tag = backend::resolve_version(version.as_deref()).ok();
            let resources = tag
                .filter(|t| backend::check_prepared(t))
                .map(|t| backend::resources(&t));
            show(&modpacks::port(
                &work(&pack)?,
                resources.as_deref(),
                from.as_deref(),
                log,
            )?)
        }
        ModpackCmd::Play {
            pack,
            version,
            allow_jar_downloads,
        } => {
            let mut cmd = backend::client(&backend::resolve_version(version.as_deref())?)?;
            modpacks::apply_profile(&mut cmd, &work(&pack)?)?;
            horizon_launcher::jars::prepare_launch(
                &mut cmd,
                &Default::default(),
                horizon_launcher::jars::Permission::headless(allow_jar_downloads),
            )
            .map_err(|error| error.to_string())?;
            return attached(cmd);
        }
        ModpackCmd::Remove { pack } => modpacks::remove(&work(&pack)?)?,
    }
    Ok(0)
}

#[cfg(test)]
mod storage_tests;
#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn native_progress_limits_redirected_rust_downloads_to_start_and_end() {
        let progress = native_progress(false);
        let fractions = [0.0, 0.01, 0.97, 0.971, 0.974, 0.98, 1.0, 1.0];
        let labels: Vec<_> = fractions
            .into_iter()
            .filter_map(|done| progress(native_mods::Step::Rust(done)))
            .collect();
        assert_eq!(
            labels,
            [
                native_mods::Step::Rust(0.0).label(),
                native_mods::Step::Rust(1.0).label(),
            ]
        );
    }

    #[test]
    fn native_progress_keeps_detailed_terminal_downloads() {
        let progress = native_progress(true);
        for done in [0.0, 0.97, 0.971, 0.974, 0.98, 1.0] {
            let step = native_mods::Step::Rust(done);
            assert_eq!(progress(step.clone()), Some(step.label()));
        }
        for done in [0, 1, 97, 100] {
            let step = native_mods::Step::Tools {
                build: "sdk".into(),
                done,
                total: 100,
            };
            assert_eq!(progress(step.clone()), Some(step.label()));
        }
    }

    #[test]
    fn native_progress_limits_each_redirected_sdk_download() {
        let progress = native_progress(false);
        let tools = |build: &str, done, total| native_mods::Step::Tools {
            build: build.into(),
            done,
            total,
        };
        for (done, shown) in [
            (0, true),
            (1, false),
            (97, false),
            (100, true),
            (100, false),
        ] {
            let step = tools("first", done, 100);
            assert_eq!(progress(step.clone()), shown.then(|| step.label()));
        }
        let next = tools("second", 20, 100);
        assert_eq!(progress(next.clone()), Some(next.label()));
        assert_eq!(progress(tools("second", 50, 100)), None);
        let complete = tools("second", 100, 100);
        assert_eq!(progress(complete.clone()), Some(complete.label()));
        let unknown = tools("third", 0, 0);
        assert_eq!(progress(unknown.clone()), Some(unknown.label()));
        assert_eq!(progress(tools("third", 0, 0)), None);
        let complete = tools("third", 100, 100);
        assert_eq!(progress(complete.clone()), Some(complete.label()));
    }

    #[test]
    fn native_progress_keeps_phase_order_and_restarts_download_reporting() {
        use native_mods::Step;
        let progress = native_progress(false);
        let first = Step::Rust(0.97);
        assert_eq!(progress(first.clone()), Some(first.label()));
        assert_eq!(progress(Step::Rust(0.98)), None);
        for step in [
            Step::PrebuiltMods,
            Step::Compile {
                done: 1,
                total: Some(2),
                name: "first".into(),
                profile: Default::default(),
            },
            Step::Compile {
                done: 2,
                total: Some(2),
                name: "second".into(),
                profile: Default::default(),
            },
            Step::Install,
            Step::Rust(0.0),
            Step::Rust(1.0),
        ] {
            assert_eq!(progress(step.clone()), Some(step.label()));
        }
    }

    #[test]
    fn attached_preserves_real_process_codes_and_unix_signals() {
        let mut codes = vec![0, 37];
        #[cfg(unix)]
        codes.push(143);
        #[cfg(windows)]
        codes.push(0xFEDC_BA98u32 as i32);
        for code in codes {
            let root = tempfile::tempdir().unwrap();
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "cli::tests::attached_status_actor",
                    "--nocapture",
                ])
                .env("HORIZON_CLI_STATUS_ROLE", "wrapper")
                .env("HORIZON_CLI_STATUS_CODE", code.to_string())
                .env("HORIZON_LAUNCHER_NO_SELF_UPDATE", "1")
                .env("HORIZON_NO_AUDIO", "1")
                .current_dir(root.path())
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::inherit());
            for (key, folder) in [
                ("HORIZON_HOME", "logs"),
                ("HORIZON_LAUNCHER_HOME", "launcher"),
                ("XDG_DATA_HOME", "xdg-data"),
                ("XDG_CONFIG_HOME", "xdg-config"),
                ("HORIZON_LAUNCHER_TRASH", "trash"),
                ("HORIZON_LAUNCHER_CLIPBOARD", "clipboard"),
                ("HORIZON_LAUNCHER_DESKTOP", "desktop"),
            ] {
                command.env(key, root.path().join(folder));
            }
            assert_eq!(command.status().unwrap().code(), Some(code));
        }
    }

    #[test]
    fn attached_status_actor() {
        let Ok(role) = std::env::var("HORIZON_CLI_STATUS_ROLE") else {
            return;
        };
        let code: i32 = std::env::var("HORIZON_CLI_STATUS_CODE")
            .unwrap()
            .parse()
            .unwrap();
        if role == "child" {
            assert!(std::env::var_os(horizon_mod_package::holder::ENV).is_none());
            assert!(std::env::var_os(horizon_mod_package::holder::ENTRY).is_none());
            use std::io::IsTerminal;
            let shape = format!(
                "{}:{}:{}",
                std::io::stdin().is_terminal(),
                std::io::stdout().is_terminal(),
                std::io::stderr().is_terminal()
            );
            assert_eq!(shape, std::env::var("HORIZON_CLI_STDIO_SHAPE").unwrap());
            #[cfg(unix)]
            if code == 143 {
                unsafe {
                    libc::signal(libc::SIGTERM, libc::SIG_DFL);
                    libc::raise(libc::SIGTERM);
                }
                panic!("SIGTERM did not terminate the actor");
            }
            std::process::exit(code);
        }
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "cli::tests::attached_status_actor",
                "--nocapture",
            ])
            .env("HORIZON_CLI_STATUS_ROLE", "child");
        use std::io::IsTerminal;
        command.env(
            "HORIZON_CLI_STDIO_SHAPE",
            format!(
                "{}:{}:{}",
                std::io::stdin().is_terminal(),
                std::io::stdout().is_terminal(),
                std::io::stderr().is_terminal()
            ),
        );
        assert_eq!(attached(command).unwrap(), code);
        std::process::exit(code);
    }

    #[test]
    fn completions_cover_each_shell_and_nested_commands() {
        for shell in [
            clap_complete::Shell::Bash,
            clap_complete::Shell::Zsh,
            clap_complete::Shell::Fish,
            clap_complete::Shell::Elvish,
            clap_complete::Shell::PowerShell,
        ] {
            let shell_name = shell.to_string();
            let args = ["horizon-launcher", "completions", shell_name.as_str()].map(str::to_owned);
            let cli = Cli::try_parse_from(args).unwrap();
            assert!(matches!(cli.command, Some(Cmd::Completions { .. })));
            let mut output = Vec::new();
            clap_complete::generate(shell, &mut command(), "horizon-launcher", &mut output);
            let script = String::from_utf8(output).unwrap();
            assert!(script.contains("horizon-launcher"));
            assert!(script.contains("versions"));
            assert!(script.contains("multiplayer"));
        }
        assert!(Cli::try_parse_from(["horizon-launcher", "completions"]).is_err());
        assert!(Cli::try_parse_from(["horizon-launcher", "completions", "unknown"]).is_err());
    }

    #[test]
    fn settings_accepts_german_and_chinese() {
        use crate::i18n::Language;
        let mut settings = backend::Settings::default();
        for (code, language) in [("de", Language::German), ("zh", Language::Chinese)] {
            set_setting(&mut settings, "language", code).unwrap();
            assert_eq!(settings.language, Some(language));
        }
        assert_eq!(
            set_setting(&mut settings, "language", "xx").unwrap_err(),
            "xx: say system, en, fr, ru, de, zh, es, uk, pl or pt-br"
        );
        set_setting(&mut settings, "language", "system").unwrap();
        assert_eq!(settings.language, None);
    }

    #[test]
    fn play_takes_an_account_for_one_launch() {
        let cli = Cli::try_parse_from(["horizon-launcher", "play", "--account", "Alex"]).unwrap();
        match cli.command {
            Some(Cmd::Play { account, .. }) => assert_eq!(account.as_deref(), Some("Alex")),
            _ => panic!("not play"),
        }
    }

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

    #[test]
    fn window_sizes() {
        assert_eq!(parse_window("1280x720"), Ok(Some((1280, 720))));
        assert_eq!(parse_window(" 1920 × 1080 "), Ok(Some((1920, 1080))));
        assert_eq!(parse_window("Default"), Ok(None));
        assert!(parse_window("0x720").is_err());
        assert!(parse_window("big").is_err());
    }

    #[test]
    fn mods_commands_name_their_instance() {
        let parse = |args: &[&str]| {
            let cli = Cli::try_parse_from([&["horizon-launcher", "mods"], args].concat()).unwrap();
            match cli.command {
                Some(Cmd::Mods { action }) => action,
                _ => panic!("expected mods"),
            }
        };
        match parse(&["install", "fresh-animations", "--instance", "Skyblock"]) {
            Some(ModsCmd::Install {
                project, instance, ..
            }) => assert_eq!(
                (project.as_str(), instance.as_deref()),
                ("fresh-animations", Some("Skyblock"))
            ),
            _ => panic!("expected mods install"),
        }
        assert!(matches!(
            parse(&["list"]),
            Some(ModsCmd::List { instance: None })
        ));
        assert!(parse(&[]).is_none());
        match parse(&[
            "add-github",
            "https://github.com/a/b",
            "--mod",
            "zoom",
            "--all",
        ]) {
            Some(ModsCmd::AddGithub {
                url,
                mod_id,
                all,
                trust_native_code,
                ..
            }) => assert_eq!(
                (url.as_str(), mod_id.as_deref(), all, trust_native_code),
                ("https://github.com/a/b", Some("zoom"), true, false)
            ),
            _ => panic!("expected mods add-github"),
        }
        assert!(matches!(
            parse(&["update-github", "--check"]),
            Some(ModsCmd::UpdateGithub {
                check: true,
                id: None,
                ..
            })
        ));
        // Search names one for a Java instance's version and loader; Check reads none: checks
        // are shared.
        assert!(matches!(
            parse(&["search", "x", "--instance", "A"]),
            Some(ModsCmd::Search { instance: Some(i), .. }) if i == "A"
        ));
        assert!(
            Cli::try_parse_from(["horizon-launcher", "mods", "check", "x", "--instance", "A"])
                .is_err()
        );
    }

    #[test]
    fn native_build_sync_is_explicit_and_never_selects_an_instance() {
        let parse = |args: &[&str]| {
            Cli::try_parse_from(
                [
                    &["horizon-launcher", "native-build", "--sync-internals"],
                    args,
                ]
                .concat(),
            )
        };
        assert!(matches!(
            parse(&["mod", "--trust-native-code"]).unwrap().command,
            Some(Cmd::NativeBuild {
                sync_internals: true,
                folder: Some(_),
                ..
            })
        ));
        assert!(parse(&[]).is_err());
        for args in [
            ["mod", "--instance", "A"],
            ["mod", "--server-output", "out"],
            ["mod", "--profile", "developer"],
        ] {
            assert!(parse(&args).is_err());
        }
    }

    #[test]
    fn native_build_export_mode_needs_a_folder_and_refuses_an_instance() {
        let parse = |args: &[&str]| {
            Cli::try_parse_from(
                [
                    &["horizon-launcher", "native-build", "--trust-native-code"],
                    args,
                ]
                .concat(),
            )
        };
        // Export mode: a mod folder and an output folder.
        match parse(&["mods", "--server-output", "engine-out"])
            .unwrap()
            .command
        {
            Some(Cmd::NativeBuild {
                folder,
                server_output,
                instance,
                version,
                update,
                profile,
                ..
            }) => {
                assert_eq!(folder.as_deref(), Some(Path::new("mods")));
                assert_eq!(server_output.as_deref(), Some(Path::new("engine-out")));
                assert_eq!(instance, None);
                assert_eq!(version, None);
                assert!(!update);
                assert_eq!(profile, None);
            }
            _ => panic!("expected native-build"),
        }
        assert!(matches!(
            parse(&[
                "mods",
                "--server-output",
                "engine-out",
                "--allow-jar-downloads"
            ])
            .unwrap()
            .command,
            Some(Cmd::NativeBuild {
                allow_jar_downloads: true,
                ..
            })
        ));
        assert!(parse(&["mods", "--allow-jar-downloads"]).is_err());
        // Backing up worlds is for an instance's build; a server export switches nothing off.
        assert!(matches!(
            parse(&["--back-up-worlds"]).unwrap().command,
            Some(Cmd::NativeBuild {
                back_up_worlds: true,
                ..
            })
        ));
        assert!(parse(&["mods", "--server-output", "engine-out", "--back-up-worlds"]).is_err());
        // The mod folder is explicit in export mode: no default from an instance.
        assert!(parse(&["--server-output", "engine-out"]).is_err());
        // Export mode never touches an instance.
        assert!(parse(&["mods", "--server-output", "engine-out", "--instance", "A"]).is_err());
        // Export mode accepts an explicit version and profile; a version
        // and --update refuse each other, as in the game mode.
        match parse(&[
            "mods",
            "--server-output",
            "engine-out",
            "--version",
            "v2026.10.13",
            "--profile",
            "optimized",
        ])
        .unwrap()
        .command
        {
            Some(Cmd::NativeBuild {
                version,
                update,
                profile,
                ..
            }) => {
                assert_eq!(version.as_deref(), Some("v2026.10.13"));
                assert!(!update);
                assert_eq!(profile, Some(native_mods::Profile::Optimized));
            }
            _ => panic!("expected native-build"),
        }
        // --update on its own: the newest installed release or Nightly, never a native build.
        match parse(&["mods", "--server-output", "engine-out", "--update"])
            .unwrap()
            .command
        {
            Some(Cmd::NativeBuild { update, .. }) => assert!(update),
            _ => panic!("expected native-build"),
        }
        // A version refuses --update, as in the game mode.
        assert!(
            parse(&[
                "mods",
                "--server-output",
                "engine-out",
                "--version",
                "v2026.10.13",
                "--update"
            ])
            .is_err()
        );
        assert!(parse(&["mods", "--allow-version-mismatch"]).is_err());
        // The game mode is unchanged: an instance build without --server-output.
        assert!(matches!(
            parse(&["--instance", "A"]).unwrap().command,
            Some(Cmd::NativeBuild { instance: Some(i), server_output: None, .. }) if i == "A"
        ));
    }

    #[test]
    fn export_mode_base_is_an_installed_release_not_a_native_build() {
        // The export mode picks the newest installed release or Nightly, never a native build
        // (`native-*`), and `--update` changes nothing when it already is the newest.
        let installed = vec![
            "v2026.10.13".to_string(),
            "v2026.09.01".to_string(),
            "native-2b03c1".to_string(),
        ];
        for update in [false, true] {
            assert_eq!(
                native_mods::base_for(None, &installed, update).as_deref(),
                Some("v2026.10.13")
            );
        }
        assert_eq!(native_mods::base_for(None, &[], false), None);
    }
}

#[cfg(test)]
#[path = "cli_catalog_tests.rs"]
mod catalog_tests;

#[path = "cli_native_permission.rs"]
mod native_permission_impl;
use native_permission_impl::{github_native, native_permission, registry_consent};

mod developer_builds;

mod settings_output;
use settings_output::settings_json;
