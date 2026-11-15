//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The host's methods both sides share of the `block_entities` area, implemented by the game and reached through [`Host`].
#[horizon_mod_api_macros::host_area]
pub trait BlockEntitiesHost {
    /// Side: both.
    /// Find a block entity in a loaded chunk of an open level, or on a client in
    /// the level it shows. An entity taken out for its tick cannot be read in that tick.
    #[access(WorldOnly)]
    fn block_entity(&self, level: LevelRef, pos: [i32; 3]) -> Option<BlockEntityRef>;
    /// Side: both.
    /// The handle [`BlockEntitiesHost::block_entity`] gives, and also for a block entity
    /// out of the world for its own tick, whose identity and attachments stay in place
    /// meanwhile: what its Part state (`BlockEntityPartState`) checks a handle against.
    /// Its data reads still find nothing during that tick.
    #[access(WorldOnly)]
    fn block_entity_identity(&self, level: LevelRef, pos: [i32; 3]) -> Option<BlockEntityRef>;
}
/// The server host's methods of the `block_entities` area, implemented by the game and reached through [`ServerHost`].
#[horizon_mod_api_macros::host_area]
pub trait BlockEntitiesServerHost {
    /// Side: server.
    /// Sends clients the block at `pos` and its block entity's update tag
    /// (`block_entity_update_tag`) again, as `Level.sendBlockUpdated`
    /// does: with the level's other block changes, to the players who see its
    /// chunk, once however often it is called before then. Changes made in a
    /// block entity's tick go out at the start of the next tick, as vanilla's
    /// do (`ServerLevel.tick` broadcasts chunks before it ticks block
    /// entities). Call it when what clients show changed (a machine started),
    /// not for every tick's progress: a menu's data slots carry that. Nothing
    /// happens for an unloaded block. It also sends the one-shot fields of
    /// `ModBlockEntityView::set_client_update_fields`.
    #[access(WorldOnly)]
    fn block_entity_sync(&mut self, pos: [i32; 3]);
}
/// The handles' reads of the `block_entities` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait BlockEntitiesBackend {
    /// Side: both.
    /// The block entity's type (`BlockEntity.getType`).
    #[access(WorldOnly)]
    fn block_entity_kind(&self, entity: BlockEntityRef) -> Option<BlockEntityTypeKey>;
    /// Side: both.
    /// The block entity's position.
    #[access(WorldOnly)]
    fn block_entity_position(&self, entity: BlockEntityRef) -> Option<[i32; 3]>;
    /// Side: both.
    /// The block entity's level.
    #[access(WorldOnly)]
    fn block_entity_level(&self, entity: BlockEntityRef) -> Option<LevelRef>;
    /// Side: both.
    /// The data saved without identity or position metadata
    /// (`BlockEntity.saveWithoutMetadata`).
    #[access(WorldOnly)]
    fn block_entity_data(&self, entity: BlockEntityRef) -> Option<Compound>;
    /// Side: both.
    /// The vanilla lock's item predicate, if present on a loaded block entity.
    #[access(WorldOnly)]
    fn block_entity_lock(&self, entity: BlockEntityRef) -> Option<Compound>;
    /// Side: server.
    /// The custom name (`Nameable.getCustomName`) as plain text. None when the block entity
    /// has none or isn't nameable (chests, barrels, furnaces, hoppers, brewing stands,
    /// banners, beacons, enchanting tables, command blocks...), or for a stale handle or a
    /// client.
    #[access(WorldOnly)]
    fn block_entity_name(&self, entity: BlockEntityRef) -> Option<String>;
    /// Side: server.
    /// The players whose open menu holds this block entity's container (for openers'
    /// containers, `ContainerOpenersCounter.getEntitiesWithContainerOpen`'s players), from the menus the opener counts read
    /// (`tick_mod_container_viewers`, `ChestBlockEntity$1.isOwnContainer`). Guests in
    /// its level, not spectators nor fake players, then the acting player for a menu
    /// their action opened, which the hub records when the action ends. None for a
    /// stale handle or a client.
    #[access(WorldOnly)]
    fn block_entity_viewers(&self, entity: BlockEntityRef) -> Option<Vec<PlayerRef>>;
    /// Side: both.
    /// A snapshot of a mod block entity's menu data slots, on the server or in the
    /// client's copy. None for a stale/missing/vanilla entity. Item storage is separate.
    #[access(WorldOnly)]
    fn block_entity_data_slots(&self, entity: BlockEntityRef) -> Option<Vec<i32>>;
    /// Side: both.
    /// Read a configured mod holder's declared internal item slot, without automation policy.
    /// Removed, replaced or reloaded holders return BlockEntityItemError::Stale.
    #[access(WorldOnly)]
    fn block_entity_item_slot(
        &self,
        entity: BlockEntityRef,
        slot: u32,
    ) -> Result<Option<ItemStack>, BlockEntityItemError>;
}
/// The handles' writes of the `block_entities` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait BlockEntitiesBackendMut {
    /// Side: server.
    /// Change the lock through the same storage used by container opening and saving.
    #[access(WorldOnly)]
    fn block_entity_set_lock(
        &mut self,
        entity: BlockEntityRef,
        lock: Option<Compound>,
    ) -> Result<bool, BlockEntityError>;
    /// Side: server.
    /// Merges `data` into the block entity's saved data and loads it, as `/data merge block`
    /// does (`BlockDataAccessor.setData`: `loadWithComponents`, `setChanged`, then
    /// `sendBlockUpdated`, so comparators re-read it and clients get the update). `id`, `x`,
    /// `y` and `z` in `data` are ignored. Ok(true) when data changed; Ok(false) for data
    /// nested too deep, no change, or data the entity cannot load. Stale or unwritable
    /// handles return BlockEntityError::Stale. Inside a transaction an
    /// abort puts the block entity, its name, `lock`, loot table roll and attachments back.
    #[access(WorldOnly)]
    fn block_entity_merge_data(
        &mut self,
        entity: BlockEntityRef,
        data: Compound,
    ) -> Result<bool, BlockEntityError>;
    /// Side: server.
    /// Replaces the saved payload, retaining identity and position metadata.
    /// Uses the same transactional load and client update path as `merge_data`.
    /// Ok(false) for unsupported data or no change; Stale for an invalidated handle.
    #[access(WorldOnly)]
    fn block_entity_replace_data(
        &mut self,
        entity: BlockEntityRef,
        data: Compound,
    ) -> Result<bool, BlockEntityError>;
    /// Side: server.
    /// `BlockEntity.setChanged`: mark the chunk and notify comparators immediately, even
    /// inside a transaction and for equal contents. Aborting storage does not undo an issued
    /// notification. Returns Stale for an invalidated handle or a level not writable here.
    #[access(WorldOnly)]
    fn block_entity_set_changed(
        &mut self,
        entity: BlockEntityRef,
    ) -> Result<bool, BlockEntityError>;
    /// Side: server.
    /// Sets the custom name, or clears it with None, as an anvil-renamed item placed with
    /// `components` sets `CustomName`. Plain text only. Clients get a block entity update.
    /// Ok(false) for an entity that is not nameable; Stale for an invalidated or
    /// unwritable handle. Inside a transaction an abort puts the name back.
    #[access(WorldOnly)]
    fn block_entity_set_name(
        &mut self,
        entity: BlockEntityRef,
        name: Option<&str>,
    ) -> Result<bool, BlockEntityError>;
    /// Side: server.
    /// Set a mod block entity's menu data slot in place, joining the current transaction.
    /// This raw storage write issues no notification; call set_changed explicitly if needed.
    /// Ok(false) for a vanilla entity, invalid slot or no change. Does not resize slots.
    #[access(WorldOnly)]
    fn block_entity_set_data_slot(
        &mut self,
        entity: BlockEntityRef,
        slot: u32,
        value: i32,
    ) -> Result<bool, BlockEntityError>;
    /// Side: server.
    /// Sizes a mod block entity's menu data explicitly, using the same silent storage write
    /// and journal as scalar data-slot writes.
    /// Removed, replaced or reloaded holders return BlockEntityDataError::Stale.
    #[access(WorldOnly)]
    fn block_entity_set_data_slots(
        &mut self,
        entity: BlockEntityRef,
        values: Vec<i32>,
    ) -> Result<bool, BlockEntityDataError>;
    /// Side: server.
    /// Replace internal storage silently under the existing transaction; ITEMS routing is
    /// separate. Removed, replaced or reloaded holders return BlockEntityItemError::Stale.
    #[access(WorldOnly)]
    fn block_entity_set_item_slot(
        &mut self,
        entity: BlockEntityRef,
        tx: &Transaction,
        slot: u32,
        stack: Option<ItemStack>,
    ) -> Result<bool, BlockEntityItemError>;
}
