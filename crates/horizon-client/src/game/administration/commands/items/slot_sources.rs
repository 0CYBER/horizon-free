//! Shared holder/context resolution before SlotCollection read or live-write consumption.
use super::{Game, Owner, R, err};
use horizon_client::datapack::DataStore;
use horizon_core::nbt::Tag;
use serde_json::Value;
use std::collections::BTreeSet;
fn id(raw: &str) -> String {
    if raw.contains(':') {
        raw.to_owned()
    } else {
        format!("minecraft:{raw}")
    }
}
/// `LimitSlotSource`'s limit is `Codec.INT`: any number's `Number.intValue()`,
/// so a pack's `2.5` and a command's `2.5d` both select two slots.
pub(super) fn limit(source: &Value) -> usize {
    let limit = &source["limit"];
    let value = match limit.as_i64() {
        Some(whole) => whole as i32,
        // Java's (int) cast saturates, and NaN becomes 0, as `as` does.
        None => limit.as_f64().unwrap_or(0.0) as i32,
    };
    usize::try_from(value).unwrap_or(0)
}
/// A typed source as the consumers read it. `tag_to_json` reads a byte as a
/// boolean; a limit keeps its number (`limit:1b`).
fn typed_source(tag: &Tag) -> Value {
    fn keep_byte_limits(tag: &Tag, value: &mut Value) {
        match tag {
            Tag::List(terms) => {
                for (term, value) in terms.iter().zip(value.as_array_mut().into_iter().flatten()) {
                    keep_byte_limits(term, value);
                }
            }
            Tag::Compound(_) => {
                if let Some(Tag::Byte(limit)) = tag.get("limit") {
                    value["limit"] = Value::from(*limit);
                }
                for child in ["slot_source", "terms"] {
                    if let Some(tag) = tag.get(child) {
                        keep_byte_limits(tag, &mut value[child]);
                    }
                }
            }
            _ => {}
        }
    }
    let mut value = horizon_client::scoreboard::tag_to_json(tag);
    keep_byte_limits(tag, &mut value);
    value
}
/// Resolve holders into the existing structured-source representation, preserving
/// ordered terms, duplicate collection visits and each transform's nesting.
pub(in crate::game) fn expand(store: &DataStore, source: &Value) -> Result<Value, String> {
    fn resolve(
        store: &DataStore,
        source: &Value,
        active: &mut BTreeSet<String>,
    ) -> Result<Value, String> {
        if let Some(name) = source.as_str() {
            if let Some(tag) = name.strip_prefix('#') {
                let names = store.slot_source_tag(&id(tag))?;
                return resolve(
                    store,
                    &Value::Array(names.into_iter().map(Value::String).collect()),
                    active,
                );
            }
            let name = id(name);
            if !active.insert(name.clone()) {
                return Err(format!("Recursive slot source {name}"));
            }
            let result = store
                .slot_sources
                .get(&name)
                .ok_or_else(|| format!("Unknown slot source {name}"))
                .and_then(|s| resolve(store, s, active));
            active.remove(&name);
            return result;
        }
        if let Some(terms) = source.as_array() {
            return terms
                .iter()
                .map(|s| resolve(store, s, active))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array);
        }
        let mut value = source.clone();
        let kind = source["type"]
            .as_str()
            .unwrap_or_default()
            .trim_start_matches("minecraft:");
        match kind {
            "group" => {
                let terms = &source["terms"];
                // GroupSlotSource's terms are a holder set: a `#tag`, a list,
                // or one holder (compactListCodec), always a list after.
                let terms = match resolve(store, terms, active)? {
                    Value::Array(list) => Value::Array(list),
                    one => Value::Array(vec![one]),
                };
                value["terms"] = terms;
            }
            "contents" | "filtered" | "limit_slots" => {
                value["slot_source"] = resolve(store, &source["slot_source"], active)?
            }
            _ => {}
        }
        Ok(value)
    }
    resolve(store, source, &mut BTreeSet::new())
}
/// Check that every holder `source` names resolves, without expanding it: each
/// source id and tag is walked once, so a chain of groups naming the next source
/// twice stays linear. The same unknown and recursive references as [`expand`].
pub(in crate::game) fn check_references(
    store: &DataStore,
    source: &Value,
    active: &mut BTreeSet<String>,
    checked: &mut BTreeSet<String>,
) -> Result<(), String> {
    if let Some(name) = source.as_str() {
        if let Some(tag) = name.strip_prefix('#') {
            let tag = format!("#{}", id(tag));
            if checked.contains(&tag) {
                return Ok(());
            }
            for name in store.slot_source_tag(&tag[1..])? {
                check_references(store, &Value::String(name), active, checked)?;
            }
            checked.insert(tag);
            return Ok(());
        }
        let name = id(name);
        if checked.contains(&name) {
            return Ok(());
        }
        if !active.insert(name.clone()) {
            return Err(format!("Recursive slot source {name}"));
        }
        let result = store
            .slot_sources
            .get(&name)
            .ok_or_else(|| format!("Unknown slot source {name}"))
            .and_then(|s| check_references(store, s, active, checked));
        active.remove(&name);
        if result.is_ok() {
            checked.insert(name);
        }
        return result;
    }
    if let Some(terms) = source.as_array() {
        return terms
            .iter()
            .try_for_each(|s| check_references(store, s, active, checked));
    }
    let kind = source["type"]
        .as_str()
        .unwrap_or_default()
        .trim_start_matches("minecraft:");
    match kind {
        "group" => check_references(store, &source["terms"], active, checked),
        "contents" | "filtered" | "limit_slots" => {
            check_references(store, &source["slot_source"], active, checked)
        }
        _ => Ok(()),
    }
}
impl Game<'_> {
    /// Literal ranges retain priority; all remaining consumers resolve the same holder snapshot.
    pub(super) fn resolved_slot_source(&self, raw: &str) -> R<Option<Value>> {
        if horizon_client::command::args::slot_ranges()
            .iter()
            .any(|(name, _)| name == raw)
        {
            return Ok(None);
        }
        let value = if self.fns.store.slot_sources.contains_key(&id(raw)) {
            Value::String(id(raw))
        } else {
            let (tag, _) = horizon_core::nbt::parse_snbt(raw)
                .map_err(|error| err("argument.resource_or_id.failed_to_parse", &[&error]))?;
            typed_source(&tag)
        };
        expand(&self.fns.store, &value)
            .map(Some)
            .map_err(|error| err("argument.resource_or_id.failed_to_parse", &[&error]))
    }
    /// ItemCommands.getSlotsFromProvider binds CONTAINER separately from THIS_ENTITY.
    /// THIS_ENTITY is `CommandSourceStack.getEntity`: the player who typed the command, the
    /// entity `execute as` names, nobody for the console or a function.
    /// Other valid context targets are absent in COMMAND_SLOT_SOURCE, never Player fallbacks.
    pub(super) fn slot_source_owner(&self, container: &Owner, source: &Value) -> Option<Owner> {
        match source["source"].as_str().unwrap_or("container") {
            "container" => Some(container.clone()),
            "this" => self.source_entity().map(Owner::Entity),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn referenced_transforms_keep_live_filter_and_limit_nesting() {
        let mut store = DataStore::default();
        store.slot_sources.insert(
            "t:range".into(),
            json!({"type":"slot_range","source":"this","slots":"weapon.mainhand"}),
        );
        store.slot_sources.insert("t:filtered".into(),json!({"type":"filtered","slot_source":"t:range","item_filter":{"items":"minecraft:stone"}}));
        store
            .slot_source_tags
            .insert("t:twice".into(), vec![("t:filtered".into(), true, false)]);
        let source = json!({"type":"limit_slots","limit":1,"slot_source":{"type":"group","terms":["t:filtered","t:filtered"]}});
        let value = expand(&store, &source).unwrap();
        let child = json!({"type":"filtered","slot_source":{"type":"slot_range","source":"this","slots":"weapon.mainhand"},"item_filter":{"items":"minecraft:stone"}});
        assert_eq!(
            value,
            json!({"type":"limit_slots","limit":1,"slot_source":{"type":"group","terms":[child,child]}})
        );
        assert_eq!(expand(&store, &json!("#t:twice")).unwrap(), json!([child]));
        // A group's single holder term is a one-term list, as an inline term is.
        assert_eq!(
            expand(&store, &json!({"type":"group","terms":"t:filtered"})).unwrap(),
            json!({"type":"group","terms":[child]})
        );
        assert_eq!(
            expand(
                &store,
                &json!({"type":"group","terms":{"type":"slot_range","slots":"container.0"}})
            )
            .unwrap(),
            json!({"type":"group","terms":[{"type":"slot_range","slots":"container.0"}]})
        );
        assert!(expand(&store, &json!("t:absent")).is_err());
        store.slot_sources.insert(
            "t:cycle".into(),
            json!({"type":"limit_slots","limit":1,"slot_source":"t:cycle"}),
        );
        assert!(expand(&store, &json!("t:cycle")).is_err());
    }

    /// A load checks every source; a chain of groups naming the next source twice
    /// expands to 2^n leaves, but its references are walked once each.
    #[test]
    fn checking_a_doubling_chain_walks_each_source_once() {
        let mut store = DataStore::default();
        let depth = 64;
        for i in 0..depth {
            let next = format!("t:s{}", i + 1);
            store.slot_sources.insert(
                format!("t:s{i}"),
                json!({"type":"group","terms":[next, next]}),
            );
        }
        let check = |store: &DataStore| {
            check_references(
                store,
                &json!("t:s0"),
                &mut BTreeSet::new(),
                &mut BTreeSet::new(),
            )
        };
        assert_eq!(
            check(&store),
            Err(format!("Unknown slot source t:s{depth}"))
        );
        store.slot_sources.insert(
            format!("t:s{depth}"),
            json!({"type":"slot_range","slots":"container.0"}),
        );
        assert_eq!(check(&store), Ok(()));
        store
            .slot_sources
            .insert(format!("t:s{depth}"), json!("t:s0"));
        assert_eq!(check(&store), Err("Recursive slot source t:s0".into()));
    }
}
