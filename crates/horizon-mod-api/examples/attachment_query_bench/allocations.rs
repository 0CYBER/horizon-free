//! Requested allocation bytes, not allocator bookkeeping or resident memory.
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};
#[derive(Clone, Copy, Debug, Default)]
pub struct Counts {
    pub allocations: usize,
    pub bytes: usize,
    pub live: isize,
}
thread_local! {
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    static COUNTS: Cell<Counts> = Cell::new(Counts::default());
}
struct Counting;
#[global_allocator]
static ALLOCATOR: Counting = Counting;
fn record(allocated: usize, freed: usize) {
    if ACTIVE.try_with(Cell::get).unwrap_or(false) {
        let _ = COUNTS.try_with(|cell| {
            let mut counts = cell.get();
            counts.allocations += usize::from(allocated != 0);
            counts.bytes += allocated;
            counts.live += allocated as isize - freed as isize;
            cell.set(counts);
        });
    }
}
// SAFETY: every allocation operation delegates unchanged to the system allocator.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record(layout.size(), 0);
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record(layout.size(), 0);
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record(size, layout.size());
        unsafe { System.realloc(ptr, layout, size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        record(0, layout.size());
        unsafe { System.dealloc(ptr, layout) }
    }
}
pub fn measured<T>(f: impl FnOnce() -> T) -> (T, Counts) {
    COUNTS.set(Counts::default());
    ACTIVE.set(true);
    let value = f();
    ACTIVE.set(false);
    (value, COUNTS.get())
}
