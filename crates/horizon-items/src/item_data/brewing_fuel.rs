//! Persistent `BrewingFuel.CODEC`, independent of gameplay fuel resolution.
use crate::properties::resolvable::{self, Float, Int};
use horizon_core::nbt::Tag;

fn read(tag: &Tag) -> Option<(Int<'_>, Float<'_>)> {
    let Tag::Compound(fields) = tag else {
        return None;
    };
    let field = |name: &str| {
        fields
            .iter()
            .find(|(key, _)| key.0.iter().copied().eq(name.encode_utf16()))
            .map(|(_, value)| value)
    };
    Some((
        resolvable::int(field("uses")?)?,
        resolvable::float(field("speed_multiplier")?)?,
    ))
}

/// `BrewingFuel.CODEC`: both fields are required, unconstrained, and unknown fields are ignored.
/// Allocates only when preparing the persistent decoded value, never for candidate matching.
pub fn decode(tag: &Tag) -> Result<Tag, String> {
    let (uses, speed) = read(tag)
        .ok_or("Expected brewing fuel uses and speed_multiplier numbers or resource keys")?;
    Ok(Tag::compound([
        ("uses", uses.to_tag()),
        ("speed_multiplier", speed.to_tag()),
    ]))
}

/// Borrow the complete decoded value; two malformed candidates never compare equal.
pub(super) fn equal(a: Option<&Tag>, b: Option<&Tag>) -> bool {
    match (a.and_then(read), b.and_then(read)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}
