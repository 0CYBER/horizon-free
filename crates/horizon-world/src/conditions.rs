//! NeoForge's load conditions (`neoforge:conditions`): a data file that has
//! the key loads only when every condition in it holds, and is silently
//! absent otherwise (NeoForge's `ICondition`). Mods' packs lean on it
//! for cross-mod recipes — a recipe for another mod's material only makes
//! sense to load when that mod's data is in the root.
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Enabled package IDs captured by the engine for one owning data load.
/// Readers share this immutable view; they never recover IDs from namespaces.
/// Resource selection happens before evaluating ordinary JSON files. Stacked
/// tags and data maps instead evaluate each layer before merge or replace.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct LoadConditions {
    mods: Vec<String>,
    registered: Option<RegisteredLookup>,
    diagnostics: Diagnostics,
}

impl LoadConditions {
    /// A vanilla data load has the built-in package only.
    pub const fn vanilla() -> Self {
        Self {
            mods: Vec::new(),
            registered: None,
            diagnostics: Diagnostics::new(),
        }
    }

    /// Capture IDs visible to the owning load's side. The engine supplies its
    /// enabled-mod inventory's server view for logical-server data.
    pub fn new<'a>(ids: impl IntoIterator<Item = &'a str>) -> Self {
        let mut mods: Vec<_> = ids
            .into_iter()
            .filter(|id| *id != "minecraft")
            .map(str::to_owned)
            .collect();
        mods.sort();
        mods.dedup();
        Self {
            mods,
            registered: None,
            diagnostics: Diagnostics::default(),
        }
    }

    /// Use the recipe source's inventory while retaining this candidate's
    /// registered handlers, evaluation cache and diagnostics.
    pub fn with_mod_inventory<'a>(&self, ids: impl IntoIterator<Item = &'a str>) -> Self {
        let mut view = self.clone();
        view.mods = Self::new(ids).mods;
        if view.mods != self.mods {
            view.diagnostics = match &self.registered {
                Some(registered) => registered
                    .0
                    .inventories
                    .lock()
                    .unwrap()
                    .entry(view.mods.clone())
                    .or_default()
                    .clone(),
                // Without handlers there are no cached answers or errors;
                // keep foreign diagnostic deduplication shared across the candidate.
                None => self.diagnostics.clone(),
            };
        }
        view
    }

    /// Retain unknown-condition diagnostics for a load that must refuse them.
    pub fn with_diagnostics(mut self) -> Self {
        if self.diagnostics.0.is_none() {
            self.diagnostics = Diagnostics::default();
        }
        self
    }

    /// Refuse ordinary errors and unknown conditions belonging to this resource folder.
    /// Other resource folders retain their existing diagnostic policy.
    pub fn check_resource_errors(&self, folder: &str) -> Result<(), String> {
        self.check_errors()?;
        if let Some(state) = &self.diagnostics.0 {
            let state = state.lock().unwrap();
            let errors: Vec<_> = state
                .foreign_errors
                .iter()
                .filter(|(path, _)| path.split('/').nth(2) == Some(folder))
                .map(|(_, message)| message.as_str())
                .collect();
            if !errors.is_empty() {
                return Err(errors.join("; "));
            }
        }
        Ok(())
    }

    pub fn mod_loaded(&self, id: &str) -> bool {
        id == "minecraft"
            || self
                .mods
                .binary_search_by(|loaded| loaded.as_str().cmp(id))
                .is_ok()
    }

    /// Attach the engine's immutable registration table to this candidate load.
    pub fn with_lookup(
        mut self,
        owned: BTreeMap<String, String>,
        lookup: impl Fn(&str, &Value) -> Option<Result<bool, String>> + Send + Sync + 'static,
    ) -> Self {
        if self.diagnostics.0.is_none() {
            self.diagnostics = Diagnostics::default();
        }
        self.registered = Some(RegisteredLookup(std::sync::Arc::new(LookupState {
            lookup: Box::new(lookup),
            owned,
            inventories: std::sync::Mutex::new(BTreeMap::from([(
                self.mods.clone(),
                self.diagnostics.clone(),
            )])),
        })));
        self
    }

    /// Whether this candidate has any registered condition handlers.
    pub fn has_registered(&self) -> bool {
        self.registered.is_some()
    }

    /// Unknown condition kinds retain the built-in fail-closed behavior.
    pub fn allows(&self, file: &Value) -> bool {
        self.allows_file(file, "data file")
    }

    /// Evaluate a resource, constructing its path only for registered handlers or diagnostics.
    pub fn allows_id(&self, file: &Value, kind: &str, id: &str) -> bool {
        self.allows_context(file, || {
            let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
            format!("data/{namespace}/{kind}/{path}.json")
        })
    }

    /// Evaluate a resource whose namespace and path are stored separately.
    pub fn allows_parts(&self, file: &Value, kind: &str, namespace: &str, path: &str) -> bool {
        self.allows_context(file, || format!("data/{namespace}/{kind}/{path}.json"))
    }

    /// Evaluate a folder resource without joining paths in the no-mod fast path.
    pub fn allows_path(&self, file: &Value, directory: &std::path::Path, name: &str) -> bool {
        self.allows_context(file, || {
            directory.join(format!("{name}.json")).display().to_string()
        })
    }

    /// Evaluate once per entry and payload, outside the diagnostics mutex.
    pub fn allows_file(&self, file: &Value, path: &str) -> bool {
        self.allows_context(file, || path.to_owned())
    }

    fn allows_context(&self, file: &Value, context: impl FnOnce() -> String) -> bool {
        let Some(payload) = file.get("neoforge:conditions") else {
            return true;
        };
        let loaded = |id: &str| self.mod_loaded(id);
        let Some(registered) = &self.registered else {
            // The ordinary path uses only borrowed nodes and a boolean. No path,
            // argument copy, name set, cache key or diagnostics lock is needed.
            let mut unknown = false;
            let answer = evaluate(file, &loaded, &|_, _| None, &BTreeMap::new(), &mut |_| {
                unknown = true
            })
            .expect("empty lookup cannot fail");
            if !unknown {
                return answer;
            }
            let mut names = BTreeSet::new();
            let _ = evaluate(file, &loaded, &|_, _| None, &BTreeMap::new(), &mut |name| {
                names.insert(name.to_owned());
            });
            self.report_unknown(&context(), names);
            return answer;
        };
        let path = context();
        let path = if path.starts_with("data/") {
            path.as_str()
        } else {
            path.rfind("/data/")
                .map_or(path.as_str(), |index| &path[index + 1..])
        };
        let state = self
            .diagnostics
            .0
            .as_ref()
            .expect("owning candidate diagnostics");
        let cell = state
            .lock()
            .unwrap()
            .cache
            .entry((path.to_owned(), payload.to_string()))
            .or_default()
            .clone();
        *cell.get_or_init(|| {
            let mut unknown = BTreeSet::new();
            let result = evaluate(
                file,
                &loaded,
                &*registered.0.lookup,
                &registered.0.owned,
                &mut |name| {
                    unknown.insert(name.to_owned());
                },
            );
            if !unknown.is_empty() {
                self.report_unknown(path, unknown);
            }
            match result {
                Ok(answer) => answer,
                Err(error) => {
                    state
                        .lock()
                        .unwrap()
                        .errors
                        .push(format!("{path}: {error}"));
                    false
                }
            }
        })
    }

    fn report_unknown(&self, path: &str, names: BTreeSet<String>) {
        let diagnostic = || {
            format!(
                "{path}: unknown load conditions [{}]; file left out",
                names.into_iter().collect::<Vec<_>>().join(", ")
            )
        };
        if let Some(state) = &self.diagnostics.0 {
            let mut state = state.lock().unwrap();
            if !state.foreign_errors.contains_key(path) {
                let diagnostic = diagnostic();
                log::error!("{diagnostic}");
                state.foreign_errors.insert(path.to_owned(), diagnostic);
            }
        } else {
            log::error!("{}", diagnostic());
        }
    }

    /// Non-fatal foreign-namespace errors emitted by this candidate, one per file.
    pub fn foreign_errors(&self) -> Vec<String> {
        self.diagnostics
            .0
            .as_ref()
            .map(|state| {
                state
                    .lock()
                    .unwrap()
                    .foreign_errors
                    .values()
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Refuse a failed candidate before publishing any of its tables.
    pub fn check_errors(&self) -> Result<(), String> {
        if let Some(state) = self.diagnostics.0.as_ref() {
            let state = state.lock().unwrap();
            if !state.errors.is_empty() {
                return Err(state.errors.join("; "));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
struct CandidateDiagnostics {
    cache: BTreeMap<(String, String), std::sync::Arc<std::sync::OnceLock<bool>>>,
    foreign_errors: BTreeMap<String, String>,
    errors: Vec<String>,
}
// Diagnostics do not contribute to a resource generation's cache identity.
#[derive(Clone, Debug)]
struct Diagnostics(Option<std::sync::Arc<std::sync::Mutex<CandidateDiagnostics>>>);
impl Diagnostics {
    const fn new() -> Self {
        Self(None)
    }
}
impl Default for Diagnostics {
    fn default() -> Self {
        Self(Some(Default::default()))
    }
}
impl PartialEq for Diagnostics {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl Eq for Diagnostics {}
impl std::hash::Hash for Diagnostics {
    fn hash<H: std::hash::Hasher>(&self, _: &mut H) {}
}

/// A lower-layer lookup over a raw condition node, including `type`.
/// `None` means the engine has no handler for this name; the engine removes
/// `type` only after finding a handler, avoiding argument copies without mods.
pub type ConditionLookup<'a> = dyn Fn(&str, &Value) -> Option<Result<bool, String>> + 'a;
type SharedLookup = dyn Fn(&str, &Value) -> Option<Result<bool, String>> + Send + Sync;
struct LookupState {
    inventories: std::sync::Mutex<BTreeMap<Vec<String>, Diagnostics>>,
    lookup: Box<SharedLookup>,
    owned: BTreeMap<String, String>,
}
#[derive(Clone)]
struct RegisteredLookup(std::sync::Arc<LookupState>);
impl std::fmt::Debug for RegisteredLookup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RegisteredLookup")
    }
}
impl PartialEq for RegisteredLookup {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for RegisteredLookup {}
impl std::hash::Hash for RegisteredLookup {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(&std::sync::Arc::as_ptr(&self.0), state);
    }
}

/// Whether a package is enabled for the owning data load.
pub type ModLoaded<'a> = dyn Fn(&str) -> bool + 'a;

/// Evaluate built-in conditions with an empty registration lookup.
pub fn conditions_hold(v: &Value, loaded: &ModLoaded) -> bool {
    conditions_hold_with_lookup(v, loaded, &|_, _| None, &BTreeMap::new())
        .expect("empty condition lookup cannot fail")
}

/// The shared evaluator, including engine-provided registered conditions.
/// Unknown nodes propagate through combinators without becoming a passing `not`.
pub fn conditions_hold_with_lookup(
    v: &Value,
    loaded: &ModLoaded,
    lookup: &ConditionLookup,
    owned: &BTreeMap<String, String>,
) -> Result<bool, String> {
    evaluate(v, loaded, lookup, owned, &mut |_| {})
}

fn evaluate(
    v: &Value,
    loaded: &ModLoaded,
    lookup: &ConditionLookup,
    owned: &BTreeMap<String, String>,
    unknown: &mut dyn FnMut(&str),
) -> Result<bool, String> {
    fn hold(
        c: &Value,
        loaded: &ModLoaded,
        lookup: &ConditionLookup,
        owned: &BTreeMap<String, String>,
        unknown: &mut dyn FnMut(&str),
    ) -> Result<Option<bool>, String> {
        Ok(match c["type"].as_str() {
            Some("neoforge:mod_loaded") => Some(c["modid"].as_str().is_some_and(loaded)),
            Some("neoforge:true") => Some(true),
            Some("neoforge:not") => hold(&c["value"], loaded, lookup, owned, unknown)?.map(|v| !v),
            Some(kind @ ("neoforge:and" | "neoforge:or")) => {
                // Visit every child: an unknown node invalidates the whole file,
                // including after a sibling already determines its boolean result.
                let mut answer = kind == "neoforge:and";
                let mut known = true;
                for child in c["values"].as_array().into_iter().flatten() {
                    match hold(child, loaded, lookup, owned, unknown)? {
                        Some(value) => {
                            if kind == "neoforge:and" {
                                answer &= value;
                            } else {
                                answer |= value;
                            }
                        }
                        None => known = false,
                    }
                }
                known.then_some(answer)
            }
            Some(name) => match lookup(name, c) {
                Some(answer) => Some(answer?),
                None => {
                    if let Some(owner) = name
                        .split_once(':')
                        .and_then(|(namespace, _)| owned.get(namespace))
                    {
                        return Err(format!(
                            "unknown condition {name} in namespace registered by mod {owner}"
                        ));
                    }
                    unknown(name);
                    None
                }
            },
            None => None,
        })
    }
    let mut allowed = true;
    for condition in v["neoforge:conditions"].as_array().into_iter().flatten() {
        allowed &= hold(condition, loaded, lookup, owned, unknown)?.unwrap_or(false);
    }
    Ok(allowed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn load_conditions_capture_package_ids_and_keep_their_generation() {
        let mut enabled = vec!["code_only".to_owned()];
        let old = LoadConditions::new(enabled.iter().map(String::as_str));
        enabled.clear();
        let next = LoadConditions::new(enabled.iter().map(String::as_str));
        let requires_code = json!({"neoforge:conditions":[{
            "type":"neoforge:mod_loaded", "modid":"code_only"
        }]});
        assert!(old.allows(&requires_code));
        assert!(!next.allows(&requires_code));
        assert!(old.mod_loaded("minecraft") && next.mod_loaded("minecraft"));
        assert!(!old.mod_loaded("resource_namespace"));
        assert!(old.allows(&json!({"values":[]})));
        assert!(!old.allows(&json!({"neoforge:conditions":[{"type":"future:condition"}]})));
    }

    #[test]
    fn load_condition_cache_identity_uses_membership_not_input_order() {
        use std::hash::{DefaultHasher, Hash, Hasher};
        let key = |conditions: &LoadConditions| {
            let mut hasher = DefaultHasher::new();
            conditions.hash(&mut hasher);
            hasher.finish()
        };
        assert_eq!(
            key(&LoadConditions::new(["b", "a", "a"])),
            key(&LoadConditions::new(["a", "b"]))
        );
        assert_ne!(
            key(&LoadConditions::new(["a"])),
            key(&LoadConditions::vanilla())
        );
        assert_eq!(
            LoadConditions::new(["minecraft"]),
            LoadConditions::vanilla()
        );
    }

    fn loaded(id: &str) -> bool {
        matches!(id, "minecraft" | "create")
    }

    #[test]
    fn a_file_without_conditions_loads() {
        assert!(conditions_hold(
            &json!({"type": "minecraft:crafting"}),
            &loaded
        ));
    }

    #[test]
    fn mod_loaded_follows_the_answer_and_minecraft_always_holds() {
        assert!(conditions_hold(
            &json!({"neoforge:conditions": [{"type": "neoforge:mod_loaded", "modid": "create"}]}),
            &loaded
        ));
        assert!(!conditions_hold(
            &json!({"neoforge:conditions": [{"type": "neoforge:mod_loaded", "modid": "ic2"}]}),
            &loaded
        ));
        assert!(conditions_hold(
            &json!({"neoforge:conditions": [{"type": "neoforge:mod_loaded", "modid": "minecraft"}]}),
            &loaded
        ));
    }

    #[test]
    fn not_and_or_combine_and_all_must_hold() {
        let not = json!({"type": "neoforge:not", "value": {"type": "neoforge:mod_loaded", "modid": "ic2"}});
        let mut c = serde_json::Map::new();
        c.insert("neoforge:conditions".to_owned(), json!([not]));
        assert!(conditions_hold(&Value::Object(c.clone()), &loaded));
        let both = json!([
            {"type": "neoforge:mod_loaded", "modid": "create"},
            {"type": "neoforge:or", "values": [
                {"type": "neoforge:mod_loaded", "modid": "ic2"},
                {"type": "neoforge:true"}
            ]}
        ]);
        c.insert("neoforge:conditions".to_owned(), both);
        assert!(conditions_hold(&Value::Object(c), &loaded));
        // One failing condition in the list fails the file.
        let failing = json!([
            {"type": "neoforge:true"},
            {"type": "neoforge:mod_loaded", "modid": "ic2"}
        ]);
        let mut f = serde_json::Map::new();
        f.insert("neoforge:conditions".to_owned(), failing);
        assert!(!conditions_hold(&Value::Object(f), &loaded));
    }

    #[test]
    fn an_unknown_kind_nested_in_the_tree_leaves_the_file_out() {
        // `not` of `not` of an unknown: the unknown is two levels down.
        let nested_not = json!({
            "neoforge:conditions": [
                {"type": "neoforge:not", "value":
                    {"type": "neoforge:not", "value":
                        {"type": "example:unknown"}}}
            ]
        });
        assert!(!conditions_hold(&nested_not, &loaded));
        // `or` with a `true` beside a `not` of an unknown: one child is
        // known-true, but the other's subtree holds an unknown.
        let nested_or = json!({
            "neoforge:conditions": [
                {"type": "neoforge:or", "values": [
                    {"type": "neoforge:true"},
                    {"type": "neoforge:not", "value":
                        {"type": "example:unknown"}}
                ]}
            ]
        });
        assert!(!conditions_hold(&nested_or, &loaded));
    }

    #[test]
    fn an_unknown_condition_does_not_load_the_file() {
        let mut c = serde_json::Map::new();
        c.insert(
            "neoforge:conditions".to_owned(),
            json!([{"type": "neoforge:future_thing"}]),
        );
        assert!(!conditions_hold(&Value::Object(c), &loaded));
    }
}

#[cfg(test)]
#[path = "condition_registry_tests.rs"]
mod registry_tests;

#[cfg(test)]
#[path = "condition_log_tests.rs"]
mod log_tests;

#[cfg(test)]
mod recipe_error_tests {
    use super::*;
    #[test]
    fn recipe_load_escalates_unknown_conditions_only_for_its_resource_folder() {
        let conditions = LoadConditions::new([])
            .with_diagnostics()
            .with_lookup(BTreeMap::new(), |_, _| None);
        let file = serde_json::json!({"neoforge:conditions":[{"type":"absent:condition"}]});
        assert!(!conditions.allows_file(&file, "data/example/tags/item/flag.json"));
        assert_eq!(conditions.check_errors(), Ok(()));
        assert_eq!(conditions.check_resource_errors("recipe"), Ok(()));
        assert!(!conditions.allows_file(&file, "data/example/recipe/deep/data/entry.json"));
        let error = conditions.check_resource_errors("recipe").unwrap_err();
        assert!(
            error.contains("data/example/recipe/deep/data/entry.json")
                && error.contains("absent:condition")
        );
        assert!(!error.contains("tags/item/flag.json"));
    }
}
