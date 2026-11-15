//! Slot-source holder-set tags share DataStore's pack ordering and snapshot.
use super::DataStore;
use std::collections::BTreeSet;
impl DataStore {
    /// TagLoader's required-member validation and first-seen membership order.
    pub fn slot_source_tag(&self, id: &str) -> Result<Vec<String>, String> {
        fn visit(
            store: &DataStore,
            id: &str,
            active: &mut BTreeSet<String>,
        ) -> Result<Vec<String>, String> {
            if !active.insert(id.to_owned()) {
                return Err(format!("Recursive slot-source tag #{id}"));
            }
            let result = (|| {
                let entries = store
                    .slot_source_tags
                    .get(id)
                    .ok_or_else(|| format!("Unknown slot-source tag #{id}"))?;
                let mut out = Vec::new();
                for (entry, required, remove) in entries {
                    // A removal (`TagLoader.EntryWithSource.remove`) takes an
                    // id or a tag's members out of the set built so far, and
                    // an unknown one removes nothing.
                    if *remove {
                        let known = |what: &str| {
                            store.slot_sources.contains_key(what)
                                || store.slot_source_tags.contains_key(what)
                        };
                        if let Some(tag) = entry.strip_prefix('#') {
                            let mut pending = vec![tag];
                            let mut walked = BTreeSet::new();
                            let mut cyclic = false;
                            while let Some(next) = pending.pop() {
                                if next == id {
                                    cyclic = true;
                                    break;
                                }
                                if !walked.insert(next) {
                                    continue;
                                }
                                for (target, _, removes) in
                                    store.slot_source_tags.get(next).into_iter().flatten()
                                {
                                    if *removes && let Some(target) = target.strip_prefix('#') {
                                        pending.push(target);
                                    }
                                }
                            }
                            if cyclic && tag > id {
                                log::warn!(
                                    "Horizon: tags {id} and {tag} form a removal cycle; only removal edge {id} -> {tag} is ignored"
                                );
                                continue;
                            }
                            let mut members = Vec::new();
                            if let Ok(found) = visit(store, tag, active) {
                                members.extend(found);
                            }
                            out.retain(|kept| !members.contains(kept));
                        } else if known(entry) {
                            out.retain(|kept| kept != entry);
                        }
                        continue;
                    }
                    let values = if let Some(tag) = entry.strip_prefix('#') {
                        visit(store, tag, active)
                    } else if store.slot_sources.contains_key(entry) {
                        Ok(vec![entry.clone()])
                    } else {
                        Err(format!("Unknown slot source {entry}"))
                    };
                    match values {
                        Ok(values) => {
                            for value in values {
                                if !out.contains(&value) {
                                    out.push(value);
                                }
                            }
                        }
                        Err(error) if *required => return Err(error),
                        Err(_) => {}
                    }
                }
                Ok(out)
            })();
            active.remove(id);
            result
        }
        visit(self, id, &mut BTreeSet::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mutual_removal_cycles_drop_the_same_named_edge_from_both_roots() {
        let mut store = DataStore::default();
        for name in ["test:one", "test:two"] {
            store
                .slot_sources
                .insert(name.into(), serde_json::json!({}));
        }
        store.slot_source_tags.insert(
            "test:a".into(),
            vec![
                ("test:one".into(), true, false),
                ("#test:b".into(), true, true),
            ],
        );
        store.slot_source_tags.insert(
            "test:b".into(),
            vec![
                ("test:one".into(), true, false),
                ("test:two".into(), true, false),
                ("#test:a".into(), true, true),
            ],
        );
        for (start, expected) in [("test:a", "test:one"), ("test:b", "test:two")] {
            let (out, warnings) =
                crate::test_log::capture(|| store.slot_source_tag(start).unwrap());
            assert_eq!(out, [expected]);
            assert!(
                warnings.iter().any(|line| line.contains("test:a")
                    && line.contains("test:b")
                    && line.contains("only removal edge test:a -> test:b is ignored")),
                "{warnings:?}"
            );
        }
    }

    #[test]
    fn holder_set_tags_keep_order_required_members_and_snapshot_authority() {
        let mut store = DataStore::default();
        for name in ["t:a", "t:b"] {
            store.slot_sources.insert(
                name.into(),
                serde_json::json!({"type":"slot_range","slots":"container.0"}),
            );
        }
        store.slot_source_tags.insert(
            "t:child".into(),
            vec![("t:b".into(), true, false), ("t:a".into(), true, false)],
        );
        store.slot_source_tags.insert(
            "t:all".into(),
            vec![
                ("t:a".into(), true, false),
                ("#t:child".into(), true, false),
                ("t:absent".into(), false, false),
            ],
        );
        assert_eq!(store.slot_source_tag("t:all").unwrap(), ["t:a", "t:b"]);
        store
            .slot_source_tags
            .get_mut("t:all")
            .unwrap()
            .push(("t:absent".into(), true, false));
        assert!(store.slot_source_tag("t:all").is_err());
        store
            .slot_source_tags
            .insert("t:cycle".into(), vec![("#t:cycle".into(), true, false)]);
        assert!(store.slot_source_tag("t:cycle").is_err());
    }
}
