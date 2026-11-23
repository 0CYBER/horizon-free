use super::Game;
use horizon_client_core::gamerules::Value as GameRuleValue;
use mod_api::GameRuleReadError;

impl Game<'_> {
    pub(super) fn game_rule(&self, id: &str) -> Result<GameRuleValue, GameRuleReadError> {
        self.ui.rules.get(id).ok_or(GameRuleReadError::UnknownRule)
    }

    /// The rule's default (`GameRules` definition), not its current value.
    pub(super) fn game_rule_default(&self, id: &str) -> Option<GameRuleValue> {
        self.ui.rules.rule(id).map(|rule| rule.default)
    }

    pub(super) fn game_rules(&self) -> Vec<(String, GameRuleValue)> {
        self.ui
            .rules
            .available_rules()
            .filter_map(|rule| Some((rule.name.clone(), self.ui.rules.get(&rule.name)?)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use horizon_client_core::gamerules::Value;

    #[test]
    #[ignore = "daily"]
    fn game_rule_default_is_vanilla_and_ignores_the_current_value() {
        super::super::native_entity_refs::tests::with_game(|game| {
            // Vanilla: keep_inventory defaults to false.
            assert_eq!(game.game_rule_default("keep_inventory"), Some(Value::Bool(false)));
            assert_eq!(
                game.game_rule_default("minecraft:keep_inventory"),
                Some(Value::Bool(false))
            );
            assert_eq!(game.game_rule_default("no_such_rule"), None);

            // A changed value leaves the default alone.
            game.ui.rules.set("keep_inventory", Value::Bool(true));
            assert_eq!(game.game_rule_default("keep_inventory"), Some(Value::Bool(false)));
            assert_eq!(game.game_rule("keep_inventory").unwrap(), Value::Bool(true));
        });
    }
}
