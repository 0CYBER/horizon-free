use crate::game::Game;
use horizon_client::gamerules::{Rule, Value};
use horizon_gameplay::commands::gamerule::GameRuleHost;

impl GameRuleHost for Game<'_> {
    fn find_rule(&self, name: &str) -> Option<Rule> {
        self.ui.rules.rule(name).cloned()
    }
    fn rule_value(&self, name: &str) -> Option<Value> {
        self.ui.rules.get(name)
    }
    fn set_rule_as_acting_player(&mut self, rule: &Rule, value: Value) {
        let name = self.lan_acting_name.clone();
        let actor = match (
            self.lan_acting_uuid,
            self.lan_acting_entity,
            name.as_deref(),
        ) {
            (Some(uuid), Some(entity_id), Some(name)) => Some(mod_api::GameRuleActor {
                uuid,
                entity_id: entity_id as i32,
                name,
            }),
            _ => None,
        };
        self.set_game_rule(rule, value, actor.as_ref());
    }
}
