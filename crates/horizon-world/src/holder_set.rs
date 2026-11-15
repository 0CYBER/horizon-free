//! Holder sets as data files write them, read by one decoder for every field
//! vanilla types as a `HolderSet`: an id, a `#tag`, a list of ids
//! (`HolderSetCodec`), and NeoForge's `neoforge:any`, `neoforge:and` /
//! `neoforge:or` (`values`) and `neoforge:not` (`value`) objects, nested freely
//! (`AnyHolderSet`, `AndHolderSet`, `OrHolderSet`, `NotHolderSet`).
//! Mod-defined holder-set types wait on the typed codec foundation (arismoko/horizon#811).
use serde_json::Value;
use std::collections::BTreeSet;

/// One registry, as a reader binds it: what an id and a tag resolve to.
pub trait HolderLookup {
    type Id: Ord + Clone;
    /// The registry's entry noun in errors (`biome`, `block`, `item`...).
    fn what(&self) -> &str;
    /// The members one entry id stands for (a block stands for all its states).
    fn entry(&self, id: &str) -> Result<Vec<Self::Id>, String>;
    /// A tag's members; `tag` has no `#`.
    fn tag(&self, tag: &str) -> Result<Vec<Self::Id>, String>;
    /// Every entry: what `any` holds and what `not` takes its value from.
    fn all(&self) -> Result<Vec<Self::Id>, String>;
}

/// The members `value` names in `lookup`'s registry, in holder-set order
/// without repeats: `or` concatenates its values, `and` keeps its first
/// value's order, `any` and `not` follow `all`.
pub fn resolve<L: HolderLookup + ?Sized>(value: &Value, lookup: &L) -> Result<Vec<L::Id>, String> {
    Ok(match value {
        Value::String(name) => unique(match name.strip_prefix('#') {
            Some(tag) => lookup.tag(tag)?,
            None => lookup.entry(name)?,
        }),
        Value::Array(list) => {
            let mut out = Vec::new();
            for element in list {
                let id = element
                    .as_str()
                    .filter(|id| !id.starts_with('#'))
                    .ok_or_else(|| format!("a {} list holds ids only", lookup.what()))?;
                out.extend(lookup.entry(id)?);
            }
            unique(out)
        }
        Value::Object(object) => {
            let kind = object.get("type").and_then(Value::as_str).unwrap_or("");
            match kind.strip_prefix("neoforge:").unwrap_or(kind) {
                "any" => unique(lookup.all()?),
                short @ ("or" | "and") => {
                    let values = object
                        .get("values")
                        .and_then(Value::as_array)
                        .ok_or_else(|| format!("{} holder set {kind}: no values", lookup.what()))?;
                    let mut sets = values.iter().map(|v| resolve(v, lookup));
                    let first = match sets.next() {
                        Some(first) => first?,
                        None => Vec::new(),
                    };
                    if short == "or" {
                        let mut out = first;
                        for set in sets {
                            out.extend(set?);
                        }
                        unique(out)
                    } else {
                        let mut out = first;
                        for set in sets {
                            let set: BTreeSet<_> = set?.into_iter().collect();
                            out.retain(|id| set.contains(id));
                        }
                        out
                    }
                }
                "not" => {
                    let value = object
                        .get("value")
                        .ok_or_else(|| format!("{} holder set {kind}: no value", lookup.what()))?;
                    let inner: BTreeSet<_> = resolve(value, lookup)?.into_iter().collect();
                    unique(lookup.all()?)
                        .into_iter()
                        .filter(|id| !inner.contains(id))
                        .collect()
                }
                _ => return Err(format!("{} set type {kind:?} is not read", lookup.what())),
            }
        }
        _ => return Err(format!("not a {} id, tag or list", lookup.what())),
    })
}

/// `ids` in order, each first occurrence kept.
fn unique<T: Ord + Clone>(ids: Vec<T>) -> Vec<T> {
    let mut seen = BTreeSet::new();
    ids.into_iter()
        .filter(|id| seen.insert(id.clone()))
        .collect()
}

/// The ids a holder set names in a registry of string ids: the shape loot,
/// enchantment and biome-modifier readers keep.
pub struct StringLookup<'a> {
    pub what: &'a str,
    /// The registry's ids; `None` when the reader can't list them, so `any`
    /// and `not` fail with an error naming the registry.
    pub ids: Option<&'a dyn Fn() -> Vec<String>>,
    pub tag: &'a dyn Fn(&str) -> Result<Vec<String>, String>,
    /// What one id stands for: itself, or an error for an id the reader refuses.
    pub entry: &'a dyn Fn(&str) -> Result<Vec<String>, String>,
}
impl HolderLookup for StringLookup<'_> {
    type Id = String;
    fn what(&self) -> &str {
        self.what
    }
    fn entry(&self, id: &str) -> Result<Vec<String>, String> {
        (self.entry)(id)
    }
    fn tag(&self, tag: &str) -> Result<Vec<String>, String> {
        (self.tag)(tag)
    }
    fn all(&self) -> Result<Vec<String>, String> {
        self.ids
            .map(|ids| ids())
            .ok_or_else(|| format!("{} universe is unavailable", self.what))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Biomes;
    impl HolderLookup for Biomes {
        type Id = &'static str;
        fn what(&self) -> &str {
            "biome"
        }
        fn entry(&self, id: &str) -> Result<Vec<&'static str>, String> {
            ["minecraft:plains", "minecraft:ocean", "minecraft:desert"]
                .into_iter()
                .find(|b| *b == id)
                .map(|b| vec![b])
                .ok_or_else(|| format!("unknown biome {id}"))
        }
        fn tag(&self, tag: &str) -> Result<Vec<&'static str>, String> {
            match tag {
                "minecraft:is_ocean" => Ok(vec!["minecraft:ocean"]),
                "minecraft:is_dry" => Ok(vec!["minecraft:desert"]),
                _ => Err(format!("unknown biome tag {tag}")),
            }
        }
        fn all(&self) -> Result<Vec<&'static str>, String> {
            Ok(vec![
                "minecraft:plains",
                "minecraft:ocean",
                "minecraft:desert",
            ])
        }
    }

    fn set(value: Value) -> Result<Vec<&'static str>, String> {
        resolve(&value, &Biomes)
    }

    #[test]
    fn unavailable_universes_and_missing_object_fields_name_the_registry() {
        let lookup = StringLookup {
            what: "instrument",
            ids: None,
            tag: &|_| Ok(Vec::new()),
            entry: &|id| Ok(vec![id.to_owned()]),
        };
        for value in [
            json!({"type":"neoforge:any"}),
            json!({"type":"neoforge:not", "value":[]}),
        ] {
            let error = resolve(&value, &lookup).unwrap_err();
            assert!(
                error.contains("instrument") && error.contains("unavailable"),
                "{error}"
            );
        }
        for kind in ["neoforge:and", "neoforge:or", "neoforge:not"] {
            let error = resolve(&json!({"type":kind}), &lookup).unwrap_err();
            assert!(
                error.contains("instrument") && error.contains(kind),
                "{error}"
            );
        }
    }

    #[test]
    fn plain_forms_resolve_as_vanilla_reads_them() {
        assert_eq!(set(json!("minecraft:plains")), Ok(vec!["minecraft:plains"]));
        assert_eq!(
            set(json!("#minecraft:is_ocean")),
            Ok(vec!["minecraft:ocean"])
        );
        assert_eq!(
            set(json!([
                "minecraft:plains",
                "minecraft:desert",
                "minecraft:plains"
            ])),
            Ok(vec!["minecraft:plains", "minecraft:desert"])
        );
        assert!(set(json!(["#minecraft:is_ocean"])).is_err());
        assert!(set(json!(3)).is_err());
    }

    #[test]
    fn neoforge_objects_nest() {
        assert_eq!(
            set(json!({"type": "neoforge:not", "value": "#minecraft:is_ocean"})),
            Ok(vec!["minecraft:plains", "minecraft:desert"])
        );
        assert_eq!(
            set(json!({"type": "neoforge:and", "values": [
                {"type": "neoforge:any"},
                {"type": "neoforge:not", "value": {"type": "neoforge:or", "values": [
                    "#minecraft:is_ocean", "minecraft:plains"]}}
            ]})),
            Ok(vec!["minecraft:desert"])
        );
        assert_eq!(set(json!({"type": "neoforge:any"})).unwrap().len(), 3);
        assert!(set(json!({"type": "neoforge:or"})).is_err());
        assert!(set(json!({"type": "example:custom"})).is_err());
    }
}
