//! Real generated columns survive a save/reopen across region choices.
use super::{tests::*, *};
use crate::{
    assets::Assets,
    biome::Biomes,
    streaming::ChunkGenerator,
    vanillagen::{TerrainResources, VanillaTerrain},
};
use horizon_world::{native_load_context::DataLoadOrigin, worldgen::overrides::WorldgenOverrides};

#[test]
fn generated_chunks_survive_restart_substitution_added_regions_and_adoption() {
    let Some(root) = std::env::var_os("HORIZON_TEST_RESOURCES").map(PathBuf::from) else {
        eprintln!("skipping generated region persistence: HORIZON_TEST_RESOURCES is unset");
        return;
    };
    let assets = Assets::load(&root).unwrap();
    let biomes = Arc::new(
        Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
            root.clone(),
        ))
        .unwrap(),
    );
    let items = crate::test_resources::item_data(&root, DataLoadOrigin::ServerDataLoad).unwrap();
    let resources = TerrainResources {
        data: None,
        dir: &root.join("worldgen"),
        registry: assets.registry,
        biomes: &biomes,
        item_data: &items,
    };
    let make = |layout| {
        VanillaTerrain::for_world_with_settings(
            resources,
            &WorldgenOverrides::default(),
            12345,
            "minecraft:normal",
            "minecraft:overworld",
            false,
            None,
            layout,
        )
        .unwrap()
    };
    let fingerprint = |chunk: &crate::chunk::Chunk| {
        let mut values = Vec::new();
        for y in (chunk.min_y()..chunk.max_y()).step_by(4) {
            for z in (0..16).step_by(4) {
                for x in (0..16).step_by(4) {
                    values.push(biomes.name(chunk.biome(x, y, z)).to_owned());
                }
            }
        }
        values
    };
    let mut alpha = declaration("alpha", "alpha:ice", "minecraft:overworld", 1000);
    alpha.region.points[0] = mod_api::BiomePoint {
        biome: "minecraft:ice_spikes".into(),
        temperature: (-2., 2.),
        humidity: (-2., 2.),
        continentalness: (-2., 2.),
        erosion: (-2., 2.),
        depth: (-2., 2.),
        weirdness: (-2., 2.),
        offset: 0.,
    };
    let scratch = tempfile::tempdir().unwrap();
    let world = scratch.path();
    let mut original = BiomeRegions::from_declarations(std::slice::from_ref(&alpha));
    Arc::make_mut(original.dimensions.get_mut("minecraft:overworld").unwrap())
        .selector
        .vanilla_weight = 1;
    original.write(world).unwrap();
    let terrain = make(original.layout("minecraft:overworld"));
    let mut worker = terrain.worker();
    let old = terrain.finish(terrain.proto(&mut worker, 0, 0));
    let old_biomes = fingerprint(&old);
    assert!(
        old_biomes
            .iter()
            .all(|biome| biome == "minecraft:ice_spikes")
    );
    let save = super::super::SaveDir::new(world.to_path_buf(), super::super::LevelShape::OVERWORLD);
    save.save(&[Arc::new(old)], assets.registry, &biomes)
        .unwrap();
    let protos = (1..=2)
        .map(|x| {
            let proto = terrain.proto(&mut worker, x, 0);
            let mut saved = terrain.saved_proto(&proto).unwrap();
            saved.decorated = x == 2;
            saved
        })
        .collect();
    save.save_protos(protos, assets.registry, &biomes).unwrap();
    save.flush().unwrap();
    drop(save);
    drop(terrain);
    let saved_file = world.join("region/r.0.0.mca");
    let before = std::fs::read(&saved_file).unwrap();
    let mut beta = alpha.clone();
    beta.mod_id = "beta".into();
    beta.id = "beta:desert".into();
    beta.region.points[0].biome = "minecraft:desert".into();
    let both = [alpha, beta.clone()];
    // Installing B without opting in leaves even newly generated terrain unchanged.
    let unchanged = open_with(world, &mods(&["alpha", "beta"]), &both)
        .unwrap()
        .unwrap();
    let terrain = make(unchanged.layout("minecraft:overworld"));
    let mut worker = terrain.worker();
    let new = terrain.finish(terrain.proto(&mut worker, 0, 0));
    assert_eq!(fingerprint(&new), old_biomes);
    // Removing A requires a recorded choice; after substitution all its cells use vanilla.
    assert!(open_with(world, &mods(&[]), &[]).is_err());
    choose_with(world, &mods(&[]), &[], true, false).unwrap();
    let substituted = open_with(world, &mods(&[]), &[]).unwrap().unwrap();
    let terrain = make(substituted.layout("minecraft:overworld"));
    let mut worker = terrain.worker();
    let vanilla = make(None);
    let mut vanilla_worker = vanilla.worker();
    assert_eq!(
        fingerprint(&terrain.finish(terrain.proto(&mut worker, 0, 0))),
        fingerprint(&vanilla.finish(vanilla.proto(&mut vanilla_worker, 0, 0)))
    );
    // The explicit opt-in takes B, while A's already saved column is still read verbatim.
    choose_with(world, &mods(&["beta"]), &[beta], false, true).unwrap();
    let adopted = BiomeRegions::read(world).unwrap().unwrap();
    let terrain = make(adopted.layout("minecraft:overworld"));
    let mut worker = terrain.worker();
    let new = terrain.finish(terrain.proto(&mut worker, 0, 0));
    assert!(
        fingerprint(&new)
            .iter()
            .all(|biome| biome == "minecraft:desert")
    );
    let reopened =
        super::super::SaveDir::new(world.to_path_buf(), super::super::LevelShape::OVERWORLD);
    for (x, decorated) in [(1, false), (2, true)] {
        let saved = reopened.load_proto(x, 0, assets.registry, &biomes).unwrap();
        assert_eq!(saved.decorated, decorated);
        assert!(
            fingerprint(&saved.column)
                .iter()
                .all(|biome| biome == "minecraft:ice_spikes")
        );
        let blocks = saved.column.clone();
        let mut resumed = terrain.proto(&mut worker, x, 0);
        terrain.restore_proto(&mut resumed, saved);
        assert_eq!(terrain.is_decorated(&resumed), decorated);
        let resumed = terrain.saved_proto(&resumed).unwrap();
        // A resumed chunk keeps the biomes it was saved with, as a loaded one does.
        assert!(
            fingerprint(&resumed.column)
                .iter()
                .all(|biome| biome == "minecraft:ice_spikes")
        );
        for y in blocks.min_y()..blocks.max_y() {
            for z in 0..16 {
                for lx in 0..16 {
                    assert_eq!(
                        resumed.column.get(lx, y, z),
                        blocks.get(lx, y, z),
                        "resumed chunk {x} block {lx} {y} {z}"
                    );
                }
            }
        }
    }
    let loaded = reopened.load(0, 0, assets.registry, &biomes).unwrap();
    assert_eq!(fingerprint(&loaded), old_biomes);
    assert_eq!(std::fs::read(&saved_file).unwrap(), before);
}
