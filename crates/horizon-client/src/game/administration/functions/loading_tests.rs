use super::*;
use serde_json::json;

#[test]
#[ignore = "daily"]
fn failed_pack_selection_keeps_the_live_library_and_saved_selection() {
    crate::game::native_entity_refs::tests::with_game(|game| {
        let dir = datapack::temp_folder().unwrap();
        let write_pack = |name: &str, predicate: serde_json::Value| {
            let pack = dir.join("datapacks").join(name);
            std::fs::create_dir_all(pack.join("data/prg/predicate")).unwrap();
            let metadata = json!({
                "pack": {
                    "description": "registry admission test",
                    "min_format": [121, 0],
                    "max_format": [121, 0]
                }
            });
            std::fs::write(pack.join("pack.mcmeta"), metadata.to_string()).unwrap();
            std::fs::write(
                pack.join(format!("data/prg/predicate/{name}.json")),
                predicate.to_string(),
            )
            .unwrap();
        };
        write_pack("base", json!({"type":"all_of", "terms":[]}));
        write_pack("extra", json!({"type":"inverted", "term":"prg:missing"}));
        let settings =
            json!({"enabled":["vanilla", "file/base"], "disabled":["file/extra"]}).to_string();
        let settings_path = dir.join("datapacks.json");
        std::fs::write(&settings_path, &settings).unwrap();
        game.world_root = dir.clone();
        game.reload_datapacks(false, &game.level.enabled_features.clone())
            .unwrap();
        game.build_registries();
        game.set_allow_commands(true);
        let scheduled = Scheduled {
            id: "prg:later".into(),
            time: 100,
            seq: 1,
        };
        game.fns.schedule.push(scheduled.clone());

        assert!(!game.run_command("datapack enable \"file/extra\""));
        assert_eq!(game.fns.enabled, ["file/base"]);
        assert_eq!(std::fs::read_to_string(&settings_path).unwrap(), settings);
        assert!(game.fns.store.predicates.contains_key("prg:base"));
        assert!(!game.fns.store.predicates.contains_key("prg:extra"));
        assert_eq!(
            game.fns.schedule.as_slice(),
            std::slice::from_ref(&scheduled)
        );

        // Repair the available pack. Enabling now commits both its library and selected id.
        write_pack("extra", json!({"type":"all_of", "terms":["prg:base"]}));
        assert!(game.run_command("datapack enable \"file/extra\""));
        assert_eq!(game.fns.enabled, ["file/base", "file/extra"]);
        assert!(game.fns.store.predicates.contains_key("prg:extra"));
        let committed = std::fs::read_to_string(&settings_path).unwrap();
        assert_ne!(committed, settings);

        // Disabling the provider would break the newly admitted dependent pack, so retain both.
        assert!(!game.run_command("datapack disable \"file/base\""));
        assert_eq!(game.fns.enabled, ["file/base", "file/extra"]);
        assert_eq!(std::fs::read_to_string(&settings_path).unwrap(), committed);
        assert!(game.fns.store.predicates.contains_key("prg:base"));
        assert_eq!(game.fns.schedule, [scheduled]);
        std::fs::remove_dir_all(dir).unwrap();
    });
}
