//! The entries of the datapack registries mods declared, as one world loads them:
//! a candidate built from the winning pack files and checked
//! as a whole, then published as a set with a session and a generation, and read through
//! handles that check both. Nothing here touches the world; the game builds a candidate before
//! it commits any table and keeps the old set when building fails. See
//! `docs/design/mod-api/mod-datapack-registries.md`.
use super::datapack_registry::{
    DatapackDecodeError, DatapackEntry, DatapackFieldKind, DatapackRegistryDeclaration,
    DatapackValue, valid_namespaced,
};
use super::datapack_snapshot::{
    MAX_SNAPSHOT_BYTES, MAX_SNAPSHOT_ENTRIES, MAX_SNAPSHOT_REGISTRIES, snapshot_size,
};
use std::collections::{BTreeMap, BTreeSet};

/// The most bytes one entry's winning file may take.
pub const MAX_DATAPACK_ENTRY_BYTES: usize = 1 << 20;

/// Why a candidate was refused. The whole candidate fails, never one entry.
#[derive(Clone, Debug, PartialEq, Eq)]
// no wire: loading runs in the game; mods read published entries, never load failures
#[non_exhaustive]
pub enum DatapackLoadReason {
    /// The winning file is not UTF-8 text.
    NotUtf8,
    /// More registries are declared than [`MAX_SNAPSHOT_REGISTRIES`]; `registry` is the first
    /// one over.
    TooManyRegistries,
    /// The winning file is more than [`MAX_DATAPACK_ENTRY_BYTES`].
    EntryTooLarge,
    /// A registry has more entries, tags, or members of one tag than [`MAX_SNAPSHOT_ENTRIES`].
    TooManyEntries,
    /// The winning files of the whole candidate are more than [`MAX_SNAPSHOT_BYTES`]; `entry` is
    /// the first one over.
    CandidateTooLarge,
    /// The snapshot of every synchronized registry would encode to more than
    /// [`MAX_SNAPSHOT_BYTES`] (field defaults and entry ids are not in the files); it names no
    /// registry, entry or file.
    SnapshotTooLarge,
    /// The entry's id (from its namespace and file name) is not a lowercase `namespace:path`.
    InvalidEntryId,
    /// The file did not decode against the registry's schema.
    Decode(DatapackDecodeError),
    /// A resolved tag has an invalid name or names an entry outside its accepted registry.
    InvalidTag {
        /// The tag's canonical name.
        tag: String,
        /// The invalid member, or empty when the tag name or registry is invalid.
        member: String,
    },
    /// A reference names an entry of a declared registry that no pack defines.
    UnresolvedReference {
        /// The referring field.
        field: String,
        /// The registry the field names.
        registry: String,
        /// The entry it names (`namespace:path`).
        target: String,
    },
}

/// A refused candidate: which registry, entry and file, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
// no wire: loading runs in the game; mods read published entries, never load failures
// exhaustive: the game builds it with a literal and reports it
pub struct DatapackLoadError {
    /// The registry's key (`example:spell`).
    pub registry: String,
    /// The entry's id (`addon:fire`), as far as it was known.
    pub entry: String,
    /// The file the entry came from.
    pub file: String,
    /// The id of the pack whose version of `file` won (`file/override`); empty when no pack is
    /// involved, as for a registry over the limit or a snapshot a server sent.
    pub pack: String,
    /// What was wrong.
    pub reason: DatapackLoadReason,
}

/// Every entry of every declared registry, decoded and checked, not yet published.
#[derive(Clone, Debug, PartialEq)]
// no wire: the game's candidate; a mod reads the published set through handles
// exhaustive: built only by [`DatapackCandidate::build`]
pub struct DatapackCandidate {
    pub(super) registries: BTreeMap<String, BTreeMap<String, DatapackEntry>>,
    pub(super) tags: BTreeMap<String, BTreeMap<String, Vec<String>>>,
    pub(super) snapshot_bytes: usize,
    pub(super) synchronized: BTreeSet<String>,
}

/// The id of the entry in `file` (`data/addon/example/spell/fire.json` is `addon:fire`), when
/// the file is in `declaration`'s folder.
fn entry_of(declaration: &DatapackRegistryDeclaration, file: &str) -> Option<String> {
    let rest = file.strip_prefix("data/")?;
    let (namespace, rest) = rest.split_once('/')?;
    let (registry_namespace, registry_path) = declaration.key.split_once(':')?;
    let path = rest
        .strip_prefix(registry_namespace)?
        .strip_prefix('/')?
        .strip_prefix(registry_path)?
        .strip_prefix('/')?
        .strip_suffix(".json")?;
    (!path.is_empty()).then(|| format!("{namespace}:{path}"))
}

impl DatapackCandidate {
    /// Decodes the winning file of every entry of every declared registry in `files`
    /// (`data/<entry namespace>/<registry namespace>/<registry path>/<entry>.json`, each with
    /// its versions in pack order, the last winning as `ResourceManager` hands them out), then
    /// resolves every reference to a declared registry against the whole candidate, so a
    /// forward reference works and an absent target refuses everything. A reference to a
    /// registry no declaration here covers (a vanilla one) is not checked. The caller has
    /// already dropped files whose pack conditions do not hold. Each version is its pack's id
    /// and its bytes, so a refusal names the pack. A candidate over a limit is refused whole:
    /// [`MAX_SNAPSHOT_REGISTRIES`] registries, [`MAX_DATAPACK_ENTRY_BYTES`] per entry,
    /// [`MAX_SNAPSHOT_ENTRIES`] entries per registry and [`MAX_SNAPSHOT_BYTES`] of winning files
    /// in all, the bounds its snapshot must also fit, and a trial encode refuses a candidate whose
    /// snapshot of every synchronized registry would be over [`MAX_SNAPSHOT_BYTES`]. The error is boxed: it is large, and a
    /// refused candidate is rare.
    pub fn build(
        declarations: &[DatapackRegistryDeclaration],
        files: &[(String, Vec<(String, Vec<u8>)>)],
    ) -> Result<Self, Box<DatapackLoadError>> {
        if let Some(over) = declarations.get(MAX_SNAPSHOT_REGISTRIES) {
            return Err(Box::new(DatapackLoadError {
                registry: over.key.clone(),
                entry: String::new(),
                file: String::new(),
                pack: String::new(),
                reason: DatapackLoadReason::TooManyRegistries,
            }));
        }
        let mut registries: BTreeMap<String, BTreeMap<String, DatapackEntry>> = declarations
            .iter()
            .map(|declaration| (declaration.key.clone(), BTreeMap::new()))
            .collect();
        let mut total = 0usize;
        for declaration in declarations {
            let fail = |entry: &str, file: &str, pack: &str, reason| {
                Box::new(DatapackLoadError {
                    registry: declaration.key.clone(),
                    entry: entry.to_owned(),
                    file: file.to_owned(),
                    pack: pack.to_owned(),
                    reason,
                })
            };
            for (file, versions) in files {
                let Some(entry) = entry_of(declaration, file) else {
                    continue;
                };
                let winner = versions.last();
                let pack = winner.map_or("", |(pack, _)| pack.as_str());
                let fail = |reason| fail(&entry, file, pack, reason);
                if !valid_namespaced(&entry) {
                    return Err(fail(DatapackLoadReason::InvalidEntryId));
                }
                let Some((_, bytes)) = winner else {
                    continue;
                };
                if bytes.len() > MAX_DATAPACK_ENTRY_BYTES {
                    return Err(fail(DatapackLoadReason::EntryTooLarge));
                }
                total += bytes.len();
                if total > MAX_SNAPSHOT_BYTES {
                    return Err(fail(DatapackLoadReason::CandidateTooLarge));
                }
                let entries = registries
                    .get_mut(&declaration.key)
                    .expect("every declaration has a registry");
                if entries.len() >= MAX_SNAPSHOT_ENTRIES {
                    return Err(fail(DatapackLoadReason::TooManyEntries));
                }
                let text =
                    std::str::from_utf8(bytes).map_err(|_| fail(DatapackLoadReason::NotUtf8))?;
                // Account raw source bytes above; conditions belong to the engine,
                // not to the declaring mod's schema.
                let stripped = serde_json::from_str::<serde_json::Value>(text)
                    .ok()
                    .and_then(|mut json| {
                        json.as_object_mut()?.remove("neoforge:conditions")?;
                        serde_json::to_string(&json).ok()
                    });
                let decoded = declaration
                    .decode_entry(stripped.as_deref().unwrap_or(text))
                    .map_err(|error| fail(DatapackLoadReason::Decode(error)))?;
                entries.insert(entry, decoded);
            }
        }
        resolve_references(declarations, &registries).map_err(|mut error| {
            // The referring entry's winning pack.
            if let Some((_, versions)) = files.iter().find(|(file, _)| *file == error.file) {
                error.pack = versions
                    .last()
                    .map_or_else(String::new, |(pack, _)| pack.clone());
            }
            error
        })?;
        let Some(snapshot_bytes) = snapshot_size(declarations, &registries, |_| true) else {
            return Err(Box::new(DatapackLoadError {
                registry: String::new(),
                entry: String::new(),
                file: String::new(),
                pack: String::new(),
                reason: DatapackLoadReason::SnapshotTooLarge,
            }));
        };
        Ok(Self {
            registries,
            tags: BTreeMap::new(),
            snapshot_bytes,
            synchronized: declarations
                .iter()
                .filter(|d| d.sync != super::datapack_registry::DatapackRegistrySync::ServerOnly)
                .map(|d| d.key.clone())
                .collect(),
        })
    }

    /// The accepted entry names of a registry, sorted, before publication.
    pub fn entries(&self, registry: &str) -> Result<Vec<&str>, DatapackReadError> {
        Ok(self
            .registries
            .get(registry)
            .ok_or(DatapackReadError::UnknownRegistry)?
            .keys()
            .map(String::as_str)
            .collect())
    }

    /// Bind the engine's fully resolved tags atomically against accepted entries.
    /// Invalid names, missing members and snapshots over the existing bounds are refused.
    pub fn bind_tags(
        &mut self,
        registry: &str,
        tags: BTreeMap<String, Vec<String>>,
    ) -> Result<(), Box<DatapackLoadError>> {
        let fail = |reason| {
            Box::new(DatapackLoadError {
                registry: registry.to_owned(),
                entry: String::new(),
                file: String::new(),
                pack: String::new(),
                reason,
            })
        };
        let entries = self.registries.get(registry).ok_or_else(|| {
            fail(DatapackLoadReason::InvalidTag {
                tag: String::new(),
                member: String::new(),
            })
        })?;
        if tags.len() > MAX_SNAPSHOT_ENTRIES
            || tags
                .values()
                .any(|members| members.len() > MAX_SNAPSHOT_ENTRIES)
        {
            return Err(fail(DatapackLoadReason::TooManyEntries));
        }
        for (tag, members) in &tags {
            if !valid_namespaced(tag) {
                return Err(fail(DatapackLoadReason::InvalidTag {
                    tag: tag.clone(),
                    member: String::new(),
                }));
            }
            if let Some(member) = members.iter().find(|id| !entries.contains_key(*id)) {
                return Err(fail(DatapackLoadReason::InvalidTag {
                    tag: tag.clone(),
                    member: member.clone(),
                }));
            }
        }
        let encoded_len = |map: &BTreeMap<String, Vec<String>>| {
            let mut out = crate::wire::Writer::new();
            out.limit(MAX_SNAPSHOT_BYTES);
            out.put(map);
            out.error().is_none().then(|| out.bytes().len())
        };
        let new_size =
            encoded_len(&tags).ok_or_else(|| fail(DatapackLoadReason::SnapshotTooLarge))?;
        let snapshot_bytes = if self.synchronized.contains(registry) {
            let old_size = self.tags.get(registry).and_then(encoded_len).unwrap_or(4);
            self.snapshot_bytes
                .checked_sub(old_size)
                .and_then(|size| size.checked_add(new_size))
                .filter(|size| *size <= MAX_SNAPSHOT_BYTES)
                .ok_or_else(|| fail(DatapackLoadReason::SnapshotTooLarge))?
        } else {
            self.snapshot_bytes
        };
        self.tags.insert(registry.to_owned(), tags);
        self.snapshot_bytes = snapshot_bytes;
        Ok(())
    }
}

/// Why a handle or a lookup failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum DatapackReadError {
    /// The handle is from another world session, as after closing and reopening the world.
    WrongSession,
    /// The handle is from an earlier generation: the registries reloaded since.
    StaleGeneration,
    /// No registry has this key.
    UnknownRegistry,
    /// The registry has no such entry (a reload removed it, or no pack defines it).
    MissingEntry,
    /// The game has no world open to read from, or no host answered.
    Unavailable,
}
impl crate::NoBackend for DatapackReadError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

/// A reference to one entry of a published set. It checks its session and generation on every
/// read, so one kept across a reload fails instead of reading the new value by name or slot.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: the game issues it; a mod holds it and reads through the set
#[derive(horizon_mod_api_macros::Wire)]
pub struct DatapackHandle {
    session: u64,
    generation: u64,
    registry: String,
    entry: String,
}

impl DatapackHandle {
    /// The registry's key.
    pub fn registry(&self) -> &str {
        &self.registry
    }

    /// The entry's id.
    pub fn entry(&self) -> &str {
        &self.entry
    }
}

/// Checks every reference to a declared registry against the whole `registries`, so a forward
/// reference works and an absent target refuses everything. A reference to a registry no
/// declaration covers (a vanilla one) is not checked.
pub(super) fn resolve_references(
    declarations: &[DatapackRegistryDeclaration],
    registries: &BTreeMap<String, BTreeMap<String, DatapackEntry>>,
) -> Result<(), Box<DatapackLoadError>> {
    for declaration in declarations {
        for (entry, decoded) in &registries[&declaration.key] {
            for field in &declaration.fields {
                let Some(value) = decoded.get(&field.name) else {
                    continue;
                };
                let Some((path, registry, target)) =
                    dangling_reference(&field.kind, value, &field.name, registries)
                else {
                    continue;
                };
                return Err(Box::new(DatapackLoadError {
                    registry: declaration.key.clone(),
                    entry: entry.clone(),
                    file: declaration.entry_file(entry).unwrap_or_default(),
                    pack: String::new(),
                    reason: DatapackLoadReason::UnresolvedReference {
                        field: path,
                        registry: registry.to_owned(),
                        target: target.to_owned(),
                    },
                }));
            }
        }
    }
    Ok(())
}

/// The first reference in `value`, nested lists and objects included, whose declared registry
/// lacks its target: `(field path, registry, target)`, the path named as the decoder names it
/// (`tags[0]`, `display.spell`).
fn dangling_reference<'a>(
    kind: &'a DatapackFieldKind,
    value: &'a DatapackValue,
    path: &str,
    registries: &BTreeMap<String, BTreeMap<String, DatapackEntry>>,
) -> Option<(String, &'a str, &'a str)> {
    match (kind, value) {
        (DatapackFieldKind::Reference { registry }, DatapackValue::Text(target)) => registries
            .get(registry)
            .is_some_and(|targets| !targets.contains_key(target))
            .then(|| (path.to_owned(), registry.as_str(), target.as_str())),
        (DatapackFieldKind::List { element, .. }, DatapackValue::List(values)) => {
            values.iter().enumerate().find_map(|(index, value)| {
                dangling_reference(element, value, &format!("{path}[{index}]"), registries)
            })
        }
        (DatapackFieldKind::Object { fields, .. }, DatapackValue::Object(values)) => {
            values.iter().find_map(|(name, value)| {
                let field = fields.iter().find(|field| field.name == *name)?;
                dangling_reference(&field.kind, value, &format!("{path}.{name}"), registries)
            })
        }
        _ => None,
    }
}

/// Why a set could not move to a new generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// no wire: the game's reload reports it
#[non_exhaustive]
pub enum DatapackGenerationExhausted {
    /// The generation counter is at its end; it never wraps.
    Exhausted,
}

/// The published entries of a world session at one generation.
#[derive(Clone, Debug, PartialEq)]
// no wire: the game holds it; a mod reads through handles
// exhaustive: built only by [`DatapackRegistrySet::first`] and [`DatapackRegistrySet::reload`]
pub struct DatapackRegistrySet {
    pub(super) session: u64,
    pub(super) generation: u64,
    pub(super) registries: BTreeMap<String, BTreeMap<String, DatapackEntry>>,
    pub(super) tags: BTreeMap<String, BTreeMap<String, Vec<String>>>,
    /// None on a server; on a client, exactly the registries in its accepted snapshot.
    pub(super) received_registries: Option<BTreeSet<String>>,
}

impl DatapackRegistrySet {
    /// Publishes `candidate` as a world session's first set: generation 1.
    pub fn first(candidate: DatapackCandidate, session: u64) -> Self {
        Self {
            session,
            generation: 1,
            registries: candidate.registries,
            tags: candidate.tags,
            received_registries: None,
        }
    }

    /// Publishes `candidate` after a successful reload of the same session: the next
    /// generation, even when the content is identical. A failed reload never gets here, so it
    /// takes no generation; the old set stays.
    pub fn reload(
        &self,
        candidate: DatapackCandidate,
    ) -> Result<Self, DatapackGenerationExhausted> {
        Ok(Self {
            session: self.session,
            generation: self
                .generation
                .checked_add(1)
                .ok_or(DatapackGenerationExhausted::Exhausted)?,
            registries: candidate.registries,
            tags: candidate.tags,
            received_registries: None,
        })
    }

    /// The generation, from 1.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Whether generic reads may access this registry. Typed reads retain their existing
    /// empty defaults for omitted declarations; a received empty registry is still available.
    pub fn registry_available(&self, registry: &str) -> bool {
        self.received_registries.as_ref().map_or_else(
            || self.registries.contains_key(registry),
            |received| received.contains(registry),
        )
    }

    /// A handle to `entry` of `registry` in this set.
    pub fn handle(&self, registry: &str, entry: &str) -> Result<DatapackHandle, DatapackReadError> {
        self.lookup(registry, entry)?;
        Ok(DatapackHandle {
            session: self.session,
            generation: self.generation,
            registry: registry.to_owned(),
            entry: entry.to_owned(),
        })
    }

    /// The entry a handle names, when the handle is of this session and generation and the
    /// entry is still there.
    pub fn get(&self, handle: &DatapackHandle) -> Result<&DatapackEntry, DatapackReadError> {
        if handle.session != self.session {
            return Err(DatapackReadError::WrongSession);
        }
        if handle.generation != self.generation {
            return Err(DatapackReadError::StaleGeneration);
        }
        self.lookup(&handle.registry, &handle.entry)
    }

    /// The entry ids of `registry` in this set, sorted.
    pub fn entries(&self, registry: &str) -> Result<Vec<&str>, DatapackReadError> {
        let entries = self
            .registries
            .get(registry)
            .ok_or(DatapackReadError::UnknownRegistry)?;
        Ok(entries.keys().map(String::as_str).collect())
    }

    /// Whether the current set contains this entry, without constructing an owned handle.
    /// Unknown registries and missing entries both return false.
    pub fn contains_entry(&self, registry: &str, id: &str) -> bool {
        self.lookup(registry, id).is_ok()
    }

    /// Current merged-tag membership. None for an unknown registry or entry; absent tags are false.
    pub fn tag_contains(&self, registry: &str, entry: &str, tag: &str) -> Option<bool> {
        self.lookup(registry, entry).ok()?;
        Some(
            self.tags
                .get(registry)
                .and_then(|tags| tags.get(tag))
                .is_some_and(|members| members.iter().any(|member| member == entry)),
        )
    }

    fn lookup(&self, registry: &str, entry: &str) -> Result<&DatapackEntry, DatapackReadError> {
        self.registries
            .get(registry)
            .ok_or(DatapackReadError::UnknownRegistry)?
            .get(entry)
            .ok_or(DatapackReadError::MissingEntry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::datapack_registry::{DatapackField, DatapackRegistrySync};

    /// `example:spell`: `{power: integer 0..100, next: reference to example:spell = example:fire}`.
    fn spell() -> DatapackRegistryDeclaration {
        DatapackRegistryDeclaration {
            mod_id: "example".into(),
            key: "example:spell".into(),
            fields: vec![
                DatapackField::required("power", DatapackFieldKind::Int { min: 0, max: 100 }),
                DatapackField::with_default(
                    "next",
                    DatapackFieldKind::Reference {
                        registry: "example:spell".into(),
                    },
                    DatapackValue::Text("example:fire".into()),
                ),
            ],
            sync: DatapackRegistrySync::ServerOnly,
        }
    }

    /// A file whose version `n` is in the pack `pack/<n>`.
    fn file(path: &str, versions: &[&str]) -> (String, Vec<(String, Vec<u8>)>) {
        (
            path.to_owned(),
            versions
                .iter()
                .enumerate()
                .map(|(n, v)| (format!("pack/{n}"), v.as_bytes().to_vec()))
                .collect(),
        )
    }

    fn power(set: &DatapackRegistrySet, entry: &str) -> DatapackValue {
        let handle = set.handle("example:spell", entry).unwrap();
        set.get(&handle).unwrap().get("power").unwrap().clone()
    }

    fn candidate(files: &[(String, Vec<(String, Vec<u8>)>)]) -> DatapackCandidate {
        DatapackCandidate::build(&[spell()], files).unwrap()
    }

    /// Base fire power 3, a higher pack's 7: one entry, the winner's value, generation 1.
    #[test]
    fn the_winning_pack_file_is_the_one_entry() {
        let files = [file(
            "data/example/example/spell/fire.json",
            &[r#"{"power": 3}"#, r#"{"power": 7}"#],
        )];
        let set = DatapackRegistrySet::first(candidate(&files), 1);
        assert_eq!(set.generation(), 1);
        assert_eq!(power(&set, "example:fire"), DatapackValue::Int(7));
        assert_eq!(set.entries("example:spell").unwrap(), ["example:fire"]);
        assert!(set.contains_entry("example:spell", "example:fire"));
        assert!(!set.contains_entry("example:spell", "example:missing"));
        assert!(!set.contains_entry("example:unknown", "example:fire"));
    }

    /// An entry in another mod's namespace sits in that namespace's folder; nested paths work;
    /// files of other registries and other folders are not entries.
    #[test]
    fn entries_come_from_their_namespaced_folder_only() {
        let files = [
            file("data/example/example/spell/fire.json", &[r#"{"power": 3}"#]),
            file("data/addon/example/spell/ice.json", &[r#"{"power": 4}"#]),
            file(
                "data/addon/example/spell/nested/hail.json",
                &[r#"{"power": 5}"#],
            ),
            file("data/example/example/other/fire.json", &[r#"{"power": 1}"#]),
            file("data/example/spell/fire.json", &[r#"{"power": 1}"#]),
            file("data/addon/example/spell/ice.txt", &[r#"{"power": 1}"#]),
        ];
        let set = DatapackRegistrySet::first(candidate(&files), 1);
        assert_eq!(
            set.entries("example:spell").unwrap(),
            ["addon:ice", "addon:nested/hail", "example:fire"]
        );
        assert_eq!(
            set.entries("example:nothing"),
            Err(DatapackReadError::UnknownRegistry)
        );
    }

    /// Reload to 9: new read, generation 2, the generation-1 handle is stale. Remove fire: a
    /// fresh lookup is MissingEntry, the old handle StaleGeneration. An identical reload
    /// still advances once.
    #[test]
    fn a_reload_stales_old_handles_and_removal_is_missing() {
        let files = [file(
            "data/example/example/spell/fire.json",
            &[r#"{"power": 7}"#],
        )];
        let one = DatapackRegistrySet::first(candidate(&files), 1);
        let held = one.handle("example:spell", "example:fire").unwrap();
        let nine = [file(
            "data/example/example/spell/fire.json",
            &[r#"{"power": 7}"#, r#"{"power": 9}"#],
        )];
        let two = one.reload(candidate(&nine)).unwrap();
        assert_eq!((one.generation(), two.generation()), (1, 2));
        assert_eq!(power(&two, "example:fire"), DatapackValue::Int(9));
        // The old handle fails on the new set, whatever the name and slot.
        assert_eq!(two.get(&held), Err(DatapackReadError::StaleGeneration));
        // On the old set it still reads the old value.
        assert_eq!(
            one.get(&held).unwrap().get("power"),
            Some(&DatapackValue::Int(7))
        );
        // Identical content: still the next generation, once.
        let three = two.reload(candidate(&nine)).unwrap();
        assert_eq!(three.generation(), 3);
        // Remove fire.
        let four = three.reload(candidate(&[])).unwrap();
        assert_eq!(
            four.handle("example:spell", "example:fire"),
            Err(DatapackReadError::MissingEntry)
        );
        let held_three = three.handle("example:spell", "example:fire").unwrap();
        assert_eq!(
            four.get(&held_three),
            Err(DatapackReadError::StaleGeneration)
        );
    }

    /// Close and reopen: a distinct session at generation 1; the old handle is WrongSession even
    /// with the same name and generation.
    #[test]
    fn a_reopened_world_is_another_session() {
        let files = [file(
            "data/example/example/spell/fire.json",
            &[r#"{"power": 7}"#],
        )];
        let first = DatapackRegistrySet::first(candidate(&files), 1);
        let held = first.handle("example:spell", "example:fire").unwrap();
        let reopened = DatapackRegistrySet::first(candidate(&files), 2);
        assert_eq!(reopened.generation(), 1);
        assert_eq!(reopened.get(&held), Err(DatapackReadError::WrongSession));
    }

    /// Invalid winning data (power 101, malformed JSON) fails the candidate with the file and
    /// field, and does not fall back to the lower-priority version; the old set is untouched.
    #[test]
    fn invalid_winning_data_refuses_the_candidate_and_keeps_the_old_set() {
        let good = [file(
            "data/example/example/spell/fire.json",
            &[r#"{"power": 7}"#],
        )];
        let old = DatapackRegistrySet::first(candidate(&good), 1);
        let bad = [file(
            "data/example/example/spell/fire.json",
            &[r#"{"power": 7}"#, r#"{"power": 101}"#],
        )];
        assert_eq!(
            DatapackCandidate::build(&[spell()], &bad),
            Err(Box::new(DatapackLoadError {
                registry: "example:spell".into(),
                entry: "example:fire".into(),
                file: "data/example/example/spell/fire.json".into(),
                pack: "pack/1".into(),
                reason: DatapackLoadReason::Decode(DatapackDecodeError::OutOfRange {
                    field: "power".into()
                }),
            }))
        );
        let malformed = [file(
            "data/example/example/spell/fire.json",
            &[r#"{"power": 7}"#, "{"],
        )];
        let refused = DatapackCandidate::build(&[spell()], &malformed).unwrap_err();
        assert!(matches!(
            refused.reason,
            DatapackLoadReason::Decode(DatapackDecodeError::Malformed(_))
        ));
        // A later valid reload still publishes.
        assert_eq!(old.generation(), 1);
        assert_eq!(power(&old, "example:fire"), DatapackValue::Int(7));
        assert_eq!(old.reload(candidate(&good)).unwrap().generation(), 2);
    }

    /// A forward reference resolves after the candidate completes; an absent target refuses it
    /// all.
    #[test]
    fn references_resolve_against_the_whole_candidate() {
        let forward = [
            file(
                "data/example/example/spell/fire.json",
                &[r#"{"power": 4, "next": "addon:ice"}"#],
            ),
            file("data/addon/example/spell/ice.json", &[r#"{"power": 2}"#]),
        ];
        // `ice` leaves `next` at its default, `example:fire`, which exists: a cycle, which
        // references by name allow.
        let set = DatapackRegistrySet::first(candidate(&forward), 1);
        assert_eq!(power(&set, "example:fire"), DatapackValue::Int(4));
        let missing = [file(
            "data/example/example/spell/fire.json",
            &[r#"{"power": 4, "next": "addon:ice"}"#],
        )];
        assert_eq!(
            DatapackCandidate::build(&[spell()], &missing),
            Err(Box::new(DatapackLoadError {
                registry: "example:spell".into(),
                entry: "example:fire".into(),
                file: "data/example/example/spell/fire.json".into(),
                pack: "pack/0".into(),
                reason: DatapackLoadReason::UnresolvedReference {
                    field: "next".into(),
                    registry: "example:spell".into(),
                    target: "addon:ice".into(),
                },
            }))
        );
    }

    /// A reference inside a list or an object is resolved too, and the refusal names its path.
    #[test]
    fn nested_references_resolve_and_name_their_path() {
        let slot = DatapackRegistryDeclaration {
            mod_id: "example".into(),
            key: "example:slot".into(),
            fields: vec![
                DatapackField::with_default(
                    "spells",
                    DatapackFieldKind::List {
                        element: Box::new(DatapackFieldKind::Reference {
                            registry: "example:spell".into(),
                        }),
                        max_len: 4,
                    },
                    DatapackValue::List(Vec::new()),
                ),
                DatapackField::with_default(
                    "display",
                    DatapackFieldKind::Object {
                        fields: vec![DatapackField::with_default(
                            "spell",
                            DatapackFieldKind::Reference {
                                registry: "example:spell".into(),
                            },
                            DatapackValue::Text("example:fire".into()),
                        )],
                        max_len: 256,
                    },
                    DatapackValue::Object(vec![(
                        "spell".into(),
                        DatapackValue::Text("example:fire".into()),
                    )]),
                ),
            ],
            sync: DatapackRegistrySync::ServerOnly,
        };
        let fire = file("data/example/example/spell/fire.json", &[r#"{"power": 1}"#]);
        let build = |slot_json: &str| {
            let files = [
                fire.clone(),
                file("data/example/example/slot/ring.json", &[slot_json]),
            ];
            DatapackCandidate::build(&[spell(), slot.clone()], &files)
                .map(|_| ())
                .map_err(|error| error.reason)
        };
        assert_eq!(
            build(r#"{"spells": ["example:fire"], "display": {"spell": "example:fire"}}"#),
            Ok(())
        );
        let unresolved = |field: &str| {
            Err(DatapackLoadReason::UnresolvedReference {
                field: field.into(),
                registry: "example:spell".into(),
                target: "addon:ice".into(),
            })
        };
        assert_eq!(
            build(r#"{"spells": ["example:fire", "addon:ice"]}"#),
            unresolved("spells[1]")
        );
        assert_eq!(
            build(r#"{"display": {"spell": "addon:ice"}}"#),
            unresolved("display.spell")
        );
    }

    /// A reference to a registry no declaration covers (a vanilla one) is not checked here.
    #[test]
    fn a_reference_to_an_undeclared_registry_is_left_to_its_owner() {
        let mut declaration = spell();
        declaration.fields[1].kind = DatapackFieldKind::Reference {
            registry: "minecraft:block".into(),
        };
        declaration.fields[1].default = Some(DatapackValue::Text("minecraft:stone".into()));
        let files = [file(
            "data/example/example/spell/fire.json",
            &[r#"{"power": 1, "next": "minecraft:nothing"}"#],
        )];
        assert!(DatapackCandidate::build(&[declaration], &files).is_ok());
    }

    /// An entry whose id is not a lowercase `namespace:path`, or whose file is not UTF-8, refuses
    /// the candidate.
    #[test]
    fn an_unusable_entry_file_refuses_the_candidate() {
        let reason = |files: &[(String, Vec<(String, Vec<u8>)>)]| {
            DatapackCandidate::build(&[spell()], files)
                .unwrap_err()
                .reason
        };
        assert_eq!(
            reason(&[file(
                "data/example/example/spell/Fire.json",
                &[r#"{"power": 1}"#]
            )]),
            DatapackLoadReason::InvalidEntryId
        );
        assert_eq!(
            reason(&[(
                "data/example/example/spell/fire.json".to_owned(),
                vec![("pack/0".to_owned(), vec![0xff, 0xfe])]
            )]),
            DatapackLoadReason::NotUtf8
        );
    }

    /// With nothing declared, no file is read and the set is empty.
    #[test]
    fn nothing_declared_decodes_nothing() {
        let files = [file(
            "data/example/example/spell/fire.json",
            &[r#"{"power": 101}"#],
        )];
        let candidate = DatapackCandidate::build(&[], &files).unwrap();
        let set = DatapackRegistrySet::first(candidate, 1);
        assert_eq!(
            set.entries("example:spell"),
            Err(DatapackReadError::UnknownRegistry)
        );
    }

    /// The generation never wraps.
    #[test]
    fn the_generation_counter_refuses_to_wrap() {
        let mut set = DatapackRegistrySet::first(candidate(&[]), 1);
        set.generation = u64::MAX;
        assert_eq!(
            set.reload(candidate(&[])),
            Err(DatapackGenerationExhausted::Exhausted)
        );
    }

    /// `{"power": 7}` padded with spaces to exactly `len` bytes.
    fn padded(len: usize) -> String {
        let head = r#"{"power": 7"#;
        format!("{head}{}}}", " ".repeat(len - head.len() - 1))
    }

    fn entry_path(name: &str) -> String {
        format!("data/example/example/spell/{name}.json")
    }

    /// The refusal of `files`, which must name `entry` and the winning pack.
    fn refusal(files: &[(String, Vec<(String, Vec<u8>)>)], entry: &str) -> DatapackLoadReason {
        let error = DatapackCandidate::build(&[spell()], files).unwrap_err();
        assert_eq!(error.registry, "example:spell");
        assert_eq!(error.entry, format!("example:{entry}"));
        assert_eq!(error.file, entry_path(entry));
        assert_eq!(error.pack, "pack/0");
        error.reason
    }

    /// An entry of exactly 1 MiB decodes; one byte more refuses the candidate, the old set
    /// stays at its generation and the next valid reload still publishes.
    #[test]
    fn an_entry_over_one_mib_refuses_the_candidate() {
        let at = [file(&entry_path("fire"), &[padded(1_048_576).as_str()])];
        let old = DatapackRegistrySet::first(candidate(&at), 1);
        assert_eq!(power(&old, "example:fire"), DatapackValue::Int(7));
        let over = [file(&entry_path("fire"), &[padded(1_048_577).as_str()])];
        assert_eq!(refusal(&over, "fire"), DatapackLoadReason::EntryTooLarge);
        assert_eq!(old.generation(), 1);
        assert_eq!(power(&old, "example:fire"), DatapackValue::Int(7));
        assert_eq!(old.reload(candidate(&at)).unwrap().generation(), 2);
    }

    /// 65,536 entries in a registry are accepted; one more refuses the candidate.
    #[test]
    #[ignore = "daily"]
    fn a_registry_over_65536_entries_refuses_the_candidate() {
        let mut files: Vec<_> = (0..65_536)
            .map(|n| match n {
                0 => file(&entry_path("fire"), &[r#"{"power": 1}"#]),
                n => file(&entry_path(&format!("e{n}")), &[r#"{"power": 1}"#]),
            })
            .collect();
        let old = DatapackRegistrySet::first(candidate(&files), 1);
        assert_eq!(old.entries("example:spell").unwrap().len(), 65_536);
        let over = "e65536".to_owned();
        files.push(file(&entry_path(&over), &[r#"{"power": 1}"#]));
        assert_eq!(refusal(&files, &over), DatapackLoadReason::TooManyEntries);
        files.pop();
        assert_eq!(old.reload(candidate(&files)).unwrap().generation(), 2);
    }

    /// Exactly 67,108,864 source bytes fit; one extra byte refuses the candidate.
    #[test]
    #[ignore = "daily"]
    fn a_candidate_over_64_mib_refuses_the_candidate() {
        let full = padded(1_048_576);
        let mut files: Vec<_> = (0..63)
            .map(|n| match n {
                0 => file(&entry_path("fire"), &[full.as_str()]),
                n => file(&entry_path(&format!("e{n}")), &[full.as_str()]),
            })
            .collect();
        files.push(file(&entry_path("half"), &[padded(524_288).as_str()]));
        files.push(file(&entry_path("last"), &[padded(524_288).as_str()]));
        let old = DatapackRegistrySet::first(candidate(&files), 1);
        files.last_mut().unwrap().1[0].1.push(b' ');
        assert_eq!(
            refusal(&files, "last"),
            DatapackLoadReason::CandidateTooLarge
        );
        files.last_mut().unwrap().1[0].1.pop();
        assert_eq!(old.generation(), 1);
        assert_eq!(power(&old, "example:fire"), DatapackValue::Int(7));
        assert_eq!(old.reload(candidate(&files)).unwrap().generation(), 2);
    }

    /// `example:spell`, synchronized, with a 64 KiB text default that no file spells out.
    fn padded_spell() -> DatapackRegistryDeclaration {
        DatapackRegistryDeclaration {
            fields: vec![
                DatapackField::required("power", DatapackFieldKind::Int { min: 0, max: 100 }),
                DatapackField::with_default(
                    "lore",
                    DatapackFieldKind::Text { max_len: 1 << 16 },
                    DatapackValue::Text("x".repeat(1 << 16)),
                ),
            ],
            sync: DatapackRegistrySync::Optional,
            ..spell()
        }
    }

    /// `count` small entries, the first `example:fire`.
    fn small_entries(count: usize) -> Vec<(String, Vec<(String, Vec<u8>)>)> {
        (0..count)
            .map(|n| match n {
                0 => file(&entry_path("fire"), &[r#"{"power": 1}"#]),
                n => file(&entry_path(&format!("e{n}")), &[r#"{"power": 1}"#]),
            })
            .collect()
    }

    /// A candidate whose files are small but whose snapshot, defaults filled in, would encode to
    /// more than 64 MiB is refused before it commits, with a named reason; the old generation
    /// stays and the next candidate that fits still publishes. A server-only registry is never
    /// sent, so the same entries there are accepted.
    #[test]
    fn a_candidate_whose_snapshot_is_over_64_mib_is_refused_before_commit() {
        let declarations = [padded_spell()];
        let fits = small_entries(1000);
        let old =
            DatapackRegistrySet::first(DatapackCandidate::build(&declarations, &fits).unwrap(), 1);
        let over = small_entries(MAX_SNAPSHOT_BYTES / (1 << 16) + 1);
        let error = DatapackCandidate::build(&declarations, &over).unwrap_err();
        assert_eq!(error.reason, DatapackLoadReason::SnapshotTooLarge);
        assert_eq!(error.registry, "");
        assert_eq!(old.generation(), 1);
        let next = DatapackCandidate::build(&declarations, &fits).unwrap();
        assert_eq!(old.reload(next).unwrap().generation(), 2);
        let server_only = DatapackRegistryDeclaration {
            sync: DatapackRegistrySync::ServerOnly,
            ..padded_spell()
        };
        assert!(DatapackCandidate::build(&[server_only], &over).is_ok());
    }

    /// A file nested 65 deep refuses the candidate (the schema has no nested field, so it is a
    /// decode error), and the old generation stays.
    #[test]
    fn an_entry_nested_65_deep_refuses_the_candidate() {
        let old = DatapackRegistrySet::first(candidate(&small_entries(1)), 1);
        let deep = format!(r#"{{"power": {}7{}}}"#, "[".repeat(64), "]".repeat(64));
        let error =
            DatapackCandidate::build(&[spell()], &[file(&entry_path("fire"), &[deep.as_str()])])
                .unwrap_err();
        assert!(matches!(error.reason, DatapackLoadReason::Decode(_)));
        assert_eq!(old.generation(), 1);
        assert_eq!(power(&old, "example:fire"), DatapackValue::Int(1));
    }

    /// 256 declared registries are accepted; the 257th refuses every candidate, naming it.
    #[test]
    fn more_than_256_registries_refuse_the_candidate() {
        let mut declarations: Vec<_> = (0..256)
            .map(|n| DatapackRegistryDeclaration {
                key: format!("example:spell_{n}"),
                ..spell()
            })
            .collect();
        assert!(DatapackCandidate::build(&declarations, &[]).is_ok());
        declarations.push(DatapackRegistryDeclaration {
            key: "example:over".into(),
            ..spell()
        });
        let error = DatapackCandidate::build(&declarations, &[]).unwrap_err();
        assert_eq!(error.registry, "example:over");
        assert_eq!(error.reason, DatapackLoadReason::TooManyRegistries);
    }
}

#[cfg(feature = "testing")]
#[path = "datapack_registry_fixture_support.rs"]
mod fixture_support;
