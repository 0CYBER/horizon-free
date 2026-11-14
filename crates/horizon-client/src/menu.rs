//! `AbstractContainerMenu.clicked` over vanilla's slot layouts: every
//! `ContainerInput` (pickup, quick move, swap, clone, throw, the three quick
//! craft drags, pickup all) for the player inventory, the crafting table, the
//! chest family, hoppers, dispensers, shulker boxes, the furnaces, the brewing
//! stand, the crafter (its client menu and the block entity's server menu with
//! `setSlotState`), the loom (`clickMenuButton`, `slotsChanged`), the
//! stonecutter (its recipe buttons and result) and the merchant (`MerchantMenu` over a `merchant::Session`: payment slots, the
//! result slot's trade, `tryMoveItems`).
//! `tools/test_menu_click_parity.py` compares every click with vanilla's menu
//! classes.
use crate::{
    banner::{self, Banners},
    brewing::Brewing,
    crafting::Recipes,
    items::{ItemStack, Items},
};
#[cfg(test)]
use horizon_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs};
use horizon_world::native_load_context::{LoopContext, LoopSide};
pub use mod_api::item::menu::{LogicalSide, MenuButtonContext};
mod models;
pub use models::{
    AnvilCraft, AnvilUpdate, AnvilVanilla, CarriedAccess, GrindstonePlace, GrindstoneTake,
    SlotRules, SlotView, StackedOnOther,
};
use std::sync::Arc;

mod bundle;
pub use bundle::fits_inside_container_items;
mod click;
mod storage;
pub use storage::{MutationStamp, Snapshot};
mod equip_movement;
mod equippable;
pub mod place;
mod remainder;

/// `ServerPlayer.getRecipeBook`: whether the player knows a recipe.
pub trait KnownRecipes {
    fn knows(&self, recipe_id: &str) -> bool;
}

impl KnownRecipes for crate::recipe_book::Settings {
    fn knows(&self, recipe_id: &str) -> bool {
        let q = crate::crafting::qualified(recipe_id);
        self.recipes.contains(&q) || self.recipes.contains(recipe_id)
    }
}

impl KnownRecipes for std::collections::BTreeSet<String> {
    fn knows(&self, recipe_id: &str) -> bool {
        let q = crate::crafting::qualified(recipe_id);
        self.contains(&q) || self.contains(recipe_id)
    }
}

impl KnownRecipes for std::collections::HashSet<String> {
    fn knows(&self, recipe_id: &str) -> bool {
        let q = crate::crafting::qualified(recipe_id);
        self.contains(&q) || self.contains(recipe_id)
    }
}

impl KnownRecipes for [String] {
    fn knows(&self, recipe_id: &str) -> bool {
        let q = crate::crafting::qualified(recipe_id);
        self.iter().any(|r| r == &q || r == recipe_id)
    }
}

impl KnownRecipes for Vec<String> {
    fn knows(&self, recipe_id: &str) -> bool {
        self.as_slice().knows(recipe_id)
    }
}

impl<const N: usize> KnownRecipes for [&str; N] {
    fn knows(&self, recipe_id: &str) -> bool {
        let q = crate::crafting::qualified(recipe_id);
        self.iter().any(|r| *r == q.as_str() || *r == recipe_id)
    }
}

impl KnownRecipes for &[&str] {
    fn knows(&self, recipe_id: &str) -> bool {
        let q = crate::crafting::qualified(recipe_id);
        self.iter().any(|r| *r == q.as_str() || *r == recipe_id)
    }
}

/// What a menu needs to know about items and recipes.
#[derive(Clone, Copy)]
pub struct Env<'a> {
    /// Authoritative stack query; absent for data-only consumers.
    pub crafting_remainder: Option<&'a dyn crate::crafting::CraftingRemainder>,
    /// UUID of the player operating this menu, when a player is available.
    pub player: Option<u128>,
    pub player_entity_id: Option<i32>,
    /// The side and settings of the resource generation this menu runs in: the
    /// one source of its side, and what a hook inside a menu loop is given.
    pub loop_context: &'a LoopContext,
    /// `IItemExtension.canEquip` for the armour slots, lent by the game (none keeps the data's).
    pub can_equip: Option<&'a dyn crate::mobs::equip::CanEquip>,
    pub items: &'a Items,
    pub recipes: &'a Recipes,
    pub brewing: &'a Brewing,
    pub banners: &'a Banners,
    /// The stonecutter's recipes.
    pub station_recipes: &'a crate::stations::StationRecipes,
    /// The grindstone's and the anvil's enchantment rules.
    pub enchantments: &'a crate::enchant::Enchantments,
    /// Item display names: the anvil compares the rename box with the
    /// input's hover name.
    pub names: &'a crate::creative::Names,
    /// What a merchant menu needs for offers made while it is open (level-ups).
    pub trade: Option<&'a crate::merchant::TradeEnv<'a>>,
    /// The level's maps (`ContainerLevelAccess`): the cartography table reads
    /// and makes them. None is a client's menu (`ContainerLevelAccess.NULL`):
    /// it makes no result, and a taken result keeps its map id.
    pub maps: Option<&'a dyn Maps>,
    /// `GameRules.LIMITED_CRAFTING`: when true, only known or special recipes craft.
    pub limited_crafting: bool,
    /// The recipes the player knows (`ServerPlayer.getRecipeBook().contains(id)`).
    pub known_recipes: Option<&'a dyn KnownRecipes>,
    /// The mods' `menu_slot_rules`, lent by the game with the read-only `Host` behind it (none
    /// keeps every slot's own answer: a data-only consumer, or a click made without the game).
    pub slot_rules: Option<&'a dyn SlotRules>,
}

/// The maps of the level a menu is in.
pub trait Maps {
    /// Stage derived maps and id reservations with a menu edit.
    fn with_staged(&self, edit: &mut dyn FnMut(&dyn Maps) -> bool) -> bool;
    /// `MapItem.getSavedData`: whether map `id` is locked, and its scale;
    /// None when the level has no such map.
    fn state(&self, id: i32) -> Option<(bool, u8)>;
    /// `MapItem.scaleMap` (`scale`) or `lockMap`: the copy of map `id` the
    /// level makes under a new id (`getFreeMapId`), None without the map.
    fn derive(&self, id: i32, scale: bool) -> Option<i32>;
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
    /// `StonecutterMenu`: the input, the result, then the inventory.
    Stonecutter,
    /// `GrindstoneMenu`: two inputs, the result, then the inventory.
    Grindstone,
    /// `AnvilMenu`: two inputs, the result, then the inventory.
    Anvil,
    /// `SmithingMenu`: template, base, addition, the result, then the inventory.
    Smithing,
    /// `EnchantmentMenu`: the item, the lapis, then the inventory; its data
    /// slots are the three costs, the seed and the clues.
    Enchanting,
    /// `BeaconMenu`: the payment slot, then the inventory; its data slots are
    /// the pyramid's levels and the two powers (`encodeEffect`: id + 1).
    Beacon,
    /// `CartographyTableMenu`: the map, the paper / map / glass pane, the
    /// result, then the inventory.
    Cartography,
    /// `LecternMenu` over the `LecternBlockEntity`: the book's one slot (no
    /// inventory slots); its data slot is the open page.
    Lectern,
    /// `MerchantMenu`: two payment slots, the result, then the inventory.
    Merchant,
    /// `HorseInventoryMenu`: the saddle and body armour slots (`ArmorSlot`s
    /// of the mount, whose entity type is `Menu::mount`), a chest of three
    /// rows of this many columns (`getInventoryColumns`), then the inventory.
    Mount(usize),
    /// `CreativeModeInventoryScreen.ItemPickerMenu`: an item tab's 45
    /// display slots, then the hotbar.
    ItemPicker,
    /// A content mod's menu type: its slots, then the inventory.
    Mod(&'static crate::containers::ModMenuType),
}

impl Kind {
    pub fn parse(name: &str) -> Option<Self> {
        if let Some(rows) = name.strip_prefix("chest") {
            return rows.parse().ok().map(Self::Chest);
        }
        if let Some(columns) = name.strip_prefix("mount") {
            return columns.parse().ok().map(Self::Mount);
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
            "stonecutter" => Self::Stonecutter,
            "grindstone" => Self::Grindstone,
            "anvil" => Self::Anvil,
            "smithing" => Self::Smithing,
            "enchanting" => Self::Enchanting,
            "beacon" => Self::Beacon,
            "cartography" => Self::Cartography,
            "lectern" => Self::Lectern,
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
    /// The stonecutter's result slot (`onTake` uses up one input).
    StoneResult,
    /// The grindstone's inputs: damageable or enchanted items.
    GrindInput,
    /// The grindstone's result slot (`onTake` clears both inputs).
    GrindResult,
    /// A smithing table input: its recipe property set (template, base, addition).
    SmithInput(usize),
    /// `ItemCombinerMenu`'s result slot (the anvil's and the smithing table's).
    CombineResult,
    /// The enchanting table's item slot (one item).
    EnchantItem,
    /// The enchanting table's lapis slot.
    Lapis,
    /// `BeaconMenu.PaymentSlot`: one beacon payment item.
    BeaconPay,
    /// The cartography table's map slot (anything with a map id).
    CartoMap,
    /// Its paper, empty map or glass pane slot.
    CartoExtra,
    /// Its result (`onTake`: one of each input, the map post-processed).
    CartoResult,
    /// `CustomCreativeSlot`: a creative tab's display slot, which the
    /// screen clicks itself; drags and PICKUP_ALL pass it by.
    CreativeDisplay,
    /// An `ArmorSlot` of a mount for this `EquipmentSlot` ("saddle" or
    /// "body"): `isEquippableInSlot` for `Menu::mount`, one item.
    MountEquip(&'static str),
    /// A mod menu type's slot.
    Mod,
    /// A mod menu type's result (`results`): players only take from it.
    ModResult,
    /// A slot the mod's screen leaves out: no click reaches it.
    Hidden,
}

#[derive(Clone, Copy)]
struct Slot {
    container: usize,
    index: usize,
    role: Role,
}

struct MenuSlotView<'a, 'e> {
    menu: &'a mut Menu,
    env: &'a Env<'e>,
    slot: usize,
}

impl SlotView for MenuSlotView<'_, '_> {
    fn with_staged(&mut self, edit: &mut dyn FnMut(&mut dyn SlotView) -> bool) -> bool {
        let mut menu = self.menu.clone();
        let mut run = |maps: Option<&dyn Maps>, maker: Option<&dyn crate::merchant::MapMaker>| {
            let trade = self.env.trade.map(|trade| crate::merchant::TradeEnv {
                maps: maker,
                ..*trade
            });
            let env = Env {
                maps,
                trade: trade.as_ref(),
                ..*self.env
            };
            edit(&mut MenuSlotView {
                menu: &mut menu,
                env: &env,
                slot: self.slot,
            })
        };
        let mut with_maps = |maker: Option<&dyn crate::merchant::MapMaker>| match self.env.maps {
            Some(maps) => maps.with_staged(&mut |maps| run(Some(maps), maker)),
            None => run(None, maker),
        };
        let accepted = match self.env.trade.and_then(|trade| trade.maps) {
            Some(maker) => maker.with_staged(&mut |maker| with_maps(Some(maker))),
            None => with_maps(None),
        };
        if accepted {
            self.menu.install_state(menu);
        }
        accepted
    }
    fn items(&self) -> &crate::items::Items {
        self.env.items
    }

    fn item(&self) -> Option<ItemStack> {
        self.menu.slot(self.slot).cloned()
    }
    fn may_place(&self, stack: &ItemStack) -> bool {
        self.menu.may_place(self.env, self.slot, stack)
    }
    fn may_pickup(&self) -> bool {
        self.menu.may_pickup(self.env, self.slot)
    }
    fn allow_modification(&self) -> bool {
        if self.menu.slot(self.slot).is_some() {
            self.menu.allow_modification(self.env, self.slot)
        } else {
            let empty = ItemStack::new("minecraft:air", 0);
            self.may_pickup() && self.may_place(&empty)
        }
    }
    fn safe_insert(&mut self, mut stack: Option<ItemStack>, count: u32) -> Option<ItemStack> {
        self.menu
            .safe_insert(self.env, self.slot, &mut stack, count);
        stack
    }
    fn safe_take(&mut self, count: u32, max: u32) -> Option<ItemStack> {
        self.menu.safe_take(self.env, self.slot, count, max)
    }
}

/// NeoForge's `ItemStackedOnOtherEvent`, before either item override.
#[inline(never)]
pub fn native_item_stacked_on_other(
    context: &LoopContext,
    event: &mut StackedOnOther<'_>,
    canceled: &mut bool,
) {
    crate::native_client_hooks::item_stacked_on_other(context, event, canceled);
}

/// `BeaconBlockEntity.BEACON_EFFECTS`: the powers of each pyramid level.
pub const BEACON_EFFECTS: [&[&str]; 4] = [
    &["minecraft:speed", "minecraft:haste"],
    &["minecraft:resistance", "minecraft:jump_boost"],
    &["minecraft:strength"],
    &["minecraft:regeneration"],
];

/// `BeaconBlockEntity.getRequiredLevelsFor`: 0 for no power, `i32::MAX`
/// for one no level gives.
pub fn beacon_required_levels(effect: Option<&str>) -> i32 {
    let Some(effect) = effect else {
        return 0;
    };
    BEACON_EFFECTS
        .iter()
        .position(|tier| tier.contains(&effect))
        .map_or(i32::MAX, |i| i as i32 + 1)
}

/// The containers a menu spans.
pub(crate) const PLAYER: usize = 0;
pub(crate) const MAIN: usize = 1;
const GRID: usize = 2;
const RESULT: usize = 3;
/// `Container.getMaxStackSize`.
const CONTAINER_MAX: u32 = 99;
const OFFHAND: usize = 40;

pub struct Menu {
    state: storage::State,
    stamp: storage::MutationStamp,
}

/// `AnvilMenu`'s state besides its slots and `cost`.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct Anvil {
    /// `itemName`: the rename box's text (`ServerboundRenameItemPacket`).
    pub name: Option<String>,
    /// `repairItemCountCost`.
    pub repair_count: u32,
    /// `onlyRenaming`.
    pub only_renaming: bool,
}

/// An anvil menu before a click, for `Menu::anvil_cancel`: the anvil's three
/// slots and cost, the player's slots and the cursor.
#[derive(Clone, Debug, PartialEq)]
pub struct AnvilUndo {
    inputs: Vec<Option<ItemStack>>,
    result: Option<ItemStack>,
    cost: i32,
    player: Vec<Option<ItemStack>>,
    carried: Option<ItemStack>,
}

/// `StonecutterMenu`'s state besides its slots and `selectedRecipeIndex`.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct Stonecutter {
    /// `recipesForInput`: indices into `StationRecipes.stonecutting`.
    pub recipes: Vec<usize>,
    /// The item the list was made for (`input`; None for nothing).
    pub input: Option<Arc<str>>,
    /// `ResultContainer.recipeUsed`.
    pub used: Option<usize>,
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

/// `LecternBlockEntity.getPageCount`: a written book's pages, else a
/// writable book's.
pub fn book_pages(stack: &ItemStack) -> usize {
    stack.data().map_or(0, |d| match &d.written {
        Some(w) => w.pages.len(),
        None => d.writable_pages.len(),
    })
}

fn tidy(slot: &mut Option<ItemStack>) {
    if slot.as_ref().is_some_and(|s| s.count == 0) {
        *slot = None;
    }
}

/// Apply the scoped rule to a slot's own answers, then enforce its item cap. The rules are asked
/// here, while the click plans, and only for a menu type a part answers; with none lent the
/// slot's answers stand.
pub fn apply_menu_slot_rules(
    rules: Option<&dyn SlotRules>,
    menu: &str,
    slot: usize,
    stack: &ItemStack,
    mut may_place: bool,
    mut max_stack: u32,
    item_max: u32,
) -> (bool, u32) {
    if let Some(rules) = rules.filter(|rules| rules.listens(menu)) {
        rules.rule(menu, slot, stack, &mut may_place, &mut max_stack);
    }
    (may_place, max_stack.clamp(1, CONTAINER_MAX).min(item_max))
}

/// Vanilla quick-craft count for one slot, based on the original carried stack.
pub(crate) fn quick_craft_count(
    kind: u8,
    source: u32,
    slots: usize,
    have: u32,
    max: u32,
    item_max: u32,
) -> u32 {
    let each = match kind {
        0 => (source as f32 / slots as f32).floor() as u32,
        1 => 1,
        2 => item_max,
        _ => source,
    };
    (each + have).min(max)
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
            Kind::Mod(t) => {
                containers[MAIN] = vec![None; t.slots];
                // A crafting grid is `CraftingMenu`'s: its slots over the
                // grid container, its result over the result container.
                let crafting = t.crafting;
                grid_size = crafting.map_or(0, |c| c.size);
                slots.extend((0..t.slots).map(|index| match crafting {
                    Some(c) if c.grid_slots().contains(&index) => Slot {
                        container: GRID,
                        index: index - c.grid,
                        role: Role::Mod,
                    },
                    Some(c) if c.result == index => Slot {
                        container: RESULT,
                        index: 0,
                        role: Role::CraftResult,
                    },
                    _ => Slot {
                        container: MAIN,
                        index,
                        role: match t.results.contains(&index) {
                            true => Role::ModResult,
                            false => Role::Mod,
                        },
                    },
                }));
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
            Kind::Stonecutter => {
                containers[MAIN] = vec![None; 1];
                slots.extend(main(1, Role::Plain));
                slots.push(Slot {
                    container: RESULT,
                    index: 0,
                    role: Role::StoneResult,
                });
                player(&mut slots);
                // selectedRecipeIndex: DataSlot.standalone() starts at 0.
                data = 1;
            }
            Kind::Grindstone => {
                containers[MAIN] = vec![None; 2];
                slots.extend(main(2, Role::GrindInput));
                slots.push(Slot {
                    container: RESULT,
                    index: 0,
                    role: Role::GrindResult,
                });
                player(&mut slots);
            }
            Kind::Anvil => {
                containers[MAIN] = vec![None; 2];
                slots.extend(main(2, Role::Plain));
                slots.push(Slot {
                    container: RESULT,
                    index: 0,
                    role: Role::CombineResult,
                });
                player(&mut slots);
                // cost: DataSlot.standalone() starts at 0.
                data = 1;
            }
            Kind::Smithing => {
                containers[MAIN] = vec![None; 3];
                slots.extend((0..3).map(|index| Slot {
                    container: MAIN,
                    index,
                    role: Role::SmithInput(index),
                }));
                slots.push(Slot {
                    container: RESULT,
                    index: 0,
                    role: Role::CombineResult,
                });
                player(&mut slots);
                // hasRecipeError.
                data = 1;
            }
            Kind::Enchanting => {
                containers[MAIN] = vec![None; 2];
                slots.push(Slot {
                    container: MAIN,
                    index: 0,
                    role: Role::EnchantItem,
                });
                slots.push(Slot {
                    container: MAIN,
                    index: 1,
                    role: Role::Lapis,
                });
                player(&mut slots);
                // costs[3], enchantmentSeed, enchantClue[3], levelClue[3].
                data = 10;
            }
            Kind::Beacon => {
                containers[MAIN] = vec![None; 1];
                slots.extend(main(1, Role::BeaconPay));
                player(&mut slots);
                // levels, primary, secondary.
                data = 3;
            }
            Kind::Cartography => {
                containers[MAIN] = vec![None; 2];
                for (index, role) in [Role::CartoMap, Role::CartoExtra].into_iter().enumerate() {
                    slots.push(Slot {
                        container: MAIN,
                        index,
                        role,
                    });
                }
                slots.push(Slot {
                    container: RESULT,
                    index: 0,
                    role: Role::CartoResult,
                });
                player(&mut slots);
            }
            Kind::Lectern => {
                containers[MAIN] = vec![None; 1];
                slots.extend(main(1, Role::Plain));
                // The open page (LecternBlockEntity.dataAccess).
                data = 1;
            }
            Kind::Mount(columns) => {
                containers[MAIN] = vec![None; 2 + columns * 3];
                for (index, slot) in ["saddle", "body"].into_iter().enumerate() {
                    slots.push(Slot {
                        container: MAIN,
                        index,
                        role: Role::MountEquip(slot),
                    });
                }
                slots.extend((2..2 + columns * 3).map(|index| Slot {
                    container: MAIN,
                    index,
                    role: Role::Plain,
                }));
                player(&mut slots);
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
            stamp: storage::MutationStamp::fresh(),
            state: storage::State {
                kind,
                registry_id: match kind {
                    Kind::Mod(kind) => Some(kind.id.into()),
                    _ => None,
                },
                slots,
                containers,
                grid_size,
                container_limits: Vec::new(),
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
                player_sounds: Vec::new(),
                crafted: Vec::new(),
                item_crafted: Vec::new(),
                crafted_counts: Vec::new(),
                merchant_remove_count: 0,
                brewed: Vec::new(),
                brewed_potion_hook: false,
                brewed_potion_takes: Vec::new(),
                item_smelted_hook: false,
                item_smelted: Vec::new(),
                item_smithed_hook: false,
                item_smithed: Vec::new(),
                output_removed: 0,
                equipped: Vec::new(),
                merchant: None,
                stone: Stonecutter::default(),
                ground: None,
                levels: 0,
                level_change: 0,
                anvil: Anvil::default(),
                combined: false,
                anvil_hook: false,
                anvil_craft: None,
                grind_hook: false,
                grind_take: None,
                grind_xp: -1,
                anvil_made: false,
                enchant_changed: false,
                enchant_button: None,
                may_build: true,
                mount: None,
                sources: Vec::new(),
            },
        }
        .with_initial_data()
    }

    /// Data slots that do not start at 0: the enchanting table's clues are -1.
    fn with_initial_data(mut self) -> Self {
        if self.state.kind == Kind::Enchanting {
            self.state.data[4..].fill(-1);
        }
        self
    }

    pub fn kind(&self) -> Kind {
        self.state.kind
    }

    /// Registry identity of this menu, or the mount sentinel for a mount menu.
    /// Player inventory and the creative picker have no registered menu type.
    pub fn registry_id(&self) -> Option<&str> {
        self.state.registry_id.as_deref()
    }

    /// Set the identity obtained from the open window's menu registry.
    pub fn set_registry_id(&mut self, id: Option<&str>) {
        self.touch();
        self.state.registry_id = id.map(Into::into);
    }

    /// `clickMenuButton`: the loom's pattern buttons and the stonecutter's
    /// recipes. False when refused.
    pub fn button(&mut self, env: &Env, id: i32) -> bool {
        self.touch();
        if self.state.kind == Kind::Lectern {
            return self.lectern_button(env, id);
        }
        if self.state.kind == Kind::Enchanting {
            return self.enchant_click(id);
        }
        if self.state.kind == Kind::Stonecutter {
            // StonecutterMenu.clickMenuButton: an invalid index is taken too.
            if self.state.data[0] == id {
                return false;
            }
            if id >= 0 && (id as usize) < self.state.stone.recipes.len() {
                self.state.data[0] = id;
                self.stone_setup_result(env, id);
            }
            return true;
        }
        if self.state.kind != Kind::Loom || id < 0 || id as usize >= self.state.loom_patterns.len()
        {
            return false;
        }
        self.state.data[0] = id;
        let pattern = self.state.loom_patterns[id as usize].clone();
        self.loom_setup_result(env, &pattern);
        true
    }

    /// `LecternMenu.clickMenuButton`: the page buttons (1 back, 2 forward,
    /// 100 + n to page n) and Take Book (3), which puts the book in the
    /// inventory or drops it.
    fn lectern_button(&mut self, env: &Env, id: i32) -> bool {
        self.touch();
        match id {
            100.. => self.set_data(0, id - 100),
            1 => self.set_data(0, self.state.data[0] - 1),
            2 => self.set_data(0, self.state.data[0] + 1),
            3 if self.state.may_build => {
                // removeItemNoUpdate: the whole book, and onBookItemRemove.
                let book = self.state.containers[MAIN][0].take();
                self.state.data[0] = 0;
                if let Some(mut book) = book
                    && !self.inventory_add(env, &mut book)
                {
                    self.drop_stack(&book);
                }
            }
            _ => return false,
        }
        true
    }

    /// `AbstractContainerMenu.setData`: the lectern's is
    /// `LecternBlockEntity.setPage`, kept within the book's pages.
    pub fn set_data(&mut self, id: usize, value: i32) {
        self.touch();
        if self.state.kind == Kind::Lectern && id == 0 {
            // Mth.clamp(page, 0, pageCount - 1).
            let count = self.state.containers[MAIN][0]
                .as_ref()
                .map_or(0, book_pages) as i32;
            self.state.data[0] = value.max(0).min(count - 1);
            return;
        }
        self.state.data[id] = value;
    }

    /// Container slot `slot` of a mod menu type is left out by its screen.
    pub fn hide_slot(&mut self, slot: usize) {
        self.touch();
        if matches!(self.state.kind, Kind::Mod(_)) && slot < self.state.slots.len() - 36 {
            self.state.slots[slot].role = Role::Hidden;
        }
    }

    /// `CrafterBlockEntity.setSlotState` (the slot state packet): only an
    /// empty slot can be disabled or enabled.
    pub fn set_slot_state(&mut self, slot: usize, enabled: bool) {
        self.touch();
        if self.state.kind == Kind::CrafterBlock
            && slot < 9
            && self.state.containers[MAIN][slot].is_none()
        {
            self.state.data[slot] = i32::from(!enabled);
        }
    }

    /// The menu after a fixture set its slots: the block crafter's slot
    /// listener has refreshed the result (`broadcastChanges`).
    pub fn settle(&mut self, env: &Env) {
        self.touch();
        if self.state.kind == Kind::CrafterBlock {
            self.crafter_result(env);
        }
    }

    /// `CrafterMenu.refreshRecipeResult`.
    fn crafter_result(&mut self, env: &Env) {
        self.touch();
        self.state.containers[RESULT][0] = env
            .recipes
            .find(env.loop_context, &self.state.containers[MAIN], 3)
            .map(|(_, stack)| stack)
            .filter(|s| s.count > 0);
    }

    /// `LoomMenu.setupResultSlot`.
    fn loom_setup_result(&mut self, env: &Env, pattern: &str) {
        self.touch();
        let mut result = None;
        if let (Some(banner_stack), Some(dye)) = (
            self.state.containers[MAIN][0].as_ref(),
            self.state.containers[MAIN][1].as_ref(),
        ) && let Some(color) = env.banners.loom_dye(env.items, dye)
        {
            result = Some(banner::loom_result(banner_stack, &color, pattern));
        }
        self.state.containers[RESULT][0] = result;
    }

    /// `LoomMenu.slotsChanged`.
    fn loom_changed(&mut self, env: &Env) {
        self.touch();
        let has_inputs =
            self.state.containers[MAIN][0].is_some() && self.state.containers[MAIN][1].is_some();
        if !has_inputs {
            self.state.containers[RESULT][0] = None;
            self.state.loom_patterns.clear();
            self.state.data[0] = -1;
            return;
        }
        let selected = self.state.data[0];
        let valid = selected >= 0 && (selected as usize) < self.state.loom_patterns.len();
        let pattern_item = self.state.containers[MAIN][2].clone();
        let previous = std::mem::replace(
            &mut self.state.loom_patterns,
            env.banners.selectable(pattern_item.as_ref()),
        );
        let shown = if self.state.loom_patterns.len() == 1 {
            self.state.data[0] = 0;
            Some(self.state.loom_patterns[0].clone())
        } else if !valid {
            self.state.data[0] = -1;
            None
        } else {
            let value = previous[selected as usize].clone();
            match self.state.loom_patterns.iter().position(|p| *p == value) {
                Some(i) => {
                    self.state.data[0] = i as i32;
                    Some(value)
                }
                None => {
                    self.state.data[0] = -1;
                    None
                }
            }
        };
        match shown {
            Some(pattern) => {
                let full = self.state.containers[MAIN][0]
                    .as_ref()
                    .is_some_and(banner::has_max_layers);
                if full {
                    self.state.data[0] = -1;
                    self.state.containers[RESULT][0] = None;
                } else {
                    self.loom_setup_result(env, &pattern);
                }
            }
            None => self.state.containers[RESULT][0] = None,
        }
    }

    /// `StonecutterMenu.slotsChanged`: a new input item lists its recipes
    /// (`setupRecipeList`), with none selected and no result.
    fn stone_changed(&mut self, env: &Env) {
        self.touch();
        let input = self.state.containers[MAIN][0]
            .as_ref()
            .map(|s| s.item.clone());
        if input == self.state.stone.input {
            return;
        }
        self.state.stone.input = input;
        self.state.data[0] = -1;
        self.state.containers[RESULT][0] = None;
        self.state.stone.recipes = match &self.state.containers[MAIN][0] {
            Some(stack) => env
                .station_recipes
                .stonecutting
                .iter()
                .enumerate()
                .filter(|(_, r)| r.input.test_in(env.loop_context, stack))
                .map(|(i, _)| i)
                .collect(),
            None => Vec::new(),
        };
    }

    /// `StonecutterMenu.setupResultSlot`.
    fn stone_setup_result(&mut self, env: &Env, index: i32) {
        self.touch();
        let recipe = usize::try_from(index)
            .ok()
            .and_then(|i| self.state.stone.recipes.get(i).copied());
        self.state.stone.used = recipe;
        self.state.containers[RESULT][0] =
            recipe.map(|r| env.station_recipes.stonecutting[r].result.clone());
    }

    /// `Slot.setChanged` on slot `s`: the loom recomputes from its inputs,
    /// the stonecutter lists its recipes, the block crafter refreshes its
    /// result.
    fn slot_changed(&mut self, env: &Env, s: usize) {
        self.touch();
        // MerchantContainer.setChanged: every slot of it, the result too.
        if self.state.kind == Kind::Merchant && self.state.slots[s].container != PLAYER {
            self.merchant_update(env);
            return;
        }
        // SmithingMenu.slotsChanged runs for its result container too.
        if self.state.kind == Kind::Smithing && self.state.slots[s].container == RESULT {
            self.smith_error();
            return;
        }
        // CartographyTableMenu: its result container calls slotsChanged too.
        if self.state.kind == Kind::Cartography && self.state.slots[s].container != PLAYER {
            self.carto_changed(env);
            return;
        }
        if self.state.slots[s].container != MAIN {
            return;
        }
        match self.state.kind {
            Kind::Loom => self.loom_changed(env),
            Kind::Stonecutter => self.stone_changed(env),
            // GrindstoneMenu.createResult.
            Kind::Grindstone => {
                self.state.containers[RESULT][0] = crate::stations::grindstone_result(
                    env.enchantments,
                    env.items,
                    self.state.containers[MAIN][0].as_ref(),
                    self.state.containers[MAIN][1].as_ref(),
                );
            }
            Kind::CrafterBlock => self.crafter_result(env),
            Kind::Anvil => self.anvil_result(env),
            Kind::Enchanting => self.enchant_changed(env),
            Kind::Smithing => {
                self.smith_result(env);
                self.smith_error();
            }
            _ => {}
        }
    }

    /// `slotsChanged(player.getInventory())`, which a bundle click calls
    /// (`BundleItem.broadcastChangesOnContainerMenu`): the menus that don't
    /// check which container changed recompute from their inputs.
    fn inventory_slots_changed(&mut self, env: &Env) {
        self.touch();
        match self.state.kind {
            Kind::Inventory | Kind::Crafting => self.recompute(env),
            Kind::Loom => self.loom_changed(env),
            Kind::Stonecutter => self.stone_changed(env),
            Kind::Cartography => self.carto_changed(env),
            Kind::Merchant => self.merchant_update(env),
            // ItemCombinerMenu.slotsChanged only for its inputs; the
            // smithing table's hasRecipeError always.
            Kind::Smithing => self.smith_error(),
            _ => {}
        }
    }

    /// `EnchantmentMenu.clickMenuButton`'s checks: the lapis, the cost and
    /// the levels (creative pays nothing). The enchanting itself needs a
    /// level and is the caller's (`enchant_button`).
    fn enchant_click(&mut self, id: i32) -> bool {
        self.touch();
        let Some(i) = usize::try_from(id).ok().filter(|&i| i < 3) else {
            return false;
        };
        let need = id + 1;
        let lapis = self.state.containers[MAIN][1]
            .as_ref()
            .map_or(0, |l| l.count as i32);
        if lapis < need && !self.state.creative {
            return false;
        }
        if self.state.data[i] <= 0
            || self.state.containers[MAIN][0].is_none()
            || (self.state.levels < need || self.state.levels < self.state.data[i])
                && !self.state.creative
        {
            return false;
        }
        self.state.enchant_button = Some(i);
        true
    }

    /// `EnchantmentMenu.slotsChanged`: no costs or clues without an
    /// enchantable item; with one, a level fills them (`enchant_changed`).
    fn enchant_changed(&mut self, env: &Env) {
        self.touch();
        let enchantable = self.state.containers[MAIN][0]
            .as_ref()
            .is_some_and(|s| crate::stations::is_enchantable(env.enchantments, s));
        if enchantable {
            self.state.enchant_changed = true;
        } else {
            self.state.data[..3].fill(0);
            self.state.data[4..].fill(-1);
        }
    }

    /// `AnvilMenu.createResult`.
    fn anvil_result(&mut self, env: &Env) {
        self.touch();
        let hover = self.state.containers[MAIN][0]
            .as_ref()
            .map(|i| crate::stations::hover_name(env.names, i))
            .unwrap_or_default();
        let out = crate::stations::anvil_result(
            env.enchantments,
            env.items,
            self.state.containers[MAIN][0].as_ref(),
            self.state.containers[MAIN][1].as_ref(),
            self.state.anvil.name.as_deref(),
            &hover,
            self.state.creative,
        );
        // onlyRenaming is reset, repairItemCountCost only where an input is.
        self.state.anvil.only_renaming = out.only_renaming;
        if self.state.containers[MAIN][0].is_some() {
            self.state.anvil.repair_count = out.repair_item_count_cost;
        }
        self.state.data[0] = out.cost;
        self.state.containers[RESULT][0] = out.result;
        self.state.anvil_made = true;
    }

    /// The anvil's result, cost and state as `createResult` left them.
    fn anvil_state(&self) -> crate::stations::AnvilResult {
        crate::stations::AnvilResult {
            result: self.state.containers[RESULT][0].clone(),
            cost: self.state.data[0],
            repair_item_count_cost: self.state.anvil.repair_count,
            only_renaming: self.state.anvil.only_renaming,
        }
    }

    /// Whether `createResult` ran since the last call (an input or the name
    /// changed, even back to what it was, or to no visible change).
    pub fn anvil_created(&mut self) -> bool {
        self.touch();
        std::mem::take(&mut self.state.anvil_made)
    }

    /// `AnvilUpdateEvent`'s question about the result `createResult` made,
    /// or `None` with no left input, where vanilla has no result.
    pub fn anvil_update(&self) -> Option<AnvilUpdate> {
        let left = self.state.containers[MAIN][0].clone()?;
        let right = self.state.containers[MAIN][1].clone();
        Some(AnvilUpdate::new(
            left,
            right,
            self.state.anvil.name.clone(),
            &self.anvil_state(),
        ))
    }

    /// The mods' answer to [`Self::anvil_update`] (see [`AnvilUpdate::answer`]).
    pub fn anvil_updated(&mut self, update: AnvilUpdate, cancel: bool) {
        self.touch();
        let mut state = self.anvil_state();
        update.answer(cancel, &mut state);
        self.state.anvil.only_renaming = state.only_renaming;
        self.state.anvil.repair_count = state.repair_item_count_cost;
        self.state.data[0] = state.cost;
        self.state.containers[RESULT][0] = state.result;
    }

    /// `GrindstoneEvent.OnPlaceItem`'s question about the result
    /// `createResult` just made.
    pub fn grindstone_place(&self) -> GrindstonePlace {
        GrindstonePlace {
            top: self.state.containers[MAIN][0].clone(),
            bottom: self.state.containers[MAIN][1].clone(),
            vanilla_output: self.state.containers[RESULT][0].clone(),
            output: None,
            xp: -1,
        }
    }

    /// The mods' answer to [`Self::grindstone_place`]: a cancel leaves no
    /// result; an output replaces vanilla's, and its `xp` (when 0 or more)
    /// is what taking it gives; otherwise vanilla's result and experience stay.
    pub fn grindstone_placed(&mut self, place: GrindstonePlace, cancel: bool) {
        self.touch();
        self.state.grind_xp = -1;
        if cancel {
            self.state.containers[RESULT][0] = None;
        } else if let Some(output) = place.output {
            self.state.grind_xp = place.xp.max(-1);
            self.state.containers[RESULT][0] = Some(output);
        }
    }

    /// The rest of a taken grindstone result's `onTake` once the mods
    /// answered `grind_take`: cancelled, the inputs stay and nothing is
    /// given. Otherwise the inputs become `new_top` and `new_bottom`, and
    /// the experience is the mods' `xp` when 0 or more (returned, given as
    /// is) or vanilla's (`ground`, rolled by the caller).
    pub fn grindstone_taken(
        &mut self,
        env: &Env,
        take: GrindstoneTake,
        cancel: bool,
    ) -> Option<i32> {
        self.touch();
        if cancel {
            return None;
        }
        if take.xp < 0 {
            self.state.ground = Some(crate::stations::grindstone_xp_base(
                env.enchantments,
                take.top.as_ref(),
                take.bottom.as_ref(),
            ));
        }
        self.put(env, 0, take.new_top);
        self.put(env, 1, take.new_bottom);
        (take.xp >= 0).then_some(take.xp)
    }

    /// `AnvilMenu.setItemName` (`ServerboundRenameItemPacket`): false when
    /// the name is refused or unchanged.
    /// `BeaconMenu.updateEffects` (`ServerboundSetBeaconPacket`): with a
    /// payment, powers `BeaconBlockEntity.validateEffects` allows for the
    /// pyramid's levels (data 0) become data 1 and 2 and the payment is used.
    /// Each power is its `mob_effect` id and name. False when refused.
    pub fn beacon_update(
        &mut self,
        env: &Env,
        primary: Option<(i32, &str)>,
        secondary: Option<(i32, &str)>,
    ) -> bool {
        self.touch();
        if self.state.kind != Kind::Beacon || self.state.containers[MAIN][0].is_none() {
            return false;
        }
        let levels = self.state.data[0];
        let (p, s) = (
            beacon_required_levels(primary.map(|e| e.1)),
            beacon_required_levels(secondary.map(|e| e.1)),
        );
        let valid = !(secondary.is_some() && levels < 4)
            && p <= levels
            && s <= levels
            && p < 4
            && !(s != 0 && s < 4 && primary.map(|e| e.0) != secondary.map(|e| e.0));
        if !valid {
            return false;
        }
        self.state.data[1] = primary.map_or(0, |e| e.0 + 1);
        self.state.data[2] = secondary.map_or(0, |e| e.0 + 1);
        self.remove(env, 0, 1);
        true
    }

    pub fn rename(&mut self, env: &Env, name: &str) -> bool {
        self.touch();
        // StringUtil.filterText, then validateName's length limit.
        let name: String = name
            .chars()
            .filter(|&c| c != '\u{a7}' && c >= ' ' && c != '\u{7f}')
            .collect();
        if self.state.kind != Kind::Anvil
            || name.encode_utf16().count() > 50
            || self.state.anvil.name.as_deref() == Some(&name)
        {
            return false;
        }
        if let Some(result) = self.state.containers[RESULT][0].as_mut() {
            result.data_mut().custom_name = (!name.trim().is_empty())
                .then(|| horizon_core::nbt::Tag::String(name.as_str().into()));
            result.tidy();
        }
        self.state.anvil.name = Some(name);
        self.anvil_result(env);
        true
    }

    /// `SmithingMenu.createResult`.
    fn smith_result(&mut self, env: &Env) {
        self.touch();
        let [a, b, c] = [0, 1, 2].map(|i| self.state.containers[MAIN][i].as_ref());
        self.state.containers[RESULT][0] =
            env.station_recipes
                .smithing_result(env.loop_context, env.enchantments, a, b, c);
    }

    /// `SmithingMenu.slotsChanged`'s `hasRecipeError`: every input filled and no result.
    fn smith_error(&mut self) {
        self.touch();
        let error = self.state.containers[MAIN].iter().all(Option::is_some)
            && self.state.containers[RESULT][0].is_none();
        self.state.data[0] = i32::from(error);
    }

    /// `CartographyTableMenu.slotsChanged` and `setupResultSlot`: a result
    /// only where the level has the map (`access.execute`), with the post
    /// processing its take runs (`MAP_POST_PROCESSING`: 0 lock, 1 scale).
    fn carto_changed(&mut self, env: &Env) {
        self.touch();
        let [map, extra] = [0, 1].map(|i| self.state.containers[MAIN][i].clone());
        if self.state.containers[RESULT][0].is_some() && (map.is_none() || extra.is_none()) {
            self.state.containers[RESULT][0] = None;
            return;
        }
        let (Some(map), Some(extra), Some(maps)) = (map, extra, env.maps) else {
            return;
        };
        let Some((locked, scale)) = map
            .data()
            .and_then(|d| d.map_id)
            .and_then(|id| maps.state(id))
        else {
            return;
        };
        let extendable = env
            .items
            .item_tags
            .get("minecraft:extendable_maps")
            .is_some_and(|t| t.contains(&*map.item));
        let copy = |count: u32, post: Option<u8>| {
            let mut out = ItemStack {
                count,
                ..map.clone()
            };
            if post.is_some() {
                out.data_mut().map_post_processing = post;
            }
            out
        };
        self.state.containers[RESULT][0] = match &*extra.item {
            "minecraft:paper" if extendable && !locked && scale < 4 => Some(copy(1, Some(1))),
            "minecraft:glass_pane" if !locked => Some(copy(1, Some(0))),
            "minecraft:map" => Some(copy(2, None)),
            _ => None,
        };
    }

    /// `MapItem.onCraftedPostProcess`, shared by menu takes and system crafting:
    /// remove the directive and name the level's scaled or locked saved copy.
    pub fn map_crafted(maps: Option<&dyn Maps>, stack: &mut ItemStack) {
        if &*stack.item != "minecraft:filled_map" {
            return;
        }
        let Some(post) = stack.data().and_then(|d| d.map_post_processing) else {
            return;
        };
        stack.data_mut().map_post_processing = None;
        let id = stack.data().and_then(|d| d.map_id);
        if let (Some(maps), Some(id)) = (maps, id)
            && let Some(new) = maps.derive(id, post == 1)
        {
            stack.data_mut().map_id = Some(new);
        }
        stack.tidy();
    }

    pub fn slot_count(&self) -> usize {
        self.state.slots.len()
    }

    pub fn slot(&self, i: usize) -> Option<&ItemStack> {
        let s = self.state.slots[i];
        self.state.containers[s.container][s.index].as_ref()
    }

    /// Load an existing container slot without applying insertion limits.
    pub fn load_slot(&mut self, i: usize, stack: Option<ItemStack>) {
        self.touch();
        let slot = self.state.slots[i];
        self.state.containers[slot.container][slot.index] = stack;
    }

    /// Sets a slot as a test fixture does (`Slot.set`).
    pub fn set_slot(&mut self, env: &Env, i: usize, stack: Option<ItemStack>) {
        self.touch();
        if self.state.kind == Kind::Lectern {
            // LecternBlockEntity.setBook: the book its slot can't set.
            self.state.containers[MAIN][0] = stack;
            return;
        }
        self.put(env, i, stack);
    }

    /// A mod menu type's crafting grid, if it has one.
    pub fn mod_crafting(&self) -> Option<crate::containers::ModCrafting> {
        match self.state.kind {
            Kind::Mod(t) => t.crafting,
            _ => None,
        }
    }

    /// A mod menu's container slots from its block entity, in slot order. A
    /// crafting grid's result is the menu's (`ResultContainer`), never the
    /// block entity's: its slot there is passed by and the result follows the
    /// grid.
    pub fn load_container(&mut self, env: &Env, items: &[Option<ItemStack>]) {
        self.touch();
        let result = self.mod_crafting().map(|c| c.result);
        for (k, stack) in items.iter().enumerate().take(self.state.slots.len() - 36) {
            if Some(k) != result {
                self.put(env, k, stack.clone());
            }
        }
        self.recompute(env);
    }

    /// `load_container` back: the container slots the block entity keeps, a
    /// crafting result's left empty.
    pub fn container_items(&self) -> Vec<Option<ItemStack>> {
        let result = self.mod_crafting().map(|c| c.result);
        (0..self.state.slots.len() - 36)
            .map(|k| (Some(k) != result).then(|| self.slot(k).cloned()).flatten())
            .collect()
    }

    pub fn player_item(&self, i: usize) -> Option<&ItemStack> {
        self.state.containers[PLAYER][i].as_ref()
    }

    pub fn set_player_item(&mut self, i: usize, stack: Option<ItemStack>) {
        self.touch();
        self.state.containers[PLAYER][i] = stack;
    }

    /// `Inventory.add`: the leftover stays in `stack`.
    pub fn give(&mut self, env: &Env, stack: &mut ItemStack) -> bool {
        self.touch();
        self.inventory_add(env, stack)
    }

    /// `Inventory.contains(stack)`.
    pub fn player_contains(&self, stack: &ItemStack) -> bool {
        self.state.containers[PLAYER]
            .iter()
            .flatten()
            .any(|have| have.same_item_same_components(stack))
    }

    // ---- item helpers ----

    fn max_stack(env: &Env, stack: &ItemStack) -> u32 {
        item_max_stack(env.items, stack)
    }

    fn damageable(env: &Env, stack: &ItemStack) -> bool {
        env.items.get(&stack.item).is_some_and(|i| i.max_damage > 0)
    }

    /// `ItemStack.isStackable`.
    fn stackable(env: &Env, stack: &ItemStack) -> bool {
        stackable(env.items, stack)
    }

    fn is_bucket(stack: &ItemStack) -> bool {
        &*stack.item == "minecraft:bucket"
    }

    fn is_fuel(env: &Env, stack: &ItemStack) -> bool {
        env.items.get(&stack.item).is_some_and(|i| i.fuel.is_some())
    }

    // ---- Slot ----

    fn may_place(&self, env: &Env, s: usize, stack: &ItemStack) -> bool {
        if self.state.slots[s].role == Role::Hidden {
            return false;
        }
        let own = self.vanilla_may_place(env, s, stack);
        self.slot_rules(env, s, stack, own, self.vanilla_slot_max(s, stack))
            .0
    }

    fn vanilla_may_place(&self, env: &Env, s: usize, stack: &ItemStack) -> bool {
        match self.state.slots[s].role {
            Role::Plain | Role::CreativeDisplay => true,
            Role::Armor(eq) => Self::armor_may_place(env, eq, stack),
            Role::MountEquip(slot) => self.mount_may_place(env, slot, stack),
            Role::Shulker => fits_inside_container_items(stack),
            Role::Mod | Role::ModResult => self.state.slots[s].role == Role::Mod,
            Role::Hidden => false,
            Role::Fuel => Self::is_fuel(env, stack) || Self::is_bucket(stack),
            Role::Output | Role::CraftResult | Role::Inert => false,
            Role::CrafterIn => self.state.data.get(self.state.slots[s].index) != Some(&1),
            Role::Potion => {
                env.brewing.is_bottle(stack)
                    || env
                        .items
                        .item_tags
                        .get("minecraft:brewing_potion_inputs")
                        .is_some_and(|m| m.contains(&*stack.item))
            }
            Role::Ingredient => env.brewing.is_reagent(stack),
            Role::BrewFuel => crate::brewing::brewing_fuel(env.items, stack).is_some(),
            Role::LoomBanner => Self::is_banner(stack),
            Role::LoomDye => env.banners.loom_dye(env.items, stack).is_some(),
            Role::LoomPattern => env.banners.is_pattern_item(env.items, stack),
            Role::GrindInput => crate::stations::grindstone_accepts(env.items, stack),
            Role::SmithInput(i) => env
                .station_recipes
                .smithing_accepts(env.loop_context, i, stack),
            Role::EnchantItem => true,
            Role::Lapis => &*stack.item == "minecraft:lapis_lazuli",
            Role::BeaconPay => env
                .items
                .item_tags
                .get("minecraft:beacon_payment_items")
                .is_some_and(|t| t.contains(&*stack.item)),
            Role::CartoMap => stack.data().is_some_and(|d| d.map_id.is_some()),
            Role::CartoExtra => Self::is_carto_extra(stack),
            Role::LoomResult
            | Role::MerchantResult
            | Role::StoneResult
            | Role::GrindResult
            | Role::CombineResult
            | Role::CartoResult => false,
        }
    }

    /// What the cartography table's second slot takes.
    fn is_carto_extra(stack: &ItemStack) -> bool {
        matches!(
            &*stack.item,
            "minecraft:paper" | "minecraft:map" | "minecraft:glass_pane"
        )
    }

    /// `BannerItem`.
    fn is_banner(stack: &ItemStack) -> bool {
        stack.item.ends_with("_banner")
    }

    fn may_pickup(&self, env: &Env, s: usize) -> bool {
        match self.state.slots[s].role {
            Role::Inert | Role::Hidden => false,
            // AnvilMenu.mayPickup: the levels to pay, and a price.
            Role::CombineResult if self.state.kind == Kind::Anvil => {
                (self.state.creative || self.state.levels >= self.state.data[0])
                    && self.state.data[0] > 0
            }
            Role::MerchantResult => self
                .merchant
                .as_ref()
                .and_then(|m| m.offers.get(m.active?))
                .is_some_and(|o| {
                    let (a, b) = (
                        self.state.containers[MAIN][0].as_ref(),
                        self.state.containers[MAIN][1].as_ref(),
                    );
                    o.satisfied_by(env.items, a, b) || o.satisfied_by(env.items, b, a)
                }),
            Role::Armor(_) | Role::MountEquip(_) => match self.slot(s) {
                Some(stack) => {
                    self.state.creative || !env.enchantments.prevents_armor_change(stack)
                }
                None => true,
            },
            _ => true,
        }
    }

    /// A result slot (`FurnaceResultSlot`, `ResultSlot`): nothing goes in, and
    /// what's taken from it can't be put back.
    pub fn is_result(&self, s: usize) -> bool {
        matches!(self.state.slots[s].role, Role::Output | Role::CraftResult)
    }

    /// How many of `stack` slot `s` holds: `Slot.getMaxStackSize(stack)` where `Slot.mayPlace`
    /// takes it, else none.
    pub fn slot_holds(&self, env: &Env, s: usize, stack: &ItemStack) -> u32 {
        match self.may_place(env, s, stack) {
            true => self.slot_max(env, s, stack),
            false => 0,
        }
    }

    /// `Slot.mayPickup`: whether slot `s` gives up what it holds.
    pub fn slot_gives(&self, env: &Env, s: usize) -> bool {
        self.may_pickup(env, s)
    }

    /// Slot `s` takes one more item of `stack` (`Slot.mayPlace`, and room).
    pub fn takes_one(&self, env: &Env, s: usize, stack: &ItemStack) -> bool {
        self.may_place(env, s, stack)
            && self.slot(s).is_none_or(|have| {
                have.same_item_same_components(stack) && have.count < self.slot_max(env, s, have)
            })
    }

    /// The slots `quickMoveStack` would put slot `index`'s items in: one item
    /// of a plain slot, the whole of a result slot, partial stacks first. Empty
    /// when they don't all fit. This runs the move, so call it on a menu that
    /// is thrown away afterwards.
    pub fn quick_move_landing(&mut self, env: &Env, index: usize) -> Vec<usize> {
        self.touch();
        let Some(mut stack) = self.slot(index).cloned() else {
            return Vec::new();
        };
        if !self.is_result(index) {
            stack.count = 1;
            let slot = self.state.slots[index];
            self.state.containers[slot.container][slot.index] = Some(stack.clone());
        }
        let before: Vec<Option<ItemStack>> = (0..self.state.slots.len())
            .map(|i| self.slot(i).cloned())
            .collect();
        self.quick_move(env, index);
        let (mut partial, mut empty) = (Vec::new(), Vec::new());
        let mut moved = 0;
        for i in (0..self.state.slots.len()).filter(|&i| i != index && !self.is_result(i)) {
            let had = before[i].as_ref().map_or(0, |b| b.count);
            let got = self
                .slot(i)
                .filter(|now| now.same_item_same_components(&stack))
                .map_or(0, |now| now.count.saturating_sub(had));
            if got > 0 {
                moved += got;
                if had > 0 {
                    partial.push(i);
                } else {
                    empty.push(i);
                }
            }
        }
        if moved < stack.count {
            return Vec::new();
        }
        partial.extend(empty);
        partial
    }

    fn slot_rules(
        &self,
        env: &Env,
        s: usize,
        stack: &ItemStack,
        own: bool,
        max: u32,
    ) -> (bool, u32) {
        let Some(id) = self.registry_id() else {
            return (own, max.min(Self::max_stack(env, stack)));
        };
        apply_menu_slot_rules(
            env.slot_rules,
            id,
            self.state.slots[s].index,
            stack,
            own,
            max,
            Self::max_stack(env, stack),
        )
    }

    /// Capacity overrides for the backing container slots, refreshed before each server edit.
    pub fn set_container_limits(&mut self, limits: Vec<Option<u32>>) {
        self.touch();
        self.state.container_limits = limits;
    }

    /// `Slot.getMaxStackSize(stack)`.
    fn slot_max(&self, env: &Env, s: usize, stack: &ItemStack) -> u32 {
        let max = self
            .slot_rules(
                env,
                s,
                stack,
                self.vanilla_may_place(env, s, stack),
                self.vanilla_slot_max(s, stack),
            )
            .1;
        self.limit_slot_max(s, max)
    }

    fn vanilla_slot_max(&self, s: usize, stack: &ItemStack) -> u32 {
        match self.state.slots[s].role {
            Role::Armor(_)
            | Role::MountEquip(_)
            | Role::Potion
            | Role::EnchantItem
            | Role::BeaconPay => 1,
            // The lectern's bookAccess holds one.
            Role::Plain
                if self.state.kind == Kind::Lectern && self.state.slots[s].container == MAIN =>
            {
                1
            }
            Role::Fuel if Self::is_bucket(stack) => 1,
            Role::Mod | Role::ModResult => CONTAINER_MAX,
            // `Container.getMaxStackSize` is vanilla's 99, which no data pack's
            // item reaches; a native mod's item may, and its own size is the cap.
            _ => u32::MAX,
        }
    }

    /// `Container.setItem` (a grid change recomputes the result).
    fn put(&mut self, env: &Env, s: usize, mut stack: Option<ItemStack>) {
        self.touch();
        tidy(&mut stack);
        let slot = self.state.slots[s];
        // LecternBlockEntity's bookAccess.setItem does nothing.
        if self.state.kind == Kind::Lectern && slot.container == MAIN {
            return;
        }
        // SimpleContainer / BaseContainerBlockEntity.setItem: limitSize to
        // getMaxStackSize(stack). The loom's result is a SimpleContainer too.
        if (slot.container == MAIN || slot.role == Role::LoomResult)
            && let Some(st) = stack.as_mut()
        {
            st.count = st.count.min(Self::max_stack(env, st));
            // The beacon's container holds one item (getMaxStackSize 1).
            if self.state.kind == Kind::Beacon {
                st.count = st.count.min(1);
            }
        }
        // CrafterBlockEntity.setItem: filling a disabled (empty) slot enables it.
        if self.state.kind == Kind::CrafterBlock
            && slot.container == MAIN
            && self.state.data[slot.index] == 1
            && self.state.containers[MAIN][slot.index].is_none()
        {
            self.state.data[slot.index] = 0;
        }
        self.state.containers[slot.container][slot.index] = stack;
        if slot.container == GRID {
            self.recompute(env);
        }
        self.slot_changed(env, s);
    }

    /// `Slot.setByPlayer(stack)`: the slot's item is the previous one.
    fn set_by_player(&mut self, env: &Env, s: usize, stack: Option<ItemStack>) {
        self.touch();
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
        self.touch();
        tidy(&mut stack);
        if let Some(slot) = self.equipment_slot(s) {
            let same = match (previous.filter(|p| p.count > 0), stack.as_ref()) {
                (None, None) => true,
                (Some(a), Some(b)) => a.same_item_same_components(b),
                _ => false,
            };
            if !same {
                self.state.equipped.push((slot, stack.clone()));
            }
        }
        self.put(env, s, stack);
    }

    /// The `EquipmentSlot` of an `ArmorSlot` or of `InventoryMenu`'s off-hand
    /// slot (the only menu slot over inventory slot 40).
    fn equipment_slot(&self, s: usize) -> Option<&'static str> {
        let slot = self.state.slots[s];
        match slot.role {
            Role::Armor(eq) => Some(equippable::ARMOR_SLOTS[eq]),
            _ if slot.container == PLAYER && slot.index == OFFHAND => Some("offhand"),
            _ => None,
        }
    }

    /// `Slot.remove`: the container's `removeItem`.
    fn remove(&mut self, env: &Env, s: usize, amount: u32) -> Option<ItemStack> {
        self.touch();
        let slot = self.state.slots[s];
        if slot.role == Role::MerchantResult {
            let count = amount.min(self.slot(s).map_or(0, |stack| stack.count));
            self.state.merchant_remove_count =
                self.state.merchant_remove_count.wrapping_add(count as i32);
        }
        // ResultContainer.removeItem takes the whole stack; the loom's result
        // is a SimpleContainer slot and splits.
        if slot.container == RESULT && slot.role != Role::LoomResult {
            return self.state.containers[RESULT][0].take();
        }
        let cell = &mut self.state.containers[slot.container][slot.index];
        let out = cell
            .as_mut()
            .filter(|c| c.count > 0)
            .map(|c| split(c, amount));
        tidy(cell);
        // FurnaceResultSlot.remove counts what it gave.
        if slot.role == Role::Output {
            self.state.output_removed += out.as_ref().map_or(0, |o| o.count);
        }
        // LecternBlockEntity: an emptied book slot is onBookItemRemove (page 0).
        if self.state.kind == Kind::Lectern && slot.container == MAIN && cell.is_none() {
            self.state.data[0] = 0;
        }
        if out.is_some() && slot.container == GRID {
            self.recompute(env);
        }
        // SimpleContainer.removeItem calls setChanged when it took something;
        // MerchantContainer.removeItem updates the result after a payment slot.
        if out.is_some()
            && matches!(
                self.state.kind,
                Kind::Loom
                    | Kind::Merchant
                    | Kind::Stonecutter
                    | Kind::Grindstone
                    | Kind::Anvil
                    | Kind::Smithing
                    | Kind::Enchanting
                    | Kind::Cartography
            )
        {
            self.slot_changed(env, s);
        }
        out
    }

    /// The anvil menu as it is before a click that may take its result.
    pub fn anvil_undo(&self) -> AnvilUndo {
        AnvilUndo {
            inputs: self.state.containers[MAIN].clone(),
            result: self.state.containers[RESULT][0].clone(),
            cost: self.state.data[0],
            player: self.state.containers[PLAYER].clone(),
            carried: self.state.carried.clone(),
        }
    }

    /// Refuses a take that `anvil_hook` deferred (a cancelled
    /// `AnvilCraftEvent.Pre`): the result goes back in its slot and the
    /// inputs, cost, inventory and cursor are as `undo` held them, so nothing
    /// the click did (cursor, shift-click, swap, drop) is left.
    pub fn anvil_cancel(&mut self, undo: AnvilUndo) {
        self.touch();
        self.state.containers[MAIN] = undo.inputs;
        self.state.containers[RESULT][0] = undo.result;
        self.state.data[0] = undo.cost;
        self.state.containers[PLAYER] = undo.player;
        self.state.carried = undo.carried;
        self.state.anvil_craft = None;
        self.state.dropped.clear();
    }

    /// `AnvilMenu.onTake`: the levels, the material used up, then the input.
    pub fn anvil_take(&mut self, env: &Env) {
        self.touch();
        let cost = self.state.data[0];
        if !self.state.creative {
            // Player.giveExperienceLevels(-cost).
            self.state.level_change -= cost;
            self.state.drops.push(format!(
                "levels{}{}",
                if cost <= 0 { "+" } else { "" },
                -cost
            ));
        }
        let repair = self.state.anvil.repair_count;
        if repair > 0 {
            let addition = self.state.containers[MAIN][1]
                .clone()
                .filter(|a| a.count > repair)
                .map(|mut a| {
                    a.count -= repair;
                    a
                });
            self.put(env, 1, addition);
        } else if !self.state.anvil.only_renaming {
            self.put(env, 1, None);
        }
        self.state.data[0] = 0;
        self.put(env, 0, None);
        self.state.combined = true;
    }

    /// `CraftingMenu.slotChangedCraftingGrid`.
    #[horizon_mod_api_macros::replaceable]
    fn recompute(&mut self, env: &Env) {
        self.touch();
        if self.state.grid_size == 0 {
            return;
        }
        let result = env
            .recipes
            .find_with(
                env.loop_context,
                &self.state.containers[GRID],
                self.state.grid_size,
                &|id| {
                    env.maps
                        .and_then(|maps| maps.state(id))
                        .map(|(_, scale)| scale)
                },
            )
            .and_then(|(id, stack)| {
                if env.limited_crafting {
                    // `RecipeCraftingHolder.setRecipeUsed`: special recipes or known recipes craft.
                    let is_special = env.recipes.recipe(id).is_some_and(|r| r.is_special());
                    let known = env.known_recipes.is_some_and(|book| book.knows(id));
                    if !is_special && !known {
                        return None;
                    }
                }
                Some(stack)
            });
        self.state.containers[RESULT][0] = result;
    }

    /// `FurnaceResultSlot.checkTakeAchievements`: what `carried` and
    /// `removeCount` were, kept for `ItemSmeltedEvent` when a server caller
    /// listens; the count starts again.
    fn furnace_take(&mut self, env: &Env, carried: &ItemStack) {
        self.touch();
        let removed = std::mem::take(&mut self.state.output_removed);
        if self.state.item_smelted_hook && env.loop_context.side() == LoopSide::Server {
            let carried = (carried.count > 0).then(|| carried.clone());
            self.state.item_smelted.push((carried, removed));
        }
    }

    /// A station's take sound, gated by its menu's `lastSoundTime`: once per
    /// tick, so a shift-click that takes many results plays it once.
    fn take_sound(&mut self, sound: &'static str) {
        self.touch();
        if !self.state.sounds.contains(&sound) {
            self.state.sounds.push(sound);
        }
    }

    /// `ResultSlot.onTake` and the other `Slot.onTake` overrides.
    fn on_take(&mut self, env: &Env, s: usize, taken: &mut ItemStack) {
        self.touch();
        if self.state.slots[s].role == Role::Output {
            self.furnace_take(env, taken);
            return;
        }
        if self.state.slots[s].role == Role::Potion {
            if self.state.brewed_potion_hook && env.loop_context.side() == LoopSide::Server {
                self.state.brewed_potion_takes.push(taken.clone());
            }
            // BrewingStandMenu.PotionSlot.onTake: a stack with potion contents.
            if matches!(
                &*taken.item,
                "minecraft:potion" | "minecraft:splash_potion" | "minecraft:lingering_potion"
            ) {
                self.state.brewed.push(taken.clone());
            }
            return;
        }
        if self.state.slots[s].role == Role::MerchantResult {
            self.merchant_check_take_achievements(taken);
            self.merchant_take(env);
            return;
        }
        if self.state.slots[s].role == Role::StoneResult {
            // StonecutterMenu's result onTake: awardUsedRecipes, one input
            // used up, the result again for the selected recipe.
            if let Some(r) = self.state.stone.used {
                self.state.crafted.push((
                    env.station_recipes.stonecutting[r].id.clone(),
                    vec![self.state.containers[MAIN][0].clone()],
                ));
            }
            if self.remove(env, 0, 1).is_some() {
                self.stone_setup_result(env, self.state.data[0]);
            }
            self.take_sound("minecraft:ui.stonecutter.take_result");
            return;
        }
        if self.state.slots[s].role == Role::GrindResult {
            if self.state.grind_hook {
                self.state.grind_take = Some(GrindstoneTake {
                    top: self.state.containers[MAIN][0].clone(),
                    bottom: self.state.containers[MAIN][1].clone(),
                    xp: self.state.grind_xp,
                    new_top: None,
                    new_bottom: None,
                });
                return;
            }
            // GrindstoneMenu's result onTake: the experience (from the inputs
            // before they go), then both inputs cleared.
            self.state.ground = Some(crate::stations::grindstone_xp_base(
                env.enchantments,
                self.state.containers[MAIN][0].as_ref(),
                self.state.containers[MAIN][1].as_ref(),
            ));
            self.put(env, 0, None);
            self.put(env, 1, None);
            return;
        }
        if self.state.slots[s].role == Role::CombineResult && self.state.kind == Kind::Anvil {
            if self.state.anvil_hook {
                self.state.anvil_craft = Some(AnvilCraft {
                    left: self.state.containers[MAIN][0].clone(),
                    right: self.state.containers[MAIN][1].clone(),
                    output: taken.clone(),
                });
                return;
            }
            self.anvil_take(env);
            return;
        }
        if self.state.slots[s].role == Role::CombineResult {
            // SmithingMenu.onTake: the `ItemSmithedEvent` hook, then
            // awardUsedRecipes, then one of each input.
            if self.state.kind == Kind::Smithing {
                let inputs = [0, 1, 2].map(|i| self.state.containers[MAIN][i].clone());
                if self.state.item_smithed_hook
                    && env.loop_context.side() == LoopSide::Server
                    && taken.count > 0
                {
                    self.state
                        .item_smithed
                        .push((inputs.clone(), taken.clone()));
                }
                if let Some((id, _)) = env.station_recipes.smithing_recipe(
                    env.loop_context,
                    env.enchantments,
                    inputs[0].as_ref(),
                    inputs[1].as_ref(),
                    inputs[2].as_ref(),
                ) {
                    self.state.crafted.push((id.to_owned(), inputs.to_vec()));
                }
            }
            for i in 0..3 {
                if let Some(mut stack) = self.state.containers[MAIN][i].clone() {
                    stack.count -= 1;
                    self.put(env, i, Some(stack));
                }
            }
            self.state.combined = true;
            return;
        }
        if self.state.slots[s].role == Role::CartoResult {
            // CartographyTableMenu's result onTake: one of each input, the
            // take sound, then Slot.onTake's setChanged. The taken stack's
            // onCraftedBy is the caller's (map_crafted).
            self.remove(env, 0, 1);
            self.remove(env, 1, 1);
            self.take_sound("minecraft:ui.cartography_table.take_result");
            self.slot_changed(env, s);
            return;
        }
        if self.state.slots[s].role == Role::LoomResult {
            // LoomMenu's result onTake: one banner and one dye are used up.
            self.remove(env, 0, 1);
            self.remove(env, 1, 1);
            if self.state.containers[MAIN][0].is_none() || self.state.containers[MAIN][1].is_none()
            {
                self.state.data[0] = -1;
            }
            self.take_sound("minecraft:ui.loom.take_result");
            return;
        }
        if matches!(self.state.kind, Kind::CrafterBlock) && self.state.slots[s].container == MAIN {
            // Slot.onTake -> CrafterSlot.setChanged -> the result refresh.
            self.slot_changed(env, s);
            return;
        }
        if self.state.slots[s].role != Role::CraftResult {
            return;
        }
        // ResultSlot.checkTakeAchievements -> Item.onCraftedBy mutates the
        // actual taken stack, before the inputs are consumed.
        if taken.count > 0 {
            Self::map_crafted(env.maps, taken);
        }
        let n = self.state.grid_size;
        let grid = self.state.containers[GRID].clone();
        let map_scale = |id| {
            env.maps
                .and_then(|maps| maps.state(id))
                .map(|(_, scale)| scale)
        };
        if let Some((id, _)) = env
            .recipes
            .find_with(env.loop_context, &grid, n, &map_scale)
        {
            let id = format!("minecraft:{id}");
            let made = (id.clone(), taken.clone(), grid.clone());
            self.state.crafted.push((id, grid.clone()));
            self.state.item_crafted.push(made);
        }
        let remaining = env
            .recipes
            .find_with(env.loop_context, &grid, n, &map_scale)
            .map(|(id, _)| env.recipes.remainders(id, &grid, n, &|s| env.remainder(s)))
            .unwrap_or_else(|| {
                // CraftingRecipe.defaultCraftingReminder
                grid.iter()
                    .map(|cell| cell.as_ref().and_then(|stack| env.remainder(stack)))
                    .collect()
            });
        for cell in 0..n * n {
            let mut here = self.state.containers[GRID][cell].take();
            if let Some(stack) = here.as_mut() {
                stack.count -= 1;
            }
            tidy(&mut here);
            self.state.containers[GRID][cell] = here;
            if let Some(replacement) = remaining.get(cell).cloned().flatten() {
                match self.state.containers[GRID][cell].as_mut() {
                    None => self.state.containers[GRID][cell] = Some(replacement),
                    Some(have) if have.same_item_same_components(&replacement) => {
                        let mut merged = replacement;
                        merged.count += have.count;
                        self.state.containers[GRID][cell] = Some(merged);
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
        self.touch();
        if stack.count > 0 {
            self.state
                .drops
                .push(crate::item_data::canon_stack(Some(stack)));
            self.state.dropped.push(stack.clone());
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
        self.touch();
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
        self.touch();
        let mut result = self.try_remove(env, s, amount, max_amount)?;
        self.on_take(env, s, &mut result);
        if self.state.slots[s].role == Role::CartoResult {
            Self::map_crafted(env.maps, &mut result);
        }
        Some(result)
    }

    /// `Slot.safeInsert`: puts up to `amount` of `input` into the slot.
    fn safe_insert(&mut self, env: &Env, s: usize, input: &mut Option<ItemStack>, amount: u32) {
        self.insert_equip_source(env, s, input, amount);
    }

    // ---- Inventory ----

    /// `Inventory.getSlotWithRemainingSpace` (selected slot 0, offhand, rest).
    fn slot_with_space(&self, env: &Env, stack: &ItemStack) -> Option<usize> {
        slot_with_space(
            &self.state.containers[PLAYER],
            self.state.selected,
            env.items,
            stack,
        )
    }

    /// `Inventory.add(stack)`: the leftover stays in `stack`.
    fn inventory_add(&mut self, env: &Env, stack: &mut ItemStack) -> bool {
        self.touch();
        inventory_add(
            &mut self.state.containers[PLAYER],
            self.state.selected,
            self.state.creative,
            env.items,
            stack,
        )
    }
}

/// `ItemStack.getMaxStackSize`: the stack's component, else the item's; a removed
/// component (`!minecraft:max_stack_size`) uses vanilla's fallback of one.
pub fn item_max_stack(items: &Items, stack: &ItemStack) -> u32 {
    let patched = stack.data().and_then(|d| {
        if d.other("!minecraft:max_stack_size").is_some() {
            return Some(1);
        }
        match d.other("minecraft:max_stack_size") {
            Some(horizon_core::nbt::Tag::Int(n)) if (1..=99).contains(n) => Some(*n as u32),
            _ => None,
        }
    });
    patched.unwrap_or_else(|| items.get(&stack.item).map_or(64, |i| i.max_stack))
}

/// Whether a stack can merge in the player's inventory.
fn stackable(items: &Items, stack: &ItemStack) -> bool {
    item_max_stack(items, stack) > 1
        && !(stack.damage > 0 && items.get(&stack.item).is_some_and(|i| i.max_damage > 0))
}

/// `Inventory.getSlotWithRemainingSpace`, preferring selected and offhand slots.
fn slot_with_space(
    inventory: &[Option<ItemStack>],
    selected: usize,
    items: &Items,
    stack: &ItemStack,
) -> Option<usize> {
    [selected, OFFHAND].into_iter().chain(0..36).find(|&i| {
        inventory[i].as_ref().is_some_and(|have| {
            have.same_item_same_components(stack)
                && stackable(items, have)
                && have.count < item_max_stack(items, have)
        })
    })
}

/// `Inventory.add`; leaves any remainder in `stack`.
pub(crate) fn inventory_add(
    inventory: &mut [Option<ItemStack>],
    selected: usize,
    creative: bool,
    items: &Items,
    stack: &mut ItemStack,
) -> bool {
    if stack.count == 0 {
        return false;
    }
    if items.get(&stack.item).is_some_and(|i| i.max_damage > 0) && stack.damage > 0 {
        if let Some(slot) = inventory[..36].iter().position(Option::is_none) {
            inventory[slot] = Some(stack.clone());
            stack.count = 0;
            return true;
        }
        if creative {
            stack.count = 0;
            return true;
        }
        return false;
    }
    loop {
        let last = stack.count;
        let slot = slot_with_space(inventory, selected, items, stack)
            .or_else(|| inventory[..36].iter().position(Option::is_none));
        if let Some(slot) = slot {
            let cell = inventory[slot].get_or_insert_with(|| {
                let mut empty = stack.clone();
                empty.count = 0;
                empty
            });
            let add = stack
                .count
                .min(item_max_stack(items, cell).saturating_sub(cell.count));
            cell.count += add;
            stack.count -= add;
        }
        if stack.count == 0 || stack.count >= last {
            if stack.count == last && creative {
                stack.count = 0;
                return true;
            }
            return stack.count < last;
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
        match crate::mobs::equip::equippable(env.items, stack).map(|e| e.slot) {
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

    /// `AbstractContainerMenu.moveItemStackTo`, with an owned equip source when subscribed.
    fn move_to(
        &mut self,
        env: &Env,
        stack: &mut ItemStack,
        start: usize,
        end: usize,
        backwards: bool,
    ) -> bool {
        self.move_to_equip_source(env, stack, start, end, backwards)
    }

    /// `quickMoveStack` of the menu's kind; the moved stack's original copy
    /// (None when nothing moved).
    fn quick_move(&mut self, env: &Env, index: usize) -> Option<ItemStack> {
        self.touch();
        let mut stack = self.slot(index)?.clone();
        let mut clicked = stack.clone();
        let total = self.state.slots.len();
        let mut check_count = true;
        let mut after_take = false;
        match self.state.kind {
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
            // A crafting grid's result: CraftingMenu.quickMoveStack's.
            Kind::Mod(t) if self.state.slots[index].role == Role::CraftResult => {
                after_take = true;
                Self::map_crafted(env.maps, &mut stack);
                self.state.containers[RESULT][0] = Some(stack.clone());
                if !self.move_to(env, &mut stack, t.slots, total, true) {
                    return None;
                }
            }
            // ChestMenu.quickMoveStack over the mod's slots.
            Kind::Mod(t) => {
                check_count = false;
                let n = t.slots;
                if index < n {
                    if !self.move_to(env, &mut stack, n, total, true) {
                        return None;
                    }
                } else if !self.move_to(env, &mut stack, 0, n, false) {
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
                let slot = self.state.slots[index];
                let commit = |m: &mut Self, stack: &ItemStack| {
                    m.containers[slot.container][slot.index] =
                        (stack.count > 0).then(|| stack.clone());
                };
                if index <= 4 {
                    if !self.move_to(env, &mut stack, 5, 41, true) {
                        return None;
                    }
                } else if crate::brewing::brewing_fuel(env.items, &clicked).is_some() {
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
            Kind::Stonecutter => {
                after_take = true;
                if index == 1 {
                    if !self.move_to(env, &mut stack, 2, 38, true) {
                        return None;
                    }
                } else if index == 0 {
                    if !self.move_to(env, &mut stack, 2, 38, false) {
                        return None;
                    }
                } else if env
                    .station_recipes
                    .stonecutting
                    .iter()
                    .any(|r| r.input.test_in(env.loop_context, &stack))
                {
                    if !self.move_to(env, &mut stack, 0, 1, false) {
                        return None;
                    }
                } else if (2..29).contains(&index) {
                    if !self.move_to(env, &mut stack, 29, 38, false) {
                        return None;
                    }
                } else if (29..38).contains(&index) && !self.move_to(env, &mut stack, 2, 29, false)
                {
                    return None;
                }
            }
            Kind::Grindstone => {
                after_take = true;
                let full = self.state.containers[MAIN].iter().all(Option::is_some);
                if index == 2 {
                    if !self.move_to(env, &mut stack, 3, 39, true) {
                        return None;
                    }
                } else if index == 0 || index == 1 {
                    if !self.move_to(env, &mut stack, 3, 39, false) {
                        return None;
                    }
                } else if full {
                    if (3..30).contains(&index) {
                        if !self.move_to(env, &mut stack, 30, 39, false) {
                            return None;
                        }
                    } else if (30..39).contains(&index)
                        && !self.move_to(env, &mut stack, 3, 30, false)
                    {
                        return None;
                    }
                } else if !self.move_to(env, &mut stack, 0, 2, false) {
                    return None;
                }
            }
            Kind::Cartography => {
                after_take = true;
                if index == 2 {
                    // onCraftedBy edits the result in its slot first.
                    Self::map_crafted(env.maps, &mut stack);
                    self.state.containers[RESULT][0] = Some(stack.clone());
                    if !self.move_to(env, &mut stack, 3, 39, true) {
                        return None;
                    }
                } else if index == 0 || index == 1 {
                    if !self.move_to(env, &mut stack, 3, 39, false) {
                        return None;
                    }
                } else if stack.data().is_some_and(|d| d.map_id.is_some()) {
                    if !self.move_to(env, &mut stack, 0, 1, false) {
                        return None;
                    }
                } else if Self::is_carto_extra(&stack) {
                    if !self.move_to(env, &mut stack, 1, 2, false) {
                        return None;
                    }
                } else if (3..30).contains(&index) {
                    if !self.move_to(env, &mut stack, 30, 39, false) {
                        return None;
                    }
                } else if (30..39).contains(&index) && !self.move_to(env, &mut stack, 3, 30, false)
                {
                    return None;
                }
            }
            // LecternMenu.quickMoveStack moves nothing.
            Kind::Lectern => return None,
            Kind::Mount(columns) => {
                // AbstractMountInventoryMenu.quickMoveStack: no count check
                // and no onTake.
                check_count = false;
                let start = 2 + columns * 3;
                if index < start {
                    if !self.move_to(env, &mut stack, start, total, true) {
                        return None;
                    }
                } else if self.may_place(env, 1, &stack) && self.state.containers[MAIN][1].is_none()
                {
                    if !self.move_to(env, &mut stack, 1, 2, false) {
                        return None;
                    }
                } else if self.may_place(env, 0, &stack) && self.state.containers[MAIN][0].is_none()
                {
                    if !self.move_to(env, &mut stack, 0, 1, false) {
                        return None;
                    }
                } else if columns == 0 || !self.move_to(env, &mut stack, 2, start, false) {
                    // Between the inventory and the hotbar: the stack shrinks
                    // in its slot and EMPTY is returned either way.
                    let (main_end, hotbar_end) = (start + 27, start + 36);
                    if (main_end..hotbar_end).contains(&index) {
                        self.move_to(env, &mut stack, start, main_end, false);
                    } else if (start..main_end).contains(&index) {
                        self.move_to(env, &mut stack, main_end, hotbar_end, false);
                    }
                    let slot = self.state.slots[index];
                    self.state.containers[slot.container][slot.index] =
                        (stack.count > 0).then(|| stack.clone());
                    return None;
                }
            }
            Kind::Beacon => {
                // BeaconMenu.quickMoveStack.
                after_take = true;
                if index == 0 {
                    if !self.move_to(env, &mut stack, 1, 37, true) {
                        return None;
                    }
                } else if self.state.containers[MAIN][0].is_none()
                    && self.may_place(env, 0, &stack)
                    && stack.count == 1
                {
                    if !self.move_to(env, &mut stack, 0, 1, false) {
                        return None;
                    }
                } else if (1..28).contains(&index) {
                    if !self.move_to(env, &mut stack, 28, 37, false) {
                        return None;
                    }
                } else if (28..37).contains(&index) {
                    if !self.move_to(env, &mut stack, 1, 28, false) {
                        return None;
                    }
                } else if !self.move_to(env, &mut stack, 1, 37, false) {
                    return None;
                }
            }
            Kind::Enchanting => {
                after_take = true;
                if index < 2 {
                    if !self.move_to(env, &mut stack, 2, 38, true) {
                        return None;
                    }
                } else if &*stack.item == "minecraft:lapis_lazuli" {
                    if !self.move_to(env, &mut stack, 1, 2, true) {
                        return None;
                    }
                } else {
                    // One item into the empty item slot (setByPlayer).
                    if self.state.containers[MAIN][0].is_some() || !self.may_place(env, 0, &stack) {
                        return None;
                    }
                    let single = ItemStack {
                        count: 1,
                        ..stack.clone()
                    };
                    stack.count -= 1;
                    // The source slot keeps the rest while the item slot changes.
                    let slot = self.state.slots[index];
                    self.state.containers[slot.container][slot.index] =
                        (stack.count > 0).then(|| stack.clone());
                    self.set_by_player(env, 0, Some(single));
                }
            }
            Kind::Anvil | Kind::Smithing => {
                // ItemCombinerMenu.quickMoveStack.
                after_take = true;
                let result = if self.state.kind == Kind::Anvil { 2 } else { 3 };
                let (start, use_row) = (result + 1, result + 28);
                let end = result + 37;
                let into_inputs = self.state.kind == Kind::Anvil
                    || (0..3).any(|i| {
                        self.state.containers[MAIN][i].is_none()
                            && env
                                .station_recipes
                                .smithing_accepts(env.loop_context, i, &stack)
                    });
                if index == result {
                    if !self.move_to(env, &mut stack, start, end, true) {
                        return None;
                    }
                } else if index < result {
                    if !self.move_to(env, &mut stack, start, end, false) {
                        return None;
                    }
                } else if into_inputs && (start..end).contains(&index) {
                    if !self.move_to(env, &mut stack, 0, result, false) {
                        return None;
                    }
                } else if (start..use_row).contains(&index) {
                    if !self.move_to(env, &mut stack, use_row, end, false) {
                        return None;
                    }
                } else if (use_row..end).contains(&index)
                    && !self.move_to(env, &mut stack, start, use_row, false)
                {
                    return None;
                }
            }
            Kind::Merchant => {
                after_take = true;
                if index == 2 {
                    if !self.move_to(env, &mut stack, 3, 39, true) {
                        return None;
                    }
                    // Slot.onQuickCraft uses the original copy and the moved count.
                    self.state.merchant_remove_count = self
                        .merchant_remove_count
                        .wrapping_add((clicked.count - stack.count) as i32);
                    self.merchant_check_take_achievements(&clicked);
                    if let Some(m) = self.state.merchant.as_mut() {
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
                    // Slot.onQuickCraft -> FurnaceResultSlot.onQuickCraft:
                    // the moved count, with the stack as it was.
                    let moved = clicked.count - stack.count;
                    if moved > 0 {
                        self.state.output_removed += moved;
                        self.furnace_take(env, &clicked);
                    }
                } else if index != 1 && index != 0 {
                    let smelts = env.recipes.cooking_accepts(
                        env.loop_context,
                        &format!("minecraft:{recipe}"),
                        &stack,
                    );
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
                    // CraftingMenu.quickMoveStack processes the live result
                    // before transfer; ResultSlot then processes its original copy.
                    Self::map_crafted(env.maps, &mut stack);
                    self.state.containers[RESULT][0] = Some(stack.clone());
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
        let slot = self.state.slots[index];
        if stack.count == 0 {
            self.state.containers[slot.container][slot.index] = None;
            // InventoryMenu: setByPlayer(EMPTY, clicked).
            self.set_by_player_from(env, index, None, Some(&clicked));
        } else {
            self.state.containers[slot.container][slot.index] = Some(stack.clone());
            self.slot_changed(env, index);
        }
        if check_count && stack.count == clicked.count {
            return None;
        }
        if after_take || matches!(self.state.kind, Kind::Inventory | Kind::Crafting) {
            // A merchant result reports the stack it moved, a crafting result the copy it
            // takes (which Item.onCraftedBy may still change).
            self.take_after_quick_move(env, index, &mut clicked, &mut stack);
        }
        // What the result slot's move left over is dropped.
        if (index == 0 && matches!(self.state.kind, Kind::Inventory | Kind::Crafting))
            || (index == 1 && self.state.kind == Kind::Stonecutter)
            || (matches!(self.state.kind, Kind::Mod(_))
                && self.state.slots[index].role == Role::CraftResult)
        {
            self.drop_stack(&stack);
        }
        Some(clicked)
    }
}

/// `MerchantMenu` and `MerchantContainer` over the menu's `merchant` session.
impl Menu {
    /// `MerchantContainer.updateSellItem`.
    pub fn merchant_update(&mut self, env: &Env) {
        self.touch();
        let Some(m) = self.state.merchant.as_mut() else {
            return;
        };
        let (a, b) = (
            self.state.containers[MAIN][0].as_ref(),
            self.state.containers[MAIN][1].as_ref(),
        );
        let (a, b) = if a.is_none() { (b, None) } else { (a, b) };
        m.active = None;
        if a.is_none() {
            self.state.containers[RESULT][0] = None;
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
            self.state.containers[RESULT][0] = result;
        }
        let has = self.state.containers[RESULT][0].is_some();
        m.notify_trade_updated(has);
    }
    /// `MerchantResultSlot.checkTakeAchievements`: award the tallied count, then reset it.
    fn merchant_check_take_achievements(&mut self, stack: &ItemStack) {
        self.touch();
        // ItemStack.getItem returns AIR for the emptied quick-move remainder.
        let item = if stack.count == 0 {
            "minecraft:air".into()
        } else {
            stack.item.clone()
        };
        self.state
            .crafted_counts
            .push((item, std::mem::take(&mut self.state.merchant_remove_count)));
    }

    /// `MerchantResultSlot.onTake`: the payment is taken and the merchant
    /// told (`notifyTrade`).
    fn merchant_take(&mut self, env: &Env) {
        self.touch();
        let Some(m) = self.state.merchant.as_mut() else {
            return;
        };
        let Some(i) = m.active else { return };
        let Some(offer) = m.offers.get(i).cloned() else {
            return;
        };
        let (mut a, mut b) = (
            self.state.containers[MAIN][0].take(),
            self.state.containers[MAIN][1].take(),
        );
        let took = offer.take(env.items, &mut a, &mut b) || offer.take(env.items, &mut b, &mut a);
        // The payment stacks shrink in place, before the merchant hears of it.
        self.state.containers[MAIN][0] = a;
        self.state.containers[MAIN][1] = b;
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
        self.touch();
        let Some(m) = self.state.merchant.as_mut() else {
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
            if let Some(mut old) = self.state.containers[MAIN][slot].clone() {
                if !self.move_to(env, &mut old, 3, 39, true) {
                    return;
                }
                self.state.containers[MAIN][slot] = (old.count > 0).then_some(old);
                self.put(env, slot, self.state.containers[MAIN][slot].clone());
            }
        }
        if self.state.containers[MAIN][0].is_some() || self.state.containers[MAIN][1].is_some() {
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
        self.touch();
        for i in 3..39 {
            let Some(have) = self.slot(i).cloned() else {
                continue;
            };
            if !cost.test(&have) {
                continue;
            }
            let current = self.state.containers[MAIN][payment].clone();
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
            let slot = self.state.slots[i];
            let mut left = have;
            left.count -= moved;
            self.state.containers[slot.container][slot.index] = (left.count > 0).then_some(left);
            self.put(env, payment, Some(payment_stack.clone()));
            if payment_stack.count >= max {
                break;
            }
        }
    }
    /// `MerchantMenu.removed` for a living player: the payment goes back to
    /// the inventory (`placeItemBackInInventory`), or is dropped.
    pub fn merchant_close(&mut self, env: &Env) {
        self.touch();
        // AbstractContainerMenu.removed: the carried stack first.
        if let Some(mut carried) = self.state.carried.take() {
            self.place_back(env, &mut carried);
        }
        for slot in [0usize, 1] {
            if let Some(mut stack) = self.state.containers[MAIN][slot].take() {
                self.place_back(env, &mut stack);
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{banner::Banners, brewing::Brewing, crafting::Recipes};
    use horizon_world::loot::PackOverlay;

    pub(crate) fn stack(item: &str, count: u32) -> Option<ItemStack> {
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

    /// A menu reads a stack's own `MAX_STACK_SIZE`: a removed component is
    /// vanilla's default of one, so two cannot merge into one slot.
    #[test]
    fn a_removed_max_stack_size_leaves_one_per_slot() {
        with_menu(Kind::Inventory, |env, menu| {
            let removed = || {
                Some(horizon_core::nbt::Tag::compound(Vec::<(
                    &str,
                    horizon_core::nbt::Tag,
                )>::new()))
            };
            let mut stone = stack("stone", 1).unwrap();
            stone
                .data_mut()
                .set_other("!minecraft:max_stack_size", removed());
            menu.set_slot(env, 9, Some(stone.clone()));
            menu.carried = Some(stone);
            menu.click(env, 9, 0, Input::Pickup).unwrap();
            // Vanilla leaves both stacks at one; the item's default of 64 no
            // longer lets the click merge them.
            assert_eq!(menu.slot(9).map(|s| s.count), Some(1));
            assert_eq!(menu.carried.as_ref().map(|s| s.count), Some(1));
        });
    }

    /// The game's prepared data: `HORIZON_TEST_RESOURCES`, else
    /// `.local/horizon-client` in the checkout.
    pub(crate) fn resource_root() -> std::path::PathBuf {
        std::env::var_os("HORIZON_TEST_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/horizon-client")
                    .join(horizon_core::VANILLA_VERSION)
            })
    }

    /// A menu over the game's data (`resource_root`); skipped without it.
    pub(crate) fn with_menu(kind: Kind, test: impl FnOnce(&Env, &mut Menu)) {
        let root = resource_root();
        if !root.join("items.json").exists() {
            return;
        }
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let items = Items::catalog(&root, &PackOverlay::NONE, &defaults).unwrap();
        let recipes = Recipes::load(
            &root,
            &items,
            &PackOverlay::NONE,
            &crate::test_resources::server_load(),
        )
        .unwrap();
        let brewing = Brewing::load(&root, &items, &PackOverlay::NONE);
        let banners = Banners::load(&root, &defaults);
        let station_recipes = crate::stations::StationRecipes::load(
            &root,
            &items,
            &PackOverlay::NONE,
            &crate::test_resources::server_load(),
        );
        let enchantments = crate::enchant::Enchantments::default();
        let names = crate::creative::Names::default();
        let env = Env {
            crafting_remainder: None,
            player: None,
            player_entity_id: None,
            loop_context: &crate::test_resources::loop_context(LoopSide::Server),
            items: &items,
            recipes: &recipes,
            brewing: &brewing,
            banners: &banners,
            station_recipes: &station_recipes,
            enchantments: &enchantments,
            names: &names,
            trade: None,
            maps: None,
            limited_crafting: false,
            can_equip: None,
            known_recipes: None,
            slot_rules: None,
        };
        let mut menu = Menu::new(kind);
        menu.creative = true;
        test(&env, &mut menu);
    }

    #[test]
    #[ignore = "requires prepared resources"]
    fn staged_slot_refusal_restores_contents_and_on_take_side_effects() {
        let mut ran = false;
        with_menu(Kind::Brewing, |env, menu| {
            ran = true;
            let potion = ItemStack::new("minecraft:potion", 1);
            for take in [false, true] {
                for accepted in [false, true] {
                    *menu = Menu::new(Kind::Brewing);
                    menu.brewed_potion_hook = true;
                    if take {
                        menu.set_slot(env, 0, Some(potion.clone()));
                    }
                    let committed = MenuSlotView { menu, env, slot: 0 }.with_staged(&mut |slot| {
                        if take {
                            assert_eq!(slot.safe_take(1, 1), Some(potion.clone()));
                        } else {
                            assert!(slot.safe_insert(Some(potion.clone()), 1).is_none());
                        }
                        accepted
                    });
                    assert_eq!(committed, accepted);
                    assert_eq!(menu.slot(0), (take != accepted).then_some(&potion));
                    assert_eq!(
                        menu.brewed_potion_takes.as_slice(),
                        if take && accepted {
                            std::slice::from_ref(&potion)
                        } else {
                            &[]
                        }
                    );
                    assert_eq!(
                        menu.brewed.as_slice(),
                        if take && accepted {
                            std::slice::from_ref(&potion)
                        } else {
                            &[]
                        }
                    );
                }
            }
        });
        assert!(ran, "prepared menu fixture was not loaded");
    }

    #[test]
    fn menu_keeps_the_open_window_registry_identity() {
        let mut menu = Menu::new(Kind::Chest(3));
        assert_eq!(menu.registry_id(), None);
        menu.set_registry_id(Some("minecraft:generic_9x3"));
        assert_eq!(menu.registry_id(), Some("minecraft:generic_9x3"));
        menu.set_registry_id(Some("example:chest"));
        assert_eq!(menu.registry_id(), Some("example:chest"));
        menu.set_registry_id(None);
        assert_eq!(menu.registry_id(), None);
    }

    #[test]
    fn inventory_add_respects_component_cap_and_remainder() {
        with_menu(Kind::Inventory, |env, menu| {
            menu.creative = false;
            let mut item = ItemStack::new("minecraft:stone", 15);
            item.data_mut().set_other(
                "minecraft:max_stack_size",
                Some(horizon_core::nbt::Tag::Int(16)),
            );
            menu.set_player_item(0, Some(item.clone()));
            for i in 1..36 {
                menu.set_player_item(i, stack("minecraft:dirt", 64));
            }
            item.count = 8;
            menu.give(env, &mut item);
            assert_eq!(menu.player_item(0).unwrap().count, 16);
            assert_eq!(item.count, 7);
        });
    }

    #[test]
    fn swap_self_does_not_duplicate_and_container_slot_is_distinct() {
        with_menu(Kind::Crafting, |env, menu| {
            menu.set_player_item(0, stack("minecraft:stone", 8));
            menu.click(env, 37, 0, Input::Swap).unwrap();
            assert_eq!(menu.player_item(0).unwrap().count, 8);
            let total: u32 = (0..menu.slot_count())
                .filter_map(|s| menu.slot(s))
                .map(|s| s.count)
                .sum();
            assert_eq!(total, 8);
        });
        with_menu(Kind::Chest(3), |env, menu| {
            menu.set_slot(env, 0, stack("minecraft:dirt", 2));
            menu.set_player_item(0, stack("minecraft:stone", 8));
            menu.click(env, 0, 0, Input::Swap).unwrap();
            assert_eq!(menu.slot(0).unwrap().count, 8);
            assert_eq!(menu.player_item(0).unwrap().count, 2);
        });
    }

    /// `CrafterBlockEntity.fillStackedContents`: a crafter's 3x3 slots are
    /// part of the recipe book's craftable contents.
    #[test]
    fn a_crafters_grid_counts_for_the_recipe_book() {
        with_menu(Kind::CrafterBlock, |env, menu| {
            menu.containers[MAIN][0] = Some(ItemStack::new("minecraft:oak_planks", 4));
            let stacked = menu.stacked_contents(env);
            let planks = crate::crafting::Ingredient::of(vec!["minecraft:oak_planks".to_owned()]);
            assert!(
                stacked.can_craft(&[&planks]),
                "the crafter's own grid stacks"
            );
        });
    }

    /// `BrewingStandMenu.FuelSlot` and `quickMoveStack` use the same component.
    #[test]
    fn brewing_fuel_slot_and_quick_move_use_stack_components() {
        let items = Items::for_tests(Default::default());
        let root = std::path::Path::new("");
        let recipes = Recipes::empty();
        let brewing = Brewing::load(root, &items, &PackOverlay::NONE);
        let banners = Banners::default();
        let station_recipes = crate::stations::StationRecipes::default();
        let enchantments = crate::enchant::Enchantments::default();
        let names = crate::creative::Names::default();
        let env = Env {
            crafting_remainder: None,
            player: None,
            player_entity_id: None,
            loop_context: &crate::test_resources::loop_context(LoopSide::Server),
            items: &items,
            recipes: &recipes,
            brewing: &brewing,
            banners: &banners,
            station_recipes: &station_recipes,
            enchantments: &enchantments,
            names: &names,
            trade: None,
            maps: None,
            limited_crafting: false,
            can_equip: None,
            known_recipes: None,
            slot_rules: None,
        };
        let mut fuel = ItemStack::new("minecraft:stone", 2);
        fuel.data_mut().set_loot_component(
            "minecraft:brewing_fuel",
            &Some(
                horizon_core::json_nbt::json_to_tag(
                    &serde_json::json!({"uses": 7, "speed_multiplier": 2.5}),
                )
                .unwrap(),
            ),
        );
        let mut menu = Menu::new(Kind::Brewing);
        assert!(menu.may_place(&env, 4, &fuel));
        menu.set_slot(&env, 5, Some(fuel));
        assert_eq!(menu.quick_move_landing(&env, 5), vec![4]);
        let ordinary = ItemStack::new("minecraft:stone", 2);
        assert!(!menu.may_place(&env, 4, &ordinary));
        menu.set_slot(&env, 5, Some(ordinary));
        assert!(!menu.quick_move_landing(&env, 5).contains(&4));
    }

    /// MerchantResultSlot's removal tally and quick-craft check reset before onTake.
    #[test]
    fn merchant_takes_record_crafted_counts_once() {
        let items = Items::for_tests(Default::default());
        let recipes = Recipes::empty();
        let brewing = Brewing::load(std::path::Path::new(""), &items, &PackOverlay::NONE);
        let banners = Banners::default();
        let station_recipes = crate::stations::StationRecipes::default();
        let enchantments = crate::enchant::Enchantments::default();
        let names = crate::creative::Names::default();
        let env = Env {
            crafting_remainder: None,
            player: None,
            player_entity_id: None,
            loop_context: &crate::test_resources::loop_context(LoopSide::Server),
            items: &items,
            recipes: &recipes,
            brewing: &brewing,
            banners: &banners,
            station_recipes: &station_recipes,
            enchantments: &enchantments,
            names: &names,
            trade: None,
            maps: None,
            limited_crafting: false,
            can_equip: None,
            known_recipes: None,
            slot_rules: None,
        };
        for (input, payment) in [
            (Input::Pickup, 8),
            (Input::QuickMove, 8),
            (Input::Pickup, 3),
        ] {
            let offer = crate::merchant::MerchantOffer {
                base_cost_a: crate::merchant::ItemCost {
                    stack: ItemStack::new("minecraft:emerald", 4),
                },
                cost_b: None,
                result: ItemStack::new("minecraft:glass", 3),
                uses: 0,
                max_uses: 100,
                reward_exp: false,
                special_price_diff: 0,
                demand: 0,
                price_multiplier: 0.0,
                xp: 1,
            };
            let mut menu = Menu::new(Kind::Merchant);
            menu.merchant = Some(Box::new(crate::merchant::Session::new(vec![offer], true)));
            menu.set_slot(&env, 0, stack("emerald", payment));
            assert_eq!(menu.slot(2).map(|s| s.count), (payment >= 4).then_some(3));
            assert!(menu.crafted_counts.is_empty(), "a preview is not a take");
            assert_eq!(menu.merchant.as_ref().unwrap().offers[0].uses, 0);
            menu.click(&env, 2, 0, input).unwrap();
            if payment < 4 {
                assert!(menu.crafted_counts.is_empty());
                assert_eq!(menu.slot(0).map(|s| s.count), Some(3));
                assert!(menu.carried.is_none());
                assert_eq!(menu.merchant.as_ref().unwrap().offers[0].uses, 0);
                continue;
            }
            if input == Input::Pickup {
                assert_eq!(menu.carried.as_ref().map(|s| s.count), Some(3));
                menu.click(&env, 31, 0, Input::Pickup).unwrap();
                menu.click(&env, 2, 0, Input::Pickup).unwrap();
            }
            let mut expected = Vec::new();
            for _ in 0..2 {
                expected.push((Arc::<str>::from("minecraft:glass"), 3));
                if input == Input::QuickMove {
                    expected.push((Arc::<str>::from("minecraft:air"), 0));
                }
            }
            assert_eq!(menu.crafted_counts, expected);
            assert_eq!(menu.merchant_remove_count, 0);
            assert_eq!(menu.merchant.as_ref().unwrap().offers[0].uses, 2);
            assert!(menu.slot(0).is_none());
            assert!(menu.slot(2).is_none());
        }
    }

    /// The Mouse Tweaks wheel's targets: vanilla's routing of one item.
    #[test]
    fn quick_move_landing_routes_a_furnace_like_vanilla() {
        with_menu(Kind::Furnace("smelting"), |env, menu| {
            menu.creative = false;
            // 0 input, 1 fuel, 2 output, 3..30 inventory, 30..39 hotbar.
            let mut landing = |item, count, from| {
                menu.set_slot(env, from, stack(item, count));
                let to = menu.quick_move_landing(env, from);
                menu.set_slot(env, from, None);
                for &t in &to {
                    menu.set_slot(env, t, None);
                }
                to
            };
            assert_eq!(landing("raw_iron", 5, 30), [0]);
            assert_eq!(landing("coal", 5, 30), [1]);
            // Neither: between the hotbar and the inventory, as a shift-click.
            assert_eq!(landing("dirt", 5, 30), [3]);
            assert_eq!(landing("dirt", 5, 3), [30]);
            // Out of the fuel slot: moveItemStackTo(3, 39, false).
            assert_eq!(landing("coal", 5, 1), [3]);
        });
    }

    struct AppleSets;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for AppleSets {
        fn recipe_property_sets(
            &self,
            _: &dyn mod_api::DataLoadContext,
            event: &mut mod_api::RecipePropertySetsEvent<'_>,
        ) {
            for set in ["furnace_input", "brewing_reagent", "smithing_addition"] {
                event.additions.push(mod_api::RecipePropertyAddition::new(
                    set,
                    vec!["minecraft:apple".to_owned()],
                ));
            }
        }
    }

    /// What a mod adds to the recipe property sets is what the furnace's
    /// shift-click, the brewing stand's reagent slot and the smithing
    /// table's addition slot take, against the real recipes: with no mod
    /// an apple goes to none of them.
    #[test]
    fn mod_additions_to_the_property_sets_reach_every_reader() {
        let check = |added: bool| {
            with_menu(Kind::Furnace("smelting"), |env, menu| {
                menu.creative = false;
                menu.set_slot(env, 30, stack("apple", 5));
                let to = menu.quick_move_landing(env, 30);
                assert_eq!(to, if added { vec![0] } else { vec![3] });
                let apple = stack("apple", 1).unwrap();
                let root = resource_root();
                let defaults = horizon_world::item_defaults::read(&root).unwrap();
                let stations = crate::stations::StationData::load(
                    &root,
                    env.items,
                    &PackOverlay::NONE,
                    &defaults,
                    &crate::test_resources::server_load(),
                );
                assert_eq!(stations.brewing.is_reagent(&apple), added);
                assert_eq!(
                    env.station_recipes
                        .smithing_accepts(env.loop_context, 2, &apple),
                    added
                );
                // Vanilla's own entries stay.
                assert!(
                    stations
                        .brewing
                        .is_reagent(&stack("nether_wart", 1).unwrap())
                );
            })
        };
        check(false);
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new("apples", Box::new(AppleSets))].into(),
            || check(true),
        );
    }

    /// `FurnaceResultSlot.checkTakeAchievements` as `ItemSmeltedEvent` sees
    /// it: a pickup passes the taken stack and its count; a shift-click with
    /// room for 2 of 64 passes the 64 with 2 removed (`onQuickCraft`), then
    /// the 62 left with 0 (`onTake`); nothing is kept without a listener.
    #[test]
    fn a_furnace_take_keeps_what_check_take_achievements_passes() {
        bare_env(|env| {
            let furnace = |hook: bool| {
                let mut menu = Menu::new(Kind::Furnace("smelting"));
                menu.item_smelted_hook = hook;
                menu.set_slot(env, 2, stack("iron_ingot", 64));
                menu
            };

            let mut menu = furnace(true);
            menu.click(env, 2, 0, Input::Pickup).unwrap();
            assert_eq!(menu.item_smelted, [(stack("iron_ingot", 64), 64)]);

            // The inventory full of dirt but for one stack with room for 2.
            let mut menu = furnace(true);
            for s in 3..39 {
                menu.set_slot(env, s, stack("dirt", 64));
            }
            menu.set_slot(env, 3, stack("iron_ingot", 62));
            menu.click(env, 2, 0, Input::QuickMove).unwrap();
            assert_eq!(menu.slot(2), stack("iron_ingot", 62).as_ref());
            assert_eq!(
                menu.item_smelted,
                [(stack("iron_ingot", 64), 2), (stack("iron_ingot", 62), 0),]
            );

            let mut menu = furnace(false);
            menu.click(env, 2, 0, Input::Pickup).unwrap();
            assert!(menu.item_smelted.is_empty());
        });
    }

    #[path = "remainder_tests.rs"]
    mod remainder;

    /// A test `Env` with no vanilla data: the anvil's and grindstone's own
    /// results don't matter where the mods answer.
    pub(super) fn bare_env(test: impl FnOnce(&Env)) {
        bare_env_in(&crate::test_resources::loop_context(LoopSide::Server), test)
    }

    fn bare_env_in(loop_context: &LoopContext, test: impl FnOnce(&Env)) {
        static RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let run = RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("horizon-bare-{}-{run}", std::process::id()));
        std::fs::create_dir_all(root.join("data/minecraft/recipe")).unwrap();
        let items = Items::for_tests(Default::default());
        let recipes = Recipes::load(
            &root,
            &items,
            &PackOverlay::NONE,
            &crate::test_resources::server_load(),
        )
        .unwrap();
        let brewing = Brewing::load(&root, &items, &PackOverlay::NONE);
        let _ = std::fs::remove_dir_all(&root);
        let (banners, station_recipes) = (Default::default(), Default::default());
        let (enchantments, names) = (Default::default(), Default::default());
        let rules = ContextRules(loop_context);
        test(&Env {
            crafting_remainder: None,
            player: Some(7),
            player_entity_id: Some(41),
            loop_context,
            items: &items,
            recipes: &recipes,
            brewing: &brewing,
            banners: &banners,
            station_recipes: &station_recipes,
            enchantments: &enchantments,
            names: &names,
            trade: None,
            maps: None,
            limited_crafting: false,
            can_equip: None,
            known_recipes: None,
            slot_rules: Some(&rules),
        });
    }

    /// The `menu_slot_rules` answers a mod menu `test:rules` gives in the tests: slot 0 takes
    /// a stack on a server only, and holds what the `rules_mod` setting says (7 without it).
    struct ContextRules<'a>(&'a LoopContext);
    impl SlotRules for ContextRules<'_> {
        fn listens(&self, menu: &str) -> bool {
            menu == "test:rules"
        }
        fn rule(
            &self,
            menu: &str,
            slot: usize,
            _: &ItemStack,
            may_place: &mut bool,
            max_stack: &mut u32,
        ) {
            if menu == "test:rules" && slot == 0 {
                *may_place = self.0.side() == LoopSide::Server;
                *max_stack = self
                    .0
                    .setting("rules_mod", "stack")
                    .map_or(7, |value| value.parse().unwrap());
            }
        }
    }

    /// `AnvilUpdateEvent`: not asked without a left input, and once per
    /// `createResult`; a mod's output takes its cost and uses its material
    /// cost of the right input when taken; a cancel leaves no result at
    /// cost 0.
    #[test]
    fn an_anvil_update_sets_the_result_cost_and_material_or_cancels() {
        bare_env(|env| {
            let mut menu = Menu::new(Kind::Anvil);
            menu.set_slot(env, 1, stack("diamond", 5));
            assert!(menu.anvil_created());
            assert_eq!(menu.anvil_update(), None);
            menu.set_slot(env, 0, stack("iron_sword", 1));
            assert!(menu.anvil_created());
            assert!(!menu.anvil_created());
            let mut update = menu.anvil_update().unwrap();
            assert_eq!((&update.vanilla.output, &update.output), (&None, &None));
            update.output = stack("netherite_sword", 1);
            update.xp_cost = 7;
            update.material_cost = 2;
            menu.anvil_updated(update.clone(), false);
            assert_eq!(
                (menu.slot(2), menu.data[0]),
                (stack("netherite_sword", 1).as_ref(), 7)
            );
            menu.levels = 30;
            menu.click(env, 2, 0, Input::Pickup).unwrap();
            assert_eq!(menu.carried, stack("netherite_sword", 1));
            assert_eq!(
                (menu.slot(0), menu.slot(1)),
                (None, stack("diamond", 3).as_ref())
            );

            let mut menu = Menu::new(Kind::Anvil);
            menu.set_slot(env, 0, stack("iron_sword", 1));
            menu.set_slot(env, 1, stack("diamond", 5));
            menu.anvil_updated(update, true);
            assert_eq!((menu.slot(2), menu.data[0]), (None, 0));
        });
    }

    #[test]
    fn an_anvil_update_preserves_signed_xp_cost_without_allowing_a_refund() {
        bare_env(|env| {
            for creative in [false, true] {
                for cost in [i32::MIN, -7, 0] {
                    let mut menu = Menu::new(Kind::Anvil);
                    menu.creative = creative;
                    menu.levels = 30;
                    menu.set_slot(env, 0, stack("iron_sword", 1));
                    menu.set_slot(env, 1, stack("diamond", 5));
                    let mut update = menu.anvil_update().unwrap();
                    update.output = stack("netherite_sword", 1);
                    update.xp_cost = cost;
                    update.material_cost = -2;
                    menu.anvil_updated(update.clone(), false);
                    assert_eq!(menu.data[0], cost);
                    assert_eq!(menu.anvil.repair_count, 0);
                    assert_eq!(menu.slot(2), stack("netherite_sword", 1).as_ref());
                    menu.click(env, 2, 0, Input::Pickup).unwrap();
                    assert_eq!(menu.carried, None);
                    assert_eq!(menu.slot(2), stack("netherite_sword", 1).as_ref());
                    assert_eq!(menu.slot(0), stack("iron_sword", 1).as_ref());
                    assert_eq!(menu.slot(1), stack("diamond", 5).as_ref());
                    assert_eq!(menu.level_change, 0);
                    menu.anvil_updated(update, true);
                    assert_eq!((menu.slot(2), menu.data[0]), (None, 0));
                }
            }
        });
    }

    /// `AnvilUpdateEvent` on a rename with no right input: asked with the
    /// text and vanilla's renamed result, which the output and cost start
    /// at; a mod that only changes the cost keeps vanilla's result, and a
    /// cancel leaves none.
    #[test]
    fn an_anvil_rename_alone_starts_at_vanillas_result() {
        bare_env(|env| {
            let renamed = || {
                let mut menu = Menu::new(Kind::Anvil);
                menu.set_slot(env, 0, stack("iron_sword", 1));
                menu.anvil_created();
                assert!(menu.rename(env, "Edge"));
                menu
            };
            let mut menu = renamed();
            let vanilla = (menu.slot(2).cloned(), menu.data[0]);
            assert!(vanilla.0.is_some());
            let mut update = menu.anvil_update().unwrap();
            assert_eq!(
                (update.right.as_ref(), update.name.as_deref()),
                (None, Some("Edge"))
            );
            assert_eq!(
                (&update.vanilla.output, &update.output),
                (&vanilla.0, &vanilla.0)
            );
            assert_eq!(
                (update.vanilla.xp_cost, update.xp_cost),
                (vanilla.1, vanilla.1)
            );
            update.xp_cost = 5;
            menu.anvil_updated(update.clone(), false);
            assert_eq!((menu.slot(2).cloned(), menu.data[0]), (vanilla.0, 5));

            let mut menu = renamed();
            let mut empty = update.clone();
            empty.output = None;
            menu.anvil_updated(empty, false);
            assert_eq!((menu.slot(2), menu.data[0]), (None, 5));

            let mut menu = renamed();
            menu.anvil_updated(update, true);
            assert_eq!((menu.slot(2), menu.data[0]), (None, 0));
        });
    }

    /// A rename beside a right input the anvil can't use changes neither
    /// the (empty) result nor the cost, but `createResult` ran: the mods
    /// are asked, and their output is the result.
    #[test]
    fn an_anvil_rename_that_changes_no_result_is_asked() {
        bare_env(|env| {
            let mut menu = Menu::new(Kind::Anvil);
            menu.set_slot(env, 0, stack("iron_sword", 1));
            menu.set_slot(env, 1, stack("dirt", 1));
            menu.anvil_created();
            let before = (menu.slot(2).cloned(), menu.data[0]);
            assert!(menu.rename(env, "Edge"));
            assert_eq!((menu.slot(2).cloned(), menu.data[0]), before);
            assert!(menu.anvil_created(), "createResult ran");
            let mut update = menu.anvil_update().unwrap();
            assert_eq!(update.name.as_deref(), Some("Edge"));
            update.output = stack("golden_sword", 1);
            menu.anvil_updated(update, false);
            assert_eq!(menu.slot(2), stack("golden_sword", 1).as_ref());
        });
    }

    /// `GrindstoneEvent`: a placed output with its experience is what the
    /// take reports; the take's answer sets the inputs and the experience,
    /// and a cancelled take keeps the inputs and gives nothing.
    #[test]
    fn grindstone_events_set_the_result_the_inputs_and_the_experience() {
        bare_env(|env| {
            let grindstone = || {
                let mut menu = Menu::new(Kind::Grindstone);
                menu.grind_hook = true;
                menu.set_slot(env, 0, stack("iron_sword", 1));
                menu.set_slot(env, 1, stack("iron_sword", 1));
                let mut place = menu.grindstone_place();
                place.output = stack("stick", 1);
                place.xp = 5;
                menu.grindstone_placed(place, false);
                menu.click(env, 2, 0, Input::Pickup).unwrap();
                menu
            };
            let mut menu = grindstone();
            assert_eq!(menu.carried, stack("stick", 1));
            let mut take = menu.grind_take.take().unwrap();
            assert_eq!(
                (&take.top, &take.bottom, take.xp),
                (&stack("iron_sword", 1), &stack("iron_sword", 1), 5)
            );
            // The inputs wait for the mods.
            assert_eq!(menu.slot(0), stack("iron_sword", 1).as_ref());
            take.new_top = stack("dirt", 1);
            assert_eq!(menu.grindstone_taken(env, take.clone(), false), Some(5));
            assert_eq!(
                (menu.slot(0), menu.slot(1)),
                (stack("dirt", 1).as_ref(), None)
            );

            let mut menu = grindstone();
            let take = menu.grind_take.take().unwrap();
            assert_eq!(menu.grindstone_taken(env, take, true), None);
            assert_eq!(menu.slot(1), stack("iron_sword", 1).as_ref());
            assert_eq!(menu.ground, None);

            let mut menu = Menu::new(Kind::Grindstone);
            menu.set_slot(env, 0, stack("iron_sword", 1));
            menu.load_slot(2, stack("golden_sword", 1));
            let mut place = menu.grindstone_place();
            assert_eq!(place.vanilla_output, stack("golden_sword", 1));
            assert_eq!(place.output, None);
            place.vanilla_output = stack("dirt", 1);
            menu.grindstone_placed(place, false);
            assert_eq!(menu.slot(2), stack("golden_sword", 1).as_ref());

            let mut place = menu.grindstone_place();
            place.vanilla_output = stack("golden_sword", 1);
            place.output = stack("stick", 1);
            menu.grindstone_placed(place, false);
            assert_eq!(menu.slot(2), stack("stick", 1).as_ref());

            let mut place = menu.grindstone_place();
            place.vanilla_output = None;
            menu.grindstone_placed(place, false);
            assert_eq!(menu.slot(2), stack("stick", 1).as_ref());

            // Cancellation wins even when a mod supplies an override.
            let mut place = menu.grindstone_place();
            place.output = stack("stick", 1);
            menu.grindstone_placed(place, true);
            assert_eq!(menu.slot(2), None);
        });
    }

    #[test]
    fn quick_move_landing_never_fills_a_result() {
        with_menu(Kind::Furnace("smelting"), |env, menu| {
            menu.creative = false;
            menu.set_slot(env, 2, stack("iron_ingot", 5));
            menu.set_slot(env, 30, stack("iron_ingot", 5));
            // An ingot with room in the output still goes elsewhere.
            assert_eq!(menu.quick_move_landing(env, 30), [3]);
            assert_eq!(menu.slot(2).map(|s| s.count), Some(5));
        });
        with_menu(Kind::Furnace("smelting"), |env, menu| {
            menu.creative = false;
            // The whole output moves: into the partial stack, then an empty slot.
            menu.set_slot(env, 2, stack("iron_ingot", 5));
            menu.set_slot(env, 20, stack("iron_ingot", 62));
            assert_eq!(menu.quick_move_landing(env, 2), [20, 38]);
        });
        with_menu(Kind::Furnace("smelting"), |env, menu| {
            menu.creative = false;
            // A full inventory: the output stays, nothing to land on.
            menu.set_slot(env, 2, stack("iron_ingot", 5));
            for i in 3..39 {
                menu.set_slot(env, i, stack("stone", 64));
            }
            assert!(menu.quick_move_landing(env, 2).is_empty());
        });
    }

    #[test]
    fn quick_move_landing_routes_a_crafting_table_like_vanilla() {
        with_menu(Kind::Crafting, |env, menu| {
            menu.creative = false;
            // 0 result, 1..10 grid, 10..37 inventory, 37..46 hotbar.
            menu.set_slot(env, 37, stack("oak_log", 2));
            assert_eq!(menu.quick_move_landing(env, 37), [1]);
        });
        with_menu(Kind::Crafting, |env, menu| {
            menu.creative = false;
            menu.set_slot(env, 1, stack("oak_log", 1));
            menu.settle(env);
            assert_eq!(menu.slot(0).map(|s| s.count), Some(4));
            assert_eq!(menu.quick_move_landing(env, 0), [45]);
        });
    }

    /// A smithing result taken is recorded for `ItemSmithedEvent` while a mod
    /// listens: the inputs before the take used them and the stack taken.
    /// A shift-click records it too (`onQuickCraft`).
    #[test]
    fn taking_a_smithing_result_records_its_inputs() {
        for input in [Input::Pickup, Input::QuickMove] {
            with_menu(Kind::Smithing, |env, menu| {
                menu.creative = false;
                menu.item_smithed_hook = true;
                let inputs = [
                    stack("netherite_upgrade_smithing_template", 1),
                    stack("diamond_sword", 1),
                    stack("netherite_ingot", 2),
                ];
                for (i, input) in inputs.iter().enumerate() {
                    menu.set_slot(env, i, input.clone());
                }
                menu.settle(env);
                menu.click(env, 3, 0, input).unwrap();
                assert_eq!(
                    menu.item_smithed,
                    vec![(inputs, stack("netherite_sword", 1).unwrap())],
                    "{input:?}"
                );
                // One ingot is left, and nothing more is recorded.
                assert_eq!(menu.slot(2).map(|s| s.count), Some(1));
            });
        }
        // With no mod listening nothing is kept.
        with_menu(Kind::Smithing, |env, menu| {
            menu.creative = false;
            menu.set_slot(env, 0, stack("netherite_upgrade_smithing_template", 1));
            menu.set_slot(env, 1, stack("diamond_sword", 1));
            menu.set_slot(env, 2, stack("netherite_ingot", 1));
            menu.settle(env);
            menu.click(env, 3, 0, Input::Pickup).unwrap();
            assert!(menu.item_smithed.is_empty());
            assert!(menu.carried.is_some());
        });
    }

    /// A crafting grid's result taken is recorded for `native_item_crafted`:
    /// the recipe the grid matched and the stack the player took.
    #[test]
    fn taking_a_crafting_result_records_the_crafted_stack() {
        with_menu(Kind::Crafting, |env, menu| {
            menu.creative = false;
            menu.set_slot(env, 1, stack("oak_log", 1));
            menu.settle(env);
            menu.click(env, 0, 0, Input::Pickup).unwrap();
            let mut grid = vec![None; 9];
            grid[0] = stack("oak_log", 1);
            assert_eq!(
                menu.item_crafted,
                vec![(
                    "minecraft:oak_planks".to_owned(),
                    stack("oak_planks", 4).unwrap(),
                    grid
                )]
            );
        });
    }

    /// The inventory's own 2x2 grid is recorded as its 4 cells, the log in
    /// the cell it was in.
    #[test]
    fn an_inventory_craft_records_its_four_grid_cells() {
        with_menu(Kind::Inventory, |env, menu| {
            menu.creative = false;
            menu.set_slot(env, 2, stack("oak_log", 1));
            menu.settle(env);
            menu.click(env, 0, 0, Input::Pickup).unwrap();
            assert_eq!(
                menu.item_crafted,
                vec![(
                    "minecraft:oak_planks".to_owned(),
                    stack("oak_planks", 4).unwrap(),
                    vec![None, stack("oak_log", 1), None, None]
                )]
            );
        });
    }

    /// A shift-click that crafts three times records three takes, each with
    /// the grid as it was before that craft.
    #[test]
    fn a_shift_click_craft_records_each_batch_with_its_grid() {
        with_menu(Kind::Crafting, |env, menu| {
            menu.creative = false;
            menu.set_slot(env, 1, stack("oak_log", 3));
            menu.settle(env);
            menu.click(env, 0, 0, Input::QuickMove).unwrap();
            let logs: Vec<_> = menu
                .item_crafted
                .iter()
                .map(|(_, _, grid)| (grid.len(), grid[0].as_ref().map(|s| s.count)))
                .collect();
            assert_eq!(logs, [(9, Some(3)), (9, Some(2)), (9, Some(1))]);
        });
    }

    /// `StonecutterMenu`'s `lastSoundTime`: a shift-click crafting a whole
    /// stack plays the take sound once, not once per item.
    #[test]
    fn a_stonecutter_shift_click_plays_the_take_sound_once() {
        with_menu(Kind::Stonecutter, |env, menu| {
            menu.creative = false;
            menu.set_slot(env, 0, stack("granite", 60));
            let polished = menu
                .stone
                .recipes
                .iter()
                .position(|&r| {
                    &*env.station_recipes.stonecutting[r].result.item
                        == "minecraft:polished_granite"
                })
                .expect("granite cuts into polished granite");
            assert!(menu.button(env, polished as i32));
            menu.click(env, 1, 0, Input::QuickMove).unwrap();
            assert_eq!(menu.slot(0), None);
            let made: u32 = (2..menu.slot_count())
                .filter_map(|i| menu.slot(i))
                .map(|s| s.count)
                .sum();
            assert_eq!(made, 60);
            assert_eq!(menu.sounds, ["minecraft:ui.stonecutter.take_result"]);
        });
    }

    #[test]
    fn listener_safe_take_consumes_crafting_inputs() {
        with_menu(Kind::Crafting, |env, menu| {
            menu.creative = false;
            menu.set_slot(env, 1, stack("oak_log", 1));
            menu.settle(env);
            let mut view = MenuSlotView { menu, env, slot: 0 };
            let mut carried = None;
            let mut event = StackedOnOther {
                player: None,
                player_entity_id: None,
                slot: 0,
                container: MAIN,
                container_slot: 0,
                primary: true,
                carried: CarriedAccess(&mut carried),
                clicked: &mut view,
                cancellation_result: false,
            };
            let listener = |event: &mut StackedOnOther<'_>| {
                event.carried.set(event.clicked.safe_take(4, 4));
            };
            listener(&mut event);
            assert_eq!(carried, stack("oak_planks", 4));
            assert!(menu.slot(1).is_none());
            let mut grid = vec![None; 9];
            grid[0] = stack("oak_log", 1);
            assert_eq!(
                menu.item_crafted,
                vec![(
                    "minecraft:oak_planks".to_owned(),
                    stack("oak_planks", 4).unwrap(),
                    grid
                )]
            );
        });
    }

    #[test]
    fn quick_move_landing_routes_plain_containers() {
        for (kind, size) in [
            (Kind::Hopper, 5),
            (Kind::Dispenser, 9),
            (Kind::Chest(3), 27),
        ] {
            with_menu(kind, |env, menu| {
                menu.creative = false;
                let hotbar = size + 27;
                menu.set_slot(env, hotbar, stack("stone", 3));
                assert_eq!(menu.quick_move_landing(env, hotbar), [0]);
            });
            with_menu(kind, |env, menu| {
                menu.creative = false;
                menu.set_slot(env, 0, stack("stone", 3));
                assert_eq!(menu.quick_move_landing(env, 0), [size + 35]);
            });
        }
    }

    #[test]
    fn takes_one_follows_the_slots_rules() {
        with_menu(Kind::Furnace("smelting"), |env, menu| {
            menu.creative = false;
            let (coal, cobble) = (stack("coal", 1).unwrap(), stack("cobblestone", 1).unwrap());
            let bucket = stack("bucket", 1).unwrap();
            assert!(menu.takes_one(env, 0, &cobble), "the input takes anything");
            assert!(menu.takes_one(env, 1, &coal));
            assert!(!menu.takes_one(env, 1, &cobble), "not fuel");
            assert!(!menu.takes_one(env, 2, &coal), "the output");
            menu.set_slot(env, 1, Some(bucket.clone()));
            assert!(
                !menu.takes_one(env, 1, &bucket),
                "one bucket in the fuel slot"
            );
            menu.set_slot(env, 1, stack("coal", 1));
            assert!(!menu.takes_one(env, 1, &bucket), "another item");
            assert!(menu.takes_one(env, 1, &coal));
        });
    }

    /// `HorseInventoryMenu`: a shift-click wears horse armour, then saddles,
    /// then fills the chest; the body slot takes only what the mount may wear.
    #[test]
    fn mount_menu_slots_take_what_the_mount_wears() {
        with_menu(Kind::Mount(5), |env, menu| {
            menu.creative = false;
            menu.mount = Some("minecraft:donkey".into());
            // 2 saddle and body, 15 chest, then the inventory from 17.
            assert_eq!(menu.slot_count(), 2 + 15 + 36);
            let hotbar = 17 + 27;
            menu.set_slot(env, hotbar, stack("iron_horse_armor", 1));
            menu.click(env, hotbar as i32, 0, Input::QuickMove).unwrap();
            // A donkey wears no horse armour: it goes in the chest.
            assert_eq!(menu.slot(1), None);
            assert_eq!(
                menu.slot(2).map(|s| &*s.item),
                Some("minecraft:iron_horse_armor")
            );
            menu.set_slot(env, hotbar, stack("saddle", 1));
            menu.click(env, hotbar as i32, 0, Input::QuickMove).unwrap();
            assert_eq!(menu.slot(0).map(|s| &*s.item), Some("minecraft:saddle"));
            // Back out of the mount: to the inventory, from the hotbar's end.
            menu.click(env, 0, 0, Input::QuickMove).unwrap();
            assert_eq!(menu.slot(0), None);
            assert_eq!(
                menu.slot(17 + 35).map(|s| &*s.item),
                Some("minecraft:saddle")
            );
        });
        with_menu(Kind::Mount(0), |env, menu| {
            menu.creative = false;
            menu.mount = Some("minecraft:horse".into());
            let hotbar = 2 + 27;
            menu.set_slot(env, hotbar, stack("diamond_horse_armor", 1));
            menu.click(env, hotbar as i32, 0, Input::QuickMove).unwrap();
            assert_eq!(
                menu.slot(1).map(|s| &*s.item),
                Some("minecraft:diamond_horse_armor")
            );
            // No chest: dirt goes between the hotbar and the inventory.
            menu.set_slot(env, hotbar, stack("dirt", 3));
            menu.click(env, hotbar as i32, 0, Input::QuickMove).unwrap();
            assert_eq!(
                menu.slot(2).map(|s| (&*s.item, s.count)),
                Some(("minecraft:dirt", 3))
            );
            assert_eq!(menu.slot(hotbar), None);
            // Placing by hand: a saddle doesn't go in the body slot.
            menu.carried = stack("saddle", 1);
            menu.click(env, 1, 0, Input::Pickup).unwrap();
            assert_eq!(
                menu.slot(1).map(|s| &*s.item),
                Some("minecraft:diamond_horse_armor")
            );
            menu.click(env, 0, 0, Input::Pickup).unwrap();
            assert_eq!(menu.slot(0).map(|s| &*s.item), Some("minecraft:saddle"));
        });
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
    fn installed_slot_rules_change_mod_menu_placement() {
        static JUMPS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        static STILL_VALID_ASKED: std::sync::atomic::AtomicUsize =
            std::sync::atomic::AtomicUsize::new(0);
        let installed =
            crate::native_client_hooks::install(crate::native_client_hooks::NativeClientHooks {
                menu_still_valid: |context, player, menu, _, eye, valid| {
                    STILL_VALID_ASKED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if menu == "test:rules" && player == 2 && eye[2] == 0.0 {
                        *valid = context.setting("rules_mod", "stack").is_some();
                    }
                },
                // A part answers on the vanilla chest menu too: the game must
                // still never ask it, because it is not a mod menu type.
                menu_still_valid_listens: |menu| {
                    matches!(menu, "test:rules" | "minecraft:generic_9x3")
                },
                item_stacked_on_other: |context, _, canceled| {
                    *canceled = context.setting("rules_mod", "stack").is_some();
                },
                living_heal: |context, _, amount, cancel| {
                    if context.setting("rules_mod", "stack").is_some() {
                        *cancel = true;
                        *amount += 1.0;
                    }
                },
                living_breathe: |context, _, kind, can_breathe, consume, _| {
                    if context.setting("rules_mod", "stack").is_some() {
                        // A cod breathes on land; a salmon loses all its air at once.
                        *can_breathe = kind == "minecraft:cod";
                        *consume = match kind {
                            "minecraft:salmon" => i32::MAX,
                            _ => *consume + 4,
                        };
                    }
                },
                farmland_trample: |context, _, _, _, _, cancel| {
                    *cancel = context.setting("rules_mod", "stack").is_some();
                },
                has_mods: || false,
                player_name_format: |context, _, displayname| {
                    if let Some(value) = context.setting("rules_mod", "stack") {
                        *displayname = crate::text::Text::literal(&value);
                    }
                },
                player_tab_list_name_format: |context, _, _, display_name| {
                    *display_name = context
                        .setting("rules_mod", "stack")
                        .map(|value| crate::text::Text::literal(&value));
                },
                living_drown: |context, _, _, drowning, damage, bubbles, _| {
                    if context.setting("rules_mod", "stack").is_some() {
                        *drowning = true;
                        *damage += 1.0;
                        *bubbles = 0;
                    }
                },
                living_fall: |context, _, _, distance, multiplier, _| {
                    if context.setting("rules_mod", "stack").is_some() {
                        *distance += 1.0;
                        *multiplier *= 2.0;
                    }
                },
                living_jump: |context, _, _| {
                    JUMPS.fetch_add(
                        usize::from(context.side() == LoopSide::Server),
                        std::sync::atomic::Ordering::Relaxed,
                    );
                },
                living_jump_power: |context, _, _, power| {
                    *power += context.setting("rules_mod", "stack").map_or(0.0, |_| 1.0);
                },
                living_speed: |context, _, _, speed| {
                    *speed += context.setting("rules_mod", "stack").map_or(0.0, |_| 2.0);
                },
                living_flying_speed: |context, _, _, speed| {
                    *speed += context.setting("rules_mod", "stack").map_or(0.0, |_| 3.0);
                },
                block_jump_factor: |context, _, factor| {
                    *factor += context.setting("rules_mod", "stack").map_or(0.0, |_| 4.0);
                },
                block_speed_factor: |context, _, factor| {
                    *factor += context.setting("rules_mod", "stack").map_or(0.0, |_| 5.0);
                },
                is_ladder: |context, _, _, _, ladder| {
                    *ladder |= context.setting("rules_mod", "stack").is_some();
                },
                makes_open_trapdoor_above_climbable: |context, _, _, _, climbable| {
                    *climbable |= context.setting("rules_mod", "stack").is_some();
                },
                friction: |context, _, kind, _, _, friction| {
                    if context.setting("rules_mod", "stack").is_some() {
                        *friction = match kind {
                            "minecraft:player" => 0.98,
                            "minecraft:cow" => f32::NAN,
                            _ => 3.0,
                        };
                    }
                },
                player_stays_on_ground_surface: |context, _, staying| {
                    *staying |= context.setting("rules_mod", "stack").is_some();
                },
                ender_entity_teleport: |context, _, _, _, target, cancel| {
                    if context.setting("rules_mod", "stack").is_some() {
                        target[1] += 1.0;
                        *cancel = true;
                    }
                },
                fluid_flow: |context, _, _, flow| {
                    flow[0] += context.setting("rules_mod", "stack").map_or(0.0, |_| 1.0);
                },
                sound_type: |context, _, _, entity, cause, sound| {
                    if context.setting("rules_mod", "stack").is_some()
                        && entity.is_some_and(|(_, kind)| kind == "minecraft:player")
                        && cause == mod_api::BlockSoundCause::Step
                    {
                        sound.step = "rules_mod:block.rules.step".into();
                        sound.volume = 2.0;
                    }
                },
            });
        assert!(installed);
        let kind = Kind::Mod(Box::leak(Box::new(crate::containers::ModMenuType {
            id: "test:rules",
            slots: 1,
            data_slots: 0,
            results: vec![],
            links: None,
            crafting: None,
        })));
        let mut menu = Menu::new(kind);
        menu.set_registry_id(Some("test:rules"));
        let stack = ItemStack::new("minecraft:stone", 1);
        // A `/reload` builds the next context from the settings then on disk; the
        // next call sees it, and the one before it keeps its own.
        let settings = std::env::temp_dir().join(format!("horizon-rules-{}", std::process::id()));
        let file = settings.join("rules_mod/stack");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let inputs = Arc::new(LoadInputs::new(Default::default(), settings.clone()));
        let load = LoadContext::new(DataLoadOrigin::ServerDataLoad, inputs, []);
        let before = load.loop_context(LoopSide::Server, []);
        assert_eq!(before.setting("rules_mod", "stack"), None);
        bare_env_in(&before, |env| {
            assert_eq!(
                menu.slot_rules(env, 0, &stack, true, CONTAINER_MAX),
                (true, 7)
            );
        });
        std::fs::write(&file, "3").unwrap();
        let after = load.loop_context(LoopSide::Server, ["rules_mod"]);
        bare_env_in(&after, |env| {
            assert_eq!(
                menu.slot_rules(env, 0, &stack, true, CONTAINER_MAX),
                (true, 3)
            );
            // Slot 28 is hotbar 0 in this one-slot mod menu; both indices
            // refer to inventory cell 0 and the rule caps that cell at three.
            menu.set_player_item(0, Some(ItemStack::new("minecraft:stone", 8)));
            menu.click(env, 28, 0, Input::Swap).unwrap();
            assert_eq!(menu.player_item(0).unwrap().count, 8);
            assert_eq!(
                (0..menu.slot_count())
                    .filter_map(|i| menu.slot(i))
                    .map(|s| s.count)
                    .sum::<u32>(),
                8
            );
            // The container's index 0 is distinct, despite the same index.
            menu.load_slot(0, Some(ItemStack::new("minecraft:dirt", 2)));
            menu.click(env, 0, 0, Input::Swap).unwrap();
            assert_eq!(menu.slot(0).unwrap().count, 3);
            assert_eq!(menu.player_item(0).unwrap().count, 5);
            assert_eq!(
                (0..menu.slot_count())
                    .filter_map(|i| menu.slot(i))
                    .map(|s| s.count)
                    .sum::<u32>(),
                10
            );
        });
        bare_env_in(&before, |env| {
            assert_eq!(
                menu.slot_rules(env, 0, &stack, true, CONTAINER_MAX),
                (true, 7)
            );
        });
        // The other loop hooks read the same context: before the reload the
        // setting is missing, after it every one sees it.
        let stacked_on_other = |context: &LoopContext, menu: &mut Menu| {
            let mut canceled = false;
            bare_env_in(context, |env| {
                let mut carried = None;
                let mut view = MenuSlotView {
                    menu: &mut *menu,
                    env,
                    slot: 0,
                };
                let mut event = StackedOnOther {
                    player: None,
                    player_entity_id: None,
                    slot: 0,
                    container: MAIN,
                    container_slot: 0,
                    primary: true,
                    carried: CarriedAccess(&mut carried),
                    clicked: &mut view,
                    cancellation_result: false,
                };
                native_item_stacked_on_other(context, &mut event, &mut canceled);
            });
            canceled
        };
        let name = |context: &LoopContext| {
            let mut displayname = crate::text::Text::literal("Alex");
            crate::lan::hub::native_player_name_format(
                context,
                &crate::text::Text::literal("Alex"),
                &mut displayname,
            );
            displayname.plain()
        };
        let tab_list = |context: &LoopContext| {
            let mut display_name = None;
            crate::lan::hub::native_player_tab_list_name_format(
                context,
                1,
                "Alex",
                &mut display_name,
            );
            display_name.map(|text| text.plain())
        };
        assert!(!stacked_on_other(&before, &mut menu));
        assert_eq!(name(&before), "Alex");
        assert_eq!(tab_list(&before), None);
        assert!(stacked_on_other(&after, &mut menu));
        assert_eq!(name(&after), "3");
        assert_eq!(tab_list(&after), Some("3".to_owned()));
        // The motion value hooks read it too, and change nothing without it.
        let values = |context: &LoopContext| {
            let (mut jump, mut speed, mut flying) = (0.5, 0.1, 0.02);
            crate::motion::native_living_jump_power(context, 1, "minecraft:cow", &mut jump);
            crate::motion::native_living_speed(context, 1, "minecraft:cow", &mut speed);
            crate::motion::native_living_flying_speed(context, 1, "minecraft:cow", &mut flying);
            (jump, speed, flying)
        };
        assert_eq!(values(&before), (0.5, 0.1, 0.02));
        assert_eq!(values(&after), (1.5, 2.1, 3.02));
        // The block factors a part edits read it too, and are as the block's own without it.
        let factors = |context: &LoopContext| {
            use horizon_world::movement_properties::BlockFactorEdits;
            let edits = crate::motion::ModBlockFactors(context);
            let state = horizon_world::registry::BlockStateId(0);
            (edits.jump(state, 0.5), edits.speed(state, 0.4))
        };
        assert_eq!(factors(&before), (0.5, 0.4));
        assert_eq!(factors(&after), (4.5, 5.4));
        // The ladder test of a block reads it too.
        let ladder = |context: &LoopContext| {
            let mut ladder = false;
            crate::motion::native_is_ladder(
                context,
                1,
                horizon_core::position::BlockPos::new(0, 0, 0),
                horizon_world::registry::BlockStateId(0),
                &mut ladder,
            );
            ladder
        };
        assert!(!ladder(&before));
        assert!(ladder(&after));
        // So does the block under an open trapdoor.
        let trapdoor_ladder = |context: &LoopContext| {
            let mut climbable = false;
            crate::motion::ladder::native_makes_open_trapdoor_above_climbable(
                context,
                horizon_world::registry::BlockStateId(0),
                horizon_core::position::BlockPos::new(0, 0, 0),
                horizon_world::registry::BlockStateId(0),
                &mut climbable,
            );
            climbable
        };
        assert!(!trapdoor_ladder(&before));
        assert!(trapdoor_ladder(&after));
        // A block's friction reads it too; an answer that is NaN or outside 0..=2 keeps the
        // block's own.
        let friction = |context: &LoopContext, kind: &str| {
            let mut friction = 0.6;
            crate::motion::native_friction(
                context,
                1,
                kind,
                horizon_core::position::BlockPos::new(0, 0, 0),
                horizon_world::registry::BlockStateId(0),
                &mut friction,
            );
            friction
        };
        assert_eq!(friction(&before, "minecraft:player"), 0.6);
        assert_eq!(friction(&after, "minecraft:player"), 0.98);
        assert_eq!(friction(&after, "minecraft:cow"), 0.6);
        assert_eq!(friction(&after, "minecraft:item"), 0.6);
        // The edge backoff and the fluid current read it too.
        let movement = |context: &LoopContext| {
            let (mut staying, mut flow) = (false, [0.25, 0.0, 0.0]);
            crate::motion::native_player_stays_on_ground_surface(context, 1, &mut staying);
            crate::motion::native_fluid_flow(
                context,
                "minecraft:water",
                horizon_core::position::BlockPos::new(0, 0, 0),
                &mut flow,
            );
            (staying, flow[0])
        };
        assert_eq!(movement(&before), (false, 0.25));
        assert_eq!(movement(&after), (true, 1.25));
        // A block's sound type reads it too, with the entity and the cause it was asked for.
        let stone = crate::blocks::Sounds {
            volume: 1.0,
            pitch: 1.0,
            break_sound: "minecraft:block.stone.break".into(),
            step: "minecraft:block.stone.step".into(),
            place: "minecraft:block.stone.place".into(),
            hit: "minecraft:block.stone.hit".into(),
            fall: "minecraft:block.stone.fall".into(),
        };
        let step = |context: &LoopContext, kind: &str, cause| {
            let mut sound = crate::block_sounds::own(&stone);
            crate::block_sounds::native_sound_type(
                context,
                horizon_core::position::BlockPos::new(0, 0, 0),
                horizon_world::registry::BlockStateId(0),
                Some((1, kind)),
                cause,
                &mut sound,
            );
            (sound.step.to_string(), sound.volume)
        };
        let (own, answered) = (
            ("minecraft:block.stone.step".to_owned(), 1.0),
            ("rules_mod:block.rules.step".to_owned(), 2.0),
        );
        use mod_api::BlockSoundCause::{Place, Step};
        assert_eq!(step(&before, "minecraft:player", Step), own);
        assert_eq!(step(&after, "minecraft:player", Step), answered);
        assert_eq!(step(&after, "minecraft:cow", Step), own);
        assert_eq!(step(&after, "minecraft:player", Place), own);
        // Fall, trample, jump and the ender teleport read it as well.
        let landing = |context: &LoopContext| {
            let (mut distance, mut multiplier, mut cancel) = (3.0, 1.0, false);
            crate::landing::native_living_fall(
                context,
                1,
                "minecraft:cow",
                &mut distance,
                &mut multiplier,
                &mut cancel,
            );
            let mut trample = false;
            crate::landing::native_farmland_trample(context, 1, [0, 0, 0], 0, 1.0, &mut trample);
            let (mut target, mut teleport) = ([1.0, 2.0, 3.0], false);
            crate::mobs::enderman::native_ender_entity_teleport(
                context,
                1,
                "minecraft:enderman",
                [0.0; 3],
                &mut target,
                &mut teleport,
            );
            crate::motion::native_living_jump(context, 1, "minecraft:cow");
            (distance, multiplier, cancel, trample, target, teleport)
        };
        assert_eq!(
            landing(&before),
            (3.0, 1.0, false, false, [1.0, 2.0, 3.0], false)
        );
        assert_eq!(
            landing(&after),
            (4.0, 2.0, false, true, [1.0, 3.0, 3.0], true)
        );
        assert_eq!(JUMPS.load(std::sync::atomic::Ordering::Relaxed), 2);
        // Heal, breathe and drown read it as well.
        let health = |context: &LoopContext| {
            let (mut amount, mut cancel) = (2.0, false);
            crate::hunger::native_living_heal(context, 1, &mut amount, &mut cancel);
            let (mut can_breathe, mut consume, mut refill) = (true, 1, 4);
            crate::breathing::native_living_breathe(
                context,
                1,
                "minecraft:zombie",
                &mut can_breathe,
                &mut consume,
                &mut refill,
            );
            let (mut drowning, mut damage, mut cancel_drown) = (false, 2.0, false);
            crate::breathing::native_living_drown(
                context,
                1,
                "minecraft:zombie",
                &mut drowning,
                &mut damage,
                &mut 8,
                &mut cancel_drown,
            );
            (amount, cancel, can_breathe, consume, drowning, damage)
        };
        assert_eq!(health(&before), (2.0, false, true, 1, false, 2.0));
        assert_eq!(health(&after), (3.0, true, false, 5, true, 3.0));
        // A listener that lets a cod breathe: on land for 300 ticks it keeps
        // full air and takes no damage.
        let mut air = 300;
        for _ in 0..300 {
            let hurt =
                crate::breathing::own_air(&after, 1, "minecraft:cod", &mut air, false, 300, true);
            assert_eq!(hurt, None);
        }
        assert_eq!(air, 300);
        // A listener that sets `bubble_count` to 0: a drowning player takes the
        // damage and shows no bubbles (`drown_bubbles` sends nothing for 0).
        let gasp = crate::breathing::Breath {
            can_breathe: false,
            consume: 1,
            refill: 0,
        };
        let mut air = -19;
        let drowned =
            crate::breathing::drown(&after, 3, "minecraft:player", &mut air, gasp, true).unwrap();
        assert_eq!((drowned.damage, drowned.bubbles, air), (Some(3.0), 0, 0));
        // A listener's `consume` of `i32::MAX` saturates instead of overflowing:
        // a water animal past -20 only loses air, another kind drowns.
        let mut air = -19;
        let hurt =
            crate::breathing::own_air(&after, 4, "minecraft:salmon", &mut air, false, 300, true);
        assert_eq!((hurt, air), (None, i32::MIN));
        let mut air = -19;
        let hurt =
            crate::breathing::own_air(&after, 4, "minecraft:salmon", &mut air, false, 6000, false);
        assert_eq!((hurt, air), (Some(3.0), 0));
        // A server's block menu: the game's `stillValid` answer arrives first and
        // the parts on its mod menu type may change it; a vanilla menu type never
        // asks them, and a menu whose block entity is gone closes without asking.
        let mod_type: &'static crate::containers::ModMenuType =
            Box::leak(Box::new(crate::containers::ModMenuType {
                id: "test:rules",
                slots: 27,
                data_slots: 0,
                results: vec![],
                links: None,
                crafting: None,
            }));
        let still_valid = |context: &LoopContext,
                           mod_type: Option<&'static crate::containers::ModMenuType>,
                           registry_id: &str,
                           k: (i32, i32, i32),
                           entity: bool| {
            let mut world = crate::world::World::new(horizon_world::registry::BlockStateId(0), 0);
            if entity {
                world
                    .block_entities
                    .insert(k, crate::containers::BlockEntity::Chest(vec![None; 27]));
            }
            let guest = crate::lan::menu::tests::guest();
            let menu = crate::lan::menu::Menu {
                window: 1,
                kind: 2,
                pos: Some(k),
                slots: crate::lan::menu::Slots::Block(k),
                size: 27,
                state: 0,
                sent: Vec::new(),
                drag: None,
                sent_data: Vec::new(),
                hidden: Vec::new(),
                mod_type,
                registry_id: registry_id.into(),
                craft: None,
                links: Vec::new(),
                asks_still_valid: crate::lan::menu::asks_still_valid(mod_type, registry_id),
            };
            menu.still_valid(context, &world, &guest)
        };
        let asked = || STILL_VALID_ASKED.load(std::sync::atomic::Ordering::Relaxed);
        // The guest's eyes are at (0, 65.62, 0), its reach 4.5 plus 4: a chest 3
        // blocks away stays open, one 20 blocks away closes, and a vanilla menu
        // type asks no part either way, though one answers for it.
        assert!(crate::lan::menu::asks_still_valid(
            Some(mod_type),
            "minecraft:generic_9x3"
        ));
        let (near, far) = ((0, 64, 3), (0, 64, 20));
        assert!(still_valid(
            &after,
            None,
            "minecraft:generic_9x3",
            near,
            true
        ));
        assert!(!still_valid(
            &after,
            None,
            "minecraft:generic_9x3",
            far,
            true
        ));
        assert_eq!(asked(), 0);
        // The part keeps the far menu open, or closes the near one.
        assert!(still_valid(&after, Some(mod_type), "test:rules", far, true));
        assert!(!still_valid(
            &before,
            Some(mod_type),
            "test:rules",
            near,
            true
        ));
        assert_eq!(asked(), 2);
        // Gone: closed whatever the part would answer, and the part is not asked.
        assert!(!still_valid(
            &after,
            Some(mod_type),
            "test:rules",
            far,
            false
        ));
        assert_eq!(asked(), 2);
        std::fs::remove_dir_all(&settings).unwrap();
    }

    /// A cancelled `AnvilCraftEvent.Pre` refuses the take: the result sits in
    /// its slot, both inputs and the cost are as they were, and neither the
    /// cursor nor the inventory holds the item (no dupe).
    #[test]
    fn a_cancelled_anvil_craft_leaves_the_menu_as_it_was() {
        let root =
            std::env::temp_dir().join(format!("horizon-anvil-cancel-{}", std::process::id()));
        std::fs::create_dir_all(root.join("data/minecraft/recipe")).unwrap();
        let items = Items::for_tests(Default::default());
        let recipes = Recipes::load(
            &root,
            &items,
            &PackOverlay::NONE,
            &crate::test_resources::server_load(),
        )
        .unwrap();
        let brewing = Brewing::load(&root, &items, &PackOverlay::NONE);
        let _ = std::fs::remove_dir_all(&root);
        let (banners, station_recipes) = (Banners::default(), Default::default());
        let (enchantments, names) = (Default::default(), Default::default());
        let env = &Env {
            crafting_remainder: None,
            player: None,
            player_entity_id: None,
            loop_context: &crate::test_resources::loop_context(LoopSide::Server),
            items: &items,
            recipes: &recipes,
            brewing: &brewing,
            banners: &banners,
            station_recipes: &station_recipes,
            enchantments: &enchantments,
            names: &names,
            trade: None,
            maps: None,
            limited_crafting: false,
            can_equip: None,
            known_recipes: None,
            slot_rules: None,
        };
        // Both a pick-up onto the cursor and a shift-click into the inventory.
        for input in [Input::Pickup, Input::QuickMove] {
            let mut menu = Menu::new(Kind::Anvil);
            menu.creative = false;
            menu.levels = 10;
            menu.anvil_hook = true;
            menu.set_slot(env, 0, stack("iron_sword", 1));
            menu.set_slot(env, 1, stack("iron_ingot", 2));
            menu.set_slot(env, 2, stack("iron_sword", 1));
            menu.data[0] = 3;
            let before = (
                menu.slot(0).cloned(),
                menu.slot(1).cloned(),
                menu.slot(2).cloned(),
            );
            let undo = menu.anvil_undo();
            menu.click(env, 2, 0, input).unwrap();
            // The take waits for the mods, and the click moved the result.
            let craft = menu.anvil_craft.take().expect("the take is deferred");
            assert_eq!(craft.output, before.2.clone().unwrap());
            assert_eq!(menu.slot(2), None);
            assert_eq!(menu.data[0], 3);
            menu.anvil_craft = Some(craft);
            menu.anvil_cancel(undo);
            assert_eq!(
                (
                    menu.slot(0).cloned(),
                    menu.slot(1).cloned(),
                    menu.slot(2).cloned()
                ),
                before
            );
            assert_eq!(menu.data[0], 3);
            assert_eq!(menu.level_change, 0);
            assert_eq!(menu.carried, None);
            assert!((0..36).all(|i| menu.player_item(i).is_none()));
            assert!(menu.anvil_craft.is_none() && menu.dropped.is_empty());
        }
    }

    /// A mod menu type: three slots, then a result (slot 3).
    fn mod_menu() -> Kind {
        Kind::Mod(Box::leak(Box::new(crate::containers::ModMenuType {
            id: "test:press",
            slots: 4,
            data_slots: 0,
            results: vec![3],
            links: None,
            crafting: None,
        })))
    }

    /// A mod menu type's result takes nothing in by a click, a drag, a swap
    /// or a shift-click (`Slot.mayPlace`), not even a top-up of its own item,
    /// and gives what it holds. A slot its screen leaves out takes no click.
    #[test]
    fn a_mod_menu_result_is_take_only() {
        // No game data: an item table of no items, each stacking to 64.
        let root = std::env::temp_dir().join(format!("horizon-mod-menu-{}", std::process::id()));
        std::fs::create_dir_all(root.join("data/minecraft/recipe")).unwrap();
        let items = Items::for_tests(Default::default());
        let recipes = Recipes::load(
            &root,
            &items,
            &PackOverlay::NONE,
            &crate::test_resources::server_load(),
        )
        .unwrap();
        let brewing = Brewing::load(&root, &items, &PackOverlay::NONE);
        let _ = std::fs::remove_dir_all(&root);
        let (banners, station_recipes) = (Banners::default(), Default::default());
        let (enchantments, names) = (Default::default(), Default::default());
        let env = &Env {
            crafting_remainder: None,
            player: None,
            player_entity_id: None,
            loop_context: &crate::test_resources::loop_context(LoopSide::Server),
            items: &items,
            recipes: &recipes,
            brewing: &brewing,
            banners: &banners,
            station_recipes: &station_recipes,
            enchantments: &enchantments,
            names: &names,
            trade: None,
            maps: None,
            limited_crafting: false,
            can_equip: None,
            known_recipes: None,
            slot_rules: None,
        };
        let menu = &mut Menu::new(mod_menu());
        {
            menu.creative = false;
            let n = |s: Option<&ItemStack>| s.map(|s| s.count);
            // 0..4 the mod's, 4..31 inventory, 31..40 hotbar.
            menu.carried = stack("stick", 4);
            menu.click(env, 3, 0, Input::Pickup).unwrap();
            menu.click(env, 0, 0, Input::Pickup).unwrap();
            assert_eq!((n(menu.slot(3)), n(menu.slot(0))), (None, Some(4)));
            // The carried ingots take the result's one instead of going in.
            menu.put(env, 3, stack("iron_ingot", 1));
            menu.carried = stack("iron_ingot", 2);
            menu.click(env, 3, 0, Input::Pickup).unwrap();
            assert_eq!((n(menu.slot(3)), n(menu.carried.as_ref())), (None, Some(3)));
            // A drag over 1 to 3 passes the result by.
            menu.carried = stack("dirt", 4);
            for (slot, button) in [(-999, 0), (1, 1), (2, 1), (3, 1), (-999, 2)] {
                menu.click(env, slot, button, Input::QuickCraft).unwrap();
            }
            let held = |m: &Menu| [0, 1, 2, 3].map(|s| n(m.slot(s)));
            assert_eq!(held(menu), [Some(4), Some(2), Some(2), None]);
            // A shift-click from the hotbar tops up no result, and a hotbar
            // key puts nothing in it but takes from it into an empty slot.
            menu.put(env, 3, stack("iron_ingot", 1));
            menu.set_player_item(0, stack("iron_ingot", 3));
            menu.click(env, 31, 0, Input::QuickMove).unwrap();
            menu.click(env, 3, 0, Input::Swap).unwrap();
            assert_eq!(
                (n(menu.slot(3)), n(menu.player_item(0))),
                (Some(1), Some(3))
            );
            menu.click(env, 3, 1, Input::Swap).unwrap();
            assert_eq!((n(menu.slot(3)), n(menu.player_item(1))), (None, Some(1)));
            // F takes it into an empty off hand, and a double click collects from it.
            menu.put(env, 3, stack("iron_ingot", 2));
            menu.click(env, 3, 40, Input::Swap).unwrap();
            assert_eq!((n(menu.slot(3)), n(menu.player_item(40))), (None, Some(2)));
            menu.put(env, 3, stack("iron_ingot", 2));
            menu.set_player_item(0, None);
            menu.set_player_item(1, None);
            menu.carried = stack("iron_ingot", 1);
            menu.click(env, 4, 0, Input::PickupAll).unwrap();
            assert_eq!((n(menu.slot(3)), n(menu.carried.as_ref())), (None, Some(3)));
            // A hidden slot takes neither a click nor a shift-click.
            menu.set_player_item(0, stack("iron_ingot", 3));
            menu.put(env, 0, None);
            menu.hide_slot(0);
            menu.carried = stack("stone", 1);
            menu.click(env, 0, 0, Input::Pickup).unwrap();
            menu.click(env, 31, 0, Input::QuickMove).unwrap();
            assert_eq!((n(menu.slot(0)), n(menu.player_item(0))), (None, Some(3)));
        }
    }

    /// A mod menu type laid out as `CraftingMenu`: the result in slot 0, the
    /// grid in 1 to 9, as many container slots as the table's.
    pub(crate) fn mod_crafting_table() -> Kind {
        Kind::Mod(Box::leak(Box::new(crate::containers::ModMenuType {
            id: "test:station",
            slots: 10,
            data_slots: 0,
            results: vec![],
            links: None,
            crafting: Some(crate::containers::ModCrafting {
                grid: 1,
                size: 3,
                result: 0,
            }),
        })))
    }

    /// Everything a click leaves: the slots, the cursor, the drops and the
    /// recipes crafted.
    type Seen = (
        Vec<Option<ItemStack>>,
        Option<ItemStack>,
        Vec<ItemStack>,
        Vec<(String, Vec<Option<ItemStack>>)>,
    );
    fn seen(menu: &mut Menu) -> Seen {
        (
            (0..menu.slot_count())
                .map(|s| menu.slot(s).cloned())
                .collect(),
            menu.carried.clone(),
            std::mem::take(&mut menu.dropped),
            std::mem::take(&mut menu.crafted),
        )
    }

    /// A mod menu type's crafting grid crafts as the crafting table does
    /// (`CraftingMenu`, which `tools/test_menu_click_parity.py` pins to
    /// vanilla): the same clicks on both leave the same slots, cursor, drops
    /// and crafted recipes, remainders and shift-crafting included.
    #[test]
    fn a_mod_crafting_grid_crafts_as_the_crafting_table() {
        type Step = (&'static [(usize, &'static str, u32)], i32, i32, Input);
        let steps: [Step; 7] = [
            // Planks over planks: sticks, taken one craft at a time.
            (
                &[(1, "oak_planks", 5), (4, "oak_planks", 3)],
                0,
                0,
                Input::Pickup,
            ),
            (&[], 0, 1, Input::Pickup),
            // A cursor of sticks takes one more craft; then shift-crafting
            // makes as many as the grid allows.
            (&[], 0, 0, Input::QuickMove),
            // Cake: the milk buckets leave their buckets behind.
            (
                &[
                    (1, "milk_bucket", 1),
                    (2, "milk_bucket", 1),
                    (3, "milk_bucket", 1),
                    (4, "sugar", 1),
                    (5, "egg", 1),
                    (6, "sugar", 1),
                    (7, "wheat", 1),
                    (8, "wheat", 1),
                    (9, "wheat", 1),
                ],
                0,
                0,
                Input::QuickMove,
            ),
            // Shift-clicks from the inventory fill the grid first.
            (&[(9 + 10, "oak_log", 3)], 19, 0, Input::QuickMove),
            (&[], 0, 0, Input::QuickMove),
            // A number key takes the result into the hotbar.
            (&[(1, "stone", 3), (4, "stick", 2)], 0, 5, Input::Swap),
        ];
        with_menu(Kind::Crafting, |env, table| {
            let station = &mut Menu::new(mod_crafting_table());
            assert_eq!(station.slot_count(), table.slot_count());
            for (k, (put, slot, button, input)) in steps.into_iter().enumerate() {
                for menu in [&mut *table, &mut *station] {
                    for &(s, item, n) in put {
                        menu.set_slot(env, s, stack(item, n));
                    }
                    menu.click(env, slot, button, input).unwrap();
                }
                let (expected, got) = (seen(table), seen(station));
                assert_eq!(got, expected, "step {k}");
                if k == 0 {
                    assert!(got.1.is_some(), "the sticks were crafted");
                }
                if k == 3 {
                    assert!(got.3.iter().any(|(id, _)| id.ends_with("cake")));
                    assert!(
                        got.0[1]
                            .as_ref()
                            .is_some_and(|b| &*b.item == "minecraft:bucket")
                    );
                }
            }
        });
    }

    mod mod_grid_tests;

    /// Extra sources (`native_recipe_sources`) count as more inventory: the
    /// grid fills as the table's does when the player holds what the sources
    /// hold, the inventory gives first, then the sources in order, and a
    /// refused transfer leaves them as they were.
    #[test]
    fn recipe_placement_takes_from_the_sources_after_the_inventory() {
        with_menu(Kind::Crafting, |env, table| {
            let station = &mut Menu::new(mod_crafting_table());
            table.set_player_item(0, stack("oak_planks", 10));
            station.set_player_item(0, stack("oak_planks", 1));
            station.sources = vec![
                vec![None, stack("dirt", 5)],
                vec![stack("oak_planks", 2), stack("oak_planks", 7)],
            ];
            for menu in [&mut *table, &mut *station] {
                menu.place_recipe(env, "minecraft:crafting_table", false, false, None);
            }
            let grid = |m: &Menu| (0..10).map(|s| m.slot(s).cloned()).collect::<Vec<_>>();
            assert_eq!(grid(station), grid(table));
            assert_eq!(station.slot(0), stack("crafting_table", 1).as_ref());
            // 1 from the inventory, then 2 and 1 from the second source.
            assert_eq!(station.player_item(0), None);
            assert_eq!(
                station.sources,
                vec![
                    vec![None, stack("dirt", 5)],
                    vec![None, stack("oak_planks", 6)]
                ]
            );
            // A furnace needs 8 cobblestone, which no one has: nothing moves.
            let before = station.sources.clone();
            let refused = station.transfer_recipe(env, "minecraft:furnace", false);
            assert_eq!(refused, Err(place::TransferError::Missing));
            assert_eq!(station.sources, before);
        });
    }

    /// The block entity holds a crafting grid but never its result: loading
    /// its slots shows what they craft, and the slots it keeps leave the
    /// result out.
    #[test]
    fn a_mod_crafting_result_is_the_menus_own() {
        with_menu(mod_crafting_table(), |env, menu| {
            let mut items = vec![None; 10];
            items[0] = stack("diamond", 1);
            items[1] = stack("oak_log", 1);
            menu.load_container(env, &items);
            assert_eq!(menu.slot(0), stack("oak_planks", 4).as_ref());
            let kept = menu.container_items();
            assert_eq!(kept[0], None);
            assert_eq!(kept[1], stack("oak_log", 1));
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

    /// #1737: a Shift+Right-Click quick move in a chest (a double chest's
    /// 54 slots) moves the stack and leaves the menu's state clean, so the
    /// next quick move still moves its stack and the inventory still
    /// answers.
    #[test]
    fn a_chest_quick_move_keeps_the_next_one_working() {
        with_menu(Kind::Chest(6), |env, menu| {
            menu.creative = false;
            // The hotbar holds two stacks; the chest is empty.
            menu.set_slot(env, 81, stack("cobblestone", 64));
            menu.set_slot(env, 82, stack("dirt", 32));
            // Shift+Right-Click on the hotbar: the stack goes into the chest.
            menu.click(env, 81, 1, Input::QuickMove).unwrap();
            assert_eq!(menu.slot(81), None);
            assert_eq!(menu.slot(0).map(|s| s.count), Some(64));
            // Nothing is left on the cursor and no quick craft is running.
            assert_eq!(menu.carried, None);
            assert_eq!(menu.quick_status, 0);
            // The next quick move still works.
            menu.click(env, 82, 1, Input::QuickMove).unwrap();
            assert_eq!(menu.slot(82), None);
            assert_eq!(menu.slot(1).map(|s| s.count), Some(32));
            assert_eq!(menu.carried, None);
            assert_eq!(menu.quick_status, 0);
        });
    }

    /// #6907: a drag in progress is seen from outside the menu, and a
    /// quick move during it only ends the drag (`doClick`'s resetQuickCraft).
    #[test]
    fn a_quick_move_during_a_drag_only_ends_it() {
        with_menu(Kind::Chest(3), |env, menu| {
            menu.creative = false;
            menu.set_slot(env, 27, stack("dirt", 10));
            menu.carried = stack("cobblestone", 64);
            assert!(!menu.quick_crafting());
            // Start a left drag, then drag across chest slot 0.
            menu.click(env, -999, 0, Input::QuickCraft).unwrap();
            menu.click(env, 0, 1, Input::QuickCraft).unwrap();
            assert!(menu.quick_crafting());
            // A shift-click now moves nothing and ends the drag.
            menu.click(env, 27, 0, Input::QuickMove).unwrap();
            assert!(!menu.quick_crafting());
            assert_eq!(menu.slot(27).map(|s| s.count), Some(10));
            assert_eq!(menu.slot(0), None);
        });
    }

    /// #1832: a potion taken from the creative inventory and put in the
    /// hotbar keeps its `PotionContents` (the brew), as vanilla's
    /// `PotionContents.createItemStack` builds it: the item tab's stack is
    /// the real one, and the hotbar slot holds it as it is.
    #[test]
    fn a_creative_potion_keeps_its_contents_in_the_hotbar() {
        let snbt = r#"{components:{"minecraft:potion_contents":{potion:"minecraft:swiftness"}},count:1,id:"minecraft:potion"}"#;
        let potion = crate::item_data::parse_stack(snbt).unwrap().unwrap();
        assert_eq!(potion.potion(), Some("minecraft:swiftness"));
        // The creative screen hands the stack over...
        let mut carried = None;
        crate::creative::click_display(
            &mut carried,
            Some(&potion),
            |_: &ItemStack| 1,
            0,
            Input::Pickup,
        );
        assert_eq!(
            carried.as_ref().and_then(|s| s.potion()),
            Some("minecraft:swiftness")
        );
        // ... and the hotbar slot keeps it: it is not the uncraftable
        // potion, which is a `minecraft:potion` with no contents at all.
        with_menu(Kind::ItemPicker, |env, menu| {
            menu.carried = carried;
            menu.click(env, 45, 0, Input::Pickup).unwrap();
            let in_hand = menu.slot(45).expect("the hotbar slot");
            assert_eq!(in_hand.potion(), Some("minecraft:swiftness"));
            assert_eq!(&*in_hand.item, "minecraft:potion");
        });
    }

    /// `RecipeCraftingHolder.setRecipeUsed`: when `limited_crafting` is false,
    /// crafting works even if the recipe is not known by the player.
    #[test]
    fn limited_crafting_disabled_allows_crafting_unknown_recipe() {
        use crate::crafting::{Ingredient, Recipe, Template};
        let mut recipes = Recipes::empty();
        recipes.crafting.push((
            "oak_planks".into(),
            Recipe::Shapeless {
                ingredients: vec![Ingredient::of(vec!["minecraft:oak_log".into()])],
                result: Template {
                    item: Some("minecraft:oak_planks".into()),
                    count: 4,
                    damage: 0,
                    extra: None,
                    components: None,
                },
            },
        ));
        let items = Items::for_tests(Default::default());
        let brewing = Brewing::load(&std::path::PathBuf::new(), &items, &PackOverlay::NONE);
        let banners = Banners::default();
        let station_recipes = crate::stations::StationRecipes::default();
        let enchantments = crate::enchant::Enchantments::default();
        let names = crate::creative::Names::default();

        let env = Env {
            crafting_remainder: None,
            items: &items,
            recipes: &recipes,
            brewing: &brewing,
            banners: &banners,
            station_recipes: &station_recipes,
            enchantments: &enchantments,
            names: &names,
            trade: None,
            maps: None,
            player: None,
            player_entity_id: None,
            loop_context: &crate::test_resources::loop_context(LoopSide::Client),
            limited_crafting: false,
            can_equip: None,
            known_recipes: None,
            slot_rules: None,
        };

        let mut menu = Menu::new(Kind::Inventory);
        menu.set_slot(&env, 1, stack("oak_log", 1));
        assert_eq!(
            menu.slot(0).map(|s| (&*s.item, s.count)),
            Some(("minecraft:oak_planks", 4)),
            "when limited_crafting is false, unlearned recipe should craft"
        );
    }

    /// `RecipeCraftingHolder.setRecipeUsed`: when `limited_crafting` is true,
    /// an unlearned recipe is blocked (result slot is None).
    #[test]
    fn limited_crafting_enabled_blocks_unknown_recipe() {
        use crate::crafting::{Ingredient, Recipe, Template};
        let mut recipes = Recipes::empty();
        recipes.crafting.push((
            "oak_planks".into(),
            Recipe::Shapeless {
                ingredients: vec![Ingredient::of(vec!["minecraft:oak_log".into()])],
                result: Template {
                    item: Some("minecraft:oak_planks".into()),
                    count: 4,
                    damage: 0,
                    extra: None,
                    components: None,
                },
            },
        ));
        let items = Items::for_tests(Default::default());
        let brewing = Brewing::load(&std::path::PathBuf::new(), &items, &PackOverlay::NONE);
        let banners = Banners::default();
        let station_recipes = crate::stations::StationRecipes::default();
        let enchantments = crate::enchant::Enchantments::default();
        let names = crate::creative::Names::default();

        let known = ["minecraft:stick"];
        let env = Env {
            crafting_remainder: None,
            items: &items,
            recipes: &recipes,
            brewing: &brewing,
            banners: &banners,
            station_recipes: &station_recipes,
            enchantments: &enchantments,
            names: &names,
            trade: None,
            maps: None,
            player: None,
            player_entity_id: None,
            loop_context: &crate::test_resources::loop_context(LoopSide::Client),
            limited_crafting: true,
            can_equip: None,
            known_recipes: Some(&known),
            slot_rules: None,
        };

        let mut menu = Menu::new(Kind::Inventory);
        menu.set_slot(&env, 1, stack("oak_log", 1));
        assert_eq!(
            menu.slot(0),
            None,
            "when limited_crafting is true, unknown recipe must produce None in result slot"
        );
    }

    /// `RecipeCraftingHolder.setRecipeUsed`: when `limited_crafting` is true,
    /// a learned recipe (present in `known_recipes`) crafts successfully.
    #[test]
    fn limited_crafting_enabled_allows_known_recipe() {
        use crate::crafting::{Ingredient, Recipe, Template};
        let mut recipes = Recipes::empty();
        recipes.crafting.push((
            "oak_planks".into(),
            Recipe::Shapeless {
                ingredients: vec![Ingredient::of(vec!["minecraft:oak_log".into()])],
                result: Template {
                    item: Some("minecraft:oak_planks".into()),
                    count: 4,
                    damage: 0,
                    extra: None,
                    components: None,
                },
            },
        ));
        let items = Items::for_tests(Default::default());
        let brewing = Brewing::load(&std::path::PathBuf::new(), &items, &PackOverlay::NONE);
        let banners = Banners::default();
        let station_recipes = crate::stations::StationRecipes::default();
        let enchantments = crate::enchant::Enchantments::default();
        let names = crate::creative::Names::default();

        let known = ["minecraft:oak_planks"];
        let env = Env {
            crafting_remainder: None,
            items: &items,
            recipes: &recipes,
            brewing: &brewing,
            banners: &banners,
            station_recipes: &station_recipes,
            enchantments: &enchantments,
            names: &names,
            trade: None,
            maps: None,
            player: None,
            player_entity_id: None,
            loop_context: &crate::test_resources::loop_context(LoopSide::Client),
            limited_crafting: true,
            can_equip: None,
            known_recipes: Some(&known),
            slot_rules: None,
        };

        let mut menu = Menu::new(Kind::Inventory);
        menu.set_slot(&env, 1, stack("oak_log", 1));
        assert_eq!(
            menu.slot(0).map(|s| (&*s.item, s.count)),
            Some(("minecraft:oak_planks", 4)),
            "when limited_crafting is true, learned recipe in known_recipes must craft"
        );
    }

    /// `RecipeCraftingHolder.setRecipeUsed`: special recipes bypass the `known_recipes` check
    /// (`recipe.value().isSpecial()`) and craft even when limited crafting is enabled.
    #[test]
    fn limited_crafting_enabled_allows_special_recipe() {
        use crate::crafting::{Ingredient, Recipe, Special, Template};
        let mut recipes = Recipes::empty();
        recipes.crafting.push((
            "dye_shulker_box".into(),
            Recipe::Special(Special::Transmute {
                input: Ingredient::of(vec!["minecraft:shulker_box".into()]),
                material: Ingredient::of(vec!["minecraft:red_dye".into()]),
                min: 1,
                max: 1,
                add_count: false,
                result: Template {
                    item: Some("minecraft:red_shulker_box".into()),
                    count: 1,
                    damage: 0,
                    extra: None,
                    components: None,
                },
            }),
        ));
        let items = Items::for_tests(Default::default());
        let brewing = Brewing::load(&std::path::PathBuf::new(), &items, &PackOverlay::NONE);
        let banners = Banners::default();
        let station_recipes = crate::stations::StationRecipes::default();
        let enchantments = crate::enchant::Enchantments::default();
        let names = crate::creative::Names::default();

        let known: [&str; 0] = [];
        let env = Env {
            crafting_remainder: None,
            items: &items,
            recipes: &recipes,
            brewing: &brewing,
            banners: &banners,
            station_recipes: &station_recipes,
            enchantments: &enchantments,
            names: &names,
            trade: None,
            maps: None,
            player: None,
            player_entity_id: None,
            loop_context: &crate::test_resources::loop_context(LoopSide::Client),
            limited_crafting: true,
            can_equip: None,
            known_recipes: Some(&known),
            slot_rules: None,
        };

        let mut menu = Menu::new(Kind::Inventory);
        menu.set_slot(&env, 1, stack("shulker_box", 1));
        menu.set_slot(&env, 2, stack("red_dye", 1));
        assert_eq!(
            menu.slot(0).map(|s| (&*s.item, s.count)),
            Some(("minecraft:red_shulker_box", 1)),
            "special recipe must craft with limited_crafting: true without being in known_recipes"
        );
    }

    /// `BrewingStandMenu.PotionSlot.onTake`: a pickup and a shift-click on a
    /// bottle slot each queue the taken stack once for the game's hook adapter;
    /// with no listener nothing is kept.
    #[test]
    fn a_bottle_slot_take_records_once_when_a_hook_listens() {
        let root = std::env::temp_dir().join(format!("horizon-brewed-{}", std::process::id()));
        std::fs::create_dir_all(root.join("data/minecraft/recipe")).unwrap();
        let items = horizon_items::items::Items::for_tests(Default::default());
        let overlay = &horizon_world::loot::PackOverlay::NONE;
        let recipes = Recipes::load(
            &root,
            &items,
            overlay,
            &crate::test_resources::server_load(),
        )
        .unwrap();
        let brewing = Brewing::load(&root, &items, overlay);
        let _ = std::fs::remove_dir_all(&root);
        let (banners, station_recipes) = (Default::default(), Default::default());
        let (enchantments, names) = (Default::default(), Default::default());
        let env = &Env {
            crafting_remainder: None,
            player: Some(7),
            player_entity_id: Some(41),
            loop_context: &crate::test_resources::loop_context(LoopSide::Server),
            items: &items,
            recipes: &recipes,
            brewing: &brewing,
            banners: &banners,
            station_recipes: &station_recipes,
            enchantments: &enchantments,
            names: &names,
            trade: None,
            maps: None,
            limited_crafting: false,
            can_equip: None,
            known_recipes: None,
            slot_rules: None,
        };
        let potion = ItemStack::new("minecraft:potion", 1);
        let mut seen = Vec::new();
        for input in [Input::Pickup, Input::QuickMove] {
            let mut menu = Menu::new(Kind::Brewing);
            menu.set_slot(env, 0, Some(potion.clone()));
            menu.click(env, 0, 0, input).unwrap();
            // Without a listener the take is only the advancement's.
            assert!(menu.brewed_potion_takes.is_empty(), "{input:?}");
            assert_eq!(menu.brewed, std::slice::from_ref(&potion), "{input:?}");

            let mut menu = Menu::new(Kind::Brewing);
            menu.brewed_potion_hook = true;
            menu.set_slot(env, 0, Some(potion.clone()));
            menu.click(env, 0, 0, input).unwrap();
            assert_eq!(
                menu.brewed_potion_takes,
                std::slice::from_ref(&potion),
                "{input:?}"
            );
            for stack in &menu.brewed_potion_takes {
                seen.push(stack.clone());
            }
        }
        assert_eq!(seen, [potion.clone(), potion]);
    }
    #[path = "../dfa_tests.rs"]
    mod dfa;
}

#[cfg(test)]
mod storage_tests;
