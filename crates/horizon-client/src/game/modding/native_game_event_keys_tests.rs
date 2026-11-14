#![cfg(test)]
use mod_api::{DataMap, DataMapError, Tag};
struct Frequencies;
impl DataMap for Frequencies {
    type Entry = mod_api::GameEventKey;
    type Value = Tag;
    fn id(&self) -> &str {
        "neoforge:vibration_frequencies"
    }
    fn decode(&self, value: &Tag) -> Option<Tag> {
        Some(value.clone())
    }
}
#[test]
#[ignore = "daily"]
fn game_event_key_registry_data_reload_and_unavailable_sides() {
    mod_platform::testing::with_mods(Default::default(), || {
        crate::game::native_entity_refs::tests::with_game(|game| {
            let root = tempfile::tempdir().unwrap();
            game.world_root = root.path().to_owned();
            game.options.dedicated = Some(Vec::new());
            let step = game.game_event_key("minecraft:step").unwrap();
            assert_eq!(step.id(game), Ok("minecraft:step".into()));
            assert_eq!(game.game_event_key("test:nope"), None);
            assert_eq!(
                step.data(game, &Frequencies),
                Err(DataMapError::Unavailable)
            );
            let pack = root.path().join("datapacks/events");
            let file = pack.join("data/neoforge/data_maps/game_event/vibration_frequencies.json");
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(
                pack.join("pack.mcmeta"),
                r#"{"pack":{"description":"Game event fixture"}}"#,
            )
            .unwrap();
            std::fs::write(
                root.path().join("datapacks.json"),
                r#"{"enabled":["file/events"],"disabled":[]}"#,
            )
            .unwrap();
            for frequency in [7, 12] {
                std::fs::write(&file, format!(r#"{{"replace":true,"values":{{"minecraft:step":{{"frequency":{frequency}}}}}}}"#)).unwrap();
                let packs = game.reload_datapacks(false, &[]).unwrap();
                let resources = game.options.resources.clone();
                game.world_tables = game
                    .stage_world_tables(&packs, frequency == 12, &resources)
                    .unwrap()
                    .0;
                assert_eq!(game.game_event_key("minecraft:step"), Some(step));
                assert_eq!(
                    step.data(game, &Frequencies),
                    Ok(Some(Tag::Compound(std::collections::BTreeMap::from([(
                        "frequency".into(),
                        Tag::Int(frequency)
                    )]))))
                );
                assert_eq!(
                    game.game_event_key("minecraft:swim")
                        .unwrap()
                        .data(game, &Frequencies),
                    Ok(None)
                );
            }
            game.options.dedicated = None;
            assert_eq!(
                step.data(game, &Frequencies),
                Err(DataMapError::Unavailable)
            );
            game.options.dedicated = Some(Vec::new());
            game.world_tables = None;
            assert_eq!(
                step.data(game, &Frequencies),
                Err(DataMapError::Unavailable)
            );
        });
    });
}
