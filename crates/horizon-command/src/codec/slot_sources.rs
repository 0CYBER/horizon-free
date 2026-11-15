//! SlotSources' holder-aware codec; all command consumers share its admission.
use super::*;
fn tag(t: &Tag, reg: &Registries) -> Result<(), String> {
    let name = as_string(t).ok_or_else(|| "Not a string".to_owned())?;
    let id = name
        .strip_prefix('#')
        .ok_or_else(|| "Not a tag id".to_owned())?;
    let id = parse_identifier(id).ok_or_else(|| format!("Not a valid resource location: {id}"))?;
    if reg.tags("minecraft:slot_source").contains(&id) {
        Ok(())
    } else {
        Err(format!(
            "Unknown tag #{id} in registry minecraft:slot_source"
        ))
    }
}
fn holder(t: &Tag, reg: &Registries) -> Result<(), String> {
    if let Some(name) = as_string(t)
        && !name.starts_with('#')
    {
        let id = parse_identifier(&name)
            .ok_or_else(|| format!("Not a valid resource location: {name}"))?;
        return if reg.ids("minecraft:slot_source").contains(&id) {
            Ok(())
        } else {
            Err(format!(
                "Can't find element '{id}' in registry 'minecraft:slot_source'"
            ))
        };
    }
    decode(t, reg)
}
/// `RegistryCodecs.holderSet` (HolderSetCodec, forceList false): a `#tag`, or
/// `ExtraCodecs.compactListCodec` of holders, a single one or a list.
fn holder_set(t: &Tag, reg: &Registries) -> Result<(), String> {
    if as_string(t).is_some_and(|s| s.starts_with('#')) {
        return tag(t, reg);
    }
    if let Tag::List(values) = t {
        for v in values {
            holder(v, reg)?;
        }
        return Ok(());
    }
    holder(t, reg)
}
/// `ExtraCodecs.POSITIVE_INT`: `Codec.INT` takes any numeric tag's
/// `intValue`, then `intRangeWithMessage(1, MAX_VALUE, ...)`.
fn positive_int(t: &Tag) -> Result<(), String> {
    let value = match *t {
        Tag::Byte(v) => i32::from(v),
        Tag::Short(v) => i32::from(v),
        Tag::Int(v) => v,
        Tag::Long(v) => v as i32,
        // Java's (int) cast saturates, and NaN becomes 0, as `as` does.
        Tag::Float(bits) => f32::from_bits(bits) as i32,
        Tag::Double(bits) => f64::from_bits(bits) as i32,
        _ => return Err("Not a number".into()),
    };
    if value < 1 {
        return Err(format!("Value must be positive: {value}"));
    }
    Ok(())
}
fn typed(t: &Tag, reg: &Registries) -> Result<(), String> {
    dispatch(t, reg, "minecraft:slot_source_type", &|id, map, t| {
        match id {
            "minecraft:contents" => {
                require(map, t, &["component"])?;
                let value = get(map, "component").expect("required component");
                let name = as_string(value).ok_or_else(|| "Not a string".to_owned())?;
                let component = parse_identifier(&name)
                    .ok_or_else(|| format!("Not a valid resource location: {name}"))?;
                let known = reg.ids("minecraft:data_component_type");
                if !known.is_empty() && !known.contains(&component) {
                    return Err(format!(
                        "Unknown registry key in ResourceKey[minecraft:root / minecraft:data_component_type]: {component}"
                    ));
                }
                if !matches!(
                    component.as_str(),
                    "minecraft:container"
                        | "minecraft:bundle_contents"
                        | "minecraft:charged_projectiles"
                ) {
                    return Err("No items in component".into());
                }
            }
            "minecraft:slot_range" => {
                require(map, t, &["slots"])?;
                let name = as_string(get(map, "slots").unwrap())
                    .ok_or_else(|| "Not a string".to_owned())?;
                if !slot_ranges().iter().any(|(known, _)| known == &name) {
                    return Err(format!("Unknown element name:{name}"));
                }
                if let Some(source) = get(map, "source") {
                    let name = as_string(source).ok_or_else(|| "Not a string".to_owned())?;
                    if !matches!(
                        name.as_str(),
                        "container"
                            | "this"
                            | "attacker"
                            | "direct_attacker"
                            | "attacking_player"
                            | "target_entity"
                            | "interacting_entity"
                            | "block_entity"
                    ) {
                        return Err(format!("Unknown element id: {name}"));
                    }
                }
            }
            "minecraft:group" => {
                require(map, t, &["terms"])?;
                return holder_set(get(map, "terms").unwrap(), reg);
            }
            "minecraft:limit_slots" => {
                require(map, t, &["limit"])?;
                positive_int(get(map, "limit").unwrap())?;
            }
            "minecraft:filtered" => {
                require(map, t, &["item_filter"])?;
                if !matches!(get(map, "item_filter"), Some(Tag::Compound(_))) {
                    return Err("Not an item predicate map".into());
                }
            }
            _ => {}
        }
        if matches!(
            id,
            "minecraft:contents" | "minecraft:limit_slots" | "minecraft:filtered"
        ) {
            require(map, t, &["slot_source"])?;
            holder(get(map, "slot_source").unwrap(), reg)?;
        }
        Ok(())
    })
}
pub(super) fn decode(t: &Tag, reg: &Registries) -> Result<(), String> {
    // SlotSources.CODEC is `RegistryCodecs.holder`: a quoted id is a
    // registry reference, as an unquoted one is (`resource_or_id`).
    if let Some(name) = as_string(t) {
        return if name.starts_with('#') {
            tag(t, reg)
        } else {
            holder(t, reg)
        };
    }
    if let Tag::List(values) = t {
        for value in values {
            holder(value, reg)?;
        }
        return Ok(());
    }
    element_or_list(t, &|value| typed(value, reg))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registered_holders_and_quoted_tag_sets_share_inline_admission() {
        let mut reg = Registries::default();
        let entry = reg
            .by_registry
            .entry("minecraft:slot_source".into())
            .or_default();
        entry.ids = vec!["complete:one".into(), "complete:empty".into()];
        entry.tags = vec!["complete:selection".into()];
        for input in [
            r##""#complete:selection""##,
            r#"["complete:one","complete:empty"]"#,
            r#"{type:"limit_slots",limit:1,slot_source:"complete:one"}"#,
            r#"{type:"limit_slots",limit:1L,slot_source:"complete:one"}"#,
            r##"{type:"group",terms:"#complete:selection"}"##,
            r#"{type:"slot_range",source:"this",slots:"weapon.mainhand"}"#,
            // A quoted single holder, at the top and as a group's terms.
            r#""complete:one""#,
            r#"{type:"group",terms:"complete:one"}"#,
            // Codec.INT reads any numeric tag's intValue.
            r#"{type:"limit_slots",limit:1b,slot_source:"complete:one"}"#,
            r#"{type:"limit_slots",limit:2.5d,slot_source:"complete:one"}"#,
        ] {
            let (tag, _) = horizon_world::nbt::parse_snbt(input).unwrap();
            assert!(decode(&tag, &reg).is_ok(), "{input}");
        }
        for input in [
            r#"["complete:missing"]"#,
            r##""#complete:missing""##,
            r#"{type:"slot_range",source:"not_a_context",slots:"container.*"}"#,
            r#"{type:"group",terms:"complete:missing"}"#,
            r#""complete:missing""#,
        ] {
            let (tag, _) = horizon_world::nbt::parse_snbt(input).unwrap();
            assert!(decode(&tag, &reg).is_err(), "{input}");
        }
        // NbtOps.getNumberValue's and ExtraCodecs.POSITIVE_INT's errors (26.3 bytecode).
        for (input, error) in [
            (
                r#"{type:"limit_slots",limit:"x",slot_source:"complete:one"}"#,
                "Not a number",
            ),
            (
                r#"{type:"limit_slots",limit:0,slot_source:"complete:one"}"#,
                "Value must be positive: 0",
            ),
            (
                r#"{type:"limit_slots",limit:-3,slot_source:"complete:one"}"#,
                "Value must be positive: -3",
            ),
            (
                r#"{type:"limit_slots",limit:0.5d,slot_source:"complete:one"}"#,
                "Value must be positive: 0",
            ),
        ] {
            // The top level wraps them in element_or_list's either error; only
            // the inner message is checked against the bytecode.
            let (tag, _) = horizon_world::nbt::parse_snbt(input).unwrap();
            let actual = decode(&tag, &reg).unwrap_err();
            assert!(actual.contains(error), "{input}: {actual}");
        }
    }
}
