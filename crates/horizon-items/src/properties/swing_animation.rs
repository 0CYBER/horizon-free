//! Ordinary persistent `SwingAnimation.CODEC`, shared by animation components.
use super::component_int;
use horizon_core::nbt::Tag;

/// `SwingAnimationType`: ordinary NBT uses names, not compressed enum ordinals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SwingAnimationType {
    None,
    Whack,
    Stab,
}

impl SwingAnimationType {
    fn decode(value: &Tag) -> Option<Self> {
        let Tag::String(value) = value else {
            return None;
        };
        [
            ("none", Self::None),
            ("whack", Self::Whack),
            ("stab", Self::Stab),
        ]
        .into_iter()
        .find_map(|(name, kind)| {
            value
                .0
                .iter()
                .copied()
                .eq(name.encode_utf16())
                .then_some(kind)
        })
    }

    fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Whack => "whack",
            Self::Stab => "stab",
        }
    }
}

/// `SwingAnimation` record equality compares the enum and converted integer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SwingAnimation {
    pub kind: SwingAnimationType,
    pub duration: i32,
}

impl SwingAnimation {
    /// `SwingAnimation.CODEC`: absent fields default; malformed present fields fail.
    /// Unknown fields are ignored, as with the vanilla record codec.
    pub fn decode(value: &Tag) -> Option<Self> {
        let Tag::Compound(_) = value else {
            return None;
        };
        let kind = value
            .get("type")
            .filter(|v| !matches!(v, Tag::End))
            .map_or(Some(SwingAnimationType::Whack), SwingAnimationType::decode)?;
        let duration = value
            .get("duration")
            .filter(|v| !matches!(v, Tag::End))
            .map_or(Some(6), component_int)?;
        (duration >= 0).then_some(Self { kind, duration })
    }

    /// Normalize once when compiling a persistent component patch.
    pub fn to_tag(self) -> Tag {
        Tag::Compound(
            [
                ("type".into(), Tag::String(self.kind.name().into())),
                ("duration".into(), Tag::Int(self.duration)),
            ]
            .into_iter()
            .collect(),
        )
    }
}
