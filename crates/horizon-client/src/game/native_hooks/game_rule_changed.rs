use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A server game rule `rule` (`minecraft:keep_inventory`) was set to
    /// `value` (`GameRuleChangedEvent`, from `GameRules.set` with a server:
    /// `/gamerule`, the game rules screen, the management protocol, a test
    /// environment, `server.properties`). `actor` is present when a player
    /// caused the change. `level.game_rules(host)` of any of the server's
    /// levels reads every rule, the new value included. Server-side.
    pub(in crate::game) fn native_game_rule_changed(
        &mut self,
        rule: &str,
        value: GameRuleValue,
        actor: Option<&GameRuleActor<'_>>,
    ) {
        self.fire_game_rule_changed(rule, value, actor);
    }
}
