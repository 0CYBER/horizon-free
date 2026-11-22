//! Chunk streaming: a worker pool generates, lights and meshes chunk columns
//! around the player, closest first. The game thread owns the world and only
//! hands immutable `Arc` snapshots to workers; results carry a sequence number
//! so stale work is dropped.
use crate::fast_hash::{FxHashMap, FxHashSet};
use crate::mod_chunk_tickets::{ModTicket, ModTickets};
use crate::section_mask::SectionMask;
use crate::{
    chunk::Chunk,
    chunk_map::ChunkMembershipDelta,
    graphics::LeafCullingMode,
    light::{LightChanges, LightTables, light_chunk},
    mesher::{MeshScratch, MeshTables, Region, SectionMesh, borrow},
    world::World,
};
use horizon_core::biome::BiomeZoom;
use horizon_threads::jobs::{Class, PerWorker, Rank, Registration, Source};
use horizon_world::worldgen::decoration_order::DecorationOrder;
use std::{
    collections::{BinaryHeap, HashSet, VecDeque},
    sync::{Arc, Condvar, Mutex, MutexGuard, RwLock, TryLockError},
};

/// How long a worker waits with nothing to do before it gives back its
/// generator's scratch buffers ([`ChunkGenerator::idle`]).
pub const IDLE_RELEASE: std::time::Duration = std::time::Duration::from_secs(5);

/// Worker CPU time and job counts by kind, for benchmarks
/// (`examples/gen_light_bench.rs`). Off unless enabled: a relaxed load per
/// job then.
pub mod job_stats;
/// A chunk under generation, opaque to the streamer.
pub type Proto = Box<dyn std::any::Any + Send>;

/// A proto checked out for saving, before its worker takes ownership.
pub struct ProtoSave {
    pub pos: (i32, i32),
    pub proto: Proto,
}

/// Borrowed, read-only generation state, before serialization on the save worker.
/// Fields synthesized only by the encoder are not available through this view.
pub struct ProtoSnapshot<'a> {
    pub dimension: &'a str,
    pub pos: [i32; 2],
    pub proto: &'a Proto,
    pub access: &'a dyn ProtoAccess,
    pub status: horizon_world::chunk_status::ChunkStatus,
}
impl ProtoSnapshot<'_> {
    pub fn block(&self, pos: [i32; 3]) -> Option<u32> {
        self.access.block(self.proto, pos)
    }
}
/// Chunk generation in vanilla's stages: a neighbour-independent proto chunk
/// (biomes, noise, surface, carvers), decoration of a 3x3 neighbourhood
/// (FEATURES, which may write into neighbours), then conversion of a chunk
/// whose eight neighbours are decorated into a final column. Implementations
/// must be thread-safe; each worker of the shared pool
/// (`horizon_threads::jobs`) keeps one `Worker` per streamer.
pub trait ChunkGenerator: Send + Sync + 'static {
    type Worker: Send + 'static;
    fn worker(&self) -> Self::Worker;
    /// The persistent wrapper supplies starts from their owning chunk, in every dimension.
    fn set_saved_starts(
        &self,
        _source: Arc<horizon_world::worldgen::generator::SavedStructureStarts>,
    ) {
    }
    fn lod_sampler(self: Arc<Self>) -> Box<dyn LodSampler>
    where
        Self: Sized,
    {
        let worker = self.worker();
        Box::new(GeneratorSampler {
            generator: self,
            worker,
        })
    }
    /// An undecorated far-terrain column, without allocating gameplay chunks.
    fn lod_surface(
        &self,
        _worker: &mut Self::Worker,
        _x: i32,
        _z: i32,
    ) -> Option<crate::lod::Surface> {
        None
    }
    /// A previously saved final chunk, bypassing generation.
    fn load(&self, _x: i32, _z: i32) -> Option<Chunk> {
        None
    }
    /// What a loaded chunk needs from the generator before the game uses it,
    /// on the worker that loaded it.
    fn prepare_loaded(&self, _worker: &mut Self::Worker, _chunk: &Chunk) {}
    /// The worker has had nothing to do for [`IDLE_RELEASE`]: give back what
    /// it keeps between jobs only to go faster.
    fn idle(&self, _worker: &mut Self::Worker) {}
    /// Every worker has had nothing to do for [`IDLE_RELEASE`]: give back the
    /// caches the workers share, which only spare work for chunks generated
    /// next to each other in a run. Results do not change.
    fn all_idle(&self) {}
    fn proto(&self, worker: &mut Self::Worker, x: i32, z: i32) -> Proto;
    fn decorates(&self) -> bool {
        false
    }
    /// Decorate `region[4]`; the region is row-major (dz, dx) around it.
    fn decorate(&self, _worker: &mut Self::Worker, _region: &mut [Proto; 9]) {}
    /// Whether a chunk next to final ones (loaded from the save, as the
    /// chunks of a world an older version made) is decorated, its features
    /// spilling into them (`proto_beside_final`, `final_writes`), as vanilla's
    /// are; otherwise it is left undecorated.
    fn decorates_beside_final(&self) -> bool {
        false
    }
    /// The proto chunk of a final chunk, for a decoration region to hold: its
    /// blocks are `snapshot`'s.
    fn proto_beside_final(&self, _worker: &mut Self::Worker, _snapshot: &Chunk) -> Option<Proto> {
        None
    }
    /// What decoration wrote into the proto chunk of a final chunk: the blocks
    /// that differ from `snapshot`, as `([x, y, z], state)`.
    fn final_writes(&self, _snapshot: &Chunk, _proto: &Proto) -> Vec<([i32; 3], u32)> {
        Vec::new()
    }
    /// The block entities decoration made at the blocks `writes` put into a
    /// final chunk (a chest of a structure piece, a spawner), with their
    /// contents.
    fn final_block_entities(
        &self,
        _entries: &[(&Proto, &[([i32; 3], u32)])],
    ) -> Vec<((i32, i32, i32), crate::chunk::block_entity::BlockEntity)> {
        Vec::new()
    }
    fn finish(&self, proto: Proto) -> Chunk;
    /// Whether `proto` has been decorated (a saved one can come back decorated).
    fn is_decorated(&self, _proto: &Proto) -> bool {
        false
    }
    /// Generation status of a proto that can be saved, if known.
    fn proto_status(&self, _proto: &Proto) -> Option<horizon_world::chunk_status::ChunkStatus> {
        None
    }
    /// Read an absolute block position from the proto without making a column copy.
    fn proto_block(&self, _proto: &Proto, _pos: [i32; 3]) -> Option<u32> {
        None
    }
    /// What to save of a proto chunk dropped before it became final.
    /// A decorated proto chunk's blocks as a column, to light a finished
    /// neighbour against (vanilla lights with neighbours at FEATURES).
    fn light_view(&self, _proto: &Proto) -> Option<Chunk> {
        None
    }
    /// A decorated proto chunk's column sections (`Chunk::sections`
    /// indexes) holding a block other than air: what its neighbours' light
    /// layers count once it is at INITIALIZE_LIGHT.
    fn non_air_sections(&self, _proto: &Proto) -> Option<SectionMask> {
        None
    }
    /// What the generator's shared caches hold: (name, entries, bytes, if
    /// known), for `horizon-ctl memory`.
    fn cache_census(&self) -> Vec<(&'static str, usize, Option<usize>)> {
        Vec::new()
    }
    /// A proto chunk's heap bytes by part, and its sections by bits per
    /// block, for `horizon-ctl memory` (`ProtoChunk::heap_bytes`).
    fn proto_heap_bytes(&self, _proto: &Proto) -> ([usize; 6], [usize; 16]) {
        ([0; 6], [0; 16])
    }
    fn saved_proto(&self, _proto: &Proto) -> Option<crate::chunk::SavedProto> {
        None
    }
    /// Put a saved proto chunk's state back onto `proto`, generated again.
    fn restore_proto(&self, _proto: &mut Proto, _saved: crate::chunk::SavedProto) {}
    /// Save proto chunks decoration wrote to, so that they come back as they
    /// were: generating them again would lose those writes.
    fn save_protos(&self, _protos: &[&Proto]) {}
    /// Save a column that finished after the player moved away: its
    /// neighbours' decoration is done, so it can't be generated again.
    fn save_finished(&self, _chunk: Chunk) {}
}
pub type LodDetail = ((i32, i32), Arc<[crate::lod::Column; 256]>);

/// Independent, thread-owned far-terrain sampling state.
pub trait LodSampler: Send {
    /// Independent sampling state; native detail generation stays with the owner.
    fn fork(&self) -> Option<Box<dyn LodSampler>> {
        None
    }
    fn request_detail(&mut self, _chunks: &[(i32, i32)], _center: [i32; 2]) {}
    fn drain_details(&mut self) -> Vec<LodDetail> {
        Vec::new()
    }
    /// A loaded chunk's summary, made on the far-terrain worker.
    fn observed(&mut self, _key: (i32, i32), _columns: &Arc<[crate::lod::Column; 256]>) {}
    /// Reduced far detail (`graphics::FarDetail::Reduced`): `grid` may then
    /// approximate wide cells.
    fn set_approximate(&mut self, _approximate: bool) {}
    fn sample(&mut self, x: i32, z: i32) -> Option<crate::lod::Surface>;
    fn column(&mut self, x: i32, z: i32) -> crate::lod::Column {
        crate::lod::Column::from_surface(self.sample(x, z))
    }
    fn reduced_column(&mut self, x: i32, z: i32, _width: i32) -> crate::lod::Column {
        self.column(x, z)
    }
    /// `side` x `side` columns from (`x`, `z`), `stride` blocks apart; none
    /// when sampling failed and should be tried again later.
    fn grid(&mut self, x: i32, z: i32, stride: i32, side: usize) -> Vec<crate::lod::Column> {
        let mut columns = Vec::with_capacity(side * side);
        for dz in 0..side {
            for dx in 0..side {
                columns.push(self.reduced_column(
                    x + dx as i32 * stride,
                    z + dz as i32 * stride,
                    stride,
                ));
            }
        }
        columns
    }
}
pub trait LodSource: Send + Sync {
    fn sampler(self: Arc<Self>) -> Box<dyn LodSampler>;
    /// Whether its samplers' `observed` chunks become its own (a server's
    /// kept terrain): tiles sampled without one are sampled again.
    fn keeps_observed(&self) -> bool {
        false
    }
}
struct GeneratorSampler<G: ChunkGenerator> {
    generator: Arc<G>,
    worker: G::Worker,
}
impl<G: ChunkGenerator> LodSampler for GeneratorSampler<G> {
    fn sample(&mut self, x: i32, z: i32) -> Option<crate::lod::Surface> {
        self.generator.lod_surface(&mut self.worker, x, z)
    }
}
impl<G: ChunkGenerator> LodSource for G {
    fn sampler(self: Arc<Self>) -> Box<dyn LodSampler> {
        ChunkGenerator::lod_sampler(self)
    }
}
/// The generator's saving, for the streamer (which doesn't know its type).
/// Read-only access to checked-out generation state.
pub trait ProtoAccess: Send + Sync {
    fn status(&self, proto: &Proto) -> Option<horizon_world::chunk_status::ChunkStatus>;
    fn block(&self, proto: &Proto, pos: [i32; 3]) -> Option<u32>;
}
impl<G: ChunkGenerator> ProtoAccess for G {
    fn status(&self, proto: &Proto) -> Option<horizon_world::chunk_status::ChunkStatus> {
        self.proto_status(proto)
    }
    fn block(&self, proto: &Proto, pos: [i32; 3]) -> Option<u32> {
        self.proto_block(proto, pos)
    }
}
trait Saves: LodSource + ProtoAccess {
    fn save_protos(&self, protos: &[&Proto]);
    fn save_finished(&self, chunk: Chunk);
}
impl<G: ChunkGenerator> Saves for G {
    fn save_protos(&self, protos: &[&Proto]) {
        ChunkGenerator::save_protos(self, protos);
    }
    fn save_finished(&self, chunk: Chunk) {
        ChunkGenerator::save_finished(self, chunk);
    }
}
/// Per-job mesher inputs that change with the dimension or the options.
#[derive(Clone)]
struct MeshSettings {
    nether: bool,
    shader_pack_lighting: bool,
    separate_ao: bool,
    shader_pack: bool,
    looks: bool,
    zoom: BiomeZoom,
    blend_radius: i32,
    smooth_lighting: bool,
    cutout_leaves: bool,
    opaque_ice: bool,
    translucent_layers: Arc<[bool]>,
    block_layers: Arc<crate::mesher::BlockLayerTable>,
    leaf_culling: LeafCullingMode,
}
enum Job {
    Generate(i32, i32),
    /// A region to decorate, with the final chunks in it (region index and
    /// the chunk as it is): their slots hold nothing until the worker makes
    /// their proto chunks.
    Decorate(i32, i32, Box<[Proto; 9]>, Vec<(usize, Chunk)>),
    Finish(Proto),
    Light(i32, i32, Box<Region>, u64, bool),
    Mesh(i32, i32, i32, Arc<Region>, u64, MeshSettings),
    /// Proto chunks checked out to be saved (`ChunkGenerator::save_protos`),
    /// off the main thread like vanilla's IOWorker.
    Save(Vec<ProtoSave>),
    /// A proto chunk checked out for its light view (`light_region`).
    LightView(i32, i32, Proto),
    /// Meshes the renderer is done with (`Streaming::free`): their memory
    /// is given back here, not in the frame.
    Free(Vec<((i32, i32, i32), SectionMesh)>),
}
/// What decoration wrote into the final chunks of its region.
#[derive(Default)]
pub struct FinalWrites {
    blocks: Vec<([i32; 3], u32)>,
    block_entities: Vec<((i32, i32, i32), crate::chunk::block_entity::BlockEntity)>,
}
pub enum Done {
    Generated(Chunk),
    /// A proto chunk, and whether it was decorated (loaded so from the save).
    Proto(i32, i32, Proto, bool),
    /// A decorated region, the region indices of its final chunks and what
    /// decoration wrote into them.
    Decorated(i32, i32, Box<[Proto; 9]>, Vec<usize>, FinalWrites),
    Lit(i32, i32, Vec<crate::chunk::Light>, u64),
    Meshed(i32, i32, i32, SectionMesh, u64),
    Saved(Vec<((i32, i32), Proto)>),
    /// A proto chunk back from `Job::LightView`, with its view.
    LightView(i32, i32, Proto, Option<Chunk>),
}
struct Queued {
    priority: i64,
    /// The column the job is for and what it adds to the column's distance
    /// (`Streaming::submit`): the priority is computed again from them when
    /// the centre moves (`Streaming::requeue`). None for saves, which go first.
    at: Option<(i32, i32)>,
    bias: i64,
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
    /// What meshing reads (block models and sprites), swapped when resource
    /// packs reload; each mesh job takes the current one.
    context: RwLock<Arc<WorkerContext>>,
    done: Mailbox,
    /// Only workers use this pool; frames never wait for its lock. Retaining
    /// cleared buffers avoids racing the renderer for the process heap.
    meshes: Mutex<MeshPool>,
    aborted: std::sync::atomic::AtomicBool,
}
#[derive(Default)]
struct MeshPool {
    meshes: Vec<SectionMesh>,
    bytes: usize,
}
impl MeshPool {
    const BYTES: usize = 16 << 20;
    const COUNT: usize = 128;
    fn take(&mut self) -> SectionMesh {
        let mesh = self.meshes.pop().unwrap_or_default();
        self.bytes -= mesh.capacity_bytes();
        mesh
    }
    /// A rejected mesh is dropped by the caller, after releasing the lock.
    fn put(&mut self, mut mesh: SectionMesh) -> Option<SectionMesh> {
        let bytes = mesh.capacity_bytes();
        if bytes == 0 || self.meshes.len() == Self::COUNT || bytes > Self::BYTES - self.bytes {
            return Some(mesh);
        }
        mesh.clear();
        self.bytes += bytes;
        self.meshes.push(mesh);
        None
    }
}

/// The chunks `wanted` in the order they decorate: by rank, lowest first. No
/// two share a rank, so only the chunks without one yet need their distance:
/// those no ticket reached (forced and extra areas) are ranked after every
/// other, nearest first. The pass that pops them stops once the workers are
/// busy, usually long before the last, so a heap orders only what it takes. A
/// server with players spread over the map holds tens of thousands of proto
/// chunks; measuring and sorting them all every tick took a fifth of its tick.
fn decoration_queue(
    order: &mut DecorationOrder,
    wanted: impl Iterator<Item = (i32, i32)>,
    distance: impl Fn(i32, i32) -> i64,
) -> std::collections::BinaryHeap<std::cmp::Reverse<(u64, i32, i32)>> {
    let mut ranked = Vec::new();
    let mut fresh = Vec::new();
    for (x, z) in wanted {
        match order.rank(x, z) {
            Some(rank) => ranked.push(std::cmp::Reverse((rank, x, z))),
            None => fresh.push((distance(x, z), x, z)),
        }
    }
    fresh.sort_unstable();
    for (_, x, z) in fresh {
        ranked.push(std::cmp::Reverse((order.rank_or_append(x, z), x, z)));
    }
    std::collections::BinaryHeap::from(ranked)
}

/// Keep only decoration ranks that can still matter: within the active ranges
/// (where arriving protos are kept) or already held in `protos`. Out-of-range
/// `generating` chunks are dropped on arrival (`Done::Proto`), so keeping
/// their old ranks would let them override a later band's row-by-row order.
fn retain_decoration_order(
    order: &mut DecorationOrder,
    ranges: &Ranges<'_>,
    protos: &FxHashMap<(i32, i32), Option<Proto>>,
) {
    order.retain(|x, z| {
        ranges.in_range(x, z, 4) || ranges.near_finished(x, z, 2) || protos.contains_key(&(x, z))
    });
}

/// Inputs to the decoration blocking predicate (`waits_for`): whether an
/// undecorated chunk can still be decorated and therefore must hold back
/// higher-ranked overlapping neighbours within Chebyshev distance 2.
struct DecorationWait<'a> {
    ranges: &'a Ranges<'a>,
    world: &'a World,
    decorated: &'a FxHashSet<(i32, i32)>,
    protos: &'a FxHashMap<(i32, i32), Option<Proto>>,
    generating: &'a FxHashSet<(i32, i32)>,
    saves_beside_final: bool,
}

impl DecorationWait<'_> {
    fn wants(&self, x: i32, z: i32) -> bool {
        !self.decorated.contains(&(x, z))
            && !self.world.chunks.contains_key(&(x, z))
            && (self.ranges.in_range(x, z, 2) || self.ranges.near_finished(x, z, 1))
    }

    fn waits_for(&self, x: i32, z: i32) -> bool {
        self.wants(x, z)
            && (-1..=1).all(|dz| {
                (-1..=1).all(|dx| {
                    let n = (x + dx, z + dz);
                    if self.world.chunks.contains_key(&n) {
                        self.saves_beside_final
                    } else {
                        self.protos.contains_key(&n)
                            || self.generating.contains(&n)
                            || self.ranges.requested(n.0, n.1, true)
                    }
                })
            })
    }
}

/// Finished jobs, from the workers to the game thread, in the order they
/// finished. Not an `mpsc` channel: its receiver waits for a sender it finds
/// half way through a send, and a worker descheduled there on a busy machine
/// held the frame for a scheduler quantum (10 to 30 ms, the RD 32 ocean
/// flight's 1% lows). The frame takes what is there without waiting; while a
/// worker holds the lock, it takes it the next frame.
#[derive(Default)]
struct Mailbox {
    done: Mutex<VecDeque<Done>>,
    arrived: Condvar,
}
impl Mailbox {
    fn send(&self, done: Done) {
        self.done.lock().unwrap().push_back(done);
        self.arrived.notify_one();
    }
    fn try_lock(&self) -> Option<MutexGuard<'_, VecDeque<Done>>> {
        match self.done.try_lock() {
            Ok(done) => Some(done),
            Err(TryLockError::WouldBlock) => None,
            Err(TryLockError::Poisoned(e)) => panic!("{e}"),
        }
    }
    /// Every finished job, or none while a worker is sending.
    #[cfg_attr(not(test), allow(dead_code))]
    fn take(&self) -> VecDeque<Done> {
        self.try_lock()
            .map(|mut done| std::mem::take(&mut *done))
            .unwrap_or_default()
    }
    /// Swap every finished job into `dst`, reusing both queues' heap buffers across frames.
    fn take_into(&self, dst: &mut VecDeque<Done>) {
        dst.clear();
        if let Some(mut done) = self.try_lock()
            && !done.is_empty()
        {
            std::mem::swap(&mut *done, dst);
        }
    }
    fn try_recv(&self) -> Option<Done> {
        self.try_lock()?.pop_front()
    }
    fn recv_timeout(&self, timeout: std::time::Duration) -> Option<Done> {
        let done = self.done.lock().unwrap();
        let (mut done, _) = self
            .arrived
            .wait_timeout_while(done, timeout, |done| done.is_empty())
            .unwrap();
        done.pop_front()
    }
}
/// [`Streaming::proto_census`].
#[derive(Debug, Default)]
pub struct ProtoCensus {
    pub protos: usize,
    /// Packed blocks, section palettes and lists, heightmaps, other terrain
    /// data, biomes, the box and its structure lists.
    pub bytes: [usize; 6],
    pub sections_by_bits: [usize; 16],
    pub light_views: usize,
    pub light_view_bytes: usize,
    /// `ChunkGenerator::cache_census`.
    pub caches: Vec<(&'static str, usize, Option<usize>)>,
}
fn plan_unloads(world: &World, unload: impl Fn(i32, i32) -> bool) -> Vec<(i32, i32)> {
    world
        .chunks
        .keys()
        .copied()
        .filter(|&(x, z)| unload(x, z))
        .collect()
}

fn commit_unloads(world: &mut World, keys: &[(i32, i32)]) -> Vec<Arc<Chunk>> {
    keys.iter()
        .filter_map(|&key| world.unload_chunk(key))
        .collect()
}

/// An update paused before removing its out-of-range live chunks.
pub struct StreamingUpdate {
    uploads: Vec<((i32, i32, i32), SectionMesh)>,
    sweep: bool,
    far: Vec<(i32, i32)>,
    paused: bool,
}

/// Result of processing generation results, pausing before a late full save.
pub enum PlanStep {
    Ready(StreamingUpdate),
    Save { chunk: Chunk, resume: PlanResume },
}

/// A save step is pending, ready for proto submission, or paused at a late full chunk.
pub enum SaveStep {
    Pending,
    Full(Chunk),
    Protos(Vec<ProtoSave>),
}

/// Work retained while a late full-chunk save callback runs.
pub struct PlanResume {
    sweep: bool,
    uploads: Vec<((i32, i32, i32), SectionMesh)>,
    stale: Vec<((i32, i32, i32), SectionMesh)>,
    deferred: std::vec::IntoIter<Done>,
    received: VecDeque<Done>,
}

/// A streaming update paused before scheduling generation after proto checkout.
pub struct StreamingCommit {
    uploads: Vec<((i32, i32, i32), SectionMesh)>,
    removed: Vec<Arc<Chunk>>,
    paused: bool,
}

pub struct Streaming {
    shared: Arc<Shared>,
    /// Columns generated and saved as they were left behind: new when they
    /// next arrive, though read from the save (`World::loaded_chunks`).
    dropped_new: FxHashSet<(i32, i32)>,
    non_air_sections: Arc<dyn Fn(&Proto) -> Option<SectionMask> + Send + Sync>,
    /// `ChunkGenerator::proto_heap_bytes`, for the memory census.
    proto_bytes: Arc<dyn Fn(&Proto) -> ([usize; 6], [usize; 16]) + Send + Sync>,
    /// `ChunkGenerator::cache_census`.
    caches: Arc<dyn Fn() -> Vec<(&'static str, usize, Option<usize>)> + Send + Sync>,
    /// This streamer's place in the shared pool (`horizon_threads::jobs`);
    /// dropping it waits for its running jobs.
    jobs: Registration,
    /// The workers its in-flight work is sized for.
    workers: usize,
    order: u64,
    /// Ticks since the columns and protos that fell out of range were swept.
    scan_age: u8,
    /// The centre and radius the sweep was last reset for.
    scan_area: ((i32, i32), i32),
    generating: FxHashSet<(i32, i32)>,
    generation_scan: GenerationScan,
    /// Proto chunks: `None` while checked out to a decoration or save job.
    protos: FxHashMap<(i32, i32), Option<Proto>>,
    /// The light jobs' views of proto chunks past the area (`light_region`),
    /// built once per proto: each is a neighbour of up to three columns and
    /// waits in failed regions until the last one loads. Dropped whenever
    /// the proto is checked out, replaced or removed.
    light_views: FxHashMap<(i32, i32), Arc<Chunk>>,
    decorated: FxHashSet<(i32, i32)>,
    decorating: usize,
    /// The final chunks a running decoration writes into: another one that
    /// reaches them waits, as vanilla's features run one after the other and
    /// each sees what the one before placed.
    decorating_finals: HashSet<(i32, i32)>,
    /// Save jobs in flight.
    saving: usize,
    /// Proto chunks checked out to a worker for their light view.
    viewing: FxHashSet<(i32, i32)>,
    /// Vanilla's decoration order around the player, and the centre and
    /// radius it was last ranked for.
    decoration_order: DecorationOrder,
    order_area: Option<((i32, i32), i32)>,
    /// What the queued jobs' priorities were computed from (`requeue`): the
    /// centre, the band, the extra areas, and the forced chunks.
    queue_basis: forced_ring::Basis,
    finishing: FxHashSet<(i32, i32)>,
    decorates: bool,
    /// `ChunkGenerator::decorates_beside_final`.
    saves_beside_final: bool,
    lighting: FxHashMap<(i32, i32), u64>,
    meshing: FxHashMap<(i32, i32, i32), u64>,
    /// The sequence each section's latest accepted mesh was dispatched at
    /// (`CompiledSectionMesh.compileTaskStartTimeNs`).
    compile_started: FxHashMap<(i32, i32, i32), u64>,
    /// Chunks whose light is current.
    lit: FxHashSet<(i32, i32)>,
    /// Columns whose sections have been meshed at least once.
    meshed: FxHashSet<(i32, i32)>,
    /// Dirty sections of loaded columns past the render distance, meshed once
    /// the player comes closer, and the area they were set aside for.
    waiting: FxHashSet<(i32, i32, i32)>,
    waiting_area: ((i32, i32), i32, usize),
    /// Proto chunks decoration wrote to since they were generated or saved.
    written: FxHashSet<(i32, i32)>,
    saves: Arc<dyn Saves>,
    /// Results received while `save` waited, for the next `update`.
    deferred: Vec<Done>,
    /// Per-update collection scratch, retained to avoid allocator churn while
    /// the view is moving or many columns are dirty.
    received_scratch: VecDeque<Done>,
    finalize_scratch: Vec<(i32, i32)>,
    light_scratch: Vec<(i64, i32, i32)>,
    gone_scratch: Vec<(i32, i32)>,
    mesh_candidates_scratch: Vec<(i64, i32, i32, SectionMask)>,
    mesh_columns_scratch: Vec<((i32, i32), SectionMask)>,
    sequence: u64,
    pub center: (i32, i32),
    /// The view distance around `center`; below 0, no area there (a dedicated
    /// server nobody is on).
    pub radius: i32,
    /// Chunks kept loaded whatever the distance: the `/forceload`ed ones and
    /// those a mod holds a ticket for. Changed only through `set_forced_area`,
    /// `clear_forced`, `set_mod_forced` and `load_forced`, through `settle`.
    pub forced: HashSet<(i32, i32)>,
    /// The `/forceload`ed chunks, the ones saved in `chunk_tickets.dat`.
    commanded: HashSet<(i32, i32)>,
    mod_tickets: ModTickets,
    /// No work is scheduled while paused (title screen).
    pub paused: bool,
    /// Finished meshes the renderer has not taken yet (the game's pending
    /// uploads): they count against the meshing room as the jobs do.
    pub upload_backlog: usize,
    /// Dimension flags: sky light, Nether face shading.
    pub sky_light: bool,
    pub nether_shading: bool,
    /// A shader pack lights the faces: sections mesh without face shading
    /// (`Mesher::shader_pack_lighting`).
    pub shader_pack_lighting: bool,
    /// OptiFine/Iris separate ambient occlusion: vertex RGB has pure tint and alpha holds AO factor
    /// (`Mesher::separate_ao`).
    pub separate_ao: bool,
    /// A `shaders/` pack draws the terrain: fluid tops split as Iris splits them
    /// (`Mesher::shader_pack`).
    pub shader_pack: bool,
    /// A shader API 0.2 pack reads the terrain's own inputs: quads carry their looks
    /// (`Mesher::looks`, `Renderer::terrain_looks`).
    pub looks: bool,
    /// The ray scene keeps them (`Renderer::ray_looks`): with `looks`, what meshes the world
    /// again when it changes.
    pub ray_looks: bool,
    /// `BiomeManager`'s zoom seed (the hashed world seed) and `biomeBlendRadius`.
    pub biome_zoom: BiomeZoom,
    pub biome_blend: i32,
    /// The `ao` and `cutoutLeaves` options.
    pub smooth_lighting: bool,
    pub cutout_leaves: bool,
    /// Draw regular ice opaque in the solid pass (`Mesher::opaque_ice`).
    pub opaque_ice: bool,
    /// Sprite layers a shader pack draws in the translucent pass (`[[materials]]
    /// translucent`), by layer; empty without one.
    pub translucent_layers: Arc<[bool]>,
    /// Per-state shader-pack model/fluid layer overrides; empty without a pack.
    pub block_layers: Arc<crate::mesher::BlockLayerTable>,
    /// What the mesher does with leaf faces inside a canopy (`LeafCullingMode`).
    pub leaf_culling: LeafCullingMode,
    /// Incremental relighting of block edits on lit chunks (see `light::relight`).
    light_tables: Arc<LightTables>,
    /// Applied edits by sequence, replayed onto light jobs snapshotted before them.
    light_log: Vec<(u64, (i32, i32, i32))>,
    light_edit_sequence: u64,
    /// Edit sequence at each in-flight light job's snapshot.
    light_snapshots: FxHashMap<(i32, i32), u64>,
    pub generated_total: u64,
    pub meshed_total: u64,
    /// Chunks come from a server (`insert_remote`/`forget`): nothing is
    /// generated and columns are only unloaded when the server says so.
    pub remote: bool,
    /// This server sends bulk block edits as whole columns whose light is
    /// from before its relight (a Horizon server): a resent resident column
    /// is lit here. Any other server's packet carries the light vanilla
    /// applies, so it is not lit again (a proxy's next server resends its
    /// whole world at the same coordinates).
    pub relight_resent: bool,
    /// More areas to keep loaded (centre chunk, radius), e.g. around the
    /// players of a server. Each is a square, like vanilla's ticket levels:
    /// chunks finish out to `radius + 1` (Chebyshev) and generation reaches
    /// past that as around the player. They are generated and kept but not meshed.
    pub extra: Vec<((i32, i32), i32)>,
    /// `extra`'s squares as a grid, made again when `extra` changes.
    cover: Option<ExtraCover>,
    /// Set while `update` runs, after it refreshed `cover`: nothing else can
    /// change `extra` meanwhile, so `cover` answers without comparing them.
    cover_current: bool,
    /// The combined `forced` set as each column's distance to the nearest one, made again when
    /// they change; read like `cover`, while `cover_current`.
    forced_cover: Option<ForcedCover>,
    /// Mesh the sections for drawing (false on a server thread).
    pub mesh: bool,
    /// The pregeneration band (`set_band`).
    band: Option<Band>,
    /// The columns and the next row the band's decoration ranks were given
    /// for (`set_band`).
    band_ranked: Option<(i32, i32, i32)>,
    /// Cached face visibility and scratch buffers for synchronous edit meshing (`mesh_immediate`).
    immediate_faces: Faces,
    immediate_scratch: MeshScratch,
    /// Frames left before nearby sections are meshed synchronously again (`NEARBY_COOLDOWN_FRAMES`).
    nearby_cooldown: immediate_mesh::NearbyCooldown,
}
/// Snapshot only after a scan found no missing candidates. Public range fields
/// and `World::chunks` can change without going through Streaming, so compare
/// their values and structural membership token rather than a frame timeout.
///
/// A scan also keeps every candidate it found, nearest first (`plan`): while
/// the ranges stay the same and no column leaves (`World::chunks` removals, or
/// `invalidate` for proto chunks dropped), no column becomes wanted, so the
/// next batches are the plan's entries still missing. A full scan looks at
/// every column of the areas (thousands at render distance 32), and one ran
/// whenever a worker finished: it took a quarter of the thread that runs
/// `Streaming::update` while a world loaded.
#[derive(Default)]
struct GenerationScan {
    exhausted: bool,
    center: (i32, i32),
    radius: i32,
    extra: Vec<((i32, i32), i32)>,
    forced: HashSet<(i32, i32)>,
    band: Option<Band>,
    chunk_revision: u64,
    plan: Vec<(i64, i32, i32)>,
    /// Entries before it are no longer missing.
    next: usize,
    /// What `plan` was made for: the ranges and decoration; the chunk
    /// map's revision is `chunk_revision`.
    plan_basis: Option<bool>,
}
impl GenerationScan {
    fn same_ranges(&self, ranges: &Ranges<'_>) -> bool {
        self.center == ranges.center
            && self.radius == ranges.radius
            && self.extra == ranges.extra
            && self.forced == *ranges.forced
            && self.band.as_ref() == ranges.band
    }
    fn unchanged(&self, ranges: &Ranges<'_>, world: &World) -> bool {
        self.exhausted
            && self.same_ranges(ranges)
            && self.chunk_revision == world.chunks.membership_revision()
    }
    /// A column may be wanted again (a proto chunk was dropped): the next
    /// call scans the areas again.
    fn invalidate(&mut self) {
        self.exhausted = false;
        self.plan_basis = None;
    }
    /// Whether `plan` still holds every candidate: same ranges, and no
    /// column left the chunk map since it was made.
    fn plan_holds(&mut self, input: &GenerationInputs<'_>) -> bool {
        if self.plan_basis != Some(input.decorates) || !self.same_ranges(&input.ranges) {
            return false;
        }
        let revision = input.world.chunks.membership_revision();
        if revision != self.chunk_revision {
            let Some(mut changes) = input.world.chunks.deltas_since(self.chunk_revision) else {
                return false;
            };
            if !changes.all(|(_, change)| matches!(change, ChunkMembershipDelta::Added(_))) {
                return false;
            }
            self.chunk_revision = revision;
        }
        true
    }
    fn remember(&mut self, ranges: Ranges<'_>, world: &World) {
        self.exhausted = true;
        self.center = ranges.center;
        self.radius = ranges.radius;
        self.extra.clear();
        self.extra.extend_from_slice(ranges.extra);
        self.forced.clone_from(ranges.forced);
        self.band = ranges.band.copied();
        self.chunk_revision = world.chunks.membership_revision();
    }
}
struct GenerationInputs<'a> {
    ranges: Ranges<'a>,
    world: &'a World,
    generating: &'a FxHashSet<(i32, i32)>,
    protos: &'a FxHashMap<(i32, i32), Option<Proto>>,
    finishing: &'a FxHashSet<(i32, i32)>,
    decorates: bool,
    remote: bool,
}
impl GenerationScan {
    /// None means the scan was skipped; Some preserves the original candidate
    /// ordering and bounded batch, including an empty exhausted scan.
    fn candidates(
        &mut self,
        input: GenerationInputs<'_>,
        capacity: usize,
    ) -> Option<Vec<(i64, i32, i32)>> {
        // Generation (load or proto), closest first, bounded in flight.
        let (margin, reach) = Ranges::margins(input.decorates);
        if input.remote
            || input.generating.len() >= capacity
            || self.unchanged(&input.ranges, input.world)
        {
            return None;
        }
        let missing = |&(_, x, z): &(i64, i32, i32)| {
            !input.world.chunks.contains_key(&(x, z))
                && !input.generating.contains(&(x, z))
                && !input.protos.contains_key(&(x, z))
                && !input.finishing.contains(&(x, z))
        };
        if self.plan_holds(&input) {
            while self.next < self.plan.len() && !missing(&self.plan[self.next]) {
                self.next += 1;
            }
            let room = capacity - input.generating.len();
            let batch: Vec<_> = self.plan[self.next..]
                .iter()
                .filter(|c| missing(c))
                .take(room)
                .copied()
                .collect();
            // Past the plan's end, the full scan below decides whether
            // anything is left (and remembers when nothing is).
            if !batch.is_empty() {
                self.exhausted = false;
                return Some(batch);
            }
        }
        {
            let mut wanted = Vec::new();
            // Each area's square of columns, as one run per row. Players'
            // areas overlap (a server's players often stand together), so
            // the runs of a row are merged and each column is looked at once.
            let mut runs: Vec<(i32, i32, i32)> = Vec::new();
            for (center, radius) in std::iter::once((input.ranges.center, input.ranges.radius))
                .chain(input.ranges.extra.iter().copied())
            {
                let r = radius + margin.max(reach + 1);
                runs.extend((center.1 - r..=center.1 + r).map(|z| (z, center.0 - r, center.0 + r)));
            }
            if let Some(band) = input.ranges.band {
                let r = margin.max(reach + 1);
                runs.extend((band.z0 - r..=band.z1 + r).map(|z| (z, band.x0 - r, band.x1 + r)));
            }
            runs.sort_unstable();
            let mut runs = runs.into_iter().peekable();
            while let Some((z, x0, mut x1)) = runs.next() {
                while let Some((_, _, next_x1)) =
                    runs.next_if(|&(nz, nx0, _)| nz == z && nx0 <= x1.saturating_add(1))
                {
                    x1 = x1.max(next_x1);
                }
                for x in x0..=x1 {
                    if !input.ranges.requested(x, z, input.decorates)
                        || input.world.chunks.contains_key(&(x, z))
                        || input.generating.contains(&(x, z))
                        || input.protos.contains_key(&(x, z))
                        || input.finishing.contains(&(x, z))
                    {
                        continue;
                    }
                    wanted.push((input.ranges.priority(x, z), x, z));
                }
            }
            // Forced chunks and their generation neighbourhood: the ring that
            // is decorated (or finished, without decoration) and the one its
            // 3x3 needs.
            let reach = if input.decorates {
                FORCED_TERRAIN_RING
            } else {
                FORCED_FEATURES_RING
            };
            for &(fx, fz) in input.ranges.forced {
                for dz in -reach..=reach {
                    for dx in -reach..=reach {
                        let (x, z) = (fx + dx, fz + dz);
                        if input.world.chunks.contains_key(&(x, z))
                            || input.generating.contains(&(x, z))
                            || input.protos.contains_key(&(x, z))
                            || input.finishing.contains(&(x, z))
                        {
                            continue;
                        }
                        wanted.push((input.ranges.priority(x, z), x, z));
                    }
                }
            }
            let ranges = Ranges {
                center: input.ranges.center,
                radius: input.ranges.radius,
                extra: input.ranges.extra,
                forced: input.ranges.forced,
                cover: input.ranges.cover,
                forced_cover: None,
                band: input.ranges.band,
            };
            self.remember(ranges, input.world);
            if wanted.is_empty() {
                self.plan.clear();
                self.plan_basis = None;
            } else {
                // A bounded batch may leave candidates behind: the next calls
                // take them from the plan as capacity opens.
                self.exhausted = false;
                self.plan_basis = Some(input.decorates);
            }
            wanted.sort_unstable();
            wanted.dedup();
            self.plan.clear();
            self.plan.extend_from_slice(&wanted);
            self.next = 0;
            wanted.truncate(capacity - input.generating.len());
            Some(wanted)
        }
    }
}
/// What a pool worker keeps for one streamer between its jobs: the
/// generator's worker, and the face visibility table and its occlusion
/// cache (like vanilla's per-thread cache: cloning it per job copied every
/// state and face shape) and the mesher's scratch.
struct WorkerLocal<W> {
    worker: W,
    faces: Faces,
    mesh_scratch: MeshScratch,
}
/// A streamer's jobs, run by the process's shared pool (`horizon_threads::jobs`).
struct Jobs<G: ChunkGenerator> {
    shared: Arc<Shared>,
    generator: Arc<G>,
    light: Arc<LightTables>,
    locals: PerWorker<WorkerLocal<G::Worker>>,
}
impl<G: ChunkGenerator> Source for Jobs<G> {
    fn next(&self) -> Option<Rank> {
        let queue = self.shared.queue.lock().unwrap();
        if self
            .shared
            .aborted
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            for q in queue.iter() {
                if matches!(q.job, Job::Save(..)) {
                    return Some(Rank::new(Class::Urgent, q.priority));
                }
            }
            return None;
        }
        queue.peek().map(|q| {
            // Jobs without a column (saves, meshes to free) go first.
            let class = if q.at.is_none() {
                Class::Urgent
            } else {
                Class::Near
            };
            Rank::new(class, q.priority)
        })
    }
    fn run(&self, worker: usize) -> bool {
        let Some(job) = self.shared.queue.lock().unwrap().pop() else {
            return false;
        };
        if self
            .shared
            .aborted
            .load(std::sync::atomic::Ordering::Relaxed)
            && !matches!(job.job, Job::Save(..))
        {
            return true;
        }
        let started = job_stats::enabled().then(|| {
            let kind = match &job.job {
                Job::Generate(..) => 1,
                Job::Decorate(..) => 2,
                Job::Finish(..) => 3,
                Job::Light(..) => 4,
                Job::Mesh(..) => 5,
                _ => 6,
            };
            // The other jobs (saves, light views, freed meshes) are many and short: counted, not
            // timed, so the clock reads don't cost more than they do.
            let cpu = if kind == 6 {
                std::time::Duration::ZERO
            } else {
                crate::spans::thread_cpu().unwrap_or_default()
            };
            (kind, cpu)
        });
        let _tag = horizon_threads::heap_tag(match &job.job {
            Job::Generate(..) => "stream.generate",
            Job::Decorate(..) => "stream.decorate",
            Job::Finish(..) => "stream.finish",
            Job::Light(..) => "stream.light",
            Job::Mesh(..) => "stream.mesh",
            _ => "stream.other",
        });
        let done = self.locals.with(
            worker,
            || {
                let _tag = horizon_threads::heap_tag("stream.worker_state");
                WorkerLocal {
                    worker: self.generator.worker(),
                    faces: None,
                    mesh_scratch: MeshScratch::default(),
                }
            },
            |local| self.run_job(job.job, local),
        );
        if let Some((kind, started)) = started {
            // A generation job that found the column saved loaded it.
            let kind = if kind == 1 && matches!(done, Some(Done::Generated(_))) {
                0
            } else {
                kind
            };
            let nanos = if kind == 6 {
                0
            } else {
                let cpu = crate::spans::thread_cpu().unwrap_or_default();
                cpu.saturating_sub(started).as_nanos() as u64
            };
            job_stats::add(kind, nanos);
        }
        if let Some(done) = done {
            self.shared.done.send(done);
        }
        true
    }
    fn idle(&self, worker: usize) {
        // A server nobody is on, a client standing still: free the
        // generator's buffers (and the mesher's face cache and scratch)
        // until the next job needs them.
        self.locals.with_existing(worker, |local| {
            self.generator.idle(&mut local.worker);
            local.faces = None;
            local.mesh_scratch = MeshScratch::default();
        });
    }
    fn all_idle(&self) {
        // No worker needs its kept meshes or light scratch: freed outside the pool lock.
        let meshes = std::mem::take(&mut *self.shared.meshes.lock().unwrap());
        drop(meshes);
        crate::light::release_scratch();
        self.generator.all_idle();
    }
}
impl<G: ChunkGenerator> Jobs<G> {
    fn run_job(&self, job: Job, local: &mut WorkerLocal<G::Worker>) -> Option<Done> {
        let (generator, shared, light) = (&self.generator, &self.shared, &self.light);
        let WorkerLocal {
            worker,
            faces,
            mesh_scratch,
        } = local;
        let done = match job {
            Job::Generate(x, z) => match generator.load(x, z) {
                Some(chunk) => {
                    generator.prepare_loaded(&mut *worker, &chunk);
                    Done::Generated(chunk)
                }
                None => {
                    let proto = generator.proto(&mut *worker, x, z);
                    let decorated = generator.is_decorated(&proto);
                    Done::Proto(x, z, proto, decorated)
                }
            },
            Job::Decorate(x, z, mut region, finals) => {
                // The final chunks' proto chunks, made here: a
                // generated proto chunk holding their blocks.
                for (i, snapshot) in &finals {
                    if let Some(proto) = generator.proto_beside_final(&mut *worker, snapshot) {
                        region[*i] = proto;
                    }
                }
                generator.decorate(&mut *worker, &mut region);
                let mut writes = FinalWrites::default();
                let blocks: [Vec<_>; 9] = std::array::from_fn(|n| {
                    finals.get(n).map_or_else(Vec::new, |(i, snapshot)| {
                        generator.final_writes(snapshot, &region[*i])
                    })
                });
                let entries: Vec<_> = finals
                    .iter()
                    .zip(&blocks)
                    .map(|((i, _), blocks)| (&region[*i], blocks.as_slice()))
                    .collect();
                writes.block_entities = generator.final_block_entities(&entries);
                writes.blocks.extend(blocks.into_iter().flatten());
                for (i, _) in &finals {
                    region[*i] = Box::new(());
                }
                let finals = finals.into_iter().map(|(i, _)| i).collect();
                Done::Decorated(x, z, region, finals, writes)
            }
            Job::Finish(proto) => {
                let mut chunk = generator.finish(proto);
                chunk.generated = true;
                Done::Generated(chunk)
            }
            Job::Save(protos) => {
                let refs: Vec<&Proto> = protos.iter().map(|p| &p.proto).collect();
                generator.save_protos(&refs);
                Done::Saved(protos.into_iter().map(|p| (p.pos, p.proto)).collect())
            }
            Job::LightView(x, z, proto) => {
                let view = generator.light_view(&proto);
                Done::LightView(x, z, proto, view)
            }
            Job::Free(meshes) => {
                for (_, mesh) in meshes {
                    let rejected = shared.meshes.lock().unwrap().put(mesh);
                    drop(rejected);
                }
                return None;
            }
            Job::Light(x, z, region, seq, sky) => {
                Done::Lit(x, z, light_chunk(&borrow(&region), light, sky), seq)
            }
            Job::Mesh(x, sy, z, region, seq, settings) => {
                let context = shared.context.read().unwrap().clone();
                let recycled = shared.meshes.lock().unwrap().take();
                let mesh = mesh_section(
                    &context,
                    &settings,
                    faces,
                    mesh_scratch,
                    &region,
                    sy,
                    recycled,
                );
                Done::Meshed(x, sy, z, mesh, seq)
            }
        };
        Some(done)
    }
}
pub struct WorkerContext {
    pub assets: Arc<crate::assets::Assets>,
    pub render: Arc<crate::assets::RenderAssets>,
    pub tables: Arc<MeshTables>,
    pub biomes: Arc<crate::biome::Biomes>,
    pub light: Arc<LightTables>,
    /// The block states a mod hides from the mesher, by state id (`Mesher::hidden_states`); empty
    /// for none.
    pub hidden_states: Arc<[bool]>,
    /// The client `BlockPart` answers the mesher reads (`Mesher::part_tables`); empty for none.
    pub part_tables: Arc<crate::mesher::part_tables::PartTables>,
}
mod bounded;
mod dynamic_emission;
mod queue_census;
pub use queue_census::QueueCensus;
mod forced_cover;
mod forced_ring;
pub mod immediate_mesh;
mod mesh_job;
mod relight_now;
mod viewed_meshing;
use bounded::nearest_first;
use dynamic_emission::light_region_ready;
use forced_cover::ForcedCover;
pub use immediate_mesh::IMMEDIATE_MESH_LIMIT;
use mesh_job::{Faces, mesh_section};
pub use viewed_meshing::{
    MAX_VIEWED_SECTIONS, MeshSource, VIEWED_LINGER, VIEWED_MESH_WORKERS, VIEWED_UPLOADS_PER_FRAME,
    ViewedLevel, ViewedMeshing,
};
impl Streaming {
    pub fn new<G: ChunkGenerator>(
        generator: Arc<G>,
        context: Arc<WorkerContext>,
        threads: usize,
    ) -> Self {
        let shared = Arc::new(Shared {
            queue: Mutex::new(BinaryHeap::new()),
            context: RwLock::new(context.clone()),
            done: Mailbox::default(),
            meshes: Mutex::new(MeshPool::default()),
            aborted: std::sync::atomic::AtomicBool::new(false),
        });
        let light_tables = context.light.clone();
        let saves: Arc<dyn Saves> = generator.clone();
        let pool = horizon_threads::jobs::pool();
        let source = Arc::new(Jobs {
            shared: shared.clone(),
            generator: generator.clone(),
            light: context.light.clone(),
            locals: horizon_threads::jobs::PerWorker::new(pool.threads()),
        });
        let jobs = pool.register(source);
        // The work kept in flight is sized for the workers this streamer may
        // use: the pool's, or fewer when asked (tests, a placeholder).
        let workers = threads.clamp(1, pool.threads());
        let non_air_sections = {
            let generator = generator.clone();
            Arc::new(move |p: &Proto| generator.non_air_sections(p))
        };
        let proto_bytes = {
            let generator = generator.clone();
            Arc::new(move |p: &Proto| generator.proto_heap_bytes(p))
        };
        let caches = {
            let generator = generator.clone();
            Arc::new(move || generator.cache_census())
        };
        Self {
            shared,
            non_air_sections,
            proto_bytes,
            caches,
            jobs,
            workers,
            order: 0,
            scan_age: 0,
            scan_area: ((i32::MIN, i32::MIN), i32::MIN),
            generating: Default::default(),
            generation_scan: GenerationScan::default(),
            protos: Default::default(),
            light_views: Default::default(),
            decorated: Default::default(),
            dropped_new: Default::default(),
            decorating: 0,
            decorating_finals: HashSet::new(),
            saving: 0,
            viewing: Default::default(),
            decoration_order: DecorationOrder::default(),
            order_area: None,
            queue_basis: Default::default(),
            finishing: Default::default(),
            decorates: generator.decorates(),
            saves_beside_final: generator.decorates_beside_final(),
            lighting: Default::default(),
            meshing: Default::default(),
            compile_started: Default::default(),
            lit: Default::default(),
            meshed: Default::default(),
            waiting: Default::default(),
            waiting_area: ((0, 0), 0, 0),
            written: Default::default(),
            saves,
            deferred: Vec::new(),
            received_scratch: VecDeque::new(),
            finalize_scratch: Vec::new(),
            light_scratch: Vec::new(),
            gone_scratch: Vec::new(),
            mesh_candidates_scratch: Vec::new(),
            mesh_columns_scratch: Vec::new(),
            sequence: 0,
            center: (0, 0),
            radius: 8,
            forced: HashSet::new(),
            commanded: HashSet::new(),
            mod_tickets: ModTickets::default(),
            paused: false,
            upload_backlog: 0,
            sky_light: true,
            nether_shading: false,
            shader_pack_lighting: false,
            separate_ao: false,
            shader_pack: false,
            looks: false,
            ray_looks: false,
            biome_zoom: BiomeZoom::new(0),
            biome_blend: 2,
            smooth_lighting: true,
            cutout_leaves: true,
            opaque_ice: false,
            translucent_layers: Arc::from([]),
            block_layers: Arc::default(),
            leaf_culling: LeafCullingMode::Off,
            light_tables,
            light_log: Vec::new(),
            light_edit_sequence: 0,
            light_snapshots: Default::default(),
            generated_total: 0,
            meshed_total: 0,
            remote: false,
            relight_resent: false,
            extra: Vec::new(),
            cover: None,
            cover_current: false,
            forced_cover: None,
            mesh: true,
            band: None,
            band_ranked: None,
            immediate_faces: None,
            immediate_scratch: MeshScratch::default(),
            nearby_cooldown: Default::default(),
        }
    }
    /// Shares generation resources, but gives distant work its own worker state.
    pub fn lod_source(&self) -> Arc<dyn LodSource> {
        self.saves.clone()
    }
    /// Resource packs reloaded (`LevelExtractor.allChanged`): mesh with the new
    /// models and sprites, and rebuild every loaded section. Meshes in flight
    /// were made with the old resources; their results are dropped.
    pub fn reload_resources(&mut self, world: &mut World, context: Arc<WorkerContext>) {
        *self.shared.context.write().unwrap() = context;
        self.reload_sections(world);
    }
    /// `LevelExtractor.allChanged`: discard mesh work made before the reload and
    /// rebuild every loaded, lit section.
    pub fn reload_sections(&mut self, world: &mut World) {
        self.shared
            .queue
            .lock()
            .unwrap()
            .retain(|job| !matches!(job.job, Job::Mesh(..)));
        self.meshing.clear();
        self.compile_started.clear();
        Self::mark_loaded_sections_dirty(world, &self.lit);
    }
    /// The resident sections `LevelExtractor.allChanged` queues again. An
    /// unlit column will queue itself when its light job completes.
    fn mark_loaded_sections_dirty(world: &mut World, lit: &FxHashSet<(i32, i32)>) {
        let mask = SectionMask::below(world.layout().sections());
        for &(x, z) in world.chunks.keys() {
            if !lit.contains(&(x, z)) {
                continue;
            }
            world.dirty_sections.insert_column_mask((x, z), mask);
        }
    }
    /// A column received from a server, with the light it sent
    /// (`ClientPacketListener.handleLevelChunkWithLight`): nothing is
    /// relit, and it and its neighbours remesh (`enableChunkLight`).
    pub fn insert_remote(&mut self, world: &mut World, chunk: Chunk) {
        let key = (chunk.x, chunk.z);
        self.lighting.remove(&key);
        self.light_snapshots.remove(&key);
        world.chunks.insert(key, Arc::new(chunk));
        self.lit.insert(key);
        self.generated_total += 1;
        let mask = SectionMask::below(world.layout().sections());
        for dz in -1..=1 {
            for dx in -1..=1 {
                let n = (key.0 + dx, key.1 + dz);
                if n == key || world.chunks.contains_key(&n) {
                    world.dirty_sections.insert_column_mask(n, mask);
                }
            }
        }
    }
    /// A resident column a server sent again whole (it changed it in bulk):
    /// its new blocks taken, the sections that did not change kept as they
    /// were, and (for a Horizon server's bulk edit) its light and its
    /// neighbours' worked out again here, off the game thread, as light is for
    /// the blocks a server changes one at a time; the sections that changed
    /// remesh (with theirs) once lit. Until then the sections that changed
    /// show the light the packet carried (a Horizon server sends the light
    /// from before its relight, then the new light; another server's is already
    /// right, which matters where the neighbours this light needs aren't all
    /// here).
    pub fn replace_remote(&mut self, world: &mut World, chunk: Chunk) {
        let key = (chunk.x, chunk.z);
        let Some(old) = world.chunks.get(&key).cloned() else {
            return self.insert_remote(world, chunk);
        };
        if !self.lit.contains(&key) {
            return self.insert_remote(world, chunk);
        }
        take_resent_column(world, key, &old, chunk, self.relight_resent);
    }
    /// A server's light update for a resident column: the sections whose
    /// light changed and their neighbours remesh
    /// (`setSectionDirtyWithNeighbors`). A resent column's sections with new
    /// biomes remesh the same way.
    pub fn light_updated(world: &mut World, x: i32, z: i32, sections: &[usize]) {
        for &sy in sections {
            for dz in -1..=1 {
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let s = sy as i32 + dy;
                        if (0..world.layout().sections() as i32).contains(&s) {
                            world.dirty_sections.insert((x + dx, s, z + dz));
                        }
                    }
                }
            }
        }
    }
    /// Give back meshes the renderer has copied: a worker frees them, ahead of
    /// other jobs, as vanilla's compile task closes its mesh data on its own
    /// thread once copied (`SectionRenderDispatcher.RenderSection.CompileTask.doTask`).
    /// Freed in the frame, the hundreds of meshes a chunk border brings waited
    /// on the allocator behind every busy worker: up to 20 ms of a frame each
    /// time while flying at render distance 32.
    pub fn free(&mut self, meshes: Vec<((i32, i32, i32), SectionMesh)>) {
        if !meshes.is_empty() {
            self.submit(None, 0, Job::Free(meshes));
        }
    }
    /// Remove resident columns before clearing their streaming and level state.
    pub fn take_remote_columns(world: &mut World, gone: &FxHashSet<(i32, i32)>) -> Vec<Arc<Chunk>> {
        let mut chunks = Vec::with_capacity(gone.len());
        for key in gone {
            chunks.extend(world.chunks.remove(key));
        }
        chunks
    }

    /// Clear streaming state after resident remote columns have been removed.
    pub fn forget_columns(&mut self, world: &mut World, gone: &FxHashSet<(i32, i32)>) {
        // Every column held (a far teleport): no pending section is left to keep, and a
        // pass over them all (80,000 at render distance 32) took 1.7 ms of the frame.
        if world.chunks.is_empty() {
            self.meshing.clear();
            self.compile_started.clear();
            self.waiting.clear();
        } else {
            forget_pending_sections(
                &mut self.meshing,
                &mut self.compile_started,
                &mut self.waiting,
                gone,
                world.layout().sections() as i32,
            );
        }
        // `ChunkTaskPriorityQueue.release` for each of them (`ChunkTaskDispatcher`'s
        // `release(col, true)` from the chunk's unload): the jobs still queued for a
        // column go with it instead of running for a level they no longer belong to.
        if !gone.is_empty()
            && let Ok(mut queue) = self.shared.queue.lock()
        {
            release_columns(&mut queue, gone);
        }
        for &(x, z) in gone {
            self.lit.remove(&(x, z));
            self.meshed.remove(&(x, z));
            self.lighting.remove(&(x, z));
            world.dirty_sections.take_column(x, z);
            world.entities_pending.remove(&(x, z));
        }
    }
    /// Queue a job for the column `at`, nearest first: its distance (`distance`)
    /// times four plus `bias`, which orders a column's jobs. A job without a
    /// column (a save, meshes to free) goes ahead of every other.
    fn submit(&mut self, at: Option<(i32, i32)>, bias: i64, job: Job) {
        self.order += 1;
        let priority = at.map_or(i64::MIN, |(x, z)| self.distance(x, z) * 4 + bias);
        self.shared.queue.lock().unwrap().push(Queued {
            priority,
            at,
            bias,
            order: self.order,
            job,
        });
        self.jobs.notify();
    }
    /// Rank the queued jobs again when what their priorities were computed
    /// from changed: the centre, the extra areas, the band, or the loading
    /// screen (ring order, `Ranges::rings`, whose values are far above the
    /// distances used after it). Vanilla's chunk task queue follows the
    /// current ticket levels in the same way. Without it, generation queued
    /// while the loading screen was up waited behind every job queued after
    /// it, and with it the decoration of the chunks around the player: at
    /// render distance 32 nothing was decorated until every proto chunk had
    /// been generated.
    fn requeue(&mut self) {
        if !forced_ring::basis_changed(self) {
            return;
        }
        let mut queue = self.shared.queue.lock().unwrap();
        rank_again(&mut queue, |x, z| self.distance(x, z));
    }
    fn receive_decorated(
        &mut self,
        x: i32,
        z: i32,
        region: [Proto; 9],
        finals: Vec<usize>,
        writes: FinalWrites,
        world: &mut World,
    ) {
        self.generation_scan.exhausted = false;
        self.decorating -= 1;
        for i in &finals {
            self.decorating_finals
                .remove(&(x + (i % 3) as i32 - 1, z + (i / 3) as i32 - 1));
        }
        self.decorated.insert((x, z));
        for (i, proto) in region.into_iter().enumerate() {
            // A final chunk keeps its own chunk: only what decoration wrote
            // into it goes there.
            if finals.contains(&i) {
                continue;
            }
            let key = (x + (i % 3) as i32 - 1, z + (i / 3) as i32 - 1);
            self.light_views.remove(&key);
            self.protos.insert(key, Some(proto));
            self.written.insert(key);
        }
        for ([bx, by, bz], state) in writes.blocks {
            let _ = world.set(bx, by, bz, horizon_world::registry::BlockStateId(state));
        }
        // After the blocks: the write of a block with a block entity made a
        // default one, which the generated one replaces.
        for (pos, entity) in writes.block_entities {
            if world.chunks.contains_key(&(pos.0 >> 4, pos.2 >> 4)) {
                world.block_entities.insert(pos, entity);
            }
        }
    }
    /// Saved proto chunks come back; those now out of range are dropped by
    /// the next `update`.
    fn receive_saved(&mut self, protos: Vec<((i32, i32), Proto)>) {
        self.generation_scan.exhausted = false;
        self.saving -= 1;
        for (key, proto) in protos {
            self.light_views.remove(&key);
            self.protos.insert(key, Some(proto));
        }
    }
    /// A read-only view of checked-out protos independent of the streaming borrow.
    pub fn proto_access(&self) -> Arc<dyn ProtoAccess> {
        self.saves.clone()
    }
    /// Check out proto chunks without submitting them yet.
    fn checkout_protos(&mut self, keys: impl IntoIterator<Item = (i32, i32)>) -> Vec<ProtoSave> {
        checkout_protos(&mut self.protos, &mut self.light_views, keys)
    }
    /// Queue checked-out protos ahead of other generation work.
    pub fn submit_protos(&mut self, protos: Vec<ProtoSave>) -> bool {
        if !protos.is_empty() {
            self.saving += 1;
            self.submit(None, 0, Job::Save(protos));
        }
        self.saving == 0
    }
    fn receive_generated(&mut self, mut chunk: Chunk, world: &mut World) -> Option<Chunk> {
        self.generation_scan.exhausted = false;
        let key = (chunk.x, chunk.z);
        self.generating.remove(&key);
        self.finishing.remove(&key);
        // A column that finished after the player moved away is dropped
        // whole: its block entities and saved entities too, or they would
        // stay in the world (and the entities spawn again, duplicated, when
        // the column loads later). A generated one is saved first: its
        // neighbours were decorated around it, so generating it again would
        // not give it back.
        if !self.ranges().chunk_kept(key.0, key.1, self.decorates) {
            if !self.protos.contains_key(&key) && !world.chunks.contains_key(&key) {
                self.decorated.remove(&key);
            }
            if chunk.fresh {
                self.dropped_new.insert(key);
                return Some(chunk);
            }
            return None;
        }
        self.decorated.insert(key);
        // Generated columns are saved like edited ones, as vanilla saves
        // every chunk it generates.
        if chunk.fresh {
            world.modified.insert(key);
        }
        world.install_chunk_block_entities(&mut chunk);
        world
            .block_names
            .extend(std::mem::take(&mut chunk.block_names));
        let entities = std::mem::take(&mut chunk.entities);
        world.entities_pending.insert((chunk.x, chunk.z));
        if !entities.is_empty() {
            world.queue_entity_nbt((chunk.x, chunk.z), entities);
        }
        self.generated_total += 1;
        // A chunk saved with its light needs no light job; its unlit
        // neighbours still light themselves from it.
        let loaded = chunk.light_loaded;
        let new = self.dropped_new.remove(&key) || chunk.generated;
        world.loaded_chunks.push((key.0, key.1, new));
        chunk.dynamic_pending = self.light_tables.holds_dynamic(&chunk);
        world.chunks.insert(key, Arc::new(chunk));
        if loaded {
            self.lit.insert(key);
            for sy in 0..world.layout().sections() as i32 {
                world.dirty_sections.insert((key.0, sy, key.1));
            }
        }
        for dz in -1..=1 {
            for dx in -1..=1 {
                let n = (key.0 + dx, key.1 + dz);
                if !loaded || world.chunks.get(&n).is_some_and(|c| !c.light_loaded) {
                    world.dirty_light.insert(n);
                }
            }
        }
        None
    }
    /// Finish a generated column's deferred save after its snapshot callback.
    pub fn save_finished(&self, chunk: Chunk) {
        self.saves.save_finished(chunk);
    }
    /// The order jobs around the player run in, lowest first.
    fn distance(&self, x: i32, z: i32) -> i64 {
        self.ranges().priority(x, z)
    }
    /// The 3x3 a light job reads: the finished columns, and for a neighbour
    /// that will not finish (past the area), its decorated proto chunk, as
    /// vanilla lights its last FULL ring against neighbours at FEATURES.
    /// A proto neighbour without its view yet is checked out to a worker,
    /// which builds the view off the server thread (`Job::LightView`); the
    /// region is asked for again once it is back.
    fn light_region(&mut self, world: &World, x: i32, z: i32) -> Option<Box<Region>> {
        let mut region: Region = Default::default();
        let mut complete = true;
        for dz in 0..3 {
            for dx in 0..3 {
                let n = (x + dx as i32 - 1, z + dz as i32 - 1);
                region[dz][dx] = match world.chunks.get(&n) {
                    Some(chunk) => Some(chunk.clone()),
                    None if !self.in_range(n.0, n.1, 1) && self.decorated.contains(&n) => {
                        if let Some(view) = self.light_views.get(&n) {
                            Some(view.clone())
                        } else {
                            let proto = self.protos.get_mut(&n)?.take()?;
                            self.viewing.insert(n);
                            self.submit(Some((x, z)), 0, Job::LightView(n.0, n.1, proto));
                            complete = false;
                            None
                        }
                    }
                    None => return None,
                };
            }
        }
        complete.then(|| Box::new(region))
    }
    /// A proto chunk back from its light view: the view waits in
    /// `light_views` for the light jobs that asked for it.
    fn receive_light_view(&mut self, x: i32, z: i32, proto: Proto, view: Option<Chunk>) {
        self.viewing.remove(&(x, z));
        self.protos.insert((x, z), Some(proto));
        if let Some(view) = view {
            self.light_views.insert((x, z), Arc::new(view));
        }
    }
    /// Whether `light_region` would find all nine columns: the same walk
    /// without building the snapshot, so the scheduler can rank the ready
    /// columns before any region is cloned. A proto neighbour whose light
    /// view turns out empty is skipped when its region is built, and asked
    /// for again, as `light_region` returning `None` always was.
    fn light_region_ready(&self, world: &World, x: i32, z: i32) -> bool {
        light_region_ready(
            world,
            |x, z| self.in_range(x, z, 1),
            &self.decorated,
            &self.protos,
            x,
            z,
        )
    }
    fn region(world: &World, x: i32, z: i32) -> Option<Arc<Region>> {
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
        Some(Arc::new(region))
    }
    pub fn is_lit(&self, x: i32, z: i32) -> bool {
        self.lit.contains(&(x, z))
    }
    /// The non-air sections of a column that is a decorated proto chunk
    /// here (vanilla's INITIALIZE_LIGHT and on), if it is one: a level
    /// chunk's light layers count them (#4294).
    pub fn proto_sections(&self, x: i32, z: i32) -> Option<SectionMask> {
        if !self.decorated.contains(&(x, z)) {
            return None;
        }
        (self.non_air_sections)(self.protos.get(&(x, z))?.as_ref()?)
    }
    /// `ChunkMap.getLatestStatus` for the loading screen's chunk map: the
    /// furthest step this column has reached, `None` before it is asked for.
    pub fn latest_status(
        &self,
        world: &World,
        x: i32,
        z: i32,
    ) -> Option<crate::level_loading::Status> {
        use crate::level_loading::Status;
        let key = (x, z);
        Some(if self.lit.contains(&key) {
            Status::Full
        } else if world.chunks.contains_key(&key) {
            Status::InitializeLight
        } else if self.decorated.contains(&key) || self.finishing.contains(&key) {
            Status::Features
        } else if self.protos.contains_key(&key) {
            Status::Terrain
        } else if self.generating.contains(&key) {
            Status::Empty
        } else {
            return None;
        })
    }
    /// The sequence the next mesh job will be dispatched after: a clock for
    /// `compile_start`, whose jobs all snapshot the world as they are
    /// dispatched (`Util.getNanos()` at the start of a compile task).
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
    /// When the section's latest accepted mesh started compiling, on the
    /// `sequence` clock (`CompiledSectionMesh.getCompileTaskStartTime`).
    pub fn compile_start(&self, section: (i32, i32, i32)) -> Option<u64> {
        self.compile_started.get(&section).copied()
    }
    pub fn is_meshing(&self, section: (i32, i32, i32)) -> bool {
        self.meshing.contains_key(&section)
    }
    pub fn pending(&self) -> usize {
        self.shared.queue.lock().unwrap().len()
    }
    /// Whether any chunk is queued or on its way from generation to a mesh.
    pub fn busy(&self) -> bool {
        self.chunks_in_flight() || !self.meshing.is_empty() || self.pending() != 0
    }
    /// Whether the near terrain is still coming in: chunks generating,
    /// decorating, finishing or being lit (or their neighbours' light views),
    /// or sections meshing for the first time. Each of those is counted from
    /// when its job is queued, so queued jobs need no separate count.
    /// Remeshing sections already drawn (block, light and fluid updates) is
    /// not loading: it kept small machines' far terrain held back
    /// (`Lod::set_near_loading`) for as long as anything nearby changed.
    pub fn terrain_loading(&self) -> bool {
        self.chunks_in_flight()
            || self
                .meshing
                .keys()
                .any(|section| !self.compile_started.contains_key(section))
    }
    fn chunks_in_flight(&self) -> bool {
        !(self.generating.is_empty()
            && self.decorating == 0
            && self.viewing.is_empty()
            && self.finishing.is_empty()
            && self.lighting.is_empty())
    }
    /// The proto chunks held (not checked out): how many, their heap bytes by
    /// part and their sections by bits per block; and the light views of
    /// them with their bytes. For `horizon-ctl memory`.
    pub fn proto_census(&self) -> ProtoCensus {
        let mut census = ProtoCensus::default();
        for proto in self.protos.values().flatten() {
            let (bytes, bits) = (self.proto_bytes)(proto);
            census.protos += 1;
            for (sum, b) in census.bytes.iter_mut().zip(bytes) {
                *sum += b;
            }
            for (sum, b) in census.sections_by_bits.iter_mut().zip(bits) {
                *sum += b;
            }
        }
        census.light_views = self.light_views.len();
        census.light_view_bytes = self.light_views.values().map(|v| v.heap_bytes()).sum();
        census.caches = (self.caches)();
        census
    }
    /// How much work each stage holds: queued jobs, then what is in flight or
    /// waiting at each step from generation to meshing.
    pub fn stages(&self) -> [(&'static str, usize); 7] {
        [
            ("queued", self.pending()),
            ("generating", self.generating.len()),
            (
                "protos",
                self.protos.values().filter(|p| p.is_some()).count(),
            ),
            ("decorating", self.decorating),
            ("finishing", self.finishing.len()),
            ("lighting", self.lighting.len()),
            ("meshing", self.meshing.len()),
        ]
    }
    /// Advance streaming: collect finished work, then schedule more.
    /// Returns meshes ready for upload and columns that were unloaded.
    pub fn update(
        &mut self,
        world: &mut World,
    ) -> (Vec<((i32, i32, i32), SectionMesh)>, Vec<Arc<Chunk>>) {
        let plan = self.plan_update(world);
        self.finish_update(world, plan)
    }

    /// Collect completed work and plan removals without removing live chunks.
    pub fn plan_update(&mut self, world: &mut World) -> StreamingUpdate {
        let mut step = self.begin_plan_update(world);
        loop {
            match step {
                PlanStep::Ready(plan) => return plan,
                PlanStep::Save { chunk, resume } => {
                    self.save_finished(chunk);
                    step = self.resume_plan_update(world, resume);
                }
            }
        }
    }

    /// Start planning, yielding before any late-generated column is saved.
    pub fn begin_plan_update(&mut self, world: &mut World) -> PlanStep {
        let mut hz_laps = crate::spans::laps();
        if self.paused {
            return PlanStep::Ready(StreamingUpdate {
                uploads: Vec::new(),
                sweep: false,
                far: Vec::new(),
                paused: true,
            });
        }
        ExtraCover::refresh(&mut self.cover, &self.extra);
        if self.forced.is_empty() {
            self.forced_cover = None;
        } else {
            ForcedCover::refresh(&mut self.forced_cover, &self.forced);
        }
        self.cover_current = true;
        // The sweeps below walk every loaded column and proto chunk. Run them
        // at once when the view moves and every eighth tick otherwise: a
        // column can stay a little past the edge without anyone noticing.
        let area = (self.center, self.radius);
        // The sweep unloads columns, saving them and dropping their block entities' bindings:
        // never while a transaction is open, whose values may still be tentative. A due sweep
        // waits for the next update instead.
        let sweep = (area != self.scan_area || self.scan_age >= 8)
            && !world.block_entities.defers_retirement();
        if sweep {
            (self.scan_area, self.scan_age) = (area, 0);
        } else {
            self.scan_age = self.scan_age.saturating_add(1);
        }
        self.requeue();
        self.relight_edits(world);
        hz_laps.lap("st_relight_edits");
        let deferred = std::mem::take(&mut self.deferred).into_iter();
        let mut received = std::mem::take(&mut self.received_scratch);
        self.shared.done.take_into(&mut received);
        self.resume_plan_update(
            world,
            PlanResume {
                sweep,
                uploads: Vec::new(),
                stale: Vec::new(),
                deferred,
                received,
            },
        )
    }

    /// Resume processing the same batch after the caller saves a late column.
    pub fn resume_plan_update(&mut self, world: &mut World, mut resume: PlanResume) -> PlanStep {
        let mut hz_laps = crate::spans::laps();
        while let Some(done) = resume
            .deferred
            .next()
            .or_else(|| resume.received.pop_front())
        {
            match done {
                Done::Proto(x, z, proto, decorated) => {
                    self.generation_scan.exhausted = false;
                    self.generating.remove(&(x, z));
                    if self.in_range(x, z, 4) || self.near_finished(x, z, 2) {
                        self.light_views.remove(&(x, z));
                        self.protos.insert((x, z), Some(proto));
                        if decorated || !self.decorates {
                            self.decorated.insert((x, z));
                        }
                    }
                }
                Done::Decorated(x, z, region, finals, writes) => {
                    self.receive_decorated(x, z, *region, finals, writes, world)
                }
                Done::Generated(chunk) => {
                    if let Some(chunk) = self.receive_generated(chunk, world) {
                        return PlanStep::Save { chunk, resume };
                    }
                }
                Done::Saved(protos) => self.receive_saved(protos),
                Done::LightView(x, z, proto, view) => self.receive_light_view(x, z, proto, view),
                Done::Lit(x, z, sections, seq) => {
                    if self.lighting.get(&(x, z)) != Some(&seq) {
                        continue;
                    }
                    self.lighting.remove(&(x, z));
                    let snapshot = self.light_snapshots.remove(&(x, z)).unwrap_or(u64::MAX);
                    self.apply_lit(world, x, z, sections, snapshot);
                }
                Done::Meshed(x, sy, z, mesh, seq) => {
                    if self.meshing.get(&(x, sy, z)) != Some(&seq) {
                        resume.stale.push(((x, sy, z), mesh));
                        continue;
                    }
                    // Its column unloaded while it was meshed: nothing would
                    // ever remove the uploaded section.
                    if !world.chunks.contains_key(&(x, z)) {
                        self.meshing.remove(&(x, sy, z));
                        resume.stale.push(((x, sy, z), mesh));
                        continue;
                    }
                    self.meshing.remove(&(x, sy, z));
                    self.compile_started.insert((x, sy, z), seq);
                    self.meshed_total += 1;
                    resume.uploads.push(((x, sy, z), mesh));
                }
            }
        }
        hz_laps.lap("st_results");
        if resume.received.capacity() > 1024 {
            resume.received.shrink_to(256);
        }
        self.received_scratch = resume.received;
        self.free(resume.stale);
        let far = if resume.sweep && !self.remote {
            let ranges = self.ranges();
            let decorates = self.decorates;
            plan_unloads(world, |x, z| ranges.unloaded(x, z, decorates))
        } else {
            Vec::new()
        };
        PlanStep::Ready(StreamingUpdate {
            uploads: resume.uploads,
            sweep: resume.sweep,
            far,
            paused: false,
        })
    }

    /// Commit a planned update after the caller has finished with live chunks.
    pub fn finish_update(
        &mut self,
        world: &mut World,
        plan: StreamingUpdate,
    ) -> (Vec<((i32, i32, i32), SectionMesh)>, Vec<Arc<Chunk>>) {
        let (commit, protos) = self.prepare_commit(world, plan);
        self.submit_protos(protos);
        self.complete_commit(world, commit)
    }

    /// Remove planned columns and check out proto saves before further scheduling.
    pub fn prepare_commit(
        &mut self,
        world: &mut World,
        plan: StreamingUpdate,
    ) -> (StreamingCommit, Vec<ProtoSave>) {
        let StreamingUpdate {
            uploads,
            sweep,
            far,
            paused,
        } = plan;
        if paused {
            return (
                StreamingCommit {
                    uploads,
                    removed: Vec::new(),
                    paused: true,
                },
                Vec::new(),
            );
        }
        let mut hz_laps = crate::spans::laps();
        let mut removed = Vec::new();
        let mut proto_saves = Vec::new();
        if sweep {
            if !far.is_empty() {
                for key in &far {
                    self.decorated.remove(key);
                }
                removed = commit_unloads(world, &far);
                for key in &far {
                    world.entities_pending.remove(key);
                    self.lit.remove(key);
                    self.meshed.remove(key);
                    self.lighting.remove(key);
                    self.light_snapshots.remove(key);
                    world.dirty_sections.take_column(key.0, key.1);
                }
                let far_set: FxHashSet<(i32, i32)> = far.iter().copied().collect();
                forget_pending_sections(
                    &mut self.meshing,
                    &mut self.compile_started,
                    &mut self.waiting,
                    &far_set,
                    world.layout().sections() as i32,
                );
            }
            // Drop far proto chunks that are not checked out.
            if !self.protos.is_empty() {
                let (center, r4) = (self.center, i64::from(self.radius + 4));
                let ranges = self.ranges();
                let far: Vec<(i32, i32)> = self
                    .protos
                    .iter()
                    .filter(|&(&(x, z), p)| {
                        p.is_some()
                            && (self.radius < 0
                                || i64::from(x - center.0).pow(2) + i64::from(z - center.1).pow(2)
                                    > r4 * r4)
                            && !ranges.in_extra(x, z, 4)
                            && !ranges.in_band(x, z, 4)
                            && !ranges.forced_within(x, z, FORCED_TERRAIN_RING)
                    })
                    .map(|(k, _)| *k)
                    .collect();
                if !far.is_empty() {
                    self.generation_scan.invalidate();
                    // Those decoration wrote to are saved first (see
                    // `ChunkGenerator::save_protos`), and dropped once back.
                    let written = drop_unsaved_protos(
                        far,
                        &mut self.written,
                        &mut self.protos,
                        &mut self.light_views,
                        &mut self.decorated,
                        &self.finishing,
                        world,
                    );
                    if !written.is_empty() {
                        proto_saves = self.checkout_protos(written);
                    }
                }
            }
        }
        hz_laps.lap("st_sweep");
        (
            StreamingCommit {
                uploads,
                removed,
                paused: false,
            },
            proto_saves,
        )
    }

    /// Resume an update after submitting its proto saves.
    pub fn complete_commit(
        &mut self,
        world: &mut World,
        commit: StreamingCommit,
    ) -> (Vec<((i32, i32, i32), SectionMesh)>, Vec<Arc<Chunk>>) {
        let StreamingCommit {
            uploads,
            removed,
            paused,
        } = commit;
        if paused {
            return (uploads, removed);
        }
        let mut light = std::mem::take(&mut self.light_scratch);
        light.clear();
        let mut gone = std::mem::take(&mut self.gone_scratch);
        gone.clear();
        let mut candidates = std::mem::take(&mut self.mesh_candidates_scratch);
        candidates.clear();
        let mut columns = std::mem::take(&mut self.mesh_columns_scratch);
        columns.clear();
        let mut hz_laps = crate::spans::laps();
        if !self.protos.is_empty() || !removed.is_empty() {
            let (protos, finishing) = (&self.protos, &self.finishing);
            self.decorated.retain(|k| {
                protos.contains_key(k) || finishing.contains(k) || world.chunks.contains_key(k)
            });
        }
        // Finalize chunks whose neighbourhood is decorated.
        if !self.protos.is_empty() {
            let mut finalize = std::mem::take(&mut self.finalize_scratch);
            finalize.clear();
            {
                let ranges = self.ranges();
                for (&(x, z), proto) in &self.protos {
                    if proto.is_none()
                        || world.chunks.contains_key(&(x, z))
                        || self.finishing.contains(&(x, z))
                        || !ranges.in_range(x, z, 1)
                    {
                        continue;
                    }
                    let ready = (-1..=1).all(|dz| {
                        (-1..=1).all(|dx| {
                            self.decorated.contains(&(x + dx, z + dz))
                                || world.chunks.contains_key(&(x + dx, z + dz))
                        })
                    });
                    if ready {
                        finalize.push((x, z));
                    }
                }
            }
            for (x, z) in finalize.drain(..) {
                let proto = self
                    .protos
                    .remove(&(x, z))
                    .flatten()
                    .expect("present proto");
                self.light_views.remove(&(x, z));
                self.finishing.insert((x, z));
                self.written.remove(&(x, z));
                self.submit(Some((x, z)), 0, Job::Finish(proto));
            }
            self.finalize_scratch = finalize;
        }
        hz_laps.lap("st_finalize");
        // Decorate chunks whose 3x3 proto neighbourhood is available, in
        // vanilla's order: decoration is order dependent where write regions
        // overlap (see `DecorationOrder`).
        let workers = self.workers;
        if self.decorates && self.order_area != Some((self.center, self.radius)) {
            let (decorated, chunks) = (&self.decorated, &world.chunks);
            // The render distance is the server view distance in singleplayer.
            self.decoration_order
                .reassign(self.center, self.radius, |x, z| {
                    decorated.contains(&(x, z)) || chunks.contains_key(&(x, z))
                });
            self.order_area = Some((self.center, self.radius));
            let ranges = Ranges {
                center: self.center,
                radius: self.radius,
                extra: &self.extra,
                forced: &self.forced,
                cover: ExtraCover::current(&self.cover, &self.extra, self.cover_current),
                forced_cover: self.forced_cover.as_ref().filter(|_| self.cover_current),
                band: self.band.as_ref(),
            };
            retain_decoration_order(&mut self.decoration_order, &ranges, &self.protos);
        }
        if self.decorates && self.decorating < workers * 2 && !self.protos.is_empty() {
            // A wanted chunk holds back higher-ranked ones only if it can be
            // decorated: every neighbour in its 3x3 is either final (when the
            // generator decorates beside final chunks) or generated as a proto.
            let waits_for = |s: &Self, x: i32, z: i32| {
                let ranges = s.ranges();
                DecorationWait {
                    ranges: &ranges,
                    world,
                    decorated: &s.decorated,
                    protos: &s.protos,
                    generating: &s.generating,
                    saves_beside_final: s.saves_beside_final,
                }
                .waits_for(x, z)
            };
            // Out for the call: ranking reads the streamer's ranges.
            let mut order = std::mem::take(&mut self.decoration_order);
            let mut wanted = {
                let ranges = Ranges {
                    center: self.center,
                    radius: self.radius,
                    extra: &self.extra,
                    forced: &self.forced,
                    cover: ExtraCover::current(&self.cover, &self.extra, self.cover_current),
                    forced_cover: self.forced_cover.as_ref().filter(|_| self.cover_current),
                    band: self.band.as_ref(),
                };
                decoration_queue(
                    &mut order,
                    self.protos
                        .iter()
                        .filter(|&(&(x, z), proto)| {
                            proto.is_some()
                                && !self.decorated.contains(&(x, z))
                                && !world.chunks.contains_key(&(x, z))
                                && (ranges.in_range(x, z, 2) || ranges.near_finished(x, z, 1))
                        })
                        .map(|(&k, _)| k),
                    |x, z| ranges.priority(x, z),
                )
            };
            self.decoration_order = order;
            if !wanted.is_empty() {
                // Most wanted chunks are blocked, and the same neighbours are
                // asked about for each of them: what `waits_for` says is kept for
                // the pass (nothing it reads changes in it but `decorated`, which
                // updates the entry). Without it, scheduling at render distance 32
                // took most of the thread that runs it while the world loaded.
                let waiting: std::cell::RefCell<FxHashMap<(i32, i32), bool>> = Default::default();
                while let Some(std::cmp::Reverse((_, x, z))) = wanted.pop() {
                    if self.decorating >= workers * 2 {
                        break;
                    }
                    if self.decoration_order.blocked(x, z, |nx, nz| {
                        if let Some(&known) = waiting.borrow().get(&(nx, nz)) {
                            return known;
                        }
                        let known = waits_for(self, nx, nz);
                        waiting.borrow_mut().insert((nx, nz), known);
                        known
                    }) {
                        continue;
                    }
                    let keys: [(i32, i32); 9] =
                        std::array::from_fn(|i| (x + (i as i32 % 3) - 1, z + (i as i32 / 3) - 1));
                    // A final neighbour cannot be generated again: a generator that
                    // can decorates beside it (a chunk of an older world), writing
                    // into it as vanilla's features do; the others skip decoration
                    // (worlds saved before proto chunks were saved get here: their
                    // final chunks' neighbours are generated again).
                    if keys.iter().any(|k| self.decorating_finals.contains(k)) {
                        continue;
                    }
                    let has_final = keys.iter().any(|k| world.chunks.contains_key(k));
                    if has_final && !self.saves_beside_final {
                        self.decorated.insert((x, z));
                        waiting.borrow_mut().insert((x, z), false);
                        continue;
                    }
                    if !keys.iter().all(|k| {
                        world.chunks.contains_key(k) || matches!(self.protos.get(k), Some(Some(_)))
                    }) {
                        continue;
                    }
                    let finals: Vec<(usize, Chunk)> = keys
                        .iter()
                        .enumerate()
                        .filter_map(|(i, k)| Some((i, (**world.chunks.get(k)?).clone())))
                        .collect();
                    let region: Box<[Proto; 9]> = Box::new(std::array::from_fn(|i| {
                        let k = &keys[i];
                        if finals.iter().any(|(f, _)| *f == i) {
                            return Box::new(()) as Proto;
                        }
                        self.light_views.remove(k);
                        self.protos
                            .insert(*k, None)
                            .flatten()
                            .expect("present proto")
                    }));
                    self.decorating += 1;
                    self.decorating_finals
                        .extend(finals.iter().map(|(i, _)| keys[*i]));
                    self.submit(Some((x, z)), 1, Job::Decorate(x, z, region, finals));
                }
            }
        }
        hz_laps.lap("st_decorate");
        self.schedule_generation(world, workers);
        hz_laps.lap("st_generate");
        // Lighting once all neighbours exist, nearest first and only as
        // much of it as the workers can chew. Vanilla's light engine also
        // hands a bounded queue to its workers and reschedules what did
        // not fit by distance; submitted whole, the light jobs of every
        // unlit column at once — each carrying a 3x3 region snapshot —
        // grew with the render distance and outlived the columns they
        // were queued for (#3629). What stays in `dirty_light` is asked
        // for again, from the current centre, as capacity frees.
        if !world.dirty_light.is_empty() {
            let room = (self.workers * 4).saturating_sub(self.lighting.len());
            {
                let ranges = self.ranges();
                for &(x, z) in world.dirty_light.iter() {
                    if !world.chunks.contains_key(&(x, z)) {
                        gone.push((x, z));
                    } else if room > 0 && self.light_region_ready(world, x, z) {
                        light.push((ranges.priority(x, z), x, z));
                    }
                }
            }
            for key in gone.drain(..) {
                world.dirty_light.remove(&key);
            }
            if room > 0 && !light.is_empty() {
                nearest_first(&mut light, room);
                for (_, x, z) in light.drain(..) {
                    let Some(region) = self.light_region(world, x, z) else {
                        continue;
                    };
                    world.dirty_light.remove(&(x, z));
                    self.sequence += 1;
                    self.lighting.insert((x, z), self.sequence);
                    self.light_snapshots
                        .insert((x, z), self.light_edit_sequence);
                    self.submit(
                        Some((x, z)),
                        1,
                        Job::Light(x, z, region, self.sequence, self.sky_light),
                    );
                }
            }
        }
        hz_laps.lap("st_light");
        // A server draws nothing: its sections are never meshed.
        if !self.mesh {
            world.dirty_sections.clear();
            world.immediate_sections.clear();
            self.cover_current = false;
            self.light_scratch = light;
            self.gone_scratch = gone;
            self.mesh_candidates_scratch = candidates;
            self.mesh_columns_scratch = columns;
            return (uploads, removed);
        }
        // Meshing once the column and its neighbours are lit, and the column
        // is within the render distance (`in_view_distance`, the columns
        // vanilla draws). Sections set aside past it come back
        // when the player moves: a column that loads with its light gets no
        // other request (#228).
        let area = (self.center, self.radius, self.forced.len());
        if self.waiting_area != area {
            self.waiting_area = area;
            if !self.waiting.is_empty() {
                let ranges = Ranges {
                    center: self.center,
                    radius: self.radius,
                    extra: &self.extra,
                    forced: &self.forced,
                    cover: ExtraCover::current(&self.cover, &self.extra, self.cover_current),
                    forced_cover: self.forced_cover.as_ref().filter(|_| self.cover_current),
                    band: self.band.as_ref(),
                };
                self.waiting.retain(|&(x, sy, z)| {
                    let back = ranges.in_view(x, z);
                    if back {
                        world.dirty_sections.insert((x, sy, z));
                    }
                    !back
                });
            }
        }
        hz_laps.lap("st_waiting");
        // Nearest first, and only as many sections as the workers can chew.
        // Every ready column used to be submitted at once, so at a wide
        // render distance the queue — and `meshing`, and each job's 3x3
        // region snapshot — grew with the number of loaded columns and the
        // jobs went stale long before a worker reached them (#3629). What
        // stays dirty is asked for again, from the current centre, as
        // capacity frees; `requeue` then has a small queue to keep close.
        // Meshes waiting for upload count too: when uploads fall behind (a
        // low frame rate at a wide distance), meshing on held gigabytes of
        // them until the system swapped.
        let room = (self.workers * 32).saturating_sub(self.meshing.len() + self.upload_backlog);
        // Column by column: skip scanning dirty columns entirely when no mesh
        // room is available or nothing is dirty.
        if room > 0 && !world.dirty_sections.is_empty() {
            columns.extend(world.dirty_sections.columns());
            let ranges = Ranges {
                center: self.center,
                radius: self.radius,
                extra: &self.extra,
                forced: &self.forced,
                cover: ExtraCover::current(&self.cover, &self.extra, self.cover_current),
                forced_cover: self.forced_cover.as_ref().filter(|_| self.cover_current),
                band: self.band.as_ref(),
            };
            for ((x, z), sections) in columns.iter().copied() {
                if !world.chunks.contains_key(&(x, z)) {
                    world.dirty_sections.take_column(x, z);
                    continue;
                }
                if !ranges.in_view(x, z) {
                    world.dirty_sections.take_column(x, z);
                    for sy in sections.iter().map(|sy| sy as i32) {
                        self.waiting.insert((x, sy, z));
                    }
                    continue;
                }
                // A neighbour being lit dirties this column again when its light
                // job finishes (new chunks relight their eight neighbours), so
                // wait for it rather than mesh once per neighbour. Columns still
                // in `dirty_light` here can't be lit yet (a neighbour of theirs is
                // missing): waiting for them could last until the player comes
                // back.
                let ready = (-1..=1).all(|dz| {
                    (-1..=1).all(|dx| {
                        let n = (x + dx, z + dz);
                        self.lit.contains(&n) && !self.lighting.contains_key(&n)
                    })
                });
                if !ready {
                    continue;
                }
                candidates.push((ranges.priority(x, z), x, z, sections));
            }
        }
        hz_laps.lap("st_candidates");
        if room > 0 && !candidates.is_empty() {
            let settings = self.mesh_settings();
            let mut queue = self.shared.queue.lock().unwrap();
            let submitted = submit_mesh_candidates(
                world,
                &mut candidates,
                room,
                &settings,
                &mut self.meshed,
                &mut self.meshing,
                &mut self.sequence,
                &mut self.order,
                &mut queue,
            );
            drop(queue);
            candidates.clear();
            if submitted > 1 {
                self.jobs.notify_all();
            } else if submitted == 1 {
                self.jobs.notify();
            }
        }
        self.cover_current = false;
        self.light_scratch = light;
        self.gone_scratch = gone;
        self.mesh_candidates_scratch = candidates;
        self.mesh_columns_scratch = columns;
        (uploads, removed)
    }
    fn schedule_generation(&mut self, world: &World, workers: usize) {
        let input = GenerationInputs {
            ranges: Ranges {
                center: self.center,
                radius: self.radius,
                extra: &self.extra,
                forced: &self.forced,
                cover: ExtraCover::current(&self.cover, &self.extra, self.cover_current),
                forced_cover: self.forced_cover.as_ref().filter(|_| self.cover_current),
                band: self.band.as_ref(),
            },
            world,
            generating: &self.generating,
            protos: &self.protos,
            finishing: &self.finishing,
            decorates: self.decorates,
            remote: self.remote,
        };
        if let Some(wanted) = self.generation_scan.candidates(input, workers * 3) {
            for (_, x, z) in wanted {
                self.generating.insert((x, z));
                self.submit(Some((x, z)), 0, Job::Generate(x, z));
            }
        }
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
        let positions = if positions.len() > INLINE_RELIGHT_MAX {
            self.relight_on_workers(world, positions)
        } else {
            positions
        };
        if positions.is_empty() {
            return;
        }
        let mut changed = LightChanges::default();
        crate::light::relight(
            &mut world.chunks,
            &self.light_tables,
            self.sky_light,
            &positions,
            false,
            &mut changed,
        );
        Self::mark_light_changes(world, &changed);
        Self::relight_beside_unlit(world, &changed);
        if !self.light_snapshots.is_empty() {
            for p in positions {
                self.light_edit_sequence += 1;
                self.light_log.push((self.light_edit_sequence, p));
            }
        }
    }
    /// Hands a big batch of edits (a `/fill` arriving block by block) to the
    /// workers: the columns they touch and their neighbours get full light
    /// jobs, as `World::edit_runs` does, instead of relighting every changed
    /// cell in the frame (clearing a 64x64x16 volume in the air relit the
    /// sky below it: 43 ms of one frame). The sections wait for those jobs
    /// before they are meshed again. Returns the edits left to light in the
    /// frame: those in columns that can't be lit whole yet (the edge of the
    /// loaded area).
    fn relight_on_workers(
        &self,
        world: &mut World,
        positions: Vec<(i32, i32, i32)>,
    ) -> Vec<(i32, i32, i32)> {
        let (columns, inline) = whole_relight_columns(positions, |x, z| {
            self.lit.contains(&(x, z)) && self.light_region_ready(world, x, z)
        });
        world.dirty_light.extend(columns);
        inline
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
            let mut changed = LightChanges::default();
            crate::light::relight(
                &mut world.chunks,
                &self.light_tables,
                self.sky_light,
                &positions,
                true,
                &mut changed,
            );
            Self::mark_light_changes(world, &changed);
            Self::relight_beside_unlit(world, &changed);
        }
        let oldest = self.light_snapshots.values().copied().min();
        match oldest {
            Some(oldest) => self.light_log.retain(|(seq, _)| *seq > oldest),
            None => self.light_log.clear(),
        }
    }
    /// A column whose light an edit changed beside a column not lit yet gets
    /// a full light job ([`crate::light::beside_unlit`]).
    fn relight_beside_unlit(world: &mut World, changed: &LightChanges) {
        let columns = crate::light::beside_unlit(changed, |x, z| {
            world.chunks.get(&(x, z)).is_some_and(|c| !c.lit)
        });
        world.dirty_light.extend(columns);
    }
    /// Sections whose meshes sample a changed light value (faces and corners
    /// read light from neighbouring blocks). Every column whose stored light
    /// changed is saved again (`ChunkHolder.sectionLightChanged` marks the
    /// chunk unsaved): an edit's light reaches up to 15 blocks into columns
    /// the edit itself did not mark, which may already have been saved.
    fn mark_light_changes(world: &mut World, changed: &LightChanges) {
        for (x, sy, z) in changed.sections() {
            world.modified.insert((x, z));
            if let Some(log) = &mut world.light_log {
                log.entry((x, z)).or_default().insert(sy as usize);
            }
        }
        changed.dirty_sections(|section| {
            world.dirty_sections.insert(section);
        });
    }
    fn ranges(&self) -> Ranges<'_> {
        Ranges {
            center: self.center,
            radius: self.radius,
            extra: &self.extra,
            forced: &self.forced,
            cover: ExtraCover::current(&self.cover, &self.extra, self.cover_current),
            forced_cover: self.forced_cover.as_ref().filter(|_| self.cover_current),
            band: self.band.as_ref(),
        }
    }
    fn near_finished(&self, x: i32, z: i32, reach: i32) -> bool {
        self.ranges().near_finished(x, z, reach)
    }
    /// Abort unvisited background chunk generation, decoration, and meshing on world exit.
    pub fn abort_generation(&mut self) {
        self.shared
            .aborted
            .store(true, std::sync::atomic::Ordering::SeqCst);
        if let Ok(mut queue) = self.shared.queue.lock() {
            let mut saves = Vec::new();
            for item in std::mem::take(&mut *queue).into_vec() {
                if matches!(item.job, Job::Save(..)) {
                    saves.push(item);
                }
            }
            *queue = std::collections::BinaryHeap::from(saves);
        }
        self.generating.clear();
        self.decorating = 0;
        self.decorating_finals.clear();
        self.finishing.clear();
        self.viewing.clear();
        self.waiting.clear();
        self.meshing.clear();
        self.lighting.clear();
        self.jobs.notify_all();
    }
    /// Save what generation holds outside the world: the proto chunks
    /// decoration wrote to. With `wait`, the decorations, columns and saves
    /// in flight are waited for first (their proto chunks are out in the
    /// jobs) and the protos are saved here, before the world's chunks;
    /// otherwise a worker saves them (vanilla's autosave hands its chunks to
    /// the IOWorker too), and those out in jobs are saved later, as they
    /// unload or at the next save.
    pub fn save(&mut self, world: &mut World, wait: bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            match self.prepare_save(world, wait, deadline) {
                SaveStep::Full(chunk) => self.save_finished(chunk),
                SaveStep::Protos(protos) => {
                    self.finish_save(protos, wait);
                    return;
                }
                SaveStep::Pending => unreachable!(),
            }
        }
    }

    /// Check out the proto saves, yielding for late full saves first.
    pub fn prepare_save(
        &mut self,
        world: &mut World,
        wait: bool,
        deadline: std::time::Instant,
    ) -> SaveStep {
        if !wait {
            let written: Vec<(i32, i32)> = self.written.drain().collect();
            return SaveStep::Protos(self.checkout_protos(written));
        }
        let aborted = self
            .shared
            .aborted
            .load(std::sync::atomic::Ordering::Relaxed);
        while (self.saving > 0)
            || (!aborted
                && (self.decorating > 0 || !self.finishing.is_empty() || !self.viewing.is_empty()))
        {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match self.shared.done.recv_timeout(left) {
                Some(Done::Decorated(x, z, region, finals, writes)) => {
                    self.receive_decorated(x, z, *region, finals, writes, world)
                }
                Some(Done::Generated(chunk)) => {
                    if let Some(chunk) = self.receive_generated(chunk, world) {
                        return SaveStep::Full(chunk);
                    }
                }
                Some(Done::Saved(protos)) => self.receive_saved(protos),
                Some(Done::LightView(x, z, proto, view)) => {
                    self.receive_light_view(x, z, proto, view)
                }
                Some(other) => self.deferred.push(other),
                None => {
                    log::warn!("Horizon: chunk generation did not finish before saving");
                    break;
                }
            }
        }
        let written: Vec<_> = self.written.drain().collect();
        SaveStep::Protos(self.checkout_protos(written))
    }

    /// Complete a prepared save, returning synchronous protos to streaming.
    pub fn finish_save(&mut self, protos: Vec<ProtoSave>, wait: bool) {
        if wait {
            if !protos.is_empty() {
                let refs: Vec<&Proto> = protos.iter().map(|p| &p.proto).collect();
                self.saves.save_protos(&refs);
                for proto in protos {
                    self.protos.insert(proto.pos, Some(proto.proto));
                }
            }
        } else {
            self.submit_protos(protos);
        }
    }
    /// Finish generation for a closing world without waiting on the event
    /// loop. No new generation is scheduled while the save screen is shown.
    /// Poll generation and yield for each late full save before proto submission.
    /// The caller keeps the same deadline across every yield in this tick.
    pub fn prepare_poll_save(
        &mut self,
        world: &mut World,
        deadline: std::time::Instant,
    ) -> SaveStep {
        self.prepare_poll_save_with_clock(world, deadline, std::time::Instant::now)
    }

    fn prepare_poll_save_with_clock(
        &mut self,
        world: &mut World,
        deadline: std::time::Instant,
        mut now: impl FnMut() -> std::time::Instant,
    ) -> SaveStep {
        while now() < deadline {
            let Some(done) = self.shared.done.try_recv() else {
                break;
            };
            match done {
                Done::Decorated(x, z, region, finals, writes) => {
                    self.receive_decorated(x, z, *region, finals, writes, world)
                }
                Done::Generated(chunk) => {
                    if let Some(chunk) = self.receive_generated(chunk, world) {
                        return SaveStep::Full(chunk);
                    }
                }
                Done::Saved(protos) => self.receive_saved(protos),
                Done::LightView(x, z, proto, view) => self.receive_light_view(x, z, proto, view),
                other => self.deferred.push(other),
            }
        }
        if now() >= deadline {
            return SaveStep::Pending;
        }
        let aborted = self
            .shared
            .aborted
            .load(std::sync::atomic::Ordering::Relaxed);
        if (self.saving > 0)
            || (!aborted
                && (self.decorating > 0 || !self.finishing.is_empty() || !self.viewing.is_empty()))
        {
            return SaveStep::Pending;
        }
        let written: Vec<_> = self.written.drain().collect();
        SaveStep::Protos(self.checkout_protos(written))
    }
    fn in_range(&self, x: i32, z: i32, margin: i32) -> bool {
        self.ranges().in_range(x, z, margin)
    }
    /// Rank the 3x3 a chunk loaded to FULL decorates, in one generation
    /// task's order (`ChunkGenerationTask`: x-outer, z-inner), after every
    /// chunk ranked before.
    pub fn rank_full_task(&mut self, x: i32, z: i32) {
        for dx in -1..=1 {
            for dz in -1..=1 {
                self.decoration_order.rank_or_append(x + dx, z + dz);
            }
        }
    }
    /// Make a column resident for an immediate read.
    /// The temporary request serves the same purpose as `ServerChunkCache.getChunk(FULL)`.
    pub fn load_full_chunk(&mut self, world: &mut World, key: (i32, i32)) {
        if world.chunks.contains_key(&key) {
            return;
        }
        self.rank_full_task(key.0, key.1);
        // Full columns finish out to radius + 1. This request is for one column.
        self.extra.push((key, -1));
        self.wait_for_full_chunk(world, key);
        self.extra.pop();
    }
    pub fn band(&self) -> Option<Band> {
        self.band
    }
    /// Update one saved forced ticket without waiting for generation. An add
    /// ranks decoration; gameplay uses `set_forced_loaded` for the load barrier.
    pub fn set_forced(&mut self, x: i32, z: i32, forced: bool) -> bool {
        let mut changed = false;
        self.set_forced_area((x, z), (x, z), forced, |_, _, c| changed = c);
        changed
    }
    /// `ServerLevel.setChunkForced`: a newly forced column is present before
    /// returning. Repeated adds and removals only report the set change.
    pub fn set_forced_loaded(&mut self, world: &mut World, x: i32, z: i32, forced: bool) -> bool {
        let changed = self.set_forced(x, z, forced);
        if forced && changed {
            self.wait_for_full_chunk(world, (x, z));
        }
        changed
    }

    /// Pump Horizon's generation and disk reads for a requested column.
    fn wait_for_full_chunk(&mut self, world: &mut World, key: (i32, i32)) {
        while !world.chunks.contains_key(&key) {
            assert!(!self.paused, "chunk load before streaming started");
            assert!(
                !self
                    .shared
                    .aborted
                    .load(std::sync::atomic::Ordering::Relaxed),
                "chunk load after generation stopped"
            );
            // Pump the existing generation queue. Keep unloads for the
            // normal owner update, which saves entities and notifies mods.
            let mut plan = self.plan_update(world);
            plan.sweep = false;
            plan.far.clear();
            let (meshes, unloaded) = self.finish_update(world, plan);
            debug_assert!(unloaded.is_empty());
            self.free(meshes);
            if !world.chunks.contains_key(&key) {
                self.wait(std::time::Duration::from_millis(5));
            }
        }
    }

    /// Release every `/forceload`ed chunk (`/forceload remove all`, a game-test
    /// batch's end). Chunks a mod holds a ticket for stay forced.
    pub fn clear_forced(&mut self) {
        let cleared: Vec<_> = self.commanded.drain().collect();
        for chunk in cleared {
            self.settle(chunk);
        }
    }
    /// The level's saved forced state: `/forceload`'s chunks, and the mods'
    /// tickets (of controllers that are registered), which wait in
    /// `take_loaded_tickets`.
    pub fn load_forced(&mut self, commanded: HashSet<(i32, i32)>, tickets: Vec<ModTicket>) {
        self.mod_tickets = ModTickets::from_saved(tickets);
        self.commanded = commanded;
        self.forced.clear();
        let held: Vec<_> = self.commanded.iter().copied().collect();
        for chunk in held {
            self.settle(chunk);
        }
    }
    /// The mods' tickets read from the save, which hold nothing until the
    /// mods have validated them and the survivors are put back with
    /// `set_mod_forced`.
    pub fn take_loaded_tickets(&mut self) -> Vec<ModTicket> {
        self.mod_tickets.take_loaded()
    }
    /// The `/forceload`ed chunks.
    pub fn commanded(&self) -> &HashSet<(i32, i32)> {
        &self.commanded
    }
    /// The chunk tickets of mods.
    pub fn mod_tickets(&self) -> &ModTickets {
        &self.mod_tickets
    }
    /// Hold or release a mod's ticket: true when the ticket set changed. A
    /// chunk stays forced while `/forceload` or any ticket holds it; one
    /// newly forced is ranked for decoration as `/forceload add` does.
    pub fn set_mod_forced(&mut self, ticket: ModTicket, forced: bool) -> bool {
        let chunk = ticket.chunk;
        let changed = if forced {
            self.mod_tickets.insert(ticket)
        } else {
            self.mod_tickets.remove(&ticket)
        };
        if self.settle(chunk) {
            self.rank_forced(chunk.0, chunk.1, chunk.0, chunk.1);
        }
        changed
    }
    /// Hold or release a runtime-only mod ticket ([`TransientTicket`]), as
    /// [`Self::set_mod_forced`] does a saved one: true when it changed.
    pub fn set_transient_forced(
        &mut self,
        ticket: crate::mod_chunk_tickets::TransientTicket,
        forced: bool,
    ) -> bool {
        let chunk = ticket.chunk;
        let changed = if forced {
            self.mod_tickets.insert_transient(ticket)
        } else {
            self.mod_tickets.remove_transient(&ticket)
        };
        if self.settle(chunk) {
            self.rank_forced(chunk.0, chunk.1, chunk.0, chunk.1);
        }
        changed
    }
    /// Bring `forced` in line with who holds `chunk`: true when it just
    /// became forced. The one writer of `forced`.
    fn settle(&mut self, chunk: (i32, i32)) -> bool {
        if self.commanded.contains(&chunk) || self.mod_tickets.holds(chunk) {
            self.forced.insert(chunk)
        } else {
            self.forced.remove(&chunk);
            false
        }
    }
    /// `ServerLevel.setChunkForced` over chunks `(x0, z0)..=(x1, z1)`, x then
    /// z: `each` hears whether each chunk changed in `/forceload`'s set. An
    /// add then ranks the area once for decoration (`forced_order`).
    pub fn set_forced_area(
        &mut self,
        (x0, z0): (i32, i32),
        (x1, z1): (i32, i32),
        forced: bool,
        mut each: impl FnMut(i32, i32, bool),
    ) {
        for x in x0..=x1 {
            for z in z0..=z1 {
                let changed = if forced {
                    self.commanded.insert((x, z))
                } else {
                    self.commanded.remove(&(x, z))
                };
                self.settle((x, z));
                each(x, z, changed);
            }
        }
        if forced {
            self.rank_forced(x0, z0, x1, z1);
        }
    }
    /// Rank the chunks `/forceload add` over chunks `(x0, z0)..=(x1, z1)`
    /// decorates in vanilla's order (`forced_order`), after every chunk
    /// ranked before.
    fn rank_forced(&mut self, x0: i32, z0: i32, x1: i32, z1: i32) {
        for (x, z) in horizon_world::worldgen::decoration_order::forced_order(x0, z0, x1, z1) {
            self.decoration_order.rank_or_append(x, z);
        }
    }
    /// Generate, light and keep the chunks of `band` (none: stop), and let
    /// go of what earlier bands held as it falls out of range. The chunks a
    /// band reaches are ranked for decoration row by row, as they first come
    /// within reach, so that a task decorates in the same order however its
    /// work is timed and shared among the workers.
    pub fn set_band(&mut self, band: Option<Band>) {
        if self.band == band {
            return;
        }
        self.band = band;
        self.generation_scan.exhausted = false;
        let Some(band) = band else {
            return;
        };
        // Decoration reaches two chunks past the band and generation three.
        const REACH: i32 = 4;
        let (x0, x1) = (band.x0 - REACH, band.x1 + REACH);
        let first = match self.band_ranked {
            Some((rx0, rx1, next)) if (rx0, rx1) == (x0, x1) => next,
            _ => band.z0 - REACH,
        };
        let end = band.z1 + REACH;
        for z in first..=end {
            for x in x0..=x1 {
                self.decoration_order.rank_or_append(x, z);
            }
        }
        self.band_ranked = Some((x0, x1, end.max(first - 1) + 1));
    }
    /// What holds up the columns `keys`, for a task that stopped advancing:
    /// how many are not loaded, loaded but unlit with a light job in flight,
    /// and unlit with none (those whose neighbours are all loaded are queued
    /// for light again).
    pub fn unstick_light(&mut self, world: &mut World, keys: &[(i32, i32)]) -> [usize; 3] {
        let mut counts = [0; 3];
        for &key in keys {
            if !world.chunks.contains_key(&key) {
                counts[0] += 1;
            } else if self.lit.contains(&key) {
            } else if self.lighting.contains_key(&key) {
                counts[1] += 1;
            } else {
                counts[2] += 1;
                if self.light_region(world, key.0, key.1).is_some()
                    || (-1..=1).any(|dz| {
                        (-1..=1).any(|dx| self.viewing.contains(&(key.0 + dx, key.1 + dz)))
                    })
                {
                    world.dirty_light.insert(key);
                }
            }
        }
        counts
    }
    /// Wait up to `timeout` for a worker to finish a job; the next `update`
    /// takes it. A server pumps generation with this between its ticks.
    pub fn wait(&mut self, timeout: std::time::Duration) {
        if let Some(done) = self.shared.done.recv_timeout(timeout) {
            self.deferred.push(done);
        }
    }
}
/// `ChunkTrackingView.isInViewDistance`: whether the column (x, z) is drawn
/// at this render distance around `center`, the camera's column
/// (`SectionOcclusionGraph.isInViewDistance`). Each axis gets a column of
/// slack, so this reaches past the circle of `radius` columns.
pub fn in_view_distance(center: (i32, i32), radius: i32, x: i32, z: i32) -> bool {
    let gap = |d: i32| i64::from((d.abs() - 1).max(0));
    let (dx, dz) = (gap(x - center.0), gap(z - center.1));
    dx * dx + dz * dz < i64::from(radius) * i64::from(radius)
}
/// How long a view's columns are waited for (`ViewArrival`): a server may never
/// fill it.
const VIEW_WAIT: std::time::Duration = std::time::Duration::from_secs(30);
/// Whether the columns of a view (centre, radius) have all arrived, as a client
/// sees them come: a server forgets the old view's columns, then sends the new
/// ones nearest first, so the view's outermost ones come last. Single player
/// joins its integrated server the same way, so `Streaming::busy` cannot see
/// what is not sent yet.
#[derive(Default)]
pub struct ViewArrival {
    view: ((i32, i32), i32),
    since: Option<std::time::Instant>,
    arrived: bool,
}
impl ViewArrival {
    /// Whether columns of `view` have yet to arrive (`has`): its outermost ones,
    /// at both ends of each of its rows and columns (`in_view_distance`). Once
    /// they have, false until the view changes; false too once it is no longer
    /// `recent`, and for a radius below 0 (no view).
    pub fn arriving(
        &mut self,
        view: ((i32, i32), i32),
        now: std::time::Instant,
        has: impl Fn((i32, i32)) -> bool,
    ) -> bool {
        if self.since.is_none() || view != self.view {
            (self.view, self.since, self.arrived) = (view, Some(now), false);
        }
        if self.arrived || view.1 < 0 || !self.recent(now) {
            return false;
        }
        let incomplete = self.arrival_incomplete(has);
        self.arrived = !incomplete;
        incomplete
    }

    /// Whether the tracked view still lacks a perimeter column, even after
    /// the UI stopped waiting or a level change cleared the columns.
    pub fn arrival_incomplete(&self, has: impl Fn((i32, i32)) -> bool) -> bool {
        let ((x, z), r) = self.view;
        if self.since.is_none() || r < 0 {
            return false;
        }
        !(-r..=r).all(|d| {
            // How far row (and column) `d` reaches: `in_view_distance` solved for it.
            let gap = i64::from((d.abs() - 1).max(0));
            let room = i64::from(r).pow(2) - gap * gap;
            room <= 0 || {
                let e = (room - 1).isqrt() as i32 + 1;
                // Both ends of row `d`, and of column `d`.
                [-e, e]
                    .into_iter()
                    .all(|e| has((x + d, z + e)) && has((x + e, z + d)))
            }
        })
    }
    /// Whether the view `arriving` was last given changed less than `VIEW_WAIT`
    /// ago: until then sections meshed are its terrain coming in, later an edit's.
    pub fn recent(&self, now: std::time::Instant) -> bool {
        self.since
            .is_some_and(|since| now.saturating_duration_since(since) < VIEW_WAIT)
    }
}
/// The rings around a `/forceload`ed chunk, in chunks. Vanilla's FORCED
/// ticket is level 31 (`ChunkMap.FORCED_TICKET_LEVEL`), the level rises one
/// per ring (`ChunkTracker.computeLevelFromNeighbor`), and the level picks
/// the column's status (`ChunkLevel.generationStatus`): full (33) two rings
/// out, features (34) at three, terrain (35) at four.
const FORCED_FULL_RING: i32 = 2;
const FORCED_FEATURES_RING: i32 = 3;
const FORCED_TERRAIN_RING: i32 = 4;
/// The areas kept loaded: circles around the player and `extra`, and forced chunks.
struct Ranges<'a> {
    center: (i32, i32),
    radius: i32,
    extra: &'a [((i32, i32), i32)],
    forced: &'a HashSet<(i32, i32)>,
    /// `extra` indexed by a grid or buckets: coverage queries without a
    /// scan of every area.
    cover: Option<&'a ExtraCover>,
    /// The combined `forced` set by distance (`Streaming::forced_cover`), while it matches.
    forced_cover: Option<&'a ForcedCover>,
    band: Option<&'a Band>,
}
/// The chunks a pregeneration task (`/pregen`) wants lit now: rows `z0..=z1`
/// of the columns `x0..=x1`, those within `circle` (a centre and a radius in
/// chunks) when there is one. Like an extra area, it finishes its chunks and
/// the ring around them, so that its own chunks can be lit, and generation
/// reaches past that. A task moves it down a strip of columns a row at a
/// time, so each chunk is generated once and only the rows in flight stay
/// loaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Band {
    pub x0: i32,
    pub x1: i32,
    pub z0: i32,
    pub z1: i32,
    pub circle: Option<((i32, i32), i32)>,
}
impl Band {
    /// Whether (x, z) lies within `margin` (Chebyshev) of one of the band's
    /// chunks: the square of side `2 * margin + 1` around it meets the
    /// rectangle at a point inside the circle.
    pub fn within(&self, x: i32, z: i32, margin: i32) -> bool {
        let (x0, x1) = (
            x.saturating_sub(margin).max(self.x0),
            x.saturating_add(margin).min(self.x1),
        );
        let (z0, z1) = (
            z.saturating_sub(margin).max(self.z0),
            z.saturating_add(margin).min(self.z1),
        );
        if x0 > x1 || z0 > z1 {
            return false;
        }
        self.circle.is_none_or(|((cx, cz), r)| {
            let dx = i64::from(cx.clamp(x0, x1)) - i64::from(cx);
            let dz = i64::from(cz.clamp(z0, z1)) - i64::from(cz);
            dx * dx + dz * dz <= i64::from(r) * i64::from(r)
        })
    }
    /// The band's chunks go after the players' (lower runs first), row by
    /// row, as the task wants them lit.
    fn priority(&self, x: i32, z: i32) -> i64 {
        const AFTER_PLAYERS: i64 = 1 << 40;
        AFTER_PLAYERS
            + (i64::from(z - self.z0 + 8).max(0) << 20)
            + i64::from(x - self.x0 + 8).max(0)
    }
}

/// For every column near the extra areas, how far it lies outside the
/// nearest one's square (Chebyshev distance less its radius; 0 inside). A server has an area per player, and streaming asks of every
/// loaded and wanted column whether one reaches it, several times a tick:
/// with fifty players, a scan of fifty squares per question.
///
/// Areas spread too far apart for that grid (players thousands of blocks from
/// each other) are found by the square of [`Self::BUCKET`] columns their centre
/// is in instead, which also answers which centre is nearest a column
/// (`Ranges::distance`, asked of every wanted column to rank it).
struct ExtraCover {
    extra: Vec<((i32, i32), i32)>,
    grid: Option<CoverGrid>,
    /// The indices in `extra` of the areas centred in each bucket.
    buckets: FxHashMap<(i32, i32), Vec<u32>>,
    /// The buckets' bounding box, inclusive: (x0, z0, x1, z1).
    bucket_box: (i32, i32, i32, i32),
    /// The largest radius in `extra`.
    radius: i32,
}
struct CoverGrid {
    x0: i32,
    z0: i32,
    width: u32,
    depth: u32,
    cells: Box<[i8]>,
}
impl ExtraCover {
    /// The largest margin the grid answers for; beyond it (and outside the
    /// grid) a column is further than that from every square.
    const REACH: i32 = 4;
    /// The largest grid, in columns (a mebibyte).
    const CELLS: u64 = 1 << 20;
    /// The side of a bucket, in columns.
    const BUCKET: i32 = 64;
    /// The farthest centre kept in buckets: far enough for any world border.
    const LIMIT: i32 = 1 << 26;
    fn matching<'a>(
        cover: &'a Option<ExtraCover>,
        extra: &[((i32, i32), i32)],
    ) -> Option<&'a ExtraCover> {
        cover.as_ref().filter(|c| c.extra == extra)
    }
    /// [`Self::matching`], told whether `cover` is known to match: the
    /// comparison reads every area, and `update` asks for each column it
    /// looks at (a server holds an area per player).
    fn current<'a>(
        cover: &'a Option<ExtraCover>,
        extra: &[((i32, i32), i32)],
        known: bool,
    ) -> Option<&'a ExtraCover> {
        if known {
            debug_assert!(cover.as_ref().is_none_or(|c| c.extra == extra));
            return cover.as_ref();
        }
        Self::matching(cover, extra)
    }
    /// Rebuild the coverage index if `extra` changed; none for one area or none
    /// (a scan is as quick).
    fn refresh(cover: &mut Option<ExtraCover>, extra: &[((i32, i32), i32)]) {
        if cover.as_ref().is_some_and(|c| c.extra == extra) {
            return;
        }
        *cover = Self::new(extra);
    }
    fn new(extra: &[((i32, i32), i32)]) -> Option<Self> {
        if extra.len() < 2
            || extra.iter().any(|&((x, z), r)| {
                !(0..=64).contains(&r) || x.abs() > Self::LIMIT || z.abs() > Self::LIMIT
            })
        {
            return None;
        }
        let mut buckets: FxHashMap<(i32, i32), Vec<u32>> = FxHashMap::default();
        let mut bucket_box = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for (i, &((x, z), _)) in extra.iter().enumerate() {
            let key = (x.div_euclid(Self::BUCKET), z.div_euclid(Self::BUCKET));
            buckets.entry(key).or_default().push(i as u32);
            bucket_box = (
                bucket_box.0.min(key.0),
                bucket_box.1.min(key.1),
                bucket_box.2.max(key.0),
                bucket_box.3.max(key.1),
            );
        }
        Some(Self {
            extra: extra.to_vec(),
            grid: CoverGrid::new(extra),
            buckets,
            bucket_box,
            radius: extra.iter().map(|&(_, r)| r).max().unwrap_or(0),
        })
    }
    /// Whether (x, z) is within `margin` of an area's square, for a
    /// `margin` up to `REACH`.
    fn within(&self, x: i32, z: i32, margin: i32) -> bool {
        if let Some(grid) = &self.grid {
            return grid.within(x, z, margin);
        }
        let reach = self.radius + margin;
        let (kx0, kx1) = (
            (x.saturating_sub(reach)).div_euclid(Self::BUCKET),
            (x.saturating_add(reach)).div_euclid(Self::BUCKET),
        );
        let (kz0, kz1) = (
            (z.saturating_sub(reach)).div_euclid(Self::BUCKET),
            (z.saturating_add(reach)).div_euclid(Self::BUCKET),
        );
        (kz0..=kz1).any(|kz| {
            (kx0..=kx1).any(|kx| {
                self.buckets.get(&(kx, kz)).is_some_and(|areas| {
                    areas.iter().any(|&i| {
                        let ((cx, cz), r) = self.extra[i as usize];
                        (i64::from(x) - i64::from(cx))
                            .abs()
                            .max((i64::from(z) - i64::from(cz)).abs())
                            <= i64::from(r + margin)
                    })
                })
            })
        })
    }
    /// The squared distance from (x, z) to the nearest area's centre.
    /// Buckets are searched ring by ring outwards: once the nearest found is
    /// no further than the next ring can be, the search stops.
    fn nearest_sq(&self, x: i32, z: i32) -> i64 {
        let d = |i: u32| {
            let (c, _) = self.extra[i as usize];
            let (dx, dz) = (i64::from(x) - i64::from(c.0), i64::from(z) - i64::from(c.1));
            dx * dx + dz * dz
        };
        let (qx, qz) = (x.div_euclid(Self::BUCKET), z.div_euclid(Self::BUCKET));
        let (x0, z0, x1, z1) = self.bucket_box;
        let last = (qx - x0)
            .abs()
            .max((qx - x1).abs())
            .max((qz - z0).abs())
            .max((qz - z1).abs());
        let mut best = i64::MAX;
        let mut looked = 0usize;
        for ring in 0..=last {
            let mut look = |kx: i32, kz: i32| {
                looked += 1;
                if let Some(areas) = self.buckets.get(&(kx, kz)) {
                    best = areas.iter().map(|&i| d(i)).fold(best, i64::min);
                }
            };
            if ring == 0 {
                look(qx, qz);
            } else {
                for k in -ring..=ring {
                    look(qx + k, qz - ring);
                    look(qx + k, qz + ring);
                }
                for k in 1 - ring..ring {
                    look(qx - ring, qz + k);
                    look(qx + ring, qz + k);
                }
            }
            // Every centre in a further ring is more than `ring` buckets away
            // on one axis.
            let bound = i64::from(ring) * i64::from(Self::BUCKET);
            if best <= bound * bound {
                break;
            }
            // Far from every area: a scan costs less than more empty rings.
            if looked > self.extra.len() * 4 {
                return (0..self.extra.len() as u32).map(d).fold(best, i64::min);
            }
        }
        best
    }
}
impl CoverGrid {
    /// None for areas too far apart for [`ExtraCover::CELLS`].
    fn new(extra: &[((i32, i32), i32)]) -> Option<Self> {
        let k = i64::from(ExtraCover::REACH);
        let (mut x0, mut z0, mut x1, mut z1) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
        for &((x, z), r) in extra {
            let (x, z, r) = (i64::from(x), i64::from(z), i64::from(r) + k);
            (x0, z0, x1, z1) = (x0.min(x - r), z0.min(z - r), x1.max(x + r), z1.max(z + r));
        }
        let (width, depth) = ((x1 - x0 + 1) as u64, (z1 - z0 + 1) as u64);
        if width * depth > ExtraCover::CELLS {
            return None;
        }
        let (x0, z0, width, depth) = (x0 as i32, z0 as i32, width as u32, depth as u32);
        let mut cells =
            vec![(ExtraCover::REACH + 1) as i8; (width * depth) as usize].into_boxed_slice();
        // Inside a square counts as 0: no margin asked for is below it. Each
        // row of a square is a run of one value with a slope at either end.
        let lower = |run: &mut [i8], v: i8| run.iter_mut().for_each(|c| *c = (*c).min(v));
        for &((cx, cz), r) in extra {
            let reach = r + ExtraCover::REACH;
            for dz in -reach..=reach {
                let start = (cz + dz - z0) as usize * width as usize + (cx - x0) as usize;
                let row = &mut cells[start - reach as usize..=start + reach as usize];
                let flat = dz.abs().max(r);
                let centre = reach as usize;
                lower(
                    &mut row[centre - flat as usize..=centre + flat as usize],
                    (flat - r) as i8,
                );
                for d in flat + 1..=reach {
                    let v = (d - r) as i8;
                    lower(&mut row[centre - d as usize..=centre - d as usize], v);
                    lower(&mut row[centre + d as usize..=centre + d as usize], v);
                }
            }
        }
        Some(Self {
            x0,
            z0,
            width,
            depth,
            cells,
        })
    }
    fn within(&self, x: i32, z: i32, margin: i32) -> bool {
        let (dx, dz) = (
            x.wrapping_sub(self.x0) as u32,
            z.wrapping_sub(self.z0) as u32,
        );
        dx < self.width
            && dz < self.depth
            && i32::from(self.cells[(dz * self.width + dx) as usize]) <= margin
    }
}
impl Ranges<'_> {
    /// `distance`, or the band's nearer priority close to it.
    fn priority(&self, x: i32, z: i32) -> i64 {
        let own = self.distance(x, z);
        match self.band {
            Some(band) if band.within(x, z, 4) => own.min(band.priority(x, z)),
            _ => own,
        }
    }
    fn player_distance(&self, x: i32, z: i32) -> i64 {
        let d = |c: (i32, i32)| {
            let (dx, dz) = (i64::from(x - c.0), i64::from(z - c.1));
            dx * dx + dz * dz
        };
        if let Some(cover) = self.cover {
            return d(self.center).min(cover.nearest_sq(x, z));
        }
        self.extra
            .iter()
            .map(|(c, _)| d(*c))
            .fold(d(self.center), i64::min)
    }

    /// How far past the view distance chunks are generated, and how far around
    /// a finished chunk (`near_finished`): decorated chunks need their
    /// neighbours generated too.
    fn margins(decorates: bool) -> (i32, i32) {
        if decorates { (3, 2) } else { (1, 1) }
    }
    /// Chunks generation asks for: loaded from the save, or generated as protos.
    fn requested(&self, x: i32, z: i32, decorates: bool) -> bool {
        let (margin, reach) = Self::margins(decorates);
        self.in_range(x, z, margin) || self.near_finished(x, z, reach)
    }
    /// Finished columns kept when they arrive: within two of the view distance
    /// or next to a finished chunk, and every column generation asked for (a
    /// saved one loads finished; dropping it had it asked for and loaded again
    /// every frame).
    fn chunk_kept(&self, x: i32, z: i32, decorates: bool) -> bool {
        self.in_range(x, z, 2) || self.near_finished(x, z, 1) || self.requested(x, z, decorates)
    }
    /// Columns unloaded: past every column kept.
    fn unloaded(&self, x: i32, z: i32, decorates: bool) -> bool {
        !self.in_range(x, z, 3) && !self.near_finished(x, z, Self::margins(decorates).1)
    }
    /// Whether a chunk finished here (`in_range(_, _, 1)`) lies within `reach`
    /// chunks of (x, z) on both axes: its decoration needs its 3x3 neighbours
    /// decorated, which need their own 3x3 neighbours generated. The ranges are
    /// circles, so a fixed margin alone misses the corners of those squares
    /// (render distance 2 never finished the chunks next to the player).
    fn near_finished(&self, x: i32, z: i32, reach: i32) -> bool {
        let gap = |d: i32| i64::from((d.abs() - reach).max(0));
        let (dx, dz) = (gap(x - self.center.0), gap(z - self.center.1));
        let r = i64::from(self.radius + 1);
        // The extra areas are squares: their finished square grown by `reach`.
        (self.radius >= 0 && dx * dx + dz * dz <= r * r)
            || self.in_extra(x, z, 1 + reach)
            || self.in_band(x, z, 1 + reach)
    }
    fn in_range(&self, x: i32, z: i32, margin: i32) -> bool {
        self.in_main_range(x, z, margin)
            || self.in_extra(x, z, margin)
            || self.in_band(x, z, margin)
    }
    /// Within `margin` of the pregeneration band.
    fn in_band(&self, x: i32, z: i32, margin: i32) -> bool {
        self.band.is_some_and(|b| b.within(x, z, margin))
    }
    /// Within `margin` of an extra area's square (Chebyshev distance, as
    /// vanilla's ticket levels spread).
    fn in_extra(&self, x: i32, z: i32, margin: i32) -> bool {
        if let Some(cover) = self.cover
            && (0..=ExtraCover::REACH).contains(&margin)
        {
            return cover.within(x, z, margin);
        }
        self.extra
            .iter()
            .any(|&(c, radius)| (x - c.0).abs().max((z - c.1).abs()) <= radius + margin)
    }
    /// The columns meshed for drawing: those vanilla draws around the
    /// player (`in_view_distance`), and forced ones.
    fn in_view(&self, x: i32, z: i32) -> bool {
        in_view_distance(self.center, self.radius, x, z) || self.forced.contains(&(x, z))
    }
    fn in_main_range(&self, x: i32, z: i32, margin: i32) -> bool {
        let r = i64::from(self.radius + margin);
        let (dx, dz) = (i64::from(x - self.center.0), i64::from(z - self.center.1));
        if self.radius >= 0 && dx * dx + dz * dz <= r * r {
            return true;
        }
        self.near_forced(x, z, margin)
    }
}
impl Drop for Streaming {
    fn drop(&mut self) {
        self.abort_generation();
        // Saves in flight hold proto chunks nothing else would save.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while self.saving > 0 {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match self.shared.done.recv_timeout(left) {
                Some(Done::Saved(_)) => self.saving -= 1,
                Some(_) => {}
                None => break,
            }
        }
        // `jobs` leaves the pool after this, once its running jobs are
        // done; the jobs still queued go with the queue.
    }
}
/// The queued jobs' priorities computed again from each one's column
/// `distance` (`Streaming::submit`).
fn rank_again(queue: &mut BinaryHeap<Queued>, distance: impl Fn(i32, i32) -> i64) {
    if queue.is_empty() {
        return;
    }
    let mut jobs = std::mem::take(queue).into_vec();
    for job in &mut jobs {
        if let Some((x, z)) = job.at {
            job.priority = distance(x, z) * 4 + job.bias;
        }
    }
    *queue = BinaryHeap::from(jobs);
}
/// `ChunkTaskPriorityQueue.release(pos, clearQueue=true)`: the jobs queued for the
/// `gone` columns are dropped, so a view that moved or shrank does not wait behind the
/// generation of the chunks it left. A job without a column (a save, meshes to free)
/// stays, as vanilla keeps the tasks of a chunk that is still loaded. Returns how many
/// jobs went.
fn release_columns(queue: &mut BinaryHeap<Queued>, gone: &FxHashSet<(i32, i32)>) -> usize {
    if queue.is_empty() || gone.is_empty() {
        return 0;
    }
    let queued = std::mem::take(queue).into_vec();
    let before = queued.len();
    let kept: Vec<Queued> = queued
        .into_iter()
        .filter(|job| !job.at.is_some_and(|at| gone.contains(&at)))
        .collect();
    let released = before - kept.len();
    *queue = BinaryHeap::from(kept);
    released
}
/// What `keep_unchanged_sections` found changed.
#[derive(Debug, Default, PartialEq)]
struct Resent {
    /// Any section's blocks changed (in a way light or the mesher sees).
    blocks: bool,
    /// The sections whose blocks did not change but whose biomes did.
    biomes: Vec<usize>,
    /// The sections whose blocks did not change but whose light the packet changes.
    light: Vec<usize>,
}
/// `new`, a resident column `old` sent again, takes `old`'s sections whose
/// blocks it holds unchanged, with `new`'s biomes and light; the sections whose blocks
/// changed keep `old`'s light (until lit again) and their cells' classes go in
/// `cells`.
fn keep_unchanged_sections(
    old: &Chunk,
    new: &mut Chunk,
    cells: &mut crate::light::LightChanges,
) -> Resent {
    let (mut a, mut b) = (Box::new([0u32; 4096]), Box::new([0u32; 4096]));
    let mut out = Resent::default();
    let (x, z) = (new.x, new.z);
    for s in 0..new.sections.len().min(old.sections.len()) {
        // (sections stored alike hold the same blocks: most of a column sent again, found without unpacking)
        let same = old.sections[s].blocks.same_storage(&new.sections[s].blocks) || {
            old.sections[s].unpack(&mut a);
            new.sections[s].unpack(&mut b);
            let classes = crate::light::LightChanges::classes_differing(&a, &b);
            if classes != 0 {
                cells.add((x, s as i32, z), classes);
            }
            classes == 0
        };
        if !same {
            out.blocks = true;
            continue;
        }
        let biomes = new.sections[s].biomes.clone();
        let changed = !same_values(&old.sections[s].biomes, &biomes);
        let light = (!same_light(&old.sections[s].light, &new.sections[s].light))
            .then(|| new.sections[s].light.clone());
        new.sections[s] = old.sections[s].clone();
        if changed {
            Arc::make_mut(&mut new.sections[s]).biomes = biomes;
            out.biomes.push(s);
        }
        if let Some(light) = light {
            Arc::make_mut(&mut new.sections[s]).light = light;
            out.light.push(s);
        }
    }
    out
}
/// Takes the server's new `chunk` over the resident column `old`: the sections
/// whose blocks did not change keep `old`'s (with `chunk`'s biomes and light),
/// and the sections whose blocks changed remesh. `relight` says the packet's
/// light is from before the server's relight (a Horizon server's bulk edit), so
/// the column and its neighbours are lit here; another server's packet already
/// carries the light vanilla applies, so no light job is queued for it.
fn take_resent_column(
    world: &mut World,
    key: (i32, i32),
    old: &Chunk,
    mut chunk: Chunk,
    relight: bool,
) {
    // The cells whose blocks changed, by section: they remesh with the neighbours within a block of them (as
    // vanilla's `setBlocksDirty`), not every section round each section changed (which, for the few blocks a
    // section often changes in a bulk edit, is mostly sections that did not).
    let mut cells = crate::light::LightChanges::default();
    cells.keyed(world.layout());
    let kept = keep_unchanged_sections(old, &mut chunk, &mut cells);
    chunk.light_loaded = old.light_loaded;
    chunk.lit = true;
    world.chunks.insert(key, Arc::new(chunk));
    world.revision = world.revision.wrapping_add(1);
    // A section with new biomes and the same blocks remeshes now (its tint), with its neighbours, whose tint
    // blends across the border: above and below too, as a block's tint can sample the biome quart of the
    // section next to it (`BiomeManager.getBiome`'s zoom reaches a quart beyond the block).
    Streaming::light_updated(world, key.0, key.1, &kept.biomes);
    // A section with the same blocks and new light takes the packet's light (vanilla's
    // `handleLevelChunkWithLight` always applies it) and remeshes as a light update would.
    Streaming::light_updated(world, key.0, key.1, &kept.light);
    if !kept.blocks {
        return;
    }
    if relight {
        for dz in -1..=1 {
            for dx in -1..=1 {
                world.dirty_light.insert((key.0 + dx, key.1 + dz));
            }
        }
    }
    cells.dirty_sections(|section| {
        world.dirty_sections.insert(section);
    });
}

/// Whether two sections' packed light (4,096 cells) holds the same values.
fn same_light(a: &crate::chunk::Light, b: &crate::chunk::Light) -> bool {
    a.same_as(b)
}
/// Whether two biome containers (64 cells) hold the same values.
fn same_values(
    a: &horizon_core::paletted::PalettedContainer,
    b: &horizon_core::paletted::PalettedContainer,
) -> bool {
    if a.same_storage(b) {
        return true;
    }
    let (mut x, mut y) = ([0u32; 64], [0u32; 64]);
    a.unpack(&mut x);
    b.unpack(&mut y);
    x == y
}

/// The most changed blocks one frame relights itself (`Streaming::relight_edits`): a 16^3
/// section's worth. Past it, the columns are lit again whole on the workers.
const INLINE_RELIGHT_MAX: usize = 4096;

/// Splits edits at `positions` into the columns to light again whole and the
/// edits to relight in place. An edited column is lit whole with its
/// neighbours (light reaches 15 blocks, never past the next column) when all
/// nine are `ready` to be lit whole; otherwise its edits stay in place.
fn whole_relight_columns(
    positions: Vec<(i32, i32, i32)>,
    ready: impl Fn(i32, i32) -> bool,
) -> (FxHashSet<(i32, i32)>, Vec<(i32, i32, i32)>) {
    let around =
        |(x, z): (i32, i32)| (-1..=1).flat_map(move |dz| (-1..=1).map(move |dx| (x + dx, z + dz)));
    let touched: FxHashSet<(i32, i32)> = positions
        .iter()
        .map(|&(x, _, z)| (x >> 4, z >> 4))
        .collect();
    let whole: FxHashSet<(i32, i32)> = touched
        .into_iter()
        .filter(|&column| around(column).all(|(x, z)| ready(x, z)))
        .collect();
    let columns = whole.iter().flat_map(|&column| around(column)).collect();
    let inline = positions
        .into_iter()
        .filter(|&(x, _, z)| !whole.contains(&(x >> 4, z >> 4)))
        .collect();
    (columns, inline)
}

/// Drops the pending sections (`meshing`, `compile_started`, `waiting`) of
/// the `gone` columns: one pass over each store when that is cheaper than a
/// lookup per section of every column (a far teleport at render distance 32
/// forgets ~3,700 columns, 1.7 ms of lookups), else by key.
fn forget_pending_sections(
    meshing: &mut FxHashMap<(i32, i32, i32), u64>,
    compile_started: &mut FxHashMap<(i32, i32, i32), u64>,
    waiting: &mut FxHashSet<(i32, i32, i32)>,
    gone: &FxHashSet<(i32, i32)>,
    sections: i32,
) {
    let lookups = gone.len().saturating_mul(sections.max(0) as usize);
    if lookups > meshing.len() + compile_started.len() + waiting.len() {
        let kept = |&(x, _, z): &(i32, i32, i32)| !gone.contains(&(x, z));
        meshing.retain(|k, _| kept(k));
        compile_started.retain(|k, _| kept(k));
        waiting.retain(kept);
    } else {
        for &(x, z) in gone {
            for sy in 0..sections {
                meshing.remove(&(x, sy, z));
                compile_started.remove(&(x, sy, z));
                waiting.remove(&(x, sy, z));
            }
        }
    }
}

/// Orders dirty section indices of a column so the visible surface section
/// (highest non-air section) and its immediate neighbours mesh first, before
/// deep underground caves and empty sky sections. Returns how many indices were
/// written into `out`.
fn order_column_sections(
    sections: SectionMask,
    center: Option<&Chunk>,
    air: u32,
    out: &mut [i32; SectionMask::CAPACITY],
) -> usize {
    let mut count = 0usize;
    for sy in sections.iter() {
        out[count] = sy as i32;
        count += 1;
    }
    if count > 1
        && let Some(chunk) = center
        && let Some(surface) = chunk.sections.iter().rposition(|s| !s.is_uniform(air))
    {
        let surface = surface as i32;
        out[..count].sort_unstable_by_key(|&sy| ((sy - surface).abs(), sy));
    }
    count
}

/// Submit up to `room` dirty sections from the nearest candidate columns.
/// Enforces the section budget inside each column's section loop and re-marks
/// any unsubmitted sections dirty so no section is lost after `take_column`.
fn submit_mesh_candidates(
    world: &mut World,
    candidates: &mut Vec<(i64, i32, i32, SectionMask)>,
    room: usize,
    settings: &MeshSettings,
    meshed: &mut FxHashSet<(i32, i32)>,
    meshing: &mut FxHashMap<(i32, i32, i32), u64>,
    sequence: &mut u64,
    order: &mut u64,
    queue: &mut BinaryHeap<Queued>,
) -> usize {
    if room == 0 || candidates.is_empty() {
        return 0;
    }
    // `room` is a section budget, while `candidates` holds columns (each
    // up to 24 sections). Bound selection to at most `room` columns, then
    // stop submitting once `remaining` sections are filled so we never
    // queue thousands of sections and pin hundreds of 3x3 `Region` snapshots
    // in a single frame.
    nearest_first(candidates, room);
    let air = world.air.0;
    let mut remaining = room;
    let mut submitted = 0usize;
    let mut order_buf = [0i32; SectionMask::CAPACITY];
    for &(dist, x, z, sections) in &*candidates {
        if remaining == 0 {
            break;
        }
        let Some(region) = Streaming::region(world, x, z) else {
            continue;
        };
        world.dirty_sections.take_column(x, z);
        let count = order_column_sections(sections, region[1][1].as_deref(), air, &mut order_buf);
        if count == 0 {
            continue;
        }
        let take = count.min(remaining);
        let first = if take == count {
            meshed.insert((x, z))
        } else {
            !meshed.contains(&(x, z))
        };
        let bias = if first { 2 } else { 0 };
        let priority = dist * 4 + bias;
        for &sy in &order_buf[..take] {
            *sequence += 1;
            *order += 1;
            meshing.insert((x, sy, z), *sequence);
            queue.push(Queued {
                priority,
                at: Some((x, z)),
                bias,
                order: *order,
                job: Job::Mesh(x, sy, z, region.clone(), *sequence, settings.clone()),
            });
            submitted += 1;
            remaining -= 1;
        }
        if take < count {
            let mut leftover = SectionMask::EMPTY;
            for &sy in &order_buf[take..count] {
                leftover.insert(sy as usize);
            }
            world.dirty_sections.insert_column_mask((x, z), leftover);
        }
    }
    submitted
}

/// Drop unsaved far proto chunks and clear their `decorated` markers when they
/// are neither finishing nor resident, returning those that must be saved first.
fn drop_unsaved_protos(
    far: Vec<(i32, i32)>,
    written: &mut FxHashSet<(i32, i32)>,
    protos: &mut FxHashMap<(i32, i32), Option<Proto>>,
    light_views: &mut FxHashMap<(i32, i32), Arc<Chunk>>,
    decorated: &mut FxHashSet<(i32, i32)>,
    finishing: &FxHashSet<(i32, i32)>,
    world: &World,
) -> Vec<(i32, i32)> {
    let mut to_save = Vec::new();
    for key in far {
        if written.remove(&key) {
            to_save.push(key);
        } else {
            protos.remove(&key);
            light_views.remove(&key);
            if !finishing.contains(&key) && !world.chunks.contains_key(&key) {
                decorated.remove(&key);
            }
        }
    }
    to_save
}

/// Take save ownership before dispatch, so no generation job can mutate the view.
fn checkout_protos(
    protos: &mut FxHashMap<(i32, i32), Option<Proto>>,
    light_views: &mut FxHashMap<(i32, i32), Arc<Chunk>>,
    keys: impl IntoIterator<Item = (i32, i32)>,
) -> Vec<ProtoSave> {
    keys.into_iter()
        .filter_map(|pos| {
            let proto = protos.get_mut(&pos)?.take()?;
            light_views.remove(&pos);
            Some(ProtoSave { pos, proto })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The queue hands out the wanted chunks in the order the full sort did
    /// (every chunk measured, sorted by distance, the new ones ranked in that
    /// order, then all sorted by rank), and ranks the new ones the same way,
    /// whatever order the chunks are listed in.
    #[test]
    fn retain_drops_out_of_range_generating_ranks_before_pregen() {
        let forced = HashSet::new();
        let empty_protos: FxHashMap<(i32, i32), Option<Proto>> = Default::default();
        let mut order = DecorationOrder::default();
        // Speculative or pre-teleport ranks around (0, 0).
        order.reassign((0, 0), 4, |_, _| false);

        // Centre moves far away to (-33, 4); (0, 0) jobs still in flight are
        // dropped on arrival, so retain must not preserve their stale ranks.
        order.reassign((-33, 4), 4, |_, _| false);
        let spawn_ranges = Ranges {
            center: (-33, 4),
            radius: 4,
            extra: &[],
            forced: &forced,
            cover: None,
            forced_cover: None,
            band: None,
        };
        retain_decoration_order(&mut order, &spawn_ranges, &empty_protos);
        for (x, z) in [(0, 0), (-1, 0), (0, -1), (1, 0)] {
            assert_eq!(order.rank(x, z), None, "stale rank leaked for ({x}, {z})");
        }

        // A subsequent `/pregen start 1 square 16 0` ranks [-3..=5, -4..=4] row
        // by row; its 5x5 decoration neighbourhood must pop strictly row by row.
        let band = Band {
            x0: 1,
            x1: 1,
            z0: 0,
            z1: 0,
            circle: None,
        };
        const REACH: i32 = 4;
        for z in band.z0 - REACH..=band.z1 + REACH {
            for x in band.x0 - REACH..=band.x1 + REACH {
                order.rank_or_append(x, z);
            }
        }
        let expected: Vec<(i32, i32)> = (-2..=2)
            .flat_map(|z| (-1..=3).map(move |x| (x, z)))
            .collect();
        let mut q = decoration_queue(&mut order, expected.iter().copied(), |_, _| 0);
        let popped: Vec<(i32, i32)> = std::iter::from_fn(|| q.pop())
            .map(|std::cmp::Reverse((_, x, z))| (x, z))
            .collect();
        assert_eq!(popped, expected);
    }

    #[test]
    fn waits_for_blocks_overlapping_neighbours_beside_final_chunks_without_deadlock() {
        let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
        // Existing final chunks along a seam (e.g. spawn border or prior pregen strip).
        for x in -35..=-31 {
            world.chunks.insert((x, -2), Arc::new(Chunk::empty(x, -2)));
        }
        let forced = HashSet::new();
        let band = Band {
            x0: -35,
            x1: -31,
            z0: -5,
            z1: -3,
            circle: None,
        };
        let ranges = Ranges {
            center: (-33, 4),
            radius: 4,
            extra: &[],
            forced: &forced,
            cover: None,
            forced_cover: None,
            band: Some(&band),
        };
        let mut order = DecorationOrder::default();
        for z in band.z0 - 4..=band.z1 + 4 {
            for x in band.x0 - 4..=band.x1 + 4 {
                order.rank_or_append(x, z);
            }
        }

        // Everything ranked before (-33, -3) is already decorated.
        let pivot = order.rank(-33, -3).unwrap();
        let mut decorated: FxHashSet<(i32, i32)> = order
            .ordered()
            .into_iter()
            .filter(|&(x, z)| order.rank(x, z).unwrap() < pivot)
            .collect();
        let protos: FxHashMap<(i32, i32), Option<Proto>> = Default::default();
        let generating: FxHashSet<(i32, i32)> = Default::default();

        // With saves_beside_final = false, a chunk beside a final chunk is skipped
        // and must not block higher-ranked neighbours.
        let wait_without = DecorationWait {
            ranges: &ranges,
            world: &world,
            decorated: &decorated,
            protos: &protos,
            generating: &generating,
            saves_beside_final: false,
        };
        assert!(!wait_without.waits_for(-33, -3));
        assert!(!order.blocked(-32, -3, |nx, nz| wait_without.waits_for(nx, nz)));

        // With saves_beside_final = true (VanillaGen), (-33, -3) WILL decorate beside
        // final chunk (-33, -2), so it must hold back overlapping (-32, -3).
        let wait_with = DecorationWait {
            ranges: &ranges,
            world: &world,
            decorated: &decorated,
            protos: &protos,
            generating: &generating,
            saves_beside_final: true,
        };
        assert!(wait_with.waits_for(-33, -3));
        assert!(order.blocked(-32, -3, |nx, nz| wait_with.waits_for(nx, nz)));

        // Deadlock-freedom: every remaining wanted chunk (including all chunks beside
        // the final seam) makes progress to completion in strictly non-decreasing rank order.
        let mut remaining: Vec<(i32, i32)> = (-7..=-1)
            .flat_map(|z| (-37..=-29).map(move |x| (x, z)))
            .filter(|&(x, z)| !decorated.contains(&(x, z)) && !world.chunks.contains_key(&(x, z)))
            .collect();
        let mut completed = Vec::new();
        while !remaining.is_empty() {
            let wait = DecorationWait {
                ranges: &ranges,
                world: &world,
                decorated: &decorated,
                protos: &protos,
                generating: &generating,
                saves_beside_final: true,
            };
            let next = remaining
                .iter()
                .copied()
                .filter(|&(x, z)| !order.blocked(x, z, |nx, nz| wait.waits_for(nx, nz)))
                .min_by_key(|&(x, z)| order.rank(x, z).unwrap())
                .expect("at least one unblocked chunk must exist (no deadlock)");
            decorated.insert(next);
            completed.push(next);
            remaining.retain(|&c| c != next);
        }
        // Every adjacent pair along the seam decorated in rank order.
        let pos_a = completed.iter().position(|&c| c == (-33, -3)).unwrap();
        let pos_b = completed.iter().position(|&c| c == (-32, -3)).unwrap();
        assert!(pos_a < pos_b);
    }

    #[test]
    fn decoration_queue_pops_in_the_order_a_full_sort_gives() {
        // Distinct for every chunk, so the full sort has one answer.
        let distance = |x: i32, z: i32| i64::from(x) * 100_000 + i64::from(z);
        let full_sort = |order: &mut DecorationOrder, wanted: &[(i32, i32)]| {
            let mut w: Vec<(u64, i64, i32, i32)> = wanted
                .iter()
                .map(|&(x, z)| (0, distance(x, z), x, z))
                .collect();
            w.sort_unstable_by_key(|w| w.1);
            for w in &mut w {
                w.0 = order.rank_or_append(w.2, w.3);
            }
            w.sort_unstable();
            w.into_iter().map(|w| (w.2, w.3)).collect::<Vec<_>>()
        };
        let ranked = |order: &mut DecorationOrder| {
            order.reassign((0, 0), 6, |x, z| (x + z).rem_euclid(5) == 0);
        };
        let (mut fast, mut full) = (DecorationOrder::default(), DecorationOrder::default());
        ranked(&mut fast);
        ranked(&mut full);
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut shuffled = |mut keys: Vec<(i32, i32)>| {
            for i in (1..keys.len()).rev() {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                keys.swap(i, (seed % (i as u64 + 1)) as usize);
            }
            keys
        };
        // Some of the player's ranked chunks, and an extra area's that no
        // ticket reached; the second time, more of the area, the first part
        // ranked by then.
        let mut wanted: Vec<(i32, i32)> = (-6i32..=6)
            .flat_map(|x| (-6..=6).map(move |z| (x, z)))
            .filter(|&(x, z)| (x * 7 + z).rem_euclid(3) != 0)
            .collect();
        wanted.extend((20..=24).flat_map(|x| (-3..=3).map(move |z| (x, z))));
        for round in 0..2 {
            if round == 1 {
                wanted.extend((25..=27).flat_map(|x| (-3..=3).map(move |z| (x, z))));
            }
            let wanted = shuffled(wanted.clone());
            let mut queue = decoration_queue(&mut fast, wanted.iter().copied(), distance);
            let popped: Vec<(i32, i32)> = std::iter::from_fn(|| queue.pop())
                .map(|std::cmp::Reverse((_, x, z))| (x, z))
                .collect();
            assert_eq!(popped, full_sort(&mut full, &wanted), "round {round}");
            assert_eq!(fast.ordered(), full.ordered(), "round {round}");
        }
    }

    #[test]
    fn taking_remote_columns_keeps_their_data_until_cleanup() {
        let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
        world.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        world.chunks.insert((1, 0), Arc::new(Chunk::empty(1, 0)));
        world
            .block_entities
            .insert((0, 64, 0), crate::containers::BlockEntity::Chest(vec![]));
        let gone = [(0, 0), (2, 0)].into_iter().collect();
        let chunks = Streaming::take_remote_columns(&mut world, &gone);
        assert_eq!(chunks.len(), 1);
        assert_eq!((chunks[0].x, chunks[0].z), (0, 0));
        assert!(!world.chunks.contains_key(&(0, 0)));
        assert!(world.chunks.contains_key(&(1, 0)));
        assert!(world.block_entities.contains_key(&(0, 64, 0)));
        assert!(Streaming::take_remote_columns(&mut world, &gone).is_empty());
    }

    #[test]
    #[ignore = "daily"]
    fn late_generated_saves_yield_before_writing_in_all_receive_loops() {
        use crate::{
            assets::Assets, biome::Biomes, light::LightTables, mesher::MeshTables, models,
        };
        use horizon_items::blocks::BlockFacts;
        use horizon_world::registry::BlockStateId;
        use std::path::PathBuf;

        #[derive(Default)]
        struct Generator(std::sync::Mutex<Vec<(i32, i32)>>);
        impl ChunkGenerator for Generator {
            type Worker = ();
            fn worker(&self) {}
            fn proto(&self, _: &mut (), _: i32, _: i32) -> Proto {
                Box::new(())
            }
            fn finish(&self, _: Proto) -> Chunk {
                Chunk::empty(0, 0)
            }
            fn save_finished(&self, chunk: Chunk) {
                self.0.lock().unwrap().push((chunk.x, chunk.z));
            }
        }
        let root = PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let assets = Arc::new(Assets::load(&root).unwrap());
        let render = Arc::new(
            assets
                .reload(&horizon_assets::resource_source::ResourceRoot::directory(
                    root.to_path_buf(),
                ))
                .unwrap(),
        );
        let facts = BlockFacts::load(&root, assets.registry).unwrap();
        let visibility = models::load_visibility(&root, assets.registry).unwrap();
        let context = Arc::new(WorkerContext {
            tables: Arc::new(MeshTables::new(&assets, &render, &facts, visibility, None).unwrap()),
            light: Arc::new(LightTables::load(&root, assets.registry, &facts).unwrap()),
            biomes: Arc::new(
                Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
                    root.to_path_buf(),
                ))
                .unwrap(),
            ),
            assets,
            render,
            hidden_states: Arc::from([]),
            part_tables: Default::default(),
        });
        let generator = Arc::new(Generator::default());
        let mut streaming = Streaming::new(generator.clone(), context, 1);
        streaming.radius = 0;
        let mut world = World::new(BlockStateId(0), 0);
        let late = |x| {
            let mut chunk = Chunk::empty(x, 0);
            chunk.fresh = true;
            Done::Generated(chunk)
        };
        streaming.finishing.extend([(100, 0), (101, 0)]);
        streaming.shared.done.send(late(100));
        streaming.shared.done.send(late(101));
        let PlanStep::Save { chunk, resume } = streaming.begin_plan_update(&mut world) else {
            panic!("first plan yield")
        };
        assert!(generator.0.lock().unwrap().is_empty());
        streaming.save_finished(chunk);
        let PlanStep::Save { chunk, resume } = streaming.resume_plan_update(&mut world, resume)
        else {
            panic!("second plan yield")
        };
        assert_eq!(*generator.0.lock().unwrap(), [(100, 0)]);
        streaming.save_finished(chunk);
        assert!(matches!(
            streaming.resume_plan_update(&mut world, resume),
            PlanStep::Ready(_)
        ));
        streaming.finishing.extend([(102, 0), (104, 0)]);
        streaming.shared.done.send(late(102));
        streaming.shared.done.send(late(104));
        let before = std::time::Instant::now();
        let poll_deadline = before + std::time::Duration::from_millis(2);
        let SaveStep::Full(chunk) =
            streaming.prepare_poll_save_with_clock(&mut world, poll_deadline, || before)
        else {
            panic!("poll yield")
        };
        assert_eq!(*generator.0.lock().unwrap(), [(100, 0), (101, 0)]);
        streaming.save_finished(chunk);
        assert!(matches!(
            streaming.prepare_poll_save_with_clock(&mut world, poll_deadline, || poll_deadline),
            SaveStep::Pending
        ));
        let next_deadline = poll_deadline + std::time::Duration::from_millis(2);
        let SaveStep::Full(chunk) =
            streaming.prepare_poll_save_with_clock(&mut world, next_deadline, || poll_deadline)
        else {
            panic!("next tick yield")
        };
        assert_eq!(*generator.0.lock().unwrap(), [(100, 0), (101, 0), (102, 0)]);
        streaming.save_finished(chunk);
        streaming.finishing.insert((103, 0));
        streaming.shared.done.send(late(103));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        let SaveStep::Full(chunk) = streaming.prepare_save(&mut world, true, deadline) else {
            panic!("blocking yield")
        };
        assert_eq!(
            *generator.0.lock().unwrap(),
            [(100, 0), (101, 0), (102, 0), (104, 0)]
        );
        streaming.save_finished(chunk);
        assert!(matches!(
            streaming.prepare_save(&mut world, true, deadline),
            SaveStep::Protos(_)
        ));
        assert_eq!(
            *generator.0.lock().unwrap(),
            [(100, 0), (101, 0), (102, 0), (104, 0), (103, 0)]
        );
    }

    #[test]
    fn checked_out_proto_is_the_read_only_state_given_to_the_save_job() {
        struct Access;
        impl ProtoAccess for Access {
            fn status(&self, _: &Proto) -> Option<horizon_world::chunk_status::ChunkStatus> {
                Some(horizon_world::chunk_status::ChunkStatus::Features)
            }
            fn block(&self, proto: &Proto, _: [i32; 3]) -> Option<u32> {
                proto.downcast_ref::<u32>().copied()
            }
        }
        let mut protos = FxHashMap::default();
        protos.insert((2, -1), Some(Box::new(73_u32) as Proto));
        protos.insert((3, -1), None);
        let mut views = FxHashMap::default();
        views.insert((2, -1), Arc::new(Chunk::empty(2, -1)));
        let saves = checkout_protos(&mut protos, &mut views, [(2, -1), (3, -1), (4, -1)]);
        assert_eq!(saves.len(), 1);
        assert!(protos[&(2, -1)].is_none());
        assert!(!views.contains_key(&(2, -1)));
        let snapshot = ProtoSnapshot {
            dimension: "minecraft:overworld",
            pos: [saves[0].pos.0, saves[0].pos.1],
            proto: &saves[0].proto,
            access: &Access,
            status: Access.status(&saves[0].proto).unwrap(),
        };
        assert_eq!(
            snapshot.status,
            horizon_world::chunk_status::ChunkStatus::Features
        );
        assert_eq!(snapshot.block([32, 0, -16]), Some(73));
        let Job::Save(saved) = Job::Save(saves) else {
            unreachable!()
        };
        assert_eq!(saved[0].proto.downcast_ref::<u32>(), Some(&73));
    }

    #[test]
    fn unload_plan_keeps_dirty_chunks_live_until_the_snapshot() {
        let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
        world.chunks.insert((10, 0), Arc::new(Chunk::empty(10, 0)));
        world.chunks.insert((11, 0), Arc::new(Chunk::empty(11, 0)));
        world.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        world.modified.insert((10, 0));
        let plan = plan_unloads(&world, |x, _| x > 5);
        assert!(world.chunks.contains_key(&(10, 0)));
        let saved = world.chunks.get(&(10, 0)).unwrap().clone();
        let unloaded = commit_unloads(&mut world, &plan);
        assert_eq!((saved.x, saved.z), (10, 0));
        assert_eq!(unloaded.len(), 2);
        assert!(!world.chunks.contains_key(&(10, 0)));
        assert!(!world.chunks.contains_key(&(11, 0)));
        assert!(world.chunks.contains_key(&(0, 0)));
    }

    #[test]
    fn a_resent_column_keeps_its_sections_but_takes_new_biomes() {
        let mut old = Chunk::empty(2, -3);
        old.set(1, 10, 1, 1);
        old.set(1, 40, 1, 1);
        old.set_biome_cell(0, 0, 0, 1);
        let mut new = old.clone();
        // a new biome under the same blocks (section 0), and a block changed (section 6)
        new.set_biome_cell(0, 0, 0, 3);
        new.set(1, 40, 1, 2);
        let mut cells = crate::light::LightChanges::default();
        cells.keyed(crate::chunk::ColumnLayout::VANILLA);
        let kept = keep_unchanged_sections(&old, &mut new, &mut cells);
        assert_eq!(
            kept,
            Resent {
                blocks: true,
                biomes: vec![0],
                light: vec![],
            }
        );
        assert_eq!(new.biome(0, -64, 0), 3);
        assert_eq!(new.block(1, -54, 1).0, 0);
        assert_eq!(new.block(1, 40, 1).0, 2);
        // the sections alike are shared with the old column
        assert!(Arc::ptr_eq(&new.sections[1], &old.sections[1]));
        assert!(!Arc::ptr_eq(&new.sections[0], &old.sections[0]));
        // nothing changed: nothing to do
        let mut same = old.clone();
        let kept = keep_unchanged_sections(&old, &mut same, &mut cells);
        assert_eq!(kept, Resent::default());
        assert!(Arc::ptr_eq(&same.sections[0], &old.sections[0]));
        // the same blocks under new light: the packet's light, as vanilla applies it
        let mut lit = old.clone();
        Arc::make_mut(&mut lit.sections[2]).light = crate::chunk::Light::Uniform(0xF0);
        Arc::make_mut(&mut old.sections[2]).light = crate::chunk::Light::Uniform(0x50);
        let kept = keep_unchanged_sections(&old, &mut lit, &mut cells);
        assert_eq!(
            kept,
            Resent {
                light: vec![2],
                ..Resent::default()
            }
        );
        assert_eq!(lit.sections[2].light.get(0), 0xF0);
        assert!(Arc::ptr_eq(&lit.sections[1], &old.sections[1]));
    }

    #[test]
    fn mesh_pool_reuses_capacity_with_clean_geometry_state() {
        let mut mesh = SectionMesh::default();
        mesh.solid.resize(64, bytemuck::Zeroable::zeroed());
        mesh.translucent.resize(16, bytemuck::Zeroable::zeroed());
        mesh.block_entities.push(([1, 2, 3], 7));
        mesh.lights.push(([1, 2, 3], 7));
        mesh.visibility = 0;
        mesh.sides = crate::mesher::Sides::unsorted(16);
        let capacities = mesh.capacity_bytes();
        let pointers = (mesh.solid.as_ptr(), mesh.translucent.as_ptr());
        let mut pool = MeshPool::default();
        assert!(pool.put(mesh).is_none());
        assert_eq!(pool.bytes, capacities);
        let reused = pool.take();
        assert_eq!(reused.capacity_bytes(), capacities);
        assert_eq!(
            pointers,
            (reused.solid.as_ptr(), reused.translucent.as_ptr())
        );
        assert!(reused.solid.is_empty());
        assert!(reused.translucent.is_empty());
        assert!(reused.block_entities.is_empty());
        assert!(reused.lights.is_empty());
        assert_eq!(reused.visibility, crate::mesher::ALL_VISIBLE);
        assert_eq!(reused.sides, crate::mesher::Sides::default());
        assert_eq!(pool.bytes, 0);
    }

    #[test]
    fn mesh_pool_caps_retained_count_and_bytes() {
        let mut pool = MeshPool::default();
        for _ in 0..MeshPool::COUNT {
            let mut mesh = SectionMesh::default();
            mesh.solid.reserve_exact(1);
            assert!(pool.put(mesh).is_none());
        }
        let mut extra = SectionMesh::default();
        extra.solid.reserve_exact(1);
        assert!(pool.put(extra).is_some());
        for _ in 0..MeshPool::COUNT {
            pool.take();
        }
        assert_eq!(pool.bytes, 0);
        for i in 0..2 {
            let mut mesh = SectionMesh::default();
            mesh.solid.reserve_exact(
                MeshPool::BYTES / std::mem::size_of::<crate::mesher::TerrainVertex>() / 2 + 1,
            );
            assert_eq!(pool.put(mesh).is_some(), i == 1);
            assert!(pool.bytes <= MeshPool::BYTES);
            assert_eq!(pool.meshes.len(), 1);
        }
        pool.take();
        assert_eq!(pool.bytes, 0);
        let mut huge = SectionMesh::default();
        huge.solid.reserve_exact(
            MeshPool::BYTES / std::mem::size_of::<crate::mesher::TerrainVertex>() + 1,
        );
        assert!(pool.put(huge).is_some());
        assert!(pool.meshes.is_empty());
        assert_eq!(pool.bytes, 0);
    }

    /// The frame never waits for a worker: while one holds the mailbox, the
    /// frame takes nothing and finds the results, in order, the next time.
    #[test]
    fn the_frame_takes_finished_jobs_without_waiting() {
        let mailbox = Mailbox::default();
        let lit = |seq| Done::Lit(0, 0, Vec::new(), seq);
        mailbox.send(lit(1));
        mailbox.send(lit(2));
        let sending = mailbox.done.lock().unwrap();
        assert!(mailbox.take().is_empty());
        assert!(mailbox.try_recv().is_none());
        drop(sending);
        let seqs: Vec<u64> = mailbox
            .take()
            .into_iter()
            .map(|done| match done {
                Done::Lit(_, _, _, seq) => seq,
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(seqs, [1, 2]);
        assert!(mailbox.take().is_empty());
        let waiting = std::time::Instant::now();
        assert!(
            mailbox
                .recv_timeout(std::time::Duration::from_millis(10))
                .is_none()
        );
        assert!(waiting.elapsed() >= std::time::Duration::from_millis(10));
    }

    #[test]
    fn mailbox_take_into_reuses_buffer_and_preserves_order() {
        let mailbox = Mailbox::default();
        let lit = |seq| Done::Lit(0, 0, Vec::new(), seq);
        mailbox.send(lit(10));
        mailbox.send(lit(20));
        let mut scratch = VecDeque::with_capacity(8);
        let sending = mailbox.done.lock().unwrap();
        mailbox.take_into(&mut scratch);
        assert!(scratch.is_empty());
        drop(sending);
        mailbox.take_into(&mut scratch);
        let seqs: Vec<u64> = scratch
            .drain(..)
            .map(|done| match done {
                Done::Lit(_, _, _, seq) => seq,
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(seqs, [10, 20]);
        assert!(scratch.capacity() > 0);
        // Taking from an empty mailbox leaves scratch empty without disturbing capacity.
        let cap = scratch.capacity();
        mailbox.take_into(&mut scratch);
        assert!(scratch.is_empty());
        assert_eq!(scratch.capacity(), cap);
    }

    #[test]
    fn order_column_sections_prioritizes_surface_section_first() {
        let mut chunk = Chunk::empty(0, 0);
        // Place ocean surface block in section 7 (y = 62 -> sy = (62 - (-64)) / 16 = 7).
        chunk.set(0, 62, 0, 1);
        let mut buf = [0i32; SectionMask::CAPACITY];
        let count = order_column_sections(SectionMask::below(24), Some(&chunk), 0, &mut buf);
        assert_eq!(count, 24);
        assert_eq!(buf[0], 7);
        assert_eq!(&buf[1..3], &[6, 8]);
        assert_eq!(&buf[3..5], &[5, 9]);
        // All-air column falls back to ascending section order.
        let empty = Chunk::empty(0, 0);
        let count = order_column_sections(SectionMask::below(4), Some(&empty), 0, &mut buf);
        assert_eq!(&buf[..count], &[0, 1, 2, 3]);
    }

    #[test]
    fn dropping_unsaved_far_protos_clears_decorated_markers() {
        let world = World::new(horizon_world::registry::BlockStateId(0), 0);
        let mut written = FxHashSet::from_iter([(11, 0)]);
        let mut protos: FxHashMap<(i32, i32), Option<Proto>> = FxHashMap::from_iter([
            ((10, 0), Some(Box::new(()) as Proto)),
            ((11, 0), Some(Box::new(()) as Proto)),
            ((12, 0), Some(Box::new(()) as Proto)),
        ]);
        let mut light_views = FxHashMap::from_iter([((10, 0), Arc::new(Chunk::empty(10, 0)))]);
        let mut decorated = FxHashSet::from_iter([(10, 0), (11, 0), (12, 0)]);
        let finishing = FxHashSet::from_iter([(12, 0)]);

        let to_save = drop_unsaved_protos(
            vec![(10, 0), (11, 0), (12, 0)],
            &mut written,
            &mut protos,
            &mut light_views,
            &mut decorated,
            &finishing,
            &world,
        );

        assert_eq!(to_save, vec![(11, 0)]);
        assert!(!protos.contains_key(&(10, 0)));
        assert!(!light_views.contains_key(&(10, 0)));
        assert!(!decorated.contains(&(10, 0)));
        assert!(protos.contains_key(&(11, 0)));
        assert!(decorated.contains(&(11, 0)));
        assert!(!protos.contains_key(&(12, 0)));
        assert!(decorated.contains(&(12, 0)));
    }

    #[test]
    fn submit_mesh_candidates_respects_section_budget_and_keeps_unsubmitted_dirty() {
        let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
        for z in -1..=1 {
            for x in -1..=2 {
                let mut chunk = Chunk::empty(x, z);
                if (x, z) == (0, 0) {
                    // Surface block in section 7 (y = 62 -> sy = 7).
                    chunk.set(0, 62, 0, 1);
                }
                world.chunks.insert((x, z), Arc::new(chunk));
            }
        }
        world
            .dirty_sections
            .insert_column_mask((0, 0), SectionMask::below(24));
        world
            .dirty_sections
            .insert_column_mask((1, 0), SectionMask::below(4));

        let settings = MeshSettings {
            nether: false,
            shader_pack_lighting: false,
            separate_ao: false,
            shader_pack: false,
            looks: false,
            zoom: BiomeZoom::new(0),
            blend_radius: 0,
            smooth_lighting: true,
            cutout_leaves: true,
            opaque_ice: false,
            translucent_layers: Arc::from([]),
            block_layers: Arc::default(),
            leaf_culling: LeafCullingMode::Off,
        };
        let mut meshed = FxHashSet::default();
        let mut meshing = FxHashMap::default();
        let mut sequence = 0u64;
        let mut order = 0u64;
        let mut queue = BinaryHeap::new();

        let drain_sections = |queue: &mut BinaryHeap<Queued>| -> Vec<(i32, i32, i32)> {
            let mut out = Vec::new();
            while let Some(item) = queue.pop() {
                if let Job::Mesh(x, sy, z, _, _, _) = item.job {
                    out.push((x, sy, z));
                }
            }
            out
        };

        // Pass 1: room (3) < column (0, 0)'s 24 dirty sections.
        let mut candidates: Vec<_> = world
            .dirty_sections
            .columns()
            .map(|((x, z), mask)| (i64::from(x * x + z * z), x, z, mask))
            .collect();
        let submitted = submit_mesh_candidates(
            &mut world,
            &mut candidates,
            3,
            &settings,
            &mut meshed,
            &mut meshing,
            &mut sequence,
            &mut order,
            &mut queue,
        );
        assert_eq!(submitted, 3);
        let first_batch = drain_sections(&mut queue);
        assert_eq!(first_batch, vec![(0, 7, 0), (0, 6, 0), (0, 8, 0)]);
        assert!(!meshed.contains(&(0, 0)));
        assert_eq!(world.dirty_sections.len(), 21 + 4);
        for sy in [6, 7, 8] {
            assert!(!world.dirty_sections.contains(&(0, sy, 0)));
        }
        for sy in (0..24).filter(|sy| ![6, 7, 8].contains(sy)) {
            assert!(world.dirty_sections.contains(&(0, sy, 0)));
        }
        for sy in 0..4 {
            assert!(world.dirty_sections.contains(&(1, sy, 0)));
        }

        // Pass 2: room (22) finishes column (0, 0)'s remaining 21 sections plus 1 from (1, 0).
        let mut candidates: Vec<_> = world
            .dirty_sections
            .columns()
            .map(|((x, z), mask)| (i64::from(x * x + z * z), x, z, mask))
            .collect();
        let submitted = submit_mesh_candidates(
            &mut world,
            &mut candidates,
            22,
            &settings,
            &mut meshed,
            &mut meshing,
            &mut sequence,
            &mut order,
            &mut queue,
        );
        assert_eq!(submitted, 22);
        let second_batch = drain_sections(&mut queue);
        assert!(meshed.contains(&(0, 0)));
        assert!(!meshed.contains(&(1, 0)));
        assert_eq!(world.dirty_sections.len(), 3);
        assert!(!world.dirty_sections.contains(&(1, 0, 0)));
        for sy in 1..4 {
            assert!(world.dirty_sections.contains(&(1, sy, 0)));
        }

        // Pass 3: room (10) drains the remaining 3 sections of (1, 0).
        let mut candidates: Vec<_> = world
            .dirty_sections
            .columns()
            .map(|((x, z), mask)| (i64::from(x * x + z * z), x, z, mask))
            .collect();
        let submitted = submit_mesh_candidates(
            &mut world,
            &mut candidates,
            10,
            &settings,
            &mut meshed,
            &mut meshing,
            &mut sequence,
            &mut order,
            &mut queue,
        );
        assert_eq!(submitted, 3);
        let third_batch = drain_sections(&mut queue);
        assert!(meshed.contains(&(1, 0)));
        assert!(world.dirty_sections.is_empty());

        let all: FxHashSet<_> = first_batch
            .into_iter()
            .chain(second_batch)
            .chain(third_batch)
            .collect();
        assert_eq!(all.len(), 28);
        assert_eq!(meshing.len(), 28);
    }

    /// Whether the forgotten columns are few (looked up by key) or many
    /// (one pass over each store), exactly their pending sections go.
    #[test]
    fn forgotten_columns_lose_their_pending_sections_either_way() {
        let pending: Vec<(i32, i32, i32)> = (0..4)
            .flat_map(|x| (0..3).map(move |sy| (x, sy, -x)))
            .collect();
        for gone in [
            vec![(1, -1)],
            (0..400)
                .map(|x| (x, -x))
                .filter(|&c| c != (2, -2))
                .collect(),
        ] {
            let gone: FxHashSet<(i32, i32)> = gone.into_iter().collect();
            let mut meshing: FxHashMap<_, u64> = pending.iter().map(|&k| (k, 1)).collect();
            let mut started = meshing.clone();
            let mut waiting: FxHashSet<_> = pending.iter().copied().collect();
            forget_pending_sections(&mut meshing, &mut started, &mut waiting, &gone, 3);
            let mut left: Vec<_> = waiting.into_iter().collect();
            left.sort();
            let expected: Vec<_> = pending
                .iter()
                .copied()
                .filter(|&(x, _, z)| !gone.contains(&(x, z)))
                .collect();
            assert_eq!(left, expected, "{} gone", gone.len());
            let mut left: Vec<_> = meshing.into_keys().collect();
            left.sort();
            assert_eq!(left, expected);
            let mut left: Vec<_> = started.into_keys().collect();
            left.sort();
            assert_eq!(left, expected);
        }
    }

    /// Edits in two neighbouring columns light those and the ten around
    /// them again; a column next to one that can't be lit whole keeps its
    /// edits in the frame.
    #[test]
    fn whole_relights_cover_the_edited_columns_and_their_neighbours() {
        // Blocks in columns (0, 0) and (1, 0).
        let positions = vec![(3, 70, 4), (15, 64, 15), (16, 80, 0), (31, 64, 9)];
        let (columns, inline) = whole_relight_columns(positions.clone(), |_, _| true);
        let mut columns: Vec<_> = columns.into_iter().collect();
        columns.sort();
        let expected: Vec<(i32, i32)> = (-1..=2)
            .flat_map(|x| (-1..=1).map(move |z| (x, z)))
            .collect();
        assert_eq!(columns, expected);
        assert!(inline.is_empty());
        // (2, 1) is next to (1, 0) only: (0, 0) is still lit whole.
        let (columns, inline) = whole_relight_columns(positions, |x, z| (x, z) != (2, 1));
        assert_eq!(inline, vec![(16, 80, 0), (31, 64, 9)]);
        assert_eq!(columns.len(), 9);
        assert!(columns.contains(&(-1, -1)) && columns.contains(&(1, 1)));
        // Negative coordinates floor to their column.
        let (west, _) = whole_relight_columns(vec![(-1, 64, -17)], |_, _| true);
        assert!(west.contains(&(-2, -3)) && west.contains(&(0, -1)) && west.len() == 9);
    }

    #[test]
    fn queued_jobs_follow_the_centre() {
        let forced = HashSet::new();
        let ranges = |center| Ranges {
            center,
            radius: 32,
            extra: &[],
            forced: &forced,
            cover: None,
            forced_cover: None,
            band: None,
        };
        let generate = |x, z, priority, order| Queued {
            priority,
            at: Some((x, z)),
            bias: 0,
            order,
            job: Job::Generate(x, z),
        };
        // Both queued around an earlier centre, where the farther job came first.
        let earlier = ranges((40, 0));
        let current = ranges((0, 0));
        let queued = || {
            BinaryHeap::from(vec![
                generate(3, 1, earlier.priority(3, 1) * 4, 1),
                generate(20, 0, earlier.priority(20, 0) * 4, 2),
                Queued {
                    priority: i64::MIN,
                    at: None,
                    bias: 0,
                    order: 3,
                    job: Job::Save(Vec::new()),
                },
            ])
        };
        let pops = |mut queue: BinaryHeap<Queued>| {
            std::iter::from_fn(move || queue.pop())
                .map(|q| q.at)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            pops(queued()),
            [None, Some((20, 0)), Some((3, 1))],
            "the stale order"
        );
        let mut queue = queued();
        rank_again(&mut queue, |x, z| current.priority(x, z));
        assert_eq!(pops(queue), [None, Some((3, 1)), Some((20, 0))]);
        // The player moved: the job near the new centre goes first.
        let mut queue = BinaryHeap::from(vec![generate(3, 1, 0, 1), generate(20, 0, 1, 2)]);
        let moved = ranges((19, 0));
        rank_again(&mut queue, |x, z| moved.priority(x, z));
        assert_eq!(queue.pop().and_then(|q| q.at), Some((20, 0)));
    }

    /// `ChunkTaskPriorityQueue.release`: a column that unloaded keeps no queued job, so
    /// a teleport or a smaller render distance does not spend its workers on the world
    /// it left; the other columns' jobs and the jobs without a column (saves, meshes to
    /// free) stay, in the same order.
    #[test]
    fn a_released_column_keeps_no_queued_job() {
        let generate = |x, z, priority, order| Queued {
            priority,
            at: Some((x, z)),
            bias: 0,
            order,
            job: Job::Generate(x, z),
        };
        let save = |order| Queued {
            priority: i64::MIN,
            at: None,
            bias: 0,
            order,
            job: Job::Save(Vec::new()),
        };
        let queued = || {
            BinaryHeap::from(vec![
                generate(0, 0, 0, 1),
                generate(0, 0, 4, 2),
                generate(1, 0, 8, 3),
                generate(9, 9, 12, 4),
                save(5),
            ])
        };
        let pops = |mut queue: BinaryHeap<Queued>| {
            std::iter::from_fn(move || queue.pop())
                .map(|q| q.at)
                .collect::<Vec<_>>()
        };
        // Nothing goes for a column that is still loaded, or for nothing at all.
        let mut queue = queued();
        assert_eq!(release_columns(&mut queue, &FxHashSet::default()), 0);
        assert_eq!(
            release_columns(&mut queue, &[(20, 20)].into_iter().collect()),
            0
        );
        assert_eq!(queue.len(), 5);
        // Both of (0, 0)'s jobs go, its neighbour's and the save stay.
        let gone: FxHashSet<(i32, i32)> = [(0, 0)].into_iter().collect();
        assert_eq!(release_columns(&mut queue, &gone), 2);
        assert_eq!(pops(queue), [None, Some((1, 0)), Some((9, 9))]);
        // Every column leaving at once: only the jobs without one are left.
        let mut queue = queued();
        let all: FxHashSet<(i32, i32)> = [(0, 0), (1, 0), (9, 9)].into_iter().collect();
        assert_eq!(release_columns(&mut queue, &all), 4);
        assert_eq!(pops(queue), [None]);
    }

    /// What the scheduler counts as ready to light: every neighbour a
    /// finished column, or a decorated proto chunk past the area — the
    /// walk `light_region` makes without its snapshot.
    #[test]
    fn light_readiness_follows_the_region_a_light_job_reads() {
        let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
        let chunk = Arc::new(Chunk::empty(0, 0));
        let nine = (-1..=1).flat_map(|dz| (-1..=1).map(move |dx| (dx, dz)));
        for key in nine {
            world.chunks.insert(key, chunk.clone());
        }
        // The kept area is the cross around (0, 0); the corners of the
        // 3x3 light reads lie past it.
        let in_range = |x: i32, z: i32| x.abs() + z.abs() <= 1;
        let none = FxHashSet::<(i32, i32)>::default();
        let empty = FxHashMap::<(i32, i32), Option<Proto>>::default();
        assert!(light_region_ready(&world, in_range, &none, &empty, 0, 0));
        // A corner is missing, and nothing stands in for it yet.
        world.chunks.remove(&(1, 1));
        assert!(!light_region_ready(&world, in_range, &none, &empty, 0, 0));
        // A decorated, checked-in proto chunk past the area lights against.
        let decorated: FxHashSet<(i32, i32)> = [(1, 1)].into_iter().collect();
        let mut protos: FxHashMap<(i32, i32), Option<Proto>> = Default::default();
        protos.insert((1, 1), Some(Box::new(()) as Proto));
        assert!(light_region_ready(
            &world, in_range, &decorated, &protos, 0, 0
        ));
        // The proto is checked out to a job: it cannot be read.
        protos.insert((1, 1), None);
        assert!(!light_region_ready(
            &world, in_range, &decorated, &protos, 0, 0
        ));
        // A neighbour inside the area waits for the chunk itself.
        world.chunks.remove(&(1, 0));
        let mut inside = decorated.clone();
        inside.insert((1, 0));
        protos.insert((1, 0), Some(Box::new(()) as Proto));
        assert!(!light_region_ready(
            &world, in_range, &inside, &protos, 0, 0
        ));
    }

    #[test]
    fn view_distance_is_vanillas_shape() {
        // ChunkTrackingView.isInViewDistance at render distance 12: a column
        // of slack on each axis, so 533 columns rather than the 441 of a
        // circle of radius 12.
        let count = |r: i32| {
            (-40..=40)
                .flat_map(|x| (-40..=40).map(move |z| (x, z)))
                .filter(|&(x, z)| in_view_distance((5, -7), r, x + 5, z - 7))
                .count()
        };
        assert_eq!(count(12), 533);
        assert_eq!(count(2), 25);
        for (x, z, inside) in [(12, 0, true), (13, 0, false), (9, 9, true), (10, 9, false)] {
            assert_eq!(in_view_distance((0, 0), 12, x, z), inside, "({x}, {z})");
            assert_eq!(
                in_view_distance((0, 0), 12, -z, -x),
                inside,
                "({}, {})",
                -z,
                -x
            );
        }
    }

    /// A view has arrived once the columns at both ends of each of its rows and
    /// columns are in: exactly those with a neighbour outside it, at every radius.
    /// Then it stays arrived until it changes; a view is `recent`, and waited for,
    /// `VIEW_WAIT` at most, and one of no radius not at all.
    #[test]
    fn a_view_has_arrived_once_its_outermost_columns_are_in() {
        let start = std::time::Instant::now();
        let center = (3, -4);
        for r in 0..=16 {
            let view: FxHashSet<(i32, i32)> = (-r - 1..=r + 1)
                .flat_map(|x| (-r - 1..=r + 1).map(move |z| (center.0 + x, center.1 + z)))
                .filter(|&(x, z)| in_view_distance(center, r, x, z))
                .collect();
            let arriving = |has: &dyn Fn((i32, i32)) -> bool| {
                ViewArrival::default().arriving((center, r), start, has)
            };
            assert!(!arriving(&|c| view.contains(&c)), "radius {r}");
            for &(x, z) in &view {
                let edge = [(x + 1, z), (x - 1, z), (x, z + 1), (x, z - 1)]
                    .iter()
                    .any(|c| !view.contains(c));
                let missing = arriving(&|c| c != (x, z) && view.contains(&c));
                assert_eq!(missing, edge, "radius {r}, column ({x}, {z})");
            }
        }
        let mut arrival = ViewArrival::default();
        let (none, all) = (|_| false, |_| true);
        assert!(arrival.arriving(((0, 0), 2), start, none));
        assert!(!arrival.arriving(((0, 0), 2), start, all));
        assert!(!arrival.arriving(((0, 0), 2), start, none));
        // Moved: waited for again, until `VIEW_WAIT` after the move.
        let (late, ms) = (start + VIEW_WAIT, std::time::Duration::from_millis(1));
        assert!(arrival.arriving(((1, 0), 2), start, none));
        assert!(arrival.arriving(((1, 0), 2), late - ms, none));
        assert!(arrival.recent(late - ms) && !arrival.recent(late));
        assert!(!arrival.arriving(((1, 0), 2), late, none));
        assert!(!arrival.arriving(((1, 0), -1), start, none));
    }

    struct GenerationFixture {
        world: World,
        center: (i32, i32),
        radius: i32,
        extra: Vec<((i32, i32), i32)>,
        forced: HashSet<(i32, i32)>,
        generating: FxHashSet<(i32, i32)>,
        protos: FxHashMap<(i32, i32), Option<Proto>>,
        finishing: FxHashSet<(i32, i32)>,
        decorates: bool,
        remote: bool,
    }
    impl GenerationFixture {
        fn new(radius: i32, decorates: bool) -> Self {
            Self {
                world: World::new(horizon_world::registry::BlockStateId(0), 0),
                center: (0, 0),
                radius,
                extra: Vec::new(),
                forced: HashSet::new(),
                generating: Default::default(),
                protos: Default::default(),
                finishing: Default::default(),
                decorates,
                remote: false,
            }
        }
        fn input(&self) -> GenerationInputs<'_> {
            GenerationInputs {
                ranges: Ranges {
                    center: self.center,
                    radius: self.radius,
                    extra: &self.extra,
                    forced: &self.forced,
                    cover: None,
                    forced_cover: None,
                    band: None,
                },
                world: &self.world,
                generating: &self.generating,
                protos: &self.protos,
                finishing: &self.finishing,
                decorates: self.decorates,
                remote: self.remote,
            }
        }
        fn fill(&mut self) {
            // Scheduler only reads membership. Share one real empty column to
            // keep a radius-32 fixture small; chunk content is never consulted.
            let chunk = Arc::new(Chunk::empty(0, 0));
            for (_, x, z) in baseline_candidates(self.input(), usize::MAX) {
                self.world.chunks.insert((x, z), chunk.clone());
            }
        }
        fn fill_steady(&mut self) {
            let chunk = Arc::new(Chunk::empty(0, 0));
            for (_, x, z) in baseline_candidates(self.input(), usize::MAX) {
                if self.input().ranges.in_range(x, z, 1) {
                    self.world.chunks.insert((x, z), chunk.clone());
                } else {
                    // Generation's outer neighbour margin remains proto;
                    // only columns in the finish range become resident.
                    self.protos.insert((x, z), Some(Box::new(())));
                }
            }
        }
        fn assert_same(&self, gate: &mut GenerationScan, capacity: usize) -> bool {
            let expected = baseline_candidates(self.input(), capacity);
            let result = gate.candidates(self.input(), capacity);
            assert_eq!(result.as_deref().unwrap_or(&[]), expected);
            result.is_some()
        }
    }

    // Original scheduling scan, without snapshot bookkeeping, for differential
    // correctness and timing. Keep this independent of the production gate.
    fn baseline_candidates(input: GenerationInputs<'_>, capacity: usize) -> Vec<(i64, i32, i32)> {
        if input.remote || input.generating.len() >= capacity {
            return Vec::new();
        }
        let margin: i32 = if input.decorates { 3 } else { 1 };
        let reach = if input.decorates { 2 } else { 1 };
        let mut wanted = Vec::new();
        let areas: Vec<_> = std::iter::once((input.ranges.center, input.ranges.radius))
            .chain(input.ranges.extra.iter().copied())
            .collect();
        let missing = |x, z| {
            !input.world.chunks.contains_key(&(x, z))
                && !input.generating.contains(&(x, z))
                && !input.protos.contains_key(&(x, z))
                && !input.finishing.contains(&(x, z))
        };
        for (center, radius) in areas {
            let r = radius + margin.max(reach + 1);
            for dz in -r..=r {
                for dx in -r..=r {
                    let (x, z) = (center.0 + dx, center.1 + dz);
                    if (input.ranges.in_range(x, z, margin)
                        || input.ranges.near_finished(x, z, reach))
                        && missing(x, z)
                    {
                        wanted.push((input.ranges.distance(x, z), x, z));
                    }
                }
            }
        }
        let forced_reach = if input.decorates {
            FORCED_TERRAIN_RING
        } else {
            FORCED_FEATURES_RING
        };
        for &(fx, fz) in input.ranges.forced {
            for dz in -forced_reach..=forced_reach {
                for dx in -forced_reach..=forced_reach {
                    let (x, z) = (fx + dx, fz + dz);
                    if missing(x, z) {
                        wanted.push((input.ranges.distance(x, z), x, z));
                    }
                }
            }
        }
        wanted.sort_unstable();
        wanted.dedup();
        wanted.truncate(capacity - input.generating.len());
        wanted
    }

    #[test]
    fn generation_gate_tracks_public_ranges_and_exact_membership() {
        for decorates in [false, true] {
            let mut f = GenerationFixture::new(2, decorates);
            f.fill();
            let mut gate = GenerationScan::default();
            assert!(f.assert_same(&mut gate, 6));
            assert!(!f.assert_same(&mut gate, 6));
            // Light/mesh state and block content do not affect generation.
            f.world.dirty_light.insert((0, 0));
            f.world.dirty_sections.insert((0, 0, 0));
            Arc::make_mut(f.world.chunks.get_mut(&(0, 0)).unwrap()).lit = true;
            assert!(!f.assert_same(&mut gate, 6));
            // Same-count removal/replacement must not leave a hole unnoticed.
            let chunk = f.world.chunks.remove(&(0, 0)).unwrap();
            f.world.chunks.insert((100, 100), chunk.clone());
            assert!(f.assert_same(&mut gate, 6));
            f.world.chunks.remove(&(100, 100));
            f.world.chunks.insert((0, 0), chunk);
            assert!(f.assert_same(&mut gate, 6));
            assert!(!f.assert_same(&mut gate, 6));
            f.center = (1, -1);
            assert!(f.assert_same(&mut gate, 6));
            f.fill();
            f.assert_same(&mut gate, 6);
            f.radius += 1;
            assert!(f.assert_same(&mut gate, 6));
            f.fill();
            f.assert_same(&mut gate, 6);
            f.extra.push(((20, -20), 2));
            assert!(f.assert_same(&mut gate, 6));
            f.fill();
            f.assert_same(&mut gate, 6);
            f.forced.insert((40, 40));
            assert!(f.assert_same(&mut gate, 6));
            f.fill();
            f.assert_same(&mut gate, 6);
            f.extra.clear();
            f.forced.clear();
            assert!(f.assert_same(&mut gate, 6));
            f.remote = true;
            assert!(!f.assert_same(&mut gate, 6));
            f.world.chunks.clear();
            f.remote = false;
            assert!(f.assert_same(&mut gate, 6));
            f.fill();
            f.assert_same(&mut gate, 6);
            f.world = World::new(horizon_world::registry::BlockStateId(0), 1);
            assert!(f.assert_same(&mut gate, 6));
        }
    }

    #[test]
    fn generation_gate_preserves_batches_stage_transitions_and_retries() {
        for decorates in [false, true] {
            let mut f = GenerationFixture::new(2, decorates);
            let mut gate = GenerationScan::default();
            let mut scheduled = FxHashSet::default();
            loop {
                let expected = baseline_candidates(f.input(), 3);
                let batch = gate.candidates(f.input(), 3).unwrap_or_default();
                assert_eq!(batch, expected);
                if batch.is_empty() {
                    break;
                }
                for (_, x, z) in batch {
                    assert!(scheduled.insert((x, z)), "duplicate scheduling");
                    f.generating.insert((x, z));
                }
                if f.generating.len() == 3 {
                    assert!(!f.assert_same(&mut gate, 3)); // capacity pressure
                } else {
                    f.assert_same(&mut gate, 3);
                }
                for key in std::mem::take(&mut f.generating) {
                    // load miss -> proto -> checked out decoration -> finish.
                    f.protos.insert(key, None);
                }
                gate.exhausted = false; // Done::Proto/Decorated invalidation
            }
            assert!(!scheduled.is_empty());
            assert!(!f.assert_same(&mut gate, 3));
            let key = (0, 0);
            f.protos.remove(&key);
            f.finishing.insert(key);
            assert!(!f.assert_same(&mut gate, 3)); // eligibility unchanged
            f.finishing.remove(&key);
            f.world.chunks.insert(key, Arc::new(Chunk::empty(0, 0)));
            gate.exhausted = false; // Done::Generated
            assert!(f.assert_same(&mut gate, 3));
            assert!(!f.assert_same(&mut gate, 3));
            // A discarded/pruned proto must be requested again, even without
            // any range or loaded-membership change.
            let key = scheduled.iter().copied().find(|k| *k != (0, 0)).unwrap();
            f.protos.remove(&key);
            gate.exhausted = false;
            assert!(f.assert_same(&mut gate, 3));
            assert_eq!(baseline_candidates(f.input(), 3).len(), 1);
        }
    }

    /// Batches taken from the plan are the full scan's, as jobs finish,
    /// columns and proto chunks leave and the centre moves.
    #[test]
    fn generation_plan_matches_full_scans_as_columns_arrive() {
        for decorates in [false, true] {
            let mut f = GenerationFixture::new(6, decorates);
            let mut gate = GenerationScan::default();
            let chunk = Arc::new(Chunk::empty(0, 0));
            let mut seed = 0x2545_F491_4F6C_DD1Du64;
            let mut rand = |n: u64| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                seed % n
            };
            let mut scheduled = 0;
            for step in 0..600 {
                let capacity = 3 + rand(6) as usize;
                let expected = baseline_candidates(f.input(), capacity);
                let batch = gate.candidates(f.input(), capacity).unwrap_or_default();
                assert_eq!(batch, expected, "step {step}");
                for (_, x, z) in batch {
                    f.generating.insert((x, z));
                    scheduled += 1;
                }
                // Some jobs finish: as a proto chunk, or a final column.
                let mut done: Vec<_> = f.generating.iter().copied().collect();
                done.sort_unstable();
                for key in done {
                    if rand(3) != 0 {
                        continue;
                    }
                    f.generating.remove(&key);
                    if rand(2) == 0 {
                        f.protos.insert(key, None);
                    } else {
                        f.world.chunks.insert(key, chunk.clone());
                    }
                    gate.exhausted = false;
                }
                // Now and then a column or a proto chunk leaves.
                if rand(20) == 0 {
                    let mut keys: Vec<_> = f.world.chunks.keys().copied().collect();
                    keys.sort_unstable();
                    if let Some(key) = keys.get(rand(keys.len().max(1) as u64) as usize) {
                        f.world.chunks.remove(key);
                    }
                }
                if rand(20) == 0 {
                    let mut keys: Vec<_> = f.protos.keys().copied().collect();
                    keys.sort_unstable();
                    if let Some(key) = keys.get(rand(keys.len().max(1) as u64) as usize) {
                        f.protos.remove(key);
                        gate.invalidate();
                    }
                }
                if step == 300 {
                    f.center = (3, -2);
                }
            }
            assert!(scheduled > 200);
        }
    }

    #[test]
    fn generation_scan_looks_at_overlapping_areas_once() {
        // A server's players: many areas around one spot, some apart, one
        // touching another only at a corner, and part of the land present.
        for decorates in [false, true] {
            let mut f = GenerationFixture::new(10, decorates);
            for i in 0..40 {
                f.extra.push(((i % 7 - 3, i / 7 - 3), 19));
            }
            f.extra.push(((200, -150), 19));
            f.extra.push(((243, -107), 19));
            f.extra.push(((-60, 5), 4));
            f.forced.insert((90, 90));
            let chunk = Arc::new(Chunk::empty(0, 0));
            for x in -40i32..40 {
                for z in -40i32..40 {
                    match (x * 7 + z * 13).rem_euclid(5) {
                        0 => drop(f.world.chunks.insert((x, z), chunk.clone())),
                        1 => drop(f.protos.insert((x, z), None)),
                        2 if x % 3 == 0 => drop(f.generating.insert((x, z))),
                        _ => {}
                    }
                }
            }
            assert!(baseline_candidates(f.input(), usize::MAX).len() > 1000);
            for capacity in [usize::MAX, f.generating.len() + 7] {
                let mut gate = GenerationScan::default();
                assert!(f.assert_same(&mut gate, capacity));
            }
        }
    }

    #[test]
    #[ignore = "reproducible actual generation scheduler benchmark; run with --ignored --nocapture"]
    fn generation_scan_benchmark() {
        use std::{hint::black_box, time::Instant};
        let mut f = GenerationFixture::new(32, true);
        f.fill_steady();
        let mut gate = GenerationScan::default();
        f.assert_same(&mut gate, 12);
        assert!(!f.assert_same(&mut gate, 12));
        const FRAMES: usize = 2000;
        let measure = |optimized: bool, dynamic: bool| {
            let mut samples = Vec::new();
            let mut scans = 0;
            let mut jobs = 0;
            for repetition in 0..9 {
                let mut fixture = GenerationFixture::new(32, true);
                fixture.fill_steady();
                let saved = fixture.world.chunks.get(&(0, 0)).unwrap().clone();
                let mut gate = GenerationScan::default();
                gate.candidates(fixture.input(), 12);
                let start = Instant::now();
                let mut count = 0;
                let mut scheduled = 0;
                for frame in 0..FRAMES {
                    // Remove a column, queue generation, then accept a saved
                    // load completion. Both schedulers see identical events.
                    if dynamic && frame % 32 == 0 {
                        fixture.world.chunks.remove(&(0, 0));
                    }
                    if dynamic && frame % 32 == 1 {
                        fixture.generating.remove(&(0, 0));
                        fixture.world.chunks.insert((0, 0), saved.clone());
                        gate.exhausted = false; // Done::Generated
                    }
                    let result = if optimized {
                        let result = gate.candidates(black_box(fixture.input()), 12);
                        count += usize::from(result.is_some());
                        result.unwrap_or_default()
                    } else {
                        count += 1;
                        baseline_candidates(black_box(fixture.input()), 12)
                    };
                    let expected_jobs = usize::from(dynamic && frame % 32 == 0);
                    assert_eq!(result.len(), expected_jobs);
                    for (_, x, z) in black_box(result) {
                        assert_eq!((x, z), (0, 0));
                        fixture.generating.insert((x, z));
                        scheduled += 1;
                    }
                }
                if repetition != 0 {
                    samples.push(start.elapsed().as_secs_f64() * 1e9 / FRAMES as f64);
                }
                assert!(fixture.generating.is_empty());
                assert_eq!(fixture.world.chunks.len(), f.world.chunks.len());
                scans = count;
                jobs = scheduled;
            }
            samples.sort_by(f64::total_cmp);
            (samples[samples.len() / 2], scans, jobs)
        };
        println!(
            "radius=32 decorates=true positions/frame=5041 loaded_keys={} proto_keys={} frames/sample={FRAMES} warmup=1 measured=8",
            f.world.chunks.len(),
            f.protos.len()
        );
        for dynamic in [false, true] {
            let (before, baseline_scans, before_jobs) = measure(false, dynamic);
            let (after, optimized_scans, after_jobs) = measure(true, dynamic);
            assert_eq!(before_jobs, after_jobs);
            println!(
                "dynamic={dynamic}: baseline={before:.1}ns/frame optimized={after:.1}ns/frame speedup={:.2}x scans={baseline_scans}/{optimized_scans} jobs={before_jobs}/{after_jobs}",
                before / after
            );
        }
    }

    /// The ranges `Streaming::update` checks at each step, from generation to
    /// light, must let every chunk the player spawns on finish and light: the
    /// 3x3 columns around the player need their own 3x3 neighbours finished,
    /// each of those its 3x3 neighbours decorated, and each of those its 3x3
    /// neighbours generated and kept. Render distance 2 once stopped short.
    #[test]
    fn spawn_neighbourhood_completes_at_every_distance() {
        let forced = HashSet::new();
        for decorates in [true, false] {
            let (margin, reach) = Ranges::margins(decorates);
            for radius in 2..=32 {
                let r = Ranges {
                    center: (5, -7),
                    radius,
                    extra: &[],
                    forced: &forced,
                    cover: None,
                    forced_cover: None,
                    band: None,
                };
                let square = |x: i32, z: i32, n: i32| {
                    (-n..=n).flat_map(move |dz| (-n..=n).map(move |dx| (x + dx, z + dz)))
                };
                let generated = |x, z| {
                    r.requested(x, z, decorates)
                        && (x - r.center.0).abs() <= radius + margin.max(reach + 1)
                        && (z - r.center.1).abs() <= radius + margin.max(reach + 1)
                };
                let proto_kept = |x, z| r.in_range(x, z, 4) || r.near_finished(x, z, 2);
                let decorated =
                    |x, z| !decorates || r.in_range(x, z, 2) || r.near_finished(x, z, 1);
                let chunk_kept = |x, z| r.chunk_kept(x, z, decorates);
                let unloaded = |x, z| r.unloaded(x, z, decorates);
                let finishes = |x: i32, z: i32| {
                    r.in_range(x, z, 1)
                        && chunk_kept(x, z)
                        && !unloaded(x, z)
                        && square(x, z, 1).all(|(nx, nz)| {
                            decorated(nx, nz)
                                && (!decorates
                                    || square(nx, nz, 1)
                                        .all(|(gx, gz)| generated(gx, gz) && proto_kept(gx, gz)))
                                && generated(nx, nz)
                                && proto_kept(nx, nz)
                        })
                };
                let (cx, cz) = r.center;
                for (x, z) in square(cx, cz, 1) {
                    for (nx, nz) in square(x, z, 1) {
                        assert!(
                            finishes(nx, nz),
                            "radius {radius} (decorates: {decorates}): ({nx}, {nz}) never finishes"
                        );
                    }
                }
            }
        }
    }

    /// A column generation asks for is kept when it arrives finished (loaded
    /// from the save) and is not unloaded right after: either way it would be
    /// asked for and loaded again on the next frame, forever (8 such columns
    /// after a teleport at render distance 16 held 8 of the in-flight slots).
    #[test]
    fn requested_columns_stay_loaded() {
        let forced = HashSet::new();
        for decorates in [true, false] {
            for radius in 2..=32 {
                let r = Ranges {
                    center: (5, -7),
                    radius,
                    extra: &[((radius * 2 + 9, 3), 4)],
                    forced: &forced,
                    cover: None,
                    forced_cover: None,
                    band: None,
                };
                let span = radius * 3 + 20;
                for x in -span..=span {
                    for z in -span..=span {
                        if r.requested(x, z, decorates) {
                            assert!(
                                r.chunk_kept(x, z, decorates),
                                "radius {radius} (decorates: {decorates}): ({x}, {z}) is asked for but dropped"
                            );
                        }
                        if r.chunk_kept(x, z, decorates) {
                            assert!(
                                !r.unloaded(x, z, decorates),
                                "radius {radius} (decorates: {decorates}): ({x}, {z}) is kept then unloaded"
                            );
                        }
                    }
                }
            }
        }
    }

    /// An idle dedicated server keeps no area around its centre: only a spawn
    /// search's chunk (FULL alone: its 3x3 decorated, 5x5 generated) and the
    /// forced chunks.
    #[test]
    fn no_area_around_an_idle_servers_centre() {
        let forced = HashSet::from([(30, 30)]);
        let extra = [((5, -3), -1)];
        let r = Ranges {
            center: (5, -3),
            radius: -1,
            extra: &extra,
            forced: &forced,
            cover: None,
            forced_cover: None,
            band: None,
        };
        for x in -5i32..=15 {
            for z in -13i32..=7 {
                let d = (x - 5).abs().max((z + 3).abs());
                assert_eq!(r.in_range(x, z, 1), d == 0, "({x}, {z})");
                assert_eq!(r.chunk_kept(x, z, true), d <= 2, "({x}, {z})");
                assert_eq!(r.requested(x, z, true), d <= 2, "({x}, {z})");
            }
        }
        assert!(r.in_range(30, 30, 1));
        let idle = Ranges { extra: &[], ..r };
        assert!(!idle.requested(5, -3, true) && idle.unloaded(5, -3, true));
    }

    /// A server player's area (`lan_update_streaming`: the server view
    /// distance + 1) finishes vanilla's FULL square: PLAYER_LOADING tickets at
    /// level 31 out to the view distance, FULL (33) two chunks past it. Every
    /// chunk the player tracks has its 3x3 neighbourhood finished, so it can
    /// be lit and sent, and generation reaches vanilla's FEATURES (34) and
    /// TERRAIN (35) rings.
    #[test]
    fn server_player_areas_finish_vanillas_full_square() {
        let forced = HashSet::new();
        for view in 2..=32 {
            let extra = [((40, -9), view + 1)];
            let r = Ranges {
                // The window's own area, far away.
                center: (-400, 300),
                radius: 2,
                extra: &extra,
                forced: &forced,
                cover: None,
                forced_cover: None,
                band: None,
            };
            let level = |x: i32, z: i32| 31 + ((x - 40).abs().max((z + 9).abs()) - view).max(0);
            for x in 40 - view - 8..=40 + view + 8 {
                for z in -9 - view - 8..=-9 + view + 8 {
                    assert_eq!(
                        r.in_range(x, z, 1),
                        level(x, z) <= 33,
                        "view {view}: ({x}, {z})"
                    );
                    assert_eq!(
                        r.requested(x, z, true),
                        level(x, z) <= 35,
                        "view {view}: ({x}, {z})"
                    );
                    assert_eq!(
                        r.unloaded(x, z, true),
                        level(x, z) > 35,
                        "view {view}: ({x}, {z})"
                    );
                    // ChunkTrackingView.contains, at any view distance the player may ask for.
                    let tracked = |v: i64| {
                        let gap = |d: i32| i64::from(((d).abs() - 2).max(0));
                        gap(x - 40).pow(2) + gap(z + 9).pow(2) < v * v
                    };
                    if tracked(i64::from(view)) {
                        for (dx, dz) in (-1..=1).flat_map(|dx| (-1..=1).map(move |dz| (dx, dz))) {
                            assert!(
                                r.in_range(x + dx, z + dz, 1),
                                "view {view}: ({x}, {z}) tracked, not lit"
                            );
                        }
                    }
                }
            }
        }
    }

    /// A `/forceload`ed chunk holds a FORCED ticket at level 31
    /// (`ChunkMap.FORCED_TICKET_LEVEL`): the level rises one per ring
    /// (`ChunkTracker.computeLevelFromNeighbor`) and picks the column's
    /// status (`ChunkLevel.generationStatus`). So vanilla finishes the two
    /// rings around it (33), gives the third its features (34) and the
    /// fourth its terrain (35), and the rings past that are left alone.
    #[test]
    fn forced_chunks_reach_vanillas_rings() {
        let forced: HashSet<(i32, i32)> = [(40, -40)].into();
        let r = Ranges {
            // The player's own area, far away.
            center: (-400, 300),
            radius: 2,
            extra: &[],
            forced: &forced,
            cover: None,
            forced_cover: None,
            band: None,
        };
        let level = |x: i32, z: i32| 31 + (x - 40).abs().max((z + 40).abs());
        for x in 40 - 6..=40 + 6 {
            for z in -40 - 6..=-40 + 6 {
                // The full ring: the column becomes a level chunk.
                assert_eq!(r.in_range(x, z, 1), level(x, z) <= 33, "({x}, {z})");
                // The features ring, and the terrain ring.
                assert_eq!(r.in_range(x, z, 2), level(x, z) <= 34, "({x}, {z})");
                assert_eq!(r.in_range(x, z, 3), level(x, z) <= 35, "({x}, {z})");
                assert_eq!(r.requested(x, z, true), level(x, z) <= 35, "({x}, {z})");
                assert_eq!(r.chunk_kept(x, z, true), level(x, z) <= 35, "({x}, {z})");
                assert_eq!(r.unloaded(x, z, true), level(x, z) > 35, "({x}, {z})");
            }
        }
        // Generation reaches the terrain ring: the features ring is
        // decorated from it.
        let mut fixture = GenerationFixture::new(2, true);
        fixture.center = (-400, 300);
        fixture.forced = forced;
        let candidates = GenerationScan::default()
            .candidates(fixture.input(), usize::MAX)
            .expect("a scan");
        for (dx, dz) in [(-4, 0), (4, 0), (0, -4), (0, 4), (3, 3)] {
            assert!(
                candidates
                    .iter()
                    .any(|&(_, x, z)| (x, z) == (40 + dx, -40 + dz)),
                "the terrain ring at ({}, {}) is not generated",
                40 + dx,
                -40 + dz
            );
        }
    }

    /// An edit's light change in a neighbouring column marks that column
    /// unsaved too, or a column saved before the edit keeps its old light on
    /// disk.
    #[test]
    fn light_spilling_into_a_saved_neighbour_marks_it_unsaved() {
        use crate::light::tests::{AIR, TORCH, tables, world};
        let tables = tables();
        let mut w = World::new(horizon_world::registry::BlockStateId(0), 0);
        for (key, chunk) in world(|c| {
            c.set(14, 0, 8, TORCH);
        }) {
            w.chunks.insert(key, chunk);
        }
        assert!(w.modified.is_empty());
        Arc::make_mut(w.chunks.get_mut(&(0, 0)).unwrap()).set(14, 0, 8, AIR);
        let mut changed = crate::light::LightChanges::default();
        crate::light::relight(
            &mut w.chunks,
            &tables,
            true,
            &[(14, 0, 8)],
            false,
            &mut changed,
        );
        assert_eq!(w.chunks[&(1, 0)].light(0, 0, 8) & 15, 0); // was 12
        Streaming::mark_light_changes(&mut w, &changed);
        assert!(w.modified.contains(&(1, 0)));
        assert!(w.modified.contains(&(0, 0)));
        // The torch never reached x -1: that column's light is unchanged.
        assert!(!w.modified.contains(&(-1, 0)));
    }

    fn resident_column(world: &mut World, light: u8) -> Chunk {
        let mut chunk = Chunk::empty(2, -3);
        chunk.set(1, 10, 1, 1);
        Arc::make_mut(&mut chunk.sections[0]).light = crate::chunk::Light::Uniform(light);
        chunk.lit = true;
        world.chunks.insert((2, -3), Arc::new(chunk.clone()));
        chunk
    }

    /// A server's resent column uses the light in its packet, as vanilla's
    /// `handleLevelChunkWithLight` does: only a Horizon server's bulk edit,
    /// whose packet light is from before its relight, is relit here. A proxy's
    /// next server resends its whole world at the same coordinates, and
    /// relighting every column and its neighbours is work that world never
    /// asked for (#10273).
    #[test]
    fn a_resent_column_is_relit_only_for_a_horizon_servers_bulk_edit() {
        for relight in [false, true] {
            let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
            let old = resident_column(&mut world, 0x50);
            let mut new = old.clone();
            new.set(1, 10, 1, 2);
            Arc::make_mut(&mut new.sections[0]).light = crate::chunk::Light::Uniform(0xF0);
            take_resent_column(&mut world, (2, -3), &old, new, relight);
            let chunk = &world.chunks[&(2, -3)];
            assert_eq!(chunk.block(1, 10, 1).0, 2);
            // The packet's light is what the column shows.
            assert_eq!(chunk.sections[0].light.get(0), 0xF0);
            // The changed section remeshes either way.
            assert!(world.dirty_sections.contains(&(2, 0, -3)));
            // Only a Horizon bulk edit queues light work, for the column and
            // its eight neighbours.
            assert_eq!(world.dirty_light.len(), if relight { 9 } else { 0 });
        }
    }
}

#[cfg(test)]
mod cover_tests {
    use super::*;

    fn areas(seed: &mut u64, n: usize, spread: i32) -> Vec<((i32, i32), i32)> {
        let mut next = |m: i32| {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 7;
            *seed ^= *seed << 17;
            (*seed % (2 * m as u64 + 1)) as i32 - m
        };
        (0..n)
            .map(|_| ((next(spread), next(spread)), 2 + next(14).abs()))
            .collect()
    }

    /// Every question streaming asks gets the same answer with the grid.
    ///
    /// A debug build checks every fourth cell of each row, shifted by row and case so the cases still cover every
    /// cell's position relative to the areas: the full check took 20 of the ~30 s of the crate's tests.
    /// `cargo test --release -p horizon-client --lib the_grid_answers_as_the_scan` checks every cell.
    #[test]
    #[ignore = "daily"]
    fn the_grid_answers_as_the_scan() {
        let stride = if cfg!(debug_assertions) { 4 } else { 1 };
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let forced: HashSet<(i32, i32)> = [(40, -40)].into();
        for case in 0..60 {
            let extra = areas(&mut seed, 2 + case % 20, [4, 30, 120][case % 3]);
            let cover = ExtraCover::new(&extra).expect("a grid");
            let ranges = |cover| Ranges {
                center: (3, -2),
                radius: 10,
                extra: &extra,
                forced: &forced,
                cover,
                forced_cover: None,
                band: None,
            };
            let (scan, grid) = (ranges(None), ranges(Some(&cover)));
            let span = [4, 30, 120][case % 3] + 40;
            for z in -span..=span {
                let first = -span + (z + case as i32).rem_euclid(stride);
                for x in (first..=span).step_by(stride as usize) {
                    for m in 0..=ExtraCover::REACH + 2 {
                        assert_eq!(
                            scan.in_extra(x, z, m),
                            grid.in_extra(x, z, m),
                            "case {case} ({x}, {z}) {m}"
                        );
                    }
                    for d in [false, true] {
                        assert_eq!(scan.unloaded(x, z, d), grid.unloaded(x, z, d));
                        assert_eq!(scan.requested(x, z, d), grid.requested(x, z, d));
                        assert_eq!(scan.chunk_kept(x, z, d), grid.chunk_kept(x, z, d));
                    }
                }
            }
        }
    }

    /// Players thousands of blocks apart: no grid fits them, and the buckets
    /// answer what a scan of every area does, the nearest centre included.
    #[test]
    #[ignore = "daily"]
    fn the_buckets_answer_as_the_scan() {
        let mut seed = 0x51_7CC1_B727_220Au64;
        let forced: HashSet<(i32, i32)> = HashSet::new();
        for case in 0..12 {
            // Spread over 6000 columns (96 000 blocks), some side by side.
            let mut extra = areas(&mut seed, 2 + case * 9, 3000);
            extra.push((extra[0].0, 10));
            extra.push(((extra[1].0.0 + 5, extra[1].0.1 - 70), 4));
            let cover = ExtraCover::new(&extra).expect("buckets");
            let ranges = |cover| Ranges {
                center: (3, -2),
                radius: 10,
                extra: &extra,
                forced: &forced,
                cover,
                forced_cover: None,
                band: None,
            };
            let (scan, buckets) = (ranges(None), ranges(Some(&cover)));
            // Around each area, and on a coarse lattice over the whole spread. A debug build
            // takes every third lattice point, shifted by case so the points still fall at
            // different positions against the 64-block bucket edges.
            let step = if cfg!(debug_assertions) { 21 } else { 7 };
            let shift = if cfg!(debug_assertions) {
                (case % 21) as i32
            } else {
                0
            };
            let near = extra.iter().flat_map(move |&((cx, cz), _)| {
                (-80 + shift..=80).step_by(step).flat_map(move |dz| {
                    (-80 + shift..=80)
                        .step_by(step)
                        .map(move |dx| (cx + dx, cz + dz))
                })
            });
            let lattice = (-3200..=3200)
                .step_by(97)
                .flat_map(|z| (-3200..=3200).step_by(89).map(move |x| (x, z)));
            for (x, z) in near.chain(lattice) {
                assert_eq!(
                    scan.distance(x, z),
                    buckets.distance(x, z),
                    "case {case} ({x}, {z})"
                );
                for m in 0..=ExtraCover::REACH {
                    assert_eq!(
                        scan.in_extra(x, z, m),
                        buckets.in_extra(x, z, m),
                        "case {case} ({x}, {z}) {m}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_stale_or_impossible_grid_is_not_used() {
        let mut cover = None;
        ExtraCover::refresh(&mut cover, &[((0, 0), 11), ((5, 5), 11)]);
        assert!(ExtraCover::matching(&cover, &[((0, 0), 11), ((5, 5), 11)]).is_some());
        assert!(ExtraCover::matching(&cover, &[((0, 0), 11), ((5, 6), 11)]).is_none());
        // Too far apart for a mebibyte of grid: buckets alone. One area alone: scans.
        let far = ExtraCover::new(&[((0, 0), 11), ((100_000, 0), 11)]).expect("buckets");
        assert!(far.grid.is_none());
        assert!(ExtraCover::new(&[((0, 0), 11)]).is_none());
        assert!(ExtraCover::new(&[((i32::MAX - 1, 0), 11), ((i32::MAX - 3, 0), 11)]).is_none());
        assert!(ExtraCover::new(&[((0, i32::MAX - 1), 11), ((0, i32::MAX - 3), 11)]).is_none());
        assert!(ExtraCover::new(&[((0, i32::MIN + 1), 11), ((0, i32::MIN + 3), 11)]).is_none());
    }

    /// cargo test --release -p horizon-client --lib cover_tests::cover_cost -- --ignored --nocapture
    #[test]
    #[ignore]
    fn cover_cost() {
        let forced = HashSet::new();
        for players in [10usize, 25, 50] {
            // The benchmark's bots: most near spawn, a fifth flying outwards.
            let extra: Vec<_> = (0..players)
                .map(|i| {
                    let a = i as f64 * 2.399;
                    let d = if i % 5 == 0 { 30.0 } else { 3.0 };
                    (((a.cos() * d) as i32, (a.sin() * d) as i32), 11)
                })
                .collect();
            let cover = ExtraCover::new(&extra).unwrap();
            let ranges = |cover| Ranges {
                center: (0, 0),
                radius: 10,
                extra: &extra,
                forced: &forced,
                cover,
                forced_cover: None,
                band: None,
            };
            // The columns streaming asks about: everything kept, and the
            // generation scan's ring past it.
            let columns: Vec<(i32, i32)> = (-60..=60)
                .flat_map(|z| (-60..=60).map(move |x| (x, z)))
                .filter(|&(x, z)| {
                    ranges(None).requested(x, z, true) || !ranges(None).unloaded(x, z, true)
                })
                .collect();
            let rounds = 100;
            let time = |r: &Ranges| {
                let t = std::time::Instant::now();
                let mut n = 0usize;
                for _ in 0..rounds {
                    for &(x, z) in &columns {
                        n += usize::from(r.unloaded(x, z, true));
                        n += usize::from(r.requested(x, z, true));
                        n += usize::from(r.in_range(x, z, 1));
                    }
                }
                std::hint::black_box(n);
                t.elapsed().as_secs_f64() * 1e3 / f64::from(rounds)
            };
            let t = std::time::Instant::now();
            for _ in 0..rounds {
                std::hint::black_box(ExtraCover::new(std::hint::black_box(&extra)));
            }
            let build = t.elapsed().as_secs_f64() * 1e3 / f64::from(rounds);
            println!(
                "{players} players, {} columns, 3 questions each: scan {:.3} ms, grid {:.3} ms (+ {build:.3} ms to build)",
                columns.len(),
                time(&ranges(None)),
                time(&ranges(Some(&cover))),
            );
        }
    }
}

#[cfg(test)]
mod reload_tests {
    use super::*;

    #[test]
    fn reload_marks_every_section_of_loaded_lit_columns() {
        let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
        world.chunks.insert((2, -3), Arc::new(Chunk::empty(2, -3)));
        world.chunks.insert((4, 5), Arc::new(Chunk::empty(4, 5)));
        let lit = FxHashSet::from_iter([(2, -3), (99, 99)]);

        Streaming::mark_loaded_sections_dirty(&mut world, &lit);

        let sections = world.layout().sections() as i32;
        assert_eq!(world.dirty_sections.len(), sections as usize);
        for sy in 0..sections {
            assert!(world.dirty_sections.contains(&(2, sy, -3)));
            assert!(!world.dirty_sections.contains(&(4, sy, 5)));
        }
    }

    #[test]
    fn reload_keeps_existing_work_and_ignores_unloaded_lit_columns() {
        let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
        world.chunks.insert((4, 5), Arc::new(Chunk::empty(4, 5)));
        world.dirty_sections.insert((4, 3, 5));
        let lit = FxHashSet::from_iter([(99, 99)]);

        Streaming::mark_loaded_sections_dirty(&mut world, &lit);

        assert_eq!(world.dirty_sections.len(), 1);
        assert!(world.dirty_sections.contains(&(4, 3, 5)));
    }
}
