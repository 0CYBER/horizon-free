use super::*;
thread_local! {
    static EXPECTED: std::cell::Cell<mod_api::Hand> = const { std::cell::Cell::new(mod_api::Hand::Main) };
    static WHO: std::cell::Cell<(u128, i32)> = const { std::cell::Cell::new((0, 0)) };
    static RESULT: std::cell::Cell<mod_api::InteractionResult> = const { std::cell::Cell::new(mod_api::InteractionResult::Pass) };
    static SEEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
pub(super) fn entity_answer(
    event: &mut mod_api::EntityInteractEvent<'_>,
) -> Option<mod_api::InteractionResult> {
    assert_eq!(event.hand, EXPECTED.get());
    assert_eq!(event.entity, 42);
    SEEN.set(true);
    Some(RESULT.get())
}
struct Part;
impl mod_api::ItemPart for Part {
    fn item_subscribed(&self) -> &'static [Option<mod_api::HookId>] {
        &[
            Some(mod_api::HookId::UseItem),
            Some(mod_api::HookId::UseOn),
            Some(mod_api::HookId::ItemInteractMob),
        ]
    }
    fn use_item(
        &self,
        _: mod_api::HostSide<'_>,
        event: &mut mod_api::UseItemEvent<'_>,
    ) -> Option<mod_api::InteractionResult> {
        assert_eq!(event.hand, EXPECTED.get());
        assert_eq!((event.player, event.player_entity_id), WHO.get());
        SEEN.set(true);
        Some(RESULT.get())
    }
    fn use_on(
        &self,
        _: mod_api::HostSide<'_>,
        event: &mut mod_api::UseOnEvent<'_>,
    ) -> Option<mod_api::InteractionResult> {
        assert_eq!(event.hand, EXPECTED.get());
        assert_eq!((event.player, event.player_entity_id), WHO.get());
        SEEN.set(true);
        Some(RESULT.get())
    }
    fn item_interact_mob(
        &self,
        _: mod_api::HostSide<'_>,
        event: &mut mod_api::ItemInteractMobEvent<'_>,
    ) -> Option<mod_api::InteractionResult> {
        assert_eq!(event.hand, EXPECTED.get());
        assert_eq!(event.entity, 42);
        assert_eq!(event.kind, "minecraft:zombie");
        SEEN.set(true);
        Some(RESULT.get())
    }
}
pub(super) fn register(registrar: &mut dyn mod_api::Registrar) {
    let item = registrar.item_key("minecraft:stone");
    registrar.item_part(item, Part);
}
#[test]
fn generated_guest_item_dispatch_preserves_hands_sides_and_results() {
    struct World;
    impl mod_api::Host for World {}
    mod_api::impl_host_areas!(World);
    impl mod_api::ServerHost for World {}
    mod_api::impl_server_host_areas!(World);
    impl mod_api::ClientHost for World {}
    mod_api::impl_client_host_areas!(World);
    install_counter();
    let stack = mod_api::ItemStack::new(REGISTERED_ITEM.get().unwrap(), 1).unwrap();
    for answer in [
        mod_api::InteractionResult::Success,
        mod_api::InteractionResult::SuccessServer,
        mod_api::InteractionResult::Consume,
        mod_api::InteractionResult::Pass,
        mod_api::InteractionResult::Fail,
        mod_api::InteractionResult::TryWithEmptyHand,
    ] {
        RESULT.set(answer);
        for client in [false, true] {
            for hand in [mod_api::Hand::Main, mod_api::Hand::Off] {
                EXPECTED.set(hand);
                // Distinct per side and hand, so a swapped or substituted identity fails.
                let who = match (client, hand) {
                    (false, mod_api::Hand::Main) => (0x1111_2222_3333_4444_5555_6666_7777_8888, 11),
                    (false, mod_api::Hand::Off) => (0x9999_aaaa_bbbb_cccc_dddd_eeee_ffff_0001, 12),
                    (true, mod_api::Hand::Main) => (0x0123_4567_89ab_cdef_0123_4567_89ab_cdef, 13),
                    (true, mod_api::Hand::Off) => (0xfedc_ba98_7654_3210_fedc_ba98_7654_3210, 14),
                };
                WHO.set(who);
                SEEN.set(false);
                let mut result = None;
                let mut world = World;
                mod_api::dispatch_use_item(
                    if client {
                        mod_api::HostSide::Client(&mut world)
                    } else {
                        mod_api::HostSide::Server(&mut world)
                    },
                    mod_api::__private::UseItemCall {
                        stack: &stack,
                        hand,
                        player: who.0,
                        player_entity_id: who.1,
                        result: &mut result,
                    },
                );
                assert_eq!(result, Some(answer));
                SEEN.set(false);
                result = None;
                mod_api::dispatch_use_on(
                    if client {
                        mod_api::HostSide::Client(&mut world)
                    } else {
                        mod_api::HostSide::Server(&mut world)
                    },
                    mod_api::__private::UseOnCall {
                        stack: &stack,
                        pos: [0, 64, 0],
                        face: [0, 1, 0],
                        hand,
                        hit: mod_api::BlockHit::new([0.5, 65.0, 0.5], [0, 1, 0], false),
                        player: who.0,
                        player_entity_id: who.1,
                        result: &mut result,
                    },
                );
                assert!(SEEN.get());
                assert_eq!(result, Some(answer));
                SEEN.set(false);
                result = None;
                mod_api::dispatch_item_interact_mob(
                    if client {
                        mod_api::HostSide::Client(&mut world)
                    } else {
                        mod_api::HostSide::Server(&mut world)
                    },
                    hand,
                    42,
                    "minecraft:zombie",
                    &stack,
                    &mut result,
                );
                assert!(SEEN.get());
                assert_eq!(result, Some(answer));
                SEEN.set(false);
                result = WasmMod {
                    subscribed: &[],
                    listeners: &[],
                    group: 0,
                    corrupt: false,
                }
                .entity_interact(
                    if client {
                        mod_api::HostSide::Client(&mut world)
                    } else {
                        mod_api::HostSide::Server(&mut world)
                    },
                    &mut mod_api::EntityInteractEvent {
                        hand,
                        entity: 42,
                        kind: "minecraft:zombie",
                        stack: Some(&stack),
                    },
                );
                assert!(SEEN.get());
                assert_eq!(result, Some(answer));
            }
        }
    }
}

pub(super) struct UseItemHandHooks;
impl UseItemHandHooks {
    pub(super) fn entity_interact(
        &self,
        _: mod_api::HostSide<'_>,
        event: &mut mod_api::EntityInteractEvent<'_>,
    ) -> Option<mod_api::InteractionResult> {
        self::entity_answer(event)
    }
    pub(super) fn register(
        &self,
        registrar: &mut dyn mod_api::Registrar,
        cx: &mod_api::RegistrationContext,
    ) {
        self::register(registrar);
    }
}
