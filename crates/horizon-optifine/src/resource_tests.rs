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
