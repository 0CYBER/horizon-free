//! Home's Mods tab in the glass look (the mockup's `InstanceMods`, `launcher-views.tsx`): "N mods
//! in {instance}" and + Find mods over one `ROW` per mod, its icon, name and "version · source".
//! The launcher's own controls fit in: the filter and Update all in the head, a mod's update
//! and its on/off `Toggle` at the row's end. Dark appearance only.
use super::*;
use crate::glass_ui::{self as ui, white};
use build_mode::ModKind;
use glass_home_lists::Listener;
use horizon_launcher::github_mods;
use horizon_theme::glass::{ink, radius};
use instance_mods::ModRow;
use servers_page::Field;

impl Launcher {
    /// Home's Mods tab, for the selected instance.
    pub(super) fn glass_instance_mods(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let busy = self.mods.busy.is_some();
        let updates = self.mods.updates.len();
        let name = self
            .selected_instance()
            .map(|i| i.name.clone())
            .unwrap_or_default();
        let m = &self.instance_mods;
        let (fresh, count) = (m.fresh, m.list.len());
        let mut body = div().flex().flex_col();
        body = body.children(self.mods_busy_plate());
        if !fresh {
            return body.child(ui::empty().child(tr("Reading the mods…")));
        }
        if count == 0 {
            return body.child(
                ui::empty()
                    .flex()
                    .gap(px(4.))
                    .child(trf("No mods in {name} yet.", &[("name", &name)]))
                    .child(ui::link("instance-mods-find", tr("Find mods")).on_click(find_mods(cx))),
            );
        }
        let note = match count {
            1 => trf("1 mod in {name}", &[("name", &name)]),
            _ => trf(
                "{count} mods in {name}",
                &[("count", &count.to_string()), ("name", &name)],
            ),
        };
        let filter = self
            .servers
            .value(Field::InstanceModsFilter)
            .trim()
            .to_lowercase();
        let field = self.field(Field::InstanceModsFilter, tr("Filter installed"), 220., cx);
        let update_all = (updates > 0 && !busy).then(|| {
            ui::quiet(
                "instance-mods-update-all",
                trf("Update all ({count})", &[("count", &updates.to_string())]),
                36.,
                &t,
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.mods_update_all();
                cx.notify();
            }))
        });
        body = body.child(
            ui::list_head(note).child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(field)
                    .children(update_all)
                    .child(
                        ui::quiet_icon(
                            "instance-mods-add",
                            Some("plus-sign"),
                            tr("Find mods"),
                            36.,
                            &t,
                        )
                        .on_click(find_mods(cx)),
                    ),
            ),
        );
        let native = self.mods.java.is_none()
            && self.instance_mods.list.iter().any(|r| {
                r.native.is_some()
                    || matches!(r.kind, Some((ModKind::Native, _)))
                    || r.entry
                        .as_ref()
                        .is_some_and(|e| native_status::is_native_file(&e.file))
            });
        body = body.children(native.then(|| self.native_build(cx)).flatten());
        // Mods the last build left out stay installed: the build says which, and why.
        let left_out = if native {
            self.native_left_out()
        } else {
            Vec::new()
        };
        // Each with Switch off, which stops the build trying it until its author updates it.
        let t = self.theme;
        let lines: Vec<Div> = left_out
            .iter()
            .map(|left| {
                let row = self
                    .instance_mods
                    .list
                    .iter()
                    .find(|r| {
                        r.on && (r.native.as_ref().is_some_and(|n| n.id == left.id)
                            || r.kind.as_ref().is_some_and(|(kind, id)| {
                                *kind == ModKind::Native && *id == left.id
                            }))
                    })
                    .cloned();
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        ui::text(13., FontWeight::NORMAL, 1.)
                            .text_color(ui::warn())
                            .flex_1()
                            .min_w_0()
                            .child(left.label()),
                    )
                    .children(row.map(|row| {
                        ui::quiet(
                            SharedString::from(format!("left-off-{}", left.id)),
                            tr("Switch off"),
                            32.,
                            &t,
                        )
                        .on_click(cx.listener(
                            move |this, _: &ClickEvent, _, cx| {
                                match &row.native {
                                    Some(native) => this.switch_native_mod(native, true),
                                    None => this.switch_mod(&row),
                                }
                                cx.notify();
                            },
                        ))
                    }))
            })
            .collect();
        body = body.children((!lines.is_empty()).then(|| self.panel().gap(px(8.)).children(lines)));
        let rows: Vec<ModRow> = self
            .instance_mods
            .list
            .iter()
            .filter(|r| filter.is_empty() || r.name.to_lowercase().contains(&filter))
            .cloned()
            .collect();
        if rows.is_empty() {
            return body.child(ui::empty().child(tr("No mod matches.")));
        }
        // A modpack has hundreds: only the rows near the view are built.
        let shown = (rows.len(), filter);
        if self.instance_mods.shown != shown {
            self.instance_mods.shown = shown;
            self.instance_mods.cull.reset();
        }
        let (cull, scroll) = (
            self.instance_mods.cull.clone(),
            self.content_scroll.handle.clone(),
        );
        let list = cull.column(
            &scroll,
            rows.len(),
            0.,
            |_| false,
            |i| {
                self.glass_mod_row(rows[i].clone(), &left_out, busy, cx)
                    .into_any_element()
            },
        );
        body.child(list)
    }

    fn glass_mod_row(
        &mut self,
        row: ModRow,
        left_out: &[native_mods::LeftOut],
        busy: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = self.theme;
        let key = SharedString::from(format!("mod-{}", row.file));
        let hovered = self.list_hover.as_ref() == Some(&key);
        let hover = self.list_hover_listener(key.clone(), cx);
        // `PixelIcon` at 36, round at `round(36 · 0.26)`; the lists' plain tile without one.
        let known = row.icon.is_some()
            || row
                .entry
                .as_ref()
                .is_some_and(|e| self.project_icon(e).is_some());
        let icon = if known {
            div()
                .relative()
                .flex_none()
                .size(px(36.))
                .rounded(px(9.))
                .overflow_hidden()
                .child(self.mod_icon(row.entry.as_ref(), row.icon.clone(), 36.))
                .into_any_element()
        } else {
            glass_home_lists::picture(None, "puzzle")
        };
        let source = if let Some(error) = &row.error {
            error.clone()
        } else if let Some(origin) = row.native.as_ref().and_then(|n| n.origin.as_ref()) {
            trf("from GitHub: {source}", &[("source", &origin.label())])
        } else {
            match &row.entry {
                Some(e) if e.dependency => trf(
                    "{source} · needed by another mod",
                    &[("source", e.source.label())],
                ),
                Some(e) => e.source.label().to_string(),
                None => tr("Local file").to_string(),
            }
        };
        let note = match row.version.is_empty() {
            true => source,
            false => format!("{} · {source}", row.version),
        };
        let update = row.entry.as_ref().and_then(|e| {
            let key = (e.source, e.project.clone());
            let number = self.mods.updates.get(&key)?.clone();
            let (source, id) = key;
            let chip = ui::quiet(
                SharedString::from(format!("mod-update-{id}")),
                trf("Update {version}", &[("version", &number)]),
                32.,
                &t,
            );
            Some(match busy {
                true => chip.opacity(0.4).cursor_default(),
                false => chip.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.mods_install(source, id.clone(), None);
                    cx.notify();
                })),
            })
        });
        let status = self.mod_status(&row, left_out, cx);
        // A GitHub mod keeps its Update (or Check for updates) and Source on the row.
        let github = row
            .native
            .as_ref()
            .and_then(|n| n.origin.as_ref())
            .map(|origin| self.github_actions(origin, busy || self.github.busy(), cx));
        let menu = self.mod_menu(&row, busy, cx);
        let switch_row = row.clone();
        let controls = div().child(
            ui::toggle(
                SharedString::from(format!("mod-switch-{}", row.file)),
                row.on,
                self.look.accent,
                self.look.on_accent,
                &t,
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                match &switch_row.native {
                    Some(native) => this.switch_native_mod(native, switch_row.on),
                    None => this.switch_mod(&switch_row),
                }
                cx.notify();
            })),
        );
        ui::list_row(key, hovered, hover, &t)
            .child(icon)
            .child(ui::list_text(row.name.clone(), note))
            .children(row.kind.as_ref().map(|(kind, _)| {
                self.with_reason(
                    kind_pill(*kind).id(SharedString::from(format!("kind-{}", row.file))),
                    kind.hint(),
                )
            }))
            .child(status.relative())
            .children(update.map(|u| u.relative()))
            .children(github.map(|g| g.relative()))
            .child(
                controls
                    .relative()
                    .flex()
                    .items_center()
                    .rounded(px(radius::CONTROL)),
            )
            .child(menu)
            .text_color(white(1.))
    }

    /// The row's one status: a dot and a short word, in the colour of what it says. Each native
    /// mod has its own: only the mods that changed need a build, and a failed build marks only
    /// the mods that stopped it.
    fn mod_status(
        &self,
        row: &ModRow,
        left_out: &[native_mods::LeftOut],
        cx: &mut Context<Self>,
    ) -> Div {
        let id = row
            .kind
            .as_ref()
            .filter(|(kind, _)| *kind == ModKind::Native)
            .map(|(_, id)| id.clone())
            .or_else(|| row.native.as_ref().map(|n| n.id.clone()));
        let mine = self
            .selected_instance()
            .is_some_and(|i| self.native_for.as_deref() == Some(i.id.as_str()));
        let unbuilt = id.as_deref().is_some_and(|id| self.native_mod_unbuilt(id));
        let left = id
            .as_deref()
            .and_then(|id| left_out.iter().find(|l| l.id == id));
        let (word, color) = match id.as_deref() {
            _ if !row.on => (tr("Off"), white(ink::QUIET)),
            _ if row.error.is_some() => (tr("Failed"), ui::danger()),
            None => (tr("Ready"), ui::live()),
            // The last build left it out: the card above says why, with Switch off.
            Some(_) if left.is_some() => (tr("Left out"), ui::warn()),
            Some(id) if mine && self.native_failed_mods.iter().any(|f| f == id) => {
                (tr("Failed"), ui::danger())
            }
            Some(_) if mine && unbuilt && self.native_step.is_some() => {
                (tr("Building"), ui::color(self.look.accent))
            }
            // Native mods run as real code: nothing builds until the player allows them.
            Some(_) if self.settings.native_mods_allowed_at.is_none() => {
                (tr("Needs permission"), ui::warn())
            }
            Some(id)
                if self.native_routes(cx).mods.iter().any(|(m, route)| {
                    m == id && matches!(route, native_mods::Route::Invalid { .. })
                }) =>
            {
                (tr("Does not fit"), ui::warn())
            }
            Some(_) if unbuilt => (tr("Needs build"), ui::warn()),
            Some(_) => (tr("Ready"), ui::live()),
        };
        let shown = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .child(div().size(px(6.)).rounded_full().bg(color))
            .child(
                ui::text(12., FontWeight::MEDIUM, 1.)
                    .text_color(color)
                    .child(word),
            )
            .id(SharedString::from(format!("mod-status-{}", row.file)));
        // Nothing here grants the permission: say where it is asked for.
        let shown = match left {
            Some(left) => self.with_reason(shown, &left.label()),
            None if word == tr("Needs permission") => self.with_reason(
                shown,
                tr("Native mods need your permission before they build. Build asks you first."),
            ),
            None => shown,
        };
        // A native mod's word opens what its chip says on the old look: how it reaches the game
        // (fits, exact build only, does not fit, did not build) and what can be done.
        let facts = id
            .as_deref()
            .filter(|_| row.on)
            .and_then(|id| self.route_facts(id, cx).map(|facts| (id.to_owned(), facts)));
        let Some((id, facts)) = facts else {
            return div().child(shown);
        };
        let open = self.mods.route_open.as_deref() == Some(id.as_str());
        let toggle = id.clone();
        let button = crate::keyboard(
            div()
                .id(SharedString::from(format!("mod-route-{}", row.file)))
                .flex()
                .items_center()
                .h(px(28.))
                .px(px(8.))
                .rounded(px(radius::SMALL))
                .cursor_pointer()
                .bg(facts.color().opacity(if open { 0.2 } else { 0. }))
                .hover(|s| s.bg(white(0.07)))
                .child(shown),
            &self.theme,
        )
        .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
            if this.closed_by_this_press(event) {
                return;
            }
            this.mods.route_open = match this.mods.route_open.as_deref() {
                Some(open) if open == toggle => None,
                _ => Some(toggle.clone()),
            };
            cx.notify();
        }));
        let menu = open.then(|| {
            gpui::deferred(
                div().absolute().top(px(32.)).right_0().child(
                    gpui::anchored()
                        .anchor(gpui::Anchor::TopRight)
                        .child(self.route_menu(&id, &facts, cx)),
                ),
            )
            .with_priority(1)
        });
        div().relative().child(button).children(menu)
    }

    /// A native mod's switch, with its consent step.
    fn switch_native_mod(&mut self, native: &native_status::Installed, on: bool) {
        if !on && self.settings.native_mods_allowed_at.is_none() {
            self.pending_consent = Some(PendingConsent::Local(native.id.clone()));
            return;
        }
        let home = self.mods.home();
        let Some(_lock) = horizon_launcher::browser::try_lock_home(&home) else {
            self.mods.message = Some(Err(tr("Wait for the current operation to finish.").into()));
            return;
        };
        self.mods.message = horizon_launcher::browser::java::set_enabled(&home, &native.id, !on)
            .err()
            .map(Err);
        self.instance_mods_changed();
    }

    /// The ⋮ button and, when open, its menu.
    fn mod_menu(&mut self, row: &ModRow, busy: bool, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let id = format!("mod-menu:{}", row.file);
        let open = self
            .mods
            .route_open
            .as_deref()
            .is_some_and(|o| o.starts_with(&id));
        let confirming = self.mods.route_open.as_deref() == Some(format!("{id}:remove").as_str());
        let again = format!("{id}:remove");
        let button = ui::icon_button(
            format!("mod-more-{}", row.file),
            "more-vertical",
            32.,
            18.,
            radius::SMALL,
            open,
            &t,
        )
        .tooltip(ui::tip(tr("More for this mod")))
        .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
            if this.closed_by_this_press(event) {
                return;
            }
            this.mods.route_open = match this.mods.route_open.as_deref() {
                Some(o) if o.starts_with(&id) => None,
                _ => Some(id.clone()),
            };
            cx.notify();
        }));
        let mut wrap = div().relative().flex_none().child(button);
        if !open {
            return wrap;
        }
        let home = self.mods.home();
        let path = home.join(&row.file);
        let folder = path.parent().map_or(home, |p| p.to_path_buf());
        let item = |name: &str, label: SharedString, note: String, locked: bool| {
            ui::menu_item(format!("mod-menu-{name}"), label, note, locked, &t)
        };
        let mut menu = ui::popover(260.)
            .bg(if ui::light() {
                gpui::white().opacity(0.94)
            } else {
                ui::color(0x16112b).opacity(0.94)
            })
            .id("mod-menu")
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _: &MouseDownEvent, _, cx| {
                this.mods.route_open = None;
                cx.notify();
            }))
            .child(
                item(
                    "folder",
                    tr("Open folder").into(),
                    tr("Where the file is").to_string(),
                    false,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.mods.route_open = None;
                    cx.open_with_system(&folder);
                    cx.notify();
                })),
            );
        if row.native.is_some() || matches!(row.kind, Some((ModKind::Native, _))) {
            menu = menu.child(
                item(
                    "log",
                    tr("View build log").into(),
                    tr("What the last build printed").to_string(),
                    false,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.mods.route_open = None;
                    this.native_log_open();
                    cx.notify();
                })),
            );
        }
        // What it reaches inside the game: off the row, never hidden (it runs as real code).
        if !row.internals.is_empty() {
            menu = menu.child(item(
                "internals",
                trf(
                    "Uses game internals ({count})",
                    &[("count", &row.internals.len().to_string())],
                )
                .into(),
                row.internals.join(", "),
                true,
            ));
        }
        menu = menu.child(
            item("file", tr("Show file").into(), row.file.clone(), false).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    this.mods.route_open = None;
                    cx.reveal_path(&path);
                    cx.notify();
                },
            )),
        );
        let label = if confirming {
            tr("Remove for good")
        } else {
            tr("Remove")
        };
        let doomed = row.clone();
        menu = menu.child(
            item(
                "remove",
                label.into(),
                tr("Takes it out of this instance").to_string(),
                busy,
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                if confirming {
                    this.remove_mod_row(&doomed);
                    this.mods.route_open = None;
                } else {
                    this.mods.route_open = Some(again.clone());
                }
                cx.notify();
            })),
        );
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
        wrap
    }

    /// Takes a mod out of the instance, whatever put it there.
    fn remove_mod_row(&mut self, row: &ModRow) {
        let home = self.mods.home();
        let result = match (&row.native, &row.entry) {
            (Some(n), _) if n.origin.is_some() => {
                let Some(_lock) = horizon_launcher::browser::try_lock_home(&home) else {
                    self.mods.message =
                        Some(Err(tr("Wait for the current operation to finish.").into()));
                    return;
                };
                github_mods::remove(&home, &n.id).map(|_| ())
            }
            (None, Some(e)) => {
                // `mods_remove` says what it did itself.
                let _ = self.mods_remove(&e.project);
                self.instance_mods_changed();
                return;
            }
            _ => std::fs::remove_file(home.join(&row.file)).map_err(|e| e.to_string()),
        };
        self.mods.message = Some(result.map(|()| trf("Removed {name}.", &[("name", &row.name)])));
        self.instance_mods_changed();
    }
}

/// Find mods: the Mods page, to add some to this instance.
fn find_mods(cx: &mut Context<Launcher>) -> Listener<ClickEvent> {
    Box::new(cx.listener(|this, _: &ClickEvent, _, cx| {
        this.go(Page::Mods);
        cx.notify();
    }))
}

/// What a Horizon mod is (Native, WebAssembly, Content), as the old look's badge says it: a
/// native one is real code, never hidden.
fn kind_pill(kind: ModKind) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .h(px(20.))
        .px(px(8.))
        .rounded_full()
        .bg(white(0.07))
        .child(ui::text(11., FontWeight::MEDIUM, 0.6).child(kind.label()))
}

impl Launcher {
    /// Under Home's head: a build that stopped or was refused, with its remedy, even when the
    /// Mods tab is not the one open (Build and play starts from the Play button).
    pub(super) fn glass_build_notice(&self, cx: &mut Context<Self>) -> Option<Div> {
        let mine = self
            .selected_instance()
            .is_some_and(|i| self.native_for.as_deref() == Some(i.id.as_str()));
        if !mine
            || self.home_tab == home_tabs::HomeTab::Mods
            || (self.native_error.is_none() && self.native_worlds.is_none())
        {
            return None;
        }
        let card = self.native_build(cx)?;
        Some(crate::glass::plate(radius::PANEL).p(px(20.)).child(card))
    }
}
