use super::*;
use crate::world::datapack_registry::{
    DatapackField, DatapackRegistrationError, DatapackRegistryDeclaration, DatapackRegistrySync,
};

/// The datapack registries mods declared.
pub fn datapack_registry_declarations() -> &'static [DatapackRegistryDeclaration] {
    // A fixture's installed mods own its declarations. Process-wide declarations are
    // only the fallback for connection threads without an inherited fixture snapshot.
    #[cfg(any(test, feature = "testing"))]
    if let Some(installed) = crate::parts::TEST_PARTS.get() {
        return &installed.datapack_registries;
    }
    #[cfg(any(test, feature = "testing"))]
    if let Some(declarations) = PROCESS_DECLARATIONS.get() {
        return declarations;
    }
    installed().map_or(&[], |installed| &installed.datapack_registries)
}

/// Declarations a test process shares with every thread. A test's own installation is per
/// thread, but the connection a server answers a client on runs on threads the test did not
/// start. A thread's scoped installation takes precedence over this fallback.
#[cfg(any(test, feature = "testing"))]
static PROCESS_DECLARATIONS: OnceLock<Vec<DatapackRegistryDeclaration>> = OnceLock::new();

/// Declare `declarations` for every thread of this test process; the first call wins.
#[cfg(any(test, feature = "testing"))]
pub fn install_process_datapack_registry_declarations(
    declarations: Vec<DatapackRegistryDeclaration>,
) {
    let _ = PROCESS_DECLARATIONS.set(declarations);
}

/// Registration of mod-owned datapack registries
/// ([`Registrar::declare_datapack_registry`](DatapackRegistriesRegistrar::declare_datapack_registry)).
pub trait DatapackRegistriesRegistrar {
    /// Declare a datapack registry of the mod's own (NeoForge's
    /// `DataPackRegistryEvent.NewRegistry`): `path` in the mod's namespace
    /// (`spell` of `example` is `example:spell`), the flat record `fields` an
    /// entry has, and how clients receive the entries.
    ///
    /// Entries are JSON files in data packs: entry `addon:fire` of
    /// `example:spell` is `data/addon/example/spell/fire.json`
    /// ([`DatapackRegistryDeclaration::entry_file`]). The game decodes every
    /// file against `fields`: no mod code runs while decoding, so native and
    /// wasm mods get the same bounds. A key the schema does not
    /// declare, a missing required key or an out-of-range value fails the
    /// entry; a default never stands in for invalid data.
    ///
    /// Only both-side packages may declare datapack registries. Refused,
    /// naming the mod: a key outside the mod's namespace (a vanilla
    /// registry's included), a malformed key, a duplicate (of this mod's or
    /// another's, caught where the declarations join), an empty or repeated
    /// field name, unusable bounds, or a default that is not a value of its
    /// field inside its bounds.
    fn declare_datapack_registry(
        &mut self,
        path: &str,
        sync: DatapackRegistrySync,
        fields: Vec<DatapackField>,
    ) -> Result<(), DatapackRegistrationError>;
}
