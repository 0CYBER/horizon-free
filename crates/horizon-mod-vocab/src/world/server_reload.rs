//! Server data reload listeners and their registration context.
#[cfg(test)]
#[path = "server_reload_tests.rs"]
mod tests;

use crate::{
    FileToIdConverter, Identifier, ReloadApply, ReloadResourceError,
    world::reload::{Boundary, ReloadFiles},
    world::reload_order::ReloadOrderError,
};
use std::{
    any::Any,
    collections::BTreeMap,
    marker::PhantomData,
    sync::{Arc, LazyLock},
};

/// Horizon's vanilla server reload listeners and ordering boundaries.
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct VanillaServerListeners;
impl VanillaServerListeners {
    /// Boundary before every reload listener.
    pub fn first() -> Identifier {
        static ID: LazyLock<Identifier> =
            LazyLock::new(|| Identifier::parse("minecraft:first").unwrap());
        ID.clone()
    }
    /// Vanilla's function library. Use this anchor rather than its string value,
    /// which need not match NeoForge's identifier.
    pub fn functions() -> Identifier {
        static ID: LazyLock<Identifier> =
            LazyLock::new(|| Identifier::parse("minecraft:function_library").unwrap());
        ID.clone()
    }
    /// Boundary after every reload listener.
    pub fn last() -> Identifier {
        static ID: LazyLock<Identifier> =
            LazyLock::new(|| Identifier::parse("minecraft:last").unwrap());
        ID.clone()
    }
}

/// Selected server data files for one converter, with pack overrides resolved.
/// Keys have their directory and extension removed. This cannot read assets or
/// filesystem paths. Its fields are private so the view can grow.
// exhaustive: its fields are private, so mods cannot build or destructure it
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
pub struct ServerReloadContext {
    pub(crate) files: ReloadFiles,
}
impl ServerReloadContext {
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

/// A server-side `SimplePreparableReloadListener`: prepare reads the new pack
/// view without changing live state; its returned apply publishes the result.
/// All prepares finish before any apply, including vanilla's, runs. Prepare
/// cannot see another listener's new published state. Dependencies order both
/// phases deterministically; they do not remove the barrier. On any error all
/// pending applies are dropped, keeping the previous data and function library.
pub trait ServerReloadListener: Send + Sync + 'static {
    /// Parse and validate new data without changing the listener's live state.
    ///
    /// As vanilla's `SimpleJsonResourceReloadListener.scanDirectory`, a
    /// listener that parses its files logs and skips one that won't parse,
    /// carrying the rest through; returning `Err` aborts the whole reload and
    /// keeps every listener's previous data. A panic in `prepare` or the
    /// returned apply is not recovered: it stops the server, and the crash
    /// report names the listener and the mod that registered it.
    fn prepare(
        &self,
        context: &ServerReloadContext,
    ) -> Result<Box<dyn ReloadApply>, ReloadResourceError>;
}
impl ServerReloadListener for Boundary {
    fn prepare(
        &self,
        _: &ServerReloadContext,
    ) -> Result<Box<dyn ReloadApply>, ReloadResourceError> {
        Ok(<dyn ReloadApply>::new(|| {}))
    }
}

/// The server reload's listener registry, as `add_server_reload_listeners` hands it out: its
/// registration methods and a read-only view of names and edges, rebuilt on each server reload.
/// Listener names need not belong to the registering mod.
pub trait ServerReloadListeners {
    /// Register a unique listener and converter, e.g. `FileToIdConverter::json("machines")`.
    /// Keep the `Arc` to read the listener's published state outside reloads.
    fn add_listener(
        &mut self,
        id: Identifier,
        converter: FileToIdConverter,
        listener: Arc<dyn ServerReloadListener>,
    ) -> Result<(), ReloadOrderError>;
    /// [`add_listener`](Self::add_listener), keeping `retained` (the listener itself, as the
    /// mod holds it) in the reload resources; mods reach it through `add_retained_listener`.
    fn add_retained(
        &mut self,
        id: Identifier,
        converter: FileToIdConverter,
        listener: Arc<dyn ServerReloadListener>,
        retained: Arc<dyn Any + Send + Sync>,
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

impl dyn ServerReloadListeners + '_ {
    /// Register a listener whose object is also kept in the reload resources, as
    /// NeoForge's `addRetainedListener(key, listener)`: the caller builds the
    /// typed `ListenerKey` (`ListenerKey::new(id)`), and reads the object back
    /// through `ServerReloadResources` after the reload, from any server hook.
    pub fn add_retained_listener<T: ServerReloadListener>(
        &mut self,
        key: &ListenerKey<T>,
        converter: FileToIdConverter,
        listener: Arc<T>,
    ) -> Result<(), ReloadOrderError> {
        self.add_retained(key.id().clone(), converter, listener.clone(), listener)
    }
}

// no wire: process-local listener or callback storage; trait objects are invoked through host dispatch, not serialized.
/// The game's [`ServerReloadListeners`].
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ServerListenerRegistry {
    core: crate::world::reload::ReloadCore<dyn ServerReloadListener>,
    retained: Vec<RetainedEntry>,
}
impl ServerReloadListeners for ServerListenerRegistry {
    fn add_listener(
        &mut self,
        id: Identifier,
        converter: FileToIdConverter,
        listener: Arc<dyn ServerReloadListener>,
    ) -> Result<(), ReloadOrderError> {
        self.core.add_listener(id, converter, listener)
    }
    fn add_retained(
        &mut self,
        id: Identifier,
        converter: FileToIdConverter,
        listener: Arc<dyn ServerReloadListener>,
        retained: Arc<dyn Any + Send + Sync>,
    ) -> Result<(), ReloadOrderError> {
        self.core.add_listener(id.clone(), converter, listener)?;
        self.retained.push(RetainedEntry { id, any: retained });
        Ok(())
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
        VanillaServerListeners::functions()
    }
}
impl ServerListenerRegistry {
    pub(crate) fn new() -> Self {
        Self {
            core: crate::world::reload::ReloadCore::new(
                VanillaServerListeners::first(),
                VanillaServerListeners::functions(),
                VanillaServerListeners::last(),
                || -> Arc<dyn ServerReloadListener> { Arc::new(Boundary) },
            ),
            retained: Vec::new(),
        }
    }
    pub(crate) fn finish(self) -> Result<OrderedServerReload, ReloadOrderError> {
        Ok(OrderedServerReload {
            listeners: self.core.finish(&VanillaServerListeners::functions())?,
            retained: self.retained,
        })
    }
}

// no wire: process-local listener or callback storage; trait objects are invoked through host dispatch, not serialized.
/// The ordered listeners and retained entries of one completed registration.
#[non_exhaustive]
pub struct OrderedServerReload {
    /// The listeners in dependency order.
    pub listeners: Vec<crate::world::reload::RegisteredListener<dyn ServerReloadListener>>,
    /// The entries the reload keeps.
    pub retained: Vec<RetainedEntry>,
}

// no wire: process-local listener or callback storage; trait objects are invoked through host dispatch, not serialized.
/// One listener retained in the reload resources, as NeoForge's
/// `ReloadableServerResources` keeps a manager.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct RetainedEntry {
    pub(crate) id: Identifier,
    pub(crate) any: Arc<dyn Any + Send + Sync>,
}

// no wire: typed lookup key for process-local retained trait objects; phantom type identity is not transferable.
/// A typed key to a retained reload listener, built by the registering mod
/// and read back through `ReloadResources`. `ListenerKey::new` accepts any
/// id; a key whose id does not match a registration, or whose type differs
/// from the registered object, reads back as `None`.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ListenerKey<T: ?Sized> {
    id: Identifier,
    marker: PhantomData<fn() -> T>,
}
impl<T: ?Sized> ListenerKey<T> {
    /// Names a listener key.
    pub fn new(id: Identifier) -> Self {
        Self {
            id,
            marker: PhantomData,
        }
    }
    /// The registered listener's name.
    pub fn id(&self) -> &Identifier {
        &self.id
    }
}
impl<T: ?Sized> Clone for ListenerKey<T> {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            marker: PhantomData,
        }
    }
}
impl<T: ?Sized> std::fmt::Debug for ListenerKey<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListenerKey").field("id", &self.id).finish()
    }
}
impl<T: ?Sized> PartialEq for ListenerKey<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl<T: ?Sized> Eq for ListenerKey<T> {}

/// The retained server reload listeners of the last successful reload, as
/// NeoForge's `ReloadableServerResources`, reached from
/// `ServerHost::reload_resources`. A new reload replaces the whole store.
pub trait ServerReloadResources {
    /// The object retained under `id`, as its registering mod handed it over; mods read it
    /// through `listener`.
    fn retained(&self, id: &Identifier) -> Option<Arc<dyn Any + Send + Sync>>;
}
impl dyn ServerReloadResources + '_ {
    /// The retained listener registered under `key` in the last successful
    /// reload; `None` when nothing is registered under it, its type differs, or
    /// no reload has succeeded yet. The downcast itself checks the type.
    pub fn listener<T: ServerReloadListener>(&self, key: &ListenerKey<T>) -> Option<Arc<T>> {
        self.retained(key.id())?.downcast::<T>().ok()
    }
}

// no wire: process-local listener or callback storage; trait objects are invoked through host dispatch, not serialized.
/// The game's [`ServerReloadResources`].
#[derive(Default)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct RetainedListeners {
    entries: BTreeMap<Identifier, Arc<dyn Any + Send + Sync>>,
}
impl ServerReloadResources for RetainedListeners {
    fn retained(&self, id: &Identifier) -> Option<Arc<dyn Any + Send + Sync>> {
        self.entries.get(id).cloned()
    }
}
impl RetainedListeners {
    pub(crate) fn install(&mut self, retained: Vec<RetainedEntry>) {
        self.entries = retained
            .into_iter()
            .map(|entry| (entry.id, entry.any))
            .collect();
    }
}
