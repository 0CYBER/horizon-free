//! The Mods page: search Modrinth (CurseForge: coming soon) for Java mods, modpacks and resource
//! packs, see where each comes from and how well it runs (a Java mod's jar is
//! checked first), install it with its checksum checked, and update or remove what was installed
//! (`horizon_launcher::browser`). Imported modpacks are listed on its Modpacks tab.
use super::servers_page::{type_key, Key};
use super::*;
use horizon_launcher::browser::{
    self, installed, java, Kind as PackKind, Project, Query, SourceId, Support,
};
use horizon_launcher::modpacks;
use horizon_port_pack::check::Check;
use std::collections::HashMap;

/// The kinds, as the Mods page's tabs: name and icon.
const KINDS: [(PackKind, &str, &str); 3] = [
    (PackKind::Mod, "Mods", "box"),
    (PackKind::Modpack, "Modpacks", "archive"),
    (PackKind::ResourcePack, "Resource packs", "image"),
];
/// Java mods checked at once.
const CHECKING: usize = 2;
/// The sources to pick from; CurseForge is shown beside them as coming soon.
const SOURCES: [(SourceId, &str); 1] = [(SourceId::Modrinth, "Modrinth")];
const PAGE: usize = 20;

pub(super) enum Event {
    /// A search's answer, for the search it was (stale answers are dropped) at that offset.
    Found(u64, usize, Result<Vec<Project>, String>),
    Progress(String, f32),
    Done(Result<String, String>, bool),
    Updates(HashMap<(SourceId, String), String>),
    /// A Java mod's check: its jar read, or why it couldn't be.
    Checked((SourceId, String), Result<Box<Check>, String>),
    /// A project page's part, an image or a filter's values (`mods_project`).
    Loaded(super::mods_project::Loaded),
}

/// Where a Java mod's check is.
pub(super) enum Checking {
    Running,
    Done(Box<Check>),
    Failed(String),
}

pub(super) struct Mods {
    pub(super) kind: PackKind,
    pub(super) source: SourceId,
    text: String,
    focus: FocusHandle,
    pub(super) focused: bool,
    everything: bool,
    /// Which search is the current one; answers for older ones are dropped.
    search: u64,
    searching: bool,
    searched: bool,
    results: Vec<Project>,
    results_list: cull::Cull,
    installed_list: cull::Cull,
    pub(super) report_list: cull::Cull,
    more: bool,
    error: Option<String>,
    installed: Vec<installed::Entry>,
    loaded: bool,
    /// The project being installed, and the line and fraction of its download.
    pub(super) busy: Option<(String, String, f32)>,
    message: Option<Result<String, String>>,
    updates: HashMap<(SourceId, String), String>,
    confirm_remove: Option<String>,
    /// Java mods' checks, by source and project.
    pub(super) checks: HashMap<(SourceId, String), Checking>,
    /// The project page, filters and images.
    pub(super) browse: super::mods_project::Browse,
    pub(super) tx: Sender<Event>,
    rx: Receiver<Event>,
}

impl Mods {
    pub(super) fn reset_lists(&self) {
        self.results_list.reset();
        self.installed_list.reset();
        self.report_list.reset();
    }

    /// Whether the Modpacks tab is open (the sidebar marks the pack shown there).
    pub(super) fn on_modpacks(&self) -> bool {
        self.kind == PackKind::Modpack
    }

    pub(super) fn new(cx: &mut App) -> Self {
        let (tx, rx) = channel();
        Self {
            kind: if std::env::var("HORIZON_LAUNCHER_PAGE").as_deref() == Ok("modpacks") {
                PackKind::Modpack
            } else {
                PackKind::Mod
            },
            source: SourceId::Modrinth,
            text: String::new(),
            focus: cx.focus_handle().tab_index(0).tab_stop(true),
            focused: false,
            everything: false,
            search: 0,
            searching: false,
            searched: false,
            results: Vec::new(),
            results_list: cull::Cull::default(),
            installed_list: cull::Cull::default(),
            report_list: cull::Cull::default(),
            more: false,
            error: None,
            installed: Vec::new(),
            loaded: false,
            busy: None,
            message: None,
            updates: HashMap::new(),
            confirm_remove: None,
            checks: HashMap::new(),
            browse: Default::default(),
            tx,
            rx,
        }
    }

    pub(super) fn note_focus(&mut self, window: &Window) {
        self.focused = self.focus.is_focused(window);
    }

    pub(super) fn entry(&self, p: &Project) -> Option<&installed::Entry> {
        self.installed
            .iter()
            .find(|e| e.source == p.source && e.project == p.id)
    }

    /// How well a result runs: a Java mod's from its check once it has one.
    pub(super) fn support(&self, p: &Project) -> Support {
        let metadata = p.support();
        if p.kind != PackKind::Mod || !metadata.installable() {
            return metadata;
        }
        match self.checks.get(&(p.source, p.id.clone())) {
            Some(Checking::Done(c)) => java::support_of(c),
            Some(Checking::Failed(e)) => Support::Partial(format!("Not checked: {e}")),
            _ => Support::Partial("Checking its jar…".into()),
        }
    }
}

/// `51404735` → `51.4M`.
fn count(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.1}k", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

/// A checksum as the page shows it: the algorithm and the first and last hex digits.
fn short_sum(checksum: &str) -> String {
    let (algo, hex) = checksum.split_once(':').unwrap_or(("", checksum));
    if hex.len() <= 20 {
        return checksum.to_string();
    }
    format!("{algo} {}…{}", &hex[..12], &hex[hex.len() - 8..])
}

impl Launcher {
    /// Opens the Mods page on its Modpacks tab, with the pack `slug` open when given (the
    /// sidebar's packs and its + button).
    pub(super) fn show_modpack(&mut self, slug: Option<String>) {
        self.go(Page::Mods);
        if slug.is_some() {
            self.pack_open = slug;
            self.mods.report_list.reset();
        }
        if self.mods.kind != PackKind::Modpack {
            self.mods.kind = PackKind::Modpack;
            self.mods.browse.filters = Default::default();
            self.mods.browse.open = None;
            if self.mods.loaded {
                self.mods_search(0);
            }
        }
    }

    /// Called with the launcher's other work: what the background sent.
    pub(super) fn mods_tick(&mut self) -> bool {
        let mut changed = false;
        let mut to_check = Vec::new();
        let mut loaded = Vec::new();
        while let Ok(event) = self.mods.rx.try_recv() {
            changed = true;
            let m = &mut self.mods;
            match event {
                Event::Found(search, offset, result) if search == m.search => {
                    m.searching = false;
                    m.searched = true;
                    match result {
                        Ok(found) => {
                            m.more = found.len() == PAGE;
                            m.results.truncate(offset);
                            m.results.extend(found.iter().cloned());
                            m.error = None;
                            to_check.extend(found);
                        }
                        Err(e) => m.error = Some(e),
                    }
                }
                Event::Found(..) => {}
                Event::Progress(line, f) => {
                    if let Some(busy) = &mut m.busy {
                        busy.1 = line;
                        busy.2 = f;
                    }
                }
                Event::Done(result, modpack) => {
                    m.busy = None;
                    m.message = Some(result);
                    m.installed = installed::Installed::load(&backend::game_home()).entries;
                    m.installed_list.reset();
                    if modpack {
                        // The Modpacks page reads its list again.
                        self.packs_loaded = None;
                    }
                }
                Event::Updates(found) => m.updates = found,
                Event::Checked(key, result) => {
                    let state = match result {
                        Ok(c) => Checking::Done(c),
                        Err(e) => Checking::Failed(e),
                    };
                    m.checks.insert(key, state);
                }
                Event::Loaded(l) => loaded.push(l),
            }
        }
        // `HORIZON_LAUNCHER_PROJECT=<slug>` (with `HORIZON_LAUNCHER_PAGE=mods`): opens that
        // project's page once a search finds it, for screenshots.
        if self.mods.browse.open.is_none() {
            if let Ok(want) = std::env::var("HORIZON_LAUNCHER_PROJECT") {
                if let Some(p) = self
                    .mods
                    .results
                    .iter()
                    .find(|p| p.id == want || p.slug == want)
                    .cloned()
                {
                    std::env::remove_var("HORIZON_LAUNCHER_PROJECT");
                    self.mods_open(p);
                }
            }
        }
        self.mods_check(to_check);
        for l in loaded {
            self.mods_loaded(l);
        }
        changed
    }

    pub(super) fn mods_search(&mut self, offset: usize) {
        let m = &mut self.mods;
        m.search += 1;
        m.searching = true;
        if offset == 0 {
            m.results.clear();
            m.more = false;
            m.results_list.reset();
            m.installed_list.reset();
        }
        let query = Query {
            everything: m.everything,
            offset,
            limit: PAGE,
            filters: m.browse.filters.clone(),
            ..Query::new(&m.text, m.kind)
        };
        let (tx, search, source) = (m.tx.clone(), m.search, m.source);
        std::thread::spawn(move || {
            let result = browser::source(source)
                .ok_or_else(|| format!("{} isn't set up.", source.label()))
                .and_then(|s| s.search(&query));
            let _ = tx.send(Event::Found(search, offset, result));
        });
    }

    /// Checks the Java mods among `found` that aren't checked yet, a few at a time, on threads:
    /// a cached check answers at once, else the jar is downloaded and read.
    fn mods_check(&mut self, found: Vec<Project>) {
        let m = &mut self.mods;
        let todo: Vec<Project> = found
            .into_iter()
            .filter(|p| p.kind == PackKind::Mod && p.support().installable())
            .filter(|p| !m.checks.contains_key(&(p.source, p.id.clone())))
            .collect();
        if todo.is_empty() {
            return;
        }
        for p in &todo {
            m.checks.insert((p.source, p.id.clone()), Checking::Running);
        }
        // A version other than 26.3 asked for: check the newest file for 1.21 and later.
        let everything = m.everything || m.browse.filters.other_version();
        let tx = m.tx.clone();
        let queue = std::sync::Mutex::new(todo.into_iter());
        std::thread::spawn(move || {
            let home = backend::game_home();
            std::thread::scope(|s| {
                for _ in 0..CHECKING {
                    s.spawn(|| {
                        while let Some(p) = queue.lock().ok().and_then(|mut q| q.next()) {
                            let result = browser::source(p.source)
                                .ok_or_else(|| format!("{} isn't set up.", p.source.label()))
                                .and_then(|source| {
                                    java::check_project(&home, &*source, &p, everything, &|_| {})
                                });
                            // The project's page, if open, shows the verdict beside its version.
                            if let Ok((version, check)) = &result {
                                let key = (p.source, p.id.clone());
                                let _ =
                                    tx.send(Event::Loaded(super::mods_project::Loaded::Checked(
                                        key,
                                        version.id.clone(),
                                        check.verdict,
                                    )));
                            }
                            let result = result.map(|(_, check)| Box::new(check));
                            let _ = tx.send(Event::Checked((p.source, p.id), result));
                        }
                    });
                }
            });
        });
    }

    /// Looks for newer versions of what the browser installed.
    fn mods_check_updates(&mut self) {
        let entries = self.mods.installed.clone();
        if entries.is_empty() {
            return;
        }
        let tx = self.mods.tx.clone();
        std::thread::spawn(move || {
            let mut found = HashMap::new();
            for e in entries {
                let Some(source) = browser::source(e.source) else {
                    continue;
                };
                if let Ok(Some(v)) = installed::update_for(&*source, &e) {
                    found.insert((e.source, e.project.clone()), v.number);
                }
            }
            let _ = tx.send(Event::Updates(found));
        });
    }

    /// Installs (or updates) a project on a thread: a pack into its folder, a modpack into
    /// Modpacks. `known` is the project when the page has it; else it is asked for first.
    pub(super) fn mods_install(&mut self, source: SourceId, id: String, known: Option<Project>) {
        if self.mods.busy.is_some() {
            return;
        }
        self.mods.message = None;
        self.mods.busy = Some((id.clone(), format!("Asking {}…", source.label()), 0.));
        self.mods.updates.remove(&(source, id.clone()));
        let everything = self.mods.everything || self.mods.browse.filters.other_version();
        // A modpack is ported for the selected version as soon as it is imported.
        let resources = self
            .selected()
            .filter(|tag| backend::is_prepared(tag))
            .map(|tag| backend::resources(tag));
        let tx = self.mods.tx.clone();
        std::thread::spawn(move || {
            let last = std::cell::Cell::new(-1);
            let progress = |line: &str, f: f32| {
                let pct = (f * 100.) as i32;
                if last.replace(pct) != pct {
                    let _ = tx.send(Event::Progress(line.to_string(), f));
                }
            };
            let mut modpack = false;
            let result = browser::source(source)
                .ok_or_else(|| format!("{} isn't set up.", source.label()))
                .and_then(|source| {
                    let p = match known {
                        Some(p) => p,
                        None => source.project(&id)?,
                    };
                    modpack = p.kind == PackKind::Modpack;
                    if modpack {
                        let folder = modpacks::ports_dir().join("downloads");
                        let file = installed::fetch_modpack(&*source, &p, &folder, &|f| {
                            progress(&p.title, f)
                        })?;
                        let log = |line: String| progress(&line, 1.);
                        let pack = modpacks::import_and_port(&file, resources.as_deref(), &log)?;
                        return Ok(format!(
                            "{} Play it from the list above.",
                            modpacks::outcome(&pack)
                        ));
                    }
                    let home = backend::game_home();
                    let done = installed::install(&*source, &p, everything, &home, &progress)?;
                    let names: Vec<String> = done
                        .iter()
                        .map(|e| format!("{} {}", e.title, e.number))
                        .collect();
                    if p.kind == PackKind::Mod {
                        let skipped = done.last().map_or(0, |e| e.skipped.len());
                        return Ok(format!(
                            "Ported {} into horizon-mods and turned on (Options > Mods in the \
                             game).{}",
                            names.join(", "),
                            if skipped > 0 {
                                " What didn't port is listed under Installed."
                            } else {
                                ""
                            }
                        ));
                    }
                    Ok(format!(
                        "Installed {}, checksums checked. Turn it on in the game: Options > \
                         Resource Packs.",
                        names.join(", ")
                    ))
                });
            let _ = tx.send(Event::Done(result, modpack));
        });
    }

    fn mods_field(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = self.theme;
        let m = &self.mods;
        let focused = m.focused;
        let shown: String = {
            let chars: Vec<char> = m.text.chars().collect();
            chars[chars.len().saturating_sub(40)..].iter().collect()
        };
        div()
            .id("mods-search")
            .track_focus(&m.focus)
            .flex()
            .items_center()
            .gap(px(10.))
            .w(px(320.))
            .h(px(theme::FIELD + 4.))
            .px(px(12.))
            .overflow_hidden()
            .border_2()
            .border_color(if focused { t.accent } else { t.outline })
            .bg(t.well)
            .text_size(px(theme::UI.px))
            .cursor_text()
            .child(blocks::icon("search", 18., t.muted))
            .child(if m.text.is_empty() && !focused {
                div().text_color(t.muted).child("Search, then Enter")
            } else {
                div().text_color(t.ink).child(shown)
            })
            .when(focused, |d| {
                d.child(div().flex_none().w(px(1.)).h(px(16.)).bg(t.ink))
            })
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                window.focus(&this.mods.focus);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                let k = &ev.keystroke;
                let command = k.modifiers.control || k.modifiers.platform;
                match type_key(
                    &mut this.mods.text,
                    false,
                    &k.key,
                    k.key_char.as_deref(),
                    command,
                ) {
                    Key::Enter => this.mods_search(0),
                    Key::Escape => window.blur(),
                    Key::Paste => {
                        if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                            let line = text.lines().next().unwrap_or_default().trim().to_string();
                            this.mods.text.push_str(&line);
                        }
                    }
                    Key::Edited => {}
                    Key::Pass => return,
                }
                if let Some((at, _)) = this.mods.text.char_indices().nth(100) {
                    this.mods.text.truncate(at);
                }
                cx.stop_propagation();
                cx.notify();
            }))
    }

    pub(super) fn mods_page(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        if !self.mods.loaded {
            self.mods.loaded = true;
            self.mods.installed = installed::Installed::load(&backend::game_home()).entries;
            self.mods_check_updates();
            self.mods_search(0);
        }
        let header = page_header(
            "Mods",
            "Java mods, modpacks and resource packs. Horizon ports what a Java mod \
             carries (blocks, items, recipes, worldgen) and says before you install what runs. \
             Each file is checked against its source's checksum before it reaches your game \
             folder.",
            &t,
        );
        let this = cx.entity().downgrade();
        let kinds = blocks::tabs("mods-kind", &KINDS, self.mods.kind, &t, {
            let this = this.clone();
            move |kind, _, cx| {
                let _ = this.update(cx, |this, cx| {
                    if this.mods.kind != kind {
                        this.mods.kind = kind;
                        // Categories and loaders differ by kind.
                        this.mods.browse.filters = Default::default();
                        this.mods.browse.open = None;
                        this.mods_search(0);
                    }
                    cx.notify();
                });
            }
        });
        let everything = self.mods.everything;
        let toggle = div()
            .id("mods-everything")
            .flex()
            .items_center()
            .gap(px(8.))
            .cursor_pointer()
            .child(switch("mods-everything-switch", everything, &t))
            .child(
                text(theme::LABEL)
                    .text_color(t.ink_soft)
                    .child("Show everything"),
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.mods.everything = !this.mods.everything;
                this.mods_search(0);
                cx.notify();
            }));
        // CurseForge support is coming soon: shown, inert, never searched (like the Microsoft
        // account's "Coming soon").
        let source_note = div().flex().child(source_switch(&t));
        let controls = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(12.))
                    .child(self.mods_field(cx))
                    .child(toggle),
            )
            .child(source_note)
            .child(self.mods_filters(cx));
        // The kinds are inventory tabs on the panel that holds the open one.
        let body = blocks::panel(&t).flex().flex_col().gap(px(24.)).p(px(20.));
        let page = |body: Div| {
            div()
                .flex()
                .flex_col()
                .gap(px(32.))
                .child(header)
                .child(div().flex().flex_col().child(kinds).child(body))
        };
        if let Some(project) = self.mods_project_page(cx) {
            return page(body.children(self.mods_busy_plate()).child(project));
        }
        let mut body = body.children(self.mods_busy_plate());
        if self.mods.kind == PackKind::Modpack {
            body = body
                .child(self.imported_packs(cx))
                .child(div().h(px(2.)).bg(t.rule))
                .child(section_title("Browse modpacks", "search", &t));
        }
        body = body.child(controls);
        if let Some(list) = self.mods_installed_list(cx) {
            body = body.child(list);
        }
        page(body.child(self.mods_results(cx)))
    }

    /// The install under way and the last install's answer.
    pub(super) fn mods_busy_plate(&self) -> Vec<Div> {
        let t = self.theme;
        let mut out = Vec::new();
        if let Some((_, line, f)) = &self.mods.busy {
            out.push(
                plate(&t)
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .p(px(16.))
                    .child(caps("Installing").text_color(t.accent))
                    .child(
                        text(theme::CAPTION)
                            .font_family(theme::MONO)
                            .text_color(t.ink_soft)
                            .truncate()
                            .child(format!("{line}  {:.0}%", f * 100.)),
                    ),
            );
        }
        match &self.mods.message {
            Some(Ok(done)) => out.push(
                plate(&t)
                    .p(px(16.))
                    .text_size(px(theme::UI.px))
                    .text_color(t.ink)
                    .child(done.clone()),
            ),
            Some(Err(e)) => out.push(notice(e.clone(), &t)),
            None => {}
        }
        out
    }

    fn mods_installed_list(&self, cx: &mut Context<Self>) -> Option<Div> {
        let t = self.theme;
        let kind = self.mods.kind;
        let entries: Vec<&installed::Entry> = self
            .mods
            .installed
            .iter()
            .filter(|e| e.kind == kind)
            .collect();
        if entries.is_empty() {
            return None;
        }
        let busy = self.mods.busy.is_some();
        let row = |i: usize| {
            let e = entries[i];
            let key = (e.source, e.project.clone());
            let mut right = div().flex().items_center().gap(px(8.));
            if let Some(number) = self.mods.updates.get(&key) {
                let mut update = button(
                    SharedString::from(format!("mods-update-{}", e.project)),
                    format!("Update to {number}"),
                    Kind::Secondary,
                    &t,
                );
                if busy {
                    update = disabled(update);
                } else {
                    let (source, id) = key.clone();
                    update = update.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.mods_install(source, id.clone(), None);
                        cx.notify();
                    }));
                }
                right = right.child(update);
            }
            let confirming = self.mods.confirm_remove.as_deref() == Some(e.project.as_str());
            let id = e.project.clone();
            let remove = button(
                SharedString::from(format!("mods-remove-{}", e.project)),
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
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                if this.mods.confirm_remove.as_ref() == Some(&id) {
                    let home = backend::game_home();
                    this.mods.message = Some(
                        installed::remove(&home, &id).map(|e| format!("Removed {}.", e.title)),
                    );
                    this.mods.installed = installed::Installed::load(&home).entries;
                    this.mods.installed_list.reset();
                    this.mods.confirm_remove = None;
                } else {
                    this.mods.confirm_remove = Some(id.clone());
                }
                cx.notify();
            }));
            right = right.child(remove);
            let detail = div()
                .flex()
                .flex_col()
                .child(format!(
                    "{}{} · from {}{} · {}",
                    e.verdict
                        .as_ref()
                        .map(|v| format!("{v} · "))
                        .unwrap_or_default(),
                    e.number,
                    e.source.label(),
                    if e.dependency {
                        ", needed by another pack"
                    } else {
                        ""
                    },
                    e.file
                ))
                .children(e.skipped.iter().map(|s| {
                    div()
                        .text_color(t.ink_soft)
                        .child(format!("Not ported: {s}"))
                }))
                .child(
                    div()
                        .font_family(theme::MONO)
                        .child(format!("checked: {}", short_sum(&e.checksum))),
                );
            index_row(
                i + 1,
                e.title.clone(),
                Some(detail.into_any_element()),
                Some(right.into_any_element()),
                &t,
            )
        };
        let list = self.mods.installed_list.column(
            &self.content_scroll.handle,
            entries.len(),
            8.,
            |index| self.mods.confirm_remove.as_ref() == Some(&entries[index].project),
            row,
        );
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(caps("Installed").text_color(t.muted))
                .child(list),
        )
    }

    fn mods_results(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let m = &self.mods;
        if let Some(e) = &m.error {
            return div().child(notice(e.clone(), &t));
        }
        if m.results.is_empty() {
            let line = if m.searching || !m.searched {
                "Searching…"
            } else if m.everything {
                "Nothing found."
            } else {
                "Nothing found that runs in Horizon. Show everything lists the rest."
            };
            return centered(line, &t);
        }
        let result_count = m.results.len();
        let rows = m.results_list.clone();
        let scroll = self.content_scroll.handle.clone();
        let row = |i: usize| {
            let project = self.mods.results[i].clone();
            let icon = self.mods_icon(&project, 56.);
            let p = &project;
            let m = &self.mods;
            let busy = m.busy.as_ref().map(|b| b.0.as_str());
            let support = m.support(p);
            let checking = p.kind == PackKind::Mod
                && matches!(
                    m.checks.get(&(p.source, p.id.clone())),
                    None | Some(Checking::Running)
                )
                && p.support().installable();
            let color = match support {
                Support::Works => t.success,
                Support::Partial(_) => t.warn,
                Support::No(_) => t.muted,
            };
            let mut right = div().flex().items_center().gap(px(8.)).child(blocks::badge(
                support.label(),
                color,
                &t,
            ));
            let page_url = p.page.clone();
            if !page_url.is_empty() {
                right = right.child(
                    button(
                        SharedString::from(format!("mods-page-{}", p.id)),
                        "Page",
                        Kind::Ghost,
                        &t,
                    )
                    .on_click(move |_, _, cx| cx.open_url(&page_url)),
                );
            }
            if busy == Some(p.id.as_str()) {
                right = right.child(caps("Installing").text_color(t.accent));
            } else if m.entry(p).is_some() {
                right = right.child(caps("Installed").text_color(t.muted));
            } else if checking {
                right = right.child(caps("Checking").text_color(t.muted));
            } else if support.installable() {
                let label = if p.kind == PackKind::Modpack {
                    "Import"
                } else {
                    "Install"
                };
                let mut install = button(
                    SharedString::from(format!("mods-install-{}", p.id)),
                    label,
                    Kind::Secondary,
                    &t,
                );
                if busy.is_some() {
                    install = disabled(install);
                } else {
                    let p = p.clone();
                    install = install.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.mods_install(p.source, p.id.clone(), Some(p.clone()));
                        cx.notify();
                    }));
                }
                right = right.child(install);
            }
            let mut by = Vec::new();
            if !p.author.is_empty() {
                by.push(format!("by {}", p.author));
            }
            by.push(format!("{} downloads", count(p.downloads)));
            by.push(format!("from {}", p.source.label()));
            if !p.license.is_empty() {
                by.push(p.license.clone());
            }
            let detail = div()
                .flex()
                .flex_col()
                .child(div().truncate().child(p.summary.clone()))
                .child(by.join(" · "))
                .children(
                    support
                        .reason()
                        .map(|r| div().text_color(t.ink_soft).child(r.to_string())),
                );
            let open = p.clone();
            let name = div()
                .id(SharedString::from(format!("mods-open-{}", p.id)))
                .flex()
                .items_center()
                .gap(px(12.))
                .cursor_pointer()
                .child(icon)
                .child(div().hover(|s| s.underline()).child(p.title.clone()))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.mods_open(open.clone());
                    cx.notify();
                }));
            index_row(
                i + 1,
                name,
                Some(detail.into_any_element()),
                Some(right.into_any_element()),
                &t,
            )
        };
        let list = rows.column(&scroll, result_count, 8., |_| false, row);
        let mut out = div().flex().flex_col().child(list);
        let m = &self.mods;
        if m.more {
            let mut more = button("mods-more", "More", Kind::Ghost, &t);
            if m.searching {
                more = disabled(more);
            } else {
                let offset = m.results.len();
                more = more.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.mods_search(offset);
                    cx.notify();
                }));
            }
            out = out.child(div().flex().pt(px(16.)).child(more));
        }
        out
    }
}

/// The source switch while Modrinth is the only source: Modrinth chosen, CurseForge shown as
/// coming soon and not clickable, like the Microsoft account's "Coming soon".
fn source_switch(t: &Theme) -> Stateful<Div> {
    let soon = div()
        .flex()
        .items_center()
        .h(px(theme::SEGMENT))
        .px(px(14.))
        .pb(px(2.))
        .opacity(theme::DISABLED)
        .child(
            blocks::pixel(14., FontWeight::MEDIUM)
                .text_color(t.muted)
                .child("CurseForge (coming soon)"),
        );
    segments("mods-source", &SOURCES, SourceId::Modrinth, t, |_, _, _| {}).child(soon)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_and_checksums_read_short() {
        assert_eq!(count(999), "999");
        assert_eq!(count(1_500), "1.5k");
        assert_eq!(count(51_404_735), "51.4M");
        let sum = format!("SHA-512:{}", "ab".repeat(64));
        assert_eq!(short_sum(&sum), "SHA-512 abababababab…abababab");
        assert_eq!(short_sum("SHA-1:abcd"), "SHA-1:abcd");
    }
}
