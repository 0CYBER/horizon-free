//! Load-phase condition handlers shared by native and wasm mods.
use crate::Identifier;
use std::{collections::BTreeMap, sync::Arc};
type Entries = BTreeMap<Identifier, (String, Arc<dyn LoadCondition>)>;

/// A condition's invalid arguments or a registration refusal.
// exhaustive: mods construct this error with a diagnostic message.
#[derive(Clone, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
pub struct ConditionError {
    /// Explanation included in the owning data file's reload diagnostic.
    pub message: String,
}

impl std::fmt::Display for ConditionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for ConditionError {}

impl ConditionError {
    /// A refusal with `message` as its diagnostic.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl From<(Identifier, String)> for ConditionError {
    fn from((id, message): (Identifier, String)) -> Self {
        Self {
            message: format!("{id}: {message}"),
        }
    }
}

/// A handler that reads the condition's JSON from a borrowed raw value.
/// Arguments are the JSON object without `type`; validate their shape and
/// return `ConditionError` for invalid mod input, never panic while decoding it.
/// Called once per entry per candidate reload, before publication. Return false
/// to omit the entry, or an error to keep the previous usable generation.
/// Registering a condition owns its namespace: unknown names there refuse the
/// reload. Foreign unknown ids anywhere inside `not`, `and` or `or` omit the
/// file, even in `or(true, unknown)`, with one ERROR-level log entry per file
/// per candidate reload naming its distinct unknown ids. The vanilla view, which
/// has no candidate store, logs on each evaluation instead.
/// A returned error or native panic refuses the candidate reload, naming the
/// file, condition and owner. Handlers must not mutate published data.
pub trait LoadCondition: Send + Sync + 'static {
    /// Read and validate the raw JSON arguments for one entry.
    fn test(&self, args: &serde_json::Value) -> Result<bool, ConditionError>;
}

/// Registrar borrowed during `register_load_conditions`.
pub trait LoadConditions {
    /// Register a handler under the dispatching mod's namespace.
    /// Duplicate names are refused with both owners in the diagnostic.
    fn register_handler(
        &mut self,
        name: Identifier,
        condition: Arc<dyn LoadCondition>,
    ) -> Result<(), ConditionError>;
}
impl dyn LoadConditions + '_ {
    /// Store a concrete handler through the shared native and wasm route.
    pub fn register(
        &mut self,
        name: Identifier,
        condition: impl LoadCondition,
    ) -> Result<(), ConditionError> {
        self.register_handler(name, Arc::new(condition))
    }
}

// no wire: process-local callback storage; handlers cross through generated dispatch.
/// Engine-owned declarations retaining the dispatch owner of each handler.
// exhaustive: private fields prevent mod construction or destructuring.
#[derive(Default)]
pub struct LoadConditionDeclarations {
    entries: Entries,
    errors: Vec<ConditionError>,
}
impl LoadConditionDeclarations {
    /// Finish registration, refusing even errors a listener ignored.
    // hidden: engine preparation consumes the declaration table.
    #[doc(hidden)]
    pub fn finish(self) -> Result<Entries, ConditionError> {
        if let Some(error) = self.errors.into_iter().next() {
            return Err(error);
        }
        Ok(self.entries)
    }
}
impl LoadConditions for LoadConditionDeclarations {
    fn register_handler(
        &mut self,
        name: Identifier,
        condition: Arc<dyn LoadCondition>,
    ) -> Result<(), ConditionError> {
        let owner = crate::content::crash::current_native_mod()
            .expect("load condition registration inside mod dispatch");
        if let Some((previous, _)) = self.entries.get(&name) {
            let error = ConditionError {
                message: format!("condition {name} registered by both {previous} and {owner}"),
            };
            self.errors.push(error.clone());
            return Err(error);
        }
        if name.namespace() != owner {
            let error = ConditionError {
                message: format!(
                    "mod {owner} cannot register condition {name} outside its namespace"
                ),
            };
            self.errors.push(error.clone());
            return Err(error);
        }
        self.entries.insert(name, (owner.to_owned(), condition));
        Ok(())
    }
}

#[cfg(test)]
mod error_tests {
    #[test]
    fn condition_error_supports_standard_error_propagation() {
        fn propagate() -> Result<(), Box<dyn std::error::Error>> {
            let value: Result<(), super::ConditionError> =
                Err(super::ConditionError::new("bad value"));
            value?;
            Ok(())
        }
        assert_eq!(propagate().unwrap_err().to_string(), "bad value");
    }
}
