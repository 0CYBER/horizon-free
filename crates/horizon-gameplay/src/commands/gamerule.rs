//! `/gamerule`.
use super::host::{CommandHost, R};
use horizon_client::{
    command::{Arg, Context},
    gamerules::{Rule, Value},
    text::Text,
};

/// What `/gamerule` needs from the game.
pub trait GameRuleHost: CommandHost {
    /// The rule called `name` (its id with or without the namespace), if the game has it.
    fn find_rule(&self, name: &str) -> Option<Rule>;
    /// The rule's value now, if it has one set.
    fn rule_value(&self, name: &str) -> Option<Value>;
    /// `GameRules.set`, as the player who ran the command (a guest's own while their command runs).
    fn set_rule_as_acting_player(&mut self, rule: &Rule, value: Value);
}

/// `/gamerule <rule> [value]`.
pub fn gamerule(host: &mut dyn GameRuleHost, ctx: &Context) -> R<i32> {
    let Some(name) = ctx.literals.get(1).cloned() else {
        return Ok(0);
    };
    let Some(rule) = host.find_rule(&name) else {
        return Ok(0);
    };
    // GameRule.id(): only the default namespace is omitted in messages.
    let rule_id = rule.name.strip_prefix("minecraft:").unwrap_or(&rule.name);
    let current = host.rule_value(&rule.name).unwrap_or(rule.default);
    let value = match ctx.get("value") {
        Some(Arg::Bool(b)) => Value::Bool(*b),
        Some(Arg::Int(i)) => Value::Int(*i),
        _ => {
            host.feedback_unbroadcast(Text::tr("commands.gamerule.query", &[&rule_id, &current]));
            return Ok(match current {
                Value::Bool(b) => i32::from(b),
                Value::Int(i) => i,
            });
        }
    };
    if current == value {
        return Err(host.error("commands.gamerule.not_set", &[&rule_id, &value]));
    }
    host.set_rule_as_acting_player(&rule, value);
    host.feedback(Text::tr("commands.gamerule.set", &[&rule_id, &value]));
    Ok(match value {
        Value::Bool(b) => i32::from(b),
        Value::Int(i) => i,
    })
}
