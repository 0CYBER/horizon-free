//! A world's data packs over the prepared worldgen bundles.
//!
//! Vanilla builds its worldgen registries from the enabled packs in order:
//! the built-in pack first, then each enabled pack, a later pack's file for
//! an id replacing an earlier one's. Tags are the exception: every pack's
//! file for a tag adds its entries, and `replace` drops what came before.
//! The prepared bundles are the built-in pack, so the packs above it are
//! entries that replace or join a bundle's (`GeneratorData::load_with`).
use serde_json::Value;
use std::{collections::BTreeMap, sync::OnceLock};

/// The bundle of `dimension/<id>.json` level stems. Nothing prepared has
/// it: a pack's level stem stands in for the world preset's.
pub const DIMENSIONS: &str = "dimensions";
/// The bundle of `dimension_type/<id>.json` dimension types. Nothing
/// prepared has it: the vanilla types' are `LevelHeight::vanilla`.
pub const DIMENSION_TYPES: &str = "dimension_types";
/// The bundle of `worldgen/noise_settings/<id>.json` noise generator settings.
pub const NOISE_SETTINGS: &str = "settings";

/// Pack folders (`data/<ns>/<kind>/`) the chunk generator reads, and the
/// bundle each joins.
const KINDS: [(&str, &str); 16] = [
    ("dimension", DIMENSIONS),
    ("dimension_type", DIMENSION_TYPES),
    ("worldgen/biome", "biomes"),
    ("worldgen/block_state_provider", "state_providers"),
    // 26.3's names: 26.2 called them configured_carver and configured_feature.
    ("worldgen/carver", "carvers"),
    ("worldgen/feature", "features"),
    ("worldgen/density_function", "densities"),
    ("worldgen/material_condition", "conditions"),
    ("worldgen/material_rule", "rules"),
    ("worldgen/noise", "noises"),
    ("worldgen/noise_settings", NOISE_SETTINGS),
    ("worldgen/placed_feature", "placed_features"),
    ("worldgen/processor_list", "processor_lists"),
    ("worldgen/structure", "structures"),
    ("worldgen/structure_set", "structure_sets"),
    ("worldgen/template_pool", "template_pools"),
];
/// Tag folders the chunk generator reads, and the tag bundle each joins.
const TAG_KINDS: [(&str, &str); 4] = [
    ("tags/block", "block_tags"),
    ("tags/fluid", "fluid_tags"),
    ("tags/worldgen/biome", "biome_tags"),
    ("tags/worldgen/carver", "carver_tags"),
];
/// The pack folders the worldgen resolver reads, in walk order.
pub const PACK_KINDS: [&str; 5] = [
    "dimension",
    "dimension_type",
    "structure",
    "tags",
    "worldgen",
];

#[cfg(test)]
use horizon_core::level_id::valid_level_id;

/// A level a mod registered at startup: a level stem and a dimension type
/// that sit below every data pack, which can replace either.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModDimension {
    pub id: String,
    /// The level stem's JSON text.
    pub stem: String,
    /// The fields of its dimension type the mod sets, as JSON text: the rest
    /// are the overworld's.
    pub dimension_type: String,
    /// The sea level the mod gives the level, for a chunk generator whose terrain has none to
    /// read (`stem_sea_level`); the noise settings' win for the noise generator.
    pub sea_level: Option<i32>,
}

static MOD_DIMENSIONS: OnceLock<Vec<ModDimension>> = OnceLock::new();

/// Hands the mods' levels to the engine, once at startup, before any world
/// opens. The first table stays.
pub fn install_mod_dimensions(dimensions: Vec<ModDimension>) -> Result<(), Vec<ModDimension>> {
    MOD_DIMENSIONS.set(dimensions)
}

/// The levels the mods registered; empty before they are installed.
pub fn mod_dimensions() -> &'static [ModDimension] {
    MOD_DIMENSIONS.get().map_or(&[], Vec::as_slice)
}

/// The sea level a level stem's noise generator builds its terrain at: its `settings`
/// (`NoiseGeneratorSettings.seaLevel`), a noise settings id that `read` resolves to its JSON or the
/// settings inline. None for any other generator, whose terrain has no sea level to read, and for
/// settings `read` cannot find.
pub fn stem_sea_level(stem: &Value, read: &dyn Fn(&str) -> Option<Value>) -> Option<i32> {
    let generator = &stem["generator"];
    if generator["type"].as_str() != Some("minecraft:noise") {
        return None;
    }
    let settings = match &generator["settings"] {
        Value::String(id) => read(id)?,
        inline @ Value::Object(_) => inline.clone(),
        _ => return None,
    };
    i32::try_from(settings["sea_level"].as_i64()?).ok()
}

/// `ns:path` for a pack file `data/<ns>/<kind>/<path>.json`.
fn resource_id(file: &str, kind: &str) -> Option<String> {
    let (ns, rest) = file.strip_prefix("data/")?.split_once('/')?;
    let path = rest
        .strip_prefix(kind)?
        .strip_prefix('/')?
        .strip_suffix(".json")?;
    Some(format!("{ns}:{path}"))
}

/// `ns:path` for a pack's structure template `data/<ns>/structure/<path>.nbt`.
pub fn template_id(file: &str) -> Option<String> {
    let (ns, rest) = file.strip_prefix("data/")?.split_once('/')?;
    let path = rest.strip_prefix("structure/")?.strip_suffix(".nbt")?;
    Some(format!("{ns}:{path}"))
}

/// What `add_file` made of a pack file.
#[derive(Debug, PartialEq, Eq)]
pub enum PackFile {
    /// An entry or tag file the generator reads.
    Read,
    /// A worldgen file of a kind the generator does not read (its folder).
    NotRead(String),
    /// A generator file whose text could not be read.
    Unreadable,
    /// Not worldgen.
    Other,
}

/// A level's build range (`LevelHeightAccessor`: `min_y`, `height`) and the
/// rest of its dimension type the game reads: `logical_height`,
/// `has_skylight` and `has_ceiling`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LevelHeight {
    pub min_y: i32,
    pub height: i32,
    pub logical_height: i32,
    pub has_skylight: bool,
    pub has_ceiling: bool,
}
impl LevelHeight {
    /// The vanilla dimension types'.
    pub fn vanilla(dimension_type: &str) -> Option<Self> {
        let (min_y, height, logical_height, has_skylight, has_ceiling) = match dimension_type {
            // overworld_caves builds like the Overworld but has a ceiling.
            "minecraft:overworld" => (-64, 384, 384, true, false),
            "minecraft:overworld_caves" => (-64, 384, 384, true, true),
            "minecraft:the_nether" => (0, 256, 128, false, true),
            "minecraft:the_end" => (0, 256, 256, true, false),
            _ => return None,
        };
        Some(Self {
            min_y,
            height,
            logical_height,
            has_skylight,
            has_ceiling,
        })
    }
    /// A dimension type's JSON, if its codec accepts the range: multiples
    /// of 16, at least 16 high, within `DimensionType.MIN_Y` (-2032) and
    /// `MAX_Y` (2031), and a logical height within the height.
    pub fn of_json(v: &Value) -> Option<Self> {
        let int = |k: &str| i32::try_from(v[k].as_i64()?).ok();
        let (min_y, height) = (int("min_y")?, int("height")?);
        let logical_height = int("logical_height").unwrap_or(height);
        let valid = min_y % 16 == 0
            && height % 16 == 0
            && height >= 16
            && min_y >= -2032
            && i64::from(min_y) + i64::from(height) <= 2032
            && (0..=height).contains(&logical_height);
        valid.then_some(Self {
            min_y,
            height,
            logical_height,
            has_skylight: v["has_skylight"].as_bool().unwrap_or(true),
            // The codec requires has_ceiling; a type the packs leave it out
            // of gets the open-sky default.
            has_ceiling: v["has_ceiling"].as_bool().unwrap_or(false),
        })
    }
    /// A level stem's `type` (a dimension type id, or one inline), with the
    /// packs' `types` over the vanilla ones.
    pub fn of_type(ty: &Value, types: &BTreeMap<String, String>) -> Option<Self> {
        match ty {
            Value::String(id) => match types.get(id) {
                Some(json) => Self::of_json(&serde_json::from_str(json).ok()?),
                None => Self::vanilla(id),
            },
            inline @ Value::Object(_) => Self::of_json(inline),
            _ => None,
        }
    }
}

/// Replacement and added bundle entries, and tag files, from data packs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct WorldgenOverrides {
    templates: BTreeMap<String, horizon_assets::resource_source::Resource>,
    /// Bundle name → id → the highest pack's JSON text for it.
    entries: BTreeMap<String, BTreeMap<String, String>>,
    /// Tag bundle name → (tag id, pack id, tag file text), lowest pack first.
    tags: BTreeMap<String, Vec<(String, String, String)>>,
}
impl WorldgenOverrides {
    /// Lazy gzip NBT entries, retaining their source generation for cache identity.
    pub fn templates(&self) -> &BTreeMap<String, horizon_assets::resource_source::Resource> {
        &self.templates
    }
    pub fn set_template(
        &mut self,
        id: String,
        resource: horizon_assets::resource_source::Resource,
    ) {
        self.templates.insert(id, resource);
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.tags.is_empty() && self.templates.is_empty()
    }
    /// A pack's file for `id` in `bundle`: called for packs from the lowest
    /// to the highest, so the last call for an id wins.
    pub fn set(&mut self, bundle: &str, id: &str, json: String) {
        self.entries
            .entry(bundle.to_owned())
            .or_default()
            .insert(id.to_owned(), json);
    }
    /// A mod's level: called before any pack's file, so a pack's replaces it.
    pub fn add_mod_dimension(&mut self, dimension: &ModDimension) {
        self.set(DIMENSIONS, &dimension.id, dimension.stem.clone());
        self.set(
            DIMENSION_TYPES,
            &dimension.id,
            dimension.dimension_type.clone(),
        );
    }
    /// A pack's tag file for the tag `id` in the tag bundle `bundle`, in
    /// pack order.
    pub fn add_tag(&mut self, bundle: &str, id: &str, pack: &str, json: String) {
        self.tags.entry(bundle.to_owned()).or_default().push((
            id.to_owned(),
            pack.to_owned(),
            json,
        ));
    }
    /// The file `file` (`data/<ns>/...`, relative to the pack root) of the
    /// pack `pack`, whose text `read` gives. Packs go from the lowest to the
    /// highest, each in its own file order.
    pub fn add_file(
        &mut self,
        pack: &str,
        file: &str,
        read: impl FnOnce() -> Option<String>,
    ) -> PackFile {
        let Some((_, rest)) = file.strip_prefix("data/").and_then(|r| r.split_once('/')) else {
            return PackFile::Other;
        };
        if let Some((bundle, id)) = KINDS
            .iter()
            .find_map(|(kind, bundle)| Some((bundle, resource_id(file, kind)?)))
        {
            let Some(json) = read() else {
                return PackFile::Unreadable;
            };
            self.set(bundle, &id, json);
            PackFile::Read
        } else if let Some((bundle, id)) = TAG_KINDS
            .iter()
            .find_map(|(kind, bundle)| Some((bundle, resource_id(file, kind)?)))
        {
            let Some(json) = read() else {
                return PackFile::Unreadable;
            };
            self.add_tag(bundle, &id, pack, json);
            PackFile::Read
        } else if rest.starts_with("worldgen/") || rest.starts_with("tags/worldgen/") {
            PackFile::NotRead(rest.rsplit_once('/').map_or(rest, |(k, _)| k).to_owned())
        } else {
            PackFile::Other
        }
    }
    /// After the last pack: leaves out the packs' new biomes (`known` says
    /// which the game has), and what names them names the rest without
    /// them: their biome tags' other biomes, and the climate points of the
    /// other biomes in a level stem's multi-noise list (the left-out
    /// biome's climate goes to its nearest neighbours). Returns what it
    /// left out, for the log.
    pub fn finish(&mut self, known: impl Fn(&str) -> bool) -> Vec<String> {
        let left_out = self.retain("biomes", &known);
        let mut warnings: Vec<String> = left_out
            .iter()
            .map(|id| {
                format!("biome {id} is new: Horizon cannot add biomes yet, so it is left out")
            })
            .collect();
        if left_out.is_empty() {
            return warnings;
        }
        let gone = |v: &Value| {
            let id = v.as_str().or_else(|| v["id"].as_str()).unwrap_or_default();
            let id = if id.contains(':') {
                id.to_owned()
            } else {
                format!("minecraft:{id}")
            };
            left_out.contains(&id)
        };
        for (_, _, json) in self.tags.get_mut("biome_tags").into_iter().flatten() {
            if let Ok(mut v) = serde_json::from_str::<Value>(json)
                && let Some(values) = v["values"].as_array_mut()
            {
                let before = values.len();
                values.retain(|e| !gone(e));
                if values.len() != before {
                    *json = v.to_string();
                }
            }
        }
        for (id, json) in self.entries.get_mut(DIMENSIONS).into_iter().flatten() {
            let Ok(mut v) = serde_json::from_str::<Value>(json) else {
                continue;
            };
            let Some(points) = v["generator"]["biome_source"]["biomes"].as_array_mut() else {
                continue;
            };
            let before = points.len();
            points.retain(|p| !gone(&p["biome"]));
            if points.len() != before {
                warnings.push(format!(
                    "dimension {id}: {} of its {before} biome climate points name left-out \
                     biomes and are removed",
                    before - points.len()
                ));
                *json = v.to_string();
            }
        }
        warnings
    }
    /// The build range of the vanilla level `level` (`minecraft:overworld`,
    /// `minecraft:the_nether`, `minecraft:the_end`) with these packs: the
    /// dimension type its level stem names (a pack's stem, else the
    /// preset's, whose type has the level's name), a pack's file for that
    /// type over the vanilla one. `None` when the packs give it a range the
    /// codec refuses.
    pub fn level_height(&self, level: &str) -> Option<LevelHeight> {
        let none = BTreeMap::new();
        let types = self.entries.get(DIMENSION_TYPES).unwrap_or(&none);
        let ty = match self.entries.get(DIMENSIONS).and_then(|d| d.get(level)) {
            Some(stem) => serde_json::from_str::<Value>(stem).ok()?["type"].clone(),
            None => Value::String(level.to_owned()),
        };
        LevelHeight::of_type(&ty, types)
    }
    /// The id and JSON of the dimension type a mod or a pack gave `level`, resolved as `level_height`
    /// does: None when the level takes a vanilla type no pack changed.
    pub fn level_type_json(&self, level: &str) -> Option<(String, Value)> {
        let none = BTreeMap::new();
        let types = self.entries.get(DIMENSION_TYPES).unwrap_or(&none);
        let ty = match self.entries.get(DIMENSIONS).and_then(|d| d.get(level)) {
            Some(stem) => serde_json::from_str::<Value>(stem).ok()?["type"].clone(),
            None => Value::String(level.to_owned()),
        };
        match &ty {
            Value::String(id) => Some((
                id.clone(),
                serde_json::from_str::<Value>(types.get(id)?).ok()?,
            )),
            inline @ Value::Object(_) => Some((level.to_owned(), inline.clone())),
            _ => None,
        }
    }
    /// The dimension type a mod or a pack gave `level`, resolved as `level_height` does: a built-in
    /// type's facts with the fields the pack's JSON names over them. An explicit stem may also
    /// choose an unchanged built-in type. None without a stem or type override, and an error
    /// when a field the game reads is refused.
    pub fn level_facts(
        &self,
        level: &str,
    ) -> Option<Result<crate::dimension_type::DimensionFacts, String>> {
        use crate::dimension_type::DimensionFacts;
        let Some((id, json)) = self.level_type_json(level) else {
            let stem = self.entries.get(DIMENSIONS)?.get(level)?;
            let stem: Value = serde_json::from_str(stem).ok()?;
            return Some(Ok(DimensionFacts::vanilla(stem["type"].as_str()?)?.clone()));
        };
        // A pack's type for a vanilla id changes that type; any other starts from the Overworld's.
        let base = DimensionFacts::base_for_type(&id);
        Some(DimensionFacts::patched(base, &id, &json))
    }
    /// The `sea_level` of the noise settings these packs give the vanilla level `level`: those its
    /// pack stem names (inline, or a pack's own file), else the preset's (`minecraft:overworld`,
    /// `nether`, `end`) when a pack replaces them. `None` when nothing the packs give changes it,
    /// for a generator that is not the noise one, and for settings the packs do not carry.
    pub fn level_sea_level(&self, level: &str) -> Option<i32> {
        let stem = self.entries.get(DIMENSIONS).and_then(|d| d.get(level));
        let settings = match stem {
            Some(stem) => {
                let generator = &serde_json::from_str::<Value>(stem).ok()?["generator"];
                if generator["type"].as_str() != Some("minecraft:noise") {
                    return None;
                }
                generator["settings"].clone()
            }
            None => Value::String(
                match level {
                    "minecraft:the_nether" => "minecraft:nether",
                    "minecraft:the_end" => "minecraft:end",
                    _ => "minecraft:overworld",
                }
                .to_owned(),
            ),
        };
        let settings = match settings {
            Value::String(id) => {
                serde_json::from_str(self.entries.get("settings")?.get(&id)?).ok()?
            }
            inline @ Value::Object(_) => inline,
            _ => return None,
        };
        i32::try_from(settings["sea_level"].as_i64()?).ok()
    }
    /// The entries packs give `bundle`.
    pub fn entries(&self, bundle: &str) -> impl Iterator<Item = (&str, &str)> {
        self.entries
            .get(bundle)
            .into_iter()
            .flatten()
            .map(|(id, json)| (id.as_str(), json.as_str()))
    }
    /// Partial startup validation of winning fill-layer leaves through their existing codec.
    /// Other feature types keep their existing lazy path; this never expands feature holders.
    pub fn validate_fill_layers(
        &self,
        registry: &crate::registry::BlockRegistry,
    ) -> Result<(), super::DecodeError> {
        use super::block_feature::{FeatureBlockBindings, FillLayerFeature, integer_properties};
        let entries: Vec<_> = self
            .entries("features")
            .filter(|(_, json)| {
                let Ok(object) = serde_json::from_str::<super::Object>(json) else {
                    return false;
                };
                let Some(kind) = object.get("type") else {
                    return false;
                };
                let Ok(kind) = serde_json::from_str::<String>(kind.get()) else {
                    return false;
                };
                super::density::identifier(&kind).ok().as_deref() == Some("minecraft:fill_layer")
            })
            .collect();
        if entries.is_empty() {
            return Ok(());
        }
        let bindings = FeatureBlockBindings::new(registry, integer_properties(registry))?;
        for (id, json) in entries {
            FillLayerFeature::from_json(json, &bindings).map_err(|error| {
                super::DecodeError::Registry(format!("configured feature {id}: {error}"))
            })?;
        }
        Ok(())
    }

    /// The tag files packs give the tag bundle `bundle`, in pack order.
    pub fn tags(&self, bundle: &str) -> impl Iterator<Item = (&str, &str, &str)> {
        self.tags
            .get(bundle)
            .into_iter()
            .flatten()
            .map(|(id, pack, json)| (id.as_str(), pack.as_str(), json.as_str()))
    }
    /// Removes the entries of `bundle` that `keep` refuses, returning their ids.
    pub fn retain(&mut self, bundle: &str, mut keep: impl FnMut(&str) -> bool) -> Vec<String> {
        let Some(entries) = self.entries.get_mut(bundle) else {
            return Vec::new();
        };
        let dropped: Vec<String> = entries.keys().filter(|id| !keep(id)).cloned().collect();
        for id in &dropped {
            entries.remove(id);
        }
        if entries.is_empty() {
            self.entries.remove(bundle);
        }
        dropped
    }
    /// `bundle` with the packs' entries over it.
    pub fn apply(
        &self,
        bundle: &str,
        mut base: BTreeMap<String, String>,
    ) -> BTreeMap<String, String> {
        for (id, json) in self.entries(bundle) {
            base.insert(id.to_owned(), json.to_owned());
        }
        base
    }
}

#[cfg(test)]
mod tests {

    /// A level's id is a folder under the world: nothing may climb out of it or name nothing.
    #[test]
    fn level_ids_must_be_safe_folder_names() {
        for id in [
            "test:pocket",
            "test:a/b",
            "test:deep/pocket-1.x",
            "my_mod.2:a_b/c-d",
        ] {
            assert!(valid_level_id(id), "{id}");
        }
        for id in [
            "test:../minecraft/overworld",
            "test:a/../../b",
            "test:a/../b",
            "test:/abs",
            "test:trailing/",
            "test:a//b",
            "test:.",
            "test:..",
            "test:./a",
            "test:",
            ":pocket",
            "..:pocket",
            ".:pocket",
            "pocket",
            "test:Pocket",
            "test:my pocket",
            "test:a\\b",
            "te st:pocket",
            "test:a:b",
        ] {
            assert!(!valid_level_id(id), "{id}");
        }
    }
    use super::*;
    #[test]
    fn a_template_id_comes_from_a_structure_nbt_file_only() {
        assert_eq!(
            template_id("data/p/structure/room/a.nbt").as_deref(),
            Some("p:room/a")
        );
        assert_eq!(template_id("data/p/structure/a.json"), None);
        assert_eq!(template_id("data/p/worldgen/structure/a.nbt"), None);
    }

    #[test]
    fn the_highest_pack_wins_and_others_join() {
        let mut o = WorldgenOverrides::default();
        assert!(o.is_empty());
        o.set("noises", "minecraft:cave", "low".into());
        o.set("noises", "minecraft:cave", "high".into());
        o.set("noises", "pack:new", "new".into());
        let base = BTreeMap::from([
            ("minecraft:cave".to_owned(), "vanilla".to_owned()),
            ("minecraft:other".to_owned(), "kept".to_owned()),
        ]);
        let merged = o.apply("noises", base);
        assert_eq!(merged["minecraft:cave"], "high");
        assert_eq!(merged["minecraft:other"], "kept");
        assert_eq!(merged["pack:new"], "new");
        assert_eq!(o.apply("densities", BTreeMap::new()).len(), 0);
    }

    #[test]
    fn pack_files_go_to_their_bundles() {
        let mut o = WorldgenOverrides::default();
        let text = |t: &str| {
            let t = t.to_owned();
            move || Some(t)
        };
        let mut add = |file: &str, json: &str| o.add_file("file/p", file, text(json));
        assert_eq!(
            add("data/minecraft/worldgen/noise/cave.json", "1"),
            PackFile::Read
        );
        assert_eq!(
            add("data/p/worldgen/density_function/a/b.json", "2"),
            PackFile::Read
        );
        assert_eq!(
            add("data/minecraft/worldgen/noise_settings/overworld.json", "3"),
            PackFile::Read
        );
        assert_eq!(
            add("data/minecraft/worldgen/structure_set/mineshafts.json", "4"),
            PackFile::Read
        );
        assert_eq!(
            add("data/minecraft/dimension/overworld.json", "5"),
            PackFile::Read
        );
        assert_eq!(
            add("data/minecraft/tags/worldgen/biome/is_hill.json", "{}"),
            PackFile::Read
        );
        assert_eq!(
            add(
                "data/minecraft/dimension_type/overworld.json",
                r#"{"min_y":-64,"height":2096}"#
            ),
            PackFile::Read
        );
        assert_eq!(
            add("data/minecraft/worldgen/carver/cave.json", "6"),
            PackFile::Read
        );
        assert_eq!(
            add("data/minecraft/worldgen/feature/water_delta.json", "7"),
            PackFile::Read
        );
        assert_eq!(
            add(
                "data/p/worldgen/block_state_provider/stony_cliffs.json",
                "8"
            ),
            PackFile::Read
        );
        assert_eq!(
            add("data/minecraft/worldgen/configured_carver/cave.json", "{}"),
            PackFile::NotRead("worldgen/configured_carver".into())
        );
        assert_eq!(
            add("data/minecraft/worldgen/world_preset/normal.json", "{}"),
            PackFile::NotRead("worldgen/world_preset".into())
        );
        assert_eq!(add("data/p/function/f.mcfunction", ""), PackFile::Other);
        assert_eq!(
            o.add_file("file/p", "data/p/worldgen/noise/x.json", || None),
            PackFile::Unreadable
        );
        let get = |bundle: &str| o.entries(bundle).collect::<Vec<_>>();
        assert_eq!(get("noises"), [("minecraft:cave", "1")]);
        assert_eq!(get("densities"), [("p:a/b", "2")]);
        assert_eq!(get("settings"), [("minecraft:overworld", "3")]);
        assert_eq!(get("structure_sets"), [("minecraft:mineshafts", "4")]);
        assert_eq!(get(DIMENSIONS), [("minecraft:overworld", "5")]);
        assert_eq!(get("carvers"), [("minecraft:cave", "6")]);
        assert_eq!(get("features"), [("minecraft:water_delta", "7")]);
        assert_eq!(get("state_providers"), [("p:stony_cliffs", "8")]);
        assert_eq!(get("structures"), []);
        assert_eq!(o.tags("biome_tags").count(), 1);
    }

    #[test]
    fn levels_take_their_height_from_the_packs_dimension_types() {
        let mut o = WorldgenOverrides::default();
        let overworld = o.level_height("minecraft:overworld");
        assert_eq!(overworld, LevelHeight::vanilla("minecraft:overworld"));
        let nether = o.level_height("minecraft:the_nether").unwrap();
        assert_eq!(
            (
                nether.logical_height,
                nether.has_skylight,
                nether.has_ceiling
            ),
            (128, false, true)
        );
        // overworld_caves builds like the Overworld but has a ceiling.
        let caves = LevelHeight::vanilla("minecraft:overworld_caves").unwrap();
        assert_eq!(
            (
                caves.min_y,
                caves.height,
                caves.has_skylight,
                caves.has_ceiling
            ),
            (-64, 384, true, true)
        );
        o.set(
            DIMENSION_TYPES,
            "minecraft:overworld",
            r#"{"min_y":-64,"height":2096,"logical_height":384,"has_skylight":true,"has_ceiling":true}"#.into(),
        );
        let tall = LevelHeight {
            min_y: -64,
            height: 2096,
            logical_height: 384,
            has_skylight: true,
            has_ceiling: true,
        };
        assert_eq!(o.level_height("minecraft:overworld"), Some(tall));
        let end = o.level_height("minecraft:the_end");
        assert_eq!(end, LevelHeight::vanilla("minecraft:the_end"));
        // A pack's level stem names another type.
        o.set(
            DIMENSIONS,
            "minecraft:the_end",
            r#"{"type":"minecraft:overworld"}"#.into(),
        );
        assert_eq!(o.level_height("minecraft:the_end"), Some(tall));
        // Ranges the codec refuses.
        for bad in [
            r#"{"min_y":-60,"height":384}"#,
            r#"{"min_y":-64,"height":2112}"#,
            r#"{"min_y":0,"height":0}"#,
            r#"{"min_y":0,"height":256,"logical_height":300}"#,
        ] {
            o.set(DIMENSION_TYPES, "minecraft:overworld", bad.into());
            assert_eq!(o.level_height("minecraft:overworld"), None, "{bad}");
        }
        // The codec requires has_ceiling: a type without it keeps the pack's
        // fields and the open-sky default.
        o.set(
            DIMENSION_TYPES,
            "minecraft:overworld",
            r#"{"min_y":-64,"height":384,"logical_height":384,"has_skylight":true}"#.into(),
        );
        let open = o.level_height("minecraft:overworld").unwrap();
        assert!((open.height, open.has_skylight, open.has_ceiling) == (384, true, false));
    }

    #[test]
    fn new_biomes_are_left_out_of_tags_and_climate_points() {
        let mut o = WorldgenOverrides::default();
        o.set("biomes", "minecraft:plains", "{}".into());
        o.set("biomes", "minecraft:molten_peaks", "{}".into());
        o.add_tag(
            "biome_tags",
            "minecraft:is_hill",
            "file/p",
            r#"{"values":["minecraft:plains","molten_peaks",{"id":"minecraft:molten_peaks","required":false}]}"#.into(),
        );
        o.set(
            DIMENSIONS,
            "minecraft:overworld",
            r#"{"generator":{"biome_source":{"biomes":[{"biome":"minecraft:plains"},{"biome":"minecraft:molten_peaks"}]}}}"#.into(),
        );
        let warnings = o.finish(|id| id == "minecraft:plains");
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert_eq!(o.entries("biomes").count(), 1);
        let (_, _, tag) = o.tags("biome_tags").next().unwrap();
        assert_eq!(tag, r#"{"values":["minecraft:plains"]}"#);
        let (_, stem) = o.entries(DIMENSIONS).next().unwrap();
        assert!(!stem.contains("molten_peaks") && stem.contains("minecraft:plains"));
    }

    #[test]
    fn a_levels_sea_level_comes_from_the_noise_settings_the_packs_give_it() {
        let mut o = WorldgenOverrides::default();
        assert_eq!(o.level_sea_level("minecraft:the_nether"), None);
        o.set("settings", "minecraft:nether", r#"{"sea_level":10}"#.into());
        assert_eq!(o.level_sea_level("minecraft:the_nether"), Some(10));
        assert_eq!(o.level_sea_level("minecraft:overworld"), None);
        o.set("settings", "test:high", r#"{"sea_level":120}"#.into());
        let stem = |settings: &str| {
            format!(r#"{{"generator":{{"type":"minecraft:noise","settings":{settings}}}}}"#)
        };
        o.set(DIMENSIONS, "minecraft:overworld", stem(r#""test:high""#));
        assert_eq!(o.level_sea_level("minecraft:overworld"), Some(120));
        o.set(
            DIMENSIONS,
            "minecraft:the_end",
            stem(r#"{"sea_level":-16}"#),
        );
        assert_eq!(o.level_sea_level("minecraft:the_end"), Some(-16));
        o.set(DIMENSIONS, "minecraft:the_nether", stem(r#""test:absent""#));
        assert_eq!(o.level_sea_level("minecraft:the_nether"), None);
        o.set(
            DIMENSIONS,
            "minecraft:the_nether",
            r#"{"generator":{"type":"minecraft:flat"}}"#.into(),
        );
        assert_eq!(o.level_sea_level("minecraft:the_nether"), None);
    }
}

#[cfg(test)]
mod level_facts_tests {
    use super::*;

    #[test]
    fn a_stem_can_choose_a_different_builtin_dimension_type() {
        let mut overrides = WorldgenOverrides::default();
        overrides.set(
            DIMENSIONS,
            "minecraft:overworld",
            r#"{"type":"minecraft:the_nether"}"#.into(),
        );
        let facts = overrides
            .level_facts("minecraft:overworld")
            .unwrap()
            .unwrap();
        assert_eq!(facts.id, "minecraft:the_nether");
        assert!(facts.fast_lava && facts.respawn_anchor_works);
        assert_eq!(facts.height.height, 256);
        overrides.set(DIMENSION_TYPES, "minecraft:the_nether", r#"{"min_y":0,"height":256,"logical_height":128,"has_ceiling":true,"has_skylight":false,"ambient_light":0.5}"#.into());
        assert_eq!(
            overrides
                .level_facts("minecraft:overworld")
                .unwrap()
                .unwrap()
                .ambient_light,
            0.5
        );
    }

    #[test]
    fn a_mods_level_takes_the_fields_it_set_over_the_overworlds() {
        let mut o = WorldgenOverrides::default();
        o.add_mod_dimension(&ModDimension {
            id: "test:pocket".into(),
            stem: r#"{"type":"test:pocket"}"#.into(),
            dimension_type: r#"{"min_y":-64,"height":384,"coordinate_scale":2.0,
                "attributes":{"minecraft:gameplay/water_evaporates":true}}"#
                .into(),
            sea_level: None,
        });
        let facts = o.level_facts("test:pocket").unwrap().unwrap();
        assert_eq!(facts.coordinate_scale, 2.0);
        assert!(facts.water_evaporates && !facts.fast_lava);
        // a level on a vanilla type no pack changed has nothing of its own
        assert!(o.level_facts("minecraft:the_nether").is_none());
    }

    #[test]
    fn a_packs_type_for_a_vanilla_id_changes_that_vanilla_type() {
        let mut o = WorldgenOverrides::default();
        o.set(
            DIMENSION_TYPES,
            "minecraft:the_nether",
            r#"{"min_y":0,"height":256,"logical_height":128,"has_ceiling":true,"has_skylight":false,"ambient_light":0.5}"#.into(),
        );
        let facts = o.level_facts("minecraft:the_nether").unwrap().unwrap();
        assert_eq!(facts.ambient_light, 0.5);
        // the rest stays the Nether's, not the Overworld's
        assert!(facts.fast_lava && facts.respawn_anchor_works);
        assert_eq!(facts.coordinate_scale, 8.0);
    }

    #[test]
    fn a_field_the_game_refuses_is_reported() {
        let mut o = WorldgenOverrides::default();
        o.add_mod_dimension(&ModDimension {
            id: "test:bad".into(),
            stem: r#"{"type":"test:bad"}"#.into(),
            dimension_type: r#"{"min_y":0,"height":64,"skybox":"moon"}"#.into(),
            sea_level: None,
        });
        assert!(o.level_facts("test:bad").unwrap().is_err());
    }
}

#[cfg(test)]
mod stem_sea_level_tests {
    use super::stem_sea_level;
    use serde_json::json;

    /// The sea level is the noise settings', named by id or inline; another generator has none.
    #[test]
    fn a_noise_stem_reads_its_settings_sea_level() {
        let read = |id: &str| (id == "test:high").then(|| json!({"sea_level": 120}));
        let named = |settings: &str| json!({"generator": {"type": "minecraft:noise", "settings": settings}});
        assert_eq!(stem_sea_level(&named("test:high"), &read), Some(120));
        assert_eq!(stem_sea_level(&named("test:absent"), &read), None);
        let inline = json!({"generator": {
            "type": "minecraft:noise", "settings": {"sea_level": -16},
        }});
        assert_eq!(stem_sea_level(&inline, &read), Some(-16));
        let flat = json!({"generator": {"type": "minecraft:flat", "settings": {"sea_level": 5}}});
        assert_eq!(stem_sea_level(&flat, &read), None);
        let bad = json!({"generator": {"type": "minecraft:noise", "settings": {"sea_level": "x"}}});
        assert_eq!(stem_sea_level(&bad, &read), None);
    }
}
