//! Version-pinned registered item identities and the component projection used
//! by duration/durability queries. Unrepresented components remain outside this
//! catalog; importing defaults does not implement their gameplay or codecs.
use crate::{DATA_VERSION, compression::invalid, item_durability::DurabilityStack};
use serde_json::Value;
use std::{collections::BTreeMap, io};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemId(u32);
impl ItemId {
    pub fn raw(self) -> u32 {
        self.0
    }
}
#[derive(Clone, Copy, Debug)]
enum Duration {
    Components,
    Constant(i32),
    Instrument,
}
#[derive(Clone, Copy, Debug)]
pub struct UseComponents {
    pub max_stack_size: Option<i32>,
    pub max_damage: Option<i32>,
    pub damage: Option<i32>,
    pub unbreakable: bool,
    pub consume_seconds: Option<f32>,
    pub block_delay_seconds: Option<f32>,
    pub kinetic_weapon: bool,
    pub instrument_seconds: Option<f32>,
}
#[derive(Debug)]
pub struct ItemDefinition {
    id: ItemId,
    name: String,
    duration: Duration,
    crossbow_release: bool,
    pub defaults: UseComponents,
}
impl ItemDefinition {
    pub fn id(&self) -> ItemId {
        self.id
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn stack(&self, count: i32) -> ItemUseStack<'_> {
        ItemUseStack {
            definition: self,
            count,
            components: self.defaults,
        }
    }
}
#[derive(Clone, Debug)]
pub struct ItemUseStack<'a> {
    definition: &'a ItemDefinition,
    pub count: i32,
    pub components: UseComponents,
}
impl ItemUseStack<'_> {
    pub fn definition(&self) -> &ItemDefinition {
        self.definition
    }
    pub fn is_empty(&self) -> bool {
        self.count <= 0 || self.definition.name == "minecraft:air"
    }
    pub fn max_stack_size(&self) -> i32 {
        if self.is_empty() {
            1
        } else {
            self.components.max_stack_size.unwrap_or(1)
        }
    }
    pub fn durability(&self) -> DurabilityStack {
        DurabilityStack {
            count: self.count,
            air: self.definition.name == "minecraft:air",
            max_damage: self.components.max_damage,
            damage: self.components.damage,
            unbreakable: self.components.unbreakable,
        }
    }
    pub fn is_stackable(&self) -> bool {
        let d = self.durability();
        self.max_stack_size() > 1 && (!d.is_damageable() || !d.is_damaged())
    }
    pub fn use_duration(&self) -> i32 {
        if self.is_empty() {
            return 0;
        }
        match self.definition.duration {
            Duration::Constant(value) => value,
            Duration::Instrument => self
                .components
                .instrument_seconds
                .map_or(0, |v| (v * 20.0).floor() as i32),
            Duration::Components => {
                if let Some(seconds) = self.components.consume_seconds {
                    (seconds * 20.0) as i32
                } else if self.components.block_delay_seconds.is_some()
                    || self.components.kinetic_weapon
                {
                    72000
                } else {
                    0
                }
            }
        }
    }
    pub fn use_on_release(&self) -> bool {
        !self.is_empty() && self.definition.crossbow_release
    }
}
#[derive(Debug)]
pub struct ItemUseCatalog {
    items: Vec<ItemDefinition>,
    names: BTreeMap<String, ItemId>,
}
fn string<'a>(v: &'a Value, key: &str) -> io::Result<&'a str> {
    v[key]
        .as_str()
        .ok_or_else(|| invalid(format!("missing item export string {key}")))
}
fn optional_int(v: &Value, key: &str) -> io::Result<Option<i32>> {
    v.get(key)
        .map(|v| {
            v.as_i64()
                .and_then(|n| i32::try_from(n).ok())
                .ok_or_else(|| invalid(format!("invalid item integer {key}")))
        })
        .transpose()
}
fn number(v: &Value, key: &str, default: f32) -> io::Result<f32> {
    v.get(key).map_or(Ok(default), |v| {
        v.as_f64()
            .map(|n| n as f32)
            .ok_or_else(|| invalid(format!("invalid item number {key}")))
    })
}
impl ItemUseCatalog {
    pub fn from_json(value: &Value) -> io::Result<Self> {
        if value["schema"].as_i64() != Some(1)
            || value["DataVersion"].as_i64() != Some(i64::from(DATA_VERSION))
        {
            return Err(invalid("item catalog version mismatch"));
        }
        let rows = value["items"]
            .as_array()
            .ok_or_else(|| invalid("missing item catalog entries"))?;
        let instruments = value["instruments"]
            .as_object()
            .ok_or_else(|| invalid("missing item instruments"))?;
        let mut items = Vec::with_capacity(rows.len());
        let mut names = BTreeMap::new();
        for (index, row) in rows.iter().enumerate() {
            let raw = u32::try_from(index).map_err(|_| invalid("item catalog too large"))?;
            if row["id"].as_u64() != Some(u64::from(raw)) {
                return Err(invalid("non-contiguous item catalog ID"));
            }
            let name = string(row, "name")?;
            if crate::worldgen::tags::identifier(name).ok().as_deref() != Some(name) {
                return Err(invalid("noncanonical item identifier"));
            }
            let id = ItemId(raw);
            if names.insert(name.to_owned(), id).is_some() {
                return Err(invalid("duplicate item identifier"));
            }
            let duration = match string(row, "duration_impl")? {
                "Item" => Duration::Components,
                "BowItem" | "CrossbowItem" | "TridentItem" => Duration::Constant(72000),
                "BundleItem" | "BrushItem" => Duration::Constant(200),
                "SpyglassItem" => Duration::Constant(1200),
                "EnderEyeItem" => Duration::Constant(0),
                "InstrumentItem" => Duration::Instrument,
                _ => return Err(invalid("unsupported item duration implementation")),
            };
            let crossbow_release = match string(row, "release_impl")? {
                "Item" => false,
                "CrossbowItem" => true,
                _ => return Err(invalid("unsupported release implementation")),
            };
            let c = row
                .get("components")
                .filter(|c| c.is_object())
                .ok_or_else(|| invalid("missing item component defaults"))?;
            let instrument_seconds = c
                .get("minecraft:instrument")
                .map(|v| {
                    let name = v
                        .as_str()
                        .ok_or_else(|| invalid("expected exported instrument reference"))?;
                    let bits = instruments
                        .get(name)
                        .and_then(Value::as_str)
                        .ok_or_else(|| invalid("unknown item instrument"))?;
                    u32::from_str_radix(bits, 16)
                        .map(f32::from_bits)
                        .map_err(|_| invalid("invalid instrument duration bits"))
                })
                .transpose()?;
            let defaults = UseComponents {
                max_stack_size: optional_int(c, "minecraft:max_stack_size")?,
                max_damage: optional_int(c, "minecraft:max_damage")?,
                damage: optional_int(c, "minecraft:damage")?,
                unbreakable: c.get("minecraft:unbreakable").is_some(),
                consume_seconds: c
                    .get("minecraft:consumable")
                    .map(|v| number(v, "consume_seconds", 1.6))
                    .transpose()?,
                block_delay_seconds: c
                    .get("minecraft:blocks_attacks")
                    .map(|v| number(v, "block_delay_seconds", 0.0))
                    .transpose()?,
                kinetic_weapon: c.get("minecraft:kinetic_weapon").is_some(),
                instrument_seconds,
            };
            items.push(ItemDefinition {
                id,
                name: name.to_owned(),
                duration,
                crossbow_release,
                defaults,
            });
        }
        if items.first().is_none_or(|i| i.name != "minecraft:air") {
            return Err(invalid("item registry must start with air"));
        }
        Ok(Self { items, names })
    }
    pub fn get(&self, id: ItemId) -> Option<&ItemDefinition> {
        self.items.get(id.0 as usize)
    }
    pub fn by_name(&self, name: &str) -> Option<&ItemDefinition> {
        self.get(*self.names.get(name)?)
    }
    pub fn items(&self) -> &[ItemDefinition] {
        &self.items
    }
}
