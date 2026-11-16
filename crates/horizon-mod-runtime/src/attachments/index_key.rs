//! Allocation-free UTF-8 lookup of lossless UTF-16 attachment keys.
use super::index::AttachmentKey;
use std::{
    borrow::Borrow,
    hash::{Hash, Hasher},
    sync::Arc,
};

#[derive(Clone)]
pub(super) enum Units<'a> {
    Saved(std::slice::Iter<'a, u16>),
    Named(std::str::EncodeUtf16<'a>),
}
impl Iterator for Units<'_> {
    type Item = u16;
    #[inline]
    fn next(&mut self) -> Option<u16> {
        match self {
            Self::Saved(it) => it.next().copied(),
            Self::Named(it) => it.next(),
        }
    }
    #[inline]
    fn count(self) -> usize {
        match self {
            Self::Saved(it) => it.len(),
            Self::Named(it) => it.count(),
        }
    }
}
impl Units<'_> {
    fn eq(self, other: Self) -> bool {
        match (self, other) {
            (Self::Saved(left), Self::Saved(right)) => left.as_slice() == right.as_slice(),
            (Self::Saved(left), Self::Named(right)) => left.copied().eq(right),
            (Self::Named(left), Self::Saved(right)) => left.eq(right.copied()),
            (Self::Named(left), Self::Named(right)) => left.eq(right),
        }
    }
}
pub(super) trait Lookup {
    fn units(&self) -> (Units<'_>, Units<'_>);
}
impl Lookup for AttachmentKey {
    fn units(&self) -> (Units<'_>, Units<'_>) {
        (Units::Saved(self.0.0.iter()), Units::Saved(self.1.0.iter()))
    }
}
pub(super) struct Named<'a>(pub &'a str, pub &'a str);
impl Lookup for Named<'_> {
    fn units(&self) -> (Units<'_>, Units<'_>) {
        (
            Units::Named(self.0.encode_utf16()),
            Units::Named(self.1.encode_utf16()),
        )
    }
}
impl PartialEq for dyn Lookup + '_ {
    fn eq(&self, other: &Self) -> bool {
        let (namespace, name) = self.units();
        let (other_namespace, other_name) = other.units();
        namespace.eq(other_namespace) && name.eq(other_name)
    }
}
impl Eq for dyn Lookup + '_ {}
impl Hash for dyn Lookup + '_ {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        let (namespace, name) = self.units();
        // Lengths delimit the lossless UTF-16 components. Pack four units per
        // hasher call rather than hashing an Option discriminant for each unit.
        // Owned keys and borrowed UTF-8 lookups must use exactly the same stream.
        for units in [namespace, name] {
            units.clone().count().hash(state);
            let mut packed = 0u64;
            let mut shift = 0;
            for unit in units {
                packed |= u64::from(unit) << shift;
                shift += 16;
                if shift == 64 {
                    state.write_u64(packed);
                    packed = 0;
                    shift = 0;
                }
            }
            if shift != 0 {
                state.write_u64(packed);
            }
        }
    }
}
#[derive(Clone)]
pub(super) struct BucketKey(pub Arc<AttachmentKey>);
impl<'a> Borrow<dyn Lookup + 'a> for BucketKey {
    fn borrow(&self) -> &(dyn Lookup + 'a) {
        self.0.as_ref()
    }
}
impl PartialEq for BucketKey {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl Eq for BucketKey {}
impl Hash for BucketKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.0.as_ref() as &dyn Lookup).hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lookup_equality_checks_both_components_and_lossless_units() {
        let key: AttachmentKey = ("namespace".into(), "name".into());
        let lookup = &key as &dyn Lookup;
        assert!(lookup == &Named("namespace", "name") as &dyn Lookup);
        assert!(lookup != &Named("namespace", "different") as &dyn Lookup);
        assert!(lookup != &Named("different", "name") as &dyn Lookup);
        let unpaired = (horizon_core::nbt::JavaString(vec![0xd800]), "name".into());
        assert!(&unpaired as &dyn Lookup != &Named("\u{fffd}", "name") as &dyn Lookup);
        let same_units = (horizon_core::nbt::JavaString(vec![0xd800]), "name".into());
        assert!(&unpaired as &dyn Lookup == &same_units as &dyn Lookup);
    }
}
