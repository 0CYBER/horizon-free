use super::super::{
    biome_source::regions::{LayoutRegion, RegionLayout, RegionPoint, SelectorParams},
    structure::{jigsaw::JigsawContext, start::StartContext},
};
use super::*;

/// Structure placement's biome checks (`couldStructureExistInColumn` and the start's
/// noise biome) read the region that owns their own column, on both sides of a border.
#[test]
fn structure_biome_checks_read_the_region_at_their_own_quart() {
    let root = std::env::var_os("HORIZON_TEST_RESOURCES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3")
        });
    if !root.join("worldgen/world_presets.json").exists() {
        eprintln!("skipping: no worldgen bundle at {}", root.display());
        return;
    }
    let registry = Box::leak(Box::new(
        crate::level::data::load_registry(&root).expect("registry"),
    ));
    let data = GeneratorData::load(&root.join("worldgen"), registry).expect("worldgen data");
    // One point over every climate: a region's quarts are all its one biome.
    let region = |id: &str, biome: &str| LayoutRegion {
        owner: "test".to_owned(),
        id: id.to_owned(),
        weight: 1,
        points: vec![RegionPoint {
            biome: biome.to_owned(),
            intervals: [(-2., 2.); 6],
            offset: 0.,
        }],
        substituted: false,
    };
    let biomes = ["minecraft:mushroom_fields", "minecraft:ice_spikes"];
    let layout = RegionLayout {
        selector: SelectorParams {
            cell: 4,
            vanilla_weight: 0,
            ..SelectorParams::current()
        },
        regions: vec![region("test:a", biomes[0]), region("test:b", biomes[1])],
    };
    let preset: Value = serde_json::from_str(&data.world_presets["minecraft:normal"]).unwrap();
    let generator_json = &preset["dimensions"]["minecraft:overworld"]["generator"];
    let settings: Value =
        serde_json::from_str(&data.settings[generator_json["settings"].as_str().unwrap()]).unwrap();
    let pipeline = Arc::new(
        Pipeline::new(
            data,
            12345,
            XoroshiroRandom::new(12345),
            generator_json,
            &settings,
            true,
            Some((-64, 384)),
            Some(&layout),
        )
        .expect("pipeline"),
    );
    let generator = OverworldGenerator {
        pipeline: pipeline.clone(),
        noise_clamped: None,
    };
    let mut worker = generator.worker();
    assert!(matches!(pipeline.source, ResolvedBiomeSource::Regions(_)));
    let id = |biome| pipeline.data.biome_id(biome).expect("vanilla biome");
    // Seed 12345, cell 4, weights [0, 1, 1]: independently calculated border quarts.
    for (x, expected, other) in [
        (7, "minecraft:ice_spikes", "minecraft:mushroom_fields"),
        (8, "minecraft:mushroom_fields", "minecraft:ice_spikes"),
    ] {
        let mut queries = StructureQueries {
            pipeline: &*pipeline,
            worker: &mut worker.state,
            valid: BTreeSet::from([id(expected)]),
        };
        assert_eq!(queries.noise_biome([x, 16, 0]), id(expected), "quart {x}");
        assert!(queries.column_has_valid_biome(x * 4, 0, 0, 64), "quart {x}");
        queries.valid = BTreeSet::from([id(other)]);
        assert!(
            !queries.column_has_valid_biome(x * 4, 0, 0, 64),
            "quart {x}"
        );
    }
}
