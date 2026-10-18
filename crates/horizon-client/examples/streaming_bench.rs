//! The chunk pipeline (generation, decoration, lighting, meshing) on its worker
//! pool, without a window: how fast a render distance fills with N workers, and
//! what the render thread pays for it in `Streaming::update`.
//!
//! streaming_bench RESOURCES [--threads N] [--radius R] [--seed S] [--fly CHUNKS_PER_S] [--seconds T] [--hz F]
//!
//! First the area around (0, 0) fills; then, with `--fly`, the centre moves along +x
//! at that speed for T seconds while frames run at F per second. Each phase reports
//! its wall time, chunks generated and sections meshed, and the time `update`
//! took on the calling thread (the render thread in the game): total, median,
//! p99 and slowest. Compile with HORIZON_BENCH_COUNT_ALLOCATIONS=1 to count the
//! allocations made on the calling thread during updates and by everything.
use horizon_client::{
    assets::Assets,
    biome::Biomes,
    blocks::BlockFacts,
    light::LightTables,
    mesher::MeshTables,
    models,
    streaming::{Streaming, WorkerContext},
    vanillagen::VanillaTerrain,
    world::World,
};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering::Relaxed},
    },
    time::{Duration, Instant},
};

const TRACK: bool = option_env!("HORIZON_BENCH_COUNT_ALLOCATIONS").is_some();
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);
thread_local! {
    static LOCAL: Cell<(u64, u64)> = const { Cell::new((0, 0)) };
}
fn count(size: usize) {
    if TRACK {
        ALLOCATIONS.fetch_add(1, Relaxed);
        BYTES.fetch_add(size as u64, Relaxed);
        let _ = LOCAL.try_with(|c| {
            let (n, b) = c.get();
            c.set((n + 1, b + size as u64));
        });
    }
}
struct CountingAllocator;
// Delegates every operation and layout unchanged to System.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        count(size);
        unsafe { System.realloc(ptr, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Default)]
struct Phase {
    updates: Vec<f64>,
    local: (u64, u64),
    uploads: usize,
}
impl Phase {
    fn update(&mut self, streaming: &mut Streaming, world: &mut World) {
        let before = LOCAL.with(Cell::get);
        let started = Instant::now();
        let (uploads, _) = streaming.update(world);
        self.updates.push(started.elapsed().as_secs_f64() * 1e3);
        let after = LOCAL.with(Cell::get);
        self.local.0 += after.0 - before.0;
        self.local.1 += after.1 - before.1;
        self.uploads += uploads.len();
        drop(uploads);
    }
    fn report(&self, name: &str, wall: Duration, streaming: &Streaming, generated: u64) {
        let mut sorted = self.updates.clone();
        sorted.sort_by(f64::total_cmp);
        let at = |p: f64| sorted[((sorted.len() as f64 * p) as usize).min(sorted.len() - 1)];
        let total: f64 = sorted.iter().sum();
        println!(
            "{name}: {:.2} s, {} chunks generated, {} sections meshed; update {:.1} ms total over {} calls, p50 {:.3} p99 {:.3} max {:.2} ms",
            wall.as_secs_f64(),
            streaming.generated_total - generated,
            self.uploads,
            total,
            sorted.len(),
            at(0.5),
            at(0.99),
            sorted.last().copied().unwrap_or(0.0),
        );
        if TRACK {
            println!(
                "  update allocations: {} ({:.1} MiB)",
                self.local.0,
                self.local.1 as f64 / 1048576.0
            );
        }
    }
}

/// Nothing queued or in flight. Sections at the edge of the area stay dirty
/// (their outer neighbours are never lit), so they don't count.
fn settled(streaming: &Streaming) -> bool {
    streaming
        .stages()
        .iter()
        .all(|&(name, n)| name == "protos" || n == 0)
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = PathBuf::from(args.first().ok_or("usage: streaming_bench RESOURCES")?);
    let flag = |name: &str, default: f64| -> f64 {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    let auto = std::thread::available_parallelism().map_or(2, |n| n.get() - 1);
    let threads = flag("--threads", auto as f64) as usize;
    let radius = flag("--radius", 8.0) as i32;
    let seed = flag("--seed", 1234.0) as i64;
    let fly = flag("--fly", 0.0);
    let seconds = flag("--seconds", 10.0);
    let hz = flag("--hz", 240.0);

    let assets = Arc::new(Assets::load(&root)?);
    let render = Arc::new(assets.reload(&root)?);
    let facts = BlockFacts::load(&root, assets.registry)?;
    let biomes = Arc::new(Biomes::load(&root)?);
    let tables = Arc::new(MeshTables::new(
        &assets,
        &render,
        &facts,
        models::load_visibility(&root, assets.registry)?,
    )?);
    let light = Arc::new(LightTables::load(&root, assets.registry, &facts)?);
    let terrain = Arc::new(VanillaTerrain::new(
        &root.join("worldgen"),
        assets.registry,
        &biomes,
        seed,
    )?);
    let context = Arc::new(WorkerContext {
        assets,
        render: render.clone(),
        tables,
        biomes,
        light,
    });
    let mut world = World::new(facts.air, seed);
    let mut streaming = Streaming::new(terrain, context, threads);
    streaming.radius = radius;
    streaming.center = (0, 0);
    println!("{threads} workers, radius {radius}, seed {seed}");
    let (allocations, bytes) = (ALLOCATIONS.load(Relaxed), BYTES.load(Relaxed));

    // Fill: update as often as the game would uncapped, until settled.
    let mut phase = Phase::default();
    let started = Instant::now();
    let mut quiet = 0;
    while quiet < 20 {
        let uploads = phase.uploads;
        phase.update(&mut streaming, &mut world);
        quiet = if settled(&streaming) && phase.uploads == uploads {
            quiet + 1
        } else {
            0
        };
        std::thread::sleep(Duration::from_micros(500));
    }
    phase.report("fill", started.elapsed(), &streaming, 0);

    if fly > 0.0 {
        let mut phase = Phase::default();
        let generated = streaming.generated_total;
        let frame = Duration::from_secs_f64(1.0 / hz);
        let started = Instant::now();
        let mut behind = Vec::new();
        while started.elapsed().as_secs_f64() < seconds {
            let next = Instant::now() + frame;
            let x = (started.elapsed().as_secs_f64() * fly) as i32;
            streaming.center = (x, 0);
            phase.update(&mut streaming, &mut world);
            // Columns within 2 chunks of the player still missing their meshes.
            let near = (-2..=2)
                .flat_map(|dz| (-2..=2).map(move |dx| (x + dx, dz)))
                .filter(|&(cx, cz)| !streaming.is_lit(cx, cz))
                .count();
            behind.push(near);
            if let Some(wait) = next.checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            }
        }
        phase.report("fly", started.elapsed(), &streaming, generated);
        let unlit = behind.iter().filter(|&&n| n > 0).count();
        println!(
            "  frames with an unlit column within 2 chunks of the player: {unlit} of {} ({:.1}%)",
            behind.len(),
            unlit as f64 * 100.0 / behind.len().max(1) as f64
        );
    }
    if TRACK {
        println!(
            "all threads: {} allocations ({:.1} MiB)",
            ALLOCATIONS.load(Relaxed) - allocations,
            (BYTES.load(Relaxed) - bytes) as f64 / 1048576.0
        );
    }
    Ok(())
}
