//! Pinned registered MobEffect particle factories and native option identity.
use crate::{DATA_VERSION, compression::invalid, synced_values::SyncedValue};
use serde_json::Value;
use std::{collections::BTreeMap, io, sync::Arc};

pub const AMBIENT_ALPHA: u8 = 38;

#[derive(Debug)]
pub struct ParticleData {
    pub kind: String,
    pub color: Option<u32>,
}
/// ColorParticleOption and SimpleParticleType use object equality. Cloning an
/// option retains that object; constructing a color option creates a new one.
#[derive(Clone, Debug)]
pub struct Particle(Arc<ParticleData>);
impl Particle {
    pub fn data(&self) -> &ParticleData {
        &self.0
    }
    pub fn same_reference(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl PartialEq for Particle {
    fn eq(&self, other: &Self) -> bool {
        self.same_reference(other)
    }
}
impl Eq for Particle {}

enum Factory {
    Color(u32),
    Constant(Particle),
}
pub struct Catalog {
    effects: BTreeMap<String, Factory>,
}
impl Catalog {
    /// The native exporter verifies inherited factories, captured arguments,
    /// option classes and identity. No recovered game assets are embedded here.
    pub fn from_json(value: &Value) -> io::Result<Self> {
        if value["data_version"].as_i64() != Some(i64::from(DATA_VERSION))
            || value["ambient_alpha"].as_u64() != Some(u64::from(AMBIENT_ALPHA))
        {
            return Err(invalid("effect particle catalog version mismatch"));
        }
        let mut effects = BTreeMap::new();
        let mut simple = BTreeMap::<String, Particle>::new();
        for item in value["effects"]
            .as_array()
            .ok_or_else(|| invalid("missing effects"))?
        {
            let id = identifier(&item["id"])?;
            let factory = match item["factory"].as_str() {
                Some("color") => Factory::Color(color(&item["color"])?),
                Some("constant") => {
                    let p = &item["particle"];
                    let kind = identifier(&p["type"])?;
                    if !p["color"].is_null() {
                        return Err(invalid("expected registered simple constant particle"));
                    }
                    let particle = simple
                        .entry(kind.clone())
                        .or_insert_with(|| Particle(Arc::new(ParticleData { kind, color: None })))
                        .clone();
                    Factory::Constant(particle)
                }
                _ => return Err(invalid("unknown registered particle factory")),
            };
            if effects.insert(id, factory).is_some() {
                return Err(invalid("duplicate effect particle definition"));
            }
        }
        Ok(Self { effects })
    }
    pub fn contains(&self, effect: &str) -> bool {
        self.effects.contains_key(effect)
    }
    /// Actual registered factories are pure: only the ambient flag changes a
    /// colored option. Constant options retain their registry object identity.
    pub fn create(&self, effect: &str, ambient: bool) -> Option<Particle> {
        Some(match self.effects.get(effect)? {
            Factory::Color(color) => Particle(Arc::new(ParticleData {
                kind: "minecraft:entity_effect".into(),
                color: Some(
                    (u32::from(if ambient { AMBIENT_ALPHA } else { 255 }) << 24)
                        | (color & 0xffffff),
                ),
            })),
            Factory::Constant(particle) => particle.clone(),
        })
    }
}
fn identifier(value: &Value) -> io::Result<String> {
    let name = value
        .as_str()
        .ok_or_else(|| invalid("expected particle identifier"))?;
    let canonical =
        crate::identifier::Identifier::parse(name).map_err(|e| invalid(format!("{e:?}")))?;
    if canonical.to_string() != name {
        return Err(invalid("noncanonical particle identifier"));
    }
    Ok(name.into())
}
fn color(value: &Value) -> io::Result<u32> {
    value
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| invalid("invalid particle color"))
}

/// The three synchronized value kinds used for living effect presentation.
/// Lists compare element object identity, including repeated fresh colors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EffectValue {
    Flags(u8),
    Ambient(bool),
    Particles(Arc<[Particle]>),
}
impl SyncedValue for EffectValue {
    fn native_equal(&self, other: &Self) -> bool {
        self == other
    }
}
