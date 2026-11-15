//! Retain numeric snapshots without retaining another copy of equal imported data.
use super::{BlockRegistry, data::Data};
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
};

struct Snapshot {
    data: Arc<Data>,
    installations: HashMap<usize, &'static BlockRegistry>,
}

// Mod-list identities are never recycled: the owning installation is process-lived.
// Separate mod lists never share a payload, even when their numeric rows happen to match.
static SNAPSHOTS: LazyLock<Mutex<HashMap<usize, Vec<Snapshot>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

impl BlockRegistry {
    /// Retain a registry for an immutable host installation.
    ///
    /// This is host plumbing, not a mod API. `mods` identifies the retained mod list;
    /// `parts` identifies its immutable Parts installation. Without a Parts identity,
    /// a future metadata owner is unknown, so only imported data may be shared.
    /// Worker resources and saved-state caches remain attached to each registry wrapper.
    #[doc(hidden)]
    pub fn intern(self, (mods, parts): (usize, Option<usize>)) -> &'static Self {
        let mut snapshots = SNAPSHOTS.lock().unwrap_or_else(|error| error.into_inner());
        let snapshots = snapshots.entry(mods).or_default();
        let snapshot = if let Some(index) = snapshots
            .iter()
            .position(|snapshot| *snapshot.data == *self.data)
        {
            &mut snapshots[index]
        } else {
            snapshots.push(Snapshot {
                data: self.data.clone(),
                installations: HashMap::new(),
            });
            snapshots
                .last_mut()
                .expect("just inserted registry snapshot")
        };
        if let Some(parts) = parts
            && let Some(&registry) = snapshot.installations.get(&parts)
        {
            return registry;
        }
        let mut registry = self;
        registry.data = snapshot.data.clone();
        let registry = Box::leak(Box::new(registry));
        if let Some(parts) = parts {
            snapshot.installations.insert(parts, registry);
        }
        registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::BlockStateId;

    fn registry(default: u32, reversed: bool) -> BlockRegistry {
        let states = if reversed {
            r#"[["",false,false,"",{"id":"test:lamp","properties":{"lit":"true"}}],["",false,false,"",{"id":"test:lamp","properties":{"lit":"false"}}]]"#
        } else {
            r#"[["",false,false,"",{"id":"test:lamp","properties":{"lit":"false"}}],["",false,false,"",{"id":"test:lamp","properties":{"lit":"true"}}]]"#
        };
        BlockRegistry::from_facts_json(
            format!(r#"{{"defaults":{{"test:lamp":{default}}},"states":{states}}}"#).as_bytes(),
        )
        .unwrap()
    }

    #[test]
    fn same_installation_reuses_wrapper_but_new_epoch_only_shares_data() {
        let first = registry(0, false).intern((101, Some(1)));
        assert!(first.mod_worker_resource.set(Arc::new(42_u32)).is_ok());
        let same = registry(0, false).intern((101, Some(1)));
        assert!(std::ptr::eq(first, same));
        assert_eq!(
            same.mod_worker_resource
                .get()
                .unwrap()
                .downcast_ref::<u32>(),
            Some(&42)
        );
        let next = registry(0, false).intern((101, Some(2)));
        assert!(!std::ptr::eq(first, next));
        assert!(Arc::ptr_eq(&first.data, &next.data));
        assert!(next.mod_worker_resource.get().is_none());
    }

    #[test]
    fn different_mods_order_and_defaults_keep_distinct_payloads() {
        let first = registry(0, false).intern((102, Some(1)));
        let other_mods = registry(0, false).intern((103, Some(1)));
        let reordered = registry(1, true).intern((102, Some(1)));
        let defaults = registry(1, false).intern((102, Some(1)));
        for other in [other_mods, reordered, defaults] {
            assert!(!std::ptr::eq(first, other));
            assert!(!Arc::ptr_eq(&first.data, &other.data));
        }
        assert_eq!(first.default_state("test:lamp"), Some(BlockStateId(0)));
        assert_eq!(defaults.default_state("test:lamp"), Some(BlockStateId(1)));
        assert_eq!(
            first
                .state(BlockStateId(0))
                .unwrap()
                .properties
                .get("lit")
                .unwrap(),
            "false"
        );
        assert_eq!(
            reordered
                .state(BlockStateId(0))
                .unwrap()
                .properties
                .get("lit")
                .unwrap(),
            "true"
        );
    }

    #[test]
    fn unknown_installations_never_share_worker_owners() {
        let first = registry(0, false).intern((104, None));
        let next = registry(0, false).intern((104, None));
        assert!(!std::ptr::eq(first, next));
        assert!(Arc::ptr_eq(&first.data, &next.data));
    }
}
