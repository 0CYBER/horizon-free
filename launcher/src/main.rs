//! Horizon launcher: installs Horizon builds, prepares them from the player's own Minecraft,
//! and starts the client or a dedicated server.

mod cli;
mod markdown;
mod mcp;
mod theme;

use gpui::{
    div, ease_out_quint, img, linear_color_stop, linear_gradient, prelude::*, px, relative, size,
    Animation, AnimationExt, AnyElement, App, Application, Bounds, ClickEvent, Context, Div,
    ElementId, FocusHandle, FontWeight, Hsla, KeyDownEvent, ObjectFit, PathPromptOptions,
    RenderImage, ScrollHandle, SharedString, Stateful, Subscription, TitlebarOptions, Window,
    WindowBounds, WindowOptions,
};
use horizon_launcher::backend::{self, Job, Msg, Release, Running, Settings, Updates};
use std::borrow::Cow;
use std::collections::VecDeque;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use theme::Theme;

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Play,
    Versions,
    Notes,
    Settings,
}

impl Page {
    const ALL: [(Page, &'static str); 4] = [
        (Page::Play, "Play"),
        (Page::Versions, "Versions"),
        (Page::Notes, "Patch notes"),
        (Page::Settings, "Settings"),
    ];
    fn index(self) -> usize {
        Self::ALL.iter().position(|(p, _)| *p == self).unwrap_or(0)
    }
}

/// The launcher's own update, as the sidebar and Settings show it.
#[derive(Clone, PartialEq)]
enum SelfUpdate {
    Idle,
    Downloading(String, f32),
    Ready(String),
    Failed(String),
}

/// The opening: the wordmark over the image holds at least `OPENING_HOLD` (until the release
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

/// The background, decoded before the first frame so the opening never shows without it.
fn background_image() -> Arc<RenderImage> {
    let mut rgba = image::load_from_memory_with_format(
        include_bytes!("../assets/background.jpg"),
        image::ImageFormat::Jpeg,
    )
    .expect("bundled background")
    .into_rgba8();
    // GPUI draws BGRA.
    for px in rgba.pixels_mut() {
        px.0.swap(0, 2);
    }
    Arc::new(RenderImage::new([image::Frame::new(rgba)]))
}

fn ease_out_cubic(x: f32) -> f32 {
    horizon_theme::ease_out_cubic(x)
}

/// How often the launcher looks for new releases while it is open.
const CHECK_EVERY: Duration = Duration::from_secs(4 * 3600);

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Client,
    Server,
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
    /// The page before the last switch and a counter, for the page and sidebar transitions.
    previous_page: Page,
    switches: usize,
    /// The dark appearance's atmosphere, drawn darkened behind everything (assets/background.jpg).
    background: Arc<RenderImage>,
    opening: Option<Opening>,
    updates: Updates,
    checked: Option<Instant>,
    update_error: Option<String>,
    self_update: SelfUpdate,
    /// The Horizon version being installed as an update, and whether it becomes the selected one.
    update_install: Option<(String, bool)>,
    /// The install bar's previous and current fraction, so it moves rather than jumps.
    bar: (f32, f32),
    mode: Mode,
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
    server: Option<Running>,
    server_log: VecDeque<String>,
    server_error: Option<String>,
    input: String,
    input_focused: bool,
    focus: FocusHandle,
    console_scroll: ScrollHandle,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    _appearance: Subscription,
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
        if settings.server_dir.is_none() {
            settings.server_dir = Some(backend::data_dir().join("server"));
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
            previous_page: Page::Play,
            switches: 0,
            opening: (!settings.skip_opening).then(|| {
                let focus = cx.focus_handle();
                window.focus(&focus);
                Opening {
                    started: None,
                    reveal: None,
                    focus,
                }
            }),
            background: background_image(),
            updates: Updates::default(),
            checked: None,
            update_error: None,
            self_update: SelfUpdate::Idle,
            update_install: None,
            bar: (0., 0.),
            mode: Mode::Client,
            releases: Load::Loading,
            installed,
            installing: None,
            install_error: None,
            confirm_remove: None,
            picker_open: false,
            settings,
            links: backend::Links::default(),
            menu_style: backend::menu_style(),
            preparing: false,
            prepare_log: VecDeque::new(),
            prepare_error: None,
            client: None,
            client_error: None,
            server: None,
            server_log: VecDeque::new(),
            server_error: None,
            input: String::new(),
            input_focused: false,
            focus: cx.focus_handle(),
            console_scroll: ScrollHandle::new(),
            tx,
            rx,
            _appearance: appearance,
        };
        let tx = this.tx.clone();
        std::thread::spawn(move || {
            let reused = backend::reuse_prepared_everywhere();
            if !reused.is_empty() {
                let _ = tx.send(Msg::Reused(reused));
            }
        });
        this.fetch_releases();
        this.fetch_links();
        this.check_updates();
        this
    }

    fn go(&mut self, page: Page) {
        if page != self.page {
            self.previous_page = self.page;
            self.page = page;
            self.switches += 1;
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

    fn fetch_links(&self) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Links(backend::links()));
        });
    }

    fn drain(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        while let Ok(msg) = self.rx.try_recv() {
            changed = true;
            match msg {
                Msg::Releases(r) => self.releases = r.map_or_else(Load::Failed, Load::Ready),
                Msg::Links(links) => self.links = links,
                // The page reads is_prepared when drawn; `changed` redraws it.
                Msg::Reused(_) => self.installed = backend::installed(),
                Msg::Progress(tag, p) => {
                    self.bar = (self.bar.1, p);
                    self.installing = Some((tag, p));
                }
                Msg::Installed(tag, result) => {
                    self.installing = None;
                    self.bar = (0., 0.);
                    self.installed = backend::installed();
                    let as_update = self.update_install.take().filter(|(t, _)| *t == tag);
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
                Msg::Line(job, line) => {
                    let log = if job == Job::Server {
                        &mut self.server_log
                    } else {
                        &mut self.prepare_log
                    };
                    log.push_back(line);
                    if log.len() > LOG_LINES {
                        log.pop_front();
                    }
                    if job == Job::Server {
                        self.console_scroll.scroll_to_bottom();
                    }
                }
                Msg::Exit(job, result) => {
                    let error = result.err();
                    match job {
                        Job::Prepare => {
                            self.preparing = false;
                            self.prepare_error = error.map(|e| {
                                let tail =
                                    self.prepare_log.iter().rev().find(|l| !l.trim().is_empty());
                                tail.map_or(e.clone(), |t| format!("{e}: {t}"))
                            });
                        }
                        Job::Client => {
                            self.client = None;
                            self.client_error = error.map(|e| {
                                let tail =
                                    self.prepare_log.iter().rev().find(|l| !l.trim().is_empty());
                                tail.map_or(e.clone(), |t| format!("{e}: {t}"))
                            });
                        }
                        Job::Server => {
                            self.server = None;
                            let old = self
                                .server_log
                                .iter()
                                .any(|l| l.contains("unknown option --dedicated"));
                            self.server_error = error.map(|e| {
                                if old {
                                    "This version has no dedicated server. Pick a newer one."
                                        .to_string()
                                } else {
                                    let tail =
                                        self.server_log.iter().rev().find(|l| !l.trim().is_empty());
                                    tail.map_or(e.clone(), |t| format!("{e}: {t}"))
                                }
                            });
                        }
                    }
                }
            }
        }
        if self.checked.is_some_and(|at| at.elapsed() > CHECK_EVERY) {
            self.check_updates();
        }
        if changed {
            cx.notify();
        }
    }

    fn selected(&self) -> Option<&String> {
        self.settings
            .selected
            .as_ref()
            .filter(|s| self.installed.contains(s))
    }

    fn install(&mut self, release: &Release) {
        let Some((url, size)) = release.asset.clone() else {
            return;
        };
        let (tag, tx) = (release.tag.clone(), self.tx.clone());
        self.installing = Some((tag.clone(), 0.0));
        self.bar = (0., 0.);
        self.install_error = None;
        std::thread::spawn(move || {
            let progress = |p| {
                let _ = tx.send(Msg::Progress(tag.clone(), p));
            };
            let result = backend::install(&tag, &url, size, progress);
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
        let Some(minecraft) = self.settings.minecraft_dir.clone() else {
            self.prepare_error = Some("Choose your Minecraft folder first.".into());
            return;
        };
        self.prepare_log.clear();
        self.prepare_error = None;
        self.prepare_log
            .push_back("Checking Java, Python and your Minecraft install".into());
        let tx = self.tx.clone();
        // The checks may download the server jar, so they run on the thread too.
        std::thread::spawn(move || {
            let log = |line| {
                let _ = tx.send(Msg::Line(Job::Prepare, line));
            };
            let result = backend::prepare_command(&tag, &minecraft, &log)
                .and_then(|cmd| backend::spawn(Job::Prepare, cmd, tx.clone()));
            match result {
                Ok(_running) => {}
                Err(e) => {
                    let _ = tx.send(Msg::Exit(Job::Prepare, Err(e)));
                }
            }
        });
        self.preparing = true;
    }

    fn play(&mut self) {
        let Some(tag) = self.selected().cloned() else {
            return;
        };
        match self.mode {
            Mode::Client => {
                if self.client.is_some() {
                    return;
                }
                self.client_error = None;
                self.prepare_log.clear();
                match backend::spawn(Job::Client, backend::client_command(&tag), self.tx.clone()) {
                    Ok(r) => self.client = Some(r),
                    Err(e) => self.client_error = Some(e),
                }
            }
            Mode::Server => {
                if let Some(server) = &mut self.server {
                    server.send("stop");
                    return;
                }
                let Some(folder) = self.settings.server_dir.clone() else {
                    return;
                };
                if let Err(e) = std::fs::create_dir_all(&folder) {
                    self.server_error = Some(e.to_string());
                    return;
                }
                self.server_error = None;
                self.server_log.clear();
                match backend::spawn(
                    Job::Server,
                    backend::server_command(&tag, &folder),
                    self.tx.clone(),
                ) {
                    Ok(r) => self.server = Some(r),
                    Err(e) => self.server_error = Some(e),
                }
            }
        }
    }

    fn choose_folder(&mut self, server: bool, cx: &mut Context<Self>) {
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
                        if server {
                            this.settings.server_dir = Some(path);
                        } else {
                            this.settings.minecraft_dir = Some(path);
                        }
                        this.settings.save();
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    fn submit_command(&mut self) {
        let line = std::mem::take(&mut self.input);
        if line.trim().is_empty() {
            return;
        }
        if let Some(server) = &mut self.server {
            server.send(line.trim_start_matches('/'));
            self.server_log.push_back(format!("> {line}"));
            self.console_scroll.scroll_to_bottom();
        }
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

/// A layer under an element's content that fades `color` in while it is hovered.
fn hover_layer(id: &SharedString, color: Hsla, radius: f32) -> AnyElement {
    let layer = div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
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

fn button(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    kind: Kind,
    t: &Theme,
) -> Stateful<Div> {
    let id: SharedString = id.into();
    let t = *t;
    let hover = match kind {
        Kind::Primary => gpui::white().opacity(theme::PRIMARY_HOVER),
        _ => t.hover,
    };
    let base = div()
        .id(id.clone())
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(px(theme::CONTROL))
        .px(px(12.))
        .rounded(px(theme::RADIUS))
        .text_size(px(theme::BODY))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .child(hover_layer(&id, hover, theme::RADIUS))
        .child(div().relative().child(label.into()));
    let base = track_hover(base, id);
    match kind {
        Kind::Primary => base
            .bg(t.accent)
            .text_color(t.on_accent)
            .active(|s| s.opacity(theme::PRESSED)),
        Kind::Secondary => base
            .border_1()
            .border_color(t.border)
            .bg(t.raised)
            .text_color(t.text)
            .active(|s| s.opacity(theme::PRESSED_SOFT)),
        Kind::Ghost => base
            .text_color(t.muted)
            .hover(move |s| s.text_color(t.text))
            .active(|s| s.opacity(theme::PRESSED_SOFT)),
    }
}

/// An on/off switch whose knob slides.
fn switch(id: &'static str, on: bool, t: &Theme) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .flex_none()
        .w(px(34.))
        .h(px(20.))
        .rounded_full()
        .cursor_pointer()
        .bg(if on { t.accent } else { t.border })
        .child(
            div()
                .absolute()
                .top(px(2.))
                .size(px(16.))
                .rounded_full()
                .bg(gpui::white())
                .with_animation(
                    ElementId::named_usize(id, on as usize),
                    Animation::new(theme::QUICK).with_easing(ease_out_quint()),
                    move |d, p| d.left(px(if on { 2. + 14. * p } else { 16. - 14. * p })),
                ),
        )
}

fn disabled(b: Stateful<Div>) -> Stateful<Div> {
    b.opacity(theme::DISABLED).cursor_default()
}

fn page_header(title: &str, description: &str, t: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            div()
                .font_family(theme::DISPLAY)
                .text_size(px(theme::TITLE))
                .line_height(px(32.))
                .text_color(t.text)
                .child(title.to_string()),
        )
        .child(
            div()
                .text_size(px(theme::BODY))
                .text_color(t.muted)
                .child(description.to_string()),
        )
}

fn divider(t: &Theme) -> Div {
    div().h(px(1.)).w_full().bg(t.border)
}

/// A settings-style row: label column, value, then an action.
fn row(label: &str, value: impl IntoElement, action: Option<Stateful<Div>>, t: &Theme) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(16.))
        .py(px(12.))
        .child(
            div()
                .w(px(theme::ROW_LABEL))
                .flex_none()
                .text_size(px(13.))
                .text_color(t.muted)
                .child(label.to_string()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(13.))
                .text_color(t.text)
                .child(value),
        )
        .children(action)
}

fn notice(text: String, t: &Theme) -> Div {
    div()
        .flex()
        .gap(px(10.))
        .p(px(12.))
        .rounded(px(6.))
        .border_1()
        .border_color(t.border)
        .text_size(px(13.))
        .line_height(px(19.))
        .text_color(t.text)
        .child(
            div()
                .flex_none()
                .mt(px(6.))
                .size(px(6.))
                .rounded_full()
                .bg(t.danger),
        )
        .child(div().flex_1().min_w_0().child(text))
}

fn centered(text: &str, t: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(12.))
        .py(px(64.))
        .text_size(px(13.))
        .text_color(t.faint)
        .child(text.to_string())
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
    fn sidebar(&self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        const STEP: f32 = theme::CONTROL + 2.;
        let mut nav = div().relative().flex().flex_col().gap(px(2.));
        // The selection is one pill that slides to the chosen item.
        let (from, to) = (
            self.previous_page.index() as f32 * STEP,
            self.page.index() as f32 * STEP,
        );
        nav = nav.child(
            div()
                .absolute()
                .left_0()
                .w_full()
                .h(px(theme::CONTROL))
                .rounded(px(theme::RADIUS))
                .bg(t.raised)
                .border_1()
                .border_color(t.border)
                .with_animation(
                    ElementId::named_usize("pill", self.switches),
                    Animation::new(theme::MOVE).with_easing(ease_out_quint()),
                    move |d, p| d.top(px(from + (to - from) * p)),
                ),
        );
        for (page, label) in Page::ALL {
            let active = self.page == page;
            let id = SharedString::from(format!("nav-{label}"));
            let item = div()
                .id(id.clone())
                .relative()
                .flex()
                .items_center()
                .h(px(theme::CONTROL))
                .px(px(10.))
                .rounded(px(theme::RADIUS))
                .text_size(px(theme::BODY))
                .cursor_pointer()
                .when(!active, |d| {
                    d.child(hover_layer(&id, t.hover.opacity(0.7), theme::RADIUS))
                })
                .child(
                    div()
                        .relative()
                        .when(active, |d| {
                            d.text_color(t.text).font_weight(FontWeight::MEDIUM)
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
        let footer = match &self.self_update {
            SelfUpdate::Ready(version) => div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().child(format!("Launcher {} → {version}", backend::VERSION)))
                .child(
                    div()
                        .id("restart")
                        .text_color(t.accent)
                        .font_weight(FontWeight::MEDIUM)
                        .cursor_pointer()
                        .hover(|s| s.opacity(0.8))
                        .child("Restart to update")
                        .on_click(|_, _, _| backend::restart()),
                ),
            _ => div().child(format!("Launcher {}", backend::VERSION)),
        };
        let discord = (!self.links.discord.is_empty()).then(|| {
            let url = self.links.discord.clone();
            div()
                .id("discord")
                .mb(px(4.))
                .text_color(t.muted)
                .cursor_pointer()
                .hover(move |s| s.text_color(t.text))
                .child("Join Discord")
                .on_click(move |_, _, cx| cx.open_url(&url))
        });
        let website = (!self.links.website.is_empty()).then(|| {
            let url = self.links.website.clone();
            div()
                .id("website")
                .mb(px(4.))
                .text_color(t.muted)
                .cursor_pointer()
                .hover(move |s| s.text_color(t.text))
                .child("Website")
                .on_click(move |_, _, cx| cx.open_url(&url))
        });
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(theme::SIDEBAR))
            .h_full()
            .bg(t.sidebar.opacity(0.8))
            .border_r_1()
            .border_color(t.border)
            .p(px(12.))
            .child(
                div()
                    .px(px(10.))
                    .pt(px(4.))
                    .pb(px(20.))
                    .font_family(theme::DISPLAY)
                    .text_size(px(22.))
                    .text_color(t.text)
                    .child("Horizon"),
            )
            .child(nav)
            .child(div().flex_1())
            .child(
                div()
                    .px(px(10.))
                    .text_size(px(theme::SMALL))
                    .text_color(t.faint)
                    .children(discord)
                    .children(website)
                    .child(footer),
            )
    }

    fn segmented(&self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let seg = |mode: Mode, label: &'static str, cx: &mut Context<Self>| {
            let active = self.mode == mode;
            div()
                .id(label)
                .flex()
                .items_center()
                .h(px(theme::SEGMENT))
                .px(px(14.))
                .rounded(px(theme::RADIUS_SEGMENT))
                .text_size(px(13.))
                .cursor_pointer()
                .when(active, |d| {
                    d.bg(t.bg)
                        .border_1()
                        .border_color(t.border)
                        .text_color(t.text)
                        .font_weight(FontWeight::MEDIUM)
                })
                .when(!active, |d| {
                    d.text_color(t.muted).hover(move |s| s.text_color(t.text))
                })
                .child(label)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.mode = mode;
                    cx.notify();
                }))
        };
        div()
            .flex()
            .p(px(2.))
            .gap(px(2.))
            .rounded(px(theme::RADIUS_GROUP))
            .bg(t.raised)
            .border_1()
            .border_color(t.border)
            .child(seg(Mode::Client, "Client", cx))
            .child(seg(Mode::Server, "Server", cx))
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
                .h(px(30.))
                .px(px(12.))
                .rounded(px(6.))
                .border_1()
                .border_color(t.border)
                .text_size(px(13.))
                .text_color(t.text)
                .cursor_pointer()
                .hover(move |s| s.bg(t.hover))
                .child(label)
                .child(div().text_color(t.faint).text_size(px(10.)).child("▾"))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.picker_open = !this.picker_open;
                    cx.notify();
                })),
        );
        if self.picker_open {
            let mut menu = div()
                .absolute()
                .top(px(36.))
                .right_0()
                .w(px(220.))
                .p(px(4.))
                .rounded(px(8.))
                .border_1()
                .border_color(t.border)
                .bg(t.raised)
                .shadow_md()
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
                        .h(px(28.))
                        .px(px(8.))
                        .rounded(px(5.))
                        .text_size(px(13.))
                        .text_color(t.text)
                        .cursor_pointer()
                        .hover(move |s| s.bg(t.hover))
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
            menu = menu.child(div().my(px(4.)).h(px(1.)).bg(t.border)).child(
                div()
                    .id("manage")
                    .flex()
                    .items_center()
                    .h(px(28.))
                    .px(px(8.))
                    .rounded(px(5.))
                    .text_size(px(13.))
                    .text_color(t.muted)
                    .cursor_pointer()
                    .hover(move |s| s.bg(t.hover).text_color(t.text))
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
        let header = page_header(
            "Play",
            "Start the game, or run a server for others to join.",
            &t,
        );
        let Some(tag) = self.selected().cloned() else {
            return div().flex().flex_col().gap(px(24.)).child(header).child(
                centered("No version installed yet.", &t).child(
                    button("go-versions", "Choose a version", Kind::Secondary, &t).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.go(Page::Versions);
                            cx.notify();
                        }),
                    ),
                ),
            );
        };
        let prepared = backend::is_prepared(&tag);
        let version = backend::game_version(&tag);
        let hero = self.hero(&tag, &version, prepared, cx);
        let running = match self.mode {
            Mode::Client => self.client.is_some(),
            Mode::Server => self.server.is_some(),
        };
        let play_label = match (self.mode, running) {
            (Mode::Client, false) => "Play",
            (Mode::Client, true) => "Running",
            (Mode::Server, false) => "Start server",
            (Mode::Server, true) => "Stop server",
        };
        let kind = if self.mode == Mode::Server && running {
            Kind::Secondary
        } else {
            Kind::Primary
        };
        let mut play = button("play", play_label, kind, &t)
            .min_w(px(120.))
            .h(px(40.))
            .px(px(24.))
            .text_size(px(theme::LARGE));
        let eula = self
            .settings
            .server_dir
            .as_ref()
            .is_some_and(|d| backend::eula_agreed(d));
        if !prepared
            || self.preparing
            || (self.mode == Mode::Client && running)
            || (self.mode == Mode::Server && !eula)
        {
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
            .child(self.segmented(cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(self.picker(cx))
                    .child(play),
            );

        let resources_value = if self.preparing {
            div().text_color(t.muted).child("Preparing…")
        } else if prepared {
            div().child(format!("Prepared for Minecraft {version}"))
        } else {
            div().text_color(t.muted).child(format!(
                "Not prepared. Horizon needs the files of your own Minecraft {version}."
            ))
        };
        let mut prepare = button(
            "prepare",
            if prepared { "Prepare again" } else { "Prepare" },
            if prepared {
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
                    .text_color(t.muted)
                    .child("Not found. Choose the official launcher's game folder.")
            },
            |p| div().child(short(p)),
        );
        let mut rows = div()
            .flex()
            .flex_col()
            .child(divider(&t))
            .child(row("Resources", resources_value, Some(prepare), &t))
            .child(divider(&t))
            .child(row(
                "Minecraft folder",
                minecraft,
                Some(button("choose-mc", "Choose…", Kind::Ghost, &t).on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| this.choose_folder(false, cx)),
                )),
                &t,
            ));
        if self.mode == Mode::Server {
            let folder = self
                .settings
                .server_dir
                .as_deref()
                .map(short)
                .unwrap_or_default();
            rows = rows.child(divider(&t)).child(row(
                "Server folder",
                div().child(folder),
                Some(
                    button("choose-server", "Choose…", Kind::Ghost, &t).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| this.choose_folder(true, cx)),
                    ),
                ),
                &t,
            ));
            let value = if eula {
                div().child("Agreed")
            } else {
                div()
                    .text_color(t.muted)
                    .child("Running a server needs your agreement to the Minecraft EULA.")
            };
            let mut actions = div().flex().gap(px(4.)).child(
                button("eula-read", "Read", Kind::Ghost, &t).on_click(cx.listener(
                    |_, _: &ClickEvent, _, cx| cx.open_url("https://aka.ms/MinecraftEULA"),
                )),
            );
            if !eula {
                actions =
                    actions.child(button("eula-agree", "Agree", Kind::Secondary, &t).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            if let Some(dir) = &this.settings.server_dir {
                                if let Err(e) = backend::agree_eula(dir) {
                                    this.server_error = Some(e);
                                }
                            }
                            cx.notify();
                        }),
                    ));
            }
            rows = rows.child(divider(&t)).child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .py(px(12.))
                    .child(
                        div()
                            .w(px(150.))
                            .flex_none()
                            .text_size(px(13.))
                            .text_color(t.muted)
                            .child("Minecraft EULA"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(13.))
                            .text_color(t.text)
                            .child(value),
                    )
                    .child(actions),
            );
        }
        rows = rows.child(divider(&t));

        let _ = header;
        let mut page = div()
            .flex()
            .flex_col()
            .gap(px(24.))
            .child(hero)
            .child(actions)
            .child(rows);
        if self.preparing {
            let last = self.prepare_log.back().cloned().unwrap_or_default();
            page = page.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(progress_bar(None, &t))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(t.faint)
                            .truncate()
                            .child(last),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(t.faint)
                            .child("The first preparation takes a few minutes."),
                    ),
            );
        }
        if let Some(e) = self.prepare_error.clone() {
            page = page.child(notice(e, &t));
        }
        if self.mode == Mode::Client {
            if let Some(e) = self.client_error.clone() {
                page = page.child(notice(e, &t));
            }
        } else {
            if let Some(e) = self.server_error.clone() {
                page = page.child(notice(e, &t));
            }
            page = page.child(self.console(cx));
        }
        page
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
        let status = format!(
            "Minecraft {version}  ·  {}",
            if prepared { "Ready" } else { "Needs preparing" }
        );
        let title = if self.mode == Mode::Server {
            "Horizon server"
        } else {
            "Horizon"
        };
        let mut hero = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .pt(px(28.))
            .pb(px(4.))
            .child(
                div()
                    .text_size(px(theme::SMALL))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(t.faint)
                    .child(status),
            )
            .child(
                div()
                    .font_family(theme::DISPLAY)
                    .text_size(px(theme::HERO))
                    .line_height(px(58.))
                    .text_color(t.text)
                    .child(title),
            )
            .child(
                div()
                    .max_w(px(600.))
                    .text_size(px(theme::LARGE))
                    .line_height(px(23.))
                    .text_color(t.muted)
                    .child(match summary {
                        Some(line) => format!("{tag}  ·  {line}"),
                        None => tag.to_string(),
                    }),
            );
        if let Some(release) = &self.updates.horizon {
            let installing = self.installing.as_ref().filter(|(t, _)| *t == release.tag);
            let line = div()
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(theme::SMALL))
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
        let opening = switch("opening", !self.settings.skip_opening, &t).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                this.settings.skip_opening = !this.settings.skip_opening;
                this.settings.save();
                cx.notify();
            },
        ));
        let menus = div()
            .id("menu-style")
            .flex()
            .flex_none()
            .p(px(2.))
            .gap(px(2.))
            .rounded(px(theme::RADIUS_GROUP))
            .bg(t.raised)
            .border_1()
            .border_color(t.border)
            .children(
                [
                    (backend::MenuStyle::Horizon, "Horizon"),
                    (backend::MenuStyle::Classic, "Classic"),
                ]
                .map(|(style, label)| {
                    let active = self.menu_style == style;
                    div()
                        .id(label)
                        .flex()
                        .items_center()
                        .h(px(theme::SEGMENT))
                        .px(px(14.))
                        .rounded(px(theme::RADIUS_SEGMENT))
                        .text_size(px(13.))
                        .cursor_pointer()
                        .when(active, |d| {
                            d.bg(t.bg)
                                .border_1()
                                .border_color(t.border)
                                .text_color(t.text)
                                .font_weight(FontWeight::MEDIUM)
                        })
                        .when(!active, |d| {
                            d.text_color(t.muted).hover(move |s| s.text_color(t.text))
                        })
                        .child(label)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            if backend::set_menu_style(style).is_ok() {
                                this.menu_style = style;
                            }
                            cx.notify();
                        }))
                }),
            );
        let menu_note = div().flex().flex_col().gap(px(2.)).child("Menu style").child(
            div().text_size(px(theme::SMALL)).text_color(t.muted).child(
                "Classic is vanilla's own screens, with no way back inside the game: switch back here. Applies the next time the game starts.",
            ),
        );
        let auto =
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child("Install new versions automatically")
                .child(div().text_size(px(theme::SMALL)).text_color(t.muted).child(
                    "Your selected version only changes if you were playing the newest one.",
                ));
        let horizon = match (&self.updates.horizon, &self.update_error) {
            (Some(r), _) => div().child(format!("{} available", r.tag)),
            (None, Some(e)) => div().text_color(t.muted).child(e.clone()),
            (None, None) if self.checked.is_some() => div().child("Up to date"),
            _ => div().text_color(t.muted).child("Checking…"),
        };
        let horizon_action = self
            .updates
            .horizon
            .as_ref()
            .filter(|_| self.installing.is_none())
            .map(|_| {
                button("settings-install", "Install", Kind::Secondary, &t).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.install_update();
                        cx.notify();
                    },
                ))
            });
        let launcher = match &self.self_update {
            SelfUpdate::Downloading(v, p) => {
                format!("{}  ·  downloading {v}, {:.0}%", backend::VERSION, p * 100.)
            }
            SelfUpdate::Ready(v) => {
                format!("{}  ·  {v} is ready, restart to use it", backend::VERSION)
            }
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
                    .on_click(|_, _, _| backend::restart()),
            ),
            SelfUpdate::Downloading(..) => None,
            _ => Some(
                button("check", "Check now", Kind::Ghost, &t).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.check_updates();
                        cx.notify();
                    },
                )),
            ),
        };
        let agents = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(16.))
            .rounded(px(8.))
            .border_1()
            .border_color(t.border)
            .bg(t.raised.opacity(0.6))
            .text_size(px(theme::BODY))
            .child(div().font_weight(FontWeight::MEDIUM).child("Use from the command line or AI agents"))
            .child(div().text_color(t.muted).child(
                "The launcher is also a command-line tool and an MCP server. Everything here works from a terminal, or from an agent such as Claude Code:",
            ))
            .child(
                div()
                    .font_family(theme::MONO)
                    .text_size(px(theme::SMALL))
                    .text_color(t.text)
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
        div()
            .flex()
            .flex_col()
            .gap(px(24.))
            .child(page_header(
                "Settings",
                "Updates, menus, and the launcher itself.",
                &t,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(divider(&t))
                    .child(row("Updates", auto, Some(toggle), &t))
                    .child(divider(&t))
                    .child(row(
                        "Opening",
                        div().child("Show opening animation"),
                        Some(opening),
                        &t,
                    ))
                    .child(divider(&t))
                    .child(row("Game menus", menu_note, Some(menus), &t))
                    .child(divider(&t))
                    .child(row("Horizon", horizon, horizon_action, &t))
                    .child(divider(&t))
                    .child(row("Launcher", div().child(launcher), launcher_action, &t))
                    .child(divider(&t))
                    .child(row(
                        "Data folder",
                        div().child(short(&backend::data_dir())),
                        None,
                        &t,
                    ))
                    .child(divider(&t)),
            )
            .child(agents)
    }

    fn console(&self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let running = self.server.is_some();
        let focused = self.input_focused;
        let lines = self.server_log.iter().map(|l| div().child(l.clone()));
        let log = div()
            .id("console")
            .h(px(260.))
            .p(px(12.))
            .overflow_y_scroll()
            .track_scroll(&self.console_scroll)
            .font_family(theme::MONO)
            .text_size(px(12.))
            .line_height(px(18.))
            .text_color(t.muted)
            .when(self.server_log.is_empty(), |d| {
                d.child(div().text_color(t.faint).child(if running {
                    "Starting…"
                } else {
                    "The server's output shows here."
                }))
            })
            .children(lines);
        let input = div()
            .id("command")
            .track_focus(&self.focus)
            .flex()
            .items_center()
            .h(px(36.))
            .px(px(12.))
            .border_t_1()
            .border_color(t.border)
            .font_family(theme::MONO)
            .text_size(px(12.))
            .cursor_text()
            .child(div().text_color(t.faint).mr(px(8.)).child(">"))
            .child(if self.input.is_empty() && !focused {
                div().text_color(t.faint).child(if running {
                    "Type a command, e.g. list"
                } else {
                    "Start the server to send commands"
                })
            } else {
                div().text_color(t.text).child(self.input.clone())
            })
            .when(focused, |d| d.child(div().w(px(1.)).h(px(14.)).bg(t.text)))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                window.focus(&this.focus);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                let k = &ev.keystroke;
                match k.key.as_str() {
                    "enter" => this.submit_command(),
                    "backspace" => {
                        this.input.pop();
                    }
                    _ if !k.modifiers.control && !k.modifiers.platform => {
                        if let Some(ch) = &k.key_char {
                            this.input.push_str(ch);
                        }
                    }
                    _ => return,
                }
                cx.stop_propagation();
                cx.notify();
            }));
        div()
            .flex()
            .flex_col()
            .rounded(px(8.))
            .border_1()
            .border_color(t.border)
            .bg(t.sidebar)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .h(px(36.))
                    .px(px(12.))
                    .border_b_1()
                    .border_color(t.border)
                    .text_size(px(12.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .text_color(t.muted)
                            .child(div().size(px(6.)).rounded_full().bg(if running {
                                t.accent
                            } else {
                                t.faint
                            }))
                            .child(if running { "Running" } else { "Stopped" }),
                    )
                    .when(running, |d| {
                        d.child(
                            button("kill", "Force stop", Kind::Ghost, &t)
                                .h(px(24.))
                                .px(px(8.))
                                .text_size(px(12.))
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    if let Some(s) = &this.server {
                                        s.kill();
                                    }
                                    cx.notify();
                                })),
                        )
                    }),
            )
            .child(log)
            .child(input)
    }

    fn versions_page(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let header = page_header(
            "Versions",
            "Builds from GitHub. Each one installs into its own folder.",
            &t,
        );
        let page = div().flex().flex_col().gap(px(24.)).child(header);
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
        let mut list = div().flex().flex_col().child(divider(&t));
        for release in releases {
            let tag = release.tag.clone();
            let installed = self.installed.contains(&tag);
            let selected = self.selected() == Some(&tag);
            let mut meta = vec![tag.clone(), release.date.clone()];
            if let Some((_, size)) = release.asset {
                meta.push(megabytes(size));
            }
            let title = if release.name.is_empty() {
                tag.clone()
            } else {
                release.name.clone()
            };
            let right: gpui::AnyElement =
                if let Some((_, p)) = self.installing.as_ref().filter(|(t, _)| *t == tag) {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .w(px(180.))
                        .child(progress_bar(Some(self.bar), &t))
                        .child(
                            div()
                                .w(px(36.))
                                .text_size(px(12.))
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
                        .when(selected && !confirming, |d| {
                            d.child(
                                div()
                                    .px(px(12.))
                                    .text_size(px(13.))
                                    .text_color(t.faint)
                                    .child("In use"),
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
                    div()
                        .text_size(px(13.))
                        .text_color(t.faint)
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
            let error = self
                .install_error
                .as_ref()
                .filter(|(t, _)| *t == tag)
                .map(|(_, e)| e.clone());
            list = list
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .py(px(14.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(16.))
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .flex_1()
                                        .min_w_0()
                                        .gap(px(3.))
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap(px(8.))
                                                .child(
                                                    div()
                                                        .text_size(px(13.5))
                                                        .text_color(t.text)
                                                        .truncate()
                                                        .child(title),
                                                )
                                                .when(installed, |d| {
                                                    d.child(
                                                        div()
                                                            .px(px(6.))
                                                            .rounded(px(4.))
                                                            .border_1()
                                                            .border_color(t.border)
                                                            .text_size(px(11.))
                                                            .text_color(t.muted)
                                                            .child("Installed"),
                                                    )
                                                }),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(12.))
                                                .text_color(t.faint)
                                                .child(meta.join("  ·  ")),
                                        ),
                                )
                                .child(right),
                        )
                        .children(error.map(|e| notice(e, &t))),
                )
                .child(divider(&t));
        }
        page.child(list)
    }

    fn notes_page(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let sponsor = (!self.links.sponsor.is_empty()).then(|| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(16.))
                .p(px(16.))
                .rounded(px(8.))
                .border_1()
                .border_color(t.border)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_size(px(13.))
                                .text_color(t.text)
                                .child("Support the work below"),
                        )
                        .child(div().text_size(px(12.5)).text_color(t.muted).child(
                            "Horizon is built in the open. Sponsors fund the next release.",
                        )),
                )
                .child(
                    button("sponsor", "Sponsor on GitHub", Kind::Secondary, &t).on_click({
                        let url = self.links.sponsor.clone();
                        move |_: &ClickEvent, _, cx| cx.open_url(&url)
                    }),
                )
        });
        let page = div()
            .flex()
            .flex_col()
            .gap(px(24.))
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
        let mut list = div().flex().flex_col();
        for (i, release) in releases.iter().enumerate() {
            if i > 0 {
                list = list.child(divider(&t));
            }
            list = list.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .py(px(24.))
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .justify_between()
                            .gap(px(16.))
                            .child(
                                div()
                                    .text_size(px(15.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(t.text)
                                    .child(if release.name.is_empty() {
                                        release.tag.clone()
                                    } else {
                                        release.name.clone()
                                    }),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(px(12.))
                                    .text_color(t.faint)
                                    .child(release.date.clone()),
                            ),
                    )
                    .child(if release.body.trim().is_empty() {
                        div()
                            .text_size(px(13.))
                            .text_color(t.faint)
                            .child("No notes for this release.")
                            .into_any_element()
                    } else {
                        markdown::render(&release.body, &t)
                    }),
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
        .h(px(4.))
        .rounded_full()
        .bg(t.accent);
    let track = div()
        .relative()
        .flex_1()
        .h(px(4.))
        .rounded_full()
        .bg(t.border)
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
        self.input_focused = self.focus.is_focused(window);
        let content = match self.page {
            Page::Play => self.play_page(cx),
            Page::Versions => self.versions_page(cx),
            Page::Notes => self.notes_page(cx),
            Page::Settings => self.settings_page(cx),
        };
        // The new page fades in and rises a few pixels.
        let content = div()
            .relative()
            .w_full()
            .max_w(px(760.))
            .child(content)
            .with_animation(
                ElementId::named_usize("page", self.switches),
                Animation::new(theme::MOVE).with_easing(ease_out_quint()),
                |d, p| d.opacity(p).top(px(8. * (1. - p))),
            );
        // The opening: the image at full strength under the wordmark, then the launcher.
        let (reveal, hold) = self.opening_phase(window);
        let side = ease_out_cubic((reveal - 0.2) / 0.7);
        let main = ease_out_cubic((reveal - 0.35) / 0.65);
        // The image shows through most at the top right, behind Play's title, and fades toward
        // the sidebar and the text below. Other pages keep it quieter.
        let veil = if self.page == Page::Play { 0.80 } else { 0.88 } * ease_out_cubic(reveal);
        let shade = ease_out_cubic(reveal);
        // A slow drift while the opening holds, settling back as the launcher appears.
        let scale = 1. + 0.04 * hold * (1. - shade);
        let atmosphere = div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .overflow_hidden()
            .when(t.dark || reveal < 1., |d| {
                d.child(
                    img(self.background.clone())
                        .absolute()
                        .left(relative((1. - scale) / 2.))
                        .top(relative((1. - scale) / 2.))
                        .w(relative(scale))
                        .h(relative(scale))
                        .object_fit(ObjectFit::Cover)
                        .when(!t.dark, |i| i.opacity(1. - shade)),
                )
                .when(t.dark, |d| {
                    d.child(div().absolute().size_full().bg(t.bg.opacity(veil)))
                        .child(
                            div()
                                .absolute()
                                .size_full()
                                .opacity(shade)
                                .bg(linear_gradient(
                                    90.,
                                    linear_color_stop(t.bg.opacity(0.9), 0.),
                                    linear_color_stop(t.bg.opacity(0.), 0.7),
                                )),
                        )
                        .child(
                            div()
                                .absolute()
                                .size_full()
                                .opacity(shade)
                                .bg(linear_gradient(
                                    180.,
                                    linear_color_stop(t.bg.opacity(0.), 0.25),
                                    linear_color_stop(t.bg.opacity(0.75), 1.),
                                )),
                        )
                })
            });
        let mut root = div()
            .relative()
            .flex()
            .size_full()
            .bg(t.bg)
            .font_family(theme::FONT)
            .text_color(t.text)
            .child(atmosphere)
            .child(
                div()
                    .relative()
                    .flex()
                    .h_full()
                    .opacity(side)
                    .left(px(-16. * (1. - side)))
                    .child(self.sidebar(cx)),
            )
            .child(
                div()
                    .id("content")
                    .relative()
                    .flex_1()
                    .h_full()
                    .opacity(main)
                    .top(px(16. * (1. - main)))
                    .overflow_y_scroll()
                    .child(div().flex().px(px(48.)).py(px(36.)).child(content)),
            );
        if let Some(opening) = &self.opening {
            // The wordmark rises and fades as the launcher comes in; a click or a key skips ahead.
            let word = 1. - ease_out_cubic(reveal / 0.6);
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
                    .when(reveal == 0., |d| d.cursor_pointer())
                    .child(
                        div()
                            .relative()
                            .top(px(-28. * (1. - word)))
                            .opacity(word)
                            .font_family(theme::DISPLAY)
                            .text_size(px(96.))
                            .text_color(gpui::white())
                            .child("Horizon"),
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
    /// Where the opening is: the reveal's progress (1 when there is no opening) and the hold's.
    fn opening_phase(&mut self, window: &mut Window) -> (f32, f32) {
        let loaded = !matches!(self.releases, Load::Loading);
        let Some(opening) = &mut self.opening else {
            return (1., 0.);
        };
        let held = opening.started.get_or_insert_with(Instant::now).elapsed();
        if opening.reveal.is_none() && ((held >= OPENING_HOLD && loaded) || held >= OPENING_MAX) {
            opening.reveal = Some(Instant::now());
        }
        let hold = held.as_secs_f32() / OPENING_MAX.as_secs_f32();
        let reveal = opening.reveal.map_or(0., |at| {
            at.elapsed().as_secs_f32() / OPENING_REVEAL.as_secs_f32()
        });
        if reveal >= 1. {
            self.opening = None;
            return (1., 0.);
        }
        window.request_animation_frame();
        (reveal, ease_out_cubic(hold))
    }

    fn skip_opening(&mut self) {
        if let Some(opening) = &mut self.opening {
            opening.reveal.get_or_insert_with(Instant::now);
        }
    }
}

fn main() {
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
                Cow::Borrowed(horizon_theme::fonts::INSTRUMENT_SERIF),
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
        cx.on_window_closed(|cx| cx.quit()).detach();
        cx.activate(true);
    });
}
