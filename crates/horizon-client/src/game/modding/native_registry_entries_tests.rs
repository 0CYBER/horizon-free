//! Real server/client Game owners, world publication and negotiated client registry limits.
use super::native_entity_refs::tests::{with_client_game, with_game};
use mod_api::{
    DatapackField, DatapackFieldKind, DatapackRegistrySync, Hooks, ModEntry, Registrar,
    RegistrationContext,
};
use std::{path::Path, sync::Arc};

struct RegistryMod;
#[mod_api::__subscriptions]
impl Hooks for RegistryMod {
    fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
        for (registry, sync) in [
            ("spell", DatapackRegistrySync::Required),
            ("secret", DatapackRegistrySync::ServerOnly),
        ] {
            registrar
                .declare_datapack_registry(
                    registry,
                    sync,
                    vec![DatapackField::required(
                        "power",
                        DatapackFieldKind::Int { min: 0, max: 100 },
                    )],
                )
                .unwrap();
        }
    }
}
fn write(path: &Path, bytes: impl AsRef<[u8]>) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}
fn request(root: &Path) -> super::world_loading::WorldRequest {
    let mut request = super::world_loading::WorldRequest::new(
        "generic registries",
        Some("generic registries"),
        Some(1),
        true,
        None,
        Arc::new(horizon_client::gamerules::RuleRegistry::vanilla()),
    )
    .unwrap();
    request.save_root = root.to_owned();
    request
}
fn host<'a>(game: &'a super::Game<'_>) -> &'a dyn mod_api::Host {
    game
}
fn id(value: &str) -> mod_api::Identifier {
    mod_api::Identifier::parse(value).unwrap()
}

#[test]
#[ignore = "needs prepared resources and isolated homes"]
fn generic_entries_follow_real_server_tables_and_atomic_reload() {
    let _publication = horizon_client::datapack_net::isolate_server_publication();
    let scratch = tempfile::tempdir().unwrap();
    let pack = scratch.path().join("datapacks/entries");
    write(
        &pack.join("pack.mcmeta"),
        r#"{"pack":{"description":"fixture","min_format":94,"max_format":94}}"#,
    );
    write(
        &scratch.path().join("datapacks.json"),
        r#"{"enabled":["vanilla","file/entries"],"disabled":[]}"#,
    );
    let fire = pack.join("data/registry_entry_test/registry_entry_test/spell/fire.json");
    let tag = pack.join("data/registry_entry_test/tags/registry_entry_test/spell/hot.json");
    write(&fire, r#"{"power":7}"#);
    write(&tag, r#"{"values":["registry_entry_test:fire"]}"#);
    write(
        &pack.join("data/registry_entry_test/registry_entry_test/secret/shadow.json"),
        r#"{"power":3}"#,
    );
    write(
        &pack.join("data/registry_entry_test/registry_entry_test/spell/conditioned.json"),
        r#"{"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"absent_registry_fixture"}],"power":"not an integer"}"#,
    );
    write(
        &pack.join("data/registry_entry_test/not_a_registry/fake.json"),
        "{}",
    );
    write(
        &pack.join("data/registry_entry_test/worldgen/noise_settings/broken.json"),
        "{}",
    );
    mod_platform::testing::with_mods(
        vec![ModEntry::new("registry_entry_test", Box::new(RegistryMod))].into(),
        || {
            with_game(|game| {
                game.load_world(request(scratch.path())).unwrap();
                let step = host(game)
                    .registry_entry("minecraft:game_event", "minecraft:step")
                    .unwrap();
                assert_eq!(
                    host(game).registry_entry_in_tag(&step, "minecraft:vibrations"),
                    Some(true)
                );
                assert_eq!(
                    host(game).registry_entry_in_tag(&step, "registry_entry_test:absent"),
                    Some(false)
                );
                assert_eq!(
                    host(game).registry_entry("minecraft:game_event", "minecraft:nope"),
                    None
                );
                assert_eq!(
                    host(game).registry_entry("minecraft:nope", "minecraft:step"),
                    None
                );
                assert_eq!(
                    host(game).registry_entries("registry_entry_test:not_a_registry"),
                    None
                );
                assert_eq!(
                    host(game).registry_entries("minecraft:biome"),
                    None,
                    "no synthetic registry aliases"
                );
                assert_eq!(
                    host(game).registry_entry("minecraft:worldgen/noise_settings", "registry_entry_test:broken"),
                    None,
                    "an unused resource is not an accepted noise-settings definition",
                );
                assert!(!host(game).registry_entries("minecraft:worldgen/noise_settings")
                    .unwrap().contains(&id("registry_entry_test:broken")));
                let stone = host(game)
                    .registry_entry("minecraft:block", "minecraft:stone")
                    .unwrap();
                assert_eq!(
                    stone.as_block_key(host(game)),
                    game.block_key("minecraft:stone")
                );
                assert_eq!(stone.as_item_key(host(game)), None);
                for (registry, entry) in [
                    ("minecraft:damage_type", "minecraft:fall"),
                    ("minecraft:villager_type", "minecraft:plains"),
                    ("minecraft:painting_variant", "minecraft:alban"),
                    ("minecraft:instrument", "minecraft:ponder_goat_horn"),
                    ("minecraft:worldgen/noise", "minecraft:aquifer_barrier"),
                    ("minecraft:trade_set", "minecraft:armorer/level_1"),
                    (
                        "minecraft:context_int_provider",
                        "minecraft:cooking/normal_burn_time_reduction_factor",
                    ),
                ] {
                    assert!(
                        host(game).registry_entry(registry, entry).is_some(),
                        "{registry} / {entry}"
                    );
                    assert!(
                        host(game)
                            .registry_entries(registry)
                            .unwrap()
                            .contains(&id(entry)),
                        "{registry}"
                    );
                }
                assert_eq!(
                    host(game).registry_entries("registry_entry_test:spell"),
                    Some(vec![id("registry_entry_test:fire")])
                );
                let held = host(game)
                    .registry_entry("registry_entry_test:spell", "registry_entry_test:fire")
                    .unwrap();
                assert_eq!(
                    host(game).registry_entry_in_tag(&held, "registry_entry_test:hot"),
                    Some(true)
                );
                assert!(
                    host(game)
                        .registry_entry("registry_entry_test:secret", "registry_entry_test:shadow")
                        .is_some()
                );
                write(&fire, "{bad json");
                assert!(!game.run_command("reload"));
                assert_eq!(
                    host(game).registry_entry_in_tag(&held, "registry_entry_test:hot"),
                    Some(true),
                    "failed candidate keeps the published entries and tags"
                );
                std::fs::remove_file(&fire).unwrap();
                write(
                    &pack.join("data/addon/registry_entry_test/spell/ice.json"),
                    r#"{"power":4}"#,
                );
                write(&tag, r#"{"replace":true,"values":["addon:ice"]}"#);
                write(
                    &pack.join("data/registry_entry_test/damage_type/late.json"),
                    r#"{"message_id":"fall","scaling":"never","exhaustion":0.0}"#,
                );
                write(
                    &pack.join("data/registry_entry_test/worldgen/noise/late.json"),
                    r#"{"base_octave":-3,"octave_count":1}"#,
                );
                assert!(game.run_command("reload"));
                assert_eq!(
                    host(game)
                        .registry_entry("registry_entry_test:spell", "registry_entry_test:fire"),
                    None
                );
                assert_eq!(
                    host(game).registry_entry_in_tag(&held, "registry_entry_test:hot"),
                    None,
                    "plain values are re-resolved after reload"
                );
                let ice = host(game)
                    .registry_entry("registry_entry_test:spell", "addon:ice")
                    .unwrap();
                assert_eq!(
                    host(game).registry_entry_in_tag(&ice, "registry_entry_test:hot"),
                    Some(true)
                );
                assert_eq!(
                    host(game).registry_entries("registry_entry_test:spell"),
                    Some(vec![id("addon:ice")])
                );
                assert_eq!(
                    host(game).registry_entry("minecraft:damage_type", "registry_entry_test:late"),
                    None,
                    "frozen registry is not a raw reload-file inventory"
                );
                assert_eq!(
                    host(game)
                        .registry_entry("minecraft:worldgen/noise", "registry_entry_test:late"),
                    None
                );
            });
        },
    );
}

#[test]
#[ignore = "needs prepared resources and isolated homes"]
fn generic_client_entries_use_only_builtin_synced_and_negotiated_tables() {
    let _publication = horizon_client::datapack_net::isolate_server_publication();
    mod_platform::testing::with_mods(
        vec![ModEntry::new("registry_entry_test", Box::new(RegistryMod))].into(),
        || {
            with_client_game(|game| {
                use horizon_protocol::{
                    connection::Pipe,
                    packets::configuration::{RegistryTags, UpdateTags},
                };
                assert_eq!(
                    host(game).registry_entry("minecraft:game_event", "minecraft:step"),
                    None
                );
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "Generic registries").unwrap();
                assert_eq!(
                    host(game).registry_entry("minecraft:damage_type", "minecraft:fall"),
                    None,
                    "not synchronized yet"
                );
                assert_eq!(
                    host(game).registry_entries("minecraft:worldgen/noise"),
                    None,
                    "prepared server-only data must not leak"
                );
                for registry in [
                    "minecraft:worldgen/feature",
                    "minecraft:worldgen/placed_feature",
                    "minecraft:worldgen/block_state_provider",
                ] {
                    assert_eq!(host(game).registry_entries(registry), None);
                    assert_eq!(host(game).registry_entry(registry, "minecraft:oak"), None);
                }
                let sync = super::lan::sync_data(
                    &game.options.resources,
                    &game.res.core.item_data,
                    &game.pack_files(),
                )
                .unwrap();
                let mut received = sync.dynamic.clone();
                received.add_tags(&sync.tags);
                game.remote.as_mut().unwrap().registries = Arc::new(received);
                let step = host(game)
                    .registry_entry("minecraft:game_event", "minecraft:step")
                    .unwrap();
                assert_eq!(
                    host(game).registry_entry_in_tag(&step, "minecraft:vibrations"),
                    Some(true)
                );
                let fall = host(game)
                    .registry_entry("minecraft:damage_type", "minecraft:fall")
                    .unwrap();
                assert!(
                    host(game)
                        .registry_entries("minecraft:damage_type")
                        .unwrap()
                        .contains(&fall.id)
                );
                let stone = host(game)
                    .registry_entry("minecraft:block", "minecraft:stone")
                    .unwrap();
                assert_eq!(
                    stone.as_block_key(host(game)),
                    game.block_key("minecraft:stone")
                );
                assert!(
                    host(game)
                        .registry_entry("minecraft:painting_variant", "minecraft:alban")
                        .is_some()
                );
                assert_eq!(
                    host(game).registry_entry("minecraft:game_event", "minecraft:nope"),
                    None
                );
                assert_eq!(
                    host(game).registry_entries("minecraft:context_int_provider"),
                    None
                );
                let declarations = mod_api::__private::datapack_registry_declarations();
                let mut candidate = mod_api::DatapackCandidate::build(
                    declarations,
                    &[
                        (
                            "data/addon/registry_entry_test/spell/remote.json".into(),
                            vec![("fixture".into(), br#"{"power":8}"#.to_vec())],
                        ),
                        (
                            "data/addon/registry_entry_test/secret/hidden.json".into(),
                            vec![("fixture".into(), br#"{"power":9}"#.to_vec())],
                        ),
                    ],
                )
                .unwrap();
                candidate
                    .bind_tags(
                        "registry_entry_test:spell",
                        [("addon:hot".into(), vec!["addon:remote".into()])].into(),
                    )
                    .unwrap();
                let server_set = mod_api::DatapackRegistrySet::first(candidate, 900);
                // A same-process server snapshot is deliberately present on the client Game too.
                game.mods.datapack_registries = Some(Arc::new(server_set.clone()));
                horizon_client::datapack_net::serve(Some(Arc::new(server_set)));
                let offer = horizon_client::datapack_net::offer().unwrap();
                let horizon_client::datapack_net::Step::Send(bytes) =
                    horizon_client::datapack_net::server_step(true, Some(&offer))
                else {
                    panic!("expected negotiated snapshot");
                };
                horizon_client::datapack_net::accept(&bytes).unwrap();
                let remote = host(game)
                    .registry_entry("registry_entry_test:spell", "addon:remote")
                    .unwrap();
                assert_eq!(
                    host(game).registry_entry_in_tag(&remote, "addon:hot"),
                    Some(true)
                );
                assert_eq!(
                    host(game).registry_entries("registry_entry_test:secret"),
                    None
                );
                assert_eq!(
                    host(game).registry_entry("registry_entry_test:secret", "addon:hidden"),
                    None
                );
                let mut replacement = game.remote.as_ref().unwrap().registries.as_ref().clone();
                replacement.registries.remove("minecraft:damage_type");
                replacement.add_tags(&UpdateTags {
                    registries: vec![RegistryTags {
                        registry: "minecraft:game_event".into(),
                        tags: vec![],
                    }],
                });
                game.remote.as_mut().unwrap().registries = Arc::new(replacement);
                assert_eq!(
                    host(game).registry_entry_in_tag(&fall, "minecraft:is_fall"),
                    None
                );
                assert_eq!(
                    host(game).registry_entry_in_tag(&step, "minecraft:vibrations"),
                    Some(false)
                );
                horizon_client::datapack_net::clear_received();
                assert_eq!(host(game).registry_entry_in_tag(&remote, "addon:hot"), None);
            });
        },
    );
}

#[test]
#[ignore = "needs prepared resources and isolated homes"]
fn generic_worldgen_entries_use_published_decoded_owner_without_terrain_locator() {
    use horizon_world::worldgen::{
        generator::GeneratorData, live::LiveFeatures, overrides::WorldgenOverrides,
    };
    let _publication = horizon_client::datapack_net::isolate_server_publication();
    let scratch = tempfile::tempdir().unwrap();
    with_game(|game| {
        game.load_world(request(scratch.path())).unwrap();
        let mut overrides = WorldgenOverrides::default();
        overrides.set("features", "registry_entry_test:accepted",
            r#"{"type":"minecraft:no_op","config":{}}"#.into());
        overrides.set("features", "registry_entry_test:rejected",
            r#"{"type":"registry_entry_test:unknown","config":{}}"#.into());
        // FeatureCatalog also uses this tag bundle for placed-holder expansion.
        overrides.set("placed_features", "registry_entry_test:accepted",
            r#"{"feature":"registry_entry_test:accepted","placement":[]}"#.into());
        overrides.set("placed_features", "registry_entry_test:rejected",
            r#"{"feature":"registry_entry_test:rejected","placement":[]}"#.into());
        overrides.set("state_providers", "registry_entry_test:accepted",
            r#"{"type":"minecraft:simple","state":{"id":"minecraft:stone"}}"#.into());
        overrides.set("state_providers", "registry_entry_test:rejected",
            r#"{"type":"registry_entry_test:unknown"}"#.into());
        overrides.set("feature_tags", "registry_entry_test:accepted",
            r#"{"values":["registry_entry_test:accepted"]}"#.into());
        overrides.set("carvers", "registry_entry_test:accepted",
            r#"{"type":"minecraft:cave","probability":1.0,"y":{"absolute":0},"count":1,"thickness":1.0,"room_vertical_radius_multiplier":1.0,"horizontal_radius_multiplier":1.0,"vertical_radius_multiplier":1.0,"floor_level":-0.7}"#.into());
        overrides.set("conditions", "registry_entry_test:accepted",
            r#"{"type":"minecraft:steep"}"#.into());
        let data = GeneratorData::load_with(
            &game.options.resources.join("worldgen"),
            game.pack.assets.registry,
            &overrides,
        ).unwrap();
        let features = Arc::new(LiveFeatures::new(&data).unwrap().unwrap());
        assert!(features.configured("registry_entry_test:accepted").is_some());
        assert!(features.configured("registry_entry_test:rejected").is_none());
        assert!(features.placed("registry_entry_test:accepted").is_some());
        for registry in [
            "minecraft:worldgen/feature",
            "minecraft:worldgen/placed_feature",
            "minecraft:worldgen/block_state_provider",
        ] {
            assert_eq!(features.registry_contains(registry, "registry_entry_test:accepted"),
                Some(true), "{registry}: fixture must pass the live owner's canonical decoder");
        }
        for registry in ["minecraft:worldgen/carver", "minecraft:worldgen/material_condition"] {
            assert_eq!(data.registry_contains(registry, "registry_entry_test:accepted"),
                Some(true), "{registry}: fixture must belong to the decoded generator owner");
        }
        game.res.behaviours().set_live_features(features);
        game.end.set_locator(None);
        assert!(game.end.locator().is_none());
        for registry in [
            "minecraft:worldgen/feature",
            "minecraft:worldgen/placed_feature",
            "minecraft:worldgen/block_state_provider",
            "minecraft:worldgen/carver",
            "minecraft:worldgen/material_condition",
        ] {
            let accepted = host(game).registry_entry(registry, "registry_entry_test:accepted")
                .unwrap_or_else(|| panic!("{registry}: decoded entry must not depend on a terrain locator"));
            let entries = host(game).registry_entries(registry)
                .unwrap_or_else(|| panic!("{registry}: published decoded registry must be available"));
            assert!(entries.contains(&id("registry_entry_test:accepted")), "{registry}");
            assert!(!entries.contains(&id("registry_entry_test:rejected")), "{registry}");
            assert_eq!(host(game).registry_entry(registry, "registry_entry_test:rejected"), None);
            assert_eq!(host(game).registry_entry_in_tag(&accepted, "registry_entry_test:missing"),
                Some(false));
            if matches!(registry, "minecraft:worldgen/feature" | "minecraft:worldgen/placed_feature") {
                assert_eq!(host(game).registry_entry_in_tag(&accepted, "registry_entry_test:accepted"),
                    Some(true));
            }
        }
        // Replacing the published owner must retire its entries even while the old Arc lives.
        let mut replacement = WorldgenOverrides::default();
        replacement.set("conditions", "registry_entry_test:replacement",
            r#"{"type":"minecraft:steep"}"#.into());
        let replacement = GeneratorData::load_with(
            &game.options.resources.join("worldgen"),
            game.pack.assets.registry,
            &replacement,
        ).unwrap();
        game.res.behaviours().set_live_features(
            Arc::new(LiveFeatures::new(&replacement).unwrap().unwrap()));
        for registry in [
            "minecraft:worldgen/feature",
            "minecraft:worldgen/placed_feature",
            "minecraft:worldgen/block_state_provider",
            "minecraft:worldgen/carver",
            "minecraft:worldgen/material_condition",
        ] {
            assert_eq!(host(game).registry_entry(registry, "registry_entry_test:accepted"), None);
            assert!(!host(game).registry_entries(registry).unwrap()
                .contains(&id("registry_entry_test:accepted")), "{registry}");
            let retired = mod_api::RegistryEntry::new(id(registry), id("registry_entry_test:accepted"));
            assert_eq!(host(game).registry_entry_in_tag(&retired, "registry_entry_test:accepted"), None);
        }
        let replacement_entry = host(game).registry_entry(
            "minecraft:worldgen/material_condition", "registry_entry_test:replacement").unwrap();
        assert!(host(game).registry_entries("minecraft:worldgen/material_condition").unwrap()
            .contains(&id("registry_entry_test:replacement")));
        assert_eq!(host(game).registry_entry_in_tag(&replacement_entry, "registry_entry_test:missing"),
            Some(false));
    });
}
