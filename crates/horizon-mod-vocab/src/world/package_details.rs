//! What a package's manifest says about it beyond its identity, read by other mods.
use std::collections::BTreeMap;

/// How a code package runs.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PackageRuntime {
    /// Native code, built against the game build it was made for and loaded beside it, not
    /// compiled into the game.
    Native,
    /// A `mod.wasm` run by the wasm runtime.
    Wasm,
}

/// Dispatch ordering requested by a manifest dependency.
///
/// This changes listener/content ordering only. `None` never removes a code
/// library's Cargo or wasm link requirements.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum DependencyOrder {
    /// The dependency dispatches before its owner (the default).
    #[default]
    After,
    /// The owner dispatches before the dependency.
    Before,
    /// Do not impose a dispatch edge.
    None,
}

/// The meaning of one manifest declaration about another package.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DependencyKind {
    /// A required dependency; code packages also require a physical API library.
    Required,
    /// An optional dependency, checked when present.
    Optional,
    /// A matching installed package refuses loading.
    Incompatible,
    /// A matching installed package produces a warning.
    Discouraged,
    /// An ordering-only `load_before` declaration, never a library requirement.
    LoadBefore,
}

/// One dependency, incompatibility, discouragement or ordering declaration.
///
/// This is the manifest as written, not a side-filtered or override-rewritten
/// resolution. Inspect `side` against the installation side when needed.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PackageDependency {
    pub(crate) id: String,
    pub(crate) version: String,
    pub(crate) kind: DependencyKind,
    pub(crate) order: DependencyOrder,
    pub(crate) side: crate::Side,
    pub(crate) reason: Option<String>,
    pub(crate) url: Option<String>,
}

impl PackageDependency {
    /// A complete readonly declaration. `LoadBefore` uses `*`, `Before`, and
    /// `Both`; its version is a marker, not a version constraint.
    pub fn new(
        id: String,
        version: String,
        kind: DependencyKind,
        order: DependencyOrder,
        side: crate::Side,
        reason: Option<String>,
        url: Option<String>,
    ) -> Self {
        Self {
            id,
            version,
            kind,
            order,
            side,
            reason,
            url,
        }
    }
    /// The other package's ID.
    pub fn id(&self) -> &str {
        &self.id
    }
    /// The semver requirement, such as `^1`; `LoadBefore` uses the marker `*`.
    pub fn version(&self) -> &str {
        &self.version
    }
    /// Whether this is required, optional, incompatible, discouraged or ordering-only.
    pub fn kind(&self) -> DependencyKind {
        self.kind
    }
    /// The requested dispatch order, independent of physical linking.
    pub fn order(&self) -> DependencyOrder {
        self.order
    }
    /// Applicable installation side; an integrated server is a client installation.
    pub fn side(&self) -> crate::Side {
        self.side
    }
    /// The author's explanation, if provided.
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }
    /// The author's HTTPS information URL, if provided.
    pub fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }
}

/// One value of a manifest's `[properties]` table.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PropertyValue {
    /// A TOML string.
    String(String),
    /// A TOML integer.
    Integer(i64),
    /// A TOML float.
    Float(f64),
    /// A TOML boolean.
    Boolean(bool),
}

/// The descriptive part of a package's manifest. Missing fields are empty or `None`.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct PackageDetails {
    /// The display name.
    pub name: String,
    /// The description.
    pub description: String,
    /// The authors, in manifest order.
    pub authors: Vec<String>,
    /// The license name, such as `MIT`.
    pub license: String,
    /// The home page link.
    pub homepage: Option<String>,
    /// Where to report problems.
    pub issues: Option<String>,
    /// The icon's path inside the package.
    pub icon_path: Option<String>,
    /// How its code runs; `None` for a content package.
    pub runtime: Option<PackageRuntime>,
    /// The manifest's `[properties]` table.
    pub properties: BTreeMap<String, PropertyValue>,
}
