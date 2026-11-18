//! A mod's game rule declaration as the engine's rule: its type, default and range.
use super::TryIntoEngine;
use horizon_client_core::gamerules::{Rule, Value};

impl TryIntoEngine<()> for mod_api::GameRuleDecl {
    type Engine = Rule;
    type Error = std::convert::Infallible;
    fn try_into_engine(&self, _: &()) -> Result<Self::Engine, Self::Error> {
        // A bool rule has no range, as vanilla's `BooleanValue`; an int rule keeps the
        // declared bounds, which `GameRules::set` checks for `/gamerule`, the screen and loading.
        let (default, min, max) = match self.default {
            mod_api::GameRuleDefault::Int { default, min, max } => (Value::Int(default), min, max),
            mod_api::GameRuleDefault::Bool(value) => (Value::Bool(value), 0, 0),
            _ => unreachable!("every GameRuleDefault the SDK declares is converted"),
        };
        Ok(Rule {
            name: self.name.clone(),
            category: self.category.clone(),
            default,
            min,
            max,
        })
    }
}
