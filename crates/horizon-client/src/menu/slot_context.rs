//! The context a `menu_slot_rules` ask is given: which of the menu's containers the slot is in,
//! and one copy of the containers' slots for the parts per click plan, built only when a part
//! listens (see [`SlotSnapshot`]).
use super::{
    CONTAINER_MAX, Env, GRID, ItemStack, MAIN, Menu, PLAYER, RESULT, SlotContext, SlotRules,
};
use mod_api::{MenuContainer, MenuSlotContext};
use std::cell::OnceCell;

/// The parts' copy of a menu's slots (`MenuSlotContext`) for one click plan: empty until the
/// first ask a part answers fills it, then shared by every ask of the plan, so a click copies
/// the slots at most once and a menu no part answers never. The menu empties it on every
/// mutation; a LAN click's view holds one for the click. A copy of a menu starts empty.
#[derive(Debug, Default)]
pub struct SlotSnapshot(OnceCell<MenuSlotContext>);

impl SlotSnapshot {
    /// The plan's copy, built by `build` on the first call.
    pub fn get_or_init(&self, build: impl FnOnce() -> MenuSlotContext) -> &MenuSlotContext {
        self.0.get_or_init(build)
    }
    /// Whether the copy was built.
    pub fn is_built(&self) -> bool {
        self.0.get().is_some()
    }
    pub(crate) fn clear(&mut self) {
        self.0.take();
    }
}

impl Clone for SlotSnapshot {
    fn clone(&self) -> Self {
        Self::default()
    }
}

/// The API's key for one of a menu's containers.
fn key(container: usize) -> MenuContainer {
    match container {
        MAIN => MenuContainer::Main,
        GRID => MenuContainer::Grid,
        RESULT => MenuContainer::Result,
        _ => MenuContainer::Player,
    }
}

/// The container a LAN view's slot `i` is in: the block's container's slots are `Main`, the
/// rest the player's.
pub fn lan_container(i: usize, container: Option<&[Option<ItemStack>]>) -> MenuContainer {
    match container {
        Some(c) if i < c.len() => MenuContainer::Main,
        _ => MenuContainer::Player,
    }
}

impl SlotContext<'_> {
    /// What slot `index` of `container` holds, keyed as `MenuSlotContext::slot`.
    pub fn slot(&self, container: MenuContainer, index: usize) -> Option<&ItemStack> {
        let slots = match container {
            MenuContainer::Main => {
                if self.main_crafting.is_some_and(|crafting| {
                    crafting.grid_slots().contains(&index) || index == crafting.result
                }) {
                    return None;
                }
                self.slots
            }
            MenuContainer::Grid => self.grid,
            MenuContainer::Result => self.result,
            _ => &[],
        };
        slots.get(index)?.as_ref()
    }
}

impl Menu {
    /// Who and where this menu's `menu_slot_rules` ask about menu slot `s` is: the env's
    /// player, the menu's block (`set_block_pos`), `s`'s container and the menu's own
    /// containers, with the menu's snapshot cell.
    pub(super) fn slot_context<'s>(&'s self, env: &Env, s: usize) -> SlotContext<'s> {
        let container = self
            .state
            .slots
            .get(s)
            .map_or(PLAYER, |slot| slot.container);
        SlotContext {
            player: env.player,
            pos: self.state.block_pos,
            container: key(container),
            slots: &self.state.containers[MAIN],
            main_crafting: None,
            grid: &self.state.containers[GRID],
            result: &self.state.containers[RESULT],
            snapshot: Some(&self.state.slot_snapshot),
        }
    }
}

/// Apply the scoped rule to a slot's own answers, then enforce its item cap. The rules are asked
/// here, while the click plans, and only for a menu type a part answers; with none lent the
/// slot's answers stand.
#[allow(clippy::too_many_arguments)]
pub fn apply_menu_slot_rules(
    rules: Option<&dyn SlotRules>,
    menu: &str,
    slot: usize,
    stack: &ItemStack,
    context: SlotContext<'_>,
    mut may_place: bool,
    mut max_stack: u32,
    item_max: u32,
) -> (bool, u32) {
    if let Some(rules) = rules.filter(|rules| rules.listens(menu)) {
        rules.rule(menu, slot, stack, context, &mut may_place, &mut max_stack);
    }
    (may_place, max_stack.clamp(1, CONTAINER_MAX).min(item_max))
}
