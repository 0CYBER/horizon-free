//! Load-owned identities in the built-in custom statistic registry.
use super::*;

/// Custom statistics declared during the shared native and wasm registration phase.
pub trait CustomStatsRegistrar {
    /// Declare `path` in the registering mod's namespace, or an explicit
    /// `namespace:path` in a manifest-owned namespace other than `minecraft`.
    /// Invalid, duplicate or frozen declarations refuse installation,
    /// even when the returned error is ignored. IDs freeze before worlds open.
    /// Servers with custom statistics require Horizon clients with the same
    /// ordered registry; the configuration-phase content identity checks it.
    fn register_custom_stat(&mut self, id: &str) -> Result<(), String>;
}

/// Validated custom statistic IDs, in declaration order after vanilla's IDs.
// hidden: engine registry installation; mods declare identities through Registrar.
#[doc(hidden)]
pub fn custom_stat_declarations() -> &'static [String] {
    installed().map_or(&[], |parts| &parts.custom_stats)
}
