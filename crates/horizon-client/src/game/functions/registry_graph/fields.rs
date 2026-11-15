//! Holder fields from the 26.3 loot/number/advancement codecs. Unknown JSON fields are not codecs.
use super::{Reference, Registry};
use serde_json::Value;

pub(super) fn kind(name: &str) -> &str {
    name.strip_prefix("minecraft:").unwrap_or(name)
}

fn value_kind(value: &Value) -> &str {
    value
        .get("type")
        .and_then(Value::as_str)
        .map(kind)
        .unwrap_or("")
}

pub(super) fn references(value: &Value, registry: Registry) -> Result<Vec<Reference>, Vec<String>> {
    let mut visitor = Fields {
        references: Vec::new(),
        errors: Vec::new(),
        follow: true,
    };
    visitor.value(value, registry, "");
    if visitor.errors.is_empty() {
        Ok(visitor.references)
    } else {
        Err(visitor.errors)
    }
}

struct Fields {
    references: Vec<Reference>,
    errors: Vec<String>,
    follow: bool,
}

impl Fields {
    fn holder(&mut self, value: &Value, registry: Registry, path: &str) {
        match value {
            Value::String(name) => self.references.push(Reference {
                registry,
                name: name.clone(),
                path: path.to_owned(),
                tag: false,
                follow: self.follow,
            }),
            Value::Array(_) if matches!(registry, Registry::Modifier | Registry::Slots) => {
                let field = if registry == Registry::Modifier {
                    "functions"
                } else {
                    "terms"
                };
                self.set(value, registry, &format!("{path}.{field}"));
            }
            _ => self.value(value, registry, path),
        }
    }

    fn field(&mut self, value: &Value, field: &str, registry: Registry, path: &str) {
        if let Some(child) = value.get(field) {
            self.holder(child, registry, &format!("{path}.{field}"));
        }
    }

    /// Some codecs bind a holder that their `validate` method never visits. This also applies
    /// to references inside an inline value at that position, not just a named outer holder.
    fn bind_field(&mut self, value: &Value, field: &str, registry: Registry, path: &str) {
        let previous = self.follow;
        self.follow = false;
        self.field(value, field, registry, path);
        self.follow = previous;
    }

    fn set(&mut self, value: &Value, registry: Registry, path: &str) {
        match value {
            Value::String(name) if name.starts_with('#') => self.references.push(Reference {
                registry,
                name: name[1..].to_owned(),
                path: path.to_owned(),
                tag: true,
                follow: self.follow,
            }),
            Value::Array(values) => {
                for (index, child) in values.iter().enumerate() {
                    self.holder(child, registry, &format!("{path}[{index}]"));
                }
            }
            Value::Null => {}
            child => self.holder(child, registry, &format!("{path}[0]")),
        }
    }

    fn value(&mut self, value: &Value, registry: Registry, path: &str) {
        // 26.3 predicates dispatch on `type`. The old `condition: reference`/`name` shape
        // is a codec failure, not a holder that should silently evaluate to false.
        if matches!(
            registry,
            Registry::Predicate | Registry::Modifier | Registry::Slots
        ) {
            if value.get("type").and_then(Value::as_str).is_none() {
                self.errors.push(format!(
                    "{path}: No key type in {} {value}",
                    registry.directory()
                ));
                return;
            }
            if registry == Registry::Predicate && value_kind(value) == "reference" {
                self.errors.push(format!(
                    "{path}: Unknown predicate type minecraft:reference"
                ));
                return;
            }
        }
        if !value.is_object() {
            return;
        }
        match registry {
            Registry::Predicate => self.condition(value, path),
            Registry::Modifier => self.modifier(value, path),
            Registry::Table => self.table(value, path),
            Registry::Float | Registry::Int => self.number(value, registry, path),
            Registry::Slots => match value_kind(value) {
                "group" => self.set(&value["terms"], Registry::Slots, &format!("{path}.terms")),
                "filtered" | "limit_slots" | "contents" => {
                    self.field(value, "slot_source", Registry::Slots, path)
                }
                _ => {}
            },
            Registry::Advancement => self.advancement(value, path),
        }
    }

    fn condition(&mut self, value: &Value, path: &str) {
        match value_kind(value) {
            "all_of" | "any_of" => self.set(
                &value["terms"],
                Registry::Predicate,
                &format!("{path}.terms"),
            ),
            "inverted" => self.field(value, "term", Registry::Predicate, path),
            "random_chance" => self.bind_field(value, "chance", Registry::Float, path),
            "int_value_check" | "float_value_check" => {
                let registry = if value_kind(value) == "int_value_check" {
                    Registry::Int
                } else {
                    Registry::Float
                };
                self.field(value, "value", registry, path);
                self.range(&value["test"], registry, &format!("{path}.range"));
            }
            "time_check" => self.range(&value["value"], Registry::Int, &format!("{path}.value")),
            "entity_scores" => {
                if let Some(scores) = value["scores"].as_object() {
                    for (name, range) in scores {
                        self.range(range, Registry::Int, &format!("{path}.scores[{name}]"));
                    }
                }
            }
            _ => {}
        }
    }

    fn range(&mut self, value: &Value, registry: Registry, path: &str) {
        if value.is_string() || value.get("type").is_some() {
            self.holder(value, registry, path);
        } else {
            self.field(value, "min", registry, path);
            self.field(value, "max", registry, path);
        }
    }

    fn modifier(&mut self, value: &Value, path: &str) {
        self.field(value, "condition", Registry::Predicate, path);
        match value_kind(value) {
            "sequence" => self.set(
                &value["functions"],
                Registry::Modifier,
                &format!("{path}.functions"),
            ),
            "filtered" => {
                self.field(value, "on_pass", Registry::Modifier, path);
                self.field(value, "on_fail", Registry::Modifier, path);
            }
            "modify_contents" => self.field(value, "modifier", Registry::Modifier, path),
            "set_contents" => self.entries(&value["entries"], &format!("{path}.entries")),
            "set_loot_table" => {
                if let Some(name) = value["loot_table_id"].as_str() {
                    self.references.push(Reference {
                        registry: Registry::Table,
                        name: name.to_owned(),
                        path: format!("{path}.loot_table_id"),
                        tag: false,
                        follow: false,
                    });
                }
            }
            "set_count" => self.field(value, "count", Registry::Int, path),
            "enchant_with_levels" => self.field(value, "levels", Registry::Int, path),
            "set_ominous_bottle_amplifier" => self.field(value, "amplifier", Registry::Int, path),
            "set_random_dyes" => self.bind_field(value, "number_of_dyes", Registry::Int, path),
            "enchanted_count_increase" => self.field(value, "count", Registry::Float, path),
            "set_damage" => self.field(value, "damage", Registry::Float, path),
            "limit_count" => self.range(&value["limit"], Registry::Int, &format!("{path}.limit")),
            "set_enchantments" => {
                if let Some(entries) = value["enchantments"].as_object() {
                    for (id, provider) in entries {
                        self.holder(
                            provider,
                            Registry::Int,
                            &format!("{path}.enchantments[{id}]"),
                        );
                    }
                }
            }
            "set_attributes" | "set_stew_effect" => {
                let (field, provider, registry) = if value_kind(value) == "set_attributes" {
                    ("modifiers", "amount", Registry::Float)
                } else {
                    ("effects", "duration", Registry::Int)
                };
                if let Some(entries) = value[field].as_array() {
                    for (index, entry) in entries.iter().enumerate() {
                        self.field(
                            entry,
                            provider,
                            registry,
                            &format!("{path}.{field}[{index}]"),
                        );
                    }
                }
            }
            "set_custom_model_data" => {
                for (field, registry) in [("floats", Registry::Float), ("colors", Registry::Int)] {
                    if let Some(entries) = value[field]["values"].as_array() {
                        for (index, entry) in entries.iter().enumerate() {
                            self.holder(
                                entry,
                                registry,
                                &format!("{path}.{field}.values[{index}]"),
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn table(&mut self, value: &Value, path: &str) {
        self.field(value, "modifier", Registry::Modifier, path);
        if let Some(pools) = value["pools"].as_array() {
            for (index, pool) in pools.iter().enumerate() {
                let here = format!("{path}.pools[{index}]");
                self.field(pool, "condition", Registry::Predicate, &here);
                self.field(pool, "modifier", Registry::Modifier, &here);
                self.field(pool, "rolls", Registry::Int, &here);
                self.field(pool, "bonus_rolls", Registry::Float, &here);
                self.entries(&pool["entries"], &format!("{here}.entries"));
            }
        }
    }

    fn entries(&mut self, values: &Value, path: &str) {
        let Some(entries) = values.as_array() else {
            return;
        };
        for (index, entry) in entries.iter().enumerate() {
            let here = format!("{path}[{index}]");
            self.field(entry, "condition", Registry::Predicate, &here);
            self.field(entry, "modifier", Registry::Modifier, &here);
            match value_kind(entry) {
                "loot_table" => {
                    self.set(&entry["value"], Registry::Table, &format!("{here}.value"))
                }
                "slots" => self.field(entry, "slot_source", Registry::Slots, &here),
                "alternatives" | "sequence" | "group" => {
                    self.entries(&entry["children"], &format!("{here}.children"))
                }
                _ => {}
            }
        }
    }

    fn number(&mut self, value: &Value, registry: Registry, path: &str) {
        match value_kind(value) {
            "conditional" => {
                self.field(value, "condition", Registry::Predicate, path);
                self.field(value, "on_true", registry, path);
                self.field(value, "on_false", registry, path);
            }
            "number_dispatcher" => {
                if let Some(cases) = value["cases"].as_array() {
                    for (index, case) in cases.iter().enumerate() {
                        let here = format!("{path}.cases[{index}]");
                        self.field(case, "condition", Registry::Predicate, &here);
                        self.field(case, "value", registry, &here);
                    }
                }
                self.field(value, "default", registry, path);
            }
            "avg" | "min" | "max" | "add" | "mul" | "length" => {
                self.set(&value["inputs"], registry, &format!("{path}.inputs"));
            }
            "sub" | "div" | "mod" | "floor_div" | "floor_mod" => {
                self.field(value, "left", registry, path);
                self.field(value, "right", registry, path);
            }
            "pow" => {
                self.field(value, "base", registry, path);
                self.field(value, "exponent", registry, path);
            }
            "abs" | "ceil" | "floor" | "negate" | "round" | "truncate" | "cos" | "sin" | "sqrt" => {
                self.field(value, "input", registry, path);
            }
            "from_int" => self.field(value, "input", Registry::Int, path),
            "from_float" => self.field(value, "input", Registry::Float, path),
            "uniform" => {
                self.field(value, "min", registry, path);
                self.field(value, "max", registry, path);
            }
            "binomial" => {
                self.field(value, "n", Registry::Int, path);
                self.field(value, "p", Registry::Float, path);
            }
            "storage" | "score" => self.field(value, "fallback", registry, path),
            "weighted_list" => {
                if let Some(entries) = value["distribution"].as_array() {
                    for (index, entry) in entries.iter().enumerate() {
                        self.field(
                            entry,
                            "data",
                            registry,
                            &format!("{path}.distribution[{index}]"),
                        );
                    }
                }
            }
            _ => {}
        }
    }

    fn advancement(&mut self, value: &Value, path: &str) {
        let Some(criteria) = value["criteria"].as_object() else {
            return;
        };
        for (name, criterion) in criteria {
            let conditions = &criterion["conditions"];
            let here = format!("{path}.criteria[{name}].conditions");
            let trigger = criterion["trigger"].as_str().map(kind).unwrap_or("");
            if trigger == "impossible" {
                continue;
            }
            self.field(conditions, "player", Registry::Predicate, &here);
            let fields: &[&str] = match trigger {
                "bred_animals" => &["parent", "partner", "child"],
                "cured_zombie_villager" => &["zombie", "villager"],
                "effects_changed" => &["source"],
                "fall_after_explosion" => &["cause"],
                "lightning_strike" => &["lightning", "bystander"],
                "target_hit" => &["projectile"],
                "villager_trade" => &["villager"],
                "fishing_rod_hooked"
                | "player_killed_entity"
                | "entity_killed_player"
                | "kill_mob_near_sculk_catalyst"
                | "thrown_item_picked_up_by_entity"
                | "thrown_item_picked_up_by_player"
                | "player_hurt_entity"
                | "player_interacted_with_entity"
                | "player_sheared_equipment"
                | "summoned_entity"
                | "tame_animal" => &["entity"],
                "any_block_use"
                | "default_block_use"
                | "item_used_on_block"
                | "placed_block"
                | "allay_drop_item_on_block" => &["location"],
                _ => &[],
            };
            for field in fields {
                self.field(conditions, field, Registry::Predicate, &here);
            }
            if matches!(trigger, "channeled_lightning" | "killed_by_arrow")
                && let Some(victims) = conditions["victims"].as_array()
            {
                for (index, victim) in victims.iter().enumerate() {
                    self.holder(
                        victim,
                        Registry::Predicate,
                        &format!("{here}.victims[{index}]"),
                    );
                }
            }
        }
    }
}
