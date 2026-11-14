//! `menu_slot_rules`' context (`MenuSlotContext`: the player, the menu's block entity and its
//! other slots) through the generated host/guest dispatch against the same part called
//! natively: a rule that reads the context answers the same on both sides.
use super::*;
use mod_api::{ItemStack, MenuContainer, MenuPart, MenuSlotContext, MenuSlotRulesEvent};
struct World;
impl mod_api::Host for World {}
mod_api::impl_host_areas!(World);
/// Takes a stack only into a block's menu of player 7, and only while main slot 0 is empty; caps
/// the stack at the number of filled slots plus one. A grid slot asked about is refused while
/// grid slot 0 holds a stack and the result slot is empty.
struct Part;
#[mod_api::horizon_part]
impl MenuPart for Part {
    fn menu_slot_rules(&self, _: &dyn mod_api::Host, event: &mut MenuSlotRulesEvent<'_>) {
        let context = event.context;
        event.may_place &= context.player == Some(7)
            && context.pos.is_some()
            && context.slot(MenuContainer::Main, 0).is_none();
        if event.container == MenuContainer::Grid {
            event.may_place &= context.slot(MenuContainer::Grid, 0).is_none()
                || context.slot(MenuContainer::Result, 0).is_some();
        }
        let filled = context.slots.iter().flatten().flatten().count() as u32;
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
    let pos = Some([1, 64, -3]);
    let main = |player, pos, slots| {
        (
            MenuContainer::Main,
            MenuSlotContext::new(player, pos, slots),
        )
    };
    let grid = |result| {
        let slots = vec![vec![None], vec![filled.clone(), None], vec![result]];
        (
            MenuContainer::Grid,
            MenuSlotContext::new(Some(7), pos, slots),
        )
    };
    let contexts = [
        main(Some(7), pos, vec![vec![None, filled.clone()]]),
        main(Some(7), pos, vec![vec![filled.clone(); 2]]),
        main(Some(8), pos, vec![vec![None]]),
        main(Some(7), None, Vec::new()),
        (MenuContainer::Main, MenuSlotContext::default()),
        // A mod crafting menu: main slot 0 empty, grid slot 0 filled, the result empty, then
        // the result filled.
        grid(None),
        grid(filled.clone()),
    ];
    // The keyed read: a grid slot and the result slot of the crafting menu, nothing past them.
    let craft = &contexts[6].1;
    assert_eq!(craft.slot(MenuContainer::Grid, 0), Some(&stack));
    assert_eq!(craft.slot(MenuContainer::Grid, 1), None);
    assert_eq!(craft.slot(MenuContainer::Result, 0), Some(&stack));
    assert_eq!(craft.slot(MenuContainer::Player, 0), None);
    let mut answers = Vec::new();
    for (container, context) in &contexts {
        let ask = |part: &dyn MenuPart| {
            let mut event = MenuSlotRulesEvent {
                menu: "test:menu",
                slot: 1,
                container: *container,
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
        [
            (true, 2),
            (false, 3),
            (false, 1),
            (false, 1),
            (false, 1),
            (false, 2),
            (true, 3)
        ]
    );
}
