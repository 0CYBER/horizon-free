//! `ItemDrop`'s item-entity state crosses the `living_drops` boundary both ways (#3423): the guest
//! reads the full-width values the host sent, and the values it writes come back.
use super::*;

/// Full-width values for every state field.
pub(super) const THROWER: u128 = 0xfedc_ba98_7654_3210_0123_4567_89ab_cdef;
pub(super) const TARGET: u128 = u128::MAX;

/// A drop of one registered item with the given state.
pub(super) fn state_drop(
    thrower: Option<u128>,
    target: Option<u128>,
    floats: bool,
    age: i32,
) -> mod_api::ItemDrop {
    let stack = mod_api::ItemStack::new(REGISTERED_ITEM.get().unwrap(), 1).unwrap();
    mod_api::ItemDrop::new(stack, [1.5, 64.0, -2.5], [0.1, 0.2, -0.1], 10)
        .with_thrower(thrower)
        .with_target(target)
        .with_no_gravity(floats)
        .with_age(age)
}

#[test]
fn wire_loopback_living_drops_carries_the_item_entity_state_both_ways() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    struct World;
    impl mod_api::Host for World {}
    mod_api::impl_host_areas!(World);
    impl mod_api::ServerHost for World {}
    mod_api::impl_server_host_areas!(World);
    install_counter();
    assert_eq!(mod_api::ItemDrop::UNLIMITED_LIFETIME, -32768);
    let source = mod_api::HurtSource {
        damage_type: "minecraft:generic",
        causing: 0,
        direct: 0,
        position: None,
    };
    // The guest checks it gets one default drop then one with every field set, and swaps them.
    let mut drops = vec![
        state_drop(None, None, false, 0),
        state_drop(Some(THROWER), Some(TARGET), true, -32768),
    ];
    WasmMod {
        subscribed: &[],
        listeners: &[],
        group: 0,
        corrupt: false,
    }
    .living_drops(
        &mut World,
        &mut mod_api::LivingDropsEvent {
            entity: 7,
            kind: "test:state_drops",
            player: None,
            source: &source,
            recently_hit: false,
            drops: &mut drops,
            cancelled: false,
        },
    );
    assert_eq!(
        drops,
        vec![
            state_drop(Some(THROWER), Some(TARGET), true, -32768),
            state_drop(None, None, false, 0),
        ]
    );
}

pub(super) struct ItemDropStateHooks;
impl ItemDropStateHooks {
    pub(super) fn living_drops(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::LivingDropsEvent<'_>,
    ) -> mod_api::Flow {
        if event.kind != "test:state_drops" {
            return mod_api::Flow::Continue;
        }
        use self::{TARGET, THROWER, state_drop};
        let full = state_drop(Some(THROWER), Some(TARGET), true, -32768);
        let plain = state_drop(None, None, false, 0);
        assert_eq!(*event.drops, vec![plain.clone(), full.clone()]);
        *event.drops = vec![full, plain];
        mod_api::Flow::Continue
    }
}
