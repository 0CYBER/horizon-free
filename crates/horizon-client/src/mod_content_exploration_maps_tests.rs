use super::*;

/// `StructureCheck.onStructureLoad`: the references a chunk saved for its start
/// count, so a map skips a start a vanilla map already pointed at.
#[test]
#[ignore = "daily"]
fn exploration_maps_skip_the_starts_a_chunk_saved_as_referenced() {
    let Some(base) = prepared() else {
        return;
    };
    let assets = Assets::load(&base).unwrap();
    let biomes = crate::biome::Biomes::load(
        &horizon_assets::resource_source::ResourceRoot::directory(base.to_path_buf()),
    )
    .unwrap();
    let item_data =
        crate::test_resources::item_data(&base, DataLoadOrigin::ServerDataLoad).unwrap();
    let terrain_resources = crate::vanillagen::TerrainResources {
        data: None,
        dir: &base.join("worldgen"),
        registry: assets.registry,
        biomes: &biomes,
        item_data: &item_data,
    };
    let terrain = crate::vanillagen::VanillaTerrain::new(terrain_resources, 1234).unwrap();
    let names = ["minecraft:buried_treasure".to_owned()];
    let first = terrain
        .locate_for_map(&names, [0, 64, 0], 50, true)
        .unwrap();
    // Another terrain, on the same seed, whose chunks saved one reference for it.
    let fresh = crate::vanillagen::VanillaTerrain::new(terrain_resources, 1234).unwrap();
    let (name, at) = (first.0.clone(), (first.1[0] >> 4, first.1[2] >> 4));
    fresh.set_saved_references(std::sync::Arc::new(move |n, x, z| {
        i32::from(n == name && (x, z) == at)
    }));
    let next = fresh.locate_for_map(&names, [0, 64, 0], 50, true).unwrap();
    assert_ne!(next.1, first.1, "the saved reference is skipped");
    assert_eq!(
        fresh.locate_for_map(&names, [0, 64, 0], 50, false).unwrap(),
        first,
        "a map that does not skip still finds it"
    );
}

/// A skip check reads each saved chunk's record once: the session keeps every
/// answer (as vanilla's `StructureCheck` does), so a second map searching past
/// the same chunks asks the save nothing it asked before (#10255: a chest with
/// exploration maps took a growing second per open reading them again).
#[test]
#[ignore = "daily"]
fn a_saved_chunks_record_is_read_once_across_map_searches() {
    let Some(base) = prepared() else {
        return;
    };
    let assets = Assets::load(&base).unwrap();
    let biomes = crate::biome::Biomes::load(
        &horizon_assets::resource_source::ResourceRoot::directory(base.clone()),
    )
    .unwrap();
    let item_data =
        crate::test_resources::item_data(&base, DataLoadOrigin::ServerDataLoad).unwrap();
    let terrain = crate::vanillagen::VanillaTerrain::new(
        crate::vanillagen::TerrainResources {
            data: None,
            dir: &base.join("worldgen"),
            registry: assets.registry,
            biomes: &biomes,
            item_data: &item_data,
        },
        1234,
    )
    .unwrap();
    let names = ["minecraft:buried_treasure".to_owned()];
    use std::sync::atomic::{AtomicUsize, Ordering};
    let reads = std::sync::Arc::new(AtomicUsize::new(0));
    let asked: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<(String, i32, i32)>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Default::default()));
    let (reads_of, asked_of) = (reads.clone(), asked.clone());
    terrain.set_saved_references(std::sync::Arc::new(move |n, x, z| {
        reads_of.fetch_add(1, Ordering::Relaxed);
        asked_of.lock().unwrap().insert((n.to_owned(), x, z));
        0
    }));
    terrain
        .locate_for_map(&names, [0, 64, 0], 50, true)
        .unwrap();
    // The same search again, now skipping the start the first pointed at:
    // it passes every chunk the first search read and may reach new ones,
    // but no chunk's record is read a second time.
    terrain
        .locate_for_map(&names, [0, 64, 0], 50, true)
        .unwrap();
    assert!(
        reads.load(Ordering::Relaxed) > 0,
        "the searches asked the save at all"
    );
    assert_eq!(
        reads.load(Ordering::Relaxed),
        asked.lock().unwrap().len(),
        "no saved chunk's record is read twice"
    );
}

/// `ChunkGenerator.findNearestMapStructure` with `createReference`: an
/// exploration map that skips known structures points at a start no map
/// pointed at before, and a reopened world remembers which.
#[test]
#[ignore = "daily"]
fn exploration_maps_skip_the_structures_maps_pointed_at() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-map-refs-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let assets = Assets::load(&base).unwrap();
    let biomes = crate::biome::Biomes::load(
        &horizon_assets::resource_source::ResourceRoot::directory(base.to_path_buf()),
    )
    .unwrap();
    let item_data =
        crate::test_resources::item_data(&base, DataLoadOrigin::ServerDataLoad).unwrap();
    let terrain_resources = crate::vanillagen::TerrainResources {
        data: None,
        dir: &base.join("worldgen"),
        registry: assets.registry,
        biomes: &biomes,
        item_data: &item_data,
    };
    let open = || {
        let terrain = crate::vanillagen::VanillaTerrain::new(terrain_resources, 1234).unwrap();
        terrain.set_reference_file(scratch.join("refs.json"));
        terrain
    };
    let names = ["minecraft:buried_treasure".to_owned()];
    let terrain = open();
    let first = terrain
        .locate_for_map(&names, [0, 64, 0], 50, true)
        .unwrap();
    assert_eq!(
        terrain.locate(&names, [0, 64, 0], 50).unwrap(),
        first,
        "/locate still finds the nearest"
    );
    let second = terrain
        .locate_for_map(&names, [0, 64, 0], 50, true)
        .unwrap();
    assert_ne!(second.1, first.1, "the second map points elsewhere");
    let again = terrain
        .locate_for_map(&names, [0, 64, 0], 50, false)
        .unwrap();
    assert_eq!(
        again, first,
        "without skipping the nearest is still the first"
    );
    drop(terrain);
    let third = open().locate_for_map(&names, [0, 64, 0], 50, true).unwrap();
    assert!(
        third.1 != first.1 && third.1 != second.1,
        "a reopened world remembers {first:?} and {second:?}, found {third:?}"
    );
    let _ = fs::remove_dir_all(&scratch);
}

/// `NoiseBasedChunkGenerator.addDebugScreenInfo`: F3's `Density` line lists
/// the noise settings' `debug_functions` with three decimals.
#[test]
#[ignore = "daily"]
fn debug_density_line_lists_the_noise_settings_functions() {
    let Some(base) = prepared() else {
        return;
    };
    let assets = Assets::load(&base).unwrap();
    let biomes = crate::biome::Biomes::load(
        &horizon_assets::resource_source::ResourceRoot::directory(base.to_path_buf()),
    )
    .unwrap();
    let item_data =
        crate::test_resources::item_data(&base, DataLoadOrigin::ServerDataLoad).unwrap();
    let terrain_resources = crate::vanillagen::TerrainResources {
        data: None,
        dir: &base.join("worldgen"),
        registry: assets.registry,
        biomes: &biomes,
        item_data: &item_data,
    };
    let terrain = crate::vanillagen::VanillaTerrain::new(terrain_resources, 1234).unwrap();
    let line = terrain.density_debug_line([0, 70, 0]).unwrap();
    let labels: Vec<&str> = line
        .strip_prefix("Density ")
        .unwrap()
        .split(' ')
        .step_by(2)
        .collect();
    assert_eq!(
        labels,
        ["N:", "T:", "V:", "C:", "E:", "D:", "W:", "PV:", "PS:"],
        "{line}"
    );
    assert!(
        line.split(' ')
            .skip(2)
            .step_by(2)
            .all(|v| v.split('.').nth(1).is_some_and(|d| d.len() == 3)),
        "{line}"
    );
}
