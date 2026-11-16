//! Stands in for the game: a counting global allocator that records which pointers it handed out.
//! Every allocation underneath is `System`, so a pointer freed by another allocator is detected, not a crash.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

const SLOTS: usize = 1 << 16;
const GONE: usize = 1;
static LIVE: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];
static FOREIGN_FREES: AtomicUsize = AtomicUsize::new(0);

fn slot(pointer: usize) -> impl Iterator<Item = &'static AtomicUsize> {
    let start = (pointer >> 4).wrapping_mul(0x9E37_79B9_7F4A_7C15) % SLOTS;
    (0..SLOTS).map(move |step| &LIVE[(start + step) % SLOTS])
}

struct Counting;
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            let found = slot(pointer as usize).find(|entry| {
                [0, GONE].into_iter().any(|empty| {
                    entry
                        .compare_exchange(empty, pointer as usize, SeqCst, SeqCst)
                        .is_ok()
                })
            });
            assert!(found.is_some(), "allocation table full");
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        let owned = slot(pointer as usize)
            .take_while(|entry| entry.load(SeqCst) != 0)
            .find(|entry| {
                entry
                    .compare_exchange(pointer as usize, GONE, SeqCst, SeqCst)
                    .is_ok()
            });
        if owned.is_none() {
            FOREIGN_FREES.fetch_add(1, SeqCst);
        }
        unsafe { System.dealloc(pointer, layout) }
    }
}

#[global_allocator]
static GAME: Counting = Counting;

/// Whether the game's allocator handed out this live pointer.
pub fn owned(pointer: *const u8) -> bool {
    slot(pointer as usize)
        .take_while(|entry| entry.load(SeqCst) != 0)
        .any(|entry| entry.load(SeqCst) == pointer as usize)
}
pub fn foreign_frees() -> usize {
    FOREIGN_FREES.load(SeqCst)
}
pub fn make() -> Vec<u8> {
    std::hint::black_box(vec![7; 4096])
}
pub fn free(value: Vec<u8>) {
    drop(std::hint::black_box(value))
}
