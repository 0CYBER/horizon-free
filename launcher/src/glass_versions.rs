//! The Versions page in the glass look: the mockup's `VersionsView`. Each Nightly on its own glass
//! card (its name, Latest on the first, when it came out and which instances play it), Install
//! or a bin on the right, and Install new versions automatically by the page's title.
//!
//! Not in the mockup, kept in its patterns: Use and In use (the version instances without their
//! own play), the space versions take, Import a game build, the native and imported builds,
//! install progress and errors, the confirmation before a removal.

use super::*;
use crate::glass_home_tabs::PatchNotes;
use crate::glass_ui::{self as ui, Tone};
use horizon_theme::glass::{ink, radius};

/// A release's day as the mockup writes it ("Nov 1"), with its time in UTC when known.
pub(super) fn published(date: &str, time: &str) -> String {
    let day = horizon_launcher::nightly::day(date);
    if time.is_empty() {
        day
    } else {
        trf("{day}, {time} UTC", &[("day", &day), ("time", time)])
    }
}

/// A version's state beside its name: a dot and a word, 13 px at 70 % (`Latest`).
fn badge(tone: Tone, label: &str) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .child(ui::dot(tone))
        .child(ui::text(13., FontWeight::NORMAL, ink::BODY).child(tr(label).to_string()))
}

/// The compact title and commit line for a published Nightly.
///
/// A nightly's short commit, which only a tooltip shows; empty for any other version.
fn commit_of(tag: &str) -> String {
    horizon_launcher::nightly::label(tag)
        .and_then(|label| {
            label
                .split_once(" · ")
                .map(|(_, commit)| commit.chars().take(7).collect())
        })
        .unwrap_or_default()
}

/// The release title from GitHub is an implementation-facing ISO date and commit. Players get
/// "Nov 7 update" and the day it came out; the commit is in a tooltip on that line.
fn nightly_card_text(release: &Release, same_day: bool) -> Option<(String, String)> {
    release.nightly.as_ref()?;
    // Two updates on one day are told apart by their time in the title, which then says all
    // the line would: the line keeps only what the card adds.
    let date = published(&release.date, if same_day { &release.time } else { "" });
    let title = trf("{date} update", &[("date", &date)]);
    let line = match same_day {
        true => String::new(),
        false => published(&release.date, &release.time),
    };
    Some((title, line))
}

impl Launcher {
    /// Version `tag` as the Versions page titles it ("Nov 11 update", with its time when another
    /// update came out the same day, [`horizon_launcher::nightly::same_day_time`]), for any screen
    /// that names it; the tag itself otherwise.
    pub(crate) fn version_name(&self, tag: &str) -> String {
        match horizon_launcher::nightly::published(tag) {
            Some((date, time)) => trf(
                "{date} update",
                &[("date", &published(&date, &time.unwrap_or_default()))],
            ),
            None => tag.to_owned(),
        }
    }

    /// The action Versions offers for this build on the instance Play starts.
    pub(super) fn use_version_action(&self, tag: &str) -> Option<(String, String)> {
        let instance = self.selected_instance().filter(|i| i.java.is_none())?;
        let latest = horizon_launcher::nightly::latest_for(
            instance.channel,
            &self.installed,
            self.settings.selected.as_deref(),
        );
        let follows = latest.as_deref() == Some(tag);
        let pin = (!follows).then_some(tag);
        (instance.version.as_deref() != pin).then(|| {
            let tip = if follows {
                trf(
                    "Play the latest in {instance}. It moves to each new one.",
                    &[("instance", &instance.name)],
                )
            } else {
                trf(
                    "Keep {instance} on this version. To follow new ones again, choose Latest in its settings, Installation.",
                    &[("instance", &instance.name)],
                )
            };
            (instance.id.clone(), tip)
        })
    }

    /// The names of the Horizon instances that play `tag`.
    pub(crate) fn instances_using(&self, tag: &str) -> Vec<&str> {
        self.instances
            .list
            .iter()
            .filter(|i| i.java.is_none() && self.version_of(i).map(String::as_str) == Some(tag))
            .map(|i| i.name.as_str())
            .collect()
    }

    /// The glass Versions page; `on_disk` and `used` as `versions_page` measured them.
    pub(super) fn glass_versions(
        &self,
        on_disk: &dyn Fn(&String) -> Option<u64>,
        used: u64,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = self.theme;
        let auto = switch("versions-auto-install", self.settings.auto_install(), &t).on_click(
            cx.listener(|this, _: &ClickEvent, _, cx| {
                this.settings.install_new_versions = Some(!this.settings.auto_install());
                this.save_settings();
                if this.settings.auto_install() {
                    this.install_update();
                }
                cx.notify();
            }),
        );
        let wide = self.window_width >= crate::glass_home::SIDE_BY_SIDE;
        let page_title = ui::page_head(
            tr("Versions"),
            tr(
                "Horizon's Nightly builds, newest first. Your worlds live in the game \
                folder, so removing a version never touches them.",
            ),
        )
        .mb_0();
        let auto_switch = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(12.))
            .child(
                ui::text(14., FontWeight::NORMAL, 0.85)
                    .child(tr("Install new versions automatically")),
            )
            .child(auto);
        let head = if wide {
            div()
                .flex()
                .flex_row()
                .items_end()
                .justify_between()
                .gap(px(20.))
                .child(page_title)
                .child(auto_switch)
                .mb(px(28.))
        } else {
            div()
                .flex()
                .flex_col()
                .items_start()
                .gap(px(16.))
                .child(page_title.w_full())
                .child(auto_switch)
                .mb(px(28.))
        };
        // The space they take and Import a game build (not in the mockup): a quiet line above
        // the list.
        let import: AnyElement = if self.importing_build {
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .w(px(180.))
                .child(progress_bar(None, &t))
                .child(ui::text(13., FontWeight::NORMAL, ink::QUIET).child(tr("Importing…")))
                .into_any_element()
        } else {
            ui::quiet("import-build", tr("Import a game build…"), 36., &t)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.import_build(cx)))
                .into_any_element()
        };
        let sponsor = (!self.links.sponsor.is_empty()).then(|| {
            let url = self.links.sponsor.clone();
            crate::glass::plate(radius::PANEL)
                .flex()
                .items_center()
                .justify_between()
                .gap(px(16.))
                .p(px(24.))
                .mb(px(24.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(
                            ui::text(16., FontWeight::SEMIBOLD, ink::FULL)
                                .child(tr("Support the work below")),
                        )
                        .child(ui::text(14., FontWeight::NORMAL, ink::BODY).child(tr(
                            "Horizon is built in the open. Sponsors fund the next release.",
                        ))),
                )
                .child(
                    ui::quiet_icon("sponsor", None, tr("Sponsor"), 36., &t)
                        .on_click(move |_: &ClickEvent, _, cx| cx.open_url(&url)),
                )
        });
        let mut page = div().flex().flex_col().child(head).children(sponsor).child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(16.))
                .mb(px(10.))
                .child(
                    ui::text(13., FontWeight::NORMAL, ink::QUIET).when(used > 0, |d| {
                        d.child(trf(
                            "Installed versions use {size}.",
                            &[("size", &megabytes(used))],
                        ))
                    }),
                )
                .child(import),
        );
        page = page
            .children(self.import_error.clone().map(|e| notice(e, &t).mb(px(10.))))
            .children(self.glass_local_builds(
                "Native mod builds",
                &native_mods::is_native,
                None,
                on_disk,
                cx,
            ))
            .children(self.glass_local_builds(
                "Imported builds",
                &build_import::is_imported,
                Some(tr(
                    "Unsigned: you picked these programs yourself, and Horizon can't \
                    check where they came from. Updates never replace them.",
                )),
                on_disk,
                cx,
            ));
        // Installed versions the release list doesn't hold (it failed, came back empty, or no
        // longer offers them) stay listed with their own Use and Delete.
        let listed: Vec<&str> = match &self.releases {
            Load::Ready(r) => r.iter().map(|r| r.tag.as_str()).collect(),
            _ => Vec::new(),
        };
        if !matches!(self.releases, Load::Loading) {
            page = page.children(self.glass_local_builds(
                "Installed here",
                &|tag| {
                    !native_mods::is_native(tag)
                        && !build_import::is_imported(tag)
                        && !listed.contains(&tag)
                },
                Some(tr(
                    "Not in Horizon's version list right now. You can still play or remove them.",
                )),
                on_disk,
                cx,
            ));
        }
        // The list's own states sit in a card, readable on any sky; a failure says it plainly.
        let state_card = |text_: String, retry: bool, cx: &mut Context<Self>| {
            crate::glass::plate(radius::PANEL)
                .flex()
                .flex_col()
                .items_start()
                .gap(px(12.))
                .p(px(20.))
                .child(ui::text(14., FontWeight::NORMAL, ink::NOTE).child(text_))
                .when(retry, |d| {
                    d.child(button("retry", "Try again", Kind::Secondary, &t).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.fetch_releases();
                            cx.notify();
                        }),
                    ))
                })
        };
        let releases = match &self.releases {
            Load::Loading => {
                return page.child(state_card(tr("Loading versions…").into(), false, cx))
            }
            Load::Failed(e) => {
                // A lost connection in plain words; anything else after what failed.
                let said = match e.starts_with("No connection") {
                    true => download_error(e),
                    false => trf("Couldn't load Horizon's versions. {error}", &[("error", e)]),
                };
                return page.child(state_card(said, true, cx));
            }
            Load::Ready(r) if r.is_empty() => {
                return page.child(state_card(
                    tr("Horizon's version list came back empty.").into(),
                    true,
                    cx,
                ));
            }
            Load::Ready(r) => r,
        };
        let keep = |i: usize| {
            let tag = &releases[i].tag;
            self.installing.as_ref().is_some_and(|(t, _)| t == tag)
                || self.install_error.as_ref().is_some_and(|(t, _)| t == tag)
                || self.confirm_remove.as_ref() == Some(tag)
                || self.removing.contains(tag)
        };
        let list = self.long_list.clone().column(
            &self.content_scroll.handle,
            releases.len(),
            10.,
            keep,
            |i| {
                let release = releases[i].clone();
                let same_day = horizon_launcher::nightly::same_day_time(&release.tag).is_some();
                let (title, mut line) =
                    nightly_card_text(&release, same_day).unwrap_or_else(|| {
                        let title = release
                            .name
                            .split_once(": ")
                            .map(|(_, rest)| rest.to_string())
                            .unwrap_or_else(|| release.name.clone());
                        let title = if title.is_empty() {
                            release.tag.clone()
                        } else {
                            title
                        };
                        (title, published(&release.date, &release.time))
                    });
                let right = self.glass_release_action(&release, cx);
                let notes = self.glass_whats_new(&release, i == 0, cx);
                let card = self
                    .glass_version_card(&release.tag, title, &mut line, i == 0, on_disk, right)
                    .children(notes);
                if i == 1 {
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .child(
                            ui::text(13., FontWeight::NORMAL, ink::QUIET)
                                .child(tr("Older versions")),
                        )
                        .child(card)
                } else {
                    card
                }
            },
        );
        page.child(list)
    }

    /// One version's card: its name (Latest on the first, In use on the one in use), `line`
    /// with who plays it, and `right`.
    fn glass_version_card(
        &self,
        tag: &str,
        title: String,
        line: &mut String,
        latest: bool,
        on_disk: &dyn Fn(&String) -> Option<u64>,
        right: AnyElement,
    ) -> Div {
        let t = self.theme;
        let look = ui::look();
        let installed = self.installed.iter().any(|v| v == tag);
        let in_use = self.selected().map(String::as_str) == Some(tag);
        let players = self.instances_using(tag);
        let mut add = |part: String| {
            if !line.is_empty() {
                line.push_str(" · ");
            }
            line.push_str(&part);
        };
        if !players.is_empty() {
            // Two names read at a glance; more become a count, so the line stays one line.
            add(match players.as_slice() {
                [first, _, _, ..] => trf(
                    "Used by {name} and {count} other instances",
                    &[("name", first), ("count", &(players.len() - 1).to_string())],
                ),
                _ => trf("Used by {names}", &[("names", &players.join(", "))]),
            });
        } else if installed {
            // The Installed badge says it is here; the line says nothing plays it yet.
            add(tr("No instance plays it").to_string());
        }
        if let Some(n) = on_disk(&tag.to_string()) {
            add(trf("{size} on disk", &[("size", &megabytes(n))]));
        }
        let error = self
            .install_error
            .as_ref()
            .filter(|(e, _)| e == tag)
            .map(|(_, e)| download_error(e));
        crate::glass::plate(radius::PANEL)
            .flex()
            .flex_col()
            .gap(px(12.))
            .p(px(20.))
            .child(
                div()
                    .relative()
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(px(2.))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(10.))
                                    .child(
                                        ui::text(16., FontWeight::SEMIBOLD, ink::FULL)
                                            .line_height(px(24.))
                                            .truncate()
                                            .child(title),
                                    )
                                    .when(latest, |d| {
                                        d.child(badge(Tone::Accent(look.accent_soft), "Latest"))
                                    })
                                    .when(in_use, |d| d.child(badge(Tone::Live, "In use")))
                                    .when(installed && !in_use, |d| {
                                        d.child(badge(Tone::Off, "Installed"))
                                    }),
                            )
                            // No empty line under a title that already says it all.
                            .children((!line.is_empty()).then(|| {
                                let commit = commit_of(tag);
                                ui::text(13., FontWeight::NORMAL, 0.6)
                                    .id(SharedString::from(format!("version-line-{tag}")))
                                    .line_height(px(19.5))
                                    .truncate()
                                    .when(!commit.is_empty(), |d| {
                                        d.tooltip(ui::tip(trf(
                                            "Build {commit}",
                                            &[("commit", &commit)],
                                        )))
                                    })
                                    .child(line.clone())
                            })),
                    )
                    .child(div().relative().flex().flex_none().child(right)),
            )
            .children(error.map(|e| notice(e, &t)))
    }

    /// What a release's card offers on its right: its install's progress, Install, or Use and
    /// the bin once installed.
    fn glass_release_action(&self, release: &Release, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        let tag = release.tag.clone();
        if let Some((_, p)) = self.installing.as_ref().filter(|(t, _)| *t == tag) {
            return self
                .glass_progress(Some(*p), tr("Installing"))
                .into_any_element();
        }
        if self.removing.contains(&tag) {
            return self
                .glass_progress(None, tr("Removing…"))
                .into_any_element();
        }
        if self.installed.contains(&tag) {
            if backend::needs_game_architecture_install(&tag, release) {
                let b = ui::quiet(
                    SharedString::from(format!("install-selected-architecture-{tag}")),
                    tr("Install selected build"),
                    36.,
                    &t,
                );
                if self.installing.is_some() {
                    return b.opacity(0.4).cursor_default().into_any_element();
                }
                let release = release.clone();
                return b
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.install(&release);
                        cx.notify();
                    }))
                    .into_any_element();
            }
            return self.glass_version_actions(&tag, cx).into_any_element();
        }
        if release.asset.is_none() {
            let (line, download) = horizon_launcher::refusals::no_build_card();
            let line = ui::text(13., FontWeight::NORMAL, ink::QUIET).child(line);
            return match download {
                Some(url) => line
                    .id(SharedString::from(format!("no-build-{tag}")))
                    .cursor_pointer()
                    .on_click(move |_: &ClickEvent, _, cx| cx.open_url(&url))
                    .into_any_element(),
                None => line.into_any_element(),
            };
        }
        // After a failed attempt the card says to try again: the button says so too.
        let failed = self.install_error.as_ref().is_some_and(|(t, _)| *t == tag);
        let b = ui::quiet(
            SharedString::from(format!("install-{tag}")),
            if failed {
                tr("Try again")
            } else {
                tr("Install")
            },
            36.,
            &t,
        );
        if self.installing.is_some() {
            b.opacity(0.4).cursor_default().into_any_element()
        } else {
            let release = release.clone();
            b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.install(&release);
                cx.notify();
            }))
            .into_any_element()
        }
    }

    /// A bar with its word, 180 px: an install's progress, or a removal's.
    fn glass_progress(&self, fraction: Option<f32>, label: &str) -> Div {
        let t = self.theme;
        let word = match fraction {
            Some(p) => format!("{:.0}%", p * 100.),
            None => label.to_string(),
        };
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .w(px(180.))
            .child(progress_bar(fraction.map(|_| self.bar), &t))
            .child(
                ui::text(13., FontWeight::NORMAL, ink::QUIET)
                    .flex_none()
                    .child(word),
            )
    }

    /// An installed version's actions: Use (when it isn't the one in use) and the bin, which asks
    /// again before it removes.
    fn glass_version_actions(&self, tag: &str, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let confirming = self.confirm_remove.as_deref() == Some(tag);
        let (t1, t2, t3) = (tag.to_string(), tag.to_string(), tag.to_string());
        let mut d = div().flex().items_center().gap(px(8.));
        if confirming {
            return d
                .child(
                    button(format!("remove-cancel-{tag}"), "Cancel", Kind::Ghost, &t).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.confirm_remove = None;
                            cx.notify();
                        }),
                    ),
                )
                .child(
                    ui::quiet(
                        SharedString::from(format!("remove-{tag}")),
                        tr("Remove for good"),
                        36.,
                        &t,
                    )
                    .text_color(ui::danger())
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            this.remove(&t2);
                            cx.notify();
                        },
                    )),
                );
        }
        if let Some((id, tip)) = self.use_version_action(tag) {
            d = d.child(
                ui::quiet(SharedString::from(format!("use-{tag}")), tr("Use"), 36., &t)
                    .tooltip(ui::tip(tip))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if let Err(e) = this.use_version_for(&id, &t1) {
                            this.install_error = Some((t1.clone(), e));
                        }
                        cx.notify();
                    })),
            );
        }
        let bin = ui::icon_button(
            SharedString::from(format!("bin-{tag}")),
            "delete-02",
            36.,
            17.,
            radius::CONTROL,
            false,
            &t,
        );
        let users = self.instances_using(tag);
        d.child(if users.is_empty() {
            bin.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.confirm_remove = Some(t3.clone());
                cx.notify();
            }))
        } else {
            disabled(bin).tooltip(ui::tip(trf(
                if users.len() > 1 {
                    "In use by {names}. Pick another version for them first."
                } else {
                    "In use by {names}. Pick another version for it first."
                },
                &[(
                    "names",
                    &match users.as_slice() {
                        [first, _, _, ..] => trf(
                            "{name} and {count} other instances",
                            &[("name", first), ("count", &(users.len() - 1).to_string())],
                        ),
                        _ => users.join(", "),
                    },
                )],
            )))
        })
    }

    /// The installed builds of one kind made here, as cards under a small heading; None when
    /// there are none.
    fn glass_local_builds(
        &self,
        title: &str,
        is: &dyn Fn(&str) -> bool,
        notice_: Option<&str>,
        on_disk: &dyn Fn(&String) -> Option<u64>,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let tags: Vec<String> = self
            .installed
            .iter()
            .chain(&self.removing)
            .filter(|v| is(v))
            .cloned()
            .collect();
        (!tags.is_empty()).then(|| {
            let mut d = div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .mb(px(24.))
                .child(ui::card_heading(tr(title).to_string()))
                .children(notice_.map(|n| notice(n.to_string(), &self.theme)));
            for tag in tags {
                let mut line = Vec::new();
                if let Some(about) = native_mods::about(&tag) {
                    line.push(trf(
                        "Built on {base}",
                        &[("base", &self.version_label(&about.base))],
                    ));
                    // The build's own name stays in the details, for reports.
                    line.push(tag.clone());
                }
                if let Some(about) = build_import::about(&tag) {
                    let base = horizon_launcher::nightly::label(&about.base).unwrap_or(about.base);
                    line.push(trf("Game files from {base}", &[("base", &base)]));
                    line.push(about.file);
                }
                let mut line = line.join(" · ");
                let right = if self.removing.contains(&tag) {
                    self.glass_progress(None, tr("Removing…"))
                        .into_any_element()
                } else {
                    self.glass_version_actions(&tag, cx).into_any_element()
                };
                let title = if native_mods::is_native(&tag) {
                    self.version_label(&tag)
                } else {
                    horizon_launcher::nightly::label(&tag)
                        .or_else(|| build_import::label(&tag))
                        .unwrap_or_else(|| tag.trim_start_matches('v').to_string())
                };
                d = d.child(self.glass_version_card(&tag, title, &mut line, false, on_disk, right));
            }
            d
        })
    }
}

impl Launcher {
    /// A release's "Patch notes" toggle and, once opened, its notes under it (the release's
    /// own Markdown). Opened state is kept per release in `notes_open`.
    pub(super) fn notes_section(&self, release: &Release, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let open = self.notes_open.contains(&release.tag);
        let tag = release.tag.clone();
        let id = SharedString::from(format!("notes-{tag}"));
        let turn = |s: gpui::Svg| {
            s.with_transformation(gpui::Transformation::rotate(gpui::radians(if open {
                std::f32::consts::PI
            } else {
                0.
            })))
        };
        let (label, chevron) = if t.glass {
            (
                ui::text(14., FontWeight::MEDIUM, ink::BODY).child(tr("Patch notes")),
                turn(ui::icon("arrow-down-01", 16., ui::white(ink::NOTE))),
            )
        } else {
            (
                text(theme::UI)
                    .text_color(t.ink_soft)
                    .child(tr("Patch notes")),
                turn(blocks::icon("chevron-down", 16., t.muted)),
            )
        };
        let toggle = div()
            .id(id)
            .flex()
            .items_center()
            .gap(px(8.))
            .cursor_pointer()
            .child(label)
            .child(chevron)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                if !this.notes_open.remove(&tag) {
                    this.notes_open.insert(tag.clone());
                }
                cx.notify();
            }));
        let body = open.then(|| {
            div()
                .max_w(px(640.))
                .child(if release.body.trim().is_empty() {
                    text(theme::UI)
                        .text_color(t.muted)
                        .child(tr("No notes for this version."))
                        .into_any_element()
                } else {
                    let (notes, rows) = self.notes.notes(release);
                    notes
                        .column(&t, &rows, &self.content_scroll.handle)
                        .into_any_element()
                })
        });
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(toggle)
            .children(body)
    }
}

impl Launcher {
    /// The tag Home's "What's new" opens: the played build's, else the newest with notes.
    pub(super) fn whats_new_tag(&self) -> Option<String> {
        let Load::Ready(list) = &self.releases else {
            return None;
        };
        let has = |r: &&Release| crate::glass_home_tabs::patch_notes(&r.body).is_some();
        let played = self
            .selected()
            .filter(|_| self.selected_instance().is_some_and(|i| i.java.is_none()));
        list.iter()
            .filter(has)
            .find(|r| Some(&r.tag) == played)
            .or_else(|| list.iter().filter(has).find(|r| r.nightly.is_some()))
            .map(|r| r.tag.clone())
    }

    /// A card's "What's new" toggle and, opened, its notes by section. None without notes.
    fn glass_whats_new(
        &self,
        release: &Release,
        newest: bool,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let notes = crate::glass_home_tabs::patch_notes(&release.body)?;
        let open = match &self.whats_new_open {
            Some(set) => set.contains(&release.tag),
            None => newest,
        };
        let tag = release.tag.clone();
        let turn = ui::icon("arrow-down-01", 16., ui::white(ink::NOTE)).with_transformation(
            gpui::Transformation::rotate(gpui::radians(if open {
                std::f32::consts::PI
            } else {
                0.
            })),
        );
        let toggle = div()
            .id(SharedString::from(format!("whats-new-{tag}")))
            .flex()
            .items_center()
            .gap(px(8.))
            .cursor_pointer()
            .child(ui::text(14., FontWeight::MEDIUM, ink::BODY).child(tr("What's new")))
            .child(turn)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                let was_newest = matches!(&this.releases, Load::Ready(l) if l.first().is_some_and(|r| r.tag == tag));
                let set = this.whats_new_open.get_or_insert_with(|| {
                    if was_newest { std::iter::once(tag.clone()).collect() } else { Default::default() }
                });
                if !set.remove(&tag) {
                    set.insert(tag.clone());
                }
                cx.notify();
            }));
        let bullets = |items: Vec<String>| {
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .children(items.into_iter().map(|item| {
                    div()
                        .flex()
                        .gap(px(10.))
                        .child(
                            div()
                                .flex_none()
                                .mt(px(9.))
                                .size(px(5.))
                                .rounded_full()
                                .bg(ui::white(0.4)),
                        )
                        .child(
                            ui::text(14., FontWeight::NORMAL, ink::STRONG)
                                .line_height(px(21.))
                                .min_w_0()
                                .child(item),
                        )
                }))
        };
        let list = open.then(|| {
            let column = div().flex().flex_col().gap(px(14.)).max_w(px(640.));
            match notes {
                PatchNotes::ForPlayers(items) => column.child(bullets(items)),
                PatchNotes::Technical(sections) => {
                    let count: usize = sections.iter().map(|(_, items)| items.len()).sum();
                    let details_open = self.technical_open.contains(&release.tag);
                    let details_tag = release.tag.clone();
                    let many = sections.len() > 1;
                    column
                        .child(
                            ui::text(14., FontWeight::NORMAL, ink::STRONG)
                                .line_height(px(21.))
                                .child(if count == 1 {
                                    tr("Fixes and improvements to the game and mods: 1 change.")
                                        .to_string()
                                } else {
                                    trf(
                                        "Fixes and improvements to the game and mods: {count} changes.",
                                        &[("count", &count.to_string())],
                                    )
                                }),
                        )
                        .child(
                            div()
                                .id(SharedString::from(format!("technical-{}", release.tag)))
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .cursor_pointer()
                                .child(
                                    ui::text(13., FontWeight::MEDIUM, ink::QUIET)
                                        .child(tr("Technical details")),
                                )
                                .child(
                                    ui::icon("arrow-down-01", 16., ui::white(ink::NOTE))
                                        .with_transformation(gpui::Transformation::rotate(
                                            gpui::radians(if details_open {
                                                std::f32::consts::PI
                                            } else {
                                                0.
                                            }),
                                        )),
                                )
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    if !this.technical_open.remove(&details_tag) {
                                        this.technical_open.insert(details_tag.clone());
                                    }
                                    cx.notify();
                                })),
                        )
                        .children(details_open.then(|| {
                            div().flex().flex_col().gap(px(14.)).children(
                                sections.into_iter().map(|(section, items)| {
                                    div()
                                        .flex()
                                        .flex_col()
                                        .children(many.then(|| {
                                            ui::text(13., FontWeight::MEDIUM, ink::QUIET)
                                                .child(section)
                                        }))
                                        .child(div().mt(px(6.)).child(bullets(items)))
                                }),
                            )
                        }))
                }
            }
        });
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(toggle)
                .children(list),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nightly_cards_show_the_date_and_keep_the_commit_for_a_tooltip() {
        let release = Release {
            tag: "nightly-2026-11-07-4aa6854".into(),
            name: "Nightly 2026-11-07 · 4aa6854".into(),
            body: String::new(),
            date: "2026-11-07".into(),
            time: "14:32".into(),
            asset: None,
            prerelease: true,
            nightly: Some("nightly".into()),
        };

        let (title, line) = nightly_card_text(&release, false).expect("nightlies have card text");
        assert_eq!(title, "Nov 7 update");
        assert_eq!(line, "Nov 7, 14:32 UTC");
        assert_eq!(commit_of(&release.tag), "4aa6854");
        let (title, line) = nightly_card_text(&release, true).expect("nightlies have card text");
        assert_eq!(title, "Nov 7, 14:32 UTC update");
        assert_eq!(line, "");
    }
}
