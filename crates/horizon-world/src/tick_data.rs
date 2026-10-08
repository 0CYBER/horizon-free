//! Save-format bridge for scheduled block/fluid ticks. Registry identity is
//! supplied by the caller; this module does not implement either type's behavior.
use crate::{compression::invalid, nbt::Tag};
use horizon_core::{
    position::BlockPos,
    ticks::{SavedTick, TickPriority, TickTypeId},
};
use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TickKind {
    Block,
    Fluid,
}
impl TickKind {
    pub const fn field(self) -> &'static str {
        match self {
            Self::Block => "block_ticks",
            Self::Fluid => "fluid_ticks",
        }
    }
}

pub fn read_list(
    tag: &Tag,
    mut resolve: impl FnMut(&str) -> Option<TickTypeId>,
) -> io::Result<Vec<SavedTick>> {
    let Tag::List(entries) = tag else {
        return Err(invalid("saved ticks are not a list"));
    };
    entries
        .iter()
        .map(|entry| {
            let Some(Tag::String(name)) = entry.get("i") else {
                return Err(invalid("saved tick is missing type name"));
            };
            let name = name.to_string().map_err(|e| invalid(e.to_string()))?;
            let kind = resolve(&name).ok_or_else(|| invalid("unknown saved tick type"))?;
            let int = |key| {
                // NbtOps/Codec.INT use Number.intValue, including narrowing
                // longs and Java's saturating float conversion.
                match entry.get(key) {
                    Some(Tag::Byte(v)) => Ok(i32::from(*v)),
                    Some(Tag::Short(v)) => Ok(i32::from(*v)),
                    Some(Tag::Int(v)) => Ok(*v),
                    Some(Tag::Long(v)) => Ok(*v as i32),
                    Some(Tag::Float(v)) => Ok(f32::from_bits(*v) as i32),
                    Some(Tag::Double(v)) => Ok(f64::from_bits(*v) as i32),
                    _ => Err(invalid(format!("saved tick is missing number {key}"))),
                }
            };
            Ok(SavedTick {
                kind,
                pos: BlockPos::new(int("x")?, int("y")?, int("z")?),
                delay: int("t")?,
                priority: TickPriority::by_value(int("p")?),
            })
        })
        .collect()
}

pub fn write_list(
    ticks: &[SavedTick],
    mut name: impl FnMut(TickTypeId) -> Option<String>,
) -> io::Result<Tag> {
    let entries = ticks
        .iter()
        .map(|tick| {
            let name = name(tick.kind).ok_or_else(|| invalid("unknown saved tick type ID"))?;
            Ok(Tag::compound([
                ("i", Tag::String(name.as_str().into())),
                ("x", Tag::Int(tick.pos.x)),
                ("y", Tag::Int(tick.pos.y)),
                ("z", Tag::Int(tick.pos.z)),
                ("t", Tag::Int(tick.delay)),
                ("p", Tag::Int(tick.priority as i32)),
            ]))
        })
        .collect::<io::Result<Vec<_>>>()?;
    Ok(Tag::List(entries))
}
