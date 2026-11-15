//! The attribute modifiers an item stack grants (`ItemStack.getAttributeModifiers`):
//! the stack's own `minecraft:attribute_modifiers` or its item's
//! (`ItemStack::default_attribute_modifiers`), then the mods' answer to
//! `item_attribute_modifier` (`ItemAttributeModifierEvent`). Every reader of a
//! stack's modifiers goes through here.
use horizon_items::item_data::ModifierRow;
use horizon_items::items::{ItemStack, Items};
use mod_api::{
    AttributeModifier, AttributeOperation, EquipmentSlotGroup, Identifier, ItemAttributeModifier,
};
use std::borrow::Cow;

/// The modifiers `stack` grants, with its item's from `items`.
pub fn attribute_modifiers<'a>(stack: &ItemStack, items: &'a Items) -> Cow<'a, [ModifierRow]> {
    modified(stack, stack.default_attribute_modifiers(items))
}

/// `attribute_modifiers` with the item's list given: `default`.
pub fn attribute_modifiers_or<'a>(
    stack: &ItemStack,
    default: &'a [ModifierRow],
) -> Cow<'a, [ModifierRow]> {
    modified(stack, stack.default_attribute_modifiers_or(default))
}

/// `rows` as the mods leave them. With no mod listening, or none changing
/// them, `rows` comes back as it was: borrowed, with nothing allocated for
/// a stack that has no list of its own.
fn modified<'a>(stack: &ItemStack, rows: Cow<'a, [ModifierRow]>) -> Cow<'a, [ModifierRow]> {
    if !mod_api::__private::listened(mod_api::HookId::ItemAttributeModifier) {
        return rows;
    }
    // A row the API can't name (an unknown operation or slot group, a bad
    // id) is kept as it is, ahead of the rest.
    let mut kept = Vec::new();
    let mut entries = Vec::with_capacity(rows.len());
    for row in rows.iter() {
        match entry(row) {
            Some(entry) => entries.push(entry),
            None => kept.push(row.clone()),
        }
    }
    let mut modifiers = mod_api::__private::game::item_attribute_modifiers(entries);
    crate::native_stack_hooks::attributes(stack, &mut modifiers);
    match mod_api::__private::game::changed_item_attribute_modifiers(modifiers) {
        Some(entries) => {
            kept.extend(entries.iter().map(row));
            Cow::Owned(kept)
        }
        None => rows,
    }
}

const SLOT_GROUPS: [(&str, EquipmentSlotGroup); 11] = [
    ("any", EquipmentSlotGroup::Any),
    ("mainhand", EquipmentSlotGroup::MainHand),
    ("offhand", EquipmentSlotGroup::OffHand),
    ("hand", EquipmentSlotGroup::Hand),
    ("feet", EquipmentSlotGroup::Feet),
    ("legs", EquipmentSlotGroup::Legs),
    ("chest", EquipmentSlotGroup::Chest),
    ("head", EquipmentSlotGroup::Head),
    ("armor", EquipmentSlotGroup::Armor),
    ("body", EquipmentSlotGroup::Body),
    ("saddle", EquipmentSlotGroup::Saddle),
];

fn entry(row: &ModifierRow) -> Option<ItemAttributeModifier> {
    let operation = match row.operation.as_str() {
        "add_value" => AttributeOperation::AddValue,
        "add_multiplied_base" => AttributeOperation::AddMultipliedBase,
        "add_multiplied_total" => AttributeOperation::AddMultipliedTotal,
        _ => return None,
    };
    let slot = SLOT_GROUPS.iter().find(|(name, _)| *name == row.slot)?.1;
    let modifier = AttributeModifier::new(Identifier::parse(&row.id).ok()?, row.amount, operation);
    Some(ItemAttributeModifier::new(
        Identifier::parse(&row.attribute).ok()?,
        modifier,
        slot,
    ))
}

fn row(entry: &ItemAttributeModifier) -> ModifierRow {
    let operation = match entry.modifier.operation {
        AttributeOperation::AddValue => "add_value",
        AttributeOperation::AddMultipliedBase => "add_multiplied_base",
        AttributeOperation::AddMultipliedTotal => "add_multiplied_total",
        _ => unreachable!("AttributeOperation: unhandled variant"),
    };
    let slot = match SLOT_GROUPS.iter().find(|(_, group)| *group == entry.slot) {
        Some((name, _)) => name,
        None => unreachable!("EquipmentSlotGroup: unhandled variant"),
    };
    ModifierRow::new(
        &entry.attribute.to_string(),
        &entry.modifier.id.to_string(),
        entry.modifier.amount,
        operation,
        slot,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every slot group and operation the API names reads back as the row it
    /// came from.
    #[test]
    fn rows_and_entries_name_the_same_modifier() {
        for (slot, _) in SLOT_GROUPS {
            for operation in ["add_value", "add_multiplied_base", "add_multiplied_total"] {
                let source = ModifierRow::new("minecraft:luck", "test:charm", 0.5, operation, slot);
                assert_eq!(row(&entry(&source).unwrap()), source);
            }
        }
        let unknown = ModifierRow::new("minecraft:luck", "test:charm", 0.5, "add_value", "pocket");
        assert!(entry(&unknown).is_none());
    }
}
