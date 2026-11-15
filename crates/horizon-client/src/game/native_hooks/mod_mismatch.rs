use super::*;
use horizon_client::save::level_mods;

impl Game<'_> {
    /// Dispatch the saved list at the earliest Game Host boundary on either side.
    /// Each resolution keeps its first resolving listener across dispatch.
    pub(in crate::game) fn native_mod_mismatch(
        &mut self,
        mismatches: &[mod_api::ModMismatch],
        resolved: &mut Vec<mod_api::ModMismatchResolution>,
    ) {
        self.fire_mod_mismatch(mismatches, resolved);
    }

    /// Before the world in `dir` opens: the saved mods whose content opening it would lose
    /// (`level_mods::missing`) that no `mod_mismatch` listener resolves. Only when one is missing
    /// do the mods hear the differences here, before the world opens; the open then uses this
    /// answer instead of asking again (`check_mod_mismatch`). Reads only: nothing in `dir` changes.
    pub(in crate::game) fn ask_missing_mods(
        &mut self,
        dir: &std::path::Path,
    ) -> Vec<level_mods::MissingMod> {
        let loaded = level_mods::Loaded::now();
        let missing = level_mods::missing_in(dir, &loaded);
        if missing.is_empty() {
            return missing;
        }
        // Without a saved mod list the hook does not fire (block namespaces alone), but the
        // answer is still kept, for the player's choice to attach to.
        let mismatches = level_mods::mismatches_in(dir, &loaded);
        let mut resolved = Vec::new();
        if let Some(mismatches) = &mismatches {
            self.native_mod_mismatch(mismatches, &mut resolved);
        }
        let mismatches = mismatches.unwrap_or_default();
        let missing = level_mods::unresolved(missing, &resolved);
        level_mods::remember_answer(dir, mismatches, resolved);
        missing
    }

    /// A server opened a world: its saved code and content mods against the loaded ones.
    /// The mods hear the differences once (before the open, when a mod was missing and the
    /// open asked first: `ask_missing_mods`), and each one nobody resolved is a warning.
    pub(in crate::game) fn check_mod_mismatch(&mut self) {
        let (mismatches, resolved) = match level_mods::take_answer(&self.save.root) {
            Some(answer) => answer,
            None => {
                if self.level.saved_mods.is_none() {
                    return;
                }
                let mismatches = self.level.mod_mismatches(&level_mods::Loaded::now().mods());
                let mut resolved = Vec::new();
                self.native_mod_mismatch(&mismatches, &mut resolved);
                (mismatches, resolved)
            }
        };
        let shown = |version: &str| -> String {
            if version.is_empty() {
                "an unknown version".to_owned()
            } else {
                version.to_owned()
            }
        };
        let mut ordered: Vec<_> = mismatches.iter().collect();
        ordered.sort_by(|a, b| a.mod_id.cmp(&b.mod_id));
        for mismatch in ordered {
            let current = mismatch
                .current_version
                .as_deref()
                .map_or_else(|| "missing".to_owned(), shown);
            if let Some(resolution) = resolved.iter().find(|r| r.mod_id == mismatch.mod_id) {
                log::debug!(
                    "Horizon: mod {} was {}, now {}; resolved by {} ({})",
                    mismatch.mod_id,
                    shown(&mismatch.saved_version),
                    current,
                    resolution.resolver,
                    if resolution.resolver == mismatch.mod_id {
                        "self"
                    } else {
                        "other mod"
                    }
                );
            } else {
                log::warn!(
                    "Horizon: mod {} was {} when the world was saved, now {}",
                    mismatch.mod_id,
                    shown(&mismatch.saved_version),
                    current
                );
            }
        }
        if mismatches
            .iter()
            .any(|m| !resolved.iter().any(|r| r.mod_id == m.mod_id))
        {
            log::warn!("Horizon: changed mods may cause the world to work incorrectly");
        }
        if mismatches
            .iter()
            .any(|m| resolved.iter().any(|r| r.mod_id == m.mod_id))
        {
            log::debug!("Horizon: changed mods may cause the world to work incorrectly");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{Hooks, HostSide, ModEntry, ModMismatchEvent};
    use std::sync::{Arc, Mutex};

    #[test]
    #[ignore = "daily"]
    fn existing_lists_fire_on_client_and_dedicated_even_without_differences() {
        struct Listener(Arc<Mutex<usize>>);
        #[mod_api::__subscriptions]
        impl Hooks for Listener {
            fn mod_mismatch(&self, _: HostSide, event: &mut ModMismatchEvent<'_>) {
                assert!(event.mismatches.is_empty());
                *self.0.lock().unwrap() += 1;
            }
        }
        let heard = Arc::new(Mutex::new(0));
        mod_platform::testing::with_mods(
            vec![ModEntry::new("listener", Box::new(Listener(heard.clone())))].into(),
            || {
                for role in [
                    execution_role::ExecutionRole::Client,
                    execution_role::ExecutionRole::DedicatedServer,
                ] {
                    super::super::super::native_entity_refs::tests::with_game(|game| {
                        game.options.execution_role = role;
                        game.level.saved_mods = None;
                        game.check_mod_mismatch();
                        game.level.saved_mods = Some(vec![]);
                        game.check_mod_mismatch();
                    });
                }
            },
        );
        assert_eq!(*heard.lock().unwrap(), 2);
    }

    /// A world saved with `gone`, a mod no test loads, and its block namespace.
    fn world_with_gone_mod(name: &str) -> std::path::PathBuf {
        use horizon_core::nbt::Tag;
        let dir = std::env::temp_dir().join(format!("horizon-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let entry = Tag::compound([
            ("ModId", Tag::String("gone".into())),
            ("ModVersion", Tag::String("1.0.0".into())),
        ]);
        horizon_client::save::write_dat(
            &dir.join("level.dat"),
            Tag::compound([
                (
                    "Data",
                    Tag::compound([("LevelName", Tag::String(name.into()))]),
                ),
                (
                    "fml",
                    Tag::compound([("LoadingModList", Tag::List(vec![entry]))]),
                ),
                (
                    level_mods::COMPOUND,
                    Tag::compound([(
                        "BlockNamespaces",
                        Tag::List(vec![Tag::String("gone".into())]),
                    )]),
                ),
            ]),
        )
        .unwrap();
        dir
    }

    /// A listener that resolves the missing mod takes it off the question, so no screen shows;
    /// without one it is asked about. Asking reads the world and writes nothing.
    #[test]
    #[ignore = "daily"]
    fn a_resolved_missing_mod_is_not_asked_about() {
        struct Migrator;
        #[mod_api::__subscriptions]
        impl Hooks for Migrator {
            fn mod_mismatch(&self, _: HostSide, event: &mut ModMismatchEvent<'_>) {
                event.resolved.push("gone".to_owned().into());
            }
        }
        let dir = world_with_gone_mod("resolved-missing-mod");
        let before = std::fs::read(dir.join("level.dat")).unwrap();
        super::super::super::native_entity_refs::tests::with_game(|game| {
            let missing = game.ask_missing_mods(&dir);
            assert_eq!(
                missing.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
                ["gone"]
            );
        });
        mod_platform::testing::with_mods(
            vec![ModEntry::new("migrator", Box::new(Migrator))].into(),
            || {
                super::super::super::native_entity_refs::tests::with_game(|game| {
                    assert!(game.ask_missing_mods(&dir).is_empty());
                    // The open uses this answer instead of asking the mods again.
                    let (_, resolved) = level_mods::take_answer(&dir).unwrap();
                    assert_eq!(resolved[0].resolver, "migrator");
                });
            },
        );
        assert_eq!(std::fs::read(dir.join("level.dat")).unwrap(), before);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
