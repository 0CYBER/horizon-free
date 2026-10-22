//! SynchedEntityData values (ClientboundSetEntityDataPacket): an entry index,
//! the EntityDataSerializers id, and the serializer's value, ending with 0xFF.
//! Serializers Horizon cannot decode end the list early; the entries before
//! them are kept.
use crate::{
    Result,
    buf::{Codec, Reader, Writer},
    item::{Slot, Stack},
};
use horizon_world::nbt::Tag;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Byte(i8),
    Int(i32),
    Long(i64),
    Float(f32),
    String(String),
    Text(Tag),
    OptText(Option<Tag>),
    Item(Option<Stack>),
    Bool(bool),
    Floats(Vec<f32>),
    Pos([i32; 3]),
    OptPos(Option<[i32; 3]>),
    OptUuid(Option<u128>),
    /// Any VarInt-valued serializer (direction, block state, pose, variants ...).
    Id(i32),
    /// Several VarInts (VillagerData).
    Ids(Vec<i32>),
    /// A particle choice: the `minecraft:particle_type` id plus its data.
    /// Rendered nowhere (the client spawns no world particles yet); decoded
    /// so the rest of an entry list survives a particle slot.
    Particle(Particle),
}
/// The optional data of one particle type (26.3 registry order).
#[derive(Clone, Debug, PartialEq)]
pub struct Particle {
    pub id: i32,
    pub data: Option<ParticleData>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum ParticleData {
    /// Block state id (block, block_marker, falling_dust, dust_pillar,
    /// block_crumble).
    State(i32),
    /// Geometry-wide or ARGB color (effect, entity_effect, tinted_leaves,
    /// flash); flame family colors keep the top bits in one Int.
    Color(#[allow(dead_code)] i32),
    /// Dust family: color then scale.
    ColorScale(i32, f32),
    /// Dust color transition: from color, to color, scale.
    ColorScale2(i32, i32, f32),
    /// Effect family: color then power.
    ColorPower(i32, f32),
    /// A bare float (dragon_breath power, sculk_charge roll).
    Scalar(f32),
    /// Delay in ticks (shriek).
    Delay(i32),
    /// A position plus ticks (trail): 3 doubles, ARGB color, duration.
    Trail([f64; 3], i32, i32),
    /// An item stack (item).
    Item(Option<Stack>),
    /// One generic Int (geyser water blocks) or Int then Float
    /// (geyser_base, geyser_poof); plus the tick count of a vibration.
    Ints(Vec<f32>),
    /// Side effect: which registered particle carries which data shape.
    Vibration {
        #[allow(dead_code)]
        source: i32,
        #[allow(dead_code)]
        eye: Option<f32>,
        #[allow(dead_code)]
        ticks: i32,
    },
}
/// Serializer ids in EntityDataSerializers registration order.
pub mod serializer {
    pub const BYTE: i32 = 0;
    pub const INT: i32 = 1;
    pub const LONG: i32 = 2;
    pub const FLOAT: i32 = 3;
    pub const STRING: i32 = 4;
    pub const COMPONENT: i32 = 5;
    pub const OPTIONAL_COMPONENT: i32 = 6;
    pub const ITEM_STACK: i32 = 7;
    pub const BOOLEAN: i32 = 8;
    pub const ROTATIONS: i32 = 9;
    pub const BLOCK_POS: i32 = 10;
    pub const OPTIONAL_BLOCK_POS: i32 = 11;
    pub const DIRECTION: i32 = 12;
    pub const OPTIONAL_LIVING_ENTITY_REFERENCE: i32 = 13;
    pub const BLOCK_STATE: i32 = 14;
    pub const OPTIONAL_BLOCK_STATE: i32 = 15;
    pub const PARTICLE: i32 = 16;
    pub const PARTICLES: i32 = 17;
    pub const VILLAGER_DATA: i32 = 18;
    pub const OPTIONAL_UNSIGNED_INT: i32 = 19;
    pub const POSE: i32 = 20;
    pub const CAT_VARIANT: i32 = 21;
    pub const CAT_SOUND_VARIANT: i32 = 22;
    pub const COW_VARIANT: i32 = 23;
    pub const COW_SOUND_VARIANT: i32 = 24;
    pub const WOLF_VARIANT: i32 = 25;
    pub const WOLF_SOUND_VARIANT: i32 = 26;
    pub const FROG_VARIANT: i32 = 27;
    pub const PIG_VARIANT: i32 = 28;
    pub const PIG_SOUND_VARIANT: i32 = 29;
    pub const CHICKEN_VARIANT: i32 = 30;
    pub const CHICKEN_SOUND_VARIANT: i32 = 31;
    pub const ZOMBIE_NAUTILUS_VARIANT: i32 = 32;
    pub const OPTIONAL_GLOBAL_POS: i32 = 33;
    pub const PAINTING_VARIANT: i32 = 34;
    pub const SNIFFER_STATE: i32 = 35;
    pub const ARMADILLO_STATE: i32 = 36;
    pub const COPPER_GOLEM_STATE: i32 = 37;
    pub const WEATHERING_COPPER_STATE: i32 = 38;
    pub const VECTOR3: i32 = 39;
    pub const QUATERNION: i32 = 40;
    pub const RESOLVABLE_PROFILE: i32 = 41;
    pub const HUMANOID_ARM: i32 = 42;
    pub const DYE_COLOR: i32 = 43;
}
fn read_value(kind: i32, r: &mut Reader) -> Result<Option<Value>> {
    use serializer::*;
    Ok(Some(match kind {
        BYTE => Value::Byte(r.i8()?),
        INT => Value::Int(r.varint()?),
        LONG => Value::Long(r.varlong()?),
        FLOAT => Value::Float(r.f32()?),
        STRING => Value::String(r.string(32767)?),
        COMPONENT => Value::Text(r.nbt()?),
        OPTIONAL_COMPONENT => Value::OptText(if r.bool()? { Some(r.nbt()?) } else { None }),
        ITEM_STACK => Value::Item(Slot::read(r)?),
        BOOLEAN => Value::Bool(r.bool()?),
        ROTATIONS | VECTOR3 => Value::Floats(vec![r.f32()?, r.f32()?, r.f32()?]),
        QUATERNION => Value::Floats(vec![r.f32()?, r.f32()?, r.f32()?, r.f32()?]),
        BLOCK_POS => Value::Pos(crate::buf::Pos::read(r)?),
        OPTIONAL_BLOCK_POS => Value::OptPos(if r.bool()? {
            Some(crate::buf::Pos::read(r)?)
        } else {
            None
        }),
        OPTIONAL_LIVING_ENTITY_REFERENCE => {
            Value::OptUuid(if r.bool()? { Some(r.uuid()?) } else { None })
        }
        // VillagerData: type, profession and level.
        VILLAGER_DATA => Value::Ids(vec![r.varint()?, r.varint()?, r.varint()?]),
        // VarInt-encoded: direction, block states, optional ints, pose, the
        // holder-registry variants (cat to zombie nautilus, painting) and the
        // enum states (sniffer to weathering copper), arm, dye.
        DIRECTION
        | BLOCK_STATE
        | OPTIONAL_BLOCK_STATE
        | OPTIONAL_UNSIGNED_INT
        | POSE
        | CAT_VARIANT..=ZOMBIE_NAUTILUS_VARIANT
        | PAINTING_VARIANT..=WEATHERING_COPPER_STATE
        | HUMANOID_ARM
        | DYE_COLOR => Value::Id(r.varint()?),
        PARTICLE | PARTICLES => {
            let id = r.varint()?;
            // `minecraft:particle_type` registry order as of 26.3.
            let data = match id {
                1 | 2 | 36 | 121 | 125 => Some(ParticleData::State(r.varint()?)),
                28 | 46 | 52 => Some(ParticleData::Color(r.i32()?)),
                21 => {
                    let color = r.i32()?;
                    let scale = r.f32()?;
                    Some(ParticleData::ColorScale(color, scale))
                }
                22 => {
                    let from = r.i32()?;
                    let to = r.i32()?;
                    let scale = r.f32()?;
                    Some(ParticleData::ColorScale2(from, to, scale))
                }
                23 | 56 => {
                    let color = r.i32()?;
                    let power = r.f32()?;
                    Some(ParticleData::ColorPower(color, power))
                }
                15 | 48 => Some(ParticleData::Scalar(r.f32()?)),
                115 => Some(ParticleData::Delay(r.varint()?)),
                59 => {
                    let x = r.f64()?;
                    let y = r.f64()?;
                    let z = r.f64()?;
                    let color = r.i32()?;
                    let duration = r.varint()?;
                    Some(ParticleData::Trail([x, y, z], color, duration))
                }
                57 => Some(ParticleData::Item(Slot::read(r)?)),
                58 => {
                    let source = r.varint()?;
                    let mut eye = None;
                    if source == 0 {
                        let _ = crate::buf::Pos::read(r)?;
                    } else {
                        let _ = r.varint()?;
                        eye = Some(r.f32()?);
                    }
                    let ticks = r.varint()?;
                    Some(ParticleData::Vibration { source, eye, ticks })
                }
                7..=10 => {
                    let mut ints = vec![r.i32()? as f32];
                    if id == 8 || id == 9 {
                        ints.push(r.f32()?);
                    }
                    Some(ParticleData::Ints(ints))
                }
                // Every other registered particle carries no data.
                _ => None,
            };
            // Not into Ok(Some(..)): the match arm is inside the outer Option.
            Value::Particle(Particle { id, data })
        }
        _ => return Ok(None),
    }))
}
fn write_value(kind: i32, v: &Value, w: &mut Writer) {
    match v {
        Value::Byte(b) => w.i8(*b),
        Value::Int(i) | Value::Id(i) => w.varint(*i),
        Value::Ids(ids) => ids.iter().for_each(|i| w.varint(*i)),
        Value::Long(l) => w.varlong(*l),
        Value::Float(f) => w.f32(*f),
        Value::String(s) => w.string(s),
        Value::Text(t) => w.nbt(t),
        Value::OptText(t) => {
            w.bool(t.is_some());
            if let Some(t) = t {
                w.nbt(t);
            }
        }
        Value::Item(s) => Slot::write(s, w),
        Value::Bool(b) => w.bool(*b),
        Value::Floats(f) => f.iter().for_each(|f| w.f32(*f)),
        Value::Pos(p) => crate::buf::Pos::write(p, w),
        Value::OptPos(p) => {
            w.bool(p.is_some());
            if let Some(p) = p {
                crate::buf::Pos::write(p, w);
            }
        }
        Value::OptUuid(u) => {
            w.bool(u.is_some());
            if let Some(u) = u {
                w.uuid(*u);
            }
        }
        Value::Particle(p) => {
            w.varint(p.id);
            if let Some(data) = &p.data {
                match data {
                    ParticleData::State(s) => w.varint(*s),
                    ParticleData::Color(c) => w.i32(*c),
                    ParticleData::ColorScale(c, s) => {
                        w.i32(*c);
                        w.f32(*s);
                    }
                    ParticleData::ColorScale2(a, b, s) => {
                        w.i32(*a);
                        w.i32(*b);
                        w.f32(*s);
                    }
                    ParticleData::ColorPower(c, p) => {
                        w.i32(*c);
                        w.f32(*p);
                    }
                    ParticleData::Scalar(f) => w.f32(*f),
                    ParticleData::Delay(d) => w.varint(*d),
                    ParticleData::Trail(at, c, d) => {
                        at.iter().for_each(|v| w.f64(*v));
                        w.i32(*c);
                        w.varint(*d);
                    }
                    ParticleData::Item(s) => Slot::write(s, w),
                    ParticleData::Ints(ints) => {
                        let mut iter = ints.iter();
                        if let Some(v) = iter.next() {
                            w.i32(*v as i32);
                        }
                        for v in iter {
                            w.f32(*v);
                        }
                    }
                    ParticleData::Vibration { source, eye, ticks } => {
                        w.varint(*source);
                        let _ = source;
                        if let Some(eye) = eye {
                            w.f32(*eye);
                        }
                        w.varint(*ticks);
                    }
                }
            }
        }
    }
    let _ = kind;
}

/// (index, serializer id, value) entries.
pub struct EntityData;
impl Codec<Vec<(u8, i32, Value)>> for EntityData {
    fn read(r: &mut Reader) -> Result<Vec<(u8, i32, Value)>> {
        let mut out = Vec::new();
        loop {
            let index = r.u8()?;
            if index == 0xFF {
                return Ok(out);
            }
            let kind = r.varint()?;
            match read_value(kind, r)? {
                Some(v) => out.push((index, kind, v)),
                None => {
                    r.rest();
                    return Ok(out);
                }
            }
        }
    }
    fn write(v: &Vec<(u8, i32, Value)>, w: &mut Writer) {
        for (index, kind, value) in v {
            w.u8(*index);
            w.varint(*kind);
            write_value(*kind, value, w);
        }
        w.u8(0xFF);
    }
}

/// ItemEntity.DATA_ITEM (after the eight Entity data entries).
pub const ITEM_ENTITY_ITEM: u8 = 8;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_stops_at_unknown_serializers() {
        let entries = vec![
            (0u8, serializer::BYTE, Value::Byte(0x02)),
            (1, serializer::INT, Value::Int(300)),
            (2, serializer::OPTIONAL_COMPONENT, Value::OptText(None)),
            (6, serializer::POSE, Value::Id(5)),
            (
                8,
                serializer::ITEM_STACK,
                Value::Item(Some(Stack::new(3, 12))),
            ),
        ];
        let mut w = Writer::new();
        EntityData::write(&entries, &mut w);
        assert_eq!(*w.data.last().unwrap(), 0xFF);
        assert_eq!(
            EntityData::read(&mut Reader::new(&w.data)).unwrap(),
            entries
        );
        // Serializer 200 is unknown: the list stops there, earlier entries kept.
        let mut bytes = vec![0, 0, 7, 9, 200, 1, 2, 3];
        bytes.push(0xFF);
        let read = EntityData::read(&mut Reader::new(&bytes)).unwrap();
        assert_eq!(read, vec![(0, 0, Value::Byte(7))]);
        // A block particle decodes fully and round-trips.
        let entries = vec![(
            10u8,
            serializer::PARTICLE,
            Value::Particle(Particle {
                id: 1,
                data: Some(ParticleData::State(2)),
            }),
        )];
        let mut w = Writer::new();
        EntityData::write(&entries, &mut w);
        assert_eq!(
            EntityData::read(&mut Reader::new(&w.data)).unwrap(),
            entries
        );
    }
}
