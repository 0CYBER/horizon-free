//! `AbstractContainerMenu.clicked` over vanilla's slot layouts: every
//! `ContainerInput` (pickup, quick move, swap, clone, throw, the three quick
//! craft drags, pickup all) for the player inventory, the crafting table, the
//! chest family, hoppers, dispensers, shulker boxes, the furnaces, the brewing
//! stand, the crafter (its client menu and the block entity's server menu with
//! `setSlotState`), the loom (`clickMenuButton`, `slotsChanged`) and the
//! merchant (`MerchantMenu` over a `merchant::Session`: payment slots, the
//! result slot's trade, `tryMoveItems`).
//! `tools/test_menu_click_parity.py` compares every click with vanilla's menu
//! classes.
use crate::{
    banner::{self, Banners},
    brewing::Brewing,
    crafting::Recipes,
    items::{ItemStack, Items},
};
use std::sync::Arc;

pub mod place;

/// What a menu needs to know about items and recipes.
pub struct Env<'a> {
    pub items: &'a Items,
    pub recipes: &'a Recipes,
    pub brewing: &'a Brewing,
    pub banners: &'a Banners,
    /// What a merchant menu needs for offers made while it is open (level-ups).
    pub trade: Option<&'a crate::merchant::TradeEnv<'a>>,
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
    Brewing,
    /// `CrafterMenu` over a plain crafting container (the client's menu).
    Crafter,
    /// `CrafterMenu` over the `CrafterBlockEntity` (the server's menu): the
    /// result follows the grid, and filling a disabled slot enables it.
    CrafterBlock,
    Loom,
    /// `MerchantMenu`: two payment slots, the result, then the inventory.
    Merchant,
    /// `CreativeModeInventoryScreen.ItemPickerMenu`: an item tab's 45
    /// display slots, then the hotbar.
    ItemPicker,
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
            "brewing" => Self::Brewing,
            "crafter" => Self::Crafter,
            "crafter_be" => Self::CrafterBlock,
            "loom" => Self::Loom,
            "merchant" => Self::Merchant,
            "item_picker" => Self::ItemPicker,
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
    /// `CrafterSlot`: refuses items while its disabled flag (data slot) is set.
    CrafterIn,
    /// `NonInteractiveResultSlot`.
    Inert,
    /// `BrewingStandMenu.PotionSlot`.
    Potion,
    /// `BrewingStandMenu.IngredientsSlot`.
    Ingredient,
    /// `BrewingStandMenu.FuelSlot`.
    BrewFuel,
    /// The loom's banner, dye and pattern slots.
    LoomBanner,
    LoomDye,
    LoomPattern,
    /// The loom's result slot (a plain slot whose `onTake` uses the inputs).
    LoomResult,
    /// `MerchantResultSlot`.
    MerchantResult,
    /// `CustomCreativeSlot`: a creative tab's display slot, which the
    /// screen clicks itself; drags and PICKUP_ALL pass it by.
    CreativeDisplay,
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
    /// The selected hotbar slot (`Inventory.selected`).
    pub selected: usize,
    /// Dropped stacks (canonical text), `xp+N` and the like, in order.
    pub drops: Vec<String>,
    /// The dropped stacks themselves, for the caller to throw.
    pub dropped: Vec<ItemStack>,
    quick_status: i32,
    quick_type: i32,
    quick_slots: Vec<usize>,
    /// `LoomMenu.selectablePatterns`.
    pub loom_patterns: Vec<Arc<str>>,
    /// Sounds the menu plays (`ui.loom.take_result`), for the caller.
    pub sounds: Vec<&'static str>,
    /// Recipes whose result was taken (`awardUsedRecipes`), for the caller.
    pub crafted: Vec<String>,
    /// `LivingEntity.onEquipItem` calls that change the item: the
    /// `EquipmentSlot` name and the new stack, in order, for the caller.
    pub equipped: Vec<(&'static str, Option<ItemStack>)>,
    /// The merchant a merchant menu trades with.
    pub merchant: Option<Box<crate::merchant::Session>>,
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
            Kind::Crafter | Kind::CrafterBlock => {
                containers[MAIN] = vec![None; 9];
                slots.extend(main(9, Role::CrafterIn));
                player(&mut slots);
                slots.push(Slot {
                    container: RESULT,
                    index: 0,
                    role: Role::Inert,
                });
                data = 10;
            }
            Kind::Loom => {
                containers[MAIN] = vec![None; 3];
                for (index, role) in [Role::LoomBanner, Role::LoomDye, Role::LoomPattern]
                    .into_iter()
                    .enumerate()
                {
                    slots.push(Slot {
                        container: MAIN,
                        index,
                        role,
                    });
                }
                slots.push(Slot {
                    container: RESULT,
                    index: 0,
                    role: Role::LoomResult,
                });
                player(&mut slots);
                data = 1;
            }
            Kind::ItemPicker => {
                containers[MAIN] = vec![None; 45];
                slots.extend(main(45, Role::CreativeDisplay));
                // addInventoryHotbarSlots.
                slots.extend((0..9).map(|index| Slot {
                    container: PLAYER,
                    index,
                    role: Role::Plain,
                }));
            }
            Kind::Merchant => {
                containers[MAIN] = vec![None; 2];
                slots.extend(main(2, Role::Plain));
                slots.push(Slot {
                    container: RESULT,
                    index: 0,
                    role: Role::MerchantResult,
                });
                player(&mut slots);
            }
            Kind::Brewing => {
                containers[MAIN] = vec![None; 5];
                for (index, role) in [
                    Role::Potion,
                    Role::Potion,
                    Role::Potion,
                    Role::Ingredient,
                    Role::BrewFuel,
                ]
                .into_iter()
                .enumerate()
                {
                    slots.push(Slot {
                        container: MAIN,
                        index,
                        role,
                    });
                }
                player(&mut slots);
                data = 4;
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
            selected: 0,
            drops: Vec::new(),
            dropped: Vec::new(),
            quick_status: 0,
            quick_type: 0,
            quick_slots: Vec::new(),
            loom_patterns: Vec::new(),
            sounds: Vec::new(),
            crafted: Vec::new(),
            equipped: Vec::new(),
            merchant: None,
        }
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// `clickMenuButton`: the loom's pattern buttons. False when refused.
    pub fn button(&mut self, env: &Env, id: i32) -> bool {
        if self.kind != Kind::Loom || id < 0 || id as usize >= self.loom_patterns.len() {
            return false;
        }
        self.data[0] = id;
        let pattern = self.loom_patterns[id as usize].clone();
        self.loom_setup_result(env, &pattern);
        true
    }

    /// `CrafterBlockEntity.setSlotState` (the slot state packet): only an
    /// empty slot can be disabled or enabled.
    pub fn set_slot_state(&mut self, slot: usize, enabled: bool) {
        if self.kind == Kind::CrafterBlock && slot < 9 && self.containers[MAIN][slot].is_none() {
            self.data[slot] = i32::from(!enabled);
        }
    }

    /// The menu after a fixture set its slots: the block crafter's slot
    /// listener has refreshed the result (`broadcastChanges`).
    pub fn settle(&mut self, env: &Env) {
        if self.kind == Kind::CrafterBlock {
            self.crafter_result(env);
        }
    }

    /// `CrafterMenu.refreshRecipeResult`.
    fn crafter_result(&mut self, env: &Env) {
        self.containers[RESULT][0] = env
            .recipes
            .find(&self.containers[MAIN], 3)
            .map(|(_, stack)| stack)
            .filter(|s| s.count > 0);
    }

    /// `LoomMenu.setupResultSlot`.
    fn loom_setup_result(&mut self, env: &Env, pattern: &str) {
        let mut result = None;
        if let (Some(banner_stack), Some(dye)) = (
            self.containers[MAIN][0].as_ref(),
            self.containers[MAIN][1].as_ref(),
        ) && let Some(color) = env.banners.loom_dye(env.items, dye)
        {
            result = Some(banner::loom_result(banner_stack, &color, pattern));
        }
        self.containers[RESULT][0] = result;
    }

    /// `LoomMenu.slotsChanged`.
    fn loom_changed(&mut self, env: &Env) {
        let has_inputs = self.containers[MAIN][0].is_some() && self.containers[MAIN][1].is_some();
        if !has_inputs {
            self.containers[RESULT][0] = None;
            self.loom_patterns.clear();
            self.data[0] = -1;
            return;
        }
        let selected = self.data[0];
        let valid = selected >= 0 && (selected as usize) < self.loom_patterns.len();
        let pattern_item = self.containers[MAIN][2].clone();
        let previous = std::mem::replace(
            &mut self.loom_patterns,
            env.banners.selectable(pattern_item.as_ref()),
        );
        let shown = if self.loom_patterns.len() == 1 {
            self.data[0] = 0;
            Some(self.loom_patterns[0].clone())
        } else if !valid {
            self.data[0] = -1;
            None
        } else {
            let value = previous[selected as usize].clone();
            match self.loom_patterns.iter().position(|p| *p == value) {
                Some(i) => {
                    self.data[0] = i as i32;
                    Some(value)
                }
                None => {
                    self.data[0] = -1;
                    None
                }
            }
        };
        match shown {
            Some(pattern) => {
                let full = self.containers[MAIN][0]
                    .as_ref()
                    .is_some_and(banner::has_max_layers);
                if full {
                    self.data[0] = -1;
                    self.containers[RESULT][0] = None;
                } else {
                    self.loom_setup_result(env, &pattern);
                }
            }
            None => self.containers[RESULT][0] = None,
        }
    }

    /// `Slot.setChanged` on slot `s`: the loom recomputes from its inputs,
    /// the block crafter refreshes its result.
    fn slot_changed(&mut self, env: &Env, s: usize) {
        // MerchantContainer.setChanged: every slot of it, the result too.
        if self.kind == Kind::Merchant && self.slots[s].container != PLAYER {
            self.merchant_update(env);
            return;
        }
        if self.slots[s].container != MAIN {
            return;
        }
        match self.kind {
            Kind::Loom => self.loom_changed(env),
            Kind::CrafterBlock => self.crafter_result(env),
            _ => {}
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
            Role::Plain | Role::CreativeDisplay => true,
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
            Role::Output | Role::CraftResult | Role::Inert => false,
            Role::CrafterIn => self.data.get(self.slots[s].index) != Some(&1),
            Role::Potion => {
                env.brewing.is_bottle(stack)
                    || env
                        .items
                        .item_tags
                        .get("minecraft:brewing_potion_inputs")
                        .is_some_and(|m| m.contains(&*stack.item))
            }
            Role::Ingredient => env.brewing.is_reagent(stack),
            Role::BrewFuel => env
                .items
                .get(&stack.item)
                .is_some_and(|i| i.brewing_fuel.is_some()),
            Role::LoomBanner => Self::is_banner(stack),
            Role::LoomDye => env.banners.loom_dye(env.items, stack).is_some(),
            Role::LoomPattern => env.banners.is_pattern_item(env.items, stack),
            Role::LoomResult | Role::MerchantResult => false,
        }
    }

    /// `BannerItem`.
    fn is_banner(stack: &ItemStack) -> bool {
        stack.item.ends_with("_banner")
    }

    fn may_pickup(&self, env: &Env, s: usize) -> bool {
        match self.slots[s].role {
            Role::Inert => false,
            Role::MerchantResult => self
                .merchant
                .as_ref()
                .and_then(|m| m.offers.get(m.active?))
                .is_some_and(|o| {
                    let (a, b) = (
                        self.containers[MAIN][0].as_ref(),
                        self.containers[MAIN][1].as_ref(),
                    );
                    o.satisfied_by(env.items, a, b) || o.satisfied_by(env.items, b, a)
                }),
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
            Role::Armor(_) | Role::Potion => 1,
            Role::Fuel if Self::is_bucket(stack) => return 1,
            _ => CONTAINER_MAX,
        };
        own.min(Self::max_stack(env, stack))
    }

    /// `Container.setItem` (a grid change recomputes the result).
    fn put(&mut self, env: &Env, s: usize, mut stack: Option<ItemStack>) {
        tidy(&mut stack);
        let slot = self.slots[s];
        // SimpleContainer / BaseContainerBlockEntity.setItem: limitSize to
        // getMaxStackSize(stack). The loom's result is a SimpleContainer too.
        if (slot.container == MAIN || slot.role == Role::LoomResult)
            && let Some(st) = stack.as_mut()
        {
            st.count = st.count.min(CONTAINER_MAX.min(Self::max_stack(env, st)));
        }
        // CrafterBlockEntity.setItem: filling a disabled (empty) slot enables it.
        if self.kind == Kind::CrafterBlock
            && slot.container == MAIN
            && self.data[slot.index] == 1
            && self.containers[MAIN][slot.index].is_none()
        {
            self.data[slot.index] = 0;
        }
        self.containers[slot.container][slot.index] = stack;
        if slot.container == GRID {
            self.recompute(env);
        }
        self.slot_changed(env, s);
    }

    /// `Slot.setByPlayer(stack)`: the slot's item is the previous one.
    fn set_by_player(&mut self, env: &Env, s: usize, stack: Option<ItemStack>) {
        let previous = self.slot(s).cloned();
        self.set_by_player_from(env, s, stack, previous.as_ref());
    }

    /// `Slot.setByPlayer(stack, previous)`: `ArmorSlot` and the inventory
    /// menu's off-hand slot first call the owner's `onEquipItem`, which acts
    /// only when the item changes (`isSameItemSameComponents`).
    fn set_by_player_from(
        &mut self,
        env: &Env,
        s: usize,
        mut stack: Option<ItemStack>,
        previous: Option<&ItemStack>,
    ) {
        tidy(&mut stack);
        if let Some(slot) = self.equipment_slot(s) {
            let same = match (previous.filter(|p| p.count > 0), stack.as_ref()) {
                (None, None) => true,
                (Some(a), Some(b)) => a.same_item_same_components(b),
                _ => false,
            };
            if !same {
                self.equipped.push((slot, stack.clone()));
            }
        }
        self.put(env, s, stack);
    }

    /// The `EquipmentSlot` of an `ArmorSlot` or of `InventoryMenu`'s off-hand
    /// slot (the only menu slot over inventory slot 40).
    fn equipment_slot(&self, s: usize) -> Option<&'static str> {
        let slot = self.slots[s];
        match slot.role {
            Role::Armor(eq) => Some(["feet", "legs", "chest", "head"][eq]),
            _ if slot.container == PLAYER && slot.index == OFFHAND => Some("offhand"),
            _ => None,
        }
    }

    /// `Slot.remove`: the container's `removeItem`.
    fn remove(&mut self, env: &Env, s: usize, amount: u32) -> Option<ItemStack> {
        let slot = self.slots[s];
        // ResultContainer.removeItem takes the whole stack; the loom's result
        // is a SimpleContainer slot and splits.
        if slot.container == RESULT && slot.role != Role::LoomResult {
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
        // SimpleContainer.removeItem calls setChanged when it took something;
        // MerchantContainer.removeItem updates the result after a payment slot.
        if out.is_some() && matches!(self.kind, Kind::Loom | Kind::Merchant) {
            self.slot_changed(env, s);
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
        if self.slots[s].role == Role::MerchantResult {
            self.merchant_take(env);
            return;
        }
        if self.slots[s].role == Role::LoomResult {
            // LoomMenu's result onTake: one banner and one dye are used up.
            self.remove(env, 0, 1);
            self.remove(env, 1, 1);
            if self.containers[MAIN][0].is_none() || self.containers[MAIN][1].is_none() {
                self.data[0] = -1;
            }
            self.sounds.push("minecraft:ui.loom.take_result");
            return;
        }
        if matches!(self.kind, Kind::CrafterBlock) && self.slots[s].container == MAIN {
            // Slot.onTake -> CrafterSlot.setChanged -> the result refresh.
            self.slot_changed(env, s);
            return;
        }
        if self.slots[s].role != Role::CraftResult {
            return;
        }
        let n = self.grid_size;
        let grid = self.containers[GRID].clone();
        if let Some((id, _)) = env.recipes.find(&grid, n) {
            self.crafted.push(format!("minecraft:{id}"));
        }
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
            self.dropped.push(stack.clone());
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
        if !self.may_pickup(env, s) {
            return None;
        }
        if !self.allow_modification(env, s) && max_amount < self.slot(s).map_or(0, |c| c.count) {
            return None;
        }
        let result = self.remove(env, s, amount.min(max_amount))?;
        if self.slot(s).is_none() {
            self.set_by_player_from(env, s, None, Some(&result));
        }
        Some(result)
    }

    fn allow_modification(&self, env: &Env, s: usize) -> bool {
        self.may_pickup(env, s)
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
                self.set_by_player(env, s, Some(part));
            }
            Some(mut current) if current.same_item_same_components(stack) => {
                stack.count -= transferable;
                current.count += transferable;
                self.set_by_player(env, s, Some(current));
            }
            Some(_) => {}
        }
        tidy(input);
    }

    // ---- Inventory ----

    /// `Inventory.getSlotWithRemainingSpace` (selected slot 0, offhand, rest).
    fn slot_with_space(&self, env: &Env, stack: &ItemStack) -> Option<usize> {
        let inv = &self.containers[PLAYER];
        [self.selected, OFFHAND]
            .into_iter()
            .chain(0..36)
            .find(|&i| {
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
                    self.slot_changed(env, dest);
                } else if target.count < max {
                    stack.count -= max - target.count;
                    target.count = max;
                    changed = true;
                    self.slot_changed(env, dest);
                }
            }
        }
        if stack.count > 0 {
            for dest in order(()) {
                if !self.has_item(dest) && self.may_place(env, dest, stack) {
                    let max = self.slot_max(env, dest, stack);
                    let part = split(stack, stack.count.min(max));
                    self.set_by_player(env, dest, Some(part));
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
            Kind::ItemPicker => {
                // ItemPickerMenu.quickMoveStack: a hotbar stack is deleted.
                if index >= total - 9 {
                    self.put(env, index, None);
                }
                return None;
            }
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
            Kind::Brewing => {
                after_take = true;
                let slot = self.slots[index];
                let commit = |m: &mut Self, stack: &ItemStack| {
                    m.containers[slot.container][slot.index] =
                        (stack.count > 0).then(|| stack.clone());
                };
                if index <= 4 {
                    if !self.move_to(env, &mut stack, 5, 41, true) {
                        return None;
                    }
                } else if env
                    .items
                    .get(&clicked.item)
                    .is_some_and(|i| i.brewing_fuel.is_some())
                {
                    let moved = self.move_to(env, &mut stack, 4, 5, false);
                    if moved
                        || (env.brewing.is_reagent(&stack)
                            && !self.move_to(env, &mut stack, 3, 4, false))
                    {
                        commit(self, &stack);
                        return None;
                    }
                } else if env.brewing.is_reagent(&stack) {
                    if !self.move_to(env, &mut stack, 3, 4, false) {
                        return None;
                    }
                } else if env.brewing.is_bottle(&stack)
                    || env
                        .items
                        .item_tags
                        .get("minecraft:brewing_potion_inputs")
                        .is_some_and(|m| m.contains(&*stack.item))
                {
                    if !self.move_to(env, &mut stack, 0, 3, false) {
                        return None;
                    }
                } else if (5..32).contains(&index) {
                    if !self.move_to(env, &mut stack, 32, 41, false) {
                        return None;
                    }
                } else if (32..41).contains(&index) {
                    if !self.move_to(env, &mut stack, 5, 32, false) {
                        return None;
                    }
                } else if !self.move_to(env, &mut stack, 5, 41, false) {
                    return None;
                }
            }
            Kind::Loom => {
                after_take = true;
                if index == 3 {
                    if !self.move_to(env, &mut stack, 4, 40, true) {
                        return None;
                    }
                } else if index > 3 {
                    let target = if Self::is_banner(&stack) {
                        Some(0)
                    } else if env.banners.loom_dye(env.items, &stack).is_some() {
                        Some(1)
                    } else if env.banners.is_pattern_item(env.items, &stack) {
                        Some(2)
                    } else {
                        None
                    };
                    match target {
                        Some(t) => {
                            if !self.move_to(env, &mut stack, t, t + 1, false) {
                                return None;
                            }
                        }
                        None if index < 31 => {
                            if !self.move_to(env, &mut stack, 31, 40, false) {
                                return None;
                            }
                        }
                        None => {
                            if index < 40 && !self.move_to(env, &mut stack, 4, 31, false) {
                                return None;
                            }
                        }
                    }
                } else if !self.move_to(env, &mut stack, 4, 40, false) {
                    return None;
                }
            }
            Kind::Merchant => {
                after_take = true;
                if index == 2 {
                    if !self.move_to(env, &mut stack, 3, 39, true) {
                        return None;
                    }
                    if let Some(m) = self.merchant.as_mut() {
                        m.events.push(crate::merchant::Event::TradeSound);
                    }
                } else if index != 0 && index != 1 {
                    if (3..30).contains(&index) {
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
            Kind::Crafter | Kind::CrafterBlock => {
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
            self.containers[slot.container][slot.index] = None;
            // InventoryMenu: setByPlayer(EMPTY, clicked).
            self.set_by_player_from(env, index, None, Some(&clicked));
        } else {
            self.containers[slot.container][slot.index] = Some(stack.clone());
            self.slot_changed(env, index);
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
                    && self.can_drag_to(s)
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
                            && self.can_drag_to(s)
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
                            self.set_by_player(env, s, Some(placed));
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
                if !self.may_pickup(env, s) {
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

    /// The PICKUP branch of `doClick` (without the bundle overrides), ending
    /// with `slot.setChanged()`.
    fn pickup(&mut self, env: &Env, s: usize, primary: bool) {
        self.pickup_inner(env, s, primary);
        self.slot_changed(env, s);
    }

    fn pickup_inner(&mut self, env: &Env, s: usize, primary: bool) {
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
            Some(clicked) if self.may_pickup(env, s) => match self.carried.take() {
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
                            self.set_by_player(env, s, Some(carried));
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
                if self.may_pickup(env, target) {
                    self.containers[PLAYER][inv_slot] = Some(taken.clone());
                    self.set_by_player(env, target, None);
                    self.on_take(env, target);
                }
            }
            (Some(mut source), None) => {
                if self.may_place(env, target, &source) {
                    let max = self.slot_max(env, target, &source);
                    if source.count > max {
                        let part = split(&mut source, max);
                        self.containers[PLAYER][inv_slot] = Some(source);
                        self.set_by_player(env, target, Some(part));
                    } else {
                        self.containers[PLAYER][inv_slot] = None;
                        self.set_by_player(env, target, Some(source));
                    }
                }
            }
            (Some(mut source), Some(mut taken)) => {
                if self.may_pickup(env, target) && self.may_place(env, target, &source) {
                    let max = self.slot_max(env, target, &source);
                    if source.count > max {
                        let part = split(&mut source, max);
                        self.containers[PLAYER][inv_slot] = Some(source);
                        self.set_by_player(env, target, Some(part));
                        self.on_take(env, target);
                        if !self.inventory_add(env, &mut taken) {
                            self.drop_stack(&taken);
                        }
                    } else {
                        self.containers[PLAYER][inv_slot] = Some(taken);
                        self.set_by_player(env, target, Some(source));
                        self.on_take(env, target);
                    }
                }
            }
            (None, None) => {}
        }
    }

    /// The PICKUP_ALL branch (double click).
    /// `canTakeItemForPickAll`: never the result slots (the loom's is a
    /// plain slot), MerchantMenu's or an item tab's display slots.
    fn can_take_for_pick_all(&self, t: usize) -> bool {
        self.kind != Kind::Merchant
            && self.slots[t].role != Role::CreativeDisplay
            && (self.slots[t].container != RESULT || self.kind == Kind::Loom)
    }

    /// `AbstractContainerScreen.mouseReleased` on a Shift double click:
    /// QUICK_MOVE on every slot of `clicked`'s container that holds
    /// `moved`'s item (`lastQuickMoved`, the stack the first click moved).
    pub fn quick_move_all(
        &mut self,
        env: &Env,
        clicked: usize,
        moved: &ItemStack,
    ) -> Result<(), &'static str> {
        self.quick_move_all_each(env, clicked, moved, &mut |_, _| {})
    }

    /// `quick_move_all`, calling `each` with the menu and the slot after each
    /// QUICK_MOVE (a server's client sends every one of them).
    pub fn quick_move_all_each(
        &mut self,
        env: &Env,
        clicked: usize,
        moved: &ItemStack,
        each: &mut dyn FnMut(&Self, usize),
    ) -> Result<(), &'static str> {
        if clicked >= self.slots.len() || !self.can_take_for_pick_all(clicked) {
            return Ok(());
        }
        let container = self.slots[clicked].container;
        for t in 0..self.slots.len() {
            if self.may_pickup(env, t)
                && self.has_item(t)
                && self.slots[t].container == container
                && self.quick_replace(t, moved, env)
            {
                self.click(env, t as i32, 0, Input::QuickMove)?;
                each(self, t);
            }
        }
        Ok(())
    }

    /// `AbstractContainerScreen.shouldAddSlotToQuickCraft`: whether a drag
    /// of type `ty` over `dragged` slots so far takes slot `s` too.
    pub fn quick_craft_accepts(&self, env: &Env, s: usize, ty: i32, dragged: usize) -> bool {
        let Some(carried) = self.carried.as_ref() else {
            return false;
        };
        s < self.slots.len()
            && (carried.count as usize > dragged || ty == 2)
            && self.quick_replace(s, carried, env)
            && self.may_place(env, s, carried)
            && self.can_drag_to(s)
    }

    /// What a quick craft drag of type `ty` over `slots` shows
    /// (`AbstractContainerScreen.extractSlot` and
    /// `recalculateQuickCraftRemaining`): each slot's would-be count and
    /// whether the slot caps it (a yellow count), then the count left in
    /// hand. Nothing while the drag spans a single slot.
    pub fn quick_craft_preview(
        &self,
        env: &Env,
        ty: i32,
        slots: &[usize],
    ) -> Option<(Vec<Option<(u32, bool)>>, u32)> {
        let carried = self.carried.as_ref()?;
        if slots.len() < 2 {
            return None;
        }
        let carried_max = Self::max_stack(env, carried);
        // AbstractContainerMenu.getQuickCraftPlaceCount.
        let place = match ty {
            0 => carried.count / slots.len() as u32,
            1 => 1,
            2 => carried_max,
            _ => carried.count,
        };
        let mut left = i64::from(if ty == 2 { carried_max } else { carried.count });
        let mut shown = Vec::with_capacity(slots.len());
        for &s in slots {
            let carry = self.slot(s).map_or(0, |i| i.count);
            let max = carried_max.min(self.slot_max(env, s, carried));
            if ty != 2 {
                left -= i64::from((place + carry).min(max)) - i64::from(carry);
            }
            shown.push(
                (s < self.slots.len()
                    && self.quick_replace(s, carried, env)
                    && self.can_drag_to(s))
                .then(|| ((place + carry).min(max), place + carry > max)),
            );
        }
        Some((shown, left.max(0) as u32))
    }

    /// `canDragTo`: ItemPickerMenu keeps drags off its display slots.
    fn can_drag_to(&self, s: usize) -> bool {
        self.slots[s].role != Role::CreativeDisplay
    }

    fn pickup_all(&mut self, env: &Env, s: usize, button: i32) {
        let Some(carried) = self.carried.clone() else {
            return;
        };
        if self.has_item(s) && self.may_pickup(env, s) {
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
                    && self.may_pickup(env, t)
                    && self.can_take_for_pick_all(t)
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

/// `MerchantMenu` and `MerchantContainer` over the menu's `merchant` session.
impl Menu {
    /// `MerchantContainer.updateSellItem`.
    pub fn merchant_update(&mut self, env: &Env) {
        let Some(m) = self.merchant.as_mut() else {
            return;
        };
        let (a, b) = (
            self.containers[MAIN][0].as_ref(),
            self.containers[MAIN][1].as_ref(),
        );
        let (a, b) = if a.is_none() { (b, None) } else { (a, b) };
        m.active = None;
        if a.is_none() {
            self.containers[RESULT][0] = None;
            m.future_xp = 0;
            return;
        }
        if !m.offers.is_empty() {
            let (active, result) =
                crate::merchant::sell_item(&m.offers, env.items, a, b, Some(m.selection_hint));
            m.active = active;
            m.future_xp = if result.is_some() {
                active.map_or(0, |i| m.offers[i].xp)
            } else {
                0
            };
            self.containers[RESULT][0] = result;
        }
        let has = self.containers[RESULT][0].is_some();
        m.notify_trade_updated(has);
    }
    /// `MerchantResultSlot.onTake`: the payment is taken and the merchant
    /// told (`notifyTrade`).
    fn merchant_take(&mut self, env: &Env) {
        let Some(m) = self.merchant.as_mut() else {
            return;
        };
        let Some(i) = m.active else { return };
        let Some(offer) = m.offers.get(i).cloned() else {
            return;
        };
        let (mut a, mut b) = (
            self.containers[MAIN][0].take(),
            self.containers[MAIN][1].take(),
        );
        let took = offer.take(env.items, &mut a, &mut b) || offer.take(env.items, &mut b, &mut a);
        // The payment stacks shrink in place, before the merchant hears of it.
        self.containers[MAIN][0] = a;
        self.containers[MAIN][1] = b;
        if took {
            let prices = m.notify_trade(i, env.trade, env.items);
            if prices {
                self.merchant_update(env);
            }
            // setItem(0, buyA), setItem(1, buyB).
            self.merchant_update(env);
            self.merchant_update(env);
        }
    }
    /// `MerchantMenu.tryMoveItems` (the select trade packet, with the
    /// selection hint set first).
    pub fn merchant_select(&mut self, env: &Env, index: usize) {
        let Some(m) = self.merchant.as_mut() else {
            return;
        };
        // MerchantContainer.setSelectionHint updates the result.
        m.selection_hint = index;
        let count = m.offers.len();
        self.merchant_update(env);
        if index >= count {
            return;
        }
        for slot in [0usize, 1] {
            if let Some(mut old) = self.containers[MAIN][slot].clone() {
                if !self.move_to(env, &mut old, 3, 39, true) {
                    return;
                }
                self.containers[MAIN][slot] = (old.count > 0).then_some(old);
                self.put(env, slot, self.containers[MAIN][slot].clone());
            }
        }
        if self.containers[MAIN][0].is_some() || self.containers[MAIN][1].is_some() {
            return;
        }
        let Some(offer) = self
            .merchant
            .as_ref()
            .and_then(|m| m.offers.get(index))
            .cloned()
        else {
            return;
        };
        self.merchant_fill(env, 0, &offer.base_cost_a);
        if let Some(b) = &offer.cost_b {
            self.merchant_fill(env, 1, b);
        }
    }
    /// `MerchantMenu.moveFromInventoryToPaymentSlot`.
    fn merchant_fill(&mut self, env: &Env, payment: usize, cost: &crate::merchant::ItemCost) {
        for i in 3..39 {
            let Some(have) = self.slot(i).cloned() else {
                continue;
            };
            if !cost.test(&have) {
                continue;
            }
            let current = self.containers[MAIN][payment].clone();
            if current
                .as_ref()
                .is_some_and(|c| !c.same_item_same_components(&have))
            {
                continue;
            }
            let max = Self::max_stack(env, &have);
            let current_count = current.as_ref().map_or(0, |c| c.count);
            let moved = max.saturating_sub(current_count).min(have.count);
            let payment_stack = ItemStack {
                count: current_count + moved,
                ..have.clone()
            };
            let slot = self.slots[i];
            let mut left = have;
            left.count -= moved;
            self.containers[slot.container][slot.index] = (left.count > 0).then_some(left);
            self.put(env, payment, Some(payment_stack.clone()));
            if payment_stack.count >= max {
                break;
            }
        }
    }
    /// `MerchantMenu.removed` for a living player: the payment goes back to
    /// the inventory (`placeItemBackInInventory`), or is dropped.
    pub fn merchant_close(&mut self, env: &Env) {
        // AbstractContainerMenu.removed: the carried stack first.
        if let Some(mut carried) = self.carried.take() {
            self.place_back(env, &mut carried);
        }
        for slot in [0usize, 1] {
            if let Some(mut stack) = self.containers[MAIN][slot].take() {
                self.place_back(env, &mut stack);
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{banner::Banners, brewing::Brewing, crafting::Recipes};

    fn stack(item: &str, count: u32) -> Option<ItemStack> {
        Some(ItemStack {
            item: format!("minecraft:{item}").into(),
            count,
            damage: 0,
            extra: None,
        })
    }

    /// An item tab's ItemPickerMenu: the 45 display slots show `shown`.
    fn with_picker(shown: &str, test: impl FnOnce(&Env, &mut Menu)) {
        with_menu(Kind::ItemPicker, |env, menu| {
            assert_eq!(menu.slot_count(), 54);
            for i in 0..45 {
                menu.set_slot(env, i, stack(shown, 1));
            }
            test(env, menu);
        });
    }

    /// A menu over the game's data (`.local/horizon-client`); skipped without it.
    pub(crate) fn with_menu(kind: Kind, test: impl FnOnce(&Env, &mut Menu)) {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("items.json").exists() {
            return;
        }
        let items = Items::catalog(&root).unwrap();
        let recipes = Recipes::load(&root, &items).unwrap();
        let brewing = Brewing::load(&root, &items);
        let banners = Banners::load(&root);
        let env = Env {
            items: &items,
            recipes: &recipes,
            brewing: &brewing,
            banners: &banners,
            trade: None,
        };
        let mut menu = Menu::new(kind);
        menu.creative = true;
        test(&env, &mut menu);
    }

    /// `onEquipItem` runs from the armour and off-hand slots' `setByPlayer`
    /// when the item changes: never from `Inventory.setItem` (a swap's source).
    #[test]
    fn armour_and_off_hand_slots_report_what_is_equipped() {
        with_menu(Kind::Inventory, |env, menu| {
            menu.creative = false;
            let name = |e: &[(&str, Option<ItemStack>)]| {
                e.iter()
                    .map(|(slot, s)| format!("{slot}:{}", s.as_ref().map_or("-", |s| &*s.item)))
                    .collect::<Vec<_>>()
            };
            // A shift-click puts the helmet on (InventoryMenu.quickMoveStack).
            menu.set_slot(env, 9, stack("iron_helmet", 1));
            menu.click(env, 9, 0, Input::QuickMove).unwrap();
            assert_eq!(name(&menu.equipped), ["head:minecraft:iron_helmet"]);
            // Taking it off; the same helmet back on is a change both ways.
            menu.equipped.clear();
            menu.click(env, 5, 0, Input::Pickup).unwrap();
            menu.click(env, 5, 0, Input::Pickup).unwrap();
            assert_eq!(
                name(&menu.equipped),
                ["head:-", "head:minecraft:iron_helmet"]
            );
            // F over a hotbar slot: the shield reaches the off hand by
            // Inventory.setItem, so nothing; a number key onto the off-hand
            // slot sets it by player.
            menu.equipped.clear();
            menu.set_slot(env, 36, stack("shield", 1));
            menu.click(env, 36, 40, Input::Swap).unwrap();
            assert_eq!(menu.slot(45).map(|s| &*s.item), Some("minecraft:shield"));
            assert!(menu.equipped.is_empty());
            menu.click(env, 45, 1, Input::Swap).unwrap();
            assert_eq!(name(&menu.equipped), ["offhand:-"]);
            // Other menus have no equipment slots.
            let mut chest = Menu::new(Kind::Chest(1));
            chest.set_slot(env, 0, stack("iron_helmet", 1));
            chest.click(env, 0, 0, Input::QuickMove).unwrap();
            assert!(chest.equipped.is_empty());
        });
    }

    #[test]
    fn a_quick_craft_drag_previews_each_slot_and_the_rest_in_hand() {
        with_menu(Kind::Inventory, |env, menu| {
            // 9 + 36 player slots: 9 is the first main inventory slot.
            menu.set_slot(env, 10, stack("stone", 60));
            menu.set_slot(env, 11, stack("dirt", 1));
            menu.carried = stack("stone", 20);
            // shouldAddSlotToQuickCraft: same item or empty, placeable, not the result.
            assert!(menu.quick_craft_accepts(env, 9, 0, 0));
            assert!(menu.quick_craft_accepts(env, 10, 0, 1));
            assert!(!menu.quick_craft_accepts(env, 11, 0, 1));
            assert!(!menu.quick_craft_accepts(env, 0, 0, 1));
            // Stone has no place in an armor slot (5).
            assert!(!menu.quick_craft_accepts(env, 5, 0, 1));
            // A right drag stops once every carried item has a slot.
            assert!(!menu.quick_craft_accepts(env, 12, 1, 20));
            // A left drag over three slots: 6 each, the 60 capped at 64 (yellow), 2 + 4 left.
            assert_eq!(menu.quick_craft_preview(env, 0, &[9]), None);
            let (shown, left) = menu.quick_craft_preview(env, 0, &[9, 10, 12]).unwrap();
            assert_eq!(
                shown,
                vec![Some((6, false)), Some((64, true)), Some((6, false))]
            );
            assert_eq!(left, 20 - 6 - 4 - 6);
            // A right drag: one each.
            let (shown, left) = menu.quick_craft_preview(env, 1, &[9, 10]).unwrap();
            assert_eq!(shown, vec![Some((1, false)), Some((61, false))]);
            assert_eq!(left, 18);
        });
    }

    #[test]
    fn item_picker_quick_move_deletes_a_hotbar_stack() {
        with_picker("stone", |env, menu| {
            menu.set_slot(env, 45, stack("dirt", 12));
            menu.set_slot(env, 46, stack("dirt", 5));
            menu.click(env, 45, 0, Input::QuickMove).unwrap();
            assert_eq!(menu.player_item(0), None);
            // Nothing moves anywhere: the other hotbar stack and the tab stay.
            assert_eq!(menu.player_item(1), stack("dirt", 5).as_ref());
            assert_eq!(menu.slot(0), stack("stone", 1).as_ref());
            assert_eq!(menu.carried, None);
        });
    }

    #[test]
    fn item_picker_drags_and_pickup_all_skip_the_display_slots() {
        with_picker("dirt", |env, menu| {
            // A right-drag over a display slot and two hotbar slots.
            menu.carried = stack("dirt", 10);
            for (slot, header) in [(-999, 0), (3, 1), (45, 1), (47, 1), (-999, 2)] {
                menu.click(env, slot, header | (1 << 2), Input::QuickCraft)
                    .unwrap();
            }
            assert_eq!(menu.player_item(0), stack("dirt", 1).as_ref());
            assert_eq!(menu.player_item(2), stack("dirt", 1).as_ref());
            assert_eq!(menu.slot(3), stack("dirt", 1).as_ref());
            assert_eq!(menu.carried, stack("dirt", 8));
            // A double click gathers from the hotbar only.
            menu.click(env, 48, 0, Input::PickupAll).unwrap();
            assert_eq!(menu.carried, stack("dirt", 10));
            assert_eq!(menu.player_item(0), None);
            assert_eq!(menu.slot(0), stack("dirt", 1).as_ref());
        });
    }
}
