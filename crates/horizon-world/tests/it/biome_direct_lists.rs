use horizon_world::worldgen::biome_source::{
    BiomeSet, BiomeSourceCatalog, BiomeSourceDefinition, CheckerboardBiomeSource, CheckerboardError,
};
use std::collections::{BTreeMap, BTreeSet};

#[test]
fn direct_biome_lists_preserve_large_ordered_duplicates() {
    let catalog = BiomeSourceCatalog::new(
        &BTreeSet::from(["minecraft:plains".into(), "minecraft:desert".into()]),
        &BTreeMap::new(),
        &BTreeSet::new(),
    )
    .unwrap();
    for len in [8192, 8193, 9000] {
        let mut names = vec!["minecraft:plains".to_owned(); len];
        names[1] = "minecraft:desert".into();
        names[len - 1] = "minecraft:desert".into();
        let json = serde_json::json!({"type":"checkerboard", "scale":0,"biomes":names}).to_string();
        let definition = catalog.decode(&json).unwrap();
        let BiomeSourceDefinition::Checkerboard {
            biomes: BiomeSet::Direct(decoded),
            scale,
        } = &definition
        else {
            panic!("wrong source")
        };
        assert_eq!(decoded, &names);
        assert_eq!(
            catalog
                .decode(&catalog.encode(&definition).unwrap())
                .unwrap(),
            definition
        );
        let source = CheckerboardBiomeSource::new(decoded.clone(), *scale).unwrap();
        // scale 0 shifts by 2: cells 0, 1, 2, -1, -2 and len + 1.
        for (quart, expected) in [
            ([0, 0, 0], "minecraft:plains"),
            ([4, 0, 0], "minecraft:desert"),
            ([8, 0, 0], "minecraft:plains"),
            ([-1, 99, 0], "minecraft:desert"),
            ([-5, 0, 0], "minecraft:plains"),
            ([(len as i32 + 1) * 4, 0, 0], "minecraft:desert"),
        ] {
            assert_eq!(source.noise_biome(quart).unwrap(), expected);
        }
        let unknown = json.replace("minecraft:desert", "minecraft:unknown");
        assert!(catalog.decode(&unknown).is_err());
    }
}

#[test]
fn direct_biome_distinct_catalog_has_no_size_ceiling() {
    let names: Vec<String> = (0..9000).map(|i| format!("test:biome_{i}")).collect();
    let catalog = BiomeSourceCatalog::new(
        &names.iter().cloned().collect(),
        &BTreeMap::new(),
        &BTreeSet::new(),
    )
    .unwrap();
    let definition = catalog
        .decode(
            &serde_json::json!({
                "type":"checkerboard", "scale":0, "biomes":names,
            })
            .to_string(),
        )
        .unwrap();
    let BiomeSourceDefinition::Checkerboard {
        biomes: BiomeSet::Direct(decoded),
        scale,
    } = definition
    else {
        panic!("wrong source")
    };
    assert_eq!(decoded, names);
    let source = CheckerboardBiomeSource::new(decoded, scale).unwrap();
    // Shift by 2, add with Java wrapping, then floor-mod by 9000.
    for (quart, expected) in [
        ([0, 0, 0], "test:biome_0"),
        ([4, 0, 8], "test:biome_3"),
        ([-1, 99, 0], "test:biome_8999"),
        ([-5, 0, -4], "test:biome_8997"),
        ([36004, 0, 0], "test:biome_1"),
    ] {
        assert_eq!(source.noise_biome(quart).unwrap(), expected);
    }
}

#[test]
fn direct_biome_tag_and_preset_catalogs_have_no_size_ceiling() {
    let tags: BTreeMap<String, Vec<String>> = (0..9000)
        .map(|i| (format!("test:tag_{i}"), vec!["test:biome".into(); 8]))
        .collect();
    let presets = (0..9000).map(|i| format!("test:preset_{i}")).collect();
    let catalog =
        BiomeSourceCatalog::new(&BTreeSet::from(["test:biome".into()]), &tags, &presets).unwrap();
    assert!(
        matches!(catalog.decode(r##"{"type":"checkerboard","biomes":"#test:tag_8999"}"##).unwrap(),
        BiomeSourceDefinition::Checkerboard { biomes: BiomeSet::Tag { members, .. }, .. } if members.len() == 8)
    );
    assert_eq!(
        catalog
            .decode(r#"{"type":"multi_noise","preset":"test:preset_8999"}"#)
            .unwrap(),
        BiomeSourceDefinition::Preset {
            name: "test:preset_8999".into(),
            inline: false
        }
    );
}

#[test]
#[ignore = "daily"]
fn inline_multi_noise_biome_list_has_no_size_ceiling() {
    let catalog = BiomeSourceCatalog::new(
        &BTreeSet::from(["test:biome".into()]),
        &BTreeMap::new(),
        &BTreeSet::new(),
    )
    .unwrap();
    let entry = serde_json::json!({
        "parameters": {
            "temperature":0.0, "humidity":0.0, "continentalness":0.0,
            "erosion":0.0, "depth":0.0, "weirdness":0.0, "offset":0.0,
        },
        "biome":"test:biome",
    });
    let json = serde_json::json!({"type":"multi_noise", "biomes":vec![entry; 65_537]}).to_string();
    let BiomeSourceDefinition::MultiNoise(entries) = catalog.decode(&json).unwrap() else {
        panic!("wrong source")
    };
    assert_eq!(entries.len(), 65_537);
    assert!(
        entries
            .iter()
            .all(|(point, name)| point == &entries[0].0 && name == "test:biome")
    );
    assert!(
        catalog
            .decode(r#"{"type":"multi_noise","biomes":[]}"#)
            .is_err()
    );
}

#[test]
fn direct_biome_empty_sampling_is_refused() {
    let source = CheckerboardBiomeSource::<()>::new(vec![], 0).unwrap();
    assert_eq!(
        source.noise_biome([0; 3]),
        Err(CheckerboardError::EmptyBiomes)
    );
}

#[test]
fn direct_biome_lengths_do_not_truncate_to_i32() {
    // Zero-sized entries exercise huge lengths without allocating huge memory.
    for len in [i32::MAX as usize + 1, u32::MAX as usize + 1] {
        let source = CheckerboardBiomeSource::new(vec![(); len], 0).unwrap();
        assert_eq!(source.noise_biome([-1, 0, 0]), Ok(&()));
        assert_eq!(source.noise_biome([0; 3]), Ok(&()));
    }
}
