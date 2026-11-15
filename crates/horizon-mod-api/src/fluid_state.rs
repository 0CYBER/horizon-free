//! Platform-owned identities for retained unavailable-fluid names.
use std::{
    collections::HashMap,
    sync::{LazyLock, RwLock},
};

#[derive(Default)]
struct Table {
    names: Vec<String>,
    slots: HashMap<String, u64>,
    utf16_slots: HashMap<Vec<u16>, u64>,
}
static TABLE: LazyLock<RwLock<Table>> = LazyLock::new(Default::default);
const MAX_IDS: usize = 4096;
const MAX_ID_BYTES: usize = 256;

/// Intern a stored name at an engine-owned data boundary.
pub fn intern(id: &str) -> Option<u64> {
    let id = if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    };
    if id.len() > MAX_ID_BYTES || !namespaced_id(&id) || id.starts_with("minecraft:") {
        return None;
    }
    TABLE.write().unwrap().insert(id)
}

impl Table {
    fn insert(&mut self, id: String) -> Option<u64> {
        if let Some(&slot) = self.slots.get(&id) {
            return Some(slot);
        }
        if self.names.len() >= MAX_IDS {
            return None;
        }
        let slot = self.names.len() as u64;
        self.names.push(id.clone());
        self.utf16_slots.insert(id.encode_utf16().collect(), slot);
        self.slots.insert(id, slot);
        Some(slot)
    }
}

/// Look up an already-interned identity without creating one.
pub fn lookup(id: &str) -> Option<u64> {
    TABLE.read().unwrap().slots.get(id).copied()
}

/// Look up an already-interned NBT name without allocation.
pub fn lookup_utf16(id: &[u16]) -> Option<u64> {
    TABLE.read().unwrap().utf16_slots.get(id).copied()
}

/// Resolve an engine-issued process-lifetime handle.
pub fn name(fluid: u64) -> String {
    TABLE.read().unwrap().names[fluid as usize].clone()
}

pub(crate) const ACCESS: crate::__private::UnresolvedFluidAccess =
    crate::__private::UnresolvedFluidAccess {
        intern,
        lookup,
        lookup_utf16,
        name,
        slot_is_live: |slot| slot < TABLE.read().unwrap().names.len() as u64,
    };

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_table_refuses_new_names_but_keeps_existing_ones() {
        let mut table = Table::default();
        for i in 0..4096 {
            assert!(table.insert(format!("test:fluid_{i}")).is_some());
        }
        assert!(table.insert("test:overflow".into()).is_none());
        assert!(table.insert("test:fluid_0".into()).is_some());
    }

    #[test]
    fn invalid_or_oversized_names_are_refused() {
        assert!(intern(&format!("test:{}", "x".repeat(257))).is_none());
        assert!(intern("test:bad space").is_none());
    }

    #[test]
    fn concurrent_intern_and_read_publish_complete_entries() {
        std::thread::scope(|scope| {
            let writer = scope.spawn(|| intern("concurrency:fluid").unwrap());
            let reader = scope.spawn(|| {
                let id = "concurrency:fluid".encode_utf16().collect::<Vec<_>>();
                loop {
                    if let Some(handle) = lookup_utf16(&id) {
                        assert_eq!(name(handle), "concurrency:fluid");
                        break;
                    }
                    std::thread::yield_now();
                }
            });
            let handle = writer.join().unwrap();
            reader.join().unwrap();
            assert_eq!(
                lookup_utf16(&"concurrency:fluid".encode_utf16().collect::<Vec<_>>()),
                Some(handle)
            );
        });
    }

    #[test]
    fn handles_refuse_unissued_slots_and_survive_other_worlds() {
        assert_eq!(intern("mystery"), None);
        assert_eq!(intern("minecraft:mystery"), None);
        let old = intern("absent:fluid").unwrap();
        assert_eq!(Some(old), intern("absent:fluid"));
        crate::__private::initialize().unwrap();
        let mut out = crate::wire::Writer::new();
        out.put(&u64::MAX);
        assert!(crate::wire::Reader::decode::<crate::UnresolvedFluid>(out.bytes()).is_err());
        let typed = crate::__private::intern_unresolved_fluid("absent:typed").unwrap();
        out.clear();
        out.put(&typed);
        assert_eq!(
            crate::wire::Reader::decode::<crate::UnresolvedFluid>(out.bytes()),
            Ok(typed)
        );
        let next = intern("other:fluid").unwrap();
        assert_ne!(old, next);
        assert_eq!(name(old), "absent:fluid");
    }
}

fn namespaced_id(id: &str) -> bool {
    let ok = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b);
    id.split_once(':').is_some_and(|(namespace, path)| {
        !namespace.is_empty()
            && !path.is_empty()
            && namespace.bytes().all(ok)
            && path.bytes().all(|b| ok(b) || b == b'/')
    })
}
#[cfg(test)]
mod identifier_tests {
    #[test]
    fn namespaced_identifier_samples() {
        for (id, expected) in [
            ("a:b/c", true),
            ("a_0.x-y:b/c.d-e_0", true),
            ("a:", false),
            (":b", false),
            ("a:b:c", false),
            ("A:b", false),
            ("a/b:c", false),
        ] {
            assert_eq!(super::namespaced_id(id), expected, "{id}");
        }
    }
}
