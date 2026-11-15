use super::*;
use horizon_world::native_load_context::DataLoadOrigin;

type SettingsSnapshot = Arc<[(String, Arc<[(String, String)]>)]>;

/// Whether this owning load belongs to registration's startup publication.
#[derive(Clone, Copy)]
pub(in super::super) enum OwningLoad {
    Startup,
    Later,
}

/// Execution side and settings ownership selected at the caller's load boundary.
#[derive(Clone, Copy)]
pub(in super::super) struct LoadRole {
    pub(in super::super) execution: ExecutionRole,
    pub(in super::super) owning: OwningLoad,
}

impl LoadRole {
    pub(in super::super) fn startup(execution: ExecutionRole) -> Self {
        Self {
            execution,
            owning: OwningLoad::Startup,
        }
    }
}

impl From<ExecutionRole> for LoadRole {
    fn from(execution: ExecutionRole) -> Self {
        Self {
            execution,
            owning: OwningLoad::Later,
        }
    }
}

impl OwningLoad {
    fn context(
        self,
        origin: DataLoadOrigin,
        inputs: Arc<LoadInputs>,
        ids: &[&str],
        registration: Option<SettingsSnapshot>,
    ) -> LoadContext {
        if let Self::Startup = self
            && let Some(settings) = registration
        {
            return LoadContext::from_settings_snapshot(
                origin,
                inputs,
                ids.iter().copied(),
                settings,
            );
        }
        LoadContext::new(origin, inputs, ids.iter().copied())
    }
}

/// Capture load inputs at the resource-generation boundary, before workers share them.
pub(in super::super) fn load_item_data(
    root: &Path,
    role: ExecutionRole,
    owning_load: OwningLoad,
) -> horizon_client::Result<ItemData> {
    let phase = mod_api::__private::PhaseOwner::scope(if role.is_client() {
        mod_api::__private::ModPhase::Setup
    } else {
        mod_api::__private::ModPhase::World
    });
    let _reload = phase.reloading()?;
    let inputs = LoadInputs::new(builtin_registry_names(root)?, Game::mod_data_root());
    let inventory = horizon_mod_package::enabled::EnabledMods::read(root)?;
    let ids: Vec<_> = if role.is_logical_server() {
        inventory.server_ids().collect()
    } else {
        inventory.client_ids().collect()
    };
    let context = owning_load.context(
        role.data_load_origin(),
        Arc::new(inputs),
        &ids,
        mod_api::__private::RegistrationData::initial_settings_snapshot(),
    );

    Ok(ItemData::load(root, context)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_keeps_registration_settings_and_later_load_captures_changes() {
        let directory = tempfile::tempdir().unwrap();
        let reader = directory.path().join("reader");
        std::fs::create_dir(&reader).unwrap();
        std::fs::write(reader.join("count"), "3").unwrap();
        let inputs = Arc::new(LoadInputs::new(BTreeMap::new(), directory.path().into()));
        let settings: SettingsSnapshot =
            horizon_world::native_load_context::capture_settings(directory.path(), ["reader"])
                .into_iter()
                .map(|(id, values)| (id, values.into_iter().collect()))
                .collect();
        let make = |role: OwningLoad| {
            role.context(
                DataLoadOrigin::ServerDataLoad,
                inputs.clone(),
                &["reader"],
                Some(settings.clone()),
            )
        };
        assert_eq!(
            make(OwningLoad::Startup)
                .setting("reader", "count")
                .as_deref(),
            Some("3")
        );
        std::fs::write(reader.join("count"), "7").unwrap();
        assert_eq!(
            make(OwningLoad::Later)
                .setting("reader", "count")
                .as_deref(),
            Some("7")
        );
        // Another initial consumer still has the registration publication.
        assert_eq!(
            make(OwningLoad::Startup)
                .setting("reader", "count")
                .as_deref(),
            Some("3")
        );
        assert_eq!(make(OwningLoad::Later).setting("reader", "absent"), None);
        for role in [OwningLoad::Startup, OwningLoad::Later] {
            let context = role.context(
                DataLoadOrigin::ClientPreload,
                inputs.clone(),
                &["silent"],
                Some(settings.clone()),
            );
            assert_eq!(context.setting("silent", "count"), None);
            assert_eq!(context.registry("minecraft:item"), None);
        }
        let fallback =
            OwningLoad::Startup.context(DataLoadOrigin::ClientPreload, inputs, &["reader"], None);
        assert_eq!(fallback.setting("reader", "count").as_deref(), Some("7"));
    }
}
