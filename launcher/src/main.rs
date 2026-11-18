//! Horizon launcher: installs Horizon builds, prepares them from the player's own Minecraft,
//! and starts the client or a dedicated server.

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use horizon_launcher_scenery::{art, glass, rain, scene, sky};
mod blocks_chrome;
mod blocks_play;
mod blocks_settings;
mod blocks_versions;
mod launcher_messages;
mod launcher_play;
mod launcher_render;
mod launcher_setup;
mod widgets;
pub(crate) use widgets::*;
mod agents_page;
mod blocks;
mod build_mode;
mod cli;
mod cli_tools;
mod collection;
mod console;
mod console_page;
mod cull;
mod datapacks_page;
mod developer_play;
mod export_page;
mod frame;
mod game_tools;
mod github_mods_page;
mod glass_account;
mod glass_home;
mod glass_home_lists;
mod glass_home_mods;
mod glass_home_packs;
mod glass_home_tabs;
mod glass_instance;
mod glass_instance_java;
mod glass_mod_states;
mod glass_palette;
mod glass_settings;
mod glass_sheet;
mod glass_shot_blur;
mod glass_storage;
mod glass_toast;
mod glass_ui;
mod glass_versions;
mod home;
mod home_tabs;
mod import_page;
mod instance_icons;
mod instance_mods;
mod instances_page;
mod internet;
mod jar_launch;
mod language_page;
mod launch_page;
mod line_edit;
mod location_page;
mod markdown;
mod mcp;
mod microsoft;
mod mod_icons;
mod modpacks_page;
mod mods_page;
mod mods_project;
mod motion;
mod multiplayer_page;
mod native_consent;
mod native_log_page;
mod native_registry_consent;
mod nav_strip;
mod new_instance;
mod notes_cache;
mod owner_page;
mod packs_page;
mod partner;
mod perf;
mod picture_cache;
mod plate;
mod profile_heads;
mod screenshots_page;
mod scrollbar;
mod servers_page;
#[cfg(windows)]
mod setup_page;
mod shell;
mod shortcut_page;
mod specimen;
mod sticky;
mod switch_page;
mod theme;
mod themes_page;
mod titlebar;
mod wardrobe_page;
mod welcome;
mod while_playing;
mod window_root;
mod worlds_page;

use gpui::{
    actions, div, ease_out_quint, prelude::*, px, relative, size, Animation, AnimationExt,
    AnyElement, App, Bounds, ClickEvent, Context, Div, ElementId, FocusHandle, FontWeight, Hsla,
    KeyBinding, KeyDownEvent, Menu, MenuItem, MouseDownEvent, PathPromptOptions, Pixels, Point,
    SharedString, Stateful, Subscription, Window, WindowBounds, WindowOptions,
};
use horizon_launcher::account::{self, Accounts, SignIn, SignInState};
use horizon_launcher::backend::{self, Job, Msg, Release, Running, Settings, Updates};
use horizon_launcher::build_import;
use horizon_launcher::i18n::{self, tr, trf};
use horizon_launcher::instances;
use horizon_launcher::native_mods;
use horizon_launcher::native_status;
use horizon_launcher::storage;
use horizon_launcher::telemetry;
use native_consent::PendingConsent;
use scrollbar::Scrollbar;
use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use theme::Theme;

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Play,
    Versions,
    /// Mods, resource packs and modpacks (imported ones and the browser's).
    Mods,
    Servers,
    /// What the player wears (`wardrobe_page.rs`), with the cosmetics flag on.
    Wardrobe,
    /// Cosmetics to try on and buy (`wardrobe_page/shop.rs`), beside the Wardrobe and under the
    /// same rule. A plain tab: no badge, and it never opens by itself.
    Shop,
    Settings,
}

impl Page {
    /// `HORIZON_LAUNCHER_PAGE=mods` (or `modpacks`, `resourcepacks`, `shaders`, `servers`, `versions`,
    /// `settings`) opens on that page, for screenshots; Home otherwise (`getting-ready`: Home
    /// preparing, `home::getting_ready_preview`; `sign-in`: the owner gate's sheet with a
    /// made-up code, `owner_page::sign_in_preview`; `new-instance`: Home with the New instance
    /// dialog open on Modded; `edit-instance`: the selected one's Edit dialog; `worlds`, `packs`...: Home on that tab, `HomeTab::from_env`).
    fn from_env() -> Page {
        let page = match std::env::var("HORIZON_LAUNCHER_PAGE").as_deref() {
            Ok("mods" | "modpacks" | "resourcepacks" | "shaders" | "datapacks") => Page::Mods,
            Ok("servers") => Page::Servers,
            Ok("wardrobe") if horizon_cosmetics::forced() => Page::Wardrobe,
            Ok("shop") if horizon_cosmetics::forced() => Page::Shop,
            Ok("versions") => Page::Versions,
            Ok("settings") => Page::Settings,
            _ => Page::Play,
        };
        // The data folder did not open: start on Settings, where the message and the two
        // recoveries are (an explicit HORIZON_LAUNCHER_PAGE still wins, for screenshots).
        page_after_startup(
            page,
            horizon_launcher::data_location::startup_error().is_some(),
        )
    }

    /// Every page, with its name and icon: the top bar shows all but Settings, which has its
    /// own button.
    const ALL: [(Page, &'static str, &'static str); 7] = [
        (Page::Play, "Home", "home"),
        (Page::Mods, "Mods", "package"),
        (Page::Servers, "Servers", "server"),
        (Page::Wardrobe, "Wardrobe", "user"),
        (Page::Shop, "Shop", "bag"),
        (Page::Versions, "Versions", "archive"),
        (Page::Settings, "Settings", "sliders"),
    ];

    /// The pages the top bar shows: all but Settings, and the Wardrobe and the Shop only when
    /// `wardrobe` (`Launcher::wardrobe_shown`).
    fn in_top_bar(wardrobe: bool) -> impl Iterator<Item = (Page, &'static str, &'static str)> {
        Self::ALL.into_iter().filter(move |(page, ..)| match page {
            Page::Settings => false,
            Page::Wardrobe | Page::Shop => wardrobe,
            _ => true,
        })
    }
}

/// The launcher's own update, as the footer and Settings show it.
#[derive(Clone, PartialEq)]
enum SelfUpdate {
    Idle,
    Downloading(String, f32),
    /// Downloaded and checked: "A new version is available" and Update, which hands it to the
    /// installer (`update_handoff`). Closing the window hands it over too.
    Ready(String),
    /// A newer launcher can't go in here: this copy can't replace itself
    /// (`backend::cannot_replace_itself`), or the installer said why (the error). The player
    /// follows the recovery advice for its install location.
    Stuck(Option<String>),
    Failed(String),
}

/// The last game the launcher started ended badly: what it left behind.
#[derive(Clone)]
struct Crash {
    /// The crash report the run wrote, if it wrote one.
    report: Option<std::path::PathBuf>,
    /// One line on what went wrong: the report's error, else how the process ended.
    summary: String,
    copied: bool,
}

/// The opening: the wordmark holds at least `OPENING_HOLD` (until the release
/// list is in, at most `OPENING_MAX`), then the launcher appears over `OPENING_REVEAL`.
const OPENING_HOLD: Duration = horizon_theme::OPENING_HOLD;
const OPENING_MAX: Duration = horizon_theme::OPENING_MAX;
const OPENING_REVEAL: Duration = horizon_theme::OPENING_REVEAL;

/// The phase of the opening at one moment: how far the reveal has gone, 0 to 1.
struct Opening {
    /// Set on the first frame, so a slow start does not eat into the hold.
    started: Option<Instant>,
    reveal: Option<Instant>,
    focus: FocusHandle,
}

fn ease_out_cubic(x: f32) -> f32 {
    horizon_theme::ease_out_cubic(x)
}

enum Load<T> {
    Loading,
    Ready(T),
    Failed(String),
}

const LOG_LINES: usize = 2000;

struct Launcher {
    theme: Theme,
    page: Page,
    /// The page under the glass Settings dialog: the last one shown before it opened.
    behind: Page,
    /// How many times the page changed, for the page transition.
    switches: usize,
    content_scroll: Scrollbar,
    /// The top bar's page tabs, including their horizontal overflow controls.
    nav_strip: nav_strip::NavStrip,
    /// The glass Settings page's own scroll: its sections beside their list.
    settings_pane: gpui::ScrollHandle,
    /// The glass Settings page's search, its focus and whether it has it.
    settings_query: line_edit::LineEdit,
    settings_query_focus: FocusHandle,
    settings_query_focused: bool,
    /// The tags whose "What's new" the glass Versions page shows open; None is the newest's.
    whats_new_open: Option<std::collections::HashSet<String>>,
    /// The section the glass Settings list marked when last drawn (`glass_settings`).
    settings_marked: usize,
    /// About's credit picture was asked for, and what came back.
    credit_asked: bool,
    /// The Game version dropdown of the instance dialog and sheet is open.
    version_menu: bool,
    credit_picture: Option<std::sync::Arc<gpui::Image>>,
    /// The instance sheet's Storage: which instance was measured, and its size once known.
    sheet_size: Option<(String, Option<u64>)>,
    /// The Mods page's project card, kept in view beside its list.
    mods_sticky: sticky::Sticky,
    /// The rows of the page's long list (Versions) last drawn.
    long_list: cull::Cull,
    opening: Option<Opening>,
    updates: Updates,
    checked: Option<Instant>,
    update_error: Option<String>,
    self_update: SelfUpdate,
    self_update_hidden: bool,
    /// The Horizon version being installed as an update: whether it becomes the selected one is
    /// judged when it finishes.
    update_install: Option<String>,
    /// The Horizon version being installed because an instance was made or saved to play it: it
    /// becomes the launcher's version only when the launcher has none.
    instance_install: Option<String>,
    /// One for an instance waiting for the install running now to end.
    install_next: Option<Release>,
    /// The install bar's previous and current fraction, so it moves rather than jumps.
    bar: (f32, f32),
    releases: Load<Vec<Release>>,
    installed: Vec<String>,
    installing: Option<(String, f32)>,
    /// The versions being deleted on background threads. Each is already out of `installed`, so
    /// nothing can launch, select or prepare it, and its row can't install or remove it again.
    /// More than one may run at once: a row's Remove stays usable while another version goes.
    removing: Vec<String>,
    /// What each installed version takes on disk, measured on a thread for the Versions page, and
    /// the installed list it was measured for.
    disk: Vec<(String, u64)>,
    storage_report: Option<storage::Report>,
    storage_pending: bool,
    /// A Clear the player still has to confirm: `Some(None)` is Clear all.
    storage_confirm: Option<Option<storage::Category>>,
    storage_error: Option<String>,
    /// Turn off for native mods was pressed once: how many switched-on native mods it would
    /// switch off, and in how many instances, counted then (`glass_storage.rs`).
    storage_revoke: Option<(usize, usize)>,
    /// Storage is counting native mods for Turn off: the count of this Settings visit.
    storage_counting: Option<u64>,
    /// Each page switch: a count from an earlier visit is dropped when it arrives.
    storage_visit: u64,
    /// Why Turn off didn't finish, shown at the Native mods row.
    storage_revoke_error: Option<String>,
    /// The build asked for (its base and back-up choice) while the permission question is open:
    /// Allow resumes it, not a plain build.
    native_consent_build: Option<(Option<String>, bool)>,
    disk_for: Vec<String>,
    install_error: Option<(String, String)>,
    /// Import a game build (`build_import`, on the Versions page): whether one is being copied in,
    /// and why the last one failed.
    importing_build: bool,
    import_error: Option<String>,
    /// Native mods (`native_mods`, on the Mods page): the build's step while it runs, why it
    /// failed, and the warning before a build while it is open, with the mods it names.
    native_step: Option<String>,
    /// Where the build under way is, for the card's stages and bar.
    native_progress: Option<native_mods::Progress>,
    /// The first build's one-time download (the compiler), in bytes; `None` once it is there.
    first_build: Option<u64>,
    /// Play was pressed while this instance's native mods needed a build (Build and play): the
    /// game starts once the build worked, if it is still the instance selected.
    play_after_build: Option<String>,
    /// How the last native build ended ("Your mods are built", or "Up to date").
    native_done: Option<String>,
    native_error: Option<String>,
    /// The mods that stopped the last build (`native_mods::Failed::Mods`): their rows say Failed,
    /// the others keep their own state.
    native_failed_mods: Vec<String>,
    /// Mods the last build would have switched off that worlds were saved with: the choice
    /// between staying and backing those worlds up first, shown with `native_error`.
    native_worlds: Option<native_mods::AtStake>,
    pending_consent: Option<PendingConsent>,
    /// Why the last Allow or Cancel failed, shown inside the dialog that the page is behind.
    consent_error: Option<String>,
    /// The instance (id) the build under way, or the last one's error, is for.
    native_for: Option<String>,
    /// The native mods the build under way (or the last one) was given (`native_build.rs`).
    native_built_mods: Vec<String>,
    /// Where truncating texts were laid out, for their full-text hover labels (`glass_ui::cut`).
    rooms: glass_ui::Rooms,
    /// What each installed native build was made from, by version. A build's version names its
    /// contents, so an entry never goes stale.
    native_builds:
        std::cell::RefCell<std::collections::HashMap<String, Option<native_status::Build>>>,
    /// The top bar's Build log panel (`native_log_page.rs`).
    native_log: native_log_page::NativeLog,
    confirm_remove: Option<String>,
    /// The releases whose patch notes the Versions page shows open.
    notes_open: std::collections::HashSet<String>,
    /// Release notes parsed from `releases`, for the patch log and Patch notes.
    notes: notes_cache::NotesCache,
    /// Versions cards whose "Technical details" are open.
    technical_open: std::collections::HashSet<String>,
    picker: glass_home::PickerState,
    /// Settings' any-colour swatch shows the hue strip (the glass look).
    accent_strip: bool,
    /// Settings' Glass and Blur sliders (the glass look).
    look_sliders: themes_page::LookSliders,
    /// Where the accent picker's hue strip is on screen (`themes_page.rs`), to map a click to a
    /// hue.
    hue_area: std::rc::Rc<std::cell::Cell<Option<Bounds<gpui::Pixels>>>>,
    /// A drag on that strip is in flight, so a release anywhere writes the accent down.
    hue_dragging: bool,
    /// Settings > Language's dropdown (`language_page.rs`).
    language_menu: bool,
    /// The top bar's quick switch of who plays (`switch_page.rs`).
    account_menu: bool,
    /// The glass sidebar's ⋮ menu (`frame.rs`).
    row_menu: frame::RowMenu,
    /// The rail's hover: whether the pointer is on it, and since when (it eases open and shut).
    rail: frame::Rail,
    /// The instance sheet's open section (`glass_sheet.rs`).
    sheet_tab: glass_sheet::SheetTab,
    /// The glass look's colours for the player's accent (`horizon_theme::glass`).
    look: horizon_theme::glass::Glass,
    /// Where the mouse went down to close a menu by pressing outside it, so that the
    /// same press, ending on the menu's own button, doesn't open it again.
    menu_closed_at: Option<Point<Pixels>>,
    /// The Wardrobe page's account and outfit (`wardrobe_page.rs`).
    wardrobe: Option<wardrobe_page::Wardrobe>,
    /// Name tag styles' marks; they outlive the account's `wardrobe`.
    wardrobe_marks: wardrobe_page::Marks,
    /// Ask the service again on the next draw of the Wardrobe (set by each visit).
    wardrobe_reload: bool,
    /// The Shop page's basket and choices (`wardrobe_page/shop.rs`).
    shop: wardrobe_page::Shop,
    /// The cosmetics service's last word on the founder set's window (`GET /v1/shop`): whether
    /// the top bar has the Wardrobe page with cosmetics off (`wardrobe_shown`).
    shop_status: Option<horizon_launcher::wardrobe::Claim>,
    /// The selected account's own answer on it from its last Wardrobe visit, kept on disk (a
    /// preview the service grants it, its claim).
    shop_kept: Option<horizon_launcher::wardrobe::Claim>,
    /// Work the look out again on the next frame (a launcher theme put on or taken off).
    restyle: bool,
    /// Why the last picture for Home's background was refused.
    background_error: Option<String>,
    /// Settings' data relocation confirmation and the worker's last result.
    location_target: Option<std::path::PathBuf>,
    location_error: Option<String>,
    location_note: Option<String>,
    settings: Settings,
    /// The last failed settings write, kept across page changes until a save succeeds.
    settings_save_error: Option<String>,
    /// The Discord, sponsor and website links: built in until links.json on main has been read.
    links: backend::Links,
    preparing: bool,
    prepare_log: VecDeque<String>,
    /// Lines received from the game and the preparer since the launcher started: it only grows,
    /// so it moves when a line arrives even while `prepare_log` is full and its length stays.
    log_revision: u64,
    prepare_error: Option<String>,
    client: Option<Running>,
    client_error: Option<String>,
    /// When the running client started, to tell its crash report from older ones.
    client_started: Option<std::time::SystemTime>,
    /// The folder the running game keeps its settings in: the game home, or a modpack's.
    client_home: Option<std::path::PathBuf>,
    crash: Option<Crash>,
    /// The Servers page (`servers_page.rs`).
    servers: servers_page::Servers,
    /// Its Friends over the internet panel (`internet.rs`).
    internet: internet::Internet,
    /// The window's own focus, so Tab reaches the controls when nothing else has the keyboard.
    /// The palette (⌘K), while open (`glass_palette`).
    palette: Option<glass_palette::Palette>,
    /// The toast last drawn (`glass_toast`), by its number.
    toast_seen: Option<u64>,
    root_focus: FocusHandle,
    /// Microsoft accounts (accounts.json), the sign-in under way and what it last showed.
    accounts: Accounts,
    auth_status: HashMap<String, account::session::Status>,
    auth_progress: HashMap<String, account::session::Phase>,
    profile_heads: profile_heads::Heads,
    /// When page switches last checked every account's sign-in.
    auth_checked: Option<Instant>,
    sign_in_target: Option<account::Account>,
    auth_busy: HashMap<String, u64>,
    auth_next: u64,
    sign_in: Option<SignIn>,
    sign_in_state: Option<SignInState>,
    /// What to say when the keychain doesn't keep sign-ins ([`account::no_keychain`]).
    keychain: Option<String>,
    /// Whether an account here owns the game, by the keychain's last answers (the owner gate);
    /// none until the keychain has been read.
    owner: Option<bool>,
    /// Play was refused (or Home's Sign in pressed): the owner gate's screen is open.
    owner_screen_open: bool,
    /// The game is waiting for its session (a token renewal), or a Java instance for its files
    /// and Java (`start_java`), before it starts.
    starting_client: Option<String>,
    /// The instance that start is for, whatever is selected by the time it ends.
    starting_instance: Option<String>,
    /// Play was pressed on a version not prepared yet: it starts once this preparation ends well.
    play_when_prepared: Option<(String, String)>,
    /// Join a friend: the code typed on the Play page, and the one the game
    /// starting now joins.
    friend_code: line_edit::LineEdit,
    friend_focused: bool,
    friend_focus: FocusHandle,
    /// The window's width at the last frame: Home's head lays out by it, as the mockup's `xl`.
    window_width: f32,
    /// The window's longer side in points, and the screen's pixels per point: the size pictures
    /// are made at (`picture_cache`).
    window_long_side: f32,
    scale_factor: f32,
    joining: Option<String>,
    /// Where the next start goes at once: `--world <folder>` or `--server <address>`.
    quick_play: Option<(&'static str, String)>,
    /// The instance the running game plays and when it started: its play time grows on exit.
    client_run: Option<(String, Instant)>,
    /// The running game's instance variables, for its post-exit command.
    client_vars: horizon_launcher::launch::Vars,
    /// Settings' Game arguments field: the line, whether it has focus, and its focus handle.
    args_input: line_edit::LineEdit,
    args_focused: bool,
    args_focus: FocusHandle,
    /// Settings' Proxy field, the same way (`proxy`).
    proxy_input: line_edit::LineEdit,
    proxy_focused: bool,
    proxy_focus: FocusHandle,
    /// Opt-in data collection: the player's answer and Settings' section (`collection`).
    collection: collection::Collection,
    /// Settings' "Connect an AI agent" (`agents_page`).
    agents: agents_page::AgentsUi,
    /// How many play Horizon now, while the server answers ("N playing now" on the Play page).
    playing: Option<u64>,
    /// How many play on the official server now, while it answers ("M on the official SMP").
    official_players: Option<u64>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    _appearance: Subscription,
    /// Redraws when the window gains or loses focus: the sky moves only in front.
    _activation: Subscription,
    /// The session's measurements (`perf.rs`), and when the steps they time began.
    perf: perf::Perf,
    install_started: Option<Instant>,
    prepare_started: Option<Instant>,
    /// The download under way as first seen (when, files done), for the time left.
    download_seen: Option<(Instant, usize)>,
    /// The tab open under Home's hero (`home_tabs.rs`).
    home_tab: home_tabs::HomeTab,
    /// Home's Play or folder menu, while open (glass look).
    home_menu: glass_home::HomeMenu,
    /// Whether the pointer is over Home's folder button, which then shows its ⋯.
    folder_hovered: bool,
    /// The list row under the pointer in Home's glass tabs (a world, a server): it shows its
    /// actions then.
    list_hover: Option<SharedString>,
    /// A Home list row's ⋯ menu, while open, and whether its Delete… was clicked once.
    list_menu: Option<(SharedString, bool)>,
    /// The size of Home's tab strip, measured as it is laid out: the glass panel's plate cuts
    /// its corner around it.
    home_strip: std::rc::Rc<std::cell::Cell<Option<gpui::Size<Pixels>>>>,
    /// When the running game was first seen running, for Home's "Playing for…".
    client_since: Option<Instant>,
    /// The selected instance's mods, for Home's Mods tab (`instance_mods.rs`).
    instance_mods: instance_mods::InstanceMods,
    /// Debug-build previews of the native mod states (`glass_mod_states.rs`).
    mod_preview: glass_mod_states::Preview,
    /// Modrinth projects' icon URLs (`instance_mods.rs`).
    project_icons: instance_mods::ProjectIcons,
    play_pressed: Option<Instant>,
    /// `horizon-launcher benchmark` started this window to measure it.
    bench: Option<perf::Bench>,
    /// The Modpacks page: the imported packs (read when the page first opens), the one shown,
    /// the report's filter, and the pipeline step running now with its log.
    packs: Vec<horizon_launcher::modpacks::Imported>,
    /// Modpacks installed from Horizon's registry, each its own profile.
    registry_packs: Vec<horizon_launcher::browser::horizon::pack::Installed>,
    packs_loaded: Option<Instant>,
    /// The imported packs' read on a thread, and whether it may still apply (`load_packs`).
    packs_reading: modpacks_page::PackReads,
    pack_open: Option<String>,
    pack_filter: Option<horizon_port_pack::report::Status>,
    pack_busy: bool,
    pack_log: VecDeque<String>,
    pack_error: Option<String>,
    /// Other launchers' modpacks on this computer, offered above the imported ones, and whether
    /// the scan for them runs now.
    found_packs: Vec<horizon_launcher::other_launchers::Found>,
    found_scanning: bool,
    /// The game starting now plays this pack's profile (its work folder).
    starting_pack: Option<std::path::PathBuf>,
    /// The folder of the instance a start is preparing, while `starting_client` is set.
    starting_work: Option<std::path::PathBuf>,
    /// The pack folder a Port step is rebuilding now.
    porting: Option<std::path::PathBuf>,
    /// The Mods page: the browser's search, installs and updates (`mods_page.rs`).
    mods: mods_page::Mods,
    github: github_mods_page::Github,
    /// The instances, for the sidebar and Play, and their dialog (`instances_page.rs`).
    instances: instances_page::Instances,
    /// Home's Worlds section, of the selected instance (`worlds_page.rs`).
    worlds: worlds_page::Worlds,
    multiplayer: multiplayer_page::Multiplayer,
    screenshots: screenshots_page::Screenshots,
    shot_blur: glass_shot_blur::ShotBlur,
    /// Pictures from disk made at their drawn size (`picture_cache`).
    pictures: picture_cache::Pictures,
    game_packs: packs_page::Packs,
    /// The games' answers about their shader API seen so far (`game_shader_api::answers`).
    shader_answers: u64,
    world_packs: datapacks_page::WorldPacks,
    export: Option<export_page::ExportPanel>,
    launch: Option<launch_page::LaunchPanel>,
    /// The memory row on Home.
    memory_slider: launch_page::MemorySlider,
    importing: import_page::Importing,
    console: console_page::Console,
    /// What the window did when the game started (`while_playing.rs`).
    aside: while_playing::Aside,
    /// The selected instance's desktop shortcut (`shortcut_page.rs`).
    shortcuts: shortcut_page::Shortcuts,
    /// The player's servers, for the sidebar until the Servers page has read them.
    side_servers: Vec<horizon_launcher::servers::Entry>,
    /// The pixel art made for this window (`art.rs`), and pictures downloaded by URL.
    sky: gpui::Entity<sky::SkyView>,
    /// The pixel sunset of the new design (dark appearance): its own view, so a frame of it
    /// alone redraws nothing else.
    scene: gpui::Entity<scene::Scene>,
    /// Hypr Neon's rain over its black night (`rain.rs`).
    rain: gpui::Entity<rain::Rain>,
    /// What is behind the launcher, drawn as its sibling (`window_root.rs`).
    backdrop: gpui::Entity<window_root::Backdrop>,
    /// What the views behind the launcher were last handed.
    pushed: Option<launcher_render::Pushed>,
    /// The window's scale, as of its last frame, for art drawn outside `render`'s reach.
    scale: f32,
}

actions!(horizon_launcher, [Quit, CloseWindow]);

/// What closing the last window does: stop the servers this launcher started, send the telemetry,
/// exit. A game that is running keeps running. Cmd+Q goes through here too.
fn shut_down(cx: &mut App) {
    servers_page::stop_on_quit();
    telemetry::send_before_exit();
    // A downloaded launcher update goes in only when the player presses Update: closing drops it.
    horizon_launcher::update_handoff::forget();
    cx.quit()
}

/// The macOS shortcuts: Cmd+Q quits, Cmd+W closes the window.
#[cfg(target_os = "macos")]
fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
    ]
}

#[cfg(not(target_os = "macos"))]
fn key_bindings() -> Vec<KeyBinding> {
    Vec::new()
}

/// A data folder that did not open starts on Settings, where the message, Retry and
/// "Use the default folder" are; an explicit page still wins.
fn page_after_startup(page: Page, data_folder_failed: bool) -> Page {
    if page == Page::Play && data_folder_failed {
        Page::Settings
    } else {
        page
    }
}

/// The menu bar on macOS, where "Quit" belongs to the app menu.
fn menus() -> Vec<Menu> {
    vec![
        Menu {
            name: "Horizon Launcher".into(),
            items: vec![MenuItem::action(tr("Quit Horizon Launcher"), Quit)],
            disabled: false,
        },
        Menu {
            name: "Window".into(),
            items: vec![MenuItem::action("Close Window", CloseWindow)],
            disabled: false,
        },
    ]
}

mod startup;

fn main() {
    startup::run();
}

/// Whether the click `event` began with the mouse-down that closed a menu at `closed_at`.
/// A keyboard click never did.
fn closed_by_press(closed_at: Option<Point<Pixels>>, event: &ClickEvent) -> bool {
    matches!(event, ClickEvent::Mouse(click) if closed_at == Some(click.down.position))
}

#[cfg(test)]
mod tests {
    #[test]
    fn an_unopenable_data_folder_starts_on_settings_instead_of_exiting() {
        use super::{page_after_startup, Page};
        assert!(matches!(
            page_after_startup(Page::Play, true),
            Page::Settings
        ));
        assert!(matches!(page_after_startup(Page::Play, false), Page::Play));
        assert!(matches!(page_after_startup(Page::Mods, true), Page::Mods));
    }

    #[test]
    fn a_menu_button_stays_closed_after_the_press_that_closed_its_menu() {
        use super::closed_by_press;
        use gpui::{point, px, ClickEvent, KeyboardClickEvent, MouseClickEvent, MouseDownEvent};
        let click = |x: f32| {
            ClickEvent::Mouse(MouseClickEvent {
                down: MouseDownEvent {
                    position: point(px(x), px(20.)),
                    ..MouseDownEvent::default()
                },
                ..MouseClickEvent::default()
            })
        };
        let closed_at = Some(point(px(10.), px(20.)));
        // The press on the button that closed the open menu: it stays closed.
        assert!(closed_by_press(closed_at, &click(10.)));
        // A later press, or one with no menu closed, toggles as before.
        assert!(!closed_by_press(closed_at, &click(11.)));
        assert!(!closed_by_press(None, &click(10.)));
        let keyboard = ClickEvent::Keyboard(KeyboardClickEvent::default());
        assert!(!closed_by_press(closed_at, &keyboard));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn cmd_q_quits_and_cmd_w_closes_the_window() {
        use super::{key_bindings, CloseWindow, Quit};
        use gpui::{Keymap, Keystroke};
        let keymap = Keymap::new(key_bindings());
        let key = |s: &'static str| [Keystroke::parse(s).unwrap()];
        let (hit, _) = keymap.bindings_for_input(&key("cmd-q"), &[]);
        assert!(hit.iter().any(|b| b.action().partial_eq(&Quit)));
        let (hit, _) = keymap.bindings_for_input(&key("cmd-w"), &[]);
        assert!(hit.iter().any(|b| b.action().partial_eq(&CloseWindow)));
    }

    #[test]
    fn a_second_version_can_be_removed_while_another_deletes() {
        let installed = ["v0.2.97".to_string(), "v0.2.76".to_string()];
        let removing = ["v0.2.97".to_string()];
        assert!(super::removable(
            &installed,
            None,
            &removing,
            &[],
            "v0.2.76"
        ));
        assert!(!super::removable(
            &installed,
            None,
            &removing,
            &[],
            "v0.2.97"
        ));
        assert!(!super::removable(
            &installed,
            Some("v0.2.76"),
            &[],
            &[],
            "v0.2.76"
        ));
        assert!(!super::removable(&installed, None, &[], &[], "v0.2.85"));
    }

    #[test]
    fn a_version_an_instance_plays_cannot_be_removed() {
        let installed = ["v0.2.97".to_string(), "v0.2.76".to_string()];
        assert!(!super::removable(
            &installed,
            None,
            &[],
            &["Horizon"],
            "v0.2.97"
        ));
        assert!(super::removable(&installed, None, &[], &[], "v0.2.97"));
    }

    #[test]
    fn a_press_on_a_sidebar_row_opens_its_menu_or_closes_it() {
        let open = |id: &str| Some((id.to_string(), false));
        assert_eq!(
            super::row_menu_after_press(None, "Horizon"),
            open("Horizon")
        );
        assert_eq!(
            super::row_menu_after_press(open("Other"), "Horizon"),
            open("Horizon")
        );
        assert_eq!(
            super::row_menu_after_press(open("Horizon"), "Horizon"),
            None
        );
        // A Delete… already pressed on another row does not carry over.
        assert_eq!(
            super::row_menu_after_press(Some(("Other".into(), true)), "Horizon"),
            open("Horizon")
        );
    }

    #[test]
    fn the_selection_survives_a_removal_or_moves_to_the_newest_left() {
        let left = ["v0.2.49".to_string(), "v0.2.48".to_string()];
        let pick =
            |saved: Option<&str>| super::selection_after_removal(&left, saved.map(String::from));
        assert_eq!(pick(Some("v0.2.48")).as_deref(), Some("v0.2.48"));
        assert_eq!(pick(Some("v0.2.47")).as_deref(), Some("v0.2.49"));
        assert_eq!(pick(None).as_deref(), Some("v0.2.49"));
        assert_eq!(
            super::selection_after_removal(&[], Some("v0.2.47".into())),
            None
        );
    }

    /// An owner's Play works on a version not prepared yet: it prepares it (downloading
    /// Minecraft from Mojang when this computer has no install), then plays. Without an owner
    /// it stays off: signing in comes first.
    #[test]
    fn play_prepares_first_for_an_owner() {
        use super::{play_press, PlayPress};
        assert_eq!(play_press(true, Some(true), false), PlayPress::Play);
        assert_eq!(play_press(true, None, false), PlayPress::Play);
        assert_eq!(
            play_press(false, Some(true), false),
            PlayPress::PrepareThenPlay
        );
        assert_eq!(play_press(false, Some(false), false), PlayPress::Nothing);
        assert_eq!(play_press(false, None, false), PlayPress::Nothing);
        assert_eq!(play_press(false, Some(true), true), PlayPress::Nothing);
        assert_eq!(play_press(true, Some(true), true), PlayPress::Nothing);
    }

    /// Home's one button never says Play over a click that prepares, or that cannot play.
    #[test]
    fn play_says_what_it_will_do() {
        use super::{play_label, play_press};
        let say = |prepared, owner| play_label(play_press(prepared, owner, false), owner);
        assert_eq!(say(true, Some(true)), "Play");
        assert_eq!(say(false, Some(true)), "Prepare & play");
        assert_eq!(say(false, Some(false)), "Sign in to play");
        assert_eq!(say(false, None), "Play");
    }

    /// A server's Join (and every other shortcut) waits like Play: not while the version's
    /// first download and preparation run (#horizon-bugs, 0.2.23: a white window opened).
    #[test]
    fn shortcuts_start_only_when_play_would() {
        use super::starts_now;
        assert!(starts_now(true, false, false));
        assert!(
            !starts_now(false, true, false),
            "first-time setup under way"
        );
        assert!(!starts_now(true, true, false), "prepared again under way");
        assert!(!starts_now(false, false, false), "not prepared");
        assert!(!starts_now(true, false, true), "already running");
    }

    /// A sidebar row says Running or Starting for the instance that was started, not for
    /// whichever one is selected (#16186).
    #[test]
    fn a_row_shows_the_game_of_its_own_instance_only() {
        use super::frame::{row_phase, running_ids, RowPhase};
        assert_eq!(row_phase("a", &["a"], None), Some(RowPhase::Running));
        assert_eq!(row_phase("b", &["a"], None), None, "B is only selected");
        assert_eq!(row_phase("a", &[], Some("a")), Some(RowPhase::Starting));
        assert_eq!(row_phase("b", &[], Some("a")), None);
        assert_eq!(row_phase("a", &[], None), None, "nothing runs");
        // Several games: one runs while another starts.
        assert_eq!(row_phase("a", &["a"], Some("b")), Some(RowPhase::Running));
        assert_eq!(row_phase("b", &["a"], Some("b")), Some(RowPhase::Starting));
        assert_eq!(row_phase("b", &["a", "b"], None), Some(RowPhase::Running));
        // The started instance deleted (ids don't change on a rename): no row has its game.
        assert_eq!(row_phase("c", &["a"], Some("b")), None);
        // The ids come from the run's own instance, and only while its game runs.
        let run = ("a".to_string(), std::time::Instant::now());
        assert_eq!(running_ids(true, Some(&run)), ["a"]);
        assert!(running_ids(false, Some(&run)).is_empty());
        assert!(
            running_ids(true, None).is_empty(),
            "a pack with no instance"
        );
    }

    #[test]
    fn a_completed_sign_in_is_handled_once_across_ui_polls() {
        use super::{sign_in_change, SignInState};
        let mut shown = None;
        let completed = SignInState::Done(Ok(horizon_launcher::account::Account {
            uuid: "11111111111111111111111111111111".into(),
            name: "TestAccount".into(),
            skin: None,
            cape: None,
        }));
        assert!(sign_in_change(&mut shown, SignInState::Starting).is_some());
        assert!(sign_in_change(&mut shown, SignInState::Starting).is_none());
        let mut account_reloads = 0;
        for _ in 0..100 {
            if matches!(
                sign_in_change(&mut shown, completed.clone()),
                Some(SignInState::Done(Ok(_)))
            ) {
                account_reloads += 1;
            }
        }
        assert_eq!(
            account_reloads, 1,
            "completed sign-in must not restart account checks every frame"
        );
        let failed = SignInState::Done(Err("Sign-in cancelled".into()));
        assert!(sign_in_change(&mut shown, failed.clone()).is_some());
        assert!(sign_in_change(&mut shown, failed).is_none());
    }

    #[test]
    fn the_app_menu_has_quit() {
        let menus = super::menus();
        assert_eq!(menus[0].name, "Horizon Launcher");
        assert!(matches!(
            &menus[0].items[0],
            gpui::MenuItem::Action { name, .. } if name == "Quit Horizon Launcher"
        ));
    }

    #[test]
    fn counts_get_thousands_separators() {
        let shown: Vec<String> = [0, 7, 999, 1000, 12_300, 1_234_500]
            .into_iter()
            .map(super::thousands)
            .collect();
        assert_eq!(shown, ["0", "7", "999", "1,000", "12,300", "1,234,500"]);
    }
}

mod native_build;
