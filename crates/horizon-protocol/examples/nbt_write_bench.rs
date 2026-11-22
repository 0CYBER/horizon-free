//! Measure packet-body writes, not fixture construction, compression or I/O.
//!
//! cargo run --release -p horizon-protocol --example nbt_write_bench -- [ITERATIONS]
//! One JSON sample per process. An operation writes and drops one packet body.

use horizon_core::nbt::Tag;
use horizon_protocol::{
    buf::{Codec, Writer},
    item::{Component, Slot, Stack, WrittenBook, component},
};
use serde_json::{Value, json};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    hint::black_box,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed},
    time::Instant,
};

struct CountingAllocator;

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;
static COUNTING: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

fn count_allocation() {
    if COUNTING.load(Relaxed) {
        ALLOCATIONS.fetch_add(1, Relaxed);
    }
}

// SAFETY: Every operation forwards the unchanged arguments to System, returns
// its result unchanged, and only touches non-allocating atomic counters.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count_allocation();
        // SAFETY: The caller supplies the layout required by GlobalAlloc::alloc.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count_allocation();
        // SAFETY: The caller supplies the layout required by GlobalAlloc::alloc_zeroed.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        count_allocation();
        // SAFETY: The pointer comes from System and the caller guarantees the old layout and new size.
        unsafe { System.realloc(ptr, layout, size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: The pointer comes from System and the caller provides its layout.
        unsafe { System.dealloc(ptr, layout) }
    }
}

fn text(value: &str) -> Tag {
    Tag::String(value.into())
}

fn colored_text(value: &str, color: &str) -> Tag {
    Tag::compound([("text", text(value)), ("color", text(color))])
}

fn nbt_fixture() -> [Option<Stack>; 3] {
    // Registry IDs are opaque to the pinned Slot writer; the patches exercise the real codecs.
    let mut named = Stack::new(1, 1);
    named.added.push((
        component::CUSTOM_NAME,
        Component::Text(colored_text("Trailblazer's Compass", "gold")),
    ));
    let mut with_lore = Stack::new(2, 1);
    with_lore.added.extend([
        (
            component::ITEM_NAME,
            Component::Text(text("Surveyor's Kit")),
        ),
        (
            component::LORE,
            Component::Lore(vec![
                text("Issue only to expedition members."),
                colored_text("Routes marked in blue", "blue"),
                text("Distance to camp: 420 blocks"),
                colored_text("Bring spare torches", "yellow"),
                text("Updated after the last snowfall."),
            ]),
        ),
    ]);
    let mut book = Stack::new(3, 1);
    book.added.push((
        component::WRITTEN_BOOK_CONTENT,
        Component::WrittenBook(WrittenBook {
            title: "Notes from the Ridge".into(),
            author: "Mira".into(),
            pages: vec![
                colored_text("Day 1: Cross the river at dawn and follow the stone markers.", "dark_blue"),
                text("Day 2: The trail splits below the old lookout. The east fork leads to camp; refill your bottles at the spring."),
                text("Day 3: Leave a lantern in the shelter for the next party. Record the weather before heading back."),
            ],
            pages_filtered: vec![None, Some(text("Day 2: Take the east fork to camp.")), None],
            resolved: true,
            ..WrittenBook::default()
        }),
    ));
    [Some(named), Some(with_lore), Some(book)]
}

fn measure(iterations: usize, packet: impl Fn() -> Writer) -> Value {
    let bytes_per_op = packet().data.len();
    let sample_iterations = iterations.min(10_000);
    for _ in 0..sample_iterations {
        black_box(packet().data);
    }
    let start = Instant::now();
    for _ in 0..iterations {
        black_box(packet().data);
    }
    let ns_per_op = start.elapsed().as_secs_f64() * 1e9 / iterations as f64;

    // Count allocation and reallocation calls in a separate pass, so the
    // extra atomic increments on the baseline don't inflate its timings.
    ALLOCATIONS.store(0, Relaxed);
    COUNTING.store(true, Relaxed);
    for _ in 0..sample_iterations {
        black_box(packet().data);
    }
    COUNTING.store(false, Relaxed);
    let allocs_per_op = ALLOCATIONS.load(Relaxed) as f64 / sample_iterations as f64;
    json!({
        "iterations": iterations,
        "bytes_per_op": bytes_per_op,
        "ns_per_op": ns_per_op,
        "allocs_per_op": allocs_per_op,
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let iterations = std::env::args()
        .nth(1)
        .map_or(Ok(500_000), |arg| arg.parse::<usize>())?;
    if iterations == 0 {
        return Err("ITERATIONS must be greater than zero".into());
    }
    let stacks = nbt_fixture();
    let nbt_mix = measure(iterations, || {
        let mut packet = Writer::new();
        let stacks = black_box(&stacks);
        packet.varint(stacks.len() as i32);
        for stack in stacks {
            Slot::write(stack, &mut packet);
        }
        packet
    });
    let varints = [
        0,
        1,
        3,
        11,
        42,
        127,
        128,
        255,
        300,
        3_000,
        16_383,
        16_384,
        65_536,
        2_097_152,
        -1,
        i32::MIN,
    ];
    let varint_control = measure(iterations, || {
        let mut packet = Writer::new();
        for &value in black_box(&varints) {
            packet.varint(value);
        }
        packet
    });
    println!(
        "{}",
        json!({"nbt_mix": nbt_mix, "varint_control": varint_control})
    );
    Ok(())
}
