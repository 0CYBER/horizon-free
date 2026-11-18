//! The per-tick menu slot sync of #15791: a slot that did not change costs no
//! packet, and a change inside a stack (a shulker's contents) still reaches
//! the client, deep comparison and all.
use super::*;
use crate::lan::menu::{Menu, Slots};
use horizon_protocol::buf::Codec;
use horizon_protocol::item::{Component, Template, component};

/// A shulker box holding 27 stacks of its own: copying it copies all of them,
/// which is what the sync used to do for every slot of every guest every tick.
fn shulker(fill: u32) -> ItemStack {
    let mut stack = ItemStack::new("minecraft:shulker_box", 1);
    stack.data_mut().container = (0..27)
        .map(|k| (k, ItemStack::new("minecraft:stone", fill + k as u32)))
        .collect();
    stack
}

/// The slots of a guest's own ender chest need no block behind them, so the
/// menu opens with nothing but a window and a size.
fn ender_menu(window: i32) -> Menu {
    Menu {
        window,
        kind: 0,
        pos: None,
        slots: Slots::Ender,
        size: 27,
        state: 0,
        sent: Vec::new(),
        drag: None,
        sent_data: Vec::new(),
        hidden: Vec::new(),
        mod_type: None,
        registry_id: "minecraft:generic_9x3".into(),
        craft: None,
        links: Vec::new(),
        asks_still_valid: false,
    }
}

/// What a shulker's `minecraft:container` component holds, as the packet
/// carries it: each filled slot and the count it was sent with. `None` when
/// the stack goes out without its contents.
fn shulker_contents(item: &horizon_protocol::item::Stack) -> Option<Vec<(usize, i32)>> {
    let (_, contents) = item
        .added
        .iter()
        .find(|(kind, _)| *kind == component::CONTAINER)?;
    let Component::Raw(bytes) = contents else {
        return None;
    };
    let mut reader = horizon_protocol::buf::Reader::new(bytes);
    let mut slots = Vec::new();
    for slot in 0..reader.count(256).ok()? {
        if reader.bool().ok()? {
            let stack: horizon_protocol::item::Stack = Template::read(&mut reader).ok()?;
            slots.push((slot, stack.count));
        }
    }
    Some(slots)
}

/// The `ContainerSetSlot` packets the guest was sent since the last drain.
fn slots_sent(rx: &std::sync::mpsc::Receiver<crate::lan::Out>) -> Vec<play::ContainerSetSlot> {
    let mut bodies = Vec::new();
    while let Ok(out) = rx.try_recv() {
        match out {
            crate::lan::Out::Packet(body) => bodies.push(body),
            crate::lan::Out::Packets(batch) => bodies.extend(batch),
            crate::lan::Out::Disconnect(_) => {}
        }
    }
    let mut sent = Vec::new();
    for body in bodies {
        let mut reader = horizon_protocol::buf::Reader::new(&body);
        let id = reader.varint().unwrap();
        if let C::ContainerSetSlot(packet) = C::decode(id, reader.rest()).unwrap() {
            sent.push(packet);
        }
    }
    sent
}

/// The inventory menu (`ContainerSetSlot` window 0): the client is told the
/// slots it was never sent, nothing more when the tick changed nothing, and
/// only the slot whose stack changed underneath, its contents included.
#[test]
fn the_inventory_menu_syncs_the_slots_that_changed_and_no_others() {
    let (mut hub, rx) = hub_with_a_guest();
    hub.guests.update(0, |g| {
        // Menu slot 0 (crafting result) and 20 (main inventory, index 20).
        g.craft_result = Some(shulker(0));
        g.inventory[20] = Some(shulker(100));
    });
    // A fresh guest: no slot is in `menu_sent`, so the ones holding a stack go
    // out on the first sync, and the empty ones stay unsent.
    hub.sync_player_state(0);
    let first = slots_sent(&rx);
    let first_slots: Vec<i16> = first.iter().map(|p| p.slot).collect();
    assert_eq!(first_slots, [0, 20], "the slots never sent, in order");
    assert!(first.iter().all(|p| p.container == 0), "the inventory menu");
    assert!(
        first
            .iter()
            .all(|p| p.item.as_ref().is_some_and(|item| item.count == 1)),
        "each slot goes out whole"
    );
    let state = hub.guests[0].menu_state;
    // The steady tick: the same stacks, so the client hears nothing.
    hub.sync_player_state(0);
    assert!(
        slots_sent(&rx).is_empty(),
        "an unchanged tick sends no slot"
    );
    assert_eq!(hub.guests[0].menu_state, state, "and no state is spent");
    // Only what the shulker holds changes: the item, its count and its damage
    // stay as they were, so only a deep comparison can see the change.
    hub.guests.update(0, |g| {
        let stack = g.inventory[20].as_mut().expect("the shulker");
        stack.data_mut().container[3].1.count = 7;
    });
    hub.sync_player_state(0);
    let changed = slots_sent(&rx);
    assert_eq!(changed.len(), 1, "one changed slot, one packet");
    assert_eq!((changed[0].container, changed[0].slot), (0, 20));
    assert_eq!(changed[0].state, (state + 1) & 0x7FFF);
    let item = changed[0]
        .item
        .as_ref()
        .expect("the whole stack, its contents with it");
    assert_eq!(item.count, 1);
    let expected: Vec<(usize, i32)> = (0..27)
        .map(|slot| (slot, if slot == 3 { 7 } else { 100 + slot as i32 }))
        .collect();
    assert_eq!(
        shulker_contents(item).expect("the shulker's contents travel in the packet"),
        expected,
        "the contents the shulker holds now, the changed one among them"
    );
    hub.sync_player_state(0);
    assert!(slots_sent(&rx).is_empty(), "and only once");
}

/// An open container (`Slots::Ender`, window 3): `sync_containers` tells the
/// client every slot once, nothing while the menu is unchanged, and the one
/// slot that changed, with the menu's own state. A slot the menu has no record
/// of is a slot never sent, even when it holds nothing.
#[test]
fn an_open_container_syncs_the_slots_that_changed_and_no_others() {
    let (mut hub, rx) = hub_with_a_guest();
    let world = World::new(horizon_world::registry::BlockStateId(0), 0);
    hub.guests.update(0, |g| {
        g.ender_items[2] = Some(shulker(0));
        g.ender_items[5] = Some(ItemStack::new("minecraft:dirt", 3));
        g.menu = Some(ender_menu(3));
    });
    // No slot was sent yet: all 27 container slots and the player's 36 go out,
    // the empty ones as empty ones.
    hub.sync_containers(&world);
    let first = slots_sent(&rx);
    assert_eq!(first.len(), 27 + 36, "the client is told each slot once");
    assert_eq!(
        first.iter().map(|p| p.slot).collect::<Vec<_>>(),
        (0..27 + 36).collect::<Vec<i16>>(),
        "the menu's own slots, then the player's, in order"
    );
    assert!(first.iter().all(|p| p.container == 3), "the menu's window");
    assert_eq!(
        first
            .iter()
            .filter(|p| p.item.is_some())
            .map(|p| p.slot)
            .collect::<Vec<_>>(),
        [2, 5]
    );
    // A steady tick sends nothing: the copies of the slots match the menu.
    hub.sync_containers(&world);
    assert!(
        slots_sent(&rx).is_empty(),
        "an unchanged tick sends no slot"
    );
    // A change deep inside one container slot: one packet with that slot.
    let before = hub.guests[0].menu.as_ref().expect("the menu").state;
    hub.guests.update(0, |g| {
        let stack = g.ender_items[2].as_mut().expect("the shulker");
        stack.data_mut().container[0].1.count = 9;
    });
    hub.sync_containers(&world);
    let changed = slots_sent(&rx);
    assert_eq!(changed.len(), 1, "one changed slot, one packet");
    assert_eq!((changed[0].container, changed[0].slot), (3, 2));
    assert_eq!(
        changed[0].state,
        (before + 1) & 0x7FFF,
        "the menu's own state, one past the last sync"
    );
    let item = changed[0]
        .item
        .as_ref()
        .expect("the shulker goes out whole, its contents with it");
    let expected: Vec<(usize, i32)> = (0..27)
        .map(|slot| (slot, if slot == 0 { 9 } else { slot as i32 }))
        .collect();
    assert_eq!(
        shulker_contents(item).expect("the shulker's contents travel in the packet"),
        expected,
        "the contents it holds now, the changed one among them"
    );
    hub.sync_containers(&world);
    assert!(slots_sent(&rx).is_empty(), "and only once");
    // A slot with no record in `sent` is a slot never told, empty or not: the
    // client would keep its own contents there, so it goes out again.
    hub.guests.update(0, |g| {
        if let Some(m) = g.menu.as_mut() {
            m.sent.truncate(27);
        }
    });
    hub.sync_containers(&world);
    let resent = slots_sent(&rx);
    assert_eq!(resent.len(), 36, "the 36 player slots have no record");
    assert!(resent.iter().all(|p| p.item.is_none()), "and hold nothing");
}
