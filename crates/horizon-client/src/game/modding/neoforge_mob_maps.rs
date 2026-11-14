//! Build a complete immutable builtin generation alongside its common-store values.
use super::Game;
use horizon_client::neoforge_mob_maps::{self as builtin, BuiltinMap, Kind, MAPS, NeoForgeMobMaps};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
type Tags = HashMap<String, HashSet<String>>;

pub(crate) struct Loaded {
    pub values: Vec<(BuiltinMap, super::native_data_maps::MapValues)>,
    pub snapshot: Arc<NeoForgeMobMaps>,
}
struct Loader<'a, 'p> {
    definition: BuiltinMap,
    overlay: &'a horizon_world::loot::PackOverlay<'p>,
    tags: &'a Tags,
    known: &'a dyn Fn(&str, &str) -> bool,
    snapshot: &'a mut NeoForgeMobMaps,
}
impl Loader<'_, '_> {
    fn install_and_encode<T>(
        &mut self,
        values: BTreeMap<String, T>,
        applied: bool,
        install: impl FnOnce(&mut NeoForgeMobMaps, BTreeMap<String, T>),
        encode: impl Fn(&T) -> horizon_core::nbt::Tag,
    ) -> super::native_data_maps::MapValues {
        let field = self.definition.kind.field();
        let encoded = values
            .iter()
            .map(|(id, value)| {
                (
                    id.clone(),
                    horizon_core::nbt::Tag::compound([(field, encode(value))]),
                )
            })
            .collect();
        if applied {
            install(self.snapshot, values);
        }
        super::native_data_maps::MapValues::Decoded(encoded)
    }
    fn merge<T: Clone>(
        &mut self,
        codec: impl Fn(&Value) -> Result<T, String>,
        install: impl FnOnce(&mut NeoForgeMobMaps, BTreeMap<String, T>),
        encode: impl Fn(&T) -> horizon_core::nbt::Tag,
    ) -> Result<super::native_data_maps::MapValues, String> {
        let definition = self.definition;
        let base = builtin::vanilla_layer(definition.kind)
            .into_iter()
            .map(|(id, raw)| {
                codec(&raw)
                    .map(|value| (id, value))
                    .map_err(|error| format!("{} vanilla seed: {error}", definition.id))
            })
            .collect::<Result<_, _>>()?;
        let (values, applied) = super::data_map_files::load(
            definition.id,
            definition.directory,
            base,
            self.overlay,
            self.tags,
            |raw| codec(super::data_map_files::entry_value(raw)?).map(Some),
            |id| (self.known)(definition.registry, id),
        );
        Ok(self.install_and_encode(values, applied, install, encode))
    }
    fn merge_number<T: Clone>(
        &mut self,
        codec: impl Fn(&serde_json::value::RawValue) -> Result<T, String>,
        install: impl FnOnce(&mut NeoForgeMobMaps, BTreeMap<String, T>),
        encode: impl Fn(&T) -> horizon_core::nbt::Tag,
    ) -> Result<super::native_data_maps::MapValues, String> {
        let definition = self.definition;
        let field = definition.kind.field();
        let base = builtin::vanilla_layer(definition.kind)
            .into_iter()
            .map(|(id, raw)| {
                let number = serde_json::value::RawValue::from_string(raw[field].to_string())
                    .expect("static JSON number");
                codec(&number)
                    .map(|value| (id, value))
                    .map_err(|error| format!("{} vanilla seed: {error}", definition.id))
            })
            .collect::<Result<_, _>>()?;
        let (values, applied) = super::data_map_files::load_numeric(
            definition.id,
            definition.directory,
            base,
            self.overlay,
            self.tags,
            super::data_map_files::NumberCodec {
                field,
                decode: codec,
            },
            |id| (self.known)(definition.registry, id),
        );
        Ok(self.install_and_encode(values, applied, install, encode))
    }
}
pub(super) fn load<'a>(
    overlay: &horizon_world::loot::PackOverlay<'_>,
    tags: impl Fn(&str) -> Option<&'a Tags>,
    known: impl Fn(&str, &str) -> bool,
) -> Result<Loaded, String> {
    let none = Tags::new();
    let mut snapshot = NeoForgeMobMaps::default();
    let mut values = Vec::with_capacity(MAPS.len());
    for definition in MAPS {
        let mut loader = Loader {
            definition,
            overlay,
            tags: tags(definition.directory).unwrap_or(&none),
            known: &known,
            snapshot: &mut snapshot,
        };
        let raw = match definition.kind {
            Kind::VillagerDistances => loader.merge_number(
                builtin::decode_distance,
                NeoForgeMobMaps::set_distances,
                |value| horizon_core::nbt::Tag::Float(value.to_bits()),
            ),
            Kind::ParrotImitations => loader.merge(
                |value| builtin::decode_sound(value, |id| known("minecraft:sound_event", id)),
                NeoForgeMobMaps::set_parrots,
                |sound| {
                    horizon_core::nbt::Tag::String(if sound.is_empty() {
                        "minecraft:intentionally_empty".into()
                    } else {
                        horizon_client::mobs::entity_sounds::full_id(sound)
                            .as_ref()
                            .into()
                    })
                },
            ),
            Kind::VillagerTypes => loader.merge(
                builtin::decode_villager_type,
                NeoForgeMobMaps::set_villager_types,
                |value| {
                    let id = format!(
                        "minecraft:{}",
                        horizon_client::mobs::render::VILLAGER_TYPES[*value as usize]
                    );
                    horizon_core::nbt::Tag::String(id.as_str().into())
                },
            ),
            Kind::RaidHeroGifts => {
                loader.merge(builtin::decode_gift, NeoForgeMobMaps::set_gifts, |value| {
                    horizon_core::nbt::Tag::String(value.as_str().into())
                })
            }
            Kind::VibrationFrequencies => loader.merge_number(
                builtin::decode_frequency,
                NeoForgeMobMaps::set_frequencies,
                |value| horizon_core::nbt::Tag::Int(*value),
            ),
            Kind::VillagerCompostables => loader.merge(
                builtin::decode_compostable,
                NeoForgeMobMaps::set_compostables,
                |value| horizon_core::nbt::Tag::Byte(i8::from(*value)),
            ),
        };
        values.push((definition, raw?));
    }
    Ok(Loaded {
        values,
        snapshot: Arc::new(snapshot),
    })
}
impl Game<'_> {
    /// Borrow this world's generation; no lock or per-reader Arc increment.
    pub(in crate::game) fn mob_maps(&self) -> &NeoForgeMobMaps {
        self.mob_map_snapshot().as_ref()
    }
    pub(in crate::game) fn mob_map_snapshot(&self) -> &Arc<NeoForgeMobMaps> {
        if let Some(tables) = &self.world_tables {
            &tables.data_maps.mob_maps
        } else {
            NeoForgeMobMaps::vanilla()
        }
    }
    /// Generation workers pin one world-owned snapshot per finalization, not
    /// a process-global last-loaded world. In-flight work retains its snapshot.
    pub(in crate::game) fn share_mob_maps(&self) {
        if !self.options.execution_role.is_logical_server() {
            return;
        }
        let snapshot = self.mob_map_snapshot();
        if let Some(terrain) = self.end.locator() {
            terrain.set_mob_maps(snapshot);
        }
        self.levels.install_mob_map_snapshots(snapshot);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn invalid_values_refuse_replace_and_removals_before_unknown_target_filtering() {
        let files = vec![("data_maps/neoforge:entity_type/acceptable_villager_distances".into(), vec![
            serde_json::to_vec(&json!({"values": {"test:wraith": {"acceptable_villager_distance": 21}}})).unwrap(),
            serde_json::to_vec(&json!({"replace": true, "values": {"test:unknown": {"acceptable_villager_distance": "bad"}}, "remove": ["minecraft:zombie", "test:wraith"]})).unwrap(),
            serde_json::to_vec(&json!({"values": {"test:unknown": {"acceptable_villager_distance": 40}}, "remove": ["test:absent"]})).unwrap(),
        ])];
        let loaded = load(
            &horizon_world::loot::PackOverlay {
                files: &files,
                ..horizon_world::loot::PackOverlay::NONE
            },
            |_| None,
            |_, id| id.starts_with("minecraft:") || id == "test:wraith",
        )
        .unwrap();
        assert_eq!(loaded.snapshot.villager_distance("test:wraith"), Some(21.0));
        let raw = &loaded
            .values
            .iter()
            .find(|(definition, _)| definition.id == "neoforge:acceptable_villager_distances")
            .unwrap()
            .1;
        let super::super::native_data_maps::MapValues::Decoded(raw) = raw else {
            panic!("builtin codec stores decoded values");
        };
        assert_eq!(
            raw.get("test:wraith"),
            Some(&horizon_core::nbt::Tag::compound([(
                "acceptable_villager_distance",
                horizon_core::nbt::Tag::Float(21.0f32.to_bits())
            )]))
        );
        assert_eq!(
            raw.get("minecraft:zombie"),
            Some(&horizon_core::nbt::Tag::compound([(
                "acceptable_villager_distance",
                horizon_core::nbt::Tag::Float(8.0f32.to_bits())
            )]))
        );
        assert!(!raw.contains_key("test:unknown"));
    }

    #[test]
    fn lexical_numeric_files_keep_infinity_and_exact_decimal_truncation_in_both_views() {
        use super::super::native_data_maps::MapValues;
        use horizon_core::nbt::Tag;
        let files = vec![
            ("data_maps/neoforge:entity_type/acceptable_villager_distances".into(), vec![br#"{"values": {
                "test:overflow": {"value": {"acceptable_villager_distance": 1e500}},
                "test:negative": -3, "test:zero": -0.0,
                "test:disabled": {"neoforge:conditions": [{"type":"neoforge:mod_loaded","modid":"absent"}], "value":"bad"}
            }}"#.to_vec()]),
            ("data_maps/neoforge:game_event/vibration_frequencies".into(), vec![br#"{"values": {
                "test:wrap": 4294967297, "test:top": {"frequency":4294967311},
                "test:decimal": {"frequency":15.999999999999999999999}
            }}"#.to_vec()]),
        ];
        let loaded = load(
            &horizon_world::loot::PackOverlay {
                files: &files,
                ..horizon_world::loot::PackOverlay::NONE
            },
            |_| None,
            |_, _| true,
        )
        .unwrap();
        assert_eq!(
            loaded.snapshot.villager_distance("test:overflow"),
            Some(f32::INFINITY)
        );
        assert_eq!(
            loaded.snapshot.villager_distance("test:negative"),
            Some(-3.0)
        );
        assert_eq!(
            loaded
                .snapshot
                .villager_distance("test:zero")
                .unwrap()
                .to_bits(),
            0x8000_0000
        );
        assert_eq!(loaded.snapshot.villager_distance("test:disabled"), None);
        assert_eq!(loaded.snapshot.vibration_frequency("test:wrap"), 1);
        assert_eq!(loaded.snapshot.vibration_frequency("test:top"), 15);
        assert_eq!(loaded.snapshot.vibration_frequency("test:decimal"), 15);
        let tables: HashMap<_, _> = loaded
            .values
            .into_iter()
            .map(|(definition, values)| (definition.id, values))
            .collect();
        let MapValues::Decoded(distances) = &tables["neoforge:acceptable_villager_distances"]
        else {
            panic!("decoded builtin");
        };
        assert_eq!(
            distances.get("test:overflow"),
            Some(&Tag::compound([(
                "acceptable_villager_distance",
                Tag::Float(f32::INFINITY.to_bits())
            )]))
        );
        let MapValues::Decoded(frequencies) = &tables["neoforge:vibration_frequencies"] else {
            panic!("decoded builtin");
        };
        assert_eq!(
            frequencies.get("test:decimal"),
            Some(&Tag::compound([("frequency", Tag::Int(15))]))
        );
    }

    #[test]
    fn profession_and_event_tags_use_their_own_registry_and_unknown_sounds_refuse_files() {
        let files: Vec<_> = [
            ("data_maps/neoforge:villager_profession/raid_hero_gifts", json!({"values": {"#test:workers": {"loot_table": "test:gifts"}}})),
            ("data_maps/neoforge:game_event/vibration_frequencies", json!({"values": {"#test:steps": {"frequency": 7.9}}})),
            ("data_maps/neoforge:entity_type/parrot_imitations", json!({"replace": true, "values": {"test:unknown": {"sound": "test:missing"}}, "remove": ["minecraft:zombie"]})),
        ].into_iter().map(|(path, value)| (path.to_owned(), vec![serde_json::to_vec(&value).unwrap()])).collect();
        let professions = HashMap::from([(
            "test:workers".into(),
            HashSet::from(["test:artisan".into()]),
        )]);
        let events = HashMap::from([("test:steps".into(), HashSet::from(["test:rustle".into()]))]);
        let loaded = load(
            &horizon_world::loot::PackOverlay {
                files: &files,
                ..horizon_world::loot::PackOverlay::NONE
            },
            |directory| match directory {
                "villager_profession" => Some(&professions),
                "game_event" => Some(&events),
                _ => None,
            },
            |_, id| id.starts_with("minecraft:") || matches!(id, "test:artisan" | "test:rustle"),
        )
        .unwrap();
        assert_eq!(
            loaded.snapshot.hero_gift("test:artisan"),
            Some("test:gifts".into())
        );
        assert_eq!(loaded.snapshot.vibration_frequency("test:rustle"), 7);
        assert!(loaded.snapshot.parrot_sounds().is_none());
        assert_eq!(
            loaded.snapshot.parrot_imitation("minecraft:zombie"),
            Some("entity.parrot.imitate.zombie")
        );
    }
}
