use super::*;

/// `RepairItemRecipe.canCombine`.
pub(super) fn pair<'a>(
    ctx: &Context,
    stacks: &[&'a ItemStack],
) -> Option<(&'a ItemStack, &'a ItemStack)> {
    let [first, second] = stacks else {
        return None;
    };
    (first.item == second.item
        && first.count == 1
        && second.count == 1
        && combine_repairable(ctx, first)
        && combine_repairable(ctx, second))
    .then_some((*first, *second))
}

/// `ItemStack.isCombineRepairable`: vanilla's answer, whether the stack has durability
/// (`max_damage` and `damage`), then the parts of its item (`ItemPart::is_combine_repairable`).
/// A `true` for a stack with no maximum damage is refused (`refuse_no_durability`), since the
/// result's must be positive. An item with no such part costs one table lookup.
fn combine_repairable(ctx: &Context, stack: &ItemStack) -> bool {
    let info = ctx.facts(stack).map(|f| &f.info);
    let vanilla = stack.carries_component(info, "minecraft:max_damage")
        && stack.carries_component(info, "minecraft:damage");
    let mut repairable = vanilla;
    crate::native_client_hooks::is_combine_repairable(ctx.context, stack, &mut repairable);
    if repairable && !vanilla && stack.max_damage(info) <= 0 {
        refuse_no_durability(&stack.item);
        return vanilla;
    }
    repairable
}

/// Says once per item that its `is_combine_repairable` parts answered `true` for a stack with
/// no durability, which the crafting grid can't repair.
fn refuse_no_durability(item: &str) {
    static REFUSED: std::sync::Mutex<std::collections::BTreeSet<String>> =
        std::sync::Mutex::new(std::collections::BTreeSet::new());
    if REFUSED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(item.to_owned())
    {
        log::warn!(
            "Horizon: E_COMBINE_REPAIR_NO_DURABILITY: an is_combine_repairable part of {item} answered true for a stack with no maximum damage; the crafting grid keeps vanilla's false"
        );
    }
}

/// `RepairItemRecipe.assemble`: integer division truncates the five percent bonus.
pub(super) fn assemble(ctx: &Context, stacks: &[&ItemStack]) -> Option<ItemStack> {
    let (first, second) = pair(ctx, stacks)?;
    let info = |s: &ItemStack| ctx.facts(s).map(|f| &f.info);
    let max = |s: &ItemStack| i64::from(s.max_damage(info(s)));
    let durability = max(first).max(max(second));
    // `ItemStack.getDamageValue` clamps the damage to `[0, max_damage]`: a patched component may exceed it.
    let damage = |s: &ItemStack| i64::from(s.effective_damage(info(s))).clamp(0, max(s).max(0));
    let remaining =
        (max(first) - damage(first)) + (max(second) - damage(second)) + durability * 5 / 100;
    let mut out = ItemStack::new(&first.item, 1);
    out.data_mut()
        .set_other("minecraft:max_damage", Some(Tag::Int(durability as i32)));
    out.damage = (durability - remaining).max(0) as u32;
    // `setDamageValue(0)` still stores `minecraft:damage`: without it the result stops being damageable.
    if out.damage == 0 {
        out.data_mut().explicit_zero_damage = true;
    }
    // Only the curses of both survive, at the higher level.
    let (a, b) = (
        first.crafting_enchantments(),
        second.crafting_enchantments(),
    );
    let mut curses: Vec<(Arc<str>, u32)> = Vec::new();
    for (id, level) in a.iter().chain(b.iter()) {
        if !ctx.recipes.curses.contains(&**id) {
            continue;
        }
        match curses.iter_mut().find(|(c, _)| c == id) {
            Some((_, l)) => *l = (*l).max(*level),
            None => curses.push((id.clone(), *level)),
        }
    }
    out.set_crafting_enchantments(curses);
    Some(out)
}

#[cfg(test)]
#[path = "repair_item_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "combine_repair_part_tests.rs"]
mod part_tests;
