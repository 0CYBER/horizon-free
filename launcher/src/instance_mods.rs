//! Home's Mods tab (docs/design-system.md, "Night"; plan R0, screens 5 and 6): every mod of the
//! selected instance, the browser's and those put in its folder by hand, with its version, where
//! it comes from, an update when the browser found one, and a switch. A Java instance turns a mod
//! off by renaming its jar (`.disabled`, as `java_instance` does); a Horizon instance's mods are
//! the registry's, switched as the game's Options > Mods does. Adding mods is the Mods page's.
use super::*;
use crate::plate::alpha;
use crate::shell::{c, Action};
use horizon_launcher::browser::{horizon, installed, java, Kind as PackKind, SourceId};
use servers_page::Field;
use std::path::{Path, PathBuf};
use theme::night;

/// A turned-off jar's suffix, which the loaders don't read.
const OFF: &str = ".disabled";

/// A mod as the tab lists it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ModRow {
    /// Its file in the instance, as it is when on (`mods/lumen-0.6.2.jar`).
    file: String,
    name: String,
    version: String,
    on: bool,
    /// What the browser knows of it; None: a file put in by hand.
    entry: Option<installed::Entry>,
}

#[derive(Default)]
pub(super) struct InstanceMods {
    home: Option<PathBuf>,
    fresh: bool,
    reading: bool,
    list: Vec<ModRow>,
    /// The browser was installing at the last draw: its end changes the list.
    was_busy: bool,
}

/// The mods of the instance at `home`: a Java one's jars in `mods/` (on or off), a Horizon one's
/// registry mods. Each jar names itself (`fabric.mod.json`, `quilt.mod.json`...), else its file.
fn read(home: &Path, java_instance: bool) -> Vec<ModRow> {
    let entries: Vec<installed::Entry> = installed::Installed::load(home)
        .entries
        .into_iter()
        .filter(|e| e.kind == PackKind::Mod)
        .collect();
    let mut rows: Vec<ModRow> = if java_instance {
        let Ok(dir) = std::fs::read_dir(home.join("mods")) else {
            return Vec::new();
        };
        dir.flatten()
            .filter_map(|f| {
                let name = f.file_name().to_string_lossy().into_owned();
                let on = name.ends_with(".jar");
                let jar = name.strip_suffix(OFF).unwrap_or(&name);
                (jar.ends_with(".jar") && f.path().is_file()).then(|| {
                    let file = format!("mods/{jar}");
                    let entry = entries.iter().find(|e| e.file == file).cloned();
                    let (name, version) = match &entry {
                        Some(e) => (e.title.clone(), e.number.clone()),
                        None => horizon_port_pack::jar::read(&f.path())
                            .map(|j| (j.name, j.version))
                            .unwrap_or_else(|_| {
                                (jar.trim_end_matches(".jar").into(), String::new())
                            }),
                    };
                    ModRow {
                        file,
                        name,
                        version,
                        on,
                        entry,
                    }
                })
            })
            .collect()
    } else {
        entries
            .into_iter()
            .map(|e| ModRow {
                file: e.file.clone(),
                name: e.title.clone(),
                version: e.number.clone(),
                on: horizon::mod_id(&e).is_some_and(|id| java::is_enabled(home, id)),
                entry: Some(e),
            })
            .collect()
    };
    rows.sort_by_key(|r| r.name.to_lowercase());
    rows
}

/// Turns a Java instance's jar on or off by renaming it.
fn switch_jar(home: &Path, file: &str, on: bool) -> Result<(), String> {
    let (jar, off) = (home.join(file), home.join(format!("{file}{OFF}")));
    let (from, to) = if on { (off, jar) } else { (jar, off) };
    std::fs::rename(&from, &to).map_err(|e| format!("{}: {e}", from.display()))
}

impl Launcher {
    /// Whether the selected instance has mods to show: Horizon's always, a Java one with a loader.
    pub(super) fn has_mods_tab(&self) -> bool {
        self.selected_instance()
            .is_some_and(|i| i.java.as_ref().is_none_or(|j| j.loader.is_some()))
    }

    /// Reads the selected instance's mods off the UI thread, once until something changes them.
    fn load_instance_mods(&mut self, cx: &mut Context<Self>) {
        let Some(i) = self.selected_instance() else {
            return;
        };
        let (home, java_instance) = (i.home.clone(), i.java.is_some());
        let m = &mut self.instance_mods;
        if m.home.as_ref() != Some(&home) {
            *m = InstanceMods {
                home: Some(home.clone()),
                ..InstanceMods::default()
            };
        }
        let busy = self.mods.busy.is_some();
        if m.was_busy && !busy {
            m.fresh = false;
        }
        m.was_busy = busy;
        if m.fresh || m.reading {
            return;
        }
        m.reading = true;
        let task = cx
            .background_executor()
            .spawn(async move { (home.clone(), read(&home, java_instance)) });
        cx.spawn(async move |this, cx| {
            let (home, list) = task.await;
            let _ = this.update(cx, |this, cx| {
                let m = &mut this.instance_mods;
                m.reading = false;
                if m.home.as_ref() == Some(&home) {
                    m.list = list;
                    m.fresh = true;
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// How many mods the selected instance has; None while they are read.
    pub(super) fn instance_mods_count(&mut self, cx: &mut Context<Self>) -> Option<usize> {
        self.mods_load();
        self.load_instance_mods(cx);
        let m = &self.instance_mods;
        m.fresh.then_some(m.list.len())
    }

    fn switch_mod(&mut self, row: &ModRow) {
        let home = self.instance_mods.home.clone().unwrap_or_default();
        let java_instance = self.selected_instance().is_some_and(|i| i.java.is_some());
        match &row.entry {
            Some(e) if !java_instance => self.mods_switch(e),
            _ => {
                self.mods.message = switch_jar(&home, &row.file, !row.on).err().map(Err);
            }
        }
        self.instance_mods.fresh = false;
    }

    /// The Mods tab: its buttons (beside the tabs) and the list.
    pub(super) fn instance_mods_tab(&mut self, cx: &mut Context<Self>) -> (Div, Div) {
        let t = self.theme;
        let n = t.night;
        let busy = self.mods.busy.is_some();
        let updates = self.mods.updates.len();
        let actions = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .children((updates > 0 && !busy).then(|| {
                shell::action("instance-mods-update-all", Action::Raised, &t)
                    .child(shell::label(trf(
                        "Update all ({count})",
                        &[("count", &updates.to_string())],
                    )))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.mods_update_all();
                        cx.notify();
                    }))
            }))
            .child(
                shell::action("instance-mods-add", Action::Raised, &t)
                    .child(shell::label(tr("Add mods").to_string()))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.go(Page::Mods);
                        cx.notify();
                    })),
            );
        let loader = self
            .selected_instance()
            .and_then(|i| i.java.as_ref())
            .map(|j| j.loader.as_ref().map(|l| l.kind.name()));
        let title = match loader {
            Some(Some(loader)) => trf("{loader} mods", &[("loader", loader)]),
            _ => tr("Horizon mods").to_string(),
        };
        let filter = self
            .servers
            .value(Field::InstanceModsFilter)
            .trim()
            .to_lowercase();
        let field = self.field(Field::InstanceModsFilter, "Filter installed", 240., cx);
        let m = &self.instance_mods;
        let rows: Vec<ModRow> = m
            .list
            .iter()
            .filter(|r| filter.is_empty() || r.name.to_lowercase().contains(&filter))
            .cloned()
            .collect();
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(16.))
            .px(px(18.))
            .py(px(14.))
            .child(
                shell::text(night::UI, c(n.ink))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(field);
        let empty = |s: &str| {
            shell::text(night::UI, c(n.muted))
                .px(px(18.))
                .py(px(16.))
                .child(tr(s).to_string())
        };
        let body: Vec<AnyElement> = if !m.fresh {
            vec![empty("Reading the mods…").into_any_element()]
        } else if m.list.is_empty() {
            vec![empty("No mods yet: Add mods finds some for this instance.").into_any_element()]
        } else if rows.is_empty() {
            vec![empty("No mod matches.").into_any_element()]
        } else {
            rows.into_iter()
                .map(|row| self.mod_row(row, busy, cx).into_any_element())
                .collect()
        };
        let card = plate::cut(
            night::PIXEL_CORNER,
            2,
            // Nearly opaque: a list reads over the sunset.
            Some(alpha(n.card, 0.94)),
            Some((alpha(n.stroke, n.stroke_alpha), 1.)),
        )
        .flex()
        .flex_col()
        .child(header)
        .children(body.into_iter().map(|row| {
            div()
                .border_t_1()
                .border_color(alpha(n.stroke, n.rule_alpha))
                .child(row)
        }));
        (
            actions,
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .children(self.mods_busy_plate())
                .child(card),
        )
    }

    fn mod_row(&self, row: ModRow, busy: bool, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let n = t.night;
        // Each mod its own colour, the same every time: a hue from its name.
        let hue = row
            .name
            .bytes()
            .fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
        let icon = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(40.))
            .rounded(px(night::RADIUS_CONTROL))
            .bg(gpui::hsla((hue % 360) as f32 / 360., 0.42, 0.52, 1.))
            .child(
                shell::text(night::TITLE, c(n.on_accent))
                    .font_weight(FontWeight::BOLD)
                    .child(
                        row.name
                            .chars()
                            .next()
                            .unwrap_or('?')
                            .to_uppercase()
                            .to_string(),
                    ),
            );
        let source = match &row.entry {
            Some(e) if e.dependency => trf(
                "{source} · needed by another mod",
                &[("source", e.source.label())],
            ),
            Some(e) => e.source.label().to_string(),
            None => trf(
                "Local file · {file}",
                &[("file", row.file.trim_start_matches("mods/"))],
            ),
        };
        let update = row.entry.as_ref().and_then(|e| {
            let key = (e.source, e.project.clone());
            let number = self.mods.updates.get(&key)?.clone();
            let (source, id): (SourceId, String) = key;
            let chip = div()
                .id(SharedString::from(format!("mod-update-{id}")))
                .flex()
                .flex_none()
                .items_center()
                .h(px(24.))
                .px(px(10.))
                .rounded(px(night::RADIUS_CONTROL))
                .bg(alpha(n.warn, 0.16))
                .cursor_pointer()
                .hover(move |s| s.bg(alpha(n.warn, 0.26)))
                .child(
                    shell::text(night::CAPTION, c(n.warn))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(trf("Update {version}", &[("version", &number)])),
                );
            Some(match busy {
                true => chip.opacity(theme::DISABLED),
                false => chip.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.mods_install(source, id.clone(), None);
                    cx.notify();
                })),
            })
        });
        let switch_row = row.clone();
        div()
            .flex()
            .items_center()
            .gap(px(14.))
            .px(px(18.))
            .py(px(12.))
            .child(icon)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(px(6.))
                            .child(
                                shell::text(night::UI, c(n.ink))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(row.name.clone()),
                            )
                            .child(shell::text(night::CAPTION, c(n.muted)).child(row.version)),
                    )
                    .child(
                        shell::text(night::CAPTION, c(n.muted))
                            .truncate()
                            .child(source),
                    ),
            )
            .children(update)
            .child(
                switch(format!("mod-switch-{}", row.file), row.on, &t).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.switch_mod(&switch_row);
                        cx.notify();
                    },
                )),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_java_instance_lists_its_jars_on_and_off() {
        let home = std::env::temp_dir().join(format!("instance-mods-{}", std::process::id()));
        let mods = home.join("mods");
        std::fs::create_dir_all(&mods).unwrap();
        std::fs::write(mods.join("zeta-1.0.jar"), b"not a zip").unwrap();
        std::fs::write(mods.join("alpha-2.0.jar.disabled"), b"not a zip").unwrap();
        std::fs::write(mods.join("notes.txt"), b"").unwrap();
        let rows = read(&home, true);
        let seen: Vec<_> = rows.iter().map(|r| (r.name.as_str(), r.on)).collect();
        assert_eq!(seen, [("alpha-2.0", false), ("zeta-1.0", true)]);
        switch_jar(&home, "mods/alpha-2.0.jar", true).unwrap();
        assert!(read(&home, true).iter().all(|r| r.on));
        let _ = std::fs::remove_dir_all(&home);
    }
}
