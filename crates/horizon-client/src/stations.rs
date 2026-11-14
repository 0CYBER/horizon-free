//! Result rules of the workstation menus, ported from vanilla's
//! `EnchantmentMenu`, `AnvilMenu`, `GrindstoneMenu`, `SmithingMenu` (with
//! `SmithingTransformRecipe` / `SmithingTrimRecipe`) and `StonecutterMenu`.
//! Rendering and clicks live in the game; this module is pure item logic.
use crate::{
    crafting::Ingredient,
    enchant::Enchantments,
    item_data::{self, Levels},
    items::{ItemStack, Items},
};
use horizon_core::random::{LegacyRandom, RandomSource};
use horizon_world::loot::PackOverlay;
use horizon_world::native_load_context::LoopContext;
mod models;
pub use models::{AnvilResult, EnchantmentRow};
use serde_json::Value;
use std::{collections::HashSet, fs, path::Path, sync::Arc};

/// Everything the workstation menus read, loaded once with the resources.
pub struct StationData {
    /// The same registry `Items` carries (`Items::enchantments`).
    pub enchantments: std::sync::Arc<Enchantments>,
    pub recipes: StationRecipes,
    pub brewing: crate::brewing::Brewing,
    pub trades: crate::merchant::Trades,
    pub professions: Arc<crate::mobs::villager::ResolvedProfessions>,
    pub banners: crate::banner::Banners,
    /// Every recipe's learnability and display count (the server recipe book).
    pub recipe_index: crate::recipe_book::RecipeIndex,
}
impl StationData {
    pub fn load(
        root: &Path,
        items: &Items,
        overlay: &PackOverlay,
        defaults: &horizon_world::item_defaults::ItemDefaults,
        context: &horizon_world::native_load_context::LoadContext,
    ) -> Self {
        let sources = crate::crafting::ResolvedSources::load_or_empty(root, overlay, context);
        let recipes = crate::crafting::Recipes::from_resolved(root, items, overlay, &sources)
            .unwrap_or_else(|error| {
                log::error!("recipes: could not load station recipe index: {error}");
                crate::crafting::Recipes::empty()
            });
        Self::from_sources(root, items, overlay, defaults, (&sources, &recipes))
    }
    /// The tables of one load whose recipes are `sources`.
    pub fn from_sources(
        root: &Path,
        items: &Items,
        overlay: &PackOverlay,
        defaults: &horizon_world::item_defaults::ItemDefaults,
        (sources, recipes): (&crate::crafting::ResolvedSources, &crate::crafting::Recipes),
    ) -> Self {
        Self {
            enchantments: std::sync::Arc::clone(&items.enchantments),
            recipes: StationRecipes::from_sources(root, items, sources),
            brewing: {
                let mut brewing = crate::brewing::Brewing::load(root, items, overlay);
                let added = |key| sources.property_additions.get(key).into_iter().flatten();
                brewing.reagents.extend(added("brewing_reagent").cloned());
                brewing.inputs.extend(added("brewing_input").cloned());
                brewing
            },
            trades: crate::merchant::Trades::load(root, overlay),
            professions: Arc::new(crate::mobs::villager::ResolvedProfessions::vanilla()),
            banners: crate::banner::Banners::load(root, defaults),
            recipe_index: crate::recipe_book::RecipeIndex::from_recipes(recipes),
        }
    }
}

// ---------------------------------------------------------------- enchanting

/// `EnchantingTableBlock.BOOKSHELF_OFFSETS`: the 5x5 ring two blocks out, two high, in
/// `BlockPos.betweenClosed` order (x fastest, then y, then z), which a float power sum can see.
pub fn bookshelf_offsets() -> Vec<[i32; 3]> {
    let mut v = Vec::new();
    for z in -2i32..=2 {
        for y in 0..=1 {
            for x in -2i32..=2 {
                if x.abs() == 2 || z.abs() == 2 {
                    v.push([x, y, z]);
                }
            }
        }
    }
    v
}
/// The three offers of `EnchantmentMenu.slotsChanged`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EnchantOffers {
    pub costs: [i32; 3],
    /// Shown clue: enchantment and level (`enchantClue` / `levelClue`).
    pub clues: [Option<(Arc<str>, u32)>; 3],
}
/// `EnchantmentMenu.getEnchantmentList`.
pub fn enchantment_list(
    e: &Enchantments,
    items: &Items,
    seed: i32,
    slot: usize,
    cost: i32,
    stack: &ItemStack,
) -> Levels {
    let mut random = LegacyRandom::new(i64::from(seed.wrapping_add(slot as i32)));
    let pool = e.tag("minecraft:in_enchanting_table").to_vec();
    let mut list = e.select(items, &mut random, stack, cost, &pool);
    if &*stack.item == "minecraft:book" && list.len() > 1 {
        let i = random.next_int_bound(list.len() as i32).unwrap_or(0) as usize;
        list.remove(i);
    }
    list
}

/// `ItemStack.isEnchantable`: an enchantable component, no enchantments
/// yet, and the enchantments component present (all items have it by
/// default; a removal patch leaves none).
pub fn is_enchantable(e: &Enchantments, stack: &ItemStack) -> bool {
    e.enchantable_of(stack) > 0
        && stack.enchantments().is_empty()
        && stack
            .data()
            .is_none_or(|d| d.other("!minecraft:enchantments").is_none())
}
/// `EnchantmentMenu.slotsChanged` for an enchantable item.
pub fn enchant_offers(
    e: &Enchantments,
    items: &Items,
    seed: i32,
    bookcases: i32,
    stack: Option<&ItemStack>,
) -> EnchantOffers {
    enchant_offers_leveled(e, items, seed, bookcases, stack, |_, cost| cost)
}
/// [`enchant_offers`], with `level_set` given each row and its cost as
/// `getEnchantmentCost` drew it, before a cost below the row is cleared:
/// what it returns is the row's cost (NeoForge's `EnchantmentLevelSetEvent`).
/// It is not asked for an item the table can't enchant.
pub fn enchant_offers_leveled(
    e: &Enchantments,
    items: &Items,
    seed: i32,
    bookcases: i32,
    stack: Option<&ItemStack>,
    mut level_set: impl FnMut(usize, i32) -> i32,
) -> EnchantOffers {
    let mut offers = EnchantOffers::default();
    let Some(stack) = stack else { return offers };
    if !is_enchantable(e, stack) {
        return offers;
    }
    let mut random = LegacyRandom::new(i64::from(seed));
    for i in 0..3 {
        let cost = e.enchantment_cost_of(&mut random, i, bookcases, stack);
        offers.costs[i] = level_set(i, cost);
        if offers.costs[i] < i as i32 + 1 {
            offers.costs[i] = 0;
        }
    }
    for i in 0..3 {
        offers.clues[i] = enchant_clue(e, items, seed, i, offers.costs[i], stack);
    }
    offers
}
/// `EnchantmentMenu.slotsChanged`'s clue for offer `slot` at `cost`: one
/// enchantment of the list that cost would give, or None for no offer.
pub fn enchant_clue(
    e: &Enchantments,
    items: &Items,
    seed: i32,
    slot: usize,
    cost: i32,
    stack: &ItemStack,
) -> Option<(Arc<str>, u32)> {
    if cost <= 0 {
        return None;
    }
    // getEnchantmentList reseeds the menu random; the clue draws continue from it.
    let mut list_random = LegacyRandom::new(i64::from(seed.wrapping_add(slot as i32)));
    let pool = e.tag("minecraft:in_enchanting_table").to_vec();
    let mut list = e.select(items, &mut list_random, stack, cost, &pool);
    if &*stack.item == "minecraft:book" && list.len() > 1 {
        let r = list_random.next_int_bound(list.len() as i32).unwrap_or(0) as usize;
        list.remove(r);
    }
    if list.is_empty() {
        return None;
    }
    let pick = list_random.next_int_bound(list.len() as i32).unwrap_or(0) as usize;
    Some(list[pick].clone())
}
/// `EnchantmentMenu.clickMenuButton` result: the enchanted item, or None.
pub fn apply_enchantment(
    e: &Enchantments,
    items: &Items,
    seed: i32,
    slot: usize,
    cost: i32,
    stack: &ItemStack,
) -> Option<ItemStack> {
    let list = enchantment_list(e, items, seed, slot, cost, stack);
    apply_enchantment_list(stack, list)
}

/// Apply a selection to a copy of the stack, storing a book's enchantments in an enchanted book.
pub fn apply_enchantment_list(stack: &ItemStack, list: Levels) -> Option<ItemStack> {
    if list.is_empty() {
        return None;
    }
    let mut out = if &*stack.item == "minecraft:book" {
        stack.transmute("minecraft:enchanted_book", stack.count)
    } else {
        stack.clone()
    };
    for (id, level) in list {
        out.enchant(&id, level);
    }
    Some(out)
}

// ---------------------------------------------------------------- anvil

/// `ItemStack.isDamageableItem`: durability, and not `minecraft:unbreakable`.
pub fn is_damageable(items: &Items, stack: &ItemStack) -> bool {
    horizon_items::properties::damageable(|id| stack.carries_component(items.get(&stack.item), id))
}
pub fn max_damage(items: &Items, stack: &ItemStack) -> i32 {
    stack.max_damage(items.get(&stack.item))
}
/// `ItemStack.getDamageValue`: the damage clamped to `[0, max_damage]`.
fn damage_value(items: &Items, stack: &ItemStack) -> i32 {
    (stack.damage.min(i32::MAX as u32) as i32).clamp(0, max_damage(items, stack).max(0))
}
/// `ItemStack.setDamageValue`: stored clamped to `[0, max_damage]`.
fn set_damage_value(items: &Items, stack: &mut ItemStack, damage: i32) {
    stack.damage = damage.clamp(0, max_damage(items, stack).max(0)) as u32;
}
/// `ItemStack.isValidRepairItem`: `REPAIRABLE` from the stack's components,
/// not only the item's default.
pub fn is_valid_repair_item(
    e: &Enchantments,
    items: &Items,
    stack: &ItemStack,
    material: &ItemStack,
) -> bool {
    if let Some(data) = stack.data() {
        if let Some(component) = data.other("minecraft:repairable") {
            return horizon_items::properties::repairable(component, &material.item, |tag| {
                items.item_in_tag(tag, &material.item)
            });
        }
        if data.other("!minecraft:repairable").is_some() {
            return false;
        }
    }
    e.props(&stack.item)
        .is_some_and(|p| Enchantments::item_in(items, &p.repairable, &material.item))
}
/// `stack.has(STORED_ENCHANTMENTS)`: the enchanted book's default component
/// unless removed, or an explicitly set one, even when empty.
fn has_stored_enchantments(stack: &ItemStack) -> bool {
    let data = stack.data();
    if data.is_some_and(|d| d.other("!minecraft:stored_enchantments").is_some()) {
        return false;
    }
    &*stack.item == "minecraft:enchanted_book"
        || !stack.stored_enchantments().is_empty()
        || data.is_some_and(|d| d.other("minecraft:stored_enchantments").is_some())
}
/// `EnchantmentHelper.canStoreEnchantments`: the stack has the component the
/// item's enchantments live in (stored for a book, enchantments otherwise).
/// A removal patch leaves none.
fn can_store_enchantments(stack: &ItemStack) -> bool {
    let removed = if &*stack.item == "minecraft:enchanted_book" {
        "!minecraft:stored_enchantments"
    } else {
        "!minecraft:enchantments"
    };
    !stack.data().is_some_and(|d| d.other(removed).is_some())
}
/// `AnvilMenu.calculateIncreasedRepairCost`.
pub fn increased_repair_cost(cost: u32) -> u32 {
    (u64::from(cost) * 2 + 1).min(i32::MAX as u64) as u32
}
/// `ItemStack.getHoverName`: the custom name, a potion's or tipped arrow's
/// contents, a written book's title, the `item_name` component, else the
/// item's or head owner's name.
pub fn hover_name(names: &crate::creative::Names, stack: &ItemStack) -> String {
    if let Some(name) = stack.custom_name() {
        return name;
    }
    if let Some(key) = crate::brewing::contents_name_key(stack) {
        return names
            .display_get(&key)
            .unwrap_or_else(|| names.display_item(&stack.item));
    }
    if let Some(title) = stack
        .data()
        .and_then(|d| d.written.as_ref())
        .map(|w| w.title.as_str())
        .filter(|t| !t.trim().is_empty())
    {
        return title.to_owned();
    }
    if let Some(head) = crate::item_tooltip::player_head_name(stack) {
        // `PlayerHeadItem.getName`: a profile's name, else the item name.
        return head.title(&|key| names.display_get(key));
    }
    if let Some(name) = stack.item_name() {
        return name;
    }
    if stack
        .data()
        .is_some_and(|d| d.other("!minecraft:item_name").is_some())
    {
        // `Item.getName` is `getOrDefault(ITEM_NAME, EMPTY)`.
        return String::new();
    }
    if stack.data().is_some_and(|d| d.ominous_banner) {
        return names
            .display_get("block.minecraft.ominous_banner")
            .unwrap_or_else(|| "Ominous Banner".to_owned());
    }
    names.display_item(&stack.item)
}
/// `AnvilMenu.createResult`. `name` is the rename box text (None = untouched).
pub fn anvil_result(
    e: &Enchantments,
    items: &Items,
    input: Option<&ItemStack>,
    addition: Option<&ItemStack>,
    name: Option<&str>,
    hover_name: &str,
    creative: bool,
) -> AnvilResult {
    let fail = AnvilResult::default();
    let Some(input) = input else { return fail };
    // EnchantmentHelper.canStoreEnchantments: the item's enchantments component
    // is there (all items have one by default; a removal patch leaves none).
    if !can_store_enchantments(input) {
        return fail;
    }
    let mut result = input.clone();
    let mut enchantments: Levels = input.crafting_enchantments().to_vec();
    let mut price: i64 = 0;
    let tax: i64 =
        i64::from(input.repair_cost()) + i64::from(addition.map_or(0, ItemStack::repair_cost));
    let mut repair_item_count_cost = 0;
    let mut naming_cost = 0;
    if let Some(addition) = addition {
        // addition.has(STORED_ENCHANTMENTS): an enchanted book (its default
        // component), or any item given stored enchantments.
        let using_book = has_stored_enchantments(addition);
        if is_damageable(items, &result) && is_valid_repair_item(e, items, input, addition) {
            let max = max_damage(items, &result);
            let mut repair = damage_value(items, &result).min(max / 4);
            if repair <= 0 {
                return fail;
            }
            let mut count = 0;
            while repair > 0 && count < addition.count {
                let damage = damage_value(items, &result) - repair;
                set_damage_value(items, &mut result, damage);
                price += 1;
                repair = damage_value(items, &result).min(max / 4);
                count += 1;
            }
            repair_item_count_cost = count;
        } else {
            if !using_book && (result.item != addition.item || !is_damageable(items, &result)) {
                return fail;
            }
            if is_damageable(items, &result) && !using_book {
                let max = max_damage(items, &result);
                let remaining1 = max_damage(items, input) - damage_value(items, input);
                let remaining2 = max_damage(items, addition) - damage_value(items, addition);
                let additional = remaining2 + max * 12 / 100;
                let remaining = remaining1 + additional;
                let damage = (max - remaining).max(0);
                if damage < damage_value(items, &result) {
                    set_damage_value(items, &mut result, damage);
                    price += 2;
                }
            }
            let mut any_compatible = false;
            let mut any_incompatible = false;
            for (id, level) in addition.crafting_enchantments() {
                let current = enchantments
                    .iter()
                    .find(|(x, _)| x == id)
                    .map_or(0, |(_, l)| *l);
                let mut level = if current == *level {
                    level + 1
                } else {
                    (*level).max(current)
                };
                let Some(def) = e.get(id) else { continue };
                let mut compatible = e.can_enchant(items, id, &input.item);
                if creative || &*input.item == "minecraft:enchanted_book" {
                    compatible = true;
                }
                for (other, _) in &enchantments {
                    if other != id && !e.compatible(id, other) {
                        compatible = false;
                        price += 1;
                    }
                }
                if !compatible {
                    any_incompatible = true;
                } else {
                    any_compatible = true;
                    level = level.min(def.max_level);
                    item_data::set_level(&mut enchantments, id, level);
                    let mut fee = def.anvil_cost as i64;
                    if using_book {
                        fee = (fee / 2).max(1);
                    }
                    price += fee * i64::from(level);
                    if input.count > 1 {
                        price = 40;
                    }
                }
            }
            if any_incompatible && !any_compatible {
                return fail;
            }
        }
    }
    match name {
        Some(n) if !n.trim().is_empty() => {
            if n != hover_name {
                naming_cost = 1;
                price += 1;
                result.data_mut().custom_name = Some(horizon_core::nbt::Tag::String(n.into()));
            }
        }
        _ => {
            if input.custom_name().is_some() {
                naming_cost = 1;
                price += 1;
                result.data_mut().custom_name = None;
            }
        }
    }
    let mut cost = if price <= 0 {
        0
    } else {
        (tax + price).clamp(0, i64::from(i32::MAX)) as i32
    };
    let mut out = (price > 0).then_some(result);
    let mut only_renaming = false;
    if naming_cost == price && naming_cost > 0 {
        if cost >= 40 {
            cost = 39;
        }
        only_renaming = true;
    }
    if cost >= 40 && !creative {
        out = None;
    }
    if let Some(r) = out.as_mut() {
        let mut base = r
            .repair_cost()
            .max(addition.map_or(0, ItemStack::repair_cost));
        if naming_cost != price || naming_cost == 0 {
            base = increased_repair_cost(base);
        }
        // `set(REPAIR_COST, base)` replaces the patch, including a removal
        // marker the input carried; zero is the component's default.
        let data = r.data_mut();
        data.set_other("!minecraft:repair_cost", None);
        data.repair_cost = base;
        r.set_crafting_enchantments(enchantments);
        r.tidy();
    }
    AnvilResult {
        result: out,
        cost,
        repair_item_count_cost,
        only_renaming,
    }
}

// ---------------------------------------------------------------- grindstone

/// `ItemStack.getMaxStackSize`: the stack's `MAX_STACK_SIZE` component (a
/// removed one is vanilla's default of 1), else the item's default.
fn max_stack_size(items: &Items, stack: &ItemStack) -> u32 {
    crate::menu::item_max_stack(items, stack)
}
/// `GrindstoneMenu` slot filter: damageable or enchanted.
pub fn grindstone_accepts(items: &Items, stack: &ItemStack) -> bool {
    is_damageable(items, stack) || stack.has_any_enchantments()
}
fn remove_non_curses(e: &Enchantments, mut item: ItemStack) -> ItemStack {
    let kept: Levels = item
        .crafting_enchantments()
        .iter()
        .filter(|(id, _)| e.is_curse(id))
        .cloned()
        .collect();
    let n = kept.len();
    item.set_crafting_enchantments(kept);
    if &*item.item == "minecraft:enchanted_book" && item.stored_enchantments().is_empty() {
        item = item.transmute("minecraft:book", item.count);
        // A book has no stored-enchantments default, so `transmuteCopy` drops
        // the enchanted book's removal patch.
        item.data_mut()
            .set_other("!minecraft:stored_enchantments", None);
    }
    let mut cost = 0;
    for _ in 0..n {
        cost = increased_repair_cost(cost);
    }
    // `set(REPAIR_COST, cost)` replaces a removal marker the input carried.
    let data = item.data_mut();
    data.set_other("!minecraft:repair_cost", None);
    data.repair_cost = cost;
    item.tidy();
    item
}
/// `GrindstoneMenu.computeResult`.
pub fn grindstone_result(
    e: &Enchantments,
    items: &Items,
    input: Option<&ItemStack>,
    additional: Option<&ItemStack>,
) -> Option<ItemStack> {
    if input.is_none() && additional.is_none() {
        return None;
    }
    if input.is_some_and(|s| s.count > 1) || additional.is_some_and(|s| s.count > 1) {
        return None;
    }
    match (input, additional) {
        (Some(a), Some(b)) => {
            if a.item != b.item {
                return None;
            }
            let durability = max_damage(items, a).max(max_damage(items, b));
            let remaining = (max_damage(items, a) - damage_value(items, a))
                + (max_damage(items, b) - damage_value(items, b))
                + durability * 5 / 100;
            let mut count = 1;
            if !is_damageable(items, a) {
                let stack_max = max_stack_size(items, a);
                if stack_max < 2 || a != b {
                    return None;
                }
                count = 2;
            }
            let mut out = ItemStack { count, ..a.clone() };
            if is_damageable(items, &out) {
                // GrindstoneMenu.mergeItems writes the larger MAX_DAMAGE first. The
                // patch drops a value equal to the item's own (PatchedDataComponentMap.set).
                let default = items.get(&out.item).map_or(0, |i| i.max_damage as i32);
                let data = out.data_mut();
                data.set_other("!minecraft:max_damage", None);
                data.set_other(
                    "minecraft:max_damage",
                    (durability != default).then_some(horizon_core::nbt::Tag::Int(durability)),
                );
                set_damage_value(items, &mut out, durability - remaining);
            }
            // mergeEnchantsFrom
            let mut levels = out.crafting_enchantments().to_vec();
            for (id, l) in b.crafting_enchantments() {
                if !e.is_curse(id) || !levels.iter().any(|(x, _)| x == id) {
                    item_data::upgrade(&mut levels, id, *l);
                }
            }
            out.set_crafting_enchantments(levels);
            Some(remove_non_curses(e, out))
        }
        (Some(s), None) | (None, Some(s)) => s
            .has_any_enchantments()
            .then(|| remove_non_curses(e, s.clone())),
        (None, None) => None,
    }
}
/// The grindstone result slot's `getExperienceAmount` before its roll: the
/// minimum costs of the inputs' non-curse enchantments.
pub fn grindstone_xp_base(
    e: &Enchantments,
    input: Option<&ItemStack>,
    additional: Option<&ItemStack>,
) -> i32 {
    let from = |s: Option<&ItemStack>| -> i32 {
        s.map_or(0, |s| {
            s.crafting_enchantments()
                .iter()
                .filter(|(id, _)| !e.is_curse(id))
                .map(|(id, l)| e.get(id).map_or(0, |d| d.min_cost.calculate(*l)))
                .sum()
        })
    };
    from(input) + from(additional)
}
/// `getExperienceAmount`'s roll of the sum: `half + nextInt(half)`.
pub fn grindstone_xp_roll(amount: i32, random: &mut LegacyRandom) -> u32 {
    if amount > 0 {
        let half = (f64::from(amount) / 2.0).ceil() as i32;
        (half + random.next_int_bound(half).unwrap_or(0)) as u32
    } else {
        0
    }
}
/// Experience from the grindstone result slot's `getExperienceAmount`.
pub fn grindstone_experience(
    e: &Enchantments,
    random: &mut LegacyRandom,
    input: Option<&ItemStack>,
    additional: Option<&ItemStack>,
) -> u32 {
    grindstone_xp_roll(grindstone_xp_base(e, input, additional), random)
}

// ---------------------------------------------------------------- smithing & stonecutting

#[derive(Clone, Debug)]
pub enum Smithing {
    Transform {
        id: String,
        template: Option<Ingredient>,
        base: Ingredient,
        addition: Option<Ingredient>,
        result: crate::crafting::Template,
    },
    Trim {
        id: String,
        template: Ingredient,
        base: Ingredient,
        addition: Ingredient,
        pattern: crate::crafting::RecipeTrimPattern,
    },
}
#[derive(Clone, Debug)]
pub struct Stonecutting {
    pub id: String,
    pub input: Ingredient,
    pub result: ItemStack,
}
/// One canonical parse shared by the station tables and the mod recipe view.
#[derive(Clone, Debug)]
pub(crate) enum ParsedStationRecipe {
    Smithing(Smithing),
    Stonecutting(Stonecutting),
}
impl ParsedStationRecipe {
    pub(crate) fn parse(id: String, v: &Value, items: &Items) -> Option<Self> {
        match v["type"].as_str().unwrap_or("") {
            "minecraft:smithing_transform" => {
                // Only omitted ingredients mean empty slots; malformed present
                // ingredients invalidate the recipe rather than becoming optional.
                let parse_optional = |key: &str| match v.get(key) {
                    None => Some(None),
                    Some(value) => Ingredient::parse(value, items).map(Some),
                };
                Some(Self::Smithing(Smithing::Transform {
                    id,
                    template: parse_optional("template")?,
                    base: Ingredient::parse(&v["base"], items)?,
                    addition: parse_optional("addition")?,
                    result: crate::crafting::Template::parse(&v["result"])?,
                }))
            }
            "minecraft:smithing_trim" => Some(Self::Smithing(Smithing::Trim {
                id,
                template: Ingredient::parse(&v["template"], items)?,
                base: Ingredient::parse(&v["base"], items)?,
                addition: Ingredient::parse(&v["addition"], items)?,
                // Use the same holder codec as item components, collapsing
                // unstyled literal text once at the recipe boundary before
                // publishing the typed snapshot.
                pattern: match crate::item_data::trim::pattern_from_nbt(
                    &crate::crafting::json_tag(v.get("pattern")?),
                )? {
                    crate::item_data::trim::Holder::Named(id) => {
                        crate::crafting::RecipeTrimPattern::Reference(id.to_string())
                    }
                    crate::item_data::trim::Holder::Direct(pattern) => {
                        crate::crafting::RecipeTrimPattern::Inline {
                            asset_id: pattern.asset_id.to_string(),
                            description: pattern.description,
                            decal: pattern.decal,
                        }
                    }
                },
            })),
            "minecraft:stonecutting" => Some(Self::Stonecutting(Stonecutting {
                id,
                input: Ingredient::parse(&v["ingredient"], items)?,
                result: crate::crafting::Template::parse(&v["result"])?.create(),
            })),
            _ => None,
        }
    }

    pub(crate) fn rule(&self) -> crate::crafting::RecipeRule {
        match self {
            Self::Stonecutting(recipe) => crate::crafting::RecipeRule::Stonecutting {
                input: recipe.input.clone(),
                result: recipe.result.clone(),
            },
            Self::Smithing(Smithing::Transform {
                template,
                base,
                addition,
                result,
                ..
            }) => crate::crafting::RecipeRule::SmithingTransform {
                template: template.clone(),
                base: base.clone(),
                addition: addition.clone(),
                result: result.view(),
            },
            Self::Smithing(Smithing::Trim {
                template,
                base,
                addition,
                pattern,
                ..
            }) => crate::crafting::RecipeRule::SmithingTrim {
                template: template.clone(),
                base: base.clone(),
                addition: addition.clone(),
                pattern: pattern.clone(),
            },
        }
    }
}
#[derive(Default)]
pub struct StationRecipes {
    pub smithing: Vec<Smithing>,
    /// Sorted by recipe id, the registry order vanilla lists them in.
    pub stonecutting: Vec<Stonecutting>,
    /// `trim_material` registry: id -> (asset name, description).
    pub trim_materials: Vec<(String, String)>,
    /// What mods added to the smithing slots' property sets (template, base,
    /// addition).
    smithing_additions: [HashSet<String>; 3],
    item_limits: std::collections::HashMap<Arc<str>, (u32, u32)>,
}
impl StationRecipes {
    pub fn load(
        root: &Path,
        items: &Items,
        overlay: &PackOverlay,
        context: &horizon_world::native_load_context::LoadContext,
    ) -> Self {
        let sources = crate::crafting::ResolvedSources::load_or_empty(root, overlay, context);
        Self::from_sources(root, items, &sources)
    }
    /// The station tables of `sources`, the recipes one load resolved.
    pub fn from_sources(
        root: &Path,
        items: &Items,
        sources: &crate::crafting::ResolvedSources,
    ) -> Self {
        let mut smithing = Vec::new();
        let mut stonecutting = Vec::new();
        for recipe in sources.station_recipes(items).iter().flatten() {
            match recipe {
                ParsedStationRecipe::Smithing(recipe) => smithing.push(recipe.clone()),
                ParsedStationRecipe::Stonecutting(recipe) if !recipe.input.items.is_empty() => {
                    stonecutting.push(recipe.clone())
                }
                ParsedStationRecipe::Stonecutting(_) => {}
            }
        }
        let mut trim_materials = Vec::new();
        let mut tm: Vec<_> = fs::read_dir(root.join("data/minecraft/trim_material"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .collect();
        tm.sort();
        for p in tm {
            if let Ok(bytes) = fs::read(&p)
                && let Ok(v) = serde_json::from_slice::<Value>(&bytes)
            {
                let id = format!(
                    "minecraft:{}",
                    p.file_stem().and_then(|s| s.to_str()).unwrap_or("")
                );
                trim_materials.push((
                    id,
                    v["description"]["translate"]
                        .as_str()
                        .unwrap_or("")
                        .to_owned(),
                ));
            }
        }
        let added = |key| {
            sources
                .property_additions
                .get(key)
                .cloned()
                .unwrap_or_default()
        };
        Self {
            smithing,
            stonecutting,
            trim_materials,
            item_limits: items
                .items
                .iter()
                .map(|(id, info)| (id.clone(), (info.max_stack, info.max_damage)))
                .collect(),
            smithing_additions: [
                added("smithing_template"),
                added("smithing_base"),
                added("smithing_addition"),
            ],
        }
    }
    /// Recipes whose input accepts `stack`, in display order.
    pub fn stonecutter(&self, context: &LoopContext, stack: &ItemStack) -> Vec<&Stonecutting> {
        self.stonecutting
            .iter()
            .filter(|r| r.input.test_in(context, stack))
            .collect()
    }
    fn slot_ok(&self, context: &LoopContext, slot: usize, stack: &ItemStack) -> bool {
        self.smithing_additions[slot].contains(&*stack.item)
            || self
                .smithing
                .iter()
                .filter(|r| match r {
                    Smithing::Transform {
                        template,
                        base,
                        addition,
                        ..
                    } => [template.as_ref(), Some(base), addition.as_ref()]
                        .into_iter()
                        .flatten()
                        .all(|i| !i.items.is_empty()),
                    Smithing::Trim {
                        template,
                        base,
                        addition,
                        ..
                    } => [template, base, addition]
                        .into_iter()
                        .all(|i| !i.items.is_empty()),
                })
                .any(|r| match r {
                    Smithing::Transform {
                        template,
                        base,
                        addition,
                        ..
                    } => [template.as_ref(), Some(base), addition.as_ref()][slot]
                        .is_some_and(|i| i.test_in(context, stack)),
                    Smithing::Trim {
                        template,
                        base,
                        addition,
                        ..
                    } => [template, base, addition][slot].test_in(context, stack),
                })
    }
    /// `RecipePropertySet` SMITHING_TEMPLATE / BASE / ADDITION.
    pub fn smithing_accepts(&self, context: &LoopContext, slot: usize, stack: &ItemStack) -> bool {
        slot < 3 && self.slot_ok(context, slot, stack)
    }
    /// `SmithingMenu.createResult`.
    pub fn smithing_result(
        &self,
        context: &LoopContext,
        e: &Enchantments,
        template: Option<&ItemStack>,
        base: Option<&ItemStack>,
        addition: Option<&ItemStack>,
    ) -> Option<ItemStack> {
        self.smithing_recipe(context, e, template, base, addition)
            .map(|(_, result)| result)
    }
    /// `SmithingMenu.createResult` with the id of the recipe that made the result.
    pub fn smithing_recipe(
        &self,
        context: &LoopContext,
        e: &Enchantments,
        template: Option<&ItemStack>,
        base: Option<&ItemStack>,
        addition: Option<&ItemStack>,
    ) -> Option<(&str, ItemStack)> {
        let nonempty = |stack: &ItemStack| stack.count > 0 && &*stack.item != "minecraft:air";
        let base = base.filter(|s| nonempty(s))?;
        let optional_matches = |ingredient: &Option<Ingredient>, stack: Option<&ItemStack>| match (
            ingredient,
            stack.filter(|s| nonempty(s)),
        ) {
            (None, None) => true,
            (Some(ingredient), Some(stack)) => ingredient.test_in(context, stack),
            _ => false,
        };
        for r in &self.smithing {
            match r {
                Smithing::Transform {
                    id,
                    template: t,
                    base: b,
                    addition: a,
                    result,
                } => {
                    if optional_matches(t, template)
                        && b.test_in(context, base)
                        && optional_matches(a, addition)
                    {
                        let output = result.over(base, 0);
                        let (mut max_stack, mut max_damage) = self
                            .item_limits
                            .get(&output.item)
                            .copied()
                            .unwrap_or((64, 0));
                        if let Some(data) = output.data() {
                            for (key, value) in &data.other {
                                match (key.as_str(), value) {
                                    (
                                        "minecraft:max_stack_size",
                                        horizon_core::nbt::Tag::Int(n),
                                    ) => max_stack = *n as u32,
                                    ("!minecraft:max_stack_size", _) => max_stack = 1,
                                    ("minecraft:max_damage", horizon_core::nbt::Tag::Int(n)) => {
                                        max_damage = *n as u32
                                    }
                                    ("!minecraft:max_damage", _) => max_damage = 0,
                                    _ => {}
                                }
                            }
                        }
                        return (output.count > 0
                            && output.count <= max_stack
                            && !(max_damage > 0 && max_stack > 1))
                            .then_some((id.as_str(), output));
                    }
                }
                Smithing::Trim {
                    id,
                    template: t,
                    base: b,
                    addition: a,
                    pattern,
                    ..
                } => {
                    if template.is_some_and(|s| t.test_in(context, s))
                        && b.test_in(context, base)
                        && addition.is_some_and(|s| a.test_in(context, s))
                    {
                        return apply_trim(e, base, addition?, pattern).map(|r| (id.as_str(), r));
                    }
                }
            }
        }
        None
    }
    /// `SmithingTrimDemoSlotDisplay.resolveForFirstStack` of a trim recipe:
    /// a random base trimmed with a random material (`pick(n)` draws below n).
    pub fn trim_demo(
        &self,
        e: &Enchantments,
        id: &str,
        pick: &mut dyn FnMut(usize) -> usize,
    ) -> Option<ItemStack> {
        let (base, addition, pattern) = self.smithing.iter().find_map(|r| match r {
            Smithing::Trim {
                id: i,
                base,
                addition,
                pattern,
                ..
            } if i == id => Some((&base.order, &addition.order, pattern)),
            _ => None,
        })?;
        if base.is_empty() || addition.is_empty() {
            return None;
        }
        // applyDemoTransformation: up to 256 draws, the first that isn't empty.
        (0..256).find_map(|_| {
            let base = ItemStack::new(&base[pick(base.len())], 1);
            let material = ItemStack::new(&addition[pick(addition.len())], 1);
            apply_trim(e, &base, &material, pattern)
        })
    }
}
/// `ItemStack.get(DataComponents.PROVIDES_TRIM_MATERIAL)`: the stack's
/// component (a name or an inline definition), else the item's default.
fn trim_material(
    e: &Enchantments,
    stack: &ItemStack,
) -> Option<crate::item_data::trim::Holder<crate::item_data::trim::Material>> {
    let data = stack.data();
    if data.is_some_and(|d| d.other("!minecraft:provides_trim_material").is_some()) {
        return None;
    }
    if let Some(tag) = data.and_then(|d| d.other("minecraft:provides_trim_material")) {
        return crate::item_data::trim::material_from_nbt(tag);
    }
    e.props(&stack.item)
        .and_then(|p| p.trim_material.clone())
        .map(|id| crate::item_data::trim::Holder::Named(id.into()))
}
/// `SmithingTrimRecipe.applyTrim`: none when the material provides no trim
/// or the base already has this one.
pub fn apply_trim(
    e: &Enchantments,
    base: &ItemStack,
    material: &ItemStack,
    pattern: &crate::crafting::RecipeTrimPattern,
) -> Option<ItemStack> {
    use crate::item_data::trim::{Holder, Pattern};
    let pattern = match pattern {
        crate::crafting::RecipeTrimPattern::Reference(id) => Holder::Named(id.as_str().into()),
        crate::crafting::RecipeTrimPattern::Inline {
            asset_id,
            description,
            decal,
        } => Holder::Direct(Pattern {
            asset_id: asset_id.as_str().into(),
            description: description.clone(),
            decal: *decal,
        }),
    };
    let trim = (trim_material(e, material)?, pattern);
    if base.data().and_then(|d| d.trim.as_ref()) == Some(&trim) {
        return None;
    }
    let mut out = ItemStack {
        count: 1,
        ..base.clone()
    };
    out.data_mut().trim = Some(trim);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enchant::tests::{sample, sample_items};
    use horizon_core::nbt::Tag;

    fn sword(ench: &[(&str, u32)]) -> ItemStack {
        let mut s = ItemStack::new("minecraft:diamond_sword", 1);
        for (id, l) in ench {
            s.enchant(id, *l);
        }
        s
    }
    fn book(ench: &[(&str, u32)]) -> ItemStack {
        let mut s = ItemStack::new("minecraft:enchanted_book", 1);
        for (id, l) in ench {
            s.enchant(id, *l);
        }
        s
    }

    fn worn(item: &str, max: i32, damage: u32) -> ItemStack {
        let mut s = ItemStack::new(item, 1);
        s.damage = damage;
        s.data_mut().set_other(
            "minecraft:max_damage",
            Some(horizon_core::nbt::Tag::Int(max)),
        );
        s
    }

    /// Anvil: `ItemStack.isValidRepairItem` reads the stack's `REPAIRABLE`
    /// component, so a patched list repairs where the item's default does not.
    #[test]
    fn anvil_reads_the_stacks_repairable_component() {
        let e = sample();
        let items = sample_items();
        let mut sword = worn("minecraft:diamond_sword", 100, 20);
        sword.data_mut().set_other(
            "minecraft:repairable",
            Some(Tag::compound([(
                "items",
                Tag::List(vec![Tag::String("minecraft:stick".into())]),
            )])),
        );
        let r = anvil_result(
            &e,
            &items,
            Some(&sword),
            Some(&ItemStack::new("minecraft:stick", 1)),
            None,
            "",
            false,
        );
        let out = r.result.expect("the stick repairs the sword");
        // 20 damage of a 100 max is under the 25 one material restores.
        assert_eq!(out.damage, 0);
        assert_eq!(r.cost, 1);
        assert_eq!(r.repair_item_count_cost, 1);
        assert_eq!(out.repair_cost(), 1);
    }

    /// Grindstone: `mergeItems` asks `ItemStack.getMaxStackSize` before the
    /// two-slot count, so a patched limit of one refuses the merge.
    #[test]
    fn grindstone_merge_reads_the_stacks_max_stack_size() {
        let e = sample();
        let items = sample_items();
        let enchanted_book = |limit: Option<Tag>| {
            let mut book = ItemStack::new("minecraft:book", 1);
            book.enchant("minecraft:sharpness", 1);
            match limit {
                Some(value) => book
                    .data_mut()
                    .set_other("minecraft:max_stack_size", Some(value)),
                None => {
                    book.data_mut().set_other(
                        "!minecraft:max_stack_size",
                        Some(Tag::compound(Vec::<(&str, Tag)>::new())),
                    );
                }
            }
            book
        };
        let one = enchanted_book(Some(Tag::Int(1)));
        assert!(grindstone_result(&e, &items, Some(&one), Some(&one)).is_none());
        // A removed component is vanilla's default of one, not the item's 64.
        let removed = enchanted_book(None);
        assert!(grindstone_result(&e, &items, Some(&removed), Some(&removed)).is_none());
        // Two plain books still combine to a count of two.
        let mut book = ItemStack::new("minecraft:book", 1);
        book.enchant("minecraft:sharpness", 1);
        let out = grindstone_result(&e, &items, Some(&book), Some(&book)).unwrap();
        assert_eq!(out.count, 2);
        assert!(!out.has_any_enchantments());
    }

    /// Smithing: `SmithingTrimRecipe.applyTrim` reads the addition's
    /// `PROVIDES_TRIM_MATERIAL`, a named material or an inline definition.
    #[test]
    fn smithing_trim_reads_the_additions_trim_material() {
        let e = sample();
        let recipes = StationRecipes {
            smithing: vec![Smithing::Trim {
                id: "minecraft:coast".into(),
                template: Ingredient::of(vec![
                    "minecraft:coast_armor_trim_smithing_template".into(),
                ]),
                base: Ingredient::of(vec!["minecraft:diamond_chestplate".into()]),
                addition: Ingredient::of(vec!["minecraft:gold_ingot".into()]),
                pattern: crate::crafting::RecipeTrimPattern::Reference("minecraft:coast".into()),
            }],
            ..Default::default()
        };
        let template = ItemStack::new("minecraft:coast_armor_trim_smithing_template", 1);
        let base = ItemStack::new("minecraft:diamond_chestplate", 1);
        let mut gold = ItemStack::new("minecraft:gold_ingot", 1);
        gold.data_mut().set_other(
            "minecraft:provides_trim_material",
            Some(Tag::String("minecraft:copper".into())),
        );
        let out = recipes
            .smithing_result(
                crate::test_resources::server_loop_context(),
                &e,
                Some(&template),
                Some(&base),
                Some(&gold),
            )
            .expect("the gold ingot provides a trim material");
        assert_eq!(
            crate::item_data::trim::named(
                out.data().and_then(|d| d.trim.as_ref()).expect("a trim")
            ),
            Some(("minecraft:copper", "minecraft:coast"))
        );
        // An inline definition is the addition's material too.
        let mut inline = ItemStack::new("minecraft:gold_ingot", 1);
        inline.data_mut().set_other(
            "minecraft:provides_trim_material",
            Some(Tag::compound([
                ("palette_id", Tag::String("minecraft:gold".into())),
                ("description", Tag::String("Gold".into())),
            ])),
        );
        let out = recipes
            .smithing_result(
                crate::test_resources::server_loop_context(),
                &e,
                Some(&template),
                Some(&base),
                Some(&inline),
            )
            .expect("the inline material applies");
        assert!(matches!(
            out.data().and_then(|d| d.trim.as_ref()).map(|t| &t.0),
            Some(crate::item_data::trim::Holder::Direct(_))
        ));
    }

    /// Enchanting: `ItemStack.isEnchantable` and `selectEnchantment` read the
    /// stack's `ENCHANTABLE`, so a patch can remove or change an offer.
    #[test]
    fn enchanting_reads_the_stacks_enchantable_component() {
        let e = sample();
        let items = sample_items();
        let mut removed = ItemStack::new("minecraft:diamond_sword", 1);
        removed.data_mut().set_other(
            "!minecraft:enchantable",
            Some(Tag::compound(Vec::<(&str, Tag)>::new())),
        );
        assert_eq!(
            enchant_offers(&e, &items, 1234, 15, Some(&removed)).costs,
            [0, 0, 0]
        );
        let mut low = ItemStack::new("minecraft:diamond_sword", 1);
        low.data_mut().set_other(
            "minecraft:enchantable",
            Some(Tag::compound([("value", Tag::Int(1))])),
        );
        let offers = enchant_offers(&e, &items, 1234, 15, Some(&low));
        assert_eq!(offers.costs, [5, 13, 30]);
        let plain = ItemStack::new("minecraft:diamond_sword", 1);
        assert!((0..100).any(|seed| {
            let offers = enchant_offers(&e, &items, seed, 15, Some(&low));
            (0..3).any(|slot| {
                let patched =
                    apply_enchantment(&e, &items, seed, slot, offers.costs[slot], &low).unwrap();
                let default =
                    apply_enchantment(&e, &items, seed, slot, offers.costs[slot], &plain).unwrap();
                patched.enchantments() != default.enchantments()
            })
        }));
        // No item default is needed: the component alone makes a stick an
        // enchantable item, with costs but no supported enchantment.
        let mut stick = ItemStack::new("minecraft:stick", 1);
        stick.data_mut().set_other(
            "minecraft:enchantable",
            Some(Tag::compound([("value", Tag::Int(15))])),
        );
        let offers = enchant_offers(&e, &items, 1234, 15, Some(&stick));
        assert_eq!(offers.costs, [5, 13, 30]);
        assert!(offers.clues.iter().all(Option::is_none));
    }

    /// Anvil: `EnchantmentHelper.canStoreEnchantments` is component presence,
    /// so a removed enchantments component leaves no result, not even a rename.
    #[test]
    fn anvil_refuses_an_item_without_its_enchantments_component() {
        let e = sample();
        let items = sample_items();
        let removed = || {
            Some(horizon_core::nbt::Tag::compound(Vec::<(
                &str,
                horizon_core::nbt::Tag,
            )>::new()))
        };
        let mut blade = sword(&[]);
        blade
            .data_mut()
            .set_other("!minecraft:enchantments", removed());
        let r = anvil_result(
            &e,
            &items,
            Some(&blade),
            None,
            Some("Blade"),
            "Diamond Sword",
            false,
        );
        assert_eq!((r.result, r.cost), (None, 0));
        let mut book = book(&[]);
        book.data_mut()
            .set_other("!minecraft:stored_enchantments", removed());
        let r = anvil_result(
            &e,
            &items,
            Some(&book),
            None,
            Some("Blade"),
            "Enchanted Book",
            false,
        );
        assert_eq!((r.result, r.cost), (None, 0));
        // The same items without the removal still rename.
        let r = anvil_result(
            &e,
            &items,
            Some(&sword(&[])),
            None,
            Some("Blade"),
            "Diamond Sword",
            false,
        );
        assert_eq!(r.cost, 1);
        assert!(r.result.is_some());
    }

    /// Grindstone: the larger MAX_DAMAGE is written and the damage follows it.
    #[test]
    fn grindstone_merge_uses_component_max_damage() {
        let e = sample();
        let items = sample_items();
        let a = worn("minecraft:diamond_sword", 100, 90);
        let b = worn("minecraft:diamond_sword", 200, 190);
        let out = grindstone_result(&e, &items, Some(&a), Some(&b)).unwrap();
        assert_eq!(
            out.data().unwrap().other("minecraft:max_damage"),
            Some(&horizon_core::nbt::Tag::Int(200))
        );
        // remaining 10 + 10 + 200 * 5 / 100 = 30 -> damage 170.
        assert_eq!(out.damage, 170);
    }

    /// Grindstone: a merged MAX_DAMAGE equal to the item's own is no patch, so the
    /// result saves without it (vanilla: two worn diamond pickaxes give a plain one).
    #[test]
    fn grindstone_merge_at_the_default_max_damage_writes_no_component() {
        let root = std::env::temp_dir().join(format!("horizon-grind-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("items.json"),
            r#"{"minecraft:diamond_sword":{"minecraft:max_stack_size":1,"minecraft:max_damage":1561}}"#,
        )
        .unwrap();
        let items = Items::catalog(
            &root,
            &PackOverlay::NONE,
            &horizon_world::item_defaults::read(&root).unwrap(),
        )
        .unwrap();
        let e = sample();
        let mut a = ItemStack::new("minecraft:diamond_sword", 1);
        a.damage = 819;
        let mut b = ItemStack::new("minecraft:diamond_sword", 1);
        b.damage = 602;
        let out = grindstone_result(&e, &items, Some(&a), Some(&b)).unwrap();
        assert_eq!(
            out.data().and_then(|d| d.other("minecraft:max_damage")),
            None
        );
        // remaining 742 + 959 + 78 = 1779 > 1561 -> damage 0.
        assert_eq!(out.damage, 0);
        // A larger patched MAX_DAMAGE on the second stack still wins and is written.
        let b = worn("minecraft:diamond_sword", 2000, 1990);
        let out = grindstone_result(&e, &items, Some(&a), Some(&b)).unwrap();
        assert_eq!(
            out.data().unwrap().other("minecraft:max_damage"),
            Some(&horizon_core::nbt::Tag::Int(2000))
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// Anvil: merging two damaged items reads each stack's own max damage.
    #[test]
    fn anvil_merge_uses_component_max_damage() {
        let e = sample();
        let items = sample_items();
        let a = worn("minecraft:diamond_sword", 100, 50);
        let b = worn("minecraft:diamond_sword", 100, 50);
        let r = anvil_result(&e, &items, Some(&a), Some(&b), None, "", false);
        // remaining 50 + 50 + 12 = 112 -> damage 0.
        assert_eq!(r.result.unwrap().damage, 0);
        assert_eq!(r.cost, 2);
    }

    /// `has(STORED_ENCHANTMENTS)` is presence: an empty component counts, an
    /// enchanted book counts by default.
    #[test]
    fn stored_enchantments_presence() {
        let mut stick = ItemStack::new("minecraft:stick", 1);
        assert!(!has_stored_enchantments(&stick));
        stick.data_mut().set_other(
            "minecraft:stored_enchantments",
            Some(horizon_core::nbt::Tag::compound(Vec::<(&str, _)>::new())),
        );
        assert!(has_stored_enchantments(&stick));
        let mut book = ItemStack::new("minecraft:enchanted_book", 1);
        assert!(has_stored_enchantments(&book));
        book.data_mut().set_other(
            "!minecraft:stored_enchantments",
            Some(horizon_core::nbt::Tag::compound(Vec::<(&str, _)>::new())),
        );
        assert!(!has_stored_enchantments(&book));
    }

    #[test]
    fn bookshelf_ring_has_32_positions() {
        let o = bookshelf_offsets();
        assert_eq!(o.len(), 32);
        assert!(o.contains(&[-2, 0, -2]) && o.contains(&[2, 1, 0]) && !o.contains(&[1, 0, 1]));
        // BlockPos.betweenClosed(-2, 0, -2, 2, 1, 2): x fastest, then y, then z.
        let start = [[-2, 0, -2], [-1, 0, -2], [0, 0, -2], [1, 0, -2], [2, 0, -2]];
        assert_eq!(o[..5], start);
        assert_eq!(o[5], [-2, 1, -2]);
        assert_eq!(o[10..12], [[-2, 0, -1], [2, 0, -1]]);
        assert_eq!(o[31], [2, 1, 2]);
    }

    /// `EnchantmentLevelSetEvent`: each row is asked once, in order, with the
    /// level `getEnchantmentCost` drew; a level below the row offers nothing,
    /// and a changed level shows the clue of the list it gives.
    #[test]
    fn a_set_enchantment_level_makes_the_offer() {
        let e = sample();
        let items = sample_items();
        let s = ItemStack::new("minecraft:diamond_sword", 1);
        let plain = enchant_offers(&e, &items, 1234, 15, Some(&s));
        let mut asked = Vec::new();
        let set = enchant_offers_leveled(&e, &items, 1234, 15, Some(&s), |row, cost| {
            asked.push((row, cost));
            [cost, 1, 10][row]
        });
        assert_eq!(asked.iter().map(|a| a.0).collect::<Vec<_>>(), [0, 1, 2]);
        assert_eq!(asked[2].1, plain.costs[2]);
        assert_eq!(set.costs, [plain.costs[0], 0, 10]);
        assert_eq!(
            (set.clues[0].clone(), set.clues[1].clone()),
            (plain.clues[0].clone(), None)
        );
        assert_eq!(set.clues[2], enchant_clue(&e, &items, 1234, 2, 10, &s));
        // An item the table can't enchant isn't asked about.
        let enchanted = sword(&[("minecraft:sharpness", 1)]);
        enchant_offers_leveled(&e, &items, 1234, 15, Some(&enchanted), |_, _| {
            panic!("asked about an item the table can't enchant")
        });
    }

    #[test]
    fn enchant_offers_are_deterministic_per_seed() {
        let e = sample();
        let items = sample_items();
        let s = ItemStack::new("minecraft:diamond_sword", 1);
        let a = enchant_offers(&e, &items, 1234, 15, Some(&s));
        let b = enchant_offers(&e, &items, 1234, 15, Some(&s));
        assert_eq!(a, b);
        assert_eq!(a.costs[2], 30, "{a:?}");
        assert!(a.costs[0] >= 1 && a.costs[0] <= a.costs[2]);
        // The clue is the first offer's random pick from the same list.
        for slot in 0..3 {
            let list = enchantment_list(&e, &items, 1234, slot, a.costs[slot], &s);
            let clue = a.clues[slot].clone().unwrap();
            assert!(list.contains(&clue));
            let applied = apply_enchantment(&e, &items, 1234, slot, a.costs[slot], &s).unwrap();
            assert_eq!(applied.enchantments(), &list[..]);
        }
        // Already enchanted items are not enchantable.
        assert_eq!(
            enchant_offers(
                &e,
                &items,
                1234,
                15,
                Some(&sword(&[("minecraft:sharpness", 1)]))
            )
            .costs,
            [0, 0, 0]
        );
        let books =
            apply_enchantment(&e, &items, 99, 2, 30, &ItemStack::new("minecraft:book", 1)).unwrap();
        assert_eq!(&*books.item, "minecraft:enchanted_book");
        assert!(!books.stored_enchantments().is_empty());
    }

    #[test]
    fn anvil_combines_and_charges_like_vanilla() {
        let e = sample();
        let items = sample_items();
        // Sharpness II + Sharpness II book = Sharpness III; book fee 1/2 -> max(1, 0) = 1 per level.
        let r = anvil_result(
            &e,
            &items,
            Some(&sword(&[("minecraft:sharpness", 2)])),
            Some(&book(&[("minecraft:sharpness", 2)])),
            None,
            "Diamond Sword",
            false,
        );
        let out = r.result.unwrap();
        assert_eq!(out.enchantment_level("minecraft:sharpness"), 3);
        assert_eq!(r.cost, 3);
        assert_eq!(out.repair_cost(), 1);
        // Incompatible only: no result.
        let r = anvil_result(
            &e,
            &items,
            Some(&sword(&[("minecraft:sharpness", 1)])),
            Some(&book(&[("minecraft:smite", 1)])),
            None,
            "Diamond Sword",
            false,
        );
        assert!(r.result.is_none());
        // Max level clamps: V + V stays V.
        let r = anvil_result(
            &e,
            &items,
            Some(&sword(&[("minecraft:sharpness", 5)])),
            Some(&book(&[("minecraft:sharpness", 5)])),
            None,
            "x",
            false,
        );
        assert_eq!(
            r.result.unwrap().enchantment_level("minecraft:sharpness"),
            5
        );
        assert_eq!(r.cost, 5);
    }

    #[test]
    fn anvil_rename_and_prior_work() {
        let e = sample();
        let items = sample_items();
        let mut s = sword(&[]);
        s.data_mut().repair_cost = 7;
        let r = anvil_result(
            &e,
            &items,
            Some(&s),
            None,
            Some("Excalibur"),
            "Diamond Sword",
            false,
        );
        assert_eq!(r.cost, 8);
        assert!(r.only_renaming);
        let out = r.result.unwrap();
        assert_eq!(out.custom_name().as_deref(), Some("Excalibur"));
        // Renaming alone does not raise the penalty.
        assert_eq!(out.repair_cost(), 7);
        // Same name: nothing to do.
        assert!(
            anvil_result(
                &e,
                &items,
                Some(&s),
                None,
                Some("Diamond Sword"),
                "Diamond Sword",
                false
            )
            .result
            .is_none()
        );
        // Too expensive at 40+ unless creative; rename-only caps at 39.
        s.data_mut().repair_cost = 63;
        let r = anvil_result(
            &e,
            &items,
            Some(&s),
            None,
            Some("A"),
            "Diamond Sword",
            false,
        );
        assert_eq!(r.cost, 39);
        assert!(r.result.is_some());
        let r = anvil_result(
            &e,
            &items,
            Some(&s),
            Some(&book(&[("minecraft:unbreaking", 1)])),
            None,
            "Diamond Sword",
            false,
        );
        assert!(r.cost >= 40 && r.result.is_none());
        assert!(
            anvil_result(
                &e,
                &items,
                Some(&s),
                Some(&book(&[("minecraft:unbreaking", 1)])),
                None,
                "Diamond Sword",
                true
            )
            .result
            .is_some()
        );
        assert_eq!(increased_repair_cost(0), 1);
        assert_eq!(increased_repair_cost(3), 7);
    }

    /// Anvil: `set(REPAIR_COST, …)` replaces a removal marker the input had,
    /// whether the result adds work or only changes the name.
    #[test]
    fn anvil_replaces_a_removed_repair_cost() {
        let e = sample();
        let items = sample_items();
        let removed = || Some(Tag::compound(Vec::<(&str, Tag)>::new()));
        let mut patched = sword(&[]);
        patched
            .data_mut()
            .set_other("!minecraft:repair_cost", removed());
        let r = anvil_result(
            &e,
            &items,
            Some(&patched),
            Some(&book(&[("minecraft:sharpness", 1)])),
            Some("Blade"),
            "Diamond Sword",
            false,
        );
        let out = r.result.expect("the enchantment and name apply");
        assert_eq!(r.cost, 2);
        assert_eq!(out.repair_cost(), 1);
        assert!(
            out.data()
                .and_then(|d| d.other("!minecraft:repair_cost"))
                .is_none()
        );
        let mut renamed = sword(&[]);
        renamed
            .data_mut()
            .set_other("!minecraft:repair_cost", removed());
        let out = anvil_result(
            &e,
            &items,
            Some(&renamed),
            None,
            Some("Blade"),
            "Diamond Sword",
            false,
        )
        .result
        .expect("the rename applies");
        assert_eq!(out.repair_cost(), 0);
        assert!(
            out.data()
                .and_then(|d| d.other("!minecraft:repair_cost"))
                .is_none()
        );
    }

    /// Enchanting: `ItemStack.isEnchantable` needs the enchantments component
    /// present; a removal patch leaves no offers.
    #[test]
    fn enchanting_needs_the_stacks_enchantments_component() {
        let e = sample();
        let items = sample_items();
        let mut removed = sword(&[]);
        removed.data_mut().set_other(
            "!minecraft:enchantments",
            Some(Tag::compound(Vec::<(&str, Tag)>::new())),
        );
        assert_eq!(
            enchant_offers(&e, &items, 1234, 15, Some(&removed)).costs,
            [0, 0, 0]
        );
        assert_ne!(
            enchant_offers(&e, &items, 1234, 15, Some(&sword(&[]))).costs,
            [0, 0, 0]
        );
    }

    /// `ItemStack.getHoverName`: a custom name, else a written book's non-blank title, else the
    /// item's name in the client's language.
    #[test]
    fn hover_name_prefers_custom_name_then_book_title_then_item_name() {
        let names = crate::creative::Names::default();
        let stack = |snbt: &str| item_data::parse_stack(snbt).unwrap().unwrap();
        let lang = crate::lang::Language::of(
            "en_us",
            [
                ("item.minecraft.diamond", "Diamond"),
                ("item.minecraft.written_book", "Written Book"),
            ]
            .map(|(k, v)| (k.to_owned(), v.to_owned())),
        );
        crate::lang::scoped(lang, || {
            let renamed = stack(
                r#"{id:"minecraft:diamond_pickaxe",count:1,components:{"minecraft:custom_name":"Hammer"}}"#,
            );
            assert_eq!(hover_name(&names, &renamed), "Hammer");
            let book = |title: &str| {
                stack(&format!(
                    r#"{{id:"minecraft:written_book",count:1,components:{{"minecraft:written_book_content":{{title:"{title}",author:"A"}}}}}}"#
                ))
            };
            assert_eq!(hover_name(&names, &book("Log")), "Log");
            assert_eq!(hover_name(&names, &book(" ")), "Written Book");
            let diamond = stack(r#"{id:"minecraft:diamond",count:1}"#);
            assert_eq!(hover_name(&names, &diamond), "Diamond");
        });
    }

    /// `ItemStack.getHoverName` falls back to `getOrDefault(ITEM_NAME, EMPTY)`,
    /// so a removed item name is the empty string, not the localized item.
    #[test]
    fn hover_name_honors_a_removed_item_name() {
        let mut sword = sword(&[]);
        sword.data_mut().set_other(
            "!minecraft:item_name",
            Some(Tag::compound(Vec::<(&str, Tag)>::new())),
        );
        assert_eq!(hover_name(&crate::creative::Names::default(), &sword), "");
    }

    /// Grindstone: transmuting an empty enchanted book to a book drops the
    /// stored-enchantments removal patch too, as `transmuteCopy` does on the
    /// book's prototype.
    #[test]
    fn grindstone_transmute_drops_a_removed_stored_enchantments() {
        let e = sample();
        let items = sample_items();
        let book = item_data::parse_stack(
            r#"{id:"minecraft:enchanted_book",count:1,components:{"!minecraft:stored_enchantments":{},"minecraft:enchantments":{"minecraft:sharpness":1}}}"#,
        )
        .unwrap()
        .unwrap();
        let out = grindstone_result(&e, &items, Some(&book), None).unwrap();
        assert_eq!(&*out.item, "minecraft:book");
        assert_eq!(out.enchantment_level("minecraft:sharpness"), 1);
        assert!(
            out.data()
                .and_then(|d| d.other("!minecraft:stored_enchantments"))
                .is_none()
        );
    }

    /// Grindstone: `set(REPAIR_COST, cost)` replaces a removal marker the
    /// input carried, whether the result keeps a curse or none.
    #[test]
    fn grindstone_replaces_a_removed_repair_cost() {
        let e = sample();
        let items = sample_items();
        let mut book = book(&[("minecraft:sharpness", 1)]);
        book.data_mut().set_other(
            "!minecraft:repair_cost",
            Some(Tag::compound(Vec::<(&str, Tag)>::new())),
        );
        let out = grindstone_result(&e, &items, Some(&book), None).unwrap();
        assert_eq!(out.repair_cost(), 0);
        assert!(
            out.data()
                .and_then(|d| d.other("!minecraft:repair_cost"))
                .is_none()
        );
    }

    /// Checks against the prepared vanilla resources when they are present.
    #[test]
    fn prepared_vanilla_data() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("items.json").exists() {
            return;
        }
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        // Station rules read item prototypes (including durability), not just tags.
        let (registry, _) = horizon_world::registry::BlockRegistry::from_facts_json_with_flags(
            std::fs::File::open(root.join("facts.json")).unwrap(),
        )
        .unwrap();
        let facts = crate::blocks::BlockFacts::load(&root, &registry).unwrap();
        let items = Items::load(&root, &registry, &facts, &PackOverlay::NONE, &defaults).unwrap();
        let data = StationData::load(
            &root,
            &items,
            &PackOverlay::NONE,
            &defaults,
            &crate::test_resources::server_load(),
        );
        let e = &data.enchantments;
        assert_eq!(e.list.len(), 43);
        assert_eq!(
            &*e.tag("minecraft:in_enchanting_table")[0],
            "minecraft:protection"
        );
        assert!(!e.compatible("minecraft:fortune", "minecraft:silk_touch"));
        assert!(e.is_curse("minecraft:binding_curse"));
        assert_eq!(e.full_name("minecraft:sharpness", 5).0, "Sharpness V");
        assert_eq!(e.enchantable("minecraft:golden_sword"), 22);
        // Brewing: water + nether wart = awkward.
        assert_eq!(data.brewing.recipes.len(), 279);
        let water = ItemStack::new("minecraft:potion", 1).with_potion("minecraft:water");
        let r = data
            .brewing
            .find(&water, &ItemStack::new("minecraft:nether_wart", 1))
            .unwrap();
        assert_eq!(r.output.potion(), Some("minecraft:awkward"));
        assert!(
            data.brewing
                .is_reagent(&ItemStack::new("minecraft:blaze_powder", 1))
        );
        // Smithing: netherite upgrade keeps enchantments.
        let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
        sword.enchant("minecraft:sharpness", 2);
        let out = data
            .recipes
            .smithing_result(
                crate::test_resources::server_loop_context(),
                e,
                Some(&ItemStack::new(
                    "minecraft:netherite_upgrade_smithing_template",
                    1,
                )),
                Some(&sword),
                Some(&ItemStack::new("minecraft:netherite_ingot", 1)),
            )
            .unwrap();
        assert_eq!(
            (&*out.item, out.enchantment_level("minecraft:sharpness")),
            ("minecraft:netherite_sword", 2)
        );
        // Anvil: a patched REPAIRABLE list repairs with an item the sword's
        // default does not accept.
        let mut repairable = ItemStack::new("minecraft:diamond_sword", 1);
        repairable.damage = 100;
        assert!(
            anvil_result(
                e,
                &items,
                Some(&repairable),
                Some(&ItemStack::new("minecraft:stick", 1)),
                None,
                "Diamond Sword",
                false,
            )
            .result
            .is_none()
        );
        repairable.data_mut().set_other(
            "minecraft:repairable",
            Some(Tag::compound([(
                "items",
                Tag::List(vec![Tag::String("minecraft:stick".into())]),
            )])),
        );
        let r = anvil_result(
            e,
            &items,
            Some(&repairable),
            Some(&ItemStack::new("minecraft:stick", 1)),
            None,
            "Diamond Sword",
            false,
        );
        assert_eq!(r.cost, 1);
        assert_eq!(r.result.unwrap().damage, 0);
        // Smithing: the addition's PROVIDES_TRIM_MATERIAL is the trim's material.
        let mut gold = ItemStack::new("minecraft:gold_ingot", 1);
        gold.data_mut().set_other(
            "minecraft:provides_trim_material",
            Some(Tag::String("minecraft:copper".into())),
        );
        let out = data
            .recipes
            .smithing_result(
                crate::test_resources::server_loop_context(),
                e,
                Some(&ItemStack::new(
                    "minecraft:coast_armor_trim_smithing_template",
                    1,
                )),
                Some(&ItemStack::new("minecraft:diamond_chestplate", 1)),
                Some(&gold),
            )
            .unwrap();
        assert_eq!(
            crate::item_data::trim::named(out.data().and_then(|d| d.trim.as_ref()).unwrap()),
            Some(("minecraft:copper", "minecraft:coast"))
        );
        // Enchanting: a patched ENCHANTABLE changes costs and selection; a
        // removed one leaves no offers at all.
        let mut low = ItemStack::new("minecraft:diamond_sword", 1);
        low.data_mut().set_other(
            "minecraft:enchantable",
            Some(Tag::compound([("value", Tag::Int(1))])),
        );
        let offers = enchant_offers(e, &items, 1234, 15, Some(&low));
        assert_eq!(offers.costs, [5, 13, 30]);
        let clue = |i: usize| offers.clues[i].as_ref().map(|(id, l)| (&**id, *l));
        assert_eq!(clue(0), Some(("minecraft:unbreaking", 1)));
        assert_eq!(clue(1), Some(("minecraft:sharpness", 2)));
        assert_eq!(clue(2), Some(("minecraft:smite", 4)));
        let applied = apply_enchantment(e, &items, 1234, 2, 30, &low).unwrap();
        assert_eq!(applied.enchantments(), &[("minecraft:smite".into(), 4)]);
        let mut removed = ItemStack::new("minecraft:diamond_sword", 1);
        removed.data_mut().set_other(
            "!minecraft:enchantable",
            Some(Tag::compound(Vec::<(&str, Tag)>::new())),
        );
        assert_eq!(
            enchant_offers(e, &items, 1234, 15, Some(&removed)).costs,
            [0, 0, 0]
        );
        assert!(
            data.recipes
                .stonecutter(
                    crate::test_resources::server_loop_context(),
                    &ItemStack::new("minecraft:stone", 1)
                )
                .len()
                >= 10
        );
        // Trades: every profession level set yields offers.
        let ctx = crate::merchant::TradeContext {
            items: &items,
            enchantments: e,
            variant: "minecraft:plains",
            origin: [0.0; 3],
            maps: None,
        };
        for p in crate::mobs::villager::profession_table()
            .entries()
            .iter()
            .filter_map(|p| p.trade_set.as_deref())
        {
            let set = data.trades.set_for(p, 1);
            let mut random = crate::merchant::sequence_random(
                12345,
                &format!(
                    "minecraft:trade_set/{}",
                    set.trim_start_matches("minecraft:")
                ),
            );
            assert!(
                !data
                    .trades
                    .offers_from_set(&ctx, &set, &mut random)
                    .is_empty(),
                "{p}"
            );
        }
    }

    #[test]
    fn grindstone_strips_non_curses() {
        let e = sample();
        let items = sample_items();
        let s = sword(&[("minecraft:sharpness", 3)]);
        let out = grindstone_result(&e, &items, Some(&s), None).unwrap();
        assert!(out.enchantments().is_empty());
        assert_eq!(out.repair_cost(), 0);
        assert!(out.extra.is_none());
        let b =
            grindstone_result(&e, &items, None, Some(&book(&[("minecraft:mending", 1)]))).unwrap();
        assert_eq!(&*b.item, "minecraft:book");
        assert!(grindstone_result(&e, &items, Some(&sword(&[])), None).is_none());
        let mut r = LegacyRandom::new(5);
        // min cost of Sharpness III = 23; ceil(23/2)=12 -> 12..23.
        let xp = grindstone_experience(&e, &mut r, Some(&s), None);
        assert!((12..24).contains(&xp));
    }
    /// Grindstone: `ItemStack.matches` compares the component patch, so an
    /// explicit `damage: 0` on an item whose prototype has no damage component
    /// keeps the two stacks from merging.
    #[test]
    fn grindstone_keeps_an_explicit_zero_damage_apart() {
        let e = sample();
        let items = sample_items();
        let zero = item_data::parse_stack(
            r#"{id:"minecraft:book",count:1,components:{"minecraft:damage":0,"minecraft:enchantments":{"minecraft:sharpness":1}}}"#,
        )
        .unwrap()
        .unwrap();
        let plain = item_data::parse_stack(
            r#"{id:"minecraft:book",count:1,components:{"minecraft:enchantments":{"minecraft:sharpness":1}}}"#,
        )
        .unwrap()
        .unwrap();
        assert!(grindstone_result(&e, &items, Some(&zero), Some(&plain)).is_none());
        let out = grindstone_result(&e, &items, Some(&zero), Some(&zero.clone())).unwrap();
        assert_eq!(out.count, 2);
        assert!(!out.has_any_enchantments());
    }

    /// `ItemStack.isDamageableItem` asks for the DAMAGE component too, so a
    /// stick patched with MAX_DAMAGE alone is no grindstone or anvil repair.
    #[test]
    fn grindstone_needs_the_damage_component() {
        let root =
            std::env::temp_dir().join(format!("horizon-grind-damage-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("data/minecraft/tags/item/enchantable")).unwrap();
        fs::write(
            root.join("items.json"),
            r#"{"minecraft:stick":{"minecraft:max_stack_size":64},
                "minecraft:diamond_sword":{"minecraft:max_stack_size":1,"minecraft:max_damage":1561,"minecraft:damage":0}}"#,
        )
        .unwrap();
        // The anvil only asks `isDamageableItem` for the combine once
        // `canEnchant` passed, so the stick joins vanilla's sharp-weapon tag.
        fs::write(
            root.join("data/minecraft/tags/item/enchantable/sharp_weapon.json"),
            r#"{"values":["minecraft:stick"]}"#,
        )
        .unwrap();
        let items = Items::catalog(
            &root,
            &PackOverlay::NONE,
            &horizon_world::item_defaults::read(&root).unwrap(),
        )
        .unwrap();
        let e = sample();
        let stick = |components: &str| {
            let snbt = format!(r#"{{id:"minecraft:stick",count:1,components:{components}}}"#);
            item_data::parse_stack(&snbt).unwrap().unwrap()
        };
        let sharp = |mut s: ItemStack| {
            s.enchant("minecraft:sharpness", 1);
            s
        };
        // `/give @s minecraft:stick[minecraft:max_damage=10,minecraft:max_stack_size=1]`
        let patched = stick(r#"{"minecraft:max_damage":10,"minecraft:max_stack_size":1}"#);
        assert_eq!(patched.damage, 0);
        assert!(!grindstone_accepts(&items, &patched));
        // Two of them are not one "repaired" stick.
        assert!(grindstone_result(&e, &items, Some(&patched), Some(&patched)).is_none());
        // The anvil's combine gate refuses a non-damageable stack
        // (`AnvilMenu.java:151`), so its enchantments do not merge either.
        let (a, b) = (sharp(patched.clone()), sharp(patched.clone()));
        assert!(
            anvil_result(&e, &items, Some(&a), Some(&b), None, "", false)
                .result
                .is_none()
        );
        // The same patch with an explicit `minecraft:damage=0` has the component.
        let explicit = stick(
            r#"{"minecraft:max_damage":10,"minecraft:max_stack_size":1,"minecraft:damage":0}"#,
        );
        assert!(grindstone_accepts(&items, &explicit));
        // So the grindstone merges two into one and the anvil combines them.
        let merged = grindstone_result(&e, &items, Some(&explicit), Some(&explicit)).unwrap();
        assert_eq!((merged.count, merged.damage), (1, 0));
        let (a, b) = (sharp(explicit.clone()), sharp(explicit.clone()));
        assert_eq!(
            anvil_result(&e, &items, Some(&a), Some(&b), None, "", false)
                .result
                .map(|s| s.enchantment_level("minecraft:sharpness")),
            Some(2)
        );
        // A sword's own durability supplies DAMAGE; removing it refuses it.
        let sword = item_data::parse_stack(r#"{id:"minecraft:diamond_sword",count:1}"#)
            .unwrap()
            .unwrap();
        assert!(grindstone_accepts(&items, &sword));
        let mut removed = sword.clone();
        removed.data_mut().set_other(
            "!minecraft:damage",
            Some(Tag::compound(Vec::<(&str, Tag)>::new())),
        );
        assert!(!grindstone_accepts(&items, &removed));
        // An enchanted book is accepted for its enchantments, not its damage.
        let mut book = ItemStack::new("minecraft:enchanted_book", 1);
        book.enchant("minecraft:sharpness", 1);
        assert!(grindstone_accepts(&items, &book));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn stonecutting_component_result_is_retained() {
        let items = Items::for_tests(Default::default());
        let json = serde_json::json!({"type":"minecraft:stonecutting","ingredient":"minecraft:stone","result":{"id":"minecraft:diamond","count":2,"components":{"minecraft:damage":7}}});
        let rule = ParsedStationRecipe::parse("test:cut".into(), &json, &items)
            .unwrap()
            .rule();
        let crate::crafting::RecipeRule::Stonecutting { result, .. } = rule else {
            panic!("wrong rule")
        };
        assert_eq!(result.item.as_ref(), "minecraft:diamond");
        assert_eq!(result.count, 2);
        assert_eq!(result.damage, 7);
    }

    #[test]
    fn smithing_transform_omitted_slots_match_empty() {
        let items = Items::for_tests(Default::default());
        let json = serde_json::json!({"type":"minecraft:smithing_transform","base":"minecraft:stone","result":"minecraft:diamond"});
        let parsed = ParsedStationRecipe::parse("test:smith".into(), &json, &items).unwrap();
        assert!(matches!(
            parsed.rule(),
            crate::crafting::RecipeRule::SmithingTransform {
                template: None,
                addition: None,
                ..
            }
        ));
        let ParsedStationRecipe::Smithing(recipe) = parsed else {
            panic!("wrong rule")
        };
        let stations = StationRecipes {
            smithing: vec![recipe],
            ..Default::default()
        };
        let base = ItemStack::new("minecraft:stone", 1);
        let ench = Enchantments::default();
        let ctx = crate::test_resources::server_loop_context();
        assert!(
            stations
                .smithing_result(ctx, &ench, None, Some(&base), None)
                .is_some()
        );
        assert!(
            stations
                .smithing_result(ctx, &ench, Some(&base), Some(&base), None)
                .is_none()
        );
    }

    #[test]
    fn smithing_rejects_invalid_assembled_stack() {
        let items = Items::for_tests(Default::default());
        let json = serde_json::json!({"type":"minecraft:smithing_transform","base":"minecraft:diamond_sword","result":{"id":"minecraft:netherite_sword","count":2}});
        let ParsedStationRecipe::Smithing(recipe) =
            ParsedStationRecipe::parse("test:smith".into(), &json, &items).unwrap()
        else {
            panic!("wrong rule")
        };
        let stations = StationRecipes {
            smithing: vec![recipe],
            item_limits: [(Arc::from("minecraft:netherite_sword"), (1, 2031))]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let base = ItemStack::new("minecraft:diamond_sword", 1);
        assert!(
            stations
                .smithing_result(
                    crate::test_resources::server_loop_context(),
                    &Enchantments::default(),
                    None,
                    Some(&base),
                    None
                )
                .is_none()
        );
    }

    #[test]
    fn smithing_inline_trim_pattern_is_retained() {
        let items = Items::for_tests(Default::default());
        let json = serde_json::json!({"type":"minecraft:smithing_trim","template":"minecraft:stick","base":"minecraft:stone","addition":"minecraft:iron_ingot","pattern":{"asset_id":"example:wave","description":{"translate":"trim.example.wave"},"decal":true}});
        let rule = ParsedStationRecipe::parse("test:trim".into(), &json, &items)
            .unwrap()
            .rule();
        let crate::crafting::RecipeRule::SmithingTrim {
            pattern:
                crate::crafting::RecipeTrimPattern::Inline {
                    asset_id,
                    description,
                    decal,
                },
            ..
        } = rule
        else {
            panic!("wrong rule")
        };
        assert_eq!(asset_id, "example:wave");
        assert_eq!(
            description.get("translate"),
            Some(&horizon_core::nbt::Tag::String("trim.example.wave".into()))
        );
        assert!(decal);
    }

    #[test]
    fn smithing_transform_overlay_matches_optional_inputs_and_takes_template_result() {
        use crate::{banner::Banners, brewing::Brewing, crafting::Recipes, creative::Names, menu};
        let root = std::env::temp_dir().join(format!("horizon-smithing-{}", std::process::id()));
        fs::create_dir_all(root.join("data/minecraft/recipe")).unwrap();
        fs::write(
            root.join("items.json"),
            r#"{
            "minecraft:stick":{"minecraft:max_stack_size":64},
            "minecraft:diamond":{"minecraft:max_stack_size":64},
            "minecraft:emerald":{"minecraft:max_stack_size":64},
            "minecraft:diamond_sword":{"minecraft:max_stack_size":1,"minecraft:max_damage":1561}
        }"#,
        )
        .unwrap();
        let context = crate::test_resources::server_load();
        let ctx = crate::test_resources::server_loop_context();
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let items = Items::catalog(&root, &PackOverlay::NONE, &defaults).unwrap();
        let recipe = serde_json::json!({"type":"minecraft:smithing_transform",
            "base":"minecraft:stick", "addition":"minecraft:diamond",
            "result":{"id":"minecraft:emerald","count":3,
                "components":{"minecraft:custom_name":"Upgraded","minecraft:repair_cost":0}}});
        let files = vec![(
            "recipe/test:upgrade".to_owned(),
            vec![serde_json::to_vec(&recipe).unwrap()],
        )];
        let overlay = PackOverlay::new(&files);
        let stations = StationRecipes::load(&root, &items, &overlay, &context);
        let e = sample();
        let mut base = ItemStack::new("minecraft:stick", 2);
        base.data_mut()
            .set_loot_name("minecraft:custom_name", &serde_json::json!("Original"));
        base.data_mut().repair_cost = 7;
        base.data_mut().set_loot_component(
            "minecraft:custom_data",
            &Some(horizon_core::json_nbt::json_to_tag(&serde_json::json!({"kept":1})).unwrap()),
        );
        let addition = ItemStack::new("minecraft:diamond", 2);
        let result = stations
            .smithing_result(ctx, &e, None, Some(&base), Some(&addition))
            .expect("omitted template accepts an empty slot");
        assert_eq!(
            (&*result.item, result.count, result.repair_cost()),
            ("minecraft:emerald", 3, 0)
        );
        assert_eq!(
            result.data().unwrap().custom_name,
            Some(horizon_core::nbt::Tag::String("Upgraded".into()))
        );
        assert!(
            result
                .data()
                .unwrap()
                .other("minecraft:custom_data")
                .is_some()
        );
        assert!(
            stations
                .smithing_result(ctx, &e, Some(&addition), Some(&base), Some(&addition))
                .is_none()
        );
        assert!(
            stations
                .smithing_result(ctx, &e, None, Some(&base), None)
                .is_none()
        );
        let recipes = Recipes::load(&root, &items, &PackOverlay::NONE, &context).unwrap();
        let brewing = Brewing::load(&root, &items, &PackOverlay::NONE);
        let banners = Banners::load(&root, &defaults);
        let names = Names::default();
        let env = menu::Env {
            crafting_remainder: None,
            player: None,
            player_entity_id: None,
            loop_context: ctx,
            items: &items,
            recipes: &recipes,
            brewing: &brewing,
            banners: &banners,
            station_recipes: &stations,
            enchantments: &e,
            names: &names,
            trade: None,
            maps: None,
            limited_crafting: false,
            can_equip: None,
            known_recipes: None,
            slot_rules: None,
        };
        let mut menu = menu::Menu::new(menu::Kind::Smithing);
        menu.set_slot(&env, 1, Some(base));
        menu.set_slot(&env, 2, Some(addition));
        assert_eq!(menu.slot(3), Some(&result));
        menu.click(&env, 3, 0, menu::Input::Pickup).unwrap();
        assert_eq!(menu.carried.as_ref(), Some(&result));
        assert!(menu.slot(0).is_none());
        assert_eq!(menu.slot(1).unwrap().count, 1);
        assert_eq!(menu.slot(2).unwrap().count, 1);
        assert_eq!(menu.crafted[0].0, "test:upgrade");
        for result in [
            serde_json::json!({"id":"minecraft:emerald","count":0}),
            serde_json::json!({"id":"minecraft:emerald","count":100}),
            serde_json::json!({"id":"minecraft:diamond_sword","count":2}),
            serde_json::json!({"id":"minecraft:emerald","components":{"minecraft:max_damage":10}}),
        ] {
            let mut invalid = recipe.clone();
            invalid["result"] = result;
            let files = vec![(
                "recipe/test:invalid".to_owned(),
                vec![serde_json::to_vec(&invalid).unwrap()],
            )];
            let stations = StationRecipes::load(&root, &items, &PackOverlay::new(&files), &context);
            assert!(
                stations
                    .smithing_result(ctx, &e, None, menu.slot(1), menu.slot(2))
                    .is_none(),
                "{invalid}"
            );
        }
        for key in ["template", "base", "addition"] {
            let mut invalid = recipe.clone();
            invalid[key] = Value::Null;
            let files = vec![(
                "recipe/test:invalid".to_owned(),
                vec![serde_json::to_vec(&invalid).unwrap()],
            )];
            let stations = StationRecipes::load(&root, &items, &PackOverlay::new(&files), &context);
            assert!(
                stations.smithing.is_empty(),
                "malformed {key} must not become optional"
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn smithing_trim_reads_an_inline_pattern() {
        let root = std::env::temp_dir().join(format!("horizon-trim-{}", std::process::id()));
        fs::create_dir_all(root.join("data/minecraft/recipe")).unwrap();
        fs::write(
            root.join("items.json"),
            r#"{"minecraft:gold_ingot":{"minecraft:max_stack_size":64,"minecraft:provides_trim_material":"minecraft:gold"}}"#,
        )
        .unwrap();
        fs::write(
            root.join("data/minecraft/recipe/coast.json"),
            serde_json::json!({
                "type": "minecraft:smithing_trim",
                "template": "minecraft:coast_armor_trim_smithing_template",
                "base": "minecraft:diamond_chestplate",
                "addition": "minecraft:gold_ingot",
                "pattern": {
                    "asset_id": "minecraft:coast",
                    "description": { "text": "Coast" },
                    "decal": false
                }
            })
            .to_string(),
        )
        .unwrap();
        let items = Items::tags_only(
            &root,
            &PackOverlay::NONE,
            &horizon_world::item_defaults::read(&root).unwrap(),
        )
        .unwrap();
        let recipes = StationRecipes::load(
            &root,
            &items,
            &PackOverlay::NONE,
            &crate::test_resources::server_load(),
        );
        assert_eq!(recipes.smithing.len(), 1);
        let out = recipes
            .smithing_result(
                crate::test_resources::server_loop_context(),
                &items.enchantments,
                Some(&ItemStack::new(
                    "minecraft:coast_armor_trim_smithing_template",
                    1,
                )),
                Some(&ItemStack::new("minecraft:diamond_chestplate", 1)),
                Some(&ItemStack::new("minecraft:gold_ingot", 1)),
            )
            .expect("the inline pattern applies");
        let trim = out.data().and_then(|d| d.trim.as_ref()).unwrap();
        assert_eq!(
            trim.1,
            crate::item_data::trim::Holder::Direct(crate::item_data::trim::Pattern {
                asset_id: "minecraft:coast".into(),
                description: Tag::String("Coast".into()),
                decal: false,
            })
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn stonecutter_result_keeps_its_components() {
        let root = std::env::temp_dir().join(format!(
            "horizon-stonecutter-components-{}",
            std::process::id()
        ));
        let dir = root.join("data/minecraft/recipe");
        fs::create_dir_all(&dir).unwrap();
        fs::write(root.join("items.json"), r#"{"minecraft:stone":{}}"#).unwrap();
        fs::write(
            dir.join("cut.json"),
            r#"{"type":"minecraft:stonecutting","ingredient":"minecraft:stone","result":{"id":"minecraft:stone","components":{"minecraft:custom_name":"Cut Stone"}}}"#,
        )
        .unwrap();
        let recipes = StationRecipes::load(
            &root,
            &sample_items(),
            &PackOverlay::NONE,
            &crate::test_resources::server_load(),
        );
        let _ = fs::remove_dir_all(&root);
        let listed = recipes.stonecutter(
            crate::test_resources::server_loop_context(),
            &ItemStack::new("minecraft:stone", 1),
        );
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].result.custom_name().as_deref(), Some("Cut Stone"));
    }

    /// A world pack's stonecutting recipe takes its place in registration
    /// order (path, then namespace), not a place after the game's own.
    #[test]
    fn stonecutter_lists_recipes_in_registration_order() {
        let root =
            std::env::temp_dir().join(format!("horizon-stonecutter-order-{}", std::process::id()));
        let dir = root.join("data/minecraft/recipe");
        fs::create_dir_all(&dir).unwrap();
        let cut = |result: &str| {
            format!(
                r#"{{"type":"minecraft:stonecutting","ingredient":"minecraft:stone","result":{{"id":"{result}"}}}}"#
            )
        };
        fs::write(dir.join("z_cut.json"), cut("minecraft:stone_slab")).unwrap();
        fs::write(dir.join("b_cut.json"), cut("minecraft:stone_bricks")).unwrap();
        let packs = [(
            "recipe/pack:a_cut".to_owned(),
            vec![cut("minecraft:stone_stairs").into_bytes()],
        )];
        let recipes = StationRecipes::load(
            &root,
            &sample_items(),
            &PackOverlay::new(&packs),
            &crate::test_resources::server_load(),
        );
        let _ = fs::remove_dir_all(&root);
        let listed: Vec<_> = recipes
            .stonecutter(
                crate::test_resources::server_loop_context(),
                &ItemStack::new("minecraft:stone", 1),
            )
            .iter()
            .map(|r| (r.id.as_str(), &*r.result.item))
            .collect();
        assert_eq!(
            listed,
            [
                ("pack:a_cut", "minecraft:stone_stairs"),
                ("minecraft:b_cut", "minecraft:stone_bricks"),
                ("minecraft:z_cut", "minecraft:stone_slab"),
            ]
        );
    }
}
