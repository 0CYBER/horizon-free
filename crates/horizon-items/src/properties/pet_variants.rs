//! Persistent registry holders used by animal item components.
use super::resolvable::identifier;
use horizon_client_core::registries::Registries;
use horizon_core::nbt::Tag;

/// `DataComponents`: these codecs use registry references, never inline records.
pub fn registry(component: &str) -> Option<&'static str> {
    Some(match component {
        "minecraft:cat/sound_variant" => "minecraft:cat_sound_variant",
        "minecraft:cat/variant" => "minecraft:cat_variant",
        "minecraft:chicken/variant" => "minecraft:chicken_variant",
        "minecraft:wolf/sound_variant" => "minecraft:wolf_sound_variant",
        "minecraft:wolf/variant" => "minecraft:wolf_variant",
        _ => return None,
    })
}

/// `RegistryFixedCodec.decode`: resolve an `Identifier.CODEC` key in the caller's snapshot.
/// The normalized key is retained once at the component write/load boundary.
pub fn decode(registry: &str, value: &Tag, registries: &Registries) -> Result<Tag, String> {
    let entries = registries
        .by_registry
        .get(registry)
        .ok_or_else(|| format!("Can't access registry {registry}"))?;
    let Tag::String(raw) = value else {
        return Err("Not a string".into());
    };
    let key = identifier(value)
        .ok_or_else(|| format!("Not a valid resource location: {}", raw.to_string_lossy()))?
        .to_tag();
    let Tag::String(canonical) = &key else {
        unreachable!("identifier encodes a string")
    };
    if !entries
        .ids
        .iter()
        .any(|id| canonical.0.iter().copied().eq(id.encode_utf16()))
    {
        return Err(format!(
            "Failed to get element {} from registry {registry}",
            canonical.to_string_lossy()
        ));
    }
    Ok(key)
}
