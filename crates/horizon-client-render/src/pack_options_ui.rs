//! The Pack Options screen (#11486), on the settings shell (`settings_shell`): the config entries
//! of the selected resource packs' Polytone `config_entries` (wiki "Polytone Configs"). Each
//! `section` is a page, named by the packs' own translations; numbers are sliders that stop at
//! the entry's `step`, value lists are choices, booleans switch and free text reads. The first
//! page starts with the pack-wide presets and Reset all. The shell gives it the keyboard, a
//! controller's focus, the search, the strip's descriptions and Undo; leaving keeps what differs
//! from how the screen opened, and the packs load again with it.
use crate::{
    lang,
    settings::Settings,
    settings_shell::{self as shell, Group, Output, Row, Sheet, Value},
};
use horizon_polytone::pack::{ConfigKind, ConfigValue, Configs};

/// A row: the pack-wide presets, Reset all, or the entry at an index of `ids`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Item {
    Presets,
    ResetAll,
    Entry(u16),
}

const TITLE: &str = "horizon.pack.options.title";
/// The first page's group of rows for the whole pack, and every page's group of entries.
const WHOLE_PACK: &str = "Whole pack";
const PACK_OPTIONS: &str = "Pack settings";
const RESET_ALL: &str = "Reset all";
const RESET_ABOUT: &str = "Puts every option of this pack back to its default.";
const AT_DEFAULTS: &str = "Every option is at its default already.";
const PRESETS_ABOUT: &str = "Sets many options at once, as the pack groups them.";
const PACK_SETTINGS: &str = "Settings made by the pack's author: they change only this pack.";
/// A pack's tooltips, with the performance line after them, run longer than Horizon's own.
const NOTE_LINES: usize = 3;

/// A translation, or `fallback` where no language has the key.
fn translated(key: &str, fallback: impl FnOnce() -> String) -> String {
    if lang::has(key) {
        lang::get(key)
    } else {
        fallback()
    }
}

/// `namespace:path` split, `minecraft` when there is no namespace.
fn split(id: &str) -> (&str, &str) {
    id.split_once(':').unwrap_or(("minecraft", id))
}

/// A name for an id without a translation: `fog_density` is "Fog density".
fn readable(path: &str) -> String {
    let words = path.rsplit('/').next().unwrap_or(path).replace('_', " ");
    let mut chars = words.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// A number as its entry shows it: whole with a whole step, else up to two decimals.
fn number_text(n: f64, step: f64) -> String {
    if step >= 1.0 && step.fract() == 0.0 {
        format!("{n:.0}")
    } else {
        let text = format!("{n:.2}");
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    }
}

/// The screen: the entries being edited, as they were when it opened (for Undo and for whether
/// leaving changes anything), and how they are paged.
#[derive(Default)]
pub(crate) struct PackOptionsScreen {
    /// The selected packs' entries and the values being picked.
    pub(crate) configs: Configs,
    opened: Configs,
    /// Entry ids, by `Item::Entry`'s index.
    ids: Vec<String>,
    /// The sections in order: the `section` id (none for entries without one) and its entries.
    sections: Vec<(Option<String>, Vec<u16>)>,
    /// Bumped on every change, for `stamp`: the values are floats, which don't hash.
    revision: u64,
}

impl PackOptionsScreen {
    /// The screen opens on `configs`, the selected packs' entries with the player's values.
    pub(crate) fn open(configs: Configs) -> Self {
        let mut ids: Vec<(String, Option<String>, i64)> = configs
            .entries()
            .iter()
            .map(|(id, e)| (id.clone(), e.section.clone(), e.display_order))
            .collect();
        ids.sort_by(|a, b| a.2.cmp(&b.2).then_with(|| a.0.cmp(&b.0)));
        // A section's place: the smallest `section_order` among its entries, else its name.
        let mut sections: Vec<(Option<String>, Option<i64>, Vec<u16>)> = Vec::new();
        for (row, (id, section, _)) in ids.iter().enumerate() {
            let order = configs.entries()[id].section_order;
            match sections.iter_mut().find(|s| &s.0 == section) {
                Some(s) => {
                    s.1 = match (s.1, order) {
                        (Some(a), Some(b)) => Some(a.min(b)),
                        (a, b) => a.or(b),
                    };
                    s.2.push(row as u16);
                }
                None => sections.push((section.clone(), order, vec![row as u16])),
            }
        }
        sections.sort_by(|a, b| match (a.1, b.1) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.0.cmp(&b.0),
        });
        Self {
            opened: configs.clone(),
            configs,
            ids: ids.into_iter().map(|(id, ..)| id).collect(),
            sections: sections.into_iter().map(|(s, _, rows)| (s, rows)).collect(),
            revision: 0,
        }
    }

    /// What leaving sends: the entries and the player's choices, if they differ from how the
    /// screen opened (an Undo, or a value stepped back, leaves nothing to send).
    pub(crate) fn finished(&self) -> Option<crate::options_ui::Event> {
        (self.configs != self.opened).then(|| crate::options_ui::Event::PackOptions {
            entries: self.configs.entries().keys().cloned().collect(),
            chosen: self.configs.chosen(),
        })
    }

    /// The namespace the presets' translations are under: of the first entry with presets.
    fn presets_namespace(&self) -> &str {
        self.configs
            .entries()
            .iter()
            .find(|(_, e)| !e.presets.is_empty())
            .map_or("minecraft", |(id, _)| split(id).0)
    }

    /// The pack-wide preset every entry's value matches, if one does.
    fn current_preset(&self) -> Option<String> {
        self.configs.presets().into_iter().find(|preset| {
            self.configs
                .entries()
                .iter()
                .all(|(id, e)| match e.presets.get(preset) {
                    Some(value) => e.accept(value).as_ref() == self.configs.get(id),
                    None => true,
                })
        })
    }

    /// A preset's label (`<namespace>.presets.<name>`), or the label of none (`.none`).
    fn preset_label(&self, preset: Option<&str>) -> String {
        let namespace = self.presets_namespace();
        match preset {
            Some(name) => translated(&format!("{namespace}.presets.{name}"), || readable(name)),
            None => translated(&format!("{namespace}.presets.none"), || {
                lang::get("horizon.pack.options.presets.custom")
            }),
        }
    }

    /// The next pack-wide preset after the current one (`back`: before it), applied.
    fn cycle_preset(&mut self, back: bool) {
        let presets = self.configs.presets();
        if presets.is_empty() {
            return;
        }
        let n = presets.len();
        let next = match self
            .current_preset()
            .and_then(|c| presets.iter().position(|p| *p == c))
        {
            Some(i) if back => (i + n - 1) % n,
            Some(i) => (i + 1) % n,
            None if back => n - 1,
            None => 0,
        };
        self.configs.apply_preset(&presets[next]);
        self.revision += 1;
    }

    /// A section's page name: `config.<namespace>.section.<id>`, by its first entry's namespace.
    fn section_name(&self, section: usize) -> String {
        let Some((id, rows)) = self.sections.get(section) else {
            return lang::get("horizon.pack.options.general");
        };
        let Some(id) = id else {
            return lang::get("horizon.pack.options.general");
        };
        let namespace = rows
            .first()
            .and_then(|&r| self.ids.get(usize::from(r)))
            .map_or("minecraft", |entry| split(entry).0);
        translated(&format!("config.{namespace}.section.{id}"), || readable(id))
    }

    /// The tooltip (`config.<namespace>.<name>.tooltip`) and the performance line after it.
    fn description(&self, id: &str) -> String {
        let (namespace, path) = split(id);
        let key = format!("config.{namespace}.{path}.tooltip");
        let tooltip = if lang::has(&key) {
            lang::get(&key)
        } else {
            String::new()
        };
        let impact = self.configs.entries()[id]
            .performance_impact
            .as_ref()
            .map(|impact| {
                let level = lang::get(&format!("horizon.pack.options.impact.{impact}"));
                lang::tr("horizon.pack.options.impact", &[&level])
            });
        match impact {
            Some(line) if tooltip.is_empty() => line,
            Some(line) => format!("{tooltip} {line}"),
            None => tooltip,
        }
    }

    /// Entry `i`'s row: a switch, a slider in its steps, a choice, or text to read.
    fn entry_row(&self, i: u16) -> Option<Row<Item>> {
        let id = self.ids.get(usize::from(i))?;
        let (namespace, path) = split(id);
        let name = translated(&format!("config.{namespace}.{path}"), || readable(path));
        let entry = &self.configs.entries()[id];
        let shown = |text: String| match &entry.value_translation {
            Some(key) if lang::has(key) => lang::tr(key, &[&text]),
            _ => text,
        };
        let value = match (&entry.kind, self.configs.get(id)?) {
            (_, ConfigValue::Bool(on)) => Value::Switch(*on),
            (&ConfigKind::Number { min, max, step }, &ConfigValue::Number(n)) => {
                let span = max - min;
                let stops = if step > 0.0 {
                    (span / step).round()
                } else {
                    0.0
                };
                Value::Slider {
                    position: if span > 0.0 {
                        ((n - min) / span) as f32
                    } else {
                        0.0
                    },
                    steps: (stops >= 1.0).then_some(stops as u32),
                    text: shown(number_text(n, step)),
                }
            }
            (ConfigKind::Text { allowed }, ConfigValue::Text(t)) if !allowed.is_empty() => {
                Value::Choice(shown(t.clone()))
            }
            (_, value) => Value::Read(shown(match value {
                ConfigValue::Number(n) => number_text(*n, 1.0),
                ConfigValue::Text(t) => t.clone(),
                ConfigValue::Bool(b) => b.to_string(),
            })),
        };
        Some(Row::new(Item::Entry(i), name, value).about(self.description(id)))
    }

    /// The rows for the whole pack: its presets, when it has some, and Reset all.
    fn whole_pack(&self) -> Group<Item> {
        let mut rows = Vec::new();
        if !self.configs.presets().is_empty() {
            let key = format!("{}.presets", self.presets_namespace());
            let name = translated(&key, || lang::get("horizon.pack.options.presets"));
            let label = self.preset_label(self.current_preset().as_deref());
            rows.push(
                Row::new(Item::Presets, name, Value::Choice(label)).about(lang::get(PRESETS_ABOUT)),
            );
        }
        let at_defaults = self.configs.chosen().is_empty();
        let reset = Row::new(Item::ResetAll, lang::get(RESET_ALL), Value::Press)
            .about(lang::get(RESET_ABOUT))
            .blocked(at_defaults.then(|| lang::get(AT_DEFAULTS)));
        rows.push(reset);
        Group {
            name: WHOLE_PACK,
            rows,
        }
    }

    fn set(&mut self, id: &str, value: ConfigValue) {
        self.configs.set(id, value);
        self.revision += 1;
    }

    /// Every entry back to its default.
    fn reset(&mut self) {
        let defaults: Vec<_> = (self.configs.entries().iter())
            .map(|(id, e)| (id.clone(), e.default.clone()))
            .collect();
        for (id, value) in defaults {
            self.set(&id, value);
        }
    }
}

impl shell::Screen for PackOptionsScreen {
    type Page = u16;
    type Row = Item;
    fn title(&self) -> &'static str {
        TITLE
    }
    fn note_lines(&self) -> usize {
        NOTE_LINES
    }
    /// The rows read only the screen's own entries, which change through it alone.
    fn stamp(&self, _s: &Settings) -> u64 {
        self.revision
    }
    /// A page per section; the first starts with the rows for the whole pack.
    fn sheets(&self, _s: &Settings) -> Vec<Sheet<u16, Item>> {
        let about = lang::get(PACK_SETTINGS);
        let mut sheets = Vec::new();
        for (i, (_, entries)) in self.sections.iter().enumerate() {
            let rows = entries.iter().filter_map(|&e| self.entry_row(e)).collect();
            let mut groups = vec![Group {
                name: PACK_OPTIONS,
                rows,
            }];
            if i == 0 {
                groups.insert(0, self.whole_pack());
            }
            sheets.push(Sheet::new(
                i as u16,
                self.section_name(i),
                about.clone(),
                groups,
            ));
        }
        if sheets.is_empty() {
            let groups = vec![self.whole_pack()];
            sheets.push(Sheet::new(0, self.section_name(0), about, groups));
        }
        sheets
    }
    /// A switch flips, a choice and the presets step (back with Shift, or Left), Reset all
    /// resets.
    fn press(&mut self, _s: &mut Settings, item: Item, back: bool) -> Vec<Output> {
        let i = match item {
            Item::Presets => {
                self.cycle_preset(back);
                return Vec::new();
            }
            Item::ResetAll => {
                self.reset();
                return Vec::new();
            }
            Item::Entry(i) => i,
        };
        let Some(id) = self.ids.get(usize::from(i)).cloned() else {
            return Vec::new();
        };
        let kind = self.configs.entries()[&id].kind.clone();
        match (kind, self.configs.get(&id).cloned()) {
            (_, Some(ConfigValue::Bool(on))) => self.set(&id, ConfigValue::Bool(!on)),
            (ConfigKind::Text { allowed }, Some(ConfigValue::Text(t))) if !allowed.is_empty() => {
                let at = allowed.iter().position(|a| *a == t).unwrap_or(0);
                let n = allowed.len();
                let next = if back { (at + n - 1) % n } else { (at + 1) % n };
                self.set(&id, ConfigValue::Text(allowed[next].clone()));
            }
            _ => {}
        }
        Vec::new()
    }
    /// A slider moves its entry to the step nearest it (`Configs::set` snaps it).
    fn slide(&mut self, _s: &mut Settings, item: Item, position: f32) {
        let Item::Entry(i) = item else { return };
        let Some(id) = self.ids.get(usize::from(i)).cloned() else {
            return;
        };
        if let ConfigKind::Number { min, max, .. } = self.configs.entries()[&id].kind {
            let value = min + f64::from(position.clamp(0.0, 1.0)) * (max - min);
            self.set(&id, ConfigValue::Number(value));
        }
    }
    /// With the "Scroll wheel changes settings" extra, the wheel steps a choice and the presets.
    fn wheel_step(&mut self, s: &mut Settings, item: Item, back: bool) -> Option<Vec<Output>> {
        let steps = match item {
            Item::Presets => true,
            Item::Entry(i) => self
                .entry_row(i)
                .is_some_and(|row| matches!(row.value, Value::Choice(_))),
            Item::ResetAll => false,
        };
        if !steps {
            return None;
        }
        let mut outputs = vec![Output::Event(crate::options_ui::Event::ButtonClick)];
        outputs.extend(self.press(s, item, back));
        Some(outputs)
    }
    fn undoes(&self) -> bool {
        true
    }
    /// The entries as the screen opened with them.
    fn undo(&mut self, _s: &mut Settings, _before: &Settings) -> Vec<Output> {
        self.configs = self.opened.clone();
        self.revision += 1;
        Vec::new()
    }
    /// Undo shows exactly while an entry differs from how the screen opened.
    fn differs(&self, _s: &Settings, _before: &Settings) -> Option<bool> {
        Some(self.configs != self.opened)
    }
}

#[cfg(test)]
mod tests;
