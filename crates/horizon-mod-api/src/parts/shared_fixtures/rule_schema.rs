/// A small independent vanilla-schema fixture for tests without a game.
pub fn rule_schema() -> crate::__private::RuleSchema<'static> {
    crate::__private::RuleSchema {
        category: &|id| matches!(id, "minecraft:drops" | "minecraft:misc"),
        position: &|_| None,
        rule_count: 0,
    }
}
