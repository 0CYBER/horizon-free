use super::*;
use mod_api::{
    DatapackField, DatapackFieldKind, DatapackReadError as E, DatapackRegistrySync, DimensionKey,
    LevelRef, Registrar, RegistrationContext, WorldEpoch,
};
struct SyncedRegistryMod;
#[mod_api::__subscriptions]
impl Hooks for SyncedRegistryMod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        r.declare_datapack_registry(
            "spell",
            DatapackRegistrySync::Required,
            vec![DatapackField::required(
                "power",
                DatapackFieldKind::Int { min: 0, max: 100 },
            )],
        )
        .unwrap();
    }
}
#[test]
#[ignore = "daily"]
fn native_datapack_host_reads_enforce_level_session_and_generation() {
    let scratch = tempfile::tempdir().unwrap();
    mod_platform::testing::with_mods(
        vec![ModEntry::new("registry_test", Box::new(SyncedRegistryMod))].into(),
        || {
            with_game(|game| {
                game.load_world(request(scratch.path())).unwrap();
                let candidate = || {
                    mod_api::DatapackCandidate::build(
                        mod_api::__private::datapack_registry_declarations(),
                        &[
                            (
                                "data/registry_test/registry_test/spell/fire.json".into(),
                                vec![("pack".to_owned(), br#"{"power":7}"#.to_vec())],
                            ),
                            (
                                "data/addon/registry_test/spell/ice.json".into(),
                                vec![("pack".to_owned(), br#"{"power":3}"#.to_vec())],
                            ),
                        ],
                    )
                    .unwrap()
                };
                game.mods.datapack_registries = Some(std::sync::Arc::new(
                    mod_api::DatapackRegistrySet::first(candidate(), 400),
                ));
                let level = game.level(game.dim.id()).unwrap();
                assert_eq!(
                    level.datapack_entries(game, "registry_test:spell"),
                    Some(vec!["addon:ice".into(), "registry_test:fire".into()])
                );
                let handle = level
                    .datapack_handle(game, "registry_test:spell", "registry_test:fire")
                    .unwrap();
                assert_eq!(
                    level.datapack_read(game, &handle),
                    Ok(vec![("power".into(), DatapackValue::Int(7))])
                );
                assert_eq!(level.datapack_entries(game, "missing:registry"), None);
                assert_eq!(
                    level.datapack_handle(game, "missing:registry", "addon:ice"),
                    Err(E::UnknownRegistry)
                );
                assert_eq!(
                    level.datapack_handle(game, "registry_test:spell", "addon:missing"),
                    Err(E::MissingEntry)
                );
                for refused in [
                    LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension()),
                    LevelRef::new(level.epoch(), DimensionKey::NETHER),
                ] {
                    assert_eq!(refused.datapack_entries(game, "registry_test:spell"), None);
                    assert_eq!(
                        refused.datapack_handle(game, "registry_test:spell", "registry_test:fire"),
                        Err(E::UnknownRegistry)
                    );
                    assert_eq!(
                        refused.datapack_read(game, &handle),
                        Err(E::UnknownRegistry)
                    );
                }
                game.mods.datapack_registries = Some(std::sync::Arc::new(
                    game.mods
                        .datapack_registries
                        .as_ref()
                        .unwrap()
                        .reload(candidate())
                        .unwrap(),
                ));
                assert_eq!(level.datapack_read(game, &handle), Err(E::StaleGeneration));
                game.mods.datapack_registries = Some(std::sync::Arc::new(
                    mod_api::DatapackRegistrySet::first(candidate(), 401),
                ));
                assert_eq!(level.datapack_read(game, &handle), Err(E::WrongSession));
                // The client must use the received snapshot, not this Game's server set.
                let declarations = mod_api::__private::datapack_registry_declarations();
                let remote = mod_api::DatapackCandidate::build(
                    declarations,
                    &[(
                        "data/registry_test/registry_test/spell/fire.json".into(),
                        vec![("pack".to_owned(), br#"{"power":19}"#.to_vec())],
                    )],
                )
                .unwrap();
                horizon_client::datapack_net::serve(Some(std::sync::Arc::new(
                    mod_api::DatapackRegistrySet::first(remote, 900),
                )));
                let offer = horizon_client::datapack_net::offer().unwrap();
                let horizon_client::datapack_net::Step::Send(bytes) =
                    horizon_client::datapack_net::server_step(true, Some(&offer))
                else {
                    panic!("snapshot required");
                };
                horizon_client::datapack_net::clear_received();
                horizon_client::datapack_net::accept(&bytes).unwrap();
                game.options.execution_role = crate::game::execution_role::ExecutionRole::Client;
                assert_eq!(
                    level.datapack_entries(game, "registry_test:spell"),
                    Some(vec!["registry_test:fire".into()])
                );
                let client_handle = level
                    .datapack_handle(game, "registry_test:spell", "registry_test:fire")
                    .unwrap();
                assert_eq!(
                    level.datapack_read(game, &client_handle),
                    Ok(vec![("power".into(), DatapackValue::Int(19))])
                );
                horizon_client::datapack_net::clear_received();
                assert_eq!(
                    level.datapack_read(game, &client_handle),
                    Err(E::UnknownRegistry)
                );
            });
        },
    );
}
