use super::*;
use horizon_assets::resource_source::ResourceRoot;
use horizon_world::registry::BlockRegistry;
use mod_api::{
    BiomeColorResolver, BlockAndTintGetter, BlockMetadata, BlockState as ModState, BlockTintSource,
    ColorResolver, ColorResolverRegistry, ColorResolvers, ResolverBiome, TintLayer, VanillaTint,
};
use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

pub(super) fn resources() -> Option<std::path::PathBuf> {
    let root = std::env::var_os("HORIZON_TEST_RESOURCES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../.local/horizon-client/26.3")
        });
    root.join("horizon-resources.json").exists().then_some(root)
}

// Independent API fixture for the air, stone and two leaf states exercised below.
pub(super) fn tint_values(registry: &BlockRegistry) -> ModTintValues {
    use horizon_mod_runtime::block_definitions::BlockDefinition;
    mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
    let empty = BTreeMap::new();
    let near = BTreeMap::from([("distance".into(), "1".into())]);
    let far = BTreeMap::from([("distance".into(), "7".into())]);
    mod_api::__private::resolve_block_keys(
        ["minecraft:air", "minecraft:stone", "minecraft:oak_leaves"],
        |name| {
            let result = if name == "minecraft:oak_leaves" {
                BlockDefinition::new(&[&near, &far], 1)
            } else {
                BlockDefinition::new(&[&empty], 0)
            };
            result.map_err(|error| format!("{error:?}"))
        },
    )
    .unwrap();
    let metadata = mod_api::__private::block_definitions().unwrap();
    let key = |name| mod_api::__private::block_key(name).unwrap();
    let state = |name, properties: &[mod_api::StateConstraint]| {
        ModState::of(metadata, key(name), properties).unwrap()
    };
    let air = state("minecraft:air", &[]);
    let stone = state("minecraft:stone", &[]);
    let far = state("minecraft:oak_leaves", &[]);
    let near = state(
        "minecraft:oak_leaves",
        &[mod_api::StateConstraint::new("distance", "1")],
    );
    let mut states = vec![air; registry.len()].into_boxed_slice();
    states[registry.default_state("minecraft:stone").unwrap().0 as usize] = stone;
    let oak = registry.default_state("minecraft:oak_leaves").unwrap();
    states[oak.0 as usize] = far;
    let mut engine_near = registry.state(oak).unwrap().clone();
    engine_near.properties.insert("distance".into(), "1".into());
    states[registry.id(&engine_near).unwrap().0 as usize] = near;
    ModTintValues { states, metadata }
}

/// A source whose three colours differ, so each reader shows which one it asked.
struct Three;
impl BlockTintSource for Three {
    fn color(&self, _: &ModState, _: &dyn BlockMetadata) -> u32 {
        0x11_2233
    }
    fn color_in_world(
        &self,
        _: &ModState,
        _: &dyn BlockMetadata,
        _: &dyn BlockAndTintGetter,
        [x, _, _]: [i32; 3],
    ) -> u32 {
        0x44_0000 | x as u32
    }
    fn color_as_terrain_particle(
        &self,
        _: &ModState,
        _: &dyn BlockMetadata,
        _: &dyn BlockAndTintGetter,
        _: [i32; 3],
    ) -> u32 {
        0x77_8899
    }
}

/// `BlockColors.register`: a registered block's layers replace vanilla's, a vanilla source in a
/// mod's list colours as it does on vanilla's own blocks, and each reader asks the source's method
/// vanilla asks: `color` without a level, `colorInWorld` for a block drawn in it,
/// `colorAsTerrainParticle` for its particles. A face past the list is untinted.
#[test]
fn registered_layers_replace_vanilla_and_each_reader_asks_its_method() {
    let Some(root) = resources() else {
        return;
    };
    let assets = Assets::load(&root).unwrap();
    let render = assets
        .reload(&ResourceRoot::directory(root.clone()))
        .unwrap();
    let facts = BlockFacts::load(&root, assets.registry).unwrap();
    let visibility = crate::models::load_visibility(&root, assets.registry).unwrap();
    let mut tables = MeshTables::new(&assets, &render, &facts, visibility, None).unwrap();
    let biomes = Biomes::load(&ResourceRoot::directory(root.clone())).unwrap();
    let state = |name| assets.registry.default_state(name).unwrap();
    let three: Arc<dyn BlockTintSource> = Arc::new(Three);
    let layers = BTreeMap::from([
        (
            "minecraft:oak_leaves".to_string(),
            Arc::from([TintLayer::Source(three.clone())]),
        ),
        (
            "minecraft:stone".to_string(),
            Arc::from([
                TintLayer::Vanilla(VanillaTint::Constant(0xAB_CDEF)),
                TintLayer::Vanilla(VanillaTint::ConstantSplit {
                    hand: 0x01_0101,
                    world: 0x02_0202,
                }),
                TintLayer::Source(three),
                TintLayer::Vanilla(VanillaTint::Grass),
            ]),
        ),
        ("mymod:missing".to_string(), Arc::from([])),
    ]);
    assert_eq!(tables.default_tint(state("minecraft:stone"), 0, 0), None);
    assert!(
        tables
            .apply_tint_layers(assets.registry, &layers, None)
            .is_err()
    );
    assert_eq!(tables.default_tint(state("minecraft:stone"), 0, 0), None);
    tables
        .apply_tint_layers(assets.registry, &layers, Some(tint_values(assets.registry)))
        .unwrap();
    let center = Chunk::empty(0, 0);
    let mut region = [[None; 3]; 3];
    region[1][1] = Some(&center);
    let (zoom, blend) = (BiomeZoom::new(0), 2);
    let world = |name| {
        tables.block_tints(
            state(name),
            [5, 70, 9],
            &region,
            &biomes,
            zoom,
            blend,
            false,
        )
    };
    let particle = |name| {
        let pos = [5, 70, 9];
        tables.terrain_particle_tint(state(name), pos, &region, &biomes, zoom, blend, false)
    };

    let oak = state("minecraft:oak_leaves");
    assert_eq!(tables.default_tint(oak, 0, 0), Some(0x11_2233));
    assert_eq!(
        tables.tint_source(oak, 0),
        Some(TintSource::Fixed(0x11_2233))
    );
    assert_eq!(world("minecraft:oak_leaves")(0), Some(0x44_0005));
    assert_eq!(particle("minecraft:oak_leaves"), Some(0x77_8899));
    // Vanilla's own foliage layer is gone: one layer, and index 1 is past it.
    assert_eq!(world("minecraft:oak_leaves")(1), None);

    let stone = state("minecraft:stone");
    let stone_world = world("minecraft:stone");
    assert_eq!(tables.default_tint(stone, 0, 0), Some(0xAB_CDEF));
    assert_eq!(stone_world(0), Some(0xAB_CDEF));
    assert_eq!(tables.default_tint(stone, 1, 0), Some(0x01_0101));
    assert_eq!(stone_world(1), Some(0x02_0202));
    assert_eq!(stone_world(2), Some(0x44_0005));
    // A vanilla source on a mod's list: the same grass as short grass at the same place.
    assert_eq!(stone_world(3), world("minecraft:short_grass")(0));
    assert!(stone_world(3).is_some());
    assert_eq!(stone_world(4), None);
    assert_eq!(tables.default_tint(stone, 4, 0), None);
}

/// Colours a leaf block by what is under it and by its `distance`, counting its calls.
struct ByGround {
    stone: mod_api::BlockKey,
    calls: AtomicUsize,
}
impl BlockTintSource for ByGround {
    fn color(&self, _: &ModState, _: &dyn BlockMetadata) -> u32 {
        0xFF_FFFF
    }
    fn color_in_world(
        &self,
        state: &ModState,
        metadata: &dyn BlockMetadata,
        level: &dyn BlockAndTintGetter,
        [x, y, z]: [i32; 3],
    ) -> u32 {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let on_stone = level.block_state([x, y - 1, z]).block() == self.stone;
        match (on_stone, state.property(metadata, "distance").unwrap()) {
            (true, Some("1")) => 0x40_80C0,
            (true, _) => 0x20_4060,
            (false, _) => 0x10_2030,
        }
    }
}

/// The chunk mesher asks a mod's `colorInWorld` once per block, whatever number of faces share
/// its tint index, hands it the block's state and a level it reads around the block, and draws
/// the faces with its answer.
#[test]
fn the_mesher_asks_a_mod_source_once_per_block_with_the_world_around_it() {
    let Some(root) = resources() else {
        return;
    };
    let assets = Assets::load(&root).unwrap();
    let facts = BlockFacts::load(&root, assets.registry).unwrap();
    let biomes = Biomes::load(&ResourceRoot::directory(root.clone())).unwrap();
    let render = assets
        .reload(&ResourceRoot::directory(root.clone()))
        .unwrap();
    let visibility = crate::models::load_visibility(&root, assets.registry).unwrap();
    let mut tables = MeshTables::new(&assets, &render, &facts, visibility.clone(), None).unwrap();
    let values = tint_values(assets.registry);
    let source = Arc::new(ByGround {
        stone: mod_api::__private::block_key("minecraft:stone").unwrap(),
        calls: AtomicUsize::new(0),
    });
    let layers = BTreeMap::from([(
        "minecraft:oak_leaves".to_string(),
        Arc::from([TintLayer::Source(source.clone())]),
    )]);
    tables
        .apply_tint_layers(assets.registry, &layers, Some(values))
        .unwrap();
    let registry = assets.registry;
    let oak = registry.default_state("minecraft:oak_leaves").unwrap();
    let mut near = registry.state(oak).unwrap().clone();
    near.properties.insert("distance".into(), "1".into());
    let near = registry.id(&near).unwrap();
    let stone = registry.default_state("minecraft:stone").unwrap();
    let mut chunk = Chunk::empty(0, 0);
    // On stone at distance 1, on stone at distance 7, and floating.
    chunk.set(2, MIN_Y + 4, 2, stone.0);
    chunk.set(2, MIN_Y + 5, 2, near.0);
    chunk.set(8, MIN_Y + 4, 8, stone.0);
    chunk.set(8, MIN_Y + 5, 8, oak.0);
    chunk.set(13, MIN_Y + 9, 13, oak.0);
    let mut region = [[None; 3]; 3];
    region[1][1] = Some(&chunk);
    let mut mesher = Mesher {
        nether_shading: false,
        shader_pack_lighting: false,
        separate_ao: false,
        shader_pack: false,
        assets: &assets,
        render: &render,
        tables: &tables,
        biomes: &biomes,
        visibility,
        zoom: BiomeZoom::new(0),
        blend_radius: 0,
        smooth_lighting: false,
        cutout_leaves: true,
        opaque_ice: false,
        translucent_layers: &[],
        leaf_culling: LeafCullingMode::Off,
        hidden_states: &[],
        part_tables: Default::default(),
    };
    let mesh = mesher.mesh(&region, 0);
    assert_eq!(source.calls.load(Ordering::Relaxed), 3);
    // The up faces are unshaded (1.0), so they carry the tint itself, packed as RGBA.
    let colours: Vec<u32> = mesh
        .solid
        .iter()
        .chain(&mesh.translucent)
        .map(|v| v.color)
        .collect();
    for tint in [0x40_80C0, 0x20_4060, 0x10_2030] {
        let packed = pack_rgb([tint >> 16 & 255, tint >> 8 & 255, tint & 255]);
        assert!(colours.contains(&packed), "no face tinted {tint:06X}");
    }
}

/// `getColor` as vanilla's foliage resolver answers it: the biome's own foliage colour.
struct FoliageCopy;
impl BiomeColorResolver for FoliageCopy {
    fn color(&self, _: &str, info: &ResolverBiome, _: f64, _: f64) -> u32 {
        info.colors.foliage
    }
}

/// The block column itself, x in red and z in blue, so a blend shows which columns it asked.
struct ByColumn;
impl BiomeColorResolver for ByColumn {
    fn color(&self, _: &str, _: &ResolverBiome, x: f64, z: f64) -> u32 {
        ((x as i32 & 255) << 16 | (z as i32 & 255)) as u32
    }
}

struct Throwing;
impl BiomeColorResolver for Throwing {
    fn color(&self, _: &str, _: &ResolverBiome, _: f64, _: f64) -> u32 {
        panic!("test resolver failure")
    }
}

/// The test resolvers, registered once for the whole test binary.
fn test_resolvers() -> [ColorResolver; 2] {
    static HANDLES: OnceLock<[ColorResolver; 2]> = OnceLock::new();
    *HANDLES.get_or_init(|| {
        let mut registry = ColorResolverRegistry::default();
        let handles = [
            registry.register(Arc::new(FoliageCopy)),
            registry.register(Arc::new(ByColumn)),
        ];
        registry.register(Arc::new(Throwing));
        assert!(register_color_resolvers(registry.into_resolvers()));
        handles
    })
}

fn mod_resolver(handle: ColorResolver) -> Resolver {
    let ColorResolver::Mod(handle) = handle else {
        panic!("a registered resolver is a mod's");
    };
    Resolver::Mod(mod_api::__private::mod_color_resolver_index(handle))
}

/// A chunk whose every quart cell holds `biome`.
fn filled(cx: i32, cz: i32, biome: u16) -> Chunk {
    let mut chunk = Chunk::empty(cx, cz);
    for qy in 0..(384 / 4) {
        for qz in 0..4 {
            for qx in 0..4 {
                chunk.set_biome_cell(qx, qy, qz, biome);
            }
        }
    }
    chunk
}

/// `RegisterColorHandlersEvent.ColorResolvers`: a mod's resolver blends over the biome blend
/// radius exactly as vanilla's do, over one biome and over a border between two, and is asked
/// with each column it blends.
#[test]
fn a_mod_resolver_blends_over_the_biomes_as_vanilla_resolvers_do() {
    let Some(root) = resources() else {
        return;
    };
    let biomes = Biomes::load(&ResourceRoot::directory(root)).unwrap();
    let [foliage, by_column] = test_resolvers().map(mod_resolver);
    let (forest, desert) = (biomes.id("minecraft:forest"), biomes.id("minecraft:desert"));
    let chunks: Vec<Vec<Chunk>> = (0..3)
        .map(|rz| (0..3).map(|rx| filled(rx - 1, rz - 1, forest)).collect())
        .collect();
    let mut east = filled(1, 0, forest);
    for qy in 0..(384 / 4) {
        east.set_biome_cell(0, qy, 2, desert);
    }
    let uniform: [[Option<&Chunk>; 3]; 3] =
        std::array::from_fn(|rz| std::array::from_fn(|rx| Some(&chunks[rz][rx])));
    let mut mixed = uniform;
    mixed[1][2] = Some(&east);
    for region in [&uniform, &mixed] {
        for radius in [0, 2, 5] {
            let new = || Tints::new(region, &biomes, BiomeZoom::new(42), radius, [0, 0, 64]);
            let (mut vanilla, mut modded) = (new(), new());
            for z in 0..16 {
                for x in 0..16 {
                    assert_eq!(
                        modded.get(foliage, x, 3, z),
                        vanilla.get(Resolver::Foliage, x, 3, z),
                        "r{radius} at {x} {z}"
                    );
                }
            }
        }
    }
    // Radius 1 averages columns x-1..=x+1 and z-1..=z+1: their mean is the block's own column.
    let mut tints = Tints::new(&uniform, &biomes, BiomeZoom::new(42), 1, [0, 0, 64]);
    for (x, z) in [(1, 1), (7, 3), (14, 12)] {
        let foliage_color = biomes.get(forest).foliage & 0xFF_FFFF;
        assert_eq!(tints.get(foliage, x, 0, z), foliage_color);
        assert_eq!(tints.get(by_column, x, 0, z), (x as u32) << 16 | z as u32);
        assert_eq!(tints.get(foliage, x, 0, z), foliage_color);
    }
}

#[test]
fn unknown_mod_resolver_does_not_allocate_cache_slots() {
    let Some(root) = resources() else { return };
    test_resolvers();
    let biomes = Biomes::load(&ResourceRoot::directory(root)).unwrap();
    let region = [[None; 3]; 3];
    let mut tints = Tints::new(&region, &biomes, BiomeZoom::new(42), 1, [0, 0, 64]);
    // A small invalid index kills the missing-check mutant without exhausting memory.
    for index in [32, u32::MAX] {
        assert_eq!(tints.get(Resolver::Mod(index), 1, 0, 1), 0xFF_FFFF);
        assert_eq!(
            tints
                .layers
                .iter()
                .flatten()
                .map(|l| l.mods.len())
                .sum::<usize>(),
            0
        );
    }
}

/// Colours oak leaves with a mod resolver, through the level as vanilla's sources read it.
struct Resolved(ColorResolver);
impl BlockTintSource for Resolved {
    fn color(&self, _: &ModState, _: &dyn BlockMetadata) -> u32 {
        0xFF_FFFF
    }
    fn color_in_world(
        &self,
        _: &ModState,
        _: &dyn BlockMetadata,
        level: &dyn BlockAndTintGetter,
        pos: [i32; 3],
    ) -> u32 {
        level.block_tint(pos, self.0)
    }
}

/// A block whose tint source reads a mod's resolver is drawn with the resolver's blended colour,
/// and a new mesh after the resolver's input changes (the biome's colours, as a biome colour
/// refresh or a pack reload changes them) draws the new colour, not a cached one.
#[test]
fn a_block_tinted_by_a_mod_resolver_meshes_with_its_colour_and_follows_its_inputs() {
    let Some(root) = resources() else {
        return;
    };
    let assets = Assets::load(&root).unwrap();
    let facts = BlockFacts::load(&root, assets.registry).unwrap();
    let mut biomes = Biomes::load(&ResourceRoot::directory(root.clone())).unwrap();
    let render = assets
        .reload(&ResourceRoot::directory(root.clone()))
        .unwrap();
    let visibility = crate::models::load_visibility(&root, assets.registry).unwrap();
    let mut tables = MeshTables::new(&assets, &render, &facts, visibility.clone(), None).unwrap();
    let [foliage, _] = test_resolvers();
    let layers = BTreeMap::from([(
        "minecraft:oak_leaves".to_string(),
        Arc::from([TintLayer::Source(Arc::new(Resolved(foliage)))]),
    )]);
    tables
        .apply_tint_layers(assets.registry, &layers, Some(tint_values(assets.registry)))
        .unwrap();
    let oak = assets
        .registry
        .default_state("minecraft:oak_leaves")
        .unwrap();
    let forest = biomes.id("minecraft:forest");
    let mut chunk = filled(0, 0, forest);
    chunk.set(6, MIN_Y + 5, 6, oak.0);
    let mut region = [[None; 3]; 3];
    region[1][1] = Some(&chunk);
    let colours = |biomes: &Biomes, tables: &MeshTables| {
        let mut mesher = Mesher {
            nether_shading: false,
            shader_pack_lighting: false,
            separate_ao: false,
            shader_pack: false,
            assets: &assets,
            render: &render,
            tables,
            biomes,
            visibility: visibility.clone(),
            zoom: BiomeZoom::new(0),
            blend_radius: 2,
            smooth_lighting: false,
            cutout_leaves: true,
            opaque_ice: false,
            translucent_layers: &[],
            leaf_culling: LeafCullingMode::Off,
            hidden_states: &[],
            part_tables: Default::default(),
        };
        let mesh = mesher.mesh(&region, 0);
        mesh.solid
            .iter()
            .chain(&mesh.translucent)
            .map(|v| v.color)
            .collect::<Vec<u32>>()
    };
    // Up faces are unshaded (1.0), so they carry the tint itself, packed as RGBA.
    let packed = |tint: u32| pack_rgb([tint >> 16 & 255, tint >> 8 & 255, tint & 255]);
    let before = biomes.get(forest).foliage & 0xFF_FFFF;
    assert!(
        colours(&biomes, &tables).contains(&packed(before)),
        "no face tinted {before:06X}"
    );
    biomes.list[usize::from(forest)].foliage = 0x12_3456;
    let after = colours(&biomes, &tables);
    assert!(
        after.contains(&packed(0x12_3456)),
        "the new mesh kept {before:06X}"
    );
    assert!(!after.contains(&packed(before)));

    // A failed native callback must still produce a mesh, and the next mesh of
    // the same section must complete normally.
    let mut failing = MeshTables::new(&assets, &render, &facts, visibility.clone(), None).unwrap();
    let mut registry = ColorResolverRegistry::default();
    registry.register(Arc::new(FoliageCopy));
    registry.register(Arc::new(ByColumn));
    let throwing = registry.register(Arc::new(Throwing));
    failing
        .apply_tint_layers(
            assets.registry,
            &BTreeMap::from([(
                "minecraft:oak_leaves".to_string(),
                Arc::from([TintLayer::Source(Arc::new(Resolved(throwing)))]),
            )]),
            Some(tint_values(assets.registry)),
        )
        .unwrap();
    assert!(colours(&biomes, &failing).contains(&packed(0xFF_FFFF)));
    assert!(colours(&biomes, &tables).contains(&packed(0x12_3456)));
}
