//! Data-pack preset selection and the custom editor's working copy.
use super::*;
use crate::save::valid_world_gen_dimensions as valid_dimensions;
use horizon_core::nbt::Tag;
use std::{
    collections::BTreeMap,
    sync::{LazyLock, RwLock},
};

static EDITORS: LazyLock<RwLock<BTreeMap<String, mod_api::PresetEditor>>> =
    LazyLock::new(Default::default);

/// Install the client registration-phase table, first registration per preset wins.
pub fn register(editors: Vec<mod_api::PresetEditor>) {
    let mut table = BTreeMap::new();
    for editor in editors {
        if !horizon_core::level_id::valid_level_id(&editor.preset)
            || !horizon_core::level_id::valid_level_id(&editor.screen)
            || editor.screen.starts_with("minecraft:")
        {
            log::warn!("Invalid world preset editor: {editor:?}");
            continue;
        }
        table.entry(editor.preset.clone()).or_insert(editor);
    }
    *EDITORS.write().unwrap() = table;
}

pub(super) fn editor(preset: &str) -> Option<mod_api::PresetEditor> {
    EDITORS.read().unwrap().get(preset).cloned()
}

pub(super) fn preset_label(id: &str) -> String {
    lang::get(&format!("generator.{}", id.replace([':', '/'], ".")))
}

impl Title {
    /// The already-enabled mod data layer, below the world's selectable packs.
    pub fn set_preset_data_source(&mut self, source: crate::resource_source::ResourceSource) {
        self.preset_data = Some(std::sync::Arc::new(crate::datapack::DataPack::from_source(
            "mods".into(),
            source,
        )));
        if let Mode::Create { options, .. } = &mut self.mode {
            options.refresh_presets(&self.resources, self.preset_data.as_deref());
        }
    }
    /// The editor currently displayed; ordinary title screens return none.
    pub fn preset_editor(&self) -> Option<&mod_api::PresetEditor> {
        match &self.mode {
            Mode::CreatePreset { editor, .. } => Some(editor),
            _ => None,
        }
    }
    /// A snapshot, so screen callbacks never retain a borrow across dispatch.
    pub fn preset_editor_dimensions(&self) -> Option<Tag> {
        match &self.mode {
            Mode::CreatePreset { dimensions, .. } => Some(dimensions.borrow().clone()),
            _ => None,
        }
    }
    /// Replace only the editor's draft, preserving its parent until Done.
    pub fn set_preset_editor_dimensions(&self, value: Tag) -> bool {
        if !valid_dimensions(&value) {
            return false;
        }
        let Mode::CreatePreset { dimensions, .. } = &self.mode else {
            return false;
        };
        *dimensions.borrow_mut() = value;
        true
    }
    pub(super) fn close_preset_editor(&mut self, apply: bool) {
        let Mode::CreatePreset {
            parent, dimensions, ..
        } = &self.mode
        else {
            return;
        };
        let mut parent = (**parent).clone();
        if apply && let Mode::Create { options, .. } = &mut parent {
            options.preset_dimensions = Some(dimensions.borrow().clone());
        }
        self.set_mode(parent);
    }
}

impl WorldOptions {
    /// Selected registry key, including data-pack presets outside the vanilla enum.
    pub fn preset_id(&self) -> &str {
        self.custom_preset
            .as_deref()
            .unwrap_or(self.world_type.id())
    }
    pub(super) fn preset_label(&self) -> String {
        preset_label(self.preset_id())
    }
    /// Reload Create World's selected pack resources, retaining a still-available choice.
    pub(super) fn refresh_presets(
        &mut self,
        root: &Path,
        mod_data: Option<&crate::datapack::DataPack>,
    ) {
        let packs = self.available_packs(root);
        let mut selected: Vec<_> = self
            .data_packs
            .iter()
            .filter_map(|id| packs.iter().find(|pack| pack.id == *id))
            .collect();
        if let Some(mod_data) = mod_data {
            selected.insert(0, mod_data);
        }
        self.presets.clear();
        for pack in &selected {
            for file in pack.list_kinds(&["worldgen/world_preset"]) {
                let Some((namespace, path)) =
                    file.strip_prefix("data/").and_then(|f| f.split_once('/'))
                else {
                    continue;
                };
                let Some(path) = path
                    .strip_prefix("worldgen/world_preset/")
                    .and_then(|p| p.strip_suffix(".json"))
                else {
                    continue;
                };
                let id = format!("{namespace}:{path}");
                if !horizon_core::level_id::valid_level_id(&id) {
                    continue;
                }
                let Some(json) = pack
                    .read(&file)
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                else {
                    continue;
                };
                let dimensions = crate::save::json_tag(&json["dimensions"]);
                if valid_dimensions(&dimensions) {
                    self.presets.insert(id, dimensions);
                }
            }
        }
        let mut tags = horizon_world::worldgen::tags::TagDefinitions::default();
        let defaults: Vec<_> = crate::save::WorldPreset::ALL
            .iter()
            .map(|p| p.id())
            .collect();
        tags.apply_json(
            "minecraft:normal",
            "vanilla",
            &serde_json::json!({"values": defaults}).to_string(),
        )
        .unwrap();
        for pack in selected {
            for file in pack.list_kinds(&["tags/worldgen/world_preset"]) {
                let Some((namespace, path)) =
                    file.strip_prefix("data/").and_then(|f| f.split_once('/'))
                else {
                    continue;
                };
                let Some(path) = path
                    .strip_prefix("tags/worldgen/world_preset/")
                    .and_then(|p| p.strip_suffix(".json"))
                else {
                    continue;
                };
                let Some(json) = pack.read(&file).and_then(|b| String::from_utf8(b).ok()) else {
                    continue;
                };
                if let Err(error) = tags.apply_json(&format!("{namespace}:{path}"), &pack.id, &json)
                {
                    log::warn!("Invalid world preset tag {file}: {error}");
                }
            }
        }
        let all_presets = defaults
            .iter()
            .copied()
            .chain([crate::save::WorldPreset::Debug.id()])
            .map(str::to_owned)
            .chain(
                self.presets
                    .keys()
                    .filter(|id| crate::save::WorldPreset::of_id(id).is_none())
                    .cloned(),
            )
            .collect();
        self.preset_order = match tags.build(|id, _| {
            (crate::save::WorldPreset::of_id(id).is_some() || self.presets.contains_key(id))
                .then(|| id.to_owned())
        }) {
            Ok(mut built) => built
                .tags
                .remove("minecraft:normal")
                .filter(|ids| !ids.is_empty())
                .unwrap_or(all_presets),
            Err(error) => {
                log::warn!("Invalid world preset tags: {error}");
                all_presets
            }
        };
        // Re-Create retains Debug even when the normal tag hides it.
        if self.preset_id() != crate::save::WorldPreset::Debug.id()
            && !self.preset_order.iter().any(|id| id == self.preset_id())
        {
            self.custom_preset = self.preset_order.first().cloned();
            if let Some(preset) = self
                .custom_preset
                .as_deref()
                .and_then(crate::save::WorldPreset::of_id)
            {
                self.world_type = preset;
                self.custom_preset = None;
            }
            self.preset_dimensions = None;
        }
    }
    pub(super) fn cycle_preset(&mut self, step: i32) {
        if self.alt_down {
            self.world_type = self.world_type.step(step, true);
            self.custom_preset = None;
            self.preset_dimensions = None;
            return;
        }
        if self.preset_order.is_empty() {
            return;
        }
        let id = self.cycled_preset(step).to_owned();
        self.custom_preset = if let Some(preset) = crate::save::WorldPreset::of_id(&id) {
            self.world_type = preset;
            None
        } else {
            Some(id)
        };
        self.preset_dimensions = None;
    }
    pub(super) fn cycled_preset(&self, step: i32) -> &str {
        if self.preset_order.is_empty() {
            return self.preset_id();
        }
        let current = self
            .preset_order
            .iter()
            .position(|id| *id == self.preset_id())
            .unwrap_or(0);
        let next =
            ((current as i32 + step.signum()).rem_euclid(self.preset_order.len() as i32)) as usize;
        &self.preset_order[next]
    }
    /// Dimensions chosen by a data-pack preset or committed by its editor.
    pub fn dimensions(&self) -> Option<Tag> {
        self.preset_dimensions
            .clone()
            .or_else(|| self.presets.get(self.preset_id()).cloned())
    }
    pub(super) fn editor_dimensions(&self) -> Option<Tag> {
        self.dimensions().or_else(|| {
            crate::save::new_world_gen_settings(
                0,
                self.world_gen(),
                self.flat_settings(),
                self.single_biome(),
            )
            .get("dimensions")
            .cloned()
        })
    }
    /// Write creation settings, including the custom editor's dimension stems.
    pub fn write_world_gen(&self, dir: &Path, seed: i64) -> crate::Result<()> {
        crate::save::write_world_gen_settings_with(
            dir,
            seed,
            self.world_gen(),
            self.flat_settings(),
            self.single_biome(),
            self.dimensions(),
        )
    }
}
