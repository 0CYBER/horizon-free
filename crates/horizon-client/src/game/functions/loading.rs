//! Admit a complete candidate before changing the live functions, tables or pack selection.
use super::{Game, PackStack, pack_stack, registry_graph};
use horizon_client::{Result, datapack::DataStore, text::Text};

/// `Main` treats a data-pack refusal as an orderly stop, not a crashed server.
#[derive(Debug)]
pub(in crate::game) struct DatapacksRefused;

impl std::fmt::Display for DatapacksRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Failed to load datapacks, can't proceed with server load")
    }
}

impl std::error::Error for DatapacksRefused {}

fn report_failure(error: &registry_graph::Error) {
    for line in &error.lines {
        if error.binding {
            log::error!("{line}");
        } else {
            log::warn!("{line}");
        }
    }
}

impl PackStack {
    /// `ReloadableServerRegistries.reload`: bind and validate the selected registries first.
    fn load_validated(&self) -> std::result::Result<DataStore, registry_graph::Error> {
        let packs = self
            .builtin
            .iter()
            .chain(self.enabled_packs())
            .collect::<Vec<_>>();
        let tags = registry_graph::check(&packs)?;
        let mut store = DataStore::load(&packs);
        tags.expand_predicates(&mut store.predicates);
        Ok(store)
    }
}

impl Game<'_> {
    /// Both dedicated and integrated servers validate before publishing world readiness.
    pub(in crate::game) fn start_datapacks(&mut self) -> Result<()> {
        if let Err(error) = self.try_reload_datapacks(false) {
            report_failure(&error);
            log::warn!(
                "Failed to load datapacks, can't proceed with server load. You can either fix your datapacks or reset to vanilla with --safeMode"
            );
            return Err(Box::new(DatapacksRefused));
        }
        Ok(())
    }

    pub(in crate::game) fn ensure_datapacks(&mut self) {
        if self.fns.loaded || self.world_root.as_os_str().is_empty() {
            return;
        }
        self.reload_datapacks(false);
    }

    /// A failed reload leaves the previous library, content tables and timer queue live.
    pub(in crate::game) fn reload_datapacks(&mut self, run_load: bool) -> bool {
        match self.try_reload_datapacks(run_load) {
            Ok(()) => true,
            Err(error) => {
                report_failure(&error);
                log::error!("Failed to execute reload");
                false
            }
        }
    }

    /// `MinecraftServer.reloadResources`: publish the replacement only after loading succeeds.
    fn try_reload_datapacks(
        &mut self,
        run_load: bool,
    ) -> std::result::Result<(), registry_graph::Error> {
        let stack = pack_stack(
            &self.world_root,
            &self.options.resources,
            self.cmds.vanilla_disabled,
            &self.level.enabled_features,
        );
        let store = stack.load_validated()?;
        self.fns.packs = stack.packs;
        self.fns.enabled = stack.enabled;
        self.fns.loaded = true;
        self.fns.load_pending = true;
        // On first open the world has built its content tables; an explicit reload replaces them.
        if run_load {
            self.reload_world_tables();
        }
        self.admit_store(store);
        // TimerQueue is live world state. A reload must not resurrect fired/cleared events.
        if !self.fns.schedule_loaded {
            self.fns.schedule_loaded = true;
            self.load_schedule();
        }
        Ok(())
    }

    /// `/datapack` stages the selected ids in memory. Persist them only after registry validation.
    pub(super) fn apply_pack_change(&mut self, previous: Vec<String>) {
        // Open archives afresh too: a repaired ZIP must not use the previous discovery's bytes.
        let mut stack = pack_stack(
            &self.world_root,
            &self.options.resources,
            self.cmds.vanilla_disabled,
            &self.level.enabled_features,
        );
        stack.enabled.clone_from(&self.fns.enabled);
        let store = match stack.load_validated() {
            Ok(store) => store,
            Err(error) => {
                report_failure(&error);
                log::error!("Failed to execute reload");
                self.cmds.vanilla_disabled = !previous.iter().any(|id| id == "vanilla");
                self.fns.enabled = previous.into_iter().filter(|id| id != "vanilla").collect();
                self.feedback(Text::translate("commands.reload.failure", &[]));
                return;
            }
        };
        self.fns.packs = stack.packs;
        self.save_datapack_settings();
        self.fns.load_pending = true;
        self.reload_world_tables();
        self.admit_store(store);
    }
}
