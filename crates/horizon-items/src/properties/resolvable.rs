//! Persistent scalar codecs for component values backed by context number providers.
use super::{component_float, component_int};
use horizon_core::nbt::{JavaString, Tag};

/// A validated resource key, borrowing its UTF-16 namespace and path.
#[derive(Clone, Copy, Debug)]
pub struct Identifier<'a> {
    namespace: Option<&'a [u16]>,
    path: &'a [u16],
}

impl PartialEq for Identifier<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
            && match (self.namespace, other.namespace) {
                (None, None) => true,
                (Some(a), Some(b)) => a == b,
                (None, Some(namespace)) | (Some(namespace), None) => {
                    namespace.iter().copied().eq("minecraft".encode_utf16())
                }
            }
    }
}
impl Eq for Identifier<'_> {}

impl Identifier<'_> {
    /// Canonical UTF-16 length without allocating the encoded identifier.
    pub fn utf16_len(&self) -> usize {
        self.namespace.map_or("minecraft".len(), <[u16]>::len) + 1 + self.path.len()
    }

    /// Encode the canonical key once while loading, not while matching a stack.
    pub fn to_tag(self) -> Tag {
        let mut value = Vec::with_capacity(self.utf16_len());
        if let Some(namespace) = self.namespace {
            value.extend_from_slice(namespace);
        } else {
            value.extend("minecraft".encode_utf16());
        }
        value.push(u16::from(b':'));
        value.extend_from_slice(self.path);
        Tag::String(JavaString(value))
    }

    /// Compare a snapshot's UTF-8 key without allocating a canonical tag or UTF-16 buffer.
    pub fn matches_str(&self, value: &str) -> bool {
        let (namespace, path) = value.split_once(':').unwrap_or(("minecraft", value));
        let namespace = if namespace.is_empty() {
            "minecraft"
        } else {
            namespace
        };
        self.path.iter().copied().eq(path.encode_utf16())
            && match self.namespace {
                Some(units) => units.iter().copied().eq(namespace.encode_utf16()),
                None => namespace == "minecraft",
            }
    }
}

fn namespace_char(value: u16) -> bool {
    u8::try_from(value).is_ok_and(|value| {
        value.is_ascii_lowercase() || value.is_ascii_digit() || matches!(value, b'_' | b'-' | b'.')
    })
}

/// `Identifier.CODEC`, without a resource lookup or a lossy string conversion.
pub fn identifier(value: &Tag) -> Option<Identifier<'_>> {
    let Tag::String(value) = value else {
        return None;
    };
    identifier_units(&value.0)
}

/// `Identifier.CODEC` for a borrowed UTF-16 slice, including a tag-key suffix.
pub fn identifier_units(value: &[u16]) -> Option<Identifier<'_>> {
    let (namespace, path) = match value.iter().position(|&value| value == u16::from(b':')) {
        Some(colon) => (&value[..colon], &value[colon + 1..]),
        None => (&[][..], value),
    };
    if namespace == [u16::from(b'.'); 2]
        || !namespace.iter().copied().all(namespace_char)
        || !path
            .iter()
            .copied()
            .all(|value| value == u16::from(b'/') || namespace_char(value))
    {
        return None;
    }
    Some(Identifier {
        namespace: (!namespace.is_empty()).then_some(namespace),
        path,
    })
}

/// `ResolvableInt`: an integer constant or an unresolved context-provider key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Int<'a> {
    Constant(i32),
    Reference(Identifier<'a>),
}
impl Int<'_> {
    /// Canonical persistent representation, allocated only at load time.
    pub fn to_tag(self) -> Tag {
        match self {
            Self::Constant(value) => Tag::Int(value),
            Self::Reference(key) => key.to_tag(),
        }
    }
}

/// `ResolvableInt.CODEC`: Java boxed-number conversion, then a resource key.
pub fn int(value: &Tag) -> Option<Int<'_>> {
    if let Some(value) = component_int(value) {
        Some(Int::Constant(value))
    } else {
        identifier(value).map(Int::Reference)
    }
}

/// `ResolvableFloat`: boxed-float equality canonicalizes NaNs but preserves signed zero.
#[derive(Clone, Copy, Debug)]
pub enum Float<'a> {
    Constant(f32),
    Reference(Identifier<'a>),
}
impl PartialEq for Float<'_> {
    fn eq(&self, other: &Self) -> bool {
        match (*self, *other) {
            (Self::Constant(a), Self::Constant(b)) => {
                a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
            }
            (Self::Reference(a), Self::Reference(b)) => a == b,
            _ => false,
        }
    }
}
impl Eq for Float<'_> {}
impl Float<'_> {
    /// Canonical persistent representation, allocated only at load time.
    pub fn to_tag(self) -> Tag {
        match self {
            Self::Constant(value) => Tag::Float(value.to_bits()),
            Self::Reference(key) => key.to_tag(),
        }
    }
}

/// `ResolvableFloat.CODEC`: unrestricted numeric f32 or an unresolved resource key.
pub fn float(value: &Tag) -> Option<Float<'_>> {
    if let Some(value) = component_float(value) {
        Some(Float::Constant(value))
    } else {
        identifier(value).map(Float::Reference)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifier_matches_snapshot_keys_without_allocating() {
        let key = Tag::String("stone".into());
        let key = identifier(&key).unwrap();
        assert!(key.matches_str("minecraft:stone"));
        assert!(key.matches_str(":stone"));
        assert!(!key.matches_str("mod:stone"));
        assert!(!key.matches_str("minecraft:stone_extra"));
        for (raw, snapshot) in [
            ("mod:path", "mod:path"),
            ("minecraft:", "minecraft:"),
            (":", "minecraft:"),
        ] {
            let raw = Tag::String(raw.into());
            assert!(identifier(&raw).unwrap().matches_str(snapshot));
        }
        let raw = Tag::String("stone".into());
        let id = identifier(&raw).unwrap();
        assert!(!id.matches_str("minecraft:stone:extra"));
        assert!(!id.matches_str("minecraft:stöne"));
    }

    #[test]
    fn identifier_utf16_lengths_match_canonical_keys() {
        for (raw, canonical, length) in [
            ("stone", "minecraft:stone", 15),
            ("provider", "minecraft:provider", 18),
            (":", "minecraft:", 10),
            ("", "minecraft:", 10),
            ("a:b", "a:b", 3),
        ] {
            let raw = Tag::String(raw.into());
            let key = identifier(&raw).unwrap();
            assert_eq!(key.utf16_len(), length);
            assert_eq!(key.to_tag(), Tag::String(canonical.into()));
        }
    }

    #[test]
    fn number_and_provider_codecs_preserve_java_equality_and_identifier_rules() {
        assert_eq!(
            int(&Tag::Long(4_294_967_297)).unwrap().to_tag(),
            Tag::Int(1)
        );
        assert_eq!(
            int(&Tag::Double(f64::NAN.to_bits())).unwrap().to_tag(),
            Tag::Int(0)
        );
        assert_eq!(
            int(&Tag::String("provider/path".into())).unwrap().to_tag(),
            Tag::String("minecraft:provider/path".into())
        );
        for (a, b) in [
            ("provider", "minecraft:provider"),
            (":", "minecraft:"),
            ("", "minecraft:"),
        ] {
            let a = Tag::String(a.into());
            let b = Tag::String(b.into());
            assert_eq!(int(&a).unwrap(), int(&b).unwrap());
            assert_eq!(float(&a).unwrap(), float(&b).unwrap());
        }
        let prefixed = [
            u16::from(b'#'),
            u16::from(b'a'),
            u16::from(b':'),
            u16::from(b'b'),
        ];
        let value = Tag::String("a:b".into());
        assert_eq!(identifier_units(&prefixed[1..]), identifier(&value));
        assert!(identifier_units(&[0xd800]).is_none());
        let zero = Tag::Float(0.0_f32.to_bits());
        let negative_zero = Tag::Double((-0.0_f64).to_bits());
        assert_ne!(float(&zero).unwrap(), float(&negative_zero).unwrap());
        let nan = Tag::Float(f32::NAN.to_bits());
        let other_nan = Tag::Double(0x7ff8_0000_0000_0001);
        assert_eq!(float(&nan).unwrap(), float(&other_nan).unwrap());
        assert_eq!(
            float(&Tag::Double(f64::INFINITY.to_bits()))
                .unwrap()
                .to_tag(),
            Tag::Float(f32::INFINITY.to_bits())
        );
        for value in [
            Tag::String("UPPER:path".into()),
            Tag::String("..:path".into()),
            Tag::String("x:bad:path".into()),
            Tag::String(JavaString(vec![0xd800])),
            Tag::Compound(Default::default()),
            Tag::List(Vec::new()),
        ] {
            assert!(int(&value).is_none(), "{value:?}");
            assert!(float(&value).is_none(), "{value:?}");
        }
    }
}
