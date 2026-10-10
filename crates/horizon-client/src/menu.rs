//! `AbstractContainerMenu.clicked` over vanilla's slot layouts: every
//! `ContainerInput` (pickup, quick move, swap, clone, throw, the three quick
//! craft drags, pickup all) for the player inventory, the crafting table, the
//! chest family, hoppers, dispensers, shulker boxes and the furnaces.
//! `tools/test_menu_click_parity.py` compares every click with vanilla's menu
//! classes.
use crate::{
    crafting::Recipes,
    items::{ItemStack, Items},
};

/// What a menu needs to know about items and recipes.
pub struct Env<'a> {
    pub items: &'a Items,
    pub recipes: &'a Recipes,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Input {
    Pickup,
    QuickMove,
    Swap,
    Clone,
    Throw,
    QuickCraft,
    PickupAll,
}

impl Input {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "PICKUP" => Self::Pickup,
            "QUICK_MOVE" => Self::QuickMove,
            "SWAP" => Self::Swap,
            "CLONE" => Self::Clone,
            "THROW" => Self::Throw,
            "QUICK_CRAFT" => Self::QuickCraft,
            "PICKUP_ALL" => Self::PickupAll,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Inventory,
    Crafting,
    Chest(usize),
    Hopper,
    Dispenser,
    Shulker,
    /// The cooking recipe type: smelting, blasting or smoking.
    Furnace(&'static str),
}

impl Kind {
    pub fn parse(name: &str) -> Option<Self> {
        if let Some(rows) = name.strip_prefix("chest") {
            return rows.parse().ok().map(Self::Chest);
        }
        Some(match name {
            "inventory" => Self::Inventory,
            "crafting" => Self::Crafting,
            "hopper" => Self::Hopper,
            "dispenser" => Self::Dispenser,
            "shulker" => Self::Shulker,
            "furnace" => Self::Furnace("smelting"),
            "blast_furnace" => Self::Furnace("blasting"),
            "smoker" => Self::Furnace("smoking"),
            _ => return None,
        })
    }
}

/// `Slot` subclasses that change the rules.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Plain,
    /// `ArmorSlot` for the equipment slot with this `EquipmentSlot.getIndex`.
    Armor(usize),
    Shulker,
    Fuel,
    /// `FurnaceResultSlot`.
    Output,
    /// `ResultSlot` of a crafting grid.
    CraftResult,
}

#[derive(Clone, Copy)]
struct Slot {
    container: usize,
    index: usize,
    role: Role,
}

/// The containers a menu spans.
const PLAYER: usize = 0;
const MAIN: usize = 1;
const GRID: usize = 2;
const RESULT: usize = 3;
/// `Container.getMaxStackSize`.
const CONTAINER_MAX: u32 = 99;
const OFFHAND: usize = 40;

pub struct Menu {
    kind: Kind,
    slots: Vec<Slot>,
    containers: [Vec<Option<ItemStack>>; 4],
    grid_size: usize,
    pub carried: Option<ItemStack>,
    pub data: Vec<i32>,
    pub creative: bool,
    /// Dropped stacks (canonical text), `xp+N` and the like, in order.
    pub drops: Vec<String>,
    quick_status: i32,
    quick_type: i32,
    quick_slots: Vec<usize>,
}

fn same_item(a: &ItemStack, b: &ItemStack) -> bool {
    a.item == b.item
}

fn split(stack: &mut ItemStack, amount: u32) -> ItemStack {
    let real = amount.min(stack.count);
    let mut out = stack.clone();
    out.count = real;
    stack.count -= real;
    out
}

fn tidy(slot: &mut Option<ItemStack>) {
    if slot.as_ref().is_some_and(|s| s.count == 0) {
        *slot = None;
    }
}

/// An index outside the slot list: vanilla's `IndexOutOfBoundsException`.
pub const OUT_OF_BOUNDS: &str = "IndexOutOfBoundsException";

impl Menu {
    pub fn new(kind: Kind) -> Self {
        let mut containers: [Vec<Option<ItemStack>>; 4] = Default::default();
        containers[PLAYER] = vec![None; 41];
        let mut slots = Vec::new();
        let mut grid_size = 0;
        let main = |n: usize, role: Role| -> Vec<Slot> {
            (0..n)
                .map(|index| Slot {
                    container: MAIN,
                    index,
                    role,
                })
                .collect()
        };
        let player = |slots: &mut Vec<Slot>| {
            for i in (9..36).chain(0..9) {
                slots.push(Slot {
                    container: PLAYER,
                    index: i,
                    role: Role::Plain,
                });
            }
        };
        let grid = |slots: &mut Vec<Slot>, n: usize| {
            slots.push(Slot {
                container: RESULT,
                index: 0,
                role: Role::CraftResult,
            });
            for index in 0..n * n {
                slots.push(Slot {
                    container: GRID,
                    index,
                    role: Role::Plain,
                });
            }
        };
        let mut data = 0;
        match kind {
            Kind::Inventory => {
                grid_size = 2;
                grid(&mut slots, 2);
                for (i, eq) in [3usize, 2, 1, 0].into_iter().enumerate() {
                    slots.push(Slot {
                        container: PLAYER,
                        index: 39 - i,
                        role: Role::Armor(eq),
                    });
                }
                player(&mut slots);
                slots.push(Slot {
                    container: PLAYER,
                    index: OFFHAND,
                    role: Role::Plain,
                });
            }
            Kind::Crafting => {
                grid_size = 3;
                grid(&mut slots, 3);
                player(&mut slots);
            }
            Kind::Chest(rows) => {
                containers[MAIN] = vec![None; rows * 9];
                slots.extend(main(rows * 9, Role::Plain));
                player(&mut slots);
            }
            Kind::Hopper => {
                containers[MAIN] = vec![None; 5];
                slots.extend(main(5, Role::Plain));
                player(&mut slots);
            }
            Kind::Dispenser => {
                containers[MAIN] = vec![None; 9];
                slots.extend(main(9, Role::Plain));
                player(&mut slots);
            }
            Kind::Shulker => {
                containers[MAIN] = vec![None; 27];
                slots.extend(main(27, Role::Shulker));
                player(&mut slots);
            }
            Kind::Furnace(_) => {
                containers[MAIN] = vec![None; 3];
                slots.extend([
                    Slot {
                        container: MAIN,
                        index: 0,
                        role: Role::Plain,
                    },
                    Slot {
                        container: MAIN,
                        index: 1,
                        role: Role::Fuel,
                    },
                    Slot {
                        container: MAIN,
                        index: 2,
                        role: Role::Output,
                    },
                ]);
                player(&mut slots);
                data = 4;
            }
        }
        containers[GRID] = vec![None; grid_size * grid_size];
        containers[RESULT] = vec![None; 1];
        Self {
            kind,
            slots,
            containers,
            grid_size,
            carried: None,
            data: vec![0; data],
            creative: false,
            drops: Vec::new(),
            quick_status: 0,
            quick_type: 0,
            quick_slots: Vec::new(),
        }
    }

    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    pub fn slot(&self, i: usize) -> Option<&ItemStack> {
        let s = self.slots[i];
        self.containers[s.container][s.index].as_ref()
    }

    /// Sets a slot as a test fixture does (`Slot.set`).
    pub fn set_slot(&mut self, env: &Env, i: usize, stack: Option<ItemStack>) {
        self.put(env, i, stack);
    }

    pub fn player_item(&self, i: usize) -> Option<&ItemStack> {
        self.containers[PLAYER][i].as_ref()
    }

    pub fn set_player_item(&mut self, i: usize, stack: Option<ItemStack>) {
        self.containers[PLAYER][i] = stack;
    }

    /// `Inventory.add`: the leftover stays in `stack`.
    pub fn give(&mut self, env: &Env, stack: &mut ItemStack) -> bool {
        self.inventory_add(env, stack)
    }

    /// `Inventory.contains(stack)`.
    pub fn player_contains(&self, stack: &ItemStack) -> bool {
        self.containers[PLAYER]
            .iter()
            .flatten()
            .any(|have| have.same_item_same_components(stack))
    }

    // ---- item helpers ----

    fn max_stack(env: &Env, stack: &ItemStack) -> u32 {
        env.items.get(&stack.item).map_or(64, |i| i.max_stack)
    }

    fn damageable(env: &Env, stack: &ItemStack) -> bool {
        env.items.get(&stack.item).is_some_and(|i| i.max_damage > 0)
    }

    /// `ItemStack.isStackable`.
    fn stackable(env: &Env, stack: &ItemStack) -> bool {
        Self::max_stack(env, stack) > 1 && !(Self::damageable(env, stack) && stack.damage > 0)
    }

    fn is_bucket(stack: &ItemStack) -> bool {
        &*stack.item == "minecraft:bucket"
    }

    fn is_fuel(env: &Env, stack: &ItemStack) -> bool {
        env.items.get(&stack.item).is_some_and(|i| i.fuel.is_some())
    }

    /// `Item.canFitInsideContainerItems`: shulker boxes cannot nest.
    fn fits_inside(stack: &ItemStack) -> bool {
        !(stack.item.ends_with("shulker_box"))
    }

    // ---- Slot ----

    fn may_place(&self, env: &Env, s: usize, stack: &ItemStack) -> bool {
        match self.slots[s].role {
            Role::Plain => true,
            Role::Armor(eq) => env
                .items
                .get(&stack.item)
                .and_then(|i| {
                    let slot = match i.equip_slot.as_deref()? {
                        "feet" => 0,
                        "legs" => 1,
                        "chest" => 2,
                        "head" => 3,
                        _ => return None,
                    };
                    let allowed = i
                        .equip_allowed
                        .as_ref()
                        .is_none_or(|a| a.contains("minecraft:player"));
                    Some(slot == eq && allowed)
                })
                .unwrap_or(false),
            Role::Shulker => Self::fits_inside(stack),
            Role::Fuel => Self::is_fuel(env, stack) || Self::is_bucket(stack),
            Role::Output | Role::CraftResult => false,
        }
    }

    fn may_pickup(&self, s: usize) -> bool {
        match self.slots[s].role {
            Role::Armor(_) => match self.slot(s) {
                Some(stack) => {
                    self.creative || stack.enchantment_level("minecraft:binding_curse") == 0
                }
                None => true,
            },
            _ => true,
        }
    }

    /// `Slot.getMaxStackSize(stack)`.
    fn slot_max(&self, env: &Env, s: usize, stack: &ItemStack) -> u32 {
        let own = match self.slots[s].role {
            Role::Armor(_) => 1,
            Role::Fuel if Self::is_bucket(stack) => return 1,
            _ => CONTAINER_MAX,
        };
        own.min(Self::max_stack(env, stack))
    }

    /// `Container.setItem` (a grid change recomputes the result).
    fn put(&mut self, env: &Env, s: usize, mut stack: Option<ItemStack>) {
        tidy(&mut stack);
        let slot = self.slots[s];
        self.containers[slot.container][slot.index] = stack;
        if slot.container == GRID {
            self.recompute(env);
        }
    }

    /// `Slot.remove`: the container's `removeItem`.
    fn remove(&mut self, env: &Env, s: usize, amount: u32) -> Option<ItemStack> {
        let slot = self.slots[s];
        if slot.container == RESULT {
            return self.containers[RESULT][0].take();
        }
        let cell = &mut self.containers[slot.container][slot.index];
        let out = cell
            .as_mut()
            .filter(|c| c.count > 0)
            .map(|c| split(c, amount));
        tidy(cell);
        if out.is_some() && slot.container == GRID {
            self.recompute(env);
        }
        out
    }

    /// `CraftingMenu.slotChangedCraftingGrid`.
    fn recompute(&mut self, env: &Env) {
        if self.grid_size == 0 {
            return;
        }
        let result = env
            .recipes
            .find(&self.containers[GRID], self.grid_size)
            .map(|(_, stack)| stack);
        self.containers[RESULT][0] = result;
    }

    /// `ResultSlot.onTake` and the other `Slot.onTake` overrides.
    fn on_take(&mut self, env: &Env, s: usize) {
        if self.slots[s].role != Role::CraftResult {
            return;
        }
        let n = self.grid_size;
        let grid = self.containers[GRID].clone();
        let remaining = env
            .recipes
            .find(&grid, n)
            .map(|(id, _)| env.recipes.remainders(id, &grid, n))
            .unwrap_or_else(|| {
                // CraftingRecipe.defaultCraftingReminder
                grid.iter()
                    .map(|cell| {
                        cell.as_ref().and_then(|stack| {
                            env.items
                                .get(&stack.item)
                                .and_then(|i| i.crafting_remainder.as_deref())
                                .map(|r| ItemStack::new(r, 1))
                        })
                    })
                    .collect()
            });
        for cell in 0..n * n {
            let mut here = self.containers[GRID][cell].take();
            if let Some(stack) = here.as_mut() {
                stack.count -= 1;
            }
            tidy(&mut here);
            self.containers[GRID][cell] = here;
            if let Some(replacement) = remaining.get(cell).cloned().flatten() {
                match self.containers[GRID][cell].as_mut() {
                    None => self.containers[GRID][cell] = Some(replacement),
                    Some(have) if have.same_item_same_components(&replacement) => {
                        let mut merged = replacement;
                        merged.count += have.count;
                        self.containers[GRID][cell] = Some(merged);
                    }
                    Some(_) => {
                        let mut left = replacement;
                        if !self.inventory_add(env, &mut left) {
                            self.drop_stack(&left);
                        }
                    }
                }
            }
        }
        self.recompute(env);
    }

    fn drop_stack(&mut self, stack: &ItemStack) {
        if stack.count > 0 {
            self.drops.push(crate::item_data::canon_stack(Some(stack)));
        }
    }

    /// `Slot.tryRemove`.
    fn try_remove(
        &mut self,
        env: &Env,
        s: usize,
        amount: u32,
        max_amount: u32,
    ) -> Option<ItemStack> {
        if !self.may_pickup(s) {
            return None;
        }
        if !self.allow_modification(env, s) && max_amount < self.slot(s).map_or(0, |c| c.count) {
            return None;
        }
        let result = self.remove(env, s, amount.min(max_amount))?;
        if self.slot(s).is_none() {
            self.put(env, s, None);
        }
        Some(result)
    }

    fn allow_modification(&self, env: &Env, s: usize) -> bool {
        self.may_pickup(s)
            && match self.slot(s) {
                Some(stack) => self.may_place(env, s, stack),
                None => true,
            }
    }

    fn safe_take(
        &mut self,
        env: &Env,
        s: usize,
        amount: u32,
        max_amount: u32,
    ) -> Option<ItemStack> {
        let result = self.try_remove(env, s, amount, max_amount)?;
        self.on_take(env, s);
        Some(result)
    }

    /// `Slot.safeInsert`: puts up to `amount` of `input` into the slot.
    fn safe_insert(&mut self, env: &Env, s: usize, input: &mut Option<ItemStack>, amount: u32) {
        let Some(stack) = input.as_mut() else { return };
        if !self.may_place(env, s, stack) {
            return;
        }
        let have = self.slot(s).map_or(0, |c| c.count);
        let max = self.slot_max(env, s, stack);
        let transferable = amount.min(stack.count).min(max.saturating_sub(have));
        if transferable == 0 {
            return;
        }
        match self.slot(s).cloned() {
            None => {
                let part = split(stack, transferable);
                self.put(env, s, Some(part));
            }
            Some(mut current) if current.same_item_same_components(stack) => {
                stack.count -= transferable;
                current.count += transferable;
                self.put(env, s, Some(current));
            }
            Some(_) => {}
        }
        tidy(input);
    }

    // ---- Inventory ----

    /// `Inventory.getSlotWithRemainingSpace` (selected slot 0, offhand, rest).
    fn slot_with_space(&self, env: &Env, stack: &ItemStack) -> Option<usize> {
        let inv = &self.containers[PLAYER];
        [0, OFFHAND].into_iter().chain(0..36).find(|&i| {
            inv[i].as_ref().is_some_and(|have| {
                have.same_item_same_components(stack)
                    && Self::stackable(env, have)
                    && have.count < CONTAINER_MAX.min(Self::max_stack(env, have))
            })
        })
    }

    /// `Inventory.addResource`: returns what did not fit.
    fn add_resource(&mut self, env: &Env, stack: &ItemStack) -> u32 {
        let slot = self.slot_with_space(env, stack).or_else(|| {
            self.containers[PLAYER][..36]
                .iter()
                .position(Option::is_none)
        });
        let Some(slot) = slot else { return stack.count };
        let cell = self.containers[PLAYER][slot].get_or_insert_with(|| {
            let mut empty = stack.clone();
            empty.count = 0;
            empty
        });
        let room = CONTAINER_MAX.min(Self::max_stack(env, cell)) - cell.count;
        let add = stack.count.min(room);
        cell.count += add;
        tidy(&mut self.containers[PLAYER][slot]);
        stack.count - add
    }

    /// `Inventory.add(stack)`: the leftover stays in `stack`.
    fn inventory_add(&mut self, env: &Env, stack: &mut ItemStack) -> bool {
        if stack.count == 0 {
            return false;
        }
        if Self::damageable(env, stack) && stack.damage > 0 {
            if let Some(slot) = self.containers[PLAYER][..36]
                .iter()
                .position(Option::is_none)
            {
                self.containers[PLAYER][slot] = Some(stack.clone());
                stack.count = 0;
                return true;
            }
            if self.creative {
                stack.count = 0;
                return true;
            }
            return false;
        }
        loop {
            let last = stack.count;
            stack.count = self.add_resource(env, stack);
            if stack.count == 0 || stack.count >= last {
                if stack.count == last && self.creative {
                    stack.count = 0;
                    return true;
                }
                return stack.count < last;
            }
        }
    }
}

/// `EquipmentSlot` of an item as the inventory menu's quick move sees it.
enum Equip {
    Armor(usize),
    Offhand,
    Other,
}

impl Menu {
    fn equip(env: &Env, stack: &ItemStack) -> Equip {
        match env
            .items
            .get(&stack.item)
            .and_then(|i| i.equip_slot.as_deref())
        {
            Some("feet") => Equip::Armor(0),
            Some("legs") => Equip::Armor(1),
            Some("chest") => Equip::Armor(2),
            Some("head") => Equip::Armor(3),
            Some("offhand") => Equip::Offhand,
            _ => Equip::Other,
        }
    }

    fn has_item(&self, s: usize) -> bool {
        self.slot(s).is_some()
    }

    /// `AbstractContainerMenu.moveItemStackTo`.
    fn move_to(
        &mut self,
        env: &Env,
        stack: &mut ItemStack,
        start: usize,
        end: usize,
        backwards: bool,
    ) -> bool {
        let mut changed = false;
        let order = |_: ()| -> Vec<usize> {
            if backwards {
                (start..end).rev().collect()
            } else {
                (start..end).collect()
            }
        };
        if Self::stackable(env, stack) {
            for dest in order(()) {
                if stack.count == 0 {
                    break;
                }
                let max = match self.slot(dest) {
                    Some(target) if target.same_item_same_components(stack) => {
                        self.slot_max(env, dest, target)
                    }
                    _ => continue,
                };
                let slot = self.slots[dest];
                let target = self.containers[slot.container][slot.index]
                    .as_mut()
                    .expect("checked");
                let total = target.count + stack.count;
                if total <= max {
                    stack.count = 0;
                    target.count = total;
                    changed = true;
                } else if target.count < max {
                    stack.count -= max - target.count;
                    target.count = max;
                    changed = true;
                }
            }
        }
        if stack.count > 0 {
            for dest in order(()) {
                if !self.has_item(dest) && self.may_place(env, dest, stack) {
                    let max = self.slot_max(env, dest, stack);
                    let part = split(stack, stack.count.min(max));
                    self.put(env, dest, Some(part));
                    changed = true;
                    break;
                }
            }
        }
        changed
    }

    /// `quickMoveStack` of the menu's kind; the moved stack's original copy
    /// (None when nothing moved).
    fn quick_move(&mut self, env: &Env, index: usize) -> Option<ItemStack> {
        let mut stack = self.slot(index)?.clone();
        let clicked = stack.clone();
        let total = self.slots.len();
        let mut check_count = true;
        let mut after_take = false;
        match self.kind {
            Kind::Chest(rows) => {
                check_count = false;
                let n = rows * 9;
                if index < n {
                    if !self.move_to(env, &mut stack, n, total, true) {
                        return None;
                    }
                } else if !self.move_to(env, &mut stack, 0, n, false) {
                    return None;
                }
            }
            Kind::Hopper => {
                check_count = false;
                if index < 5 {
                    if !self.move_to(env, &mut stack, 5, 41, true) {
                        return None;
                    }
                } else if !self.move_to(env, &mut stack, 0, 5, false) {
                    return None;
                }
            }
            Kind::Shulker => {
                check_count = false;
                if index < 27 {
                    if !self.move_to(env, &mut stack, 27, total, true) {
                        return None;
                    }
                } else if !self.move_to(env, &mut stack, 0, 27, false) {
                    return None;
                }
            }
            Kind::Dispenser => {
                after_take = true;
                if index < 9 {
                    if !self.move_to(env, &mut stack, 9, 45, true) {
                        return None;
                    }
                } else if !self.move_to(env, &mut stack, 0, 9, false) {
                    return None;
                }
            }
            Kind::Furnace(recipe) => {
                after_take = true;
                if index == 2 {
                    if !self.move_to(env, &mut stack, 3, 39, true) {
                        return None;
                    }
                } else if index != 1 && index != 0 {
                    let smelts = env
                        .recipes
                        .cook(&format!("minecraft:{recipe}"), &stack)
                        .is_some();
                    if smelts {
                        if !self.move_to(env, &mut stack, 0, 1, false) {
                            return None;
                        }
                    } else if Self::is_fuel(env, &stack) {
                        if !self.move_to(env, &mut stack, 1, 2, false) {
                            return None;
                        }
                    } else if (3..30).contains(&index) {
                        if !self.move_to(env, &mut stack, 30, 39, false) {
                            return None;
                        }
                    } else if (30..39).contains(&index)
                        && !self.move_to(env, &mut stack, 3, 30, false)
                    {
                        return None;
                    }
                } else if !self.move_to(env, &mut stack, 3, 39, false) {
                    return None;
                }
            }
            Kind::Crafting => {
                after_take = true;
                if index == 0 {
                    if !self.move_to(env, &mut stack, 10, 46, true) {
                        return None;
                    }
                } else if (10..46).contains(&index) {
                    if !self.move_to(env, &mut stack, 1, 10, false) {
                        if index < 37 {
                            if !self.move_to(env, &mut stack, 37, 46, false) {
                                return None;
                            }
                        } else if !self.move_to(env, &mut stack, 10, 37, false) {
                            return None;
                        }
                    }
                } else if !self.move_to(env, &mut stack, 10, 46, false) {
                    return None;
                }
            }
            Kind::Inventory => {
                after_take = true;
                let equip = Self::equip(env, &clicked);
                let armor_free = |m: &Self, eq: usize| !m.has_item(8 - eq);
                if index == 0 {
                    if !self.move_to(env, &mut stack, 9, 45, true) {
                        return None;
                    }
                } else if (1..9).contains(&index) {
                    if !self.move_to(env, &mut stack, 9, 45, false) {
                        return None;
                    }
                } else if matches!(equip, Equip::Armor(eq) if armor_free(self, eq)) {
                    let Equip::Armor(eq) = equip else {
                        unreachable!()
                    };
                    let pos = 8 - eq;
                    if !self.move_to(env, &mut stack, pos, pos + 1, false) {
                        return None;
                    }
                } else if matches!(equip, Equip::Offhand) && !self.has_item(45) {
                    if !self.move_to(env, &mut stack, 45, 46, false) {
                        return None;
                    }
                } else if (9..36).contains(&index) {
                    if !self.move_to(env, &mut stack, 36, 45, false) {
                        return None;
                    }
                } else if (36..45).contains(&index) {
                    if !self.move_to(env, &mut stack, 9, 36, false) {
                        return None;
                    }
                } else if !self.move_to(env, &mut stack, 9, 45, false) {
                    return None;
                }
            }
        }
        // The stack was edited in place in its slot.
        let slot = self.slots[index];
        if stack.count == 0 {
            self.put(env, index, None);
        } else {
            self.containers[slot.container][slot.index] = Some(stack.clone());
        }
        if check_count && stack.count == clicked.count {
            return None;
        }
        if after_take || matches!(self.kind, Kind::Inventory | Kind::Crafting) {
            self.on_take(env, index);
        }
        if index == 0 && matches!(self.kind, Kind::Inventory | Kind::Crafting) {
            self.drop_stack(&stack);
        }
        Some(clicked)
    }

    fn quick_replace(&self, s: usize, stack: &ItemStack, env: &Env) -> bool {
        match self.slot(s) {
            None => true,
            Some(have) if have.same_item_same_components(stack) => {
                have.count <= Self::max_stack(env, stack)
            }
            Some(_) => false,
        }
    }

    fn reset_quick_craft(&mut self) {
        self.quick_status = 0;
        self.quick_slots.clear();
    }

    /// `AbstractContainerMenu.clicked`: Err carries a vanilla exception name.
    pub fn click(
        &mut self,
        env: &Env,
        slot_index: i32,
        button: i32,
        input: Input,
    ) -> Result<(), &'static str> {
        let size = self.slots.len() as i32;
        let in_range = |i: i32| (0..size).contains(&i);
        if input == Input::QuickCraft {
            let expected = self.quick_status;
            self.quick_status = button & 3;
            if ((expected != 1 || self.quick_status != 2) && expected != self.quick_status)
                || self.carried.is_none()
            {
                self.reset_quick_craft();
            } else if self.quick_status == 0 {
                self.quick_type = (button >> 2) & 3;
                if self.quick_type == 0
                    || self.quick_type == 1
                    || (self.quick_type == 2 && self.creative)
                {
                    self.quick_status = 1;
                    self.quick_slots.clear();
                } else {
                    self.reset_quick_craft();
                }
            } else if self.quick_status == 1 {
                if !in_range(slot_index) {
                    return Err(OUT_OF_BOUNDS);
                }
                let s = slot_index as usize;
                let carried = self.carried.clone().expect("checked");
                if self.quick_replace(s, &carried, env)
                    && self.may_place(env, s, &carried)
                    && (self.quick_type == 2 || carried.count as usize > self.quick_slots.len())
                    && !self.quick_slots.contains(&s)
                {
                    self.quick_slots.push(s);
                }
            } else if self.quick_status == 2 {
                if !self.quick_slots.is_empty() {
                    if self.quick_slots.len() == 1 {
                        let s = self.quick_slots[0] as i32;
                        let ty = self.quick_type;
                        self.reset_quick_craft();
                        return self.click(env, s, ty, Input::Pickup);
                    }
                    let mut source = self.carried.clone().expect("checked");
                    let mut remaining = source.count as i64;
                    let carried = self.carried.clone().expect("checked");
                    for s in self.quick_slots.clone() {
                        if self.quick_replace(s, &carried, env)
                            && self.may_place(env, s, &carried)
                            && (self.quick_type == 2
                                || carried.count as usize >= self.quick_slots.len())
                        {
                            let have = self.slot(s).map_or(0, |c| c.count);
                            let max =
                                Self::max_stack(env, &source).min(self.slot_max(env, s, &source));
                            let each = match self.quick_type {
                                0 => (source.count as f32 / self.quick_slots.len() as f32).floor()
                                    as u32,
                                1 => 1,
                                2 => Self::max_stack(env, &source),
                                _ => source.count,
                            };
                            let new_count = (each + have).min(max);
                            remaining -= i64::from(new_count) - i64::from(have);
                            let mut placed = source.clone();
                            placed.count = new_count;
                            self.put(env, s, Some(placed));
                        }
                    }
                    source.count = remaining.max(0) as u32;
                    self.carried = Some(source);
                    tidy(&mut self.carried);
                }
                self.reset_quick_craft();
            } else {
                self.reset_quick_craft();
            }
            return Ok(());
        }
        if self.quick_status != 0 {
            self.reset_quick_craft();
            return Ok(());
        }
        if matches!(input, Input::Pickup | Input::QuickMove) && (button == 0 || button == 1) {
            let primary = button == 0;
            if slot_index == -999 {
                if let Some(carried) = self.carried.as_mut() {
                    if primary {
                        let dropped = self.carried.take().expect("some");
                        self.drop_stack(&dropped);
                    } else {
                        let one = split(carried, 1);
                        self.drop_stack(&one);
                        tidy(&mut self.carried);
                    }
                }
            } else if input == Input::QuickMove {
                if slot_index < 0 {
                    return Ok(());
                }
                if !in_range(slot_index) {
                    return Err(OUT_OF_BOUNDS);
                }
                let s = slot_index as usize;
                if !self.may_pickup(s) {
                    return Ok(());
                }
                let mut clicked = self.quick_move(env, s);
                while let Some(c) = &clicked {
                    if !self.slot(s).is_some_and(|now| same_item(now, c)) {
                        break;
                    }
                    clicked = self.quick_move(env, s);
                }
            } else {
                if slot_index < 0 {
                    return Ok(());
                }
                if !in_range(slot_index) {
                    return Err(OUT_OF_BOUNDS);
                }
                self.pickup(env, slot_index as usize, primary);
            }
        } else if input == Input::Swap && ((0..9).contains(&button) || button == 40) {
            if !in_range(slot_index) {
                return Err(OUT_OF_BOUNDS);
            }
            self.swap(env, slot_index as usize, button as usize);
        } else if input == Input::Clone
            && self.creative
            && self.carried.is_none()
            && slot_index >= 0
        {
            if !in_range(slot_index) {
                return Err(OUT_OF_BOUNDS);
            }
            if let Some(stack) = self.slot(slot_index as usize) {
                let mut copy = stack.clone();
                copy.count = Self::max_stack(env, stack);
                self.carried = Some(copy);
            }
        } else if input == Input::Throw && self.carried.is_none() && slot_index >= 0 {
            if !in_range(slot_index) {
                return Err(OUT_OF_BOUNDS);
            }
            let s = slot_index as usize;
            let amount = if button == 0 {
                1
            } else {
                self.slot(s).map_or(0, |c| c.count)
            };
            let mut taken = self.safe_take(env, s, amount, u32::MAX);
            if let Some(t) = &taken {
                self.drop_stack(t);
            }
            if button == 1 {
                while let Some(t) = &taken {
                    if !self.slot(s).is_some_and(|now| same_item(now, t)) {
                        break;
                    }
                    taken = self.safe_take(env, s, amount, u32::MAX);
                    if let Some(t) = &taken {
                        self.drop_stack(t);
                    }
                }
            }
        } else if input == Input::PickupAll && slot_index >= 0 {
            if !in_range(slot_index) {
                return Err(OUT_OF_BOUNDS);
            }
            self.pickup_all(env, slot_index as usize, button);
        }
        Ok(())
    }

    /// The PICKUP branch of `doClick` (without the bundle overrides).
    fn pickup(&mut self, env: &Env, s: usize, primary: bool) {
        let clicked = self.slot(s).cloned();
        match clicked {
            None => {
                if let Some(mut carried) = self.carried.take() {
                    let amount = if primary { carried.count } else { 1 };
                    let mut input = Some(carried.clone());
                    self.safe_insert(env, s, &mut input, amount);
                    carried = input.unwrap_or_else(|| {
                        let mut e = carried.clone();
                        e.count = 0;
                        e
                    });
                    self.carried = Some(carried);
                    tidy(&mut self.carried);
                }
            }
            Some(clicked) if self.may_pickup(s) => match self.carried.take() {
                None => {
                    let amount = if primary {
                        clicked.count
                    } else {
                        clicked.count.div_ceil(2)
                    };
                    if let Some(taken) = self.try_remove(env, s, amount, u32::MAX) {
                        self.carried = Some(taken);
                        self.on_take(env, s);
                    }
                }
                Some(mut carried) => {
                    if self.may_place(env, s, &carried) {
                        if clicked.same_item_same_components(&carried) {
                            let amount = if primary { carried.count } else { 1 };
                            let mut input = Some(carried.clone());
                            self.safe_insert(env, s, &mut input, amount);
                            carried = input.unwrap_or_else(|| {
                                let mut e = carried.clone();
                                e.count = 0;
                                e
                            });
                            self.carried = Some(carried);
                            tidy(&mut self.carried);
                        } else if carried.count <= self.slot_max(env, s, &carried) {
                            self.carried = Some(clicked);
                            self.put(env, s, Some(carried));
                        } else {
                            self.carried = Some(carried);
                        }
                    } else if clicked.same_item_same_components(&carried) {
                        let room = Self::max_stack(env, &carried).saturating_sub(carried.count);
                        self.carried = Some(carried.clone());
                        if let Some(taken) = self.try_remove(env, s, clicked.count, room) {
                            if let Some(c) = self.carried.as_mut() {
                                c.count += taken.count;
                            }
                            self.on_take(env, s);
                        }
                    } else {
                        self.carried = Some(carried);
                    }
                }
            },
            Some(_) => {}
        }
    }

    /// The SWAP branch of `doClick`.
    fn swap(&mut self, env: &Env, target: usize, button: usize) {
        let inv_slot = button;
        let source = self.containers[PLAYER][inv_slot].clone();
        let target_stack = self.slot(target).cloned();
        if source.is_none() && target_stack.is_none() {
            return;
        }
        match (source, target_stack) {
            (None, Some(taken)) => {
                if self.may_pickup(target) {
                    self.containers[PLAYER][inv_slot] = Some(taken.clone());
                    self.put(env, target, None);
                    self.on_take(env, target);
                }
            }
            (Some(mut source), None) => {
                if self.may_place(env, target, &source) {
                    let max = self.slot_max(env, target, &source);
                    if source.count > max {
                        let part = split(&mut source, max);
                        self.containers[PLAYER][inv_slot] = Some(source);
                        self.put(env, target, Some(part));
                    } else {
                        self.containers[PLAYER][inv_slot] = None;
                        self.put(env, target, Some(source));
                    }
                }
            }
            (Some(mut source), Some(mut taken)) => {
                if self.may_pickup(target) && self.may_place(env, target, &source) {
                    let max = self.slot_max(env, target, &source);
                    if source.count > max {
                        let part = split(&mut source, max);
                        self.containers[PLAYER][inv_slot] = Some(source);
                        self.put(env, target, Some(part));
                        self.on_take(env, target);
                        if !self.inventory_add(env, &mut taken) {
                            self.drop_stack(&taken);
                        }
                    } else {
                        self.containers[PLAYER][inv_slot] = Some(taken);
                        self.put(env, target, Some(source));
                        self.on_take(env, target);
                    }
                }
            }
            (None, None) => {}
        }
    }

    /// The PICKUP_ALL branch (double click).
    fn pickup_all(&mut self, env: &Env, s: usize, button: i32) {
        let Some(carried) = self.carried.clone() else {
            return;
        };
        if self.has_item(s) && self.may_pickup(s) {
            return;
        }
        let total = self.slots.len() as i32;
        let (start, step) = if button == 0 { (0, 1) } else { (total - 1, -1) };
        let carried_max = Self::max_stack(env, &carried);
        for pass in 0..2 {
            let mut i = start;
            while i >= 0 && i < total && self.carried.as_ref().map_or(0, |c| c.count) < carried_max
            {
                let t = i as usize;
                i += step;
                let have = self.carried.clone().expect("some");
                if self.has_item(t)
                    && self.quick_replace(t, &have, env)
                    && self.may_pickup(t)
                    && self.slots[t].container != RESULT
                {
                    let item = self.slot(t).expect("has item").clone();
                    if pass != 0 || item.count != Self::max_stack(env, &item) {
                        let room = carried_max - have.count;
                        if let Some(removed) = self.safe_take(env, t, item.count, room)
                            && let Some(c) = self.carried.as_mut()
                        {
                            c.count += removed.count;
                        }
                    }
                }
            }
        }
    }
}
