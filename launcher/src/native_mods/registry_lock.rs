//! The trust boundary for a mod-owned registry lock. Never resolve missing pins online.
use std::collections::BTreeMap;

const CRATES_IO: &str = horizon_addresses::CARGO_CRATES_IO_SOURCE;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Identity {
    pub name: String,
    pub version: semver::Version,
    pub source: String,
}

#[derive(Clone, Debug)]
pub(super) struct Record {
    checksum: Option<String>,
    edges: std::collections::BTreeSet<Identity>,
    raw: toml::Table,
}

#[derive(Clone, Debug)]
struct Reference {
    name: String,
    version: Option<semver::Version>,
    source: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct RegistryLock {
    pub packages: BTreeMap<Identity, Record>,
    local: BTreeMap<Identity, Record>,
}

impl RegistryLock {
    /// Parse untrusted lock records once; consumers use typed identities and edges.
    pub fn parse(text: &str) -> Result<Self, String> {
        let lock: toml::Table = text.parse().map_err(|e| format!("Cargo.lock: {e}"))?;
        let records = lock
            .get("package")
            .and_then(toml::Value::as_array)
            .ok_or("Cargo.lock: missing packages")?;
        let mut parsed = Self::default();
        let mut pending = Vec::new();
        for record in records {
            let table = record.as_table().ok_or("Cargo.lock: invalid package")?;
            let field = |key| {
                table
                    .get(key)
                    .and_then(toml::Value::as_str)
                    .ok_or_else(|| format!("Cargo.lock: package missing {key}"))
            };
            let name = field("name")?.to_owned();
            let version = semver::Version::parse(field("version")?)
                .map_err(|e| format!("Cargo.lock: {name}: {e}"))?;
            let source = match table.get("source") {
                None => String::new(),
                Some(value) if value.as_str() == Some(CRATES_IO) => CRATES_IO.to_owned(),
                Some(_) => {
                    return Err(format!(
                        "Cargo.lock: {name} {version}: only crates.io sources are allowed"
                    ))
                }
            };
            let checksum = if source.is_empty() {
                None
            } else {
                let checksum = field("checksum")?;
                if checksum.len() != 64
                    || !checksum
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                {
                    return Err(format!(
                        "Cargo.lock: {name} {version}: invalid crates.io checksum"
                    ));
                }
                Some(checksum.to_owned())
            };
            let mut dependencies = Vec::new();
            if let Some(deps) = table.get("dependencies") {
                for dep in deps.as_array().ok_or("Cargo.lock: invalid dependencies")? {
                    let dep = dep
                        .as_str()
                        .ok_or("Cargo.lock: invalid dependency reference")?;
                    let parts: Vec<_> = dep.split_whitespace().collect();
                    if parts.is_empty() || parts.len() > 3 {
                        return Err(format!("Cargo.lock: invalid dependency {dep}"));
                    }
                    let version = parts
                        .get(1)
                        .map(|version| semver::Version::parse(version))
                        .transpose()
                        .map_err(|e| format!("Cargo.lock: dependency {dep}: {e}"))?;
                    let source = parts
                        .get(2)
                        .map(|source| {
                            source
                                .strip_prefix('(')
                                .and_then(|source| source.strip_suffix(')'))
                                .map(str::to_owned)
                                .ok_or("Cargo.lock: invalid dependency source")
                        })
                        .transpose()?;
                    dependencies.push(Reference {
                        name: parts[0].into(),
                        version,
                        source,
                    });
                }
            }
            let identity = Identity {
                name,
                version,
                source,
            };
            pending.push((identity.clone(), dependencies));
            let records = if identity.source.is_empty() {
                &mut parsed.local
            } else {
                &mut parsed.packages
            };
            if records
                .insert(
                    identity.clone(),
                    Record {
                        checksum,
                        edges: Default::default(),
                        raw: table.clone(),
                    },
                )
                .is_some()
            {
                return Err(format!(
                    "Cargo.lock: duplicate package {} {}",
                    identity.name, identity.version
                ));
            }
        }
        for (id, references) in pending {
            let mut edges = std::collections::BTreeSet::new();
            for dep in references {
                let matches: Vec<_> = parsed
                    .packages
                    .keys()
                    .chain(parsed.local.keys())
                    .filter(|id| {
                        id.name == dep.name
                            && dep.version.as_ref().is_none_or(|v| v == &id.version)
                            && dep
                                .source
                                .as_ref()
                                .is_none_or(|source| source == &id.source)
                    })
                    .collect();
                if matches.len() != 1 {
                    return Err(format!(
                        "Cargo.lock: ambiguous or missing dependency {}",
                        dep.name
                    ));
                }
                edges.insert(matches[0].clone());
            }
            parsed
                .packages
                .get_mut(&id)
                .or_else(|| parsed.local.get_mut(&id))
                .expect("parsed identity")
                .edges = edges;
        }
        Ok(parsed)
    }

    pub fn extend(&mut self, other: &Self) -> Result<(), String> {
        for (identity, record) in &other.packages {
            if let Some(previous) = self.packages.get(identity) {
                if previous.checksum != record.checksum {
                    return Err(format!(
                        "Cargo.lock: conflicting pins for {} {}",
                        identity.name, identity.version
                    ));
                }
            } else {
                self.packages.insert(identity.clone(), record.clone());
            }
        }
        Ok(())
    }

    /// Authorize only same-version registry-to-SDK substitutions in mod-owned records.
    /// Game records keep their original edges, and the registry identity being replaced
    /// is removed so it cannot authorize a second, non-SDK copy of the patched crate.
    pub fn allow_game_patch_edges(
        &mut self,
        game: &Self,
        names: &std::collections::BTreeSet<String>,
    ) {
        let replacements: BTreeMap<_, _> = game
            .local
            .keys()
            .filter(|id| names.contains(&id.name))
            .map(|id| {
                (
                    Identity {
                        source: CRATES_IO.into(),
                        ..id.clone()
                    },
                    id.clone(),
                )
            })
            .collect();
        self.packages.retain(|id, _| !replacements.contains_key(id));
        for (id, record) in &mut self.packages {
            if game.packages.contains_key(id) {
                continue;
            }
            record.edges = record
                .edges
                .iter()
                .map(|edge| replacements.get(edge).unwrap_or(edge).clone())
                .collect();
        }
    }

    /// Compare resolved identities and checksums, and normalize Cargo's optional dependency
    /// qualifiers before comparing edges. Adding a second version qualifies existing edges.
    #[cfg(test)]
    pub fn check_resolution(&self, resolved: &Self, game: &Self) -> Result<(), String> {
        self.check_resolved(resolved, game, true)
    }

    /// A mod-only workspace may omit game packages and their inactive optional edges.
    pub fn check_partial_resolution(&self, resolved: &Self, game: &Self) -> Result<(), String> {
        self.check_resolved(resolved, game, false)
    }

    fn check_resolved(
        &self,
        resolved: &Self,
        game: &Self,
        require_game: bool,
    ) -> Result<(), String> {
        // The prebuilt game retains its features. A narrower Cargo workspace may
        // drop game-owned edges, but must never add edges or change identities.
        for (id, record) in resolved.packages.iter().chain(resolved.local.iter()) {
            let owners: Vec<_> = self
                .packages
                .iter()
                .chain(self.local.iter())
                .filter(|(old, _)| old.name == id.name)
                .collect();
            if owners.is_empty() {
                continue;
            }
            if !owners.iter().any(|(old, pin)| {
                *old == id
                    && if let Some(owner) = game.packages.get(id).or_else(|| game.local.get(id)) {
                        record.edges.is_subset(&owner.edges)
                    } else {
                        pin.edges == record.edges
                    }
            }) {
                let labels = |edges: &std::collections::BTreeSet<Identity>| {
                    edges
                        .iter()
                        .map(|edge| format!("{} {}", edge.name, edge.version))
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                return Err(format!("Cargo.lock: game or pinned package {} moved or changed its dependency graph: [{}] -> [{}]", id.name, owners.iter().map(|(old, pin)| format!("{} {}: {}", old.name, old.version, labels(&pin.edges))).collect::<Vec<_>>().join("; "), labels(&record.edges)));
            }
        }
        for (id, record) in &resolved.packages {
            let Some(pin) = self.packages.get(id) else {
                let versions = game
                    .packages
                    .keys()
                    .filter(|old| old.name == id.name)
                    .map(|old| old.version.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(format!(
                    "Cargo.lock: {} moved from [{}] to {}; resolution is not pinned by a mod lock",
                    id.name, versions, id.version
                ));
            };
            if pin.checksum != record.checksum {
                return Err(format!(
                    "Cargo.lock: checksum changed for {} {}",
                    id.name, id.version
                ));
            }
        }
        if !require_game {
            return Ok(());
        }
        for id in game.packages.keys().chain(game.local.keys()) {
            let Some(record) = resolved.packages.get(id).or_else(|| resolved.local.get(id)) else {
                let versions = resolved
                    .packages
                    .keys()
                    .filter(|new| new.name == id.name)
                    .map(|new| new.version.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(format!(
                    "Cargo.lock: game crate {} moved from {} to [{}]",
                    id.name, id.version, versions
                ));
            };
            let original = game
                .packages
                .get(id)
                .or_else(|| game.local.get(id))
                .expect("game identity");
            if !record.edges.is_subset(&original.edges) {
                let labels = |edges: &std::collections::BTreeSet<Identity>| {
                    edges
                        .iter()
                        .map(|id| format!("{} {}", id.name, id.version))
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                return Err(format!(
                    "Cargo.lock: game dependency graph changed for {} {}: [{}] -> [{}]",
                    id.name,
                    id.version,
                    labels(&original.edges),
                    labels(&record.edges)
                ));
            }
        }
        Ok(())
    }

    pub fn check_mod(
        &self,
        mod_id: &str,
        root: &str,
        skip_local: &std::collections::BTreeSet<String>,
        pins: &Self,
    ) -> Result<(), String> {
        let root = self
            .local
            .keys()
            .find(|id| id.name == root)
            .ok_or_else(|| format!("{mod_id}: staged Cargo.lock has no mod root"))?;
        let mut pending = vec![root.clone()];
        let mut seen = std::collections::BTreeSet::new();
        while let Some(id) = pending.pop() {
            if !seen.insert(id.clone()) {
                continue;
            }
            let record = self
                .packages
                .get(&id)
                .or_else(|| self.local.get(&id))
                .expect("resolved edge identity");
            if !id.source.is_empty() && !pins.packages.contains_key(&id) {
                let versions = pins
                    .packages
                    .keys()
                    .filter(|pin| pin.name == id.name)
                    .map(|pin| pin.version.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(format!(
                    "{mod_id}: {} resolved to {}, but its own Cargo.lock pins [{}]",
                    id.name, id.version, versions
                ));
            }
            pending.extend(
                record
                    .edges
                    .iter()
                    .filter(|dep| !dep.source.is_empty() || !skip_local.contains(&dep.name))
                    .cloned(),
            );
        }
        Ok(())
    }

    pub fn game_package_names(&self) -> std::collections::BTreeSet<String> {
        self.local.keys().map(|id| id.name.clone()).collect()
    }

    pub fn seed(&self, text: &str) -> Result<String, String> {
        let mut lock: toml::Table = text.parse().map_err(|e| format!("Cargo.lock: {e}"))?;
        let records = lock
            .get_mut("package")
            .and_then(toml::Value::as_array_mut)
            .ok_or("Cargo.lock: missing packages")?;
        records.retain(|record| record.get("source").is_none());
        records.extend(
            self.packages
                .values()
                .map(|record| toml::Value::Table(record.raw.clone())),
        );
        Ok(lock.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn lock(source: &str, checksum: &str) -> String {
        format!("version = 4\n[[package]]\nname = 'itoa'\nversion = '1.0.15'\nsource = '{source}'\nchecksum = '{checksum}'\n")
    }
    fn patched_graph(version: &str, registry: bool) -> String {
        let source = if registry {
            "source='registry+https://github.com/rust-lang/crates.io-index'\nchecksum='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'"
        } else {
            ""
        };
        format!("version=4\n[[package]]\nname='allocator'\nversion='1.0.0'\nsource='registry+https://github.com/rust-lang/crates.io-index'\nchecksum='bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'\ndependencies=['allocator-sys']\n[[package]]\nname='allocator-sys'\nversion='{version}'\n{source}\n")
    }

    #[test]
    fn isolated_game_patch_version_movement_is_refused() {
        let game =
            RegistryLock::parse("version=4\n[[package]]\nname='allocator-sys'\nversion='0.1.49'\n")
                .unwrap();
        let moved =
            RegistryLock::parse("version=4\n[[package]]\nname='allocator-sys'\nversion='0.1.50'\n")
                .unwrap();
        let error = game
            .check_resolution(&moved, &RegistryLock::default())
            .unwrap_err();
        assert!(error.contains("allocator-sys"), "{error}");
    }

    #[test]
    fn isolated_game_patch_registry_source_movement_is_refused() {
        let game =
            RegistryLock::parse("version=4\n[[package]]\nname='allocator-sys'\nversion='0.1.49'\n")
                .unwrap();
        let moved = RegistryLock::parse(&format!("version=4\n[[package]]\nname='allocator-sys'\nversion='0.1.49'\nsource='{CRATES_IO}'\nchecksum='{}'\n", "a".repeat(64))).unwrap();
        let error = game
            .check_resolution(&moved, &RegistryLock::default())
            .unwrap_err();
        assert!(error.contains("allocator-sys"), "{error}");
    }

    #[test]
    fn registry_to_game_owned_local_patch_is_pinned_and_movement_refuses_by_name() {
        let game = RegistryLock::parse(&patched_graph("0.1.49", false)).unwrap();
        game.check_resolution(&game, &RegistryLock::default())
            .unwrap();
        for (version, registry) in [("0.1.50", false), ("0.1.49", true)] {
            let moved = RegistryLock::parse(&patched_graph(version, registry)).unwrap();
            let error = game
                .check_resolution(&moved, &RegistryLock::default())
                .unwrap_err();
            assert!(error.contains("allocator-sys"), "{error}");
        }
    }

    #[test]
    fn game_version_movement_is_named_and_added_versions_are_allowed() {
        let game = RegistryLock::parse(&lock(CRATES_IO, &"a".repeat(64))).unwrap();
        let other =
            RegistryLock::parse(&lock(CRATES_IO, &"b".repeat(64)).replace("1.0.15", "0.4.8"))
                .unwrap();
        let mut allowed = game.clone();
        allowed.extend(&other).unwrap();
        allowed.check_resolution(&allowed, &game).unwrap();
        let error = allowed.check_resolution(&other, &game).unwrap_err();
        assert!(
            error.contains("itoa") && error.contains("1.0.15") && error.contains("0.4.8"),
            "{error}"
        );
    }

    #[test]
    fn a_local_game_edge_cannot_move_even_when_both_versions_remain_locked() {
        let registry = lock(CRATES_IO, &"a".repeat(64));
        let game = format!(
            "{registry}\n[[package]]\nname='game'\nversion='1.0.0'\ndependencies=['itoa']\n"
        );
        let old = RegistryLock::parse(&game).unwrap();
        let new_registry = lock(CRATES_IO, &"b".repeat(64)).replace("1.0.15", "0.4.8");
        let moved = format!(
            "{}\n{}",
            game.replace("dependencies=['itoa']", "dependencies=['itoa 0.4.8']"),
            new_registry.replace("version = 4\n", "")
        );
        let resolved = RegistryLock::parse(&moved).unwrap();
        let mut allowed = old.clone();
        allowed
            .extend(&RegistryLock::parse(&new_registry).unwrap())
            .unwrap();
        let error = allowed.check_resolution(&resolved, &old).unwrap_err();
        for detail in ["game", "itoa", "1.0.15", "0.4.8"] {
            assert!(error.contains(detail), "{error}");
        }
    }

    #[test]
    fn a_mod_cannot_borrow_another_mods_transitive_pin() {
        let owned = RegistryLock::parse(&lock(CRATES_IO, &"a".repeat(64))).unwrap();
        let other = lock(CRATES_IO, &"b".repeat(64)).replace("1.0.15", "0.4.8");
        let resolved = RegistryLock::parse(&format!(
            "{other}\n[[package]]\nname='mod'\nversion='1.0.0'\ndependencies=['itoa']\n"
        ))
        .unwrap();
        let error = resolved
            .check_mod("example", "mod", &Default::default(), &owned)
            .unwrap_err();
        for detail in ["example", "itoa", "1.0.15", "0.4.8"] {
            assert!(error.contains(detail), "{error}");
        }
    }

    #[test]
    fn sdk_and_declared_mod_roots_are_opaque_to_a_mods_registry_lock() {
        let resolved = RegistryLock::parse(&format!("{}\n[[package]]\nname='mod'\nversion='1.0.0'\ndependencies=['sdk','other']\n[[package]]\nname='sdk'\nversion='1.0.0'\ndependencies=['itoa']\n[[package]]\nname='other'\nversion='1.0.0'\ndependencies=['itoa']\n", lock(CRATES_IO, &"a".repeat(64)))).unwrap();
        resolved
            .check_mod(
                "example",
                "mod",
                &std::collections::BTreeSet::from(["sdk".into(), "other".into()]),
                &RegistryLock::default(),
            )
            .unwrap();
    }

    #[test]
    fn source_and_checksum_boundary() {
        let checksum = "a".repeat(64);
        assert_eq!(
            RegistryLock::parse(&lock(CRATES_IO, &checksum))
                .unwrap()
                .packages
                .len(),
            1
        );
        for source in [
            "git+https://example.invalid/repo",
            "registry+https://example.invalid/index",
            "sparse+https://index.crates.io/",
        ] {
            assert!(RegistryLock::parse(&lock(source, &checksum))
                .unwrap_err()
                .contains("only crates.io"));
        }
        for checksum in ["", "no checksum", &"A".repeat(64), &"a".repeat(63)] {
            assert!(RegistryLock::parse(&lock(CRATES_IO, checksum))
                .unwrap_err()
                .contains("checksum"));
        }
        assert!(RegistryLock::parse("version = 4").is_err());
    }
    #[test]
    fn union_preserves_pins_and_refuses_conflicting_checksums() {
        let mut a = RegistryLock::parse(&lock(CRATES_IO, &"a".repeat(64))).unwrap();
        let same = a.clone();
        a.extend(&same).unwrap();
        assert_eq!(a.packages.len(), 1);
        let different = RegistryLock::parse(&lock(CRATES_IO, &"b".repeat(64))).unwrap();
        assert!(a.extend(&different).unwrap_err().contains("itoa 1.0.15"));
    }
}
