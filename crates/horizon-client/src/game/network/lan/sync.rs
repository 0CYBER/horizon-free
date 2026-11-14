//! Cached server data and the preload/reload boundaries that own its resolved registries.
use super::Game;
use horizon_client::{
    crafting::ResolvedSources,
    lan::data::{FrozenDamage, SyncData},
};
use horizon_world::item_defaults::ItemData;
use std::sync::{Arc, Mutex};

/// The recipes one server load resolved (the `ingredient` hook asked once per
/// occurrence), shared by every table and the network copy that reads them.
struct ServerSources {
    root: std::path::PathBuf,
    generation: u64,
    overlay: u64,
    sources: Arc<ResolvedSources>,
}

/// One slot, held while it is built: the builder of a generation and pack set
/// is the only one that asks the hook, and the others wait for its answer.
static SOURCES: Mutex<Option<ServerSources>> = Mutex::new(None);

/// Every byte of the packs' files, what a pack set is keyed by.
pub(super) fn overlay_key(overlay: &horizon_world::loot::PackOverlay) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    overlay.files.hash(&mut hasher);
    overlay.conditions.hash(&mut hasher);
    hasher.finish()
}

/// This server's recipes for `overlay`, resolved once per generation and pack
/// set: the first caller builds them and later ones adopt them (`/reload`
/// forgets them first, so it asks again).
pub(in crate::game) fn server_sources(
    root: &std::path::Path,
    item_data: &ItemData,
    overlay: &horizon_world::loot::PackOverlay,
) -> std::io::Result<Arc<ResolvedSources>> {
    let generation = item_data.defaults().generation();
    let key = overlay_key(overlay);
    let mut slot = SOURCES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(held) = slot
        .as_ref()
        .filter(|held| held.generation == generation && held.overlay == key && held.root == root)
    {
        return Ok(held.sources.clone());
    }
    let sources = Arc::new(
        ResolvedSources::load(root, overlay, &server_data_load(root, item_data)?)
            .map_err(|error| std::io::Error::other(error.to_string()))?,
    );
    *slot = Some(ServerSources {
        root: root.to_owned(),
        generation,
        overlay: key,
        sources: sources.clone(),
    });
    Ok(sources)
}

/// Build server recipes privately so a refused reload leaves the current cache intact.
pub(in crate::game) fn prepare_server_sources(
    root: &std::path::Path,
    item_data: &ItemData,
    overlay: &horizon_world::loot::PackOverlay,
) -> std::io::Result<Arc<ResolvedSources>> {
    Ok(Arc::new(
        ResolvedSources::load(root, overlay, &server_data_load(root, item_data)?)
            .map_err(|error| std::io::Error::other(error.to_string()))?,
    ))
}

pub(in crate::game) fn install_server_sources(
    root: &std::path::Path,
    item_data: &ItemData,
    overlay: &horizon_world::loot::PackOverlay,
    sources: Arc<ResolvedSources>,
) {
    *SOURCES.lock().unwrap_or_else(|e| e.into_inner()) = Some(ServerSources {
        root: root.to_owned(),
        generation: item_data.defaults().generation(),
        overlay: overlay_key(overlay),
        sources,
    });
}

/// The recipes of a load of `origin`: a server's are shared, a client's own.
pub(in crate::game) fn load_sources(
    root: &std::path::Path,
    item_data: &ItemData,
    origin: horizon_world::native_load_context::DataLoadOrigin,
    overlay: &horizon_world::loot::PackOverlay,
) -> std::io::Result<Arc<ResolvedSources>> {
    use horizon_world::native_load_context::DataLoadOrigin;
    match origin {
        DataLoadOrigin::ServerDataLoad => server_sources(root, item_data, overlay),
        DataLoadOrigin::ClientPreload | _ => Ok(Arc::new(
            ResolvedSources::load(root, overlay, &data_load_context(root, item_data, origin)?)
                .map_err(|error| std::io::Error::other(error.to_string()))?,
        )),
    }
}

/// Let go of the resolved recipes: a `/reload`, a world closed or a preload no
/// world used. Not a world opening, which adopts what its preload resolved.
pub(in crate::game) fn forget_server_sources() {
    *SOURCES.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// What `sync_data` loaded from this resource generation.
pub(super) struct Synced {
    pub(super) root: std::path::PathBuf,
    pub(super) with_packs: bool,
    pub(super) generation: u64,
    pub(super) result: std::result::Result<Arc<SyncData>, String>,
}

/// Only damage is bound eagerly for gameplay. Full wire data remains lazy
/// except when the entering world actually defines damage types.
#[derive(Default)]
pub(super) struct SyncCache {
    pub(super) data: Option<Synced>,
    pub(super) damage: Option<FrozenDamage>,
    pub(super) world_packs: bool,
}

static SYNC: Mutex<SyncCache> = Mutex::new(SyncCache {
    data: None,
    damage: None,
    world_packs: false,
});

/// Load wire data once, retaining this session's frozen damage definitions.
pub(in crate::game) fn sync_data(
    root: &std::path::Path,
    item_data: &ItemData,
    overlay: &horizon_world::loot::PackOverlay,
) -> std::result::Result<Arc<SyncData>, String> {
    let mut sync = SYNC.lock().unwrap_or_else(|e| e.into_inner());
    cached_sync_data(&mut sync, root, overlay, item_data)
}

pub(super) fn cached_sync_data(
    sync: &mut SyncCache,
    root: &std::path::Path,
    overlay: &horizon_world::loot::PackOverlay,
    item_data: &ItemData,
) -> std::result::Result<Arc<SyncData>, String> {
    let packs = !overlay.files.is_empty();
    let generation = item_data.defaults().generation();
    if sync
        .data
        .as_ref()
        .is_some_and(|cached| cached.generation != generation || cached.root != root)
    {
        sync.data = None;
    }
    let load = || {
        let sources =
            server_sources(root, item_data, overlay).map_err(|error| error.to_string())?;
        match &sync.damage {
            Some(damage) => SyncData::load_with_frozen_damage(
                root,
                overlay,
                damage,
                item_data.defaults(),
                &sources,
            ),
            None => SyncData::load(root, overlay, item_data.defaults(), &sources),
        }
        .map(Arc::new)
    };
    // A late vanilla preload must not replace a packed world's lazy snapshot.
    if sync.data.is_none() && sync.world_packs && !packs {
        return load();
    }
    sync.data
        .get_or_insert_with(|| Synced {
            root: root.to_owned(),
            with_packs: packs,
            generation,
            result: load(),
        })
        .result
        .clone()
}

pub(in crate::game) fn open_world_sync_data(
    root: &std::path::Path,
    item_data: &ItemData,
    overlay: &horizon_world::loot::PackOverlay,
    damage: &horizon_world::damage_source::Catalog,
) -> std::result::Result<(), String> {
    let mut sync = SYNC.lock().unwrap_or_else(|e| e.into_inner());
    initialize_world_sync_data(&mut sync, root, overlay, damage, item_data)
}

pub(super) fn initialize_world_sync_data(
    sync: &mut SyncCache,
    root: &std::path::Path,
    overlay: &horizon_world::loot::PackOverlay,
    damage: &horizon_world::damage_source::Catalog,
    item_data: &ItemData,
) -> std::result::Result<(), String> {
    sync.world_packs = !overlay.files.is_empty();
    if sync.world_packs
        || sync.data.as_ref().is_some_and(|cached| {
            cached.generation != item_data.defaults().generation() || cached.root != root
        })
    {
        sync.data = None;
    }
    sync.damage = Some(FrozenDamage::new(damage, overlay));
    if overlay.kind("damage_type").next().is_some() {
        cached_sync_data(sync, root, overlay, item_data)?;
    }
    Ok(())
}

/// Let go of what `sync_data` loaded: a preload no world used
/// (game/world/world_loading.rs), or the previous session when another world opens.
pub(in crate::game) fn forget_sync_data() {
    *SYNC.lock().unwrap_or_else(|e| e.into_inner()) = SyncCache::default();
}

/// Without a hub yet, retain this world's frozen registries through `/reload`
/// too. World entry discards this snapshot before loading another world's.
pub(in crate::game) fn cached_reload_sync() -> Option<Arc<SyncData>> {
    SYNC.lock()
        .unwrap_or_else(|e| e.into_inner())
        .data
        .as_ref()
        .and_then(|cached| cached.result.as_ref().ok())
        .cloned()
}

/// Keep lazy-cache admission aligned with the current packs even when no
/// complete wire snapshot has been loaded yet. Damage definitions stay frozen.
#[cfg(test)]
pub(super) fn prepare_sync_reload(
    sync: &mut SyncCache,
    overlay: &horizon_world::loot::PackOverlay,
) -> Option<Arc<SyncData>> {
    sync.world_packs = !overlay.files.is_empty();
    sync.data
        .as_ref()
        .and_then(|cached| cached.result.as_ref().ok())
        .cloned()
}

/// `/reload` (MinecraftServer.reloadResources) under a running hub: the
/// tags, recipes and advancements read again with the world's packs, the
/// synchronized registries `kept` (the hub's) as they were, and that becomes
/// what `sync_data` gives.
pub(in crate::game) fn prepare_wire_reload(
    root: &std::path::Path,
    item_data: &ItemData,
    overlay: &horizon_world::loot::PackOverlay,
    kept: &SyncData,
    sources: &ResolvedSources,
) -> std::result::Result<Arc<SyncData>, String> {
    SyncData::reload(root, overlay, kept, item_data.defaults(), sources).map(|mut sync| {
        sync.features.clone_from(&kept.features);
        Arc::new(sync)
    })
}

pub(in crate::game) fn install_wire_reload(
    root: &std::path::Path,
    item_data: &ItemData,
    overlay: &horizon_world::loot::PackOverlay,
    sync: Option<Arc<SyncData>>,
) {
    // Kept pack dialogs count as packs: a world opened without packs after
    // a `/reload` that dropped them must read its own registries again.
    let mut cache = SYNC.lock().unwrap_or_else(|e| e.into_inner());
    cache.world_packs = !overlay.files.is_empty();
    cache.data = sync.map(|sync| Synced {
        root: root.to_owned(),
        with_packs: !overlay.files.is_empty() || sync.has_pack_entries(),
        generation: item_data.defaults().generation(),
        result: Ok(sync),
    });
}

impl Game<'_> {
    /// The commands' `minecraft:dialog` ids as the clients get them in the
    /// configuration phase (`sync`, the world's packs' dialogs among them):
    /// `/dialog show` parses an id against these and sends its index.
    pub(in crate::game) fn sync_dialog_ids(&mut self, sync: &SyncData) {
        let Some(names) = sync.dynamic.registries.get("minecraft:dialog") else {
            return;
        };
        let ids = names.iter().map(|(name, _)| name.clone()).collect();
        self.ui
            .registries
            .by_registry
            .entry("minecraft:dialog".to_owned())
            .or_default()
            .ids = ids;
    }
}

/// A server's own data load over `item_data`'s registry inputs: the owner of
/// the data a server reads and sends, whichever window shares the `ItemData`.
pub(in crate::game) fn server_data_load(
    root: &std::path::Path,
    item_data: &ItemData,
) -> std::io::Result<horizon_world::native_load_context::LoadContext> {
    data_load_context(
        root,
        item_data,
        horizon_world::native_load_context::DataLoadOrigin::ServerDataLoad,
    )
}

fn data_load_context(
    root: &std::path::Path,
    item_data: &ItemData,
    origin: horizon_world::native_load_context::DataLoadOrigin,
) -> std::io::Result<horizon_world::native_load_context::LoadContext> {
    use horizon_world::native_load_context::DataLoadOrigin;
    let inventory = horizon_mod_package::enabled::EnabledMods::read(root)?;
    let ids: Vec<_> = if origin == DataLoadOrigin::ServerDataLoad {
        inventory.server_ids().collect()
    } else {
        inventory.client_ids().collect()
    };
    Ok(item_data.context().for_load(origin, ids))
}

/// The registries and tags a server sends without data packs over them, read
/// on a helper thread while a server's world prepares: most worlds have no
/// packs, so it is what `sync_data` would load once the world is open.
pub(in crate::game) struct SyncPreload {
    root: std::path::PathBuf,
    generation: u64,
    worker: std::thread::JoinHandle<std::result::Result<SyncData, String>>,
}

/// Start reading the server's data beside the world's preparation; `None`
/// when it is read already, or being read (`preload_world_data`).
pub(in crate::game) fn preload_sync_data(item_data: &Arc<ItemData>) -> Option<SyncPreload> {
    let generation = item_data.defaults().generation();
    if !SYNC.try_lock().is_ok_and(|sync| {
        sync.damage.is_none()
            && sync
                .data
                .as_ref()
                .is_none_or(|cached| cached.generation != generation)
    }) {
        return None;
    }
    let root = item_data.root().to_owned();
    let item_data = Arc::clone(item_data);
    // Loading recipes asks mods' ingredient and recipe property set hooks, wasm mods included.
    horizon_guest_stack::spawn("Server data preload".into(), move || {
        let none = horizon_world::loot::PackOverlay::NONE;
        SyncData::load(
            item_data.root(),
            &none,
            item_data.defaults(),
            server_sources(item_data.root(), &item_data, &none)
                .map_err(|error| error.to_string())?
                .as_ref(),
        )
    })
    .map(|worker| SyncPreload {
        root,
        generation,
        worker,
    })
    .ok()
}

impl SyncPreload {
    /// The world is open: without packs of its own (`overlay`), what was read
    /// becomes `sync_data`'s; with them, `sync_data` loads them as before.
    pub(in crate::game) fn adopt(self, overlay: &horizon_world::loot::PackOverlay) {
        if !overlay.files.is_empty() {
            return;
        }
        if let Ok(read) = self.worker.join() {
            let mut sync = SYNC.lock().unwrap_or_else(|e| e.into_inner());
            if !sync.world_packs && sync.data.as_ref().is_none_or(|cached| cached.with_packs) {
                sync.data = Some(Synced {
                    root: self.root,
                    with_packs: false,
                    generation: self.generation,
                    result: read.map(Arc::new),
                });
            }
        }
    }
}
