use crate::{blink::Blinks, cem::files, ctm, emissive::Emissive, random_textures::RandomTextures};
use horizon_assets::resource_source::ResourceSource;
use std::{fs, io::Write};

#[test]
fn folder_and_archive_feature_scans_load_the_same_resources_without_unpacking() {
    let root =
        std::env::temp_dir().join(format!("horizon-optifine-archive-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let folder = root.join("folder");
    let archive = root.join("pack.jar");
    let mut writer = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
    for (name, text) in [
        (
            "assets/minecraft/optifine/ctm/block_stone.properties",
            "method=fixed\ntiles=0\n",
        ),
        ("assets/minecraft/textures/entity/cow/cow.png", "png"),
        ("assets/minecraft/textures/entity/cow/cow_blink.png", "png"),
        (
            "assets/minecraft/textures/entity/cow/cow_blink.properties",
            "blinkLength=7\nblinkFrequency=30\n",
        ),
        ("assets/minecraft/textures/entity/cow/cow_glow.png", "png"),
        (
            "assets/minecraft/optifine/emissive.properties",
            "suffix.emissive=_glow\n",
        ),
        (
            "assets/minecraft/optifine/random/entity/cow/cow2.png",
            "png",
        ),
        (
            "assets/minecraft/optifine/random/entity/cow/cow2_blink.png",
            "png",
        ),
        (
            "assets/minecraft/optifine/random/entity/cow/cow2_glow.png",
            "png",
        ),
        (
            "assets/minecraft/emf/cem/cow.jem",
            r#"{"texture":"textures/entity/cow/cow.png","models":[{"part":"body","model":"piece.jpm"}]}"#,
        ),
        (
            "assets/minecraft/emf/cem/piece.jpm",
            r#"{"boxes":[{"coordinates":[0,0,0,4,2,4],"uvFront":[0,0,4,2]}]}"#,
        ),
        ("assets/minecraft/optifine/cem/cow/cow.jem", "{}"),
    ] {
        let path = folder.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
        writer
            .start_file(
                name,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated),
            )
            .unwrap();
        writer.write_all(text.as_bytes()).unwrap();
    }
    writer.finish().unwrap();
    for source in [
        ResourceSource::directory(folder),
        ResourceSource::archive(&archive).unwrap(),
    ] {
        let rules = ctm::load(&source);
        assert_eq!(rules.len(), 1);
        assert_eq!(
            rules[0].path,
            "assets/minecraft/optifine/ctm/block_stone.properties"
        );
        let random = RandomTextures::scan(&source);
        assert_eq!(
            random.get("entity/cow/cow").unwrap().texture(2),
            Some("minecraft:optifine/random/entity/cow/cow2")
        );
        let blinks = Blinks::scan(&source, &random);
        let blink = blinks.get("entity/cow/cow").unwrap();
        assert_eq!((blink.length, blink.frequency), (7, 30));
        assert!(
            blinks
                .get("minecraft:optifine/random/entity/cow/cow2")
                .is_some()
        );
        let emissive = Emissive::scan(&source, &random);
        assert_eq!(
            emissive.overlay("entity/cow/cow"),
            Some("entity/cow/cow_glow")
        );
        assert_eq!(
            emissive.overlay("minecraft:optifine/random/entity/cow/cow2"),
            Some("minecraft:optifine/random/entity/cow/cow2_glow")
        );
        let models = files::jem_files(&source);
        assert_eq!(
            models["cow"],
            [
                "assets/minecraft/emf/cem/cow.jem",
                "assets/minecraft/optifine/cem/cow/cow.jem"
            ]
        );
        let model = files::load_jem(&source, &models["cow"][0]).unwrap();
        assert!(model.warnings.is_empty(), "{:?}", model.warnings);
        assert_eq!(model.parts.len(), 1);
        assert_eq!(model.parts[0].part, "body");
        assert!(!model.parts[0].bone.quads.is_empty());
    }
    assert!(!root.join("assets").exists());
    assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn better_grass_reads_a_pack_archive() {
    use crate::better_grass::{Rules, Surface::*};

    let archive = std::env::temp_dir().join(format!(
        "horizon-optifine-better-grass-{}.zip",
        std::process::id()
    ));
    let mut writer = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
    let properties = br"
        ! Hillside pack
        grass = true
        grass = FaLsE
        dirt_path: false
        mycelium false
        podzol=unspecified
        crimson_nylium=false
        warped_nylium=false
        grass.snow=true
        mycelium.snow=false
        podzol.snow=false
        dirt_path.snow=true
        grass.multilayer=TRUE
        texture.grass : block/\
            heather
        texture.dirt_path=trails:block/chalk_path.png
        texture.mycelium=textures/block/moss_carpet.png
        texture.podzol=./pine_needles
        texture.crimson_nylium=~/plants/roots.png
        texture.warped_nylium=minecraft:block/twisted_roots
        texture.grass_side=block/heather_side
        texture.dirt_path_side=trails:textures/block/chalk_bank
        texture.snow=block/frost
    ";
    writer
        .start_file(
            "assets/minecraft/optifine/bettergrass.properties",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    writer.write_all(properties).unwrap();
    for file in [
        "assets/minecraft/textures/block/heather.png",
        "assets/trails/textures/block/chalk_path.png",
        "assets/minecraft/textures/block/moss_carpet.png",
        "assets/minecraft/optifine/pine_needles.png",
        "assets/minecraft/optifine/plants/roots.png",
        "assets/minecraft/textures/block/twisted_roots.png",
        "assets/minecraft/textures/block/heather_side.png",
        "assets/trails/textures/block/chalk_bank.png",
        "assets/minecraft/textures/block/frost.png",
    ] {
        // Rules check presence; the renderer, not this loader, decodes the image.
        writer
            .start_file(file, zip::write::SimpleFileOptions::default())
            .unwrap();
    }
    writer.finish().unwrap();
    let rules = Rules::load(&ResourceSource::archive(&archive).unwrap()).unwrap();
    fs::remove_file(archive).unwrap();

    for (surface, dry, snowy, sprite) in [
        (Grass, false, true, "minecraft:block/heather"),
        (DirtPath, false, false, "trails:block/chalk_path"),
        (Mycelium, false, false, "minecraft:block/moss_carpet"),
        (
            Podzol,
            true,
            false,
            "assets/minecraft/optifine/pine_needles.png",
        ),
        (
            CrimsonNylium,
            false,
            false,
            "assets/minecraft/optifine/plants/roots.png",
        ),
        (WarpedNylium, false, false, "minecraft:block/twisted_roots"),
    ] {
        assert_eq!(rules.enabled(surface, false), dry, "{surface:?}");
        assert_eq!(rules.enabled(surface, true), snowy, "{surface:?}");
        assert_eq!(rules.texture(surface, false), sprite, "{surface:?}");
    }
    for surface in [Grass, Mycelium, Podzol] {
        assert_eq!(rules.texture(surface, true), "minecraft:block/frost");
    }
    assert!(rules.grass_multilayer());
    assert_eq!(rules.grass_side(), "minecraft:block/heather_side");
    assert_eq!(rules.dirt_path_side(), "trails:block/chalk_bank");
}

#[test]
fn better_grass_uses_winning_properties_and_falls_back_for_missing_textures() {
    use crate::better_grass::{Rules, Surface};
    use horizon_assets::resource_source::Resource;
    use std::collections::BTreeMap;

    const FILE: &str = "assets/minecraft/optifine/bettergrass.properties";
    assert!(Rules::load(&ResourceSource::default()).is_none());
    let empty = ResourceSource::indexed(BTreeMap::from([(
        FILE.to_owned(),
        Resource::bytes(Vec::new()),
    )]));
    let rules = Rules::load(&empty).unwrap();
    assert!(rules.enabled(Surface::Grass, false));
    assert!(rules.enabled(Surface::Grass, true));
    assert!(!rules.grass_multilayer());

    let low = BTreeMap::from([
        (FILE.to_owned(), Resource::bytes(b"grass=false\n".to_vec())),
        (
            "assets/trails/textures/block/needles.png".to_owned(),
            Resource::bytes(Vec::new()),
        ),
    ]);
    let underlay = ResourceSource::indexed(low.clone());
    let mut winners = low;
    winners.insert(
        FILE.to_owned(),
        Resource::bytes(
            b"podzol=false\ntexture.grass=block/absent\ntexture.grass_side=./../private\n\
              texture.podzol=trails:block/needles"
                .to_vec(),
        ),
    );
    let rules = Rules::load(&ResourceSource::layered(winners, underlay)).unwrap();
    assert!(rules.enabled(Surface::Grass, false));
    assert!(!rules.enabled(Surface::Podzol, false));
    assert_eq!(
        rules.texture(Surface::Grass, false),
        "minecraft:block/grass_block_top"
    );
    assert_eq!(rules.grass_side(), "minecraft:block/grass_block_side");
    assert_eq!(
        rules.texture(Surface::Podzol, false),
        "trails:block/needles"
    );
}
