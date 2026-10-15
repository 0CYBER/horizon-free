//! Sky and block light for one chunk, flood-filled over its 3x3 neighbourhood
//! with vanilla's `LightEngine` rules:
//! - light entering a block loses `max(1, getLightDampening)` (`getOpacity`);
//! - it cannot cross a face where the two blocks' occlusion faces together
//!   cover the whole square (`shapeOccludes`, using `getFaceOcclusionShape`
//!   only for states with `canOcclude && useShapeForLightOcclusion`), which is
//!   what makes slabs, stairs, snow layers etc. block light directionally;
//! - sky sources run down each column to the first occluded edge
//!   (`ChunkSkyLightSources.isEdgeOccluded`: dampening != 0 or the faces
//!   between the two blocks occlude).
//!
//! The final values of vanilla's queue-based engine are the maximum over all
//! paths, so a flood fill to the fixed point reproduces them. Light travels at
//! most 15 blocks, so the 48-wide region determines the centre exactly.
//! `relight_block` applies vanilla's incremental decrease/increase passes
//! directly to resident chunks after a block change.
use crate::{
    blocks::BlockFacts,
    chunk::{Chunk, HEIGHT, Light, MIN_Y, SECTIONS, index},
};
use horizon_core::geometry::{Direction, ShapeOp, VoxelShape};
use horizon_world::registry::{BlockRegistry, BlockStateId};
use std::{
    collections::{HashMap, VecDeque},
    path::Path,
    sync::Arc,
};

const W: usize = 48;
/// Face ID 0: empty occlusion face; 1: the full square.
const EMPTY: u16 = 0;
const FULL: u16 = 1;

#[derive(Clone, Copy)]
struct Cell {
    dampening: u8,
    emission: u8,
    /// Any face is non-empty (`!isEmptyShape`), so edges need the face table.
    shaped: bool,
    faces: [u16; 6],
}
pub struct LightTables {
    cells: Vec<Cell>,
    /// `faceShapeOccludes(a, b)` for every pair of distinct face shapes.
    occludes: Vec<bool>,
    faces: usize,
}
impl LightTables {
    /// Dampening and emission only: every occlusion face is empty.
    pub fn new(facts: &BlockFacts) -> Self {
        Self {
            cells: facts
                .states
                .iter()
                .map(|s| Cell {
                    dampening: s.dampening.min(15),
                    emission: s.emission.min(15),
                    shaped: false,
                    faces: [EMPTY; 6],
                })
                .collect(),
            occludes: vec![false, true, true, true],
            faces: 2,
        }
    }
    /// Full tables with the vanilla occlusion shapes from `occlusion.nbt`.
    pub fn load(root: &Path, registry: &BlockRegistry, facts: &BlockFacts) -> crate::Result<Self> {
        use horizon_world::{
            collision::CachedCollisionRegistry,
            nbt::{self, Limits},
        };
        let data = nbt::read_named(
            &std::fs::read(root.join("occlusion.nbt"))?,
            Limits::default(),
        )?
        .0
        .tag;
        let occlusion = CachedCollisionRegistry::from_nbt(&data, registry)?;
        let mut tables = Self::new(facts);
        let block = VoxelShape::block();
        let mut shapes: Vec<VoxelShape> = vec![VoxelShape::empty(), block.clone()];
        let mut by_geometry: HashMap<Vec<u64>, u16> = HashMap::new();
        let mut by_shape: HashMap<*const VoxelShape, [u16; 6]> = HashMap::new();
        for (id, cell) in tables.cells.iter_mut().enumerate() {
            let state = &facts.states[id];
            // LightEngine.isEmptyShape: !canOcclude || !useShapeForLightOcclusion.
            if !state.can_occlude || !state.shape_occlusion {
                continue;
            }
            let Some(shape) = occlusion.cached_shape(BlockStateId(id as u32))? else {
                continue;
            };
            if shape.is_empty() {
                continue;
            }
            let faces = if let Some(faces) = by_shape.get(&std::ptr::from_ref(shape)) {
                *faces
            } else {
                let mut faces = [FULL; 6];
                // BlockState.initCache: solidRender shapes use the full faces.
                if shape.join_is_not_empty(&block, ShapeOp::XOR)? {
                    for direction in Direction::ALL {
                        let face = shape.face_shape(direction)?;
                        faces[direction as usize] = if face.is_empty() {
                            EMPTY
                        } else if face.same_identity(&block) {
                            FULL
                        } else {
                            let key: Vec<u64> = face
                                .boxes()
                                .iter()
                                .flat_map(|b| {
                                    b.min_position()
                                        .components()
                                        .into_iter()
                                        .chain(b.max_position().components())
                                })
                                .map(f64::to_bits)
                                .collect();
                            *by_geometry.entry(key).or_insert_with(|| {
                                shapes.push(face.clone());
                                (shapes.len() - 1) as u16
                            })
                        };
                    }
                }
                by_shape.insert(std::ptr::from_ref(shape), faces);
                faces
            };
            cell.faces = faces;
            cell.shaped = faces.iter().any(|f| *f != EMPTY);
        }
        let n = shapes.len();
        let mut occludes = vec![false; n * n];
        for a in 0..n {
            for b in 0..n {
                occludes[a * n + b] = if a == FULL as usize || b == FULL as usize {
                    true
                } else if a == EMPTY as usize && b == EMPTY as usize {
                    false
                } else {
                    // Shapes.faceShapeOccludes: the union covers the whole face.
                    let union = shapes[a].join_unoptimized(&shapes[b], ShapeOp::OR)?;
                    !block.join_is_not_empty(&union, ShapeOp::ONLY_FIRST)?
                };
            }
        }
        tables.occludes = occludes;
        tables.faces = n;
        Ok(tables)
    }
    /// `LightEngine.shapeOccludes(from, to, direction)`.
    #[inline]
    fn blocked(&self, from: usize, to: usize, direction: usize) -> bool {
        let (a, b) = (&self.cells[from], &self.cells[to]);
        if !a.shaped && !b.shaped {
            return false;
        }
        let fa = a.faces[direction] as usize;
        let fb = b.faces[direction ^ 1] as usize;
        self.occludes[fa * self.faces + fb]
    }
    /// `ChunkSkyLightSources.isEdgeOccluded(top, bottom)`.
    #[inline]
    fn edge_occluded(&self, top: usize, bottom: usize) -> bool {
        self.cells[bottom].dampening != 0 || self.blocked(top, bottom, Direction::Down as usize)
    }
    pub fn emission(&self, state: u16) -> u8 {
        self.cells[state as usize].emission
    }
    pub fn dampening(&self, state: u16) -> u8 {
        self.cells[state as usize].dampening
    }
    /// Whether a state change can change light (`hasDifferentLightProperties`).
    pub fn differs(&self, old: u16, new: u16) -> bool {
        let (a, b) = (&self.cells[old as usize], &self.cells[new as usize]);
        old != new
            && (a.dampening != b.dampening || a.emission != b.emission || a.shaped || b.shaped)
    }
}
// Direction index order matches `Direction` (down, up, north, south, west, east);
// `d ^ 1` is the opposite direction.
const OFFSETS: [(i32, i32, i32); 6] = [
    (0, -1, 0),
    (0, 1, 0),
    (0, 0, -1),
    (0, 0, 1),
    (-1, 0, 0),
    (1, 0, 0),
];

/// `region[dz][dx]` holds the chunks around the centre (index 1,1).
/// `sky` is false in dimensions without skylight (the Nether).
pub fn light_chunk(
    region: &[[Option<&Chunk>; 3]; 3],
    tables: &LightTables,
    sky_light: bool,
) -> Vec<Light> {
    region[1][1].expect("centre chunk");
    // Highest occupied section in the region bounds the work.
    let mut top_section = 0;
    for chunk in region.iter().flatten().flatten() {
        for (i, s) in chunk.sections.iter().enumerate().rev() {
            if !s.is_uniform(0) {
                top_section = top_section.max(i + 1);
                break;
            }
        }
    }
    // One extra section above the highest occupied one: block light rises
    // up to 14 blocks into it (sky light there is always 15).
    let h = ((top_section + 1) * 16).min(HEIGHT as usize);
    let at = |x: usize, y: usize, z: usize| (y * W + z) * W + x;
    let mut states = vec![0u16; W * W * h];
    let mut sky = vec![0u8; W * W * h];
    let mut block = vec![0u8; W * W * h];
    let mut queue = VecDeque::new();
    let mut sky_queue = VecDeque::new();
    let mut ids = Box::new([0u16; 4096]);
    for (dz, row) in region.iter().enumerate() {
        for (dx, chunk) in row.iter().enumerate() {
            let Some(chunk) = chunk else { continue };
            for (si, section) in chunk.sections.iter().enumerate().take(top_section) {
                if section.is_uniform(0) {
                    continue;
                }
                section.unpack(&mut ids);
                for (i, &id) in ids.iter().enumerate() {
                    let (lx, lz, ly) = (i & 15, (i >> 4) & 15, i >> 8);
                    let p = at(dx * 16 + lx, si * 16 + ly, dz * 16 + lz);
                    states[p] = id;
                    let emission = tables.cells[id as usize].emission;
                    if emission > 0 {
                        block[p] = emission;
                        queue.push_back(p as u32);
                    }
                }
            }
        }
    }
    // Columns of missing neighbour chunks neither hold nor pass light.
    let present = |x: usize, z: usize| region[z / 16][x / 16].is_some();
    let mut lowest = vec![h as i32; W * W];
    for z in 0..W {
        for x in 0..W {
            if !present(x, z) || !sky_light {
                continue;
            }
            // The top layer (above the occupied range) is always open sky.
            let mut y = h as i32 - 1;
            sky[at(x, y as usize, z)] = 15;
            while y > 0
                && !tables.edge_occluded(
                    states[at(x, y as usize, z)] as usize,
                    states[at(x, y as usize - 1, z)] as usize,
                )
            {
                y -= 1;
                sky[at(x, y as usize, z)] = 15;
            }
            lowest[z * W + x] = y;
        }
    }
    for z in 0..W {
        for x in 0..W {
            if !present(x, z) || !sky_light {
                continue;
            }
            let own = lowest[z * W + x];
            let mut deepest = own;
            for (nx, nz) in [
                (x.wrapping_sub(1), z),
                (x + 1, z),
                (x, z.wrapping_sub(1)),
                (x, z + 1),
            ] {
                if nx < W && nz < W && present(nx, nz) {
                    deepest = deepest.max(lowest[nz * W + nx]);
                }
            }
            // Seed propagation from the source cells that border darker cells.
            let from = own.max(0) as usize;
            let to = (deepest as usize).min(h - 1);
            for y in from..=to {
                if sky[at(x, y, z)] == 15 {
                    sky_queue.push_back(at(x, y, z) as u32);
                }
            }
        }
    }
    let spread = |light: &mut [u8], queue: &mut VecDeque<u32>| {
        while let Some(p) = queue.pop_front() {
            let p = p as usize;
            let level = light[p];
            if level <= 1 {
                continue;
            }
            let x = p % W;
            let z = (p / W) % W;
            let y = p / (W * W);
            let from = states[p] as usize;
            for (d, (ox, oy, oz)) in OFFSETS.iter().enumerate() {
                let (nx, ny, nz) = (x as i32 + ox, y as i32 + oy, z as i32 + oz);
                if nx < 0 || nz < 0 || ny < 0 || nx >= W as i32 || nz >= W as i32 || ny >= h as i32
                {
                    continue;
                }
                let (nx, ny, nz) = (nx as usize, ny as usize, nz as usize);
                if !present(nx, nz) {
                    continue;
                }
                let q = at(nx, ny, nz);
                let to = states[q] as usize;
                let next = level.saturating_sub(tables.cells[to].dampening.max(1));
                if next > light[q] && !tables.blocked(from, to, d) {
                    light[q] = next;
                    queue.push_back(q as u32);
                }
            }
        }
    };
    spread(&mut sky, &mut sky_queue);
    spread(&mut block, &mut queue);
    pack_lighting(h, &sky, &block, sky_light)
}

/// Extract just the centre column's light; never copy worker snapshot sections.
fn pack_lighting(h: usize, sky: &[u8], block: &[u8], sky_light: bool) -> Vec<Light> {
    let at = |x: usize, y: usize, z: usize| (y * W + z) * W + x;
    let open = if sky_light { 0xF0 } else { 0x00 };
    (0..SECTIONS)
        .map(|si| {
            if si * 16 >= h {
                return Light::Uniform(open);
            }
            let mut data = Box::new([open; 4096]);
            for ly in 0..16 {
                let y = si * 16 + ly;
                if y >= h {
                    continue;
                }
                for lz in 0..16 {
                    for lx in 0..16 {
                        let p = at(16 + lx, y, 16 + lz);
                        data[index(lx as i32, ly as i32, lz as i32)] = sky[p] << 4 | block[p];
                    }
                }
            }
            if data.iter().all(|&v| v == data[0]) {
                Light::Uniform(data[0])
            } else {
                Light::Full(data)
            }
        })
        .collect()
}

/// Move a worker's light into current sections, preserving later block/biome
/// edits and immutable snapshots retained by other workers.
pub fn apply_lighting(chunk: &mut Chunk, lights: Vec<Light>) {
    for (section, light) in chunk.sections.iter_mut().zip(lights) {
        Arc::make_mut(section).light = light;
    }
}

/// Resident chunks the incremental relighter reads and writes.
pub trait LightWorld {
    fn state(&self, x: i32, y: i32, z: i32) -> Option<u16>;
    /// Packed light (sky << 4 | block), None outside loaded chunks.
    fn light(&self, x: i32, y: i32, z: i32) -> Option<u8>;
    fn set_light(&mut self, x: i32, y: i32, z: i32, packed: u8);
}
impl<S: std::hash::BuildHasher> LightWorld for HashMap<(i32, i32), Arc<Chunk>, S> {
    fn state(&self, x: i32, y: i32, z: i32) -> Option<u16> {
        if !(MIN_Y..MIN_Y + HEIGHT).contains(&y) {
            return None;
        }
        self.get(&(x >> 4, z >> 4))
            .filter(|c| c.lit)
            .map(|c| c.get(x & 15, y, z & 15))
    }
    fn light(&self, x: i32, y: i32, z: i32) -> Option<u8> {
        if !(MIN_Y..MIN_Y + HEIGHT).contains(&y) {
            return None;
        }
        self.get(&(x >> 4, z >> 4))
            .filter(|c| c.lit)
            .map(|c| c.light(x & 15, y, z & 15))
    }
    fn set_light(&mut self, x: i32, y: i32, z: i32, packed: u8) {
        let Some(chunk) = self.get_mut(&(x >> 4, z >> 4)) else {
            return;
        };
        let s = ((y - MIN_Y) >> 4) as usize;
        let i = index(x, y, z);
        if chunk.sections[s].light(i) == packed {
            return;
        }
        let section = Arc::make_mut(&mut Arc::make_mut(chunk).sections[s]);
        if let Light::Uniform(v) = section.light {
            section.light = Light::Full(Box::new([v; 4096]));
        }
        if let Light::Full(a) = &mut section.light {
            a[i] = packed;
        }
    }
}
type Pos = (i32, i32, i32);
fn level(world: &impl LightWorld, sky: bool, p: Pos) -> Option<u8> {
    world
        .light(p.0, p.1, p.2)
        .map(|v| if sky { v >> 4 } else { v & 15 })
}
fn set_level(world: &mut impl LightWorld, sky: bool, p: Pos, value: u8) {
    let Some(old) = world.light(p.0, p.1, p.2) else {
        return;
    };
    let packed = if sky {
        (old & 0x0F) | value << 4
    } else {
        (old & 0xF0) | value
    };
    world.set_light(p.0, p.1, p.2, packed);
}
/// `ChunkSkyLightSources`: the Y of the upper block of the highest occluded
/// edge in the column (every block at or above it is a sky source).
pub fn lowest_sky_source(world: &impl LightWorld, tables: &LightTables, x: i32, z: i32) -> i32 {
    let mut y = MIN_Y + HEIGHT - 1;
    let Some(mut top) = world.state(x, y, z) else {
        return MIN_Y;
    };
    while y > MIN_Y {
        let Some(bottom) = world.state(x, y - 1, z) else {
            break;
        };
        if tables.edge_occluded(top as usize, bottom as usize) {
            return y;
        }
        top = bottom;
        y -= 1;
    }
    MIN_Y
}
/// Vanilla's `checkBlock` + `runLightUpdates` for changed positions on
/// resident, lit chunks: clear the light that may have come through or from
/// the old states (decrease pass), then re-seed sources and propagate back in
/// from the brighter boundary (increase pass). Returns every position whose
/// stored light changed, for remeshing. `force` clears around each position
/// as if it had been fully lit, for light computed from outdated blocks.
pub fn relight(
    world: &mut impl LightWorld,
    tables: &LightTables,
    sky_light: bool,
    positions: &[Pos],
    force: bool,
) -> Vec<Pos> {
    let mut changed = Vec::new();
    relight_layer(
        world,
        tables,
        false,
        positions.to_vec(),
        force,
        &mut changed,
    );
    if sky_light {
        let mut seeds = positions.to_vec();
        let mut columns: HashMap<(i32, i32), i32> = HashMap::new();
        for &(x, _, z) in positions {
            if columns.contains_key(&(x, z)) {
                continue;
            }
            let source = lowest_sky_source(world, tables, x, z);
            columns.insert((x, z), source);
            // Cells whose source status changed: non-15 at or above the new
            // lowest source, and old 15s (sources) below it.
            for y in source..MIN_Y + HEIGHT {
                if level(world, true, (x, y, z)).is_some_and(|l| l != 15) {
                    seeds.push((x, y, z));
                }
            }
            let mut y = source - 1;
            while y >= MIN_Y && level(world, true, (x, y, z)) == Some(15) {
                seeds.push((x, y, z));
                y -= 1;
            }
        }
        relight_layer(world, tables, true, seeds, force, &mut changed);
    }
    changed.sort_unstable();
    changed.dedup();
    changed
}
fn relight_layer(
    world: &mut impl LightWorld,
    tables: &LightTables,
    sky: bool,
    seeds: Vec<Pos>,
    force: bool,
    changed: &mut Vec<Pos>,
) {
    let mut decrease: VecDeque<(Pos, u8)> = VecDeque::new();
    let mut cleared: Vec<Pos> = Vec::new();
    let mut increase: VecDeque<Pos> = VecDeque::new();
    for p in seeds {
        let Some(old) = level(world, sky, p) else {
            continue;
        };
        if old > 0 {
            set_level(world, sky, p, 0);
            changed.push(p);
        }
        cleared.push(p);
        decrease.push_back((p, if force { 15 } else { old }));
    }
    while let Some((p, from)) = decrease.pop_front() {
        for (ox, oy, oz) in OFFSETS {
            let q = (p.0 + ox, p.1 + oy, p.2 + oz);
            let Some(l) = level(world, sky, q) else {
                continue;
            };
            if l == 0 {
                continue;
            }
            // Sky light runs straight down without loss, so a 15 below a
            // cleared 15 may have come from it too.
            if l < from || (sky && oy == -1 && l == 15 && from == 15) {
                set_level(world, sky, q, 0);
                changed.push(q);
                cleared.push(q);
                decrease.push_back((q, l));
            } else {
                increase.push_back(q);
            }
        }
    }
    // Sources inside the cleared area light up again.
    let mut sources: HashMap<(i32, i32), i32> = HashMap::new();
    for p in cleared {
        let seed = if sky {
            let lowest = *sources
                .entry((p.0, p.2))
                .or_insert_with(|| lowest_sky_source(world, tables, p.0, p.2));
            if p.1 >= lowest { 15 } else { 0 }
        } else {
            world
                .state(p.0, p.1, p.2)
                .map_or(0, |s| tables.cells[s as usize].emission)
        };
        if seed > level(world, sky, p).unwrap_or(15) {
            set_level(world, sky, p, seed);
            changed.push(p);
            increase.push_back(p);
        }
    }
    while let Some(p) = increase.pop_front() {
        let Some(l) = level(world, sky, p) else {
            continue;
        };
        if l <= 1 {
            continue;
        }
        let Some(from) = world.state(p.0, p.1, p.2) else {
            continue;
        };
        for (d, (ox, oy, oz)) in OFFSETS.iter().enumerate() {
            let q = (p.0 + ox, p.1 + oy, p.2 + oz);
            let Some(to) = world.state(q.0, q.1, q.2) else {
                continue;
            };
            let Some(ql) = level(world, sky, q) else {
                continue;
            };
            let next = l.saturating_sub(tables.cells[to as usize].dampening.max(1));
            if next > ql && !tables.blocked(from as usize, to as usize, d) {
                set_level(world, sky, q, next);
                changed.push(q);
                increase.push_back(q);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AIR: u16 = 0;
    const STONE: u16 = 1;
    const TORCH: u16 = 2;
    const SLAB: u16 = 3;
    /// Air, stone, torch and a bottom slab (full bottom face, half sides,
    /// empty top), with a third face shape for the half sides.
    fn tables() -> LightTables {
        let cell = |dampening, emission, faces: [u16; 6]| Cell {
            dampening,
            emission,
            shaped: faces.iter().any(|f| *f != EMPTY),
            faces,
        };
        LightTables {
            cells: vec![
                cell(0, 0, [EMPTY; 6]),
                cell(15, 0, [FULL; 6]),
                cell(0, 14, [EMPTY; 6]),
                cell(0, 0, [FULL, EMPTY, 2, 2, 2, 2]),
            ],
            // empty, full, half: only pairs with a full face cover the square.
            occludes: vec![false, true, false, true, true, true, false, true, false],
            faces: 3,
        }
    }
    fn world(edit: impl Fn(&mut Chunk)) -> HashMap<(i32, i32), Arc<Chunk>> {
        let mut chunks = HashMap::new();
        for z in -1..=1 {
            for x in -1..=1 {
                let mut c = Chunk::empty(x, z);
                for y in MIN_Y..0 {
                    for i in 0..256 {
                        c.set(i & 15, y, i >> 4, STONE);
                    }
                }
                if (x, z) == (0, 0) {
                    edit(&mut c);
                }
                chunks.insert((x, z), c);
            }
        }
        let lit: Vec<_> = chunks
            .keys()
            .map(|&(x, z)| {
                let region: [[Option<&Chunk>; 3]; 3] = std::array::from_fn(|dz| {
                    std::array::from_fn(|dx| chunks.get(&(x + dx as i32 - 1, z + dz as i32 - 1)))
                });
                let mut c = chunks[&(x, z)].clone();
                apply_lighting(&mut c, light_chunk(&region, &tables(), true));
                c.lit = true;
                ((x, z), Arc::new(c))
            })
            .collect();
        lit.into_iter().collect()
    }
    fn sky(world: &HashMap<(i32, i32), Arc<Chunk>>, x: i32, y: i32, z: i32) -> u8 {
        world.light(x, y, z).unwrap() >> 4
    }
    fn block(world: &HashMap<(i32, i32), Arc<Chunk>>, x: i32, y: i32, z: i32) -> u8 {
        world.light(x, y, z).unwrap() & 15
    }

    #[test]
    fn torch_light_falls_off_by_one_per_block() {
        let w = world(|c| {
            c.set(8, 0, 8, TORCH);
        });
        assert_eq!(block(&w, 8, 0, 8), 14);
        assert_eq!(block(&w, 9, 0, 8), 13);
        assert_eq!(block(&w, 8, 3, 8), 11);
        assert_eq!(block(&w, 8, -1, 8), 0);
    }
    #[test]
    fn bottom_slab_takes_sky_from_above_but_not_through_its_bottom() {
        // A slab roof over a closed stone room at y 0..2.
        let w = world(|c| {
            for x in 4..13 {
                for z in 4..13 {
                    c.set(x, 3, z, SLAB);
                    for y in 0..3 {
                        let wall = x == 4 || x == 12 || z == 4 || z == 12;
                        c.set(x, y, z, if wall { STONE } else { AIR });
                    }
                }
            }
        });
        assert_eq!(
            sky(&w, 8, 3, 8),
            15,
            "slab cell is lit through its open top"
        );
        // Without the shape test the room would be lit (slabs have no dampening).
        assert_eq!(sky(&w, 8, 2, 8), 0, "the full bottom face stops sky light");
    }
    #[test]
    fn incremental_relight_matches_full_relight() {
        let mut w = world(|c| {
            c.set(8, 0, 8, TORCH);
        });
        let edits = [
            ((8, 0, 8), AIR),
            ((9, 1, 8), STONE),
            ((3, 2, 3), TORCH),
            ((8, 5, 8), SLAB),
            ((8, -1, 8), AIR),
        ];
        for ((x, y, z), state) in edits {
            Arc::make_mut(w.get_mut(&(0, 0)).unwrap()).set(x, y, z, state);
            relight(&mut w, &tables(), true, &[(x, y, z)], false);
        }
        let region: [[Option<&Chunk>; 3]; 3] = std::array::from_fn(|dz| {
            std::array::from_fn(|dx| w.get(&(dx as i32 - 1, dz as i32 - 1)).map(|c| &**c))
        });
        let full = light_chunk(&region, &tables(), true);
        for (si, s) in full.iter().enumerate() {
            for i in 0..4096 {
                assert_eq!(
                    s.get(i),
                    w[&(0, 0)].sections[si].light(i),
                    "section {si} index {i}"
                );
            }
        }
    }
    #[test]
    fn lighting_result_preserves_current_edits_and_snapshots() {
        let mut current = Chunk::empty(0, 0);
        current.set(8, MIN_Y, 8, TORCH);
        let region = [[None; 3], [None, Some(&current), None], [None; 3]];
        let lights = light_chunk(&region, &tables(), true);
        let Light::Full(data) = &lights[0] else {
            panic!("torch light varies");
        };
        let result_storage = data.as_ptr();
        let snapshot = current.clone();
        current.set(8, MIN_Y, 8, STONE);
        Arc::make_mut(&mut current.sections[0]).biomes.set(0, 3);
        let edited_snapshot = current.clone();
        apply_lighting(&mut current, lights);
        let Light::Full(data) = &current.sections[0].light else {
            panic!("torch light varies");
        };
        assert_eq!(
            result_storage,
            data.as_ptr(),
            "returned light storage moves without cloning"
        );
        current.lit = true;
        assert_eq!(current.get(8, MIN_Y, 8), STONE);
        assert_eq!(current.sections[0].biomes.get(0), 3);
        assert_eq!(snapshot.get(8, MIN_Y, 8), TORCH);
        assert_eq!(edited_snapshot.sections[0].light(index(8, MIN_Y, 8)), 0xF0);
        let mut world = HashMap::from([((0, 0), Arc::new(current))]);
        // Streaming replays edits made after the job snapshot with force=true.
        relight(&mut world, &tables(), true, &[(8, MIN_Y, 8)], true);
        let region = [[None; 3], [None, Some(&*world[&(0, 0)]), None], [None; 3]];
        let fresh = light_chunk(&region, &tables(), true);
        for (section, light) in world[&(0, 0)].sections.iter().zip(fresh) {
            for i in 0..4096 {
                assert_eq!(section.light(i), light.get(i));
            }
        }
    }

    #[test]
    fn empty_chunk_missing_neighbours_and_no_sky() {
        let mut chunk = Chunk::empty(0, 0);
        for sky in [false, true] {
            let region = [[None; 3], [None, Some(&chunk), None], [None; 3]];
            let lights = light_chunk(&region, &tables(), sky);
            assert_eq!(lights.len(), SECTIONS);
            assert!(
                lights
                    .iter()
                    .all(|v| matches!(v, Light::Uniform(n) if *n == if sky { 0xF0 } else { 0 }))
            );
            let section = Arc::as_ptr(&chunk.sections[0]);
            apply_lighting(&mut chunk, lights);
            assert_eq!(section, Arc::as_ptr(&chunk.sections[0]));
        }
    }

    // Original main's worker packaging and streaming application. Kept here
    // so the benchmark compares the real former copy/compaction work.
    fn baseline_pack(
        center: &Chunk,
        h: usize,
        sky: &[u8],
        block: &[u8],
    ) -> Vec<Arc<crate::chunk::Section>> {
        let at = |x: usize, y: usize, z: usize| (y * W + z) * W + x;
        let mut sections = Vec::with_capacity(SECTIONS);
        for (si, section) in center.sections.iter().enumerate() {
            let mut out = (**section).clone();
            if si * 16 >= h {
                out.light = Light::Uniform(0xF0);
            } else {
                let mut data = Box::new([0u8; 4096]);
                for ly in 0..16 {
                    let y = si * 16 + ly;
                    if y >= h {
                        for i in 0..256 {
                            data[ly << 8 | i] = 0xF0;
                        }
                        continue;
                    }
                    for lz in 0..16 {
                        for lx in 0..16 {
                            let p = at(16 + lx, y, 16 + lz);
                            data[index(lx as i32, ly as i32, lz as i32)] = sky[p] << 4 | block[p];
                        }
                    }
                }
                out.light = Light::Full(data);
            }
            out.compact();
            sections.push(Arc::new(out));
        }
        sections
    }
    fn baseline_apply(chunk: &mut Chunk, sections: Vec<Arc<crate::chunk::Section>>) {
        for (target, lit) in chunk.sections.iter_mut().zip(sections) {
            let mut section = (**target).clone();
            section.light = lit.light.clone();
            *target = Arc::new(section);
        }
    }

    #[test]
    #[ignore = "microbenchmark; run with --ignored --nocapture"]
    fn benchmark_worker_light_results() {
        use std::{hint::black_box, time::Instant};
        let mut fixture = Chunk::empty(0, 0);
        for (si, s) in fixture.sections.iter_mut().enumerate() {
            let section = Arc::make_mut(s);
            for i in 0..4096 {
                section.set(i, ((i + si) % 16) as u16);
            }
            for i in 0..64 {
                section.biomes.set(i, (i % 4) as u32);
            }
            section.light = Light::Full(Box::new([0x87; 4096]));
        }
        let h = HEIGHT as usize;
        let sky: Vec<_> = (0..W * W * h).map(|i| (i % 16) as u8).collect();
        let block: Vec<_> = (0..W * W * h).map(|i| ((i / 16) % 16) as u8).collect();
        let mut old = fixture.clone();
        let mut new = fixture.clone();
        baseline_apply(&mut old, baseline_pack(&fixture, h, &sky, &block));
        apply_lighting(&mut new, pack_lighting(h, &sky, &block, true));
        for (a, b) in old.sections.iter().zip(&new.sections) {
            for i in 0..4096 {
                assert_eq!(a.light(i), b.light(i));
                assert_eq!(a.get(i), b.get(i));
            }
            for i in 0..64 {
                assert_eq!(a.biomes.get(i), b.biomes.get(i));
            }
        }
        for shared in [false, true] {
            let mut times = [Vec::new(), Vec::new()];
            for round in 0..11 {
                for optimized in [round % 2 == 0, round % 2 != 0] {
                    let mut current = fixture.clone();
                    let held = shared.then(|| current.clone());
                    // Warm up and detach for the uniquely-owned scenario.
                    apply_lighting(&mut current, pack_lighting(h, &sky, &block, true));
                    let started = Instant::now();
                    for _ in 0..500 {
                        let retained = shared.then(|| current.clone());
                        if optimized {
                            apply_lighting(
                                black_box(&mut current),
                                pack_lighting(h, black_box(&sky), black_box(&block), true),
                            );
                        } else {
                            baseline_apply(
                                black_box(&mut current),
                                baseline_pack(
                                    black_box(&fixture),
                                    h,
                                    black_box(&sky),
                                    black_box(&block),
                                ),
                            );
                        }
                        black_box(retained);
                        black_box(&current);
                    }
                    if round > 0 {
                        times[usize::from(optimized)]
                            .push(started.elapsed().as_secs_f64() * 1e6 / 500.0);
                    }
                    black_box(held);
                }
            }
            for values in &mut times {
                values.sort_by(f64::total_cmp);
            }
            println!(
                "pack+apply shared={shared}: baseline {:.3} us optimized {:.3} us speedup {:.2}x",
                times[0][5],
                times[1][5],
                times[0][5] / times[1][5]
            );
        }
        let payload: usize = fixture.sections.iter().map(|s| s.heap_bytes()).sum();
        println!(
            "fixture heap payload {payload} bytes; old worker clones it plus 24 section Arcs; old application clones current payload and 98304 returned light bytes. New worker clones zero section payload; new application clones current payload only when shared."
        );
    }
}
