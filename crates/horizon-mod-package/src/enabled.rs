//! The accepted, enabled packages captured at a resource-generation boundary.
//!
//! This is distinct from the startup inventory used for registration: disabled
//! and refused packages do not enter this snapshot. The owning game constructs
//! it from accepted packages, and every data reader shares that generation.
use mod_vocab::Side;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, io, path::Path};

/// The enabled-package snapshot stored beside generated registry tables.
pub const MARKER: &str = "horizon-enabled-mods.json";

/// Package identifiers and their declared sides, never resource namespaces.
/// Sorted entries make serialized snapshots and content-cache keys deterministic.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(transparent)]
pub struct EnabledMods(BTreeMap<String, Side>);

impl EnabledMods {
    /// Read this generation's inventory. A vanilla root has no mod snapshot.
    pub fn read(root: &Path) -> io::Result<Self> {
        match fs::read(root.join(MARKER)) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }

    /// Capture packages already accepted and enabled by the owning loader.
    /// A package present on both sides remains visible to server data loading.
    pub fn new(packages: impl IntoIterator<Item = (String, Side)>) -> Self {
        let mut entries = BTreeMap::new();
        for (id, side) in packages {
            entries
                .entry(id)
                .and_modify(|old| {
                    if side == Side::Both || *old == Side::Both {
                        *old = Side::Both;
                    } else if side == Side::Server {
                        *old = Side::Server;
                    }
                })
                .or_insert(side);
        }
        Self(entries)
    }

    /// The accepted packages visible to the client preload: not server-only ones.
    pub fn client_ids(&self) -> impl Iterator<Item = &str> {
        self.0
            .iter()
            .filter(|(_, side)| matches!(side, Side::Client | Side::Both))
            .map(|(id, _)| id.as_str())
    }

    /// Packages whose data may participate in the logical server's load.
    pub fn server_ids(&self) -> impl Iterator<Item = &str> {
        self.0
            .iter()
            .filter(|(_, side)| matches!(side, Side::Both | Side::Server))
            .map(|(id, _)| id.as_str())
    }

    /// Whether the snapshot contains no enabled packages.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_data_excludes_client_only_packages_and_serialization_keeps_sides() {
        let inventory = EnabledMods::new([
            ("integration".into(), Side::Both),
            ("client_overlay".into(), Side::Client),
            ("server_only".into(), Side::Server),
        ]);
        assert_eq!(
            inventory.client_ids().collect::<Vec<_>>(),
            ["client_overlay", "integration"]
        );
        assert_eq!(
            inventory.server_ids().collect::<Vec<_>>(),
            ["integration", "server_only"]
        );
        let restored: EnabledMods =
            serde_json::from_slice(&serde_json::to_vec(&inventory).unwrap()).unwrap();
        assert_eq!(restored, inventory);
    }

    #[test]
    fn cache_identity_changes_with_membership_and_side_but_not_input_order() {
        let both = EnabledMods::new([("integration".into(), Side::Both)]);
        let client = EnabledMods::new([("integration".into(), Side::Client)]);
        let empty = EnabledMods::default();
        let encode = |mods: &EnabledMods| serde_json::to_vec(mods).unwrap();
        assert_ne!(encode(&both), encode(&client));
        assert_ne!(encode(&both), encode(&empty));
        let duplicate = EnabledMods::new([
            ("integration".into(), Side::Client),
            ("integration".into(), Side::Both),
        ]);
        assert_eq!(encode(&both), encode(&duplicate));
        assert_eq!(both.server_ids().collect::<Vec<_>>(), ["integration"]);
    }
}
