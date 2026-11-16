//! Thread-local allocation counts for tests of allocation-free paths.
use std::alloc::{GlobalAlloc, Layout};
use std::cell::Cell;

thread_local! {
    static COUNT: Cell<Option<usize>> = const { Cell::new(None) };
}

fn record() {
    let _ = COUNT.try_with(|count| {
        if let Some(value) = count.get() {
            count.set(Some(value + 1));
        }
    });
}

/// The normal game allocator with a test-only, thread-local call counter.
pub struct Allocator;
/// The allocator installed by the game while allocation counting is enabled.
pub const ALLOCATOR: Allocator = Allocator;

// SAFETY: every operation delegates the unchanged arguments to mimalloc.
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record();
        unsafe { mimalloc::MiMalloc.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record();
        unsafe { mimalloc::MiMalloc.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { mimalloc::MiMalloc.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record();
        unsafe { mimalloc::MiMalloc.realloc(ptr, layout, size) }
    }
}

/// Count allocations, zeroed allocations and reallocations made by this thread during `f`.
/// Nested measurements are not supported; the counter is reset even if `f` panics.
pub fn measure<T>(f: impl FnOnce() -> T) -> (T, usize) {
    COUNT.with(|count| assert!(count.get().is_none(), "nested allocation measurement"));
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            COUNT.with(|count| count.set(None));
        }
    }
    COUNT.with(|count| count.set(Some(0)));
    let reset = Reset;
    let result = f();
    let count = COUNT.with(|count| count.get().unwrap());
    drop(reset);
    (result, count)
}
