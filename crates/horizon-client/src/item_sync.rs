//! The data components a stack keeps when it crosses the wire, and how the
//! other side reads them back. A creative slot's packet carries a component
//! patch, and before this the encoder wrote damage alone, so everything else
//! (enchantments, a custom name, trim, repair cost) was gone once the server
//! echoed the inventory back.
use crate::item_data::{Levels, StackData};
use crate::items::ItemStack;
use horizon_protocol::item::{Component, Stack, component};
use horizon_world::nbt::Tag;
use std::sync::Arc;

/// `Slot.STREAM_CODEC`'s patch for the components Horizon models, with the
/// registries' ids for enchantments and trim. A name the registry does not
/// know is left out: id 0 is a different entry, not a missing one.
pub fn encode(
    stack: &ItemStack,
    enchantment: impl Fn(&str) -> Option<i32>,
    trim_material: impl Fn(&str) -> Option<i32>,
    trim_pattern: impl Fn(&str) -> Option<i32>,
) -> Vec<(i32, Component)> {
    let mut out = Vec::new();
    if stack.damage > 0 {
        out.push((component::DAMAGE, Component::Int(stack.damage as i32)));
    }
    let Some(d) = stack.data() else {
        return out;
    };
    let levels = |levels: &Levels| -> Option<Component> {
        let list: Vec<(i32, i32)> = levels
            .iter()
            .filter_map(|(name, level)| Some((enchantment(name)?, *level as i32)))
            .collect();
        (!list.is_empty()).then_some(Component::Enchantments(list))
    };
    if let Some(c) = levels(&d.enchantments) {
        out.push((component::ENCHANTMENTS, c));
    }
    if let Some(c) = levels(&d.stored_enchantments) {
        out.push((component::STORED_ENCHANTMENTS, c));
    }
    if d.repair_cost > 0 {
        out.push((component::REPAIR_COST, Component::Int(d.repair_cost as i32)));
    }
    if let Some(name) = &d.custom_name {
        // A plain string is a valid text component, and `decode_components`
        // reads it back as one.
        out.push((
            component::CUSTOM_NAME,
            Component::Text(Tag::String(name.as_str().into())),
        ));
    }
    if let Some((material, pattern)) = &d.trim
        && let (Some(material), Some(pattern)) = (trim_material(material), trim_pattern(pattern))
    {
        out.push((component::TRIM, Component::Trim { material, pattern }));
    }
    out
}

/// The same patch read back into `out`, the fields `item_data::decode_components`
/// fills from the equivalent `components` compound.
pub fn apply(
    stack: &Stack,
    out: &mut ItemStack,
    enchantment: impl Fn(i32) -> Option<Arc<str>>,
    trim_material: impl Fn(i32) -> Option<Arc<str>>,
    trim_pattern: impl Fn(i32) -> Option<Arc<str>>,
) {
    out.damage = stack.damage().max(0) as u32;
    let read_levels = |kind: i32, into: &mut Levels| {
        if let Some(Component::Enchantments(list)) = stack.component(kind) {
            for (id, level) in list {
                if let Some(name) = enchantment(*id) {
                    into.push((name, (*level).max(0) as u32));
                }
            }
        }
    };
    let data: &mut StackData = out.data_mut();
    read_levels(component::ENCHANTMENTS, &mut data.enchantments);
    read_levels(
        component::STORED_ENCHANTMENTS,
        &mut data.stored_enchantments,
    );
    if let Some(Component::Int(v)) = stack.component(component::REPAIR_COST) {
        data.repair_cost = (*v).max(0) as u32;
    }
    if let Some(Component::Text(t)) = stack.component(component::CUSTOM_NAME) {
        data.custom_name = crate::item_data::read_text(t);
    }
    if let Some(Component::Trim { material, pattern }) = stack.component(component::TRIM)
        && let (Some(material), Some(pattern)) = (trim_material(*material), trim_pattern(*pattern))
    {
        data.trim = Some((material, pattern));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(name: &str) -> Option<i32> {
        Some(match name {
            "minecraft:sharpness" => 3,
            "minecraft:gold" => 7,
            "minecraft:sentry" => 2,
            _ => return None,
        })
    }
    fn name_of(id: i32) -> Option<Arc<str>> {
        Some(Arc::from(match id {
            3 => "minecraft:sharpness",
            7 => "minecraft:gold",
            2 => "minecraft:sentry",
            _ => return None,
        }))
    }

    fn stack_with_data() -> ItemStack {
        let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
        stack.damage = 5;
        let d = stack.data_mut();
        d.enchantments.push((Arc::from("minecraft:sharpness"), 4));
        d.stored_enchantments
            .push((Arc::from("minecraft:sharpness"), 2));
        d.repair_cost = 9;
        d.custom_name = Some("Keeper".to_owned());
        d.trim = Some((Arc::from("minecraft:gold"), Arc::from("minecraft:sentry")));
        stack
    }

    /// Every modelled component survives the packet, so the server's echo does
    /// not hand back a bare item.
    #[test]
    fn a_stack_keeps_its_components_across_the_wire() {
        let stack = stack_with_data();
        let mut packet = Stack::new(1, 1);
        packet.added = encode(&stack, named, named, named);

        let mut back = ItemStack::new("minecraft:diamond_sword", 1);
        apply(&packet, &mut back, name_of, name_of, name_of);
        back.tidy();

        assert_eq!(back.damage, 5);
        let d = back.data().expect("components");
        assert_eq!(d.enchantments, vec![(Arc::from("minecraft:sharpness"), 4)]);
        assert_eq!(
            d.stored_enchantments,
            vec![(Arc::from("minecraft:sharpness"), 2)]
        );
        assert_eq!(d.repair_cost, 9);
        assert_eq!(d.custom_name.as_deref(), Some("Keeper"));
        assert_eq!(
            d.trim,
            Some((Arc::from("minecraft:gold"), Arc::from("minecraft:sentry")))
        );
    }

    /// A name the registry does not know is a missing id, not id 0, which would
    /// name the registry's first entry instead.
    #[test]
    fn an_unknown_enchantment_is_left_out_rather_than_sent_as_zero() {
        let mut stack = ItemStack::new("minecraft:book", 1);
        stack
            .data_mut()
            .enchantments
            .push((Arc::from("mod:unknown"), 1));
        assert!(encode(&stack, named, named, named).is_empty());
    }
}
