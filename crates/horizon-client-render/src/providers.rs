//! Vanilla's `context_int_provider` and `context_float_provider` values (furnace
//! fuel times, brewing times), evaluated from the data pack's JSON.
use crate::items::{ItemStack, Items};
use serde_json::Value;
use std::{collections::HashMap, fs, path::Path};

/// Evaluator for `context_int_provider` / `context_float_provider` values.
pub struct Providers {
    ints: HashMap<String, Value>,
    floats: HashMap<String, Value>,
}
fn load_dir(dir: &Path, namespace: &str, out: &mut HashMap<String, Value>) {
    let Ok(folder) = horizon_world::json_folder::read(dir) else {
        return;
    };
    for file in &folder.files {
        if let Ok(bytes) = file.bytes()
            && let Ok(value) = serde_json::from_slice(bytes)
        {
            out.insert(format!("{namespace}:{}", file.name), value);
        }
    }
}
impl Providers {
    pub fn load(root: &Path) -> Self {
        let mut ints = HashMap::new();
        let mut floats = HashMap::new();
        let mut namespaces: std::collections::BTreeSet<String> =
            std::iter::once("minecraft".to_owned())
                .chain(horizon_items::content_namespaces::namespaces(root))
                .collect();
        namespaces.extend(
            fs::read_dir(root.join("data"))
                .into_iter()
                .flatten()
                .flatten()
                .filter(|entry| entry.path().is_dir())
                .map(|entry| entry.file_name().to_string_lossy().into_owned()),
        );
        for namespace in namespaces {
            let data = root.join("data").join(&namespace);
            load_dir(&data.join("context_int_provider"), &namespace, &mut ints);
            load_dir(
                &data.join("context_float_provider"),
                &namespace,
                &mut floats,
            );
        }
        Self { ints, floats }
    }
    /// The accepted definitions used by provider evaluation, never a resource-file inventory.
    pub fn registry(&self, registry: &str) -> Option<&HashMap<String, Value>> {
        match registry {
            "minecraft:context_int_provider" => Some(&self.ints),
            "minecraft:context_float_provider" => Some(&self.floats),
            _ => None,
        }
    }
    fn eval(v: &Value, fast: bool, table: &HashMap<String, Value>, depth: u32) -> f64 {
        if depth > 16 {
            return 0.0;
        }
        match v {
            Value::Number(n) => n.as_f64().unwrap_or(0.0),
            Value::String(name) => table
                .get(name)
                .map_or(0.0, |v| Self::eval(v, fast, table, depth + 1)),
            Value::Object(o) => {
                let f =
                    |k: &str| Self::eval(o.get(k).unwrap_or(&Value::Null), fast, table, depth + 1);
                match o.get("type").and_then(Value::as_str).unwrap_or("") {
                    "minecraft:div" => {
                        let r = f("right");
                        if r == 0.0 {
                            0.0
                        } else {
                            (f("left") / r).trunc()
                        }
                    }
                    "minecraft:mul" => f("left") * f("right"),
                    "minecraft:add" | "minecraft:sum" => f("left") + f("right"),
                    "minecraft:sub" => f("left") - f("right"),
                    "minecraft:constant" => f("value"),
                    "minecraft:conditional" => {
                        // The only condition in the pinned data: block/fast_cooking.
                        let condition = o.get("condition").and_then(Value::as_str)
                            == Some("minecraft:block/fast_cooking");
                        if condition && fast {
                            f("on_true")
                        } else {
                            f("on_false")
                        }
                    }
                    _ => 0.0,
                }
            }
            _ => 0.0,
        }
    }
    /// The chance that a `compostable` layers provider adds a layer: the weight of its non-zero
    /// layers in the default distribution (the composter's empty-level case is left out), or its
    /// plain number.
    pub fn layer_chance(&self, v: &Value) -> f32 {
        let def = v.as_str().and_then(|n| self.ints.get(n)).unwrap_or(v);
        let base = def.get("default").unwrap_or(def);
        if let Some(n) = base.as_f64() {
            return n as f32;
        }
        let weights: Vec<(f64, f64)> = base["distribution"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|e| {
                (
                    e["data"].as_f64().unwrap_or(0.0),
                    e["weight"].as_f64().unwrap_or(0.0),
                )
            })
            .collect();
        let total: f64 = weights.iter().map(|(_, w)| w).sum();
        if total <= 0.0 {
            return 0.0;
        }
        (weights
            .iter()
            .filter(|(d, _)| *d > 0.0)
            .map(|(_, w)| w)
            .sum::<f64>()
            / total) as f32
    }
    /// A plain `context_int_provider` value or reference (no context conditions).
    pub fn int_value(&self, v: &Value) -> i32 {
        Self::eval(v, false, &self.ints, 0) as i32
    }
    /// A plain `context_float_provider` value or reference; `default` when absent.
    pub fn float_value(&self, v: &Value, default: f32) -> f32 {
        if v.is_null() {
            default
        } else {
            Self::eval(v, false, &self.floats, 0) as f32
        }
    }
    #[cfg(test)]
    pub fn empty() -> Self {
        Self {
            ints: HashMap::new(),
            floats: HashMap::new(),
        }
    }
    pub fn burn_time(&self, items: &Items, stack: &ItemStack, fast: bool) -> i32 {
        items
            .get(&stack.item)
            .and_then(|i| i.fuel.as_ref())
            .map_or(0, |(time, _)| Self::eval(time, fast, &self.ints, 0) as i32)
    }
    pub fn speed(&self, items: &Items, stack: &ItemStack, fast: bool) -> f32 {
        items
            .get(&stack.item)
            .and_then(|i| i.fuel.as_ref())
            .map_or(1.0, |(_, speed)| {
                Self::eval(speed, fast, &self.floats, 0) as f32
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_compost_chance_is_the_weight_of_the_non_zero_layers() {
        let providers = Providers::empty();
        assert_eq!(providers.layer_chance(&json!(0.3)), 0.3);
        let layers = json!({"default": {"distribution": [
            {"data": 0, "weight": 1}, {"data": 1, "weight": 3}]}});
        assert_eq!(providers.layer_chance(&layers), 0.75);
        assert_eq!(providers.layer_chance(&json!("minecraft:missing")), 0.0);
    }

    #[test]
    fn a_burn_time_is_the_provider_resolved() {
        let providers = Providers::empty();
        assert_eq!(providers.int_value(&json!(200)), 200);
        let sum = json!({"type": "minecraft:add", "left": 100, "right": 50});
        assert_eq!(providers.int_value(&sum), 150);
    }

    #[test]
    fn registry_reads_share_accepted_names_with_provider_evaluation() {
        let root = std::env::temp_dir().join(format!(
            "horizon-provider-entries-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let folder = root.join("data/example/context_int_provider/sub");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("value.json"), "17").unwrap();
        std::fs::write(folder.join("invalid.json"), "{").unwrap();
        let providers = Providers::load(&root);
        let table = providers
            .registry("minecraft:context_int_provider")
            .unwrap();
        assert_eq!(table.len(), 1);
        assert_eq!(table.get("example:sub/value"), Some(&json!(17)));
        assert!(!table.contains_key("example:sub/invalid"));
        assert_eq!(providers.int_value(&json!("example:sub/value")), 17);
        assert!(providers.registry("example:unknown").is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}
