use super::*;

#[test]
fn padded_reuse_resets_missing_neighbors_and_matches_chunk_reads() {
    let mut center = Chunk::empty(0, 0);
    center.set(0, MIN_Y, 0, 7);
    center.set(15, MIN_Y + 15, 15, 9);
    let mut neighbor = Chunk::empty(1, 0);
    neighbor.set(0, MIN_Y + 2, 3, 11);
    let mut region = [[None; 3]; 3];
    region[1][1] = Some(&center);
    region[1][2] = Some(&neighbor);
    let mut pad = Padded::new(&region, 0);
    let buffers = (pad.blocks.as_ptr(), pad.light.as_ptr());
    let mut own = Box::new([u32::MAX; 4096]);
    for current in [region, [[None; 3]; 3], region] {
        pad.fill(&current, 0, &mut own);
        for y in -B..16 + B {
            for z in -B..16 + B {
                for x in -B..16 + B {
                    let chunk = current[((z + 16) >> 4) as usize][((x + 16) >> 4) as usize];
                    assert_eq!(
                        pad.blocks[pi(x, y, z)],
                        chunk.map_or(0, |c| c.get(x & 15, MIN_Y + y, z & 15))
                    );
                    assert_eq!(
                        pad.light[pi(x, y, z)],
                        chunk.map_or(0xF0, |c| c.light(x & 15, MIN_Y + y, z & 15))
                    );
                }
            }
        }
        assert_eq!(buffers, (pad.blocks.as_ptr(), pad.light.as_ptr()));
    }
}

/// Run explicitly with prepared assets; never silently pass without exercising geometry.
#[test]
#[ignore = "requires prepared 26.3 resources (HORIZON_MESH_TEST_RESOURCES or .local/horizon-client/26.3)"]
fn reused_mesh_matches_fresh_geometry_across_sections_and_resource_reload() {
    let root = std::env::var_os("HORIZON_MESH_TEST_RESOURCES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../.local/horizon-client/26.3")
        });
    let assets = Assets::load(&root).unwrap();
    let facts = BlockFacts::load(&root, assets.registry).unwrap();
    let biomes = Biomes::load(&root).unwrap();
    let state = |name| assets.registry.default_state(name).unwrap().0;
    let names = [
        "minecraft:stone",
        "minecraft:glass",
        "minecraft:grass_block",
        "minecraft:oak_leaves",
        "minecraft:water",
        "minecraft:chest",
    ];
    let mut dense = Chunk::empty(0, 0);
    for z in 0..16 {
        for x in 0..16 {
            dense.set(
                x,
                MIN_Y + 4,
                z,
                state(names[((x + z) as usize) % names.len()]),
            );
        }
    }
    let mut sparse = Chunk::empty(-3, 5);
    sparse.set(0, MIN_Y + 15, 0, state("minecraft:short_grass"));
    sparse.set(15, MIN_Y + 1, 15, state("minecraft:lava"));
    let empty = Chunk::empty(0, 0);
    let mut neighbor = Chunk::empty(1, 0);
    neighbor.set(0, MIN_Y + 4, 0, state("minecraft:stone"));
    let mut scratch = MeshScratch::default();
    let mut recycled = SectionMesh::default();
    // The same scratch and output buffers survive a resource reload. They must
    // contain no references into the previous RenderAssets or MeshTables.
    for reload in 0..2 {
        let render = assets.reload(&root).unwrap();
        let visibility = crate::models::load_visibility(&root, assets.registry).unwrap();
        let tables = MeshTables::new(&assets, &render, &facts, visibility.clone()).unwrap();
        let mut mesher = Mesher {
            nether_shading: false,
            assets: &assets,
            render: &render,
            tables: &tables,
            biomes: &biomes,
            visibility,
            zoom: BiomeZoom::new(42),
            blend_radius: 2,
            smooth_lighting: reload == 0,
            cutout_leaves: reload == 0,
        };
        for (case, (center, east)) in [
            (&empty, None),
            (&dense, Some(&neighbor)),
            (&sparse, None),
            (&empty, None),
            (&dense, None),
        ]
        .into_iter()
        .enumerate()
        {
            let mut region = [[None; 3]; 3];
            region[1][1] = Some(center);
            region[1][2] = east;
            let mut fresh = mesher.mesh(&region, 0);
            recycled = mesher.mesh_reusing(&region, 0, recycled, &mut scratch);
            if reload == 0 && case == 0 {
                assert!(scratch.own.is_none());
                assert_eq!(scratch.padded.blocks.capacity(), 0);
                assert_eq!(scratch.padded.light.capacity(), 0);
                assert_eq!(scratch.visibility.capacity(), 0);
            }
            fresh.sort_solid();
            recycled.sort_solid();
            assert_eq!(
                bytemuck::cast_slice::<TerrainVertex, u8>(&fresh.solid),
                bytemuck::cast_slice::<TerrainVertex, u8>(&recycled.solid)
            );
            assert_eq!(
                bytemuck::cast_slice::<TerrainVertex, u8>(&fresh.translucent),
                bytemuck::cast_slice::<TerrainVertex, u8>(&recycled.translucent)
            );
            assert_eq!(fresh.block_entities, recycled.block_entities);
            assert_eq!(fresh.visibility, recycled.visibility);
            assert_eq!(fresh.sides, recycled.sides);
            if std::ptr::eq(center, &dense) {
                assert!(!fresh.solid.is_empty());
                assert!(!fresh.translucent.is_empty());
                assert!(!fresh.block_entities.is_empty());
            }
        }
    }
}

/// An isolated scratch-stage comparison, not a whole-mesher or FPS benchmark.
/// Output each sample so the caller can retain the observations and spread.
#[test]
#[ignore = "opt-in scratch microbenchmark; run with --nocapture --test-threads=1"]
fn benchmark_padded_reuse() {
    const ITERATIONS: usize = 4096;
    let mut chunk = Chunk::empty(0, 0);
    for y in 0..16 {
        for z in 0..16 {
            for x in 0..16 {
                chunk.set(x, MIN_Y + y, z, 1 + ((x + y + z) % 7) as u32);
            }
        }
    }
    let mut region = [[None; 3]; 3];
    region[1][1] = Some(&chunk);
    let mut pad = Padded::new(&region, 0);
    let mut own = Box::new([0; 4096]);
    let pointers = (pad.blocks.as_ptr(), pad.light.as_ptr(), own.as_ptr());
    let bytes = pad.blocks.capacity() * 4 + pad.light.capacity() + 4096 * 4;
    // Warm both code paths and the process allocator before recording samples.
    for _ in 0..1024 {
        std::hint::black_box(Padded::new(std::hint::black_box(&region), 0));
        pad.fill(std::hint::black_box(&region), 0, &mut own);
        std::hint::black_box(&pad);
    }
    for sample in 0..9 {
        // Alternate execution order to expose, rather than hide, order effects.
        for reused in [sample % 2 == 0, sample % 2 != 0] {
            let start = std::time::Instant::now();
            for _ in 0..ITERATIONS {
                if reused {
                    pad.fill(std::hint::black_box(&region), 0, &mut own);
                    std::hint::black_box(&pad);
                } else {
                    std::hint::black_box(Padded::new(std::hint::black_box(&region), 0));
                }
            }
            println!(
                "{{\"sample\":{sample},\"reused\":{reused},\"iterations\":{ITERATIONS},\"elapsed_ms\":{:.6},\"scratch_bytes\":{bytes}}}",
                start.elapsed().as_secs_f64() * 1000.0
            );
            assert_eq!(
                pointers,
                (pad.blocks.as_ptr(), pad.light.as_ptr(), own.as_ptr())
            );
        }
    }
}
