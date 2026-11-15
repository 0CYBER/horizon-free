//! `TooltipDisplay.CODEC`: numeric booleans and a reference-identity component set.
use super::resolvable::{Identifier, identifier, identifier_units};
use horizon_core::nbt::Tag;

/// Borrowed decoded fields; registry lookup happens only while compiling a payload.
#[derive(Clone, Copy, Debug)]
pub struct TooltipDisplay<'a> {
    pub hide_tooltip: bool,
    hidden: &'a [Tag],
}

impl<'a> TooltipDisplay<'a> {
    /// `NbtOps.getBooleanValue` uses doubleValue != 0, not byte/int conversion.
    pub fn parse(value: &'a Tag) -> Option<Self> {
        let Tag::Compound(_) = value else {
            return None;
        };
        let hide_tooltip = match value.get("hide_tooltip") {
            None | Some(Tag::End) => false,
            Some(Tag::Byte(v)) => *v != 0,
            Some(Tag::Short(v)) => *v != 0,
            Some(Tag::Int(v)) => *v != 0,
            Some(Tag::Long(v)) => *v != 0,
            Some(Tag::Float(v)) => f32::from_bits(*v) != 0.0,
            Some(Tag::Double(v)) => f64::from_bits(*v) != 0.0,
            _ => return None,
        };
        // NbtOps accepts all CollectionTags. Numeric arrays can only decode as an
        // empty component list, since each nonempty element must be a string key.
        let hidden = match value.get("hidden_components") {
            None | Some(Tag::End) => &[][..],
            Some(Tag::List(values)) => values.as_slice(),
            Some(Tag::ByteArray(values)) if values.is_empty() => &[],
            Some(Tag::IntArray(values)) if values.is_empty() => &[],
            Some(Tag::LongArray(values)) if values.is_empty() => &[],
            _ => return None,
        };
        if !hidden.iter().all(|value| identifier(value).is_some()) {
            return None;
        }
        Some(Self {
            hide_tooltip,
            hidden,
        })
    }

    pub fn components(self) -> impl Iterator<Item = Identifier<'a>> {
        self.hidden
            .iter()
            .map(|value| identifier(value).expect("decoded component key"))
    }

    /// Java Set equality ignores insertion order and repeated references.
    pub fn equal(self, other: Self) -> bool {
        self.hide_tooltip == other.hide_tooltip
            && self
                .components()
                .all(|id| other.components().any(|other| id == other))
            && other
                .components()
                .all(|id| self.components().any(|other| id == other))
    }

    /// `Raid.getBannerComponentPatch`: the implicit ominous-banner value.
    pub fn is_ominous(self) -> bool {
        const BANNER: &[u16] = &[
            b'b' as u16,
            b'a' as u16,
            b'n' as u16,
            b'n' as u16,
            b'e' as u16,
            b'r' as u16,
            b'_' as u16,
            b'p' as u16,
            b'a' as u16,
            b't' as u16,
            b't' as u16,
            b'e' as u16,
            b'r' as u16,
            b'n' as u16,
            b's' as u16,
        ];
        let banner = identifier_units(BANNER).expect("literal component key");
        !self.hide_tooltip && !self.hidden.is_empty() && self.components().all(|id| id == banner)
    }
}
