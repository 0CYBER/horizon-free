//! Moved here from the game so mods can name it.
#[allow(unused_imports)]
use crate::*;
/// One click on the local player's open menu, submitted with `MenuRef::click`.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
pub enum MenuClick {
    /// Pick up, place or drop the carried stack; `None` means outside.
    Pickup {
        slot: Option<usize>,
        button: MouseButton,
    },
    /// Shift-click a menu-order slot into the opposite inventory.
    QuickMove { slot: usize },
    /// Exchange a menu-order slot with a hotbar slot or the offhand.
    Swap { slot: usize, with: SwapWith },
    /// Clone a menu-order slot in creative mode.
    Clone { slot: usize },
    /// Drop one item or the whole stack from a menu-order slot.
    Throw { slot: usize, whole_stack: bool },
    /// Distribute the carried stack across menu-order slots as one complete gesture.
    Drag { kind: DragKind, slots: Vec<usize> },
    /// Collect matching stacks into the carried stack.
    PickupAll { slot: usize },
}

/// The mouse button used to pick up or place a stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the two vanilla pickup buttons
#[derive(horizon_mod_api_macros::Wire)]
pub enum MouseButton {
    Left,
    Right,
}

/// The inventory location exchanged with a menu slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: vanilla swaps target the hotbar or the offhand
#[derive(horizon_mod_api_macros::Wire)]
pub enum SwapWith {
    /// A hotbar index in 0..=8; other indices refuse the entire batch.
    Hotbar(u8),
    Offhand,
}

/// Whether `open_menu` or `PlayerRef::open_mod_menu` first sends the client a close for the menu
/// it replaces, as a Java `MenuProvider` chooses (`shouldTriggerClientSideContainerClosingOnOpen`).
/// Either way the old menu closes on the server, its cursor stack going back to the player's
/// inventory and `player_container_close` firing once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: vanilla's choice is a boolean
#[derive(horizon_mod_api_macros::Wire)]
pub enum MenuReplace {
    /// `ServerPlayer.closeContainer`, vanilla's default: the client closes the old screen first,
    /// which centres the mouse again.
    CloseFirst,
    /// `ServerPlayer.doCloseContainer`: the new screen takes the old one's place, title and rows
    /// included, and the mouse stays where it is.
    InPlace,
}

/// How a complete drag distributes the carried stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: vanilla has split, one-per-slot and creative-fill drags
#[derive(horizon_mod_api_macros::Wire)]
pub enum DragKind {
    Split,
    One,
    /// Fill each slot in creative mode only.
    Clone,
}

/// Identity and location of a player pressing a mod container button.
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct MenuButtonContext {
    pub player: u128,
    pub player_entity_id: i32,
    pub pos: [i32; 3],
    pub id: i32,
}
/// The logical side on which a menu click runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(horizon_mod_api_macros::Wire)]
pub enum LogicalSide {
    Client,
    Server,
}
/// Operations on the clicked slot, including its placement and take rules.
pub trait SlotView {
    fn item(&self) -> Option<ItemStack>;
    /// Test placement without imposing resident stack count or damage maxima.
    /// Unknown item identities or unrepresentable component payloads still refuse.
    fn may_place(&self, stack: &ItemStack) -> bool;
    fn may_pickup(&self) -> bool;
    fn allow_modification(&self) -> bool;
    fn safe_insert(&mut self, stack: Option<ItemStack>, count: u32) -> Option<ItemStack>;
    fn safe_take(&mut self, count: u32, max: u32) -> Option<ItemStack>;
}
// no wire: borrowed call-scoped view; transport uses owned snapshots rather than serializing references.
/// Access to the carried stack, as `createCarriedSlotAccess`.
// exhaustive: the game builds it with a literal; mods only read it
pub struct CarriedAccess<'a>(#[doc(hidden)] pub &'a mut Option<ItemStack>);
impl CarriedAccess<'_> {
    pub fn get(&self) -> Option<&ItemStack> {
        self.0.as_ref()
    }
    pub fn set(&mut self, stack: Option<ItemStack>) {
        *self.0 = stack;
    }
}
// no wire: borrowed call-scoped view; transport uses owned snapshots rather than serializing references.
/// A pickup click's slot and cursor, including empty stacks.
// exhaustive: the game builds it with a literal; mods only read it
pub struct StackedOnOther<'a> {
    pub player: Option<u128>,
    pub player_entity_id: Option<i32>,
    pub slot: usize,
    pub container: usize,
    pub container_slot: usize,
    pub primary: bool,
    pub carried: CarriedAccess<'a>,
    pub clicked: &'a mut dyn SlotView,
    pub cancellation_result: bool,
}
/// What `AnvilUpdateEvent` asks about once `createResult` made vanilla's
/// result with a left input, and what the mods may change: the `output`,
/// its level `cost` and `material_cost`, the units of `right` used (0 or
/// more than it holds: all of it), each starting at vanilla's.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct AnvilUpdate {
    pub left: ItemStack,
    pub right: Option<ItemStack>,
    /// The rename box's text, `None` when the client sent none.
    pub name: Option<String>,
    /// Vanilla's result for these inputs (`getVanillaResult`), whatever
    /// the mods set.
    pub vanilla: AnvilVanilla,
    pub output: Option<ItemStack>,
    /// Signed level cost, preserved as set by `AnvilUpdateEvent.setXpCost`.
    /// A negative cost prevents taking the result (as does zero), not a level refund.
    pub xp_cost: i32,
    pub material_cost: i32,
}
/// `AnvilUpdateEvent.VanillaResult`: the output, level cost and material
/// cost `createResult` made.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct AnvilVanilla {
    pub output: Option<ItemStack>,
    pub xp_cost: i32,
    pub material_cost: i32,
}
impl AnvilUpdate {
    /// The question about `vanilla`, the result `createResult` made from
    /// these inputs and name: the output, level cost and material cost
    /// start at its own.
    pub fn new(
        left: ItemStack,
        right: Option<ItemStack>,
        name: Option<String>,
        vanilla: &crate::item::stations::AnvilResult,
    ) -> Self {
        let vanilla = AnvilVanilla {
            output: vanilla.result.clone(),
            xp_cost: vanilla.cost,
            material_cost: i32::try_from(vanilla.repair_item_count_cost).unwrap_or(i32::MAX),
        };
        Self {
            left,
            right,
            name,
            output: vanilla.output.clone(),
            xp_cost: vanilla.xp_cost,
            material_cost: vanilla.material_cost,
            vanilla,
        }
    }

    /// The mods' answer, in place of `result`: a cancel leaves no result at
    /// cost 0; otherwise their output, signed level cost and nonnegative
    /// material cost. A changed output is no longer only a rename.
    pub fn answer(self, cancel: bool, result: &mut crate::item::stations::AnvilResult) {
        if cancel {
            *result = crate::item::stations::AnvilResult::default();
            return;
        }
        result.only_renaming &= self.output == self.vanilla.output;
        result.cost = self.xp_cost;
        result.repair_item_count_cost = self.material_cost.max(0) as u32;
        result.result = self.output;
    }
}
/// What `GrindstoneEvent.OnPlaceItem` asks about once the inputs changed,
/// and what the mods may set: an `output` (none until a mod sets one) and,
/// with it, `xp` (-1 for vanilla's experience). `vanilla_output` is the
/// vanilla's result, to read; set `output` to replace it.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct GrindstonePlace {
    pub top: Option<ItemStack>,
    pub bottom: Option<ItemStack>,
    pub vanilla_output: Option<ItemStack>,
    pub output: Option<ItemStack>,
    pub xp: i32,
}
/// A grindstone result taken, as `GrindstoneEvent.OnTakeItem` reports it:
/// the inputs, the experience (-1 for vanilla's, or what OnPlaceItem set)
/// and what the inputs hold afterwards (empty, as vanilla's).
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct GrindstoneTake {
    pub top: Option<ItemStack>,
    pub bottom: Option<ItemStack>,
    pub xp: i32,
    pub new_top: Option<ItemStack>,
    pub new_bottom: Option<ItemStack>,
}
/// An anvil craft as `AnvilCraftEvent` reports it: the inputs and the result
/// taken, before the crafting changed anything.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct AnvilCraft {
    pub left: Option<ItemStack>,
    pub right: Option<ItemStack>,
    pub output: ItemStack,
}
