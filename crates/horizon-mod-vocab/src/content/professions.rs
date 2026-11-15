//! Declarative, load-owned villager professions.
use crate::Identifier;

/// A profession's persistent resource identity.
// exhaustive: a key newtype mods construct directly.
#[derive(Clone, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
pub struct ProfessionKey(pub Identifier);
/// A point-of-interest resource identity.
// exhaustive: a key newtype mods construct directly.
#[derive(Clone, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
pub struct PoiKey(pub Identifier);
/// A trade-set resource identity, independent of the profession's name.
// exhaustive: a key newtype mods construct directly.
#[derive(Clone, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
pub struct TradeSetKey(pub Identifier);

/// Declarative profession metadata, collected once for the owning mod load.
/// References resolve together before any candidate table becomes active.
// exhaustive: a mod declares its profession as a struct literal.
#[derive(Clone, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
pub struct ProfessionDefinition {
    /// Persistent qualified identity; the minecraft namespace is reserved.
    pub key: ProfessionKey,
    /// Custom POI block-state selectors owned by this profession.
    pub workstations: Vec<ProfessionWorkstation>,
    /// POIs at which an existing job remains valid.
    pub held_poi: Vec<PoiKey>,
    /// POIs from which an unemployed villager may acquire this job.
    pub acquirable_poi: Vec<PoiKey>,
    /// Items requested by this profession; this grants no farmer AI.
    pub requested_items: Vec<Identifier>,
    /// Secondary work-site blocks.
    pub secondary_blocks: Vec<Identifier>,
    /// Optional work sound.
    pub work_sound: Option<Identifier>,
    /// Explicit level (1–5) to trade-set mappings; omitted levels add no offers.
    pub trades: Vec<(u8, TradeSetKey)>,
}

/// A candidate registry refusal. A refusal publishes no candidate entries.
#[derive(Clone, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum ProfessionRegistrationError {
    /// A declared custom workstation has no acquisition profession.
    UnacquirablePoi {
        /// The custom workstation identity.
        key: PoiKey,
    },
    /// A workstation selector is invalid or conflicts with another POI.
    InvalidWorkstation {
        /// The POI being declared.
        key: PoiKey,
        /// The exact resolution failure.
        reason: String,
    },
    /// Multiple owners declared one identity; owners are sorted deterministically.
    DuplicateProfession {
        /// The conflicting profession identity.
        key: ProfessionKey,
        /// Every declaring mod, in sorted order.
        owners: Vec<String>,
    },
    /// Vanilla profession identities cannot be replaced.
    ReservedProfession {
        /// The referenced resource identity.
        key: ProfessionKey,
    },
    /// Acquisition must have one unambiguous profession per custom POI.
    OverlappingPoi {
        /// The shared acquisition POI.
        poi: PoiKey,
        /// Conflicting profession identities.
        professions: Vec<ProfessionKey>,
    },
    /// The candidate refers to a missing POI.
    UnknownPoi {
        /// The referenced resource identity.
        key: PoiKey,
    },
    /// The candidate refers to a missing trade set.
    UnknownTradeSet {
        /// The referenced resource identity.
        key: TradeSetKey,
    },
    /// The candidate refers to a missing item.
    UnknownItem {
        /// The referenced resource identity.
        key: Identifier,
    },
    /// The candidate refers to a missing secondary block.
    UnknownBlock {
        /// The referenced resource identity.
        key: Identifier,
    },
    /// The candidate refers to a missing work sound.
    UnknownSound {
        /// The referenced resource identity.
        key: Identifier,
    },
    /// A trade level is outside 1–5 or appears more than once.
    InvalidTradeLevel {
        /// The declaring profession.
        key: ProfessionKey,
        /// The invalid or repeated level.
        level: u8,
    },
}

/// Load-scoped profession registrar. Registration records the calling mod as owner.
pub trait VillagerProfessions {
    /// Declare one profession; all references are checked before publication.
    fn register(&mut self, definition: ProfessionDefinition);
}

// no wire: engine-owned registrar backend, borrowed only during registration.
/// Engine storage for declarations and their dispatch owners.
// exhaustive: its fields are private; only the engine constructs it.
#[derive(Default)]
pub struct ProfessionDeclarations {
    entries: Vec<(String, ProfessionDefinition)>,
}
impl ProfessionDeclarations {
    /// Consume the collected declarations, retaining the engine-recorded owner.
    // hidden: only the game consumes the collected declarations.
    #[doc(hidden)]
    pub fn into_entries(self) -> Vec<(String, ProfessionDefinition)> {
        self.entries
    }
}
impl VillagerProfessions for ProfessionDeclarations {
    fn register(&mut self, definition: ProfessionDefinition) {
        let owner = crate::content::crash::current_native_mod()
            .expect("profession registration inside mod dispatch");
        self.entries.push((owner.into(), definition));
    }
}

/// A custom workstation selector; unspecified block properties match every value.
// exhaustive: a mod declares its workstation as a struct literal.
#[derive(Clone, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
pub struct ProfessionWorkstation {
    /// Custom POI identity, outside the minecraft namespace.
    pub poi: PoiKey,
    /// Block resource identity.
    pub block: Identifier,
    /// Required state property values.
    pub properties: Vec<(String, String)>,
}

impl std::fmt::Display for ProfessionRegistrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnacquirablePoi { key } => {
                write!(f, "UnacquirablePoi: {} has no acquiring profession", key.0)
            }
            Self::InvalidWorkstation { key, reason } => {
                write!(f, "InvalidWorkstation: {}: {reason}", key.0)
            }
            Self::DuplicateProfession { key, owners } => write!(
                f,
                "DuplicateProfession: {} declared by {}",
                key.0,
                owners.join(", ")
            ),
            Self::ReservedProfession { key } => {
                write!(f, "ReservedProfession: {} is reserved", key.0)
            }
            Self::OverlappingPoi { poi, professions } => write!(
                f,
                "OverlappingPoi: {} acquired by {}",
                poi.0,
                professions
                    .iter()
                    .map(|p| p.0.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::UnknownPoi { key } => write!(f, "UnknownPoi: {}", key.0),
            Self::UnknownTradeSet { key } => write!(f, "UnknownTradeSet: {}", key.0),
            Self::UnknownItem { key } => write!(f, "UnknownItem: {key}"),
            Self::UnknownBlock { key } => write!(f, "UnknownBlock: {key}"),
            Self::UnknownSound { key } => write!(f, "UnknownSound: {key}"),
            Self::InvalidTradeLevel { key, level } => write!(
                f,
                "InvalidTradeLevel: {} level {level} is repeated or outside 1–5",
                key.0
            ),
        }
    }
}
