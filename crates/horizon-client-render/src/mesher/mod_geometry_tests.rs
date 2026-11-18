//! The section geometry design's acceptance cases (docs/design/mod-api/section-geometry.md)
//! that the mesher alone decides.
use super::mod_geometry::{
    Batch, Budget, MAX_IN_FLIGHT_BYTES, MAX_QUADS, MAX_SECTION_BYTES, QUAD_BYTES, Renderer,
};
use super::mod_tint_tests::{resources, tint_values};
use super::*;
use horizon_assets::resource_source::ResourceRoot;
use mod_api::{LightLayer, SectionGeometry, SectionGeometryContext, SectionQuad};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

const GOLD: &str = "minecraft:block/gold_block";

fn quad(translucent: bool) -> SectionQuad {
    SectionQuad {
        positions: [
            [2.0, 8.0, 2.0],
            [2.0, 8.0, 3.0],
            [3.0, 8.0, 3.0],
            [3.0, 8.0, 2.0],
        ],
        uvs: [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
        color: 0xFF20_4060,
        light: [15, 0],
        translucent,
    }
}

/// One decorative gold quad in every section, lit by the sky light the `level` reads there.
#[derive(Default)]
struct Decorative {
    calls: AtomicUsize,
}
impl SectionGeometry for Decorative {
    fn add(&self, origin: [i32; 3], quads: &mut dyn SectionGeometryContext) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let mut quad = quad(false);
        let above = [origin[0] + 2, origin[1] + 8, origin[2] + 2];
        quad.light = [quads.level().brightness(LightLayer::Sky, above) as u8, 3];
        assert!(quads.add(GOLD, quad));
    }
}

/// A good quad before a bad one, or too many quads: refused whole.
enum Bad {
    Corner(f32),
    Sprite,
    Many,
}
impl SectionGeometry for Bad {
    fn add(&self, _: [i32; 3], quads: &mut dyn SectionGeometryContext) {
        let good = quad(false);
        assert!(quads.add(GOLD, good));
        let mut bad = good;
        let accepted = match self {
            Bad::Corner(value) => {
                bad.positions[1][0] = *value;
                quads.add(GOLD, bad)
            }
            Bad::Sprite => quads.add("missing:block/nothing", bad),
            Bad::Many => (1..=MAX_QUADS).all(|_| quads.add(GOLD, good)),
        };
        assert!(!accepted);
        assert!(!quads.add(GOLD, good), "a refused batch stays refused");
        assert_eq!(quads.remaining(), 0);
    }
}

struct Panics;
impl SectionGeometry for Panics {
    fn add(&self, _: [i32; 3], quads: &mut dyn SectionGeometryContext) {
        quads.add(GOLD, quad(false));
        panic!("a broken renderer");
    }
}

struct Translucent;
impl SectionGeometry for Translucent {
    fn add(&self, _: [i32; 3], quads: &mut dyn SectionGeometryContext) {
        assert!(quads.add("minecraft:block/glass", quad(true)));
    }
}

fn sprites() -> BTreeMap<String, u32> {
    BTreeMap::from([(GOLD.to_owned(), 7)])
}

fn full_budget() -> Budget {
    Budget {
        quads: MAX_QUADS,
        bytes: MAX_SECTION_BYTES,
    }
}

#[test]
fn a_quad_past_the_section_budget_is_refused_before_it_is_kept() {
    let sprites = sprites();
    let shared = AtomicUsize::new(0);
    let mut budget = Budget {
        quads: 3,
        bytes: MAX_SECTION_BYTES,
    };
    let mut batch = Batch::new(&sprites, &mut budget, &shared);
    assert_eq!(batch.remaining(), 3);
    assert!((0..3).all(|_| batch.add(GOLD, quad(false))));
    assert_eq!(batch.staged.len(), 3);
    assert_eq!(batch.remaining(), 0);
    assert!(
        !batch.add(GOLD, quad(false)),
        "the fourth is past the budget"
    );
    assert_eq!(batch.staged.capacity(), 0, "the refused quads are freed");
    assert!(batch.refused.is_some());
    assert_eq!(batch.in_flight, 0);
    assert_eq!(
        shared.load(Ordering::Acquire),
        0,
        "in-flight bytes given back"
    );
    drop(batch);
    assert_eq!(budget.quads, 3, "a refused batch gives its budget back");
}

#[test]
fn the_section_budget_is_shared_by_its_blocks_and_every_renderer() {
    let sprites = sprites();
    let shared = AtomicUsize::new(0);
    // A section whose own blocks already hold all but 5 quads.
    let vertex = TerrainVertex::new([0.0; 3], [0.0; 2], 0, 0, 0);
    let out = SectionMesh {
        solid: vec![vertex; (MAX_QUADS - 5) * 4],
        ..SectionMesh::default()
    };
    let mut budget = Budget::left_after(&out);
    assert_eq!(budget.quads, 5);
    let mut first = Batch::new(&sprites, &mut budget, &shared);
    assert!((0..3).all(|_| first.add(GOLD, quad(false))));
    drop(first);
    assert_eq!(shared.load(Ordering::Acquire), 0);
    // The second renderer gets what the first left, 2, and its third quad refuses it whole.
    let mut second = Batch::new(&sprites, &mut budget, &shared);
    assert_eq!(second.remaining(), 2);
    assert!(second.add(GOLD, quad(false)) && second.add(GOLD, quad(false)));
    assert!(!second.add(GOLD, quad(false)));
    drop(second);
    assert_eq!(budget.quads, 2, "the first renderer's 3 stay counted");
    // Bytes bind too: a section near its 32 MiB has room for whole quads only.
    let mut bytes = Budget {
        quads: MAX_QUADS,
        bytes: 2 * QUAD_BYTES + QUAD_BYTES / 2,
    };
    let mut batch = Batch::new(&sprites, &mut bytes, &shared);
    assert_eq!(batch.remaining(), 2);
    assert!(batch.add(GOLD, quad(false)) && batch.add(GOLD, quad(false)));
    assert!(!batch.add(GOLD, quad(false)));
}

#[test]
fn the_meshing_workers_share_one_in_flight_budget() {
    let sprites = sprites();
    // Other sections' batches hold all but two quads' worth.
    let shared = AtomicUsize::new(MAX_IN_FLIGHT_BYTES - 2 * QUAD_BYTES);
    let mut budget = full_budget();
    let mut batch = Batch::new(&sprites, &mut budget, &shared);
    assert!(batch.add(GOLD, quad(false)) && batch.add(GOLD, quad(false)));
    assert_eq!(shared.load(Ordering::Acquire), MAX_IN_FLIGHT_BYTES);
    assert!(!batch.add(GOLD, quad(false)), "a third passes 128 MiB");
    assert!(batch.staged.is_empty());
    let others = MAX_IN_FLIGHT_BYTES - 2 * QUAD_BYTES;
    assert_eq!(shared.load(Ordering::Acquire), others);
    drop(batch);
    assert_eq!(budget, full_budget());
    // An accepted batch holds its bytes until it drops, after its quads are in the mesh.
    let mut budget = full_budget();
    let mut batch = Batch::new(&sprites, &mut budget, &shared);
    assert!(batch.add(GOLD, quad(false)));
    assert_eq!(shared.load(Ordering::Acquire), others + QUAD_BYTES);
    drop(batch);
    assert_eq!(shared.load(Ordering::Acquire), others);
}

#[test]
fn an_invalid_quad_refuses_the_batch_and_its_earlier_quads() {
    let sprites = sprites();
    let shared = AtomicUsize::new(0);
    let mut budget = full_budget();
    let mut batch = Batch::new(&sprites, &mut budget, &shared);
    assert!(batch.add(GOLD, quad(false)));
    let mut bad = quad(false);
    bad.uvs[2][1] = f32::NAN;
    assert!(!batch.add(GOLD, bad));
    assert!(batch.staged.is_empty());
    assert_eq!(shared.load(Ordering::Acquire), 0);
    assert!(!batch.add(GOLD, quad(false)));
    drop(batch);
    assert_eq!(budget, full_budget());
}

struct World {
    assets: Assets,
    render: RenderAssets,
    facts: BlockFacts,
    biomes: Biomes,
    root: std::path::PathBuf,
}

impl World {
    fn load() -> Option<Self> {
        let root = resources()?;
        let assets = Assets::load(&root).unwrap();
        let facts = BlockFacts::load(&root, assets.registry).unwrap();
        let biomes = Biomes::load(&ResourceRoot::directory(root.clone())).unwrap();
        let render = assets
            .reload(&ResourceRoot::directory(root.clone()))
            .unwrap();
        Some(Self {
            assets,
            render,
            facts,
            biomes,
            root,
        })
    }

    fn visibility(&self) -> FaceVisibility {
        crate::models::load_visibility(&self.root, self.assets.registry).unwrap()
    }

    /// Mesh tables whose section geometry is `renderers`.
    fn tables(&self, renderers: Vec<Arc<dyn SectionGeometry>>) -> MeshTables {
        let mut tables = MeshTables::new(
            &self.assets,
            &self.render,
            &self.facts,
            self.visibility(),
            None,
        )
        .unwrap();
        tables.section_geometry = renderers.into_iter().map(Renderer::new).collect();
        tables.mod_tints.values = Some(tint_values(self.assets.registry));
        tables
    }

    fn mesher<'a>(&'a self, tables: &'a MeshTables) -> Mesher<'a> {
        Mesher {
            nether_shading: false,
            shader_pack_lighting: false,
            separate_ao: false,
            shader_pack: false,
            looks: false,
            assets: &self.assets,
            render: &self.render,
            tables,
            biomes: &self.biomes,
            visibility: self.visibility(),
            zoom: BiomeZoom::new(0),
            blend_radius: 0,
            smooth_lighting: false,
            cutout_leaves: true,
            opaque_ice: false,
            translucent_layers: &[],
            leaf_culling: LeafCullingMode::Off,
            hidden_states: &[],
            part_tables: Default::default(),
        }
    }

    /// A chunk with one stone block in section 0 and section 1 empty.
    fn cube(&self) -> Chunk {
        let stone = self
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        let mut chunk = Chunk::empty(0, 0);
        chunk.set(2, MIN_Y + 4, 2, stone.0);
        chunk
    }
}

fn quads(vertices: &[TerrainVertex]) -> usize {
    vertices.len() / 4
}

#[test]
fn a_decorative_quad_joins_its_section_once_per_compilation() {
    let Some(world) = World::load() else {
        return;
    };
    let decorative = Arc::new(Decorative::default());
    let tables = world.tables(vec![decorative.clone()]);
    let mut mesher = world.mesher(&tables);
    let chunk = world.cube();
    let mut region = [[None; 3]; 3];
    region[1][1] = Some(&chunk);
    // Isolated cube plus one quad: 7 quads, the callback asked once.
    let cube = mesher.mesh(&region, 0);
    assert_eq!(quads(&cube.solid), 7);
    assert_eq!(decorative.calls.load(Ordering::Relaxed), 1);
    let gold = world.render.sprite_layers[GOLD] as u16;
    let added: Vec<_> = cube
        .solid
        .iter()
        .filter(|v| v.layer & !OPAQUE_LAYER == gold)
        .collect();
    assert_eq!(added.len(), 4);
    assert!(
        added.iter().all(|v| v.color == 0xFF60_4020),
        "ARGB packed as RGBA"
    );
    assert!(added.iter().all(|v| v.light == (15 * 16) | (3 * 16) << 8));
    // Empty section plus one quad: 1 quad after the build and after each of 3 remeshes into the
    // same buffers.
    let mut scratch = MeshScratch::default();
    let mut empty = mesher.mesh_reusing(&region, 1, SectionMesh::default(), &mut scratch);
    assert_eq!(quads(&empty.solid), 1);
    for _ in 0..3 {
        empty = mesher.mesh_reusing(&region, 1, empty, &mut scratch);
        assert_eq!(quads(&empty.solid), 1);
    }
    assert_eq!(decorative.calls.load(Ordering::Relaxed), 5);
}

#[test]
fn a_refused_contribution_adds_nothing_and_the_built_in_cube_keeps_six_faces() {
    let Some(world) = World::load() else {
        return;
    };
    let renderers: Vec<Arc<dyn SectionGeometry>> = vec![
        Arc::new(Panics),
        Arc::new(Bad::Corner(f32::NAN)),
        Arc::new(Bad::Corner(f32::INFINITY)),
        Arc::new(Bad::Corner(16.5)),
        Arc::new(Bad::Sprite),
        Arc::new(Bad::Many),
        Arc::new(Translucent),
    ];
    let tables = world.tables(renderers);
    let mut mesher = world.mesher(&tables);
    let chunk = world.cube();
    let mut region = [[None; 3]; 3];
    region[1][1] = Some(&chunk);
    for _ in 0..2 {
        let mesh = mesher.mesh(&region, 0);
        assert_eq!(quads(&mesh.solid), 6, "no offending quad added");
        assert_eq!(
            quads(&mesh.translucent),
            1,
            "the next renderer still adds its quad"
        );
    }
    assert!(
        tables.section_geometry[0].failed(),
        "the panicking one is disabled"
    );
    // No listeners: the mesh is the built-in one.
    let none = world.tables(Vec::new());
    let baseline = world.mesher(&none).mesh(&region, 0);
    assert_eq!(quads(&baseline.solid), 6);
    assert!(baseline.translucent.is_empty());
    assert!(world.mesher(&none).mesh(&region, 1).solid.is_empty());
}
