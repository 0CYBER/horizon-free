//! The integrated command inventory shares the frozen custom statistic declarations.
use super::Game;

impl Game<'_> {
    pub(super) fn sync_stat_registries(&mut self) {
        self.ui.registries.sync_damage_type_ids();
        let custom = mod_api::__private::custom_stat_declarations();
        if custom.is_empty() {
            return;
        }
        let entry = self
            .ui
            .registries
            .by_registry
            .entry("minecraft:custom_stat".into())
            .or_default();
        for id in custom {
            if !entry.ids.contains(id) {
                entry.ids.push(id.clone());
            }
        }
    }
}
