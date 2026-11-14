//! The Mods tab's build card and the per-mod route chips (docs/modding/src/how-to/native-mods.md).
//! The card picks the build mode, shows where a build is (a percentage, one pill per stage) and
//! cancels it. A chip on each native mod says how it reaches the game; its menu gives the reason
//! and the actions.
use super::*;
use crate::glass_ui as ui;
use native_mods::{Profile, Route, Stage};
use native_status::State;

/// The running or finished native snapshot; serialize readers to avoid racing cache leases.
pub(super) struct RouteMemo {
    key: String,
    result: Option<Result<native_mods::Routes, String>>,
}

/// The update check for one instance and newer game, while it runs (`None`) and once it ended.
pub(super) struct UpdateMemo {
    key: String,
    result: Option<Result<native_mods::UpdateCheck, String>>,
}

/// What a mod in the list is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ModKind {
    /// Rust compiled into the game.
    Native,
    /// A sandboxed `mod.wasm`: never needs a build.
    Wasm,
    /// Data and assets only.
    Content,
}

impl ModKind {
    pub(super) fn of(manifest: &horizon_mod_package::manifest::Manifest) -> Self {
        if manifest.native.is_some() {
            Self::Native
        } else if manifest.wasm.is_some() {
            Self::Wasm
        } else {
            Self::Content
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Native => tr("Native"),
            Self::Wasm => tr("WebAssembly"),
            Self::Content => tr("Content"),
        }
    }
}

/// The colours a card or chip draws with, in either look.
#[derive(Clone, Copy)]
struct Pal {
    ink: Hsla,
    muted: Hsla,
    faint: Hsla,
    accent: Hsla,
    success: Hsla,
    warn: Hsla,
    danger: Hsla,
}

impl Pal {
    fn of(t: &Theme) -> Self {
        if t.glass {
            Self {
                ink: ui::white(1.),
                muted: ui::white(0.6),
                faint: ui::white(0.07),
                accent: t.accent,
                success: ui::live(),
                warn: ui::warn(),
                danger: ui::danger(),
            }
        } else {
            Self {
                ink: t.ink,
                muted: t.muted,
                faint: t.well,
                accent: t.accent,
                success: t.success,
                warn: t.warn,
                danger: t.danger,
            }
        }
    }
}

/// The share of a whole build each stage covers, for the one percentage.
fn percent(progress: Option<native_mods::Progress>) -> u32 {
    let Some(p) = progress else { return 0 };
    let (from, span) = match p.stage {
        Stage::Prepare => (0., 0.15),
        Stage::Compile => (0.15, 0.8),
        Stage::Install => (0.95, 0.05),
    };
    ((from + span * p.fraction.unwrap_or(0.).clamp(0., 1.)) * 100.).round() as u32
}

impl Launcher {
    /// The worlds stop of the instance on screen, if one is pending: its choice row is the only
    /// way on, so the panel's Build and the menu's Build now wait for it.
    fn worlds_stop(&self, mine: bool) -> Option<&native_mods::AtStake> {
        self.native_worlds.as_ref().filter(|_| mine)
    }

    /// Whether the worlds stop on screen is about mod `mod_id`: its switch and its menu's Turn
    /// off wait for the choice, which backs up the worlds that hold its content.
    pub(super) fn stop_names(&self, mod_id: &str) -> bool {
        let here = self
            .selected_instance()
            .map(|instance| instance.id.as_str());
        self.native_for.as_deref() == here
            && self
                .worlds_stop(true)
                .is_some_and(|stake| stake.mods.iter().any(|id| id == mod_id))
    }

    /// A build stopped for worlds at stake: the choice is shown, and the mods that failed to
    /// compile stay marked past it.
    pub(super) fn keep_stop(&mut self, stake: &native_mods::AtStake) {
        self.native_worlds = Some(stake.clone());
        self.mods.unbuilt = self
            .native_for
            .clone()
            .map(|instance| (instance, stake.unbuilt.clone()));
    }

    /// The selected instance's build card: the build mode, and the build under way or its end.
    pub(super) fn native_build(&self, cx: &mut Context<Self>) -> Option<Div> {
        let t = self.theme;
        let pal = Pal::of(&t);
        let instance = self.selected_instance()?;
        let state = self.native_state_of(instance);
        let mine = self.native_for.as_deref() == Some(instance.id.as_str());
        let building = self.native_step.as_ref().filter(|_| mine);
        let error = self.native_error.as_ref().filter(|_| mine);
        let mut card = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(self.mode_head(state, error, mine, pal, cx));
        card = match building {
            Some(step) => card.child(self.panel().child(self.building_card(step, pal, cx))),
            None => card.child(self.mode_status(error, mine, pal, cx)),
        };
        if building.is_none() && error.is_none() && state != State::NoMods {
            if let Some(update) = self.update_card(instance, pal, cx) {
                card = card.child(update);
            }
        }
        Some(card)
    }

    /// Before an update of an instance with native mods (design-fit §10.1): when a newer release
    /// or Nightly is installed, every mod is checked against it first, and the card says how
    /// many fit and which will be switched off. Update builds on it; staying changes nothing.
    /// When worlds were saved with a mod it switches off, staying is the default and the other
    /// choice backs those worlds up first.
    fn update_card(
        &self,
        instance: &instances::Instance,
        pal: Pal,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        if self.native_step.is_some() || self.settings.native_mods_allowed_at.is_none() {
            return None;
        }
        let plays = self.version_of(instance)?.clone();
        let to = native_mods::update_for(&plays, &self.installed)?;
        let home = self.mods.home();
        let key = format!(
            "{}{to}",
            native_status::fingerprint(&home, Some(to.as_str()))
        );
        if self.mods.update_stayed.borrow().as_deref() == Some(key.as_str()) {
            return None;
        }
        let mut memo = self.mods.native_update.borrow_mut();
        if let Some(previous) = memo.as_ref().filter(|previous| previous.key == key) {
            // While it runs, nothing; a check that failed leaves it to the build to say why.
            let check = previous.result.as_ref()?.as_ref().ok()?.clone();
            drop(memo);
            return Some(self.update_panel(check, &plays, key, pal, cx));
        }
        *memo = Some(UpdateMemo {
            key: key.clone(),
            result: None,
        });
        drop(memo);
        let work = cx
            .background_executor()
            .spawn(async move { native_mods::update_check(&home, &to).map_err(String::from) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                *this.mods.native_update.borrow_mut() = Some(UpdateMemo {
                    key,
                    result: Some(result),
                });
                cx.notify();
            });
        })
        .detach();
        None
    }

    /// The update's summary and its two choices.
    fn update_panel(
        &self,
        check: native_mods::UpdateCheck,
        plays: &str,
        key: String,
        pal: Pal,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = self.theme;
        let current = native_mods::about(plays).map_or_else(|| plays.to_owned(), |a| a.base);
        let stay = button(
            "native-update-stay",
            native_mods::fit::stay(&current),
            if check.worlds.is_empty() {
                Kind::Secondary
            } else {
                Kind::Primary
            },
            &t,
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            *this.mods.update_stayed.borrow_mut() = Some(key.clone());
            cx.notify();
        }));
        let to = check.to.clone();
        let back_up = !check.worlds.is_empty();
        let take = button(
            "native-update-take",
            if back_up {
                native_mods::fit::back_up_and_update(check.worlds.len())
            } else {
                native_mods::fit::update()
            },
            if back_up {
                Kind::Secondary
            } else {
                Kind::Primary
            },
            &t,
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.build_native_on(Some(to.clone()), back_up);
            cx.notify();
        }));
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(text_in(13., pal.ink).child(check.summary.clone()));
        if back_up {
            body = body.child(
                text_in(13., pal.warn).child(native_mods::fit::update_worlds(&check.worlds)),
            );
        }
        // The default choice first.
        let choices = div().flex().flex_wrap().gap(px(8.));
        let choices = if back_up {
            choices.child(stay).child(take)
        } else {
            choices.child(take).child(stay)
        };
        self.panel().child(body.child(choices))
    }

    /// Where the instance's native mods stand, read again only when what it depends on changes.
    fn native_state_of(&self, instance: &instances::Instance) -> State {
        let version = self.version_of(instance).cloned();
        let home = self.mods.home();
        let profile = Profile::Optimized;
        let key = format!(
            "{}{profile:?}",
            native_status::fingerprint(&home, version.as_deref())
        );
        let mut memo = self.mods.native_state.borrow_mut();
        match memo.as_ref() {
            Some((k, state)) if *k == key => *state,
            _ => {
                let built = version.as_deref().and_then(native_status::built);
                let state = native_status::in_mode(
                    native_status::state(&native_status::enabled(&home), built.as_deref()),
                    version.as_deref().and_then(native_status::built_profile),
                    profile,
                );
                *memo = Some((key, state));
                state
            }
        }
    }

    /// A bordered panel, as the build card and its warning sit on the page.
    fn panel(&self) -> Div {
        let t = self.theme;
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .p(px(16.))
            .when(t.glass, |d| {
                d.rounded(px(horizon_theme::glass::radius::LARGE))
                    .bg(ui::white(0.04))
            })
            .when(!t.glass, |d| {
                d.bg(t.well).border_2().border_color(t.outline)
            })
    }

    /// "Build mode", and what the build needs next with its Build button.
    fn mode_head(
        &self,
        state: State,
        error: Option<&String>,
        mine: bool,
        pal: Pal,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = self.theme;
        let needs = match state {
            State::NoMods | State::UpToDate => None,
            State::NotBuilt => Some(tr("Not built")),
            State::Outdated => Some(tr("Needs rebuild")),
        };
        let can_build = self.pending_consent.is_none()
            && !(self.native_step.is_some() && mine)
            // While worlds are at stake the choice row below is the only decision.
            && self.worlds_stop(mine).is_none()
            && (error.is_some() || needs.is_some());
        let mut right = div().flex().items_center().gap(px(8.));
        if self.native_step.is_none() {
            if let Some(label) = needs {
                right = right.child(blocks::badge(label, pal.warn, &t));
            } else if state == State::UpToDate {
                right = right.child(blocks::badge(tr("Up to date"), pal.success, &t));
            }
        }
        if can_build && self.native_step.is_none() {
            right = right.child(button("native-build", "Build", Kind::Primary, &t).on_click(
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    if this.settings.native_mods_allowed_at.is_some() {
                        this.build_native();
                    } else {
                        this.request_native_build_consent();
                    }
                    cx.notify();
                }),
            ));
        }
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .child(
                text_in(14., pal.ink)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(tr("Native mod build")),
            )
            .child(right)
    }

    /// Under the cards, when no build runs: a failed build's error, how the last one ended, and
    /// another instance's build holding this one.
    fn mode_status(
        &self,
        error: Option<&String>,
        mine: bool,
        pal: Pal,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = self.theme;
        let other = self.native_step.is_some() && !mine;
        let mut card = div().flex().flex_col().gap(px(10.));
        if let Some(e) = error {
            card = card.child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .gap(px(12.))
                    .child(text_in(13., pal.danger).flex_1().min_w_0().child(e.clone()))
                    .child(
                        button("native-show-log", "Show log", Kind::Ghost, &t).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.native_log_open();
                                cx.notify();
                            }),
                        ),
                    ),
            );
        }
        if let Some(stake) = self.worlds_stop(mine).filter(|_| error.is_some()) {
            card = card.child(self.worlds_choice(stake, pal, cx));
        }
        if let Some(done) = self.native_done.as_ref().filter(|_| mine) {
            card = card.child(text_in(13., pal.muted).child(done.clone()));
        }
        if other {
            let name = self
                .native_for
                .as_deref()
                .and_then(|id| self.instances.list.iter().find(|i| i.id == id))
                .map_or("", |i| i.name.as_str());
            card = card.child(text_in(13., pal.muted).child(trf(
                "{instance} is building: one build at a time.",
                &[("instance", name)],
            )));
        }
        card
    }

    /// The choice when the build would switch off mods that worlds were saved with (the error
    /// above names them): staying is the default and changes nothing; the other choice backs
    /// those worlds up, where it says, then builds without the mods.
    fn worlds_choice(&self, stake: &native_mods::AtStake, pal: Pal, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        // The same build again: on the base it stopped on, the worlds backed up first.
        let base = stake.base.clone();
        let stay = native_mods::fit::dont_build().to_string();
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(text_in(13., pal.muted).child(trf(
                "Backups go to {path}, one zip per world.",
                &[("path", &stake.backups.display().to_string())],
            )))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(
                        button("native-worlds-stay", stay, Kind::Primary, &t).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.native_worlds = None;
                                this.native_error = None;
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        button(
                            "native-worlds-back-up",
                            native_mods::fit::back_up_and_build(
                                stake.worlds.len(),
                                stake.mods.len(),
                                stake.unbuilt.len(),
                            ),
                            Kind::Secondary,
                            &t,
                        )
                        .on_click(cx.listener(
                            move |this, _: &ClickEvent, _, cx| {
                                this.build_native_on(Some(base.clone()), true);
                                cx.notify();
                            },
                        )),
                    ),
            )
    }

    /// The card while a build runs: a large percentage, a bar, a pill per stage, and the log and
    /// cancel buttons.
    fn building_card(&self, step: &str, pal: Pal, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let progress = self.native_progress;
        let percent = percent(progress);
        let stage = progress.map(|p| p.stage);
        let pills = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .children(Stage::ALL.map(|s| {
                let (color, glyph) = match stage {
                    Some(now) if now == s => (pal.accent, None),
                    Some(now) if now > s => (pal.success, Some("tick-02")),
                    _ => (pal.muted, None),
                };
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .h(px(24.))
                    .px(px(10.))
                    .rounded_full()
                    .bg(color.opacity(if stage == Some(s) { 0.18 } else { 0.1 }))
                    .children(glyph.map(|g| ui::icon(g, 12., color)))
                    .child(text_in(12., color).child(s.label()))
            }));
        let radius = horizon_theme::glass::radius::CONTROL;
        let icons = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(
                ui::icon_button(
                    "native-build-log",
                    "computer-terminal-01",
                    32.,
                    16.,
                    radius,
                    false,
                    &t,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.native_log_open();
                    cx.notify();
                })),
            )
            .child(
                ui::icon_button("native-build-cancel", "stop", 32., 16., radius, false, &t)
                    .on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                        native_mods::cancel();
                        cx.notify();
                    })),
            );
        let bar = div()
            .h(px(6.))
            .w_full()
            .rounded_full()
            .overflow_hidden()
            .bg(pal.faint)
            .child(
                div()
                    .h_full()
                    .rounded_full()
                    .w(gpui::relative(percent as f32 / 100.))
                    .bg(pal.accent),
            );
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .child(
                                text_in(34., pal.ink)
                                    .line_height(px(36.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(format!("{percent}%")),
                            )
                            .child(pills),
                    )
                    .child(icons),
            )
            .child(bar)
            .child(text_in(13., pal.muted).truncate().child(step.to_owned()))
    }

    /// Each turned-on native mod's route; filesystem and release reads never run while drawing.
    fn native_routes(&self, cx: &mut Context<Self>) -> native_mods::Routes {
        let home = self.mods.home();
        let plays = self
            .selected_instance()
            .and_then(|instance| self.version_of(instance));
        let base = native_mods::base_for(plays.map(String::as_str), &self.installed, false);
        let key = format!(
            "{}{:?}",
            native_status::fingerprint(&home, base.as_deref()),
            self.settings.native_mods_allowed_at
        );
        let mut memo = self.mods.native_routes.borrow_mut();
        if let Some(previous) = memo.as_ref() {
            let Some(result) = &previous.result else {
                return native_mods::Routes::default();
            };
            if previous.key == key {
                return result.as_ref().ok().cloned().unwrap_or_default();
            }
        }
        *memo = Some(RouteMemo {
            key: key.clone(),
            result: None,
        });
        drop(memo);
        let work = cx.background_executor().spawn(async move {
            native_mods::routes(&home, base.as_deref()).map_err(String::from)
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                *this.mods.native_routes.borrow_mut() = Some(RouteMemo {
                    key,
                    result: Some(result),
                });
                cx.notify();
            });
        })
        .detach();
        native_mods::Routes::default()
    }

    /// The chip that says how `mod_id` reaches the game, with its menu when open.
    pub(super) fn route_chip(&self, mod_id: &str, cx: &mut Context<Self>) -> Option<AnyElement> {
        let t = self.theme;
        let pal = Pal::of(&t);
        let instance = self.selected_instance()?;
        let state = self.native_state_of(instance);
        let busy = self.native_step.is_some();
        let building = busy && self.native_for.as_deref() == Some(instance.id.as_str());
        let routes = self.native_routes(cx);
        let mine = self.native_for.as_deref() == Some(instance.id.as_str());
        let stop = self.worlds_stop(mine);
        let route = routes
            .mods
            .iter()
            .find(|(id, _)| id == mod_id)
            .map(|(_, r)| r)?;
        let (glyph, color, headline, reason): (&str, Hsla, SharedString, String) = if building {
            (
                "refresh",
                pal.accent,
                tr("Building").into(),
                tr("The build is running. Cancel it from the card above.").to_string(),
            )
        } else if busy {
            (
                "refresh",
                pal.muted,
                tr("Another build is running").into(),
                tr("This instance's build waits until it finishes.").to_string(),
            )
        } else if self
            .mods
            .unbuilt
            .as_ref()
            .is_some_and(|(of, ids)| of == &instance.id && ids.iter().any(|id| id == mod_id))
        {
            // Its fit was only a prediction: the compile said otherwise, until the next build.
            (
                "information-circle",
                pal.warn,
                native_mods::fit::didnt_build_headline(mod_id).into(),
                if stop.is_some() {
                    native_mods::fit::did_not_build_pending(mod_id)
                } else {
                    native_mods::fit::did_not_build_last(mod_id)
                },
            )
        } else if state == State::UpToDate {
            (
                "tick-02",
                pal.success,
                tr("Nothing to build").into(),
                tr("Your game already has this mod.").to_string(),
            )
        } else {
            match route {
                Route::Invalid { reason } => ("information-circle", pal.warn, tr("Needs attention").into(), reason.clone()),
                Route::Prebuilt => (
                    "flash",
                    pal.accent,
                    tr("Needs no build by itself").into(),
                    tr("This mod uses the game as it ships, so Horizon can start without building when every enabled mod is like this.").to_string(),
                ),
                // Made for another build: it fits this one, and the build compiles it as usual.
                Route::Fits { line } => ("tick-02", pal.success, tr("Fits this game").into(), line.clone()),
                // It stays installed; the build leaves it out and says so.
                Route::Unfit {
                    headline,
                    line,
                    actions,
                } => (
                    "information-circle",
                    pal.warn,
                    headline.clone().into(),
                    match stop.filter(|stake| stake.mods.iter().any(|id| id == mod_id)) {
                        Some(_) => format!("{line}\n{}", native_mods::fit::actions_pending(mod_id)),
                        None => format!("{line}\n{actions}"),
                    },
                ),
            }
        };
        let open = self.mods.route_open.as_deref() == Some(mod_id);
        let id = mod_id.to_owned();
        let chip = crate::keyboard(
            div()
                .id(SharedString::from(format!("route-{mod_id}")))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(28.))
                .rounded(px(8.))
                .bg(color.opacity(if open { 0.24 } else { 0.14 }))
                .cursor_pointer()
                .child(ui::icon(glyph, 16., color)),
            &t,
        )
        .on_click(cx.listener({
            let id = id.clone();
            move |this, _: &ClickEvent, _, cx| {
                this.mods.route_open = match this.mods.route_open.as_deref() {
                    Some(open) if open == id => None,
                    _ => Some(id.clone()),
                };
                cx.notify();
            }
        }));
        let mut wrap = div().relative().flex_none().child(chip);
        if open {
            let home = self.mods.home();
            // A mod the stop names is decided on the panel: turning it off here would skip the
            // backup that keeps its worlds' content.
            let named = self.stop_names(mod_id);
            let off = {
                let id = id.clone();
                ui::menu_item(
                    "route-off",
                    trf("Turn off {mod}", &[("mod", mod_id)]).into(),
                    if named {
                        tr("Choose what to do about your worlds first").to_string()
                    } else {
                        tr("Keeps the file, leaves it out of the game").to_string()
                    },
                    busy || named,
                    &t,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if this.native_step.is_some() || named {
                        return;
                    }
                    this.mods.message =
                        horizon_launcher::browser::java::set_enabled(&home, &id, false)
                            .err()
                            .map(Err);
                    this.mods.route_open = None;
                    this.instance_mods_changed();
                    cx.notify();
                }))
            };
            let worlds_pending = stop.is_some();
            let build_now = ui::menu_item(
                "route-build",
                tr("Build now").into(),
                if worlds_pending {
                    tr("Choose what to do about your worlds first").to_string()
                } else {
                    tr("Builds the game with your mods").to_string()
                },
                busy || state == State::NoMods || worlds_pending,
                &t,
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                if this.native_step.is_some() || worlds_pending {
                    return;
                }
                this.mods.route_open = None;
                if this.settings.native_mods_allowed_at.is_some() {
                    this.build_native();
                } else {
                    this.request_native_build_consent();
                }
                cx.notify();
            }));
            let log = ui::menu_item(
                "route-log",
                tr("Show last log").into(),
                tr("What the last build printed").to_string(),
                false,
                &t,
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.mods.route_open = None;
                this.native_log_open();
                cx.notify();
            }));
            let backing = if ui::light() {
                gpui::white().opacity(0.94)
            } else {
                ui::color(0x16112b).opacity(0.94)
            };
            let menu = ui::popover(280.)
                .bg(backing)
                .id("route-menu")
                .occlude()
                .on_mouse_down_out(cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    this.mods.route_open = None;
                    cx.notify();
                }))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .px(px(12.))
                        .py(px(8.))
                        .child(
                            text_in(14., pal.ink)
                                .font_weight(FontWeight::MEDIUM)
                                .child(headline),
                        )
                        .children(reason.split('\n').map(|paragraph| {
                            text_in(12., pal.muted)
                                .pb(px(4.))
                                .child(paragraph.to_owned())
                        })),
                )
                .child(off)
                .child(build_now)
                .child(log);
            wrap = wrap.child(
                gpui::deferred(
                    div()
                        .absolute()
                        .top(px(34.))
                        .right_0()
                        .child(gpui::anchored().anchor(gpui::Anchor::TopRight).child(menu)),
                )
                .with_priority(1),
            );
        }
        Some(wrap.into_any_element())
    }

    /// Declared game internals beside the mod's route.
    pub(super) fn internals_chip(&self, file: &str, paths: &[String]) -> Option<Stateful<Div>> {
        if paths.is_empty() {
            return None;
        }
        let pal = Pal::of(&self.theme);
        let paths = paths.to_vec();
        let count = paths.len().to_string();
        let background = self.theme.well;
        Some(
            div()
                .id(SharedString::from(format!("internals-{file}")))
                .flex()
                .flex_none()
                .items_center()
                .h(px(18.))
                .px(px(7.))
                .rounded_full()
                .bg(pal.faint)
                .child(
                    text_in(11., pal.muted)
                        .child(trf("Uses game internals ({count})", &[("count", &count)])),
                )
                .tooltip(move |_, cx| {
                    cx.new(|_| NoteTooltip {
                        lines: paths.clone(),
                        ink: pal.ink,
                        background,
                    })
                    .into()
                }),
        )
    }

    /// `control` with `reason` shown on hover: how a disabled control says why.
    pub(super) fn with_reason(&self, control: Stateful<Div>, reason: &str) -> Stateful<Div> {
        let ink = Pal::of(&self.theme).ink;
        let background = self.theme.well;
        let reason = reason.to_owned();
        control.tooltip(move |_, cx| {
            cx.new(|_| NoteTooltip {
                lines: vec![reason.clone()],
                ink,
                background,
            })
            .into()
        })
    }

    /// The kind badge beside a mod's name.
    pub(super) fn kind_badge(&self, kind: ModKind) -> Div {
        let pal = Pal::of(&self.theme);
        div()
            .flex()
            .flex_none()
            .items_center()
            .h(px(18.))
            .px(px(7.))
            .rounded_full()
            .bg(pal.faint)
            .child(text_in(11., pal.muted).child(kind.label()))
    }
}

/// A tooltip of text lines: a mod's declared internals, or why a control is disabled.
struct NoteTooltip {
    lines: Vec<String>,
    ink: Hsla,
    background: Hsla,
}

impl Render for NoteTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .p(px(8.))
            .rounded(px(8.))
            .bg(self.background)
            .children(
                self.lines
                    .iter()
                    .map(|path| text_in(12., self.ink).child(path.clone())),
            )
    }
}

/// Text at `size` px in `color`, in the look's face.
fn text_in(size: f32, color: Hsla) -> Div {
    div()
        .font_family(horizon_theme::glass::FONT)
        .text_size(px(size))
        .line_height(px((size * 1.4).round()))
        .text_color(color)
}
