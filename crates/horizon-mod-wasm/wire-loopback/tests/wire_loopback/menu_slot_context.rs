//! `menu_slot_rules`' context (`MenuSlotContext`: the player, the menu's block entity and its
//! other slots) through the generated host/guest dispatch against the same part called
//! natively: a rule that reads the context answers the same on both sides.
use super::*;
use mod_api::{ItemStack, MenuPart, MenuSlotContext, MenuSlotRulesEvent};
struct World;
impl mod_api::Host for World {}
mod_api::impl_host_areas!(World);
/// Takes a stack only into a block's menu of player 7, and only while slot 0 is empty; caps the
/// stack at the number of filled slots plus one.
struct Part;
#[mod_api::horizon_part]
impl MenuPart for Part {
    fn menu_slot_rules(&self, _: &dyn mod_api::Host, event: &mut MenuSlotRulesEvent<'_>) {
        let context = event.context;
        event.may_place &= context.player == Some(7)
            && context.pos.is_some()
            && context.slots.first().is_none_or(Option::is_none);
        let filled = context.slots.iter().flatten().count() as u32;
        event.max_stack = event.max_stack.min(filled + 1);
    }
}
#[test]
fn menu_slot_context_reads_the_same_through_the_wire() {
    install_counter();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let proxy = WasmHandler {
        index: guest::add_handler(vec![guest::view::<dyn MenuPart>(Arc::new(Part))]),
        module: WasmMod {
            group: 0,
            subscribed: &[],
            listeners: &[],
            corrupt: false,
        },
    };
    let stack = ItemStack::new(super::prepared_equip::helmet(), 1).unwrap();
    let filled = Some(stack.clone());
    let contexts = [
        MenuSlotContext::new(Some(7), Some([1, 64, -3]), vec![None, filled.clone()]),
        MenuSlotContext::new(Some(7), Some([1, 64, -3]), vec![filled.clone(), filled.clone()]),
        MenuSlotContext::new(Some(8), Some([1, 64, -3]), vec![None]),
        MenuSlotContext::new(Some(7), None, Vec::new()),
        MenuSlotContext::default(),
    ];
    let mut answers = Vec::new();
    for context in &contexts {
        let ask = |part: &dyn MenuPart| {
            let mut event = MenuSlotRulesEvent {
                menu: "test:menu",
                slot: 1,
                stack: &stack,
                context,
                may_place: true,
                max_stack: 64,
            };
            part.menu_slot_rules(&World, &mut event);
            (event.may_place, event.max_stack)
        };
        let (wasm, direct) = (ask(&proxy), ask(&Part));
        assert_eq!(wasm, direct, "{context:?}");
        answers.push(direct);
    }
    assert_eq!(
        answers,
        [(true, 2), (false, 3), (false, 1), (false, 1), (false, 1)]
    );
}
