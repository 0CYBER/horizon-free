//! Owned registration data, independent of package parsing and engine storage.

/// A game rule declared by a mod during registration.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct GameRuleDecl {
    /// Namespaced rule identifier.
    pub name: String,
    /// Category identifier, resolved after every mod registers.
    pub category: String,
    /// The rule's type and its initial value in a new world.
    pub default: GameRuleDefault,
}

/// A declared rule's type and initial value, as vanilla's `GameRuleType` (`BOOL` or `INT`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum GameRuleDefault {
    /// An on/off rule ([`GameRulesRegistrar::game_rule_bool`](crate::GameRulesRegistrar::game_rule_bool)).
    Bool(bool),
    /// An integer rule ([`GameRulesRegistrar::game_rule_int`](crate::GameRulesRegistrar::game_rule_int)):
    /// `/gamerule` and the game rules screen refuse values outside `min..=max`.
    Int {
        /// Initial value, within `min..=max`.
        default: i32,
        /// Smallest accepted value.
        min: i32,
        /// Largest accepted value.
        max: i32,
    },
}

// no wire: borrowed parser or registration callbacks execute only in their owning process.
/// The vanilla rule metadata supplied once by the owning game at registration.
/// This is a borrowed view like `KnownIds`, not a second registration lifecycle.
// exhaustive: the game supplies the complete schema with a literal
// hidden: only the game supplies the vanilla schema used to install declarations.
#[doc(hidden)]
pub struct RuleSchema<'a> {
    /// Whether a category already exists in the game.
    pub category: &'a dyn Fn(&str) -> bool,
    /// The dense position of an existing game rule, or none.
    pub position: &'a dyn Fn(&str) -> Option<u32>,
    /// Number of game-owned rules preceding mod declarations.
    pub rule_count: u32,
}

impl RuleSchema<'_> {
    /// Validate all declarations without installing any of them.
    pub fn validate(
        &self,
        categories: &[String],
        rules: &[GameRuleDecl],
    ) -> Result<Vec<u32>, Vec<String>> {
        let mut slots = Vec::with_capacity(rules.len());
        let mut names = std::collections::HashSet::new();
        let mut errors = Vec::new();
        for rule in rules {
            let owner = rule
                .name
                .split_once(':')
                .map_or("unknown", |(owner, _)| owner);
            if (self.position)(&rule.name).is_some() || !names.insert(&rule.name) {
                errors.push(format!("mod {owner}: Duplicate({:?})", rule.name));
            } else if let GameRuleDefault::Int { default, min, max } = rule.default
                && !(min..=max).contains(&default)
            {
                errors.push(format!("mod {owner}: InvalidRange({:?})", rule.name));
            } else if !(self.category)(&rule.category) && !categories.contains(&rule.category) {
                errors.push(format!(
                    "mod {owner}: UnknownCategory {{ rule: {:?}, category: {:?} }}",
                    rule.name, rule.category
                ));
            } else if let Some(slot) = u32::try_from(slots.len())
                .ok()
                .and_then(|n| self.rule_count.checked_add(n))
            {
                slots.push(slot);
            } else {
                errors.push(format!(
                    "mod {owner}: rule slot overflow for {:?}",
                    rule.name
                ));
            }
        }
        if errors.is_empty() {
            Ok(slots)
        } else {
            Err(errors)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rule(name: &str, category: &str) -> GameRuleDecl {
        GameRuleDecl {
            name: name.into(),
            category: category.into(),
            default: GameRuleDefault::Bool(true),
        }
    }
    #[test]
    fn schema_accepts_forward_categories_and_keeps_dense_declaration_order() {
        let schema = RuleSchema {
            category: &|id| id == "minecraft:drops",
            position: &|_| None,
            rule_count: 17,
        };
        assert_eq!(
            schema.validate(
                &["later:machines".into()],
                &[
                    rule("first:active", "later:machines"),
                    rule("first:drops", "minecraft:drops"),
                ]
            ),
            Ok(vec![17, 18])
        );
    }
    #[test]
    fn schema_reports_all_refusals_without_partial_slots() {
        let schema = RuleSchema {
            category: &|_| false,
            position: &|id| (id == "game:existing").then_some(0),
            rule_count: 1,
        };
        let errors = schema
            .validate(
                &[],
                &[
                    rule("game:existing", "missing:category"),
                    rule("first:bad", "missing:category"),
                    rule("first:bad", "missing:category"),
                ],
            )
            .unwrap_err();
        assert_eq!(errors.len(), 3);
        assert!(errors[0].contains("Duplicate"));
        assert!(errors[1].contains("UnknownCategory") && errors[1].contains("mod first"));
        assert!(errors[2].contains("Duplicate"));
    }
}

/// A declared block entity type and the block identifiers it supports.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct BlockEntityTypeDecl {
    /// Namespaced type identifier validated during registration.
    pub id: String,
    /// Supported block identifiers, in declaration order.
    pub blocks: Vec<String>,
}
