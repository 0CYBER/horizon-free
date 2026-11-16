//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The host's methods both sides share of the `item_entities` area, implemented by the game and reached through [`Host`].
#[horizon_mod_api_macros::host_area]
pub trait ItemEntitiesHost {
    /// Side: both.
    /// Narrows an entity to a dropped item, or None for another kind, a
    /// stale handle, a client, or an item temporarily taken out for ticking.
    #[access(WorldOnly)]
    fn item_entity(&self, entity: EntityRef) -> Option<ItemEntityRef>;
}
/// The handles' reads of the `item_entities` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait ItemEntitiesBackend {
    /// Side: server.
    /// `ItemEntity.getItem`: the complete stack, or None if unavailable.
    #[access(WorldOnly)]
    fn item_entity_stack(&self, item: ItemEntityRef) -> Option<ItemStack>;
    /// Side: server.
    /// The signed pickup delay; the legacy infinite sentinel reads as vanilla's
    /// 32767. None also covers a stored value outside i32.
    #[access(WorldOnly)]
    fn item_entity_pickup_delay(&self, item: ItemEntityRef) -> Option<i32>;
    /// Side: server.
    /// `ItemEntity.getAge`, including negative lifetime sentinels.
    #[access(WorldOnly)]
    fn item_entity_age(&self, item: ItemEntityRef) -> Option<i32>;
    /// Side: server.
    /// The UUID set by `ItemEntity.setTarget`, limiting pickup to that player;
    /// None also means an unavailable item. This is not the thrower.
    #[access(WorldOnly)]
    fn item_entity_target(&self, item: ItemEntityRef) -> Option<u128>;
    /// Side: server.
    /// `ItemEntity.getOwner`: the thrower as a resolved entity, not the pickup
    /// target. None for no thrower, an unloaded or different-level thrower,
    /// an unavailable handle, or one Horizon cannot resolve.
    #[access(WorldOnly)]
    fn item_entity_owner(&self, item: ItemEntityRef) -> Option<EntityRef>;
}
/// The handles' writes of the `item_entities` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait ItemEntitiesBackendMut {
    /// Side: server.
    /// Writes a nonempty complete stack using `ItemEntity.setItem`. Empty
    /// stacks are refused: vanilla discards the item on its next tick. To
    /// remove an item entity, use the removal handle. False also for an
    /// unavailable item or outside a server.
    ///
    /// This scope is the port's own transactional requirement. NeoForge adds
    /// no event, deferral or drop capture around a block entity's tick; newly
    /// spawned items join immediately. With no scope open, the engine behaves
    /// exactly as vanilla for these item operations: writes are immediate,
    /// the overlay stays absent, and nothing is staged.
    ///
    /// During a mod block-entity tick this write is proposed, not published:
    /// later Parts read the proposal, and it commits only with the accepted
    /// final block-entity snapshot. A stale item or rejected guest response
    /// rejects the whole tick proposal. Outside that scope writes are immediate.
    /// Pickup delay and target changes, spawning, motion, block changes and
    /// non-item removals are deliberately immediate, not rolled back with it.
    #[access(WorldOnly)]
    fn item_entity_set_stack(
        &mut self,
        item: ItemEntityRef,
        stack: ItemStack,
    ) -> Result<bool, StackWriteError>;
    /// Side: server.
    /// `ItemEntity.setPickUpDelay`.
    #[access(WorldOnly)]
    fn item_entity_set_pickup_delay(&mut self, item: ItemEntityRef, ticks: i32) -> bool;
    /// Side: server.
    /// `ItemEntity.setDefaultPickUpDelay`: ten ticks.
    #[access(WorldOnly)]
    fn item_entity_set_default_pickup_delay(&mut self, item: ItemEntityRef) -> bool;
    /// Side: server.
    /// `ItemEntity.setNoPickUpDelay`.
    #[access(WorldOnly)]
    fn item_entity_set_no_pickup_delay(&mut self, item: ItemEntityRef) -> bool;
    /// Side: server.
    /// `ItemEntity.setNeverPickUp`: the infinite-delay sentinel 32767.
    #[access(WorldOnly)]
    fn item_entity_set_never_pickup(&mut self, item: ItemEntityRef) -> bool;
    /// Side: server.
    /// `ItemEntity.setTarget`; None allows any player to pick it up.
    #[access(WorldOnly)]
    fn item_entity_set_target(&mut self, item: ItemEntityRef, target: Option<u128>) -> bool;
}
