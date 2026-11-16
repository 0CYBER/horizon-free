//! Client asset reload listeners and their registration context, the client
//! counterpart of `server_reload`, as NeoForge's
//! `AddClientReloadListenersEvent` beside its server event.
#[cfg(test)]
#[path = "client_reload_tests.rs"]
mod tests;

use crate::{
    FileToIdConverter, Identifier, ReloadApply, ReloadResourceError,
    world::reload::{Boundary, ReloadFiles},
    world::reload_order::ReloadOrderError,
};
use std::sync::{Arc, LazyLock};

/// Horizon's vanilla client reload listeners and ordering boundaries, from the
/// 26.3 `Minecraft` resource-manager registration order: language, textures,
/// shaders, sounds, the atlas, fonts, the grass and foliage colour listeners,
/// the model manager, equipment assets, paletted textures, then the entity and
/// block-entity render dispatchers, particles, the game renderer, the level
/// extractor, clouds, the GPU warnlist, regional compliance and the GUI.
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct VanillaClientListeners;
impl VanillaClientListeners {
    /// Boundary before every reload listener.
    pub fn first() -> Identifier {
        static ID: LazyLock<Identifier> =
            LazyLock::new(|| Identifier::parse("minecraft:first").unwrap());
        ID.clone()
    }
    /// Vanilla's model manager. Use this anchor rather than its string value,
    /// which need not match NeoForge's identifier.
    pub fn model_manager() -> Identifier {
        static ID: LazyLock<Identifier> =
            LazyLock::new(|| Identifier::parse("minecraft:model_manager").unwrap());
        ID.clone()
    }
    /// Boundary after every reload listener.
    pub fn last() -> Identifier {
        static ID: LazyLock<Identifier> =
            LazyLock::new(|| Identifier::parse("minecraft:last").unwrap());
        ID.clone()
    }
}

/// Selected client asset files for one converter, with resource-pack overrides
/// resolved. Keys have their directory and extension removed. This cannot read
/// data files or filesystem paths. Its fields are private so the view can grow.
// exhaustive: its fields are private, so mods cannot build or destructure it
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
pub struct ClientReloadContext {
    pub(crate) files: ReloadFiles,
}
impl ClientReloadContext {
    pub(crate) fn new(files: ReloadFiles) -> Self {
        Self { files }
    }
    /// Converted IDs, across all namespaces, in deterministic resource-ID order.
    pub fn list(&self) -> impl Iterator<Item = &Identifier> {
        self.files.list()
    }
    /// The winning bytes for a converted ID, or None outside this directory.
    pub fn read(&self, id: &Identifier) -> Option<&[u8]> {
        self.files.read(id)
    }
}

/// A client-side `SimplePreparableReloadListener`, as `ServerReloadListener`
/// reads data: prepare reads the new pack view without changing live state;
/// its returned apply publishes the result. All prepares finish before any
/// apply, including vanilla's, runs. Prepare cannot see another listener's new
/// published state. Dependencies order both phases deterministically; they do
/// not remove the barrier. On any error all pending applies are dropped,
/// keeping every listener's previous assets.
pub trait ClientReloadListener: Send + Sync + 'static {
    /// Parse and validate new assets without changing the listener's live state.
    ///
    /// As vanilla's `SimpleJsonResourceReloadListener.scanDirectory`, a
    /// listener that parses its files logs and skips one that won't parse,
    /// carrying the rest through; returning `Err` aborts the whole reload and
    /// keeps every listener's previous assets. A panic in `prepare` or the
    /// returned apply is not recovered: it stops the game, and the crash report
    /// names the listener and the mod that registered it.
    fn prepare(
        &self,
        context: &ClientReloadContext,
    ) -> Result<Box<dyn ReloadApply>, ReloadResourceError>;
}
impl ClientReloadListener for Boundary {
    fn prepare(
        &self,
        _: &ClientReloadContext,
    ) -> Result<Box<dyn ReloadApply>, ReloadResourceError> {
        Ok(<dyn ReloadApply>::new(|| {}))
    }
}

/// The client reload's listener registry, as `add_client_reload_listeners` hands it out: its
/// registration methods and a read-only view of names and edges, rebuilt on each client reload.
/// Listener names need not belong to the registering mod.
pub trait ClientReloadListeners {
    /// Register a unique listener and converter, e.g. `FileToIdConverter::json("machines")`.
    /// Keep the `Arc` to read the listener's published state outside reloads.
    fn add_listener(
        &mut self,
        id: Identifier,
        converter: FileToIdConverter,
        listener: Arc<dyn ClientReloadListener>,
    ) -> Result<(), ReloadOrderError>;
    /// Require `first` before `second`; both names must already be registered.
    fn add_dependency(
        &mut self,
        first: Identifier,
        second: Identifier,
    ) -> Result<(), ReloadOrderError>;
    /// The registered names in registration order, boundaries included.
    fn get_registry(&self) -> Vec<Identifier>;
    /// The dependency edges as `(first, second)` names.
    fn get_graph(&self) -> Vec<(Identifier, Identifier)>;
    /// The name of the vanilla anchor preceding mod listeners by default.
    fn get_last_vanilla_listener(&self) -> Identifier;
}

// no wire: process-local listener or callback storage; trait objects are invoked through host dispatch, not serialized.
/// The game's [`ClientReloadListeners`].
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ClientListenerRegistry {
    core: crate::world::reload::ReloadCore<dyn ClientReloadListener>,
}
impl ClientReloadListeners for ClientListenerRegistry {
    fn add_listener(
        &mut self,
        id: Identifier,
        converter: FileToIdConverter,
        listener: Arc<dyn ClientReloadListener>,
    ) -> Result<(), ReloadOrderError> {
        self.core.add_listener(id, converter, listener)
    }
    fn add_dependency(
        &mut self,
        first: Identifier,
        second: Identifier,
    ) -> Result<(), ReloadOrderError> {
        self.core.add_dependency(first, second)
    }
    fn get_registry(&self) -> Vec<Identifier> {
        self.core.registry().map(|(id, _)| id.clone()).collect()
    }
    fn get_graph(&self) -> Vec<(Identifier, Identifier)> {
        self.core
            .graph()
            .map(|(first, second)| (first.clone(), second.clone()))
            .collect()
    }
    fn get_last_vanilla_listener(&self) -> Identifier {
        VanillaClientListeners::model_manager()
    }
}
impl ClientListenerRegistry {
    pub(crate) fn new() -> Self {
        Self {
            core: crate::world::reload::ReloadCore::new(
                VanillaClientListeners::first(),
                VanillaClientListeners::model_manager(),
                VanillaClientListeners::last(),
                || -> Arc<dyn ClientReloadListener> { Arc::new(Boundary) },
            ),
        }
    }
    pub(crate) fn finish(
        self,
    ) -> Result<
        Vec<crate::world::reload::RegisteredListener<dyn ClientReloadListener>>,
        ReloadOrderError,
    > {
        self.core.finish(&VanillaClientListeners::model_manager())
    }
}
