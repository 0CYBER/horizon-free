//! Opt-in compiled-guest registration ordering and published capability metadata.
use mod_api::{
    CapabilityKey, ComponentId, EnergyHandler, EntityCapabilityKey, HostTransaction,
    ItemCapabilityKey, ItemComponentEnergy, ItemPlace, Registrar, RegistrationContext, ServerHost,
};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

pub const PROBE: &str = "capability_registration_probe";
pub const BLOCK: CapabilityKey<dyn EnergyHandler> =
    CapabilityKey::new("wasm_fixture:registration_energy");
pub const ITEM: ItemCapabilityKey<dyn EnergyHandler> =
    ItemCapabilityKey::new("wasm_fixture:registration_item_energy");
pub const ENTITY: EntityCapabilityKey<dyn EnergyHandler, Option<mod_api::Direction>> =
    EntityCapabilityKey::new("wasm_fixture:registration_entity_energy");
pub const PROXY: CapabilityKey<dyn EnergyHandler> = CapabilityKey::new("wasm_fixture:proxy_energy");
pub const NON_PROXY: CapabilityKey<dyn EnergyHandler> =
    CapabilityKey::new("wasm_fixture:non_proxy_energy");
pub const BLOCKS: [&str; 4] = [
    "minecraft:stone",
    "minecraft:dirt",
    "minecraft:sand",
    "minecraft:gravel",
];
pub const ITEMS: [&str; 4] = [
    "minecraft:stick",
    "minecraft:snowball",
    "minecraft:egg",
    "minecraft:flint",
];
pub const ENTITIES: [&str; 4] = [
    "minecraft:pig",
    "minecraft:cow",
    "minecraft:sheep",
    "minecraft:chicken",
];
pub const PLACE: ItemPlace = ItemPlace::PlayerSlot {
    player: 19,
    slot: 0,
};
static ACTIVE: AtomicBool = AtomicBool::new(false);
static CONTAINER_CALLS: AtomicU32 = AtomicU32::new(0);

pub fn battery(place: ItemPlace, capacity: u32) -> Box<dyn EnergyHandler> {
    Box::new(ItemComponentEnergy::new(
        place,
        ComponentId::new("wasm_fixture:stored_energy").unwrap(),
        capacity,
        37,
        1000,
    ))
}

pub fn covered(r: &mut dyn Registrar, index: usize, expected: bool) {
    let block = r.block_key(BLOCKS[index]);
    let item = r.item_key(ITEMS[index]);
    let entity = r.entity_type_key(ENTITIES[index]);
    assert_eq!(r.is_block_capability_registered(&BLOCK, &block), expected);
    assert_eq!(r.is_item_capability_registered(&ITEM, &item), expected);
    assert_eq!(
        r.is_entity_capability_registered(&ENTITY, &entity),
        expected
    );
}

pub fn provide(r: &mut dyn Registrar, index: usize) {
    let block = r.block_key(BLOCKS[index]);
    let capacity = (index as u32 + 1) * 100;
    r.block_capability(&BLOCK, &block, move |_, _, _, _| {
        Some(battery(PLACE, capacity))
    });
    r.item_capability(&ITEM, ITEMS[index], move |_, place| {
        Some(battery(place.clone(), capacity))
    });
    r.entity_capability(&ENTITY, ENTITIES[index], move |_, _, _| {
        Some(battery(PLACE, capacity))
    });
}

pub fn register(r: &mut dyn Registrar, cx: &RegistrationContext) {
    if cx.mods().get("capability_game_probe").is_some() {
        let chest = r.block_key("minecraft:chest");
        assert!(r.is_block_capability_registered(&mod_api::ITEMS, &chest));
        assert!(r.is_block_capability_registered(&mod_api::STORAGE, &chest));
        let cauldron = r.block_key("minecraft:water_cauldron");
        assert!(r.is_block_capability_registered(&mod_api::FLUIDS, &cauldron));
        r.set_capability_proxyable(
            &CapabilityKey::<dyn EnergyHandler>::new("fixture:game_coverage_completed"),
            true,
        );
    }
    if cx.mods().get(PROBE).is_none() {
        return;
    }
    ACTIVE.store(true, Ordering::Relaxed);
    covered(r, 0, true);
    for index in 1..4 {
        covered(r, index, false);
    }
    // Each kind independently installs a fallback only for the uncovered current type.
    for index in 0..2 {
        let capacity = (index as u32 + 1) * 100;
        let block = r.block_key(BLOCKS[index]);
        if !r.is_block_capability_registered(&BLOCK, &block) {
            r.block_capability(&BLOCK, &block, move |_, _, _, _| {
                Some(battery(PLACE, capacity))
            });
        }
        let item = r.item_key(ITEMS[index]);
        if !r.is_item_capability_registered(&ITEM, &item) {
            r.item_capability(&ITEM, ITEMS[index], move |_, place| {
                Some(battery(place.clone(), capacity))
            });
        }
        let entity = r.entity_type_key(ENTITIES[index]);
        if !r.is_entity_capability_registered(&ENTITY, &entity) {
            r.entity_capability(&ENTITY, ENTITIES[index], move |_, _, _| {
                Some(battery(PLACE, capacity))
            });
        }
    }
    covered(r, 1, true);
    covered(r, 2, false);
    covered(r, 3, false);
    let container = r.item_key(ITEMS[1]);
    assert!(!r.is_item_capability_registered(&mod_api::ITEM_ITEMS, &container));
    r.item_container(&container, |host: &dyn mod_api::Host| {
        CONTAINER_CALLS.fetch_add(1, Ordering::Relaxed);
        host.is_loaded("capability_dynamic_slots").then_some(3)
    });
    for _ in 0..2 {
        assert!(r.is_item_capability_registered(&mod_api::ITEM_ITEMS, &container));
    }
    // Declaration coverage must not execute the dynamic callback, even to test for None.
    assert_eq!(CONTAINER_CALLS.load(Ordering::Relaxed), 0);
    assert!(!r.block_capability_proxyable(PROXY.name));
    r.set_capability_proxyable(&PROXY, true);
    r.set_capability_proxyable(&PROXY, true);
    assert!(r.block_capability_proxyable(PROXY.name));
    r.set_capability_proxyable(&NON_PROXY, false);
    r.set_capability_proxyable(&NON_PROXY, false);
    if cx.mods().get("capability_proxy_conflict_false").is_some() {
        r.set_capability_proxyable(&PROXY, false);
    }
    if cx.mods().get("capability_proxy_conflict_true").is_some() {
        r.set_capability_proxyable(&NON_PROXY, true);
    }
}

fn exercise(host: &mut dyn ServerHost, handler: &dyn EnergyHandler, capacity: u32) {
    assert_eq!(handler.capacity(host), capacity);
    assert_eq!(handler.amount(host), 0);
    assert_eq!(
        host.transaction(|host, tx| Ok::<_, ()>(handler.insert(host, tx, 53))),
        Ok(37)
    );
    assert_eq!(handler.amount(host), 37);
    assert_eq!(
        host.transaction(|host, tx| Err::<(), _>(handler.extract(host, tx, 14))),
        Err(14)
    );
    assert_eq!(handler.amount(host), 37);
    assert_eq!(
        host.transaction(|host, tx| Ok::<_, ()>(handler.extract(host, tx, 1000))),
        Ok(37)
    );
    assert_eq!(handler.amount(host), 0);
}

pub fn started(host: &mut dyn ServerHost) {
    if !ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    assert_eq!(CONTAINER_CALLS.load(Ordering::Relaxed), 2);
    let names = mod_api::block_capability_names(host);
    let proxyable = mod_api::block_capability_proxyable_names(host);
    for name in [BLOCK.name, PROXY.name, NON_PROXY.name] {
        assert_eq!(
            names.iter().filter(|entry| entry.as_str() == name).count(),
            1
        );
    }
    assert!(!mod_api::block_capability_proxyable(host, BLOCK.name));
    assert!(mod_api::block_capability_proxyable(host, PROXY.name));
    assert!(!mod_api::block_capability_proxyable(host, NON_PROXY.name));
    assert!(!mod_api::block_capability_proxyable(
        host,
        "wasm_fixture:unknown"
    ));
    let filtered: Vec<_> = names
        .iter()
        .filter(|name| mod_api::block_capability_proxyable(host, name))
        .cloned()
        .collect();
    assert_eq!(proxyable, filtered);
    assert_eq!(names, mod_api::block_capability_names(host));
    let level = host.level("minecraft:overworld").unwrap();
    for index in 0..3 {
        let capacity = (index as u32 + 1) * 100;
        let block = level
            .capability(host, &BLOCK, [index, 64, 0], None)
            .unwrap();
        exercise(host, &*block, capacity);
        let entity = host.entity(index as u32 + 1).unwrap();
        let entity = entity.capability(host, &ENTITY, None).unwrap();
        exercise(host, &*entity, capacity);
        let item = ItemPlace::PlayerSlot {
            player: 19,
            slot: index,
        };
        let item = item.capability(host, &ITEM).unwrap();
        exercise(host, &*item, capacity);
    }
    assert!(level.capability(host, &BLOCK, [3, 64, 0], None).is_none());
    let absent = host.entity(4).unwrap();
    assert!(absent.capability(host, &ENTITY, None).is_none());
    assert!(
        ItemPlace::PlayerSlot {
            player: 19,
            slot: 3
        }
        .capability(host, &ITEM)
        .is_none()
    );
    // Host-side assertion witnesses that the guest finished, not merely that it loaded.
    let item = PLACE.capability(host, &ITEM).unwrap();
    assert_eq!(
        host.transaction(|host, tx| Ok::<_, ()>(item.insert(host, tx, 23))),
        Ok(23)
    );
}
