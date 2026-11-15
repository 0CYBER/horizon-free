use serde_json::Value;
type Entries = Vec<((String, String), Value, bool)>;
pub(super) fn filter(
    entries: &mut Entries,
    damage_already_selected: bool,
    conditions: &horizon_world::conditions::LoadConditions,
    kind: &str,
) {
    if !damage_already_selected {
        entries.retain(|((namespace, id), value, _)| {
            conditions.allows_parts(value, kind, namespace, id)
        });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn damage_winner_does_not_evaluate_discarded_root_definition() {
        let conditions = horizon_world::conditions::LoadConditions::new([]).with_lookup(
            std::collections::BTreeMap::from([("testmod".into(), "testmod".into())]),
            |_, _| None,
        );
        let mut root = vec![(
            ("testmod".into(), "damage".into()),
            serde_json::json!({"neoforge:conditions":[{"type":"testmod:nope"}]}),
            false,
        )];
        filter(&mut root, true, &conditions, "damage_type");
        assert_eq!(conditions.check_errors(), Ok(()));
        filter(&mut root, false, &conditions, "damage_type");
        assert!(
            conditions
                .check_errors()
                .unwrap_err()
                .contains("data/testmod/damage_type/damage.json")
        );
    }
}
