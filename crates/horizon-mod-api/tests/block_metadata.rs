//! Worker reads cross the actual API metadata adapter without TLS table lookup or allocation.
use mod_api::{BlockMetadata, BlockState};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
thread_local! { static COUNT: Cell<Option<usize>> = const { Cell::new(None) }; }
struct Allocator;
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;
fn allocation() {
    let _ = COUNT.try_with(|count| {
        if let Some(n) = count.get() {
            count.set(Some(n + 1));
        }
    });
}
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        allocation();
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        allocation();
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        allocation();
        unsafe { System.realloc(ptr, layout, size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}
#[test]
fn metadata_resource_worker_reads_allocate_nothing() {
    mod_api::__private::install_parts(
        &[],
        &mod_api::__private::RuleSchema {
            category: &|_| false,
            position: &|_| None,
            rule_count: 0,
        },
        |_| false,
    )
    .unwrap();
    let properties = std::collections::BTreeMap::from([("lit".into(), "false".into())]);
    mod_api::__private::resolve_block_keys(["probe:lamp"], |_| {
        horizon_mod_runtime::block_definitions::BlockDefinition::new(&[&properties], 0)
            .map_err(|e| format!("{e:?}"))
    })
    .unwrap();
    let metadata: &'static (dyn BlockMetadata + Send + Sync) =
        mod_api::__private::block_definitions().unwrap();
    let state = BlockState::of(
        metadata,
        mod_api::__private::block_key("probe:lamp").unwrap(),
        &[],
    )
    .unwrap();
    std::thread::spawn(move || {
        COUNT.set(Some(0));
        for _ in 0..1_000 {
            assert_eq!(state.property(metadata, "lit"), Ok(Some("false")));
            assert_eq!(state.property(metadata, "missing"), Ok(None));
        }
        let count = COUNT.replace(None).unwrap();
        assert_eq!(count, 0);
    })
    .join()
    .unwrap();
    assert_eq!(state.property(metadata, "lit"), Ok(Some("false")));
}
