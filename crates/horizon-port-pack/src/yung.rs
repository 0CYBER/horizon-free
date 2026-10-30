//! YUNG's API's worldgen, carried as the mods write it (clean room:
//! `cleanroom/structures/spec-yungsapi.md`).
//!
//! The game reads `yungsapi:yung_jigsaw` structures and
//! `yungsapi:yung_single_element` pool elements as they ship
//! (`horizon_world::worldgen::structure::jigsaw::Draws::Yung`), so their files
//! pass through unchanged. What the game does not run yet is counted by name
//! in the carry report: the fields whose behaviour the specification leaves
//! open. YUNG's other structure, placement and processor types are left to the
//! general code, which lists them as unsupported or drops them.
use serde_json::Value;
use std::collections::BTreeMap;

const JIGSAW: &str = "yungsapi:yung_jigsaw";
const ELEMENT: &str = "yungsapi:yung_single_element";
/// A terrain adaptation that adapts nothing.
const NO_ADAPTATION: &str = "yungsapi:none";

fn count(lost: &mut BTreeMap<String, usize>, rule: String) {
    *lost.entry(rule).or_default() += 1;
}

/// Whether a field changes generation in a way the game does not run yet:
/// present, and not the value that changes nothing.
fn active(v: &Value, field: &str) -> bool {
    match &v[field] {
        Value::Null | Value::Bool(false) => false,
        Value::Object(o) => o.get("type").and_then(Value::as_str) != Some(NO_ADAPTATION),
        _ => true,
    }
}

/// A `yung_jigsaw` structure: `Some(false)` (carried as it is), else `None`.
pub(crate) fn upgrade(kind: &str, v: &Value, lost: &mut BTreeMap<String, usize>) -> Option<bool> {
    if kind != "structure" || v["type"] != JIGSAW {
        return None;
    }
    // spec-yungsapi §2: OPEN-Y1, OPEN-Y2 and §5.
    for field in [
        "adapt_noise",
        "max_y",
        "min_y",
        "enhanced_terrain_adaptation",
    ] {
        if active(v, field) {
            count(lost, format!("{JIGSAW} {field}"));
        }
    }
    Some(false)
}

/// A `yung_single_element` pool element, carried as it is; whether it was one.
pub(crate) fn upgrade_pool_element(element: &Value, lost: &mut BTreeMap<String, usize>) -> bool {
    if element["element_type"] != ELEMENT {
        return false;
    }
    // spec-yungsapi §4 and §5. `name`, `max_count`, `is_priority`,
    // `min_required_depth` and `ignore_bounds` are run (§3, §3.2).
    for field in ["condition", "enhanced_terrain_adaptation", "modifiers"] {
        if active(element, field) {
            count(lost, format!("{ELEMENT} {field}"));
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn jigsaws_and_elements_are_carried_as_written() {
        let mut lost = BTreeMap::new();
        let dungeon =
            json!({"type": JIGSAW, "start_pool": "a:b", "size": 10, "adapt_noise": false});
        assert_eq!(upgrade("structure", &dungeon, &mut lost), Some(false));
        assert!(lost.is_empty());
        let stronghold = json!({"type": JIGSAW, "max_y": 0,
            "enhanced_terrain_adaptation": {"type": "yungsapi:carved"}});
        assert_eq!(upgrade("structure", &stronghold, &mut lost), Some(false));
        assert_eq!(lost[&format!("{JIGSAW} max_y")], 1);
        assert_eq!(lost[&format!("{JIGSAW} enhanced_terrain_adaptation")], 1);
        assert_eq!(
            upgrade("structure", &json!({"type": "minecraft:jigsaw"}), &mut lost),
            None
        );
        assert_eq!(upgrade("structure_set", &dungeon, &mut lost), None);

        let mut lost = BTreeMap::new();
        let pile = json!({"element_type": ELEMENT, "name": "loot_pile_0", "max_count": 1,
            "enhanced_terrain_adaptation": {"type": NO_ADAPTATION}, "is_priority": true,
            "min_required_depth": 2, "ignore_bounds": true});
        assert!(upgrade_pool_element(&pile, &mut lost));
        assert!(lost.is_empty());
        let gated = json!({"element_type": ELEMENT, "condition": {"type": "yungsapi:not"}});
        assert!(upgrade_pool_element(&gated, &mut lost));
        assert_eq!(lost[&format!("{ELEMENT} condition")], 1);
        let single = json!({"element_type": "minecraft:single_pool_element"});
        assert!(!upgrade_pool_element(&single, &mut lost));
    }
}
