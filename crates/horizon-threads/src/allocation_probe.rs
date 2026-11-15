//! Opt-in test allocator accounting on the invoking thread, not live-byte estimates.
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};
#[derive(Clone, Copy, Debug, Default)]
pub struct Counts {
    pub calls: usize,
    pub bytes: usize,
    pub large_calls: usize,
    pub large_bytes: usize,
}
thread_local! { static COUNTS: Cell<Option<Counts>> = const { Cell::new(None) }; }
fn record(size: usize) {
    let _ = COUNTS.try_with(|cell| {
        if let Some(mut counts) = cell.get() {
            counts.calls += 1;
            counts.bytes += size;
            if size >= 1 << 20 {
                counts.large_calls += 1;
                counts.large_bytes += size;
            }
            cell.set(Some(counts));
        }
    });
}
/// Run one synchronous production path while measuring requested allocation bytes.
/// Includes realloc requests; does not claim all allocated bytes were copied.
pub fn measure<R>(run: impl FnOnce() -> R) -> (R, Counts) {
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            COUNTS.with(|c| c.set(None));
        }
    }
    COUNTS.with(|c| {
        assert!(c.get().is_none());
        c.set(Some(Counts::default()));
    });
    let _guard = Guard;
    let result = run();
    let counts = COUNTS.with(|c| c.get().unwrap());
    (result, counts)
}
/// Test-only global allocator; accounting is inactive outside `measure`.
pub struct Allocator;
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record(size);
        unsafe { System.realloc(ptr, layout, size) }
    }
}
