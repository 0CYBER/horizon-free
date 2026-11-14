#![cfg(test)]
use mod_api::{DataMap, DataMapError, Tag};
struct VillagerTypes;
impl DataMap for VillagerTypes {
    type Entry = mod_api::BiomeKey;
    type Value = Tag;
    fn id(&self) -> &str {
        "neoforge:villager_types"
    }
    fn decode(&self, value: &Tag) -> Option<Tag> {
        Some(value.clone())
    }
}
#[test]
#[ignore = "daily"]
fn biome_key_world_install_reload_and_unavailable_sides() {
    mod_platform::testing::with_mods(Default::default(), || {
        crate::game::native_entity_refs::tests::with_game(|game| {
            let root = tempfile::tempdir().unwrap();
            game.world_root = root.path().to_owned();
            game.options.dedicated = Some(Vec::new());
            assert_eq!(game.biome_key("minecraft:plains"), None);
            let pack = root.path().join("datapacks/biomes");
            let file = pack.join("data/neoforge/data_maps/worldgen/biome/villager_types.json");
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(
                pack.join("pack.mcmeta"),
                r#"{"pack":{"description":"Biome fixture"}}"#,
            )
            .unwrap();
            std::fs::write(
                root.path().join("datapacks.json"),
                r#"{"enabled":["file/biomes"],"disabled":[]}"#,
            )
            .unwrap();
            let mut previous: Option<mod_api::BiomeKey> = None;
            for value in ["minecraft:plains", "minecraft:savanna"] {
                std::fs::write(&file, format!(r#"{{"replace":true,"values":{{"minecraft:plains":{{"villager_type":"{value}"}}}}}}"#)).unwrap();
                let packs = game.reload_datapacks(false, &[]).unwrap();
                let resources = game.options.resources.clone();
                game.world_tables = game
                    .stage_world_tables(&packs, previous.is_some(), &resources)
                    .unwrap()
                    .0;
                let plains = game.biome_key("minecraft:plains").unwrap();
                assert_eq!(plains.id(game), Ok("minecraft:plains".into()));
                assert_eq!(game.biome_key("test:nope"), None);
                assert_eq!(
                    plains.data(game, &VillagerTypes),
                    Ok(Some(Tag::Compound(std::collections::BTreeMap::from([(
                        "villager_type".into(),
                        Tag::String(value.into())
                    )]))))
                );
                assert_eq!(
                    game.biome_key("minecraft:desert")
                        .unwrap()
                        .data(game, &VillagerTypes),
                    Ok(None)
                );
                if let Some(old) = previous {
                    assert_eq!(old.id(game), Err(mod_api::BiomeKeyError::WrongGeneration));
                    assert_eq!(
                        old.data(game, &VillagerTypes),
                        Err(DataMapError::InvalidKey)
                    );
                }
                previous = Some(plains);
            }
            let key = previous.unwrap();
            game.options.dedicated = None;
            assert_eq!(game.biome_key("minecraft:plains"), None);
            assert_eq!(
                key.data(game, &VillagerTypes),
                Err(DataMapError::Unavailable)
            );
            game.options.dedicated = Some(Vec::new());
            game.world_tables = None;
            assert_eq!(
                key.data(game, &VillagerTypes),
                Err(DataMapError::Unavailable)
            );
        });
    });
}
