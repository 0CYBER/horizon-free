//! Discoverable packs supplied by mods, separately from individual resource files
//! and reload listeners (`AddPackFindersEvent`).
use crate::{Identifier, Text};

/// The largest resource [`ResourcesHost::resource`](crate::prelude::ResourcesHost::resource)
/// returns, in bytes. A larger winning file reads as `None` rather than as a
/// truncated or unbounded value.
pub const MAX_RESOURCE_BYTES: u64 = 64 * 1024 * 1024;

/// Which repository is being discovered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum PackType {
    /// Client assets selected in Resource Packs.
    ClientResources,
    /// Server data selected for a world or with `/datapack`.
    ServerData,
}

/// How a pack is labelled in the selection screen. Feature packs also appear
/// among the world's experiments when their metadata declares feature flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum PackSource {
    /// A pack supplied by a mod, labelled as built in.
    BuiltIn,
    /// An optional experiment described by its `pack.mcmeta` feature flags.
    Feature,
}

/// Where an always-active pack enters a selection that does not already name it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum PackPosition {
    /// Above other packs, giving its resources higher priority.
    Top,
    /// Below other packs, giving its resources lower priority.
    Bottom,
}

/// A pack bundled with a mod. `location` names the mod and a directory beneath
/// its package root, e.g. `example:assets/example/resourcepacks/high_contrast`. That directory must
/// contain `pack.mcmeta`. Packages may be folders or archives; discovery and
/// reload read the same logical directory without extracting it.
///
/// The repository id is `mod/<location>`. Optional packs remain unselected until
/// the player selects them. Always-active packs cannot be disabled; `position`
/// determines their default priority (later resource layers win).
#[derive(Clone, Debug)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct PackRegistration {
    /// Mod id and logical directory beneath that mod's package root.
    pub location: Identifier,
    /// The kind of repository this pack contributes to.
    pub pack_type: PackType,
    /// The pack screen's title; use a translated component for visible text.
    pub title: Text,
    /// The pack's source label and experiment classification.
    pub source: PackSource,
    /// Require this pack in every selection, preventing it from being disabled.
    pub always_active: bool,
    /// The default insertion position of an always-active pack.
    pub position: PackPosition,
    /// Keep the pack out of the pack screens (Resource Packs, Data Packs, Experiments), for an
    /// implementation detail players should not see or toggle. A selected hidden pack still loads;
    /// `/datapack` still lists it.
    pub hidden: bool,
}
impl PackRegistration {
    /// An optional built-in pack, offered at the top of the selection.
    pub fn new(location: Identifier, pack_type: PackType, title: Text) -> Self {
        Self {
            location,
            pack_type,
            title,
            source: PackSource::BuiltIn,
            always_active: false,
            position: PackPosition::Top,
            hidden: false,
        }
    }
}

/// A registration that the repository could not accept. No partial pack is added.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum PackFinderError {
    /// The title failed component conversion; no pack was added.
    Text(crate::TextFormError),
    /// The requested directory is outside the package's logical root.
    InvalidPath,
    /// No loaded package matches the location's namespace, or it cannot be opened.
    MissingPackage,
    /// The directory does not contain readable pack metadata.
    InvalidMetadata,
    /// Another source already supplied the same repository id.
    Duplicate,
}
impl std::fmt::Display for PackFinderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Text(error) => error.fmt(f),
            Self::InvalidPath => f.write_str("the pack location leaves its package"),
            Self::MissingPackage => f.write_str("the pack's mod package is not loaded"),
            Self::InvalidMetadata => f.write_str("the pack metadata could not be read"),
            Self::Duplicate => f.write_str("the repository already contains this pack id"),
        }
    }
}
impl std::error::Error for PackFinderError {}

impl From<crate::TextFormError> for PackFinderError {
    fn from(error: crate::TextFormError) -> Self {
        Self::Text(error)
    }
}

/// The typed builder handed to `add_pack_finders`, rebuilt for each discovery.
/// Registration performs no resource reload and does not select optional packs.
pub trait PackFinders {
    /// The kind of repository currently being discovered.
    fn pack_type(&self) -> PackType;
    /// Add a bundled pack to this repository. A registration for the other pack
    /// type is ignored, allowing one hook implementation to describe both kinds.
    /// Refuses an invalid title with [`PackFinderError::Text`], including components
    /// deeper than 512 nodes; a refusal leaves the repository unchanged.
    fn add_pack_finders(&mut self, pack: PackRegistration) -> Result<(), PackFinderError>;
}
