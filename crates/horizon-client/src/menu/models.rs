//! Engine menu records and slot access.
use super::ItemStack;
/// Operations on the clicked slot, including its placement and take rules.
pub trait SlotView {
    /// Run against staged owner state; commit all changes only when accepted.
    fn with_staged(&mut self, edit: &mut dyn FnMut(&mut dyn SlotView) -> bool) -> bool;
    fn items(&self) -> &horizon_items::items::Items;
    fn item(&self) -> Option<ItemStack>;
    fn may_place(&self, stack: &ItemStack) -> bool;
    fn may_pickup(&self) -> bool;
    fn allow_modification(&self) -> bool;
    fn safe_insert(&mut self, stack: Option<ItemStack>, count: u32) -> Option<ItemStack>;
    fn safe_take(&mut self, count: u32, max: u32) -> Option<ItemStack>;
}
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
/// Who and where a `menu_slot_rules` ask is about: the player whose menu it is, the menu's
/// block, the container the asked slot indexes, and the menu's own containers' slots as they
/// stand when the click plans (read only, borrowed; the player's inventory slots are not in
/// it). None where the menu has no such thing: a client's prediction knows no other player,
/// and a menu opened with no block has no `pos`. `snapshot` is the click plan's one copy for
/// the parts (see [`super::SlotSnapshot`]); a context without one copies per ask.
#[derive(Clone, Copy, Debug, Default)]
pub struct SlotContext<'a> {
    pub player: Option<u128>,
    pub pos: Option<[i32; 3]>,
    pub container: mod_api::MenuContainer,
    pub slots: &'a [Option<ItemStack>],
    /// A flattened LAN container stores grid/result values in Main's unused positions.
    /// These positions are empty in the prediction's separate Main container.
    pub main_crafting: Option<crate::containers::ModCrafting>,
    pub grid: &'a [Option<ItemStack>],
    pub result: &'a [Option<ItemStack>],
    pub snapshot: Option<&'a super::SlotSnapshot>,
}

/// `menu_slot_rules` inside a menu click, which cannot reach the game: the game lends the menu
/// one for the click. A click asks it once for each slot and stack it consults.
pub trait SlotRules {
    /// Whether a part answers the menu type `menu`; a menu type none answers is never asked.
    fn listens(&self, menu: &str) -> bool;
    /// The slot's own answers arrive in `may_place` and `max_stack`; the parts may change them.
    /// `context` is who and where the ask is about (see [`SlotContext`]).
    fn rule(
        &self,
        menu: &str,
        slot: usize,
        stack: &ItemStack,
        context: SlotContext<'_>,
        may_place: &mut bool,
        max_stack: &mut u32,
    );
}

/// What `AnvilUpdateEvent` asks about once `createResult` made vanilla's
/// result with a left input, and what the mods may change: the `output`,
/// its level `cost` and `material_cost`, the units of `right` used (0 or
/// more than it holds: all of it), each starting at vanilla's.
#[derive(Clone, Debug, PartialEq)]
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
        vanilla: &crate::stations::AnvilResult,
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
    pub fn answer(self, cancel: bool, result: &mut crate::stations::AnvilResult) {
        if cancel {
            *result = crate::stations::AnvilResult::default();
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
pub struct AnvilCraft {
    pub left: Option<ItemStack>,
    pub right: Option<ItemStack>,
    pub output: ItemStack,
}
