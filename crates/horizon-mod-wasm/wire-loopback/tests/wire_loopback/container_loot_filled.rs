use super::*;

// The shared guest fixture forwards this hook here, alongside its test.
pub(super) fn listen(event: &mut mod_api::ContainerLootFilledEvent<'_>) {
    assert_eq!(event.context.table, "test:first_access");
    assert_eq!(event.context.dimension, "minecraft:overworld");
    assert_eq!(event.context.origin, [8.5, 64.5, 8.5]);
    assert_eq!(event.context.block, Some([8, 64, 8]));
    assert_eq!(event.context.entity, None);
    assert_eq!(event.context.opener, Some(7));
    assert_eq!(event.drops.len(), 1);
    event.drops.clear();
    event
        .drops
        .push(mod_api::ItemStack::new(REGISTERED_ITEM.get().unwrap(), 3).unwrap());
}

#[test]
fn wire_loopback_container_loot_filled() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    struct World;
    impl mod_api::Host for World {}
    mod_api::impl_host_areas!(World);
    impl mod_api::LevelHost for World {}
    install_counter();
    let key = REGISTERED_ITEM.get().unwrap();
    let mut drops = vec![mod_api::ItemStack::new(key, 1).unwrap()];
    let context =
        mod_api::ContainerLootFilled::from(mod_api::__private::built::ContainerLootFilled {
            table: "test:first_access",
            dimension: "minecraft:overworld",
            origin: [8.5, 64.5, 8.5],
            block: Some([8, 64, 8]),
            entity: None,
            opener: Some(7),
        });
    let mut event = mod_api::ContainerLootFilledEvent {
        context: &context,
        drops: &mut drops,
    };
    WasmMod {
        subscribed: &[],
        listeners: &[],
        group: 0,
        corrupt: false,
    }
    .container_loot_filled(&World, &mut event);
    assert_eq!(drops, vec![mod_api::ItemStack::new(key, 3).unwrap()]);
}

pub(super) struct ContainerLootFilledHooks;
impl ContainerLootFilledHooks {
    pub(super) fn container_loot_filled(
        &self,
        _: &dyn mod_api::LevelHost,
        event: &mut mod_api::ContainerLootFilledEvent<'_>,
    ) {
        self::listen(event);
    }
}
