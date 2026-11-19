//! Banner pattern layers (`BannerPatternLayers`) and their NBT form.
use horizon_core::nbt::Tag;
use std::sync::Arc;

/// `BannerPattern`: a `banner_pattern` registry reference, or a direct holder
/// that carries its own `asset_id` and `translation_key`
/// (`BannerPattern.DIRECT_CODEC`, inline definitions allowed).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PatternRef {
    /// A registry reference: the pattern's id.
    Id(Arc<str>),
    /// A direct holder: the definition the layer carries itself.
    Inline {
        /// `asset_id`, the texture under `entity/banner` and `entity/shield`.
        asset: Arc<str>,
        /// `translation_key`, the tooltip name's prefix.
        translation_key: Arc<str>,
    },
}
impl PatternRef {
    /// The registry id, when this is a reference (`None` for a direct holder,
    /// which no registry holds).
    pub fn id(&self) -> Option<&str> {
        match self {
            Self::Id(id) => Some(id),
            Self::Inline { .. } => None,
        }
    }
}

/// `BannerPatternLayers.Layer`: a pattern and the dye name it is painted in.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BannerLayer {
    pub pattern: PatternRef,
    pub color: Arc<str>,
}
impl BannerLayer {
    /// A layer whose pattern is a registry reference.
    pub fn new(pattern: &str, color: &str) -> Self {
        Self {
            pattern: PatternRef::Id(Arc::from(pattern)),
            color: Arc::from(color),
        }
    }
}

/// The `banner_patterns` NBT list of layers.
pub fn layers_to_nbt(layers: &[BannerLayer]) -> Tag {
    Tag::List(
        layers
            .iter()
            .map(|l| {
                // `Holder`'s encode: a reference writes its id, a direct holder
                // its own definition.
                let pattern = match &l.pattern {
                    PatternRef::Id(id) => Tag::String((&**id).into()),
                    PatternRef::Inline {
                        asset,
                        translation_key,
                    } => Tag::compound([
                        ("asset_id", Tag::String((&**asset).into())),
                        ("translation_key", Tag::String((&**translation_key).into())),
                    ]),
                };
                Tag::compound([
                    ("pattern", pattern),
                    ("color", Tag::String((&*l.color).into())),
                ])
            })
            .collect(),
    )
}
pub fn layers_from_nbt(tag: Option<&Tag>) -> Vec<BannerLayer> {
    let Some(Tag::List(list)) = tag else {
        return Vec::new();
    };
    // The list codec's partial value keeps layers that decoded successfully;
    // `TagValueInput.read` returns that partial value on an error.
    let layer = |e: &Tag| -> Option<BannerLayer> {
        let text = |k: &str| match e.get(k) {
            Some(Tag::String(s)) => Some(s.to_string_lossy()),
            _ => None,
        };
        // `RegistryFileCodec`: an identifier is a registry reference; anything
        // else is decoded with `BannerPattern.DIRECT_CODEC` (`asset_id`, then
        // `translation_key`) and becomes a direct holder.
        let pattern = match e.get("pattern")? {
            // `Identifier.CODEC` rejects an invalid id, which fails the layer.
            Tag::String(id) => PatternRef::Id(Arc::from(
                horizon_client_core::identifier::normalize_id(&id.to_string_lossy())?.as_str(),
            )),
            Tag::Compound(c) => {
                let field = |k: &str| match c.get(&k.into()) {
                    Some(Tag::String(s)) => Some(s.to_string_lossy()),
                    _ => None,
                };
                // `Identifier.CODEC` rejects an invalid `asset_id`.
                let asset = horizon_client_core::identifier::normalize_id(&field("asset_id")?)?;
                PatternRef::Inline {
                    asset: Arc::from(asset.as_str()),
                    translation_key: Arc::from(field("translation_key")?.as_str()),
                }
            }
            _ => return None,
        };
        let color = text("color")?;
        if !crate::firework::DYE_FIREWORK_COLORS
            .iter()
            .any(|(name, _)| *name == color)
        {
            return None;
        }
        Some(BannerLayer {
            pattern,
            color: Arc::from(color.as_str()),
        })
    };
    list.iter().filter_map(layer).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(pattern: &str, color: &str) -> Tag {
        Tag::compound([
            ("pattern", Tag::String(pattern.into())),
            ("color", Tag::String(color.into())),
        ])
    }

    #[test]
    fn invalid_layers_are_dropped_from_saved_layers() {
        let mixed = Tag::List(vec![
            layer("brick", "red"),
            layer("brick", "not_a_color"),
            Tag::compound([("pattern", Tag::String("brick".into()))]),
            layer("globe", "blue"),
        ]);
        assert_eq!(
            layers_from_nbt(Some(&mixed)),
            vec![
                BannerLayer::new("minecraft:brick", "red"),
                BannerLayer::new("minecraft:globe", "blue")
            ]
        );
        let valid = Tag::List(vec![layer("brick", "red"), layer("globe", "blue")]);
        let layers = layers_from_nbt(Some(&valid));
        assert_eq!(layers.len(), 2, "both valid layers are kept");
        assert_eq!(layers[0], BannerLayer::new("minecraft:brick", "red"));
        // And the round trip of a valid list is unchanged.
        assert_eq!(layers_from_nbt(Some(&layers_to_nbt(&layers))), layers);
    }
}
