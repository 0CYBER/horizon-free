# Register game rule categories

In `Hooks::register`, call `registrar.game_rule_category("machines")` to declare a category in your mod's namespace. The call returns `Result<(), GameRuleError>`. A full ID in a manifest-owned namespace is also accepted.

Declare a boolean rule with `registrar.game_rule_bool("enabled", "mymod:machines", true)`. Handle its `Result<BoolGameRuleKey, GameRuleError>` and retain the key from `Ok(key)`. Read the value with `level.game_rule_bool(host, key)`, which returns `Option<bool>`.

Declare an integer rule, like vanilla's `random_tick_speed`, with `registrar.game_rule_int("cap", "mymod:machines", 8, 0, 64)`: the default, then the smallest and largest value. It returns `Result<IntGameRuleKey, GameRuleError>`. Read the value with `level.game_rule_int(host, key)`, which returns `Option<i32>` and is always within the range. `/gamerule mymod:cap 12` sets it and `/gamerule mymod:cap 70` is refused, as vanilla refuses an out-of-range integer. The game rules screen shows it as a number box, and remote clients get its value like any other rule.

| Rule | Behavior |
| --- | --- |
| Invalid IDs, unowned namespaces or duplicate declarations | Registration returns `GameRuleError::InvalidId`, `ForeignNamespace` or `Duplicate`; loading is refused even if you ignore the error. |
| Integer range | `min` above `max`, or a default outside `min..=max`, returns `GameRuleError::InvalidRange` and refuses loading. A saved value outside the range is ignored on load, and the rule keeps its default. |
| Category references | Resolved after all mods register; an unknown category refuses loading. |
| Translation | The category key is `gamerule.category.<namespace>.<path>`. |
| In-world game rules screen | The host displays a category when it contains a matching rule. Remote clients do not receive mod categories. |
| World creation | Mod rules use their declared defaults; the world-creation rules screen does not list them. |
| Persistence | Values are saved with the world, including values belonging to a missing mod. |
