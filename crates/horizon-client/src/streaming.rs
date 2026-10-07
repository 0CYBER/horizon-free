//! Chunk streaming: a worker pool generates, lights and meshes chunk columns
//! around the player, closest first. The game thread owns the world and only
//! hands immutable `Arc` snapshots to workers; results carry a sequence number
//! so stale work is dropped.
use crate::{
    chunk::{Chunk, SECTIONS},
    light::{LightTables, light_chunk},
    mesher::{MeshTables, Mesher, Region, SectionMesh, borrow},
    world::World,
};
use std::{
    collections::{BinaryHeap, HashMap, HashSet},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

/// Produces fully generated chunk columns. Implementations must be thread-safe;
/// each worker thread owns one `Worker` for per-thread caches.
pub trait ChunkGenerator: Send + Sync + 'static {
    type Worker: Send + 'static;
    fn worker(&self) -> Self::Worker;
    fn generate(&self, worker: &mut Self::Worker, x: i32, z: i32) -> Chunk;
}
enum Job {
    Generate(i32, i32),
    Light(i32, i32, Box<Region>, u64),
    Mesh(i32, i32, i32, Box<Region>, u64),
}
pub enum Done {
    Generated(Chunk),
    Lit(i32, i32, Vec<Arc<crate::chunk::Section>>, u64),
    Meshed(i32, i32, i32, SectionMesh, u64),
}
struct Queued {
    priority: i64,
    order: u64,
    job: Job,
}
impl PartialEq for Queued {
    fn eq(&self, o: &Self) -> bool {
        self.priority == o.priority && self.order == o.order
    }
}
impl Eq for Queued {}
impl PartialOrd for Queued {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Queued {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        // BinaryHeap is a max-heap: lower priority value = more urgent.
        o.priority
            .cmp(&self.priority)
            .then(o.order.cmp(&self.order))
    }
}
struct Shared {
    queue: Mutex<BinaryHeap<Queued>>,
    ready: Condvar,
    stop: AtomicBool,
}
pub struct Streaming {
    shared: Arc<Shared>,
    results: mpsc::Receiver<Done>,
    threads: Vec<thread::JoinHandle<()>>,
    order: u64,
    generating: HashSet<(i32, i32)>,
    lighting: HashMap<(i32, i32), u64>,
    meshing: HashMap<(i32, i32, i32), u64>,
    /// Chunks whose light is current.
    lit: HashSet<(i32, i32)>,
    /// Columns whose sections have been meshed at least once.
    meshed: HashSet<(i32, i32)>,
    sequence: u64,
    pub center: (i32, i32),
    pub radius: i32,
    pub generated_total: u64,
    pub meshed_total: u64,
}
pub struct WorkerContext {
    pub assets: Arc<crate::assets::Assets>,
    pub tables: Arc<MeshTables>,
    pub biomes: Arc<crate::biome::Biomes>,
    pub light: Arc<LightTables>,
}
impl Streaming {
    pub fn new<G: ChunkGenerator>(generator: Arc<G>, context: Arc<WorkerContext>, threads: usize) -> Self {
        let shared = Arc::new(Shared {
            queue: Mutex::new(BinaryHeap::new()),
            ready: Condvar::new(),
            stop: AtomicBool::new(false),
        });
        let (send, results) = mpsc::channel();
        let threads = (0..threads.max(1))
            .map(|i| {
                let shared = shared.clone();
                let send = send.clone();
                let generator = generator.clone();
                let context = context.clone();
                thread::Builder::new()
                    .name(format!("horizon-worker-{i}"))
                    .spawn(move || {
                        let mut worker = generator.worker();
                        let mut mesher = Mesher {
                            assets: &context.assets,
                            tables: &context.tables,
                            biomes: &context.biomes,
                            visibility: context.tables.visibility.clone(),
                        };
                        loop {
                            let job = {
                                let mut queue = shared.queue.lock().unwrap();
                                loop {
                                    if shared.stop.load(Ordering::Relaxed) {
                                        return;
                                    }
                                    if let Some(job) = queue.pop() {
                                        break job.job;
                                    }
                                    queue = shared.ready.wait(queue).unwrap();
                                }
                            };
                            let done = match job {
                                Job::Generate(x, z) => {
                                    Done::Generated(generator.generate(&mut worker, x, z))
                                }
                                Job::Light(x, z, region, seq) => {
                                    Done::Lit(x, z, light_chunk(&borrow(&region), &context.light), seq)
                                }
                                Job::Mesh(x, sy, z, region, seq) => {
                                    Done::Meshed(x, sy, z, mesher.mesh(&borrow(&region), sy), seq)
                                }
                            };
                            if send.send(done).is_err() {
                                return;
                            }
                        }
                    })
                    .expect("spawn worker")
            })
            .collect();
        Self {
            shared,
            results,
            threads,
            order: 0,
            generating: HashSet::new(),
            lighting: HashMap::new(),
            meshing: HashMap::new(),
            lit: HashSet::new(),
            meshed: HashSet::new(),
            sequence: 0,
            center: (0, 0),
            radius: 8,
            generated_total: 0,
            meshed_total: 0,
        }
    }
    fn submit(&mut self, priority: i64, job: Job) {
        self.order += 1;
        self.shared.queue.lock().unwrap().push(Queued {
            priority,
            order: self.order,
            job,
        });
        self.shared.ready.notify_one();
    }
    fn distance(&self, x: i32, z: i32) -> i64 {
        let (dx, dz) = (i64::from(x - self.center.0), i64::from(z - self.center.1));
        dx * dx + dz * dz
    }
    fn region(world: &World, x: i32, z: i32) -> Option<Box<Region>> {
        let mut region: Region = Default::default();
        for dz in 0..3 {
            for dx in 0..3 {
                region[dz][dx] = Some(world.chunks.get(&(x + dx as i32 - 1, z + dz as i32 - 1))?.clone());
            }
        }
        Some(Box::new(region))
    }
    pub fn is_lit(&self, x: i32, z: i32) -> bool {
        self.lit.contains(&(x, z))
    }
    pub fn pending(&self) -> usize {
        self.shared.queue.lock().unwrap().len()
    }
    /// Advance streaming: collect finished work, then schedule more.
    /// Returns meshes ready for upload and columns that were unloaded.
    pub fn update(&mut self, world: &mut World) -> (Vec<((i32, i32, i32), SectionMesh)>, Vec<Arc<Chunk>>) {
        let mut uploads = Vec::new();
        while let Ok(done) = self.results.try_recv() {
            match done {
                Done::Generated(chunk) => {
                    let key = (chunk.x, chunk.z);
                    self.generating.remove(&key);
                    if self.in_range(key.0, key.1, 2) {
                        self.generated_total += 1;
                        world.chunks.insert(key, Arc::new(chunk));
                        for dz in -1..=1 {
                            for dx in -1..=1 {
                                world.dirty_light.insert((key.0 + dx, key.1 + dz));
                            }
                        }
                    }
                }
                Done::Lit(x, z, sections, seq) => {
                    if self.lighting.get(&(x, z)) != Some(&seq) {
                        continue;
                    }
                    self.lighting.remove(&(x, z));
                    if let Some(chunk) = world.chunks.get_mut(&(x, z)) {
                        let chunk = Arc::make_mut(chunk);
                        // Keep blocks edited since the snapshot; take only light.
                        for (target, lit) in chunk.sections.iter_mut().zip(sections) {
                            let mut section = (**target).clone();
                            section.light = lit.light.clone();
                            *target = Arc::new(section);
                        }
                        chunk.lit = true;
                        self.lit.insert((x, z));
                        for dz in -1..=1 {
                            for dx in -1..=1 {
                                for sy in 0..SECTIONS as i32 {
                                    world.dirty_sections.insert((x + dx, sy, z + dz));
                                }
                            }
                        }
                    }
                }
                Done::Meshed(x, sy, z, mesh, seq) => {
                    if self.meshing.get(&(x, sy, z)) != Some(&seq) {
                        continue;
                    }
                    self.meshing.remove(&(x, sy, z));
                    self.meshed_total += 1;
                    uploads.push(((x, sy, z), mesh));
                }
            }
        }
        // Unload far columns.
        let far: Vec<_> = world
            .chunks
            .keys()
            .copied()
            .filter(|&(x, z)| !self.in_range(x, z, 3))
            .collect();
        let mut removed = Vec::with_capacity(far.len());
        for key in &far {
            if let Some(chunk) = world.chunks.remove(key) {
                removed.push(chunk);
            }
            self.lit.remove(key);
            self.meshed.remove(key);
        }
        // Generation, closest first, bounded in flight.
        let workers = self.threads.len();
        if self.generating.len() < workers * 3 {
            let r = self.radius + 1;
            let mut wanted = Vec::new();
            for dz in -r..=r {
                for dx in -r..=r {
                    let (x, z) = (self.center.0 + dx, self.center.1 + dz);
                    if !self.in_range(x, z, 1)
                        || world.chunks.contains_key(&(x, z))
                        || self.generating.contains(&(x, z))
                    {
                        continue;
                    }
                    wanted.push((self.distance(x, z), x, z));
                }
            }
            wanted.sort_unstable();
            for (d, x, z) in wanted.into_iter().take(workers * 3 - self.generating.len()) {
                self.generating.insert((x, z));
                self.submit(d * 4, Job::Generate(x, z));
            }
        }
        // Lighting once all neighbours exist.
        let light: Vec<_> = world.dirty_light.iter().copied().collect();
        for (x, z) in light {
            if !world.chunks.contains_key(&(x, z)) {
                world.dirty_light.remove(&(x, z));
                continue;
            }
            let Some(region) = Self::region(world, x, z) else {
                continue;
            };
            world.dirty_light.remove(&(x, z));
            self.sequence += 1;
            self.lighting.insert((x, z), self.sequence);
            self.submit(self.distance(x, z) * 4 + 1, Job::Light(x, z, region, self.sequence));
        }
        // Meshing once the column and its neighbours are lit.
        let dirty: Vec<_> = world.dirty_sections.iter().copied().collect();
        let mut columns: HashMap<(i32, i32), Option<Box<Region>>> = HashMap::new();
        for (x, sy, z) in dirty {
            if !world.chunks.contains_key(&(x, z)) || !self.in_range(x, z, 0) {
                world.dirty_sections.remove(&(x, sy, z));
                continue;
            }
            let ready = (-1..=1).all(|dz| (-1..=1).all(|dx| self.lit.contains(&(x + dx, z + dz))));
            if !ready || self.lighting.contains_key(&(x, z)) {
                continue;
            }
            let region = columns
                .entry((x, z))
                .or_insert_with(|| Self::region(world, x, z));
            let Some(region) = region.clone() else { continue };
            world.dirty_sections.remove(&(x, sy, z));
            let first = self.meshed.insert((x, z));
            self.sequence += 1;
            self.meshing.insert((x, sy, z), self.sequence);
            self.submit(
                self.distance(x, z) * 4 + if first { 2 } else { 0 },
                Job::Mesh(x, sy, z, region, self.sequence),
            );
        }
        (uploads, removed)
    }
    fn in_range(&self, x: i32, z: i32, margin: i32) -> bool {
        let r = i64::from(self.radius + margin);
        self.distance(x, z) <= r * r
    }
}
impl Drop for Streaming {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.ready.notify_all();
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}
