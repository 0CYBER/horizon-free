//! Biome modifiers declared in code: the operations the data-file modifiers
//! (`data/<ns>/neoforge/biome_modifier/`) already do, as data a mod registers with
//! [`Registrar::declare_biome_modifier`](crate::BiomeModifiersRegistrar::declare_biome_modifier).
use serde_json::{Value, json};

/// Vanilla's generation steps, by name, in order (`GenerationStep.Decoration`).
const STEPS: [&str; 11] = [
    "raw_generation",
    "lakes",
    "local_modifications",
    "underground_structures",
    "surface_structures",
    "strongholds",
    "underground_ores",
    "underground_decoration",
    "fluid_springs",
    "vegetal_decoration",
    "top_layer_modification",
];

/// One mob spawner entry of a [`BiomeModifierOperation::AddSpawns`] (`SpawnerData`).
#[derive(Clone, Debug, PartialEq)]
// exhaustive: a mod builds it with a literal; the game reads it
#[derive(horizon_mod_api_macros::Wire)]
pub struct BiomeModifierSpawner {
    /// The entity type (`mymod:ghoul`).
    pub entity: String,
    /// Relative weight in the spawn category; zero is allowed.
    pub weight: u32,
    /// Smallest group size.
    pub min_count: u32,
    /// Largest group size; at least `min_count`.
    pub max_count: u32,
}

/// A change to a colour that 26.3 treats as an optional override of the biome's `effects`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: a mod builds it with a literal; the game reads it
#[derive(horizon_mod_api_macros::Wire)]
pub enum BiomeColourChange {
    /// Sets the colour, 24-bit RGB (0..=0xFFFFFF).
    Set {
        /// The colour.
        rgb: u32,
    },
    /// Removes the override, so the colour comes from the colormap again.
    Remove,
}

/// One ambient particle of a biome (`minecraft:visual/ambient_particles`).
#[derive(Clone, Debug, PartialEq)]
// exhaustive: a mod builds it with a literal; the game reads it
#[derive(horizon_mod_api_macros::Wire)]
pub struct BiomeAmbientParticle {
    /// The particle type (`minecraft:ash`).
    pub particle: String,
    /// The chance per tick and block, 0.0..=1.0.
    pub probability: f64,
}

/// A random ambient sound of a biome (`additions` of `minecraft:audio/ambient_sounds`).
#[derive(Clone, Debug, PartialEq)]
// exhaustive: a mod builds it with a literal; the game reads it
#[derive(horizon_mod_api_macros::Wire)]
pub struct BiomeAmbientAddition {
    /// The sound event (`minecraft:ambient.cave`).
    pub sound: String,
    /// The finite chance per tick (not range-clamped).
    pub tick_chance: f64,
}

/// The cave mood sound of a biome (`mood` of `minecraft:audio/ambient_sounds`).
#[derive(Clone, Debug, PartialEq)]
// exhaustive: a mod builds it with a literal; the game reads it
#[derive(horizon_mod_api_macros::Wire)]
pub struct BiomeAmbientMood {
    /// The sound event.
    pub sound: String,
    /// Ticks the mood takes to build up.
    pub tick_delay: u32,
    /// How far around the player a spot for the sound is searched.
    pub block_search_extent: u32,
    /// How far from the listener the sound is placed.
    pub offset: f64,
}

/// The default background music of a biome (`default` of `minecraft:audio/background_music`).
#[derive(Clone, Debug, PartialEq)]
// exhaustive: a mod builds it with a literal; the game reads it
#[derive(horizon_mod_api_macros::Wire)]
pub struct BiomeMusic {
    /// The sound event (`minecraft:music.overworld.meadow`).
    pub sound: String,
    /// Fewest ticks between two tracks.
    pub min_delay: u32,
    /// Most ticks between two tracks, independently nonnegative.
    pub max_delay: u32,
}

/// What a biome modifier does to its biomes.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum BiomeModifierOperation {
    /// `add_features`: append placed features to one generation step.
    AddFeatures {
        /// Placed feature ids (`mymod:ore`) or tags (`#mymod:tag`).
        features: Vec<String>,
        /// The generation step (`underground_ores`, ...).
        step: String,
    },
    /// `remove_features`: take placed features out of generation steps.
    RemoveFeatures {
        /// Placed feature ids or tags.
        features: Vec<String>,
        /// The steps to take them out of; none means every step.
        steps: Vec<String>,
    },
    /// `add_spawns`: append spawners to their entity types' spawn categories.
    AddSpawns {
        /// The spawners to add.
        spawners: Vec<BiomeModifierSpawner>,
    },
    /// `remove_spawns`: take entity types out of every spawn category.
    RemoveSpawns {
        /// Entity type ids or tags.
        entity_types: Vec<String>,
    },
    /// `add_spawn_costs`: set the spawn cost of entity types.
    AddSpawnCosts {
        /// Entity type ids or tags.
        entity_types: Vec<String>,
        /// The `energy_budget` of the cost.
        energy_budget: f64,
        /// The `charge` of the cost.
        charge: f64,
    },
    /// `remove_spawn_costs`: take entity types out of `spawn_costs`.
    RemoveSpawnCosts {
        /// Entity type ids or tags.
        entity_types: Vec<String>,
    },
    /// `add_carvers`: append configured carvers to the biome's `carvers`.
    AddCarvers {
        /// Configured carver ids (`mymod:pit`) or tags.
        carvers: Vec<String>,
    },
    /// `remove_carvers`: take configured carvers out of the biome's `carvers`.
    RemoveCarvers {
        /// Configured carver ids or tags.
        carvers: Vec<String>,
    },
    /// `horizon:modify_biome` (NeoForge's MODIFY phase): set single values of the biome, after
    /// every addition and removal. A `None` leaves the value as it is.
    ModifyBiome {
        /// `temperature`.
        temperature: Option<f64>,
        /// `downfall`.
        downfall: Option<f64>,
        /// `has_precipitation`.
        has_precipitation: Option<bool>,
        /// `temperature_modifier`: `none` or `frozen`.
        temperature_modifier: Option<String>,
        /// `effects.sky_color`, 24-bit RGB (0..=0xFFFFFF).
        sky_color: Option<u32>,
        /// `effects.fog_color`, 24-bit RGB.
        fog_color: Option<u32>,
        /// `effects.water_color`, 24-bit RGB.
        water_color: Option<u32>,
        /// `effects.water_fog_color`, 24-bit RGB.
        water_fog_color: Option<u32>,
        /// `effects.foliage_color`: set it or remove the override.
        foliage_color: Option<BiomeColourChange>,
        /// `effects.dry_foliage_color`: set it or remove the override.
        dry_foliage_color: Option<BiomeColourChange>,
        /// `effects.grass_color`: set it or remove the override.
        grass_color: Option<BiomeColourChange>,
        /// `minecraft:visual/ambient_particles`: replaces the biome's particles (an empty list
        /// removes them).
        ambient_particles: Option<Vec<BiomeAmbientParticle>>,
        /// `loop` of `minecraft:audio/ambient_sounds`. Setting this, `ambient_mood` or
        /// `ambient_additions` replaces the biome's whole ambient sounds value with the ones set.
        ambient_loop: Option<String>,
        /// `additions` of `minecraft:audio/ambient_sounds`.
        ambient_additions: Option<Vec<BiomeAmbientAddition>>,
        /// `mood` of `minecraft:audio/ambient_sounds`.
        ambient_mood: Option<BiomeAmbientMood>,
        /// `default` of `minecraft:audio/background_music`; replaces the whole value, so the
        /// biome's creative and underwater music go.
        background_music: Option<BiomeMusic>,
        /// `minecraft:audio/music_volume`, 0.0..=1.0.
        music_volume: Option<f64>,
    },
}

/// A biome modifier: the biomes it applies to and what it does to them.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: a mod builds it with a literal; the game reads it
#[derive(horizon_mod_api_macros::Wire)]
pub struct BiomeModifier {
    /// Biome ids (`minecraft:plains`) or biome tags (`#minecraft:is_forest`).
    pub biomes: Vec<String>,
    /// The change.
    pub operation: BiomeModifierOperation,
}

/// Why a biome modifier was refused
/// ([`Registrar::declare_biome_modifier`](crate::BiomeModifiersRegistrar::declare_biome_modifier)).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum BiomeModifierError {
    /// The modifier's own id is empty or in a namespace the declaring mod does not own.
    InvalidId,
    /// Another modifier of the mod has the id.
    Duplicate,
    /// A list that must name something is empty.
    EmptyList,
    /// An id or tag is not namespaced (`mymod:thing`, a tag with a leading `#`).
    InvalidName,
    /// A generation step name vanilla does not have.
    UnknownStep,
    /// A spawner outside signed integer bounds, with an empty group, or inverted group bounds.
    InvalidSpawner,
    /// A spawn cost that is not a finite number.
    InvalidCost,
    /// A climate number that is not finite, a colour above 0xFFFFFF, or a temperature modifier
    /// other than `none` or `frozen`.
    InvalidValue,
}

fn name_ok(name: &str, tags: bool) -> bool {
    let id = match name.strip_prefix('#') {
        Some(tag) if tags => tag,
        Some(_) => return false,
        None => name,
    };
    id.split_once(':')
        .is_some_and(|(namespace, path)| !namespace.is_empty() && !path.is_empty())
}

fn names_ok(names: &[String], tags: bool) -> Result<(), BiomeModifierError> {
    if names.is_empty() {
        return Err(BiomeModifierError::EmptyList);
    }
    match names.iter().all(|n| name_ok(n, tags)) {
        true => Ok(()),
        false => Err(BiomeModifierError::InvalidName),
    }
}

fn step_ok(step: &str) -> Result<(), BiomeModifierError> {
    match STEPS.contains(&step) {
        true => Ok(()),
        false => Err(BiomeModifierError::UnknownStep),
    }
}

impl BiomeModifier {
    /// Whether every field is something the game can apply; the first problem found.
    pub fn check(&self) -> Result<(), BiomeModifierError> {
        names_ok(&self.biomes, true)?;
        match &self.operation {
            BiomeModifierOperation::AddFeatures { features, step } => {
                names_ok(features, true)?;
                step_ok(step)
            }
            BiomeModifierOperation::RemoveFeatures { features, steps } => {
                names_ok(features, true)?;
                steps.iter().map(String::as_str).try_for_each(step_ok)
            }
            BiomeModifierOperation::AddSpawns { spawners } => {
                if spawners.is_empty() {
                    return Err(BiomeModifierError::EmptyList);
                }
                for spawner in spawners {
                    names_ok(std::slice::from_ref(&spawner.entity), false)?;
                    if spawner.weight > i32::MAX as u32
                        || spawner.min_count == 0
                        || spawner.min_count > spawner.max_count
                        || spawner.max_count > i32::MAX as u32
                    {
                        return Err(BiomeModifierError::InvalidSpawner);
                    }
                }
                Ok(())
            }
            BiomeModifierOperation::RemoveSpawns { entity_types }
            | BiomeModifierOperation::RemoveSpawnCosts { entity_types } => {
                names_ok(entity_types, true)
            }
            BiomeModifierOperation::AddCarvers { carvers }
            | BiomeModifierOperation::RemoveCarvers { carvers } => names_ok(carvers, true),
            BiomeModifierOperation::ModifyBiome {
                temperature,
                downfall,
                temperature_modifier,
                sky_color,
                fog_color,
                water_color,
                water_fog_color,
                foliage_color,
                dry_foliage_color,
                grass_color,
                ambient_particles,
                ambient_loop,
                ambient_additions,
                ambient_mood,
                background_music,
                music_volume,
                ..
            } => {
                let change = |c: &Option<BiomeColourChange>| match c {
                    Some(BiomeColourChange::Set { rgb }) => Some(*rgb),
                    _ => None,
                };
                let (foliage_color, dry_foliage_color, grass_color) = (
                    change(foliage_color),
                    change(dry_foliage_color),
                    change(grass_color),
                );
                let id = |s: &String| name_ok(s, false);
                let unit = |n: &f64| (0.0..=1.0).contains(n);
                let ticks = |n: &u32| i32::try_from(*n).is_ok();
                let effects_ok = ambient_particles
                    .iter()
                    .flatten()
                    .all(|p| id(&p.particle) && unit(&p.probability))
                    && ambient_loop.iter().all(id)
                    && ambient_additions
                        .iter()
                        .flatten()
                        .all(|a| id(&a.sound) && a.tick_chance.is_finite())
                    && ambient_mood.iter().all(|m| {
                        id(&m.sound)
                            && ticks(&m.tick_delay)
                            && ticks(&m.block_search_extent)
                            && m.offset.is_finite()
                    })
                    && background_music
                        .iter()
                        .all(|m| id(&m.sound) && ticks(&m.max_delay) && ticks(&m.min_delay))
                    && music_volume.iter().all(unit);
                let finite = [temperature, downfall]
                    .iter()
                    .all(|n| n.is_none_or(f64::is_finite));
                let colours = [
                    sky_color,
                    fog_color,
                    water_color,
                    water_fog_color,
                    &foliage_color,
                    &dry_foliage_color,
                    &grass_color,
                ];
                let modifier = temperature_modifier
                    .as_deref()
                    .is_none_or(|m| m == "none" || m == "frozen");
                let colours_ok = colours.iter().all(|c| c.is_none_or(|c| c <= 0xFF_FFFF));
                match finite && modifier && colours_ok && effects_ok {
                    true => Ok(()),
                    false => Err(BiomeModifierError::InvalidValue),
                }
            }
            BiomeModifierOperation::AddSpawnCosts {
                entity_types,
                energy_budget,
                charge,
            } => {
                names_ok(entity_types, true)?;
                match energy_budget.is_finite() && charge.is_finite() {
                    true => Ok(()),
                    false => Err(BiomeModifierError::InvalidCost),
                }
            }
        }
    }

    /// The modifier as the NeoForge-shaped JSON file the data-file reader takes, so a modifier
    /// declared in code and one shipped as a file are applied by the same code.
    pub fn json(&self) -> String {
        // One entry is the plain id, as NeoForge writes a single holder.
        let set = |names: &[String]| match names {
            [one] => json!(one),
            many => json!(many),
        };
        let mut file = match &self.operation {
            BiomeModifierOperation::AddFeatures { features, step } => json!({
                "type": "neoforge:add_features",
                "features": set(features),
                "step": step,
            }),
            BiomeModifierOperation::RemoveFeatures { features, steps } => {
                let mut file = json!({
                    "type": "neoforge:remove_features",
                    "features": set(features),
                });
                if !steps.is_empty() {
                    file["steps"] = json!(steps);
                }
                file
            }
            BiomeModifierOperation::AddSpawns { spawners } => json!({
                "type": "neoforge:add_spawns",
                "spawners": spawners.iter().map(|s| json!({
                    "type": s.entity,
                    "weight": s.weight,
                    "minCount": s.min_count,
                    "maxCount": s.max_count,
                })).collect::<Vec<Value>>(),
            }),
            BiomeModifierOperation::RemoveSpawns { entity_types } => json!({
                "type": "neoforge:remove_spawns",
                "entity_types": set(entity_types),
            }),
            BiomeModifierOperation::AddSpawnCosts {
                entity_types,
                energy_budget,
                charge,
            } => json!({
                "type": "neoforge:add_spawn_costs",
                "entity_types": set(entity_types),
                "spawn_cost": {"energy_budget": energy_budget, "charge": charge},
            }),
            BiomeModifierOperation::RemoveSpawnCosts { entity_types } => json!({
                "type": "neoforge:remove_spawn_costs",
                "entity_types": set(entity_types),
            }),
            BiomeModifierOperation::AddCarvers { carvers } => json!({
                "type": "neoforge:add_carvers",
                "carvers": set(carvers),
            }),
            BiomeModifierOperation::RemoveCarvers { carvers } => json!({
                "type": "neoforge:remove_carvers",
                "carvers": set(carvers),
            }),
            BiomeModifierOperation::ModifyBiome {
                temperature,
                downfall,
                has_precipitation,
                temperature_modifier,
                sky_color,
                fog_color,
                water_color,
                water_fog_color,
                foliage_color,
                dry_foliage_color,
                grass_color,
                ambient_particles,
                ambient_loop,
                ambient_additions,
                ambient_mood,
                background_music,
                music_volume,
            } => {
                let mut file = json!({"type": "horizon:modify_biome"});
                // The vanilla attribute values, under the biome's `attributes`.
                let mut attributes = serde_json::Map::new();
                if let Some(list) = ambient_particles {
                    attributes.insert(
                        "minecraft:visual/ambient_particles".into(),
                        list.iter()
                            .map(|p| {
                                json!({"particle": {"type": p.particle}, "probability": p.probability})
                            })
                            .collect(),
                    );
                }
                let mut sounds = serde_json::Map::new();
                if let Some(sound) = ambient_loop {
                    sounds.insert("loop".into(), json!(sound));
                }
                if let Some(list) = ambient_additions {
                    let additions = list
                        .iter()
                        .map(|a| json!({"sound": a.sound, "tick_chance": a.tick_chance}));
                    sounds.insert("additions".into(), additions.collect());
                }
                if let Some(m) = ambient_mood {
                    sounds.insert(
                        "mood".into(),
                        json!({"sound": m.sound, "tick_delay": m.tick_delay,
                            "block_search_extent": m.block_search_extent, "offset": m.offset}),
                    );
                }
                if !sounds.is_empty() {
                    attributes.insert(
                        "minecraft:audio/ambient_sounds".into(),
                        Value::Object(sounds),
                    );
                }
                if let Some(m) = background_music {
                    attributes.insert(
                        "minecraft:audio/background_music".into(),
                        json!({"default": {"sound": m.sound, "min_delay": m.min_delay,
                            "max_delay": m.max_delay}}),
                    );
                }
                if let Some(volume) = music_volume {
                    attributes.insert("minecraft:audio/music_volume".into(), json!(volume));
                }
                if !attributes.is_empty() {
                    file["attributes"] = Value::Object(attributes);
                }
                for (key, value) in [
                    ("temperature", json!(temperature)),
                    ("downfall", json!(downfall)),
                    ("has_precipitation", json!(has_precipitation)),
                    ("temperature_modifier", json!(temperature_modifier)),
                ] {
                    if !value.is_null() {
                        file[key] = value;
                    }
                }
                let mut effects = serde_json::Map::new();
                let change = |c: &Option<BiomeColourChange>| match c {
                    Some(BiomeColourChange::Set { rgb }) => Some(json!(rgb)),
                    // `null` removes the override.
                    Some(BiomeColourChange::Remove) => Some(Value::Null),
                    None => None,
                };
                for (key, colour) in [
                    ("sky_color", sky_color.map(|c| json!(c))),
                    ("fog_color", fog_color.map(|c| json!(c))),
                    ("water_color", water_color.map(|c| json!(c))),
                    ("water_fog_color", water_fog_color.map(|c| json!(c))),
                    ("foliage_color", change(foliage_color)),
                    ("dry_foliage_color", change(dry_foliage_color)),
                    ("grass_color", change(grass_color)),
                ] {
                    if let Some(colour) = colour {
                        effects.insert(key.into(), colour);
                    }
                }
                if !effects.is_empty() {
                    file["effects"] = Value::Object(effects);
                }
                file
            }
        };
        file["biomes"] = set(&self.biomes);
        file.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    fn modifier(operation: BiomeModifierOperation) -> BiomeModifier {
        BiomeModifier {
            biomes: strings(&["#minecraft:is_forest"]),
            operation,
        }
    }

    #[test]
    fn a_valid_modifier_checks_and_renders_the_data_file_shape() {
        let m = modifier(BiomeModifierOperation::AddFeatures {
            features: strings(&["ruby:ruby_ore"]),
            step: "underground_ores".into(),
        });
        assert_eq!(m.check(), Ok(()));
        let file: Value = serde_json::from_str(&m.json()).unwrap();
        assert_eq!(
            file,
            json!({
                "type": "neoforge:add_features",
                "biomes": "#minecraft:is_forest",
                "features": "ruby:ruby_ore",
                "step": "underground_ores",
            })
        );
    }

    #[test]
    fn carver_operations_render_neoforges_field_names() {
        let add = modifier(BiomeModifierOperation::AddCarvers {
            carvers: strings(&["ruby:pit"]),
        });
        assert_eq!(add.check(), Ok(()));
        let file: Value = serde_json::from_str(&add.json()).unwrap();
        assert_eq!(file["type"], "neoforge:add_carvers");
        assert_eq!(file["carvers"], "ruby:pit");
        let remove = modifier(BiomeModifierOperation::RemoveCarvers {
            carvers: strings(&["minecraft:cave", "ruby:pit"]),
        });
        let file: Value = serde_json::from_str(&remove.json()).unwrap();
        assert_eq!(file["type"], "neoforge:remove_carvers");
        assert_eq!(file["carvers"], json!(["minecraft:cave", "ruby:pit"]));
        let empty = modifier(BiomeModifierOperation::AddCarvers { carvers: vec![] });
        assert_eq!(empty.check(), Err(BiomeModifierError::EmptyList));
    }

    #[test]
    fn modify_biome_renders_vanilla_biome_field_names_and_checks_values() {
        let operation =
            |temperature_modifier: Option<&str>, sky_color| BiomeModifierOperation::ModifyBiome {
                temperature: Some(0.25),
                downfall: None,
                has_precipitation: Some(false),
                temperature_modifier: temperature_modifier.map(String::from),
                sky_color,
                fog_color: None,
                water_color: Some(0x3938c9),
                water_fog_color: None,
                foliage_color: None,
                dry_foliage_color: None,
                grass_color: None,
                ambient_particles: None,
                ambient_loop: None,
                ambient_additions: None,
                ambient_mood: None,
                background_music: None,
                music_volume: None,
            };
        let ok = modifier(operation(Some("frozen"), Some(0xc8e0ff)));
        assert_eq!(ok.check(), Ok(()));
        let file: Value = serde_json::from_str(&ok.json()).unwrap();
        assert_eq!(
            file,
            json!({
                "type": "horizon:modify_biome",
                "biomes": "#minecraft:is_forest",
                "temperature": 0.25,
                "has_precipitation": false,
                "temperature_modifier": "frozen",
                "effects": {"sky_color": 0xc8e0ff, "water_color": 0x3938c9},
            })
        );
        for bad in [
            operation(Some("warm"), None),
            operation(None, Some(0x100_0000)),
        ] {
            assert_eq!(modifier(bad).check(), Err(BiomeModifierError::InvalidValue));
        }
    }

    fn effects() -> BiomeModifierOperation {
        BiomeModifierOperation::ModifyBiome {
            temperature: None,
            downfall: None,
            has_precipitation: None,
            temperature_modifier: None,
            sky_color: None,
            fog_color: None,
            water_color: None,
            water_fog_color: None,
            foliage_color: None,
            dry_foliage_color: None,
            grass_color: None,
            ambient_particles: Some(vec![BiomeAmbientParticle {
                particle: "minecraft:ash".into(),
                probability: 0.25,
            }]),
            ambient_loop: Some("minecraft:ambient.warped_forest.loop".into()),
            ambient_additions: Some(vec![BiomeAmbientAddition {
                sound: "minecraft:ambient.warped_forest.additions".into(),
                tick_chance: -0.5,
            }]),
            ambient_mood: Some(BiomeAmbientMood {
                sound: "minecraft:ambient.warped_forest.mood".into(),
                tick_delay: 6000,
                block_search_extent: 8,
                offset: 2.0,
            }),
            background_music: Some(BiomeMusic {
                sound: "huts:music.dunes".into(),
                min_delay: 25000,
                max_delay: 24000,
            }),
            music_volume: Some(0.5),
        }
    }

    #[test]
    fn modify_biome_sets_or_removes_the_optional_colour_overrides() {
        let mut op = effects();
        if let BiomeModifierOperation::ModifyBiome {
            foliage_color,
            dry_foliage_color,
            grass_color,
            ..
        } = &mut op
        {
            *foliage_color = Some(BiomeColourChange::Remove);
            *dry_foliage_color = Some(BiomeColourChange::Set { rgb: 0x8c3a04 });
            *grass_color = Some(BiomeColourChange::Set { rgb: 0x1000000 });
        }
        assert_eq!(
            modifier(op.clone()).check(),
            Err(BiomeModifierError::InvalidValue)
        );
        if let BiomeModifierOperation::ModifyBiome { grass_color, .. } = &mut op {
            *grass_color = Some(BiomeColourChange::Remove);
        }
        assert_eq!(modifier(op.clone()).check(), Ok(()));
        let file: Value = serde_json::from_str(&modifier(op).json()).unwrap();
        assert_eq!(
            file["effects"],
            json!({"foliage_color": null, "dry_foliage_color": 0x8c3a04, "grass_color": null})
        );
    }

    #[test]
    fn modify_biome_renders_the_effect_attributes_and_refuses_bad_values() {
        let ok = modifier(effects());
        assert_eq!(ok.check(), Ok(()));
        let file: Value = serde_json::from_str(&ok.json()).unwrap();
        assert_eq!(
            file["attributes"],
            json!({
                "minecraft:visual/ambient_particles":
                    [{"particle": {"type": "minecraft:ash"}, "probability": 0.25}],
                "minecraft:audio/ambient_sounds": {
                    "loop": "minecraft:ambient.warped_forest.loop",
                    "additions": [{"sound": "minecraft:ambient.warped_forest.additions",
                        "tick_chance": -0.5}],
                    "mood": {"sound": "minecraft:ambient.warped_forest.mood", "tick_delay": 6000,
                        "block_search_extent": 8, "offset": 2.0},
                },
                "minecraft:audio/background_music": {"default": {"sound": "huts:music.dunes",
                    "min_delay": 25000, "max_delay": 24000}},
                "minecraft:audio/music_volume": 0.5,
            })
        );
        let with = |change: &dyn Fn(&mut BiomeModifierOperation)| {
            let mut op = effects();
            change(&mut op);
            modifier(op).check()
        };
        let set = |op: &mut BiomeModifierOperation, f: &dyn Fn(&mut Vec<BiomeAmbientParticle>)| {
            if let BiomeModifierOperation::ModifyBiome {
                ambient_particles: Some(list),
                ..
            } = op
            {
                f(list);
            }
        };
        assert_eq!(
            with(&|op| set(op, &|l| l[0].probability = 1.5)),
            Err(BiomeModifierError::InvalidValue)
        );
        assert_eq!(
            with(&|op| set(op, &|l| l[0].particle = "ash".into())),
            Err(BiomeModifierError::InvalidValue)
        );
        let volume = |op: &mut BiomeModifierOperation| {
            if let BiomeModifierOperation::ModifyBiome { music_volume, .. } = op {
                *music_volume = Some(f64::NAN);
            }
        };
        assert_eq!(with(&volume), Err(BiomeModifierError::InvalidValue));
        let music = |op: &mut BiomeModifierOperation| {
            if let BiomeModifierOperation::ModifyBiome {
                background_music: Some(m),
                ..
            } = op
            {
                m.min_delay = m.max_delay + 1;
            }
        };
        assert_eq!(with(&music), Ok(()));
    }

    #[test]
    fn spawn_operations_render_neoforges_field_names() {
        let spawns = modifier(BiomeModifierOperation::AddSpawns {
            spawners: vec![BiomeModifierSpawner {
                entity: "ruby:ghoul".into(),
                weight: 5,
                min_count: 1,
                max_count: 3,
            }],
        });
        assert_eq!(spawns.check(), Ok(()));
        let file: Value = serde_json::from_str(&spawns.json()).unwrap();
        assert_eq!(
            file["spawners"],
            json!([{"type": "ruby:ghoul", "weight": 5, "minCount": 1, "maxCount": 3}])
        );
        let cost = modifier(BiomeModifierOperation::AddSpawnCosts {
            entity_types: strings(&["ruby:ghoul", "minecraft:zombie"]),
            energy_budget: 0.5,
            charge: 0.25,
        });
        let file: Value = serde_json::from_str(&cost.json()).unwrap();
        assert_eq!(
            file["entity_types"],
            json!(["ruby:ghoul", "minecraft:zombie"])
        );
        assert_eq!(
            file["spawn_cost"],
            json!({"energy_budget": 0.5, "charge": 0.25})
        );
        // No steps means every step: the file leaves `steps` out.
        let remove = modifier(BiomeModifierOperation::RemoveFeatures {
            features: strings(&["minecraft:ore_coal_upper"]),
            steps: Vec::new(),
        });
        let file: Value = serde_json::from_str(&remove.json()).unwrap();
        assert!(file.get("steps").is_none());
    }

    #[test]
    fn spawn_boundaries_match_the_reader() {
        for (weight, min_count, max_count, valid) in [
            (0, 1, 1, true),
            (i32::MAX as u32, 1, i32::MAX as u32, true),
            (1, 0, 1, false),
            (u32::MAX, 1, 1, false),
            (1, u32::MAX, u32::MAX, false),
            (1, 1, u32::MAX, false),
        ] {
            let m = modifier(BiomeModifierOperation::AddSpawns {
                spawners: vec![BiomeModifierSpawner {
                    entity: "minecraft:zombie".into(),
                    weight,
                    min_count,
                    max_count,
                }],
            });
            assert_eq!(m.check().is_ok(), valid);
        }
    }

    #[test]
    fn invalid_modifiers_are_refused_by_reason() {
        let bad = |op| modifier(op).check();
        assert_eq!(
            bad(BiomeModifierOperation::AddFeatures {
                features: strings(&["ore"]),
                step: "underground_ores".into(),
            }),
            Err(BiomeModifierError::InvalidName)
        );
        assert_eq!(
            bad(BiomeModifierOperation::AddFeatures {
                features: strings(&["ruby:ore"]),
                step: "underground".into(),
            }),
            Err(BiomeModifierError::UnknownStep)
        );
        assert_eq!(
            bad(BiomeModifierOperation::RemoveSpawns {
                entity_types: Vec::new()
            }),
            Err(BiomeModifierError::EmptyList)
        );
        // A spawner names one entity type, never a tag.
        let spawner = |entity: &str, weight, min_count, max_count| BiomeModifierSpawner {
            entity: entity.into(),
            weight,
            min_count,
            max_count,
        };
        for (spawner, error) in [
            (
                spawner("#ruby:ghouls", 1, 1, 1),
                BiomeModifierError::InvalidName,
            ),
            (
                spawner("ruby:ghoul", 1, 0, 1),
                BiomeModifierError::InvalidSpawner,
            ),
            (
                spawner("ruby:ghoul", 1, 4, 2),
                BiomeModifierError::InvalidSpawner,
            ),
        ] {
            assert_eq!(
                bad(BiomeModifierOperation::AddSpawns {
                    spawners: vec![spawner]
                }),
                Err(error)
            );
        }
        assert_eq!(
            bad(BiomeModifierOperation::AddSpawnCosts {
                entity_types: strings(&["ruby:ghoul"]),
                energy_budget: f64::NAN,
                charge: 1.0,
            }),
            Err(BiomeModifierError::InvalidCost)
        );
        // The biomes are checked too.
        let mut m = modifier(BiomeModifierOperation::RemoveSpawns {
            entity_types: strings(&["minecraft:zombie"]),
        });
        m.biomes = strings(&["plains"]);
        assert_eq!(m.check(), Err(BiomeModifierError::InvalidName));
    }
}
