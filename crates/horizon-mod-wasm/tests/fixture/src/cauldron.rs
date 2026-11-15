//! Assertions execute inside wasm against the real Game's generated host imports.
use mod_api::HostBlockStates;
use std::sync::OnceLock;

static WATER_CAULDRON: OnceLock<mod_api::BlockKey> = OnceLock::new();

pub fn register(registrar: &mut dyn mod_api::Registrar) {
    let key = registrar.block_key("minecraft:water_cauldron");
    assert_eq!(*WATER_CAULDRON.get_or_init(|| key), key);
}

pub fn read(host: &dyn mod_api::Host) {
    let block = *WATER_CAULDRON.get().unwrap();
    let water = host.fluid_key("minecraft:water").unwrap();
    let content = host.cauldron_content_for_block(block).unwrap();
    assert_eq!(content.block, block);
    assert_eq!(content.fluid, water);
    assert_eq!(content.total, 1000);
    assert_eq!(content.max_level, 3);
    assert_eq!(content.level_property.as_deref(), Some("level"));
    assert_eq!(host.cauldron_content_for_fluid(water), Some(content));
    let lava = host
        .cauldron_content_for_fluid(host.fluid_key("minecraft:lava").unwrap())
        .unwrap();
    assert_eq!(lava.max_level, 1);
    assert_eq!(lava.level_property, None);
    assert_eq!(
        host.cauldron_content_for_fluid(host.fluid_key("minecraft:empty").unwrap()),
        None
    );
    let state = host.block_states().states_of(lava.block).unwrap().first();
    assert_eq!(host.cauldron_level(state), Some(1));
}
