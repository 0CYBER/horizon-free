//! The light sources a `[ray_tracing] lights = true` pack's rays aim at (docs/api/shaders.md, the
//! prelude's `ray_light`, `ray_lights_near` and `ray_light_at`): the sources nearest the camera and a
//! grid of cells over them, each listing the sources whose light reaches it, strongest first.
//! A pass shades a point with the few sources its cell lists, each tested with a shadow ray,
//! instead of every source in the world.
//!
//! The buffer is rebuilt on a worker thread when the camera moves on to another 16-block step or
//! the light sources change, from the sources the section meshes list (`SectionMesh::lights`),
//! and uploaded once built; its corner, relative to the player's camera
//! (`frame.camera_position`), is written every frame.

use crate::fast_hash::FxHashMap;
use glam::{DVec3, IVec3, Vec3};
use horizon_shader_api::stages::{
    RAY_LIGHT_CELL, RAY_LIGHT_GRID, RAY_LIGHT_HEADER, RAY_LIGHT_MAX, RAY_LIGHT_PER_CELL,
    RAY_LIGHT_WORDS,
};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use super::block_light::SectionLights;
use horizon_items::blocks::Fluid;

/// The grid moves in steps of this many blocks.
const STEP: i32 = 16;
/// The least time between two rebuilds for changed sources alone (the grid not moved).
const SOURCE_REBUILDS: Duration = Duration::from_millis(100);
/// Where the cells' counts start, and then their lists: after the header and the sources.
const COUNTS: u32 = RAY_LIGHT_HEADER + RAY_LIGHT_MAX * RAY_LIGHT_WORDS;
const CELLS: u32 = RAY_LIGHT_GRID * RAY_LIGHT_GRID * RAY_LIGHT_GRID;
const LISTS: u32 = COUNTS + CELLS;
/// The buffer's words.
const WORDS: u32 = LISTS + CELLS * RAY_LIGHT_PER_CELL;

/// What `ray_light` reads of a light-giving block state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Source {
    /// Its `[[lights]]` colour as written (the prelude decodes it), white where it has none.
    pub color: [f32; 3],
    /// Vanilla's light level, 1 to 15.
    pub level: u8,
    /// Its `[[materials]]` id, 0 for none (`material_of`).
    pub material: u32,
}

/// Each listed block state's `Source`, by state id.
pub type Sources = FxHashMap<u32, Source>;

/// The source a block state lists, if it gives off light of its own: not a fluid's (lava; a
/// lantern under water is listed), whose lakes would fill every cell. `color` is its
/// `[[lights]]` colour, `layers` the sprite layers its model's faces draw with, in order, and
/// `materials` the pack's `(layer, id)`, by layer.
pub fn source(
    (emission, fluid): (u8, Fluid),
    color: Option<[f32; 3]>,
    layers: impl IntoIterator<Item = u32>,
    materials: &[(u32, u32)],
) -> Option<Source> {
    let fluid_light = matches!(fluid, Fluid::Lava | Fluid::Modded);
    (emission > 0 && !fluid_light).then(|| Source {
        color: color.unwrap_or([1.0; 3]),
        level: emission,
        material: material_of(layers, materials),
    })
}

/// The id `RayHit.material` reports on the first face, of `layers`, whose sprite layer has one
/// in `materials` (sorted by layer); 0 where none does.
fn material_of(layers: impl IntoIterator<Item = u32>, materials: &[(u32, u32)]) -> u32 {
    let id = |layer| (materials.binary_search_by_key(&layer, |m| m.0).ok()).map(|i| materials[i].1);
    layers.into_iter().find_map(id).unwrap_or(0)
}

pub struct RayLights {
    pub buffer: wgpu::Buffer,
    /// The world corner and the sources' revision of the last rebuild sent, and when.
    built: Option<([i32; 3], u64, Instant)>,
    /// The corner of the words the buffer holds, which positions are written against.
    shown: Option<[i32; 3]>,
    /// The worker (`start`), whether it runs a rebuild, and the buffers of the last one, for
    /// the next.
    worker: Option<Worker>,
    running: bool,
    spare: Job,
    /// When a worker last failed to start (the next try waits `RESPAWN`), and whether that was
    /// logged: the render thread never builds, so the buffer keeps what it holds meanwhile.
    failed: Option<Instant>,
    logged: bool,
    /// Counts the worlds: a rebuild for an earlier one is dropped when it comes back.
    epoch: u64,
    /// The world changed: the next frame empties the buffer's list and grid.
    blank: bool,
    /// The light-giving block states, for a worker started again.
    table: Sources,
}

/// The least time between two tries to start a worker that failed to start.
const RESPAWN: Duration = Duration::from_secs(1);

/// A rebuild for the worker: the world it is for, the grid's corner, the camera, the
/// light-giving blocks of the sections in its reach (`gather`), and the words to build into.
#[derive(Default)]
struct Job {
    epoch: u64,
    origin: [i32; 3],
    camera: DVec3,
    blocks: Vec<Arc<[([i32; 3], u32)]>>,
    words: Vec<u32>,
}

/// The thread that builds the words (`Builder`), so a rebuild costs the render thread only the
/// gathering of the sections' lists (one shared handle each, whatever they hold) and the upload.
struct Worker {
    jobs: Sender<Job>,
    done: Receiver<Job>,
}

impl Worker {
    fn spawn(table: Sources) -> std::io::Result<Self> {
        let (jobs, job_rx) = mpsc::channel::<Job>();
        let (done_tx, done) = mpsc::channel();
        std::thread::Builder::new()
            .name("ray lights".into())
            .spawn(move || {
                let mut builder = Builder::default();
                while let Ok(mut job) = job_rx.recv() {
                    builder.build(&table, &mut job);
                    // A section's list replaced since is freed here, not on the render thread.
                    job.blocks.clear();
                    if done_tx.send(job).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self { jobs, done })
    }
}

impl RayLights {
    /// The buffer alone, for the passes' bind groups: `start` gives it its sources once a pass
    /// that reads them is made.
    pub fn new(device: &wgpu::Device) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Native shader pack ray light sources"),
            size: u64::from(WORDS) * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            buffer,
            built: None,
            shown: None,
            worker: None,
            running: false,
            spare: Job::default(),
            failed: None,
            logged: false,
            epoch: 0,
            blank: false,
            table: Sources::default(),
        }
    }

    /// The light-giving block states, and the worker that builds from them.
    pub fn start(&mut self, table: Sources) {
        self.table = table;
        self.worker = None;
        self.spawned(Instant::now());
    }

    /// The worker, started (again) if there is none, unless the last try failed under
    /// `RESPAWN` ago.
    fn spawned(&mut self, now: Instant) -> Option<&Worker> {
        let waits = self
            .failed
            .is_some_and(|at| now.duration_since(at) < RESPAWN);
        if self.worker.is_none() && !waits {
            match Worker::spawn(self.table.clone()) {
                Ok(w) => (self.worker, self.failed) = (Some(w), None),
                Err(e) => {
                    self.failed = Some(now);
                    if !std::mem::replace(&mut self.logged, true) {
                        log::warn!("Horizon: the ray light sources' worker doesn't start: {e}");
                    }
                }
            }
        }
        self.worker.as_ref()
    }

    /// Every section is gone (a new world, a new dimension): the buffer is emptied next frame,
    /// and a rebuild still running for the old one is dropped.
    pub fn clear(&mut self) {
        self.epoch += 1;
        (self.built, self.shown, self.blank) = (None, None, true);
    }

    /// Places the grid around `camera`. The sources and the cells are rebuilt, on the worker,
    /// when it moved on to another step, or when `revision` (of `sections`' sources) changed, at
    /// most every `SOURCE_REBUILDS` for that (a redstone clock would otherwise rebuild every
    /// frame); a frame uploads them once they are built, a frame or two later. A worker gone is
    /// started again and given the latest request; the render thread never builds. `min_y` is
    /// the lowest block of the level's columns, which the sections' keys count from.
    pub fn update(
        &mut self,
        queue: &wgpu::Queue,
        camera: DVec3,
        (sections, min_y): (&SectionLights, i32),
        revision: u64,
    ) {
        match self.worker.as_ref().map(|w| w.done.try_recv()) {
            Some(Ok(job)) => self.landed(queue, job),
            // The worker is gone, and its rebuild with it: asked again of the next one.
            Some(Err(TryRecvError::Disconnected)) => {
                self.worker = None;
                if std::mem::take(&mut self.running) {
                    self.built = None;
                }
            }
            Some(Err(TryRecvError::Empty)) | None => {}
        }
        if std::mem::take(&mut self.blank) {
            // No sources and no grid (`ray_lights_near` lists nothing) until the new world's.
            queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&[0u32; 2]));
        }
        let (origin, now) = (grid_origin(camera), Instant::now());
        if !self.running && due(self.built, origin, revision, now) {
            let mut job = std::mem::take(&mut self.spare);
            (job.epoch, job.origin, job.camera) = (self.epoch, origin, camera);
            gather(origin, min_y, sections, &mut job.blocks);
            let sent = match self.spawned(now) {
                Some(w) => w.jobs.send(job).map_err(|mpsc::SendError(job)| job),
                None => Err(job),
            };
            match sent {
                // Stamped once sent: gathering or a worker's start doesn't count toward the wait.
                Ok(()) => {
                    (self.running, self.built) = (true, Some((origin, revision, Instant::now())));
                }
                // No worker, or it is gone: the request stays due, for the next one.
                Err(mut job) => {
                    if self.worker.take().is_some() {
                        self.failed = None;
                    }
                    job.blocks.clear();
                    self.spare = job;
                }
            }
        }
        // Against the corner of the words the buffer holds, which only an upload moves.
        if let Some(shown) = self.shown {
            let corner = (IVec3::from(shown).as_dvec3() - camera).as_vec3();
            let corner = corner.to_array().map(f32::to_bits);
            queue.write_buffer(&self.buffer, 16, bytemuck::cast_slice(&corner));
        }
    }

    /// The worker's rebuild came back: uploaded, unless it is for an earlier world.
    fn landed(&mut self, queue: &wgpu::Queue, job: Job) {
        self.running = false;
        match job.epoch == self.epoch {
            true => self.upload(queue, job),
            false => self.spare = job,
        }
    }

    /// Uploads the built `job`'s words: the header and its sources, then every cell's count
    /// and list (the sources past its count, left from an earlier build, are never read).
    fn upload(&mut self, queue: &wgpu::Queue, job: Job) {
        let words = &job.words;
        let sources = (RAY_LIGHT_HEADER + words[0] * RAY_LIGHT_WORDS) as usize;
        queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&words[..sources]));
        let cells = bytemuck::cast_slice(&words[COUNTS as usize..]);
        queue.write_buffer(&self.buffer, u64::from(COUNTS) * 4, cells);
        self.shown = Some(job.origin);
        self.spare = job;
    }

    /// Waits for the worker's rebuild, if one runs, and uploads it: a test's next frame then
    /// reads it.
    #[cfg(test)]
    pub(in crate::gfx) fn settle(&mut self, queue: &wgpu::Queue) {
        if let Some(job) = (self.worker.as_ref())
            .filter(|_| self.running)
            .and_then(|w| w.done.recv().ok())
        {
            self.landed(queue, job);
        }
    }
}

/// Whether to rebuild the grid last sent (`built`: its corner, revision and when) for the corner
/// `origin` and the sources' `revision` at `now`: at once for a new corner, at most every
/// `SOURCE_REBUILDS` for new sources alone.
fn due(
    built: Option<([i32; 3], u64, Instant)>,
    origin: [i32; 3],
    revision: u64,
    now: Instant,
) -> bool {
    match built {
        None => true,
        Some((o, _, _)) if o != origin => true,
        Some((_, r, at)) => r != revision && now.duration_since(at) >= SOURCE_REBUILDS,
    }
}

/// The grid's world corner for `camera`: on the 16-block steps, the camera near its middle.
fn grid_origin(camera: DVec3) -> [i32; 3] {
    let half = f64::from(RAY_LIGHT_GRID) * f64::from(RAY_LIGHT_CELL) / 2.0;
    ((camera - half) / f64::from(STEP))
        .floor()
        .as_ivec3()
        .to_array()
        .map(|v| v * STEP)
}

/// The light-giving block lists of the sections that the grid at `origin`, or light reaching
/// into it (15 blocks), touches, into `blocks`: only the sections in that range are looked up,
/// not the whole map, which may hold the render distance, and each list is shared, not copied
/// (a lava sea's lists are long). The sections' keys count from `min_y`.
fn gather(
    origin: [i32; 3],
    min_y: i32,
    sections: &SectionLights,
    blocks: &mut Vec<Arc<[([i32; 3], u32)]>>,
) {
    let span = RAY_LIGHT_GRID as i32 * RAY_LIGHT_CELL as i32;
    let lo = origin.map(|v| (v - 15).div_euclid(16));
    let hi = origin.map(|v| (v + span + 15).div_euclid(16));
    let below = min_y.div_euclid(16);
    blocks.clear();
    for x in lo[0]..=hi[0] {
        for y in lo[1]..=hi[1] {
            for z in lo[2]..=hi[2] {
                if let Some(lights) = sections.get(&(x, y - below, z)) {
                    blocks.push(lights.clone());
                }
            }
        }
    }
}

/// A source found for the grid: its block, its centre from the grid's corner, its colour, level
/// and material, and its distance² to the camera.
#[derive(Clone, Copy)]
struct Found {
    block: [i32; 3],
    at: Vec3,
    color: [f32; 3],
    level: f32,
    material: u32,
    distance: f64,
}

/// The nearest the camera first; a tie by block, so the order never depends on the map's.
fn nearest_first(a: &Found, b: &Found) -> std::cmp::Ordering {
    (a.distance.total_cmp(&b.distance)).then(a.block.cmp(&b.block))
}

/// The sources of `blocks` (`gather`) that `table` lists, the nearest `RAY_LIGHT_MAX` to
/// `camera`, nearest first, into `found`.
fn find(
    table: &Sources,
    (origin, camera): ([i32; 3], DVec3),
    blocks: &[Arc<[([i32; 3], u32)]>],
    found: &mut Vec<Found>,
) {
    let corner = IVec3::from(origin).as_dvec3();
    found.clear();
    for &(block, state) in blocks.iter().flat_map(|lights| lights.iter()) {
        let Some(source) = table.get(&state) else {
            continue;
        };
        let center = IVec3::from(block).as_dvec3() + 0.5;
        found.push(Found {
            block,
            at: (center - corner).as_vec3(),
            color: source.color,
            level: f32::from(source.level),
            material: source.material,
            distance: center.distance_squared(camera),
        });
    }
    let max = RAY_LIGHT_MAX as usize;
    if found.len() > max {
        found.select_nth_unstable_by(max, nearest_first);
        found.truncate(max);
    }
    found.sort_unstable_by(nearest_first);
}

/// The cells the light of a source at `at` reaching `reach` blocks touches, as the first and the
/// last on each axis, if any.
fn reached(at: Vec3, reach: f32) -> Option<(IVec3, IVec3)> {
    let n = RAY_LIGHT_GRID as f32;
    let first = ((at - reach) / RAY_LIGHT_CELL).floor().max(Vec3::ZERO);
    let last = ((at + reach) / RAY_LIGHT_CELL)
        .floor()
        .min(Vec3::splat(n - 1.0));
    (!first.cmpgt(last).any()).then(|| (first.as_ivec3(), last.as_ivec3()))
}

/// How much of a source's light at `at` reaching `reach` blocks is left at the cell `cell`'s
/// nearest point: the strength the cells rank their sources by.
fn left_at(at: Vec3, reach: f32, cell: IVec3) -> f32 {
    let low = cell.as_vec3() * RAY_LIGHT_CELL;
    let near = at.clamp(low, low + RAY_LIGHT_CELL);
    reach - near.distance(at)
}

/// What it takes to build the buffer's words, kept between rebuilds, so a rebuild into words
/// of an earlier one allocates nothing.
#[derive(Default)]
struct Builder {
    /// The strength of each cell's listed sources, in the lists' slots.
    strengths: Vec<f32>,
    found: Vec<Found>,
}

impl Builder {
    /// Rebuilds `job`'s words for its grid and blocks (the corner itself left as it was: the
    /// frame writes it). The source words past the count are cleared, and a cell's slots past
    /// its count keep what they held: nothing reads them.
    fn build(&mut self, table: &Sources, job: &mut Job) {
        let per = RAY_LIGHT_PER_CELL as usize;
        if job.words.len() != WORDS as usize {
            job.words = vec![0; WORDS as usize];
        }
        if self.strengths.len() != CELLS as usize * per {
            self.strengths = vec![0.0; CELLS as usize * per];
        }
        find(
            table,
            (job.origin, job.camera),
            &job.blocks,
            &mut self.found,
        );
        let words = &mut job.words;
        words[1] = RAY_LIGHT_GRID;
        words[2] = RAY_LIGHT_CELL.to_bits();
        words[3] = RAY_LIGHT_PER_CELL;
        words[8] = COUNTS;
        words[9] = LISTS;
        let (before, now) = (words[0], self.found.len() as u32);
        words[0] = now;
        let source_words = |count: u32| (RAY_LIGHT_HEADER + count * RAY_LIGHT_WORDS) as usize;
        if before > now {
            words[source_words(now)..source_words(before)].fill(0);
        }
        for (i, s) in self.found.iter().enumerate() {
            let w = source_words(i as u32);
            let floats = [
                s.at.x, s.at.y, s.at.z, s.level, s.color[0], s.color[1], s.color[2],
            ];
            for (k, f) in floats.into_iter().enumerate() {
                words[w + k] = f.to_bits();
            }
            words[w + 7] = s.material;
        }
        // Each cell's sources: those whose light (as far as its level) reaches the cell's box,
        // the strongest at the cell's nearest point first, the earlier source first on a tie.
        let (counts, lists) = words[COUNTS as usize..].split_at_mut(CELLS as usize);
        counts.fill(0);
        let n = RAY_LIGHT_GRID as usize;
        for (i, s) in self.found.iter().enumerate() {
            let Some((first, last)) = reached(s.at, s.level) else {
                continue;
            };
            for z in first.z..=last.z {
                for y in first.y..=last.y {
                    for x in first.x..=last.x {
                        let left = left_at(s.at, s.level, IVec3::new(x, y, z));
                        if left <= 0.0 {
                            continue;
                        }
                        let c = x as usize + n * (y as usize + n * z as usize);
                        let slots = c * per..(c + 1) * per;
                        let strengths = &mut self.strengths[slots.clone()];
                        let list = &mut lists[slots];
                        let count = counts[c] as usize;
                        // After every listed source at least as strong.
                        let mut at = count;
                        while at > 0 && strengths[at - 1] < left {
                            at -= 1;
                        }
                        if at == per {
                            continue;
                        }
                        let end = count.min(per - 1);
                        strengths.copy_within(at..end, at + 1);
                        list.copy_within(at..end, at + 1);
                        strengths[at] = left;
                        list[at] = i as u32;
                        counts[c] = (count + 1).min(per) as u32;
                    }
                }
            }
        }
    }
}

/// The buffer's words for the grid with its corner at `origin`, from a builder of their own.
#[cfg(test)]
fn build(
    table: &Sources,
    origin: [i32; 3],
    camera: DVec3,
    (sections, min_y): (&SectionLights, i32),
) -> Vec<u32> {
    let mut job = Job {
        origin,
        camera,
        ..Job::default()
    };
    gather(origin, min_y, sections, &mut job.blocks);
    Builder::default().build(table, &mut job);
    job.words
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(color: [f32; 3], level: u8, material: u32) -> Source {
        Source {
            color,
            level,
            material,
        }
    }

    /// `(state, block)` sources keyed by their sections, as the meshes list them, in columns
    /// from `min_y`.
    fn sections_from(blocks: &[([i32; 3], u32)], min_y: i32) -> SectionLights {
        let mut sections: FxHashMap<crate::gfx::SectionKey, Vec<_>> = FxHashMap::default();
        for &(p, state) in blocks {
            let key = (p[0] >> 4, (p[1] - min_y) >> 4, p[2] >> 4);
            sections.entry(key).or_default().push((p, state));
        }
        sections.into_iter().map(|(k, v)| (k, v.into())).collect()
    }

    /// `sections_from` in vanilla's columns.
    fn sections(blocks: &[([i32; 3], u32)]) -> SectionLights {
        sections_from(blocks, crate::chunk::MIN_Y)
    }

    const MIN_Y: i32 = crate::chunk::MIN_Y;

    /// A torch at the camera and a dim source in its cell: both are listed nearest first, with
    /// their colour, level and material, a cell the torch's light still reaches lists only the
    /// torch, one past it nothing.
    #[test]
    fn the_cells_list_the_sources_that_reach_them() {
        let mut table = Sources::default();
        table.insert(7, lit([2.0, 1.0, 0.0], 14, 5));
        table.insert(9, lit([1.0, 1.0, 1.0], 3, 0));
        let sections = sections(&[([8, 64, 8], 7), ([12, 64, 12], 9), ([5, 64, 5], 1)]);
        let camera = DVec3::new(8.5, 64.5, 8.5);
        let origin = grid_origin(camera);
        assert_eq!(origin, [-64, 0, -64]);
        let words = build(&table, origin, camera, (&sections, MIN_Y));
        let float = |i: usize| f32::from_bits(words[i]);
        // State 1 gives no light: two sources, the torch first.
        assert_eq!(words[0], 2);
        let torch = RAY_LIGHT_HEADER as usize;
        assert_eq!(
            [float(torch), float(torch + 1), float(torch + 2)],
            [72.5, 64.5, 72.5]
        );
        assert_eq!(float(torch + 3), 14.0);
        assert_eq!(float(torch + 4), 2.0);
        assert_eq!(words[torch + 7], 5);
        assert_eq!(words[torch + RAY_LIGHT_WORDS as usize + 7], 0);
        let cell = |p: [i32; 3]| {
            let c = p.map(|v| v as usize / RAY_LIGHT_CELL as usize);
            c[0] + RAY_LIGHT_GRID as usize * (c[1] + RAY_LIGHT_GRID as usize * c[2])
        };
        let listed = |p: [i32; 3]| {
            let c = cell(p);
            let count = words[COUNTS as usize + c] as usize;
            let at = LISTS as usize + c * RAY_LIGHT_PER_CELL as usize;
            words[at..at + count].to_vec()
        };
        // The torch's cell: the torch (all 14 levels left) before the dim source.
        assert_eq!(listed([72, 64, 72]), vec![0, 1]);
        // The cell from x 80: 7.5 blocks from the torch, 3.5 from the dim source (level 3).
        assert_eq!(listed([85, 64, 72]), vec![0]);
        assert_eq!(listed([100, 64, 72]), Vec::<u32>::new());
    }

    /// Lava is left out, a lantern under water is listed; a block's material is the first of
    /// its faces' sprite layers that has one, 0 where none does; a block without light is none.
    #[test]
    fn a_source_is_a_block_s_own_light_with_its_faces_material() {
        let materials = [(3, 40), (8, 12)];
        let orange = Some([1.0, 0.5, 0.1]);
        assert_eq!(source((15, Fluid::Lava), orange, [9], &materials), None);
        assert_eq!(
            source((15, Fluid::Water), orange, [2, 8, 3], &materials),
            Some(lit([1.0, 0.5, 0.1], 15, 12))
        );
        assert_eq!(
            source((14, Fluid::None), None, [1, 2], &materials),
            Some(lit([1.0; 3], 14, 0))
        );
        assert_eq!(source((0, Fluid::None), orange, [3], &materials), None);
    }

    /// The builder before it kept its words: every section of the map walked, a `Vec` a cell,
    /// sorted (stably: the earlier source first on a tie) and cut to 24.
    fn reference(
        table: &Sources,
        origin: [i32; 3],
        camera: DVec3,
        sections: &SectionLights,
    ) -> Vec<u32> {
        let n = RAY_LIGHT_GRID as usize;
        let mut words = vec![0u32; WORDS as usize];
        words[1] = RAY_LIGHT_GRID;
        words[2] = RAY_LIGHT_CELL.to_bits();
        words[3] = RAY_LIGHT_PER_CELL;
        words[8] = COUNTS;
        words[9] = LISTS;
        let span = RAY_LIGHT_GRID as i32 * RAY_LIGHT_CELL as i32;
        let lo = origin.map(|v| (v - 15).div_euclid(16));
        let hi = origin.map(|v| (v + span + 15).div_euclid(16));
        let mut sources = Vec::new();
        for (key, lights) in sections {
            let key = [key.0, key.1 + crate::chunk::MIN_Y.div_euclid(16), key.2];
            if (0..3).any(|i| key[i] < lo[i] || key[i] > hi[i]) {
                continue;
            }
            for &(block, state) in lights.iter() {
                let Some(source) = table.get(&state) else {
                    continue;
                };
                let center = IVec3::from(block).as_dvec3() + 0.5;
                sources.push(Found {
                    block,
                    at: (center - IVec3::from(origin).as_dvec3()).as_vec3(),
                    color: source.color,
                    level: f32::from(source.level),
                    material: source.material,
                    distance: center.distance_squared(camera),
                });
            }
        }
        sources.sort_by(nearest_first);
        sources.truncate(RAY_LIGHT_MAX as usize);
        words[0] = sources.len() as u32;
        for (i, s) in sources.iter().enumerate() {
            let w = (RAY_LIGHT_HEADER + i as u32 * RAY_LIGHT_WORDS) as usize;
            let floats = [
                s.at.x, s.at.y, s.at.z, s.level, s.color[0], s.color[1], s.color[2],
            ];
            for (k, f) in floats.into_iter().enumerate() {
                words[w + k] = f.to_bits();
            }
            words[w + 7] = s.material;
        }
        let mut cells: Vec<Vec<(f32, u32)>> = vec![Vec::new(); n * n * n];
        for (i, s) in sources.iter().enumerate() {
            let Some((first, last)) = reached(s.at, s.level) else {
                continue;
            };
            for z in first.z..=last.z {
                for y in first.y..=last.y {
                    for x in first.x..=last.x {
                        let left = left_at(s.at, s.level, IVec3::new(x, y, z));
                        if left > 0.0 {
                            cells[x as usize + n * (y as usize + n * z as usize)]
                                .push((left, i as u32));
                        }
                    }
                }
            }
        }
        let per = RAY_LIGHT_PER_CELL as usize;
        for (c, list) in cells.iter_mut().enumerate() {
            list.sort_by(|a, b| b.0.total_cmp(&a.0));
            list.truncate(per);
            words[COUNTS as usize + c] = list.len() as u32;
            for (k, &(_, i)) in list.iter().enumerate() {
                words[LISTS as usize + c * per + k] = i;
            }
        }
        words
    }

    /// `count` sources of four levels at random blocks around `camera` (some out of the grid's
    /// reach), and a quarter of them packed in 8 blocks, whose cells overflow 24.
    fn random_blocks(seed: u64, count: usize, camera: [i32; 3]) -> Vec<([i32; 3], u32)> {
        let mut state = seed;
        let mut next = |below: i32| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((state >> 33) % below as u64) as i32
        };
        // One state a block, as in a world.
        let blocks: std::collections::BTreeMap<_, _> = (0..count)
            .map(|i| {
                let spread = if i % 4 == 0 { 8 } else { 180 };
                let block = camera.map(|c| c + next(spread) - spread / 2);
                (block, 1 + next(4) as u32)
            })
            .collect();
        blocks.into_iter().collect()
    }

    /// The kept builder's words are the reference builder's, for random sources past the
    /// 1024 cap and cells past 24; built again into the same words for other sources, every
    /// word a shader reads (the header, the sources, the counts, each cell's listed slots) is
    /// the reference's too.
    #[test]
    fn the_kept_builder_lists_what_the_reference_builder_does() {
        let mut table = Sources::default();
        for (state, level) in [(1, 15), (2, 14), (3, 7), (4, 3)] {
            table.insert(state, lit([0.5, 1.0, 0.25], level, state * 10));
        }
        let (mut builder, mut job) = (Builder::default(), Job::default());
        for (seed, count, camera) in [
            (1, 1500, [3, 70, -9]),
            (2, 300, [40, 20, 17]),
            (3, 1200, [3, 70, -9]),
        ] {
            let sections = sections(&random_blocks(seed, count, camera));
            let camera = IVec3::from(camera).as_dvec3() + 0.3;
            let origin = grid_origin(camera);
            let want = reference(&table, origin, camera, &sections);
            assert_eq!(build(&table, origin, camera, (&sections, MIN_Y)), want);
            let full = (0..CELLS as usize).filter(|&c| want[COUNTS as usize + c] == 24);
            assert!(seed == 2 || full.count() > 0, "no cell overflows");
            (job.origin, job.camera) = (origin, camera);
            gather(origin, MIN_Y, &sections, &mut job.blocks);
            builder.build(&table, &mut job);
            let got = &job.words;
            assert_eq!(
                got[..COUNTS as usize],
                want[..COUNTS as usize],
                "seed {seed}"
            );
            for c in 0..CELLS as usize {
                let count = want[COUNTS as usize + c] as usize;
                assert_eq!(got[COUNTS as usize + c] as usize, count);
                let at = LISTS as usize + c * RAY_LIGHT_PER_CELL as usize;
                assert_eq!(got[at..at + count], want[at..at + count], "seed {seed}");
            }
        }
    }

    /// A new corner rebuilds at once; changed sources alone at most every 100 ms; nothing
    /// changed, never.
    #[test]
    fn source_only_rebuilds_wait_100_ms() {
        let start = Instant::now();
        let built = Some(([0, 0, 0], 7, start));
        let later = |ms| start + Duration::from_millis(ms);
        assert!(due(None, [0, 0, 0], 7, start));
        assert!(due(built, [16, 0, 0], 7, later(1)));
        assert!(!due(built, [0, 0, 0], 8, later(99)));
        assert!(due(built, [0, 0, 0], 8, later(100)));
        assert!(!due(built, [0, 0, 0], 7, later(1000)));
    }

    /// Waits on a test's worker for its rebuild, and uploads it: the next frame reads it.
    #[test]
    fn the_worker_s_rebuild_is_uploaded_with_its_own_corner() {
        let Some((_, device, queue)) = crate::gfx::test_gpu::optional_device() else {
            return;
        };
        let mut table = Sources::default();
        table.insert(1, lit([1.0; 3], 15, 1));
        let sections = sections(&[([2, 64, 0], 1)]);
        let mut lights = RayLights::new(&device);
        lights.start(table);
        assert!(lights.worker.is_some());
        let camera = DVec3::new(0.5, 64.5, 0.5);
        lights.update(&queue, camera, (&sections, MIN_Y), 1);
        assert!(lights.running && lights.shown.is_none());
        lights.settle(&queue);
        assert_eq!(lights.shown, Some(grid_origin(camera)));
        assert_eq!(lights.spare.words[0], 1);
        assert!(lights.spare.blocks.is_empty());
    }

    /// A new world drops the old one's rebuild still on the worker and empties the buffer; its
    /// first frame rebuilds.
    #[test]
    fn a_rebuild_for_the_world_before_is_dropped() {
        let Some((_, device, queue)) = crate::gfx::test_gpu::optional_device() else {
            return;
        };
        let mut table = Sources::default();
        table.insert(1, lit([1.0; 3], 15, 1));
        let sections = sections(&[([2, 64, 0], 1)]);
        let mut lights = RayLights::new(&device);
        lights.start(table);
        let camera = DVec3::new(0.5, 64.5, 0.5);
        lights.update(&queue, camera, (&sections, MIN_Y), 1);
        lights.clear();
        lights.settle(&queue);
        assert_eq!(lights.shown, None);
        let empty = SectionLights::default();
        lights.update(&queue, camera, (&empty, MIN_Y), 2);
        assert!(lights.running && !lights.blank);
        lights.settle(&queue);
        assert_eq!(lights.shown, Some(grid_origin(camera)));
        assert_eq!(lights.spare.words[0], 0);
    }

    /// A worker gone with a rebuild under way, and one gone before a rebuild is sent to it: each
    /// time a new worker is started and given the latest request; this thread builds nothing.
    #[test]
    fn a_lost_worker_s_rebuild_is_built_on_a_new_one() {
        let Some((_, device, queue)) = crate::gfx::test_gpu::optional_device() else {
            return;
        };
        let mut table = Sources::default();
        table.insert(1, lit([1.0; 3], 15, 1));
        let sections = sections(&[([2, 64, 0], 1)]);
        let mut lights = RayLights::new(&device);
        lights.table = table;
        let camera = DVec3::new(0.5, 64.5, 0.5);
        // Gone with its rebuild: nothing comes back.
        let (jobs, _job_rx) = mpsc::channel();
        let (_, done) = mpsc::channel();
        lights.worker = Some(Worker { jobs, done });
        (lights.running, lights.built) = (true, Some((grid_origin(camera), 1, Instant::now())));
        lights.update(&queue, camera, (&sections, MIN_Y), 1);
        assert!(lights.worker.is_some() && lights.running && lights.shown.is_none());
        lights.settle(&queue);
        assert_eq!(lights.shown, Some(grid_origin(camera)));
        assert_eq!(lights.spare.words[0], 1);
        // Gone before the next rebuild is sent: it stays due, for the worker started next frame.
        let (jobs, _) = mpsc::channel();
        let (_done_tx, done) = mpsc::channel();
        lights.worker = Some(Worker { jobs, done });
        let stepped = camera + DVec3::X * 16.0;
        lights.update(&queue, stepped, (&sections, MIN_Y), 1);
        assert!(lights.worker.is_none() && !lights.running);
        lights.update(&queue, stepped, (&sections, MIN_Y), 1);
        assert!(lights.running);
        lights.settle(&queue);
        assert_eq!(lights.shown, Some(grid_origin(stepped)));
    }

    /// Sections keyed from a level's own lowest block (below vanilla's): the sources deep in it
    /// are found.
    #[test]
    fn the_sections_are_keyed_from_the_level_s_lowest_block() {
        let mut table = Sources::default();
        table.insert(1, lit([1.0; 3], 15, 1));
        let min_y = -256;
        let sections = sections_from(&[([2, -200, 0], 1)], min_y);
        let camera = DVec3::new(0.5, -199.5, 0.5);
        let origin = grid_origin(camera);
        assert_eq!(build(&table, origin, camera, (&sections, min_y))[0], 1);
        assert_eq!(build(&table, origin, camera, (&sections, MIN_Y))[0], 0);
    }

    /// A bench, not a check: the CPU cost of one rebuild on the worker, and the render thread's
    /// cost a frame (the gathering, the sending and the upload) with a rebuild asked every frame,
    /// in release (`--release -- --ignored --nocapture`), for 1024 sources spread over the grid,
    /// 1024 packed in 16×4×16 blocks at the camera, none, and the spread 1024 with 100 sections
    /// full of lava around them (409,600 listed blocks, none a source), with the cost of
    /// copying every listed block on the render thread, as the gathering did before it shared
    /// the sections' lists. Kept as a regression bench: a
    /// rebuild on the render thread took about 2.3 ms for 1024 sources while it allocated a list
    /// a cell; the render thread's share should stay well under HDR pacing's 0.5 ms.
    #[test]
    #[ignore = "timing"]
    fn rebuild_timings() {
        let mut table = Sources::default();
        table.insert(1, lit([1.0; 3], 15, 1));
        let camera = DVec3::new(0.5, 64.5, 0.5);
        let spread: Vec<_> = (0..16)
            .flat_map(|x| (0..4).flat_map(move |y| (0..16).map(move |z| (x, y, z))))
            .map(|(x, y, z)| ([x * 8 - 64, y * 32 + 2, z * 8 - 64], 1))
            .collect();
        let packed: Vec<_> = (0..16)
            .flat_map(|x| (0..4).flat_map(move |y| (0..16).map(move |z| (x, y, z))))
            .map(|(x, y, z)| ([x - 8, y + 62, z - 8], 1))
            .collect();
        // Lava (state 2, not a source) fills sections -5 to 4 in x and z, blocks 48 to 63 high.
        let lava = (-80..80)
            .flat_map(|x| (48..64).flat_map(move |y| (-80..80).map(move |z| ([x, y, z], 2))));
        let lava: Vec<_> = spread.iter().copied().chain(lava).collect();
        let device = crate::gfx::test_gpu::optional_device();
        let cases = [
            ("spread", spread, 1024),
            ("packed", packed, 1024),
            ("none", Vec::new(), 0),
            ("spread + lava", lava, 1024),
        ];
        for (name, blocks, count) in cases {
            let sections = sections(&blocks);
            let origin = grid_origin(camera);
            let runs: u32 = 200;
            let (mut builder, mut job) = (Builder::default(), Job::default());
            (job.origin, job.camera) = (origin, camera);
            gather(origin, MIN_Y, &sections, &mut job.blocks);
            let start = Instant::now();
            let copied: Vec<_> = job.blocks.iter().flat_map(|l| l.iter().copied()).collect();
            let copy = start.elapsed().as_secs_f64() * 1e6;
            assert_eq!(copied.len(), blocks.len());
            let start = Instant::now();
            for _ in 0..runs {
                builder.build(&table, &mut job);
            }
            let built = start.elapsed().as_secs_f64() * 1e6 / f64::from(runs);
            let words = &job.words;
            assert_eq!(words[0], count);
            let uploaded = RAY_LIGHT_HEADER + words[0] * RAY_LIGHT_WORDS + WORDS - COUNTS;
            let mut line = format!(
                "{name}: worker build {built:.0} µs, {} bytes, copying the {} listed blocks {copy:.0} µs",
                uploaded * 4,
                blocks.len()
            );
            if let Some((_, device, queue)) = &device {
                let mut lights = RayLights::new(device);
                lights.start(table.clone());
                let (mut took, mut uploads) = (Vec::new(), 0);
                // A step to and fro every frame: a rebuild asked each, as source changes can't.
                for frame in 0..runs {
                    let camera = camera + DVec3::X * f64::from(16 * (frame % 2));
                    let shown = lights.shown;
                    let start = Instant::now();
                    lights.update(queue, camera, (&sections, MIN_Y), 0);
                    took.push(start.elapsed().as_secs_f64() * 1e6);
                    uploads += usize::from(lights.shown != shown);
                    queue.submit([]);
                    // About a frame at 240 fps.
                    std::thread::sleep(Duration::from_millis(4));
                }
                // The first frame only sends the first job (nothing is built yet to upload).
                let first = took.remove(0);
                let mean = took.iter().sum::<f64>() / f64::from(runs - 1);
                took.sort_by(f64::total_cmp);
                let (p99, most) = (took[took.len() * 99 / 100], took[took.len() - 1]);
                line += &format!(
                    ", render thread first {first:.0} µs, then {mean:.0} µs a frame \
                     (99% under {p99:.0}, most {most:.0}), {uploads} uploads"
                );
            }
            eprintln!("{line}");
        }
    }
}
