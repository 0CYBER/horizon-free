//! Exercise the actual world-open and command-reload publication coordinator.
use super::*;
use mod_api::{
    DatapackField, DatapackFieldKind, DatapackRegistrySync, DatapackValue, Registrar,
    RegistrationContext,
};
struct RegistryMod;
#[mod_api::__subscriptions]
impl Hooks for RegistryMod {
    fn data_map_types(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::DataMapTypesEvent<'_>,
    ) {
        event.types.push(mod_api::game::DataMapType {
            id: "registry_test:weights".into(),
            registry: "registry_test:spell".into(),
        });
    }
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        r.declare_datapack_registry(
            "spell",
            DatapackRegistrySync::ServerOnly,
            vec![DatapackField::required(
                "power",
                DatapackFieldKind::Int { min: 0, max: 100 },
            )],
        )
        .unwrap();
    }
}
fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}
fn power(game: &crate::game::Game<'_>) -> DatapackValue {
    let set = game.mods.datapack_registries.as_ref().unwrap();
    set.get(
        &set.handle("registry_test:spell", "registry_test:fire")
            .unwrap(),
    )
    .unwrap()
    .get("power")
    .unwrap()
    .clone()
}
#[test]
#[ignore = "daily"]
fn datapack_game_open_reload_and_exhaustion_are_atomic() {
    let _publication = horizon_client::datapack_net::isolate_server_publication();
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    for (name, value) in [("base", 3), ("override", 7)] {
        let pack = root.join("datapacks").join(name);
        write(
            &pack.join("pack.mcmeta"),
            r#"{"pack":{"description":"fixture","min_format":94,"max_format":94}}"#,
        );
        write(
            &pack.join("data/registry_test/registry_test/spell/fire.json"),
            &format!(r#"{{"power":{value}}}"#),
        );
    }
    write(
        &root.join("datapacks.json"),
        r#"{"enabled":["vanilla","file/base","file/override"],"disabled":[]}"#,
    );
    let winning = root.join("datapacks/override/data/registry_test/registry_test/spell/fire.json");
    let tag = root.join("datapacks/base/data/registry_test/tags/block/publication.json");
    write(&tag, r#"{"values":["minecraft:stone"]}"#);
    write(
        &root.join("datapacks/base/data/registry_test/registry_test/spell/conditioned.json"),
        r#"{"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"missing_condition_fixture"}],"power":"invalid"}"#,
    );
    mod_platform::testing::with_mods(
        vec![ModEntry::new("registry_test", Box::new(RegistryMod))].into(),
        || {
            with_game(|game| {
                game.load_world(request(root)).unwrap();
                assert_eq!(power(game), DatapackValue::Int(7));
                assert_eq!(
                    game.mods
                        .datapack_registries
                        .as_ref()
                        .unwrap()
                        .entries("registry_test:spell")
                        .unwrap(),
                    ["registry_test:fire"]
                );
                let first = game
                    .mods
                    .datapack_registries
                    .as_ref()
                    .unwrap()
                    .handle("registry_test:spell", "registry_test:fire")
                    .unwrap();
                assert_eq!(
                    game.mods.datapack_registries.as_ref().unwrap().generation(),
                    1
                );
                write(&winning, "{bad");
                assert!(!game.run_command("reload"));
                assert_eq!(power(game), DatapackValue::Int(7));
                assert_eq!(
                    game.mods.datapack_registries.as_ref().unwrap().generation(),
                    1
                );
                write(&winning, r#"{"power":9}"#);
                assert!(game.run_command("reload"));
                assert_eq!(power(game), DatapackValue::Int(9));
                assert_eq!(
                    horizon_client::datapack_net::served(),
                    game.mods.datapack_registries
                );
                assert_eq!(
                    game.mods.datapack_registries.as_ref().unwrap().generation(),
                    2
                );
                assert!(game.run_command("reload"));
                assert_eq!(
                    game.mods.datapack_registries.as_ref().unwrap().generation(),
                    3
                );
                let features = game.level.enabled_features.clone();
                game.reload_datapacks(true, &features).unwrap();
                assert_eq!(
                    game.mods.datapack_registries.as_ref().unwrap().generation(),
                    4
                );
                assert!(game.run_command("datapack disable \"file/override\""));
                assert_eq!(power(game), DatapackValue::Int(3));
                assert_eq!(
                    game.mods.datapack_registries.as_ref().unwrap().generation(),
                    5
                );
                assert!(game.run_command("datapack enable \"file/override\" last"));
                assert_eq!(power(game), DatapackValue::Int(9));
                assert_eq!(
                    game.mods.datapack_registries.as_ref().unwrap().generation(),
                    6
                );
                game.reload_datapacks(false, &features).unwrap();
                assert_eq!(
                    game.mods.datapack_registries.as_ref().unwrap().generation(),
                    6
                );
                std::sync::Arc::make_mut(game.mods.datapack_registries.as_mut().unwrap())
                    .exhaust_generation_for_testing();
                let old_set = game.mods.datapack_registries.clone();
                let old_files = game
                    .world_tables
                    .as_ref()
                    .map(|tables| tables.files.clone());
                write(&winning, r#"{"power":11}"#);
                write(&tag, r#"{"values":["minecraft:dirt"]}"#);
                assert!(
                    !game.run_command("reload"),
                    "generation exhaustion must refuse the whole publication"
                );
                assert_eq!(game.mods.datapack_registries, old_set);
                assert_eq!(
                    horizon_client::datapack_net::served().unwrap().generation(),
                    6
                );
                assert_eq!(
                    game.world_tables
                        .as_ref()
                        .map(|tables| tables.files.clone()),
                    old_files
                );
                game.finish_close_world().unwrap();
                game.options.execution_role =
                    crate::game::execution_role::ExecutionRole::DedicatedServer;
                game.load_world(request(root)).unwrap();
                assert_eq!(
                    game.mods.datapack_registries.as_ref().unwrap().get(&first),
                    Err(mod_api::DatapackReadError::WrongSession)
                );
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn remote_reload_never_decodes_local_registry_files() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    write(
        &root.join("datapacks/local/pack.mcmeta"),
        r#"{"pack":{"description":"remote poison"}}"#,
    );
    write(
        &root.join("datapacks/local/data/registry_test/registry_test/spell/fire.json"),
        "{malformed",
    );
    write(
        &root.join("datapacks.json"),
        r#"{"enabled":["file/local"],"disabled":[]}"#,
    );
    mod_platform::testing::with_mods(
        vec![ModEntry::new("registry_test", Box::new(RegistryMod))].into(),
        || {
            crate::game::native_entity_refs::tests::with_client_game(|game| {
                let (pipe, _server) = horizon_protocol::connection::Pipe::pair();
                game.connect_local(pipe, "DatapackRemote").unwrap();
                game.world_root = root.to_owned();
                game.reload_datapacks(true, &[]).unwrap();
                assert!(game.mods.datapack_registries.is_none());
            });
        },
    );
}
/// A reload over a decode limit, or with a bad winning file, is refused naming the pack and
/// file; the old generation stays live and the next valid reload publishes.
#[test]
#[ignore = "daily"]
fn refused_reload_names_the_pack_and_keeps_the_old_generation() {
    let _publication = horizon_client::datapack_net::isolate_server_publication();
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let pack = root.join("datapacks/override");
    write(
        &pack.join("pack.mcmeta"),
        r#"{"pack":{"description":"fixture","min_format":94,"max_format":94}}"#,
    );
    let winning = pack.join("data/registry_test/registry_test/spell/fire.json");
    write(&winning, r#"{"power":7}"#);
    write(
        &root.join("datapacks.json"),
        r#"{"enabled":["vanilla","file/override"],"disabled":[]}"#,
    );
    mod_platform::testing::with_mods(
        vec![ModEntry::new("registry_test", Box::new(RegistryMod))].into(),
        || {
            with_game(|game| {
                game.load_world(request(root)).unwrap();
                let features = game.level.enabled_features.clone();
                let generation = |game: &crate::game::Game<'_>| {
                    game.mods.datapack_registries.as_ref().unwrap().generation()
                };
                assert_eq!(generation(game), 1);
                let over = format!(
                    r#"{{"power":7{}}}"#,
                    " ".repeat(mod_api::MAX_DATAPACK_ENTRY_BYTES)
                );
                for (text, reason) in [(over.as_str(), "EntryTooLarge"), ("{bad", "Malformed")] {
                    write(&winning, text);
                    let Err(mod_api::__private::reload::ReloadStageError::DataSource(message)) =
                        game.reload_datapacks(true, &features)
                    else {
                        panic!("the reload is refused");
                    };
                    assert!(
                        message.contains(
                            "pack file/override, file data/registry_test/registry_test/spell/fire.json"
                        ) && message.contains(reason),
                        "{message}"
                    );
                    assert_eq!(power(game), DatapackValue::Int(7));
                    assert_eq!(generation(game), 1);
                    assert_eq!(
                        horizon_client::datapack_net::served(),
                        game.mods.datapack_registries
                    );
                }
                write(&winning, r#"{"power":9}"#);
                assert!(game.run_command("reload"));
                assert_eq!(power(game), DatapackValue::Int(9));
                assert_eq!(generation(game), 2);
            });
        },
    );
}
#[path = "datapack_host_read_tests.rs"]
mod datapack_host_read_tests;

#[path = "datapack_game_delivery_tests.rs"]
mod datapack_game_delivery_tests;

// Each case uses real world open/reload, not a preloaded membership predicate.
fn with_membership_fixture(run: impl FnOnce(&mut crate::game::Game<'_>, &Path)) {
    let _publication = horizon_client::datapack_net::isolate_server_publication();
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    for name in ["base", "later"] {
        write(
            &root.join(format!("datapacks/{name}/pack.mcmeta")),
            r#"{"pack":{"description":"fixture","min_format":94,"max_format":94}}"#,
        );
    }
    write(
        &root.join("datapacks/base/data/registry_test/registry_test/spell/fire.json"),
        r#"{"power":3}"#,
    );
    write(
        &root.join("datapacks/later/data/registry_test/registry_test/spell/ice.json"),
        r#"{"power":7}"#,
    );
    write(
        &root.join("datapacks/base/data/registry_test/data_maps/registry_test/spell/weights.json"),
        r#"{"values":{"registry_test:fire":11,"registry_test:ice":17,"registry_test:ghost":99}}"#,
    );
    write(
        &root.join("datapacks.json"),
        r#"{"enabled":["vanilla","file/base"],"disabled":["file/later"]}"#,
    );
    mod_platform::testing::with_mods(
        vec![ModEntry::new("registry_test", Box::new(RegistryMod))].into(),
        || {
            with_game(|game| {
                game.load_world(request(root)).unwrap();
                run(game, root);
            })
        },
    );
}
fn weights(game: &crate::game::Game<'_>) -> Vec<(String, serde_json::Value)> {
    game.world_tables
        .as_ref()
        .unwrap()
        .data_maps
        .json_values("registry_test:spell", "registry_test:weights")
        .map(|(id, value)| (id.to_owned(), value.clone()))
        .collect()
}
#[test]
#[ignore = "daily"]
fn data_map_first_open_uses_staged_membership() {
    with_membership_fixture(|game, _| {
        assert_eq!(
            weights(game),
            vec![("registry_test:fire".into(), serde_json::json!(11))]
        );
    });
}
#[test]
#[ignore = "daily"]
fn data_map_reload_add_uses_staged_membership() {
    with_membership_fixture(|game, root| {
        write(
            &root.join("datapacks/base/data/registry_test/registry_test/spell/ice.json"),
            r#"{"power":5}"#,
        );
        assert!(game.run_command("reload"));
        assert_eq!(
            weights(game),
            vec![
                ("registry_test:fire".into(), serde_json::json!(11)),
                ("registry_test:ice".into(), serde_json::json!(17))
            ]
        );
    });
}
#[test]
#[ignore = "daily"]
fn data_map_reload_remove_uses_staged_membership() {
    with_membership_fixture(|game, root| {
        std::fs::remove_file(
            root.join("datapacks/base/data/registry_test/registry_test/spell/fire.json"),
        )
        .unwrap();
        assert!(game.run_command("reload"));
        assert_eq!(weights(game), vec![]);
    });
}
#[test]
#[ignore = "daily"]
fn data_map_later_enabled_layer_uses_staged_membership() {
    with_membership_fixture(|game, _| {
        assert!(game.run_command("datapack enable \"file/later\" last"));
        assert_eq!(
            weights(game),
            vec![
                ("registry_test:fire".into(), serde_json::json!(11)),
                ("registry_test:ice".into(), serde_json::json!(17))
            ]
        );
    });
}
