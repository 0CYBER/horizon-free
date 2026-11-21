//! Home's head in the glass look (the mockup's `HomeView`, `launcher-views.tsx`): the instance's
//! edition, its icon and name, how long it was played, then Play with its menu, the version, the
//! instance's mods and its folders, and joining a friend on the right. Dark appearance only.
use super::*;
use crate::glass_ui::{self as ui, white};
use gpui::deferred;
use horizon_theme::glass::{fill, ink, radius};
use std::time::Instant;

/// Play, the version and the big icons are this tall (`h-14`).
const TALL: f32 = 56.;
/// The window width from which Home's head is laid out as the mockup's `xl` (1280 px); under it
/// Play and the version are narrower and the invite code takes what is left of their row. The
/// account page and the theme editor also put their two columns side by side from it.
pub(super) const WIDE: f32 = 1280.;
/// The window width from which pages put their head and cards side by side (Overview, Mods,
/// Versions, Settings' About); under it they stack, and Home's tabs drop their icons.
pub(super) const SIDE_BY_SIDE: f32 = 1200.;
/// Under this window width the top bar's tabs tighten and Settings' sections go above the page.
pub(super) const COMPACT: f32 = 1180.;

/// The Version menu's state (Home's picker).
#[derive(Default)]
pub(super) struct PickerState {
    /// The menu is open (the old look's; the glass look keeps versions in instance settings).
    pub(super) open: bool,
}

/// Which of Home's menus is open.
#[derive(Clone, Copy, PartialEq, Default)]
pub(super) enum HomeMenu {
    #[default]
    None,
    Play,
    Folder,
}

impl Launcher {
    /// The head: what Home shows above its tabs. `tag`: the Horizon version it plays (empty for a
    /// Java instance); `prepared`: whether that version's files are ready.
    pub(super) fn glass_home_head(
        &mut self,
        tag: &str,
        prepared: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let java = self.selected_instance().and_then(|i| i.java.clone());
        // How long this game has run, for "Playing for 5 min now".
        match (&self.client, self.client_since) {
            (Some(_), None) => self.client_since = Some(Instant::now()),
            (None, Some(_)) => self.client_since = None,
            _ => {}
        }
        // The invite code stays on the actions' row at every width (unlike the mockup's
        // `xl:flex-row`, which moves it under them): on the right, 340 wide from 1280 px; under
        // that it takes what is left, and the puzzle (the Mods tab below does the same) gives
        // way to it.
        let wide = self.window_width >= WIDE;
        let mods = ui::big_icon("home-mods", "puzzle", &self.theme)
            .tooltip(ui::tip(tr("Mods in this instance")))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.home_tab = home_tabs::HomeTab::Mods;
                cx.notify();
            }));
        // Instance settings: the sheet down the right side.
        let settings = |this: &mut Self, cx: &mut Context<Self>| {
            let instance = this.selected_instance().cloned()?;
            // Memory is set only for Java instances.
            let tip = match instance.java {
                Some(_) => "Edit this instance: its name, version, memory and more",
                None => "Edit this instance: its name, icon, version and more",
            };
            Some(
                ui::big_icon("home-settings", "pencil-edit-02", &this.theme)
                    .tooltip(ui::tip(tr(tip)))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.open_instance_sheet(instance.clone(), glass_sheet::SheetTab::General);
                        cx.notify();
                    })),
            )
        };
        let actions = if wide {
            div()
                .mt(px(32.))
                .flex()
                // One line, unlike the mockup's `flex-wrap`: at 1280 px the folder's ⋯ would wrap the
                // row away from under the pointer, which hides it again.
                .items_center()
                .gap(px(10.))
                .child(self.play_group(tag, prepared || java.is_some(), cx))
                .children(Some(mods))
                .children(self.folder_group(java.is_none(), cx))
                .children(settings(self, cx))
                .child(div().flex_1().min_w(px(22.)))
                .child(self.glass_join_friend(tag, true, cx))
        } else {
            div()
                .mt(px(32.))
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(self.play_group(tag, prepared || java.is_some(), cx))
                        .children(self.folder_group(java.is_none(), cx))
                        .children(settings(self, cx)),
                )
                .child(self.glass_join_friend(tag, false, cx))
        };
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .pt(px(64. - crate::glass::CONTENT_INSET))
            .child(motion::enter(
                ElementId::named_usize("home-title", self.switches),
                1,
                self.home_title(cx),
            ))
            .child(motion::enter(
                ElementId::named_usize("home-status", self.switches),
                2,
                self.played_text(cx),
            ))
            .child(motion::enter(
                ElementId::named_usize("home-actions", self.switches),
                3,
                actions,
            ))
    }

    /// The edition switch over the rail, in the top bar where Horizon's mark was: Horizon's mark
    /// and the grass block on a segmented control whose lit pill slides to the selected
    /// instance's edition. An instance's edition is chosen when it is made, so the other side
    /// goes to an instance of that edition ([`Self::switch_edition`]).
    pub(super) fn edition_switch(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        const ID: &str = "edition-switch";
        let java = self.selected_instance().is_some_and(|i| i.java.is_some());
        let horizon = self.horizon_mark(20.);
        let grass = match self.grass_mark(20.) {
            Some(image) => gpui::img(image)
                .flex_none()
                .size(px(20.))
                .rounded(px(5.))
                .into_any_element(),
            None => div().into_any_element(),
        };
        let origin = ui::PillOrigin::default();
        let mut seg = div()
            .id(ID)
            .relative()
            .flex()
            .flex_none()
            .gap(px(2.))
            .p(px(3.))
            .rounded(px(radius::LARGE))
            .bg(white(0.04))
            .border_1()
            .border_color(white(ui::edge::REST))
            .child(ui::pill_origin(&origin));
        for (i, (label, mark)) in [
            ("Horizon: your Horizon instances", horizon),
            ("Java Edition: your Java instances", grass),
        ]
        .into_iter()
        .enumerate()
        {
            let on = (i == 1) == java;
            let d = div()
                .id(SharedString::from(format!("{ID}-{i}")))
                .relative()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(30.))
                .rounded(px(radius::CONTROL - 1.))
                .child(
                    div()
                        .relative()
                        .opacity(if on { 1. } else { 0.5 })
                        .child(mark),
                )
                .child(ui::pill_probe(ID, i, origin.clone()))
                .tooltip(ui::tip(tr(label)));
            let d = if on {
                d
            } else {
                d.cursor_pointer()
                    .hover(|s| s.bg(white(fill::ROW_HOVER)))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.switch_edition(i == 1, cx);
                        this.go(Page::Play);
                        cx.notify();
                    }))
            };
            seg = seg.child(crate::keyboard(d, &self.theme));
        }
        let pill = ui::sliding_pill(ID, usize::from(java), || {
            div()
                .absolute()
                .top(px(3.))
                .bottom(px(3.))
                .rounded(px(radius::CONTROL - 1.))
                .border_1()
                .border_color(ui::color(ui::look().accent).opacity(0.6))
                .bg(ui::color(ui::look().accent).opacity(0.2))
        });
        // The pill first, so the marks are drawn over it.
        seg.children(pill)
    }

    /// An edition tab's click: the instance of that edition (`java`) played last, or, with none,
    /// the New instance dialog on Vanilla or Horizon.
    pub(super) fn switch_edition(&mut self, java: bool, cx: &mut Context<Self>) {
        let pick = edition_pick(&self.instances.list, java).cloned();
        match pick {
            Some(instance) => self.select_instance(&instance),
            None => {
                self.open_instance_dialog(None);
                let kind = if java {
                    new_instance::NewKind::Vanilla
                } else {
                    new_instance::NewKind::Horizon
                };
                self.pick_kind(kind, cx);
            }
        }
    }

    /// The instance's icon and its name in Bricolage, 64 px at this width
    /// (`clamp(2.75rem, 5vw, 4rem)`). Clickable to open the edit dialog (fixes #8689).
    fn home_title(&mut self, cx: &mut Context<Self>) -> Div {
        let Some(instance) = self.selected_instance().cloned() else {
            return div();
        };
        let edit = instance.clone();
        let cut = ui::cut(
            cx,
            &self.rooms,
            "home-title",
            &instance.name,
            horizon_theme::glass::DISPLAY_FONT,
            44.,
            FontWeight::SEMIBOLD,
        );
        let version = self.version_chip(cx);
        let whats_new = self.whats_new_link(cx);
        div()
            .mt(px(16.))
            .flex()
            .items_center()
            .gap(px(16.))
            .min_w_0()
            .child(
                div()
                    .id("home-instance-icon")
                    .cursor_pointer()
                    .hover(|s| s.opacity(0.85))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.open_instance_sheet(edit.clone(), glass_sheet::SheetTab::General);
                        cx.notify();
                    }))
                    .child(self.glass_instance_icon(&instance, 56.)),
            )
            .child(
                div()
                    .id("home-title")
                    .relative()
                    .min_w_0()
                    .truncate()
                    .font_family(horizon_theme::glass::DISPLAY_FONT)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(44.))
                    .line_height(px(48.))
                    .text_color(white(ink::FULL))
                    // A long name is cut at the header's width: the whole of it on hover.
                    .when(cut, |d| d.tooltip(ui::tip(instance.name.clone())))
                    .child(ui::room(&self.rooms, "home-title".into()))
                    .child(instance.name),
            )
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(8.))
                    .children(version)
                    .children(whats_new),
            )
    }

    /// "You've played for 41 h 06 min! Last played today.", or how long it has run now; then
    /// the players counts the launcher knows (not in the mockup).
    fn played_text(&self, cx: &mut Context<Self>) -> Div {
        let line = match (self.client_since, self.selected_instance()) {
            (Some(since), _) => trf(
                "Playing for {time} now. Have fun!",
                &[("time", &instances::play_time(since.elapsed()))],
            ),
            (None, Some(i)) => match i.last_played {
                Some(when) => trf(
                    "You've played for {time}! Last played {when}.",
                    &[
                        ("time", &played_for(i.played)),
                        ("when", &last_played(when)),
                    ],
                ),
                // Signed out, the button says "Sign in to play": the line says the same.
                None if self.owner == Some(false)
                    && !horizon_launcher::developer_offline::selected() =>
                {
                    tr("You haven't played this one yet. Sign in to start.").into()
                }
                None => tr("You haven't played this one yet. Press Play to start.").into(),
            },
            (None, None) => String::new(),
        };
        let link = |id: &'static str, label: String| {
            div()
                .id(id)
                .cursor_pointer()
                .hover(|s| s.text_color(white(ink::FULL)))
                .child(label)
        };
        let playing = self.playing.map(|count| {
            let label = format!(
                "· {}",
                trf("{count} playing now", &[("count", &thousands(count))])
            );
            let url = telemetry::stats_url(&self.links.website);
            link("playing-now", label).on_click(move |_: &ClickEvent, _, cx| {
                if let Some(url) = &url {
                    cx.open_url(url);
                }
            })
        });
        let official = self.official_players.map(|count| {
            link(
                "official-players",
                format!(
                    "· {}",
                    trf(
                        "{count} on the official SMP",
                        &[("count", &thousands(count))]
                    )
                ),
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.go(Page::Servers);
                cx.notify();
            }))
        });
        // A local unsigned developer base is said as such, with its trust warning.
        let developer = self
            .selected_instance()
            .and_then(|i| self.version_of(i))
            .and_then(|tag| {
                horizon_launcher::developer_install::instance_status(tag, backend::is_prepared(tag))
            });
        // A soft dark scrim keeps the line readable over the brightest sky (a low sun, snow).
        ui::text(15., FontWeight::NORMAL, ink::STRONG)
            .mt(px(10.))
            .ml(px(-10.))
            .px(px(10.))
            .py(px(6.))
            .rounded(px(radius::CONTROL))
            .bg(gpui::black().opacity(0.28))
            .self_start()
            .flex()
            .flex_wrap()
            .gap(px(4.))
            .child(line)
            .children(playing)
            .children(official)
            .children(developer.map(|status| div().w_full().text_color(ui::warn()).child(status)))
    }

    /// Play and its ⋯ menu (`PlayGroup`): Stop while the game runs, Update when the launcher's
    /// version has a newer one, the progress while preparing, updating or starting.
    fn play_group(&mut self, tag: &str, ready: bool, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let accent = ui::color(self.look.accent);
        let running = self.client.is_some();
        let follows = self
            .selected_instance()
            .is_some_and(|i| i.follows_horizon());
        let stable = self
            .selected_instance()
            .is_some_and(|i| i.channel == horizon_launcher::instances::Channel::Stable);
        // A Stable instance follows the newest stable build, not each Nightly.
        let stable_update = self.stable_update(tag);
        let update = if stable {
            stable_update.as_ref().map(|r| r.tag.clone())
        } else {
            self.updates
                .horizon
                .as_ref()
                .filter(|_| follows && !tag.is_empty())
                .map(|r| r.tag.clone())
        };
        let installing = update
            .as_ref()
            .and_then(|u| self.installing.as_ref().filter(|(t, _)| t == u))
            .map(|(_, p)| *p);
        // The version this instance keeps, downloading since it was made or edited to play it.
        let own = self
            .selected_instance()
            .and_then(|i| i.version.as_ref())
            .and_then(|v| {
                let queued = self.install_next.as_ref().is_some_and(|r| r.tag == *v);
                let running = self.installing.as_ref().filter(|(t, _)| t == v);
                running.map(|(_, p)| Some(*p)).or(queued.then_some(None))
            });
        let mine = self
            .selected_instance()
            .is_some_and(|i| self.native_for.as_deref() == Some(i.id.as_str()));
        let building = mine && self.native_step.is_some();
        // Native mods that changed since the last build: Play builds them first, one button.
        let needs_build = !building
            && self
                .selected_instance()
                .filter(|i| i.java.is_none())
                .is_some_and(|i| {
                    matches!(
                        self.native_state_of(i),
                        native_status::State::NotBuilt | native_status::State::Outdated
                    )
                });
        let busy: Option<(SharedString, Option<f32>)> =
            match (installing, self.preparing, &self.starting_client) {
                (Some(p), _, _) => Some((tr("Updating").into(), Some(p))),
                (_, _, _) if own.is_some() => Some((tr("Installing").into(), own.flatten())),
                // A Stable update never replaces another download: it waits, and says so.
                _ if stable && update.is_some() && self.stable_update_waits() => {
                    Some((tr("Waiting for another download").into(), None))
                }
                (_, true, _) => Some((
                    tr("Preparing").into(),
                    horizon_launcher::mojang::progress().map(|(d, n)| d as f32 / n.max(1) as f32),
                )),
                (_, _, Some(_)) => Some((
                    self.play_step
                        .now()
                        .unwrap_or(tr("Starting"))
                        .to_owned()
                        .into(),
                    None,
                )),
                // This instance's mods building (Build and play): the build's overall progress,
                // the same figure as its card.
                _ if building => Some((
                    match self.native_stopping {
                        true => tr("Stopping the build…").into(),
                        false => tr("Building mods").into(),
                    },
                    self.native_progress
                        .map(|p| build_mode::percent(Some(p)) as f32 / 100.),
                )),
                // One build at a time: another instance's build first, and this one says whose.
                _ if needs_build && self.native_step.is_some() => Some((
                    trf(
                        "Waiting for {instance}",
                        &[(
                            "instance",
                            self.native_for
                                .as_deref()
                                .and_then(|id| self.instances.list.iter().find(|i| i.id == id))
                                .map_or("", |i| i.name.as_str()),
                        )],
                    )
                    .into(),
                    None,
                )),
                _ => None,
            };
        let open = self.home_menu == HomeMenu::Play;
        // What is drawn on the accent takes its own ink (white on the default violet), in the
        // light look too, where `white` is the page's dark ink; Stop sits on a quiet fill.
        let on_accent = ui::color(self.look.on_accent);
        let glyph = if running && busy.is_none() {
            white(ink::FULL)
        } else {
            on_accent
        };
        let more = div()
            .id("play-more")
            .occlude()
            .absolute()
            .right(px(8.))
            .flex()
            .items_center()
            .justify_center()
            .size(px(40.))
            .rounded(px(radius::CONTROL))
            .cursor_pointer()
            .when(open, |d| d.bg(gpui::black().opacity(0.2)))
            .when(!open, |d| {
                d.child(ui::back_tint(radius::CONTROL, gpui::black().opacity(0.2)))
            })
            .child(ui::icon("more-horizontal", 20., glyph).relative())
            .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                cx.stop_propagation();
                if !this.closed_by_this_press(event) {
                    this.home_menu = match this.home_menu {
                        HomeMenu::Play => HomeMenu::None,
                        _ => HomeMenu::Play,
                    };
                }
                cx.notify();
            }));
        // A build under way, or a start's pre-launch command, is stopped from the button it was
        // started from.
        let more = if building || self.pre_launch_stoppable() {
            div()
                .id("play-cancel")
                .occlude()
                .absolute()
                .right(px(8.))
                .flex()
                .items_center()
                .justify_center()
                .size(px(40.))
                .rounded(px(radius::CONTROL))
                .cursor_pointer()
                .hover(|s| s.bg(gpui::black().opacity(0.2)))
                .child(ui::icon("stop", 18., glyph))
                .tooltip(ui::tip(if building {
                    tr("Stop the build")
                } else {
                    tr("Stop")
                }))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    if building {
                        // A second press while it stops would only stop it again.
                        if !this.native_stopping {
                            this.stop_native_build();
                        }
                    } else {
                        this.stop_pre_launch();
                    }
                    cx.notify();
                }))
        } else {
            more
        };
        let base = div()
            .relative()
            .flex()
            .items_center()
            .flex_none()
            // At least the mockup's width, wider when the label needs it ("Prepare & play",
            // "Sign in to play", a long translation).
            .min_w(px(if self.window_width >= WIDE {
                200.
            } else {
                160.
            }))
            .h(px(TALL))
            .overflow_hidden()
            .border_1()
            .rounded(px(radius::LARGE));
        let body = |label: SharedString, icon: &'static str| {
            div()
                .flex()
                .flex_1()
                .h_full()
                .items_center()
                .justify_center()
                .gap(px(10.))
                .pl(px(20.))
                .pr(px(52.))
                .whitespace_nowrap()
                .font_family(horizon_theme::glass::FONT)
                .text_size(px(18.))
                .font_weight(FontWeight::SEMIBOLD)
                // The glyph leans into the press while the pointer is on Play.
                .child(
                    ui::icon(icon, 20., glyph)
                        .group_hover("play-glyph", |s| s.ml(px(3.)).mr(px(-3.))),
                )
                .child(label)
        };
        let busy_none = busy.is_none();
        let group = match busy {
            Some((label, progress)) => base
                .bg(accent.opacity(0.35))
                .border_color(accent.opacity(0.5))
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .h_full()
                        .w(gpui::relative(progress.unwrap_or(0.).clamp(0., 1.)))
                        .bg(accent),
                )
                .child(
                    div()
                        .relative()
                        .flex()
                        .flex_1()
                        .justify_center()
                        .pl(px(16.))
                        .pr(px(48.))
                        .whitespace_nowrap()
                        .text_color(on_accent)
                        .font_family(horizon_theme::glass::FONT)
                        .text_size(px(16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(match progress {
                            Some(p) => format!("{label} {:.0}%", p * 100.),
                            None => format!("{label}…"),
                        }),
                ),
            None if running => base
                .bg(white(0.14))
                .border_color(white(ui::edge::HOVER))
                .child(
                    body(tr("Stop").into(), "stop")
                        .id("play")
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            if let Some(client) = &this.client {
                                client.kill();
                            }
                            cx.notify();
                        })),
                ),
            None => {
                let owner = if horizon_launcher::developer_offline::selected() {
                    Some(true)
                } else {
                    self.owner
                };
                // `HORIZON_LAUNCHER_PREVIEW_OWNER=1`: Play as for an owner with the game's files
                // ready, to drive Build and play in debug builds without a sign-in.
                let preview = cfg!(debug_assertions)
                    && std::env::var_os("HORIZON_LAUNCHER_PREVIEW_OWNER").is_some_and(|v| v == "1");
                let (ready, owner) = if preview {
                    (true, Some(true))
                } else {
                    (ready, owner)
                };
                let press = play_press(ready, owner, false);
                let build_first =
                    needs_build && update.is_none() && matches!(press, PlayPress::Play);
                let (label, icon) = match &update {
                    Some(_) => (tr("Update"), "download-04"),
                    None if build_first => (tr("Build and play"), "hammer"),
                    None => (tr(play_label(press, owner)), "play"),
                };
                let main = crate::track_hover(
                    body(label.into(), icon).id("play").cursor_pointer(),
                    SharedString::from("play"),
                );
                let main = match (&update, press) {
                    (None, PlayPress::Play) if build_first => {
                        main.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.play_after_build = this.selected_instance().map(|i| i.id.clone());
                            if this.settings.native_mods_allowed_at.is_some() {
                                this.build_native();
                            } else {
                                this.request_native_build_consent();
                            }
                            cx.notify();
                        }))
                    }
                    (Some(_), _) if stable => {
                        let release = stable_update.clone();
                        main.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            if let Some(release) = release.clone() {
                                this.install_stable_update(release);
                            }
                            cx.notify();
                        }))
                    }
                    (Some(_), _) => main.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.install_update();
                        cx.notify();
                    })),
                    (None, PlayPress::Play) => {
                        main.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.picker.open = false;
                            this.play();
                            cx.notify();
                        }))
                    }
                    (None, PlayPress::PrepareThenPlay) => {
                        main.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.picker.open = false;
                            this.play_when_prepared = this
                                .selected()
                                .cloned()
                                .zip(this.selected_instance().map(|i| i.id.clone()));
                            this.prepare(backend::MinecraftFrom::Install);
                            cx.notify();
                        }))
                    }
                    // No account here owns the game: the button says "Sign in to play", so a
                    // press signs in, as the top bar's Sign in with Microsoft does.
                    (None, PlayPress::Nothing) if owner == Some(false) => {
                        main.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.picker.open = false;
                            this.sign_in_to_play();
                            cx.notify();
                        }))
                    }
                    (None, PlayPress::Nothing) => main.opacity(0.5).cursor_default(),
                };
                // The gradient's far end leans away from the label's ink (deeper under white
                // ink, lighter under dark ink), so the label and the menu dots on that end
                // keep their contrast across the whole button.
                let (accent_hex, ink) = (self.look.accent, self.look.on_accent);
                let far = if horizon_theme::contrast(ink, 0x000000)
                    > horizon_theme::contrast(ink, 0xffffff)
                {
                    horizon_theme::glass::mix_oklab(accent_hex, 0x000000, 0.75)
                } else {
                    ui::accent2()
                };
                base.bg(ui::grad(135., accent, ui::color(far)))
                    .border_color(white(0.16))
                    .shadow(ui::glows(vec![
                        // The site's primary: a light line along the top edge, a glow under it.
                        gpui::BoxShadow {
                            color: white(0.22),
                            offset: gpui::point(px(0.), px(1.)),
                            blur_radius: px(0.),
                            spread_radius: px(0.),
                            inset: true,
                        },
                        gpui::BoxShadow {
                            color: accent.opacity(0.7),
                            offset: gpui::point(px(0.), px(6.)),
                            blur_radius: px(22.),
                            spread_radius: px(-8.),
                            inset: false,
                        },
                    ]))
                    .text_color(on_accent)
                    .child(main)
                    .child(ui::sheen(&SharedString::from("play")))
            }
        };
        // Ready to play: the accent's glow comes up under the pointer, a press dims it.
        let primary = !running && busy_none;
        let soft = ui::color(self.look.accent_soft);
        let group = group
            .id("play-group")
            .group("play-glyph")
            .when(primary, |d| {
                d.hover(move |s| {
                    s.border_color(white(0.34)).shadow(ui::glows(vec![
                        gpui::BoxShadow {
                            color: white(0.28),
                            offset: gpui::point(px(0.), px(1.)),
                            blur_radius: px(0.),
                            spread_radius: px(0.),
                            inset: true,
                        },
                        gpui::BoxShadow {
                            color: accent.opacity(0.95),
                            offset: gpui::point(px(0.), px(12.)),
                            blur_radius: px(34.),
                            spread_radius: px(-8.),
                            inset: false,
                        },
                        gpui::BoxShadow {
                            color: soft.opacity(0.5),
                            offset: gpui::point(px(0.), px(0.)),
                            blur_radius: px(0.),
                            spread_radius: px(1.),
                            inset: false,
                        },
                    ]))
                })
                .active(|s| s.opacity(0.92).shadow(Vec::new()))
            });
        let mut wrap = div().relative().child(keyboard(group, &t).child(more));
        if open {
            let locked = running || !busy_none;
            let mut items: Vec<(&'static str, SharedString, String, bool, PlayItem)> = Vec::new();
            if !tag.is_empty() {
                let version = backend::game_version(tag);
                items.push((
                    "play-prepare",
                    tr(if backend::is_prepared(tag) {
                        "Prepare again"
                    } else {
                        "Prepare"
                    })
                    .into(),
                    trf(
                        "Read {version}'s files from your own copy",
                        &[("version", &version)],
                    ),
                    locked,
                    PlayItem::Prepare,
                ));
            }
            if update.is_some() {
                items.push((
                    "play-skip",
                    tr("Play without updating").into(),
                    trf(
                        "Stay on {version}",
                        &[("version", &self.version_label(tag))],
                    ),
                    locked || !ready,
                    PlayItem::Play,
                ));
            }
            let id = self
                .selected_instance()
                .map(|i| (i.id.clone(), i.name.clone()));
            if let Some((id, _)) = &id {
                let has = self.has_shortcut(id);
                items.push((
                    "play-shortcut",
                    tr(if has {
                        "Remove the desktop shortcut"
                    } else {
                        "Create a desktop shortcut"
                    })
                    .into(),
                    tr("Starts this instance in one click").into(),
                    false,
                    PlayItem::Shortcut,
                ));
                items.push((
                    "play-command",
                    tr("Copy the launch command").into(),
                    launch_command(id),
                    false,
                    PlayItem::Command,
                ));
            }
            let mut menu = ui::popover(290.)
                .id("play-menu")
                .occlude()
                .on_mouse_down_out(cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    this.home_menu = HomeMenu::None;
                    this.menu_closed_at = Some(event.position);
                    cx.notify();
                }));
            for (key, label, note, locked, what) in items {
                let id = id.clone();
                let item = ui::menu_item(key, label, note, locked, &t);
                menu = menu.child(if locked {
                    item
                } else {
                    item.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.home_menu = HomeMenu::None;
                        match what {
                            PlayItem::Prepare => this.prepare(backend::MinecraftFrom::Install),
                            PlayItem::Play => this.play(),
                            PlayItem::Shortcut => {
                                if let Some((id, name)) = id.clone() {
                                    this.toggle_shortcut(id, name);
                                }
                            }
                            PlayItem::Command => {
                                if let Some((id, _)) = &id {
                                    copy_to_clipboard(
                                        cx,
                                        gpui::ClipboardItem::new_string(launch_command(id)),
                                    );
                                }
                            }
                        }
                        cx.notify();
                    }))
                });
            }
            wrap = wrap.child(
                deferred(div().absolute().top(px(TALL + 8.)).left_0().child(menu)).with_priority(1),
            );
        }
        wrap
    }

    /// "What's new" beside the version button: the Versions page with the played build's notes
    /// open.
    fn whats_new_link(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let tag = self.whats_new_tag()?;
        Some(
            hero_button("home-whats-new", "sparkles", tr("What's new").into()).on_click(
                cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.whats_new_open = Some(std::iter::once(tag.clone()).collect());
                    this.go(Page::Versions);
                    cx.notify();
                }),
            ),
        )
    }

    /// What the instance plays, at the title's right: "Latest version", "Nov 11 update",
    /// "26.3 · Fabric 0.17". Its versions are in instance settings, which a click opens on
    /// Installation.
    fn version_chip(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let instance = self.selected_instance()?.clone();
        let label = self.sheet_version(&instance);
        Some(
            hero_button("home-version", "package-01", label.into())
                .tooltip(ui::tip(tr("Change the version in instance settings")))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.open_instance_sheet(instance.clone(), glass_sheet::SheetTab::Installation);
                    cx.notify();
                })),
        )
    }

    /// The folder button and its ⋯ (`FolderGroup`), which shows under the pointer or while its
    /// menu is open: the instance's folder, its screenshots, and (Horizon) the Minecraft folder.
    fn folder_group(&self, horizon: bool, cx: &mut Context<Self>) -> Option<Div> {
        let t = self.theme;
        let instance = self.selected_instance()?;
        let home = instance.home.clone();
        let open = self.home_menu == HomeMenu::Folder;
        let opener = home.clone();
        let more = div()
            .id("folder-more")
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .h(px(40.))
            .w(px(36.))
            .mr(px(8.))
            .rounded(px(radius::CONTROL))
            .cursor_pointer()
            .when(open, |d| {
                d.w(px(36.))
                    .mr(px(8.))
                    .opacity(1.)
                    .bg(white(fill::SELECTED))
            })
            .group_hover("folder-group", |s| s.w(px(36.)).mr(px(8.)).opacity(1.))
            .when(!open, |d| {
                d.child(ui::back_tint(radius::CONTROL, white(fill::SELECTED)))
            })
            .child(ui::icon("more-horizontal", 20., white(ink::STRONG)).relative())
            .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                if !this.closed_by_this_press(event) {
                    this.home_menu = match this.home_menu {
                        HomeMenu::Folder => HomeMenu::None,
                        _ => HomeMenu::Folder,
                    };
                }
                cx.notify();
            }));
        let group = div()
            .id("folder-group")
            .relative()
            .flex()
            .items_center()
            .h(px(TALL))
            .rounded(px(radius::LARGE))
            .bg(white(fill::BUTTON))
            .border_1()
            .border_color(white(if open {
                ui::edge::STRONG
            } else {
                ui::edge::REST
            }))
            .child(ui::back_edge(radius::LARGE, 0.05, true))
            .child(
                div()
                    .id("home-folder")
                    .relative()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(TALL - 2.))
                    .cursor_pointer()
                    .active(|s| s.bg(white(0.1)))
                    .child(ui::icon("folder-01", 21., white(ink::STRONG)))
                    .tooltip(ui::tip(tr("Open this instance's folder")))
                    .on_click(move |_, _, cx| {
                        let _ = std::fs::create_dir_all(&opener);
                        crate::glass_toast::open(cx, &opener);
                    }),
            )
            .when(open || self.folder_hovered, |d| d.child(more))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.folder_hovered = *hovered;
                cx.notify();
            }));
        let mut wrap = div().relative().child(keyboard(group, &t));
        if open {
            let shots = home.join("screenshots");
            let count = std::fs::read_dir(&shots).map_or(0, |d| d.count());
            let mut items = vec![
                (
                    "folder-instance",
                    tr("Open the instance folder"),
                    short(&home),
                    Some(home.clone()),
                ),
                (
                    "folder-shots",
                    tr("Open the screenshots folder"),
                    if count == 1 {
                        tr("1 screenshot").into()
                    } else {
                        trf("{count} screenshots", &[("count", &count.to_string())])
                    },
                    Some(shots),
                ),
            ];
            if horizon {
                items.push((
                    "folder-minecraft",
                    tr("Choose the Minecraft folder"),
                    self.settings
                        .minecraft_dir
                        .as_deref()
                        .map_or_else(|| tr("Not found").into(), short),
                    None,
                ));
            }
            let mut menu = ui::popover(290.)
                .id("folder-menu")
                .occlude()
                .on_mouse_down_out(cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    this.home_menu = HomeMenu::None;
                    this.menu_closed_at = Some(event.position);
                    cx.notify();
                }));
            for (key, label, note, path) in items {
                menu = menu.child(ui::menu_item(key, label.into(), note, false, &t).on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.home_menu = HomeMenu::None;
                        match &path {
                            Some(p) => {
                                let _ = std::fs::create_dir_all(p);
                                crate::glass_toast::open(cx, p);
                            }
                            None => this.choose_folder(cx),
                        }
                        cx.notify();
                    }),
                ));
            }
            let edit = instance.clone();
            let note = trf("Edit {name}", &[("name", &instance.name)]);
            menu = menu.child(
                ui::menu_item(
                    "folder-edit-instance",
                    tr("Edit instance").into(),
                    note,
                    false,
                    &t,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.home_menu = HomeMenu::None;
                    this.open_instance_sheet(edit.clone(), glass_sheet::SheetTab::General);
                    cx.notify();
                })),
            );
            wrap = wrap.child(
                deferred(div().absolute().top(px(TALL + 8.)).left_0().child(menu)).with_priority(1),
            );
        }
        Some(wrap)
    }

    /// The invite code field (`JoinFriend`): 340 wide (`wide`), else what its row leaves, with a
    /// shorter placeholder; the icon inside on the left, Join inside on the right.
    fn glass_join_friend(&self, tag: &str, wide: bool, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let focused = self.friend_focused;
        let join_label = tr("Join");
        let pr = friend_code_padding_right(join_label);
        let field = div()
            .id("friend-code")
            .track_focus(&self.friend_focus)
            .flex()
            .items_center()
            .size_full()
            .pl(px(48.))
            .pr(px(pr))
            .rounded(px(radius::LARGE))
            .bg(white(if focused { 0.09 } else { 0.06 }))
            .border_1()
            .border_color(if focused {
                ui::color(ui::look().accent_soft).opacity(0.7)
            } else {
                white(ui::edge::REST)
            })
            .relative()
            .child(ui::back_edge(radius::LARGE, 0., true))
            .font_family(horizon_theme::glass::FONT)
            .text_size(px(14.))
            .text_color(white(ink::FULL))
            .cursor_text()
            .overflow_hidden()
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .items_center()
                    .overflow_hidden()
                    .child(line_edit::content(
                        &self.friend_code,
                        &t,
                        focused,
                        false,
                        false,
                        if wide {
                            "Paste a friend's invite code"
                        } else {
                            "Paste an invite code"
                        },
                    )),
            )
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                window.focus(&this.friend_focus, cx);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                match line_edit::handle(&mut this.friend_code, ev, cx) {
                    line_edit::Key::Pass => return,
                    line_edit::Key::Enter => this.join_friend(),
                    line_edit::Key::Escape => window.blur(cx),
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }));
        let field = line_edit::mouse(field, cx, |this| &mut this.friend_code);
        let valid = horizon_friends::invite::parse(self.friend_code.text()).is_some();
        let can = valid && (tag.is_empty() || self.can_start(tag));
        let join = ui::quiet("join-friend", join_label, 40., &t);
        let join = if can {
            join.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.join_friend();
                cx.notify();
            }))
        } else {
            join.opacity(0.4).cursor_default()
        };
        let wide_w = 340. + (pr - 84.);
        div()
            .relative()
            .when(wide, |d| d.flex_none().w(px(wide_w)))
            .when(!wide, |d| d.w(px(wide_w)).max_w_full())
            .h(px(TALL))
            .child(field)
            .child(
                div()
                    .absolute()
                    .left(px(16.))
                    .top(px((TALL - 19.) / 2.))
                    .child(ui::icon("user-add-01", 19., white(ink::NOTE))),
            )
            .child(div().absolute().right(px(8.)).top(px(8.)).child(join))
    }
}

/// Right padding for the invite code text field. In English ("Join"), the button is ~58 px wide,
/// so 84 px leaves a clean margin from the button placed at right: 8 px. In locales with longer
/// labels (French "Rejoindre", German "Beitreten", Ukrainian "Приєднатися"), the button is wider;
/// expand the field's padding so the button never overlaps the entered text or placeholder (#9507).
fn friend_code_padding_right(join_label: &str) -> f32 {
    let btn_width = 28. + (join_label.chars().count() as f32 * 8.5).max(30.);
    (8. + btn_width + 12.).max(84.)
}

/// The instance of one edition (`java`) an edition tab goes to: of those the player can select,
/// the one played last; among never played ones, the first listed.
/// One of the two buttons at the hero title's right: 40 px glass with an 18 px icon and its
/// label, the icons in one stroke and ink so the pair reads as one.
fn hero_button(id: &'static str, icon: &'static str, label: SharedString) -> Stateful<Div> {
    let id = SharedString::from(id);
    let group = SharedString::from(format!("{id}-group"));
    track_hover(
        div()
            .id(id.clone())
            .group(group.clone())
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .h(px(40.))
            .pl(px(14.))
            .pr(px(16.))
            .rounded(px(radius::CONTROL))
            .bg(white(fill::BUTTON))
            .border_1()
            .border_color(white(ui::edge::REST))
            .cursor_pointer()
            .font_family(horizon_theme::glass::FONT)
            .text_size(px(14.))
            .line_height(px(20.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(white(ink::BODY))
            .hover(|s| s.text_color(white(ink::FULL)))
            .active(|s| s.bg(white(0.18)))
            .child(ui::hover_edge(&id, radius::CONTROL, 0.06, true))
            .child(
                ui::icon(icon, 18., white(ink::STRONG))
                    .group_hover(group, |s| s.text_color(white(ink::FULL)))
                    .relative(),
            )
            .child(div().relative().child(label)),
        id,
    )
}

fn edition_pick(
    list: &[horizon_launcher::instances::Instance],
    java: bool,
) -> Option<&horizon_launcher::instances::Instance> {
    use horizon_launcher::instances::Kind;
    list.iter()
        .filter(|i| matches!(i.kind, Kind::Default | Kind::Own) && i.java.is_some() == java)
        .rev()
        .max_by_key(|i| i.last_played)
}

/// How long an instance was played, as the mockup writes it: "41 hours", "1 hour", "25 min".
fn played_for(played: Duration) -> String {
    match played.as_secs() / 60 {
        0 => tr("under a minute").into(),
        m @ 1..=59 => trf("{count} min", &[("count", &m.to_string())]),
        60..=119 => tr("1 hour").into(),
        m => trf("{count} hours", &[("count", &(m / 60).to_string())]),
    }
}

/// When something was last played, as the mockup writes it: "just now", "25 min ago", "2 hours
/// ago" within a day, then "yesterday", then the day ("Oct 31").
pub(super) fn last_played(when: std::time::SystemTime) -> String {
    let local: chrono::DateTime<chrono::Local> = when.into();
    since(
        std::time::SystemTime::now()
            .duration_since(when)
            .unwrap_or_default(),
        (chrono::Local::now().date_naive() - local.date_naive()).num_days(),
        &local.format("%Y-%m-%d").to_string(),
    )
}

/// [`last_played`] from how long ago it was, how many calendar days back, and its date.
fn since(ago: Duration, days: i64, date: &str) -> String {
    match ago.as_secs() / 60 {
        0 => tr("just now").into(),
        m @ 1..=59 => trf("{count} min ago", &[("count", &m.to_string())]),
        60..=119 => tr("1 hour ago").into(),
        m if m < 24 * 60 => trf("{count} hours ago", &[("count", &(m / 60).to_string())]),
        _ if days <= 1 => tr("yesterday").into(),
        _ => glass_versions::published(date, ""),
    }
}

/// What a Play menu row does.
#[derive(Clone, Copy)]
enum PlayItem {
    Prepare,
    Play,
    Shortcut,
    Command,
}

/// The command that plays instance `id` without the window, as its desktop shortcut runs it.
fn launch_command(id: &str) -> String {
    format!("horizon-launcher play --instance {id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_edition_tab_goes_to_that_editions_instance_played_last() {
        use horizon_launcher::instances::{Instance, JavaGame, Kind};
        use std::time::{SystemTime, UNIX_EPOCH};
        let at = |secs: u64| Some(UNIX_EPOCH + Duration::from_secs(secs));
        let one = |id: &str, kind: Kind, java: bool, last: Option<SystemTime>| Instance {
            id: id.into(),
            name: id.into(),
            kind,
            work: Default::default(),
            home: Default::default(),
            icon: None,
            group: None,
            version: None,
            channel: Default::default(),
            played: Duration::ZERO,
            last_played: last,
            launch: Default::default(),
            java: java.then(JavaGame::default),
            native_profile: Default::default(),
        };
        let list = [
            one("horizon", Kind::Default, false, at(50)),
            one("vanilla-old", Kind::Own, true, at(10)),
            one("vanilla-new", Kind::Own, true, at(20)),
            one("port:pack", Kind::Port, true, at(99)),
            one("mine", Kind::Own, false, None),
        ];
        let id = |java| edition_pick(&list, java).map(|i| i.id.as_str());
        // A modpack isn't selected from Home, however recent.
        assert_eq!(id(true), Some("vanilla-new"));
        assert_eq!(id(false), Some("horizon"));
        // Never played: the first listed; none of an edition: none (New instance opens).
        let fresh = [
            one("a", Kind::Own, true, None),
            one("b", Kind::Own, true, None),
        ];
        assert_eq!(edition_pick(&fresh, true).map(|i| i.id.as_str()), Some("a"));
        assert!(edition_pick(&fresh, false).is_none());
    }

    #[test]
    fn played_time_reads_as_the_mockup_writes_it() {
        let h = |s: u64| played_for(Duration::from_secs(s));
        assert_eq!(h(148_000), "41 hours");
        assert_eq!(h(5_400), "1 hour");
        assert_eq!(h(1_500), "25 min");
    }

    #[test]
    fn last_played_counts_hours_within_a_day_then_days() {
        let m = |min: u64| Duration::from_secs(min * 60);
        assert_eq!(since(m(0), 0, "2026-11-03"), "just now");
        assert_eq!(since(m(25), 0, "2026-11-03"), "25 min ago");
        assert_eq!(since(m(130), 0, "2026-11-03"), "2 hours ago");
        // Under 24 hours, even across midnight.
        assert_eq!(since(m(20 * 60), 1, "2026-11-02"), "20 hours ago");
        assert_eq!(since(m(30 * 60), 1, "2026-11-02"), "yesterday");
        assert_eq!(since(m(3 * 24 * 60), 3, "2026-10-31"), "Oct 31");
    }

    #[test]
    fn friend_code_padding_right_clears_localized_join_button() {
        for label in [
            "Join",
            "Rejoindre",
            "Beitreten",
            "Приєднатися",
            "Unirse",
            "Dołącz",
            "Reunir-se",
        ] {
            let pr = friend_code_padding_right(label);
            let btn_width = 28. + label.chars().count() as f32 * 8.5;
            let btn_right_extent = 8. + btn_width;
            assert!(
                pr >= btn_right_extent + 10.,
                "padding {pr} must clear button right extent {btn_right_extent} for label '{label}'"
            );
        }
    }
}
