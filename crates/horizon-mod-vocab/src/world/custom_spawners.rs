//! A level's custom spawners (`CustomSpawner`): the list `ModifyCustomSpawnersEvent` edits.
//!
//! NeoForge's list holds `CustomSpawner` objects. A guest-implemented object cannot cross to a
//! wasm mod, so a mod's spawner is a [`SpawnerKey`] it owns, and the game asks the mods to tick
//! it through the `custom_spawner_tick` hook, once per tick, in list order.
use std::{fmt, sync::Arc};

/// A spawner vanilla runs for every level (`MinecraftServer`'s custom spawners).
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SpawnerKind {
    /// `PhantomSpawner`: phantoms for players who have not slept.
    Phantom,
    /// `PatrolSpawner`: pillager patrols.
    Patrol,
    /// `CatSpawner`: cats in villages and swamp huts.
    Cat,
    /// `VillageSiege`: zombie sieges on a village at night.
    VillageSiege,
    /// `WanderingTraderSpawner`: the wandering trader and its llamas.
    WanderingTrader,
}

/// The identity of a mod's spawner, `namespace:path` (`mymod:rat_swarm`), checked once where a
/// mod names it: what [`CustomSpawners::add`] takes and `custom_spawner_tick` hands back.
// no wire: decoding calls SpawnerKey::new to enforce a valid namespaced spawner identifier.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct SpawnerKey(Arc<str>);

impl SpawnerKey {
    /// `id` as a spawner key, or None where it isn't a namespaced identifier.
    pub fn new(id: &str) -> Option<Self> {
        horizon_core::identifier::Identifier::parse_namespaced(id)?;
        Some(Self(id.into()))
    }
    /// The id, as given (`mymod:rat_swarm`).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&SpawnerKey> for String {
    fn from(key: &SpawnerKey) -> Self {
        key.0.to_string()
    }
}

impl TryFrom<String> for SpawnerKey {
    type Error = SpawnerError;
    fn try_from(id: String) -> Result<Self, SpawnerError> {
        Self::new(&id).ok_or(SpawnerError::InvalidKey(id))
    }
}

/// One entry of a level's custom spawner list.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SpawnerEntry {
    /// One of vanilla's spawners.
    Builtin(SpawnerKind),
    /// A mod's spawner, ticked through `custom_spawner_tick`.
    Mod(SpawnerKey),
}

/// A list of spawners could not be made.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum SpawnerError {
    /// The key is not a namespaced identifier.
    InvalidKey(String),
    /// The entry is in the list twice.
    Duplicate(SpawnerEntry),
}

impl fmt::Display for SpawnerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidKey(id) => write!(f, "`{id}` is not a namespaced spawner key"),
            Self::Duplicate(entry) => write!(f, "{entry:?} is in the spawner list twice"),
        }
    }
}

impl std::error::Error for SpawnerError {}

/// A level's custom spawners, in the order they tick: what `ModifyCustomSpawnersEvent` edits.
/// It starts with vanilla's, and a spawner a mod adds goes at the end.
// no wire: decoding TryFrom<Vec<SpawnerEntry>> rejects duplicate builtin or mod spawner entries.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct CustomSpawners(Vec<SpawnerEntry>);

impl CustomSpawners {
    /// The list a level starts with: vanilla's spawners in the order the game ticks them.
    // hidden: the game builds each level's list, mods only edit it
    #[doc(hidden)]
    pub fn vanilla() -> Self {
        use SpawnerKind::*;
        Self(
            [WanderingTrader, Patrol, Cat, VillageSiege, Phantom]
                .map(SpawnerEntry::Builtin)
                .into(),
        )
    }
    /// Adds a mod's spawner at the end; false where the list already has it.
    pub fn add(&mut self, key: SpawnerKey) -> bool {
        let entry = SpawnerEntry::Mod(key);
        let new = !self.0.contains(&entry);
        if new {
            self.0.push(entry);
        }
        new
    }
    /// Takes `entry` out, so it never ticks; false where it was not in the list.
    pub fn remove(&mut self, entry: &SpawnerEntry) -> bool {
        let before = self.0.len();
        self.0.retain(|e| e != entry);
        self.0.len() != before
    }
    /// Whether `entry` is in the list.
    pub fn contains(&self, entry: &SpawnerEntry) -> bool {
        self.0.contains(entry)
    }
    /// The entries, in tick order.
    pub fn entries(&self) -> &[SpawnerEntry] {
        &self.0
    }
}

impl From<&CustomSpawners> for Vec<SpawnerEntry> {
    fn from(list: &CustomSpawners) -> Self {
        list.0.clone()
    }
}

impl TryFrom<Vec<SpawnerEntry>> for CustomSpawners {
    type Error = SpawnerError;
    fn try_from(entries: Vec<SpawnerEntry>) -> Result<Self, SpawnerError> {
        match entries
            .iter()
            .enumerate()
            .find(|(i, e)| entries[..*i].contains(e))
        {
            Some((_, e)) => Err(SpawnerError::Duplicate(e.clone())),
            None => Ok(Self(entries)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(id: &str) -> SpawnerKey {
        SpawnerKey::new(id).unwrap()
    }

    #[test]
    fn a_key_is_a_namespaced_id() {
        assert!(SpawnerKey::new("mymod:rat_swarm").is_some());
        assert_eq!(SpawnerKey::new("My Mod:rats"), None);
        assert_eq!(
            SpawnerKey::try_from("no spaces".to_owned()),
            Err(SpawnerError::InvalidKey("no spaces".to_owned()))
        );
    }

    #[test]
    fn mod_spawners_go_last_in_the_order_they_were_added() {
        let mut list = CustomSpawners::vanilla();
        assert!(list.add(key("a:one")));
        assert!(list.add(key("b:two")));
        assert!(!list.add(key("a:one")), "a spawner is added once");
        let tail: Vec<_> = list.entries().iter().rev().take(2).rev().collect();
        assert_eq!(
            tail,
            [
                &SpawnerEntry::Mod(key("a:one")),
                &SpawnerEntry::Mod(key("b:two"))
            ]
        );
    }

    #[test]
    fn removing_takes_an_entry_out_once() {
        let mut list = CustomSpawners::vanilla();
        let cats = SpawnerEntry::Builtin(SpawnerKind::Cat);
        assert!(list.remove(&cats));
        assert!(!list.contains(&cats));
        assert!(!list.remove(&cats));
    }

    #[test]
    fn a_list_with_an_entry_twice_is_refused() {
        let cats = SpawnerEntry::Builtin(SpawnerKind::Cat);
        assert_eq!(
            CustomSpawners::try_from(vec![cats.clone(), cats.clone()]),
            Err(SpawnerError::Duplicate(cats))
        );
    }
}
