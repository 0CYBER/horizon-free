//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The host's registry-wide key reads of the `keys` area, implemented by the game
/// and reached through [`Host`].
#[horizon_mod_api_macros::host_area]
pub trait KeysHost {
    /// Side: server. The biome in the installed world registry, or None on a client,
    /// before world data is installed, or for an unknown id. Reacquire after reload.
    #[access(WorldOnly)]
    fn biome_key(&self, id: &str) -> Option<BiomeKey>;

    /// Side: both. Exact startup game-event id; None before registry completion or for an unknown id.
    #[access(Metadata)]
    fn game_event_key(&self, id: &str) -> Option<GameEventKey>;
    /// Side: both.
    /// The key of the block named `id`, for a name that only arrives at run time (data, config,
    /// a command, a network message). `id` is the full registry name, `minecraft:stone` or
    /// `mymod:machine`. None for a name the registry does not hold, and before the registry is
    /// complete (during registration, use `Registrar::block_key`). Does not allocate.
    #[access(Metadata)]
    fn block_key(&self, id: &str) -> Option<BlockKey>;
    /// Side: both.
    /// The key of the effect named `id`: the full registry name, `minecraft:speed` or
    /// `mymod:rage`. None for a name the registry does not hold, and before the registry is
    /// complete (during registration, use `Registrar::effect_key`). Does not allocate.
    #[access(Metadata)]
    fn effect_key(&self, id: &str) -> Option<EffectKey>;
    /// Side: both.
    /// Resolve a canonical entry in the current side's live registry. None for an
    /// unknown registry or entry, or before that registry is available.
    #[access(WorldOnly)]
    fn registry_entry(&self, registry: &str, id: &str) -> Option<RegistryEntry>;
    /// Side: both.
    /// Entry identifiers in the current table, each once. None for an unavailable or unknown
    /// registry; an available empty registry returns an empty list.
    #[access(WorldOnly)]
    fn registry_entries(&self, registry: &str) -> Option<Vec<Identifier>>;
    /// Side: both.
    /// Membership in the current side's merged tags. None for an unavailable registry or
    /// entry; an absent tag, or a tag not containing the entry, returns false.
    #[access(WorldOnly)]
    fn registry_entry_in_tag(&self, entry: &RegistryEntry, tag: &str) -> Option<bool>;
    /// Side: both.
    /// Resolve a live typed key's load-owned slot, never its protocol numeric id.
    // hidden: RegistryEntry narrowing transports a host-issued opaque key.
    #[doc(hidden)]
    #[access(WorldOnly)]
    fn registry_entry_key_slot(&self, entry: &RegistryEntry) -> Option<u32>;
    /// Side: both.
    /// Every block key in the registry's registration order: vanilla's first, then
    /// mod blocks in the order they registered. A block registry lists a name once
    /// per state; this yields it once. Empty before the block table completes.
    #[access(Metadata)]
    fn block_keys(&self) -> Vec<BlockKey>;
    /// Side: both.
    /// Every item key in the registry's registration order: vanilla's first, then
    /// mod items in the order they registered. Empty before the item table completes.
    #[access(Metadata)]
    fn item_keys(&self) -> Vec<ItemKey>;
    /// Side: both.
    /// Every built-in entity-type key in registry order (`BuiltInRegistries.ENTITY_TYPE`
    /// order): vanilla's first, then mod types in the order they registered.
    #[access(Metadata)]
    fn entity_type_keys(&self) -> Vec<EntityTypeKey>;
    /// Side: both.
    /// Every built-in menu-type key in registry order (`BuiltInRegistries.MENU` order):
    /// vanilla's first, then mod types in the order they registered.
    #[access(Metadata)]
    fn menu_keys(&self) -> Vec<MenuKey>;
    /// Side: both.
    /// Every enchantment key in global sorted loaded-registry order, including
    /// vanilla and mod data-pack entries together (`RegistryDataLoader` sorts by id).
    #[access(Metadata)]
    fn enchantment_keys(&self) -> Vec<EnchantmentKey>;
    /// Side: both.
    /// The network id of `name` in `registry` (`Registry.getId`): the number a packet carries for
    /// it, for a mod that builds or reads raw packets (`minecraft:particle_type` and
    /// `minecraft:flame`). Built-in registries answer their fixed ids, mod entries included;
    /// data-driven ones (`minecraft:damage_type`, `minecraft:biome`, ...) answer the order the
    /// server synced them in; custom data component types are included. A client reads its own
    /// connection's registries, a server the ones it sends. The id holds for one connection and one
    /// data reload: do not keep it across a reconnect, a reconfiguration or `/reload`. None for an
    /// unknown registry or name, on a client before that registry was synced, and with no
    /// connection. Allocates nothing; each registry is searched in order.
    #[access(WorldOnly)]
    fn registry_protocol_id(&self, registry: &Identifier, name: &Identifier) -> Option<i32>;
}
/// The handles' reads of the `keys` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait KeysBackend {
    /// Side: server. Canonical biome id in the installed world generation.
    /// A token from another world or reload returns `BiomeKeyError::WrongGeneration`.
    #[access(WorldOnly)]
    fn biome_key_id(&self, key: BiomeKey) -> Result<String, BiomeKeyError>;

    /// Side: both. Exact id of an installed game-event slot; invalid keys are refused.
    #[access(Metadata)]
    fn game_event_key_id(&self, key: GameEventKey) -> Result<String, GameEventKeyError>;
    /// Side: both.
    /// The canonical block id retained by the load-owned registration slot. An invalid slot
    /// is a boundary error, including when a wasm guest supplies it.
    #[access(Metadata)]
    fn block_key_id(&self, key: BlockKey) -> String;
    /// Side: both.
    /// The canonical item id retained by the load-owned registration slot.
    #[access(Metadata)]
    fn item_key_id(&self, key: ItemKey) -> String;
    /// Side: both.
    /// The canonical effect id retained by the load-owned registration slot.
    #[access(Metadata)]
    fn effect_key_id(&self, key: EffectKey) -> String;
    /// Side: both. Registered MobEffect.getCategory(); unavailable before registry completion.
    #[access(Metadata)]
    fn effect_category(&self, key: EffectKey) -> Option<EffectCategory>;
    /// Side: both.
    /// The canonical fluid id of a key from the completed registry, `minecraft:water`. An
    /// invalid slot is a boundary error, including when a wasm guest supplies it.
    #[access(Metadata)]
    fn fluid_key_id(&self, key: FluidKey) -> String;
    /// Side: both.
    /// The canonical block entity type id retained by the load-owned slot.
    #[access(WorldOnly)]
    fn block_entity_type_key_id(&self, key: BlockEntityTypeKey) -> String;
    /// Side: both.
    /// The canonical entity type id retained by the load-owned slot.
    #[access(Metadata)]
    fn entity_type_key_id(&self, key: EntityTypeKey) -> String;
    /// Side: both.
    /// The canonical menu type id retained by the load-owned slot.
    #[access(Metadata)]
    fn menu_key_id(&self, key: MenuKey) -> String;
    /// Side: both.
    /// The canonical enchantment id retained by the load-owned slot.
    #[access(Metadata)]
    fn enchantment_key_id(&self, key: EnchantmentKey) -> String;
}
