//! Install the opening's authoritative rules before either early server hook.
use super::{Game, world_loading::PreparedWorld};
use horizon_client::gamerules::{GameRules, Value};

impl Game<'_> {
    pub(super) fn prepare_world_and_rules(
        &mut self,
        request: super::world_loading::WorldRequest,
    ) -> crate::Result<PreparedWorld> {
        self.join_world_rest()?;
        self.perf.world_opening();
        let prepared = request.prepare(
            &self.res,
            &self.pack,
            &self.options,
            &self.settings,
            &horizon_world::datafix::progress::UpgradeProgress::new(),
        )?;
        self.load_opening_game_rules(&prepared);
        Ok(prepared)
    }

    pub(super) fn prepare_early_game_rules(
        &mut self,
        loader: super::world_loading::WorldLoader,
    ) -> crate::Result<PreparedWorld> {
        let prepared = loader.finish()?;
        self.load_opening_game_rules(&prepared);
        Ok(prepared)
    }

    pub(super) fn load_opening_game_rules(&mut self, prepared: &PreparedWorld) {
        let level = &prepared.level;
        let mut rules = GameRules::load_with_registry(
            &prepared.save_root,
            &level.enabled_features,
            level.rule_registry.clone(),
        );
        for (id, raw) in &level.game_rules {
            let value = match rules.get(id) {
                Some(Value::Bool(_)) => Value::Bool(*raw != 0),
                Some(Value::Int(_)) => Value::Int(*raw),
                None => continue,
            };
            rules.set(id, value);
        }
        self.ui.rules = rules;
    }
}
