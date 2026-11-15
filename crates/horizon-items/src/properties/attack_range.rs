//! The persistent `AttackRange.CODEC`; this does not resolve gameplay reach.
use super::resolvable::{Float, float};
use horizon_core::nbt::Tag;

const FIELDS: [(&str, f32, f32); 6] = [
    ("min_reach", 0.0, 64.0),
    ("max_reach", 3.0, 64.0),
    ("min_creative_reach", 0.0, 64.0),
    ("max_creative_reach", 5.0, 64.0),
    ("hitbox_margin", 0.3, 1.0),
    ("mob_factor", 1.0, 2.0),
];

/// `AttackRange.CODEC`: six optional boxed floats, with inclusive field bounds.
/// Both `ExtraCodecs.floatRange` and `Codec.floatRange` use Float.compareTo:
/// negative zero, NaNs and infinities fail these nonnegative finite ranges.
/// There is no minimum/maximum ordering constraint and no reference union.
/// Unknown map fields do not participate in the record value.
pub fn decode(value: &Tag) -> Option<[Float<'static>; 6]> {
    let Tag::Compound(map) = value else {
        return None;
    };
    let mut fields = [Float::Constant(0.0); 6];
    for (index, (name, default, maximum)) in FIELDS.into_iter().enumerate() {
        // Avoid Tag::get's thread-local UTF-16 buffer, including its first-use allocation.
        let value = map.iter().find_map(|(key, value)| {
            key.0
                .iter()
                .copied()
                .eq(name.encode_utf16())
                .then_some(value)
        });
        let decoded = match value {
            Some(Tag::End) | None => Float::Constant(default),
            Some(value) => float(value)?,
        };
        let Float::Constant(number) = decoded else {
            return None;
        };
        if number.is_sign_negative() || !(0.0..=maximum).contains(&number) {
            return None;
        }
        fields[index] = Float::Constant(number);
    }
    Some(fields)
}

/// Allocate the recipe's canonical payload once, never for live candidates.
pub fn normalized(value: &Tag) -> Option<Tag> {
    let fields = decode(value)?;
    Some(Tag::compound(
        FIELDS
            .into_iter()
            .zip(fields)
            .map(|((name, _, _), value)| (name, value.to_tag())),
    ))
}
