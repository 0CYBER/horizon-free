//! The Mods page: its tabs are the kinds (mods, modpacks, resource packs, shader packs, data
//! packs). Search Horizon's own registry for what was made for Horizon, or Modrinth for
//! resource packs, shader packs and data packs (a Java instance's supported files
//! always come from Modrinth, for its version), see where each comes
//! from and how well it runs (a Java mod's jar is checked first; a Horizon mod says whether it is
//! signed and what it may do), install it with its checksum checked, turn Horizon mods on and
//! off, and update or remove what was installed (`horizon_launcher::browser`). Imported modpacks
//! are listed on its Modpacks tab.
use super::line_edit::{self, Key, LineEdit};
use super::*;
use horizon_launcher::browser::{
    self, horizon, installed, java, java_instance, Kind as PackKind, Project, Query, SourceId,
    Support,
};
use horizon_launcher::modpacks;
use horizon_launcher::native_status;
use horizon_port_pack::check::Check;
use java_instance::Target;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The kinds, as the Mods page's tabs: name and icon.
const KINDS: [(PackKind, &str, &str); 5] = [
    (PackKind::Mod, "Mods", "box"),
    (PackKind::Modpack, "Modpacks", "archive"),
    (PackKind::ResourcePack, "Resource packs", "image"),
    (PackKind::Shader, "Shader packs", "zap"),
    (PackKind::DataPack, "Data packs", "notes"),
];
/// Java mods checked at once.
const CHECKING: usize = 2;
const PAGE: usize = 20;
const DEFAULT_SCAN_PAGES: usize = 3;
const SOURCES: [(SourceId, &str); 2] = [
    (SourceId::Horizon, "Horizon"),
    (SourceId::Modrinth, "Modrinth"),
];

#[path = "glass_mods.rs"]
mod glass_mods;

pub(super) enum Event {
    /// A search's answer, for the search it was (stale answers are dropped) at that offset.
    Found(u64, usize, Result<browser::SearchPage, String>),
    Progress(String, f32),
    Done(Result<String, String>, bool),
    ConfirmPack {
        title: String,
        reason: String,
        action: &'static str,
        reply: Sender<bool>,
    },
    UpdateApplied(PathBuf, (SourceId, String)),
    Updates(u64, HashMap<(SourceId, String), UpdateOffer>),
    /// A Java mod's check: its jar read, or why it couldn't be.
    Checked((SourceId, String), Result<Box<Check>, String>),
    /// What a Horizon registry project says before install: signed or not, what it may do.
    About(HashMap<String, Vec<String>>),
    /// Registry files taken down that the launcher turned off.
    TakenDown(Vec<String>),
    /// What the automatic updates did (`horizon::auto`).
    AutoUpdated(Vec<String>),
    /// A project page's part, an image or a filter's values (`mods_project`).
    Loaded(super::mods_project::Loaded),
    /// The palette's registry mods for its search of that number.
    Palette(u64, Vec<Project>),
}

pub(super) struct UpdateOffer {
    installed_version: String,
    number: String,
}

fn current_offers(
    found: HashMap<(SourceId, String), UpdateOffer>,
    installed: &installed::Installed,
) -> HashMap<(SourceId, String), String> {
    found
        .into_iter()
        .filter_map(|((source, id), offer)| {
            installed
                .find(source, &id)
                .filter(|entry| entry.version == offer.installed_version)
                .map(|_| ((source, id), offer.number))
        })
        .collect()
}

fn confirm_pack(tx: &Sender<Event>, title: String, reason: &str, action: &'static str) -> bool {
    let (reply, answer) = channel();
    tx.send(Event::ConfirmPack {
        title,
        reason: reason.to_owned(),
        action,
        reply,
    })
    .is_ok()
        && answer.recv().unwrap_or(false)
}

/// The save folder distinguishes worlds with the same in-game name.
fn world_picker_folder(path: &Path) -> String {
    let folder = path
        .file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy();
    trf("Folder: {folder}", &[("folder", &folder)])
}

fn world_picker_text(name: &str, folder: &str, t: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .child(
            text(theme::LABEL)
                .text_color(t.ink)
                .truncate()
                .child(name.to_owned()),
        )
        .child(
            text(theme::CAPTION)
                .text_color(t.muted)
                .truncate()
                .child(folder.to_owned()),
        )
}

/// Where a Java mod's check is.
pub(super) enum Checking {
    Running,
    Done(Box<Check>),
    Failed(String),
}

pub(super) enum InstallRequest {
    Browse(Box<Project>),
    Palette(Box<Project>),
    Installed(SourceId, String),
}

impl InstallRequest {
    pub(super) fn item(&self) -> (SourceId, &str) {
        match self {
            Self::Browse(p) | Self::Palette(p) => (p.source, &p.id),
            Self::Installed(source, id) => (*source, id),
        }
    }
}

pub(super) struct Mods {
    pub(super) kind: PackKind,
    pub(super) source: SourceId,
    text: LineEdit,
    focus: FocusHandle,
    pub(super) focused: bool,
    pub(super) everything: bool,
    /// Which search is the current one; answers for older ones are dropped.
    search: u64,
    /// The submitted page, including filters and offset, for More and Try again.
    submitted: Option<Query>,
    searching: bool,
    searched: bool,
    results: Vec<Project>,
    results_list: cull::Cull,
    installed_list: cull::Cull,
    pub(super) report_list: cull::Cull,
    next_offset: Option<usize>,
    error: Option<String>,
    /// The failed search's own words are shown under its plain message.
    pub(super) error_details: bool,
    pub(super) installed: Vec<installed::Entry>,
    /// The registry native mods' versions the player picked on their Versions tab, by id
    /// (`horizon::chosen`), read with `installed`.
    pub(super) chosen: std::collections::BTreeMap<String, String>,
    /// The Horizon the selected instance played when its updates were last looked for.
    pub(super) checked_for: Option<String>,
    loaded: bool,
    /// Whether the page searched for the instance as it is (its version and loader).
    asked: bool,
    instance_id: Option<String>,
    /// The game folder the browser installs into and lists: the selected instance's.
    home: Option<PathBuf>,
    /// The selected Java instance's version and loader: its mods and resource packs are the
    /// source's own files for them, put in as they are.
    pub(super) java: Option<Target>,
    /// Its worlds (folder, name), the last played first, and the one data packs go into.
    worlds: Vec<(PathBuf, String)>,
    world: Option<PathBuf>,
    world_picker_open: bool,
    /// The project being installed, and the line and fraction of its download.
    pub(super) busy: Option<(String, String, f32)>,
    pub(super) message: Option<Result<String, String>>,
    pub(super) updates: HashMap<(SourceId, String), String>,
    update_check: u64,
    confirm_remove: Option<String>,
    /// The project the glass look's detail card shows; the first result when none is.
    pub(super) picked: Option<String>,
    /// The project the glass detail card showed last.
    pub(super) shown: Option<Project>,
    /// An install awaiting explicit native-code consent, before any download.
    /// The selected instance's native build state, the mods it would build again
    /// (`native_status::unbuilt`), and the `native_status::fingerprint` they were read for: read
    /// again only when that changes.
    pub(super) native_state: std::cell::RefCell<Option<crate::build_mode::NativeStateMemo>>,
    /// Native routes read by a worker for this instance's files, consent and game version.
    pub(super) native_routes: std::cell::RefCell<Option<crate::build_mode::RouteMemo>>,
    /// What updating the native mods to a newer installed game would do, read in the background.
    pub(super) native_update: std::cell::RefCell<Option<crate::build_mode::UpdateMemo>>,
    /// The update the player chose to stay off (its memo key): its summary isn't shown again.
    pub(super) update_stayed: std::cell::RefCell<Option<String>>,
    /// Installed wasm declarations, reread only when the instance's files or enable list change.
    /// The mod whose route menu is open.
    pub(super) route_open: Option<String>,
    /// The instance and the mods whose compile failed in the last stop: their chips say so until
    /// the next build attempt, whatever the player chose.
    pub(super) unbuilt: Option<(String, Vec<String>)>,
    native_files: std::cell::RefCell<Option<(String, Vec<native_status::Installed>)>>,
    /// Whether any tracked file asks for a native build, by the same key as `native_files`:
    /// telling an old-format `.hmod` apart reads the archive, never on each draw.
    tracked_native: std::cell::RefCell<Option<(String, bool)>>,
    /// Java mods' checks, by source and project.
    pub(super) checks: HashMap<(SourceId, String), Checking>,
    /// Horizon registry projects' trust and capabilities, by id ([`horizon::trust_and_capabilities`]).
    pub(super) about: HashMap<String, Vec<String>>,
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

    /// The game folder installs go into: the selected instance's, once the page has read it.
    /// Reads what the browser installed in the game folder again, with the versions picked.
    pub(super) fn reload_installed(&mut self) {
        let home = self.home();
        self.installed = installed::Installed::load(&home).entries;
        self.chosen = horizon::chosen(&home);
    }

    pub(super) fn home(&self) -> PathBuf {
        self.home.clone().unwrap_or_else(backend::game_home)
    }

    /// Reads the instance's worlds again; the world picked for data packs stays when it is
    /// still there, else the last played.
    fn read_worlds(&mut self) {
        self.worlds = horizon_launcher::worlds::list(&self.home())
            .into_iter()
            .map(|w| (w.path, w.name))
            .collect();
        if !self
            .worlds
            .iter()
            .any(|(p, _)| Some(p) == self.world.as_ref())
        {
            self.world = self.worlds.first().map(|(p, _)| p.clone());
        }
    }

    fn selected_world(&self) -> Option<&(PathBuf, String)> {
        let selected = self.world.as_ref()?;
        self.worlds.iter().find(|(path, _)| path == selected)
    }

    /// Whether the Modpacks tab is open (the sidebar marks the pack shown there).
    pub(super) fn on_modpacks(&self) -> bool {
        self.kind == PackKind::Modpack
    }

    pub(super) fn new(cx: &mut App) -> Self {
        let (tx, rx) = channel();
        let page = std::env::var("HORIZON_LAUNCHER_PAGE");
        let kind = match page.as_deref() {
            Ok("modpacks") => PackKind::Modpack,
            Ok("resourcepacks") => PackKind::ResourcePack,
            Ok("shaders") => PackKind::Shader,
            Ok("datapacks") => PackKind::DataPack,
            _ => PackKind::Mod,
        };
        Self {
            kind,
            source: source_for(kind),
            text: LineEdit::new(String::new(), 100, 40),
            focus: cx.focus_handle().tab_index(0).tab_stop(true),
            focused: false,
            everything: false,
            search: 0,
            submitted: None,
            searching: false,
            searched: false,
            results: Vec::new(),
            results_list: cull::Cull::default(),
            installed_list: cull::Cull::default(),
            report_list: cull::Cull::default(),
            next_offset: None,
            error: None,
            error_details: false,
            installed: Vec::new(),
            chosen: Default::default(),
            checked_for: None,
            loaded: false,
            asked: false,
            instance_id: None,
            home: None,
            java: None,
            worlds: Vec::new(),
            world: None,
            world_picker_open: false,
            busy: None,
            message: None,
            native_state: Default::default(),
            native_routes: Default::default(),
            native_update: Default::default(),
            update_stayed: Default::default(),
            route_open: None,
            unbuilt: None,
            native_files: Default::default(),
            tracked_native: Default::default(),
            updates: HashMap::new(),
            update_check: 0,
            confirm_remove: None,
            picked: None,
            shown: None,
            checks: HashMap::new(),
            about: HashMap::new(),
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

    /// The Java instance `kind` goes into as files, when one is selected.
    pub(super) fn java_for(&self, kind: PackKind) -> Option<&Target> {
        self.java.as_ref().filter(|t| t.takes(kind))
    }

    pub(super) fn source(&self) -> SourceId {
        browse_source(self.kind, self.source, self.java.as_ref())
    }

    fn install_kind(&self, request: &InstallRequest) -> Option<PackKind> {
        match request {
            InstallRequest::Browse(p) if p.kind == self.kind && p.source == self.source() => {
                Some(p.kind)
            }
            InstallRequest::Palette(p)
                if p.kind == PackKind::Mod && p.source == SourceId::Horizon =>
            {
                Some(p.kind)
            }
            InstallRequest::Installed(source, id) => self
                .installed
                .iter()
                .find(|e| e.source == *source && e.project == *id)
                .map(|e| e.kind),
            _ => None,
        }
    }

    fn new_query(&self) -> Query {
        let everything = self.everything && self.source() != SourceId::Horizon;
        match self.java_for(self.kind) {
            Some(target) => Query {
                offset: 0,
                limit: PAGE,
                ..target.search(self.text.text(), self.kind, &self.browse.filters)
            },
            None => Query {
                everything,
                offset: 0,
                limit: PAGE,
                filters: self.browse.filters.clone(),
                ..Query::new(self.text.text(), self.kind)
            },
        }
    }

    fn begin_page(&mut self, offset: usize) -> Option<Query> {
        let query = self.submitted.as_mut()?;
        query.offset = offset;
        self.asked = true;
        self.search += 1;
        self.searching = true;
        self.error = None;
        Some(query.clone())
    }

    /// Only the filtered default pack browser may advance on its own.
    fn filtered_pack_defaults(&self) -> bool {
        self.can_show_everything()
            && self
                .submitted
                .as_ref()
                .is_some_and(|query| query.text.is_empty() && !query.everything)
    }

    fn next_default_page(&self) -> Option<usize> {
        let offset = self.next_offset?;
        (offset < PAGE * DEFAULT_SCAN_PAGES
            && self.filtered_pack_defaults()
            && self.shown().is_empty())
        .then_some(offset)
    }

    fn reset_browse_context(&mut self) {
        self.asked = false;
        self.everything = false;
        self.browse.filters = Default::default();
        self.browse.open = None;
        self.world_picker_open = false;
        self.picked = None;
        self.shown = None;
        self.search += 1;
        self.submitted = None;
        self.searching = false;
        self.searched = false;
        self.error = None;
        self.next_offset = None;
        self.results.clear();
        self.results_list.reset();
    }

    fn set_target(
        &mut self,
        instance_id: Option<String>,
        home: Option<PathBuf>,
        java: Option<Target>,
    ) -> bool {
        let changed = self.instance_id != instance_id || self.home != home || self.java != java;
        if changed {
            self.reset_browse_context();
            self.update_check += 1;
            self.instance_id = instance_id;
            self.home = home;
            self.java = java;
        }
        changed
    }

    fn set_kind(&mut self, kind: PackKind) -> bool {
        if self.kind == kind {
            return false;
        }
        self.kind = kind;
        self.source = source_for(kind);
        self.reset_browse_context();
        if kind == PackKind::DataPack {
            self.read_worlds();
        }
        true
    }

    /// Keep the last card across an empty search within the current browse context.
    fn picked_project(&mut self) -> Option<Project> {
        let shown = self.shown();
        let mut listed = shown.iter().map(|&i| &self.results[i]);
        let found = self
            .picked
            .as_ref()
            .and_then(|id| listed.clone().find(|p| &p.id == id))
            .or(listed.next())
            .cloned();
        if let Some(p) = found {
            self.shown = Some(p.clone());
            return Some(p);
        }
        self.shown.clone().filter(|p| {
            self.searched
                && !self.filtered_pack_defaults()
                && !self.searching
                && self.error.is_none()
                && p.kind == self.kind
                && p.source == self.source()
        })
    }

    fn can_show_everything(&self) -> bool {
        self.source() == SourceId::Modrinth && self.java.is_none() && self.kind != PackKind::Shader
    }

    /// Whether a Java mod's check is still to come before it can be installed.
    pub(super) fn checking(&self, p: &Project) -> bool {
        self.java_for(p.kind).is_none()
            && p.kind == PackKind::Mod
            && p.source != SourceId::Horizon
            && matches!(
                self.checks.get(&(p.source, p.id.clone())),
                None | Some(Checking::Running)
            )
            && p.support().installable()
    }

    /// The results listed, by index into `results` (#18071).
    pub(super) fn shown(&self) -> Vec<usize> {
        shown(&self.results, self.everything, |p| self.support(p))
    }

    /// Flips Show everything. Its checks go too: `everything` picks which file a check reads.
    pub(super) fn toggle_everything(&mut self) {
        self.everything = !self.everything;
        if !self.everything {
            self.browse.filters.game_version = None;
            if self.kind == PackKind::ResourcePack {
                self.browse.filters.categories.retain(|tag| tag != "modded");
            }
        }
        self.checks.clear();
    }

    /// How well a result runs: a Java mod's from its check once it has one. In a Java
    /// instance, a result was searched for its version and loader: installing checks the file.
    pub(super) fn support(&self, p: &Project) -> Support {
        if !can_install_in(p.kind, p.source, self.java.as_ref()) {
            return Support::No(
                tr("This pack cannot be installed in the selected instance.").into(),
            );
        }
        if self.java_for(p.kind).is_some() {
            return Support::Works;
        }
        let metadata = p.support();
        if p.kind != PackKind::Mod || p.source == SourceId::Horizon || !metadata.installable() {
            return metadata;
        }
        match self.checks.get(&(p.source, p.id.clone())) {
            Some(Checking::Done(c)) => java::support_of(c),
            Some(Checking::Failed(e)) => {
                Support::Partial(trf("Not checked: {error}", &[("error", e)]))
            }
            _ => Support::Partial(tr("Checking its jar…").into()),
        }
    }

    pub(super) fn needs_pack_review(&self, p: &Project) -> bool {
        reviews_resource_pack(p.kind, p.source, self.java.as_ref())
    }
}

/// The indices of `results` to list: with Show everything off, what doesn't run in Horizon
/// stays out. A Modrinth mod's verdict comes from its jar check after the search, so the search
/// alone can't leave these out; `results` keeps every row, as paging counts on it.
fn shown(
    results: &[Project],
    everything: bool,
    support: impl Fn(&Project) -> Support,
) -> Vec<usize> {
    (0..results.len())
        .filter(|&i| everything || support(&results[i]).installable())
        .collect()
}

/// `51404735` → `51.4M`.
fn count(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.1}k", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

/// A registry project's rating: five pixel stars, the mean's nearest whole number of them lit, the
/// mean and how many; then its GitHub stars ([`github_stars`]). "No ratings yet" only when it has
/// neither; None while the ratings aren't known and it has no GitHub stars.
pub(super) fn stars(id: &str, t: &theme::Theme) -> Option<Div> {
    let github = github_stars(id, t);
    let row = div().flex().items_center().gap(px(2.));
    let row = match horizon::rating(id) {
        Some(Some(r)) => {
            let lit = r.average.round() as usize;
            row.children((0..5).map(|i| {
                blocks::icon(
                    "star",
                    12.,
                    if i < lit {
                        t.warn
                    } else {
                        t.muted.opacity(0.6)
                    },
                )
            }))
            .child(div().ml(px(4.)).child(format!(
                "{:.1} ({})",
                r.average,
                count(r.count)
            )))
        }
        Some(None) if github.is_none() => row.text_color(t.muted).child(tr("No ratings yet")),
        _ => row,
    };
    match github {
        Some(g) => Some(row.gap(px(8.)).child(g)),
        None if horizon::rating(id).is_some() => Some(row),
        None => None,
    }
}

/// A registry project's GitHub stars: GitHub's mark, a star and how many, as its repository
/// shows them; None when it has none (`horizon::github_stars`).
pub(super) fn github_stars(id: &str, t: &theme::Theme) -> Option<Div> {
    let n = horizon::github_stars(id)?;
    Some(
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .text_color(t.muted)
            .child(blocks::icon("glass/github", 12., t.muted))
            .child(blocks::icon("star", 12., t.warn))
            .child(count(n)),
    )
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
        if self.mods.set_kind(PackKind::Modpack) && self.mods.loaded {
            self.mods_search(0);
        }
    }

    /// Called with the launcher's other work: what the background sent.
    pub(super) fn mods_tick(
        &mut self,
        window: gpui::AnyWindowHandle,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut changed = false;
        let mut next_page = None;
        let mut to_check = Vec::new();
        let mut loaded = Vec::new();
        let mut done = false;
        while let Ok(event) = self.mods.rx.try_recv() {
            changed = true;
            let m = &mut self.mods;
            match event {
                Event::Palette(search, found) => {
                    if let Some(p) = self.palette.as_mut().filter(|p| p.search == search) {
                        p.mods = found;
                    }
                }
                Event::Found(search, offset, result) if search == m.search => {
                    m.searching = false;
                    m.searched = true;
                    match result {
                        Ok(page) => {
                            let found = page.projects;
                            m.next_offset = page.next_offset;
                            if offset == 0 {
                                m.results.clear();
                                m.results_list.reset();
                            }
                            m.results.extend(found.iter().cloned());
                            m.error = None;
                            next_page = m.next_default_page();
                            to_check.extend(found);
                        }
                        Err(e) => {
                            if offset == 0 {
                                m.results.clear();
                                m.next_offset = None;
                                m.results_list.reset();
                            }
                            m.error = Some(e);
                        }
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
                    done = true;
                    m.busy = None;
                    if let Ok(said) = &result {
                        crate::glass_toast::say(said.clone());
                    }
                    m.message = Some(result);
                    m.reload_installed();
                    m.installed_list.reset();
                    if modpack {
                        // The Modpacks page reads its list again.
                        self.packs_loaded = None;
                    }
                }
                Event::ConfirmPack {
                    title,
                    reason,
                    action,
                    reply,
                } => {
                    match cx.update_window(window, |_, window, cx| {
                        window.prompt(
                            gpui::PromptLevel::Warning,
                            &title,
                            Some(&reason),
                            &[tr("Cancel"), action],
                            cx,
                        )
                    }) {
                        Ok(answer) => cx
                            .spawn(async move |_, _| {
                                let _ = reply.send(matches!(answer.await, Ok(1)));
                            })
                            .detach(),
                        Err(_) => {
                            let _ = reply.send(false);
                        }
                    }
                }
                Event::UpdateApplied(home, key) => {
                    if m.home() == home {
                        m.updates.remove(&key);
                    }
                }
                Event::Updates(check, found) if check == m.update_check => {
                    let current = installed::Installed::load(&m.home());
                    m.updates = current_offers(found, &current);
                }
                Event::Updates(..) => {}
                Event::Checked(key, result) => {
                    let state = match result {
                        Ok(c) => Checking::Done(c),
                        Err(e) => Checking::Failed(e),
                    };
                    m.checks.insert(key, state);
                    // A row may leave the list (`shown`): measure them again (`cull`).
                    m.results_list.reset();
                }
                Event::Loaded(l) => loaded.push(l),
                Event::About(about) => {
                    // Rows grow by a line: measure them again (`cull`).
                    m.about.extend(about);
                    m.results_list.reset();
                }
                Event::AutoUpdated(lines) => {
                    if !lines.is_empty() {
                        m.message = Some(Ok(lines.join("\n")));
                        m.reload_installed();
                        m.installed_list.reset();
                    }
                }
                Event::TakenDown(lines) => {
                    m.message = Some(Ok(lines.join("\n")));
                    m.reload_installed();
                    m.installed_list.reset();
                }
            }
        }
        // `HORIZON_LAUNCHER_PROJECT=<slug>` (with `HORIZON_LAUNCHER_PAGE=mods`): opens that
        // project's page once a search finds it, for screenshots. Once per run, kept in a flag
        // rather than by removing the variable: `remove_var` races `getenv` on the download, check
        // and keyring threads running by now (unsound, and `unsafe` from edition 2024).
        static PROJECT_OPENED: std::sync::atomic::AtomicBool =
            std::sync::atomic::AtomicBool::new(false);
        if self.mods.browse.open.is_none()
            && !PROJECT_OPENED.load(std::sync::atomic::Ordering::Relaxed)
        {
            if let Ok(want) = std::env::var("HORIZON_LAUNCHER_PROJECT") {
                if let Some(p) = self
                    .mods
                    .results
                    .iter()
                    .find(|p| p.id == want || p.slug == want)
                    .cloned()
                {
                    PROJECT_OPENED.store(true, std::sync::atomic::Ordering::Relaxed);
                    self.mods_open(p);
                }
            }
        }
        if let Some(offset) = next_page {
            self.fetch_mods_page(offset);
        }
        self.mods_check(to_check);
        if done {
            // A data pack put into a world shows in its list on Home.
            self.world_packs.reread();
        }
        for l in loaded {
            self.mods_loaded(l);
        }
        changed
    }

    pub(super) fn mods_search(&mut self, offset: usize) {
        self.mods_search_keeping(offset, false);
    }

    /// Searches; with `keep`, the rows already listed stay on screen until the new ones arrive,
    /// so the page doesn't shrink (and the scroll doesn't jump to the top) while it waits.
    pub(super) fn mods_search_keeping(&mut self, offset: usize, keep: bool) {
        if offset == 0 {
            let query = self.mods.new_query();
            if !keep {
                self.mods.results.clear();
                self.mods.next_offset = None;
                self.mods.results_list.reset();
            }
            self.mods.installed_list.reset();
            self.mods.submitted = Some(query);
        }
        self.fetch_mods_page(offset);
    }

    fn retry_mods_page(&mut self) {
        if let Some(query) = &self.mods.submitted {
            self.fetch_mods_page(query.offset);
        }
    }

    fn fetch_mods_page(&mut self, offset: usize) {
        let m = &mut self.mods;
        let source = m.source();
        let Some(query) = m.begin_page(offset) else {
            return;
        };
        let (tx, search) = (m.tx.clone(), m.search);
        std::thread::spawn(move || {
            let result = browser::source(source)
                .ok_or_else(|| trf("{source} isn't set up.", &[("source", source.label())]))
                .and_then(|s| s.search_page(&query));
            let about: HashMap<String, Vec<String>> = match (&result, source) {
                (Ok(page), SourceId::Horizon) => {
                    let instance = horizon_launcher::instances::selected();
                    page.projects
                        .iter()
                        .map(|p| {
                            let lines = horizon::trust_and_capabilities(&p.id, Some(&instance));
                            (p.id.clone(), lines)
                        })
                        .collect()
                }
                _ => HashMap::new(),
            };
            let _ = tx.send(Event::Found(search, offset, result));
            if !about.is_empty() {
                let _ = tx.send(Event::About(about));
            }
        });
    }

    /// Reads the registry's index again when something installed came from it, and turns off
    /// what it took down since (`horizon::turn_off_revoked`).
    fn mods_taken_down(&mut self) {
        if !self
            .mods
            .installed
            .iter()
            .any(|e| e.source == SourceId::Horizon)
        {
            return;
        }
        let tx = self.mods.tx.clone();
        let home = self.mods.home();
        std::thread::spawn(move || {
            let Ok(index) = horizon::index() else {
                return;
            };
            let lines = horizon::turn_off_revoked(&index, &home);
            if !lines.is_empty() {
                let _ = tx.send(Event::TakenDown(lines));
            }
        });
    }

    /// At start: recover interrupted updates, then update every Horizon instance's registry
    /// content when enabled ([`auto_update`]), reading the selected instance's list again after.
    pub(super) fn mods_auto_update(&mut self) {
        let enabled = self.settings.auto_updates_mods();
        let list: Vec<AutoTarget> = self
            .instances
            .list
            .iter()
            .map(|i| AutoTarget {
                instance: i.clone(),
            })
            .collect();
        let tx = self.mods.tx.clone();
        std::thread::spawn(move || {
            let lines = auto_update(&list, enabled, &|_| {});
            if !lines.is_empty() {
                let _ = tx.send(Event::AutoUpdated(lines));
            }
        });
    }

    /// Turns a Horizon mod the browser installed on or off, as the game's Options > Mods does;
    /// in a Java instance, renames its file.
    pub(super) fn mods_switch(&mut self, e: &installed::Entry) {
        let home = self.mods.home();
        // An update holds the folder: the window doesn't wait for it.
        let Some(_lock) = browser::try_lock_home(&home) else {
            self.mods.message = Some(Err(tr("Wait for the current operation to finish.").into()));
            return;
        };
        if self.mods.java.is_some() {
            let on = !java_instance::is_on(&home, e);
            self.mods.message = Some(java_instance::set_on(&home, e, on).map(|()| {
                format!(
                    "{} is {}. The game reads this when it starts.",
                    e.title,
                    if on { "on" } else { "off" }
                )
            }));
            return;
        }
        let Some(id) = horizon::mod_id(e) else {
            return;
        };
        let on = !java::is_enabled(&home, id);
        if on
            && native_status::is_native_file(&e.file)
            && self.settings.native_mods_allowed_at.is_none()
        {
            self.pending_consent = Some(PendingConsent::Toggle(e.clone()));
            return;
        }
        let revoked = horizon::known_index().and_then(|index| {
            let hash = e.checksum.trim_start_matches("SHA-256:");
            index.revoked(&e.project, hash).map(|r| r.reason.clone())
        });
        self.mods.message = Some(match revoked.filter(|_| on) {
            Some(reason) => Err(format!("{} was taken down: {reason}", e.title)),
            None => java::set_enabled(&home, id, on).map(|()| {
                format!(
                    "{} is {}. The game reads this when it starts.",
                    e.title,
                    if on { "on" } else { "off" }
                )
            }),
        });
    }

    /// Checks the Java mods among `found` that aren't checked yet, a few at a time, on threads:
    /// a cached check answers at once, else the jar is downloaded and read.
    fn mods_check(&mut self, found: Vec<Project>) {
        let m = &mut self.mods;
        if m.java.is_some() {
            return;
        }
        let todo: Vec<Project> = found
            .into_iter()
            .filter(|p| p.kind == PackKind::Mod && p.source != SourceId::Horizon)
            .filter(|p| p.support().installable())
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
            // Checks are kept in the default game folder, shared by every instance.
            let home = backend::game_home();
            std::thread::scope(|s| {
                for _ in 0..CHECKING {
                    s.spawn(|| {
                        while let Some(p) = queue.lock().ok().and_then(|mut q| q.next()) {
                            let result = browser::source(p.source)
                                .ok_or_else(|| {
                                    trf("{source} isn't set up.", &[("source", p.source.label())])
                                })
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
        self.mods.update_check += 1;
        let check = self.mods.update_check;
        if entries.is_empty() {
            return;
        }
        let tx = self.mods.tx.clone();
        let java = self.mods.java.clone();
        let home = self.mods.home();
        std::thread::spawn(move || {
            let mut found = HashMap::new();
            for e in entries {
                // A Horizon row whose file is missing offers Remove, not an update. (A Java
                // instance's jar turned off is renamed: its recorded file isn't there.)
                if java.is_none() && installed::file_missing(&home, &e) {
                    continue;
                }
                let Some(source) = browser::source(e.source) else {
                    continue;
                };
                let newer = match &java {
                    Some(target) => java_instance::update_for(&*source, target, &e),
                    None => installed::update_for(&*source, &e),
                };
                if let Ok(Some(v)) = newer {
                    found.insert(
                        (e.source, e.project),
                        UpdateOffer {
                            installed_version: e.version,
                            number: v.number,
                        },
                    );
                }
            }
            let _ = tx.send(Event::Updates(check, found));
        });
    }

    /// What Update all updates and what its button counts: each installed pack a newer version
    /// was found for, but not a Horizon mod whose file is gone now (its row offers Remove).
    pub(super) fn mods_updatable(&self) -> Vec<&installed::Entry> {
        let home = self.mods.home();
        let mut todo: Vec<_> = self
            .mods
            .installed
            .iter()
            .filter(|e| {
                self.mods
                    .updates
                    .contains_key(&(e.source, e.project.clone()))
            })
            .filter(|e| self.mods.java.is_some() || !installed::file_missing(&home, e))
            .collect();
        todo.dedup_by(|a, b| a.source == b.source && a.project == b.project);
        todo
    }

    /// Updates every installed pack a newer version was found for, one after the other on a
    /// thread, each checked as an install is. One failing doesn't stop the others.
    pub(super) fn mods_update_all(&mut self) {
        self.mods_load();
        let todo = self.mods_updatable().into_iter().cloned().collect();
        self.mods_update_entries(todo);
    }

    fn mods_update_entries(&mut self, todo: Vec<installed::Entry>) {
        if self.mods.busy.is_some() || todo.is_empty() {
            return;
        }
        self.mods.message = None;
        self.mods.busy = Some(("all".into(), tr("Updating…").into(), 0.));
        let tx = self.mods.tx.clone();
        let home = self.mods.home();
        let java = self.mods.java.clone();
        let instance = self
            .selected_instance()
            .map_or_else(|| tr("Horizon").to_owned(), |i| i.name.clone());
        std::thread::spawn(move || {
            let count = todo.len();
            let (mut done, mut failed) = (Vec::new(), Vec::new());
            for (i, entry) in todo.into_iter().enumerate() {
                let step = |line: &str, f: f32| {
                    let overall = (i as f32 + f) / count as f32;
                    let _ = tx.send(Event::Progress(
                        trf(
                            "{current} of {count}: {line}",
                            &[
                                ("current", &(i + 1).to_string()),
                                ("count", &count.to_string()),
                                ("line", line),
                            ],
                        ),
                        overall,
                    ));
                };
                let updated = browser::source(entry.source)
                    .ok_or_else(|| {
                        trf(
                            "{source} isn't set up.",
                            &[("source", entry.source.label())],
                        )
                    })
                    .and_then(|source| {
                        if reviews_resource_pack(entry.kind, entry.source, java.as_ref()) {
                            return installed::update_modrinth_resource_pack(
                                &*source,
                                &entry,
                                &home,
                                &|version, reason| {
                                    let title = trf(
                                        "Update {pack} to {version} in {instance}?",
                                        &[
                                            ("pack", &entry.title),
                                            ("version", &version.number),
                                            ("instance", &instance),
                                        ],
                                    );
                                    confirm_pack(&tx, title, reason, tr("Update"))
                                },
                                &step,
                            );
                        }
                        let p = source.project(&entry.project)?;
                        if p.kind != entry.kind || !can_install_in(p.kind, p.source, java.as_ref())
                        {
                            return Err(tr(
                                "This pack cannot be installed in the selected instance.",
                            )
                            .into());
                        }
                        match &java {
                            Some(target) => {
                                java_instance::install(&*source, target, &p, &home, &step)
                            }
                            None => installed::install(&*source, &p, true, &home, &step),
                        }
                    });
                match updated {
                    Ok(entries) => {
                        done.extend(
                            entries
                                .into_iter()
                                .filter(|e| e.project == entry.project)
                                .map(|e| format!("{} {}", e.title, e.number)),
                        );
                        let _ = tx.send(Event::UpdateApplied(
                            home.clone(),
                            (entry.source, entry.project),
                        ));
                    }
                    Err(e) => failed.push(trf(
                        "{pack}: {reason}",
                        &[("pack", &entry.title), ("reason", &e)],
                    )),
                }
            }
            let updated = match done.is_empty() {
                true => String::new(),
                false => trf(
                    "Updated {packs}, checksums checked.",
                    &[("packs", &done.join(", "))],
                ),
            };
            let result = match failed.is_empty() {
                true if updated.is_empty() => Ok(tr("Already up to date.").into()),
                true => Ok(updated),
                false => Err(format!(
                    "{updated}\n{}",
                    trf("Not updated: {packs}", &[("packs", &failed.join("\n"))])
                )
                .trim()
                .to_string()),
            };
            let _ = tx.send(Event::Done(result, false));
        });
    }

    /// Installs (or updates) a project on a thread: a pack into its folder, a modpack into
    /// Modpacks.
    pub(super) fn mods_install(&mut self, request: InstallRequest) {
        if self.mods.busy.is_some() {
            return;
        }
        self.mods_load();
        let Some(kind) = self.mods.install_kind(&request) else {
            return;
        };
        let (source, id) = request.item();
        if !can_install_in(kind, source, self.mods.java.as_ref()) {
            self.mods.message = Some(Err(tr(
                "This pack cannot be installed in the selected instance.",
            )
            .into()));
            return;
        }
        if matches!(request, InstallRequest::Installed(..))
            && reviews_resource_pack(kind, source, self.mods.java.as_ref())
        {
            if let Some(entry) = self
                .mods
                .installed
                .iter()
                .find(|e| e.source == source && e.project == id)
            {
                self.mods_update_entries(vec![entry.clone()]);
            }
            return;
        }
        let pack = match &request {
            InstallRequest::Browse(p) if self.mods.needs_pack_review(p) => Some(p.as_ref()),
            _ => None,
        };
        let mut reviewed_pack = None;
        if let Some(p) = pack {
            let open = self
                .mods
                .browse
                .open
                .as_ref()
                .filter(|o| o.project.source == p.source && o.project.id == p.id);
            match open.and_then(|o| o.pack_review.as_ref()) {
                Some(Ok(review)) if review.support.installable() => {
                    reviewed_pack = Some(review.clone());
                }
                _ => {
                    if open.is_none() {
                        self.mods_open(p.clone());
                    }
                    return;
                }
            }
        }
        // The first native mod: the trust notice, before anything is installed.
        let first_native = source == SourceId::Horizon
            && self.settings.native_mods_allowed_at.is_none()
            && horizon::is_native_mod(id);
        if first_native {
            self.pending_consent = Some(PendingConsent::Install(request));
            self.content_scroll
                .handle
                .set_offset(gpui::point(px(0.), px(0.)));
            return;
        }
        let id = id.to_owned();
        let known = match request {
            InstallRequest::Browse(p) | InstallRequest::Palette(p) => Some(*p),
            InstallRequest::Installed(..) => None,
        };
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
        let home = self.mods.home();
        let java = self.mods.java.clone();
        let instance = self
            .selected_instance()
            .map_or_else(|| "Horizon".to_string(), |i| i.name.clone());
        let world = self.mods.selected_world().cloned();
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
                .ok_or_else(|| trf("{source} isn't set up.", &[("source", source.label())]))
                .and_then(|source| {
                    let p = match known {
                        Some(p) => p,
                        None => source.project(&id)?,
                    };
                    if p.kind != kind || !can_install_in(p.kind, p.source, java.as_ref()) {
                        return Err(
                            tr("This pack cannot be installed in the selected instance.").into(),
                        );
                    }
                    if let Some(target) = java.as_ref().filter(|t| t.takes(p.kind)) {
                        let done = java_instance::install(&*source, target, &p, &home, &progress)?;
                        let names: Vec<String> = done
                            .iter()
                            .map(|e| format!("{} {}", e.title, e.number))
                            .collect();
                        return Ok(trf(
                            "Installed {mods}, checksums checked.",
                            &[("mods", &names.join(", "))],
                        ));
                    }
                    if p.kind == PackKind::DataPack {
                        let (world, name) = world.ok_or_else(|| {
                            tr("Pick the world the data pack goes into first.").to_string()
                        })?;
                        let done =
                            installed::install_datapack(&*source, &p, everything, &world, &|f| {
                                progress(&p.title, f)
                            })?;
                        return Ok(trf(
                            "Put {pack} {version} in {world}'s data packs, checksum checked: the \
                             game turns it on when the world opens. {coverage}",
                            &[
                                ("pack", &p.title),
                                ("version", &done.number),
                                ("world", &name),
                                ("coverage", &done.coverage.summary()),
                            ],
                        ));
                    }
                    modpack = p.kind == PackKind::Modpack;
                    if modpack && p.source == SourceId::Horizon {
                        let pack = horizon::pack::install(&p.id, &progress)?;
                        if let Some(url) = &p.icon {
                            horizon_launcher::instances::adopt_icon(
                                &horizon::pack::home(&pack.work),
                                url,
                            );
                        }
                        return Ok(trf(
                            "Installed {pack} {version} as its own profile with {mods}, each \
                             checked. Play it from the list above.",
                            &[
                                ("pack", &pack.name),
                                ("version", &pack.version),
                                ("mods", &pack.mods.join(", ")),
                            ],
                        ));
                    }
                    if modpack {
                        let folder = modpacks::ports_dir().join("downloads");
                        let file = installed::fetch_modpack(&*source, &p, &folder, &|f| {
                            progress(&p.title, f)
                        })?;
                        let log = |line: String| progress(&line, 1.);
                        let pack = modpacks::import_and_port(&file, resources.as_deref(), &log)?;
                        if let Some(url) = &p.icon {
                            horizon_launcher::instances::adopt_icon(
                                &modpacks::profile_home(&pack.work),
                                url,
                            );
                        }
                        return Ok(format!(
                            "{} {}",
                            modpacks::outcome(&pack),
                            tr("Play it from the list above.")
                        ));
                    }
                    let done = match reviewed_pack {
                        Some(review) => {
                            if !review.confirm_optional_mods(|version, reason| {
                                let title = trf(
                                    "Install {pack} {version} in {instance}?",
                                    &[
                                        ("pack", &p.title),
                                        ("version", &version.number),
                                        ("instance", &instance),
                                    ],
                                );
                                confirm_pack(&tx, title, reason, tr("Install"))
                            }) {
                                return Err(tr("Installation canceled.").into());
                            }
                            installed::install_reviewed_resource_pack(
                                &*source, review, &home, &progress,
                            )
                        }
                        None => installed::install(&*source, &p, everything, &home, &progress),
                    }?;
                    let names: Vec<String> = done
                        .iter()
                        .map(|e| format!("{} {}", e.title, e.number))
                        .collect();
                    let mods = names.join(", ");
                    if done.iter().any(|e| native_status::is_native_file(&e.file)) {
                        return Ok(trf(
                            "Installed {mods} into {instance}. Build to play with it.",
                            &[("mods", &mods), ("instance", &instance)],
                        ));
                    }
                    if p.source == SourceId::Horizon && p.kind == PackKind::Mod {
                        return Ok(trf(
                            "Installed {mods} into horizon-mods and turned it on, checksum \
                             checked (Options > Mods in the game).",
                            &[("mods", &mods)],
                        ));
                    }
                    if p.kind == PackKind::Mod {
                        let skipped = done.last().map_or(0, |e| e.skipped.len());
                        let ported = trf(
                            "Ported {mods} into horizon-mods and turned on (Options > Mods in \
                             the game).",
                            &[("mods", &mods)],
                        );
                        return Ok(if skipped > 0 {
                            format!(
                                "{ported} {}",
                                tr("What didn't port is listed under Installed.")
                            )
                        } else {
                            ported
                        });
                    }
                    Ok(pack_install_notice(p.kind, p.source, &mods))
                });
            let _ = tx.send(Event::Done(result, modpack));
        });
    }

    pub(super) fn mods_field(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = self.theme;
        let m = &self.mods;
        let focused = m.focused;
        let placeholder = if t.glass {
            self.mods_placeholder()
        } else {
            tr("Search, then Enter").into()
        };
        let field = div()
            .id("mods-search")
            .track_focus(&m.focus)
            .flex()
            .items_center()
            .gap(px(10.))
            .when(!t.glass, |d| d.w(px(320.)))
            .h(px(theme::FIELD + 4.))
            .px(px(12.))
            .overflow_hidden()
            .border_2()
            .border_color(if focused { t.accent } else { t.outline })
            .bg(t.well)
            .text_size(px(theme::UI.px))
            .cursor_text()
            .child(if t.glass {
                crate::glass_ui::icon("search-01", 17., crate::glass_ui::white(0.55))
                    .into_any_element()
            } else {
                blocks::icon("search", 18., t.muted).into_any_element()
            })
            .child(line_edit::content_with(
                &m.text,
                &t,
                focused,
                false,
                false,
                placeholder,
            ))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                window.focus(&this.mods.focus, cx);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                let had_text = !this.mods.text.text().trim().is_empty();
                match line_edit::handle(&mut this.mods.text, ev, cx) {
                    Key::Enter => this.mods_search(0),
                    _ if had_text
                        && this.mods.text.text().trim().is_empty()
                        && this.mods.source() == SourceId::Modrinth =>
                    {
                        this.mods_search(0)
                    }
                    Key::Escape => window.blur(cx),
                    Key::Pass => return,
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }));
        // The glass look's: `rounded-[12px] h-11 bg-white/[0.07] px-3.5`, a 17 px search icon.
        let field = if t.glass {
            use horizon_theme::glass::{fill, radius};
            field
                .h(px(44.))
                .px(px(14.))
                .border_0()
                .rounded(px(radius::ROW))
                .bg(crate::glass_ui::white(if focused {
                    fill::FIELD_FOCUS
                } else {
                    fill::FIELD
                }))
                .font_family(horizon_theme::glass::FONT)
                .text_size(px(14.))
        } else {
            field
        };
        line_edit::mouse(field, cx, |this| &mut this.mods.text)
    }

    /// Reads what the browser installed in the selected instance (once per instance) and looks
    /// for updates: the Mods page and Home's Mods tab (`instance_mods.rs`) show it. Packs and mods
    /// go into the selected instance: picking another reads its list.
    pub(super) fn mods_load(&mut self) {
        let instance = self.selected_instance();
        let id = instance.map(|i| i.id.clone());
        let home = instance.map(|i| i.home.clone());
        let java = instance.and_then(|i| i.java.as_ref()).map(|j| j.target());
        let target_changed = self.mods.set_target(id, home, java);
        // The instance moved to another Horizon (picked, or an update it follows): the versions
        // that fit change, so look for updates again.
        let plays = self
            .selected_instance()
            .and_then(|i| i.version.clone())
            .or_else(|| self.installed.first().cloned());
        if self.mods.loaded
            && !target_changed
            && self.mods.java.is_none()
            && plays != self.mods.checked_for
        {
            self.mods.checked_for = plays.clone();
            self.mods.updates.clear();
            self.mods_check_updates();
        }
        if !self.mods.loaded || target_changed {
            self.mods.checked_for = plays;
            self.mods.reload_installed();
            self.mods.installed_list.reset();
            self.mods.updates.clear();
            self.mods.confirm_remove = None;
            self.mods.read_worlds();
            self.mods_check_updates();
            self.mods_taken_down();
            self.mods.loaded = true;
        }
    }

    /// Opens a kind's tab: its first source, its own filters, a new search.
    pub(super) fn mods_set_kind(&mut self, kind: PackKind) {
        if self.mods.set_kind(kind) {
            self.mods_search(0);
        }
    }

    /// The source control is shared by the Glass and Blocks layouts.
    pub(super) fn mods_source_switch(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        switches(self.mods.kind, self.mods.java.is_some()).then(|| {
            let this = cx.entity().downgrade();
            segments(
                "mods-sources",
                &SOURCES,
                self.mods.source(),
                &self.theme,
                move |source, _, cx| {
                    let _ = this.update(cx, |this, cx| {
                        if this.mods.source != source {
                            this.mods.source = source;
                            this.mods.reset_browse_context();
                            this.mods_search(0);
                            cx.notify();
                        }
                    });
                },
            )
        })
    }

    fn mods_everything_toggle(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = self.theme;
        let label = if t.glass {
            crate::glass_ui::text(13., FontWeight::NORMAL, 0.7)
        } else {
            text(theme::LABEL).text_color(t.ink_soft)
        };
        keyboard(div().id("mods-everything"), &t)
            .flex()
            .items_center()
            .gap(px(if t.glass { 10. } else { 8. }))
            .cursor_pointer()
            .child(widgets::switch_face(
                "mods-everything-switch",
                self.mods.everything,
                &t,
            ))
            .child(label.child(tr("Show everything")))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.mods.toggle_everything();
                this.mods_search(0);
                cx.notify();
            }))
    }

    /// Remove, then Remove for good: an installed project leaves the instance on the second
    /// click.
    pub(super) fn mods_remove_click(&mut self, id: String) {
        if self.mods.confirm_remove.as_ref() == Some(&id) {
            let _ = self.mods_remove(&id);
        } else {
            self.mods.confirm_remove = Some(id);
        }
    }

    /// The project `id` leaves the selected instance now; what happened, for the page's notice.
    pub(super) fn mods_remove(&mut self, id: &str) -> Result<String, String> {
        let home = self.mods.home();
        let Some(_lock) = browser::try_lock_home(&home) else {
            let busy: Result<String, String> =
                Err(tr("Wait for the current operation to finish.").into());
            self.mods.message = Some(busy.clone());
            return busy;
        };
        let removed = match self.mods.java {
            Some(_) => java_instance::remove(&home, id),
            None => installed::remove(&home, id),
        }
        .map(|e| trf("Removed {name}.", &[("name", &e.title)]));
        if let Ok(said) = &removed {
            crate::glass_toast::say(said.clone());
        }
        self.mods.message = Some(removed.clone());
        self.mods.reload_installed();
        self.mods.installed_list.reset();
        self.mods.confirm_remove = None;
        removed
    }

    /// The palette's search (`glass_palette`): up to four of Horizon's registry mods for `text`,
    /// answered as [`Event::Palette`]. None for a Java instance, which takes Modrinth's files.
    pub(super) fn palette_mods(&mut self, text: &str, search: u64) {
        self.mods_load();
        if self.mods.java.is_some() {
            return;
        }
        let tx = self.mods.tx.clone();
        let query = Query {
            limit: 4,
            ..Query::new(text, PackKind::Mod)
        };
        std::thread::spawn(move || {
            let found = browser::source(SourceId::Horizon)
                .ok_or_else(|| "not set up".to_string())
                .and_then(|s| s.search(&query))
                .unwrap_or_default();
            let _ = tx.send(Event::Palette(search, found));
        });
    }

    /// Whether the selected instance has `p` (the palette's Install or Remove).
    pub(super) fn mods_has(&self, p: &Project) -> bool {
        self.mods.entry(p).is_some()
    }

    pub(super) fn mods_page(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        self.mods_load();
        if !self.mods.asked {
            self.mods_search(0);
        }
        if t.glass {
            return self.glass_mods_page(cx);
        }
        let header = page_header(
            "Mods",
            tr("Browse the Horizon Registry and find resource, shader and data packs on Modrinth."),
            &t,
        );
        let this = cx.entity().downgrade();
        let open = self.mods.kind;
        let kinds = blocks::tabs("mods-kind", &KINDS, open, &t, {
            let this = this.clone();
            move |kind, _, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.mods_set_kind(kind);
                    cx.notify();
                });
            }
        });
        // The registry lists only what was made for Horizon: no "everything", and of the filters
        // only its categories.
        let registry = self.mods.source() == SourceId::Horizon;
        let filters = self.mods_filters(cx);
        // The registry's sort sits below the GitHub row, above the list it orders.
        let registry_sort = registry.then(|| self.mods_sort_bar(cx));
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
                    .when(self.mods.can_show_everything(), |d| {
                        d.child(self.mods_everything_toggle(cx))
                    }),
            )
            .children(
                self.mods_source_switch(cx)
                    .map(|switch| div().flex().child(switch)),
            )
            .child(filters)
            .children(self.mods_target_line(cx));
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
        // Native mods from a GitHub link: Horizon instances, on the Mods tab.
        if self.mods.kind == PackKind::Mod && self.mods.java.is_none() {
            body = body.child(self.github_panel(cx));
        }
        body = body.children(registry_sort);
        if let Some(list) = self.mods_installed_list(cx) {
            body = body.child(list);
        }
        page(body.child(self.mods_results(cx)))
    }

    /// Which instance installs go into, even when it is the only one: native mods build into it.
    /// A modpack becomes an instance of its own.
    fn mods_target_line(&self, cx: &mut Context<Self>) -> Option<Div> {
        let instance = self.selected_instance()?;
        if self.mods.kind == PackKind::DataPack {
            return Some(self.mods_world_picker(&instance.name, cx));
        }
        // The glass page's heading already says where installs go.
        if self.mods.kind == PackKind::Modpack || self.theme.glass {
            return None;
        }
        let name = [("instance", instance.name.as_str())];
        Some(text(theme::CAPTION).text_color(self.theme.muted).child(
            if self.instances.list.len() < 2 {
                trf("Installs into {instance}.", &name)
            } else {
                trf(
                    "Installs into {instance}: pick another instance on the left.",
                    &name,
                )
            },
        ))
    }

    /// The world data packs go into: one of the instance's, picked here.
    fn mods_world_picker(&self, instance: &str, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let caption = |s: String| text(theme::CAPTION).text_color(t.muted).child(s);
        let Some((path, name)) = self.mods.selected_world() else {
            return div().child(caption(trf(
                "A data pack goes into a world, and {instance} has none yet: create one in the game, then come back.",
                &[("instance", instance)],
            )));
        };
        let open = self.mods.world_picker_open;
        let folder = world_picker_folder(path);
        let rounding = if t.glass {
            horizon_theme::glass::radius::ROW
        } else {
            theme::RADIUS
        };
        let trigger = keyboard(div().id("mods-world-picker"), &t)
            .flex()
            .items_center()
            .gap(px(12.))
            .h(px(48.))
            .px(px(12.))
            .rounded(px(rounding))
            .bg(t.plate_soft)
            .hover(move |s| s.bg(t.plate))
            .cursor_pointer()
            .child(world_picker_text(name, &folder, &t))
            .tooltip(crate::glass_ui::tip(folder))
            .child(
                blocks::icon("chevron-down", 16., t.muted).with_transformation(
                    gpui::Transformation::rotate(gpui::radians(if open {
                        std::f32::consts::PI
                    } else {
                        0.
                    })),
                ),
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.mods.world_picker_open = !this.mods.world_picker_open;
                cx.notify();
            }));
        let mut picker = div()
            .id("mods-world-picker-wrap")
            .flex()
            .flex_col()
            .gap(px(6.))
            .w_full()
            .max_w(px(360.))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" && this.mods.world_picker_open {
                    this.mods.world_picker_open = false;
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .child(caption(tr("Install data packs into this world").into()))
            .child(trigger);
        if open {
            let mut choices = div()
                .id("mods-world-choices")
                .flex()
                .flex_col()
                .max_h(px(242.))
                .overflow_y_scroll()
                .on_scroll(|_, _, cx| cx.stop_propagation())
                .p(px(4.))
                .rounded(px(rounding))
                .bg(t.plate_soft);
            for (i, (path, name)) in self.mods.worlds.iter().enumerate() {
                let selected = self.mods.world.as_ref() == Some(path);
                let folder = world_picker_folder(path);
                let row = keyboard(div().id(("mods-world", i)), &t)
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(12.))
                    .h(px(46.))
                    .px(px(8.))
                    .rounded(px(rounding))
                    .cursor_pointer()
                    .when(selected, |d| d.bg(t.plate))
                    .hover(move |s| s.bg(t.plate))
                    .child(world_picker_text(name, &folder, &t))
                    .tooltip(crate::glass_ui::tip(folder))
                    .when(selected, |d| d.child(blocks::icon("check", 16., t.accent)));
                let path = path.clone();
                choices = choices.child(row.on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.mods.world = Some(path.clone());
                        this.mods.world_picker_open = false;
                        cx.notify();
                    },
                )));
            }
            picker = picker.child(choices);
        }
        div().child(picker)
    }

    /// The install under way and the last install's answer.
    /// The one-time notice before the first native mod is installed: native mods aren't
    /// sandboxed, and what the build takes. Its button installs the mod.
    pub(super) fn mods_busy_plate(&self) -> Vec<Div> {
        let t = self.theme;
        let mut out = Vec::new();
        if let Some((what, line, f)) = &self.mods.busy {
            let caption = if what == "all" {
                tr("Updating")
            } else {
                tr("Installing")
            };
            out.push(
                plate(&t)
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .p(px(16.))
                    .child(caps(caption).text_color(t.accent))
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

    /// Re-read local/GitHub files when the folder changes, not on every frame.
    fn native_files(&self) -> Vec<native_status::Installed> {
        let home = self.mods.home();
        let key = native_status::fingerprint(&home, None);
        let mut memo = self.mods.native_files.borrow_mut();
        if memo.as_ref().is_none_or(|(k, _)| *k != key) {
            *memo = Some((key, native_status::untracked(&home)));
            self.mods.installed_list.reset();
        }
        memo.as_ref().unwrap().1.clone()
    }

    /// Whether a tracked file in `entries` asks for a native build
    /// ([`native_status::tracked_asks_for_build`]), kept until the native folder changes.
    fn tracked_asks_for_build(&self, entries: &[&installed::Entry]) -> bool {
        let home = self.mods.home();
        let key = format!(
            "{}{}",
            native_status::fingerprint(&home, None),
            entries
                .iter()
                .map(|e| e.file.as_str())
                .collect::<Vec<_>>()
                .join(
                    "
"
                )
        );
        let mut memo = self.mods.tracked_native.borrow_mut();
        if memo.as_ref().is_none_or(|(k, _)| *k != key) {
            let asks = entries
                .iter()
                .any(|e| native_status::tracked_asks_for_build(&home, &e.file));
            *memo = Some((key, asks));
        }
        memo.as_ref().unwrap().1
    }

    fn mods_installed_list(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        let t = self.theme;
        let kind = self.mods.kind;
        // Each project's own icon (Modrinth's, asked for once), at the row's size.
        let ids = self
            .mods
            .installed
            .iter()
            .filter(|e| e.kind == kind && e.source == SourceId::Modrinth)
            .map(|e| e.project.clone())
            .collect();
        self.ask_project_icons(ids, cx);
        let icons: Vec<Option<PathBuf>> = self
            .mods
            .installed
            .clone()
            .iter()
            .filter(|e| e.kind == kind)
            .map(|e| self.project_icon(e))
            .collect();
        let entries: Vec<&installed::Entry> = self
            .mods
            .installed
            .iter()
            .filter(|e| e.kind == kind)
            .collect();
        let native_entries = if kind == PackKind::Mod && self.mods.java.is_none() {
            self.native_files()
        } else {
            Vec::new()
        };
        if entries.is_empty() && native_entries.is_empty() {
            return None;
        }
        let busy = self.mods.busy.is_some();
        let native = native_status::asks_for_build(&native_entries)
            || self.mods.java.is_none() && self.tracked_asks_for_build(&entries);
        let left_out = if native {
            self.native_left_out()
        } else {
            Vec::new()
        };
        let instance = self.selected_instance().map(|i| i.name.clone());
        let row = |i: usize| {
            if i >= entries.len() {
                return self.native_mod_row(&native_entries[i - entries.len()], busy, cx);
            }
            let e = entries[i];
            let key = (e.source, e.project.clone());
            let is_native = self.mods.java.is_none() && native_status::is_native_file(&e.file);
            let mut right = div().flex().items_center().gap(px(8.));
            if let Some(number) = self.mods.updates.get(&key) {
                let mut update = button(
                    SharedString::from(format!("mods-update-{}", e.project)),
                    trf("Update to {version}", &[("version", number)]),
                    Kind::Secondary,
                    &t,
                );
                if busy {
                    update = disabled(update);
                } else {
                    let (source, id) = key.clone();
                    update = update.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.mods_install(InstallRequest::Installed(source, id.clone()));
                        cx.notify();
                    }));
                }
                right = right.child(update);
            }
            if let Some(kept) = self
                .mods
                .chosen
                .get(&e.project)
                .filter(|_| e.source == SourceId::Horizon)
            {
                let mut recommended = button(
                    SharedString::from(format!("mods-recommended-{}", e.project)),
                    trf("Use recommended (kept at {version})", &[("version", kept)]),
                    Kind::Ghost,
                    &t,
                );
                if busy {
                    recommended = disabled(recommended);
                } else {
                    let id = e.project.clone();
                    recommended =
                        recommended.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.mods_install(InstallRequest::Installed(
                                SourceId::Horizon,
                                id.clone(),
                            ));
                            cx.notify();
                        }));
                }
                right = right.child(recommended);
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
                this.mods_remove_click(id.clone());
                cx.notify();
            }));
            let on = match &self.mods.java {
                _ if e.kind != PackKind::Mod => None,
                Some(_) => Some(java_instance::is_on(&self.mods.home(), e)),
                None if e.source == SourceId::Horizon => Some(
                    horizon::mod_id(e).is_some_and(|id| java::is_enabled(&self.mods.home(), id)),
                ),
                None => None,
            };
            if let Some(on) = on {
                let entry = e.clone();
                right = right.child(
                    div()
                        .id(SharedString::from(format!("mods-switch-{}", e.project)))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .cursor_pointer()
                        // Its id sits under the row's own, so one name serves every row.
                        .child(switch("mods-switch-knob", on, &t))
                        .child(text(theme::LABEL).text_color(t.ink_soft).child(if on {
                            tr("On")
                        } else {
                            tr("Off")
                        }))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.mods_switch(&entry);
                            cx.notify();
                        })),
                );
            }
            if is_native {
                right = right.child(blocks::badge(tr("NATIVE"), t.muted, &t));
            }
            if is_native && self.settings.native_mods_allowed_at.is_none() {
                let entry = e.clone();
                right = right
                    .child(blocks::badge(tr("Needs permission"), t.muted, &t))
                    .child(
                        button(
                            SharedString::from(format!("native-permission-{}", e.project)),
                            "Allow native mods",
                            Kind::Secondary,
                            &t,
                        )
                        .on_click(cx.listener(
                            move |this, _: &ClickEvent, _, cx| {
                                this.pending_consent = Some(PendingConsent::Toggle(entry.clone()));
                                cx.notify();
                            },
                        )),
                    );
            }
            if is_native {
                let folder = self.mods.home().join(native_mods::INSTALLED);
                right = right.child(
                    button(
                        SharedString::from(format!("mods-folder-{}", e.project)),
                        "Open folder",
                        Kind::Ghost,
                        &t,
                    )
                    .on_click(move |_, _, cx| {
                        let _ = std::fs::create_dir_all(&folder);
                        crate::glass_toast::open(cx, &folder);
                    }),
                );
            }
            right = right.child(remove);
            let detail = div()
                .flex()
                .flex_col()
                .child({
                    let from = e
                        .source
                        .shown_for(&e.project)
                        .map(|s| trf("from {source}", &[("source", s)]));
                    let needed = e
                        .dependency
                        .then(|| tr("needed by another pack").to_string());
                    let origin: Vec<String> = from.into_iter().chain(needed).collect();
                    e.verdict
                        .iter()
                        .map(|v| tr(v).to_string())
                        .chain([e.number.clone()])
                        .chain((!origin.is_empty()).then(|| origin.join(", ")))
                        .chain([e.file.clone()])
                        .collect::<Vec<_>>()
                        .join(" · ")
                })
                .children(instance.as_ref().filter(|_| is_native).map(|name| {
                    div()
                        .text_color(t.ink)
                        .child(trf("Installed in {instance}", &[("instance", name)]))
                }))
                .children(e.skipped.iter().map(|s| {
                    div()
                        .text_color(t.ink_soft)
                        .child(trf("Not ported: {item}", &[("item", s)]))
                }))
                .child(div().font_family(theme::MONO).child(trf(
                    "checked: {checksum}",
                    &[("checksum", &short_sum(&e.checksum))],
                )));
            slot_row(
                Some(blocks::slot(
                    icons[i].clone().map(gpui::ImageSource::from),
                    &e.title,
                    40.,
                    &t,
                )),
                e.title.clone(),
                Some(detail.into_any_element()),
                Some(right.into_any_element()),
                &t,
            )
        };
        // Every kind's updates at once, whichever tab is open.
        let updatable = self.mods_updatable().len();
        let update_all = (updatable > 0).then(|| {
            let b = button(
                "mods-update-all",
                trf("Update all ({count})", &[("count", &updatable.to_string())]),
                Kind::Secondary,
                &t,
            );
            if busy {
                disabled(b)
            } else {
                b.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.mods_update_all();
                    cx.notify();
                }))
            }
        });
        let list = self.mods.installed_list.column(
            &self.content_scroll.handle,
            entries.len() + native_entries.len(),
            8.,
            |index| {
                entries
                    .get(index)
                    .is_some_and(|e| self.mods.confirm_remove.as_ref() == Some(&e.project))
            },
            row,
        );
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(caps(tr("Installed")).text_color(t.muted))
                        .children(update_all),
                )
                .children(native.then(|| self.native_build(cx)).flatten())
                .children(
                    left_out
                        .iter()
                        .map(|left| div().text_color(t.warn).child(left.label())),
                )
                .child(list),
        )
    }

    fn mods_retry_button(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        button("mods-retry", "Try again", Kind::Secondary, &self.theme).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                this.retry_mods_page();
                cx.notify();
            },
        ))
    }

    fn mods_results(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let m = &self.mods;
        if let Some(e) = &m.error {
            return div()
                .child(notice(e.clone(), &t))
                .child(self.mods_retry_button(cx));
        }
        let shown = m.shown();
        if shown.is_empty() {
            let line = if m.kind == PackKind::Shader && m.java.is_some() {
                tr("Shader packs for Java instances aren't supported yet.")
            } else if m.searching || !m.searched {
                tr("Searching…")
            } else if !m.results.is_empty() {
                tr("Nothing found that runs in Horizon. Show everything lists the rest.")
            } else if let Some(target) = m.java_for(m.kind) {
                return centered(
                    &trf(
                        "Nothing found for {target}.",
                        &[("target", &target.describe(m.kind))],
                    ),
                    &t,
                );
            } else if m.source() == SourceId::Horizon {
                tr("Nothing in Horizon's registry matches yet.")
            } else if m.everything || !m.can_show_everything() {
                tr("Nothing found.")
            } else {
                tr("Nothing found that runs in Horizon. Show everything lists the rest.")
            };
            let out = div().flex().flex_col().child(centered(line, &t));
            return self.mods_more(out, cx);
        }
        let result_count = shown.len();
        let rows = m.results_list.clone();
        let scroll = self.content_scroll.handle.clone();
        let row = |i: usize| {
            let project = self.mods.results[shown[i]].clone();
            let icon = self.mods_icon(&project, 56.);
            let p = &project;
            let m = &self.mods;
            let busy = m.busy.as_ref().map(|b| b.0.as_str());
            let support = m.support(p);
            let checking = m.checking(p);
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
                right = right.child(caps(tr("Installing")).text_color(t.accent));
            } else if m.entry(p).is_some() {
                right = right.child(caps(tr("Installed")).text_color(t.muted));
            } else if checking {
                right = right.child(caps(tr("Checking")).text_color(t.muted));
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
                        this.mods_install(InstallRequest::Browse(Box::new(p.clone())));
                        cx.notify();
                    }));
                }
                right = right.child(install);
            }
            let mut by = Vec::new();
            if !p.author.is_empty() {
                by.push(trf("by {author}", &[("author", &p.author)]));
            }
            by.push(trf("{count} downloads", &[("count", &count(p.downloads))]));
            if let Some(source) = p.source.shown_for(&p.id) {
                by.push(trf("from {source}", &[("source", source)]));
            }
            if !p.license.is_empty() {
                by.push(p.license.clone());
            }
            let detail = div()
                .flex()
                .flex_col()
                .child(div().truncate().child(p.summary.clone()))
                .child(by.join(" · "))
                .children(
                    (p.source == SourceId::Horizon)
                        .then(|| stars(&p.id, &t))
                        .flatten(),
                )
                .children(
                    support
                        .reason()
                        .map(|r| div().text_color(t.ink_soft).child(r.to_string())),
                )
                .children(
                    m.about
                        .get(&p.id)
                        .filter(|_| p.source == SourceId::Horizon)
                        .map(|lines| div().text_color(t.ink_soft).child(lines.join(" · "))),
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
        self.mods_more(div().flex().flex_col().child(list), cx)
    }

    /// The list's More, under `out` while the source has more pages.
    fn mods_more(&self, mut out: Div, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let m = &self.mods;
        if let Some(offset) = m.next_offset {
            let mut more = button("mods-more", "More", Kind::Ghost, &t);
            if m.searching {
                more = disabled(more);
            } else {
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

fn pack_install_notice(kind: PackKind, source: SourceId, mods: &str) -> String {
    if kind == PackKind::Shader && source == SourceId::Modrinth {
        trf(
            "Installed {mods}, checksums checked. Turn it on in the game under Options > Video Settings > Shaders > Shader Packs.",
            &[("mods", mods)],
        )
    } else {
        trf(
            "Installed {mods}, checksums checked. Turn it on in the game: Options > Resource Packs.",
            &[("mods", mods)],
        )
    }
}

/// An instance whose registry projects may update.
pub(super) struct AutoTarget {
    pub(super) instance: instances::Instance,
}

/// Updates the registry projects of `targets`: Horizon's own instances only, never a modpack's
/// profile, whose pack pins its mods. Returns what happened for the page's notice.
/// `progress` hears the project each update is downloading.
pub(super) fn auto_update(
    targets: &[AutoTarget],
    enabled: bool,
    progress: &dyn Fn(&str),
) -> Vec<String> {
    let mut lines = Vec::new();
    for t in targets {
        let i = &t.instance;
        let pack = i.id.starts_with("pack:") || i.id.starts_with("port:");
        let (false, None) = (pack, &i.java) else {
            continue;
        };
        let ctx = horizon::auto::Context {
            in_pack: horizon::auto::in_pack(&i.home),
        };
        if let Some(report) = horizon::auto::run(&i.home, &ctx, enabled, &|line, _| progress(line))
        {
            lines.extend(report.lines());
        }
    }
    lines
}

fn source_for(kind: PackKind) -> SourceId {
    match kind {
        PackKind::ResourcePack | PackKind::DataPack => SourceId::Modrinth,
        PackKind::Mod | PackKind::Modpack | PackKind::Shader => SourceId::Horizon,
    }
}

fn switches(kind: PackKind, java: bool) -> bool {
    !java
        && matches!(
            kind,
            PackKind::ResourcePack | PackKind::Shader | PackKind::DataPack
        )
}

fn browse_source(kind: PackKind, picked: SourceId, java: Option<&Target>) -> SourceId {
    match java {
        Some(target) if target.takes(kind) => SourceId::Modrinth,
        Some(_) => SourceId::Horizon,
        None if switches(kind, false) => picked,
        None => source_for(kind),
    }
}

fn can_install_in(kind: PackKind, source: SourceId, java: Option<&Target>) -> bool {
    match (java, kind) {
        (Some(_), PackKind::Shader) => false,
        (Some(_), PackKind::Mod | PackKind::ResourcePack) => source != SourceId::Horizon,
        (Some(_), PackKind::Modpack | PackKind::DataPack) => source == SourceId::Horizon,
        (None, _) => true,
    }
}

fn reviews_resource_pack(kind: PackKind, source: SourceId, java: Option<&Target>) -> bool {
    java.is_none() && kind == PackKind::ResourcePack && source == SourceId::Modrinth
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn same_named_worlds_keep_distinct_data_pack_destinations(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let mut mods = Mods::new(cx);
            let first = Path::new("saves").join("New World");
            let second = Path::new("saves").join("New World (1)");
            mods.worlds = vec![
                (first.clone(), "New World".into()),
                (second.clone(), "New World".into()),
            ];

            mods.world = Some(second.clone());
            let (path, name) = mods.selected_world().expect("selected world");
            assert_eq!(path, &second);
            assert_eq!(name, "New World");
            assert_ne!(world_picker_folder(&first), world_picker_folder(path));
            assert!(world_picker_folder(path).contains("New World (1)"));
        });
    }

    #[test]
    fn stale_discovery_keeps_only_offers_for_the_still_installed_versions() {
        let entries = ["updated", "unchanged"].map(|id| serde_json::json!({
                "source": "modrinth", "kind": "resource-pack", "project": id, "slug": id,
                "title": id, "version": if id == "updated" { "v2" } else { "v1" },
                "number": "1", "file": format!("resourcepacks/{id}.zip"), "checksum": "", "page": ""
        }));
        let installed: installed::Installed =
            serde_json::from_value(serde_json::json!({ "entries": entries })).unwrap();
        let found = ["updated", "unchanged", "removed"]
            .into_iter()
            .map(|id| {
                (
                    (SourceId::Modrinth, id.into()),
                    UpdateOffer {
                        installed_version: "v1".into(),
                        number: "2".into(),
                    },
                )
            })
            .collect();
        assert_eq!(
            current_offers(found, &installed),
            HashMap::from([((SourceId::Modrinth, "unchanged".into()), "2".into())])
        );
    }

    #[test]
    fn modrinth_shader_install_names_the_shader_packs_menu() {
        let shader = pack_install_notice(PackKind::Shader, SourceId::Modrinth, "Example");
        assert!(shader.contains("Options > Video Settings > Shaders > Shader Packs"));
        let resource = pack_install_notice(PackKind::ResourcePack, SourceId::Modrinth, "Example");
        assert!(resource.contains("Options > Resource Packs"));
    }

    fn project(kind: PackKind, source: SourceId) -> Project {
        Project {
            source,
            id: "example".into(),
            slug: "example".into(),
            title: "Example".into(),
            summary: String::new(),
            author: String::new(),
            downloads: 0,
            page: String::new(),
            icon: None,
            license: String::new(),
            kind,
            game_versions: Vec::new(),
            loaders: Vec::new(),
            blocked: None,
        }
    }

    #[gpui::test]
    fn cleared_resource_pack_search_scans_with_the_submitted_filters(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            let mut mods = Mods::new(cx);
            mods.set_kind(PackKind::ResourcePack);
            assert_eq!(mods.source(), SourceId::Modrinth);
            mods.text.set("stone".into());
            mods.browse.filters.categories.push("realistic".into());
            mods.submitted = Some(mods.new_query());
            let mut rejected = project(mods.kind, mods.source());
            rejected.blocked = Some("Incompatible".into());
            mods.results = vec![rejected.clone(); PAGE];
            mods.next_offset = Some(PAGE);
            assert_eq!(mods.next_default_page(), None);

            mods.text.set(String::new());
            mods.submitted = Some(mods.new_query());
            assert_eq!(mods.next_default_page(), Some(PAGE));
            let submitted = mods.begin_page(PAGE).unwrap();
            mods.text.set("unsubmitted edit".into());
            mods.error = Some("connection interrupted".into());
            let offset = mods.submitted.as_ref().unwrap().offset;
            assert_eq!(mods.begin_page(offset), Some(submitted.clone()));
            assert!(submitted.text.is_empty());
            assert_eq!(submitted.filters.categories, ["realistic"]);
            assert!(mods.error.is_none() && mods.searching);

            mods.results.resize(2 * PAGE, rejected.clone());
            mods.next_offset = Some(2 * PAGE);
            assert_eq!(mods.next_default_page(), Some(2 * PAGE));
            mods.results.resize(DEFAULT_SCAN_PAGES * PAGE, rejected);
            mods.next_offset = Some(DEFAULT_SCAN_PAGES * PAGE);
            assert_eq!(mods.next_default_page(), None);
            assert_eq!(mods.next_offset, Some(DEFAULT_SCAN_PAGES * PAGE));
            mods.results.truncate(PAGE);
            mods.next_offset = Some(2 * PAGE);
            mods.results.push(project(mods.kind, mods.source()));
            assert_eq!(mods.next_default_page(), None);
            mods.reset_browse_context();
            assert!(mods.begin_page(PAGE).is_none());
        });
    }

    #[gpui::test]
    fn pack_scan_respects_show_everything_and_instance_routing(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let mut mods = Mods::new(cx);
            for kind in [PackKind::DataPack, PackKind::ResourcePack] {
                mods.set_kind(kind);
                mods.submitted = Some(mods.new_query());
                assert!(mods.filtered_pack_defaults());
                mods.toggle_everything();
                mods.submitted = Some(mods.new_query());
                assert!(!mods.filtered_pack_defaults());
            }
            for kind in [PackKind::Mod, PackKind::Shader, PackKind::Modpack] {
                mods.set_kind(kind);
                mods.submitted = Some(mods.new_query());
                assert!(!mods.filtered_pack_defaults());
            }
            mods.set_kind(PackKind::ResourcePack);
            mods.set_target(
                Some("java".into()),
                None,
                Some(Target {
                    minecraft: "1.21.1".into(),
                    loader: Some("fabric".into()),
                }),
            );
            mods.submitted = Some(mods.new_query());
            assert!(!mods.filtered_pack_defaults());
        });
    }

    #[gpui::test]
    fn palette_install_is_independent_of_the_active_pack_tab(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let mut mods = Mods::new(cx);
            mods.set_kind(PackKind::ResourcePack);
            let registry_mod = Box::new(project(PackKind::Mod, SourceId::Horizon));
            let browse = InstallRequest::Browse(registry_mod.clone());
            let palette = InstallRequest::Palette(registry_mod);
            assert_eq!(mods.source(), SourceId::Modrinth);
            assert_eq!(mods.install_kind(&browse), None);
            let kind = mods.install_kind(&palette).unwrap();
            let (source, _) = palette.item();
            assert!(can_install_in(kind, source, None));

            let java = Target {
                minecraft: "1.21.1".into(),
                loader: Some("fabric".into()),
            };
            assert!(!can_install_in(kind, source, Some(&java)));
        });
    }

    #[gpui::test]
    fn empty_results_after_switching_instances_drop_the_cached_card(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let mut mods = Mods::new(cx);
            mods.set_target(Some("horizon".into()), Some(PathBuf::from("horizon")), None);
            mods.set_kind(PackKind::Shader);
            mods.source = SourceId::Modrinth;
            let mut project = project(PackKind::Shader, SourceId::Modrinth);
            project.loaders.push("iris".into());
            mods.results.push(project);
            mods.searched = true;
            assert!(mods.picked_project().is_some());
            mods.results.clear();
            assert!(mods.picked_project().is_some());

            let java = Target {
                minecraft: "1.21.1".into(),
                loader: Some("fabric".into()),
            };
            mods.set_target(
                Some("fabric".into()),
                Some(PathBuf::from("fabric")),
                Some(java.clone()),
            );
            mods.searched = true;
            assert!(mods.picked_project().is_none());
            assert!(mods.shown.is_none());
            assert!(!can_install_in(
                PackKind::Shader,
                SourceId::Modrinth,
                Some(&java)
            ));
            assert!(can_install_in(
                PackKind::ResourcePack,
                SourceId::Modrinth,
                Some(&java)
            ));
            assert!(can_install_in(PackKind::Shader, SourceId::Modrinth, None));
        });
    }

    #[gpui::test]
    fn pack_tabs_show_source_controls_for_the_right_context(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let mut mods = Mods::new(cx);
            let java = Target {
                minecraft: "1.21.1".into(),
                loader: Some("fabric".into()),
            };
            for (kind, default, switch, everything, java_source) in [
                (
                    PackKind::Mod,
                    SourceId::Horizon,
                    false,
                    false,
                    SourceId::Modrinth,
                ),
                (
                    PackKind::Modpack,
                    SourceId::Horizon,
                    false,
                    false,
                    SourceId::Horizon,
                ),
                (
                    PackKind::ResourcePack,
                    SourceId::Modrinth,
                    true,
                    true,
                    SourceId::Modrinth,
                ),
                (
                    PackKind::Shader,
                    SourceId::Horizon,
                    true,
                    false,
                    SourceId::Horizon,
                ),
                (
                    PackKind::DataPack,
                    SourceId::Modrinth,
                    true,
                    true,
                    SourceId::Horizon,
                ),
            ] {
                assert_eq!(source_for(kind), default);
                mods.kind = kind;
                for picked in [SourceId::Horizon, SourceId::Modrinth] {
                    mods.source = picked;
                    mods.java = None;
                    assert_eq!(mods.source(), if switch { picked } else { default });
                    assert_eq!(
                        switches(kind, mods.java.is_some()),
                        switch,
                        "{kind:?} {picked:?}"
                    );
                    assert_eq!(
                        mods.can_show_everything(),
                        everything && picked == SourceId::Modrinth,
                        "{kind:?} {picked:?}"
                    );

                    mods.java = Some(java.clone());
                    assert_eq!(mods.source(), java_source);
                    assert!(
                        !switches(kind, mods.java.is_some()),
                        "Java {kind:?} {picked:?}"
                    );
                    assert!(!mods.can_show_everything(), "Java {kind:?} {picked:?}");
                }
            }
        });
    }

    #[gpui::test]
    fn resetting_browse_keeps_the_drawer_open_and_clears_filters(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let mut mods = Mods::new(cx);
            mods.browse.show_filters = true;
            mods.browse.filters.categories.push("decoration".into());

            mods.reset_browse_context();

            assert!(mods.browse.show_filters);
            assert_eq!(mods.browse.filters.picked(), 0);
        });
    }

    #[gpui::test]
    fn turning_off_show_everything_removes_filters_that_become_unavailable(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            let mut mods = Mods::new(cx);
            for kind in [PackKind::ResourcePack, PackKind::DataPack] {
                mods.kind = kind;
                mods.everything = true;
                let filters = &mut mods.browse.filters;
                filters.game_version = Some("1.21.1".into());
                filters.categories = vec!["decoration".into()];
                if kind == PackKind::ResourcePack {
                    filters.categories.push("modded".into());
                }

                mods.toggle_everything();

                assert!(!mods.everything);
                let filters = &mods.browse.filters;
                assert_eq!(filters.game_version, None);
                assert!(!filters.other_version());
                assert_eq!(filters.categories, ["decoration"]);
                assert_eq!(filters.picked(), 1);
            }
        });
    }

    #[test]
    fn counts_and_checksums_read_short() {
        assert_eq!(count(999), "999");
        assert_eq!(count(1_500), "1.5k");
        assert_eq!(count(51_404_735), "51.4M");
        let sum = format!("SHA-512:{}", "ab".repeat(64));
        assert_eq!(short_sum(&sum), "SHA-512 abababababab…abababab");
        assert_eq!(short_sum("SHA-1:abcd"), "SHA-1:abcd");
    }

    #[test]
    fn show_everything_lists_what_doesnt_run() {
        let project = |id: &str| Project {
            source: SourceId::Modrinth,
            id: id.into(),
            slug: id.into(),
            title: id.into(),
            summary: String::new(),
            author: String::new(),
            downloads: 0,
            page: String::new(),
            icon: None,
            license: String::new(),
            kind: PackKind::Mod,
            game_versions: vec!["26.3".into()],
            loaders: vec!["fabric".into()],
            blocked: None,
        };
        let results: Vec<Project> = ["fabric-api", "ferrite-core", "sodium", "fresh"]
            .into_iter()
            .map(project)
            .collect();
        // The verdicts the jar checks gave; `fresh` is still being checked.
        let support = |p: &Project| match p.id.as_str() {
            "fabric-api" | "sodium" => Support::No("Uses mixins.".into()),
            "ferrite-core" => Support::Works,
            _ => Support::Partial("Checking its jar…".into()),
        };
        assert_eq!(shown(&results, false, support), [1, 3]);
        assert_eq!(shown(&results, true, support), [0, 1, 2, 3]);
    }
}
