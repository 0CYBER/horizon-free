//! Bounded surface terrain for distances beyond the resident chunk area.
//! Far tiles never enter World, lighting, simulation or the section cave graph.
use crate::{
    blocks::{BlockFacts, Fluid},
    chunk::{Chunk, HEIGHT, MIN_Y},
    fast_hash::{FxHashMap, FxHashSet},
    mesher::{FaceGroups, SectionMesh, TerrainVertex},
    streaming::{LodSource, WorkerContext},
};
use horizon_core::position::BlockPos;
use horizon_world::registry::BlockStateId;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

const CELLS: usize = 16;
const MAX_SAVED: usize = 8192;
/// Top solid face and optional fluid face, in absolute block coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Surface {
    pub height: i32,
    pub state: BlockStateId,
    pub water: Option<(i32, BlockStateId)>,
    pub biome: u16,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileKey {
    pub x: i32,
    pub z: i32,
    pub level: u8,
}
impl TileKey {
    pub fn size(self) -> i32 {
        64 << self.level
    }
    pub fn origin(self) -> [i32; 3] {
        [self.x * self.size(), 0, self.z * self.size()]
    }
    fn children(self) -> [Self; 4] {
        std::array::from_fn(|i| Self {
            x: self.x * 2 + (i & 1) as i32,
            z: self.z * 2 + (i >> 1) as i32,
            level: self.level - 1,
        })
    }
    fn distance(self, camera: [i32; 2]) -> i32 {
        let [x, _, z] = self.origin();
        (x - camera[0])
            .max(camera[0] - x - self.size())
            .max(z - camera[1])
            .max(camera[1] - z - self.size())
            .max(0)
    }
}
#[derive(Default)]
struct Plan {
    roots: Vec<TileKey>,
    nodes: FxHashSet<TileKey>,
    split: FxHashSet<TileKey>,
}
impl Plan {
    fn new(camera: [i32; 2], detail: i32, distance: u32) -> Self {
        let range = distance.min(1024) as i32 * 16;
        let mut level = 0;
        while (64 << level) < range {
            level += 1;
        }
        let size = 64 << level;
        let mut result = Self::default();
        for z in (camera[1] - range).div_euclid(size)..=(camera[1] + range).div_euclid(size) {
            for x in (camera[0] - range).div_euclid(size)..=(camera[0] + range).div_euclid(size) {
                let key = TileKey { x, z, level };
                result.roots.push(key);
                result.visit(key, camera, detail, range);
            }
        }
        result
    }
    fn visit(&mut self, key: TileKey, camera: [i32; 2], detail: i32, range: i32) {
        if key.distance(camera) > range {
            return;
        }
        self.nodes.insert(key);
        // Inside the near area, stride4 divides chunk boundaries exactly.
        if key.level > 0 && key.distance(camera) < (key.size() * 2).max(detail * 16 + 64) {
            self.split.insert(key);
            for child in key.children() {
                self.visit(child, camera, detail, range);
            }
        }
    }
    fn select(
        &self,
        key: TileKey,
        ready: &FxHashMap<TileKey, Vec<Option<Surface>>>,
        out: &mut Vec<TileKey>,
    ) -> bool {
        if !self.nodes.contains(&key) {
            return true;
        }
        if self.split.contains(&key) {
            let start = out.len();
            if key
                .children()
                .into_iter()
                .all(|child| self.select(child, ready, out))
            {
                return true;
            }
            if ready.contains_key(&key) {
                out.truncate(start);
                out.push(key);
                return true;
            }
            // Keep any children already available while the parent is loading.
            out.truncate(start);
            for child in key.children() {
                self.select(child, ready, out);
            }
            false
        } else if ready.contains_key(&key) {
            out.push(key);
            true
        } else {
            false
        }
    }
}
struct Job {
    key: TileKey,
}
struct Done {
    key: TileKey,
    samples: Vec<Option<Surface>>,
}
struct Shared {
    jobs: Mutex<VecDeque<Job>>,
    wake: Condvar,
    stop: AtomicBool,
}
#[derive(Clone, Copy)]
struct Style {
    layer: u16,
    tint: i32,
    translucent: bool,
    light: u16,
}
/// One background worker and at most two outstanding sample jobs. Only the
/// current view's quadtree and a bounded set of visited summaries are retained.
pub struct Lod {
    shared: Arc<Shared>,
    thread: Option<thread::JoinHandle<()>>,
    results: mpsc::Receiver<Done>,
    context: Arc<WorkerContext>,
    area: Option<([i32; 2], i32, u32)>,
    plan: Plan,
    pending: FxHashSet<TileKey>,
    ready: FxHashMap<TileKey, Vec<Option<Surface>>>,
    active: FxHashSet<TileKey>,
    dirty: FxHashSet<TileKey>,
    removed: Vec<TileKey>,
    covered: FxHashSet<(i32, i32)>,
    covered_bounds: Option<[i32; 4]>,
    saved: FxHashMap<(i32, i32), [Option<Surface>; 16]>,
    saved_order: VecDeque<(i32, i32)>,
    styles: FxHashMap<BlockStateId, Style>,
}
impl Lod {
    pub fn new(source: Arc<dyn LodSource>, context: Arc<WorkerContext>) -> Self {
        let shared = Arc::new(Shared {
            jobs: Mutex::new(VecDeque::new()),
            wake: Condvar::new(),
            stop: AtomicBool::new(false),
        });
        let worker = shared.clone();
        let (tx, results) = mpsc::channel();
        let handle = thread::Builder::new()
            .name("Far terrain".into())
            .spawn(move || {
                let mut sampler = source.sampler();
                loop {
                    let job = {
                        let mut jobs = worker.jobs.lock().unwrap();
                        while jobs.is_empty() && !worker.stop.load(Ordering::Relaxed) {
                            jobs = worker.wake.wait(jobs).unwrap();
                        }
                        if worker.stop.load(Ordering::Relaxed) {
                            break;
                        }
                        jobs.pop_front().unwrap()
                    };
                    let [x, _, z] = job.key.origin();
                    let stride = job.key.size() / CELLS as i32;
                    let mut samples = Vec::with_capacity(CELLS * CELLS);
                    for dz in 0..CELLS {
                        if worker.stop.load(Ordering::Relaxed) {
                            break;
                        }
                        for dx in 0..CELLS {
                            samples.push(
                                sampler.sample(x + dx as i32 * stride, z + dz as i32 * stride),
                            );
                        }
                    }
                    if tx
                        .send(Done {
                            key: job.key,
                            samples,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .expect("far terrain worker");
        Self {
            shared,
            thread: Some(handle),
            results,
            context,
            area: None,
            plan: Plan::default(),
            pending: FxHashSet::default(),
            ready: FxHashMap::default(),
            active: FxHashSet::default(),
            dirty: FxHashSet::default(),
            removed: Vec::new(),
            covered: FxHashSet::default(),
            covered_bounds: None,
            saved: FxHashMap::default(),
            saved_order: VecDeque::new(),
            styles: FxHashMap::default(),
        }
    }
    pub fn stats(&self) -> (usize, usize) {
        (self.active.len(), self.ready.len())
    }
    pub fn removed(&mut self) -> Vec<TileKey> {
        std::mem::take(&mut self.removed)
    }
    pub fn invalidate_chunk(&mut self, x: i32, z: i32) {
        for &key in &self.active {
            let [ox, _, oz] = key.origin();
            if x * 16 < ox + key.size()
                && (x + 1) * 16 > ox
                && z * 16 < oz + key.size()
                && (z + 1) * 16 > oz
            {
                self.dirty.insert(key);
            }
        }
    }
    /// Sample only sixteen columns, from current block storage (heightmaps can
    /// be stale after edits). Memory stays bounded while exploring indefinitely.
    pub fn observe(&mut self, chunk: &Chunk, facts: &BlockFacts) {
        let values = std::array::from_fn(|i| {
            let x = (i & 3) as i32 * 4;
            let z = (i >> 2) as i32 * 4;
            let mut water = None;
            for y in (MIN_Y..MIN_Y + HEIGHT).rev() {
                let state = BlockStateId(chunk.get(x, y, z));
                if facts.is_air(state) {
                    continue;
                }
                let f = facts.get(state);
                if !matches!(f.fluid, Fluid::None) {
                    if water.is_none() {
                        water = Some((y + 1, state));
                    }
                } else if f.solid_render || f.name.ends_with("_leaves") {
                    return Some(Surface {
                        height: y + 1,
                        state,
                        water,
                        biome: chunk.biome(x, y, z),
                    });
                }
            }
            water.map(|(y, state)| Surface {
                height: MIN_Y,
                state: facts.air,
                water: Some((y, state)),
                biome: chunk.biome(x, y - 1, z),
            })
        });
        let key = (chunk.x, chunk.z);
        if self.saved.get(&key) == Some(&values) {
            return;
        }
        if !self.saved.contains_key(&key) {
            while self.saved.len() >= MAX_SAVED {
                if let Some(old) = self.saved_order.pop_front() {
                    self.saved.remove(&old);
                }
            }
            self.saved_order.push_back(key);
        }
        self.saved.insert(key, values);
        self.invalidate_chunk(chunk.x, chunk.z);
    }
    pub fn update(
        &mut self,
        camera: [f64; 3],
        detail: i32,
        distance: u32,
        covered: FxHashSet<(i32, i32)>,
    ) -> Vec<(TileKey, SectionMesh)> {
        let camera = [camera[0].floor() as i32, camera[2].floor() as i32];
        // Replan at chunk boundaries, not on every camera movement.
        let area = (
            [camera[0].div_euclid(16), camera[1].div_euclid(16)],
            detail,
            distance,
        );
        if self.area != Some(area) {
            self.area = Some(area);
            self.plan = Plan::new(camera, detail, distance);
            self.ready.retain(|key, _| self.plan.nodes.contains(key));
        }
        while let Ok(done) = self.results.try_recv() {
            self.pending.remove(&done.key);
            if self.plan.nodes.contains(&done.key) && done.samples.len() == CELLS * CELLS {
                self.ready.insert(done.key, done.samples);
            }
        }
        if self.covered != covered {
            let changes: Vec<_> = self
                .covered
                .symmetric_difference(&covered)
                .copied()
                .collect();
            for (x, z) in changes {
                self.invalidate_chunk(x, z);
            }
            self.covered_bounds = covered.iter().fold(None, |bounds, &(x, z)| {
                Some(match bounds {
                    None => [x * 16, z * 16, (x + 1) * 16, (z + 1) * 16],
                    Some([x0, z0, x1, z1]) => [
                        x0.min(x * 16),
                        z0.min(z * 16),
                        x1.max((x + 1) * 16),
                        z1.max((z + 1) * 16),
                    ],
                })
            });
            self.covered = covered;
        }
        let mut selected = Vec::new();
        for &key in &self.plan.roots {
            self.plan.select(key, &self.ready, &mut selected);
        }
        let next: FxHashSet<_> = selected.iter().copied().collect();
        self.removed.extend(self.active.difference(&next).copied());
        self.dirty.extend(next.difference(&self.active).copied());
        self.active = next;
        self.dirty.retain(|key| self.active.contains(key));
        let keys: Vec<_> = self.dirty.drain().collect();
        let meshes = keys.into_iter().map(|key| (key, self.mesh(key))).collect();
        let mut wanted: Vec<_> = self
            .plan
            .nodes
            .iter()
            .copied()
            .filter(|key| !self.ready.contains_key(key) && !self.pending.contains(key))
            .collect();
        // Coarse ancestors first: cover the horizon quickly, then refine.
        wanted
            .sort_unstable_by_key(|key| (std::cmp::Reverse(key.level), key.distance(camera), *key));
        let mut jobs = self.shared.jobs.lock().unwrap();
        for key in wanted
            .into_iter()
            .take(2usize.saturating_sub(self.pending.len()))
        {
            self.pending.insert(key);
            jobs.push_back(Job { key });
        }
        self.shared.wake.notify_one();
        meshes
    }
    fn style(&mut self, state: BlockStateId) -> Style {
        if let Some(style) = self.styles.get(&state) {
            return *style;
        }
        let registry = self.context.assets.registry;
        let name = registry.state(state).map(|s| s.name.as_str()).unwrap_or("");
        let fluid = match name {
            "minecraft:water" | "minecraft:bubble_column" => Some(("block/water_still", true, -2)),
            "minecraft:lava" => Some(("block/lava_still", false, -1)),
            _ => None,
        };
        let light =
            240 | (u16::from(self.context.tables.facts(state).map_or(0, |f| f.emission)) << 12);
        let style = if let Some((sprite, translucent, tint)) = fluid {
            Style {
                layer: self
                    .context
                    .assets
                    .sprite_layers
                    .get(sprite)
                    .or_else(|| {
                        self.context
                            .assets
                            .sprite_layers
                            .get(&format!("minecraft:{sprite}"))
                    })
                    .copied()
                    .unwrap_or(0) as u16,
                tint,
                translucent,
                light,
            }
        } else {
            let models = self
                .context
                .assets
                .model_catalog
                .collect(state, BlockPos::new(0, 0, 0))
                .ok();
            let face = models.as_ref().and_then(|m| {
                m.iter()
                    .flat_map(|(_, m)| &m.faces)
                    .find(|f| f.quad.facing == horizon_assets::model::Direction::Up)
            });
            Style {
                layer: face.map_or(0, |f| f.layer as u16),
                tint: if face.is_some_and(|f| f.quad.tint_index >= 0) {
                    if name.contains("leaves") { -3 } else { 0 }
                } else {
                    -1
                },
                translucent: face.is_some_and(|f| f.translucent),
                light,
            }
        };
        self.styles.insert(state, style);
        style
    }
    fn mesh(&mut self, key: TileKey) -> SectionMesh {
        let mut mesh = SectionMesh::default();
        let stride = key.size() / CELLS as i32;
        let [ox, _, oz] = key.origin();
        let mut samples = self.ready[&key].clone();
        for z in 0..CELLS {
            for x in 0..CELLS {
                let (wx, wz) = (ox + x as i32 * stride, oz + z as i32 * stride);
                if let Some(saved) = self.saved.get(&(wx.div_euclid(16), wz.div_euclid(16))) {
                    samples[z * CELLS + x] =
                        saved[((wz.rem_euclid(16) / 4) * 4 + wx.rem_euclid(16) / 4) as usize];
                }
            }
        }
        for z in 0..CELLS {
            for x in 0..CELLS {
                let (wx, wz) = (ox + x as i32 * stride, oz + z as i32 * stride);
                let Some(sample) = samples[z * CELLS + x] else {
                    continue;
                };
                // Only fine cells overlap near chunks; coarse parent fallbacks
                // also clip individual chunk-sized pieces while children load.
                let overlaps = self.covered_bounds.is_some_and(|[x0, z0, x1, z1]| {
                    wx < x1 && wx + stride > x0 && wz < z1 && wz + stride > z0
                });
                let step = if overlaps { stride.min(16) } else { stride };
                for dz in (0..stride).step_by(step as usize) {
                    for dx in (0..stride).step_by(step as usize) {
                        if self
                            .covered
                            .contains(&((wx + dx).div_euclid(16), (wz + dz).div_euclid(16)))
                        {
                            continue;
                        }
                        let (x0, z0, x1, z1) = (
                            (x as i32 * stride + dx) as f32,
                            (z as i32 * stride + dz) as f32,
                            (x as i32 * stride + dx + step) as f32,
                            (z as i32 * stride + dz + step) as f32,
                        );
                        let y = sample.height as f32;
                        if y > MIN_Y as f32 {
                            let style = self.style(sample.state);
                            let color = self.color(style, sample.biome);
                            quad(
                                &mut mesh,
                                style,
                                color,
                                [[x0, y, z1], [x1, y, z1], [x1, y, z0], [x0, y, z0]],
                                1.0,
                            );
                            for (nx, nz, cell_edge, neighbor, points, shade) in [
                                (
                                    x as i32 - 1,
                                    z as i32,
                                    dx == 0,
                                    [wx + dx - 1, wz + dz],
                                    [[x0, y, z0], [x0, y, z1], [x0, 0.0, z1], [x0, 0.0, z0]],
                                    0.6,
                                ),
                                (
                                    x as i32 + 1,
                                    z as i32,
                                    dx + step == stride,
                                    [wx + dx + step, wz + dz],
                                    [[x1, y, z1], [x1, y, z0], [x1, 0.0, z0], [x1, 0.0, z1]],
                                    0.6,
                                ),
                                (
                                    x as i32,
                                    z as i32 - 1,
                                    dz == 0,
                                    [wx + dx, wz + dz - 1],
                                    [[x1, y, z0], [x0, y, z0], [x0, 0.0, z0], [x1, 0.0, z0]],
                                    0.8,
                                ),
                                (
                                    x as i32,
                                    z as i32 + 1,
                                    dz + step == stride,
                                    [wx + dx, wz + dz + step],
                                    [[x0, y, z1], [x1, y, z1], [x1, 0.0, z1], [x0, 0.0, z1]],
                                    0.8,
                                ),
                            ] {
                                let cut_edge = self.covered.contains(&(
                                    neighbor[0].div_euclid(16),
                                    neighbor[1].div_euclid(16),
                                ));
                                let floor = if cut_edge {
                                    // Near terrain may be below the coarse top;
                                    // close the cut instead of exposing its underside.
                                    MIN_Y
                                } else if !cell_edge {
                                    continue;
                                } else if (0..CELLS as i32).contains(&nx)
                                    && (0..CELLS as i32).contains(&nz)
                                {
                                    samples[(nz as usize) * CELLS + nx as usize]
                                        .map_or(MIN_Y, |s| s.height)
                                } else {
                                    MIN_Y
                                };
                                if floor < sample.height {
                                    let mut p = points;
                                    p[2][1] = floor as f32;
                                    p[3][1] = floor as f32;
                                    p.reverse();
                                    quad(&mut mesh, style, color, p, shade);
                                }
                            }
                        }
                        if let Some((height, state)) = sample.water {
                            let style = self.style(state);
                            let color = self.color(style, sample.biome);
                            let y = height as f32 - 1.0 / 9.0;
                            quad(
                                &mut mesh,
                                style,
                                color,
                                [[x0, y, z1], [x1, y, z1], [x1, y, z0], [x0, y, z0]],
                                1.0,
                            );
                        }
                    }
                }
            }
        }
        mesh.faces = FaceGroups::group(&mut mesh.solid);
        mesh
    }
    fn color(&self, style: Style, biome: u16) -> u32 {
        let b = self.context.biomes.get(biome);
        match style.tint {
            -2 => b.water,
            -3 => b.foliage,
            0 => b.grass,
            _ => 0xffffff,
        }
    }
}
fn quad(mesh: &mut SectionMesh, style: Style, color: u32, points: [[f32; 3]; 4], shade: f32) {
    let channel = |shift: u32| (((color >> shift) & 255u32) as f32 * shade) as u32;
    let color = channel(16) | (channel(8) << 8) | (channel(0) << 16) | 0xff000000;
    let out = if style.translucent {
        &mut mesh.translucent
    } else {
        &mut mesh.solid
    };
    out.extend(
        points
            .into_iter()
            .zip([[0., 1.], [1., 1.], [1., 0.], [0., 0.]])
            .map(|(pos, uv)| TerrainVertex {
                pos,
                uv,
                layer: style.layer,
                light: style.light,
                color,
            }),
    );
}
impl Drop for Lod {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.wake.notify_one();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn far_plan_is_bounded_and_near_tiles_align_with_chunks() {
        for camera in [[0, 0], [-1, -17], [1023, -4097]] {
            for distance in [64, 256, 1024] {
                let plan = Plan::new(camera, 12, distance);
                assert!(plan.nodes.len() < 2500, "{} nodes", plan.nodes.len());
                for key in &plan.nodes {
                    if !plan.split.contains(key) && key.distance(camera) < 12 * 16 {
                        assert_eq!(key.level, 0);
                    }
                }
                let cells = plan
                    .nodes
                    .iter()
                    .filter(|k| !plan.split.contains(k))
                    .count();
                assert!(cells < 2000);
            }
        }
        assert_eq!(
            TileKey {
                x: -1,
                z: -2,
                level: 1
            }
            .origin(),
            [-128, 0, -256]
        );
    }
    #[test]
    fn parent_covers_missing_children_then_retires_without_overlap() {
        let parent = TileKey {
            x: -1,
            z: 0,
            level: 1,
        };
        let mut plan = Plan::default();
        plan.roots.push(parent);
        plan.nodes.insert(parent);
        plan.split.insert(parent);
        plan.nodes.extend(parent.children());
        let mut ready = FxHashMap::default();
        ready.insert(parent, vec![]);
        ready.insert(parent.children()[0], vec![]);
        let mut selected = Vec::new();
        assert!(plan.select(parent, &ready, &mut selected));
        assert_eq!(selected, [parent]);
        for child in parent.children() {
            ready.insert(child, vec![]);
        }
        selected.clear();
        assert!(plan.select(parent, &ready, &mut selected));
        assert_eq!(selected, parent.children());
    }
    #[test]
    fn coarse_quad_keeps_winding_light_and_transparency() {
        let mut mesh = SectionMesh::default();
        let style = Style {
            layer: 7,
            tint: -1,
            translucent: true,
            light: 240,
        };
        quad(
            &mut mesh,
            style,
            0x123456,
            [
                [0., 70., 64.],
                [64., 70., 64.],
                [64., 70., 0.],
                [0., 70., 0.],
            ],
            1.,
        );
        assert!(mesh.solid.is_empty());
        assert_eq!(mesh.translucent.len(), 4);
        assert_eq!(mesh.translucent[0].color, 0xff563412);
        let q = &mesh.translucent;
        assert!(
            (glam::Vec3::from(q[1].pos) - glam::Vec3::from(q[0].pos))
                .cross(glam::Vec3::from(q[2].pos) - glam::Vec3::from(q[0].pos))
                .y
                > 0.
        );
    }
    #[test]
    #[ignore = "requires prepared models/textures in HORIZON_TEST_RESOURCES"]
    fn real_mesh_clips_near_chunks_and_preserves_water_and_edited_surfaces() -> crate::Result<()> {
        use crate::{
            assets::Assets, biome::Biomes, light::LightTables, mesher::MeshTables,
            testgen::TestTerrain,
        };
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("HORIZON_TEST_RESOURCES"),
        );
        let assets = Arc::new(Assets::load(&root)?);
        let facts = BlockFacts::load(&root, assets.registry)?;
        let biomes = Arc::new(Biomes::load(&root)?);
        let tables = Arc::new(MeshTables::new(
            &assets,
            &facts,
            crate::models::load_visibility(&root, assets.registry)?,
        )?);
        let context = Arc::new(WorkerContext {
            light: Arc::new(LightTables::load(&root, assets.registry, &facts)?),
            tables,
            assets: assets.clone(),
            biomes: biomes.clone(),
        });
        let source = Arc::new(TestTerrain::new(assets.registry, &biomes)?);
        let mut lod = Lod::new(source, context);
        let stone = assets.registry.default_state("minecraft:stone").unwrap();
        let water = assets.registry.default_state("minecraft:water").unwrap();
        let key = TileKey {
            x: -1,
            z: -1,
            level: 3,
        };
        lod.ready.insert(
            key,
            vec![
                Some(Surface {
                    height: 70,
                    state: stone,
                    water: Some((80, water)),
                    biome: biomes.id("minecraft:plains"),
                });
                CELLS * CELLS
            ],
        );
        // A chunk covers only one quarter of this parent's 32² cell. Both
        // opaque terrain and water must leave precisely that 16² footprint.
        lod.covered.insert((-1, -2));
        lod.covered_bounds = Some([-16, -32, 0, -16]);
        let mesh = lod.mesh(key);
        let top_area = |vertices: &[TerrainVertex], y: f32| {
            vertices
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|quad| quad.iter().all(|v| (v.pos[1] - y).abs() < 0.001))
                .map(|quad| {
                    let a = glam::Vec3::from(quad[0].pos);
                    let b = glam::Vec3::from(quad[1].pos);
                    let c = glam::Vec3::from(quad[2].pos);
                    let normal = (b - a).cross(c - a);
                    assert!(normal.y > 0.0, "surface must face the camera from above");
                    let center = (a + c) * 0.5;
                    let [ox, _, oz] = key.origin();
                    let chunk = (
                        ((center.x as i32) + ox).div_euclid(16),
                        ((center.z as i32) + oz).div_euclid(16),
                    );
                    assert!(
                        !lod.covered.contains(&chunk),
                        "LOD overlaps a full-detail chunk"
                    );
                    normal.y
                })
                .sum::<f32>()
        };
        let expected = (key.size() * key.size() - 16 * 16) as f32;
        assert_eq!(top_area(&mesh.solid, 70.0), expected);
        assert_eq!(top_area(&mesh.translucent, 80.0 - 1.0 / 9.0), expected);
        assert_eq!(
            mesh.faces.quads.iter().sum::<u32>() as usize,
            mesh.solid.len() / 4
        );
        assert!(mesh.translucent.iter().all(|v| v.light == 240));
        let side = |axis: usize, plane: f32, low: f32, high: f32| {
            let other = if axis == 0 { 2 } else { 0 };
            mesh.solid
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|quad| {
                    quad.iter().all(|v| v.pos[axis] == plane)
                        && quad.iter().map(|v| v.pos[1]).fold(f32::INFINITY, f32::min)
                            == MIN_Y as f32
                        && quad
                            .iter()
                            .map(|v| v.pos[1])
                            .fold(f32::NEG_INFINITY, f32::max)
                            == 70.0
                        && quad
                            .iter()
                            .map(|v| v.pos[other])
                            .fold(f32::INFINITY, f32::min)
                            == low
                        && quad
                            .iter()
                            .map(|v| v.pos[other])
                            .fold(f32::NEG_INFINITY, f32::max)
                            == high
                })
                .map(move |quad| {
                    let a = glam::Vec3::from(quad[0].pos);
                    (glam::Vec3::from(quad[1].pos) - a).cross(glam::Vec3::from(quad[2].pos) - a)
                        [axis]
                })
                .collect::<Vec<_>>()
        };
        // The two exposed edges face the missing full-detail chunk and seal
        // the height mismatch all the way down. Shared subpieces add no walls.
        let east_cut = side(0, 496.0, 480.0, 496.0);
        assert_eq!(east_cut.len(), 1);
        assert!(east_cut[0] > 0.0);
        let north_cut = side(2, 496.0, 496.0, 512.0);
        assert_eq!(north_cut.len(), 1);
        assert!(north_cut[0] < 0.0);
        assert!(side(0, 496.0, 496.0, 512.0).is_empty());

        // Block edits do not refresh Chunk.heights; observation must scan the
        // actual blocks, including builds above the old heightmap.
        let mut chunk = Chunk::empty(-1, -1);
        chunk.set(0, 120, 0, stone.0);
        chunk.set(4, 54, 4, stone.0);
        for y in 55..64 {
            chunk.set(4, y, 4, water.0);
        }
        lod.observe(&chunk, &facts);
        let observed = &lod.saved[&(-1, -1)];
        assert_eq!(observed[0].unwrap().height, 121);
        assert_eq!(observed[0].unwrap().state, stone);
        assert_eq!(observed[5].unwrap().height, 55);
        assert_eq!(observed[5].unwrap().water, Some((64, water)));
        Ok(())
    }
}
