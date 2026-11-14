//! Immutable, per-world built-in data maps. File codecs run before installation;
//! simulation readers use exact registry identities and never parse JSON or lock
//! a process-global current world. The common store owns each generation.
use crate::mobs::{Species, parrot, render::VILLAGER_TYPES, species};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, LazyLock};

#[derive(Clone, Copy, Debug)]
pub enum Kind {
    VillagerDistances,
    ParrotImitations,
    VillagerTypes,
    RaidHeroGifts,
    VibrationFrequencies,
    VillagerCompostables,
}
impl Kind {
    pub const fn field(self) -> &'static str {
        match self {
            Self::VillagerDistances => "acceptable_villager_distance",
            Self::ParrotImitations => "sound",
            Self::VillagerTypes => "villager_type",
            Self::RaidHeroGifts => "loot_table",
            Self::VibrationFrequencies => "frequency",
            Self::VillagerCompostables => "can_villager_compost",
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct BuiltinMap {
    pub id: &'static str,
    pub directory: &'static str,
    pub registry: &'static str,
    pub kind: Kind,
}
pub const MAPS: [BuiltinMap; 6] = [
    BuiltinMap {
        id: "neoforge:acceptable_villager_distances",
        directory: "entity_type",
        registry: "minecraft:entity_type",
        kind: Kind::VillagerDistances,
    },
    BuiltinMap {
        id: "neoforge:parrot_imitations",
        directory: "entity_type",
        registry: "minecraft:entity_type",
        kind: Kind::ParrotImitations,
    },
    BuiltinMap {
        id: "neoforge:villager_types",
        directory: "worldgen/biome",
        registry: "minecraft:worldgen/biome",
        kind: Kind::VillagerTypes,
    },
    BuiltinMap {
        id: "neoforge:raid_hero_gifts",
        directory: "villager_profession",
        registry: "minecraft:villager_profession",
        kind: Kind::RaidHeroGifts,
    },
    BuiltinMap {
        id: "neoforge:vibration_frequencies",
        directory: "game_event",
        registry: "minecraft:game_event",
        kind: Kind::VibrationFrequencies,
    },
    BuiltinMap {
        id: "neoforge:villager_compostables",
        directory: "item",
        registry: "minecraft:item",
        kind: Kind::VillagerCompostables,
    },
];
/// The professions with a hero gift (`GiveGiftToHero.GIFTS`).
const GIFT_PROFESSIONS: [&str; 13] = [
    "armorer",
    "butcher",
    "cartographer",
    "cleric",
    "farmer",
    "fisherman",
    "fletcher",
    "leatherworker",
    "librarian",
    "mason",
    "shepherd",
    "toolsmith",
    "weaponsmith",
];

/// `WorkAtComposter.COMPOSTABLE_ITEMS`.
const COMPOSTABLES: [&str; 2] = ["minecraft:wheat_seeds", "minecraft:beetroot_seeds"];

/// `SoundEvents.EMPTY`'s id: the happy ghast's silent imitation.
const EMPTY_SOUND: &str = "minecraft:intentionally_empty";

/// `VillagerHostilesSensor.ACCEPTABLE_DISTANCE_FROM_HOSTILES`.
pub fn vanilla_villager_distance(s: Species) -> Option<f32> {
    Some(match s {
        Species::Drowned
        | Species::Husk
        | Species::Vex
        | Species::Zombie
        | Species::ZombieVillager => 8.0,
        Species::Evoker | Species::Illusioner | Species::Ravager => 12.0,
        Species::Pillager => 15.0,
        Species::Vindicator | Species::Zoglin => 10.0,
        _ => return None,
    })
}

/// Canonical Minecraft ids may be stored by their local name internally.
fn local_id(id: &str) -> &str {
    id.strip_prefix("minecraft:").unwrap_or(id)
}
fn full_id(id: &str) -> String {
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}
fn vanilla_species() -> impl Iterator<Item = (Species, String)> {
    species::ALL
        .iter()
        .map(|d| (d.species, species::vanilla_id(d.species).to_owned()))
}
fn field<'a>(value: &'a Value, name: &str) -> &'a Value {
    value.get(name).unwrap_or(value)
}
fn resource(value: &Value, name: &str) -> Result<String, String> {
    let raw = field(value, name)
        .as_str()
        .ok_or_else(|| format!("{name} must be an identifier"))?;
    horizon_core::identifier::Identifier::parse(raw)
        .map(|id| id.to_string())
        .map_err(|e| format!("{name}: {e}"))
}
fn number(value: &serde_json::value::RawValue) -> Result<&str, String> {
    let text = value.get().trim();
    // RawValue already guarantees JSON syntax; only its value kind is unknown.
    if matches!(text.as_bytes().first(), Some(b'-' | b'0'..=b'9')) {
        Ok(text)
    } else {
        Err("value must be numeric".into())
    }
}
/// Parse the original decimal straight to f32, without an f64 intermediate.
pub fn decode_distance(value: &serde_json::value::RawValue) -> Result<f32, String> {
    number(value)?
        .parse()
        .map_err(|error| format!("invalid distance: {error}"))
}
/// Registry lookup is part of the sound codec, before file installation.
pub fn decode_sound(value: &Value, known: impl Fn(&str) -> bool) -> Result<&'static str, String> {
    let id = resource(value, "sound")?;
    if !known(&id) {
        return Err(format!("unknown sound {id}"));
    }
    Ok(if id == EMPTY_SOUND {
        ""
    } else {
        crate::mobs::entity_sounds::intern_full_id(local_id(&id))
    })
}
pub fn decode_villager_type(value: &Value) -> Result<u8, String> {
    let id = resource(value, "villager_type")?;
    VILLAGER_TYPES
        .iter()
        .position(|name| id == full_id(name))
        .map(|i| i as u8)
        .ok_or_else(|| format!("unknown villager type {id}"))
}
pub fn decode_gift(value: &Value) -> Result<String, String> {
    resource(value, "loot_table")
}
/// Gson's exact decimal intValue: truncate, retain the low 32 bits, then
/// apply the codec's range. Work is bounded by input length, not its exponent.
pub fn decode_frequency(value: &serde_json::value::RawValue) -> Result<i32, String> {
    let text = number(value)?;
    let negative = text.starts_with('-');
    let unsigned = text.strip_prefix('-').unwrap_or(text);
    let (mantissa, exponent) = unsigned.split_once(['e', 'E']).unwrap_or((unsigned, "0"));
    let exponent_negative = exponent.starts_with('-');
    let exponent = exponent
        .trim_start_matches(['+', '-'])
        .bytes()
        .fold(0i128, |n, digit| {
            n.saturating_mul(10)
                .saturating_add(i128::from(digit - b'0'))
        });
    let exponent = if exponent_negative {
        exponent.saturating_neg()
    } else {
        exponent
    };
    let before = mantissa.find('.').unwrap_or(mantissa.len());
    let count = mantissa.bytes().filter(|byte| *byte != b'.').count() as i128;
    let point = (before as i128).saturating_add(exponent);
    let mut bits = 0u32;
    if point > 0 && point < count + 32 {
        for digit in mantissa
            .bytes()
            .filter(|byte| *byte != b'.')
            .take(point.min(count) as usize)
        {
            bits = bits.wrapping_mul(10).wrapping_add(u32::from(digit - b'0'));
        }
        for _ in 0..(point - count).max(0) {
            bits = bits.wrapping_mul(10);
        }
    }
    if negative {
        bits = bits.wrapping_neg();
    }
    let frequency = bits as i32;
    if (1..=15).contains(&frequency) {
        Ok(frequency)
    } else {
        Err("frequency must truncate and wrap to 1..15".into())
    }
}
pub fn decode_compostable(value: &Value) -> Result<bool, String> {
    field(value, "can_villager_compost")
        .as_bool()
        .ok_or_else(|| "can_villager_compost must be boolean".into())
}
/// Vanilla's table as map `id`'s values: the layer the packs' files merge over.
pub fn vanilla_layer(kind: Kind) -> BTreeMap<String, Value> {
    match kind {
        Kind::VillagerDistances => vanilla_species()
            .filter_map(|(s, id)| {
                Some((
                    id,
                    json!({ "acceptable_villager_distance": vanilla_villager_distance(s)? }),
                ))
            })
            .collect(),
        Kind::ParrotImitations => vanilla_species()
            .filter_map(|(s, id)| {
                let sound = parrot::imitation(s)?;
                let sound = if sound.is_empty() {
                    EMPTY_SOUND.to_owned()
                } else {
                    full_id(sound)
                };
                Some((id, json!({ "sound": sound })))
            })
            .collect(),
        Kind::VillagerTypes => crate::mobs::finalize::BIOME_VILLAGER_TYPES
            .iter()
            .map(|(biome, t)| {
                let kind = VILLAGER_TYPES[*t as usize];
                (full_id(biome), json!({ "villager_type": full_id(kind) }))
            })
            .collect(),
        Kind::RaidHeroGifts => GIFT_PROFESSIONS
            .iter()
            .map(|p| {
                let table = format!("minecraft:gameplay/hero_of_the_village/{p}_gift");
                (full_id(p), json!({ "loot_table": table }))
            })
            .collect(),
        Kind::VibrationFrequencies => crate::sculk::EVENTS
            .iter()
            .filter(|(_, f)| *f > 0)
            .map(|(event, f)| ((*event).to_owned(), *f))
            .chain(
                crate::sculk::RESONANCE
                    .iter()
                    .enumerate()
                    .map(|(i, event)| ((*event).to_owned(), i as i32 + 1)),
            )
            .map(|(event, f)| (event, json!({ "frequency": f })))
            .collect(),
        Kind::VillagerCompostables => COMPOSTABLES
            .iter()
            .map(|item| ((*item).to_owned(), json!({ "can_villager_compost": true })))
            .collect(),
    }
}

#[derive(Debug, Default)]
pub struct NeoForgeMobMaps {
    villager_distances: Option<HashMap<String, f32>>,
    parrot_imitations: Option<ParrotImitations>,
    villager_types: Option<HashMap<String, u8>>,
    hero_gifts: Option<HashMap<String, String>>,
    vibration_frequencies: Option<HashMap<String, i32>>,
    villager_compostables: Option<HashMap<String, bool>>,
}
#[derive(Debug)]
struct ParrotImitations {
    by_id: HashMap<String, &'static str>,
    sounds: Vec<&'static str>,
}
fn keyed<T>(values: BTreeMap<String, T>) -> HashMap<String, T> {
    values
        .into_iter()
        .map(|(id, value)| (local_id(&id).to_owned(), value))
        .collect()
}
static VANILLA_DISTANCES: LazyLock<HashMap<String, f32>> = LazyLock::new(|| {
    vanilla_species()
        .filter_map(|(species, id)| {
            Some((
                local_id(&id).to_owned(),
                vanilla_villager_distance(species)?,
            ))
        })
        .collect()
});
static VANILLA_IMITATIONS: LazyLock<HashMap<String, &'static str>> = LazyLock::new(|| {
    vanilla_species()
        .filter_map(|(species, id)| Some((local_id(&id).to_owned(), parrot::imitation(species)?)))
        .collect()
});
static VANILLA: LazyLock<Arc<NeoForgeMobMaps>> = LazyLock::new(Default::default);
impl NeoForgeMobMaps {
    /// Immutable no-file snapshot, never replaced by another world's load.
    pub fn vanilla() -> &'static Arc<Self> {
        &VANILLA
    }
    pub fn set_distances(&mut self, values: BTreeMap<String, f32>) {
        self.villager_distances = Some(keyed(values));
    }
    pub fn set_parrots(&mut self, values: BTreeMap<String, &'static str>) {
        // Membership is vanilla union map keys. A removed vanilla entry uses
        // the parrot's own ambient sound, including in the cached rare draw.
        let union: BTreeSet<String> = VANILLA_IMITATIONS
            .keys()
            .map(|id| full_id(id))
            .chain(values.keys().cloned())
            .collect();
        let by_id = keyed(values);
        let sounds = union
            .iter()
            .map(|id| {
                by_id
                    .get(local_id(id))
                    .copied()
                    .unwrap_or("entity.parrot.ambient")
            })
            .collect();
        self.parrot_imitations = Some(ParrotImitations { by_id, sounds });
    }
    pub fn set_villager_types(&mut self, values: BTreeMap<String, u8>) {
        self.villager_types = Some(keyed(values));
    }
    pub fn set_gifts(&mut self, values: BTreeMap<String, String>) {
        self.hero_gifts = Some(keyed(values));
    }
    pub fn set_frequencies(&mut self, values: BTreeMap<String, i32>) {
        self.vibration_frequencies = Some(keyed(values));
    }
    pub fn set_compostables(&mut self, values: BTreeMap<String, bool>) {
        self.villager_compostables = Some(keyed(values));
    }
    /// Exact entity type, never its template/species. Removed vanilla entries
    /// retain vanilla distance; absent custom types are not hostile.
    pub fn villager_distance(&self, id: &str) -> Option<f32> {
        let id = local_id(id);
        self.villager_distances
            .as_ref()
            .and_then(|table| table.get(id))
            .or_else(|| VANILLA_DISTANCES.get(id))
            .copied()
    }
    pub fn parrot_imitation(&self, id: &str) -> Option<&'static str> {
        let id = local_id(id);
        match &self.parrot_imitations {
            None => VANILLA_IMITATIONS.get(id).copied(),
            Some(map) => map.by_id.get(id).copied().or_else(|| {
                VANILLA_IMITATIONS
                    .contains_key(id)
                    .then_some("entity.parrot.ambient")
            }),
        }
    }
    /// None preserves the legacy no-file random selection and draw order.
    pub fn parrot_sounds(&self) -> Option<&[&'static str]> {
        self.parrot_imitations.as_ref().map(|p| p.sounds.as_slice())
    }
    pub fn villager_type(&self, biome: &str) -> u8 {
        match &self.villager_types {
            Some(types) => types.get(local_id(biome)).copied().unwrap_or(0),
            None => crate::mobs::finalize::villager_type(biome),
        }
    }
    pub fn hero_gift(&self, profession: &str) -> Option<String> {
        let gifts = self.hero_gifts.as_ref()?;
        Some(
            gifts.get(local_id(profession)).cloned().unwrap_or_else(|| {
                "minecraft:gameplay/hero_of_the_village/unemployed_gift".to_owned()
            }),
        )
    }
    pub fn vibration_frequency(&self, event: &str) -> i32 {
        match &self.vibration_frequencies {
            Some(frequencies) => frequencies.get(local_id(event)).copied().unwrap_or(0),
            None => crate::sculk::frequency(event),
        }
    }
    pub fn villager_compostable(&self, item: &str) -> bool {
        match &self.villager_compostables {
            Some(items) => items.get(local_id(item)).copied().unwrap_or(false),
            None => COMPOSTABLES
                .iter()
                .any(|known| local_id(known) == local_id(item)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numeric_codecs_use_java_truncation_and_unrestricted_float_distance() {
        let raw = |text: &str| serde_json::value::RawValue::from_string(text.to_owned()).unwrap();
        assert_eq!(decode_distance(&raw("-3")), Ok(-3.0));
        assert_eq!(decode_distance(&raw("1e500")), Ok(f32::INFINITY));
        assert_eq!(decode_distance(&raw("1e300")), Ok(f32::INFINITY));
        assert_eq!(
            decode_distance(&raw("-0.0")).unwrap().to_bits(),
            0x8000_0000
        );
        for (text, expected) in [
            ("4294967297", 1),
            ("4294967311", 15),
            ("15.999999999999999999999", 15),
            ("1.5", 1),
            ("-4294967295", 1),
            ("4294967297e0", 1),
        ] {
            assert_eq!(decode_frequency(&raw(text)), Ok(expected), "{text}");
        }
        for text in [
            "2147483648",
            "-1",
            "0",
            "16",
            "-0.5",
            "1e500",
            "1e-500",
            "\"7\"",
            "true",
        ] {
            assert!(decode_frequency(&raw(text)).is_err(), "{text}");
        }
    }
    #[test]
    fn exact_type_overrides_and_removed_vanilla_fallback_are_world_local() {
        let mut first = NeoForgeMobMaps::default();
        first.set_distances(BTreeMap::from([("test:wraith".into(), 21.0)]));
        let second = NeoForgeMobMaps::default();
        assert_eq!(first.villager_distance("test:wraith"), Some(21.0));
        assert_eq!(second.villager_distance("test:wraith"), None);
        assert_eq!(first.villager_distance("minecraft:zombie"), Some(8.0));
        assert_eq!(first.villager_distance("minecraft:pillager"), Some(15.0));
        assert_eq!(first.villager_distance("test:zombie_template"), None);
        assert_eq!(first.villager_distance("minecraft:cow"), None);
        assert!(second.parrot_sounds().is_none());
        first.set_parrots(BTreeMap::from([("test:wraith".into(), "test:howl")]));
        assert_eq!(first.parrot_imitation("test:wraith"), Some("test:howl"));
        assert_eq!(
            first.parrot_imitation("minecraft:zombie"),
            Some("entity.parrot.ambient")
        );
        assert_eq!(second.parrot_imitation("test:wraith"), None);
        assert_eq!(
            second.parrot_imitation("minecraft:zombie"),
            Some("entity.parrot.imitate.zombie")
        );
        assert!(first.parrot_sounds().unwrap().contains(&"test:howl"));
        assert!(
            first
                .parrot_sounds()
                .unwrap()
                .contains(&"entity.parrot.ambient")
        );
    }
}
