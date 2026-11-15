//! JSON slot displays use the same source-stack presentation as packet displays.
use super::*;
use horizon_core::nbt::{Compound, Tag};

pub(super) fn display_stacks(display: &Compound, items: &Items) -> Option<Vec<ItemStack>> {
    let json = horizon_core::json_nbt::tag_to_json(&Tag::Compound(display.clone())).ok()?;
    resolve(&json, items)
}
fn resolve(display: &Value, items: &Items) -> Option<Vec<ItemStack>> {
    let kind = display.get("type")?.as_str()?;
    Some(match kind {
        "minecraft:empty" => Vec::new(),
        "minecraft:any_fuel" => items
            .order
            .iter()
            .filter(|id| items.get(id).is_some_and(|i| i.fuel.is_some()))
            .map(|id| ItemStack::new(id, 1))
            .collect(),
        "minecraft:item" => vec![ItemStack::new(display.get("item")?.as_str()?, 1)],
        "minecraft:item_stack" => {
            vec![super::super::Template::parse(display.get("item")?)?.create()]
        }
        "minecraft:tag" => holder_stacks(display.get("tag")?, items)?,
        "minecraft:composite" => {
            let mut stacks = Vec::new();
            for child in display.get("contents")?.as_array()? {
                stacks.extend(resolve(child, items)?);
            }
            stacks
        }
        "minecraft:with_any_potion" => resolve(display.get("contents")?, items)?,
        "minecraft:only_with_component" => {
            let component = display.get("component")?.as_str()?;
            resolve(display.get("contents")?, items)?
                .into_iter()
                .filter(|stack| stack.carries_component(items.get(&stack.item), component))
                .collect()
        }
        "minecraft:dyed" => resolve(display.get("target")?, items)?,
        "minecraft:smithing_trim" => resolve(display.get("base")?, items)?,
        "minecraft:with_remainder" => resolve(display.get("input")?, items)?,
        _ => return None,
    })
}

fn holder_stacks(value: &Value, items: &Items) -> Option<Vec<ItemStack>> {
    match value {
        Value::String(id) => Some(match id.strip_prefix('#') {
            Some(tag) => items
                .item_tag_order
                .get(tag)
                .into_iter()
                .flatten()
                .map(|id| ItemStack::new(id, 1))
                .collect(),
            None => vec![ItemStack::new(id, 1)],
        }),
        Value::Array(values) => {
            let mut out = Vec::new();
            for value in values {
                out.extend(holder_stacks(value, items)?);
            }
            Some(out)
        }
        _ => None,
    }
}
