use super::*;

/// The bounds for general entity types, declared
/// once so the registrar, the codecs and the tests quote the same numbers.
/// The payload bound applies to mod-owned payload bytes only, never to a
/// record of a vanilla type. The quarantine bounds (with no vanilla
/// analogue) cap what the game preserves of a missing mod's records.
// exhaustive: the design's limits are one block; no further constants join it
pub mod limits {
    /// The most payload bytes one general entity record may carry. A payload
    /// one byte larger is refused, never truncated into a valid-looking
    /// entity.
    pub const PAYLOAD_LIMIT: usize = 65_536;
    /// The deepest nesting of a general entity's decoded data. The
    /// payload decode checks it; the record tree itself has the stricter
    /// passenger-depth bound.
    pub const MAX_NESTING: u32 = 32;
    /// The deepest chain of passengers a general entity tree may carry.
    pub const MAX_PASSENGER_DEPTH: u32 = 16;
    /// The largest record a missing mod's quarantined general entity may
    /// occupy, as its encoded NBT size. A larger record is skipped with a
    /// warning rather than preserved. There is no vanilla analogue:
    /// vanilla deletes unknown-mod entities outright, and Horizon's quarantine
    /// preserves records for missing mods.
    pub const QUARANTINE_RECORD_LIMIT: u64 = 69_632;
    /// The most quarantined general entity records one chunk may carry. The
    /// next one is skipped with a warning. There is no vanilla analogue:
    /// vanilla has no quarantine to bound.
    pub const QUARANTINE_CHUNK_LIMIT: usize = 256;
}

/// A general entity type's spawn category (supplied at
/// declaration; general types start as `MISC`, which never gains natural
/// spawning implicitly).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum GeneralEntityCategory {
    /// Neither monster nor creature: no natural-spawn policy of its own.
    Misc,
}

/// Whether instances of a declared general type persist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum GeneralSavePolicy {
    /// Instances save like ordinary entities do.
    Save,
    /// `noSave`: instances produce zero persisted records.
    NoSave,
}

/// A general entity type a mod declares, as the game reads it when it freezes
/// the type registry. The registry is data: no mod type adds an entry
/// to any hand-kept engine list.
#[derive(Clone, Debug)]
// exhaustive: only registration builds it; the game reads it
#[derive(horizon_mod_api_macros::Wire)]
pub struct GeneralEntityTypeDeclaration {
    /// The declaring mod's id: the first key of the registry's sort order.
    pub mod_id: String,
    /// The type's namespaced id (`mymod:robot`).
    pub id: String,
    /// The type's spawn category.
    pub category: GeneralEntityCategory,
    /// The type's finite width in blocks, as supplied to the vanilla builder.
    pub width: f64,
    /// The type's finite height in blocks, as supplied to the vanilla builder.
    pub height: f64,
    /// Whether instances of the type persist.
    pub save_policy: GeneralSavePolicy,
    /// Whether the type may be summoned by name.
    pub summonable: bool,
    /// Tracking distance in chunks, stored directly like vanilla's signed integer.
    pub tracking_chunks: i32,
    /// Update interval in ticks, stored directly like vanilla's signed integer.
    /// `i32::MAX` is vanilla's no-update-interval value.
    pub update_interval: i32,
    /// The payload schema version instances of the type carry.
    pub payload_version: u16,
    /// The ids of loaded mods this type requires, every one of which must be
    /// present for the declaration to stand.
    pub required_features: Vec<String>,
}

/// A placeholder to build a declaration from: empty ids, a miscellaneous type that saves, vanilla's
/// default player-sized box and no requirements. Registration validates what a mod sets.
impl Default for GeneralEntityTypeDeclaration {
    fn default() -> Self {
        Self {
            mod_id: String::new(),
            id: String::new(),
            category: GeneralEntityCategory::Misc,
            width: 0.6,
            height: 1.8,
            save_policy: GeneralSavePolicy::Save,
            summonable: true,
            tracking_chunks: 5,
            update_interval: 3,
            payload_version: 1,
            required_features: Vec::new(),
        }
    }
}

/// Why a mod's general entity type declaration was refused
/// ([`Registrar::declare_general_entity`](GeneralTypesRegistrar::declare_general_entity)).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum GeneralEntityRegistrationError {
    /// The registry is frozen: declarations are only accepted while the game
    /// is still in its registration phase, before any world or connection
    /// opens.
    Frozen,
    /// Only both-side packages may declare entity types, which the network
    /// names.
    WrongSide,
    /// The type's id is malformed, empty or in a namespace the declaring mod
    /// does not own (a vanilla type's included).
    InvalidId,
    /// Another declaration already covers the type.
    Duplicate,
    /// A dimension is NaN or infinite.
    NotFinite,
    /// A required feature names no loaded mod.
    UnknownDependency,
}

/// The general entity types mods declared.
pub fn general_entity_type_declarations() -> &'static [GeneralEntityTypeDeclaration] {
    installed().map_or(&[], |installed| &installed.general_entity_types)
}

/// Registration of general entity types: one frozen registry of
/// namespaced types, filled before worlds or connections open.
pub trait GeneralTypesRegistrar {
    /// Declare a general entity type of the mod's own (vanilla's
    /// `EntityType` builder, NeoForge's `RegisterEvent`): `path` in the mod's
    /// namespace (`robot` of `mymod` is `mymod:robot`), its category, its
    /// finite `width` and `height` in blocks, its save and summon
    /// policy, its signed integer tracking distance and update interval, its payload
    /// schema version and the loaded mods it requires.
    ///
    /// An id outside the mod's manifest-owned namespaces, a malformed id, a
    /// duplicate of this mod's or another's, a non-finite dimension, a required feature naming no
    /// loaded mod, or a declaration after the registry froze (the
    /// registration phase has ended) refuses, naming the mod, without
    /// panicking. Invalid input is refused, never clamped.
    fn declare_general_entity(
        &mut self,
        path: &str,
        category: GeneralEntityCategory,
        width: f64,
        height: f64,
        save_policy: GeneralSavePolicy,
        summonable: bool,
        tracking_chunks: i32,
        update_interval: i32,
        payload_version: u16,
        required_features: &[&str],
    ) -> Result<(), GeneralEntityRegistrationError>;
}
