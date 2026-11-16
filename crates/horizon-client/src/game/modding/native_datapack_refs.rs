//! Datapack registry reads through a level handle.
use super::Game;
use mod_api::LevelRef;

impl Game<'_> {
    /// The datapack registry set of this level's world session, when the level is the open one:
    /// the server's own, or on a client the one its server sent at the join.
    fn level_datapack_set(
        &self,
        level: LevelRef,
    ) -> Option<std::sync::Arc<mod_api::DatapackRegistrySet>> {
        (self.level_handle_current(level) && level.dimension() == self.native_level_key())
            .then_some(())?;
        if self.is_logical_server() {
            self.mods.datapack_registries.clone()
        } else {
            horizon_client::datapack_net::received()
        }
    }

    /// The entry ids of a mod's datapack registry as the world last loaded it.
    pub(super) fn level_datapack_entries(
        &self,
        level: LevelRef,
        registry: &str,
    ) -> Option<Vec<String>> {
        let entries = self
            .level_datapack_set(level)?
            .entries(registry)
            .ok()?
            .into_iter()
            .map(str::to_owned)
            .collect();
        Some(entries)
    }

    /// A handle to one entry at the generation now loaded.
    pub(super) fn level_datapack_handle(
        &self,
        level: LevelRef,
        registry: &str,
        entry: &str,
    ) -> Result<mod_api::DatapackHandle, mod_api::DatapackReadError> {
        self.level_datapack_set(level)
            .ok_or(mod_api::DatapackReadError::UnknownRegistry)?
            .handle(registry, entry)
    }

    /// The fields of the entry a handle names, after the handle's session and generation check.
    pub(super) fn level_datapack_read(
        &self,
        level: LevelRef,
        handle: &mod_api::DatapackHandle,
    ) -> Result<Vec<(String, mod_api::DatapackValue)>, mod_api::DatapackReadError> {
        let set = self
            .level_datapack_set(level)
            .ok_or(mod_api::DatapackReadError::UnknownRegistry)?;
        Ok(set.get(handle)?.values.clone())
    }
}

#[cfg(test)]
mod tests {
    use mod_api::{DatapackReadError as E, *};

    #[test]
    #[ignore = "daily"]
    fn level_registry_reads_check_world_session_and_generation() {
        struct RegistryMod;
        #[mod_api::__subscriptions]
        impl Hooks for RegistryMod {
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
        mod_platform::testing::with_mods(
            vec![ModEntry::new("example", Box::new(RegistryMod))].into(),
            || {
                super::super::native_entity_refs::tests::with_game(|game| {
                    game.install_live_features();
                    let level = game.level(game.dim.id()).unwrap();
                    let candidate = || {
                        DatapackCandidate::build(
                            mod_api::__private::datapack_registry_declarations(),
                            &[(
                                "data/addon/example/spell/fire.json".into(),
                                vec![("pack".to_owned(), br#"{"power":7}"#.to_vec())],
                            )],
                        )
                        .unwrap()
                    };
                    let first = DatapackRegistrySet::first(candidate(), 123);
                    game.mods.datapack_registries = Some(std::sync::Arc::new(first.clone()));
                    assert_eq!(
                        game.level_datapack_entries(level, "example:spell"),
                        Some(vec!["addon:fire".into()])
                    );
                    for invalid in [
                        LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension()),
                        LevelRef::new(level.epoch(), DimensionKey::NETHER),
                    ] {
                        assert_eq!(game.level_datapack_entries(invalid, "example:spell"), None);
                        assert_eq!(
                            game.level_datapack_handle(invalid, "example:spell", "addon:fire"),
                            Err(E::UnknownRegistry)
                        );
                    }
                    let handle = game
                        .level_datapack_handle(level, "example:spell", "addon:fire")
                        .unwrap();
                    assert_eq!(
                        game.level_datapack_read(level, &handle),
                        Ok(vec![("power".into(), DatapackValue::Int(7))])
                    );
                    assert_eq!(
                        game.level_datapack_handle(level, "example:missing", "addon:fire"),
                        Err(E::UnknownRegistry)
                    );
                    assert_eq!(
                        game.level_datapack_handle(level, "example:spell", "addon:missing"),
                        Err(E::MissingEntry)
                    );
                    game.mods.datapack_registries =
                        Some(std::sync::Arc::new(first.reload(candidate()).unwrap()));
                    assert_eq!(
                        game.level_datapack_read(level, &handle),
                        Err(E::StaleGeneration)
                    );
                    game.mods.datapack_registries = Some(std::sync::Arc::new(
                        DatapackRegistrySet::first(candidate(), 124),
                    ));
                    assert_eq!(
                        game.level_datapack_read(level, &handle),
                        Err(E::WrongSession)
                    );
                });
            },
        );
    }
}
