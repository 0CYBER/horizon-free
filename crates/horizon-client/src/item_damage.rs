//! The shared ItemStack.canBeHurtBy component rule.
use crate::items::ItemStack;

/// Borrow defaults and the preserved NBT patch instead of materializing a render stack.
pub fn stack_can_be_hurt_by(
    default: Option<&serde_json::Value>,
    catalog: &horizon_world::damage_source::Catalog,
    stack: &ItemStack,
    name: &str,
) -> Option<bool> {
    use horizon_core::nbt::Tag;
    let damage = catalog.id(name)?;
    // Identifiers are ASCII. Compare UTF-16 patch strings directly, including
    // the implicit minecraft namespace, without allocating a conversion.
    let matches = |units: &[u16]| {
        let tag = units.first() == Some(&(b'#' as u16));
        let units = if tag { &units[1..] } else { units };
        let units = units.strip_prefix(&[b':' as u16]).unwrap_or(units);
        let equal = |candidate: &str| {
            if units.contains(&(b':' as u16)) {
                units.iter().copied().eq(candidate.encode_utf16())
            } else {
                candidate
                    .strip_prefix("minecraft:")
                    .is_some_and(|path| units.iter().copied().eq(path.encode_utf16()))
            }
        };
        if tag {
            catalog
                .tags()
                .any(|id| equal(id) && catalog.in_tag(damage, id))
        } else {
            equal(name)
        }
    };
    if let Some(data) = stack.data() {
        if data
            .other
            .iter()
            .any(|(key, _)| key == "!minecraft:damage_resistant")
        {
            return Some(true);
        }
        if let Some((_, component)) = data
            .other
            .iter()
            .find(|(key, _)| key == "minecraft:damage_resistant")
        {
            let types = match component {
                Tag::Compound(fields) => fields
                    .iter()
                    .find(|(key, _)| key.0.iter().copied().eq("types".encode_utf16()))
                    .map(|(_, value)| value),
                _ => None,
            };
            let resistant = match types {
                Some(Tag::String(id)) => matches(&id.0),
                Some(Tag::List(ids)) => ids
                    .iter()
                    .any(|id| matches!(id, Tag::String(id) if matches(&id.0))),
                _ => false,
            };
            return Some(!resistant);
        }
    }
    let types = default.and_then(|component| component.get("types"));
    let matches_json = |id: &str| {
        let (tag, id) = id.strip_prefix('#').map_or((false, id), |id| (true, id));
        let id = id.strip_prefix(':').unwrap_or(id);
        let equal = |candidate: &str| {
            candidate == id
                || (!id.contains(':') && candidate.strip_prefix("minecraft:") == Some(id))
        };
        if tag {
            if id.contains(':') {
                catalog.in_tag(damage, id)
            } else {
                catalog
                    .tags()
                    .any(|id| equal(id) && catalog.in_tag(damage, id))
            }
        } else {
            equal(name)
        }
    };
    Some(!types.is_some_and(|types| {
        types.as_str().is_some_and(matches_json)
            || types.as_array().is_some_and(|ids| {
                ids.iter()
                    .filter_map(serde_json::Value::as_str)
                    .any(matches_json)
            })
    }))
}

#[cfg(test)]
#[path = "item_damage_tests.rs"]
mod tests;
