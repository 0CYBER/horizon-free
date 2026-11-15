//! The handlers and their codecs, against the fixture host: literal amounts on writes, the
//! hostile saved shapes refused, and the journal's nested snapshots.
use super::*;
use crate::{HostTransaction, ItemKey, testing::FakeHost};
use horizon_core::nbt::JavaString;
use std::cell::RefCell;

thread_local! {
    /// The one item `accepts_the_chosen` lets in: a `fn` pointer cannot capture it.
    static CHOSEN: RefCell<Option<ItemKey>> = const { RefCell::new(None) };
}
/// A `valid` the tests can point a handler at: `true` for the item `CHOSEN` holds.
fn accepts_the_chosen(resource: &ItemResource) -> bool {
    CHOSEN.with_borrow(|chosen| {
        let Some(chosen) = chosen else {
            return false;
        };
        resource
            .to_stack(1)
            .is_some_and(|stack| stack.item() == *chosen)
    })
}

static ITEMS: AttachmentType<StackList<ItemResource>> = AttachmentType {
    namespace: "attached_storage_test",
    name: "items",
    default: Vec::new,
    codec: Some(ITEM_STACKS),
    copy_on_death: false,
    sync: false,
};
static VALID_ITEMS: AttachmentType<StackList<ItemResource>> = AttachmentType {
    namespace: "attached_storage_test",
    name: "valid_items",
    default: Vec::new,
    codec: Some(ITEM_STACKS),
    copy_on_death: false,
    sync: false,
};
static FLUIDS: AttachmentType<StackList<FluidResource>> = AttachmentType {
    namespace: "attached_storage_test",
    name: "fluids",
    default: Vec::new,
    codec: Some(FLUID_STACKS),
    copy_on_death: false,
    sync: false,
};
static ENERGY: AttachmentType<u32> = AttachmentType {
    namespace: "attached_storage_test",
    name: "energy",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync: false,
};
const CHUNK: Holder = Holder::Chunk([2, 3]);

/// Complete the fixture load's tables with `items` and `fluids`, so the stacks below resolve.
fn resolve(items: &[&str], fluids: &[&str]) {
    let mut host = FakeHost::new();
    if !items.is_empty() {
        host.registries
            .resolve_items(
                items
                    .iter()
                    .map(|id| Identifier::parse(id).unwrap())
                    .collect::<Vec<_>>(),
            )
            .unwrap();
    }
    if !fluids.is_empty() {
        host.registries
            .resolve_fluids(fluids.iter().copied())
            .unwrap();
    }
}

/// The stack of the resolved item `id`, one of it.
fn item(id: &str) -> ItemResource {
    let key = item_key_by_identifier(&Identifier::parse(id).unwrap()).unwrap();
    ItemResource::of(&ItemStack::new(key, 1).unwrap())
}

/// One of the resolved fluid `id`.
fn fluid(id: &str) -> FluidResource {
    let key = crate::content::registry_keys::fluid_key(id).unwrap();
    FluidResource::of(&FluidStack {
        fluid: FluidIdentity::Resolved(key),
        amount: 1,
        components: None,
    })
}

/// The writes run in the World phase: the boundary's access checks refuse a call outside one.
fn in_world(test: impl FnOnce()) {
    let _world = crate::world::phase::PhaseOwner::scope(crate::world::phase::ModPhase::World);
    test();
}

/// The water amount the fluids codec saves and reads back, with its index.
#[test]
fn a_fluid_tank_round_trips_through_its_codec_and_moves() {
    in_world(|| {
        resolve(&[], &["minecraft:water"]);
        let mut host = FakeHost::new();
        let water = fluid("minecraft:water");
        let tanks = AttachedFluids::new(&FLUIDS, CHUNK, 3, 1000, |_| true).unwrap();
        let moved =
            host.transaction(|host, tx| Ok::<_, ()>(tanks.insert(host, tx, 1, &water, 250)));
        assert_eq!(moved, Ok(250));
        assert_eq!(tanks.amount(&mut host, 1), 250);
        // The codec of the saved attachment reads back to the same index and amount.
        let server: &mut dyn ServerHost = &mut host;
        let held = FLUIDS.get_existing(server, CHUNK).unwrap().unwrap();
        let codec = FLUIDS.codec.as_ref().unwrap();
        let saved = (codec.encode)(&held).unwrap();
        let reloaded = (codec.decode)(&saved).unwrap();
        assert_eq!(reloaded[1], Some((water.clone(), 250)));
        assert!(reloaded[..1].iter().all(Option::is_none));
        // A transfer takes the same amount out again.
        let came =
            host.transaction(|host, tx| Ok::<_, ()>(tanks.extract(host, tx, 1, &water, 250)));
        assert_eq!(came, Ok(250));
        assert_eq!(tanks.amount(&mut host, 1), 0);
    });
}

/// Extraction of a part of what an index holds, and none of another resource.
#[test]
fn extraction_takes_part_and_none_of_another_resource() {
    in_world(|| {
        resolve(&["minecraft:dirt", "minecraft:cobblestone"], &[]);
        let mut host = FakeHost::new();
        let dirt = item("minecraft:dirt");
        let cobble = item("minecraft:cobblestone");
        let slots = AttachedItems::new(&ITEMS, CHUNK, 9, 64, |_| true).unwrap();
        let went = host.transaction(|host, tx| Ok::<_, ()>(slots.insert(host, tx, 4, &dirt, 30)));
        assert_eq!(went, Ok(30));
        let none =
            host.transaction(|host, tx| Ok::<_, ()>(slots.extract(host, tx, 4, &cobble, 30)));
        assert_eq!(none, Ok(0), "none of another resource");
        let came = host.transaction(|host, tx| Ok::<_, ()>(slots.extract(host, tx, 4, &dirt, 12)));
        assert_eq!(came, Ok(12));
        assert_eq!(slots.amount(&mut host, 4), 18);
        let rest = host.transaction(|host, tx| Ok::<_, ()>(slots.extract(host, tx, 4, &dirt, 50)));
        assert_eq!(rest, Ok(18), "at most what remains");
        let empty = host.transaction(|host, tx| Ok::<_, ()>(slots.extract(host, tx, 4, &dirt, 1)));
        assert_eq!(empty, Ok(0));
        assert_eq!(slots.amount(&mut host, 4), 0);
    });
}

/// `capacity` is the physical limit of an in-range index, `is_valid` the permission, and
/// extraction independent of validity.
#[test]
fn capacity_stays_physical_while_validity_refuses_inserts_only() {
    in_world(|| {
        resolve(&["minecraft:dirt", "minecraft:cobblestone"], &[]);
        let mut host = FakeHost::new();
        let dirt = item("minecraft:dirt");
        let cobble = item("minecraft:cobblestone");
        CHOSEN.with_borrow_mut(|chosen| *chosen = Some(dirt.to_stack(1).unwrap().item()));
        let only_dirt =
            AttachedItems::new(&VALID_ITEMS, CHUNK, 4, 500, accepts_the_chosen).unwrap();
        assert_eq!(
            only_dirt.capacity(&mut host, 2, &cobble),
            64,
            "the physical limit (the item's stack size), though cobblestone is not valid here"
        );
        assert!(!only_dirt.is_valid(&mut host, 2, &cobble));
        let refused =
            host.transaction(|host, tx| Ok::<_, ()>(only_dirt.insert(host, tx, 2, &cobble, 10)));
        assert_eq!(refused, Ok(0));
        let went =
            host.transaction(|host, tx| Ok::<_, ()>(only_dirt.insert(host, tx, 2, &dirt, 120)));
        assert_eq!(went, Ok(64), "at most the item's stack size per index");
        // A cobblestone another (permissive) handler's write stored reads out through this one.
        let anything = AttachedItems::new(&VALID_ITEMS, CHUNK, 4, 500, |_| true).unwrap();
        let _ = host.transaction(|host, tx| Ok::<_, ()>(anything.insert(host, tx, 3, &cobble, 7)));
        let came =
            host.transaction(|host, tx| Ok::<_, ()>(only_dirt.extract(host, tx, 3, &cobble, 7)));
        assert_eq!(came, Ok(7), "extraction is independent of validity");
    });
}

/// The per-call limits and a nested abort: the inner transaction's write is undone, the
/// outer's stays.
#[test]
fn energy_limits_and_a_nested_abort() {
    in_world(|| {
        resolve(&[], &[]);
        let mut host = FakeHost::new();
        let battery = AttachedEnergy::new(&ENERGY, CHUNK, 100, 30, 20);
        let past_insert = host.transaction(|host, tx| Ok::<_, ()>(battery.insert(host, tx, 150)));
        assert_eq!(past_insert, Ok(30), "at most max_insert per call");
        let past_extract = host.transaction(|host, tx| Ok::<_, ()>(battery.extract(host, tx, 25)));
        assert_eq!(past_extract, Ok(20), "at most max_extract per call");
        let nested = host.transaction(|outer_host, outer_tx| {
            let aborted = outer_host.transaction(|host, tx| {
                battery.insert(host, tx, 10);
                Err::<(), _>(())
            });
            assert_eq!(aborted, Err(()));
            let came = battery.extract(&mut *outer_host, outer_tx, 5);
            assert_eq!(came, 5);
            Ok::<_, ()>(came)
        });
        assert_eq!(nested, Ok(5));
        assert_eq!(
            battery.amount(&mut host),
            5,
            "the inner write was undone, the outer's stayed"
        );
    });
}

/// A write through a smaller view keeps the higher indices a bigger one stored.
#[test]
fn a_smaller_view_keeps_higher_indices() {
    in_world(|| {
        resolve(&["minecraft:dirt"], &[]);
        let mut host = FakeHost::new();
        let dirt = item("minecraft:dirt");
        let big = AttachedItems::new(&ITEMS, CHUNK, 9, 64, |_| true).unwrap();
        let went = host.transaction(|host, tx| Ok::<_, ()>(big.insert(host, tx, 7, &dirt, 12)));
        assert_eq!(went, Ok(12));
        let small = AttachedItems::new(&ITEMS, CHUNK, 3, 64, |_| true).unwrap();
        let went = host.transaction(|host, tx| Ok::<_, ()>(small.insert(host, tx, 0, &dirt, 5)));
        assert_eq!(went, Ok(5));
        assert_eq!(big.amount(&mut host, 7), 12, "the higher index survived");
        assert_eq!(big.amount(&mut host, 0), 5);
        assert_eq!(
            small.amount(&mut host, 7),
            0,
            "past the smaller view's size"
        );
    });
}

/// A hostile `Slot` fails the decode before anything is allocated to its size.
#[test]
fn a_hostile_slot_is_refused_not_allocated() {
    in_world(|| {
        let hostile = |slot: i32| {
            let mut entry = Compound::new();
            entry.insert("Slot".into(), Tag::Int(slot));
            entry.insert("id".into(), Tag::String("minecraft:dirt".into()));
            entry.insert("count".into(), Tag::Int(1));
            let mut saved = Compound::new();
            saved.insert(
                JavaString::from("stacks"),
                Tag::List(vec![Tag::Compound(entry)]),
            );
            saved
        };
        assert_eq!(
            decode_stacks(&hostile(i32::MAX), |_| Some(((), 1))),
            Err(AttachmentError::Decode)
        );
        // The boundary itself is fine.
        let list = decode_stacks(&hostile((MAX_SLOTS - 1) as i32), |_| Some(((), 1))).unwrap();
        assert_eq!(list.len(), MAX_SLOTS);
        assert_eq!(list[MAX_SLOTS - 1], Some(((), 1)));
    });
}

/// The constructor, handler writes and saved slots share the same literal boundary.
#[test]
fn handler_and_codec_limits_agree() {
    in_world(|| {
        resolve(&["minecraft:dirt"], &[]);
        let mut host = FakeHost::new();
        let dirt = item("minecraft:dirt");
        assert!(AttachedItems::new(&ITEMS, CHUNK, 4097, 64, |_| true).is_none());
        let slots = AttachedItems::new(&ITEMS, CHUNK, 4096, 64, |_| true).unwrap();
        assert_eq!(
            host.transaction(|host, tx| Ok::<_, ()>(slots.insert(host, tx, 4095, &dirt, 1))),
            Ok(1)
        );
        assert_eq!(
            host.transaction(|host, tx| Ok::<_, ()>(slots.insert(host, tx, 4096, &dirt, 1))),
            Ok(0)
        );
        let server: &mut dyn ServerHost = &mut host;
        let mut held = ITEMS.get_existing(server, CHUNK).unwrap().unwrap();
        let codec = ITEMS.codec.as_ref().unwrap();
        let saved = (codec.encode)(&held).unwrap();
        let decoded = (codec.decode)(&saved).unwrap();
        assert_eq!(decoded.len(), 4096);
        assert_eq!(decoded[4095], Some((dirt.clone(), 1)));
        let mut hostile = saved;
        let Some(Tag::List(entries)) = hostile.get_mut(&"stacks".into()) else {
            panic!("saved stacks")
        };
        let Tag::Compound(entry) = &mut entries[0] else {
            panic!("saved entry")
        };
        entry.insert("Slot".into(), Tag::Int(4096));
        assert_eq!((codec.decode)(&hostile), Err(AttachmentError::Decode));
        held.push(Some((dirt, 1)));
        assert_eq!((codec.encode)(&held), Err(AttachmentError::Encode));
    });
}
