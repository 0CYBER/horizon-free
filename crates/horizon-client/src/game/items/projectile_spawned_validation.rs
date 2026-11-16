//! Admission checks for the subset of projectile loot contexts implemented here.
use serde_json::Value;

pub(super) fn requirements_supported(value: &Value) -> bool {
    if value.is_null() {
        return true;
    }
    match value["condition"].as_str() {
        Some("minecraft:entity_properties") => {
            value["entity"] == "this"
                && value["predicate"].as_object().is_some_and(|p| {
                    p.keys()
                        .all(|k| k == "type" || k == "minecraft:entity_type")
                })
        }
        Some("minecraft:random_chance") => true,
        Some("minecraft:all_of" | "minecraft:any_of") => value["terms"]
            .as_array()
            .is_some_and(|terms| terms.iter().all(requirements_supported)),
        Some("minecraft:inverted") => requirements_supported(&value["term"]),
        _ => false,
    }
}

pub(super) fn valid_ranges(value: &Value) -> bool {
    match value {
        Value::Object(fields) => {
            if value["type"] == "minecraft:clamped" {
                let Some(min) = value["min"].as_f64() else {
                    return false;
                };
                let Some(max) = value["max"].as_f64() else {
                    return false;
                };
                if !(min.is_finite() && max.is_finite() && max > min) {
                    return false;
                }
            }
            fields.values().all(valid_ranges)
        }
        Value::Array(values) => values.iter().all(valid_ranges),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn fire_flags_refuse_before_effect_admission() {
        assert!(!requirements_supported(
            &json!({"condition":"minecraft:entity_properties","entity":"this","predicate":{"minecraft:flags":{"is_on_fire":true}}})
        ));
        assert!(requirements_supported(
            &json!({"condition":"minecraft:entity_properties","entity":"this","predicate":{"type":"minecraft:arrow"}})
        ));
    }
    #[test]
    fn nested_invalid_clamp_refuses_before_evaluation() {
        assert!(!valid_ranges(
            &json!({"duration":{"type":"minecraft:clamped","value":1,"min":2,"max":0}})
        ));
        assert!(!valid_ranges(
            &json!({"type":"minecraft:clamped","value":1,"min":2,"max":2})
        ));
        assert!(valid_ranges(
            &json!({"type":"minecraft:clamped","value":1,"min":0,"max":2})
        ));
    }
}
