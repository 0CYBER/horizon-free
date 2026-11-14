use super::*;

impl Game<'_> {
    /// Slot placement and stack limits for registered vanilla and mod menus: `menu` is the
    /// menu type, `slot` the backing container index, `stack` the stack being asked about.
    /// `may_place` and `max_stack` arrive holding the slot's own answers, which the menu type's
    /// parts may change; the click bounds the final maximum. A read-only fire: a click asks once
    /// for each slot and stack it consults while it plans, and never again to validate or apply.
    /// `context` is who and where the ask is about (`MenuSlotContext`): the player, the menu's
    /// block entity and its own container slots, read only; None where the menu has none.
    #[inline(never)]
    pub(in crate::game) fn native_menu_slot_rules(
        &self,
        call: mod_api::__private::MenuSlotRulesCall<'_>,
    ) {
        self.fire_menu_slot_rules(call);
    }
}
