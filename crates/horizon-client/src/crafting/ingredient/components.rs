//! Component patches are validated and decoded once, before publication.
use super::*;
use horizon_core::nbt::{Compound, Tag};
use horizon_items::item_data::ingredient_components::{component_equal, removed, visit_components};
use std::collections::HashMap;

/// Decoded patch and item prototypes shared by concrete candidate checks.
#[derive(Clone, Debug)]
pub struct ComponentPredicate {
    expected: ItemStack,
    retained: Compound,
    patched: HashSet<String>,
    removed: Vec<String>,
    strict: bool,
    defaults: HashMap<String, (ItemStack, Vec<String>)>,
}
fn decode(patch: Compound) -> ItemStack {
    let (damage, extra) = crate::item_data::decode_components(Some(&Tag::Compound(patch)));
    ItemStack {
        item: "minecraft:air".into(),
        count: 1,
        damage,
        extra,
    }
}
fn component_id(id: &str) -> Result<String, String> {
    horizon_core::identifier::Identifier::parse(id)
        .map(|id| id.to_string())
        .map_err(|error| error.to_string())
}
pub(super) fn validate(rule: &IngredientRule, items: &Items) -> Result<(), String> {
    let IngredientRule::Components { patch, removed, .. } = rule;
    let known = |id: &str| {
        items
            .component_registry
            .ids("minecraft:data_component_type")
            .iter()
            .any(|name| name == id)
            || items
                .component_registry
                .persistent_components
                .iter()
                .any(|name| name == id)
            || horizon_command::item::DECODED_COMPONENTS.contains(&id)
            || horizon_protocol::item::custom::id(id).is_some()
    };
    let mut visited = HashSet::new();
    for id in patch
        .keys()
        .map(|key| key.to_string_lossy())
        .chain(removed.iter().cloned())
    {
        let id = component_id(&id)?;
        if !visited.insert(id.clone()) {
            return Err(format!("repeated component {id}"));
        }
        if !known(&id) {
            return Err(format!("unregistered component {id}"));
        }
    }
    for (id, value) in patch {
        let id = component_id(&id.to_string_lossy())?;
        if id.starts_with("minecraft:") {
            horizon_command::item::decoded_component_value(&id, value, &items.component_registry)
                .map_err(|error| format!("component {id}: {error}"))?;
        } else {
            // component_types declares persistent, arbitrary NBT values.
            horizon_core::nbt::validate_component_value(value)
                .map_err(|error| format!("component {id}: {error}"))?;
        }
    }
    Ok(())
}
impl ComponentPredicate {
    pub(super) fn compile(
        rule: &IngredientRule,
        candidates: &HashSet<String>,
        items: &Items,
    ) -> Result<Self, String> {
        validate(rule, items)?;
        let IngredientRule::Components {
            patch,
            removed,
            strict,
        } = rule;
        let decoded = patch
            .iter()
            .map(|(id, value)| {
                let name = component_id(&id.to_string_lossy())?;
                let value = if name.starts_with("minecraft:") {
                    horizon_command::item::decoded_component_value(
                        &name,
                        value,
                        &items.component_registry,
                    )?
                } else {
                    value.clone()
                };
                Ok((name.as_str().into(), value))
            })
            .collect::<Result<Compound, String>>()?;
        let expected = decode(decoded.clone());
        let defaults = candidates
            .iter()
            .map(|id| {
                let values = items
                    .component_defaults
                    .get(id)
                    .cloned()
                    .unwrap_or_default();
                let names = values.keys().map(|id| id.to_string_lossy()).collect();
                (id.clone(), (decode(values), names))
            })
            .collect();
        Ok(Self {
            expected,
            retained: decoded,
            patched: patch
                .keys()
                .map(|key| component_id(&key.to_string_lossy()))
                .collect::<Result<_, _>>()?,
            removed: removed
                .iter()
                .map(|id| component_id(id))
                .collect::<Result<_, _>>()?,
            strict: *strict,
            defaults,
        })
    }
    pub(super) fn matches(&self, stack: &ItemStack) -> bool {
        if stack.count == 0 || stack.item.as_ref() == "minecraft:air" {
            return false;
        }
        let default = self.defaults.get(stack.item.as_ref());
        let effective = |id: &str| {
            if stack.carries_component(None, id) {
                Some(stack)
            } else if removed(stack, id) {
                None
            } else {
                default
                    .filter(|(_, names)| names.iter().any(|name| name == id))
                    .map(|(stack, _)| stack)
            }
        };
        if !self.patched.iter().all(|id| {
            effective(id).is_some_and(|actual| component_equal(id, &self.expected, actual))
        }) || self.removed.iter().any(|id| effective(id).is_some())
        {
            return false;
        }
        if self.strict {
            let mut valid = true;
            visit_components(stack, |id| valid &= self.patched.contains(id));
            if !valid
                || default.is_some_and(|(_, names)| {
                    names
                        .iter()
                        .any(|id| effective(id).is_some() && !self.patched.contains(id))
                })
            {
                return false;
            }
        }
        true
    }
    pub(super) fn shown(&self, order: &[String]) -> Vec<ItemStack> {
        order
            .iter()
            .map(|id| {
                // ItemStack carries a patch: its renderer reads the prototype itself.
                let mut values = self.retained.clone();
                for id in &self.removed {
                    values.insert(
                        format!("!{id}").as_str().into(),
                        Tag::Compound(Default::default()),
                    );
                }
                let mut stack = decode(values);
                stack.item = id.as_str().into();
                stack
            })
            .collect()
    }
}

/// JSON optional record fields treat null as absent before the strict NBT boundary.
pub(super) fn json_patch(data: &serde_json::Map<String, Value>) -> Option<Compound> {
    const OPTIONAL_FIELDS: &[(&str, &[&str])] = &[
        (
            "weapon",
            &["item_damage_per_attack", "disable_blocking_for_seconds"],
        ),
        ("food", &["can_always_eat"]),
        (
            "custom_model_data",
            &["floats", "flags", "strings", "colors"],
        ),
        ("use_cooldown", &["cooldown_group"]),
        ("lodestone_tracker", &["target", "tracked"]),
        ("container_loot", &["seed"]),
        ("tooltip_display", &["hide_tooltip", "hidden_components"]),
        ("attack_animation", &["type", "duration"]),
        (
            "use_effects",
            &["can_sprint", "interact_vibrations", "speed_multiplier"],
        ),
        (
            "attack_range",
            &[
                "min_reach",
                "max_reach",
                "min_creative_reach",
                "max_creative_reach",
                "hitbox_margin",
                "mob_factor",
            ],
        ),
    ];
    let mut normalized = None;
    for (id, value) in data {
        let component = id.strip_prefix("minecraft:").unwrap_or(id);
        let optional = OPTIONAL_FIELDS
            .iter()
            .find(|(name, _)| *name == component)
            .map_or(&[][..], |(_, fields)| *fields);
        if let Some(record) = value.as_object() {
            // Integral JSON-to-NBT conversion would discard weapon's forbidden sign.
            if component == "weapon"
                && record
                    .get("disable_blocking_for_seconds")
                    .and_then(Value::as_f64)
                    .is_some_and(|seconds| seconds == 0.0 && seconds.is_sign_negative())
            {
                return None;
            }
            let mut normalized_record = None;
            for field in optional {
                if record.get(*field).is_some_and(Value::is_null) {
                    normalized_record
                        .get_or_insert_with(|| record.clone())
                        .remove(*field);
                }
            }
            if let Some(record) = normalized_record {
                normalized
                    .get_or_insert_with(|| data.clone())
                    .insert(id.clone(), Value::Object(record));
            }
        }
    }
    crate::json_nbt::json_to_compound(normalized.as_ref().unwrap_or(data)).ok()
}

// Keep conversion failures alongside codec errors at the recipe publication boundary.
pub(super) fn validate_recipe_components(value: &Value, items: &Items) -> Result<(), String> {
    let object = value.as_object().ok_or("malformed components ingredient")?;
    let keys = object
        .get("components")
        .and_then(Value::as_object)
        .map(|patch| {
            patch
                .keys()
                .map(|id| id.strip_prefix('!').unwrap_or(id).to_owned())
                .collect()
        })
        .unwrap_or_default();
    let ids = IngredientRule::Components {
        patch: Default::default(),
        removed: keys,
        strict: false,
    };
    validate(&ids, items)?;
    let conversion_error = std::cell::RefCell::new(None);
    let parsed = WalkedIngredient::walk(
        value,
        &items.item_tag_order,
        &|_| None,
        &resolved_items,
        &|data| {
            let mut converted = horizon_core::nbt::Compound::new();
            for (id, value) in data {
                let single = serde_json::Map::from_iter([(id.clone(), value.clone())]);
                match json_patch(&single) {
                    Some(field) => converted.extend(field),
                    None => {
                        *conversion_error.borrow_mut() =
                            Some(format!("component {id}: malformed JSON payload"));
                        return None;
                    }
                }
            }
            Some(converted)
        },
    );
    if let Some(error) = conversion_error.into_inner() {
        return Err(error);
    }
    let rule = parsed
        .and_then(|parsed| parsed.rule)
        .ok_or_else(|| "malformed components ingredient".to_owned())?;
    validate(&rule, items)
}

pub(super) fn validate_recipe(value: &Value, items: &Items, recipe: &str) -> bool {
    match value {
        Value::Object(object) => match ingredient_kind(object) {
            Some("neoforge:components") => {
                if let Err(error) = validate_recipe_components(value, items) {
                    log::warn!("recipe {recipe} rejected: {error}");
                    return false;
                }
                true
            }
            Some("neoforge:compound" | "neoforge:intersection") => ["children", "ingredients"]
                .iter()
                .filter_map(|key| object.get(*key))
                .all(|value| validate_recipe(value, items, recipe)),
            Some("neoforge:difference") => ["base", "subtracted"]
                .iter()
                .filter_map(|key| object.get(*key))
                .all(|value| validate_recipe(value, items, recipe)),
            Some("neoforge:custom_display") => object
                .get("base")
                .is_none_or(|value| validate_recipe(value, items, recipe)),
            _ => {
                crate::crafting::INGREDIENT_KEYS
                    .iter()
                    .filter_map(|key| object.get(*key))
                    .all(|value| validate_recipe(value, items, recipe))
                    && crate::crafting::INGREDIENT_MAPS
                        .iter()
                        .filter_map(|key| object.get(*key).and_then(Value::as_object))
                        .flat_map(|map| map.values())
                        .all(|value| validate_recipe(value, items, recipe))
            }
        },
        Value::Array(values) => values
            .iter()
            .all(|value| validate_recipe(value, items, recipe)),
        _ => true,
    }
}
