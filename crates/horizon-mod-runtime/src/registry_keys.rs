//! Load-owned declarations and resolved mappings for registration-slot keys.
use std::collections::{BTreeMap, HashMap};
use std::sync::{LazyLock, RwLock};

#[derive(Clone, Default)]
pub struct Requests {
    pub game_rules: Names,
    pub blocks: Names,
    pub fluids: Names,
    pub declared_fluids: HashMap<String, &'static str>,
    pub items: Names,
    pub effects: Names,
    pub particles: Names,
    pub block_entity_types: Names,
    pub entity_types: Names,
    pub game_events: Names,
}

#[derive(Clone, Default)]
pub struct Names {
    pub entries: Vec<Request>,
    pub slots: HashMap<String, u32>,
    /// Typed identifier lookup without formatting a string on metadata reads.
    pub identifiers: BTreeMap<horizon_core::identifier::Identifier, u32>,
    pub utf16_slots: HashMap<Vec<u16>, u32>,
    pub world_fluids: Vec<Option<u32>>,
    /// Slots in the completed registry's registration order, when a completion
    /// recorded one. Empty while the table holds only mod requests.
    pub order: Vec<u32>,
}

#[derive(Clone)]
pub struct Request {
    pub id: String,
    pub mods: Vec<&'static str>,
}

impl Names {
    pub fn request(&mut self, mod_id: &'static str, id: &str) -> Result<u32, String> {
        // Match the registry boundary's identifier classification, including the default namespace.
        let id = if id.contains(':') {
            id.to_owned()
        } else {
            format!("minecraft:{id}")
        };
        let identifier = horizon_core::identifier::Identifier::parse_namespaced(&id)
            .ok_or_else(|| format!("mod {mod_id}: invalid registry key {id:?}"))?;
        self.request_parsed(mod_id, id, identifier)
    }

    /// Request a name already validated by the owning identifier boundary.
    pub fn request_identifier(
        &mut self,
        mod_id: &'static str,
        identifier: &horizon_core::identifier::Identifier,
    ) -> Result<u32, String> {
        self.request_parsed(mod_id, identifier.to_string(), identifier.clone())
    }

    fn request_parsed(
        &mut self,
        mod_id: &'static str,
        id: String,
        identifier: horizon_core::identifier::Identifier,
    ) -> Result<u32, String> {
        let slot = if let Some(slot) = self.slots.get(&id) {
            *slot
        } else {
            let slot = u32::try_from(self.entries.len())
                .map_err(|_| "registry key slots exhausted".to_owned())?;
            self.slots.insert(id.clone(), slot);
            self.entries.push(Request {
                id,
                mods: Vec::new(),
            });
            slot
        };
        self.identifiers.insert(identifier, slot);
        let mods = &mut self.entries[slot as usize].mods;
        if !mods.contains(&mod_id) {
            mods.push(mod_id);
        }
        Ok(slot)
    }

    pub fn id(&self, slot: u32) -> Option<&str> {
        self.entries
            .get(slot as usize)
            .map(|entry| entry.id.as_str())
    }

    /// Complete registry identities without changing requested slots. Publish the returned
    /// table only after every forward reference has resolved.
    pub fn complete<'a, E>(
        &self,
        ids: impl IntoIterator<Item = &'a str>,
        unknown: impl FnMut(&'static str, &str) -> E,
    ) -> Result<Self, Vec<E>> {
        let all: Vec<_> = ids.into_iter().collect();
        let known: std::collections::BTreeSet<_> = all.iter().copied().collect();
        self.resolve(|id| known.contains(id).then_some(()), unknown)?;
        let mut names = self.clone();
        // Slots keep their historical sorted completion order, so adding an
        // entry to one registry generation never renumbers another.
        for id in &known {
            names
                .request("horizon", id)
                .expect("registry owns valid ids");
        }
        let mut seen = std::collections::BTreeSet::new();
        names.order = all
            .into_iter()
            .filter(|id| seen.insert(*id))
            .map(|id| {
                *names
                    .slots
                    .get(id)
                    .expect("completed registry owns every id")
            })
            .collect();
        Ok(names)
    }

    pub fn resolve<T, E>(
        &self,
        mut resolve: impl FnMut(&str) -> Option<T>,
        mut unknown: impl FnMut(&'static str, &str) -> E,
    ) -> Result<Vec<T>, Vec<E>> {
        let mut values = Vec::with_capacity(self.entries.len());
        let mut errors = Vec::new();
        for entry in &self.entries {
            match resolve(&entry.id) {
                Some(value) => values.push(value),
                None => errors.extend(entry.mods.iter().map(|mod_id| unknown(mod_id, &entry.id))),
            }
        }
        if errors.is_empty() {
            Ok(values)
        } else {
            Err(errors)
        }
    }
}

/// A particle's resolved engine-independent provider identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// no wire: local resolved provider metadata, not a cross-target particle payload.
pub enum ParticleKind {
    /// An index supplied by the game's vanilla particle registry.
    Vanilla(u32),
    /// A mod's named sprite provider, addressed by its retained name.
    Custom,
}

/// One immutable load result: identities and their resolved providers publish together.
pub struct Particles {
    pub names: Names,
    pub kinds: Vec<ParticleKind>,
    /// Reverse vanilla-provider lookup, built once with the forward table.
    pub vanilla_slots: HashMap<u32, u32>,
}

/// Resolve each provider once, keeping the same requested-slot lifecycle as other keys.
pub fn resolve_particles<'a>(
    requested: &Names,
    ids: impl IntoIterator<Item = &'a str>,
    resolve: impl FnMut(&str) -> Option<ParticleKind>,
) -> Result<Particles, Vec<String>> {
    let unknown = |owner, id: &str| format!("mod {owner} requested unknown particle {id}");
    let names = requested.complete(ids, unknown)?;
    let kinds = names.resolve(resolve, unknown)?;
    let vanilla_slots = kinds
        .iter()
        .enumerate()
        .filter_map(|(slot, kind)| match kind {
            ParticleKind::Vanilla(index) => Some((*index, slot as u32)),
            ParticleKind::Custom => None,
        })
        .collect();
    Ok(Particles {
        names,
        kinds,
        vanilla_slots,
    })
}

// Tag names are process-lifetime identities; only membership belongs to reloadable data.
// Reuse the same slot and identifier tables as registered keys, without a per-world cache.
static TAG_NAMES: LazyLock<RwLock<Names>> = LazyLock::new(RwLock::default);

/// Resolve a validated tag name once. Unknown membership does not prevent naming a tag.
/// An unavailable table or exhausted index space refuses resolution without panicking.
pub fn tag_slot(id: &horizon_core::identifier::Identifier) -> Option<u32> {
    if let Some(slot) = TAG_NAMES.read().ok()?.identifiers.get(id) {
        return Some(*slot);
    }
    TAG_NAMES
        .write()
        .ok()?
        .request_identifier("horizon", id)
        .ok()
}

/// Borrow a retained tag name while its table is read-locked. No string is allocated.
pub fn with_tag_name<R>(slot: u32, read: impl FnOnce(Option<&str>) -> R) -> R {
    match TAG_NAMES.read() {
        Ok(names) => read(names.id(slot)),
        Err(_) => read(None),
    }
}

/// The vanilla dimensions, at the slots of `DimensionKey`'s constants.
const VANILLA_DIMENSIONS: [&str; 3] = [
    "minecraft:overworld",
    "minecraft:the_nether",
    "minecraft:the_end",
];

/// The dimensions past vanilla's the game has met, in slot order. Only grows: a key handed out
/// stays valid until exit, and both of a process's `Game`s number a dimension alike.
#[derive(Default)]
struct ExtraDimensions {
    names: Vec<&'static str>,
    slots: HashMap<&'static str, u32>,
}

static EXTRA_DIMENSIONS: LazyLock<RwLock<ExtraDimensions>> = LazyLock::new(RwLock::default);

/// The key of dimension `id`, numbering it when the game meets it for the first time.
pub fn dimension_slot(id: &str) -> u32 {
    if let Some(slot) = VANILLA_DIMENSIONS.iter().position(|name| *name == id) {
        return slot as u32;
    }
    let first = VANILLA_DIMENSIONS.len() as u32;
    if let Some(slot) = EXTRA_DIMENSIONS.read().unwrap().slots.get(id) {
        return first + slot;
    }
    let mut extra = EXTRA_DIMENSIONS.write().unwrap();
    if let Some(slot) = extra.slots.get(id) {
        return first + slot;
    }
    let slot = extra.names.len() as u32;
    let name: &'static str = Box::leak(id.to_owned().into_boxed_str());
    extra.names.push(name);
    extra.slots.insert(name, slot);
    first + slot
}

/// The name of a dimension's key, None for a slot the game never gave out.
pub fn dimension_id(slot: u32) -> Option<&'static str> {
    let slot = slot as usize;
    VANILLA_DIMENSIONS.get(slot).copied().or_else(|| {
        let extra = EXTRA_DIMENSIONS.read().unwrap();
        extra.names.get(slot - VANILLA_DIMENSIONS.len()).copied()
    })
}

/// The resolved effect slots. Effects remain fixed for the process, as does their registry.
#[derive(Default)]
pub struct Effects {
    pub names: Names,
    pub indices: Vec<usize>,
    pub keys: Vec<u32>,
}

/// A failure to resolve an effect declaration against the complete registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectResolutionError {
    /// A requesting mod named an effect absent from the registry.
    UnknownEffect { mod_id: &'static str, id: String },
    /// The resolver returned an index outside the supplied registry.
    InvalidIndex {
        id: String,
        index: usize,
        registry_len: usize,
    },
}

impl std::fmt::Display for EffectResolutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownEffect { mod_id, id } => {
                write!(f, "mod {mod_id} requested unknown effect {id}")
            }
            Self::InvalidIndex {
                id,
                index,
                registry_len,
            } => {
                write!(
                    f,
                    "effect {id} resolved to index {index} outside registry of length {registry_len}"
                )
            }
        }
    }
}

impl std::error::Error for EffectResolutionError {}

/// Resolve requested effects once, after every mod's content has been registered.
/// Refuses missing effects and resolver indices outside the supplied registry.
pub fn resolve_effects<'a>(
    requests: &Names,
    ids: impl IntoIterator<Item = &'a str>,
    mut resolve: impl FnMut(&str) -> Option<usize>,
) -> Result<Effects, Vec<EffectResolutionError>> {
    let all: Vec<_> = ids.into_iter().collect();
    let mut names = requests.clone();
    // Hooks and effect enumeration may expose entries no mod explicitly requested.
    for id in &all {
        names
            .request("horizon", id)
            .expect("registry owns valid ids");
    }
    let indices = names.resolve(&mut resolve, |mod_id, id| {
        EffectResolutionError::UnknownEffect {
            mod_id,
            id: id.to_owned(),
        }
    })?;
    let mut keys = vec![u32::MAX; all.len()];
    for (slot, &index) in indices.iter().enumerate() {
        let Some(key) = keys.get_mut(index) else {
            return Err(vec![EffectResolutionError::InvalidIndex {
                id: names.entries[slot].id.clone(),
                index,
                registry_len: all.len(),
            }]);
        };
        *key = slot as u32;
    }
    Ok(Effects {
        names,
        indices,
        keys,
    })
}

/// Resolve all block entity types after registration and content loading.
pub fn resolve_block_entity_types<'a, T>(
    requests: &Names,
    ids: impl IntoIterator<Item = &'a str>,
    resolve: impl FnMut(&str) -> Option<T>,
) -> Result<Names, Vec<String>> {
    let mut names = requests.clone();
    names.resolve(resolve, |mod_id, id| {
        format!("mod {mod_id} requested unknown block entity type {id}")
    })?;
    for id in ids {
        names
            .request("horizon", id)
            .expect("registry owns valid ids");
    }
    Ok(names)
}

#[cfg(test)]
mod item_tag_tests {
    use super::*;
    use horizon_core::identifier::Identifier;
    #[test]
    fn typed_requests_share_canonical_slots_and_preserve_requesters() {
        let mut names = Names::default();
        let slot = names.request("first", "stick").unwrap();
        let identifier = Identifier::parse("minecraft:stick").unwrap();
        assert_eq!(names.request_identifier("second", &identifier), Ok(slot));
        assert_eq!(names.entries.len(), 1);
        assert_eq!(names.entries[0].mods, ["first", "second"]);
        assert_eq!(names.identifiers.get(&identifier), Some(&slot));
        assert!(names.request("bad", "Not valid").is_err());
        assert_eq!(names.entries.len(), 1);
    }
    #[test]
    fn completion_records_the_input_order_once_per_name() {
        let names = Names::default()
            .complete(
                ["test:zebra", "minecraft:air", "test:zebra", "test:apple"],
                |_, _| (),
            )
            .unwrap();
        let order: Vec<_> = names
            .order
            .iter()
            .map(|&slot| names.id(slot).unwrap())
            .collect();
        // The registry's order, not sorted and not request order; a block name
        // repeated by its states is listed once.
        assert_eq!(order, ["test:zebra", "minecraft:air", "test:apple"]);
        // Completion still numbers slots in its historical sorted order.
        assert_eq!(names.order, [2, 0, 1]);
    }

    #[test]
    fn tag_name_keys_keep_their_identity_and_refuse_unknown_slots() {
        let first = Identifier::parse("test:retained_tag_name").unwrap();
        let slot = tag_slot(&first).unwrap();
        tag_slot(&Identifier::parse("test:unrelated_tag_name").unwrap()).unwrap();
        assert_eq!(tag_slot(&first), Some(slot));
        assert!(with_tag_name(slot, |name| name == Some("test:retained_tag_name")));
        assert!(with_tag_name(u32::MAX, |name| name.is_none()));
    }
}
