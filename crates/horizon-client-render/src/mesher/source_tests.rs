//! Original synthetic block/model/world controls; no prepared game resources.
use super::*;
use crate::assets::{Properties, SpritePixels};
use horizon_world::registry::BlockRegistry;
use serde_json::json;
use std::{collections::BTreeMap, path::PathBuf};

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn fixture() -> (Scratch, Assets, RenderAssets, MeshTables, Biomes) {
    let path = std::env::temp_dir().join(format!(
        "terrain-source-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&path).unwrap();
    let write = |name: &str, value: serde_json::Value| {
        let file = path.join(name);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, value.to_string()).unwrap();
    };
    let registry = json!({"defaults":{"minecraft:air":0,"fixture:signal":1},"states":[
        ["air",true,false,"", "minecraft:air"],
        ["signal",false,false,"",{"id":"fixture:signal","properties":{"active":"false","waterlogged":"false"}}],
        ["signal",false,true,"",{"id":"fixture:signal","properties":{"active":"true","waterlogged":"true"}}]
    ]});
    let registry = Box::leak(Box::new(
        BlockRegistry::from_facts_json(registry.to_string().as_bytes()).unwrap(),
    ));
    write(
        "assets/fixture/blockstates/signal.json",
        json!({"multipart":[
            {"apply":{"model":"fixture:block/low"}}, {"apply":{"model":"fixture:block/extended"}}
        ]}),
    );
    for (name, from, to) in [
        ("low", [0, 4, 0], [8, 4, 8]),
        ("extended", [20, 12, 0], [28, 12, 8]),
    ] {
        write(
            &format!("assets/fixture/models/block/{name}.json"),
            json!({
                "textures":{"all":"fixture:block/shared"},
                "elements":[{"from":from,"to":to,"faces":{"up":{"texture":"#all"}}}]
            }),
        );
    }
    // The same authored sprite is used by every part of both states.
    let png_path = path.join("assets/fixture/textures/block/shared.png");
    std::fs::create_dir_all(png_path.parent().unwrap()).unwrap();
    let encode = |file: PathBuf| {
        let mut encoder = png::Encoder::new(std::fs::File::create(file).unwrap(), 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[63, 127, 191, 255])
            .unwrap();
    };
    encode(png_path);
    let facts = BTreeMap::from([(
        "fixture:signal".into(),
        json!({
            "schema":{"active":{"named":["false","true"]},"waterlogged":{"named":["false","true"]}},
            "states":[{"active":"false","waterlogged":"false"},{"active":"true","waterlogged":"true"}]
        }),
    )]);
    let mut sprites = crate::sprites::Sprites::default();
    let catalog = crate::models::StateModels::from_facts(
        &horizon_assets::resource_source::ResourceSource::directory(path.clone()),
        registry,
        &["fixture:signal"],
        &mut sprites,
        &facts,
    )
    .unwrap();
    let render = RenderAssets {
        sprite_size: 1,
        sprite_minimum_size: 1,
        pixels: SpritePixels::new(vec![255; 4]),
        texture_layers: 1,
        model_catalog: catalog,
        model_library: Err("unused synthetic item models".into()),
        model_placements: vec![],
        model_gallery: vec![],
        animations: vec![],
        sprite_layers: BTreeMap::new(),
        emissive: vec![],
        translucent_layers: vec![false],
        sprite_mips: vec![],
        pbr_normals: vec![],
        pbr_specular: vec![],
        pbr_size: 1,
        connected: Default::default(),
        fusion: Default::default(),
        mod_animations: Default::default(),
        mod_pipelines: Default::default(),
        generation: 1,
    };
    let assets = Assets {
        registry,
        properties: Properties::for_tests(&[
            (true, false, false, false),
            (false, false, false, false),
            (false, true, false, false),
        ]),
        material_states: [BlockStateId(0); 6],
        blocks_fluid_flow: Default::default(),
    };
    let air = Info {
        solid: false,
        full: false,
        permeable: true,
        shade: 1.0,
        emission: 0,
        tier: 0,
        emissive: false,
        fluid: FluidState::EMPTY,
        model: false,
        block_entity: false,
        tints: [Tint::None; 2],
        tint_count: 0,
        colormap: None,
        offset: None,
        legacy_solid: false,
        overlay: false,
        leaves: false,
        pane: false,
        opaque_cube: false,
        blocks_flow: false,
        ice: false,
        opaque_ice: false,
        sturdy: 0,
        face: [0; 6],
        cover: [0.0; 6],
    };
    let info = vec![
        air,
        Info { model: true, ..air },
        Info {
            model: true,
            emission: 13,
            fluid: FluidState::WATER,
            ..air
        },
    ];
    let ao_info = lighting::AoInfo::build_table(&info);
    let tables = MeshTables {
        info,
        solid: vec![false; 3],
        full_faces: vec![0; 3],
        ao_info,
        visibility: FaceVisibility::empty(),
        water_still: 0,
        water_flow: 0,
        water_overlay: 0,
        lava_still: 0,
        lava_flow: 0,
        animated_layers: vec![false],
        colormaps: Vec::new(),
        water_colormap: None,
        mod_fluids: vec![],
        mod_tints: Default::default(),
        section_geometry: std::sync::Arc::new([]),
    };
    let maps = path.join("assets/minecraft/textures/colormap");
    std::fs::create_dir_all(&maps).unwrap();
    for name in ["grass", "foliage", "dry_foliage"] {
        encode(maps.join(format!("{name}.png")));
    }
    write(
        "data/minecraft/worldgen/biome/plains.json",
        json!({"temperature":0.5,"downfall":0.5,"effects":{"water_color":4159204}}),
    );
    let biomes = Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
        path.clone(),
    ))
    .unwrap();
    (Scratch(path), assets, render, tables, biomes)
}

#[test]
fn a_dispatched_mesh_keeps_streaming_busy_until_its_result_is_adopted() {
    use crate::{
        streaming::{ChunkGenerator, Proto, Streaming, WorkerContext},
        world::World,
    };
    use std::{
        sync::{Mutex, mpsc},
        time::Duration,
    };

    struct HoldFirstJob {
        entered: mpsc::Sender<()>,
        release: Mutex<Option<mpsc::Receiver<()>>>,
    }
    impl ChunkGenerator for HoldFirstJob {
        type Worker = ();
        fn worker(&self) {
            let release = self.release.lock().unwrap().take();
            if let Some(release) = release {
                self.entered.send(()).unwrap();
                release.recv_timeout(Duration::from_secs(5)).unwrap();
            }
        }
        fn proto(&self, _: &mut (), _: i32, _: i32) -> Proto {
            unreachable!("a remote view only meshes received columns")
        }
        fn finish(&self, _: Proto) -> Chunk {
            unreachable!("a remote view only meshes received columns")
        }
    }

    let (_scratch, assets, render, tables, biomes) = fixture();
    let context = Arc::new(WorkerContext {
        assets: Arc::new(assets),
        render: Arc::new(render),
        tables: Arc::new(tables),
        biomes: Arc::new(biomes),
        light: Arc::new(crate::light::tests::tables()),
        hidden_states: Arc::from([]),
        part_tables: Default::default(),
    });
    let (entered, running) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let generator = Arc::new(HoldFirstJob {
        entered,
        release: Mutex::new(Some(resume)),
    });
    let mut streaming = Streaming::new(generator, context, 1);
    streaming.remote = true;
    let mut world = World::new(BlockStateId(0), 0);
    for z in -1..=1 {
        for x in -1..=1 {
            streaming.insert_remote(&mut world, Chunk::empty(x, z));
        }
    }
    world.dirty_sections.clear();
    let section = (0, 0, 0);
    world.dirty_sections.insert(section);

    assert!(streaming.update(&mut world).0.is_empty());
    running.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(streaming.pending(), 0);
    assert!(world.dirty_sections.is_empty());
    let busy = streaming.busy();
    release.send(()).unwrap();
    assert!(busy);

    streaming.wait(Duration::from_secs(5));
    let (uploads, _) = streaming.update(&mut world);
    assert_eq!(uploads.len(), 1);
    assert_eq!(uploads[0].0, section);
    assert!(!streaming.busy());
}

#[test]
fn terrain_source_cpu_multipart_and_waterlogged_faces_keep_their_owner() {
    let (_scratch, assets, render, tables, biomes) = fixture();
    let mut chunk = Chunk::with_layout(3, -2, crate::chunk::ColumnLayout::new(0, 16).unwrap());
    chunk.set(2, 3, 4, 1);
    chunk.set(6, 3, 4, 2);
    let region = [
        [None, None, None],
        [None, Some(&chunk), None],
        [None, None, None],
    ];
    let mut mesher = Mesher {
        hidden_states: &[],
        part_tables: Default::default(),
        nether_shading: false,
        shader_pack_lighting: false,
        separate_ao: false,
        shader_pack: true,
        looks: false,
        assets: &assets,
        render: &render,
        tables: &tables,
        biomes: &biomes,
        visibility: FaceVisibility::empty(),
        zoom: BiomeZoom::new(0),
        blend_radius: 0,
        smooth_lighting: false,
        cutout_leaves: true,
        opaque_ice: false,
        translucent_layers: &[],
        leaf_culling: LeafCullingMode::Off,
    };
    let mut mesh = mesher.mesh(&region, 0);
    assert_eq!(mesh.solid.len(), 16, "two parts per source state");
    assert!(
        !mesh.translucent.is_empty(),
        "waterlogged fluid geometry was emitted"
    );
    assert_eq!(
        mesh.solid_sources,
        [
            TerrainSource::new(1, [2, 3, 4], 0, false),
            TerrainSource::new(1, [2, 3, 4], 0, false),
            TerrainSource::new(2, [6, 3, 4], 13, false),
            TerrainSource::new(2, [6, 3, 4], 13, false)
        ]
    );
    assert_eq!(mesh.translucent_sources.len(), mesh.translucent.len() / 4);
    assert!(
        mesh.translucent_sources
            .iter()
            .all(|s| *s == TerrainSource::new(2, [6, 3, 4], 13, true))
    );
    let before: Vec<_> = mesh
        .solid
        .as_chunks::<4>()
        .0
        .iter()
        .zip(&mesh.solid_sources)
        .map(|(q, s)| (bytemuck::cast_slice::<_, u8>(q).to_vec(), *s))
        .collect();
    mesh.sort_solid();
    for (q, s) in mesh
        .solid
        .as_chunks::<4>()
        .0
        .iter()
        .zip(&mesh.solid_sources)
    {
        assert!(
            before.contains(&(bytemuck::cast_slice::<_, u8>(q).to_vec(), *s)),
            "sort changed face ownership"
        );
    }
    // Reusing the same mesh for native/default rendering clears metadata and keeps
    // native model vertices byte-for-byte (fluid splitting already differs with packs), but
    // for the alpha a `shaders/` pack takes (`for_shaders_format`).
    mesher.shader_pack = false;
    let native = mesher.mesh_reusing(&region, 0, mesh, &mut MeshScratch::default());
    assert!(native.solid_sources.is_empty() && native.translucent_sources.is_empty());
    for q in native.solid.as_chunks::<4>().0.iter() {
        let q = q.map(|v| TerrainVertex {
            color: crate::mesher::for_shaders_format(v.color, None),
            ..v
        });
        assert!(
            before
                .iter()
                .any(|(bytes, _)| bytes == bytemuck::cast_slice::<_, u8>(&q)),
            "source tracking altered native vertices"
        );
    }
}

#[test]
fn terrain_source_cpu_partial_metadata_never_inherits_a_previous_face() {
    let mut mesh = SectionMesh::default();
    let quad = |y| {
        [[0.0, y, 0.0], [0.0, y, 1.0], [1.0, y, 1.0], [1.0, y, 0.0]]
            .map(|pos| TerrainVertex::new(pos, [0.0; 2], 0, 0, 0))
    };
    mesh.solid.extend(quad(1.0));
    let start = [mesh.solid.len(), 0];
    mesh.solid.extend(quad(3.0));
    mesh.solid.extend(quad(2.0));
    mesh.record_source(start, TerrainSource::new(71, [15, 0, 8], 7, false));
    assert_eq!(mesh.solid_sources[0], TerrainSource::default());
    assert_eq!(
        bytemuck::cast_slice::<_, u32>(&mesh.solid_sources[1..]),
        [71, 124943, 71, 124943]
    );
    mesh.sort_solid();
    assert_eq!(
        mesh.solid
            .as_chunks::<4>()
            .0
            .iter()
            .map(|q| q[0].pos[1])
            .collect::<Vec<_>>(),
        [3.0, 2.0, 1.0]
    );
    assert_eq!(
        mesh.solid_sources,
        [
            TerrainSource::new(71, [15, 0, 8], 7, false),
            TerrainSource::new(71, [15, 0, 8], 7, false),
            TerrainSource::default()
        ]
    );
    mesh.clear();
    assert!(mesh.solid_sources.is_empty());
}

#[test]
fn smooth_lighting_compact_ao_and_precomputed_cull_match_fallback_byte_for_byte() {
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, value: serde_json::Value| {
        let file = dir.path().join(name);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, value.to_string()).unwrap();
    };
    let registry = json!({
        "defaults": {
            "minecraft:air": 0,
            "fixture:cube": 1,
            "fixture:culled_part": 2,
            "fixture:multi": 3,
            "fixture:glow": 4
        },
        "states": [
            ["air", true, false, "", "minecraft:air"],
            ["cube", false, false, "", "fixture:cube"],
            ["culled_part", false, false, "", "fixture:culled_part"],
            ["multi", false, false, "", "fixture:multi"],
            ["glow", false, false, "", "fixture:glow"]
        ]
    });
    let registry = Box::leak(Box::new(
        BlockRegistry::from_facts_json(registry.to_string().as_bytes()).unwrap(),
    ));

    // Models: full culled cube, non-solid culled-only block, mixed culled+uncullable multipart, and glowing cube.
    let full_culled_model = json!({
        "textures": {"all": "fixture:block/shared"},
        "elements": [{
            "from": [0, 0, 0], "to": [16, 16, 16],
            "faces": {
                "down": {"texture": "#all", "cullface": "down"},
                "up": {"texture": "#all", "cullface": "up"},
                "north": {"texture": "#all", "cullface": "north"},
                "south": {"texture": "#all", "cullface": "south"},
                "west": {"texture": "#all", "cullface": "west"},
                "east": {"texture": "#all", "cullface": "east"}
            }
        }]
    });
    write(
        "assets/fixture/models/block/cube.json",
        full_culled_model.clone(),
    );
    write(
        "assets/fixture/models/block/culled_part.json",
        full_culled_model.clone(),
    );
    write("assets/fixture/models/block/glow.json", full_culled_model);
    write(
        "assets/fixture/models/block/multi_culled.json",
        json!({
            "textures": {"all": "fixture:block/shared"},
            "elements": [{
                "from": [0, 0, 0], "to": [16, 16, 16],
                "faces": {
                    "up": {"texture": "#all", "cullface": "up"},
                    "north": {"texture": "#all", "cullface": "north"}
                }
            }]
        }),
    );
    write(
        "assets/fixture/models/block/multi_inner.json",
        json!({
            "textures": {"all": "fixture:block/shared"},
            "elements": [{
                "from": [4, 4, 4], "to": [12, 12, 12],
                "faces": {"up": {"texture": "#all"}}
            }]
        }),
    );
    for name in ["cube", "culled_part", "glow"] {
        write(
            &format!("assets/fixture/blockstates/{name}.json"),
            json!({"variants": {"": {"model": format!("fixture:block/{name}")}}}),
        );
    }
    write(
        "assets/fixture/blockstates/multi.json",
        json!({"multipart": [
            {"apply": {"model": "fixture:block/multi_culled"}},
            {"apply": {"model": "fixture:block/multi_inner"}}
        ]}),
    );

    let encode = |file: PathBuf| {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let mut encoder = png::Encoder::new(std::fs::File::create(file).unwrap(), 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[200, 180, 160, 255])
            .unwrap();
    };
    encode(dir.path().join("assets/fixture/textures/block/shared.png"));
    for name in ["grass", "foliage", "dry_foliage"] {
        encode(
            dir.path()
                .join(format!("assets/minecraft/textures/colormap/{name}.png")),
        );
    }
    write(
        "data/minecraft/worldgen/biome/plains.json",
        json!({"temperature": 0.5, "downfall": 0.5, "effects": {"water_color": 4159204}}),
    );

    let facts = BTreeMap::from([
        ("fixture:cube".into(), json!({"schema": {}, "states": [{}]})),
        (
            "fixture:culled_part".into(),
            json!({"schema": {}, "states": [{}]}),
        ),
        (
            "fixture:multi".into(),
            json!({"schema": {}, "states": [{}]}),
        ),
        ("fixture:glow".into(), json!({"schema": {}, "states": [{}]})),
    ]);
    let mut sprites = crate::sprites::Sprites::default();
    let catalog = crate::models::StateModels::from_facts(
        &horizon_assets::resource_source::ResourceSource::directory(dir.path().to_path_buf()),
        registry,
        &[
            "fixture:cube",
            "fixture:culled_part",
            "fixture:multi",
            "fixture:glow",
        ],
        &mut sprites,
        &facts,
    )
    .unwrap();
    let render = RenderAssets {
        sprite_size: 1,
        sprite_minimum_size: 1,
        pixels: SpritePixels::new(vec![255; 4]),
        texture_layers: 1,
        model_catalog: catalog,
        model_library: Err("unused synthetic item models".into()),
        model_placements: vec![],
        model_gallery: vec![],
        animations: vec![],
        sprite_layers: BTreeMap::new(),
        emissive: vec![],
        translucent_layers: vec![false],
        sprite_mips: vec![],
        pbr_normals: vec![],
        pbr_specular: vec![],
        pbr_size: 1,
        connected: Default::default(),
        fusion: Default::default(),
        mod_animations: Default::default(),
        mod_pipelines: Default::default(),
        generation: 1,
    };
    let assets = Assets {
        registry,
        properties: Properties::for_tests(&[
            (true, false, false, false),
            (false, false, true, true),
            (false, false, false, false),
            (false, false, false, false),
            (false, false, true, true),
        ]),
        material_states: [BlockStateId(0); 6],
        blocks_fluid_flow: Default::default(),
    };
    let biomes = Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
        dir.path().to_path_buf(),
    ))
    .unwrap();

    let air = Info {
        solid: false,
        full: false,
        permeable: true,
        shade: 1.0,
        emission: 0,
        tier: 0,
        emissive: false,
        fluid: FluidState::EMPTY,
        model: false,
        block_entity: false,
        tints: [Tint::None; 2],
        tint_count: 0,
        colormap: None,
        offset: None,
        legacy_solid: false,
        overlay: false,
        leaves: false,
        pane: false,
        opaque_cube: false,
        blocks_flow: false,
        ice: false,
        opaque_ice: false,
        sturdy: 0,
        face: [0; 6],
        cover: [0.0; 6],
    };
    let make_tables = |info: Vec<Info>, with_ao_table: bool| MeshTables {
        solid: info.iter().map(|i| i.solid).collect(),
        full_faces: info
            .iter()
            .map(|i| (0..6).fold(0, |bits, d| bits | u8::from(i.face[d] == 1) << d))
            .collect(),
        ao_info: if with_ao_table {
            lighting::AoInfo::build_table(&info)
        } else {
            Vec::new()
        },
        info,
        visibility: FaceVisibility::empty(),
        water_still: 0,
        water_flow: 0,
        water_overlay: 0,
        lava_still: 0,
        lava_flow: 0,
        animated_layers: vec![false],
        colormaps: Vec::new(),
        water_colormap: None,
        mod_fluids: vec![],
        mod_tints: Default::default(),
        section_geometry: std::sync::Arc::new([]),
    };
    let base_info = vec![
        air,
        Info {
            solid: true,
            full: true,
            permeable: false,
            shade: 0.2,
            model: true,
            opaque_cube: true,
            face: [1; 6],
            ..air
        },
        Info {
            solid: false,
            full: false,
            permeable: true,
            shade: 1.0,
            model: true,
            ..air
        },
        Info {
            solid: false,
            full: false,
            permeable: true,
            shade: 1.0,
            model: true,
            ..air
        },
        Info {
            solid: true,
            full: true,
            permeable: false,
            shade: 0.2,
            emission: 15,
            emissive: true,
            model: true,
            opaque_cube: true,
            face: [1; 6],
            ..air
        },
    ];
    let tables_fast = make_tables(base_info.clone(), true);
    assert_eq!(tables_fast.ao_info.len(), base_info.len());
    let tables_fallback = make_tables(base_info.clone(), false);
    assert!(tables_fallback.ao_info.is_empty());

    let mut chunk = Chunk::with_layout(0, 0, crate::chunk::ColumnLayout::new(0, 16).unwrap());
    // Enclosed culled-only non-solid block at (3, 3, 3): hidden bitmask won't skip it,
    // so only precomputed cull_mask + visible_mask == 0 && !has_uncullable culls it completely.
    chunk.set(3, 3, 3, 2);
    for [dx, dy, dz] in [
        [1, 0, 0],
        [-1, 0, 0],
        [0, 1, 0],
        [0, -1, 0],
        [0, 0, 1],
        [0, 0, -1],
    ] {
        chunk.set(3 + dx, 3 + dy, 3 + dz, 1);
    }
    // Culled-only block at (8, 4, 8) with 5 cube neighbours and open top (+Y): emits 1 face.
    chunk.set(8, 4, 8, 2);
    for [dx, dy, dz] in [[1, 0, 0], [-1, 0, 0], [0, -1, 0], [0, 0, 1], [0, 0, -1]] {
        chunk.set(8 + dx, 4 + dy, 8 + dz, 1);
    }
    // Emissive block diagonally above (8, 4, 8) to exercise emissive/emission AO neighbour reads.
    chunk.set(9, 5, 8, 4);
    // Multipart block at (11, 5, 5): both culled faces of part 0 (Up, North) are hidden by cubes,
    // while part 1's uncullable face must still emit.
    chunk.set(11, 5, 5, 3);
    chunk.set(11, 6, 5, 1);
    chunk.set(11, 5, 4, 1);

    let mut light = Box::new([0u8; 4096]);
    for (idx, cell) in light.iter_mut().enumerate() {
        let sky = ((idx * 7 + 3) % 16) as u8;
        let block = ((idx * 5 + 1) % 16) as u8;
        *cell = (sky << 4) | block;
    }
    std::sync::Arc::make_mut(&mut chunk.sections[0]).light = crate::chunk::Light::Full(light);

    let region = [
        [None, None, None],
        [None, Some(&chunk), None],
        [None, None, None],
    ];
    let run_mesh = |tables: &MeshTables, separate_ao: bool, shader_pack: bool| {
        let mut mesher = Mesher {
            hidden_states: &[],
            part_tables: Default::default(),
            nether_shading: false,
            shader_pack_lighting: false,
            separate_ao,
            shader_pack,
            looks: true,
            assets: &assets,
            render: &render,
            tables,
            biomes: &biomes,
            visibility: FaceVisibility::empty(),
            zoom: BiomeZoom::new(0),
            blend_radius: 0,
            smooth_lighting: true,
            cutout_leaves: true,
            opaque_ice: false,
            translucent_layers: &[],
            leaf_culling: LeafCullingMode::Off,
        };
        mesher.mesh(&region, 0)
    };

    for (separate_ao, shader_pack) in [(false, false), (false, true), (true, false)] {
        let fast = run_mesh(&tables_fast, separate_ao, shader_pack);
        let fallback = run_mesh(&tables_fallback, separate_ao, shader_pack);
        assert!(!fast.solid.is_empty());
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&fast.solid),
            bytemuck::cast_slice::<_, u8>(&fallback.solid),
            "solid vertex bytes mismatch (separate_ao={separate_ao}, shader_pack={shader_pack})"
        );
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&fast.translucent),
            bytemuck::cast_slice::<_, u8>(&fallback.translucent),
            "translucent vertex bytes mismatch"
        );
        assert_eq!(fast.solid_tints, fallback.solid_tints);
        assert_eq!(fast.solid_looks, fallback.solid_looks);
        assert_eq!(fast.visibility, fallback.visibility);
        if shader_pack {
            // Verify exact per-block quad counts for early-cull and multipart per-part skip.
            let enclosed = TerrainSource::new(2, [3, 3, 3], 0, false);
            let open_top = TerrainSource::new(2, [8, 4, 8], 0, false);
            let multi = TerrainSource::new(3, [11, 5, 5], 0, false);
            assert_eq!(
                fast.solid_sources
                    .iter()
                    .filter(|&&s| s == enclosed)
                    .count(),
                0,
                "fully occluded culled-only block must emit zero quads"
            );
            assert_eq!(
                fast.solid_sources
                    .iter()
                    .filter(|&&s| s == open_top)
                    .count(),
                1,
                "single exposed direction must emit one quad"
            );
            assert_eq!(
                fast.solid_sources.iter().filter(|&&s| s == multi).count(),
                1,
                "multipart with all culled parts hidden must emit only its uncullable part"
            );
        }
    }

    // A state with a non-{0.2, 1.0} shade automatically falls back to full Info and preserves exact AO shading.
    let mut custom_info = base_info;
    custom_info[1].shade = 0.5;
    let tables_custom = make_tables(custom_info, true);
    assert!(
        tables_custom.ao_info.is_empty(),
        "custom shade 0.5 must disable compact AoInfo table"
    );
    let custom_mesh = run_mesh(&tables_custom, false, false);
    let fast_mesh = run_mesh(&tables_fast, false, false);
    assert_eq!(custom_mesh.solid.len(), fast_mesh.solid.len());
    assert_ne!(
        bytemuck::cast_slice::<_, u8>(&custom_mesh.solid),
        bytemuck::cast_slice::<_, u8>(&fast_mesh.solid),
        "fallback path must apply the custom 0.5 shade instead of clamping to 0.2/1.0"
    );
}

#[test]
fn models_mut_face_cull_direction_edit_refreshes_mesher_culling() {
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, value: serde_json::Value| {
        let file = dir.path().join(name);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, value.to_string()).unwrap();
    };
    let registry = json!({
        "defaults": {
            "minecraft:air": 0,
            "fixture:cube": 1,
            "fixture:panel": 2
        },
        "states": [
            ["air", true, false, "", "minecraft:air"],
            ["cube", false, false, "", "fixture:cube"],
            ["panel", false, false, "", "fixture:panel"]
        ]
    });
    let registry = Box::leak(Box::new(
        BlockRegistry::from_facts_json(registry.to_string().as_bytes()).unwrap(),
    ));

    write(
        "assets/fixture/models/block/cube.json",
        json!({
            "textures": {"all": "fixture:block/shared"},
            "elements": [{
                "from": [0, 0, 0], "to": [16, 16, 16],
                "faces": {
                    "down": {"texture": "#all", "cullface": "down"},
                    "up": {"texture": "#all", "cullface": "up"},
                    "north": {"texture": "#all", "cullface": "north"},
                    "south": {"texture": "#all", "cullface": "south"},
                    "west": {"texture": "#all", "cullface": "west"},
                    "east": {"texture": "#all", "cullface": "east"}
                }
            }]
        }),
    );
    write(
        "assets/fixture/models/block/panel.json",
        json!({
            "textures": {"all": "fixture:block/shared"},
            "elements": [{
                "from": [0, 0, 0], "to": [16, 16, 16],
                "faces": {
                    "north": {"texture": "#all", "cullface": "north"}
                }
            }]
        }),
    );
    for name in ["cube", "panel"] {
        write(
            &format!("assets/fixture/blockstates/{name}.json"),
            json!({"variants": {"": {"model": format!("fixture:block/{name}")}}}),
        );
    }

    let encode = |file: PathBuf| {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let mut encoder = png::Encoder::new(std::fs::File::create(file).unwrap(), 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[200, 180, 160, 255])
            .unwrap();
    };
    encode(dir.path().join("assets/fixture/textures/block/shared.png"));
    for name in ["grass", "foliage", "dry_foliage"] {
        encode(
            dir.path()
                .join(format!("assets/minecraft/textures/colormap/{name}.png")),
        );
    }
    write(
        "data/minecraft/worldgen/biome/plains.json",
        json!({"temperature": 0.5, "downfall": 0.5, "effects": {"water_color": 4159204}}),
    );

    let facts = BTreeMap::from([
        ("fixture:cube".into(), json!({"schema": {}, "states": [{}]})),
        (
            "fixture:panel".into(),
            json!({"schema": {}, "states": [{}]}),
        ),
    ]);
    let mut sprites = crate::sprites::Sprites::default();
    let catalog = crate::models::StateModels::from_facts(
        &horizon_assets::resource_source::ResourceSource::directory(dir.path().to_path_buf()),
        registry,
        &["fixture:cube", "fixture:panel"],
        &mut sprites,
        &facts,
    )
    .unwrap();
    let mut render = RenderAssets {
        sprite_size: 1,
        sprite_minimum_size: 1,
        pixels: SpritePixels::new(vec![255; 4]),
        texture_layers: 1,
        model_catalog: catalog,
        model_library: Err("unused synthetic item models".into()),
        model_placements: vec![],
        model_gallery: vec![],
        animations: vec![],
        sprite_layers: BTreeMap::new(),
        emissive: vec![],
        translucent_layers: vec![false],
        sprite_mips: vec![],
        pbr_normals: vec![],
        pbr_specular: vec![],
        pbr_size: 1,
        connected: Default::default(),
        fusion: Default::default(),
        mod_animations: Default::default(),
        mod_pipelines: Default::default(),
        generation: 1,
    };
    let assets = Assets {
        registry,
        properties: Properties::for_tests(&[
            (true, false, false, false),
            (false, false, true, true),
            (false, false, false, false),
        ]),
        material_states: [BlockStateId(0); 6],
        blocks_fluid_flow: Default::default(),
    };
    let biomes = Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
        dir.path().to_path_buf(),
    ))
    .unwrap();

    let air = Info {
        solid: false,
        full: false,
        permeable: true,
        shade: 1.0,
        emission: 0,
        tier: 0,
        emissive: false,
        fluid: FluidState::EMPTY,
        model: false,
        block_entity: false,
        tints: [Tint::None; 2],
        tint_count: 0,
        colormap: None,
        offset: None,
        legacy_solid: false,
        overlay: false,
        leaves: false,
        pane: false,
        opaque_cube: false,
        blocks_flow: false,
        ice: false,
        opaque_ice: false,
        sturdy: 0,
        face: [0; 6],
        cover: [0.0; 6],
    };
    let info = vec![
        air,
        Info {
            solid: true,
            full: true,
            permeable: false,
            shade: 0.2,
            model: true,
            opaque_cube: true,
            face: [1; 6],
            ..air
        },
        Info {
            solid: false,
            full: false,
            permeable: true,
            shade: 1.0,
            model: true,
            ..air
        },
    ];
    let tables = MeshTables {
        solid: info.iter().map(|i| i.solid).collect(),
        full_faces: info
            .iter()
            .map(|i| (0..6).fold(0, |bits, d| bits | u8::from(i.face[d] == 1) << d))
            .collect(),
        ao_info: lighting::AoInfo::build_table(&info),
        info,
        visibility: FaceVisibility::empty(),
        water_still: 0,
        water_flow: 0,
        water_overlay: 0,
        lava_still: 0,
        lava_flow: 0,
        animated_layers: vec![false],
        colormaps: Vec::new(),
        water_colormap: None,
        mod_fluids: vec![],
        mod_tints: Default::default(),
        section_geometry: std::sync::Arc::new([]),
    };

    let mut chunk = Chunk::with_layout(0, 0, crate::chunk::ColumnLayout::new(0, 16).unwrap());
    // Place the non-solid panel at (8, 4, 8) with an opaque cube directly to its North (z - 1)
    // and air to its South (z + 1).
    chunk.set(8, 4, 8, 2);
    chunk.set(8, 4, 7, 1);

    let region = [
        [None, None, None],
        [None, Some(&chunk), None],
        [None, None, None],
    ];
    let panel_source = TerrainSource::new(2, [8, 4, 8], 0, false);
    let panel_quads = |render: &RenderAssets| {
        let mut mesher = Mesher {
            hidden_states: &[],
            part_tables: Default::default(),
            nether_shading: false,
            shader_pack_lighting: false,
            separate_ao: false,
            shader_pack: true,
            looks: false,
            assets: &assets,
            render,
            tables: &tables,
            biomes: &biomes,
            visibility: FaceVisibility::empty(),
            zoom: BiomeZoom::new(0),
            blend_radius: 0,
            smooth_lighting: true,
            cutout_leaves: true,
            opaque_ice: false,
            translucent_layers: &[],
            leaf_culling: LeafCullingMode::Off,
        };
        mesher
            .mesh(&region, 0)
            .solid_sources
            .iter()
            .filter(|&&s| s == panel_source)
            .count()
    };

    // Before mutation: north-culled face is hidden by the opaque north neighbor, and the cache is warm.
    assert_eq!(panel_quads(&render), 0);
    let warm_ptr = render.model_catalog.cull_flags().as_ptr();
    assert_eq!(panel_quads(&render), 0);
    assert!(
        std::ptr::eq(warm_ptr, render.model_catalog.cull_flags().as_ptr()),
        "unmutated models must reuse the cached cull flags"
    );

    let set_panel_cull = |render: &mut RenderAssets, cull: Option<ModelDirection>| {
        let mut edited = 0usize;
        for (variant, model) in render.model_catalog.models_mut() {
            if variant.model == "fixture:block/panel" {
                model.faces[0].quad.cull = cull;
                edited += 1;
            }
        }
        assert_eq!(edited, 1);
    };

    // Edit the north-culled face via `models_mut()` to become uncullable:
    // the mesher must invalidate and emit it despite the opaque north neighbor.
    set_panel_cull(&mut render, None);
    assert_eq!(
        panel_quads(&render),
        1,
        "north-culled face edited to uncullable via models_mut() must not be discarded"
    );

    // Edit back to north-culled: opaque north neighbor hides it again.
    set_panel_cull(&mut render, Some(ModelDirection::North));
    assert_eq!(
        panel_quads(&render),
        0,
        "uncullable face edited to north-culled must be culled next to opaque north neighbor"
    );

    // Edit cull direction from north to south (where the neighbor is air): face emits again.
    set_panel_cull(&mut render, Some(ModelDirection::South));
    assert_eq!(
        panel_quads(&render),
        1,
        "face edited to south-culled must emit when south neighbor is air"
    );
    let refreshed_ptr = render.model_catalog.cull_flags().as_ptr();
    assert!(std::ptr::eq(
        refreshed_ptr,
        render.model_catalog.cull_flags().as_ptr()
    ));
}
