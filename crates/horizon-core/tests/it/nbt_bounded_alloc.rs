//! A bounded entry's list is refused from its header, before the reader reserves memory for
//! the count it declares. This is its own test binary because it counts the allocations of
//! the whole process.
use horizon_core::nbt::{Bounded, Limits, read_named_bounded};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

/// The system allocator, recording the largest single request.
struct Largest;
static LARGEST: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every call forwards to `System` with the same arguments; only a counter is added.
unsafe impl GlobalAlloc for Largest {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LARGEST.fetch_max(layout.size(), Ordering::Relaxed);
        // SAFETY: the caller's contract for `alloc` is passed on unchanged.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` came from `System.alloc` with this layout.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Largest = Largest;

#[test]
fn a_bounded_list_past_its_budget_reserves_nothing_for_its_count() {
    const ELEMENTS: usize = 1_000_000;
    let budget = Bounded {
        parents: &[&["p"]],
        key: "b",
        max_bytes: 512 * 1024,
        max_depth: 2,
    };
    // {p:{b:[1_000_000 bytes]}}: the file holds every element, the budget does not.
    let mut raw = vec![10, 0, 0, 10, 0, 1, b'p', 9, 0, 1, b'b', 1];
    raw.extend((ELEMENTS as i32).to_be_bytes());
    raw.resize(raw.len() + ELEMENTS, 0);
    raw.extend([0, 0]);
    LARGEST.store(0, Ordering::Relaxed);
    let (named, used, rejected) = read_named_bounded(&raw, Limits::saved(), budget).unwrap();
    let largest = LARGEST.load(Ordering::Relaxed);
    assert_eq!((used, rejected), (raw.len(), 1));
    assert_eq!(named.tag.get("p").and_then(|p| p.get("b")), None);
    // A `Vec<Tag>` for the declared count would be tens of megabytes.
    assert!(largest < ELEMENTS, "largest allocation {largest} bytes");
}
