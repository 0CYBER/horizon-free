//! Horizon launcher: installs Horizon builds, prepares them from the player's own Minecraft,
//! and starts the client or a dedicated server.

mod cli;
mod collection;
mod game_tools;
mod internet;
mod markdown;
mod mcp;
mod modpacks_page;
mod mods_page;
mod perf;
mod scrollbar;
mod servers_page;
mod theme;

use gpui::{
    div, ease_out_quint, prelude::*, px, relative, size, Animation, AnimationExt, AnyElement, App,
    Application, Bounds, ClickEvent, Context, Div, ElementId, FocusHandle, FontWeight, Hsla,
    KeyDownEvent, PathPromptOptions, SharedString, Stateful, Subscription, TitlebarOptions, Window,
    WindowBounds, WindowOptions,
};
use horizon_launcher::account::{self, Accounts, SignIn, SignInState};
use horizon_launcher::backend::{self, Job, Msg, Release, Running, Settings, Updates};
use horizon_launcher::telemetry;
use scrollbar::Scrollbar;
use std::borrow::Cow;
use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use theme::Theme;

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Play,
    Versions,
    Mods,
    Modpacks,
    Servers,
    Notes,
    Settings,
}

impl Page {
    const ALL: [(Page, &'static str); 7] = [
        (Page::Play, "Play"),
        (Page::Versions, "Versions"),
        (Page::Mods, "Mods"),
        (Page::Modpacks, "Modpacks"),
        (Page::Servers, "Servers"),
        (Page::Notes, "Patch notes"),
        (Page::Settings, "Settings"),
    ];
}

/// The launcher's own update, as the footer and Settings show it.
#[derive(Clone, PartialEq)]
enum SelfUpdate {
    Idle,
    Downloading(String, f32),
    Ready(String),
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

/// How often the launcher looks for new releases while it is open.
const CHECK_EVERY: Duration = Duration::from_secs(4 * 3600);

enum Load<T> {
    Loading,
    Ready(T),
    Failed(String),
}

const LOG_LINES: usize = 2000;

struct Launcher {
    theme: Theme,
    page: Page,
    /// How many times the page changed, for the page transition.
    switches: usize,
    content_scroll: Scrollbar,
    opening: Option<Opening>,
    updates: Updates,
    checked: Option<Instant>,
    update_error: Option<String>,
    self_update: SelfUpdate,
    /// The Horizon version being installed as an update, and whether it becomes the selected one.
    update_install: Option<(String, bool)>,
    /// The install bar's previous and current fraction, so it moves rather than jumps.
    bar: (f32, f32),
    releases: Load<Vec<Release>>,
    installed: Vec<String>,
    installing: Option<(String, f32)>,
    install_error: Option<(String, String)>,
    confirm_remove: Option<String>,
    picker_open: bool,
    settings: Settings,
    /// The Discord, sponsor and website links: built in until links.json on main has been read.
    links: backend::Links,
    menu_style: backend::MenuStyle,
    preparing: bool,
    prepare_log: VecDeque<String>,
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
    root_focus: FocusHandle,
    /// Microsoft accounts (accounts.json), the sign-in under way and what it last showed.
    accounts: Accounts,
    sign_in: Option<SignIn>,
    sign_in_state: Option<SignInState>,
    keychain: bool,
    /// The game is waiting for its session (a token renewal) before it starts.
    starting_client: Option<String>,
    /// Join a friend: the code typed on the Play page, and the one the game
    /// starting now joins.
    friend_code: String,
    friend_focused: bool,
    friend_focus: FocusHandle,
    joining: Option<String>,
    /// Settings' Game arguments field: the line, whether it has focus, and its focus handle.
    args_input: String,
    args_focused: bool,
    args_focus: FocusHandle,
    /// Opt-in data collection: the player's answer and Settings' section (`collection`).
    collection: collection::Collection,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    _appearance: Subscription,
    /// The session's measurements (`perf.rs`), and when the steps they time began.
    perf: perf::Perf,
    install_started: Option<Instant>,
    prepare_started: Option<Instant>,
    play_pressed: Option<Instant>,
    /// `horizon-launcher benchmark` started this window to measure it.
    bench: Option<perf::Bench>,
    /// The Modpacks page: the imported packs (read when the page first opens), the one shown,
    /// the report's filter, and the pipeline step running now with its log.
    packs: Vec<horizon_launcher::modpacks::Imported>,
    packs_loaded: Option<Instant>,
    pack_open: Option<String>,
    pack_filter: Option<horizon_port_pack::report::Status>,
    pack_busy: bool,
    pack_log: VecDeque<String>,
    pack_error: Option<String>,
    /// The game starting now plays this pack's profile (its work folder).
    starting_pack: Option<std::path::PathBuf>,
    /// The Mods page: the browser's search, installs and updates (`mods_page.rs`).
    mods: mods_page::Mods,
}

impl Launcher {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (tx, rx) = channel();
        let appearance = cx.observe_window_appearance(window, |this, window, cx| {
            this.theme = Theme::for_appearance(window.appearance());
            cx.notify();
        });
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(80))
                .await;
            if this.update(cx, |this, cx| this.drain(cx)).is_err() {
                break;
            }
        })
        .detach();
        let mut settings = Settings::load();
        if settings.minecraft_dir.is_none() {
            settings.minecraft_dir = backend::default_minecraft_dir();
        }
        // Release builds on Linux keep their entry in the applications menu up to date (a moved
        // binary changes it); `HORIZON_LAUNCHER_DESKTOP_ENTRY=1` does it in a development build.
        if cfg!(target_os = "linux")
            && settings.desktop_entry()
            && (!cfg!(debug_assertions)
                || std::env::var_os("HORIZON_LAUNCHER_DESKTOP_ENTRY").is_some())
        {
            std::thread::spawn(|| {
                let _ = horizon_launcher::desktop::install(backend::exe_path());
            });
        }
        let installed = backend::installed();
        if settings
            .selected
            .as_ref()
            .is_none_or(|s| !installed.contains(s))
        {
            settings.selected = installed.first().cloned();
        }
        let mut this = Self {
            theme: Theme::for_appearance(window.appearance()),
            page: Page::Play,
            switches: 0,
            content_scroll: Scrollbar::default(),
            opening: (!settings.skip_opening).then(|| {
                let focus = cx.focus_handle();
                window.focus(&focus);
                Opening {
                    started: None,
                    reveal: None,
                    focus,
                }
            }),
            updates: Updates::default(),
            checked: None,
            update_error: None,
            self_update: SelfUpdate::Idle,
            update_install: None,
            bar: (0., 0.),
            releases: Load::Loading,
            installed,
            installing: None,
            install_error: None,
            confirm_remove: None,
            picker_open: false,
            args_input: settings.game_args.clone(),
            args_focused: false,
            args_focus: cx.focus_handle(),
            settings,
            links: backend::Links::default(),
            menu_style: backend::menu_style(),
            preparing: false,
            prepare_log: VecDeque::new(),
            prepare_error: None,
            client: None,
            client_error: None,
            client_started: None,
            client_home: None,
            crash: None,
            servers: servers_page::Servers::new(),
            internet: internet::Internet::new(),
            root_focus: cx.focus_handle(),
            accounts: Accounts::load(),
            sign_in: None,
            sign_in_state: None,
            keychain: true,
            starting_client: None,
            friend_code: String::new(),
            friend_focused: false,
            friend_focus: cx.focus_handle(),
            joining: None,
            collection: collection::Collection::load(),
            tx,
            rx,
            _appearance: appearance,
            perf: perf::Perf::new("session"),
            install_started: None,
            prepare_started: None,
            play_pressed: None,
            bench: perf::Bench::from_env(),
            packs: Vec::new(),
            packs_loaded: None,
            pack_open: None,
            pack_filter: None,
            pack_busy: false,
            pack_log: VecDeque::new(),
            pack_error: None,
            starting_pack: None,
            mods: mods_page::Mods::new(cx),
        };
        if this.bench.is_some() {
            this.perf.keep_local_log(false);
        }
        let tx = this.tx.clone();
        let keychain_tx = this.tx.clone();
        // Asking the keychain can block (D-Bus), so not on this thread.
        std::thread::spawn(move || {
            let _ = keychain_tx.send(Msg::Keychain(account::store().persistent()));
        });
        std::thread::spawn(move || {
            let reused = backend::reuse_prepared_everywhere();
            if !reused.is_empty() {
                let _ = tx.send(Msg::Reused(reused));
            }
        });
        this.fetch_startup_releases();
        this.fetch_links();
        // What the game and the launcher kept, if the player allows it and there is a server.
        telemetry::send_in_background();
        this
    }

    fn go(&mut self, page: Page) {
        if page != self.page {
            self.page = page;
            self.switches += 1;
            self.content_scroll = Scrollbar::default();
        }
        self.picker_open = false;
    }

    fn check_updates(&mut self) {
        self.checked = Some(Instant::now());
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Updates(backend::check_updates()));
        });
    }

    fn update_launcher(&mut self, release: Release) {
        if !backend::self_update_allowed()
            || matches!(
                self.self_update,
                SelfUpdate::Downloading(..) | SelfUpdate::Ready(_)
            )
        {
            return;
        }
        let version = backend::launcher_version(&release.tag)
            .unwrap_or_default()
            .to_string();
        self.self_update = SelfUpdate::Downloading(version, 0.);
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let progress = |p| {
                let _ = tx.send(Msg::LauncherProgress(p));
            };
            let _ = tx.send(Msg::LauncherUpdated(backend::self_update(
                &release, progress,
            )));
        });
    }

    /// Installs the Horizon version the last check found, keeping the player's choice of
    /// version unless they were on the newest one.
    fn install_update(&mut self) {
        let Some(release) = self.updates.horizon.clone() else {
            return;
        };
        if self.installing.is_some() {
            return;
        }
        let select =
            backend::select_after_update(self.settings.selected.as_deref(), &self.installed);
        self.update_install = Some((release.tag.clone(), select));
        self.install(&release);
    }

    fn fetch_releases(&mut self) {
        self.releases = Load::Loading;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Releases(backend::releases()));
        });
    }

    fn fetch_startup_releases(&mut self) {
        self.checked = Some(Instant::now());
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let updates = backend::releases_and_updates(|releases| {
                let _ = tx.send(Msg::Releases(releases));
            });
            let _ = tx.send(Msg::Updates(updates));
        });
    }

    fn fetch_links(&self) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Links(backend::links()));
        });
    }

    fn drain(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        self.perf.tick();
        match self.bench.as_mut().map(|b| b.tick(&self.perf)) {
            Some(perf::Step::Go(page)) => {
                self.go(Page::ALL[page % Page::ALL.len()].0);
                changed = true;
            }
            Some(perf::Step::Quit) => cx.quit(),
            Some(perf::Step::Wait) | None => {}
        }
        while let Ok(msg) = self.rx.try_recv() {
            changed = true;
            match msg {
                Msg::Releases(r) => {
                    if let (Ok(_), Some(checked)) = (&r, self.checked) {
                        self.perf.record_since("launcher.release_list_ms", checked);
                    }
                    self.releases = r.map_or_else(Load::Failed, Load::Ready)
                }
                Msg::Links(links) => self.links = links,
                // The page reads is_prepared when drawn; `changed` redraws it.
                Msg::Reused(_) => self.installed = backend::installed(),
                Msg::Keychain(persistent) => self.keychain = persistent,
                Msg::Session(session) => {
                    if let Some(tag) = self.starting_client.take() {
                        let mut cmd = backend::client_command(&tag);
                        match session {
                            Ok(Some(file)) => {
                                cmd.arg("--session-file").arg(file);
                            }
                            Ok(None) => {}
                            // Still play, offline, and say why.
                            Err(e) => self.client_error = Some(format!("Playing offline: {e}")),
                        }
                        self.accounts = Accounts::load();
                        if let Some(work) = self.starting_pack.take() {
                            if let Err(e) =
                                horizon_launcher::modpacks::apply_profile(&mut cmd, &work)
                            {
                                self.client_error = Some(e);
                                continue;
                            }
                        }
                        if let Some(code) = self.joining.take() {
                            if let Err(e) = backend::join_friend(&mut cmd, &code) {
                                self.client_error = Some(e);
                            }
                        }
                        backend::append_game_args(&mut cmd);
                        self.client_home = cmd.get_current_dir().map(|d| d.to_path_buf());
                        match backend::spawn(Job::Client, cmd, self.tx.clone()) {
                            Ok(r) => self.client = Some(r),
                            Err(e) => self.client_error = Some(e),
                        }
                    }
                }
                Msg::Progress(tag, p) => {
                    self.bar = (self.bar.1, p);
                    self.installing = Some((tag, p));
                }
                Msg::Installed(tag, result) => {
                    self.installing = None;
                    self.bar = (0., 0.);
                    self.installed = backend::installed();
                    let as_update = self.update_install.take().filter(|(t, _)| *t == tag);
                    if let (Ok(()), Some(started)) = (&result, self.install_started.take()) {
                        let name = if as_update.is_some() {
                            "launcher.update_ms"
                        } else {
                            "launcher.install_ms"
                        };
                        self.perf.record_since(name, started);
                    }
                    match result {
                        Ok(()) => {
                            if self.updates.horizon.as_ref().is_some_and(|r| r.tag == tag) {
                                self.updates.horizon = None;
                            }
                            if as_update.is_none_or(|(_, select)| select) {
                                self.settings.selected = Some(tag);
                            }
                            self.settings.save();
                        }
                        Err(e) => self.install_error = Some((tag, e)),
                    }
                }
                Msg::Updates(Ok(updates)) => {
                    self.update_error = None;
                    self.updates = updates;
                    if self.settings.auto_install() {
                        self.install_update();
                    }
                    if let Some(release) = self.updates.launcher.clone() {
                        self.update_launcher(release);
                    }
                }
                Msg::Updates(Err(e)) => self.update_error = Some(e),
                Msg::LauncherProgress(p) => {
                    if let SelfUpdate::Downloading(_, f) = &mut self.self_update {
                        *f = p;
                    }
                }
                Msg::LauncherUpdated(result) => {
                    let version = match &self.self_update {
                        SelfUpdate::Downloading(v, _) => v.clone(),
                        _ => String::new(),
                    };
                    self.self_update = match result {
                        Ok(()) => SelfUpdate::Ready(version),
                        Err(e) => SelfUpdate::Failed(e),
                    };
                }
                Msg::PackLine(line) => {
                    self.pack_log.push_back(line);
                    if self.pack_log.len() > LOG_LINES {
                        self.pack_log.pop_front();
                    }
                }
                Msg::PackDone(result) => self.pack_done(result),
                Msg::Line(job, line) => {
                    if job == Job::Client && line.starts_with(perf::GAME_FIRST_FRAME) {
                        if let Some(pressed) = self.play_pressed.take() {
                            self.perf
                                .record_since("launcher.play_to_game_frame_ms", pressed);
                        }
                    }
                    // The MCP server keeps its own server's log; the launcher's are the preparer's
                    // and the game's.
                    if job != Job::Server {
                        self.prepare_log.push_back(line);
                        if self.prepare_log.len() > LOG_LINES {
                            self.prepare_log.pop_front();
                        }
                    }
                }
                Msg::Exit(job, result) => {
                    let error = result.err();
                    match job {
                        Job::Prepare => {
                            self.preparing = false;
                            if let (None, Some(started)) = (&error, self.prepare_started.take()) {
                                self.perf.record_since("launcher.prepare_ms", started);
                            }
                            // A preparer process's exit status says little, so its last line
                            // follows; the launcher's own errors are full sentences already.
                            self.prepare_error = error.map(|e| {
                                let tail = self
                                    .prepare_log
                                    .iter()
                                    .rev()
                                    .find(|l| !l.trim().is_empty())
                                    .filter(|_| e.starts_with("Exited with"));
                                tail.map_or(e.clone(), |t| format!("{e}: {t}"))
                            });
                        }
                        Job::Client => {
                            self.client = None;
                            let report = self
                                .client_started
                                .take()
                                .and_then(backend::crash_report_since);
                            if error.is_some() && report.is_none() && self.switched_to_dx12() {
                                self.client_error = Some(
                                    "Horizon's Vulkan renderer crashed while starting on this \
                                     graphics card, so the game now uses DirectX 12. Press Play \
                                     to start it again. Video Settings > Graphics API switches \
                                     back."
                                        .into(),
                                );
                            } else if error.is_some() || report.is_some() {
                                let summary = report
                                    .as_ref()
                                    .and_then(|p| std::fs::read_to_string(p).ok())
                                    .and_then(|text| backend::crash_summary(&text))
                                    .or_else(|| {
                                        let tail = self
                                            .prepare_log
                                            .iter()
                                            .rev()
                                            .find(|l| !l.trim().is_empty());
                                        error.as_ref().map(|e| {
                                            tail.map_or(e.clone(), |t| format!("{e}: {t}"))
                                        })
                                    })
                                    .unwrap_or_default();
                                self.crash = Some(Crash {
                                    report,
                                    summary,
                                    copied: false,
                                });
                            }
                        }
                        Job::Server => {}
                    }
                }
            }
        }
        changed |= self.servers_tick();
        changed |= self.mods_tick();
        if let Some(sign_in) = &self.sign_in {
            let state = sign_in.state();
            let changed_state = format!("{state:?}") != format!("{:?}", self.sign_in_state);
            if changed_state {
                if let SignInState::Done(Ok(_)) = state {
                    self.accounts = Accounts::load();
                }
                self.sign_in_state = Some(state);
                changed = true;
            }
        }
        if self.checked.is_some_and(|at| at.elapsed() > CHECK_EVERY) {
            self.check_updates();
        }
        if changed {
            cx.notify();
        }
    }

    /// After a crash with no report: when the game's output ends while its Vulkan renderer
    /// started, a native crash in the graphics driver (#746), switches the game to DirectX 12
    /// (Windows only; only where the player left the graphics API on Auto). True when it did.
    fn switched_to_dx12(&self) -> bool {
        if !cfg!(windows) || !backend::vulkan_start_crashed(&self.prepare_log) {
            return false;
        }
        // The GPU is the same for every profile: the game home's setting and the crashed run's.
        let home = backend::game_home();
        let mut homes = vec![home.clone()];
        homes.extend(self.client_home.clone().filter(|h| *h != home));
        let mut switched = false;
        for home in homes {
            match backend::switch_to_dx12(&home) {
                Ok(changed) => switched |= changed,
                Err(e) => eprintln!("Horizon launcher: could not switch to DirectX 12: {e}"),
            }
        }
        switched
    }

    fn selected(&self) -> Option<&String> {
        self.settings
            .selected
            .as_ref()
            .filter(|s| self.installed.contains(s))
    }

    fn install(&mut self, release: &Release) {
        if release.asset.is_none() {
            return;
        }
        let (release, tx) = (release.clone(), self.tx.clone());
        let tag = release.tag.clone();
        self.installing = Some((tag.clone(), 0.0));
        self.install_started = Some(Instant::now());
        self.bar = (0., 0.);
        self.install_error = None;
        std::thread::spawn(move || {
            let progress = |p| {
                let _ = tx.send(Msg::Progress(tag.clone(), p));
            };
            let result = backend::install(&release, progress);
            let _ = tx.send(Msg::Installed(tag, result));
        });
    }

    fn remove(&mut self, tag: &str) {
        if let Err(e) = backend::remove_version(tag) {
            self.install_error = Some((tag.to_string(), e));
        }
        self.confirm_remove = None;
        self.installed = backend::installed();
        self.settings.selected = Settings::load()
            .selected
            .or_else(|| self.installed.first().cloned());
    }

    fn prepare(&mut self) {
        let Some(tag) = self.selected().cloned() else {
            return;
        };
        let minecraft = self
            .settings
            .minecraft_dir
            .clone()
            .unwrap_or_else(|| horizon_prepare::artifacts::minecraft_dir(None));
        self.prepare_log.clear();
        self.prepare_error = None;
        self.prepare_log
            .push_back("Checking Java and your Minecraft install".into());
        let tx = self.tx.clone();
        // The checks may download the server jar, so they run on the thread too.
        std::thread::spawn(move || {
            let log = |line| {
                let _ = tx.send(Msg::Line(Job::Prepare, line));
            };
            let result = backend::preparer(&tag, &minecraft, &log)
                .inspect(|(_, minecraft)| backend::sounds(&tag, minecraft, &log))
                .and_then(|(p, _)| match p {
                    backend::Preparer::Process(cmd) => {
                        backend::spawn(Job::Prepare, cmd, tx.clone()).map(|_running| false)
                    }
                    backend::Preparer::InProcess(options) => {
                        backend::prepare_in_process(&options, &log).map(|_| true)
                    }
                });
            match result {
                // A process sends its own Exit when it ends.
                Ok(false) => {}
                Ok(true) => {
                    let _ = tx.send(Msg::Exit(Job::Prepare, Ok(())));
                }
                Err(e) => {
                    let _ = tx.send(Msg::Exit(Job::Prepare, Err(e)));
                }
            }
        });
        self.preparing = true;
        self.prepare_started = Some(Instant::now());
    }

    fn play(&mut self) {
        let Some(tag) = self.selected().cloned() else {
            return;
        };
        if self.client.is_some() || self.starting_client.is_some() {
            return;
        }
        self.starting_pack = None;
        self.start_client(tag);
    }

    /// Starts the game: the session first (a network round trip when the token needs
    /// renewing), then `drain` starts the process when it arrives.
    fn start_client(&mut self, tag: String) {
        self.client_error = None;
        self.crash = None;
        self.prepare_log.clear();
        // A little earlier than the spawn: file times can be coarse.
        self.client_started =
            Some(std::time::SystemTime::now() - std::time::Duration::from_secs(2));
        // The session (a network round trip when the token needs renewing) comes first;
        // `drain` starts the game when it arrives.
        self.starting_client = Some(tag);
        self.play_pressed = Some(Instant::now());
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let session = (|| {
                let mut accounts = Accounts::load();
                let Some((a, secrets)) = account::session_for_launch(
                    &account::Endpoints::live(),
                    account::store(),
                    &mut accounts,
                )?
                else {
                    return Ok(None);
                };
                account::write_session_file(&backend::game_home().join("sessions"), &a, &secrets)
                    .map(Some)
            })();
            let _ = tx.send(Msg::Session(session));
        });
    }

    fn choose_folder(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = paths.await {
                if let Some(path) = paths.pop() {
                    let _ = this.update(cx, |this, cx| {
                        this.settings.minecraft_dir = Some(path);
                        this.settings.save();
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }
}

// ----- Building blocks ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Primary,
    Secondary,
    Ghost,
}

/// The element under the pointer and the one it just left, with when that happened. Hover
/// backgrounds ease in and out from these rather than snapping.
struct HoverState {
    on: Option<(SharedString, Instant)>,
    off: Option<(SharedString, Instant)>,
}

static HOVER: Mutex<HoverState> = Mutex::new(HoverState {
    on: None,
    off: None,
});

fn track_hover(el: Stateful<Div>, id: SharedString) -> Stateful<Div> {
    el.on_hover(move |hovered, window, _| {
        let mut state = HOVER.lock().unwrap();
        let is_on = state.on.as_ref().is_some_and(|(h, _)| *h == id);
        if *hovered && !is_on {
            state.off = state.on.take();
            state.on = Some((id.clone(), Instant::now()));
        } else if !*hovered && is_on {
            state.off = Some((id.clone(), Instant::now()));
            state.on = None;
        }
        window.refresh();
    })
}

/// A layer under an element's content that fades `color` in while it is hovered. `bleed` lets
/// it reach past the element on the left and right, as the hover plate of a list row does.
fn hover_layer(id: &SharedString, color: Hsla, radius: f32, bleed: f32) -> AnyElement {
    let layer = div()
        .absolute()
        .top_0()
        .bottom_0()
        .left(px(-bleed))
        .right(px(-bleed))
        .rounded(px(radius));
    let state = HOVER.lock().unwrap();
    // Each transition gets its own animation id, from when it started, so it plays once.
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let start = *START.get_or_init(Instant::now);
    let key = |dir: &str, at: Instant| {
        ElementId::named_usize(format!("{id}-{dir}"), (at - start).as_micros() as usize)
    };
    let ease = || Animation::new(theme::QUICK).with_easing(ease_out_quint());
    if let Some((_, at)) = state.on.as_ref().filter(|(h, _)| h == id) {
        return layer
            .with_animation(key("in", *at), ease(), move |d, t| {
                d.bg(color.opacity(color.a * t))
            })
            .into_any_element();
    }
    if let Some((_, at)) = state
        .off
        .as_ref()
        .filter(|(h, at)| h == id && at.elapsed() < theme::QUICK)
    {
        return layer
            .with_animation(key("out", *at), ease(), move |d, t| {
                d.bg(color.opacity(color.a * (1. - t)))
            })
            .into_any_element();
    }
    layer.into_any_element()
}

/// Text set in one step of the type scale.
fn text(step: theme::Step) -> Div {
    let d = div().text_size(px(step.px)).line_height(px(step.line_px()));
    let d = if step.mono {
        d.font_family(theme::MONO)
    } else {
        d
    };
    if step.weight >= 500 {
        d.font_weight(FontWeight::MEDIUM)
    } else {
        d
    }
}

/// A list index: `01`, `02`… in mono, in the column on the left of an index row.
fn index(n: usize, t: &Theme) -> Div {
    text(theme::MICRO)
        .w(px(theme::INDEX))
        .flex_none()
        .text_color(t.muted)
        .child(format!("{n:02}"))
}

/// Horizon's mark: two stems, their reflection below, and the horizon across in the accent.
fn mark(size: f32, t: &Theme) -> Div {
    let u = size / 16.;
    let stem = |x: f32, y: f32, color: Hsla| {
        div()
            .absolute()
            .left(px(x * u))
            .top(px(y * u))
            .w(px(2. * u))
            .h(px(4. * u))
            .bg(color)
    };
    div()
        .relative()
        .flex_none()
        .size(px(size))
        .child(stem(4., 3., t.ink))
        .child(stem(10., 3., t.ink))
        .child(stem(4., 8., t.ink.opacity(0.36)))
        .child(stem(10., 8., t.ink.opacity(0.36)))
        .child(
            div()
                .absolute()
                .left(px(u))
                .top(px(7. * u))
                .w(px(14. * u))
                .h(px(u))
                .bg(t.accent),
        )
}

/// The last thing the player did was press Tab: focused controls show a ring (a click focuses them
/// too, without one).
static KEYBOARD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Makes a control reachable with Tab, pressed with Enter or Space, with a ring when the keyboard
/// put it in focus.
fn keyboard(el: Stateful<Div>, t: &Theme) -> Stateful<Div> {
    let (ring, gap) = (t.accent, t.page);
    // A ring with a gap of page colour, so it shows around filled buttons too. GPUI draws nothing
    // for a shadow without blur: half a pixel keeps it crisp.
    let shadow = |color, spread: f32| gpui::BoxShadow {
        color,
        offset: gpui::point(px(0.), px(0.)),
        blur_radius: px(0.5),
        spread_radius: px(spread),
    };
    el.tab_index(0).focus(move |s| {
        if KEYBOARD.load(Ordering::Relaxed) {
            s.shadow(vec![shadow(ring, 4.), shadow(gap, 1.5)])
        } else {
            s
        }
    })
}

fn button(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    kind: Kind,
    t: &Theme,
) -> Stateful<Div> {
    let id: SharedString = id.into();
    let t = *t;
    let hover = match kind {
        Kind::Primary => t.on_accent.opacity(0.12),
        Kind::Secondary => t.plate,
        Kind::Ghost => t.plate_soft,
    };
    let base = div()
        .id(id.clone())
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(px(theme::CONTROL))
        .px(px(16.))
        .rounded(px(theme::RADIUS_PILL))
        .text_size(px(theme::LABEL.px))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .child(hover_layer(&id, hover, theme::RADIUS_PILL, 0.))
        .child(div().relative().child(label.into()));
    let base = keyboard(track_hover(base, id), &t);
    match kind {
        Kind::Primary => base
            .bg(t.accent)
            .text_color(t.on_accent)
            .active(|s| s.opacity(theme::PRESSED)),
        Kind::Secondary => base
            .bg(t.plate_soft)
            .text_color(t.ink)
            .active(|s| s.opacity(theme::PRESSED)),
        Kind::Ghost => base
            .text_color(t.muted)
            .hover(move |s| s.text_color(t.ink))
            .active(|s| s.opacity(theme::PRESSED)),
    }
}

/// An on/off switch whose knob slides.
fn switch(id: &'static str, on: bool, t: &Theme) -> Stateful<Div> {
    keyboard(div().id(id), t)
        .relative()
        .flex_none()
        .w(px(36.))
        .h(px(20.))
        .rounded_full()
        .cursor_pointer()
        .bg(if on { t.accent } else { t.plate })
        .child(
            div()
                .absolute()
                .top(px(2.))
                .size(px(16.))
                .rounded_full()
                .bg(gpui::white())
                .with_animation(
                    ElementId::named_usize(id, on as usize),
                    Animation::new(theme::MOVE).with_easing(ease_out_quint()),
                    move |d, p| d.left(px(if on { 2. + 16. * p } else { 18. - 16. * p })),
                ),
        )
}

/// A segmented control: a soft pill track, the chosen segment an ink pill.
fn segments<T: Copy + PartialEq + 'static>(
    id: &'static str,
    options: &[(T, &'static str)],
    current: T,
    t: &Theme,
    on_pick: impl Fn(T, &mut Window, &mut App) + Clone + 'static,
) -> Stateful<Div> {
    let t = *t;
    div()
        .id(id)
        .flex()
        .flex_none()
        .p(px(2.))
        .gap(px(2.))
        .rounded(px(theme::RADIUS_PILL))
        .bg(t.plate_soft)
        .children(options.iter().map(|&(value, label)| {
            let active = current == value;
            let pick = on_pick.clone();
            keyboard(div().id(SharedString::from(format!("{id}-{label}"))), &t)
                .flex()
                .items_center()
                .h(px(theme::SEGMENT))
                .px(px(14.))
                .rounded(px(theme::RADIUS_PILL))
                .text_size(px(theme::LABEL.px))
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .when(active, |d| d.bg(t.ink).text_color(t.page))
                .when(!active, |d| {
                    d.text_color(t.muted).hover(move |s| s.text_color(t.ink))
                })
                .child(label)
                .on_click(move |_: &ClickEvent, window, cx| pick(value, window, cx))
        }))
}

fn disabled(b: Stateful<Div>) -> Stateful<Div> {
    b.opacity(theme::DISABLED).cursor_default().tab_stop(false)
}

/// A page's title in display type and one line on what it is for.
fn page_header(title: &str, description: &str, t: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(
            text(theme::DISPLAY)
                .text_color(t.ink)
                .child(title.to_string()),
        )
        .child(
            text(theme::UI)
                .max_w(px(520.))
                .text_color(t.muted)
                .child(description.to_string()),
        )
}

/// One row of an index list: its number, a name with an optional detail under it, then a
/// value or a control on the right, and a rule below.
fn index_row(
    n: usize,
    name: impl IntoElement,
    detail: Option<AnyElement>,
    right: Option<AnyElement>,
    t: &Theme,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(16.))
        .min_h(px(theme::ROW))
        .py(px(12.))
        .border_b_1()
        .border_color(t.rule)
        .child(index(n, t))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(text(theme::UI).text_color(t.ink).child(name))
                .children(detail.map(|d| {
                    text(theme::CAPTION)
                        .text_color(t.muted)
                        .child(d)
                        .into_any_element()
                })),
        )
        .children(right)
}

/// A plate that stands apart from the page: notices, the console, the agents note.
fn plate(t: &Theme) -> Div {
    div().rounded(px(theme::RADIUS_PLATE)).bg(t.plate_soft)
}

/// Something went wrong: a dot in the danger colour, then what happened.
fn notice(text_: String, t: &Theme) -> Div {
    plate(t)
        .flex()
        .gap(px(12.))
        .p(px(16.))
        .text_size(px(theme::UI.px))
        .line_height(px(21.))
        .text_color(t.ink)
        .child(
            div()
                .flex_none()
                .mt(px(7.))
                .size(px(6.))
                .rounded_full()
                .bg(t.danger),
        )
        .child(div().flex_1().min_w_0().child(text_))
}

/// An empty state: a short sentence, and the one thing to do about it.
fn centered(text_: &str, t: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .items_start()
        .gap(px(16.))
        .py(px(48.))
        .child(text(theme::UI).text_color(t.muted).child(text_.to_string()))
}

/// A mono line in upper case: the status above a title, the footer's links.
fn caps(s: impl AsRef<str>) -> Div {
    text(theme::MICRO).child(s.as_ref().to_uppercase())
}

fn short(path: &std::path::Path) -> String {
    let home = dirs::home_dir().unwrap_or_default();
    match path.strip_prefix(&home) {
        Ok(rest) if !home.as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => path.display().to_string(),
    }
}

fn megabytes(bytes: u64) -> String {
    format!("{:.0} MB", bytes as f64 / 1_000_000.0)
}

// ----- Pages -------------------------------------------------------------------------------------

impl Launcher {
    /// The top bar: the mark and the name on the left, the pages on the right.
    fn topbar(&self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let mut nav = div().flex().items_center().gap(px(2.));
        for (page, label) in Page::ALL {
            let active = self.page == page;
            let id = SharedString::from(format!("nav-{label}"));
            let item = div()
                .id(id.clone())
                .relative()
                .flex()
                .items_center()
                .h(px(theme::CONTROL))
                .px(px(12.))
                .rounded(px(theme::RADIUS_PILL))
                .text_size(px(theme::LABEL.px))
                .cursor_pointer()
                .when(active, |d| d.bg(t.plate_soft))
                .when(!active, |d| {
                    d.child(hover_layer(&id, t.plate_soft, theme::RADIUS_PILL, 0.))
                })
                .child(
                    div()
                        .relative()
                        .when(active, |d| {
                            d.text_color(t.ink).font_weight(FontWeight::MEDIUM)
                        })
                        .when(!active, |d| d.text_color(t.muted))
                        .child(label),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.go(page);
                    cx.notify();
                }));
            nav = nav.child(track_hover(item, id));
        }
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .h(px(72.))
            .px(px(theme::GUTTER))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(mark(18., &t))
                    .child(
                        text(theme::LABEL)
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(t.ink)
                            .child("Horizon"),
                    ),
            )
            .child(nav)
    }

    /// The footer: the launcher's version (or its update), and the community links.
    fn footer(&self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let link = |id: &'static str, label: &'static str, url: String| {
            caps(label)
                .id(id)
                .text_color(t.muted)
                .cursor_pointer()
                .hover(move |s| s.text_color(t.ink))
                .on_click(move |_, _, cx| cx.open_url(&url))
        };
        let version = match &self.self_update {
            SelfUpdate::Ready(version) => div()
                .flex()
                .gap(px(12.))
                .child(caps(format!("Launcher updated to {version}")).text_color(t.muted))
                .child(
                    caps("Restart")
                        .id("restart")
                        .text_color(t.accent)
                        .cursor_pointer()
                        .hover(|s| s.opacity(0.8))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.self_update = SelfUpdate::Failed(backend::restart());
                            cx.notify();
                        })),
                ),
            _ => caps(format!("Launcher {}", backend::VERSION)).text_color(t.muted),
        };
        let mut links = div().flex().gap(px(20.));
        if !self.links.discord.is_empty() {
            links = links.child(link("discord", "Discord", self.links.discord.clone()));
        }
        if !self.links.website.is_empty() {
            links = links.child(link("website", "Website", self.links.website.clone()));
        }
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .h(px(48.))
            .px(px(theme::GUTTER))
            .child(version)
            .child(links)
    }

    fn picker(&self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let label = self
            .selected()
            .cloned()
            .unwrap_or_else(|| "No version".into());
        let mut wrap = div().relative().child(
            div()
                .id("picker")
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(theme::CONTROL_LARGE))
                .px(px(16.))
                .rounded(px(theme::RADIUS_PILL))
                .bg(t.plate_soft)
                .font_family(theme::MONO)
                .text_size(px(theme::CAPTION.px))
                .text_color(t.ink)
                .cursor_pointer()
                .hover(move |s| s.bg(t.plate))
                .child(label)
                .child(div().text_color(t.muted).text_size(px(10.)).child("▾"))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.picker_open = !this.picker_open;
                    cx.notify();
                })),
        );
        if self.picker_open {
            let mut menu = div()
                .absolute()
                .top(px(theme::CONTROL_LARGE + 6.))
                .right_0()
                .w(px(240.))
                .p(px(6.))
                .rounded(px(theme::RADIUS_PLATE))
                .border_1()
                .border_color(t.rule)
                .bg(t.surface)
                .flex()
                .flex_col();
            for tag in &self.installed {
                let selected = self.selected() == Some(tag);
                let tag = tag.clone();
                menu = menu.child(
                    div()
                        .id(SharedString::from(format!("pick-{tag}")))
                        .flex()
                        .items_center()
                        .justify_between()
                        .h(px(32.))
                        .px(px(10.))
                        .rounded(px(theme::RADIUS))
                        .font_family(theme::MONO)
                        .text_size(px(theme::CAPTION.px))
                        .text_color(t.ink)
                        .cursor_pointer()
                        .hover(move |s| s.bg(t.plate_soft))
                        .child(tag.clone())
                        .when(selected, |d| {
                            d.child(div().size(px(6.)).rounded_full().bg(t.accent))
                        })
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.settings.selected = Some(tag.clone());
                            this.settings.save();
                            this.picker_open = false;
                            cx.notify();
                        })),
                );
            }
            menu = menu.child(div().my(px(6.)).h(px(1.)).bg(t.rule)).child(
                div()
                    .id("manage")
                    .flex()
                    .items_center()
                    .h(px(32.))
                    .px(px(10.))
                    .rounded(px(theme::RADIUS))
                    .text_size(px(theme::LABEL.px))
                    .text_color(t.muted)
                    .cursor_pointer()
                    .hover(move |s| s.bg(t.plate_soft).text_color(t.ink))
                    .child("Manage versions")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.go(Page::Versions);
                        this.picker_open = false;
                        cx.notify();
                    })),
            );
            wrap = wrap.child(gpui::deferred(menu).with_priority(1));
        }
        wrap
    }

    fn play_page(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let Some(tag) = self.selected().cloned() else {
            return div()
                .flex()
                .flex_col()
                .gap(px(24.))
                .child(page_header(
                    "Play",
                    "Start the game, or run a server for others to join.",
                    &t,
                ))
                .child(centered("No version installed yet.", &t).child(
                    button("go-versions", "Choose a version", Kind::Secondary, &t).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.go(Page::Versions);
                            cx.notify();
                        }),
                    ),
                ));
        };
        let prepared = backend::is_prepared(&tag);
        // Prepared without sounds (#1035): Prepare again is the fix, so it stands out.
        let silent = prepared && !backend::has_sounds(&tag);
        let version = backend::game_version(&tag);
        let hero = self.hero(&tag, &version, prepared, cx);
        let running = self.client.is_some() || self.starting_client.is_some();
        let play_label = if running { "Running" } else { "Play" };
        let mut play = button("play", play_label, Kind::Primary, &t)
            .min_w(px(132.))
            .h(px(theme::CONTROL_LARGE))
            .px(px(28.))
            .text_size(px(theme::UI.px));
        if !prepared || self.preparing || running {
            play = disabled(play);
        } else {
            play = play.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.picker_open = false;
                this.play();
                cx.notify();
            }));
        }
        let actions = div()
            .flex()
            .items_center()
            .justify_between()
            .child(
                button("host", "Host a server", Kind::Ghost, &t).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.go(Page::Servers);
                        cx.notify();
                    },
                )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(self.picker(cx))
                    .child(play),
            );

        let resources_value = if self.preparing {
            "Preparing…".to_string()
        } else {
            backend::resources_summary(prepared, !silent, &version)
        };
        let mut prepare = button(
            "prepare",
            if prepared { "Prepare again" } else { "Prepare" },
            if prepared && !silent {
                Kind::Ghost
            } else {
                Kind::Secondary
            },
            &t,
        );
        if self.preparing {
            prepare = disabled(prepare);
        } else {
            prepare = prepare.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.prepare();
                cx.notify();
            }));
        }
        let minecraft = self.settings.minecraft_dir.as_deref().map_or_else(
            || {
                div()
                    .child("Not found. Choose the folder where your launcher keeps Minecraft.")
                    .into_any_element()
            },
            |p| {
                div()
                    .font_family(theme::MONO)
                    .child(short(p))
                    .into_any_element()
            },
        );
        let mut rows = div()
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(t.rule)
            .child(index_row(
                1,
                "Resources",
                Some(div().child(resources_value).into_any_element()),
                Some(prepare.into_any_element()),
                &t,
            ))
            .child(index_row(
                2,
                "Minecraft folder",
                Some(minecraft),
                Some(
                    button("choose-mc", "Choose…", Kind::Ghost, &t)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.choose_folder(cx)))
                        .into_any_element(),
                ),
                &t,
            ));
        {
            let who = match self.accounts.selected() {
                Some(a) => format!("{} (Microsoft account)", a.name),
                None => "Offline: online servers need a Microsoft account.".to_string(),
            };
            rows = rows.child(self.join_friend_row(prepared && !running, cx));
            rows = rows.child(index_row(
                4,
                "Playing as",
                Some(div().child(who).into_any_element()),
                Some(
                    button("accounts", "Accounts…", Kind::Ghost, &t)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.go(Page::Settings);
                            cx.notify();
                        }))
                        .into_any_element(),
                ),
                &t,
            ));
        }

        let mut page = div()
            .flex()
            .flex_col()
            .gap(px(32.))
            .child(hero)
            .child(actions);
        // The preparation's progress and error right under the buttons, where the player
        // looks after pressing Prepare, not below the fold (#666).
        if self.preparing {
            let last = self.prepare_log.back().cloned().unwrap_or_default();
            page = page.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(progress_bar(None, &t))
                    .child(
                        text(theme::MICRO)
                            .text_color(t.muted)
                            .truncate()
                            .child(last),
                    )
                    .child(
                        text(theme::CAPTION)
                            .text_color(t.muted)
                            .child("The first preparation takes a few minutes."),
                    ),
            );
        }
        if let Some(e) = self.prepare_error.clone() {
            page = page.child(notice(e, &t));
        }
        page = page.child(rows);
        if let Some(e) = self.client_error.clone() {
            page = page.child(notice(e, &t));
        }
        if let Some(crash) = self.crash.clone() {
            page = page.child(self.crash_notice(crash, cx));
        }
        // The signature: the name, huge and faint, at the foot of the page.
        page.child(
            div()
                .mt(px(32.))
                .ml(px(-14.))
                .h(px(theme::WORDMARK.px * 0.74))
                .overflow_hidden()
                .text_size(px(theme::WORDMARK.px))
                .line_height(px(theme::WORDMARK.px))
                .font_weight(FontWeight::MEDIUM)
                .text_color(if t.dark { t.plate_soft } else { t.plate })
                .child("horizon"),
        )
    }

    /// "Horizon crashed": the report and the logs, and where to report it.
    fn crash_notice(&self, crash: Crash, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let mut actions = div().flex().flex_wrap().gap(px(8.)).mt(px(8.));
        if let Some(report) = crash.report.clone() {
            let open = report.clone();
            actions = actions
                .child(
                    button("crash-open", "Open report", Kind::Secondary, &t)
                        .bg(t.plate)
                        .on_click(move |_, _, cx| cx.open_with_system(&open)),
                )
                .child(
                    button(
                        "crash-copy",
                        if crash.copied {
                            "Copied"
                        } else {
                            "Copy report"
                        },
                        Kind::Secondary,
                        &t,
                    )
                    .bg(t.plate)
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            if let Ok(text) = std::fs::read_to_string(&report) {
                                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                                if let Some(c) = &mut this.crash {
                                    c.copied = true;
                                }
                            }
                            cx.notify();
                        },
                    )),
                );
        }
        actions = actions.child(
            button("crash-logs", "Open logs folder", Kind::Secondary, &t)
                .bg(t.plate)
                .on_click(|_, _, cx| {
                    let dir = backend::logs_dir();
                    let _ = std::fs::create_dir_all(&dir);
                    cx.open_with_system(&dir);
                }),
        );
        let discord = self.links.discord.clone();
        if !discord.is_empty() {
            actions = actions.child(
                button("crash-discord", "Report on Discord", Kind::Secondary, &t)
                    .bg(t.plate)
                    .on_click(move |_, _, cx| cx.open_url(&discord)),
            );
        }
        actions = actions.child(
            button("crash-dismiss", "Dismiss", Kind::Ghost, &t).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    this.crash = None;
                    cx.notify();
                },
            )),
        );
        let help = if crash.report.is_some() {
            "When you report it on Discord, attach the crash report file (or paste it with Copy report) and say what you were doing."
        } else {
            "The game ended without a crash report. When you report it on Discord, attach latest.log from the logs folder and say what you were doing."
        };
        let mut body = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                text(theme::HEADING)
                    .text_color(t.ink)
                    .child("Horizon crashed"),
            );
        if !crash.summary.is_empty() {
            body = body.child(div().text_color(t.ink_soft).child(crash.summary));
        }
        if let Some(report) = &crash.report {
            body = body.child(
                text(theme::MICRO)
                    .text_color(t.muted)
                    .truncate()
                    .child(short(report)),
            );
        }
        body = body.child(actions).child(
            text(theme::CAPTION)
                .mt(px(4.))
                .text_color(t.muted)
                .child(help),
        );
        plate(&t)
            .flex()
            .gap(px(14.))
            .p(px(24.))
            .text_size(px(theme::UI.px))
            .line_height(px(21.))
            .text_color(t.ink)
            .child(
                div()
                    .flex_none()
                    .mt(px(11.))
                    .size(px(6.))
                    .rounded_full()
                    .bg(t.danger),
            )
            .child(body)
    }

    /// The Play page's focal point: what will start, and a line from its notes.
    fn hero(&self, tag: &str, version: &str, prepared: bool, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let summary = match &self.releases {
            Load::Ready(list) => list
                .iter()
                .find(|r| r.tag == tag)
                .and_then(|r| backend::notes_summary(&r.body)),
            _ => None,
        };
        let title = "Horizon";
        let mut hero =
            div()
                .flex()
                .flex_col()
                .gap(px(14.))
                .pt(px(24.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(caps(format!("Minecraft {version}")).text_color(t.muted))
                        .child(div().size(px(4.)).rounded_full().bg(t.rule))
                        .child(
                            caps(if prepared { "Ready" } else { "Needs preparing" })
                                .text_color(if prepared { t.success } else { t.muted }),
                        ),
                )
                .child(text(theme::DISPLAY).text_color(t.ink).child(title))
                .child(text(theme::BODY).max_w(px(560.)).text_color(t.muted).child(
                    match summary {
                        Some(line) => format!("{}. {line}", tag.trim_start_matches('v')),
                        None => tag.trim_start_matches('v').to_string(),
                    },
                ));
        if let Some(release) = &self.updates.horizon {
            let installing = self.installing.as_ref().filter(|(t, _)| *t == release.tag);
            let line = div()
                .flex()
                .items_center()
                .gap(px(10.))
                .text_size(px(theme::CAPTION.px))
                .text_color(t.muted)
                .child(div().size(px(6.)).rounded_full().bg(t.accent))
                .child(format!("{} available", release.tag));
            hero = hero.child(match installing {
                Some((_, p)) => line
                    .child(
                        div()
                            .w(px(120.))
                            .flex()
                            .child(progress_bar(Some(self.bar), &t)),
                    )
                    .child(format!("{:.0}%", p * 100.)),
                None => line.child(
                    div()
                        .id("install-update")
                        .text_color(t.accent)
                        .font_weight(FontWeight::MEDIUM)
                        .cursor_pointer()
                        .hover(|s| s.opacity(0.8))
                        .child("Install")
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.install_update();
                            cx.notify();
                        })),
                ),
            });
        }
        hero
    }

    fn settings_page(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let toggle = switch("auto-install", self.settings.auto_install(), &t).on_click(
            cx.listener(|this, _: &ClickEvent, _, cx| {
                this.settings.install_new_versions = Some(!this.settings.auto_install());
                this.settings.save();
                if this.settings.auto_install() {
                    this.install_update();
                }
                cx.notify();
            }),
        );
        let betas = switch("beta-builds", self.settings.beta_builds, &t).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                this.settings.beta_builds = !this.settings.beta_builds;
                this.settings.save();
                // What counts as new changed: check again (and install it if that is on).
                this.check_updates();
                cx.notify();
            },
        ));
        let opening = switch("opening", !self.settings.skip_opening, &t).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                this.settings.skip_opening = !this.settings.skip_opening;
                this.settings.save();
                cx.notify();
            },
        ));
        // Linux: the launcher's entry in the applications menu (`horizon_launcher::desktop`).
        let menu_entry = switch("desktop-entry", self.settings.desktop_entry(), &t).on_click(
            cx.listener(|this, _: &ClickEvent, _, cx| {
                let on = !this.settings.desktop_entry();
                this.settings.desktop_entry = Some(on);
                this.settings.save();
                let _ = if on {
                    horizon_launcher::desktop::install(backend::exe_path()).map(|_| ())
                } else {
                    horizon_launcher::desktop::remove()
                };
                cx.notify();
            }),
        );
        let this = cx.entity().downgrade();
        let menus = segments(
            "menu-style",
            &[
                (backend::MenuStyle::Horizon, "Horizon"),
                (backend::MenuStyle::Classic, "Classic"),
            ],
            self.menu_style,
            &t,
            move |style, _, cx| {
                let _ = this.update(cx, |this, cx| {
                    if backend::set_menu_style(style).is_ok() {
                        this.menu_style = style;
                    }
                    cx.notify();
                });
            },
        );
        let horizon = match (&self.updates.horizon, &self.update_error) {
            (Some(r), _) => format!("{} available", r.tag),
            (None, Some(e)) => e.clone(),
            (None, None) if self.checked.is_some() => "Up to date".to_string(),
            _ => "Checking…".to_string(),
        };
        let horizon_action = self
            .updates
            .horizon
            .as_ref()
            .filter(|_| self.installing.is_none())
            .map(|_| {
                button("settings-install", "Install", Kind::Secondary, &t)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.install_update();
                        cx.notify();
                    }))
                    .into_any_element()
            });
        let launcher = match &self.self_update {
            SelfUpdate::Downloading(v, p) => {
                format!("{}  ·  downloading {v}, {:.0}%", backend::VERSION, p * 100.)
            }
            SelfUpdate::Ready(v) => format!("Updated to {v}. Restart the launcher to use it."),
            SelfUpdate::Failed(e) => format!("{}  ·  the update failed: {e}", backend::VERSION),
            SelfUpdate::Idle => match &self.updates.launcher {
                Some(r) => format!(
                    "{}  ·  {} available",
                    backend::VERSION,
                    backend::launcher_version(&r.tag).unwrap_or_default()
                ),
                None if self.checked.is_some() && self.update_error.is_none() => {
                    format!("{}  ·  up to date", backend::VERSION)
                }
                None => backend::VERSION.to_string(),
            },
        };
        let launcher_action = match &self.self_update {
            SelfUpdate::Ready(_) => Some(
                button("restart-settings", "Restart", Kind::Secondary, &t)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.self_update = SelfUpdate::Failed(backend::restart());
                        cx.notify();
                    }))
                    .into_any_element(),
            ),
            SelfUpdate::Downloading(..) => None,
            _ => Some(
                button("check", "Check now", Kind::Ghost, &t)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.check_updates();
                        cx.notify();
                    }))
                    .into_any_element(),
            ),
        };
        let mono = |s: String| div().font_family(theme::MONO).child(s).into_any_element();
        let plain = |s: &str| div().child(s.to_string()).into_any_element();
        let agents = plate(&t)
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(24.))
            .text_size(px(theme::UI.px))
            .line_height(px(21.))
            .child(
                text(theme::HEADING)
                    .text_color(t.ink)
                    .child("Use from the command line or AI agents"),
            )
            .child(div().max_w(px(600.)).text_color(t.muted).child(
                "The launcher is also a command-line tool and an MCP server. Everything here works from a terminal, or from an agent such as Claude Code:",
            ))
            .child(
                text(theme::CAPTION)
                    .font_family(theme::MONO)
                    .text_color(t.ink)
                    .child("claude mcp add horizon -- horizon-launcher mcp"),
            )
            .child(
                div()
                    .id("readme")
                    .text_color(t.accent)
                    .cursor_pointer()
                    .hover(|s| s.opacity(0.8))
                    .child("Commands and tools, in the README")
                    .on_click(|_, _, cx| cx.open_url(backend::README_URL)),
            );
        let (accounts, n) = self.account_rows(cx);
        div()
            .flex()
            .flex_col()
            .gap(px(32.))
            .child(page_header(
                "Settings",
                "Accounts, updates, menus, and the launcher itself.",
                &t,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .border_t_1()
                    .border_color(t.rule)
                    .child(accounts)
                    .child(index_row(
                        n + 1,
                        "Install new versions automatically",
                        Some(plain(
                            "Your selected version only changes if you were playing the newest one.",
                        )),
                        Some(toggle.into_any_element()),
                        &t,
                    ))
                    .child(index_row(
                        n + 2,
                        "Beta builds",
                        Some(plain(
                            "New versions as soon as they are released, before they become stable 6 hours later. Off: only stable versions install by themselves.",
                        )),
                        Some(betas.into_any_element()),
                        &t,
                    ))
                    .child(index_row(
                        n + 3,
                        "Show the opening",
                        Some(plain("The wordmark, while the release list loads.")),
                        Some(opening.into_any_element()),
                        &t,
                    ))
                    .child(index_row(
                        n + 4,
                        "Game menus",
                        Some(plain(
                            "Classic is vanilla's own screens, with no way back inside the game: switch back here. Applies the next time the game starts.",
                        )),
                        Some(menus.into_any_element()),
                        &t,
                    ))
                    .child(self.game_args_row(n + 5, cx))
                    .child(index_row(n + 6, "Horizon", Some(plain(&horizon)), horizon_action, &t))
                    .child(index_row(
                        n + 7,
                        "Launcher",
                        Some(plain(&launcher)),
                        launcher_action,
                        &t,
                    ))
                    .child(index_row(
                        n + 8,
                        "Data folder",
                        Some(mono(short(&backend::data_dir()))),
                        None,
                        &t,
                    ))
                    .child(index_row(
                        n + 9,
                        "Logs",
                        Some(mono(format!(
                            "{}  ·  crash reports beside it",
                            short(&backend::logs_dir())
                        ))),
                        Some(
                            button("open-logs", "Open", Kind::Secondary, &t)
                                .on_click(|_, _, cx| {
                                    let dir = backend::logs_dir();
                                    let _ = std::fs::create_dir_all(&dir);
                                    cx.open_with_system(&dir);
                                })
                                .into_any_element(),
                        ),
                        &t,
                    ))
                    .when(cfg!(target_os = "linux"), |d| {
                        d.child(index_row(
                            n + 10,
                            "Applications menu",
                            Some(plain(if self.settings.desktop_entry() {
                                "Horizon and its icon are in your desktop's menu."
                            } else {
                                "Not in your desktop's menu."
                            })),
                            Some(menu_entry.into_any_element()),
                            &t,
                        ))
                    }),
            )
            .child(self.collection_section(cx))
            .child(agents)
    }

    /// Settings' Game arguments row: the extra arguments the game starts with, kept in
    /// `settings.json` and appended to the launcher's own (quotes group a word with spaces).
    fn game_args_row(&self, n: usize, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let focused = self.args_focused;
        let field = div()
            .id("game-args")
            .track_focus(&self.args_focus)
            .flex()
            .items_center()
            .w(px(300.))
            .h(px(30.))
            .px(px(12.))
            .overflow_hidden()
            .rounded(px(6.))
            .border_1()
            .border_color(if focused { t.accent } else { t.rule })
            .bg(t.surface)
            .font_family(theme::MONO)
            .text_size(px(12.))
            .cursor_text()
            .child(if self.args_input.is_empty() && !focused {
                div()
                    .text_color(t.muted)
                    .child("For example --server-thread")
            } else {
                // The end of a long line stays in view while typing.
                let chars: Vec<char> = self.args_input.chars().collect();
                let shown: String = chars[chars.len().saturating_sub(44)..].iter().collect();
                div().text_color(t.ink).child(shown)
            })
            .when(focused, |d| d.child(div().w(px(1.)).h(px(14.)).bg(t.ink)))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                window.focus(&this.args_focus);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                let k = &ev.keystroke;
                let paste = k.key == "v" && (k.modifiers.control || k.modifiers.platform);
                match k.key.as_str() {
                    "backspace" => {
                        this.args_input.pop();
                    }
                    _ if paste => {
                        if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                            this.args_input.push_str(text.trim());
                        }
                    }
                    _ if !k.modifiers.control && !k.modifiers.platform => {
                        if let Some(ch) = &k.key_char {
                            this.args_input.push_str(ch);
                        }
                    }
                    _ => return,
                }
                this.args_input.truncate(1024);
                this.settings.game_args = this.args_input.clone();
                this.settings.save();
                cx.stop_propagation();
                cx.notify();
            }));
        index_row(
            n,
            "Game arguments",
            Some(
                div()
                    .child("Extra arguments for the game, passed through as-is (quotes group a word with spaces). Applies the next time the game starts.")
                    .into_any_element(),
            ),
            Some(field.into_any_element()),
            &t,
        )
    }

    /// Settings' Microsoft accounts: who is signed in and plays, the sign-in with its code. The
    /// rows are numbered from 01; returns them and how many there are.
    fn account_rows(&mut self, cx: &mut Context<Self>) -> (Div, usize) {
        let t = self.theme;
        let mut rows = div().flex().flex_col();
        let mut n = 0;
        let selected = self.accounts.selected.clone();
        for (i, a) in self.accounts.accounts.clone().into_iter().enumerate() {
            let plays = selected.as_ref() == Some(&a.uuid);
            let uuid = a.uuid.clone();
            let mut actions = div().flex().gap(px(4.));
            if plays {
                actions = actions.child(
                    button(format!("offline-{i}"), "Play offline", Kind::Ghost, &t).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.accounts.selected = None;
                            let _ = this.accounts.save();
                            cx.notify();
                        }),
                    ),
                );
            } else {
                let uuid = uuid.clone();
                actions = actions.child(
                    button(format!("select-{i}"), "Play as", Kind::Secondary, &t).on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.accounts.selected = Some(uuid.clone());
                            let _ = this.accounts.save();
                            cx.notify();
                        }),
                    ),
                );
            }
            actions = actions.child(
                button(format!("sign-out-{i}"), "Sign out", Kind::Ghost, &t).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        let store = account::store();
                        if let Err(e) = this.accounts.remove(store, &uuid) {
                            this.sign_in_state = Some(SignInState::Done(Err(e)));
                        }
                        let _ = this.accounts.save();
                        cx.notify();
                    },
                )),
            );
            n += 1;
            rows = rows.child(index_row(
                n,
                a.name.clone(),
                Some(
                    div()
                        .child(if plays {
                            "Microsoft account · plays"
                        } else {
                            "Microsoft account · signed in"
                        })
                        .into_any_element(),
                ),
                Some(actions.into_any_element()),
                &t,
            ));
        }
        let configured = account::client_id().is_some();
        let busy = matches!(
            self.sign_in_state,
            Some(SignInState::Starting | SignInState::Code(_))
        );
        let detail = match &self.sign_in_state {
            _ if !configured => div().child(account::NOT_CONFIGURED),
            Some(SignInState::Starting) => div().child("Asking Microsoft…"),
            Some(SignInState::Code(code)) => div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(format!(
                    "Open {} in your browser and enter this code:",
                    code.verification_uri
                ))
                .child(
                    text(theme::HEADING)
                        .font_family(theme::MONO)
                        .text_color(t.ink)
                        .child(code.user_code.clone()),
                ),
            Some(SignInState::Done(Ok(a))) => div().child(format!("Signed in as {}.", a.name)),
            Some(SignInState::Done(Err(e))) => div().text_color(t.danger).child(e.clone()),
            None if self.accounts.accounts.is_empty() => div().child(
                "Sign in to play on online servers with your name and skin. Without an account the game plays offline.",
            ),
            None => div().child("Add another account."),
        };
        let detail = if self.keychain || self.accounts.accounts.is_empty() {
            detail
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(detail)
                .child(div().child(account::NO_KEYCHAIN))
        };
        let action = if !configured {
            None
        } else if let Some(SignInState::Code(code)) = &self.sign_in_state {
            let (uri, user_code) = (code.verification_uri.clone(), code.user_code.clone());
            Some(
                div()
                    .flex()
                    .gap(px(4.))
                    .child(
                        button("sign-in-copy", "Copy code", Kind::Secondary, &t).on_click(
                            move |_, _, cx| {
                                cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                    user_code.clone(),
                                ))
                            },
                        ),
                    )
                    .child(
                        button("sign-in-open", "Open link", Kind::Secondary, &t)
                            .on_click(move |_, _, cx| cx.open_url(&uri)),
                    )
                    .child(
                        button("sign-in-cancel", "Cancel", Kind::Ghost, &t).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                if let Some(s) = this.sign_in.take() {
                                    s.cancel();
                                }
                                this.sign_in_state = None;
                                cx.notify();
                            },
                        )),
                    )
                    .into_any_element(),
            )
        } else if busy {
            None
        } else {
            Some(
                button("sign-in", "Sign in with Microsoft", Kind::Secondary, &t)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.sign_in = Some(SignIn::start());
                        this.sign_in_state = Some(SignInState::Starting);
                        cx.notify();
                    }))
                    .into_any_element(),
            )
        };
        n += 1;
        let title = if self.accounts.accounts.is_empty() {
            "Microsoft account"
        } else {
            "Another account"
        };
        rows = rows.child(index_row(
            n,
            title,
            Some(detail.into_any_element()),
            action,
            &t,
        ));
        (rows, n)
    }

    /// Join a friend: a code field and Join, which starts the game straight
    /// into the friend's world (errors show in the game).
    fn join_friend_row(&self, can_play: bool, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let focused = self.friend_focused;
        let field = div()
            .id("friend-code")
            .track_focus(&self.friend_focus)
            .flex()
            .items_center()
            .w(px(260.))
            .h(px(30.))
            .px(px(12.))
            .overflow_hidden()
            .rounded(px(6.))
            .border_1()
            .border_color(if focused { t.accent } else { t.rule })
            .bg(t.surface)
            .font_family(theme::MONO)
            .text_size(px(12.))
            .cursor_text()
            .child(if self.friend_code.is_empty() && !focused {
                div().text_color(t.muted).child("Paste an invite code")
            } else {
                // The end of a long code stays in view while typing.
                let shown: String = {
                    let chars: Vec<char> = self.friend_code.chars().collect();
                    chars[chars.len().saturating_sub(30)..].iter().collect()
                };
                div().text_color(t.ink).child(shown)
            })
            .when(focused, |d| d.child(div().w(px(1.)).h(px(14.)).bg(t.ink)))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                window.focus(&this.friend_focus);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                let k = &ev.keystroke;
                let paste = k.key == "v" && (k.modifiers.control || k.modifiers.platform);
                match k.key.as_str() {
                    "enter" => this.join_friend(),
                    "backspace" => {
                        this.friend_code.pop();
                    }
                    _ if paste => {
                        if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                            this.friend_code.push_str(text.trim());
                        }
                    }
                    _ if !k.modifiers.control && !k.modifiers.platform => {
                        if let Some(ch) = &k.key_char {
                            this.friend_code.push_str(ch);
                        }
                    }
                    _ => return,
                }
                this.friend_code.truncate(256);
                cx.stop_propagation();
                cx.notify();
            }));
        let valid = horizon_friends::invite::parse(&self.friend_code).is_some();
        let mut join = button("join-friend", "Join", Kind::Secondary, &t);
        if can_play && valid {
            join = join.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.join_friend();
                cx.notify();
            }));
        } else {
            join = disabled(join);
        }
        index_row(
            3,
            "Join a friend",
            Some(
                div()
                    .child("Paste the code a friend shows when they invite you.")
                    .into_any_element(),
            ),
            Some(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(field)
                    .child(join)
                    .into_any_element(),
            ),
            &t,
        )
    }

    fn join_friend(&mut self) {
        if horizon_friends::invite::parse(&self.friend_code).is_none() {
            return;
        }
        self.joining = Some(self.friend_code.clone());
        self.play();
        if self.starting_client.is_none() {
            self.joining = None;
        }
    }

    fn versions_page(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let header = page_header(
            "Versions",
            "Builds from GitHub. Each one installs into its own folder.",
            &t,
        );
        let page = div().flex().flex_col().gap(px(32.)).child(header);
        let releases = match &self.releases {
            Load::Loading => return page.child(centered("Loading releases…", &t)),
            Load::Failed(e) => {
                return page.child(centered(&format!("Couldn't load releases. {e}"), &t).child(
                    button("retry", "Try again", Kind::Secondary, &t).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.fetch_releases();
                            cx.notify();
                        },
                    )),
                ))
            }
            Load::Ready(r) if r.is_empty() => return page.child(centered("No releases yet.", &t)),
            Load::Ready(r) => r.clone(),
        };
        let mut list = div().flex().flex_col().border_t_1().border_color(t.rule);
        for (i, release) in releases.into_iter().enumerate() {
            let tag = release.tag.clone();
            let installed = self.installed.contains(&tag);
            let selected = self.selected() == Some(&tag);
            let channel = if release.beta { "Beta" } else { "Stable" };
            let mut meta = vec![channel.to_string(), release.date.clone()];
            if let Some((_, size)) = release.asset {
                meta.push(megabytes(size));
            }
            let summary = release
                .name
                .split_once(": ")
                .map(|(_, rest)| rest.to_string())
                .unwrap_or_else(|| release.name.clone());
            let right: gpui::AnyElement =
                if let Some((_, p)) = self.installing.as_ref().filter(|(t, _)| *t == tag) {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .w(px(180.))
                        .child(progress_bar(Some(self.bar), &t))
                        .child(
                            text(theme::MICRO)
                                .w(px(36.))
                                .text_color(t.muted)
                                .child(format!("{:.0}%", p * 100.)),
                        )
                        .into_any_element()
                } else if installed {
                    let confirming = self.confirm_remove.as_ref() == Some(&tag);
                    let (t1, t2) = (tag.clone(), tag.clone());
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .when(!selected && !confirming, |d| {
                            d.child(
                                button(format!("use-{tag}"), "Use", Kind::Ghost, &t).on_click(
                                    cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        this.settings.selected = Some(t1.clone());
                                        this.settings.save();
                                        cx.notify();
                                    }),
                                ),
                            )
                        })
                        .child(
                            button(
                                format!("remove-{tag}"),
                                if confirming {
                                    "Remove for good"
                                } else {
                                    "Remove"
                                },
                                if confirming {
                                    Kind::Secondary
                                } else {
                                    Kind::Ghost
                                },
                                &t,
                            )
                            .when(confirming, |b| b.text_color(t.danger))
                            .on_click(cx.listener(
                                move |this, _: &ClickEvent, _, cx| {
                                    if this.confirm_remove.as_ref() == Some(&t2) {
                                        this.remove(&t2);
                                    } else {
                                        this.confirm_remove = Some(t2.clone());
                                    }
                                    cx.notify();
                                },
                            )),
                        )
                        .into_any_element()
                } else if release.asset.is_none() {
                    text(theme::CAPTION)
                        .text_color(t.muted)
                        .child("No build for this platform")
                        .into_any_element()
                } else {
                    let mut b = button(format!("install-{tag}"), "Install", Kind::Secondary, &t);
                    if self.installing.is_some() {
                        b = disabled(b);
                    } else {
                        b = b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.install(&release);
                            cx.notify();
                        }));
                    }
                    b.into_any_element()
                };
            let status = if selected {
                Some(caps("In use").text_color(t.accent))
            } else if installed {
                Some(caps("Installed").text_color(t.ink_soft))
            } else {
                None
            };
            let error = self
                .install_error
                .as_ref()
                .filter(|(t, _)| *t == tag)
                .map(|(_, e)| e.clone());
            let id = SharedString::from(format!("release-{tag}"));
            let row = div()
                .id(id.clone())
                .relative()
                .flex()
                .flex_col()
                .gap(px(10.))
                .py(px(16.))
                .border_b_1()
                .border_color(t.rule)
                .child(hover_layer(
                    &id,
                    t.plate_soft,
                    theme::RADIUS_PLATE,
                    theme::PLATE_BLEED,
                ))
                .child(
                    div()
                        .relative()
                        .flex()
                        .items_center()
                        .gap(px(16.))
                        .child(index(i + 1, &t))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .gap(px(4.))
                                .child(
                                    text(theme::TITLE)
                                        .text_color(t.ink)
                                        .child(tag.trim_start_matches('v').to_string()),
                                )
                                .child(
                                    text(theme::UI)
                                        .text_color(t.muted)
                                        .truncate()
                                        .child(summary),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .items_end()
                                .gap(px(2.))
                                .flex_none()
                                .children(status)
                                .child(caps(meta.join("  ·  ")).text_color(t.muted)),
                        )
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .w(px(190.))
                                .flex_none()
                                .child(right),
                        ),
                )
                .children(error.map(|e| notice(e, &t)));
            list = list.child(track_hover(row, id));
        }
        page.child(list)
    }

    fn notes_page(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let sponsor = (!self.links.sponsor.is_empty()).then(|| {
            plate(&t)
                .flex()
                .items_center()
                .justify_between()
                .gap(px(16.))
                .p(px(24.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(
                            text(theme::HEADING)
                                .text_color(t.ink)
                                .child("Support the work below"),
                        )
                        .child(text(theme::UI).text_color(t.muted).child(
                            "Horizon is built in the open. Sponsors fund the next release.",
                        )),
                )
                .child(
                    button("sponsor", "Sponsor on GitHub", Kind::Secondary, &t)
                        .bg(t.plate)
                        .on_click({
                            let url = self.links.sponsor.clone();
                            move |_: &ClickEvent, _, cx| cx.open_url(&url)
                        }),
                )
        });
        let page = div()
            .flex()
            .flex_col()
            .gap(px(32.))
            .child(page_header(
                "Patch notes",
                "What changed in each release, newest first.",
                &t,
            ))
            .children(sponsor);
        let releases = match &self.releases {
            Load::Loading => return page.child(centered("Loading patch notes…", &t)),
            Load::Failed(e) => {
                return page.child(
                    centered(&format!("Couldn't load patch notes. {e}"), &t).child(
                        button("retry-notes", "Try again", Kind::Secondary, &t).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.fetch_releases();
                                cx.notify();
                            }),
                        ),
                    ),
                )
            }
            Load::Ready(r) => r.clone(),
        };
        let mut list = div().flex().flex_col().border_t_1().border_color(t.rule);
        for (i, release) in releases.iter().enumerate() {
            let summary = release
                .name
                .split_once(": ")
                .map(|(_, rest)| rest.to_string())
                .unwrap_or_default();
            list = list.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(20.))
                    .py(px(28.))
                    .border_b_1()
                    .border_color(t.rule)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(16.))
                            .child(index(i + 1, &t))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_w_0()
                                    .gap(px(4.))
                                    .child(
                                        text(theme::TITLE)
                                            .text_color(t.ink)
                                            .child(release.tag.trim_start_matches('v').to_string()),
                                    )
                                    .when(!summary.is_empty(), |d| {
                                        d.child(text(theme::UI).text_color(t.muted).child(summary))
                                    }),
                            )
                            .child(caps(release.date.clone()).flex_none().text_color(t.muted)),
                    )
                    .child(
                        div()
                            .pl(px(theme::INDEX + 16.))
                            .max_w(px(theme::INDEX + 16. + 640.))
                            .child(if release.body.trim().is_empty() {
                                text(theme::UI)
                                    .text_color(t.muted)
                                    .child("No notes for this release.")
                                    .into_any_element()
                            } else {
                                markdown::render(&release.body, &t)
                            }),
                    ),
            );
        }
        page.child(list)
    }
}

/// A thin bar. A known fraction eases from its previous value; an unknown one shuttles slowly.
fn progress_bar(fraction: Option<(f32, f32)>, t: &Theme) -> Div {
    let bar = div()
        .absolute()
        .top_0()
        .h(px(2.))
        .rounded_full()
        .bg(t.accent);
    let track = div()
        .relative()
        .flex_1()
        .h(px(2.))
        .rounded_full()
        .bg(t.rule)
        .overflow_hidden();
    match fraction {
        Some((from, to)) => {
            let (from, to) = (from.clamp(0.02, 1.), to.clamp(0.02, 1.));
            track.child(bar.left_0().with_animation(
                ElementId::named_usize("bar", (to * 1000.) as usize),
                Animation::new(theme::MOVE).with_easing(ease_out_quint()),
                move |d, p| d.w(relative(from + (to - from) * p)),
            ))
        }
        None => track.child(
            bar.w(relative(0.3)).with_animation(
                "shuttle",
                Animation::new(Duration::from_millis(1600))
                    .repeat()
                    .with_easing(gpui::ease_in_out),
                |d, p| d.left(relative(-0.3 + 1.3 * p)),
            ),
        ),
    }
}

impl Render for Launcher {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        self.perf.frame();
        self.servers.note_focus(window);
        self.friend_focused = self.friend_focus.is_focused(window);
        self.mods.note_focus(window);
        self.args_focused = self.args_focus.is_focused(window);
        let content = match self.page {
            Page::Play => self.play_page(cx),
            Page::Versions => self.versions_page(cx),
            Page::Mods => self.mods_page(cx),
            Page::Modpacks => self.modpacks_page(cx),
            Page::Servers => self.servers_page(cx),
            Page::Notes => self.notes_page(cx),
            Page::Settings => self.settings_page(cx),
        };
        // The new page fades in and rises a few pixels.
        let content = div()
            .relative()
            .w_full()
            // Let the page shrink inside the padded flex row so long notices can wrap.
            .min_w_0()
            .max_w(px(1152.))
            .child(content)
            .with_animation(
                ElementId::named_usize("page", self.switches),
                Animation::new(theme::ENTER).with_easing(ease_out_quint()),
                |d, p| d.opacity(p).top(px(theme::RISE * (1. - p))),
            );
        let (reveal, held) = self.opening_phase(window);
        let ui = ease_out_cubic((reveal - 0.3) / 0.7);
        if self.opening.is_none() && window.focused(cx).is_none() {
            window.focus(&self.root_focus);
        }
        let mut root = div()
            .id("root")
            .track_focus(&self.root_focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                let k = &ev.keystroke;
                match k.key.as_str() {
                    "tab" => {
                        KEYBOARD.store(true, Ordering::Relaxed);
                        if k.modifiers.shift {
                            window.focus_prev();
                        } else {
                            window.focus_next();
                        }
                    }
                    "escape" if this.servers_escape() => {}
                    _ => return,
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .capture_any_mouse_down(|_, _, _| KEYBOARD.store(false, Ordering::Relaxed))
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(t.page)
            .font_family(theme::FONT)
            .text_color(t.ink)
            .overflow_hidden()
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .size_full()
                    .opacity(ui)
                    .top(px(theme::RISE * (1. - ui)))
                    .child(self.topbar(cx))
                    .child(
                        self.content_scroll
                            .wrap(
                                div()
                                    .id("content")
                                    .relative()
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .child(
                                        div()
                                            .flex()
                                            .px(px(theme::GUTTER))
                                            .pt(px(24.))
                                            .pb(px(48.))
                                            .child(content),
                                    ),
                                &t,
                            )
                            .flex_1(),
                    )
                    .child(self.footer(cx)),
            );
        if self.page == Page::Servers {
            if let Some(dialog) = self.create_dialog(cx) {
                root = root.child(dialog);
            }
        }
        if self.asking_consent() {
            root = root.child(self.consent_screen(cx));
        }
        if let Some(opening) = &self.opening {
            // The wordmark arrives letter by letter, then gives way to the launcher; a click or
            // a key skips ahead.
            let gone = 1. - ease_out_cubic(reveal / 0.6);
            let word = div()
                .flex()
                .text_size(px(theme::WORDMARK.px * 0.6))
                .line_height(px(theme::WORDMARK.px * 0.6))
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.ink)
                .children("horizon".chars().enumerate().map(|(i, c)| {
                    let at = held - i as f32 * theme::STAGGER.as_secs_f32() * 1.5;
                    let p = ease_out_cubic(at / theme::ENTER.as_secs_f32());
                    div()
                        .relative()
                        .opacity(p)
                        .top(px(theme::RISE * 2. * (1. - p)))
                        .child(c.to_string())
                }));
            root = root.child(
                div()
                    .id("opening")
                    .track_focus(&opening.focus)
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(t.page.opacity(gone))
                    .when(reveal == 0., |d| d.cursor_pointer())
                    .child(
                        div()
                            .relative()
                            .top(px(-24. * (1. - gone)))
                            .opacity(gone)
                            .child(word),
                    )
                    .when(reveal == 0., |d| {
                        d.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.skip_opening();
                            cx.notify();
                        }))
                        .on_key_down(cx.listener(
                            |this, _: &KeyDownEvent, _, cx| {
                                this.skip_opening();
                                cx.notify();
                            },
                        ))
                    }),
            );
        }
        root
    }
}

impl Launcher {
    /// Where the opening is: the reveal's progress (1 when there is no opening) and how long
    /// the wordmark has been showing, in seconds.
    fn opening_phase(&mut self, window: &mut Window) -> (f32, f32) {
        let loaded = !matches!(self.releases, Load::Loading);
        let Some(opening) = &mut self.opening else {
            return (1., 10.);
        };
        let held = opening.started.get_or_insert_with(Instant::now).elapsed();
        if opening.reveal.is_none() && ((held >= OPENING_HOLD && loaded) || held >= OPENING_MAX) {
            opening.reveal = Some(Instant::now());
        }
        let reveal = opening.reveal.map_or(0., |at| {
            at.elapsed().as_secs_f32() / OPENING_REVEAL.as_secs_f32()
        });
        if reveal >= 1. {
            self.opening = None;
            return (1., 10.);
        }
        window.request_animation_frame();
        (reveal, held.as_secs_f32())
    }

    fn skip_opening(&mut self) {
        if let Some(opening) = &mut self.opening {
            opening.reveal.get_or_insert_with(Instant::now);
        }
    }
}

fn main() {
    perf::process_started();
    backend::exe_path();
    if let Some(code) = cli::run() {
        std::process::exit(code);
    }
    Application::new().run(|cx: &mut App| {
        cx.text_system()
            .add_fonts(vec![
                Cow::Borrowed(horizon_theme::fonts::GEIST_REGULAR),
                Cow::Borrowed(horizon_theme::fonts::GEIST_MEDIUM),
                Cow::Borrowed(horizon_theme::fonts::GEIST_SEMIBOLD),
                Cow::Borrowed(horizon_theme::fonts::GEIST_MONO),
            ])
            .expect("bundled fonts");
        let bounds = Bounds::centered(None, size(px(1040.), px(700.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(820.), px(560.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Horizon".into()),
                    ..Default::default()
                }),
                app_id: Some("horizon-launcher".into()),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| Launcher::new(window, cx)),
        )
        .expect("window");
        cx.on_window_closed(|cx| {
            servers_page::stop_on_quit();
            telemetry::send_before_exit();
            cx.quit()
        })
        .detach();
        cx.activate(true);
    });
}
