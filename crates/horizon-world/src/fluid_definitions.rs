//! Fluid definitions exported from the pinned game's registry, validated at load.
use crate::fluid::FluidState;
use std::{
    collections::{BTreeMap, HashMap},
    fmt, io,
};

/// Why prepared fluid definitions cannot be loaded.
#[derive(Debug)]
pub enum FluidDefinitionLoadError {
    /// The preparation predates this export.
    Missing,
    /// An entry, property, value or default is malformed.
    Invalid,
}
impl fmt::Display for FluidDefinitionLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "fluid_state_definitions {self:?}: re-run horizon-prepare"
        )
    }
}
impl std::error::Error for FluidDefinitionLoadError {}

/// One world fluid's validated definition and decoded default.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct FluidDefinition {
    /// Legal serialized values, in property-name order.
    pub properties: Vec<(String, Vec<String>)>,
    /// Decoded once, never reparsed by API queries.
    pub default: FluidState,
}

/// Decode the full export against the same registry; never publish a partial table.
pub fn load(
    value: &serde_json::Value,
    fluids: &[&str],
) -> io::Result<HashMap<String, FluidDefinition>> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Export {
        properties: BTreeMap<String, Vec<String>>,
        default: serde_json::Value,
    }
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            FluidDefinitionLoadError::Invalid,
        )
    };
    let entries = value
        .get("fluid_state_definitions")
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                FluidDefinitionLoadError::Missing,
            )
        })?
        .as_object()
        .ok_or_else(invalid)?;
    if entries.len() != fluids.len() {
        return Err(invalid());
    }
    fluids
        .iter()
        .map(|&id| {
            let export: Export =
                serde_json::from_value(entries.get(id).ok_or_else(invalid)?.clone())
                    .map_err(|_| invalid())?;
            let default = decode_exact(&export.default).ok_or_else(invalid)?;
            if default.kind().name() != id {
                return Err(invalid());
            }
            let defaults = export
                .default
                .get("properties")
                .and_then(serde_json::Value::as_object);
            if defaults.map_or(0, |p| p.len()) != export.properties.len() {
                return Err(invalid());
            }
            for (name, values) in &export.properties {
                if values.is_empty()
                    || values
                        .iter()
                        .enumerate()
                        .any(|(i, v)| values[..i].contains(v))
                    || !defaults
                        .and_then(|p| p.get(name))
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|v| values.iter().any(|allowed| allowed == v))
                {
                    return Err(invalid());
                }
                for value in values {
                    let mut candidate = export.default.clone();
                    candidate["properties"][name] = value.clone().into();
                    decode_exact(&candidate).ok_or_else(invalid)?;
                }
            }
            Ok((
                id.to_owned(),
                FluidDefinition {
                    properties: export.properties.into_iter().collect(),
                    default,
                },
            ))
        })
        .collect()
}

// The engine codec is lenient; preparation metadata must roundtrip exactly.
fn decode_exact(value: &serde_json::Value) -> Option<FluidState> {
    let state = FluidState::from_json(&value.to_string()).ok()?;
    let canonical: serde_json::Value = serde_json::from_str(&state.to_json().ok()?).ok()?;
    (canonical == *value).then_some(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_are_exact_and_errors_are_typed() {
        let mut value = serde_json::json!({"fluid_state_definitions":{"minecraft:water":{"properties":{"falling":["true","false"]},"default":{"id":"minecraft:water","properties":{"falling":"true"}}}}});
        let definitions = load(&value, &["minecraft:water"]).unwrap();
        assert!(definitions["minecraft:water"].default.is_source());
        assert!(definitions["minecraft:water"].default.falling());
        let cached = postcard::to_allocvec(&definitions).unwrap();
        let restored: HashMap<String, FluidDefinition> = postcard::from_bytes(&cached).unwrap();
        assert_eq!(
            restored["minecraft:water"].default,
            definitions["minecraft:water"].default
        );
        assert_eq!(
            restored["minecraft:water"].properties,
            definitions["minecraft:water"].properties
        );
        for properties in [
            serde_json::json!({"falling": []}),
            serde_json::json!({"falling": ["true", "true"]}),
            serde_json::json!({"falling": ["true", "bad"]}),
        ] {
            let mut malformed = value.clone();
            malformed["fluid_state_definitions"]["minecraft:water"]["properties"] = properties;
            assert!(load(&malformed, &["minecraft:water"]).is_err());
        }
        assert!(load(&value, &["minecraft:lava"]).is_err());
        for malformed in [
            serde_json::json!({"id":"water","properties":{"falling":"true"}}),
            serde_json::json!({"id":"minecraft:water","properties":{"falling":"bad"}}),
            serde_json::json!({"id":"minecraft:lava","properties":{"falling":"true"}}),
        ] {
            value["fluid_state_definitions"]["minecraft:water"]["default"] = malformed;
            let error = load(&value, &["minecraft:water"]).unwrap_err();
            assert!(matches!(
                error
                    .get_ref()
                    .and_then(|e| e.downcast_ref::<FluidDefinitionLoadError>()),
                Some(FluidDefinitionLoadError::Invalid)
            ));
        }
        let error = load(&serde_json::json!({}), &[]).unwrap_err();
        assert!(matches!(
            error
                .get_ref()
                .and_then(|e| e.downcast_ref::<FluidDefinitionLoadError>()),
            Some(FluidDefinitionLoadError::Missing)
        ));
    }
}
