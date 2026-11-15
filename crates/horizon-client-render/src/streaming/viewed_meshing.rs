//! Viewed levels' sections meshed (#4187): each level a world view names gets a set of meshed
//! sections of its own, apart from the player's level, as Immersive Portals keeps a chunk render
//! dispatcher per dimension (`MyBuiltChunkStorage`). The player's level never waits for them:
//! their jobs run on the shared pool in the far class, after every queued job of the player's
//! level, on at most [`VIEWED_MESH_WORKERS`] workers at once. The game names the levels it
//! wants each frame ([`ViewedLevel::wanted`]); a set lives while its level is wanted and
//! [`VIEWED_LINGER`] after, and goes at once with its level.
//!
//! The sets are the meshes on the client's side; their copy into the renderer and their drawing
//! are the next parts of #4187. Nothing here touches the player's world or its `Streaming`.
use super::{
    MeshSettings, Streaming, WorkerContext,
    mesh_job::{Faces, mesh_section},
};
use crate::{
    chunk::free_columns,
    fast_hash::FxHashMap,
    mesher::{MeshScratch, Region, SectionMesh},
    section_mask::SectionMask,
    world::World,
};
use horizon_threads::jobs::{Class, PerWorker, Pool, Rank, Registration, Source};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex, TryLockError,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

/// How long a viewed level's set stays after no view names it: a portal that leaves the screen
/// for a moment finds its level meshed when it comes back.
pub const VIEWED_LINGER: Duration = Duration::from_secs(5);
/// The most sections one viewed level's set holds, meshed or being meshed: 8192, a third of the
/// level's 2048 columns of 24 sections (`viewed_worlds::MAX_COLUMNS`), as most of a column is air
/// and keeps no mesh. Further sections wait, dirty, until some leave.
pub const MAX_VIEWED_SECTIONS: usize = 8192;
/// The workers viewed levels mesh on at once: one, or a quarter of the pool where that is fewer.
/// A pool under four workers keeps one too (a quarter of it is none), and the player's level
/// still goes first there: a viewed job only starts on a worker no job of it is queued for.
pub const VIEWED_MESH_WORKERS: usize = 1;
/// The most viewed sections that enter their sets in a frame, all levels together; the rest
/// wait for the next frames.
pub const VIEWED_UPLOADS_PER_FRAME: usize = 4;
/// The most viewed jobs queued or running, all levels together: each holds its column's 3x3
/// region, and jobs past what one worker meshes in a few frames would only go stale.
const MAX_IN_FLIGHT: usize = 64;

/// A section: its column's x, its index from the level's bottom, its column's z.
type Section = (i32, i32, i32);

/// What a viewed section is meshed with.
#[derive(Clone)]
pub struct MeshSource(Kind);

#[derive(Clone)]
enum Kind {
    /// The player's mesher and graphics settings (`Streaming::viewed_mesh_source`).
    Mesher(Arc<WorkerContext>, MeshSettings),
    /// Any other mesher: a section `sy` of the region's centre column.
    Custom(Arc<dyn Fn(&Region, i32) -> SectionMesh + Send + Sync>),
}

impl MeshSource {
    /// Meshes with `mesh` instead of the block mesher.
    pub fn custom(mesh: impl Fn(&Region, i32) -> SectionMesh + Send + Sync + 'static) -> Self {
        Self(Kind::Custom(Arc::new(mesh)))
    }
    /// The same, shaded as a level with the Nether's cardinal light or not.
    fn for_level(&self, nether_shading: bool) -> Self {
        match &self.0 {
            Kind::Mesher(context, settings) => Self(Kind::Mesher(
                context.clone(),
                MeshSettings {
                    nether: nether_shading,
                    ..settings.clone()
                },
            )),
            Kind::Custom(_) => self.clone(),
        }
    }
    fn mesh(&self, local: &mut (Faces, MeshScratch), region: &Region, sy: i32) -> SectionMesh {
        match &self.0 {
            Kind::Mesher(context, settings) => {
                let (faces, scratch) = local;
                mesh_section(
                    context,
                    settings,
                    faces,
                    scratch,
                    region,
                    sy,
                    SectionMesh::default(),
                )
            }
            Kind::Custom(mesh) => mesh(region, sy),
        }
    }
}

impl Streaming {
    /// Viewed levels mesh with the player's mesher and graphics settings, each with its own
    /// dimension's shading.
    pub fn viewed_mesh_source(&self) -> MeshSource {
        let context = self.shared.context.read().unwrap().clone();
        MeshSource(Kind::Mesher(context, self.mesh_settings()))
    }
}

/// A viewed level, as the game hands it over each frame.
pub struct ViewedLevel<'a> {
    /// Its world's own id: a level opened again is another.
    pub id: u64,
    /// A view names it this frame.
    pub wanted: bool,
    pub nether_shading: bool,
    pub world: &'a mut World,
}

struct Job {
    level: u64,
    section: Section,
    sequence: u64,
    region: Arc<Region>,
    source: MeshSource,
}

struct Meshed {
    level: u64,
    section: Section,
    sequence: u64,
    mesh: SectionMesh,
}

/// The viewed levels' jobs, a source of the shared pool.
struct Jobs {
    queue: Mutex<VecDeque<Job>>,
    running: AtomicUsize,
    done: Mutex<VecDeque<Meshed>>,
    locals: PerWorker<(Faces, MeshScratch)>,
}

/// One of the [`VIEWED_MESH_WORKERS`] places, given back however its job ends.
struct Running<'a>(&'a AtomicUsize);

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Source for Jobs {
    fn next(&self) -> Option<Rank> {
        if self.running.load(Ordering::SeqCst) >= VIEWED_MESH_WORKERS {
            return None;
        }
        // In the far class: after every queued near job, the player's level's meshing among them.
        (!self.queue.lock().unwrap().is_empty()).then_some(Rank::new(Class::Far, 0))
    }
    fn run(&self, worker: usize) -> bool {
        if self.running.fetch_add(1, Ordering::SeqCst) >= VIEWED_MESH_WORKERS {
            self.running.fetch_sub(1, Ordering::SeqCst);
            return false;
        }
        let _place = Running(&self.running);
        let Some(job) = self.queue.lock().unwrap().pop_front() else {
            return false;
        };
        let mesh = self.locals.with(worker, Default::default, |local| {
            job.source.mesh(local, &job.region, job.section.1)
        });
        self.done.lock().unwrap().push_back(Meshed {
            level: job.level,
            section: job.section,
            sequence: job.sequence,
            mesh,
        });
        true
    }
    fn idle(&self, worker: usize) {
        self.locals.clear(worker);
    }
    fn label(&self) -> &'static str {
        "stream.viewed_mesh"
    }
}

/// One viewed level's meshed sections.
#[derive(Default)]
struct LevelSet {
    /// The sections with something to draw, by section.
    meshes: FxHashMap<Section, SectionMesh>,
    /// Sections being meshed, with their job's sequence: a section dirtied again or a column gone
    /// makes an older job's mesh stale.
    meshing: FxHashMap<Section, u64>,
    /// Since when no view names the level.
    unwanted_since: Option<Instant>,
}

impl LevelSet {
    /// Column `(x, z)` left the level: its sections leave the set.
    fn drop_column(&mut self, x: i32, z: i32, sections: usize) {
        for sy in 0..sections as i32 {
            self.meshes.remove(&(x, sy, z));
            self.meshing.remove(&(x, sy, z));
        }
    }
}

/// Every viewed level's set, and their jobs on the pool.
pub struct ViewedMeshing {
    jobs: Arc<Jobs>,
    registration: Registration,
    sets: FxHashMap<u64, LevelSet>,
    sequence: u64,
}

impl Default for ViewedMeshing {
    fn default() -> Self {
        Self::in_pool(horizon_threads::jobs::pool())
    }
}

impl ViewedMeshing {
    /// Meshing on `pool`'s workers.
    pub fn in_pool(pool: &Pool) -> Self {
        let jobs = Arc::new(Jobs {
            queue: Mutex::default(),
            running: AtomicUsize::new(0),
            done: Mutex::default(),
            locals: PerWorker::new(pool.threads()),
        });
        Self {
            registration: pool.register(jobs.clone()),
            jobs,
            sets: FxHashMap::default(),
            sequence: 0,
        }
    }

    /// No level has a set: nothing is kept or meshed.
    pub fn is_empty(&self) -> bool {
        self.sets.is_empty()
    }

    /// The meshed sections of level `id`, while it has a set.
    pub fn sections(&self, id: u64) -> Option<impl Iterator<Item = (Section, &SectionMesh)>> {
        let set = self.sets.get(&id)?;
        Some(set.meshes.iter().map(|(&section, mesh)| (section, mesh)))
    }

    /// Level `id` closed: its set goes now, with its queued jobs.
    pub fn forget(&mut self, id: u64) {
        if let Some(set) = self.sets.remove(&id) {
            self.free(id, set);
        }
    }

    fn free(&mut self, id: u64, set: LevelSet) {
        self.jobs
            .queue
            .lock()
            .unwrap()
            .retain(|job| job.level != id);
        let count = set.meshes.len();
        free_columns(set.meshes, count);
    }

    /// A frame: the sets of the viewed levels open now made, kept or freed, up to
    /// [`VIEWED_UPLOADS_PER_FRAME`] finished sections taken into them, and the wanted levels'
    /// dirty sections queued. A level missing from `levels` closed.
    pub fn frame<'a>(
        &mut self,
        source: &MeshSource,
        levels: impl IntoIterator<Item = ViewedLevel<'a>>,
        now: Instant,
    ) {
        let mut levels: Vec<ViewedLevel<'a>> = levels.into_iter().collect();
        let closed: Vec<u64> = self
            .sets
            .keys()
            .copied()
            .filter(|id| !levels.iter().any(|level| level.id == *id))
            .collect();
        for id in closed {
            self.forget(id);
        }
        for level in &mut levels {
            if level.wanted {
                let set = self.sets.entry(level.id).or_insert_with(|| {
                    // A new set meshes the whole level.
                    let world = &mut *level.world;
                    let all = SectionMask::below(world.layout().sections());
                    let columns: Vec<_> = world.chunks.keys().copied().collect();
                    for column in columns {
                        world.dirty_sections.insert_column_mask(column, all);
                    }
                    LevelSet::default()
                });
                set.unwanted_since = None;
            } else if let Some(set) = self.sets.get_mut(&level.id) {
                let since = *set.unwanted_since.get_or_insert(now);
                if now.saturating_duration_since(since) >= VIEWED_LINGER {
                    self.forget(level.id);
                }
            }
        }
        self.receive();
        for level in levels.iter_mut().filter(|level| level.wanted) {
            self.schedule(source, level);
        }
    }

    /// Up to [`VIEWED_UPLOADS_PER_FRAME`] finished sections into their sets; stale ones dropped.
    fn receive(&mut self) {
        let mut done = match self.jobs.done.try_lock() {
            Ok(done) => done,
            // A worker is handing one in: taken next frame.
            Err(TryLockError::WouldBlock) => return,
            Err(TryLockError::Poisoned(e)) => panic!("{e}"),
        };
        let mut taken = 0;
        while taken < VIEWED_UPLOADS_PER_FRAME
            && let Some(meshed) = done.pop_front()
        {
            let Some(set) = self.sets.get_mut(&meshed.level) else {
                continue;
            };
            if set.meshing.get(&meshed.section) != Some(&meshed.sequence) {
                continue;
            }
            set.meshing.remove(&meshed.section);
            let mesh = meshed.mesh;
            if mesh.solid.is_empty()
                && mesh.translucent.is_empty()
                && mesh.packed.is_empty()
                && mesh.block_entities.is_empty()
                && mesh.lights.is_empty()
            {
                set.meshes.remove(&meshed.section);
            } else {
                set.meshes.insert(meshed.section, mesh);
            }
            taken += 1;
        }
    }

    /// Level `level`'s dirty sections queued, as far as its set and the jobs in flight have room.
    /// A column whose eight neighbours are not all there waits: one arriving dirties it again.
    fn schedule(&mut self, source: &MeshSource, level: &mut ViewedLevel<'_>) {
        let world = &mut *level.world;
        let in_flight: usize = self.sets.values().map(|set| set.meshing.len()).sum();
        let Some(set) = self.sets.get_mut(&level.id) else {
            return;
        };
        if world.dirty_sections.is_empty() || in_flight >= MAX_IN_FLIGHT {
            return;
        }
        let mut room = MAX_IN_FLIGHT - in_flight;
        let sections = world.layout().sections();
        let source = source.for_level(level.nether_shading);
        let columns: Vec<_> = world.dirty_sections.columns().map(|(c, _)| c).collect();
        let mut queue = self.jobs.queue.lock().unwrap();
        for (x, z) in columns {
            if room == 0 {
                break;
            }
            if !world.chunks.contains_key(&(x, z)) {
                world.dirty_sections.take_column(x, z);
                set.drop_column(x, z, sections);
                continue;
            }
            let Some(region) = Streaming::region(world, x, z) else {
                world.dirty_sections.take_column(x, z);
                continue;
            };
            for sy in world.dirty_sections.take_column(x, z).iter() {
                let section = (x, sy as i32, z);
                let new = !set.meshes.contains_key(&section) && !set.meshing.contains_key(&section);
                if room == 0 || (new && set.meshes.len() + set.meshing.len() >= MAX_VIEWED_SECTIONS)
                {
                    world.dirty_sections.insert(section);
                    continue;
                }
                self.sequence += 1;
                set.meshing.insert(section, self.sequence);
                queue.push_back(Job {
                    level: level.id,
                    section,
                    sequence: self.sequence,
                    region: region.clone(),
                    source: source.clone(),
                });
                room -= 1;
            }
        }
        let queued = !queue.is_empty();
        drop(queue);
        if queued {
            self.registration.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::Chunk;
    use horizon_world::registry::BlockStateId;
    use std::sync::mpsc;

    /// A 3x3 of columns around the origin: only the centre one has its eight neighbours.
    fn level() -> World {
        let mut world = World::new(BlockStateId(0), 0);
        for z in -1..=1 {
            for x in -1..=1 {
                world.chunks.insert((x, z), Arc::new(Chunk::empty(x, z)));
            }
        }
        world
    }

    /// A mesh with something to draw.
    fn drawn() -> SectionMesh {
        let mut mesh = SectionMesh::default();
        mesh.block_entities.push(([0, 0, 0], 1));
        mesh
    }

    fn frame(meshing: &mut ViewedMeshing, source: &MeshSource, world: &mut World, wanted: bool) {
        let level = ViewedLevel {
            id: 7,
            wanted,
            nether_shading: false,
            world,
        };
        meshing.frame(source, [level], Instant::now());
    }

    fn count(meshing: &ViewedMeshing) -> Option<usize> {
        meshing.sections(7).map(Iterator::count)
    }

    /// Frames until level 7's set holds `n` sections, or 10 s.
    fn mesh_until(meshing: &mut ViewedMeshing, source: &MeshSource, world: &mut World, n: usize) {
        let start = Instant::now();
        while count(meshing) != Some(n) {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "{:?}",
                count(meshing)
            );
            frame(meshing, source, world, true);
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// The player's level's near jobs, ranked by their number.
    struct Burst {
        jobs: Mutex<VecDeque<i64>>,
        log: Arc<Mutex<Vec<String>>>,
        started: Mutex<mpsc::Sender<()>>,
        gate: Mutex<mpsc::Receiver<()>>,
    }

    impl Source for Burst {
        fn next(&self) -> Option<Rank> {
            let jobs = self.jobs.lock().unwrap();
            jobs.front().map(|&i| Rank::new(Class::Near, i))
        }
        fn run(&self, _worker: usize) -> bool {
            let Some(i) = self.jobs.lock().unwrap().pop_front() else {
                return false;
            };
            if i == 0 {
                self.started.lock().unwrap().send(()).unwrap();
                self.gate.lock().unwrap().recv().unwrap();
            }
            self.log.lock().unwrap().push(format!("player {i}"));
            true
        }
    }

    /// A viewed section queued while a burst of the player's level's meshing waits on the only
    /// worker is meshed after all of it.
    #[test]
    fn a_viewed_section_queued_behind_the_players_burst_finishes_after_it() {
        let pool = Pool::new(1, "viewed-burst");
        let log = Arc::new(Mutex::new(Vec::new()));
        let (started, on_start) = mpsc::channel();
        let (open, gate) = mpsc::channel();
        let burst = Arc::new(Burst {
            jobs: Mutex::new((0..8).collect()),
            log: log.clone(),
            started: Mutex::new(started),
            gate: Mutex::new(gate),
        });
        let _player = pool.register(burst);
        on_start.recv().unwrap();
        let mut meshing = ViewedMeshing::in_pool(&pool);
        let viewed_log = log.clone();
        let source = MeshSource::custom(move |_, sy| {
            viewed_log.lock().unwrap().push(format!("viewed {sy}"));
            drawn()
        });
        let mut world = level();
        let sections = world.layout().sections();
        frame(&mut meshing, &source, &mut world, true);
        open.send(()).unwrap();
        mesh_until(&mut meshing, &source, &mut world, sections);
        let log = log.lock().unwrap();
        let player: Vec<String> = (0..8).map(|i| format!("player {i}")).collect();
        assert_eq!(log[..8], player[..], "{log:?}");
        assert_eq!(log.len(), 8 + sections);
        assert!(log[8..].iter().all(|entry| entry.starts_with("viewed")));
    }

    /// On a pool of four workers, viewed sections are meshed one at a time.
    #[test]
    fn viewed_levels_mesh_on_one_worker_at_once() {
        let pool = Pool::new(4, "viewed-share");
        let mut meshing = ViewedMeshing::in_pool(&pool);
        let busy = Arc::new([AtomicUsize::new(0), AtomicUsize::new(0)]);
        let seen = busy.clone();
        let source = MeshSource::custom(move |_, _| {
            let now = seen[0].fetch_add(1, Ordering::SeqCst) + 1;
            seen[1].fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(2));
            seen[0].fetch_sub(1, Ordering::SeqCst);
            drawn()
        });
        let mut world = level();
        let sections = world.layout().sections();
        mesh_until(&mut meshing, &source, &mut world, sections);
        assert_eq!(busy[1].load(Ordering::SeqCst), 1);
    }

    /// Nothing is meshed for a level no view names; a wanted one's finished sections enter its
    /// set four a frame; a column gone takes its sections out; the set stays five seconds after
    /// the level is no longer wanted, and goes at once when the level closes.
    #[test]
    fn a_viewed_set_follows_what_is_wanted_and_open() {
        let pool = Pool::new(2, "viewed-set");
        let mut meshing = ViewedMeshing::in_pool(&pool);
        let source = MeshSource::custom(|_, _| drawn());
        let mut world = level();
        let sections = world.layout().sections();
        frame(&mut meshing, &source, &mut world, false);
        assert!(meshing.is_empty());
        assert!(world.dirty_sections.is_empty());
        assert!(meshing.jobs.queue.lock().unwrap().is_empty());

        frame(&mut meshing, &source, &mut world, true);
        let start = Instant::now();
        while meshing.jobs.done.lock().unwrap().len() < sections {
            assert!(start.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(1));
        }
        frame(&mut meshing, &source, &mut world, true);
        assert_eq!(count(&meshing), Some(VIEWED_UPLOADS_PER_FRAME));
        frame(&mut meshing, &source, &mut world, true);
        assert_eq!(count(&meshing), Some(2 * VIEWED_UPLOADS_PER_FRAME));
        mesh_until(&mut meshing, &source, &mut world, sections);

        world.unload_chunk((0, 0));
        world
            .dirty_sections
            .insert_column_mask((0, 0), SectionMask::below(sections));
        frame(&mut meshing, &source, &mut world, true);
        assert_eq!(count(&meshing), Some(0));

        let t0 = Instant::now();
        let at = |meshing: &mut ViewedMeshing, after: Duration, world: &mut World| {
            let level = ViewedLevel {
                id: 7,
                wanted: false,
                nether_shading: false,
                world,
            };
            meshing.frame(&source, [level], t0 + after);
        };
        at(&mut meshing, Duration::ZERO, &mut world);
        at(
            &mut meshing,
            VIEWED_LINGER - Duration::from_millis(1),
            &mut world,
        );
        assert_eq!(count(&meshing), Some(0), "kept while it lingers");
        at(&mut meshing, VIEWED_LINGER, &mut world);
        assert_eq!(count(&meshing), None);

        let mut world = level();
        frame(&mut meshing, &source, &mut world, true);
        assert!(count(&meshing).is_some());
        meshing.frame(&source, [], Instant::now());
        assert_eq!(count(&meshing), None, "closed");
        assert!(meshing.is_empty());
        assert!(meshing.jobs.queue.lock().unwrap().is_empty());
    }
}
