//! `menu_slot_rules` on the game's side. A menu click cannot reach the game, so the game lends
//! itself to the click as its [`SlotRules`] (`Env::slot_rules`), with the read-only `Host`
//! behind it: the click asks while it plans, once for each slot and stack it consults, and the
//! answers stay in what it planned. A menu type no part answers is never asked, and nothing is
//! converted for it.
use super::*;
use crate::game::api_values::TryIntoApi;
use horizon_client::menu::{SlotContext, SlotRules};

impl SlotRules for Game<'_> {
    fn listens(&self, menu: &str) -> bool {
        mod_api::__private::has_part_hook(mod_api::HookId::MenuSlotRules)
            && mod_api::__private::menu_listens(mod_api::HookId::MenuSlotRules, menu)
    }

    fn rule(
        &self,
        menu: &str,
        slot: usize,
        stack: &ItemStack,
        context: SlotContext<'_>,
        may_place: &mut bool,
        max_stack: &mut u32,
    ) {
        // A stack that cannot cross to the API is not asked about: the slot's answers stand.
        let Ok(stack) = stack.try_into_api() else {
            return;
        };
        // The menu's other slots, read only; one that cannot cross shows empty.
        let slots: Vec<Option<mod_api::ItemStack>> = context
            .slots
            .iter()
            .map(|slot| slot.as_ref().and_then(|stack| stack.try_into_api().ok()))
            .collect();
        // A listener that panics is a failed ask, not a failed click: the slot's own answers
        // stand (the dispatch writes nothing back from a failed pass), the failure is logged by
        // name, and the click goes on. The next ask, and the next click, are asked as ever.
        let asked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let (mut place, mut maximum) = (*may_place, *max_stack);
            self.native_menu_slot_rules(mod_api::__private::MenuSlotRulesCall {
                menu,
                slot,
                stack: &stack,
                context: &mod_api::MenuSlotContext::new(context.player, context.pos, slots),
                may_place: &mut place,
                max_stack: &mut maximum,
            });
            (place, maximum)
        }));
        match asked {
            Ok((place, maximum)) => (*may_place, *max_stack) = (place, maximum),
            Err(_) => log::error!(
                "menu_slot_rules: a listener failed for {menu} slot {slot}; the slot's own answers stand"
            ),
        }
    }
}
