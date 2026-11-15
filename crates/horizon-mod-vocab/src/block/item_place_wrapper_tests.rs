use super::*;
use crate::prelude::ItemsHost;
use crate::testing::FakeHost;
use crate::{HostTransaction, Identifier, Tag};

const STEVE: u128 = 7;

/// A host with Steve online and `ids` registered, each stacking to its count, and their
/// resources.
fn host_with(items: &[(&str, i32)]) -> (FakeHost, Vec<ItemResource>) {
    let mut host = FakeHost::new();
    host.join(STEVE, 1);
    let ids: Vec<_> = items
        .iter()
        .map(|(id, _)| Identifier::parse(id).unwrap())
        .collect();
    host.registries.resolve_items(ids.clone()).unwrap();
    for (id, stack) in items {
        host.registries
            .item_component(id, "minecraft:max_stack_size", Tag::Int(*stack));
    }
    let resources = ids
        .iter()
        .map(|id| ItemResource::of(&ItemStack::new(host.item_key(id).unwrap(), 1).unwrap()))
        .collect();
    (host, resources)
}

fn infinite_hand() -> ItemPlace {
    ItemPlace::InfiniteMaterials {
        player: STEVE,
        inner: Box::new(ItemPlace::PlayerHand {
            player: STEVE,
            hand: Hand::Main,
        }),
    }
}

/// `place.insert` of `amount` in a transaction that commits.
fn insert(host: &mut FakeHost, place: &ItemPlace, resource: &ItemResource, amount: u32) -> u32 {
    let host: &mut dyn ServerHost = host;
    host.transaction(|host, tx| Ok::<_, ()>(place.insert(host, tx, resource, amount)))
        .unwrap()
}

fn extract(host: &mut FakeHost, place: &ItemPlace, resource: &ItemResource, amount: u32) -> u32 {
    let host: &mut dyn ServerHost = host;
    host.transaction(|host, tx| Ok::<_, ()>(place.extract(host, tx, resource, amount)))
        .unwrap()
}

/// What Steve holds, by slot and count.
fn slots(host: &FakeHost) -> Vec<(i32, u32)> {
    let steve = host.players.get(STEVE).unwrap();
    steve
        .slots
        .iter()
        .map(|(&slot, s)| (slot, s.count()))
        .collect()
}

/// What the game gave Steve beyond his slots, as `Player.drop` would when the inventory is full.
fn given(host: &FakeHost) -> Vec<u32> {
    let steve = host.players.get(STEVE).unwrap();
    steve.given.iter().map(ItemStack::count).collect()
}

#[test]
fn infinite_materials_drops_the_one_when_the_inventory_is_full() {
    let (mut host, r) = host_with(&[("minecraft:water_bucket", 1), ("minecraft:dirt", 64)]);
    let (bucket, dirt) = (&r[0], &r[1]);
    let steve = host.players.get_mut(STEVE).unwrap();
    steve.infinite_materials = true;
    for slot in 0..36 {
        steve.slots.insert(slot, dirt.to_stack(64).unwrap());
    }
    // Taken whole; the one he has none of goes to the game's give-or-drop, not lost.
    assert_eq!(insert(&mut host, &infinite_hand(), bucket, 3), 3);
    assert_eq!(given(&host), [1]);
    assert_eq!(slots(&host).len(), 36);
    // Now he has one: nothing more is given.
    host.players
        .get_mut(STEVE)
        .unwrap()
        .slots
        .insert(5, bucket.to_stack(1).unwrap());
    assert_eq!(insert(&mut host, &infinite_hand(), bucket, 1), 1);
    assert_eq!(given(&host), [1]);
}

#[test]
fn infinite_materials_puts_the_one_in_an_empty_slot_and_gives_without_taking() {
    let (mut host, r) = host_with(&[("minecraft:water_bucket", 1), ("minecraft:dirt", 64)]);
    let (bucket, dirt) = (&r[0], &r[1]);
    let steve = host.players.get_mut(STEVE).unwrap();
    steve.infinite_materials = true;
    steve.slots.insert(0, dirt.to_stack(10).unwrap());
    assert_eq!(insert(&mut host, &infinite_hand(), bucket, 1), 1);
    // Slot 0 holds dirt, so the bucket goes where the inventory has room; the fake records it
    // as given, as `Inventory.add` places it.
    assert_eq!(given(&host), [1]);
    // An extract gives what the hand holds without taking it.
    assert_eq!(extract(&mut host, &infinite_hand(), dirt, 64), 10);
    assert_eq!(slots(&host), [(0, 10)]);
}

#[test]
fn inserting_nothing_into_infinite_materials_mints_nothing() {
    let (mut host, r) = host_with(&[("minecraft:water_bucket", 1)]);
    let bucket = &r[0];
    host.players.get_mut(STEVE).unwrap().infinite_materials = true;
    assert_eq!(insert(&mut host, &infinite_hand(), bucket, 0), 0);
    assert!(slots(&host).is_empty());
    assert!(given(&host).is_empty());
}

#[test]
fn without_infinite_materials_it_is_the_place_inside() {
    let (mut host, r) = host_with(&[("minecraft:dirt", 64)]);
    let dirt = &r[0];
    host.players
        .get_mut(STEVE)
        .unwrap()
        .slots
        .insert(0, dirt.to_stack(10).unwrap());
    assert_eq!(extract(&mut host, &infinite_hand(), dirt, 4), 4);
    assert_eq!(slots(&host), [(0, 6)]);
}

#[test]
fn wrappers_nest_far_deeper_than_any_stack_would_recurse() {
    let (mut host, r) = host_with(&[("minecraft:dirt", 64)]);
    let dirt = &r[0];
    host.players
        .get_mut(STEVE)
        .unwrap()
        .slots
        .insert(3, dirt.to_stack(5).unwrap());
    let mut deep = ItemPlace::PlayerSlot {
        player: STEVE,
        slot: 3,
    };
    for depth in 0..100_000 {
        deep = match depth % 3 {
            0 => ItemPlace::Strict(Box::new(deep)),
            1 => ItemPlace::OneByOne(Box::new(deep)),
            _ => ItemPlace::InfiniteMaterials {
                player: STEVE,
                inner: Box::new(deep),
            },
        };
    }
    let server: &mut dyn ServerHost = &mut host;
    assert_eq!(deep.amount(server), 1);
    assert_eq!(extract(&mut host, &deep, dirt, 5), 1);
    assert_eq!(slots(&host), [(3, 4)]);
    let cloned = deep.clone();
    assert_eq!(cloned.amount(&mut host), 1);
    drop(cloned);
    drop(deep);
}

#[test]
fn the_wasm_boundary_refuses_wrappers_nested_past_its_depth() {
    // `Strict` `wrappers` deep around a player slot, as the wire writes it.
    let wrapped = |wrappers: usize| {
        let mut bytes = Vec::new();
        for _ in 0..wrappers {
            bytes.extend(11u32.to_le_bytes());
        }
        bytes.extend(0u32.to_le_bytes());
        bytes.extend(STEVE.to_le_bytes());
        bytes.extend(3i32.to_le_bytes());
        crate::wire::Reader::decode::<ItemPlace>(&bytes)
    };
    // Each wrapper takes two of the wire's 32 levels, its box and the place inside.
    assert!(wrapped(15).is_ok());
    assert_eq!(wrapped(16), Err(crate::wire::WireError::Depth));
    assert_eq!(wrapped(100_000), Err(crate::wire::WireError::Depth));
}
