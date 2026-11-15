use super::*;
pub(super) struct PreparedWorldReload {
    pub(super) structure_spawns: horizon_client::mobs::spawn_selection::StructureSpawns,
    pub(super) advancements: Arc<horizon_client::advancements::Tree>,
    pub(super) tables: Option<WorldTables>,
    pub(super) health: horizon_client::health::HealthResources,
    pub(super) parts: PreparedParts,
    pub(super) wire: Option<Arc<horizon_client::lan::data::SyncData>>,
    pub(super) datapack_registries: Option<mod_api::DatapackRegistrySet>,
}

pub(super) fn registry_versions(
    versions: Vec<(String, Vec<u8>)>,
    conditions: &horizon_world::conditions::LoadConditions,
    path: &str,
) -> Vec<(String, Vec<u8>)> {
    // Keep the existing built-in-only selection behavior. Registered callbacks
    // must never run on losing ordinary registry definitions.
    let versions = if conditions.has_registered() {
        versions.into_iter().next_back().into_iter().collect()
    } else {
        versions
    };
    versions
        .into_iter()
        .filter_map(|(pack, bytes)| {
            // The candidate refuses oversized entries without parsing them.
            if bytes.len() > mod_api::MAX_DATAPACK_ENTRY_BYTES {
                return Some((pack, bytes));
            }
            let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                return Some((pack, bytes));
            };
            if !conditions.allows_file(&json, path) {
                return None;
            }
            Some((pack, bytes))
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn load_conditions_registry_winner_and_builtin_order() {
        let conditions = horizon_world::conditions::LoadConditions::new([]).with_lookup(
            std::collections::BTreeMap::from([("testmod".into(), "testmod".into())]),
            |name, _| (name == "testmod:bad").then(|| Err("bad arguments".into())),
        );
        let lower = br#"{"neoforge:conditions":[{"type":"testmod:bad"}],"value":1}"#.to_vec();
        let upper = br#"{"value":2}"#.to_vec();
        assert_eq!(
            registry_versions(
                vec![("lower".into(), lower), ("upper".into(), upper.clone())],
                &conditions,
                "data/testmod/registry/value.json"
            ),
            vec![("upper".into(), upper)]
        );
        assert_eq!(conditions.check_errors(), Ok(()));
        let builtins = horizon_world::conditions::LoadConditions::vanilla();
        let first = br#"{"value":1}"#.to_vec();
        let disabled = br#"{"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"absent"}],"value":2}"#.to_vec();
        assert_eq!(
            registry_versions(
                vec![
                    ("first".into(), first.clone()),
                    ("disabled".into(), disabled)
                ],
                &builtins,
                "builtin.json"
            ),
            vec![("first".into(), first)]
        );
    }
}
