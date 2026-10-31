//! "Add from GitHub" on the Mods page: a link to a repository, release, tag, branch or commit,
//! resolved to an exact release file or commit (`horizon_launcher::github_mods`), shown with the
//! plain warning and a link to the exact source, and installed only after the player says they
//! trust it. Mods installed that way list where they came from, with "Check for updates".
use super::line_edit::{self, Key, LineEdit};
use super::*;
use horizon_launcher::github_mods::{self, Origin, Packed, Resolved};
use std::path::{Path, PathBuf};

enum Event {
    /// A link (or an update check for a mod id) resolved.
    Resolved(Result<Resolved, String>, Option<String>),
    /// The mods of a source, fetched and checked, when there is a choice to make.
    Fetched(Resolved, Result<Vec<Packed>, String>),
    Installed(Result<String, String>),
}

enum Stage {
    Idle,
    Working(String),
    /// The warning and the source link, before anything is downloaded. `only`: the mod an update
    /// is for.
    Review(Resolved, Option<String>),
    Pick(Resolved, Vec<Packed>),
}

pub(super) struct Github {
    text: LineEdit,
    focus: FocusHandle,
    focused: bool,
    stage: Stage,
    message: Option<Result<String, String>>,
    /// The mods installed from GitHub in `home`, and what the last update check found for each.
    home: Option<PathBuf>,
    origins: Vec<Origin>,
    newer: std::collections::HashMap<String, Resolved>,
    confirm_remove: Option<String>,
    tx: Sender<Event>,
    rx: Receiver<Event>,
}

impl Github {
    pub(super) fn new(cx: &mut App) -> Self {
        let (tx, rx) = channel();
        Self {
            text: LineEdit::new(String::new(), 300, 40),
            focus: cx.focus_handle().tab_index(0).tab_stop(true),
            focused: false,
            stage: Stage::Idle,
            message: None,
            home: None,
            origins: Vec::new(),
            newer: Default::default(),
            confirm_remove: None,
            tx,
            rx,
        }
    }

    pub(super) fn note_focus(&mut self, window: &Window) {
        self.focused = self.focus.is_focused(window);
    }

    fn busy(&self) -> bool {
        matches!(self.stage, Stage::Working(_))
    }

    fn reread(&mut self, home: &Path) {
        self.home = Some(home.to_owned());
        self.origins = github_mods::origins(home);
    }
}

/// Installs the chosen mods of `mods` and says what was installed.
fn finish(
    home: &Path,
    found: &Resolved,
    mods: &[Packed],
    only: Option<&str>,
    all: bool,
) -> Result<String, String> {
    let done = github_mods::choose(mods, only, all)?
        .into_iter()
        .map(|p| github_mods::install(home, found, p))
        .collect::<Result<Vec<_>, _>>()?;
    let names: Vec<String> = done
        .iter()
        .map(|o| format!("{} {}", o.id, o.version))
        .collect();
    Ok(format!(
        "Installed {} from GitHub ({}). Build it from Settings > Native mods.",
        names.join(", "),
        found.label()
    ))
}

impl Launcher {
    pub(super) fn github_tick(&mut self) -> bool {
        let mut changed = false;
        while let Ok(event) = self.github.rx.try_recv() {
            changed = true;
            match event {
                Event::Resolved(Err(e), _) => {
                    self.github.stage = Stage::Idle;
                    self.github.message = Some(Err(e));
                }
                Event::Resolved(Ok(found), None) => self.github.stage = Stage::Review(found, None),
                Event::Resolved(Ok(found), Some(id)) => {
                    self.github.stage = Stage::Idle;
                    let have = self.github.origins.iter().find(|o| o.id == id);
                    if have.is_some_and(|o| github_mods::is_update(o, &found)) {
                        self.github.newer.insert(id, found);
                    } else {
                        self.github.message = Some(Ok(format!("{id} is up to date.")));
                    }
                }
                Event::Fetched(found, Ok(mods)) => self.github.stage = Stage::Pick(found, mods),
                Event::Fetched(_, Err(e)) => {
                    self.github.stage = Stage::Idle;
                    self.github.message = Some(Err(e));
                }
                Event::Installed(result) => {
                    self.github.stage = Stage::Idle;
                    if result.is_ok() {
                        self.github.text.set(String::new());
                        self.github.newer.clear();
                        // Installing a native mod turns Native mods on, so Settings offers Build.
                        self.settings.native_mods = true;
                        self.settings.native_mods_noticed = true;
                        self.settings.save();
                    }
                    self.github.message = Some(result);
                    let home = self.mods.home();
                    self.github.reread(&home);
                }
            }
        }
        changed
    }

    /// Resolves what the field holds, through GitHub's API on a thread.
    fn github_resolve(&mut self, link: github_mods::Link, update_of: Option<String>) {
        if self.github.busy() {
            return;
        }
        self.github.message = None;
        self.github.stage =
            Stage::Working(format!("Asking GitHub about {}/{}…", link.owner, link.repo));
        let tx = self.github.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Event::Resolved(
                github_mods::resolve(&link, &github_mods::api),
                update_of,
            ));
        });
    }

    fn github_add(&mut self) {
        match github_mods::parse(self.github.text.text()) {
            Ok(link) => self.github_resolve(link, None),
            Err(e) => self.github.message = Some(Err(e)),
        }
    }

    /// Downloads and checks what the player agreed to; installs it unless there is a choice.
    fn github_install(&mut self, found: Resolved, only: Option<String>) {
        self.github.stage = Stage::Working(format!("Downloading {}…", found.label()));
        let tx = self.github.tx.clone();
        let home = self.mods.home();
        std::thread::spawn(move || {
            let fetched = github_mods::fetch(&found);
            let result = match fetched {
                Ok(mods) if mods.len() == 1 || only.is_some() => {
                    let done = finish(&home, &found, &mods, only.as_deref(), false);
                    github_mods::discard(&found);
                    Event::Installed(done)
                }
                other => Event::Fetched(found, other),
            };
            let _ = tx.send(result);
        });
    }

    fn github_pick(&mut self, found: Resolved, mods: Vec<Packed>, id: Option<String>) {
        let home = self.mods.home();
        let done = finish(&home, &found, &mods, id.as_deref(), id.is_none());
        github_mods::discard(&found);
        let _ = self.github.tx.send(Event::Installed(done));
        self.github.stage = Stage::Working("Installing…".into());
    }

    fn github_field(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = self.theme;
        let focused = self.github.focused;
        let field = div()
            .id("github-link")
            .track_focus(&self.github.focus)
            .flex()
            .items_center()
            .gap(px(10.))
            .w(px(460.))
            .h(px(theme::FIELD + 4.))
            .px(px(12.))
            .overflow_hidden()
            .border_2()
            .border_color(if focused { t.accent } else { t.outline })
            .bg(t.well)
            .text_size(px(theme::UI.px))
            .cursor_text()
            .child(line_edit::content(
                &self.github.text,
                &t,
                focused,
                false,
                false,
                "https://github.com/owner/repo",
            ))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                window.focus(&this.github.focus);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                match line_edit::handle(&mut this.github.text, ev, cx) {
                    Key::Enter => this.github_add(),
                    Key::Escape => window.blur(),
                    Key::Pass => return,
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }));
        line_edit::mouse(field, cx, |this| &mut this.github.text)
    }

    /// The Add from GitHub panel of the Mods tab, and the mods installed that way.
    pub(super) fn github_panel(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let home = self.mods.home();
        if self.github.home.as_deref() != Some(home.as_path()) {
            self.github.reread(&home);
            self.github.newer.clear();
        }
        let busy = self.github.busy();
        let mut add = button("github-add", "Add from GitHub", Kind::Secondary, &t);
        add = match busy {
            true => disabled(add),
            false => add.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.github_add();
                cx.notify();
            })),
        };
        let mut panel = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(caps("Add a native mod from GitHub").text_color(t.muted))
            .child(
                text(theme::CAPTION).text_color(t.muted).child(
                    "A link to a repository (its latest release), a release, a branch or tag, or a commit. \
                     The launcher installs one exact release file or commit, never a branch that moves.",
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(12.))
                    .child(self.github_field(cx))
                    .child(add),
            );
        match std::mem::replace(&mut self.github.stage, Stage::Idle) {
            Stage::Idle => {}
            Stage::Working(line) => {
                self.github.stage = Stage::Working(line.clone());
                panel = panel.child(text(theme::CAPTION).text_color(t.ink_soft).child(line));
            }
            Stage::Review(found, only) => {
                panel = panel.child(self.github_review(&found, only.clone(), cx));
                self.github.stage = Stage::Review(found, only);
            }
            Stage::Pick(found, mods) => {
                panel = panel.child(self.github_choice(&found, &mods, cx));
                self.github.stage = Stage::Pick(found, mods);
            }
        }
        match &self.github.message {
            Some(Ok(done)) => {
                panel = panel.child(plate(&t).p(px(12.)).text_color(t.ink).child(done.clone()))
            }
            Some(Err(e)) => panel = panel.child(notice(e.clone(), &t)),
            None => {}
        }
        for origin in self.github.origins.clone() {
            panel = panel.child(self.github_row(&origin, busy, cx));
        }
        panel
    }

    /// The warning, the exact source, and the only way forward: saying they trust it.
    fn github_review(&self, found: &Resolved, only: Option<String>, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let what = match (&found.asset, &found.content_only) {
            (Some(asset), _) => format!("The release file {} ({}).", asset.name, found.label()),
            (None, Some(file)) => format!(
                "The source of {}. Its release ships {file}, a content mod: installing those from GitHub is not supported yet.",
                found.label()
            ),
            (None, None) => format!("The source at commit {}.", found.commit),
        };
        let (url, open) = (found.source_url(), found.source_url());
        let install = found.clone();
        plate(&t)
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(16.))
            .text_size(px(theme::UI.px))
            .text_color(t.ink)
            .child(caps("Not reviewed by the Horizon registry").text_color(t.danger))
            .child(what)
            .child(
                "Native mods run with full access to this computer. Read the source before you install it.",
            )
            .child(text(theme::CAPTION).font_family(theme::MONO).text_color(t.ink_soft).child(url))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(
                        button("github-source", "Open the source", Kind::Secondary, &t)
                            .on_click(move |_, _, cx| cx.open_url(&open)),
                    )
                    .child(
                        button("github-trust", "I read it and trust it: install", Kind::Primary, &t)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.github_install(install.clone(), only.clone());
                                cx.notify();
                            })),
                    )
                    .child(
                        button("github-cancel", "Cancel", Kind::Ghost, &t).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.github.stage = Stage::Idle;
                                cx.notify();
                            },
                        )),
                    ),
            )
    }

    /// A source with several mods: one, or all.
    fn github_choice(&self, found: &Resolved, mods: &[Packed], cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let mut buttons = div().flex().flex_wrap().gap(px(8.));
        for (i, p) in mods.iter().enumerate() {
            let (found, all, id) = (found.clone(), mods.to_vec(), p.id.clone());
            buttons = buttons.child(
                button(
                    SharedString::from(format!("github-pick-{i}")),
                    format!("{} {}", p.id, p.version),
                    Kind::Secondary,
                    &t,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.github_pick(found.clone(), all.clone(), Some(id.clone()));
                    cx.notify();
                })),
            );
        }
        let (every, listed) = (found.clone(), mods.to_vec());
        buttons = buttons.child(
            button("github-pick-all", "Install all", Kind::Primary, &t).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    this.github_pick(every.clone(), listed.clone(), None);
                    cx.notify();
                },
            )),
        );
        plate(&t)
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(16.))
            .text_size(px(theme::UI.px))
            .text_color(t.ink)
            .child(format!(
                "{} holds {} mods. Install which?",
                found.name(),
                mods.len()
            ))
            .child(buttons)
    }

    /// One mod installed from GitHub: where from, check for updates, remove.
    fn github_row(&self, origin: &Origin, busy: bool, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let id = origin.id.clone();
        let mut right = div().flex().flex_wrap().items_center().gap(px(8.));
        let known = self.github.newer.get(&id).cloned();
        let main = match &known {
            Some(latest) => {
                let (latest, only) = (latest.clone(), id.clone());
                button(
                    SharedString::from(format!("github-update-{id}")),
                    format!("Update to {}", latest.label()),
                    Kind::Primary,
                    &t,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.github.stage = Stage::Review(latest.clone(), Some(only.clone()));
                    cx.notify();
                }))
            }
            None => {
                let (link, only) = (origin.follows(), id.clone());
                let check = button(
                    SharedString::from(format!("github-check-{id}")),
                    "Check for updates",
                    Kind::Secondary,
                    &t,
                );
                match busy {
                    true => disabled(check),
                    false => check.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.github_resolve(link.clone(), Some(only.clone()));
                        cx.notify();
                    })),
                }
            }
        };
        right = right.child(main);
        let url = origin.source_url();
        right = right.child(
            button(
                SharedString::from(format!("github-src-{id}")),
                "Source",
                Kind::Ghost,
                &t,
            )
            .on_click(move |_, _, cx| cx.open_url(&url)),
        );
        let confirming = self.github.confirm_remove.as_deref() == Some(id.as_str());
        let target = id.clone();
        right = right.child(
            button(
                SharedString::from(format!("github-remove-{id}")),
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
                if this.github.confirm_remove.as_ref() == Some(&target) {
                    let home = this.mods.home();
                    this.github.message = Some(
                        github_mods::remove(&home, &target).map(|_| format!("Removed {target}.")),
                    );
                    this.github.reread(&home);
                    this.github.confirm_remove = None;
                } else {
                    this.github.confirm_remove = Some(target.clone());
                }
                cx.notify();
            })),
        );
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(format!("{} {}", origin.id, origin.version))
                    .child(
                        text(theme::CAPTION)
                            .text_color(t.ink_soft)
                            .child(format!("from GitHub: {}", origin.label())),
                    )
                    .child(
                        text(theme::CAPTION)
                            .font_family(theme::MONO)
                            .text_color(t.muted)
                            .child(format!("sha256 {}", origin.sha256)),
                    ),
            )
            .child(right)
    }
}
