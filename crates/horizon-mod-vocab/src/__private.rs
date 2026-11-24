//! The game's side of the mod API: the installed tables, the reload machinery, the structs the
//! game fills for its hooks (`built`) and what it reads of handles and the API's own types
//! (`game`). None of it is part of the API a mod is written against.
use crate::*;
#[path = "__private/registry_installation.rs"]
mod registry_installation;

/// Native libraries' launcher-to-game registration contract, not a mod-facing API.
/// The launcher generates manifest rows independently of hook factories. The game checks
/// them against the packages beside its executable (`horizon_mod_package::runtime`).
/// These types live in the shared API closure so every library uses the same Rust ABI.
pub mod native {
    /// A point listener a library registered. None exists since the injection points were removed
    /// (#19576): `NativeMod::slots` returns an empty list of it.
    pub enum SlotListener {}

    /// One dynamically loaded native library, registered before the engine seal.
    pub struct NativeMod {
        /// Hook factories owned by this library.
        pub entries: fn() -> Vec<crate::ModEntry>,
        /// Always empty since the injection points were removed (#19576). The field stays until
        /// the next planned break of this registration ABI.
        pub slots: fn() -> Vec<SlotListener>,
        /// Manifest identities (empty for the patch library).
        pub built: &'static [BuiltMod],
        /// Install patch copies and return their verification rows.
        pub installs: fn() -> Vec<(&'static str, &'static [&'static str], bool)>,
    }

    impl NativeMod {
        /// The library's registration, as its generated entry module builds it.
        pub fn new(
            entries: fn() -> Vec<crate::ModEntry>,
            slots: fn() -> Vec<SlotListener>,
            built: &'static [BuiltMod],
            installs: fn() -> Vec<(&'static str, &'static [&'static str], bool)>,
        ) -> Self {
            Self {
                entries,
                slots,
                built,
                installs,
            }
        }
    }

    /// One native mod supplied by a library, with its manifest's `side`.
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct BuiltMod {
        pub id: &'static str,
        pub version: &'static str,
        pub side: crate::Side,
    }
}

/// Switch an installed mod's dispatches on or off, returning whether it was found.
pub fn set_mod_enabled(id: &str, on: bool) -> bool {
    crate::set_mod_enabled(id, on)
}

/// Resolve a listener's owner once at load, for dispatch through [`mod_index_on`].
pub fn mod_index(id: &str) -> Option<usize> {
    crate::mods().iter().position(|entry| entry.id == id)
}

/// Whether a resolved listener's owner is currently enabled.
pub fn mod_index_on(index: usize) -> bool {
    crate::mod_index_on(index)
}

/// Marks the thread as inside mod `id`'s `call` until the guard drops, as a hook's dispatch does,
/// so a panic in it names both.
pub fn enter(id: &'static str, call: &'static str) -> crate::content::crash::ModGuard {
    crate::mark_mod(id, call)
}

/// Install the process-wide ordered list of mods.
pub fn install(mods: Mods) {
    crate::install(mods);
}

/// Whether an installed mod subscribes to `hook`.
pub fn listened(hook: HookId) -> bool {
    crate::listened(hook)
}

/// Whether at least one enabled mod subscribes to `hook`, visiting only its subscribers.
pub fn enabled_listened(hook: HookId) -> bool {
    crate::runtime::enabled_listened(hook)
}

/// Whether any part subscribed to the scoped entity hook `hook` is attached to
/// entity type `kind`, so a family's walk can resolve its rows' slots once per
/// pass instead of naming each row's type.
pub fn entity_part_hooked(hook: HookId, name: &'static str, kind: &str) -> bool {
    crate::parts::parts(hook, name)
        .for_hook(hook)
        .is_some_and(|table| !table.entity(|| Some(kind)).is_empty())
}

/// A read backend wrapper for a host the game builds narrowed: the mob tick's
/// goal view hands itself out as its own backend.
pub fn backend_ref(inner: &dyn Backend) -> crate::BackendRef<'_> {
    crate::BackendRef::of(inner)
}
/// Call `read` on `host`'s real backend, or `None` where the host has none:
/// the mob tick's goal view forwards the level's own reads — blocks, loaded
/// chunks, time, height, weather, block entities — through it; the
/// entity data its snapshot does not carry stays refused. Returned borrows stay tied to the host.
pub fn backend_read<'a, R>(
    host: &'a dyn crate::Host,
    read: impl FnOnce(&'a dyn Backend) -> R,
) -> Option<R> {
    host.backend().map(|backend| read(backend.open()))
}
/// The trait mods installed, in dispatch order: the game's own list, not part of the mod surface.
pub fn mods() -> &'static [ModEntry] {
    crate::mods()
}

/// Actual installed code identities, including mods with no hook subscriptions.
/// The engine uses this dispatch inventory to apply its saved code switches.
pub fn installed_code_ids() -> impl Iterator<Item = &'static str> {
    crate::mods().iter().map(|entry| entry.id)
}

/// What the installed code mod `id` carries that a joining client must have, as the end of a
/// sentence about the mod: its first subscription to a client-side hook, its first part that
/// subscribes to one, or its first required payload channel sent to clients. The engine refuses
/// a `side = "server"` mod that has one. Read once the parts are installed.
pub fn client_requirement(id: &str) -> Option<String> {
    let mine = || crate::mods().iter().filter(|entry| entry.id == id);
    if let Some(hook) = mine().find_map(|entry| entry.client_hook()) {
        return Some(format!("listens to the client hook {hook}"));
    }
    let installed = crate::parts::installed()?;
    if let Some((part, hook)) = installed.client_part(id) {
        return Some(format!(
            "attaches the part {part}, which listens to the client hook {hook}"
        ));
    }
    let channel = installed.keys.payloads.as_ref()?.client_bound(id)?;
    Some(format!("sends the required payload {channel} to clients"))
}

/// The command argument types mods registered at startup, by namespaced id, for the game's
/// command trees. Not part of the mod surface.
pub fn command_argument_types() -> Vec<(String, std::sync::Arc<dyn crate::CommandArgumentType>)> {
    crate::parts::installed_command_arguments()
        .iter()
        .map(|info| (info.id.to_string(), info.argument.clone()))
        .collect()
}

/// The selector tokens mods registered at startup, by name, for the game's command
/// registries. Not part of the mod surface.
pub fn entity_selector_types() -> impl Iterator<Item = (&'static str, &'static crate::SelectorBase)>
{
    crate::parts::installed_entity_selectors()
        .iter()
        .map(|info| (info.name.as_ref(), &info.base))
}

/// Resolve an installed selector's handler by reference, without allocating or cloning a resource.
pub fn entity_selector_type(name: &str) -> Option<&'static dyn crate::EntitySelectorType> {
    let selectors = crate::parts::installed_entity_selectors();
    selectors
        .binary_search_by(|info| info.name.as_ref().cmp(name))
        .ok()
        .map(|index| selectors[index].selector.as_ref())
}

/// The startup package inventory at an owning engine load boundary.
pub fn loaded_mods() -> LoadedMods {
    LoadedMods::new()
}

/// The data-pack and resource reloads the game runs.
pub mod reload {
    include!(concat!(
        env!("OUT_DIR"),
        "/mod_api_exports_private_reload.rs"
    ));
}

/// The structs the game fills for its hooks, each turned into the API's own with `From`.
pub mod built {
    include!(concat!(
        env!("OUT_DIR"),
        "/mod_api_exports_private_built.rs"
    ));
    /// The generator's surface snapshot, constructed by the game and only read by mods.
    pub fn terrain_column(
        height: i32,
        block: String,
        water: Option<i32>,
        biome: String,
    ) -> crate::TerrainColumn {
        crate::TerrainColumn {
            height,
            block,
            water,
            biome,
        }
    }
    /// A retained material run, constructed by the game from the LOD tile.
    pub fn lod_column_footprint(
        min_y: i16,
        top_y: i16,
        state: crate::BlockStateId,
        biome: String,
        sky_light: u8,
        block_light: u8,
    ) -> crate::LodColumnFootprint {
        crate::LodColumnFootprint {
            min_y,
            top_y,
            state,
            biome,
            sky_light,
            block_light,
        }
    }
    /// A retained LOD tile's exact cell ordering and scale.
    pub fn lod_tile_footprint(
        cell_width: i32,
        columns: Vec<Option<crate::LodColumnFootprint>>,
    ) -> crate::LodTileFootprint {
        crate::LodTileFootprint {
            cell_width,
            columns,
        }
    }
    /// The replay this window plays, constructed by the game and only read by mods.
    pub fn replay_state(
        recording: String,
        standalone: bool,
        position_ms: u32,
        duration_ms: u32,
        speed: f64,
        paused: bool,
        ended: bool,
    ) -> crate::ReplayState {
        crate::ReplayState {
            recording,
            standalone,
            position_ms,
            duration_ms,
            speed,
            paused,
            ended,
        }
    }
    /// The mod export's status snapshot, constructed by the game.
    pub fn export_status(
        request: u32,
        name: String,
        state: crate::ExportState,
        [frame, frames]: [u32; 2],
        [width, height]: [u32; 2],
        (fps_num, fps_den): (u32, u32),
        bytes: u64,
    ) -> crate::ExportStatus {
        crate::ExportStatus {
            request, name, state, frame, frames, width, height, fps_num, fps_den, bytes,
        }
    }
    /// Wrap a slot in the persistent instance group store.
    pub fn instance_group_ref(generation: u64, slot: u32) -> crate::InstanceGroupRef {
        crate::InstanceGroupRef::from_parts(generation, slot).expect("published group generation")
    }
    /// Wrap a slot in the model store's published generation.
    pub fn model_ref(generation: u64, slot: u32) -> crate::ModelRef {
        crate::ModelRef(generation, slot)
    }
    /// Wrap a slot in the pipeline set's published generation.
    pub fn render_pipeline_ref(generation: u64, slot: u32) -> crate::RenderPipelineRef {
        crate::RenderPipelineRef::from_parts(generation, slot)
            .expect("published pipeline generation")
    }
}

/// What the game reads of handles and calls on the API's own types.
pub mod game {
    include!(concat!(env!("OUT_DIR"), "/mod_api_exports_private_game.rs"));
    use crate::*;
    use std::fmt::Debug;

    /// A type's fields the game reads, as `value.fields().name`.
    pub trait Fields {
        type Fields<'a>
        where
            Self: 'a;
        fn fields(&self) -> Self::Fields<'_>;
    }

    /// A block entity handle the game names.
    pub fn block_entity_ref(
        level: LevelRef,
        position: [i32; 3],
        generation: u64,
    ) -> BlockEntityRef {
        BlockEntityRef {
            level,
            position,
            generation,
        }
    }
    /// A client mod's sound handle the game names.
    pub fn sound_ref(id: u64) -> SoundRef {
        SoundRef { id }
    }
    /// A boss bar handle the game names.
    pub fn boss_bar_ref(epoch: WorldEpoch, uuid: u128) -> BossBarRef {
        BossBarRef { epoch, uuid }
    }
    /// A team handle the game names.
    pub fn team_ref(epoch: WorldEpoch, name: String) -> TeamRef {
        TeamRef { epoch, name }
    }
    /// A chunk handle the game names.
    pub fn chunk_ref(level: LevelRef, pos: [i32; 2]) -> ChunkRef {
        ChunkRef { level, pos }
    }
    /// An open menu's handle the game names.
    pub fn menu_ref(player: PlayerRef, window: i32, generation: u64) -> MenuRef {
        MenuRef {
            player,
            window,
            generation,
        }
    }
    /// A player handle the game names.
    pub fn player_ref(entity: EntityRef, uuid: u128) -> PlayerRef {
        PlayerRef { entity, uuid }
    }
    /// The player a completion is for, as the game knows them.
    pub fn command_sender(uuid: u128, name: String, permission: PermissionLevel) -> CommandSender {
        CommandSender {
            uuid,
            name,
            permission,
        }
    }
    /// A stack's modifiers for `item_attribute_modifier`, as its component
    /// gives them.
    pub fn item_attribute_modifiers(entries: Vec<ItemAttributeModifier>) -> ItemAttributeModifiers {
        ItemAttributeModifiers {
            entries,
            changed: false,
        }
    }
    /// The modifiers the hook left, or `None` when no mod changed them.
    pub fn changed_item_attribute_modifiers(
        modifiers: ItemAttributeModifiers,
    ) -> Option<Vec<ItemAttributeModifier>> {
        modifiers.changed.then_some(modifiers.entries)
    }

    /// A stack's enchantments for `get_enchantment_level`, as its
    /// component gives them.
    pub fn item_enchantments(entries: Vec<(Identifier, u32)>) -> ItemEnchantments {
        ItemEnchantments {
            entries,
            changed: false,
        }
    }
    /// The enchantments the hook left, and whether a mod changed them.
    pub fn item_enchantments_entries(
        enchantments: ItemEnchantments,
    ) -> (Vec<(Identifier, u32)>, bool) {
        (enchantments.entries, enchantments.changed)
    }

    pub struct ModelRefFields {
        pub generation: u64,
        pub slot: u32,
    }
    impl Fields for ModelRef {
        type Fields<'a> = ModelRefFields;
        fn fields(&self) -> ModelRefFields {
            ModelRefFields {
                generation: self.0,
                slot: self.1,
            }
        }
    }

    impl Fields for RenderPipelineRef {
        type Fields<'a> = ModelRefFields;
        fn fields(&self) -> ModelRefFields {
            ModelRefFields {
                generation: self.0,
                slot: self.1,
            }
        }
    }

    pub struct BlockEntityRefFields {
        pub level: LevelRef,
        pub position: [i32; 3],
        pub generation: u64,
    }
    impl Fields for BlockEntityRef {
        type Fields<'a> = BlockEntityRefFields;
        fn fields(&self) -> BlockEntityRefFields {
            BlockEntityRefFields {
                level: self.level,
                position: self.position,
                generation: self.generation,
            }
        }
    }

    pub struct SoundRefFields {
        pub id: u64,
    }
    impl Fields for SoundRef {
        type Fields<'a> = SoundRefFields;
        fn fields(&self) -> SoundRefFields {
            SoundRefFields { id: self.id }
        }
    }

    pub struct BossBarRefFields {
        pub epoch: WorldEpoch,
        pub uuid: u128,
    }
    impl Fields for BossBarRef {
        type Fields<'a> = BossBarRefFields;
        fn fields(&self) -> BossBarRefFields {
            BossBarRefFields {
                epoch: self.epoch,
                uuid: self.uuid,
            }
        }
    }

    #[non_exhaustive]
    pub struct TeamRefFields<'a> {
        pub epoch: WorldEpoch,
        pub name: &'a str,
    }
    impl Fields for TeamRef {
        type Fields<'a> = TeamRefFields<'a>;
        fn fields(&self) -> TeamRefFields<'_> {
            TeamRefFields {
                epoch: self.epoch,
                name: &self.name,
            }
        }
    }

    pub struct MenuRefFields {
        pub player: PlayerRef,
        pub window: i32,
        pub generation: u64,
    }
    impl Fields for MenuRef {
        type Fields<'a> = MenuRefFields;
        fn fields(&self) -> MenuRefFields {
            MenuRefFields {
                player: self.player,
                window: self.window,
                generation: self.generation,
            }
        }
    }

    pub struct PlayerRefFields {
        pub entity: EntityRef,
        pub uuid: u128,
    }
    impl Fields for PlayerRef {
        type Fields<'a> = PlayerRefFields;
        fn fields(&self) -> PlayerRefFields {
            PlayerRefFields {
                entity: self.entity,
                uuid: self.uuid,
            }
        }
    }

    pub fn original_player_ref(token: u64) -> OriginalPlayerRef {
        OriginalPlayerRef { token }
    }
    impl Fields for OriginalPlayerRef {
        type Fields<'a> = u64;
        fn fields(&self) -> u64 {
            self.token
        }
    }

    pub struct ModEntryFields<'a> {
        pub id: &'static str,
        pub hooks: &'a dyn Hooks,
    }
    impl Fields for ModEntry {
        type Fields<'a> = ModEntryFields<'a>;
        fn fields(&self) -> ModEntryFields<'_> {
            ModEntryFields {
                id: self.id,
                hooks: &*self.hooks,
            }
        }
    }

    pub trait PartsLookup {
        fn for_hook(&self, hook: HookId) -> Option<&crate::parts::HookParts>;
    }
    impl PartsLookup for Parts {
        fn for_hook(&self, hook: HookId) -> Option<&crate::parts::HookParts> {
            Parts::for_hook(self, hook)
        }
    }

    pub trait CapabilityChecked {
        fn checked(&self, answer: u32, asked: u32, at: &dyn Debug) -> Option<u32>;
    }
    impl<T: ?Sized> CapabilityChecked for Capability<T> {
        fn checked(&self, answer: u32, asked: u32, at: &dyn Debug) -> Option<u32> {
            Capability::checked(self, answer, asked, at)
        }
    }

    pub trait FileToIdConverterPaths {
        fn directory(&self) -> &str;
        fn file_to_id(&self, file: &str) -> Option<Identifier>;
    }
    impl FileToIdConverterPaths for FileToIdConverter {
        fn directory(&self) -> &str {
            FileToIdConverter::directory(self)
        }
        fn file_to_id(&self, file: &str) -> Option<Identifier> {
            FileToIdConverter::file_to_id(self, file)
        }
    }

    pub trait RegistrationContextNew {
        fn new() -> Self;
    }
    impl RegistrationContextNew for RegistrationContext {
        fn new() -> Self {
            RegistrationContext::new()
        }
    }

    pub trait ReloadContextNew {
        fn new(files: crate::world::reload::ReloadFiles) -> Self;
    }
    impl ReloadContextNew for ServerReloadContext {
        fn new(files: crate::world::reload::ReloadFiles) -> Self {
            ServerReloadContext::new(files)
        }
    }
    impl ReloadContextNew for ClientReloadContext {
        fn new(files: crate::world::reload::ReloadFiles) -> Self {
            ClientReloadContext::new(files)
        }
    }

    pub trait ServerReloadListenersOrder {
        fn new() -> Self;
        fn finish(
            self,
        ) -> Result<crate::world::server_reload::OrderedServerReload, ReloadOrderError>;
    }
    impl ServerReloadListenersOrder for crate::world::server_reload::ServerListenerRegistry {
        fn new() -> Self {
            crate::world::server_reload::ServerListenerRegistry::new()
        }
        fn finish(
            self,
        ) -> Result<crate::world::server_reload::OrderedServerReload, ReloadOrderError> {
            crate::world::server_reload::ServerListenerRegistry::finish(self)
        }
    }

    pub trait ClientReloadListenersOrder {
        fn new() -> Self;
        #[allow(clippy::type_complexity)]
        fn finish(
            self,
        ) -> Result<
            Vec<crate::world::reload::RegisteredListener<dyn ClientReloadListener>>,
            ReloadOrderError,
        >;
    }
    impl ClientReloadListenersOrder for crate::client::client_reload::ClientListenerRegistry {
        fn new() -> Self {
            crate::client::client_reload::ClientListenerRegistry::new()
        }
        fn finish(
            self,
        ) -> Result<
            Vec<crate::world::reload::RegisteredListener<dyn ClientReloadListener>>,
            ReloadOrderError,
        > {
            crate::client::client_reload::ClientListenerRegistry::finish(self)
        }
    }

    pub trait ServerReloadResourcesInstall {
        fn install(&mut self, retained: Vec<crate::world::server_reload::RetainedEntry>);
    }
    impl ServerReloadResourcesInstall for crate::world::server_reload::RetainedListeners {
        fn install(&mut self, retained: Vec<crate::world::server_reload::RetainedEntry>) {
            crate::world::server_reload::RetainedListeners::install(self, retained)
        }
    }

    pub trait BackendRefOf<'a> {
        fn of(inner: &'a dyn Backend) -> Self;
    }
    impl<'a> BackendRefOf<'a> for BackendRef<'a> {
        fn of(inner: &'a dyn Backend) -> Self {
            BackendRef::of(inner)
        }
    }

    pub trait BackendMutRefOf<'a> {
        fn of(inner: &'a mut dyn BackendMut) -> Self;
    }
    impl<'a> BackendMutRefOf<'a> for BackendMutRef<'a> {
        fn of(inner: &'a mut dyn BackendMut) -> Self {
            BackendMutRef::of(inner)
        }
    }

    pub trait ClientBackendMutRefOf<'a> {
        fn of(inner: &'a mut dyn ClientBackendMut) -> Self;
    }
    impl<'a> ClientBackendMutRefOf<'a> for ClientBackendMutRef<'a> {
        fn of(inner: &'a mut dyn ClientBackendMut) -> Self {
            ClientBackendMutRef::of(inner)
        }
    }

    mod modblockentityviewnew {
        #[allow(unused_imports)]
        use crate::*;
        #[allow(unused_imports)]
        use serde_json::{Map, Value};
        #[allow(unused_imports)]
        use std::sync::{Arc, OnceLock};
        #[allow(unused_imports)]
        use std::time::Duration;
        pub trait ModBlockEntityViewNew<'a>: Sized {
            fn new(entity: &'a mut ModBlockEntity) -> Self;
        }
        impl<'a> ModBlockEntityViewNew<'a> for ModBlockEntityView<'a> {
            fn new(entity: &'a mut ModBlockEntity) -> Self {
                ModBlockEntityView::new(entity)
            }
        }
    }

    mod reloadfilesnew {
        #[allow(unused_imports)]
        use crate::world::reload::*;
        #[allow(unused_imports)]
        use crate::*;
        #[allow(unused_imports)]
        use crate::{
            FileToIdConverter, Identifier,
            world::reload_order::{ReloadOrder, ReloadOrderError},
        };
        #[allow(unused_imports)]
        use std::{collections::BTreeMap, sync::Arc};
        pub trait ReloadFilesNew: Sized {
            fn new(files: BTreeMap<Identifier, Vec<u8>>) -> Self;
        }
        impl ReloadFilesNew for ReloadFiles {
            fn new(files: BTreeMap<Identifier, Vec<u8>>) -> Self {
                ReloadFiles::new(files)
            }
        }
    }
}

/// Construct a fluid state from load-resolved data supplied by the engine boundary.
pub fn fluid_state(
    fluid: crate::FluidKey,
    amount: u8,
    falling: bool,
    source: bool,
) -> crate::FluidState {
    crate::FluidState {
        fluid,
        amount,
        falling,
        source,
    }
}

#[path = "__private/ingredient_parser.rs"]
pub mod ingredient_parser;

include!(concat!(env!("OUT_DIR"), "/mod_api_exports_private.rs"));
