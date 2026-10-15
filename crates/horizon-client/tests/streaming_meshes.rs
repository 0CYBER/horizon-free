//! Every column the player comes close to gets meshed (#228): columns that
//! load with their light, past the render distance, used to drop their
//! sections' mesh requests there, and nothing asked again once the player
//! came closer, so they stayed invisible until a block in them changed.
use horizon_client::{
    assets::Assets,
    biome::Biomes,
    blocks::BlockFacts,
    chunk::{Chunk, HEIGHT},
    light::LightTables,
    mesher::MeshTables,
    models,
    streaming::{ChunkGenerator, Proto, Streaming, WorkerContext},
    world::World,
};
use horizon_world::registry::BlockStateId;
use std::{
    collections::HashSet,
    path::Path,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

fn resources() -> Option<std::path::PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3");
    root.join("items.json").exists().then_some(root)
}

/// A saved world: every column loads with its light, as saved chunks do.
struct Saved;
impl ChunkGenerator for Saved {
    type Worker = ();
    fn worker(&self) {}
    fn load(&self, x: i32, z: i32) -> Option<Chunk> {
        let mut chunk = Chunk::empty(x, z);
        chunk.lit = true;
        chunk.light_loaded = true;
        Some(chunk)
    }
    fn proto(&self, _: &mut (), _: i32, _: i32) -> Proto {
        unreachable!("every column is saved")
    }
    fn finish(&self, _: Proto) -> Chunk {
        unreachable!("every column is saved")
    }
}

#[test]
fn columns_loaded_past_the_render_distance_mesh_when_they_come_in_range() {
    let Some(root) = resources() else {
        return;
    };
    let assets = Arc::new(Assets::load(&root).unwrap());
    let facts = BlockFacts::load(&root, assets.registry).unwrap();
    let visibility = models::load_visibility(&root, assets.registry).unwrap();
    let context = Arc::new(WorkerContext {
        tables: Arc::new(MeshTables::new(&assets, &facts, visibility).unwrap()),
        light: Arc::new(LightTables::load(&root, assets.registry, &facts).unwrap()),
        biomes: Arc::new(Biomes::load(&root).unwrap()),
        assets,
    });
    let mut streaming = Streaming::new(Arc::new(Saved), context, 3);
    streaming.radius = 4;
    let mut world = World::new(BlockStateId(0), 0);
    let mut meshed = HashSet::new();
    let mut settle_at = |streaming: &mut Streaming, center: (i32, i32)| {
        streaming.center = center;
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut quiet = 0;
        while quiet < 40 {
            assert!(Instant::now() < deadline, "streaming never settled");
            let before = (world.chunks.len(), meshed.len());
            let (uploads, _) = streaming.update(&mut world);
            meshed.extend(uploads.into_iter().map(|(key, _)| key));
            thread::sleep(Duration::from_millis(5));
            let idle = streaming.pending() == 0 && before == (world.chunks.len(), meshed.len());
            quiet = if idle { quiet + 1 } else { 0 };
        }
    };
    // Columns up to three chunks past the render distance load around the
    // first position; the second brings some of them within it.
    settle_at(&mut streaming, (0, 0));
    settle_at(&mut streaming, (3, 0));
    let (cx, cz): (i32, i32) = (3, 0);
    let mut checked = 0;
    for x in cx - 4..=cx + 4 {
        for z in cz - 4..=cz + 4 {
            if (x - cx).pow(2) + (z - cz).pow(2) > 16 {
                continue;
            }
            for sy in 0..HEIGHT / 16 {
                assert!(
                    meshed.contains(&(x, sy, z)),
                    "section {x} {sy} {z} is in range but was never meshed"
                );
            }
            checked += 1;
        }
    }
    assert_eq!(checked, 49);
}
