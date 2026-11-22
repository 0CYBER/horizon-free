//! The Settings page in the glass look: the mockup's `SettingsView`. A list of the sections on
//! the left, and the sections as glass cards in a pane that scrolls on its own under the title;
//! the list follows the scroll and takes the pane to a section. The rows inside are the mockup's
//! `SettingRow`s ([`glass_ui::flat_rows`]).
//!
//! Not in the mockup, kept in its patterns: the appearance, the Home background, the data and
//! logs folders and the applications menu (About's rows), the AI clients and the hidden developer options. The mockup's About tiles for Horizon and
//! the launcher open Updates here, beside the switches that decide how updates install. The mockup's Native mods switch is the
//! native mods' build profile here: they are always on.

use super::*;
use crate::glass_ui::{self as ui, white};
use horizon_theme::glass::{ink, radius};

/// The list's entries: name and icon. The settings sections, in the mockup's order.
const SECTIONS: [(&str, &str); 10] = [
    ("General", "sliders-horizontal"),
    ("Appearance", "color-picker"),
    ("Accounts", "user"),
    ("Updates", "refresh"),
    ("Game", "game-controller-01"),
    ("Network", "globe-02"),
    ("Storage", "package-01"),
    ("Data & privacy", "view"),
    ("AI", "ai-chat-02"),
    ("About", "information-circle"),
];

/// What the search finds in each section, in [`SECTIONS`]' order: its rows' names and words a
/// player may look for them by. English; a query matches them and their translation.
const INDEX: [&[&str]; 10] = [
    &[
        "Language",
        "While playing",
        "Show the Horizon logo at startup",
    ],
    &[
        "Theme",
        "Accent",
        "Glass",
        "Blur",
        "Performance mode",
        "Motion",
        "Glow",
        "Gradients",
        "Shine on hover",
        "Effects",
        "Home background",
        "Reset the launcher's look",
        "Dark",
        "Light",
    ],
    &[
        "Microsoft account",
        "Sign in with Microsoft",
        "Sign out",
        "Play as",
        "Skin",
    ],
    &[
        "Install new versions automatically",
        "Delete old versions automatically",
        "Check now",
        "Nightly",
    ],
    &[
        "Game architecture",
        "Arguments",
        "Update mods automatically",
        "Memory",
    ],
    &["Proxy"],
    &["Storage", "Disk space", "Cache"],
    &["Data collection", "Telemetry", "Crash reports"],
    &["No AI", "Agents", "MCP"],
    &[
        "Credits",
        "GitHub",
        "Version",
        "Data folder",
        "Logs",
        "Applications menu",
    ],
];

/// The settings `query` finds: (section, the row's name, or `None` when the section's own name
/// matched). Every word of the query must be in the name, in English or translated.
fn search(query: &str) -> Vec<(usize, Option<&'static str>)> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return Vec::new();
    }
    let finds = |en: &str| {
        let hay = format!("{en} {}", tr(en)).to_lowercase();
        words.iter().all(|w| hay.contains(w.as_str()))
    };
    let mut hits = Vec::new();
    for (i, &(label, _)) in SECTIONS.iter().enumerate() {
        if finds(label) {
            hits.push((i, None));
        }
        hits.extend(INDEX[i].iter().filter(|e| finds(e)).map(|&e| (i, Some(e))));
    }
    hits
}

/// What `settings_page` builds before it knows which look draws it.
pub(super) struct Built {
    pub install: Stateful<Div>,
    pub prune: Stateful<Div>,
    pub opening: Stateful<Div>,
    pub menu_entry: Stateful<Div>,
    pub horizon: String,
    pub horizon_action: Option<AnyElement>,
    pub launcher: String,
    pub launcher_action: Option<AnyElement>,
}

/// A section's card (`hzl-glass rounded-[20px] p-2`).
fn card() -> Div {
    crate::glass::plate(radius::PANEL)
        .flex()
        .flex_col()
        .gap(px(2.))
        .p(px(6.))
}

/// Section `i`'s card, under its heading: the section's icon in the accent and its name.
fn section(i: usize) -> Div {
    let (label, icon) = SECTIONS[i];
    card().child(
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(14.))
            .pt(px(12.))
            .pb(px(4.))
            .child(ui::icon(icon, 18., ui::color(ui::look().accent)))
            .child(
                ui::text(17., FontWeight::SEMIBOLD, ink::FULL)
                    .font_family(horizon_theme::glass::DISPLAY_FONT)
                    .child(tr(label).to_string()),
            ),
    )
}

/// A tile (the mockup's `AboutTile`, `rounded-[14px] bg-white/[0.045] p-5`): its lead, name, a
/// line under it and what it does.
fn tile(lead: AnyElement, title: String, note: String, action: AnyElement) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(140.))
        .rounded(px(radius::LARGE))
        .bg(white(0.045))
        .border_1()
        .border_color(white(ui::edge::REST))
        .p(px(20.))
        .child(div().child(lead))
        .child(
            ui::text(15., FontWeight::SEMIBOLD, ink::FULL)
                .mt(px(16.))
                .child(title),
        )
        .child(
            ui::text(13., FontWeight::NORMAL, 0.6)
                .mt(px(2.))
                .truncate()
                .child(note),
        )
        .child(div().mt(px(16.)).flex().items_start().child(action))
}

/// The scroll spy: which section is being read, given each section's top from the pane's top
/// (`tops`, in order), the pane's `height`, how far it is `scrolled` and how far it can be (`max`).
/// The last section whose top passed a reading line, which sits at 30 % of the pane (the mockup's)
/// and slides down to the pane's bottom over the last 70 % of a pane of scroll: the short last
/// sections, which never reach 30 %, each come in turn as the end nears, instead of the mockup's
/// jump from wherever it was to the last one at the very end.
fn spy(tops: &[f32], height: f32, scrolled: f32, max: f32) -> usize {
    let rest = height * 0.3;
    let travel = height - rest;
    let tail = travel.min(max);
    let line = if tail > 0. {
        rest + travel * ((scrolled - (max - tail)) / tail).clamp(0., 1.)
    } else {
        rest
    };
    tops.iter().rposition(|&top| top <= line + 0.5).unwrap_or(0)
}

fn note(s: &str) -> Option<AnyElement> {
    Some(div().child(tr(s).to_string()).into_any_element())
}

impl Launcher {
    pub(super) fn glass_settings(&mut self, b: Built, cx: &mut Context<Self>) -> Div {
        if crate::glass_storage::count_now(
            self.storage_report.is_some(),
            self.storage_pending,
            self.storage_error.is_some(),
            self.storage_tried == self.storage_visit,
        ) {
            self.refresh_storage(None, false);
        }
        let t = self.theme;
        let sections = ui::flat_rows(|| self.glass_settings_sections(b, cx));
        let query = self.settings_query.text().trim().to_string();
        let searching = !query.is_empty();
        let hits = search(&query);
        // While searching, the list shows what was found and the pane only the cards it is in.
        let mut shown: Vec<usize> = hits.iter().map(|h| h.0).collect();
        shown.dedup();
        let active = if searching {
            usize::MAX
        } else {
            let active = self.settings_section(sections.len());
            self.settings_marked = active;
            active
        };
        let found = !searching || !shown.is_empty();
        let sections: Vec<Div> = if searching {
            sections
                .into_iter()
                .enumerate()
                .filter(|(i, _)| shown.contains(i))
                .map(|(_, d)| d)
                .collect()
        } else {
            sections
        };
        let wide = self.window_width >= crate::glass_home::COMPACT;
        let items: Vec<Stateful<Div>> = if searching {
            hits.iter()
                .enumerate()
                .map(|(k, &(i, row))| {
                    let (label, icon) = SECTIONS[i];
                    let at = shown.iter().position(|&s| s == i).unwrap_or(0);
                    let (title, sub) = match row {
                        Some(row) => (tr(row).to_string(), Some(tr(label).to_string())),
                        None => (tr(label).to_string(), None),
                    };
                    nav_item(
                        format!("settings-hit-{k}"),
                        icon,
                        title,
                        sub,
                        false,
                        wide,
                        &t,
                    )
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            this.settings_pane.scroll_to_top_of_item(at);
                            cx.notify();
                        },
                    ))
                })
                .collect()
        } else {
            SECTIONS
                .iter()
                .enumerate()
                .map(|(i, &(label, icon))| {
                    let title = tr(label).to_string();
                    nav_item(
                        format!("settings-nav-{i}"),
                        icon,
                        title,
                        None,
                        i == active,
                        wide,
                        &t,
                    )
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            this.settings_pane.scroll_to_top_of_item(i);
                            cx.notify();
                        },
                    ))
                })
                .collect()
        };
        let field = self.settings_search_field(&query, cx);
        let nav = if wide {
            card()
                .flex_none()
                .w(px(220.))
                .gap(px(4.))
                .child(field.mb(px(6.)))
                .when(!found, |d| {
                    d.child(
                        ui::text(13., FontWeight::NORMAL, ink::QUIET)
                            .px(px(12.))
                            .py(px(8.))
                            .child(tr("No setting has that name.")),
                    )
                })
                .children(items)
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .flex_none()
                .w_full()
                .gap(px(8.))
                .child(field)
                .child(
                    crate::glass::plate(radius::PANEL)
                        .id("settings-nav-strip")
                        .flex_none()
                        .w_full()
                        .flex_row()
                        .overflow_x_scroll()
                        .on_scroll(|_, _, cx| cx.stop_propagation())
                        .gap(px(4.))
                        .p(px(6.))
                        .children(items),
                )
                .into_any_element()
        };
        let sections = if found {
            sections
        } else {
            vec![card()
                .items_center()
                .gap(px(10.))
                .py(px(48.))
                .child(ui::icon("search-01", 28., white(ink::DIM)))
                .child(ui::text(15., FontWeight::MEDIUM, ink::FULL).child(trf(
                    "Nothing in Settings matches “{query}”",
                    &[("query", &query)],
                )))
                .child(
                    ui::text(13., FontWeight::NORMAL, ink::QUIET)
                        .child(tr("Try another word, or clear the search.")),
                )]
        };
        let pane = div()
            .id("settings-pane")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .track_scroll(&self.settings_pane)
            // A scroll redraws the pane from its layer without rendering the page again, so the
            // list follows it here: on the next frame, once the offset has moved, and only when
            // the section changed.
            .on_scroll(cx.listener(|_, _: &gpui::ScrollEvent, window, cx| {
                cx.on_next_frame(window, |this, _, cx| {
                    if this.settings_section(SECTIONS.len()) != this.settings_marked {
                        cx.notify();
                    }
                });
            }))
            .rounded(px(radius::PANEL))
            .flex()
            .flex_col()
            .gap(px(16.))
            .pb(px(8.))
            .children(sections);
        let content = if wide {
            div()
                .flex()
                .flex_row()
                .items_start()
                .gap(px(12.))
                .flex_1()
                .min_h_0()
                .child(nav)
                .child(pane)
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .flex_1()
                .min_h_0()
                .child(nav)
                .child(pane)
        };
        div()
            .flex()
            .flex_col()
            .w_full()
            .h_full()
            .child(
                // The dialog's head: its name, with room on the right for its close button.
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(px(64.))
                    .pr(px(48.))
                    .child(
                        ui::text(22., FontWeight::SEMIBOLD, ink::FULL)
                            .font_family(horizon_theme::glass::DISPLAY_FONT)
                            .child(tr("Settings").to_string()),
                    ),
            )
            .child(content)
    }

    /// Escape closes the Settings dialog, back to the page under it.
    pub(super) fn settings_escape(&mut self) -> bool {
        if self.theme.glass && self.page == Page::Settings {
            self.go(self.behind);
            return true;
        }
        false
    }

    /// Settings as a dialog over the page the player was on (the glass look): the list and
    /// pane of [`Self::glass_settings`] on a plate, closed by its ✕, a press outside or Escape.
    pub(super) fn settings_dialog(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = self.theme;
        let body = self.settings_page(cx);
        let close = ui::icon_button(
            "settings-close",
            "cancel-01",
            36.,
            18.,
            radius::CONTROL,
            false,
            &t,
        )
        .tooltip(ui::tip(tr("Close")))
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.go(this.behind);
            cx.notify();
        }));
        let card = ui::dialog_plate(radius::MODAL)
            .id("settings-card")
            .occlude()
            .relative()
            .flex()
            .flex_col()
            .w_full()
            .max_w(px(960.))
            .h_full()
            .max_h(px(660.))
            .px(px(18.))
            .pb(px(16.))
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .child(body),
            )
            .child(div().absolute().top(px(14.)).right(px(14.)).child(close));
        div()
            .id("settings-dialog")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .p(px(24.))
            .occlude()
            .on_scroll(|_, _, cx| cx.stop_propagation())
            // Frosted glass over the page the player was on: the blurred sky laid partly over it,
            // so the page shows through softened, then the dimming. Performance mode keeps only
            // the dimming, as the mockup does.
            .when(!self.settings.performance(), |d| {
                d.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .opacity(0.62)
                        .child(ui::scrim_backdrop()),
                )
            })
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .bg(ui::scrim().opacity(if ui::light() { 0.2 } else { 0.4 }))
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, _, cx| {
                            this.go(this.behind);
                            cx.notify();
                        }),
                    ),
            )
            .child(card)
    }

    /// The section the list marks, from where the pane's sections are now ([`spy`]). A section's
    /// bounds are where layout put it, before the scroll (`ScrollHandle` keeps the offset apart):
    /// the scroll is taken off them here. Reading them as they were left the list on the first
    /// section until the pane hit its end.
    fn settings_section(&self, count: usize) -> usize {
        let pane = self.settings_pane.bounds();
        let scrolled = -self.settings_pane.offset().y;
        let tops: Vec<f32> = (0..count)
            .map_while(|i| self.settings_pane.bounds_for_item(i))
            .map(|b| f32::from(b.top() - pane.top() - scrolled))
            .collect();
        spy(
            &tops,
            f32::from(pane.size.height),
            f32::from(scrolled),
            f32::from(self.settings_pane.max_offset().y),
        )
    }

    /// The Game card's switch for registry mods' automatic updates (`horizon::auto`).
    fn mods_auto_update_row(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let on = self.settings.auto_updates_mods();
        ui::setting_row(
            tr("Update registry mods automatically"),
            Some(
                div()
                    .child(tr(
                        "At start and before you play, content installed from Horizon's registry moves to its newest version for this game. Never inside a modpack. Code mods and updates that would remove blocks or items wait for you.",
                    ))
                    .into_any_element(),
            ),
            Some(
                crate::switch("mods-auto-update", on, &t)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.settings.mods_auto_update = Some(!this.settings.auto_updates_mods());
                        this.save_settings();
                        cx.notify();
                    }))
                    .into_any_element(),
            ),
        )
    }

    /// Updates' two tiles (the mockup's `AboutTile`s): Horizon and its update, the launcher and its
    /// own.
    fn update_tiles(
        &mut self,
        horizon: String,
        horizon_action: Option<AnyElement>,
        launcher: String,
        launcher_action: Option<AnyElement>,
    ) -> Div {
        // Named as the Versions page names it; the exact build is on About.
        let installed = self
            .settings
            .selected
            .as_deref()
            .map(|tag| self.version_name(tag));
        let tone = if self.updates.horizon.is_some() {
            ui::Tone::Accent(ui::look().accent_soft)
        } else if self.update_error.is_some() {
            ui::Tone::Warn
        } else if self.checked.is_some() {
            ui::Tone::Live
        } else {
            ui::Tone::Busy
        };
        let ink = match tone {
            ui::Tone::Live => ui::live(),
            ui::Tone::Warn => ui::warn(),
            _ => white(0.7),
        };
        let status = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(px(13.))
            .text_color(ink)
            .child(ui::dot(tone))
            .child(div().min_w_0().truncate().child(horizon));
        let launcher_mark = div()
            .flex()
            .items_center()
            .justify_center()
            .size(px(42.))
            .rounded(px(11.))
            .bg(white(0.08))
            .child(ui::icon("computer", 21., white(0.8)))
            .into_any_element();
        let wide = self.window_width >= crate::glass_home::SIDE_BY_SIDE;
        div()
            .flex()
            .when(wide, |d| d.flex_row())
            .when(!wide, |d| d.flex_col())
            .gap(px(8.))
            .child(tile(
                self.horizon_mark(42.),
                "Horizon".into(),
                installed.unwrap_or_else(|| tr("Nightly").to_string()),
                div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(12.))
                    .child(status)
                    .children(horizon_action)
                    .into_any_element(),
            ))
            .child(tile(
                launcher_mark,
                tr("Launcher").to_string(),
                launcher,
                div().children(launcher_action).into_any_element(),
            ))
    }

    /// The cards, one per entry of [`SECTIONS`], in its order.
    fn glass_settings_sections(&mut self, b: Built, cx: &mut Context<Self>) -> Vec<Div> {
        let t = self.theme;
        let n = 0;
        let account = section(2).child(self.glass_account(cx));
        let updates = section(3)
            .child(self.update_tiles(b.horizon, b.horizon_action, b.launcher, b.launcher_action))
            .child(index_row(
                0,
                tr("Install new versions automatically"),
                note("Your selected version only changes if you were playing the newest one."),
                Some(b.install.into_any_element()),
                &t,
            ))
            .child(index_row(
                0,
                tr("Delete old versions automatically"),
                Some(
                    div()
                        .child(trf(
                            "Keeps the {count} newest, the one in use and the ones your instances use. Deleted versions install again from the Versions page.",
                            &[("count", &self.settings.keep_newest().to_string())],
                        ))
                        .into_any_element(),
                ),
                Some(b.prune.into_any_element()),
                &t,
            ));
        // No light or dark to pick: each theme is one look.
        let appearance = section(1)
            .child(self.launcher_theme_row(cx))
            .child(self.glass_accent_row(cx))
            .child(self.look_slider_row(themes_page::LookKnob::Glass, cx))
            .child(self.look_slider_row(themes_page::LookKnob::Blur, cx))
            .child(self.performance_row(cx))
            .child(self.motion_row(cx))
            .children(self.effects_rows(cx))
            .child(self.home_background_row(cx))
            .child(self.reset_look_row(cx));
        let launcher = section(0)
            .child(self.language_row(cx))
            .child(self.while_playing_row(n, cx))
            .child(index_row(
                0,
                tr("Show the Horizon logo at startup"),
                note("Shown for a moment while the version list loads."),
                Some(b.opening.into_any_element()),
                &t,
            ));
        let network = section(5).child(self.proxy_row(n, cx));
        let game = section(4)
            .child(self.game_architecture_row(n, cx))
            .child(self.game_args_row(n + 1, cx))
            .child(self.mods_auto_update_row(cx));
        let ai = section(8)
            .child(self.no_ai_row(n, cx))
            .children(self.agents_section(cx));
        let mono = |s: String| {
            div()
                .font_family(theme::MONO)
                .text_size(px(12.))
                .child(s)
                .into_any_element()
        };
        let about = section(9)
            .child(self.version_row(&t))
            .child(self.location_row(0, cx).mb(px(0.)))
            .child(index_row(
                0,
                tr("Logs"),
                Some(mono(format!(
                    "{}  ·  {}",
                    short(&backend::logs_dir()),
                    tr("crash reports beside it")
                ))),
                Some(
                    button("open-logs", "Open", Kind::Secondary, &t)
                        .on_click(|_, _, cx| {
                            let dir = backend::logs_dir();
                            let _ = std::fs::create_dir_all(&dir);
                            crate::glass_toast::open(cx, &dir);
                        })
                        .into_any_element(),
                ),
                &t,
            ))
            .when(cfg!(target_os = "linux"), |d| {
                d.child(index_row(
                    0,
                    tr("Applications menu"),
                    note(if self.settings.desktop_entry() {
                        "Horizon and its icon are in your desktop's menu."
                    } else {
                        "Not in your desktop's menu."
                    }),
                    Some(b.menu_entry.into_any_element()),
                    &t,
                ))
            });
        vec![
            launcher,
            appearance,
            account,
            updates,
            game,
            network,
            section(6).child(self.glass_storage(cx)),
            section(7).child(self.collection_section(cx)),
            ai,
            about,
        ]
    }

    /// About's version line, with a button that copies it (for a bug report).
    fn version_row(&self, t: &Theme) -> Div {
        let nightly = tr("Nightly").to_string();
        let line = trf(
            "Launcher {launcher}, game {game}",
            &[
                ("launcher", env!("CARGO_PKG_VERSION")),
                (
                    "game",
                    self.selected().map_or(nightly.as_str(), String::as_str),
                ),
            ],
        )
        .to_string();
        let copied = line.clone();
        index_row(
            0,
            tr("Version"),
            Some(div().child(line).into_any_element()),
            Some(
                button("copy-version", "Copy", Kind::Secondary, t)
                    .on_click(move |_, _, cx| {
                        copy_to_clipboard(cx, gpui::ClipboardItem::new_string(copied.clone()))
                    })
                    .into_any_element(),
            ),
            t,
        )
    }
}

/// An entry of the settings list: a section, or while searching a setting found (its section
/// under it). The one being read is tinted with the accent, outlined in it and marked by a bar.
pub(super) fn nav_item(
    id: String,
    icon: &'static str,
    title: String,
    sub: Option<String>,
    on: bool,
    wide: bool,
    t: &Theme,
) -> Stateful<Div> {
    let id = SharedString::from(id);
    let accent = ui::look().accent;
    let soft = ui::look().accent_soft;
    let d = div()
        .id(id.clone())
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(if wide { 12. } else { 8. }))
        .min_h(px(if wide { 40. } else { 34. }))
        .py(px(if sub.is_some() { 6. } else { 0. }))
        .px(px(if wide { 12. } else { 10. }))
        .rounded(px(radius::ROW))
        .border_1()
        .cursor_pointer()
        .font_family(horizon_theme::glass::FONT)
        .text_size(px(14.))
        .active(|s| s.bg(white(0.1)));
    let d = if on {
        d.bg(ui::color(accent).opacity(0.16))
            .border_color(ui::color(soft).opacity(0.5))
            .font_weight(FontWeight::MEDIUM)
            .text_color(white(ink::FULL))
            .child(
                div()
                    .absolute()
                    .left(px(-1.))
                    .top(px(10.))
                    .bottom(px(10.))
                    .w(px(3.))
                    .rounded_full()
                    .bg(ui::color(soft)),
            )
    } else {
        d.border_color(gpui::transparent_black())
            .text_color(white(0.7))
            .hover(|s| s.text_color(white(ink::FULL)))
            .child(ui::hover_edge(&id, radius::ROW, 0.06, true))
    };
    let d = d
        .child(
            ui::icon(icon, 17., if on { ui::color(soft) } else { white(0.65) })
                .relative()
                .flex_none(),
        )
        .child(
            div()
                .relative()
                .flex()
                .flex_col()
                .min_w_0()
                .child(div().truncate().child(title))
                .children(sub.map(|s| {
                    ui::text(12., FontWeight::NORMAL, ink::QUIET)
                        .truncate()
                        .child(s)
                })),
        );
    keyboard(track_hover(d, id), t)
}

impl Launcher {
    /// The search over the settings list: what it holds filters the list and the pane. Escape
    /// clears it, then leaves it.
    fn settings_search_field(&mut self, query: &str, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = self.theme;
        let focused = self.settings_query_focused;
        let soft = ui::look().accent_soft;
        let field = div()
            .id("settings-search")
            .track_focus(&self.settings_query_focus)
            .relative()
            .flex()
            .items_center()
            .gap(px(10.))
            .w_full()
            .h(px(40.))
            .pl(px(12.))
            .pr(px(6.))
            .overflow_hidden()
            .rounded(px(radius::ROW))
            .bg(white(if focused { 0.11 } else { 0.06 }))
            .border_1()
            .border_color(if focused {
                ui::color(soft).opacity(0.7)
            } else {
                white(ui::edge::REST)
            })
            .child(ui::back_edge(radius::ROW, 0., true))
            .cursor_text()
            .font_family(horizon_theme::glass::FONT)
            .text_size(px(14.))
            .child(ui::icon(
                "search-01",
                16.,
                if focused {
                    ui::color(soft)
                } else {
                    white(ink::QUIET)
                },
            ))
            .child(div().flex_1().min_w_0().child(line_edit::content_with(
                &self.settings_query,
                &t,
                focused,
                false,
                false,
                tr("Search settings").into(),
            )))
            .when(!query.is_empty(), |d| {
                d.child(
                    ui::icon_button(
                        "settings-search-clear",
                        "cancel-01",
                        26.,
                        14.,
                        8.,
                        false,
                        &t,
                    )
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.settings_query.set(String::new());
                        cx.stop_propagation();
                        cx.notify();
                    })),
                )
            })
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                window.focus(&this.settings_query_focus, cx);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                match line_edit::handle(&mut this.settings_query, ev, cx) {
                    line_edit::Key::Pass => return,
                    line_edit::Key::Escape => {
                        if this.settings_query.text().is_empty() {
                            window.blur(cx);
                        } else {
                            this.settings_query.set(String::new());
                        }
                    }
                    line_edit::Key::Edited => this.settings_pane.scroll_to_item(0),
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }));
        line_edit::mouse(field, cx, |this| &mut this.settings_query)
    }
}

#[cfg(test)]
mod search_tests {
    use super::{search, SECTIONS};

    #[test]
    fn search_finds_rows_and_sections_by_every_word() {
        let look = SECTIONS.iter().position(|s| s.0 == "Appearance").unwrap();
        assert!(search("blur").contains(&(look, Some("Blur"))));
        assert!(search("APPEAR").contains(&(look, None)));
        assert!(search("old delete")
            .iter()
            .any(|h| h.1 == Some("Delete old versions automatically")));
        assert!(search("zzzz nothing").is_empty());
        assert!(search("   ").is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::spy;

    /// A 600 px pane over the eight settings cards: tall ones, then About, too short for its
    /// top ever to reach 30 % of the pane.
    const TOPS: [f32; 8] = [0., 420., 700., 1000., 1300., 1600., 1900., 2200.];
    const HEIGHT: f32 = 600.;
    const MAX: f32 = 2400. - HEIGHT;

    fn at(scrolled: f32) -> usize {
        let tops: Vec<f32> = TOPS.iter().map(|t| t - scrolled).collect();
        spy(&tops, HEIGHT, scrolled, MAX)
    }

    #[test]
    fn the_list_follows_the_section_read_at_30_percent() {
        assert_eq!(at(0.), 0);
        assert_eq!(
            at(300.),
            1,
            "Updates' top is 120 px down: in the upper 30 %"
        );
        assert_eq!(at(600.), 2);
        assert_eq!(
            at(1000.),
            3,
            "Network's top at the pane's top, Game's 300 px down"
        );
        assert_eq!(at(1150.), 4, "Game's top 150 px down: in the upper 30 %");
    }

    #[test]
    fn near_the_end_the_short_last_section_is_marked_for_a_stretch() {
        let seen: Vec<usize> = (0..=MAX as usize)
            .step_by(10)
            .map(|s| at(s as f32))
            .collect();
        // Every section is marked on the way down, in order, never skipped.
        let mut order = seen.clone();
        order.dedup();
        assert_eq!(order, (0..8).collect::<Vec<_>>());
        // About is marked for a stretch of scroll before the end, not for one step.
        let about = seen.iter().filter(|&&s| s == 7).count();
        assert!(about >= 8, "About marked for {about} steps of 10 px");
        assert_eq!(at(MAX), 7);
    }

    #[test]
    fn a_pane_that_does_not_scroll_marks_the_first() {
        assert_eq!(spy(&[0., 200.], 600., 0., 0.), 0);
        assert_eq!(spy(&[], 600., 0., 0.), 0);
    }
}
