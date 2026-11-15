use super::*;

#[test]
#[ignore = "daily"]
fn restored_proto_keeps_saved_biomes_when_saved_again_and_finished() {
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES").expect("prepared resources"),
    );
    let assets = crate::assets::Assets::load(&root).expect("assets");
    let biomes = Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
        root.clone(),
    ))
    .expect("biomes");
    let item_data = crate::test_resources::item_data(
        &root,
        horizon_world::native_load_context::DataLoadOrigin::ServerDataLoad,
    )
    .expect("item data");
    let terrain = VanillaTerrain::new(
        TerrainResources {
            data: None,
            dir: &root.join("worldgen"),
            registry: assets.registry,
            biomes: &biomes,
            item_data: &item_data,
        },
        12345,
    )
    .expect("terrain");
    let mut worker = terrain.worker();
    let mut proto = terrain.proto(&mut worker, 0, 0);
    let mut saved = terrain.saved_proto(&proto).expect("generated proto");
    let missing = biomes.saved_id("missing:proto_biome");
    let forest = biomes.id("minecraft:forest");
    for (i, section) in saved.column.sections.iter_mut().enumerate() {
        let section = Arc::make_mut(section);
        section.biomes = PalettedContainer::new(BIOME_STRATEGY, u32::from(missing));
        section.biomes.set(i % 64, u32::from(forest));
    }
    let expected = saved.column.clone();
    terrain.restore_proto(&mut proto, saved);
    let resaved = terrain.saved_proto(&proto).expect("restored proto");
    let finished = terrain.finish(proto);
    for column in [&resaved.column, &finished] {
        for y in (expected.min_y()..expected.max_y()).step_by(4) {
            for z in (0..16).step_by(4) {
                for x in (0..16).step_by(4) {
                    assert_eq!(column.biome(x, y, z), expected.biome(x, y, z));
                }
            }
        }
    }
    assert_eq!(
        biomes.saved_name(finished.biome(4, expected.min_y(), 0)),
        "missing:proto_biome"
    );
}
