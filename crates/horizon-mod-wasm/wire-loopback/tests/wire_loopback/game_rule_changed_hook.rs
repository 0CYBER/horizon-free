pub(super) struct GameRuleChangedHooks;
impl GameRuleChangedHooks {
    pub(super) fn game_rule_changed(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::GameRuleChangedEvent<'_>,
    ) {
        assert_eq!(event.rule, "test:rule");
        if let Some(actor) = event.actor {
            assert_eq!(
                (actor.uuid, actor.entity_id, actor.name),
                (123, 45, "test:actor")
            );
            event.value = mod_api::GameRuleValue::Bool(true);
        } else {
            event.value = mod_api::GameRuleValue::Bool(false);
        }
    }
}
