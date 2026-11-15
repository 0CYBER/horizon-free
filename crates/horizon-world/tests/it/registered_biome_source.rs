//! A level stem naming a biome source a mod registered, as the noise generator reads it: the
//! codec takes it by id, its possible biomes are the mod's list (what structures and decoration
//! read), and its biome at a quart cell is the mod's answer for the world's seed. No other test
//! of this binary installs the process-wide table.
use horizon_world::worldgen::{
    biome_source::{BiomeSourceCatalog, BiomeSourceDefinition, ResolvedBiomeSource},
    mod_biome_sources::{ModBiomeSource, ModBiomeSources, install_mod_biome_sources},
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Desert, plains, desert again, then the badlands, along quart x from the seed.
struct Bands(Vec<String>);
impl ModBiomeSource for Bands {
    fn possible_biomes(&self) -> &[String] {
        &self.0
    }
    fn noise_biome(&self, [x, _, _]: [i32; 3], seed: i64) -> u16 {
        (i64::from(x) + seed).rem_euclid(4) as u16
    }
}

#[test]
fn the_noise_generators_codec_reads_a_registered_source() {
    let names = [
        "minecraft:desert",
        "minecraft:plains",
        "minecraft:desert",
        "minecraft:badlands",
    ];
    let bands = Bands(names.iter().map(|b| (*b).to_owned()).collect());
    let sources = ModBiomeSources::from([("test:bands".to_owned(), Arc::new(bands) as _)]);
    install_mod_biome_sources(sources).map_err(drop).unwrap();
    let world = [
        "minecraft:badlands",
        "minecraft:desert",
        "minecraft:ocean",
        "minecraft:plains",
    ];
    let catalog = BiomeSourceCatalog::new(
        &world.iter().map(|b| (*b).to_owned()).collect(),
        &BTreeMap::new(),
        &BTreeSet::new(),
    )
    .unwrap();
    let definition = catalog.decode(r#"{"type":"test:bands"}"#).unwrap();
    assert_eq!(
        definition,
        BiomeSourceDefinition::Registered("test:bands".into())
    );
    assert_eq!(
        catalog.encode(&definition).unwrap(),
        r#"{"type":"test:bands"}"#
    );

    let source = definition.resolve(&BTreeMap::new(), 3).unwrap();
    assert_eq!(
        source.possible_biomes(),
        BTreeSet::from(["minecraft:badlands", "minecraft:desert", "minecraft:plains"])
    );
    let ResolvedBiomeSource::Registered(level) = &source else {
        panic!("not the registered source: {source:?}");
    };
    // Seed 3 starts quart x 0 on the fourth band.
    let row: Vec<&str> = (0..4)
        .map(|x| level.noise_biome([x, 0, 0]).as_str())
        .collect();
    assert_eq!(
        row,
        [
            "minecraft:badlands",
            "minecraft:desert",
            "minecraft:plains",
            "minecraft:desert"
        ]
    );

    // A world without the badlands refuses the stem, naming the source and the biome.
    let catalog = BiomeSourceCatalog::new(
        &world[1..].iter().map(|b| (*b).to_owned()).collect(),
        &BTreeMap::new(),
        &BTreeSet::new(),
    )
    .unwrap();
    assert_eq!(
        catalog
            .decode(r#"{"type":"test:bands"}"#)
            .unwrap_err()
            .to_string(),
        "biome source test:bands names the unknown biome minecraft:badlands"
    );

    // A source nobody registered is refused naming its id.
    assert_eq!(
        catalog
            .decode(r#"{"type":"test:gone"}"#)
            .unwrap_err()
            .to_string(),
        "unknown biome source test:gone"
    );
}
