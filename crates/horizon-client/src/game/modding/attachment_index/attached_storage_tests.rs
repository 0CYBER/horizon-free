//! Stock attached storage (`AttachedItems`, `AttachedEnergy`) through real holder writes and
//! the transaction journal.
use super::*;
use crate::game::api_values::TryIntoApi;
use crate::game::native_entity_refs::tests::with_game;
use horizon_client::{chunk::Chunk, items::ItemStack};
use mod_api::{
    AttachedEnergy, AttachedItems, AttachmentType, EnergyHandler, HostTransaction, ITEM_STACKS,
    ItemResource, ResourceHandler, ServerHost, StackList,
};

static ITEMS: AttachmentType<StackList<ItemResource>> = AttachmentType {
    namespace: "storage_test",
    name: "items",
    default: Vec::new,
    codec: Some(ITEM_STACKS),
    copy_on_death: false,
    sync: false,
};
static ENERGY: AttachmentType<u32> = AttachmentType {
    namespace: "storage_test",
    name: "energy",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync: false,
};
const CHUNK: Holder = Holder::Chunk([2, 3]);

fn chunk_game(game: &mut Game<'_>) {
    game.options.dedicated = Some(Vec::new());
    game.native_level_opened(Dim::Overworld);
    game.world
        .chunks
        .insert((2, 3), Arc::new(Chunk::empty(2, 3)));
}

#[test]
#[ignore = "daily"]
fn attached_items_are_undone_by_an_abort_and_saved_by_their_codec() {
    with_game(|game| {
        chunk_game(game);
        let dirt = ItemResource::of(&ItemStack::new("minecraft:dirt", 1).try_into_api().unwrap());
        let slots = AttachedItems::new(&ITEMS, CHUNK, 9, 64, |_| true).unwrap();
        let host: &mut dyn ServerHost = game;
        let aborted =
            host.transaction(|host, tx| Err::<(), _>(slots.insert(host, tx, 4, &dirt, 10)));
        assert_eq!(aborted, Err(10));
        assert_eq!(ITEMS.has(host, CHUNK), Ok(false));
        let went = host.transaction(|host, tx| Ok::<_, ()>(slots.insert(host, tx, 4, &dirt, 10)));
        assert_eq!(went, Ok(10));
        assert_eq!(slots.amount(host, 4), 10);
        // What the save writes reads back to the same index.
        let held = ITEMS.get_existing(host, CHUNK).unwrap().unwrap();
        let codec = ITEMS.codec.as_ref().unwrap();
        let reloaded = (codec.decode)(&(codec.encode)(&held).unwrap()).unwrap();
        assert_eq!(reloaded[4], Some((dirt, 10)));
        assert!(reloaded[..4].iter().all(Option::is_none));
    });
}

#[test]
#[ignore = "daily"]
fn attached_energy_refuses_past_its_capacity() {
    with_game(|game| {
        chunk_game(game);
        let battery = AttachedEnergy::new(&ENERGY, CHUNK, 100, 1000, 1000);
        let host: &mut dyn ServerHost = game;
        let insert = |host: &mut dyn ServerHost, amount| {
            host.transaction(|host, tx| Ok::<_, ()>(battery.insert(host, tx, amount)))
        };
        assert_eq!(insert(&mut *host, 150), Ok(100));
        assert_eq!(insert(&mut *host, 1), Ok(0));
        assert_eq!(battery.amount(host), 100);
    });
}
