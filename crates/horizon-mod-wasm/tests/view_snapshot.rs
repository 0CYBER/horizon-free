//! Compare the generated borrowed encoder with the owned snapshot conversion.
// Generated fixtures exercise every shape; TODO(arismoko/horizon#2273): make the generator emit clippy-clean code.
#![allow(clippy::all, unused_comparisons, unreachable_patterns, dead_code)]
#[path = "support/no_hooks.rs"]
mod no_hooks;
use api::wire::Writer;
use mod_api as api;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct Counting;
thread_local! {
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    static CALLS: Cell<usize> = const { Cell::new(0) };
}
fn allocated() {
    if ACTIVE.try_with(Cell::get).unwrap_or(false) {
        CALLS.with(|n| n.set(n.get() + 1));
    }
}
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        allocated();
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        allocated();
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        allocated();
        unsafe { System.realloc(p, layout, size) }
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        unsafe { System.dealloc(p, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;
fn count(f: impl FnOnce()) -> usize {
    CALLS.with(|n| n.set(0));
    ACTIVE.with(|n| n.set(true));
    f();
    ACTIVE.with(|n| n.set(false));
    CALLS.with(Cell::get)
}
#[test]
fn borrowed_snapshot_matches_owned_without_clone_allocations() {
    mod_api::__private::initialize().unwrap();
    use horizon_core::nbt::Tag;
    let mut entity = api::ModBlockEntity::default();
    entity.nbt.insert("progress".into(), Tag::Int(7));
    entity.nbt.insert(
        "nested".into(),
        Tag::List(vec![Tag::String("payload".into())]),
    );
    entity.items = vec![None; 5];
    entity.data = vec![7, 200];
    api::ModBlockEntityView::new(&mut entity).set_client_update_fields(entity_update());
    let mut owned = Writer::new();
    let mut borrowed = Writer::new();
    // Reserve outside the measured region, as on the warmed hook path.
    owned.replace(Vec::with_capacity(4096));
    borrowed.replace(Vec::with_capacity(4096));
    let old = count(|| {
        for _ in 0..1000 {
            owned.clear();
            owned.put(&api::item::containers::ModBlockEntitySnapshot::from(
                &entity,
            ));
        }
    });
    let new = count(|| {
        for _ in 0..1000 {
            borrowed.clear();
            api::ModBlockEntityView::new(&mut entity).__wire_put_snapshot(&mut borrowed);
        }
    });
    assert_eq!(owned.bytes(), borrowed.bytes());
    assert!(owned.error().is_none());
    assert!(borrowed.error().is_none());
    assert!(old > 0);
    assert_eq!(new, 0, "borrowed snapshot encoding allocated");
    println!("native snapshot allocator calls per 1000: owned={old}, borrowed={new}");
    let input = owned.bytes().to_vec();
    let old_roundtrip = count(|| {
        for _ in 0..1000 {
            let decoded: api::ModBlockEntity = api::wire::Reader::new(&input).take().unwrap();
            owned.clear();
            owned.put(&api::item::containers::ModBlockEntitySnapshot::from(
                &decoded,
            ));
        }
    });
    let new_roundtrip = count(|| {
        for _ in 0..1000 {
            let mut decoded: api::ModBlockEntity = api::wire::Reader::new(&input).take().unwrap();
            borrowed.clear();
            api::ModBlockEntityView::new(&mut decoded).__wire_put_snapshot(&mut borrowed);
        }
    });
    assert_eq!(owned.bytes(), borrowed.bytes());
    assert_eq!(old_roundtrip - new_roundtrip, old - new);
    println!(
        "native decode/reply allocator calls per 1000: owned={old_roundtrip}, borrowed={new_roundtrip}"
    );
    // A refused writer must still refuse the whole value, without touching state.
    borrowed.clear();
    borrowed.limit(1);
    api::ModBlockEntityView::new(&mut entity).__wire_put_snapshot(&mut borrowed);
    assert!(borrowed.error().is_some());
    assert_eq!(entity.data, [7, 200]);
}
#[test]
fn static_string_id_reply_decodes_without_allocating() {
    mod_api::__private::initialize().unwrap();
    use api::wire::{Reader, guest};
    use std::rc::Rc;
    guest::reset_static_strings();
    let mut definition = Writer::new();
    definition.put(&1u8);
    definition.put(&0u32);
    definition.put("minecraft:overworld");
    let bytes = definition.into_bytes();
    guest::set_transport(Some(Rc::new(move |_, _| bytes.clone())));
    let first = guest::host(0, guest::begin());
    assert_eq!(first.take_static::<()>().unwrap().0, "minecraft:overworld");
    drop(first);
    let mut known = Writer::new();
    known.put(&0u8);
    known.put(&0u32);
    let bytes = known.into_bytes();
    guest::set_transport(Some(Rc::new(move |_, _| bytes.clone())));
    let reply = guest::host(0, guest::begin());
    guest::set_transport(None);
    let mut old = Writer::new();
    old.put("minecraft:overworld");
    api::wire::intern("minecraft:overworld").unwrap();
    let before = count(|| {
        for _ in 0..1000 {
            let text: String = Reader::new(old.bytes()).take().unwrap();
            std::hint::black_box(api::wire::intern(&text).unwrap());
        }
    });
    let after = count(|| {
        for _ in 0..1000 {
            std::hint::black_box(reply.take_static::<()>().unwrap().0);
        }
    });
    assert_eq!(before, 1000);
    assert_eq!(after, 0);
    println!(
        "native static-reply decode allocator calls per 1000: string+intern={before}, id={after}"
    );
}
fn entity_update() -> api::Compound {
    let mut update = api::Compound::new();
    update.insert("visible".into(), horizon_core::nbt::Tag::Int(9));
    update
}
