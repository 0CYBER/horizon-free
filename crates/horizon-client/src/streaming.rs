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

/// A chunk under generation, opaque to the streamer.
pub type Proto = Box<dyn std::any::Any + Send>;
/// Chunk generation in vanilla's stages: a neighbour-independent proto chunk
/// (biomes, noise, surface, carvers), decoration of a 3x3 neighbourhood
/// (FEATURES, which may write into neighbours), then conversion of a chunk
/// whose eight neighbours are decorated into a final column. Implementations
/// must be thread-safe; each worker thread owns one `Worker`.
pub trait ChunkGenerator: Send + Sync + 'static {
    type Worker: Send + 'static;
    fn worker(&self) -> Self::Worker;
    /// A previously saved final chunk, bypassing generation.
    fn load(&self, _x: i32, _z: i32) -> Option<Chunk> {
        None
    }
    fn proto(&self, worker: &mut Self::Worker, x: i32, z: i32) -> Proto;
    fn decorates(&self) -> bool {
        false
    }
    /// Decorate `region[4]`; the region is row-major (dz, dx) around it.
    fn decorate(&self, _worker: &mut Self::Worker, _region: &mut [Proto; 9]) {}
    fn finish(&self, proto: Proto) -> Chunk;
}
enum Job {
    Generate(i32, i32),
    Decorate(i32, i32, Box<[Proto; 9]>),
    Finish(Proto),
    Light(i32, i32, Box<Region>, u64, bool),
    Mesh(i32, i32, i32, Box<Region>, u64, bool),
}
pub enum Done {
    Generated(Chunk),
    Proto(i32, i32, Proto),
    Decorated(i32, i32, Box<[Proto; 9]>),
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
    /// Proto chunks: `None` while checked out to a decoration job.
    protos: HashMap<(i32, i32), Option<Proto>>,
    decorated: HashSet<(i32, i32)>,
    decorating: usize,
    finishing: HashSet<(i32, i32)>,
    decorates: bool,
    lighting: HashMap<(i32, i32), u64>,
    meshing: HashMap<(i32, i32, i32), u64>,
    /// Chunks whose light is current.
    lit: HashSet<(i32, i32)>,
    /// Columns whose sections have been meshed at least once.
    meshed: HashSet<(i32, i32)>,
    sequence: u64,
    pub center: (i32, i32),
    pub radius: i32,
    /// `/forceload`ed chunks: kept loaded whatever the distance.
    pub forced: HashSet<(i32, i32)>,
    /// No work is scheduled while paused (title screen).
    pub paused: bool,
    /// Dimension flags: sky light, Nether face shading.
    pub sky_light: bool,
    pub nether_shading: bool,
    /// Incremental relighting of block edits on lit chunks (see `light::relight`).
    light_tables: Arc<LightTables>,
    /// Applied edits by sequence, replayed onto light jobs snapshotted before them.
    light_log: Vec<(u64, (i32, i32, i32))>,
    light_edit_sequence: u64,
    /// Edit sequence at each in-flight light job's snapshot.
    light_snapshots: HashMap<(i32, i32), u64>,
    pub generated_total: u64,
    pub meshed_total: u64,
    /// Chunks come from a server (`insert_remote`/`forget`): nothing is
    /// generated and columns are only unloaded when the server says so.
    pub remote: bool,
}
pub struct WorkerContext {
    pub assets: Arc<crate::assets::Assets>,
    pub tables: Arc<MeshTables>,
    pub biomes: Arc<crate::biome::Biomes>,
    pub light: Arc<LightTables>,
}
impl Streaming {
    pub fn new<G: ChunkGenerator>(
        generator: Arc<G>,
        context: Arc<WorkerContext>,
        threads: usize,
    ) -> Self {
        let shared = Arc::new(Shared {
            queue: Mutex::new(BinaryHeap::new()),
            ready: Condvar::new(),
            stop: AtomicBool::new(false),
        });
        let (send, results) = mpsc::channel();
        let light_tables = context.light.clone();
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
                            nether_shading: false,
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
                                Job::Generate(x, z) => match generator.load(x, z) {
                                    Some(chunk) => Done::Generated(chunk),
                                    None => Done::Proto(x, z, generator.proto(&mut worker, x, z)),
                                },
                                Job::Decorate(x, z, mut region) => {
                                    generator.decorate(&mut worker, &mut region);
                                    Done::Decorated(x, z, region)
                                }
                                Job::Finish(proto) => Done::Generated(generator.finish(proto)),
                                Job::Light(x, z, region, seq, sky) => Done::Lit(
                                    x,
                                    z,
                                    light_chunk(&borrow(&region), &context.light, sky),
                                    seq,
                                ),
                                Job::Mesh(x, sy, z, region, seq, nether) => {
                                    mesher.nether_shading = nether;
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
            protos: HashMap::new(),
            decorated: HashSet::new(),
            decorating: 0,
            finishing: HashSet::new(),
            decorates: generator.decorates(),
            lighting: HashMap::new(),
            meshing: HashMap::new(),
            lit: HashSet::new(),
            meshed: HashSet::new(),
            sequence: 0,
            center: (0, 0),
            radius: 8,
            forced: HashSet::new(),
            paused: false,
            sky_light: true,
            nether_shading: false,
            light_tables,
            light_log: Vec::new(),
            light_edit_sequence: 0,
            light_snapshots: HashMap::new(),
            generated_total: 0,
            meshed_total: 0,
            remote: false,
        }
    }
    /// A column received from a server: store it and relight it and its neighbours.
    pub fn insert_remote(&mut self, world: &mut World, chunk: Chunk) {
        let key = (chunk.x, chunk.z);
        self.lit.remove(&key);
        self.lighting.remove(&key);
        world.chunks.insert(key, Arc::new(chunk));
        self.generated_total += 1;
        for dz in -1..=1 {
            for dx in -1..=1 {
                world.dirty_light.insert((key.0 + dx, key.1 + dz));
            }
        }
    }
    /// A column the server unloaded (ClientboundForgetLevelChunkPacket).
    pub fn forget(&mut self, world: &mut World, x: i32, z: i32) -> Option<Arc<Chunk>> {
        self.lit.remove(&(x, z));
        self.meshed.remove(&(x, z));
        self.lighting.remove(&(x, z));
        self.meshing.retain(|k, _| (k.0, k.2) != (x, z));
        world.dirty_sections.retain(|k| (k.0, k.2) != (x, z));
        world.chunks.remove(&(x, z))
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
                region[dz][dx] = Some(
                    world
                        .chunks
                        .get(&(x + dx as i32 - 1, z + dz as i32 - 1))?
                        .clone(),
                );
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
    pub fn update(
        &mut self,
        world: &mut World,
    ) -> (Vec<((i32, i32, i32), SectionMesh)>, Vec<Arc<Chunk>>) {
        let mut uploads = Vec::new();
        if self.paused {
            return (uploads, Vec::new());
        }
        self.relight_edits(world);
        while let Ok(done) = self.results.try_recv() {
            match done {
                Done::Proto(x, z, proto) => {
                    self.generating.remove(&(x, z));
                    if self.in_range(x, z, 4) {
                        self.protos.insert((x, z), Some(proto));
                        if !self.decorates {
                            self.decorated.insert((x, z));
                        }
                    }
                }
                Done::Decorated(x, z, region) => {
                    self.decorating -= 1;
                    self.decorated.insert((x, z));
                    for (i, proto) in region.into_iter().enumerate() {
                        let key = (x + (i % 3) as i32 - 1, z + (i / 3) as i32 - 1);
                        self.protos.insert(key, Some(proto));
                    }
                }
                Done::Generated(mut chunk) => {
                    let key = (chunk.x, chunk.z);
                    world
                        .block_entities
                        .extend(std::mem::take(&mut chunk.block_entities));
                    world
                        .block_names
                        .extend(std::mem::take(&mut chunk.block_names));
                    let entities = std::mem::take(&mut chunk.entities);
                    if !entities.is_empty() {
                        world.loaded_entities.push(((chunk.x, chunk.z), entities));
                    }
                    self.generating.remove(&key);
                    self.finishing.remove(&key);
                    self.decorated.insert(key);
                    if self.in_range(key.0, key.1, 2) {
                        self.generated_total += 1;
                        // A chunk saved with its light needs no light job; its
                        // unlit neighbours still light themselves from it.
                        let loaded = chunk.light_loaded;
                        world.chunks.insert(key, Arc::new(chunk));
                        if loaded {
                            self.lit.insert(key);
                            for sy in 0..SECTIONS as i32 {
                                world.dirty_sections.insert((key.0, sy, key.1));
                            }
                        }
                        for dz in -1..=1 {
                            for dx in -1..=1 {
                                let n = (key.0 + dx, key.1 + dz);
                                if !loaded || world.chunks.get(&n).is_some_and(|c| !c.light_loaded)
                                {
                                    world.dirty_light.insert(n);
                                }
                            }
                        }
                    }
                }
                Done::Lit(x, z, sections, seq) => {
                    if self.lighting.get(&(x, z)) != Some(&seq) {
                        continue;
                    }
                    self.lighting.remove(&(x, z));
                    let snapshot = self.light_snapshots.remove(&(x, z)).unwrap_or(u64::MAX);
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
                        self.replay_light_edits(world, x, z, snapshot);
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
            .filter(|&(x, z)| !self.remote && !self.in_range(x, z, 3))
            .collect();
        for key in &far {
            self.decorated.remove(key);
        }
        let mut removed = Vec::with_capacity(far.len());
        for key in &far {
            if let Some(chunk) = world.chunks.remove(key) {
                removed.push(chunk);
            }
            self.lit.remove(key);
            self.meshed.remove(key);
        }
        // Drop far proto chunks that are not checked out.
        let (cx, cz, r4) = (self.center.0, self.center.1, i64::from(self.radius + 4));
        let forced = &self.forced;
        self.protos.retain(|&(x, z), p| {
            let d = i64::from(x - cx).pow(2) + i64::from(z - cz).pow(2);
            p.is_none()
                || d <= r4 * r4
                || forced
                    .iter()
                    .any(|f| (x - f.0).abs() <= 3 && (z - f.1).abs() <= 3)
        });
        let (protos, finishing) = (&self.protos, &self.finishing);
        self.decorated.retain(|k| {
            protos.contains_key(k) || finishing.contains(k) || world.chunks.contains_key(k)
        });
        // Finalize chunks whose neighbourhood is decorated.
        let candidates: Vec<(i32, i32)> = self.protos.keys().copied().collect();
        for (x, z) in candidates {
            if world.chunks.contains_key(&(x, z))
                || self.finishing.contains(&(x, z))
                || !self.in_range(x, z, 1)
            {
                continue;
            }
            let ready = (-1..=1).all(|dz| {
                (-1..=1).all(|dx| {
                    self.decorated.contains(&(x + dx, z + dz))
                        || world.chunks.contains_key(&(x + dx, z + dz))
                })
            });
            if ready && let Some(Some(_)) = self.protos.get(&(x, z)) {
                let proto = self
                    .protos
                    .remove(&(x, z))
                    .flatten()
                    .expect("present proto");
                self.finishing.insert((x, z));
                self.submit(self.distance(x, z) * 4, Job::Finish(proto));
            }
        }
        // Decorate chunks whose 3x3 proto neighbourhood is available.
        let workers = self.threads.len();
        if self.decorates && self.decorating < workers * 2 {
            let mut wanted: Vec<(i64, i32, i32)> = self
                .protos
                .iter()
                .filter(|(k, p)| {
                    p.is_some() && !self.decorated.contains(k) && self.in_range(k.0, k.1, 2)
                })
                .map(|(k, _)| (self.distance(k.0, k.1), k.0, k.1))
                .collect();
            wanted.sort_unstable();
            for (d, x, z) in wanted {
                if self.decorating >= workers * 2 {
                    break;
                }
                let keys: Vec<(i32, i32)> =
                    (0..9).map(|i| (x + (i % 3) - 1, z + (i / 3) - 1)).collect();
                // A final (saved) neighbour cannot receive writes: skip decoration.
                if keys.iter().any(|k| world.chunks.contains_key(k)) {
                    self.decorated.insert((x, z));
                    continue;
                }
                if !keys
                    .iter()
                    .all(|k| matches!(self.protos.get(k), Some(Some(_))))
                {
                    continue;
                }
                let region: Vec<Proto> = keys
                    .iter()
                    .map(|k| {
                        self.protos
                            .insert(*k, None)
                            .flatten()
                            .expect("present proto")
                    })
                    .collect();
                let region: Box<[Proto; 9]> = match region.try_into() {
                    Ok(r) => Box::new(r),
                    Err(_) => continue,
                };
                self.decorating += 1;
                self.submit(d * 4 + 1, Job::Decorate(x, z, region));
            }
        }
        // Generation (load or proto), closest first, bounded in flight.
        let margin = if self.decorates { 3 } else { 1 };
        if !self.remote && self.generating.len() < workers * 3 {
            let r = self.radius + margin;
            let mut wanted = Vec::new();
            for dz in -r..=r {
                for dx in -r..=r {
                    let (x, z) = (self.center.0 + dx, self.center.1 + dz);
                    if !self.in_range(x, z, margin)
                        || world.chunks.contains_key(&(x, z))
                        || self.generating.contains(&(x, z))
                        || self.protos.contains_key(&(x, z))
                        || self.finishing.contains(&(x, z))
                    {
                        continue;
                    }
                    wanted.push((self.distance(x, z), x, z));
                }
            }
            // Forced chunks and their generation neighbourhood.
            let reach = margin - 1;
            for &(fx, fz) in &self.forced {
                for dz in -reach..=reach {
                    for dx in -reach..=reach {
                        let (x, z) = (fx + dx, fz + dz);
                        if world.chunks.contains_key(&(x, z))
                            || self.generating.contains(&(x, z))
                            || self.protos.contains_key(&(x, z))
                            || self.finishing.contains(&(x, z))
                        {
                            continue;
                        }
                        wanted.push((self.distance(x, z), x, z));
                    }
                }
            }
            wanted.sort_unstable();
            wanted.dedup();
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
            self.light_snapshots
                .insert((x, z), self.light_edit_sequence);
            self.submit(
                self.distance(x, z) * 4 + 1,
                Job::Light(x, z, region, self.sequence, self.sky_light),
            );
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
            let Some(region) = region.clone() else {
                continue;
            };
            world.dirty_sections.remove(&(x, sy, z));
            let first = self.meshed.insert((x, z));
            self.sequence += 1;
            self.meshing.insert((x, sy, z), self.sequence);
            self.submit(
                self.distance(x, z) * 4 + if first { 2 } else { 0 },
                Job::Mesh(x, sy, z, region, self.sequence, self.nether_shading),
            );
        }
        (uploads, removed)
    }
    /// Relight block edits on lit chunks in place (vanilla's incremental light
    /// engine) instead of recomputing the 3x3 chunks around each edit.
    fn relight_edits(&mut self, world: &mut World) {
        if world.light_edits.is_empty() {
            return;
        }
        let edits = std::mem::take(&mut world.light_edits);
        let positions: Vec<(i32, i32, i32)> = edits
            .iter()
            .filter(|(_, old, new)| self.light_tables.differs(*old, *new))
            .map(|(p, _, _)| *p)
            .collect();
        if positions.is_empty() {
            return;
        }
        let changed = crate::light::relight(
            &mut world.chunks,
            &self.light_tables,
            self.sky_light,
            &positions,
            false,
        );
        Self::mark_light_changes(world, &changed);
        if !self.light_snapshots.is_empty() {
            for p in positions {
                self.light_edit_sequence += 1;
                self.light_log.push((self.light_edit_sequence, p));
            }
        }
    }
    /// A light job computed from blocks older than some edits: redo those
    /// edits on top of its result.
    fn replay_light_edits(&mut self, world: &mut World, x: i32, z: i32, snapshot: u64) {
        let positions: Vec<(i32, i32, i32)> = self
            .light_log
            .iter()
            .filter(|(seq, p)| {
                *seq > snapshot && ((p.0 >> 4) - x).abs() <= 1 && ((p.2 >> 4) - z).abs() <= 1
            })
            .map(|(_, p)| *p)
            .collect();
        if !positions.is_empty() {
            let changed = crate::light::relight(
                &mut world.chunks,
                &self.light_tables,
                self.sky_light,
                &positions,
                true,
            );
            Self::mark_light_changes(world, &changed);
        }
        let oldest = self.light_snapshots.values().copied().min();
        match oldest {
            Some(oldest) => self.light_log.retain(|(seq, _)| *seq > oldest),
            None => self.light_log.clear(),
        }
    }
    /// Sections whose meshes sample a changed light value (faces and corners
    /// read light from neighbouring blocks).
    fn mark_light_changes(world: &mut World, changed: &[(i32, i32, i32)]) {
        let mut last = None;
        for &(x, y, z) in changed {
            for dz in -1..=1 {
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let s = (
                            (y + dy - crate::chunk::MIN_Y) >> 4,
                            (x + dx) >> 4,
                            (z + dz) >> 4,
                        );
                        if last == Some(s) || !(0..SECTIONS as i32).contains(&s.0) {
                            continue;
                        }
                        last = Some(s);
                        world.dirty_sections.insert((s.1, s.0, s.2));
                    }
                }
            }
        }
    }
    fn in_range(&self, x: i32, z: i32, margin: i32) -> bool {
        let r = i64::from(self.radius + margin);
        if self.distance(x, z) <= r * r {
            return true;
        }
        let reach = (margin - 1).max(0);
        self.forced
            .iter()
            .any(|f| (x - f.0).abs() <= reach && (z - f.1).abs() <= reach)
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
