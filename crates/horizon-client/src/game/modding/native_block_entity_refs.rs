//! Block entity handles: storage writes join the journal; explicit world notifications happen
//! immediately and survive abort. Raw mod storage writes and restores synthesize no notice.
use super::Game;
use horizon_client::{
    block_entity_nbt,
    containers::{BlockEntity, ModBlockEntity},
    lan::menu::{Menu, Slots},
    save,
    world::World,
};
use horizon_core::nbt::{Compound, Tag};
use horizon_items::items::ItemStack;
use horizon_world::registry::BlockStateId;
use mod_api::__private::game::*;
use mod_api::{
    BlockEntityDataError, BlockEntityError, BlockEntityItemError, BlockEntityRef,
    BlockEntityTypeKey, LevelRef, PlayerRef, Transaction,
};

/// The most chunks one `level_block_entities` read spans.
const MAX_BLOCK_ENTITY_CHUNKS: i64 = 64;

impl Game<'_> {
    fn native_block_entity_target(&self, entity: BlockEntityRef) -> Option<(&World, &BlockEntity)> {
        let world = self.native_level_read_world(entity.fields().level)?;
        let fields = entity.fields();
        if world.block_entities.generation(&(
            fields.position[0],
            fields.position[1],
            fields.position[2],
        )) != Some(fields.generation)
        {
            return None;
        }
        Some((world, loaded_block_entity(world, fields.position)?))
    }

    /// Find a block entity in a loaded chunk of an open level, or on a client in
    /// the level it shows. An entity taken out for its tick cannot be read in that tick.
    pub(super) fn block_entity(&self, level: LevelRef, pos: [i32; 3]) -> Option<BlockEntityRef> {
        let world = self.native_level_read_world(level)?;
        loaded_block_entity(world, pos)?;
        let generation = world.block_entities.generation(&(pos[0], pos[1], pos[2]))?;
        Some(mod_api::__private::game::block_entity_ref(
            level, pos, generation,
        ))
    }

    /// `block_entity`, also for a block entity taken out for its own tick: its generation
    /// stays in place while it is out.
    pub(super) fn block_entity_identity(
        &self,
        level: LevelRef,
        pos: [i32; 3],
    ) -> Option<BlockEntityRef> {
        let world = self.native_level_read_world(level)?;
        if !world.loaded(pos[0], pos[2]) {
            return None;
        }
        let generation = world.block_entities.generation(&(pos[0], pos[1], pos[2]))?;
        Some(mod_api::__private::game::block_entity_ref(
            level, pos, generation,
        ))
    }

    /// The block entities in the box `min` to `max` (maxima exclusive) of an open server
    /// level, in its loaded chunks, sorted by position (`LevelChunk.getBlockEntities` over the
    /// box's chunks). None for an empty box, one wholly outside the level's height and one over
    /// more than 64 chunks.
    pub(super) fn level_block_entities(
        &self,
        level: LevelRef,
        min: [i32; 3],
        max: [i32; 3],
    ) -> Option<Vec<mod_api::BlockEntityAt>> {
        let world = self.native_level_world(level)?;
        if (0..3).any(|axis| min[axis] >= max[axis]) {
            return None;
        }
        let bottom = world.shape.min_y;
        if max[1] <= bottom || i64::from(min[1]) >= i64::from(bottom) + i64::from(world.shape.height)
        {
            return None;
        }
        // `max` is past the box, so its last block is `max - 1`: no overflow, min < max.
        let (chunk_min, chunk_max) = ([min[0] >> 4, min[2] >> 4], [(max[0] - 1) >> 4, (max[2] - 1) >> 4]);
        let across = |axis: usize| i64::from(chunk_max[axis]) - i64::from(chunk_min[axis]) + 1;
        if across(0) * across(1) > MAX_BLOCK_ENTITY_CHUNKS {
            return None;
        }
        let inside = |(x, y, z): (i32, i32, i32)| {
            (min[0]..max[0]).contains(&x) && (min[1]..max[1]).contains(&y) && (min[2]..max[2]).contains(&z)
        };
        let mut found = Vec::new();
        for chunk_x in chunk_min[0]..=chunk_max[0] {
            for chunk_z in chunk_min[1]..=chunk_max[1] {
                if !world.chunks.contains_key(&(chunk_x, chunk_z)) {
                    continue;
                }
                for &pos in world.block_entities.in_chunk((chunk_x, chunk_z)) {
                    if !inside(pos) {
                        continue;
                    }
                    let Some(entity) = world
                        .block_entities
                        .get(&pos)
                        .filter(|entity| !save::block_entity_placeholder(entity))
                    else {
                        continue;
                    };
                    let (x, y, z) = pos;
                    let Some(block) = self.pack.assets.registry.state(world.block(x, y, z)) else {
                        continue;
                    };
                    let id = save::block_entity_id(entity, &block.name).to_owned();
                    found.push(mod_api::BlockEntityAt::new([x, y, z], id));
                }
            }
        }
        found.sort_unstable_by_key(|at| at.pos);
        Some(found)
    }

    /// The data of the block entity at `pos`, as saved without identity or position
    /// metadata, for the hook that hears its block's removal: the entity is gone from
    /// the world by then, so it is read first. None for a block without one, or while
    /// no mod listens.
    pub(in crate::game) fn removed_block_entity_data(
        &self,
        pos: [i32; 3],
        state: BlockStateId,
    ) -> Option<Compound> {
        if !mod_api::__private::listened(mod_api::HookId::PlayerDestroyedBlock) {
            return None;
        }
        let entity = loaded_block_entity(&self.world, pos)?;
        let block = &self.pack.assets.registry.state(state)?.name;
        save_without_metadata(&self.world, (pos[0], pos[1], pos[2]), entity, block)
    }

    /// The saved data of `entity` at `key` without identity or position metadata, with its
    /// `CustomName`, for a block `block`: what a handle to it reads.
    pub(in crate::game) fn saved_block_entity_data(
        &self,
        key: (i32, i32, i32),
        entity: &BlockEntity,
        block: &str,
    ) -> Option<Compound> {
        save_without_metadata(&self.world, key, entity, block)
    }

    /// Opens a block entity's menu for a connected player (`ServerPlayer.openMenu`).
    /// Returns Stale for an invalidated entity; Ok(false) for a wrong-level player,
    /// non-provider, cancelled open, or an unconnected player such as a fake player.
    pub(super) fn player_open_menu(
        &mut self,
        player: PlayerRef,
        entity: BlockEntityRef,
    ) -> Result<bool, BlockEntityError> {
        self.native_block_entity_target(entity)
            .ok_or(BlockEntityError::Stale)?;
        if self.player_handle_read(player).is_none() || !self.level_in_place(entity.fields().level)
        {
            return Ok(false);
        }
        let Some(conn) = self.lan.as_ref().and_then(|host| {
            host.hub
                .guests
                .iter()
                .find(|guest| {
                    guest.uuid == player.fields().uuid
                        && guest.placed
                        && guest.dimension == entity.fields().level.dimension_id()
                })
                .map(|guest| guest.conn)
        }) else {
            return Ok(false);
        };
        let [x, y, z] = entity.fields().position;
        match self.open_block_menu(conn, (x, y, z)) {
            super::lan_actions::MenuOpen::Stale => Err(BlockEntityError::Stale),
            opened => Ok(opened == super::lan_actions::MenuOpen::Opened),
        }
    }

    /// The block entity's type (`BlockEntity.getType`).
    pub(super) fn block_entity_kind(&self, entity: BlockEntityRef) -> Option<BlockEntityTypeKey> {
        let (world, block_entity) = self.native_block_entity_target(entity)?;
        let [x, y, z] = entity.fields().position;
        let block = &self.pack.assets.registry.state(world.block(x, y, z))?.name;
        mod_api::__private::block_entity_type_key(save::block_entity_id(block_entity, block))
    }

    /// The block entity's position.
    pub(super) fn block_entity_position(&self, entity: BlockEntityRef) -> Option<[i32; 3]> {
        self.native_block_entity_target(entity)?;
        Some(entity.fields().position)
    }

    /// The block entity's level.
    pub(super) fn block_entity_level(&self, entity: BlockEntityRef) -> Option<LevelRef> {
        self.native_block_entity_target(entity)?;
        Some(entity.fields().level)
    }

    /// The data saved without identity or position metadata
    /// (`BlockEntity.saveWithoutMetadata`).
    pub(super) fn block_entity_data(&self, entity: BlockEntityRef) -> Option<Compound> {
        let (world, block_entity) = self.native_block_entity_target(entity)?;
        let [x, y, z] = entity.fields().position;
        let block = &self.pack.assets.registry.state(world.block(x, y, z))?.name;
        save_without_metadata(world, (x, y, z), block_entity, block)
    }

    /// The vanilla lock's item predicate, if present on a loaded block entity.
    pub(super) fn block_entity_lock(&self, entity: BlockEntityRef) -> Option<Compound> {
        let (world, _) = self.native_block_entity_target(entity)?;
        let [x, y, z] = entity.fields().position;
        match world.block_lock((x, y, z))? {
            Tag::Compound(lock) => Some(lock.clone()),
            _ => None,
        }
    }

    /// Change the lock through the same storage used by container opening and saving.
    pub(super) fn block_entity_set_lock(
        &mut self,
        entity: BlockEntityRef,
        lock: Option<Compound>,
    ) -> Result<bool, BlockEntityError> {
        let (key, block) = self
            .native_block_entity_write_target(entity)
            .ok_or(BlockEntityError::Stale)?;
        if matches!((self.world.block_lock(key), &lock), (None, None))
            || matches!((self.world.block_lock(key), &lock), (Some(Tag::Compound(old)), Some(new)) if old == new)
        {
            return Ok(false);
        }
        let Some(mut tag) = self
            .world
            .block_entities
            .get(&key)
            .and_then(|current| save_with_metadata(&self.world, key, current, &block))
        else {
            return Ok(false);
        };
        match lock {
            Some(lock) => {
                tag.insert("lock".into(), Tag::Compound(lock));
            }
            None => {
                tag.remove(&"lock".into());
            }
        }
        self.journal_block_entity(key);
        self.world.set_block_lock(key, &Tag::Compound(tag));
        self.block_entity_updated(key);
        self.container_changed(key);
        Ok(true)
    }

    /// A loaded block entity of the server level in place, when a write may change it.
    fn native_block_entity_write_key(&self, entity: BlockEntityRef) -> Option<(i32, i32, i32)> {
        if !self.level_in_place(entity.fields().level) {
            return None;
        }
        let pos = entity.fields().position;
        self.native_block_entity_target(entity)?;
        Some((pos[0], pos[1], pos[2]))
    }

    /// The block entity's key and block name when a write may change it.
    fn native_block_entity_write_target(
        &self,
        entity: BlockEntityRef,
    ) -> Option<((i32, i32, i32), String)> {
        let key = self.native_block_entity_write_key(entity)?;
        let block = self
            .pack
            .assets
            .registry
            .state(self.world.block(key.0, key.1, key.2))?
            .name
            .clone();
        Some((key, block))
    }

    /// A snapshot of a mod block entity's menu data slots, on the server or in the
    /// client's copy. None for a stale/missing/vanilla entity. Item storage is separate.
    pub(super) fn block_entity_data_slots(&self, entity: BlockEntityRef) -> Option<Vec<i32>> {
        let (_, BlockEntity::Mod(block_entity)) = self.native_block_entity_target(entity)? else {
            return None;
        };
        Some(block_entity.data.clone())
    }

    /// Set a mod block entity's menu data slot silently in the current transaction.
    /// Ok(false) for a vanilla entity, invalid slot or equal value; Stale for an invalidated
    /// or unwritable handle. Notification is explicit; this does not resize slots.
    pub(super) fn block_entity_set_data_slot(
        &mut self,
        entity: BlockEntityRef,
        slot: u32,
        value: i32,
    ) -> Result<bool, BlockEntityError> {
        let key = self
            .native_block_entity_write_key(entity)
            .ok_or(BlockEntityError::Stale)?;
        let Some(block_entity) = self.mod_block_entity_write_target(key) else {
            return Ok(false);
        };
        let index = slot as usize;
        if block_entity
            .data
            .get(index)
            .is_none_or(|current| *current == value)
        {
            return Ok(false);
        }
        self.write_mod_block_entity(key, |block_entity| block_entity.data[index] = value);
        Ok(true)
    }

    /// Sizes a mod block entity's menu data explicitly, using the same write
    /// target, journal and Changed notice as scalar data-slot writes.
    pub(super) fn block_entity_set_data_slots(
        &mut self,
        entity: BlockEntityRef,
        values: Vec<i32>,
    ) -> Result<bool, BlockEntityDataError> {
        let limit = usize::from(horizon_mod_package::content::MAX_MENU_SLOTS);
        if values.len() > limit {
            return Err(BlockEntityDataError::TooManySlots {
                count: values.len() as u64,
                limit: limit as u64,
            });
        }
        let key = self
            .native_block_entity_write_key(entity)
            .ok_or(BlockEntityDataError::Stale)?;
        let block_entity = self
            .mod_block_entity_write_target(key)
            .ok_or(BlockEntityDataError::Unavailable)?;
        if block_entity.data == values {
            return Ok(false);
        }
        self.write_mod_block_entity(key, |block_entity| block_entity.data = values);
        Ok(true)
    }

    fn mod_block_entity_write_target(&self, key: (i32, i32, i32)) -> Option<&ModBlockEntity> {
        let BlockEntity::Mod(block_entity) = self.world.block_entities.get(&key)? else {
            return None;
        };
        Some(block_entity)
    }

    // The target was resolved above, and journaling invokes no listener that could replace it.
    fn write_mod_block_entity(
        &mut self,
        key: (i32, i32, i32),
        edit: impl FnOnce(&mut ModBlockEntity),
    ) {
        self.journal_block_entity(key);
        self.edit_mod_block_entity(key, edit);
    }

    /// `edit` on a resolved mod holder; the caller journaled the storage. Notification is the
    /// mod's explicit responsibility, including any notice it wants while restoring storage.
    fn edit_mod_block_entity(
        &mut self,
        key: (i32, i32, i32),
        edit: impl FnOnce(&mut ModBlockEntity),
    ) {
        let BlockEntity::Mod(block_entity) = self.world.block_entities.get_mut(&key).unwrap()
        else {
            unreachable!("resolved mod holder replaced before its write")
        };
        edit(block_entity);
    }

    /// Read a configured mod holder's declared internal item slot, without automation policy.
    pub(super) fn block_entity_item_slot(
        &self,
        entity: BlockEntityRef,
        slot: u32,
    ) -> Result<Option<ItemStack>, BlockEntityItemError> {
        let (world, BlockEntity::Mod(block_entity)) = self
            .native_block_entity_target(entity)
            .ok_or(BlockEntityItemError::Stale)?
        else {
            return Err(BlockEntityItemError::Unavailable);
        };
        let [x, y, z] = entity.fields().position;
        mod_item_slots(world, (x, y, z), slot)?;
        Ok(block_entity.items.get(slot as usize).cloned().flatten())
    }

    /// Replace internal storage under the existing transaction; external ITEMS routing is separate.
    pub(super) fn block_entity_set_item_slot(
        &mut self,
        entity: BlockEntityRef,
        tx: &Transaction,
        slot: u32,
        stack: Option<ItemStack>,
    ) -> Result<bool, BlockEntityItemError> {
        if !self.transaction_holds(tx) {
            return Ok(false);
        }
        let key = self
            .native_block_entity_write_key(entity)
            .ok_or(BlockEntityItemError::Stale)?;
        let block_entity = self
            .mod_block_entity_write_target(key)
            .ok_or(BlockEntityItemError::Unavailable)?;
        let slots = mod_item_slots(&self.world, key, slot)?;
        if let Some(stack) = &stack {
            if stack.count == 0
                || stack.item.as_ref() == "minecraft:air"
                || self.items().get(&stack.item).is_none()
            {
                return Err(BlockEntityItemError::InvalidStack);
            }
            let limit = self.stack_max_size(stack);
            if stack.count > limit {
                return Err(BlockEntityItemError::StackTooLarge {
                    count: stack.count,
                    limit,
                });
            }
        }
        let index = slot as usize;
        if block_entity.items.get(index).and_then(Option::as_ref) == stack.as_ref() {
            return Ok(false);
        }
        // A slot of a list already its full length: the journal keeps that slot, not the
        // entity's whole saved data. A list still to grow is kept whole, length and all.
        if index < block_entity.items.len() {
            self.journal_block_entity_slot(key, index);
            self.edit_mod_block_entity(key, |block_entity| block_entity.items[index] = stack);
            return Ok(true);
        }
        self.write_mod_block_entity(key, |block_entity| {
            let items = &mut block_entity.items;
            if items.len() < slots {
                items.resize(slots, None);
            }
            items[index] = stack;
        });
        Ok(true)
    }

    /// Merges `data` into the block entity's saved data and loads it, as `/data merge block`
    /// does (`BlockDataAccessor.setData`: `loadWithComponents`, `setChanged`, then
    /// `sendBlockUpdated`, so comparators re-read it and clients get the update). `id`, `x`,
    /// `y` and `z` in `data` are ignored. Ok(true) when data changed; Ok(false) for data
    /// nested too deep, no change, or data the entity cannot load. Stale or unwritable
    /// handles return BlockEntityError::Stale. Inside a transaction an
    /// abort puts the block entity, its name, `lock`, loot table roll and attachments back.
    pub(super) fn block_entity_merge_data(
        &mut self,
        entity: BlockEntityRef,
        data: Compound,
    ) -> Result<bool, BlockEntityError> {
        self.write_block_entity_data(entity, data, false)
    }

    /// Replaces the saved payload, retaining identity and position metadata.
    /// Uses the same transactional load and client update path as `merge_data`.
    /// Ok(false) for unsupported data or no change; Stale for an invalidated handle.
    pub(super) fn block_entity_replace_data(
        &mut self,
        entity: BlockEntityRef,
        data: Compound,
    ) -> Result<bool, BlockEntityError> {
        self.write_block_entity_data(entity, data, true)
    }

    fn write_block_entity_data(
        &mut self,
        entity: BlockEntityRef,
        data: Compound,
        replace: bool,
    ) -> Result<bool, BlockEntityError> {
        let (key, block) = self
            .native_block_entity_write_target(entity)
            .ok_or(BlockEntityError::Stale)?;
        let mut nbt = data;
        for field in ["id", "x", "y", "z"] {
            nbt.remove(&field.into());
        }
        let nbt = Tag::Compound(nbt);
        if horizon_client::nbt_path::too_deep(&nbt, 0) {
            return Ok(false);
        }
        let Some(old) = self
            .world
            .block_entities
            .get(&key)
            .and_then(|current| save_with_metadata(&self.world, key, current, &block))
            .map(Tag::Compound)
        else {
            return Ok(false);
        };
        let base = if replace {
            let mut metadata = Compound::new();
            if let Tag::Compound(fields) = &old {
                for key in ["id", "x", "y", "z"] {
                    if let Some(value) = fields.get(&key.into()) {
                        metadata.insert(key.into(), value.clone());
                    }
                }
            }
            Tag::Compound(metadata)
        } else {
            old.clone()
        };
        let merged = horizon_client::data_cmd::merged(&base, &nbt);
        if merged == old {
            return Ok(false);
        }
        self.journal_block_entity(key);
        if !self.load_block_entity_data(key, &merged) {
            return Ok(false);
        }
        self.block_entity_updated(key);
        self.container_changed(key);
        Ok(true)
    }

    /// `BlockEntity.setChanged`: notify at once, including inside a transaction. An abort
    /// restores storage, not notifications already delivered to observers or comparators.
    pub(super) fn block_entity_set_changed(
        &mut self,
        entity: BlockEntityRef,
    ) -> Result<bool, BlockEntityError> {
        let key = self
            .native_block_entity_write_key(entity)
            .ok_or(BlockEntityError::Stale)?;
        self.container_changed(key);
        Ok(true)
    }

    /// The custom name (`Nameable.getCustomName`) as plain text. None when the block entity
    /// has none or isn't nameable (chests, barrels, furnaces, hoppers, brewing stands,
    /// banners, beacons, enchanting tables, command blocks...), or for a stale handle or a
    /// client.
    pub(super) fn block_entity_name(&self, entity: BlockEntityRef) -> Option<String> {
        let (world, block_entity) = self.native_block_entity_target(entity)?;
        let [x, y, z] = entity.fields().position;
        match block_entity {
            BlockEntity::Banner(banner) => banner.name.as_ref().map(tag_name),
            _ => world.block_names.get(&(x, y, z)).map(tag_name),
        }
    }

    /// The players whose open menu holds this block entity's container (for openers'
    /// containers, `ContainerOpenersCounter.getEntitiesWithContainerOpen`'s players), from the menus the opener counts read
    /// (`tick_mod_container_viewers`, `ChestBlockEntity$1.isOwnContainer`). Guests in
    /// its level, not spectators nor fake players, then the acting player for a menu
    /// their action opened, which the hub records when the action ends. None for a
    /// stale handle or a client.
    pub(super) fn block_entity_viewers(&self, entity: BlockEntityRef) -> Option<Vec<PlayerRef>> {
        if !self.is_logical_server() {
            return None;
        }
        self.native_block_entity_target(entity)?;
        let epoch = self.native_lifecycle.epoch()?;
        let level = entity.fields().level;
        let [x, y, z] = entity.fields().position;
        let key = (x, y, z);
        let mut viewers: Vec<PlayerRef> = self
            .lan
            .iter()
            .flat_map(|host| host.hub.guests.simulation_iter())
            .filter(|g| {
                // The acting guest's mode is live in `Game` during its turn; the hub
                // hears of a change only in `leave_guest`.
                let spectator = if self.lan_acting_uuid == Some(g.uuid) {
                    self.spectator()
                } else {
                    g.spectator()
                };
                g.participates_online()
                    && g.placed
                    && !spectator
                    && g.dimension == level.dimension_id()
                    && g.menu.as_ref().is_some_and(|m| menu_holds(m, key))
            })
            .filter_map(|g| {
                let id = u32::try_from(g.entity_id).ok()?;
                Some(player_ref(mod_api::EntityRef::new(epoch, id), g.uuid))
            })
            .collect();
        if self.level_in_place(level)
            && !self.lan_acting_fake
            && !self.spectator()
            && self
                .container
                .is_some_and(|open| open == key || self.chest_other_half(open) == Some(key))
            && let Some(player) = self.native_acting_player()
            && !viewers.contains(&player)
        {
            viewers.push(player);
        }
        Some(viewers)
    }

    /// Sets the custom name, or clears it with None, as an anvil-renamed item placed with
    /// `components` sets `CustomName`. Plain text only. Clients get a block entity update.
    /// Ok(false) for an entity that is not nameable; Stale for an invalidated or
    /// unwritable handle. Inside a transaction an abort puts the name back.
    pub(super) fn block_entity_set_name(
        &mut self,
        entity: BlockEntityRef,
        name: Option<&str>,
    ) -> Result<bool, BlockEntityError> {
        let (key, block) = self
            .native_block_entity_write_target(entity)
            .ok_or(BlockEntityError::Stale)?;
        let nameable = self
            .world
            .block_entities
            .get(&key)
            .is_some_and(|current| nameable(key, current, &block));
        if !nameable {
            return Ok(false);
        }
        self.journal_block_entity(key);
        match self.world.block_entities.get_mut(&key) {
            Some(BlockEntity::Banner(banner)) => banner.name = name.map(|name| Tag::String(name.into())),
            _ => {
                match name {
                    Some(name) => self.world.block_names.insert(key, Tag::String(name.into())),
                    None => self.world.block_names.remove(&key),
                };
            }
        }
        self.block_entity_updated(key);
        Ok(true)
    }
}

/// The block entity as `/data get block` shows it (`saveWithFullMetadata`), with the custom
/// name Horizon keeps beside the model.
fn save_with_metadata(
    world: &World,
    pos: (i32, i32, i32),
    entity: &BlockEntity,
    block: &str,
) -> Option<Compound> {
    let mut tag = save::block_entity_data(pos, entity, block, world.block_lock(pos));
    Game::write_pending_loot(world, pos, &mut tag);
    let Tag::Compound(mut data) = tag else {
        return None;
    };
    if let Some(name) = world.block_names.get(&pos) {
        data.insert("CustomName".into(), name.clone());
    }
    Some(data)
}

fn tag_name(name: &Tag) -> String {
    match name {
        Tag::String(text) => text.to_string_lossy(),
        component => horizon_client::component::name_text(component).plain(),
    }
}

pub(super) fn save_without_metadata(
    world: &World,
    pos: (i32, i32, i32),
    entity: &BlockEntity,
    block: &str,
) -> Option<Compound> {
    let mut data = save_with_metadata(world, pos, entity, block)?;
    for key in ["id", "x", "y", "z"] {
        data.remove(&key.into());
    }
    Some(data)
}

/// Whether the block entity keeps a `CustomName` (`Nameable`): its saved form does.
fn nameable(pos: (i32, i32, i32), entity: &BlockEntity, block: &str) -> bool {
    let mut tag = save::block_entity_encode::encode(pos, entity, block);
    if let Tag::Compound(map) = &mut tag {
        map.insert("CustomName".into(), Tag::String("x".into()));
    }
    block_entity_nbt::canonical(&tag).is_some_and(|tag| tag.get("CustomName").is_some())
}

fn mod_item_slots(
    world: &World,
    key: (i32, i32, i32),
    slot: u32,
) -> Result<usize, BlockEntityItemError> {
    let slots = world
        .block_entities
        .mod_container(world.block(key.0, key.1, key.2).0)
        .ok_or(BlockEntityItemError::Unavailable)?
        .slots;
    if slot as usize >= slots {
        return Err(BlockEntityItemError::SlotOutOfBounds {
            slot,
            slots: slots as u32,
        });
    }
    Ok(slots)
}

fn loaded_block_entity(world: &World, [x, y, z]: [i32; 3]) -> Option<&BlockEntity> {
    if !world.loaded(x, z) {
        return None;
    }
    world
        .block_entities
        .get(&(x, y, z))
        .filter(|entity| !save::block_entity_placeholder(entity))
}

/// Whether an open menu holds the container at `key` (`isOwnContainer`): a double
/// chest's holds both halves, an ender chest's the block it was opened from.
fn menu_holds(menu: &Menu, key: (i32, i32, i32)) -> bool {
    match menu.slots {
        Slots::Block(at) => at == key,
        Slots::Double(first, second) => first == key || second == key,
        Slots::Ender => menu.pos == Some(key),
        Slots::Native => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::chunk::Chunk;
    use horizon_world::registry::BlockStateId;
    use mod_api::HostTransaction;
    use mod_api::ServerHost;
    use std::sync::Arc;

    #[test]
    fn saved_data_omits_metadata_but_keeps_contents() {
        let world = World::new(BlockStateId(0), 0);
        let entity = BlockEntity::Chest(vec![None; 27]);
        let data = save_without_metadata(&world, (1, 64, 2), &entity, "minecraft:chest").unwrap();
        for key in ["id", "x", "y", "z"] {
            assert!(!data.contains_key(&key.into()));
        }
        assert!(data.contains_key(&"Items".into()));
    }

    #[test]
    #[ignore = "daily"]
    fn retained_handles_reject_replacements_and_rollback_restores_identity() {
        use super::super::native_entity_refs::tests::with_game;
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let pos = (1, 64, 1);
            let holder = || {
                BlockEntity::Mod(Box::new(ModBlockEntity::new(
                    "test:holder".into(),
                    Default::default(),
                )))
            };
            game.world.block_entities.insert(pos, holder());
            let original = game.block_entity(level, [1, 64, 1]).unwrap();
            assert_eq!(original.set_data_slots(game, vec![7]), Ok(true));
            let mut tentative = None;
            let _: Result<(), ()> = game.transaction(|game, _| {
                game.journal_block_entity(pos);
                game.world
                    .block_entities
                    .insert(pos, BlockEntity::Chest(vec![]));
                assert_eq!(original.data_slots(game), None);
                tentative = game.block_entity(level, [1, 64, 1]);
                Err(())
            });
            assert_eq!(original.data_slots(game), Some(vec![7]));
            assert_eq!(tentative.unwrap().data(game), None);
            let (lease, taken) = game.world.block_entities.take(&pos).unwrap();
            assert_eq!(original.data_slots(game), None);
            assert!(game.world.block_entities.put(lease, taken));
            assert_eq!(original.data_slots(game), Some(vec![7]));
            game.world.block_entities.remove(&pos);
            game.world.block_entities.insert(pos, holder());
            let replacement = game.block_entity(level, [1, 64, 1]).unwrap();
            assert_ne!(original, replacement);
            let stale = Err(BlockEntityError::Stale);
            assert_eq!(original.set_lock(game, None), stale);
            assert_eq!(original.set_name(game, Some("stale")), stale);
            assert_eq!(original.merge_data(game, Compound::new()), stale);
            assert_eq!(original.replace_data(game, Compound::new()), stale);
            assert_eq!(original.set_changed(game), stale);
            assert_eq!(original.set_data_slot(game, 0, 99), stale);
            assert_eq!(
                original.set_data_slots(game, vec![99]),
                Err(BlockEntityDataError::Stale)
            );
            assert_eq!(
                original.item_slot(game, 0),
                Err(BlockEntityItemError::Stale)
            );
            assert_eq!(replacement.data_slots(game), Some(vec![]));
            assert_eq!(replacement.set_data_slots(game, vec![8]), Ok(true));
            game.world.block_entities.load_chunk(vec![(pos, holder())]);
            assert_eq!(
                replacement.set_data_slots(game, vec![99]),
                Err(BlockEntityDataError::Stale)
            );
        });
    }

    /// `LevelRef::block_entities`: the box's block entities in loaded chunks, by position, with
    /// the type their block gives them; a mod block entity out for its tick is left out.
    #[test]
    #[ignore = "daily"]
    fn block_entities_in_a_box_are_listed_by_position() {
        use super::super::native_entity_refs::tests::with_game;
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            for chunk in [(0, 0), (1, 0)] {
                game.world
                    .chunks
                    .insert(chunk, Arc::new(Chunk::empty(chunk.0, chunk.1)));
            }
            let level = game.level("minecraft:overworld").unwrap();
            let state = |id: &str| game.pack.assets.registry.default_state(id).unwrap();
            let placed = [
                ((20, 70, 3), state("minecraft:daylight_detector"), BlockEntity::DaylightDetector),
                ((1, 64, 1), state("minecraft:chest"), BlockEntity::Chest(vec![None; 27])),
                ((1, 64, 0), state("minecraft:barrel"), BlockEntity::Chest(vec![None; 27])),
            ];
            for ((x, y, z), block, entity) in placed {
                game.world.set(x, y, z, block).unwrap();
                game.world.block_entities.insert((x, y, z), entity);
            }
            // Out for its tick, and in a chunk that isn't loaded.
            game.world
                .block_entities
                .insert((2, 64, 2), BlockEntity::Mod(Box::default()));
            game.world
                .block_entities
                .insert((40, 64, 1), BlockEntity::Chest(vec![None; 27]));
            let listed = |game: &Game<'_>, min, max| {
                game.level_block_entities(level, min, max).map(|found| {
                    found
                        .into_iter()
                        .map(|at| (at.pos, at.id))
                        .collect::<Vec<_>>()
                })
            };
            let kind = str::to_owned;
            assert_eq!(
                listed(game, [0, -64, 0], [48, 320, 16]),
                Some(vec![
                    ([1, 64, 0], kind("minecraft:barrel")),
                    ([1, 64, 1], kind("minecraft:chest")),
                    ([20, 70, 3], kind("minecraft:daylight_detector")),
                ])
            );
            assert_eq!(
                listed(game, [0, 64, 1], [16, 65, 2]),
                Some(vec![([1, 64, 1], kind("minecraft:chest"))]),
                "maxima are exclusive"
            );
            assert_eq!(listed(game, [-128, 0, -128], [0, 1, 0]), Some(vec![]));
            assert_eq!(listed(game, [0, 64, 0], [16, 64, 16]), None, "empty");
            assert_eq!(listed(game, [0, 320, 0], [16, 400, 16]), None, "above");
            assert_eq!(listed(game, [0, -100, 0], [16, -64, 16]), None, "below");
            assert!(listed(game, [0, 0, 0], [128, 1, 128]).is_some(), "8 by 8");
            assert_eq!(listed(game, [0, 0, 0], [129, 1, 128]), None, "9 by 8");
        });
    }

    #[test]
    fn lookup_requires_loaded_chunk_and_tracks_removal_and_replacement() {
        let mut world = World::new(BlockStateId(0), 0);
        let pos = [-1, 64, 0];
        let key = (-1, 64, 0);
        world.block_entities.insert(key, BlockEntity::Chest(vec![]));
        assert!(loaded_block_entity(&world, pos).is_none());
        world.chunks.insert((-1, 0), Arc::new(Chunk::empty(-1, 0)));
        assert!(matches!(
            loaded_block_entity(&world, pos),
            Some(BlockEntity::Chest(_))
        ));
        world.block_entities.remove(&key);
        assert!(loaded_block_entity(&world, pos).is_none());
        world
            .block_entities
            .insert(key, BlockEntity::DaylightDetector);
        assert!(matches!(
            loaded_block_entity(&world, pos),
            Some(BlockEntity::DaylightDetector)
        ));
        world
            .block_entities
            .insert(key, BlockEntity::Mod(Box::default()));
        assert!(loaded_block_entity(&world, pos).is_none());
    }

    fn compound(snbt: &str) -> Compound {
        match horizon_core::nbt::parse_snbt(snbt).unwrap().0 {
            Tag::Compound(c) => c,
            _ => panic!("not a compound"),
        }
    }

    #[test]
    #[ignore = "daily"]
    fn player_opens_provider_and_refuses_non_provider() {
        use mod_api::*;
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "game::native_block_entity_refs::tests::player_opens_provider_and_refuses_non_provider",
            "HORIZON_BE_MENU_CASE",
            &[
                "provider",
                "replacement",
                "existing",
                "close",
                "close_reentry",
            ],
        ) {
            return;
        }
        struct ReplaceDuringOpen;
        impl ReplaceDuringOpen {
            fn replace(host: &mut dyn ServerHost) {
                let level = host.level("minecraft:overworld").unwrap();
                for block in ["minecraft:stone", "minecraft:chest"] {
                    let state = host.block_state_id(block).unwrap();
                    assert_eq!(
                        level.set_block(host, [0, 64, 0], state, BlockUpdateFlags::NEIGHBORS),
                        Ok(true)
                    );
                }
            }
        }
        #[mod_api::__subscriptions]
        impl Hooks for ReplaceDuringOpen {
            fn inventory_block_start(
                &self,
                host: &mut dyn ServerHost,
                event: &mut InventoryBlockStartEvent<'_>,
            ) -> Flow {
                if event.pos == [0, 64, 0]
                    && !std::env::var("HORIZON_BE_MENU_CASE")
                        .unwrap()
                        .starts_with("close")
                {
                    Self::replace(host);
                }
                Flow::Continue
            }
            fn player_container_close(
                &self,
                host: &mut dyn ServerHost,
                _event: &mut PlayerContainerCloseEvent<'_>,
            ) {
                let case = std::env::var("HORIZON_BE_MENU_CASE").unwrap();
                if case.starts_with("close") {
                    Self::replace(host);
                    if case == "close_reentry" {
                        let level = host.level("minecraft:overworld").unwrap();
                        let other = host.block_entity(level, [5, 64, 0]).unwrap();
                        let player = host.player(12).unwrap();
                        assert!(player.open_menu(host, other).unwrap());
                    }
                }
            }
        }
        let case = std::env::var("HORIZON_BE_MENU_CASE").unwrap();
        let replacing = case != "provider";
        if replacing {
            let mods: Mods = vec![ModEntry::new("replace", Box::new(ReplaceDuringOpen))].into();
            mod_platform::__private::install(mods.clone());
            crate::game::api_values::install_parts(&mods).unwrap();
        }
        super::super::native_guest_turn_tests::with_guests(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            for (x, name) in [
                (0, "minecraft:chest"),
                (2, "minecraft:furnace"),
                (4, "minecraft:oak_sign"),
            ] {
                let state = game.pack.assets.registry.default_state(name).unwrap();
                assert_eq!(
                    game.level_set_block(level, [x, 64, 0], state, BlockUpdateFlags::NEIGHBORS),
                    Ok(true)
                );
            }
            let player = game.player(12).unwrap();
            let chest = game.block_entity(level, [0, 64, 0]).unwrap();
            if replacing {
                use horizon_client::lan::{Out, PlayerSender};
                use horizon_protocol::{buf::Reader, ids::play::clientbound};
                let (tx, rx) = std::sync::mpsc::channel();
                game.lan.as_mut().unwrap().hub.update_guest(12, |guest| {
                    guest.sender = PlayerSender::new(tx, Arc::default());
                });
                let previous = if case != "replacement" {
                    let state = game.world.block(0, 64, 0);
                    assert_eq!(
                        game.level_set_block(level, [5, 64, 0], state, BlockUpdateFlags::NEIGHBORS),
                        Ok(true)
                    );
                    let other = game.block_entity(level, [5, 64, 0]).unwrap();
                    assert!(player.open_menu(game, other).unwrap());
                    let host = game.lan.as_ref().unwrap();
                    Some(host.hub.guest(12).unwrap().menu.as_ref().unwrap().window)
                } else {
                    None
                };
                while rx.try_recv().is_ok() {}
                assert_eq!(player.open_menu(game, chest), Err(BlockEntityError::Stale));
                let replacement = game.block_entity(level, [0, 64, 0]).unwrap();
                assert_ne!(chest, replacement);
                assert_eq!(chest.position(game), None);
                let menu = game
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guest(12)
                    .unwrap()
                    .menu
                    .as_ref();
                assert_eq!(menu.is_some(), case == "close_reentry");
                if let Some(menu) = menu {
                    assert!(matches!(menu.slots, Slots::Block((5, 64, 0))));
                }
                let mut opens = Vec::new();
                let mut closes = Vec::new();
                for out in rx.try_iter() {
                    if let Out::Packet(packet) = out {
                        let mut reader = Reader::new(&packet);
                        match reader.varint().unwrap() {
                            clientbound::OPEN_SCREEN => opens.push(reader.varint().unwrap()),
                            clientbound::CONTAINER_CLOSE => closes.push(reader.varint().unwrap()),
                            _ => {}
                        }
                    }
                }
                assert_eq!(
                    opens,
                    menu.map(|menu| menu.window).into_iter().collect::<Vec<_>>()
                );
                let expected_closes = if case == "close_reentry" {
                    Vec::new()
                } else {
                    previous.into_iter().collect()
                };
                assert_eq!(closes, expected_closes);
                return;
            }
            let furnace = game.block_entity(level, [2, 64, 0]).unwrap();
            let sign = game.block_entity(level, [4, 64, 0]).unwrap();
            use super::super::lan_actions::MenuOpen;
            assert!(!player.open_menu(game, sign).unwrap());
            assert_eq!(game.open_block_menu(999, (0, 64, 0)), MenuOpen::Refused);
            game.world
                .block_entities
                .insert((0, 64, 0), BlockEntity::Chest(vec![None; 2]));
            assert!(!player.open_menu(game, chest).unwrap());
            game.world
                .block_entities
                .insert((0, 64, 0), BlockEntity::Chest(vec![None; 27]));
            let dimension = game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                std::mem::replace(&mut guest.dimension, "minecraft:the_nether".to_owned())
            });
            assert!(!player.open_menu(game, chest).unwrap());
            {
                let value = dimension;
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| guest.dimension = value);
            }
            let host = game.lan.take();
            assert!(!player.open_menu(game, chest).unwrap());
            assert_eq!(game.open_block_menu(12, (0, 64, 0)), MenuOpen::Refused);
            game.lan = host;
            let mut guest = game.lan.as_mut().unwrap().hub.guests.remove(0);
            guest.kind = horizon_client::lan::guest::GuestKind::Fake;
            game.lan.as_mut().unwrap().hub.guests.insert(0, guest);
            assert_eq!(game.open_block_menu(12, (0, 64, 0)), MenuOpen::Cancelled);
            assert!(!player.open_menu(game, chest).unwrap());
            let mut guest = game.lan.as_mut().unwrap().hub.guests.remove(0);
            guest.kind = horizon_client::lan::guest::GuestKind::Network;
            game.lan.as_mut().unwrap().hub.guests.insert(0, guest);
            assert!(player.open_menu(game, chest).unwrap());
            let stick_lock = compound("{items:\"minecraft:stick\"}");
            assert!(chest.set_lock(game, Some(stick_lock.clone())).unwrap());
            assert_eq!(chest.lock(game), Some(stick_lock));
            assert!(!player.open_menu(game, chest).unwrap());
            {
                let value = Some(horizon_client::items::ItemStack::new("minecraft:stick", 1));
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |g| g.inventory[0] = value);
            }
            assert!(player.open_menu(game, chest).unwrap());
            {
                let value = None;
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |g| g.inventory[0] = value);
            }
            assert!(chest.set_lock(game, None).unwrap());
            assert!(player.open_menu(game, chest).unwrap());
            assert!(player.open_menu(game, furnace).unwrap());

            // Both halves of a double chest participate, whichever half is opened.
            use horizon_world::chunk::BlockState;
            for (x, kind) in [(6, "left"), (7, "right")] {
                let state = game
                    .pack
                    .assets
                    .registry
                    .resolve(&BlockState {
                        name: "minecraft:chest".into(),
                        properties: [
                            ("facing".into(), "north".into()),
                            ("type".into(), kind.into()),
                            ("waterlogged".into(), "false".into()),
                        ]
                        .into_iter()
                        .collect(),
                    })
                    .unwrap();
                assert_eq!(
                    game.level_set_block(level, [x, 64, 0], state, BlockUpdateFlags::NEIGHBORS),
                    Ok(true)
                );
            }
            let left = game.block_entity(level, [6, 64, 0]).unwrap();
            let right = game.block_entity(level, [7, 64, 0]).unwrap();
            assert!(game.chest_partner((6, 64, 0)).is_some());
            for half in [left, right] {
                assert!(
                    half.set_lock(game, Some(compound("{items:\"minecraft:stick\"}")))
                        .unwrap()
                );
                assert!(!player.open_menu(game, left).unwrap());
                assert!(!player.open_menu(game, right).unwrap());
                assert!(half.set_lock(game, None).unwrap());
                assert!(player.open_menu(game, left).unwrap());
            }
            game.world.block_entities.remove(&(2, 64, 0));
            assert_eq!(
                player.open_menu(game, furnace),
                Err(BlockEntityError::Stale)
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn viewers_follow_open_menus_per_level_and_leave_out_fake_players() {
        use super::super::Dim;
        use horizon_world::chunk::BlockState;
        use mod_api::{BlockUpdateFlags, EntityRef, Host};
        super::super::native_guest_turn_tests::with_guests(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let chest = |game: &mut Game<'_>, level, x: i32, kind: &str| {
                let state = game
                    .pack
                    .assets
                    .registry
                    .resolve(&BlockState {
                        name: "minecraft:chest".into(),
                        properties: [
                            ("facing".into(), "north".into()),
                            ("type".into(), kind.into()),
                            ("waterlogged".into(), "false".into()),
                        ]
                        .into_iter()
                        .collect(),
                    })
                    .unwrap();
                assert_eq!(
                    game.level_set_block(level, [x, 64, 0], state, BlockUpdateFlags::NEIGHBORS),
                    Ok(true)
                );
                game.block_entity(level, [x, 64, 0]).unwrap()
            };
            let single = chest(game, level, 0, "single");
            let left = chest(game, level, 6, "left");
            let right = chest(game, level, 7, "right");
            let epoch = game.native_lifecycle.epoch().unwrap();
            // with_guests: uuids and entity ids 12 and 13.
            let guest12 = player_ref(EntityRef::new(epoch, 12), 12);
            let guest13 = player_ref(EntityRef::new(epoch, 13), 13);
            let host: &dyn Host = game;
            assert_eq!(single.viewers(host), Some(vec![]));

            // Two players open the chest, then close it one by one.
            assert!(guest12.open_menu(game, single).unwrap());
            assert!(guest13.open_menu(game, single).unwrap());
            assert_eq!(single.viewers(game), Some(vec![guest12, guest13]));
            game.guest_close_all(12);
            assert_eq!(single.viewers(game), Some(vec![guest13]));
            game.guest_close_all(13);
            assert_eq!(single.viewers(game), Some(vec![]));

            // A menu the acting player's action opened counts before the hub records it,
            // and only once when the hub already has it.
            game.as_guest(13, |game| {
                game.container = Some((0, 64, 0));
                assert_eq!(single.viewers(game), Some(vec![guest13]));
                game.container = None;
            })
            .unwrap();
            assert!(guest12.open_menu(game, single).unwrap());
            game.as_guest(12, |game| {
                game.container = Some((0, 64, 0));
                assert_eq!(single.viewers(game), Some(vec![guest12]));
                game.container = None;
            })
            .unwrap();
            game.guest_close_all(12);

            // A guest that turns spectator during its own turn stops being a viewer then,
            // though the hub still has the old mode until the turn ends.
            assert!(guest12.open_menu(game, single).unwrap());
            game.as_guest(12, |game| {
                assert!(game.set_game_mode(horizon_client::command::args::GameMode::Spectator));
                assert_eq!(single.viewers(game), Some(vec![]));
            })
            .unwrap();
            game.guest_close_all(12);
            game.as_guest(12, |game| {
                assert!(game.set_game_mode(horizon_client::command::args::GameMode::Survival));
            })
            .unwrap();

            // A double chest's menu holds both halves, whichever was opened.
            assert!(guest12.open_menu(game, right).unwrap());
            assert_eq!(left.viewers(game), Some(vec![guest12]));
            assert_eq!(right.viewers(game), Some(vec![guest12]));
            assert_eq!(single.viewers(game), Some(vec![]));

            // A fake player holding the same menu is not a viewer.
            assert!(guest13.open_menu(game, left).unwrap());
            let mut fake = game.lan.as_mut().unwrap().hub.guests.remove(1);
            fake.kind = horizon_client::lan::guest::GuestKind::Fake;
            game.lan.as_mut().unwrap().hub.guests.insert(1, fake);
            assert_eq!(left.viewers(game), Some(vec![guest12]));

            // The same position in another level has none of these viewers.
            let nether = LevelRef::new(epoch, mod_api::DimensionKey::NETHER);
            game.enter_level(Dim::Nether).unwrap();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let nether_right = chest(game, nether, 7, "single");
            assert_eq!(nether_right.viewers(game), Some(vec![]));
            assert_eq!(right.viewers(game), Some(vec![guest12]), "parked level");
            game.enter_level(Dim::Overworld).unwrap();

            // A stale handle reads None.
            game.world.block_entities.remove(&(7, 64, 0));
            assert_eq!(right.viewers(game), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn writes_merge_data_name_and_roll_back_with_a_transaction() {
        use super::super::native_entity_refs::tests::{run_fresh_cases, with_game_at};
        use mod_api::BlockUpdateFlags;
        if run_fresh_cases(
            "game::native_block_entity_refs::tests::writes_merge_data_name_and_roll_back_with_a_transaction",
            "HORIZON_VANILLA_BE_REGISTRY_CHILD",
            &["vanilla"],
        ) {
            return;
        }
        let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let root = super::super::mods::content_root(&base, &Default::default())
            .unwrap()
            .prepared()
            .to_path_buf();
        with_game_at(root, |game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let flags = BlockUpdateFlags::NEIGHBORS;
            let states = ["minecraft:chest", "minecraft:daylight_detector"];
            for (i, name) in states.into_iter().enumerate() {
                let state = game.pack.assets.registry.default_state(name).unwrap();
                assert_eq!(
                    game.level_set_block(level, [i as i32 * 2, 64, 0], state, flags),
                    Ok(true)
                );
            }
            let chest = game.block_entity(level, [0, 64, 0]).unwrap();
            let sensor = game.block_entity(level, [2, 64, 0]).unwrap();
            let stick = "{Items:[{Slot:3b,id:\"minecraft:stick\",count:3}]}";
            let slot3 = |game: &Game<'_>| match game.world.block_entities.get(&(0, 64, 0)) {
                Some(BlockEntity::Chest(items)) => items[3].as_ref().map(|s| s.count),
                _ => panic!("chest block entity missing"),
            };

            // A merge loads the data, ignores the identity keys and keeps the rest.
            assert!(
                chest
                    .merge_data(
                        game,
                        compound(&format!("{{id:\"minecraft:furnace\",x:9,{}", &stick[1..]))
                    )
                    .unwrap()
            );
            assert_eq!(slot3(game), Some(3));
            assert_eq!(
                chest.kind(game).map(|kind| kind.id(game)),
                Some("minecraft:chest".to_owned())
            );
            assert_eq!(chest.position(game), Some([0, 64, 0]));
            assert!(
                !chest.merge_data(game, compound(stick)).unwrap(),
                "no change"
            );
            assert!(chest.merge_data(game, compound("{Items:[]}")).unwrap());
            assert_eq!(slot3(game), None);

            // Names: a chest has one, a daylight detector does not.
            assert_eq!(chest.name(game), None);
            assert!(chest.set_name(game, Some("Loot")).unwrap());
            assert_eq!(chest.name(game), Some("Loot".to_owned()));
            let data = chest.data(game).unwrap();
            assert_eq!(
                data.get(&"CustomName".into()),
                Some(&Tag::String("Loot".into()))
            );
            assert!(!sensor.set_name(game, Some("Nope")).unwrap());
            assert_eq!(sensor.name(game), None);
            assert!(chest.set_name(game, None).unwrap());
            assert_eq!(chest.name(game), None);

            // An unrelated API merge must preserve the loaded rich name, not
            // remove it through the plain-text name accessor's projection.
            let styled = "{text:\"Styled\",color:\"aqua\",extra:[{text:\" chest\",bold:1b}]}";
            let name = Tag::Compound(compound(styled));
            assert!(game.load_block_entity_nbt([0, 64, 0], &format!("{{CustomName:{styled}}}")));
            assert_eq!(game.world.block_names.get(&(0, 64, 0)), Some(&name));
            assert!(chest.merge_data(game, compound(stick)).unwrap());
            assert_eq!(slot3(game), Some(3));
            assert_eq!(game.world.block_names.get(&(0, 64, 0)), Some(&name));
            assert_eq!(
                chest.data(game).unwrap().get(&"CustomName".into()),
                Some(&name)
            );
            assert!(chest.replace_data(game, compound("{}")).unwrap());
            assert_eq!((slot3(game), chest.name(game)), (None, None));

            // An abort puts back data and name; a commit keeps them.
            let host: &mut dyn ServerHost = game;
            let aborted: Result<(), ()> = host.transaction(|host, _| {
                assert!(chest.merge_data(host, compound(stick)).unwrap());
                assert!(chest.set_name(host, Some("Gone")).unwrap());
                assert!(chest.set_changed(host).unwrap());
                Err(())
            });
            assert_eq!(aborted, Err(()));
            assert_eq!((slot3(game), chest.name(game)), (None, None));
            assert!(game.transactions.is_empty());
            let host: &mut dyn ServerHost = game;
            let committed: Result<(), ()> = host.transaction(|host, _| {
                assert!(chest.merge_data(host, compound(stick)).unwrap());
                assert!(chest.set_name(host, Some("Kept")).unwrap());
                Ok(())
            });
            assert_eq!(committed, Ok(()));
            assert_eq!(
                (slot3(game), chest.name(game)),
                (Some(3), Some("Kept".to_owned()))
            );

            // Replacement uses the same journal and restores the original payload on abort.
            let aborted: Result<(), ()> = (game as &mut dyn ServerHost).transaction(|host, _| {
                assert!(chest.replace_data(host, compound("{}")).unwrap());
                Err(())
            });
            assert_eq!(aborted, Err(()));
            assert_eq!(
                (slot3(game), chest.name(game)),
                (Some(3), Some("Kept".to_owned()))
            );

            // Merge preserves omitted data; replacement clears the name and items.
            assert!(
                chest
                    .merge_data(game, compound("{CustomName:\"Changed\"}"))
                    .unwrap()
            );
            assert_eq!(slot3(game), Some(3));
            assert_eq!(chest.name(game), Some("Changed".to_owned()));
            game.world.block_log = Some(Vec::new());
            assert!(chest.replace_data(game, compound("{}")).unwrap());
            assert_eq!((slot3(game), chest.name(game)), (None, None));
            assert_eq!(game.world.block_log, Some(vec![(0, 64, 0)]));

            // Clients are told: the block log names the position, and the chunk is dirty.
            game.world.block_log = Some(Vec::new());
            game.world.modified.clear();
            assert!(chest.set_name(game, Some("Told")).unwrap());
            assert_eq!(game.world.block_log, Some(vec![(0, 64, 0)]));
            assert!(game.world.modified.contains(&(0, 0)));
            game.world.modified.clear();
            assert!(chest.set_changed(game).unwrap());
            assert!(game.world.modified.contains(&(0, 0)));

            // A removed block entity is a stale handle.
            game.world.block_entities.remove(&(0, 64, 0));
            let stale = Err(BlockEntityError::Stale);
            assert_eq!(chest.replace_data(game, compound("{}")), stale);
            assert_eq!(chest.set_changed(game), stale);
            assert_eq!(chest.merge_data(game, compound(stick)), stale);
            assert_eq!(chest.set_name(game, Some("x")), stale);
            assert_eq!(chest.name(game), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn an_abort_puts_back_the_lock_and_the_loot_table_roll() {
        use super::super::native_entity_refs::tests::with_game;
        use mod_api::BlockUpdateFlags;
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let chest = game.pack.assets.registry.default_state("minecraft:chest");
            let flags = BlockUpdateFlags::NEIGHBORS;
            assert_eq!(
                game.level_set_block(level, [0, 64, 0], chest.unwrap(), flags),
                Ok(true)
            );
            let chest = game.block_entity(level, [0, 64, 0]).unwrap();
            let key = (0, 64, 0);
            let lock = compound("{lock:{items:\"minecraft:stick\"}}");
            let table = compound("{LootTable:\"minecraft:chests/simple_dungeon\"}");
            let before = game.world.block_entity_extras(key);

            let host: &mut dyn ServerHost = game;
            let aborted: Result<(), ()> = host.transaction(|host, _| {
                assert!(
                    chest
                        .set_lock(host, Some(compound("{items:\"minecraft:stick\"}")))
                        .unwrap()
                );
                assert!(chest.merge_data(host, table.clone()).unwrap());
                Err(())
            });
            assert_eq!(aborted, Err(()));
            assert_eq!(game.world.block_entity_extras(key), before);
            assert!(game.world.block_lock(key).is_none());

            let host: &mut dyn ServerHost = game;
            let committed: Result<(), ()> = host.transaction(|host, _| {
                assert!(chest.merge_data(host, lock.clone()).unwrap());
                assert!(chest.merge_data(host, table.clone()).unwrap());
                Ok(())
            });
            assert_eq!(committed, Ok(()));
            assert!(game.world.block_lock(key).is_some());
            assert_ne!(game.world.block_entity_extras(key), before);
        });
    }

    /// `LevelWriteHost::set_block` in a level hook: the block entity goes with
    /// its block and comes with the new one, as the Host's write does.
    #[test]
    #[ignore = "daily"]
    fn a_level_hook_set_block_replaces_whole_states_and_their_block_entities() {
        use super::super::mod_api_host::LevelTicks;
        use super::super::native_entity_refs::tests::with_game;
        use horizon_world::chunk::BlockState;
        use mod_api::{BlockUpdateFlags, LevelWriteHost};

        fn write(
            game: &mut Game<'_>,
            pos: [i32; 3],
            state: mod_api::BlockState,
            flags: BlockUpdateFlags,
        ) -> bool {
            game.with_level(|_, level, ticks| {
                LevelWriteHost::set_block(&mut LevelTicks { level, ticks }, pos, state, flags)
            })
        }
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let state = |name: &str, age: Option<&str>| {
                registry
                    .resolve(&BlockState {
                        name: name.into(),
                        properties: age
                            .map(|age| ("age".into(), age.into()))
                            .into_iter()
                            .collect(),
                    })
                    .unwrap()
            };
            let stone = state("minecraft:stone", None);
            let chest = state("minecraft:chest", None);
            let young = state("minecraft:wheat", Some("0"));
            let grown = state("minecraft:wheat", Some("7"));
            use crate::game::api_values::TryIntoApi;
            let api_stone = stone.try_into_api().unwrap();
            let api_chest = chest.try_into_api().unwrap();
            let api_young = young.try_into_api().unwrap();
            let api_grown = grown.try_into_api().unwrap();
            let flags = BlockUpdateFlags::NEIGHBORS | BlockUpdateFlags::KNOWN_SHAPE;
            let key = (0, 64, 0);

            assert!(write(game, [0, 64, 0], api_stone, flags));
            assert!(game.world.block_entities.get(&key).is_none());
            assert!(write(game, [0, 64, 0], api_chest, flags));
            assert!(matches!(
                game.world.block_entities.get(&key),
                Some(BlockEntity::Chest(_))
            ));
            assert!(write(game, [0, 64, 0], api_stone, flags));
            assert_eq!(game.world.block(0, 64, 0), stone);
            assert!(game.world.block_entities.get(&key).is_none());

            // A crop's whole-state growth: no block entity, and a repeat changes nothing.
            assert!(write(game, [0, 64, 0], api_young, flags));
            assert!(write(game, [0, 64, 0], api_grown, flags));
            assert_eq!(game.world.block(0, 64, 0), grown);
            assert!(game.world.block_entities.get(&key).is_none());
            assert!(!write(game, [0, 64, 0], api_grown, flags));

            // Unloaded, outside the build height and invalid: refused, nothing loaded.
            let chunks = game.world.chunks.len();
            assert!(!write(game, [16, 64, 0], api_stone, flags));
            assert!(!write(game, [0, 10000, 0], api_stone, flags));
            assert!(!write(
                game,
                [0, 64, 0],
                mod_api::BlockStateRecord::new(api_stone.block(), u16::MAX).into(),
                flags
            ));
            assert_eq!(game.world.chunks.len(), chunks);
            assert_eq!(game.world.block(0, 64, 0), grown);

            // The same Host write still creates the block entity.
            assert!(game.native_set_block("minecraft:overworld", [1, 64, 0], chest, flags.bits()));
            assert!(matches!(
                game.world.block_entities.get(&(1, 64, 0)),
                Some(BlockEntity::Chest(_))
            ));

            // A client's copy of the level is not the server's.
            game.options.dedicated = None;
            assert!(!write(game, [0, 64, 0], api_stone, flags));
            assert_eq!(game.world.block(0, 64, 0), grown);
        });
    }
}
