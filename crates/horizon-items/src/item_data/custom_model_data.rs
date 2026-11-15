//! `CustomModelData.CODEC`: ordered lists with empty defaults and ignored unknown fields.
use super::component_bool;
use crate::properties::component_float;
use horizon_core::color::rgb_color_from_tag;
use horizon_core::nbt::Tag;
use std::borrow::Cow;

#[derive(Clone, Copy)]
struct List<'a>(Option<&'a Tag>);
impl<'a> List<'a> {
    fn len(self) -> usize {
        match self.0 {
            None => 0,
            Some(Tag::List(v)) => v.len(),
            Some(Tag::ByteArray(v)) => v.len(),
            Some(Tag::IntArray(v)) => v.len(),
            Some(Tag::LongArray(v)) => v.len(),
            _ => unreachable!("validated list"),
        }
    }
    fn get(self, index: usize) -> Cow<'a, Tag> {
        match self.0 {
            Some(Tag::List(v)) => Cow::Borrowed(&v[index]),
            Some(Tag::ByteArray(v)) => Cow::Owned(Tag::Byte(v[index] as i8)),
            Some(Tag::IntArray(v)) => Cow::Owned(Tag::Int(v[index])),
            Some(Tag::LongArray(v)) => Cow::Owned(Tag::Long(v[index])),
            _ => unreachable!("nonempty validated list"),
        }
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Float,
    Flag,
    String,
    Color,
}
const FIELDS: [(&str, Kind); 4] = [
    ("floats", Kind::Float),
    ("flags", Kind::Flag),
    ("strings", Kind::String),
    ("colors", Kind::Color),
];
fn float_bits(value: &Tag) -> Option<u32> {
    let value = component_float(value)?;
    // Java Float.equals canonicalizes all NaNs but distinguishes signed zero.
    Some(if value.is_nan() {
        0x7fc00000
    } else {
        value.to_bits()
    })
}
impl Kind {
    fn valid(self, value: &Tag) -> bool {
        match self {
            Self::Float => component_float(value).is_some(),
            Self::Flag => component_bool(value).is_some(),
            Self::String => matches!(value, Tag::String(_)),
            Self::Color => rgb_color_from_tag(value).is_some(),
        }
    }
    fn equal(self, a: &Tag, b: &Tag) -> bool {
        match self {
            Self::Float => float_bits(a).is_some_and(|a| Some(a) == float_bits(b)),
            Self::Flag => component_bool(a).is_some_and(|a| Some(a) == component_bool(b)),
            Self::String => matches!((a, b), (Tag::String(a), Tag::String(b)) if a == b),
            Self::Color => rgb_color_from_tag(a).is_some_and(|a| Some(a) == rgb_color_from_tag(b)),
        }
    }
    fn canonical(self, value: &Tag) -> Option<Tag> {
        Some(match self {
            Self::Float => Tag::Float(float_bits(value)?),
            Self::Flag => Tag::Byte(i8::from(component_bool(value)?)),
            Self::String if matches!(value, Tag::String(_)) => value.clone(),
            Self::String => return None,
            Self::Color => Tag::Int(rgb_color_from_tag(value)?),
        })
    }
}
fn lists(value: &Tag) -> Option<[List<'_>; 4]> {
    let Tag::Compound(map) = value else {
        return None;
    };
    let mut lists = [List(None); 4];
    for (index, (name, _)) in FIELDS.iter().enumerate() {
        // Compare UTF-16 keys directly; neither lookup nor string matching allocates.
        let raw = map
            .iter()
            .find(|(key, _)| key.0.iter().copied().eq(name.encode_utf16()))
            .map(|(_, value)| value);
        if !matches!(
            raw,
            None | Some(Tag::List(_) | Tag::ByteArray(_) | Tag::IntArray(_) | Tag::LongArray(_))
        ) {
            return None;
        }
        lists[index] = List(raw);
    }
    Some(lists)
}

/// Validate the persistent codec without allocating or changing retained stack storage.
pub fn valid(value: &Tag) -> bool {
    lists(value).is_some_and(|lists| {
        FIELDS
            .iter()
            .zip(lists)
            .all(|((_, kind), list)| (0..list.len()).all(|index| kind.valid(&list.get(index))))
    })
}

/// Decode once at ingredient load into the canonical record/list representation.
pub fn decode(value: &Tag) -> Option<Tag> {
    let lists = lists(value)?;
    let fields = FIELDS
        .iter()
        .zip(lists)
        .map(|((name, kind), list)| {
            let values = (0..list.len())
                .map(|index| kind.canonical(&list.get(index)))
                .collect::<Option<Vec<_>>>()?;
            Some(((*name).into(), Tag::List(values)))
        })
        .collect::<Option<horizon_core::nbt::Compound>>()?;
    Some(Tag::Compound(fields))
}

/// Compare borrowed live records in Java list order, including exact UTF-16 strings.
pub fn equal(a: &Tag, b: &Tag) -> bool {
    let (Some(a), Some(b)) = (lists(a), lists(b)) else {
        return false;
    };
    FIELDS.iter().zip(a).zip(b).all(|(((_, kind), a), b)| {
        a.len() == b.len() && (0..a.len()).all(|index| kind.equal(&a.get(index), &b.get(index)))
    })
}
