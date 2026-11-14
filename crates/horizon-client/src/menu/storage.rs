//! Menu-owned state and conservative mutation versions.
//!
//! Mutable access invalidates a whole-menu read set even when the value is
//! restored before the next read. Storage identity changes when a menu is
//! cloned or replaced; moving a menu retains its identity.
use super::*;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicU64, Ordering};

/// Opaque identity and mutation version for an immutable menu read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MutationStamp {
    instance: u64,
    revision: u64,
}

impl MutationStamp {
    pub(super) fn fresh() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self {
            instance: NEXT
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
                .expect("menu identities exhausted"),
            revision: 0,
        }
    }

    fn advance(&mut self) {
        self.revision = self
            .revision
            .checked_add(1)
            .expect("menu mutation version exhausted");
    }
}

impl Clone for Menu {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
            stamp: MutationStamp::fresh(),
        }
    }
}

impl Deref for Menu {
    type Target = State;

    fn deref(&self) -> &State {
        &self.state
    }
}

impl DerefMut for Menu {
    fn deref_mut(&mut self) -> &mut State {
        self.touch();
        &mut self.state
    }
}

impl Menu {
    /// Immutable reads do not advance this version. A mutable field borrow or
    /// menu operation advances it before user code can edit or panic.
    pub fn mutation_stamp(&self) -> MutationStamp {
        self.stamp
    }

    /// Capture actual menu state for the game transaction's undo journal.
    /// The stamp is deliberately excluded: rollback must invalidate live reads.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot(self.state.clone())
    }

    /// Install already planned values without invoking rules, recipes or hooks.
    pub fn restore(&mut self, snapshot: Snapshot) {
        self.touch();
        self.state = snapshot.0;
    }

    pub(super) fn touch(&mut self) {
        self.stamp.advance();
        self.state.slot_snapshot.clear();
    }

    /// Commit a staged edit into this same menu; do not install the private
    /// preview's identity or revision as the live owner's version.
    pub(super) fn install_state(&mut self, staged: Self) {
        self.restore(Snapshot(staged.state));
    }
}

/// Engine state only; never a public mod token or a serialized hook payload.
#[derive(Clone)]
pub struct Snapshot(State);

/// The menu's editable values. Access through `Menu` advances its version;
/// callers cannot mutate the identity or reset a revision.
#[derive(Clone)]
pub struct State {
    pub(super) kind: Kind,
    pub(super) registry_id: Option<Box<str>>,
    /// The block whose menu this is, for `menu_slot_rules`; None with no block.
    pub(super) block_pos: Option<[i32; 3]>,
    /// The parts' copy of the slots for this click plan; emptied by every mutation.
    pub(super) slot_snapshot: super::SlotSnapshot,
    pub(super) slots: Vec<Slot>,
    pub(super) containers: [Vec<Option<ItemStack>>; 4],
    pub(super) grid_size: usize,
    pub(super) container_limits: Vec<Option<u32>>,
    pub carried: Option<ItemStack>,
    pub data: Vec<i32>,
    pub creative: bool,
    /// The selected hotbar slot (`Inventory.selected`).
    pub selected: usize,
    /// Dropped stacks (canonical text), `xp+N` and the like, in order.
    pub drops: Vec<String>,
    /// The dropped stacks themselves, for the caller to throw.
    pub dropped: Vec<ItemStack>,
    pub(super) quick_status: i32,
    pub(super) quick_type: i32,
    pub(super) quick_slots: Vec<usize>,
    /// `LoomMenu.selectablePatterns`.
    pub loom_patterns: Vec<Arc<str>>,
    /// Sounds the menu plays (`ui.loom.take_result`), for the caller.
    pub sounds: Vec<&'static str>,
    /// Sounds the player makes (a bundle's insert and remove), for the
    /// caller to play at the player as `Entity.playSound`.
    pub player_sounds: Vec<&'static str>,
    /// Recipes whose result was taken (`awardUsedRecipes`) and the stacks that
    /// went into each, for the caller.
    pub crafted: Vec<(String, Vec<Option<ItemStack>>)>,
    /// A crafting grid's result taken (`ResultSlot.onTake`): the recipe, the
    /// stack taken and the grid row by row before the take used it, for the
    /// caller (`native_item_crafted`).
    pub item_crafted: Vec<(String, ItemStack, Vec<Option<ItemStack>>)>,
    /// `ItemStack.onCraftedBy` counts from merchant result callbacks, without a recipe id.
    pub crafted_counts: Vec<(Arc<str>, i32)>,
    /// `MerchantResultSlot.removeCount`, reset by checkTakeAchievements.
    pub(super) merchant_remove_count: i32,
    /// Potions taken from a brewing stand's bottle slots
    /// (`PotionSlot.onTake`), for the caller.
    pub brewed: Vec<ItemStack>,
    /// Set by a server caller when a mod listens: every stack taken from a
    /// brewing stand's bottle slots then goes to `brewed_potion_takes`.
    pub brewed_potion_hook: bool,
    /// Stacks taken from brewing stand bottle slots, for
    /// `PlayerBrewedPotionEvent`.
    pub brewed_potion_takes: Vec<ItemStack>,
    /// Set by a server caller when a mod listens: every
    /// `FurnaceResultSlot.checkTakeAchievements` then goes to `item_smelted`.
    pub item_smelted_hook: bool,
    /// What each `checkTakeAchievements` of a furnace's result slot passed,
    /// for `ItemSmeltedEvent`: the stack (`None` for vanilla's empty one)
    /// and `removeCount`.
    pub item_smelted: Vec<(Option<ItemStack>, u32)>,
    /// Set by a server caller when a mod listens: every smithing result
    /// taken then goes to `item_smithed`.
    pub item_smithed_hook: bool,
    /// Each smithing result taken (`SmithingMenu.onTake`, or `onQuickCraft`
    /// for a shift-click), for `ItemSmithedEvent`: the template, base and
    /// addition as they were before the take used them, and the stack taken.
    pub item_smithed: Vec<([Option<ItemStack>; 3], ItemStack)>,
    /// `FurnaceResultSlot.removeCount`.
    pub(super) output_removed: u32,
    /// `LivingEntity.onEquipItem` calls that change the item: the
    /// `EquipmentSlot` name and the new stack, in order, for the caller.
    pub equipped: Vec<(&'static str, Option<ItemStack>)>,
    /// The merchant a merchant menu trades with.
    pub merchant: Option<Box<crate::merchant::Session>>,
    /// The stonecutter's recipe list (`recipesForInput`).
    pub stone: Stonecutter,
    /// The grindstone result's `getExperienceAmount` sum before its roll,
    /// set when the result was taken: the caller (with a level) awards the
    /// orbs and plays level event 1042.
    pub ground: Option<i32>,
    /// `Player.experienceLevel`, which the anvil's result needs.
    pub levels: i32,
    /// The levels the menu gave the player (`giveExperienceLevels`), for the caller.
    pub level_change: i32,
    /// `AnvilMenu`'s state besides its slots and its cost data slot.
    pub anvil: Anvil,
    /// The anvil's or the smithing table's result was taken: the caller (with
    /// a level) runs the rest of its `onTake` (the anvil's wear and level
    /// event 1030 or 1029, the smithing table's 1044).
    pub combined: bool,
    /// The caller fires `AnvilCraftEvent.Pre` for a taken anvil result: its
    /// `onTake` waits in `anvil_craft` for the caller to run `anvil_take`, or
    /// to refuse the take with `anvil_cancel` (from an `anvil_undo` made
    /// before the click).
    pub anvil_hook: bool,
    /// The anvil result taken while `anvil_hook` is set, as it was before
    /// crafting.
    pub anvil_craft: Option<AnvilCraft>,
    /// The caller fires `GrindstoneEvent.OnTakeItem` for a taken grindstone
    /// result: its `onTake` waits in `grind_take` for `grindstone_taken`.
    pub grind_hook: bool,
    /// The grindstone result taken while `grind_hook` is set, with its
    /// inputs as they were.
    pub grind_take: Option<GrindstoneTake>,
    /// `GrindstoneEvent.OnPlaceItem`'s experience for the result now in the
    /// slot, or -1 for vanilla's.
    pub(super) grind_xp: i32,
    /// `AnvilMenu.createResult` ran since `anvil_created` last asked.
    pub(super) anvil_made: bool,
    /// The enchanting table's item changed to an enchantable one: the caller
    /// (with a level) fills the costs and clues (`slotsChanged`'s
    /// `access.execute`).
    pub enchant_changed: bool,
    /// An enchanting button the menu took (`clickMenuButton` passed its
    /// checks): the caller (with a level) enchants.
    pub enchant_button: Option<usize>,
    /// `Player.mayBuild` (not adventure mode): the lectern's Take Book.
    pub may_build: bool,
    /// A mount menu's entity type (`minecraft:horse`): what its saddle and
    /// body slots take.
    pub mount: Option<String>,
    /// Containers a recipe placement may also take ingredients from, after
    /// the player's inventory (`native_recipe_sources`): the caller fills
    /// them before and reads them back after.
    pub sources: Vec<Vec<Option<ItemStack>>>,
}
