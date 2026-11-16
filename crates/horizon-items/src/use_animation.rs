//! The use animation a `consumable` component names.

use horizon_core::use_animation::UseAnimation;

/// `ItemUseAnimation.CODEC`'s names; a consumable with none eats.
pub fn consumable_animation(v: &serde_json::Value) -> UseAnimation {
    animation_name(
        v.get("animation")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("eat")
            .encode_utf16(),
    )
}

/// Decode an animation name without allocating a UTF-8 copy of an NBT string.
pub fn animation_name(name: impl Iterator<Item = u16> + Clone) -> UseAnimation {
    [
        ("eat", UseAnimation::Eat),
        ("drink", UseAnimation::Drink),
        ("block", UseAnimation::Block),
        ("bow", UseAnimation::Bow),
        ("trident", UseAnimation::Trident),
        ("crossbow", UseAnimation::Crossbow),
        ("spyglass", UseAnimation::Spyglass),
        ("toot_horn", UseAnimation::TootHorn),
        ("brush", UseAnimation::Brush),
        ("bundle", UseAnimation::Bundle),
        ("spear", UseAnimation::Spear),
    ]
    .into_iter()
    .find_map(|(text, animation)| name.clone().eq(text.encode_utf16()).then_some(animation))
    .unwrap_or(UseAnimation::None)
}
