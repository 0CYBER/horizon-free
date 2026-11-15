//! `ServerPlaceRecipe`: what the server does with a recipe book click
//! (`ServerboundPlaceRecipePacket`) in the inventory, crafting table and
//! furnace menus, with `StackedContents`' exact matching of items to
//! ingredients (`RecipePicker`). `tools/test_place_recipe_parity.py`
//! compares every placement with vanilla's.
//!
//! Vanilla keeps the available items in a `Reference2IntOpenHashMap` keyed by
//! item holders, whose iteration order follows their identity hash codes: it
//! differs between runs of the game, and decides which of several fitting
//! items fills an ingredient. `Stacked` keeps an explicit order: the order
//! items were first counted, or one given (the harness passes the oracle's).
use super::{Env, GRID, Kind, MAIN, Menu, PLAYER, RESULT, split, tidy};
use crate::{
    crafting::{Ingredient, Placement},
    items::ItemStack,
};
use std::{collections::HashMap, sync::Arc};
#[path = "place/predicate.rs"]
mod predicate;
pub use predicate::can_craft_matching;

/// `StackedContents<Holder<Item>>`: item counts in an iteration order.
#[derive(Clone, Debug, Default)]
pub struct Stacked {
    keys: Vec<Arc<str>>,
    amounts: Vec<i32>,
    index: HashMap<Arc<str>, usize>,
}

/// `Inventory.isUsableForCrafting`: not damaged, enchanted or renamed.
pub fn usable_for_crafting(stack: &ItemStack) -> bool {
    stack.damage == 0 && stack.enchantments().is_empty() && stack.custom_name().is_none()
}

impl Stacked {
    /// `StackedContents.account`.
    pub fn account(&mut self, item: &Arc<str>, count: i32) {
        match self.index.get(item) {
            Some(&i) => self.amounts[i] += count,
            None => {
                self.index.insert(item.clone(), self.keys.len());
                self.keys.push(item.clone());
                self.amounts.push(count);
            }
        }
    }
    /// `StackedItemContents.accountStack(stack, maxCount)`.
    pub fn account_stack(&mut self, stack: &ItemStack, max: u32) {
        if stack.count > 0 {
            self.account(&stack.item, stack.count.min(max) as i32);
        }
    }
    /// `accountSimpleStack`.
    pub fn account_simple(&mut self, stack: &ItemStack, max: u32) {
        if usable_for_crafting(stack) {
            self.account_stack(stack, max);
        }
    }
    /// Reorders the keys: the given items first, in that order (vanilla's hash
    /// map order for the same items), then the rest as they were.
    pub fn set_order(&mut self, order: &[String]) {
        let mut keys: Vec<Arc<str>> = order
            .iter()
            .filter_map(|o| self.index.get_key_value(o.as_str()).map(|(k, _)| k.clone()))
            .collect();
        for k in &self.keys {
            if !keys.contains(k) {
                keys.push(k.clone());
            }
        }
        let amounts = keys.iter().map(|k| self.amounts[self.index[k]]).collect();
        self.index = keys
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, k)| (k, i))
            .collect();
        self.keys = keys;
        self.amounts = amounts;
    }
    pub fn keys(&self) -> &[Arc<str>] {
        &self.keys
    }
    /// `canCraft(ingredients, null)` without touching the counts (the recipe
    /// book's craftable test; the answer does not depend on the order; no
    /// ingredients at all can always be found).
    pub fn can_craft(&self, ingredients: &[&Ingredient]) -> bool {
        self.clone().try_pick(ingredients, 1, None)
    }
    fn pick(
        &mut self,
        ingredients: &[&Ingredient],
        amount: i32,
        all: bool,
        output: Option<&mut Vec<Arc<str>>>,
    ) -> i32 {
        let keys = &self.keys;
        let mut picker = Picker::new(&mut self.amounts, ingredients.len(), |key, ingredient| {
            if ingredients[ingredient].items.contains(&*keys[key]) {
                i32::MAX
            } else {
                0
            }
        });
        let mut selected = Vec::new();
        let target = output.as_ref().map(|_| &mut selected);
        let result = if all {
            picker.try_pick_all(amount, target)
        } else {
            i32::from(picker.try_pick(amount, target))
        };
        if let Some(output) = output {
            output.extend(selected.into_iter().map(|index| keys[index].clone()));
        }
        result
    }
    /// `tryPick` (`canCraft(recipe, amount, output)`).
    /// When `output` is `None` this is a non-destructive check: `self` is not
    /// mutated. When `output` is `Some`, the matched items are deducted from
    /// `self` and recorded in `output`.
    pub fn try_pick(
        &mut self,
        ingredients: &[&Ingredient],
        amount: i32,
        output: Option<&mut Vec<Arc<str>>>,
    ) -> bool {
        self.pick(ingredients, amount, false, output) != 0
    }
    /// `tryPickAll` (`getBiggestCraftableStack`).
    pub fn try_pick_all(
        &mut self,
        ingredients: &[&Ingredient],
        max_size: i32,
        output: Option<&mut Vec<Arc<str>>>,
    ) -> i32 {
        self.pick(ingredients, max_size, true, output)
    }
}

#[path = "place/picker.rs"]
pub(super) mod picker;
use picker::Picker;
#[path = "place/custom.rs"]
mod custom;
#[path = "place/custom_access.rs"]
mod custom_access;
#[path = "place/custom_plan.rs"]
pub mod custom_plan;

/// `RecipeBookMenu.PostPlaceAction`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostPlace {
    Nothing,
    /// The client shows the recipe as a ghost (`ClientboundPlaceGhostRecipePacket`).
    Ghost,
}

/// Why a viewer transfer could not move a complete set of ingredients.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferError {
    Missing,
    NoRoom,
    Unavailable,
}

/// A viewer transfer worked out without moving anything (`Menu::check_transfer`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferCheck {
    pub result: Result<(), TransferError>,
    /// The recipe's cells the items can't fill, when `result` is `Missing`.
    pub missing: Vec<usize>,
}

/// The menu slots a placement works on.
struct Grid {
    width: usize,
    height: usize,
    /// `inputGridSlots` and `slotsToClear` as menu slot indices.
    inputs: Vec<usize>,
    clear: Vec<usize>,
}

impl Menu {
    /// `ItemStack.onCraftedBy`'s count for `n` of `stack` (AIR for an emptied quick move).
    pub(super) fn crafted_by(&mut self, stack: &ItemStack, n: i32) {
        let empty = stack.count == 0;
        let item = if empty { "minecraft:air" } else { &*stack.item };
        self.crafted_counts.push((item.into(), n));
    }

    /// `on_take` for a quick-moved slot. A merchant, stonecutter, smithing or furnace result
    /// reports the stack its move left behind (`StonecutterMenu`/`ItemCombinerMenu.quickMoveStack`;
    /// their `onQuickCraft` already saw the original). A
    /// crafting result reports the copy it takes (which Item.onCraftedBy may still change)
    /// minus what stayed behind: `ResultSlot.onQuickCraft` counts only the items that moved.
    pub(super) fn take_after_quick_move(
        &mut self,
        env: &Env,
        index: usize,
        clicked: &mut ItemStack,
        stack: &mut ItemStack,
    ) {
        let role = self.slots[index].role;
        let rest = matches!(
            role,
            super::Role::MerchantResult | super::Role::Output | super::Role::StoneResult
        ) || (role == super::Role::CombineResult && self.kind == Kind::Smithing);
        if rest {
            return self.on_take(env, index, stack);
        }
        let left = if self.slots[index].role == super::Role::CraftResult {
            stack.count
        } else {
            0
        };
        clicked.count -= left;
        self.on_take(env, index, clicked);
        clicked.count += left;
    }

    fn place_grid(&self) -> Option<Grid> {
        match self.state.kind {
            Kind::Inventory => Some(Grid {
                width: 2,
                height: 2,
                inputs: (1..5).collect(),
                clear: (1..5).collect(),
            }),
            Kind::Crafting => Some(Grid {
                width: 3,
                height: 3,
                inputs: (1..10).collect(),
                clear: (1..10).collect(),
            }),
            Kind::Furnace(_) => Some(Grid {
                width: 1,
                height: 1,
                inputs: vec![0],
                clear: vec![0, 2],
            }),
            // A mod menu type's crafting grid places as a crafting table's.
            Kind::Mod(t) => t.crafting.map(|c| Grid {
                width: c.size,
                height: c.size,
                inputs: c.grid_slots().collect(),
                clear: c.grid_slots().collect(),
            }),
            _ => None,
        }
    }

    /// `Inventory.fillStackedContents` then the menu's
    /// `fillCraftSlotsStackedContents`.
    pub fn stacked_contents(&self, env: &Env) -> Stacked {
        self.stacked_for(env, false)
    }

    /// The counted items; a viewer's (`viewer`) counts every stack, worn,
    /// enchanted and renamed ones too, as crafting by hand takes them.
    fn stacked_for(&self, env: &Env, viewer: bool) -> Stacked {
        let mut stacked = Stacked::default();
        let account = |stacked: &mut Stacked, stack: &ItemStack| {
            if viewer {
                stacked.account_stack(stack, Self::max_stack(env, stack));
            } else {
                stacked.account_simple(stack, Self::max_stack(env, stack));
            }
        };
        for stack in self.state.containers[PLAYER][..36].iter().flatten() {
            account(&mut stacked, stack);
        }
        // The mods' extra sources, after the inventory.
        for stack in self.state.sources.iter().flatten().flatten() {
            account(&mut stacked, stack);
        }
        match self.state.kind {
            // A furnace's SimpleContainer counts all three slots, unfiltered.
            Kind::Furnace(_) => {
                for stack in self.state.containers[MAIN].iter().flatten() {
                    stacked.account_stack(stack, Self::max_stack(env, stack));
                }
            }
            // A crafter's `fillStackedContents`: its 3x3 slots are the menu's
            // MAIN container (`CrafterMenu.addSlots`), not a crafting GRID.
            Kind::Crafter | Kind::CrafterBlock => {
                for stack in self.containers[MAIN].iter().flatten() {
                    stacked.account_simple(stack, Self::max_stack(env, stack));
                }
            }
            _ => {
                for stack in self.state.containers[GRID].iter().flatten() {
                    account(&mut stacked, stack);
                }
            }
        }
        stacked
    }

    /// `RecipeBookMenu.handlePlacement` for the recipe `id`
    /// (`ServerPlaceRecipe.placeRecipe`). `allow_drop` is the player being in
    /// creative; `order` the iteration order of the counted items, if known.
    pub fn place_recipe(
        &mut self,
        env: &Env,
        id: &str,
        use_max: bool,
        allow_drop: bool,
        order: Option<&[String]>,
    ) -> PostPlace {
        self.touch();
        let Some(grid) = self.place_grid() else {
            return PostPlace::Nothing;
        };
        let Some(placement) = env.recipes.placement(id) else {
            return PostPlace::Nothing;
        };
        if placement.impossible() {
            return PostPlace::Nothing;
        }
        if !allow_drop && !self.test_clear_grid(env, &grid) {
            return PostPlace::Nothing;
        }
        let mut stacked = self.stacked_contents(env);
        if let Some(order) = order {
            stacked.set_order(order);
        }
        let ingredients: Vec<&Ingredient> = placement.ingredients.iter().collect();
        let post = if stacked.try_pick(&ingredients, 1, None) {
            let matches = self.recipe_in_grid(env, id);
            self.place(
                env,
                &grid,
                &placement,
                &mut stacked,
                use_max,
                matches,
                false,
            );
            PostPlace::Nothing
        } else {
            self.clear_grid(env, &grid);
            PostPlace::Ghost
        };
        // AbstractCraftingMenu.finishPlacingRecipe: the result follows the grid.
        self.recompute(env);
        post
    }

    /// Move one set of ingredients from the player's real inventory and grid,
    /// or with `all` as many as they allow (`useMaxItems`: the largest count
    /// every grid slot can take, alternatives shared out by
    /// `StackedItemContents`, capped at the ingredients' stack size).
    /// The viewer leaves both alone on failure, including in creative mode.
    /// Unlike the recipe book it takes worn, enchanted and renamed items, as
    /// JEI's transfer and crafting by hand do.
    pub fn transfer_recipe(&mut self, env: &Env, id: &str, all: bool) -> Result<(), TransferError> {
        self.touch();
        let (grid, placement, mut stacked) = self.transfer_match(env, id)?;
        self.transfer_place(env, id, &grid, &placement, &mut stacked, all)?;
        self.recompute(env);
        Ok(())
    }

    /// What `transfer_recipe(env, id, false)` would answer, worked out on a
    /// copy of the menu: the same matching, then the same placement on the
    /// copy for the room it needs. With `Missing`, the recipe's cells the
    /// counted items can't fill. `self` is only read, so nothing moves, no
    /// source changes and nothing drops.
    pub fn check_transfer(&self, env: &Env, id: &str) -> TransferCheck {
        let result = match self.transfer_match(env, id) {
            Ok((grid, placement, mut stacked)) => {
                let mut copy = self.clone();
                copy.transfer_place(env, id, &grid, &placement, &mut stacked, false)
            }
            Err(error) => Err(error),
        };
        let missing = match result {
            Err(TransferError::Missing) => self.missing_cells(env, id),
            _ => Vec::new(),
        };
        TransferCheck { result, missing }
    }

    /// A viewer transfer's matching, shared by `transfer_recipe` and
    /// `check_transfer`: whether the recipe goes into this menu's grid, then
    /// `stacked_contents` (the inventory, the mods' sources and the grid) and
    /// one set picked from them. Reads the menu only.
    fn transfer_match(
        &self,
        env: &Env,
        id: &str,
    ) -> Result<(Grid, Placement, Stacked), TransferError> {
        if !(matches!(self.state.kind, Kind::Inventory | Kind::Crafting)
            || self.mod_crafting().is_some())
            || self.state.carried.is_some()
        {
            return Err(TransferError::Unavailable);
        }
        let grid = self.place_grid().ok_or(TransferError::Unavailable)?;
        let placement = env
            .recipes
            .placement(id)
            .ok_or(TransferError::Unavailable)?;
        if placement.impossible()
            || placement.ingredients.len() > grid.inputs.len()
            || placement
                .shaped
                .is_some_and(|(w, h)| w > grid.width || h > grid.height)
        {
            return Err(TransferError::Unavailable);
        }
        let mut stacked = self.stacked_for(env, true);
        let ingredients: Vec<_> = placement.ingredients.iter().collect();
        if !stacked.try_pick(&ingredients, 1, None) {
            return Err(TransferError::Missing);
        }
        Ok((grid, placement, stacked))
    }

    /// The placement of a matched transfer (`transfer_match`). The vanilla
    /// placement can return leftover grid items as drops: a viewer transfer
    /// keeps the original slots if it cannot move every input without
    /// dropping anything.
    fn transfer_place(
        &mut self,
        env: &Env,
        id: &str,
        grid: &Grid,
        placement: &Placement,
        stacked: &mut Stacked,
        all: bool,
    ) -> Result<(), TransferError> {
        self.touch();
        let previous = self.state.containers.clone();
        let sources = self.state.sources.clone();
        let (drops, dropped) = (self.state.drops.len(), self.state.dropped.len());
        self.place(env, grid, placement, stacked, all, false, true);
        let no_room = self.state.dropped.len() != dropped;
        if no_room || !self.recipe_in_grid(env, id) {
            self.state.containers = previous;
            self.state.sources = sources;
            self.state.drops.truncate(drops);
            self.state.dropped.truncate(dropped);
            return Err(if no_room {
                TransferError::NoRoom
            } else {
                TransferError::Unavailable
            });
        }
        Ok(())
    }

    /// The cells of recipe `id` the counted items can't fill
    /// (`check_transfer`): its ingredients taken one at a time in order, each
    /// kept when the items still cover it with the ones kept before, as
    /// `StackedItemContents` matches them, so alternatives and shared items
    /// count as the transfer counts them. Indices into the recipe's `slots`
    /// (width by height for a shaped recipe, else its ingredients in order).
    fn missing_cells(&self, env: &Env, id: &str) -> Vec<usize> {
        let Some(placement) = env.recipes.placement(id) else {
            return Vec::new();
        };
        let mut stacked = self.stacked_contents(env);
        let mut kept: Vec<&Ingredient> = Vec::new();
        let mut lacking = Vec::new();
        for (k, ingredient) in placement.ingredients.iter().enumerate() {
            kept.push(ingredient);
            if !stacked.try_pick(&kept, 1, None) {
                kept.pop();
                lacking.push(k as i32);
            }
        }
        (placement.slots.iter().enumerate())
            .filter(|(_, k)| lacking.contains(k))
            .map(|(cell, _)| cell)
            .collect()
    }

    /// `recipeMatches`: the recipe already fills the grid.
    fn recipe_in_grid(&self, env: &Env, id: &str) -> bool {
        match self.state.kind {
            Kind::Furnace(_) => {
                let name = id.trim_start_matches("minecraft:");
                match (
                    env.recipes.cooking.iter().find(|c| c.id == name),
                    self.state.containers[MAIN][0].as_ref(),
                ) {
                    (Some(c), Some(input)) => c.ingredient.test_in(env.loop_context, input),
                    _ => false,
                }
            }
            _ => env.recipes.recipe_matches(
                env.loop_context,
                id,
                &self.state.containers[GRID],
                self.state.grid_size,
            ),
        }
    }

    fn place(
        &mut self,
        env: &Env,
        grid: &Grid,
        placement: &Placement,
        stacked: &mut Stacked,
        use_max: bool,
        matches: bool,
        viewer: bool,
    ) {
        self.touch();
        let ingredients: Vec<&Ingredient> = placement.ingredients.iter().collect();
        let biggest = stacked.try_pick_all(&ingredients, i32::MAX, None);
        if matches {
            for &s in &grid.inputs {
                if let Some(stack) = self.slot(s)
                    && (biggest.min(Self::max_stack(env, stack) as i32) as i64)
                        < i64::from(stack.count) + 1
                {
                    return;
                }
            }
        }
        // calculateAmountToCraft
        let Some(amount) = self.recipe_place_amount(&grid.inputs, use_max, matches, biggest) else {
            return;
        };
        let mut used = Vec::new();
        if !stacked.try_pick(&ingredients, amount, Some(&mut used)) {
            return;
        }
        // clampToMaxStackSize
        let adjusted = used.iter().fold(amount, |n, item| {
            n.min(env.items.get(item).map_or(1, |i| i.max_stack) as i32)
        });
        if adjusted != amount {
            used.clear();
            if !stacked.try_pick(&ingredients, adjusted, Some(&mut used)) {
                return;
            }
        }
        self.clear_grid(env, grid);
        let (w, h) = placement.shaped.unwrap_or((grid.width, grid.height));
        let slots: Vec<i32> = placement.slots.clone();
        for (ingredient, index) in
            crate::recipe_book::place_cells(grid.width, grid.height, w, h, &slots)
        {
            if ingredient == -1 {
                continue;
            }
            let target = grid.inputs[index];
            let item = used[ingredient as usize].clone();
            let mut remaining = adjusted;
            while remaining > 0 {
                remaining = self.move_item_to_grid(target, &item, remaining, viewer);
                if remaining == -1 {
                    break;
                }
            }
        }
    }

    /// `moveItemToGrid`: the rest still to move, or -1 when nothing matches.
    /// The inventory first, then the mods' extra sources in order. A viewer's
    /// (`viewer`) takes worn, enchanted and renamed stacks too.
    fn move_item_to_grid(&mut self, target: usize, item: &str, count: i32, viewer: bool) -> i32 {
        self.touch();
        let existing = self.slot(target).cloned();
        let fits = |s: &Option<ItemStack>| {
            s.as_ref().is_some_and(|s| {
                &*s.item == item
                    && (viewer || usable_for_crafting(s))
                    && existing
                        .as_ref()
                        .is_none_or(|e| e.same_item_same_components(s))
            })
        };
        let found = match (0..36).find(|&i| fits(&self.state.containers[PLAYER][i])) {
            Some(i) => Some((None, i)),
            None => self
                .sources
                .iter()
                .enumerate()
                .find_map(|(k, source)| source.iter().position(fits).map(|i| (Some(k), i))),
        };
        let Some((source, i)) = found else {
            return -1;
        };
        let requested = count;
        let slot = self.state.slots[target];
        let count = if slot.container == super::MAIN {
            match self
                .state
                .container_limits
                .get(slot.index)
                .copied()
                .flatten()
            {
                Some(limit) => {
                    let room = limit.saturating_sub(existing.as_ref().map_or(0, |s| s.count));
                    if room == 0 {
                        return -1;
                    }
                    count.min(room as i32)
                }
                None => count,
            }
        } else {
            count
        };
        let cell = match source {
            None => &mut self.state.containers[PLAYER][i],
            Some(k) => &mut self.state.sources[k][i],
        };
        let taken = match cell.as_mut() {
            Some(have) if (count as u32) < have.count => split(have, count as u32),
            _ => cell.take().expect("found"),
        };
        tidy(cell);
        let taken_count = taken.count as i32;
        let slot = self.state.slots[target];
        match self.state.containers[slot.container][slot.index].as_mut() {
            None => self.state.containers[slot.container][slot.index] = Some(taken),
            Some(have) => have.count += taken.count,
        }
        requested - taken_count
    }

    /// `clearGrid`: the slots to clear go back to the inventory (or drop),
    /// then the grid empties (`clearCraftingContent`).
    fn clear_grid(&mut self, env: &Env, grid: &Grid) {
        self.touch();
        for &s in &grid.clear {
            let slot = self.state.slots[s];
            if let Some(stack) = self.state.containers[slot.container][slot.index].clone() {
                let mut copy = stack;
                self.place_back(env, &mut copy);
            }
        }
        match self.state.kind {
            Kind::Furnace(_) => {
                for &s in &grid.clear {
                    let slot = self.state.slots[s];
                    self.state.containers[slot.container][slot.index] = None;
                }
            }
            _ => {
                self.state.containers[GRID].fill(None);
                self.state.containers[RESULT][0] = None;
            }
        }
    }

    /// `Inventory.placeItemBackInInventory`.
    pub(super) fn place_back(&mut self, env: &Env, stack: &mut ItemStack) {
        self.touch();
        while stack.count > 0 {
            let slot = self
                .slot_with_space(env, stack)
                .or_else(|| (0..36).find(|&i| self.state.containers[PLAYER][i].is_none()));
            let Some(slot) = slot else {
                let dropped = std::mem::replace(
                    stack,
                    ItemStack {
                        count: 0,
                        ..stack.clone()
                    },
                );
                self.drop_stack(&dropped);
                break;
            };
            let have = self.state.containers[PLAYER][slot]
                .as_ref()
                .map_or(0, |s| s.count);
            let space = Self::max_stack(env, stack).saturating_sub(have);
            let mut part = split(stack, space);
            self.inventory_add_at(env, slot, &mut part);
        }
    }

    /// `Inventory.add(slot, stack)`.
    fn inventory_add_at(&mut self, env: &Env, slot: usize, stack: &mut ItemStack) {
        self.touch();
        if stack.count == 0 {
            return;
        }
        if Self::damageable(env, stack) && stack.damage > 0 {
            self.state.containers[PLAYER][slot] = Some(stack.clone());
            stack.count = 0;
            return;
        }
        loop {
            let last = stack.count;
            // addResource(slot, stack)
            let cell = self.state.containers[PLAYER][slot].get_or_insert_with(|| ItemStack {
                count: 0,
                ..stack.clone()
            });
            let room = Self::max_stack(env, cell).saturating_sub(cell.count);
            let add = stack.count.min(room);
            cell.count += add;
            stack.count -= add;
            tidy(&mut self.state.containers[PLAYER][slot]);
            if stack.count == 0 || stack.count >= last {
                if stack.count == last && self.state.creative {
                    stack.count = 0;
                }
                return;
            }
        }
    }

    /// `testClearGrid`: whether the inventory can take the grid back.
    fn test_clear_grid(&self, env: &Env, grid: &Grid) -> bool {
        let mut free_slots: Vec<ItemStack> = Vec::new();
        let free_in_inventory = self.state.containers[PLAYER][..36]
            .iter()
            .filter(|s| s.is_none())
            .count();
        for &s in &grid.inputs {
            let Some(mut stack) = self.slot(s).cloned() else {
                continue;
            };
            let with_space = self.slot_with_space(env, &stack);
            if with_space.is_none() && free_slots.len() <= free_in_inventory {
                for listed in &mut free_slots {
                    let max = Self::max_stack(env, listed);
                    if listed.item == stack.item
                        && listed.count != max
                        && listed.count + stack.count <= max
                    {
                        listed.count += stack.count;
                        stack.count = 0;
                        break;
                    }
                }
                if stack.count > 0 {
                    if free_slots.len() >= free_in_inventory {
                        return false;
                    }
                    free_slots.push(stack);
                }
            } else if with_space.is_none() {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod transfer_all_tests;

#[cfg(test)]
mod transfer_check_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn ing(items: &[&str]) -> Ingredient {
        Ingredient::of(items.iter().map(|s| (*s).to_owned()).collect())
    }
    fn have(items: &[(&str, i32)]) -> Stacked {
        let mut s = Stacked::default();
        for (item, n) in items {
            s.account(&Arc::from(*item), *n);
        }
        s
    }
    #[test]
    fn a_single_item_can_fill_several_ingredients() {
        let planks = ing(&["minecraft:oak_planks", "minecraft:birch_planks"]);
        let reqs = [&planks, &planks, &planks, &planks];
        let mut a = have(&[("minecraft:oak_planks", 2), ("minecraft:birch_planks", 2)]);
        assert!(a.can_craft(&reqs));
        // Two sets need 8 planks, but only 4 are there.
        assert!(!a.try_pick(&reqs, 2, None));
        assert_eq!(a.try_pick_all(&reqs, i32::MAX, None), 1);
        assert_eq!(
            have(&[("minecraft:oak_planks", 64)]).try_pick_all(&reqs, i32::MAX, None),
            16
        );
        // Picking leaves the counts as they were.
        assert!(a.try_pick(&reqs, 1, None));
        assert!(a.can_craft(&reqs));
    }
    #[test]
    fn the_matching_reassigns_flexible_ingredients() {
        // The first ingredient takes either item, the second only the stick:
        // the first item tried (the stick) moves over to free it.
        let any = ing(&["minecraft:stick", "minecraft:bone"]);
        let stick = ing(&["minecraft:stick"]);
        let mut a = have(&[("minecraft:stick", 1), ("minecraft:bone", 1)]);
        let mut used = Vec::new();
        assert!(a.try_pick(&[&any, &stick], 1, Some(&mut used)));
        assert_eq!(&*used[0], "minecraft:bone");
        assert_eq!(&*used[1], "minecraft:stick");
        assert!(!have(&[("minecraft:stick", 1)]).can_craft(&[&any, &stick]));
    }
    #[test]
    fn the_order_decides_between_fitting_items() {
        let planks = ing(&["minecraft:oak_planks", "minecraft:birch_planks"]);
        let mut a = have(&[("minecraft:oak_planks", 4), ("minecraft:birch_planks", 4)]);
        let mut used = Vec::new();
        a.try_pick(&[&planks], 1, Some(&mut used));
        assert_eq!(&*used[0], "minecraft:oak_planks");
        a.set_order(&["minecraft:birch_planks".to_owned()]);
        used.clear();
        a.try_pick(&[&planks], 1, Some(&mut used));
        assert_eq!(&*used[0], "minecraft:birch_planks");
    }
    #[test]
    fn damaged_or_named_stacks_are_not_counted() {
        let mut named = ItemStack::new("minecraft:stick", 4);
        named.data_mut().custom_name = Some(horizon_core::nbt::Tag::String("x".into()));
        let mut damaged = ItemStack::new("minecraft:iron_sword", 1);
        damaged.damage = 3;
        let mut s = Stacked::default();
        for stack in [&named, &damaged, &ItemStack::new("minecraft:stick", 2)] {
            s.account_simple(stack, 64);
        }
        assert_eq!(s.keys().len(), 1);
        assert!(s.can_craft(&[&ing(&["minecraft:stick"]), &ing(&["minecraft:stick"])]));
        assert!(!s.can_craft(&[&ing(&["minecraft:stick"]); 3]));
    }
}

#[cfg(test)]
mod container_limit_tests {
    #[test]
    fn recipe_placement_stops_at_the_container_override() {
        let mut menu = super::Menu::new(super::Kind::Furnace("smelting"));
        menu.set_container_limits(vec![Some(10); 3]);
        menu.containers[super::PLAYER][0] =
            Some(crate::items::ItemStack::new("minecraft:stick", 20));
        assert_eq!(menu.move_item_to_grid(0, "minecraft:stick", 20, false), 10);
        assert_eq!(menu.slot(0).unwrap().count, 10);
        assert_eq!(menu.move_item_to_grid(0, "minecraft:stick", 10, false), -1);
        assert_eq!(
            menu.containers[super::PLAYER][0].as_ref().unwrap().count,
            10
        );
    }
}
