//! Prepared terrain rules and bounded resident-column work for outbound terrain.
use super::chunks::{ChunkEncoder, TerrainBody, TerrainEncoder, TerrainSections};
use crate::{
    chunk::Chunk,
    fast_hash::{FxHashMap, FxHashSet},
    section_mask::SectionMask,
    world::World,
};
use horizon_core::nbt::Tag;
use mod_api::{BlockState, TerrainPattern, TerrainView, TerrainViewError};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, Weak, mpsc},
};

pub(super) type Column = (i32, i32);
pub(super) type Pos = (i32, i32, i32);
pub(super) type RuleId = (&'static str, u32);

pub(super) const FACES: [Pos; 6] = [
    (-1, 0, 0),
    (1, 0, 0),
    (0, -1, 0),
    (0, 1, 0),
    (0, 0, -1),
    (0, 0, 1),
];
const HORIZONTAL: [Column; 4] = [(-1, 0), (1, 0), (0, -1), (0, 1)];
const IN_FLIGHT: usize = 32;
const CACHE_BYTES: usize = 32 << 20;
const CACHE_COLUMNS: usize = 512;

pub(super) fn column((x, _, z): Pos) -> Column {
    (x >> 4, z >> 4)
}

pub(super) fn adjacent((x, z): Column) -> impl Iterator<Item = Column> {
    HORIZONTAL.into_iter().map(move |(dx, dz)| (x + dx, z + dz))
}

pub(super) fn offset((x, y, z): Pos, (dx, dy, dz): Pos) -> Pos {
    (x + dx, y + dy, z + dz)
}

pub(super) fn known(chunk: &Chunk, x: i32, y: i32, z: i32) -> Option<u32> {
    chunk
        .section_at(y)
        .ok()
        .map(|s| s.get(crate::chunk::index(x, y, z)))
}

fn world_state(world: &World, pos @ (x, y, z): Pos) -> Option<u32> {
    world
        .chunks
        .get(&column(pos))
        .and_then(|c| known(c, x, y, z))
}

struct Band {
    ys: std::ops::Range<i32>,
    targets: FxHashSet<u32>,
    replacements: Vec<u32>,
    pattern: TerrainPattern,
}

pub(super) struct Rules {
    pub(super) generation: u64,
    pub(super) spec: TerrainView,
    bands: Vec<Band>,
    targets: FxHashSet<u32>,
    occluding: FxHashSet<u32>,
}

impl Rules {
    fn compile(
        generation: u64,
        spec: &TerrainView,
        resolve: impl Fn(BlockState) -> Option<u32>,
    ) -> Result<Self, TerrainViewError> {
        if spec.bands.len() > 16
            || spec.reveal_radius > 4
            || spec.occluding.len().saturating_add(
                spec.bands
                    .iter()
                    .map(|band| band.targets.len())
                    .sum::<usize>(),
            ) > 65_536
        {
            return Err(TerrainViewError::InvalidRules);
        }
        let invalid = TerrainViewError::InvalidRules;
        let occluding = spec
            .occluding
            .iter()
            .copied()
            .map(&resolve)
            .collect::<Option<_>>()
            .ok_or(invalid)?;
        let mut bands = Vec::new();
        for band in &spec.bands {
            if band.min_y >= band.max_y
                || band.replacements.is_empty()
                || band.replacements.len() > 256
                || !matches!(
                    band.pattern,
                    TerrainPattern::Block(_) | TerrainPattern::Layer(_)
                )
            {
                return Err(TerrainViewError::InvalidRules);
            }
            bands.push(Band {
                ys: band.min_y..band.max_y,
                targets: band
                    .targets
                    .iter()
                    .copied()
                    .map(&resolve)
                    .collect::<Option<_>>()
                    .ok_or(invalid)?,
                replacements: band
                    .replacements
                    .iter()
                    .copied()
                    .map(&resolve)
                    .collect::<Option<_>>()
                    .ok_or(invalid)?,
                pattern: band.pattern,
            });
        }
        Ok(Self {
            generation,
            spec: spec.clone(),
            targets: bands
                .iter()
                .flat_map(|b| b.targets.iter().copied())
                .collect(),
            bands,
            occluding,
        })
    }

    pub(super) fn eligible(&self, y: i32, palette: Option<&[u32]>) -> bool {
        self.bands
            .iter()
            .any(|band| band.ys.start < y + 16 && y < band.ys.end)
            && palette.is_none_or(|values| values.iter().any(|v| self.targets.contains(v)))
    }

    pub(super) fn targets_state(&self, y: i32, state: u32) -> bool {
        self.targets.contains(&state)
            && self
                .bands
                .iter()
                .any(|band| band.ys.contains(&y) && band.targets.contains(&state))
    }

    pub(super) fn covers(&self, state: Option<u32>) -> bool {
        state.map_or(self.spec.unavailable_occludes, |s| {
            self.occluding.contains(&s)
        })
    }

    pub(super) fn state(
        &self,
        pos: Pos,
        state: u32,
        mut read: impl FnMut(Pos) -> Option<u32>,
    ) -> u32 {
        let (x, y, z) = pos;
        if !self.targets.contains(&state) {
            return state;
        }
        let Some(band) = self
            .bands
            .iter()
            .find(|b| b.ys.contains(&y) && b.targets.contains(&state))
        else {
            return state;
        };
        if !FACES
            .iter()
            .all(|&face| self.covers(read(offset(pos, face))))
        {
            return state;
        }
        if band.replacements.len() == 1 {
            return band.replacements[0];
        }
        let ((x, z), seed) = match band.pattern {
            TerrainPattern::Block(seed) => ((x, z), seed),
            TerrainPattern::Layer(seed) => (column(pos), seed),
            _ => unreachable!("terrain patterns are validated by the SDK"),
        };
        let mut hash = seed
            ^ (x as u64).wrapping_mul(0x9e3779b97f4a7c15)
            ^ (y as u64).wrapping_mul(0xbf58476d1ce4e5b9)
            ^ (z as u64).wrapping_mul(0x94d049bb133111eb);
        hash = (hash ^ (hash >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        hash = (hash ^ (hash >> 27)).wrapping_mul(0x94d049bb133111eb);
        band.replacements[((hash ^ (hash >> 31)) % band.replacements.len() as u64) as usize]
    }

    pub(super) fn in_world(&self, world: &World, pos: Pos, state: u32) -> u32 {
        self.state(pos, state, |p| world_state(world, p))
    }
}

pub(super) type Neighbours = [Option<Arc<Chunk>>; 4];

pub(super) fn neighbours(world: &World, (x, z): Column) -> Neighbours {
    HORIZONTAL.map(|(dx, dz)| world.chunks.get(&(x + dx, z + dz)).cloned())
}

/// The cells on adjacent faces whose authoritative states this projection actually read.
pub(super) struct Faces {
    faces: [Face; 4],
    /// What a saved column's body conceals, to replay the reveals of a viewer (`Faces::saved`).
    concealed: Option<Vec<Concealed>>,
}

/// A block the body of a saved column shows in another state: what it is, and the tag a reveal sends.
pub(super) struct Concealed {
    pub(super) pos: Pos,
    pub(super) state: u32,
    pub(super) tag: Option<(i32, Tag)>,
}

struct Face {
    stamp: Option<Weak<Chunk>>,
    reads: Vec<FaceRead>,
}

struct FaceRead {
    y: i32,
    along: u8,
    covered: bool,
}

fn current_chunk(chunk: Option<&Arc<Chunk>>, stamp: Option<&Weak<Chunk>>) -> bool {
    chunk.map(Arc::as_ptr) == stamp.map(Weak::as_ptr)
}

impl Faces {
    pub(super) fn new(neighbours: &Neighbours) -> Self {
        Self {
            faces: neighbours.each_ref().map(|c| Face {
                stamp: c.as_ref().map(Arc::downgrade),
                reads: Vec::new(),
            }),
            concealed: None,
        }
    }

    /// [`Self::new`] for a column read from the save, which also keeps what its body conceals: no
    /// level holds the column to read it from again.
    pub(super) fn saved(neighbours: &Neighbours) -> Self {
        Self {
            concealed: Some(Vec::new()),
            ..Self::new(neighbours)
        }
    }

    pub(super) fn conceal(&mut self, pos: Pos, state: u32) {
        if let Some(list) = self.concealed.as_mut() {
            list.push(Concealed {
                pos,
                state,
                tag: None,
            });
        }
    }

    pub(super) fn concealed(&self) -> &[Concealed] {
        self.concealed.as_deref().unwrap_or_default()
    }

    pub(super) fn concealed_mut(&mut self) -> &mut [Concealed] {
        self.concealed.as_deref_mut().unwrap_or_default()
    }

    pub(super) fn read(&mut self, face: usize, (x, y, z): Pos, covered: bool) {
        let along = (if face < 2 { z } else { x }) & 15;
        self.faces[face].reads.push(FaceRead {
            y,
            along: along as u8,
            covered,
        });
    }

    pub(super) fn current(&mut self, world: &World, rules: &Rules, at: Column) -> bool {
        for (i, (key, face)) in adjacent(at).zip(&mut self.faces).enumerate() {
            if face.reads.is_empty() {
                continue;
            }
            let chunk = world.chunks.get(&key);
            if !current_chunk(chunk, face.stamp.as_ref()) {
                if face.reads.iter().any(|read| {
                    let along = i32::from(read.along);
                    let (x, z) = match i {
                        0 => (15, along),
                        1 => (0, along),
                        2 => (along, 15),
                        _ => (along, 0),
                    };
                    rules.covers(chunk.and_then(|c| known(c, x, read.y, z))) != read.covered
                }) {
                    return false;
                }
                face.stamp = chunk.map(Arc::downgrade);
            }
        }
        true
    }

    pub(super) fn bytes(&self) -> usize {
        self.faces
            .iter()
            .map(|face| face.reads.capacity() * std::mem::size_of::<FaceRead>())
            .sum::<usize>()
            + self
                .concealed
                .as_ref()
                .map_or(0, |list| list.capacity() * std::mem::size_of::<Concealed>())
    }
}

impl Selection {
    pub(super) fn state(&self, world: &World, rules: &Rules, pos: Pos, state: u32) -> u32 {
        if self.reveals.contains(&pos) {
            state
        } else {
            rules.in_world(world, pos, state)
        }
    }

    /// [`Self::state`] as it was before the writes `previous` holds (the states they replaced).
    pub(super) fn state_before(
        &self,
        world: &World,
        rules: &Rules,
        pos: Pos,
        state: u32,
        previous: &FxHashMap<Pos, u32>,
    ) -> u32 {
        if self.reveals.contains(&pos) {
            return state;
        }
        rules.state(pos, state, |p| {
            previous
                .get(&p)
                .map(|&s| Some(s))
                .unwrap_or_else(|| world_state(world, p))
        })
    }
}

#[derive(Default)]
pub(super) struct Selection {
    pub(super) id: Option<RuleId>,
    pub(super) refresh: bool,
    pub(super) reveals: FxHashSet<Pos>,
}

#[derive(Default)]
pub(super) struct Views {
    rules: FxHashMap<RuleId, Arc<Rules>>,
    generation: u64,
    pub(super) players: FxHashMap<u32, FxHashMap<String, Selection>>,
    pub(super) resident: Resident,
}

impl Views {
    pub(super) fn define(
        &mut self,
        id: RuleId,
        spec: &TerrainView,
        resolve: impl Fn(BlockState) -> Option<u32>,
    ) -> Result<(), TerrainViewError> {
        if let Some(old) = self.rules.get(&id) {
            if old.spec == *spec {
                return Ok(());
            }
        } else if self.rules.len() == 16 {
            return Err(TerrainViewError::Limit);
        }
        let rules = if let Some(equal) = self.rules.values().find(|rules| rules.spec == *spec) {
            equal.clone()
        } else {
            let next = self.generation + 1;
            let rules = Rules::compile(next, spec, resolve)?;
            self.generation = next;
            Arc::new(rules)
        };
        self.rules.insert(id, rules);
        for selection in self.players.values_mut().flat_map(|v| v.values_mut()) {
            if selection.id == Some(id) {
                selection.refresh = true;
                selection.reveals.clear();
            }
        }
        Ok(())
    }

    pub(super) fn select(
        &mut self,
        conn: u32,
        dimension: &str,
        owner: &'static str,
        id: Option<u32>,
    ) -> Result<(), TerrainViewError> {
        let id = id.map(|id| (owner, id));
        if id.is_some_and(|id| !self.rules.contains_key(&id)) {
            return Err(TerrainViewError::UnknownView);
        }
        let dimensions = self.players.entry(conn).or_default();
        if let Some(selection) = dimensions.get_mut(dimension) {
            if selection.id == id {
                return Ok(());
            }
            if selection.id.is_some_and(|(held, _)| held != owner) {
                return Err(TerrainViewError::Owned);
            }
            if let (Some(old), Some(new)) = (selection.id, id)
                && Arc::ptr_eq(&self.rules[&old], &self.rules[&new])
            {
                selection.id = id;
                return Ok(());
            }
            *selection = Selection {
                id,
                refresh: true,
                ..Selection::default()
            };
        } else if id.is_some() {
            // A released slot (its refresh still pending) is not held by the mod; the second bound
            // keeps those entries from piling up in dimensions that get no pass.
            let held = dimensions.values().filter(|s| s.id.is_some()).count();
            if held == 32 || dimensions.len() == 64 {
                return Err(TerrainViewError::Limit);
            }
            dimensions.insert(
                dimension.to_owned(),
                Selection {
                    id,
                    refresh: true,
                    ..Selection::default()
                },
            );
        }
        Ok(())
    }

    pub(super) fn targets_state(&self, y: i32, state: u32) -> bool {
        self.rules
            .values()
            .any(|rules| rules.targets_state(y, state))
    }

    pub(super) fn view(&self, conn: u32, dimension: &str) -> Option<(&Selection, &Arc<Rules>)> {
        let selection = self.players.get(&conn)?.get(dimension)?;
        Some((selection, self.rules.get(&selection.id?)?))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key(Arc<str>, u64, Column);

pub(in crate::lan) struct EncodeContext<'a> {
    pub dimension: &'a str,
    pub encoder: &'a ChunkEncoder,
    pub world: &'a World,
    pub protos: &'a dyn Fn(i32, i32) -> Option<SectionMask>,
    pub include_far: bool,
}

fn window(at: Column) -> impl Iterator<Item = Column> {
    (-1..=1).flat_map(move |dz| (-1..=1).map(move |dx| (at.0 + dx, at.1 + dz)))
}

/// Light dependencies are occupancy masks. An interior edit or a relight in another column can
/// move its allocation without changing which light sections this packet includes.
struct LightStamp {
    chunk: Option<Weak<Chunk>>,
    proto: Option<SectionMask>,
    occupied: SectionMask,
}

impl LightStamp {
    fn update(&mut self, ctx: &EncodeContext<'_>, encoder: &TerrainEncoder, at: Column) {
        let chunk = ctx.world.chunks.get(&at);
        let proto = if chunk.is_none() {
            (ctx.protos)(at.0, at.1)
        } else {
            None
        };
        if current_chunk(chunk, self.chunk.as_ref()) && self.proto == proto {
            return;
        }
        self.occupied = if let Some(chunk) = chunk {
            ctx.encoder.occupied(chunk)
        } else {
            proto.map_or(SectionMask::EMPTY, |p| encoder.proto_occupied(p))
        };
        self.chunk = chunk.map(Arc::downgrade);
        self.proto = proto;
    }
}

fn occupancy(lights: &[LightStamp; 9]) -> SectionMask {
    lights
        .iter()
        .fold(SectionMask::EMPTY, |mask, light| mask | light.occupied)
}

#[derive(Clone)]
pub(super) enum Ready {
    Ordinary,
    Shared(Arc<TerrainBody>),
}

struct Cached {
    chunk: Weak<Chunk>,
    faces: Faces,
    lights: [LightStamp; 9],
    around: SectionMask,
    encoder: TerrainEncoder,
    ready: Ready,
    include_far: bool,
}

impl Cached {
    fn bytes(&self) -> usize {
        self.faces.bytes()
            + match &self.ready {
                Ready::Ordinary => 0,
                Ready::Shared(body) => body.bytes(),
            }
    }

    fn current(&mut self, ctx: &EncodeContext<'_>, chunk: &Arc<Chunk>, rules: &Rules) -> bool {
        let at = (chunk.x, chunk.z);
        if !self.encoder.matches(ctx.encoder)
            || Weak::as_ptr(&self.chunk) != Arc::as_ptr(chunk)
            || !self.faces.current(ctx.world, rules, at)
        {
            return false;
        }
        if matches!(self.ready, Ready::Ordinary) {
            return true;
        }
        for (at, light) in window(at).zip(&mut self.lights) {
            light.update(ctx, &self.encoder, at);
        }
        self.around == occupancy(&self.lights) && (!ctx.include_far || self.include_far)
    }
}

struct Job {
    key: Key,
    chunk: Arc<Chunk>,
    columns: [Option<Arc<Chunk>>; 9],
    protos: [Option<SectionMask>; 9],
    encoder: TerrainEncoder,
    rules: Arc<Rules>,
    include_far: bool,
}

impl Job {
    fn run(self, sections: &mut TerrainSections) -> (Key, Cached) {
        let neighbours = [3, 5, 1, 7].map(|i| self.columns[i].clone());
        let mut faces = Faces::new(&neighbours);
        let wires = self
            .encoder
            .encode(&self.chunk, &neighbours, &self.rules, &mut faces);
        let unchanged = wires.unchanged();
        let lights = std::array::from_fn(|i| {
            let chunk = self.columns[i].as_ref();
            let proto = self.protos[i];
            let occupied = if unchanged {
                SectionMask::EMPTY
            } else {
                chunk.map_or_else(
                    || proto.map_or(SectionMask::EMPTY, |p| self.encoder.proto_occupied(p)),
                    |chunk| self.encoder.occupied(chunk),
                )
            };
            LightStamp {
                chunk: chunk.map(Arc::downgrade),
                proto,
                occupied,
            }
        });
        let around = occupancy(&lights);
        let ready = if unchanged {
            Ready::Ordinary
        } else {
            Ready::Shared(Arc::new(self.encoder.body(
                &self.chunk,
                wires,
                around,
                self.include_far,
                sections,
            )))
        };
        (
            self.key,
            Cached {
                chunk: Arc::downgrade(&self.chunk),
                faces,
                lights,
                around,
                encoder: self.encoder,
                ready,
                include_far: self.include_far,
            },
        )
    }
}

type Done = (Key, Cached);

struct Worker {
    jobs: mpsc::SyncSender<Job>,
    done: mpsc::Receiver<Done>,
}

impl Worker {
    fn start() -> Option<Self> {
        let (jobs, work) = mpsc::sync_channel::<Job>(IN_FLIGHT);
        let (reply, done) = mpsc::channel();
        let work = Arc::new(Mutex::new(work));
        let workers =
            std::thread::available_parallelism().map_or(1, |n| if n.get() >= 4 { 2 } else { 1 });
        let mut started = false;
        for index in 0..workers {
            let (work, reply) = (work.clone(), reply.clone());
            started |= std::thread::Builder::new()
                .name(format!("Terrain view encoder #{index}"))
                .spawn(move || {
                    let mut sections = TerrainSections::default();
                    loop {
                        let Ok(job) = work.lock().unwrap().recv() else {
                            return;
                        };
                        if reply.send(job.run(&mut sections)).is_err() {
                            return;
                        }
                    }
                })
                .is_ok();
        }
        started.then_some(Self { jobs, done })
    }
}

#[derive(Default)]
pub(super) struct Resident {
    #[cfg(test)]
    pub(in crate::lan) submitted: usize,
    worker: Option<Worker>,
    dimension: Option<Arc<str>>,
    // Every candidate is checked or queued once per level pass, even while many guests share
    // the same queue at spawn. None means the work is still pending or the queue had no room.
    pass: FxHashMap<Key, Option<Ready>>,
    pending: FxHashSet<Key>,
    cache: FxHashMap<Key, Cached>,
    ordering: VecDeque<Key>,
    bytes: usize,
}

impl Resident {
    pub(super) fn poll(&mut self) {
        self.pass.clear();
        let Some(worker) = self.worker.as_ref() else {
            return;
        };
        while let Ok((key, result)) = worker.done.try_recv() {
            self.pending.remove(&key);
            self.bytes += result.bytes();
            self.cache.insert(key.clone(), result);
            self.ordering.push_back(key);
            while self.bytes > CACHE_BYTES || self.cache.len() > CACHE_COLUMNS {
                if let Some(old) = self
                    .ordering
                    .pop_front()
                    .and_then(|k| self.cache.remove(&k))
                {
                    self.bytes -= old.bytes();
                }
            }
        }
    }

    pub(super) fn can_queue(&self) -> bool {
        self.pending.len() < IN_FLIGHT
    }

    pub(super) fn can_warm(&self) -> bool {
        self.pending.len() < IN_FLIGHT / 2
    }

    pub(super) fn get(
        &mut self,
        ctx: &EncodeContext<'_>,
        rules: &Arc<Rules>,
        at: Column,
    ) -> Option<Ready> {
        if self.dimension.as_deref() != Some(ctx.dimension) {
            self.dimension = Some(Arc::from(ctx.dimension));
        }
        let key = Key(
            self.dimension.as_ref().unwrap().clone(),
            rules.generation,
            at,
        );
        if let Some(ready) = self.pass.get(&key) {
            return ready.clone();
        }
        let ready = self.lookup(&key, ctx, rules);
        if self.pass.len() < CACHE_COLUMNS {
            self.pass.insert(key, ready.clone());
        }
        ready
    }

    fn lookup(&mut self, key: &Key, ctx: &EncodeContext<'_>, rules: &Arc<Rules>) -> Option<Ready> {
        let at = key.2;
        let chunk = ctx.world.chunks.get(&at)?;
        if let Some(cached) = self.cache.get_mut(key) {
            if cached.current(ctx, chunk, rules) {
                return Some(cached.ready.clone());
            }
            self.bytes -= cached.bytes();
            self.cache.remove(key);
            self.ordering.retain(|k| k != key);
        }
        if self.pending.contains(key) {
            return None;
        }
        if !ctx.encoder.terrain_eligible(chunk, rules) {
            return Some(Ready::Ordinary);
        }
        if !self.can_queue() {
            return None;
        }
        if self.worker.is_none() {
            self.worker = Worker::start();
        }
        let mut columns = std::array::from_fn(|_| None);
        let mut protos = [None; 9];
        for (i, at) in window(at).enumerate() {
            if let Some(chunk) = ctx.world.chunks.get(&at) {
                columns[i] = Some(chunk.clone());
            } else {
                protos[i] = (ctx.protos)(at.0, at.1);
            }
        }
        if self
            .worker
            .as_ref()?
            .jobs
            .try_send(Job {
                key: key.clone(),
                chunk: chunk.clone(),
                columns,
                protos,
                encoder: ctx.encoder.terrain_encoder(),
                rules: rules.clone(),
                include_far: ctx.include_far,
            })
            .is_ok()
        {
            self.pending.insert(key.clone());
            #[cfg(test)]
            {
                self.submitted += 1;
            }
        }
        None
    }
}
