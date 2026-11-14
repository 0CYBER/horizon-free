//! Entity and level handles for native server mods (docs/modding/src/how-to/native-mods.md,
//! "Entity and level handles", #7387): `entity(id)` turns an event's
//! entity id into an `EntityRef`, and each `native_entity_*` helper reads the
//! entity a handle names. They read where the world is simulated: on a client,
//! with no world open, or for a handle of an earlier open, they return `None`,
//! except the reads marked side `both` (`entity`, `level`, `entity_position`,
//! `entity_rotation`, `entity_velocity`), which on a client answer from its own
//! copy: the local player and the entities the server told it of, in the level
//! it shows. The entities are those of the level in place (the one a hook runs
//! in) and the server's players, whatever their level.
#[path = "native_entity_refs/dragon_parts_table.rs"]
pub(in crate::game) mod dragon_parts_table;
#[path = "native_entity_refs/dragon_query.rs"]
mod dragon_query;
#[cfg(test)]
#[path = "native_entity_refs/roll_loot_tests.rs"]
mod roll_loot_tests;
#[path = "native_entity_refs/save_lending.rs"]
mod save_lending;
use super::{
    Game,
    commands::Target,
    entity::{Entity as _, entity_lists_of, for_each_entity},
    transactions::After,
};
use horizon_core::geometry::{Aabb, Vec3};
use horizon_items::items::ItemStack;
use horizon_world::registry::BlockStateId;
use mod_api::{
    BlockFace, BlockShapeKind, BlockUpdateFlags, ClipBlock, ClipFluid, ClipHit, DimensionKey,
    EntityRayHit, EntityRef, LevelRef, NeighbourUpdate,
};

/// The engine's direction of a mod API block face.
pub(super) fn block_face_direction(face: BlockFace) -> horizon_core::geometry::Direction {
    use horizon_core::geometry::Direction;
    match face {
        BlockFace::Down => Direction::Down,
        BlockFace::Up => Direction::Up,
        BlockFace::North => Direction::North,
        BlockFace::South => Direction::South,
        BlockFace::West => Direction::West,
        BlockFace::East => Direction::East,
        _ => unreachable!("BlockFace: unhandled variant"),
    }
}

/// `ProjectileUtil.getEntityHitResult`'s test on each candidate: its box grown by `margin`
/// and clipped against the segment. Every hit, nearest first; a segment that starts inside a
/// box does not hit it (`AABB.clip`).
fn entity_ray_hits(
    from: [f64; 3],
    to: [f64; 3],
    margin: f64,
    candidates: impl IntoIterator<Item = (EntityRef, [[f64; 3]; 2])>,
) -> Vec<EntityRayHit> {
    let start = Vec3::new(from[0], from[1], from[2]);
    let end = Vec3::new(to[0], to[1], to[2]);
    let mut hits: Vec<EntityRayHit> = candidates
        .into_iter()
        .filter_map(|(entity, [lo, hi])| {
            let grown = Aabb::new(
                Vec3::new(lo[0], lo[1], lo[2]),
                Vec3::new(hi[0], hi[1], hi[2]),
            )
            .inflate(Vec3::new(margin, margin, margin));
            let at = grown.clip(start, end)?;
            let location = [at.x, at.y, at.z];
            let distance = (0..3)
                .map(|i| (location[i] - from[i]).powi(2))
                .sum::<f64>()
                .sqrt();
            Some(EntityRayHit {
                entity,
                location,
                distance,
            })
        })
        .collect();
    hits.sort_by(|a, b| a.distance.total_cmp(&b.distance));
    hits
}

impl Game<'_> {
    /// An entity tracked by the remote client but absent from local entity lists.
    fn native_client_tracked(&self, entity: EntityRef) -> Option<(&str, [f64; 3])> {
        let remote = self.remote.as_ref()?;
        if !remote.logged_in || self.native_lifecycle.epoch() != Some(entity.epoch()) {
            return None;
        }
        remote.tracked_entity(i32::try_from(entity.id()).ok()?)
    }

    /// What a handle names now, or None for a stale or foreign one.
    pub(super) fn native_entity_target(&self, entity: EntityRef) -> Option<Target> {
        if self.block_entity_tick_items.is_some() {
            return self.scoped_native_entity_target(entity);
        }
        self.unscoped_native_entity_target(entity)
    }
    /// What a handle names on a client: the local player, or an entity the
    /// server told this client of (`ClientLevel.getEntity`, whose entities are
    /// the ones the server tracks to it). None for a stale handle, with no
    /// login, or for an entity the client doesn't have.
    pub(super) fn native_client_entity_target(&self, entity: EntityRef) -> Option<Target> {
        let remote = self.remote.as_ref().filter(|r| r.logged_in)?;
        (self.native_lifecycle.epoch() == Some(entity.epoch())).then_some(())?;
        let id = i32::try_from(entity.id()).ok()?;
        if id == remote.entity_id {
            return self.player.as_ref().map(|_| Target::Player);
        }
        // The server tracks the dragon alone; its parts are the client's own copies of it.
        if let Some(part) = self.level_dragon_part(entity.id()) {
            return Some(part);
        }
        remote.tracked(id)?;
        match self.level_entity(id)? {
            Target::Mob(mob) if !self.mobs.iter().any(|m| m.id == mob && !m.removed) => None,
            target => Some(target),
        }
    }
    /// What a handle names for a read that works on both sides: the server's
    /// entity (`native_entity_target`) or, on a client, its own copy of it.
    pub(super) fn native_entity_read_target(&self, entity: EntityRef) -> Option<Target> {
        if self.is_logical_server() {
            self.native_entity_target(entity)
        } else {
            self.native_client_entity_target(entity)
        }
    }
    /// The entity `id` (the id events carry) as a handle for the `entity_*`
    /// methods: None with no world open, or for no such entity in the level
    /// in place or among the server's players. On a client (API
    /// @API:client_entity_refs@), any entity the server shows this window,
    /// this window's own player included. API 373.
    pub(super) fn entity(&self, id: u32) -> Option<EntityRef> {
        let entity = EntityRef::new(self.native_lifecycle.epoch()?, id);
        (self.native_entity_read_target(entity).is_some()
            || self.native_client_tracked(entity).is_some())
        .then_some(entity)
    }
    pub(super) fn entity_by_uuid(&self, uuid: mod_api::EntityUuid) -> Option<EntityRef> {
        if !self.is_logical_server() {
            return None;
        }
        let epoch = self.native_lifecycle.epoch()?;
        let dimension = self.dim.key();
        let uuid = horizon_client::lan::playerdata::uuid_ints(uuid.0);
        let mut after = None;
        loop {
            let id = self
                .resident_entities
                .table
                .borrow_mut()
                .level_uuid_candidate(dimension, uuid, after)?;
            let entity = EntityRef::new(epoch, id.get());
            if self
                .native_entity_target(entity)
                .is_some_and(|target| self.resident_source_target_visible(dimension, target))
            {
                return Some(entity);
            }
            // A queued departure can overlap a replacement with the same UUID.
            after = Some(id);
        }
    }
    /// Whether the entity is made of parts that are entities of their own
    /// (`Entity.isMultipartEntity`): the dragon, not one of its parts.
    pub(super) fn entity_is_multipart(&self, entity: EntityRef) -> Option<bool> {
        Some(matches!(
            self.native_entity_read_target(entity)?,
            Target::Dragon(_)
        ))
    }
    /// The entities a multipart entity is made of (`Entity.getParts`), in
    /// `getSubEntities` order, each resolved as any handle is (`entity`): the dragon's
    /// parts have the eight ids after its own. Empty for any other entity.
    pub(super) fn entity_parts(&self, entity: EntityRef) -> Option<Vec<EntityRef>> {
        let Target::Dragon(id) = self.native_entity_read_target(entity)? else {
            return Some(Vec::new());
        };
        (1..=8)
            .map(|offset| self.entity(id.checked_add(offset)?))
            .collect()
    }
    /// The level `dimension` (`minecraft:the_nether`) as a handle: on a client,
    /// the level it shows, whatever the server calls it, or one the server opened
    /// for viewing (`client_level_key`). None with no world open, or for a
    /// dimension the server hasn't loaded. API 373.
    pub(super) fn level(&self, dimension: &str) -> Option<LevelRef> {
        let epoch = self.native_lifecycle.epoch()?;
        if !self.is_logical_server() {
            return Some(LevelRef::new(epoch, self.client_level_key(dimension)?));
        }
        let dim = std::iter::once(self.dim)
            .chain(self.levels.parked_dims())
            .find(|d| d.id() == dimension)?;
        Some(self.level_ref(epoch, dim.key()))
    }
    /// The client level in place as a handle: the level this window shows and
    /// ticks. None before login, while reconfiguring, or with no world open.
    pub(super) fn native_client_level(&self) -> Option<LevelRef> {
        let epoch = self.native_lifecycle.epoch()?;
        let remote = self.remote.as_ref().filter(|r| r.has_level())?;
        Some(LevelRef::new(epoch, remote.level_key))
    }
    /// The name of a dimension's key.
    pub(in crate::game) fn dimension_key_id(&self, key: DimensionKey) -> Option<String> {
        mod_api::__private::dimension_id(key).map(str::to_owned)
    }
    /// The key of the level in place: on a client the level it shows, which
    /// `Dim` may have no name for.
    pub(super) fn native_level_key(&self) -> DimensionKey {
        self.remote
            .as_ref()
            .map_or_else(|| self.dim.key(), |r| r.level_key)
    }
    /// Loaded server levels, in vanilla dimension order. Empty without a server world.
    pub(super) fn native_list_levels(&self) -> Vec<LevelRef> {
        if !self.is_logical_server() || self.native_lifecycle.epoch().is_none() {
            return Vec::new();
        }
        self.loaded_dims()
            .into_iter()
            .filter_map(|dim| self.level(dim.id()))
            .collect()
    }
    /// All lookup members of the active server level, without spatial filtering.
    /// Parts are not lookup members. Refuses remote, stale and parked levels.
    pub(super) fn level_all_entities(&self, level: LevelRef) -> Option<Vec<EntityRef>> {
        if self.block_entity_tick_items.is_some() {
            return self.scoped_level_all_entities(level);
        }
        self.unscoped_level_all_entities(level)
    }
    /// Entities whose bounding boxes intersect this box (`Level.getEntities`).
    /// Face-only contact does not intersect; spectator players are excluded.
    /// Returns None for a stale handle or a parked level: its entity handles
    /// cannot resolve until the level is in place, and for a bound that is not
    /// finite (NaN or infinite). A minimum above its maximum is the same box
    /// with the two swapped, as vanilla's `AABB`. On a client, see
    /// `client_entities_in`. The walk
    /// is in entity-list order followed by players; each overlap requires a
    /// handle lookup, so cost grows with the box's contents. Tridents cannot
    /// yet resolve; frames, paintings and other special entities
    /// are not on the entity-list walk. The dragon follows ordinary entities;
    /// its eight intersecting parts follow players (`Level.getEntities`).
    /// During `hears_projectile_impacts`, the projectile list is taken out of
    /// the game and is not visible; it is restored for `projectile_impact`.
    pub(super) fn level_entities_in(
        &self,
        level: LevelRef,
        min: [f64; 3],
        max: [f64; 3],
    ) -> Option<Vec<EntityRef>> {
        if !self.level_handle_current(level) || !min.iter().chain(&max).all(|v| v.is_finite()) {
            return None;
        }
        let bounds = Aabb::new(
            Vec3::new(min[0], min[1], min[2]),
            Vec3::new(max[0], max[1], max[2]),
        );
        if !self.is_logical_server() {
            return self.client_entities_in(level, bounds);
        }
        if level.dimension() != self.dim.key() {
            return None;
        }
        let mut ids = self.listed_entities_in(bounds);
        self.append_dragon_query_ids(bounds, false, &mut ids);
        ids.extend(
            self.fishing_hooks_here()
                .filter(|(hook, _)| bounds.intersects(hook.bounds()))
                .map(|(hook, _)| hook.id()),
        );
        for guest in self.lan.iter().flat_map(|h| &h.hub.guests) {
            if !guest.participates_in_player_simulation()
                || !guest.placed
                || guest.dimension != level.dimension_id()
            {
                continue;
            }
            let body = if self.lan_acting == Some(guest.conn)
                && let Some(player) = &self.player
            {
                if self.mode_id() == 3 {
                    continue;
                }
                player.bounds()
            } else {
                if guest.game_mode == 3 {
                    continue;
                }
                guest.bounds()
            };
            if bounds.intersects(body)
                && let Ok(id) = u32::try_from(guest.entity_id)
            {
                ids.push(id);
            }
        }
        self.append_dragon_query_ids(bounds, true, &mut ids);
        Some(
            ids.into_iter()
                .map(|id| EntityRef::new(level.epoch(), id))
                .filter(|handle| self.native_entity_target(*handle).is_some())
                .collect(),
        )
    }
    /// The entity lists' entities whose boxes meet `bounds`, in list order.
    fn listed_entities_in(&self, bounds: Aabb) -> Vec<u32> {
        let mut ids = Vec::new();
        for_each_entity!(entity_lists_of!(self), entity => {
            // The join flush assigns zero IDs later; a handle could alias another entity.
            if entity.id() != 0 && !entity.is_removed() && bounds.intersects(entity.bounds()) {
                ids.push(entity.id());
            }
        });
        ids
    }
    /// `ClientLevel.getEntities` for `level_entities_in`: the entities the server
    /// tracks to this client in entity-list order, other players among them (a
    /// tracked player has a client copy, so its handle resolves), the dragon and
    /// then the window's own player unless it spectates, then the dragon's parts.
    /// None before login or for a level the client does not show.
    fn client_entities_in(&self, level: LevelRef, bounds: Aabb) -> Option<Vec<EntityRef>> {
        let remote = self.remote.as_ref().filter(|r| r.logged_in)?;
        if level.dimension() != self.native_level_key() {
            return None;
        }
        let mut ids = self.listed_entities_in(bounds);
        self.append_dragon_query_ids(bounds, false, &mut ids);
        if self.mode_id() != 3
            && let Some(player) = &self.player
            && bounds.intersects(player.bounds())
            && let Ok(id) = u32::try_from(remote.entity_id)
        {
            ids.push(id);
        }
        self.append_dragon_query_ids(bounds, true, &mut ids);
        Some(
            ids.into_iter()
                .map(|id| EntityRef::new(level.epoch(), id))
                .filter(|handle| self.native_entity_read_target(*handle).is_some())
                .collect(),
        )
    }
    /// Every entity the segment `from`..`to` passes through, nearest first, each box grown by
    /// `margin` (`ProjectileUtil.getEntityHitResult`'s test, without its filter: the caller
    /// takes the first hit it accepts). `exclude` is skipped. None for a bad margin or point,
    /// and where `level_entities_in` is None.
    pub(super) fn level_ray_trace_entities(
        &self,
        level: LevelRef,
        from: [f64; 3],
        to: [f64; 3],
        margin: f64,
        exclude: Option<EntityRef>,
    ) -> Option<Vec<EntityRayHit>> {
        if !(margin.is_finite() && margin >= 0.0 && from.iter().chain(&to).all(|v| v.is_finite())) {
            return None;
        }
        // Every box the grown test can hit meets the segment's own box grown the same way.
        let min = std::array::from_fn(|i| from[i].min(to[i]) - margin);
        let max = std::array::from_fn(|i| from[i].max(to[i]) + margin);
        let candidates = self.level_entities_in(level, min, max)?;
        Some(entity_ray_hits(
            from,
            to,
            margin,
            candidates
                .into_iter()
                .filter(|entity| Some(*entity) != exclude)
                .filter_map(|entity| Some((entity, self.entity_bounding_box(entity)?))),
        ))
    }
    /// What the block at `pos` would drop (`Block.getDrops`) with `tool` and `entity` as the
    /// breaker, without breaking it: the level's `block_drops_of`, which draws from the random
    /// as vanilla's roll does. None for a refused write, a stale or parked handle, a client, an
    /// unloaded or out-of-height position, and an `entity` that names nothing.
    pub(super) fn level_block_drops(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        tool: Option<&ItemStack>,
        entity: Option<EntityRef>,
    ) -> Option<Vec<ItemStack>> {
        if self.entity_ids_exhausted() {
            return None;
        }
        if !self.native_level_write_allowed(level, pos) || self.dim.key() != level.dimension() {
            return None;
        }
        if let Some(entity) = entity {
            self.native_entity_target(entity)?;
        }
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let at = horizon_core::position::BlockPos::new(pos[0], pos[1], pos[2]);
        Some(self.with_level(|_, level, _| level.block_drops_of(at, state, tool, entity)))
    }
    /// One roll of the world's loot table `table` (`LootTable.getRandomItems`) with `origin` as
    /// ORIGIN, `tool` as TOOL and `entity` as THIS_ENTITY when given: the context
    /// `/loot … fish <table> <pos> <tool>` gives. It draws from the table's random sequence,
    /// else the level random, and the stacks come split to their stack sizes. None for a
    /// missing table, a stale handle or one of a parked level, a client, and an `entity` that
    /// names nothing.
    pub(super) fn level_roll_loot(
        &mut self,
        level: LevelRef,
        table: &str,
        origin: [f64; 3],
        tool: Option<&ItemStack>,
        entity: Option<EntityRef>,
    ) -> Option<Vec<ItemStack>> {
        if !self.is_logical_server()
            || !self.level_handle_current(level)
            || self.dim.key() != level.dimension()
            || !world_items!(self).has_loot_table(table)
        {
            return None;
        }
        if let Some(entity) = entity {
            self.native_entity_target(entity)?;
        }
        let place = self.loot_place_at(origin, None);
        horizon_world::loot::with_native_loot_place(place, || {
            let this_entity = entity.is_some();
            match world_items!(self).loot_sequence(table) {
                Some(sequence) => {
                    let random = self.rand_sequences.get(&sequence, self.level.seed);
                    world_items!(self).roll_loot_table(table, tool, this_entity, random)
                }
                None => world_items!(self).roll_loot_table(
                    table,
                    tool,
                    this_entity,
                    &mut self.world.random,
                ),
            }
        })
    }
    /// Set a block through Level.setBlock. Typed errors distinguish stale/unavailable
    /// levels; Ok(false) preserves vanilla's position/no-change refusal.
    pub(super) fn level_set_block(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
        flags: BlockUpdateFlags,
    ) -> Result<bool, mod_api::LevelWriteError> {
        if !self.native_level_write_position(level, pos)? {
            return Ok(false);
        }
        Ok(self.native_set_block(level.dimension_id(), pos, state, flags.bits()))
    }
    /// Remove a block in place (`Level.removeBlock`), preserving its fluid.
    pub(super) fn level_remove_block(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        moved: bool,
    ) -> bool {
        if !self.native_level_write_allowed(level, pos) || self.dim.key() != level.dimension() {
            return false;
        }
        let changed =
            self.with_level(|_, level, ticks| level.remove_block_write(ticks, pos, moved));
        let _ = self.process_level_events();
        changed
    }
    /// Tell the neighbours of `pos` that a block changed there, as vanilla's own writes do:
    /// `Neighbours` (`Level.updateNeighborsAt`: a lever's or a button's own block, and the one
    /// it is attached to, when the signal it gives changes), `NeighboursExcept(direction)`
    /// (`updateNeighborsAtExceptFromFacing`: a diode's front) or `OutputSignal`
    /// (`updateNeighbourForOutputSignal`: the comparators beside it, when its comparator output
    /// changes without its state changing). `source` is the state whose block vanilla passes as
    /// the one that changed. False, with nothing done, off the level in place, for a position
    /// that isn't loaded, or for a `source` that is no block state.
    pub(super) fn level_update_neighbors(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        source: BlockStateId,
        update: NeighbourUpdate,
    ) -> Result<bool, mod_api::LevelWriteError> {
        if !self.native_level_write_position(level, pos)? {
            return Ok(false);
        }
        let at = horizon_core::position::BlockPos::new(pos[0], pos[1], pos[2]);
        self.with_level(|b, level, ticks| {
            b.neighbour_update(level, ticks, at, b.block(source), update)
        });
        let _ = self.process_level_events();
        Ok(true)
    }
    /// Tell the block at `pos` that `source`'s block beside it changed (`Level.neighborChanged`),
    /// as a diode tells the block in front of it before updating that block's neighbours. False,
    /// with nothing done, as for `update_neighbors`: off the level in place, for a position that
    /// isn't loaded, or for a `source` that is no block state.
    pub(super) fn level_neighbor_changed(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        source: BlockStateId,
    ) -> Result<bool, mod_api::LevelWriteError> {
        if !self.native_level_write_position(level, pos)? {
            return Ok(false);
        }
        let at = horizon_core::position::BlockPos::new(pos[0], pos[1], pos[2]);
        self.with_level(|b, level, ticks| b.neighbor_changed_at(level, ticks, at, b.block(source)));
        let _ = self.process_level_events();
        Ok(true)
    }
    /// Destroy a block in the level in place (`Level.destroyBlock`), optionally dropping
    /// resources with `tool` (None is an empty hand). With `collect`, return the dropped
    /// stacks without spawning their item entities; otherwise return an empty list after
    /// spawning them. The one level drop path runs loot, block-drop hooks and experience
    /// once, in vanilla's random order. `drop` false suppresses resources entirely.
    /// None means nothing broke: air, a refused write, a stale handle, a client, an
    /// unloaded/out-of-height position, or a parked level.
    pub(super) fn level_destroy_block(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        drop: bool,
        tool: Option<&ItemStack>,
        collect: bool,
    ) -> Option<Vec<ItemStack>> {
        if !self.native_level_write_allowed(level, pos) || self.dim.key() != level.dimension() {
            return None;
        }
        let at = horizon_core::position::BlockPos::new(pos[0], pos[1], pos[2]);
        let (changed, drops) = self.with_level(|b, level, ticks| {
            level.drop_tool = Some(((pos[0], pos[1], pos[2]), tool.cloned()));
            level.drop_into = collect.then(Vec::new);
            let changed = b.destroy_block(level, ticks, at, drop);
            (changed, level.drop_into.take().unwrap_or_default())
        });
        let _ = self.process_level_events();
        changed.then_some(drops)
    }

    /// Send `ServerLevel.destroyBlockProgress` in this level: stages 0..9 show a crack,
    /// any other stage removes it. A machine uses a stable negative breaker id.
    pub(super) fn level_set_break_progress(
        &mut self,
        level: LevelRef,
        breaker: i32,
        pos: [i32; 3],
        progress: i32,
    ) -> bool {
        if !self.native_level_write_allowed(level, pos) || self.dim.key() != level.dimension() {
            return false;
        }
        self.after_commit(After::BreakProgress(breaker, pos, progress));
        true
    }
    /// `Level.sendBlockUpdated`: queue the block at `pos` to be sent to the clients again.
    pub(super) fn level_send_block_updated(&mut self, level: LevelRef, pos: [i32; 3]) -> bool {
        if !self.native_level_write_allowed(level, pos) || self.dim.key() != level.dimension() {
            return false;
        }
        self.after_commit(After::NotifyClients((pos[0], pos[1], pos[2])));
        true
    }
    /// `LevelRenderer.destroyBlockProgress` for this client's own view only.
    pub(super) fn level_set_client_break_progress(
        &mut self,
        level: LevelRef,
        breaker: i32,
        pos: [i32; 3],
        progress: i32,
    ) -> bool {
        if self.is_logical_server() || self.native_level_read_world(level).is_none() {
            return false;
        }
        self.destroy_block_progress(breaker, pos, progress);
        true
    }
    /// `ItemStack.getDestroySpeed` of `tool` on the block at `pos`.
    pub(super) fn level_block_break_speed(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        tool: Option<&ItemStack>,
    ) -> Option<f32> {
        let world = self.native_level_read_world(level)?;
        if !world.loaded(pos[0], pos[2]) || !world.shape.contains_y(pos[1]) {
            return None;
        }
        let state = world.block(pos[0], pos[1], pos[2]);
        let name = &self.res.facts().get(state).name;
        Some(self.items().mining_speed(tool, name))
    }
    /// Apply bone meal to the block at `pos` as a dispenser does (`BoneMealItem.growCrop`, then
    /// `growWaterPlant`): the mods' `bonemeal` hook first, level event 1505 when something grew.
    /// False for a refused write, a stale or parked handle, a client, and a block that did not
    /// grow.
    pub(super) fn level_bone_meal(&mut self, level: LevelRef, pos: [i32; 3]) -> bool {
        if !self.native_level_write_allowed(level, pos) || self.dim.key() != level.dimension() {
            return false;
        }
        let at = horizon_core::position::BlockPos::new(pos[0], pos[1], pos[2]);
        let mut stack = horizon_client::items::ItemStack::new("minecraft:bone_meal", 1);
        let ask = self.bonemeal(at, false, &mut stack);
        let grown = self.with_level(|b, level, ticks| {
            ask.crop
                .unwrap_or_else(|| b.grow_crop(level, ticks, at, ask.check))
                || b.grow_water_plant(level, ticks, at, None)
        });
        if grown {
            self.level_events
                .push(horizon_world::level::LevelEvent::LevelEvent {
                    id: 1505,
                    pos: at,
                    data: 15,
                    source: None,
                });
        }
        let _ = self.process_level_events();
        grown
    }
    /// Set the biome of the cell holding `pos` to `biome` (`/fillbiome` for one cell). False for a
    /// refused write, a stale or parked handle, a client, an unknown biome and an unchanged cell.
    pub(super) fn level_set_biome(&mut self, level: LevelRef, pos: [i32; 3], biome: &str) -> bool {
        if !self.native_level_write_allowed(level, pos) || self.dim.key() != level.dimension() {
            return false;
        }
        self.set_biome_at(pos, biome)
    }
    /// One residency boundary for state-bearing block writes; position refusal is vanilla.
    fn native_level_write_position(
        &self,
        level: LevelRef,
        pos: [i32; 3],
    ) -> Result<bool, mod_api::LevelWriteError> {
        let world = self.native_level_world_result(level)?;
        if self.dim.key() != level.dimension() {
            return Err(mod_api::LevelWriteError::Unavailable);
        }
        Ok(!self.world.carrier_fences.locks_cell(&pos)
            && world.loaded(pos[0], pos[2])
            && world.shape.contains_y(pos[1]))
    }
    pub(crate) fn native_level_write_allowed(&self, level: LevelRef, pos: [i32; 3]) -> bool {
        // A carrier transfer fence holds this cell: every mod write
        // through the level gate refuses while the operation runs.
        !self.world.carrier_fences.locks_cell(&pos)
            && self.is_logical_server()
            && self.level_handle_current(level)
            && self.world.loaded(pos[0], pos[2])
            && self.world.shape.contains_y(pos[1])
    }
    /// The supplied state's block-local outline or collision boxes at a level
    /// position with an empty context. The world is never modified. Returns None
    /// on a client, for a stale handle or for a closed level. Unloaded neighbors
    /// read as air; collision errors return no boxes, like `block_collision`.
    pub(super) fn level_block_shape(
        &self,
        level: LevelRef,
        state: BlockStateId,
        pos: [i32; 3],
        kind: BlockShapeKind,
    ) -> Option<Vec<[f64; 6]>> {
        if !self.is_logical_server() || !self.level_handle_current(level) {
            return None;
        }
        let (world, anims) = if self.dim.key() == level.dimension() {
            (&self.world, &self.be_anims)
        } else {
            let dim = self
                .levels
                .parked_dims()
                .into_iter()
                .find(|dim| dim.key() == level.dimension())?;
            (
                self.levels.parked_world(dim)?,
                self.levels.parked_anims(dim)?,
            )
        };
        Some(self.state_shape_boxes(world, anims, state, pos, kind))
    }

    /// Gameplay's face support in the current or a parked server level, including
    /// live block entities for dynamic shapes. None for stale or closed levels,
    /// unloaded blocks, unsupported states or a shape query error.
    pub(super) fn level_block_face_sturdy(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        face: BlockFace,
        support: mod_api::SupportType,
    ) -> Option<bool> {
        let state = self.level_block_state(level, pos)?;
        let direction = block_face_direction(face);
        let b = self.res.behaviours();
        if !b.has_cached_support(state) {
            use horizon_client::collisions::SceneCollisions;
            use horizon_world::{block_support::SupportType, dynamic_support::DynamicSupport};
            let binding = self.res.collision();
            let dynamic = DynamicSupport::from_state(binding.registry().state(state)?).ok()??;
            let world = self.native_level_world(level)?;
            let mut scene = SceneCollisions {
                world,
                binding,
                subject: None,
            };
            let support = match support {
                mod_api::SupportType::Full => SupportType::Full,
                mod_api::SupportType::Center => SupportType::Center,
                mod_api::SupportType::Rigid => SupportType::Rigid,
                _ => unreachable!("mod_api::SupportType: unhandled variant"),
            };
            return dynamic
                .is_face_sturdy(&mut scene, pos, direction, support)
                .ok();
        }
        Some(match support {
            mod_api::SupportType::Full => b.is_face_sturdy(state, direction),
            mod_api::SupportType::Center => b.is_face_sturdy_center(state, direction),
            mod_api::SupportType::Rigid => b.is_face_sturdy_rigid(state, direction),
            _ => unreachable!("mod_api::SupportType: unhandled variant"),
        })
    }

    /// Clip an empty-context ray against collider or outline shapes, without fluids, in the
    /// world `native_level_view_world` reads. None on failed reads, dynamic shapes, invalid
    /// rays or traversal limits.
    pub(super) fn level_clip(
        &self,
        level: LevelRef,
        from: [f64; 3],
        to: [f64; 3],
        blocks: ClipBlock,
        fluids: ClipFluid,
    ) -> Option<ClipHit> {
        use horizon_client::collisions::SceneCollisions;
        use horizon_core::geometry::Direction;
        use horizon_world::{
            collision_context::CollisionContext,
            world_ray::{RayBlockMode, RayFluidMode, clip_world},
        };
        let world = self.native_level_view_world(level)?;
        let mut rays = super::explosion::ColliderRays {
            scene: SceneCollisions {
                world,
                binding: self.collision,
                subject: None,
            },
            context: CollisionContext::empty(false),
            clip_data: Some(super::explosion::ClipData {
                facts: self.res.facts(),
                behaviours: self.res.behaviours(),
                tables: &self.pack.context.tables,
                registry: self.pack.assets.registry,
            }),
        };
        let mode = match blocks {
            ClipBlock::Collider => RayBlockMode::Collider,
            ClipBlock::Outline => RayBlockMode::Outline,
            _ => unreachable!("ClipBlock: unhandled variant"),
        };
        let fluid = match fluids {
            ClipFluid::None => RayFluidMode::None,
            _ => unreachable!("ClipFluid: unhandled variant"),
        };
        let result = clip_world(
            &mut rays,
            Vec3::new(from[0], from[1], from[2]),
            Vec3::new(to[0], to[1], to[2]),
            mode,
            fluid,
            4096,
        )
        .ok()?;
        let h = result.hit;
        let face = match h.direction {
            Direction::Down => BlockFace::Down,
            Direction::Up => BlockFace::Up,
            Direction::North => BlockFace::North,
            Direction::South => BlockFace::South,
            Direction::West => BlockFace::West,
            Direction::East => BlockFace::East,
        };
        Some(ClipHit {
            miss: result.miss,
            block: [h.block.x, h.block.y, h.block.z],
            face,
            location: [h.location.x, h.location.y, h.location.z],
            inside: h.inside,
            world_border: false,
        })
    }

    /// The entity's position (`Entity.position`): its feet, the middle of
    /// its box for a painting or an item frame.
    pub(super) fn entity_position(&self, entity: EntityRef) -> Option<[f64; 3]> {
        if self.native_lifecycle.epoch() == Some(entity.epoch())
            && let Some((id, position)) = self.native_lifecycle.entity_sections.write_position
            && id == entity.id()
        {
            return Some(position);
        }
        match self.native_entity_read_target(entity) {
            Some(target) => self.target_position(target),
            None => self
                .native_client_tracked(entity)
                .map(|(_, position)| position),
        }
    }
    /// The block the entity's position is in (`Entity.blockPosition`). API
    /// 373.
    pub(super) fn entity_block_position(&self, entity: EntityRef) -> Option<[i32; 3]> {
        self.entity_position(entity)
            .map(|p| p.map(|v| v.floor() as i32))
    }
    /// The entity's yaw and pitch in degrees (`Entity.getYRot`, `getXRot`); 0
    /// for the kinds that don't turn. API 373.
    pub(super) fn entity_rotation(&self, entity: EntityRef) -> Option<[f32; 2]> {
        Some(self.target_rotation(self.native_entity_read_target(entity)?))
    }
    /// The entity's type (`Entity.getType`): `minecraft:zombie`. API
    /// 373.
    pub(super) fn entity_kind(&self, entity: EntityRef) -> Option<mod_api::EntityTypeKey> {
        match self.native_entity_read_target(entity) {
            Some(target) => self.native_entity_kind_key(target),
            None => {
                mod_api::__private::resolved_entity_type_key(self.native_client_tracked(entity)?.0)
            }
        }
    }
    /// The entity's UUID (`Entity.getUUID`), the one it is saved with; an
    /// entity that has none yet gets it now, as a new vanilla entity does.
    /// On a client, the one the server added it with (a player's is its
    /// profile's). API 373.
    pub(super) fn entity_uuid(&mut self, entity: EntityRef) -> Option<u128> {
        if let Some(r) = &self.remote {
            let id = i32::try_from(entity.id()).ok()?;
            if self.native_entity_read_target(entity).is_none()
                && self.native_client_tracked(entity).is_none()
            {
                return None;
            }
            if let Some(words) =
                self.known_resident_entity_uuid_at(self.native_level_key(), entity.id())
            {
                return Some(uuid_from_words(words));
            }
            if let Some((uuid, _)) = r.level_player(id) {
                return Some(uuid);
            }
            return self
                .persist
                .uuids
                .get(&entity.id())
                .copied()
                .map(uuid_from_words);
        }
        let words = match self.native_entity_target(entity)? {
            Target::Player => {
                let dimension = self.current_player_resident_location()?.owner.dimension;
                return self
                    .known_resident_entity_uuid_at(dimension, entity.id())
                    .map(uuid_from_words);
            }
            Target::Guest(conn) => return self.command_guest(conn).map(|g| g.uuid),
            _ => self.uuid_for(entity.id()),
        };
        Some(uuid_from_words(words))
    }
    /// The level the entity is in (`Entity.level`): on a client, the
    /// client's level. API 373.
    pub(super) fn entity_level(&self, entity: EntityRef) -> Option<LevelRef> {
        let key = match self.native_entity_read_target(entity) {
            Some(t) if self.is_logical_server() => self.target_level(t).key(),
            Some(_) => self.native_level_key(),
            None => self
                .native_client_tracked(entity)
                .map(|_| self.native_level_key())?,
        };
        Some(self.level_ref(entity.epoch(), key))
    }
}

/// A saved UUID's four ints (`UUIDUtil.uuidFromIntArray`) as one number.
fn uuid_from_words(words: [i32; 4]) -> u128 {
    words
        .iter()
        .fold(0, |uuid, w| (uuid << 32) | u128::from(*w as u32))
}

include!("native_entity_refs/tick_item_read.rs");
include!("native_entity_refs/tick_item_selector.rs");

#[cfg(test)]
mod ray_tests {
    use super::*;

    fn entity(id: u32) -> EntityRef {
        EntityRef::new(mod_api::WorldEpoch::new(1), id)
    }

    /// A unit box with its lowest corner at `x` on the x axis, around the ray along x.
    fn at(x: f64) -> [[f64; 3]; 2] {
        [[x, -0.5, -0.5], [x + 1.0, 0.5, 0.5]]
    }

    #[test]
    fn hits_come_nearest_first_where_the_ray_enters_each_box() {
        let hits = entity_ray_hits(
            [0.0; 3],
            [10.0, 0.0, 0.0],
            0.0,
            [
                (entity(1), at(6.0)),
                (entity(2), at(2.0)),
                (entity(3), at(20.0)),
            ],
        );
        let order: Vec<_> = hits.iter().map(|h| h.entity).collect();
        assert_eq!(
            order,
            [entity(2), entity(1)],
            "the box past the segment's end is not hit"
        );
        for (hit, x) in hits.iter().zip([2.0, 6.0]) {
            assert!((hit.location[0] - x).abs() < 1e-9 && hit.location[1] == 0.0);
            assert!((hit.distance - x).abs() < 1e-9);
        }
    }

    #[test]
    fn the_margin_grows_every_box_and_a_start_inside_one_misses_it() {
        // 0.3 above the ray: missed without a margin, hit with one.
        let above = [[3.0, 0.8, -0.5], [4.0, 1.8, 0.5]];
        let to = [10.0, 0.0, 0.0];
        assert!(entity_ray_hits([0.0; 3], to, 0.0, [(entity(1), above)]).is_empty());
        let grown = entity_ray_hits([0.0; 3], to, 1.0, [(entity(1), above)]);
        assert_eq!(grown.len(), 1);
        assert!(
            (grown[0].location[0] - 2.0).abs() < 1e-9,
            "entered at the grown box's face"
        );
        // `AABB.clip` from inside finds no entry, as for a projectile still in its shooter.
        assert!(entity_ray_hits([0.5, 0.0, 0.0], to, 0.0, [(entity(1), at(0.0))]).is_empty());
    }
}

#[cfg(test)]
pub(super) mod tests {
    /// Keep guest setup and calls together, preserving libtest identity and panic payloads.
    pub(in crate::game) use horizon_guest_stack::run_on_guest_stack;

    /// Resolve a fixture after registration or a late Parts reinstall from its loaded table.
    pub(in crate::game) fn resolve_parts(game: &super::Game<'_>) -> Result<(), String> {
        crate::game::api_values::resolve_block_definitions(game.pack.assets.registry)?;
        mod_api::__private::resolve_items(game.items().registry_order.iter().map(|id| &**id))
            .map_err(|errors| format!("fixture item keys refused: {errors:?}"))?;
        game.resolve_parts().map_err(|error| error.to_string())
    }
    use crate::game::api_values::TryIntoApi;
    use mod_api::__private::game::*;
    /// Isolate declarations and frozen registries without changing another test's resources.
    pub(in crate::game) fn run_fresh_cases(name: &str, flag: &str, cases: &[&str]) -> bool {
        run_fresh_cases_at_once(name, flag, cases, 1)
    }

    /// [`run_fresh_cases`] with up to `at_once` children running together, for a test that runs
    /// alone in its job (the wasm fixture's): each child still has its own process and folders.
    pub(in crate::game) fn run_fresh_cases_at_once(
        name: &str,
        flag: &str,
        cases: &[&str],
        at_once: usize,
    ) -> bool {
        if std::env::var_os(flag).is_some() {
            return false;
        }
        let resources =
            std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap())
                .canonicalize()
                .unwrap();
        let scratch_owner = tempfile::tempdir().unwrap();
        let target = scratch_owner.path();
        let run = |case: &str| {
            let scratch = target.join(format!("fresh-case-{case}"));
            std::fs::create_dir(&scratch).unwrap();
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--ignored",
                    "--exact",
                    name,
                    "--nocapture",
                    "--test-threads=4",
                ])
                .current_dir(&scratch)
                .env(flag, case)
                .env("HORIZON_TEST_RESOURCES", &resources)
                .env("HORIZON_NO_AUDIO", "1")
                .env("HORIZON_LAUNCHER_NO_SELF_UPDATE", "1");
            for key in [
                "HORIZON_HOME",
                "HORIZON_LAUNCHER_HOME",
                "XDG_DATA_HOME",
                "XDG_CONFIG_HOME",
                "XDG_RUNTIME_DIR",
                "HORIZON_LAUNCHER_TRASH",
                "HORIZON_LAUNCHER_CLIPBOARD",
                "HORIZON_LAUNCHER_DESKTOP",
            ] {
                let home = scratch.join(key);
                std::fs::create_dir(&home).unwrap();
                command.env(key, home);
            }
            let output = output_within(&mut command, name, case);
            std::fs::remove_dir_all(scratch).unwrap();
            output
        };
        for group in cases.chunks(at_once.max(1)) {
            let run = &run;
            let outputs: Vec<_> = std::thread::scope(|scope| {
                let running: Vec<_> = group
                    .iter()
                    .map(|&case| scope.spawn(move || run(case)))
                    .collect();
                running
                    .into_iter()
                    .map(|child| child.join().unwrap())
                    .collect()
            });
            for (case, output) in group.iter().zip(outputs) {
                let stdout = String::from_utf8_lossy(&output.stdout);
                // The child's log, captured with this test's own output.
                print!("{stdout}");
                eprint!("{}", String::from_utf8_lossy(&output.stderr));
                assert!(output.status.success(), "{name}: {case}");
                assert!(
                    ran_one_test(&stdout),
                    "{name}: {case}: the child matched no test of that name"
                );
            }
        }
        true
    }

    /// The longest a fresh child may run: a hang in its startup or shutdown fails the parent
    /// with the child killed, instead of waiting for the whole test run's own limit.
    const FRESH_CASE_LIMIT: std::time::Duration = std::time::Duration::from_secs(900);

    /// `command`'s output, killing the child if it outlives [`FRESH_CASE_LIMIT`].
    fn output_within(
        command: &mut std::process::Command,
        name: &str,
        case: &str,
    ) -> std::process::Output {
        use std::io::Read;
        use std::process::Stdio;
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let out = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stdout.read_to_end(&mut bytes);
            bytes
        });
        let err = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stderr.read_to_end(&mut bytes);
            bytes
        });
        let deadline = std::time::Instant::now() + FRESH_CASE_LIMIT;
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("{name}: {case}: the child did not finish within {FRESH_CASE_LIMIT:?}");
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        std::process::Output {
            status,
            stdout: out.join().unwrap(),
            stderr: err.join().unwrap(),
        }
    }

    /// Whether libtest's summary says exactly one test passed: `--exact` with a name no test
    /// has runs nothing and still exits 0.
    fn ran_one_test(stdout: &str) -> bool {
        stdout
            .lines()
            .any(|line| line.starts_with("test result: ok. 1 passed;"))
    }

    /// A child run with a name no test has fails the parent instead of passing vacuously.
    #[test]
    #[ignore = "runs in an isolated scratch process"]
    #[should_panic(expected = "matched no test of that name")]
    fn a_fresh_case_whose_name_matches_no_test_fails() {
        run_fresh_cases(
            "game::native_entity_refs::tests::no_test_has_this_name",
            "HORIZON_NO_MATCH_CHILD",
            &["missing"],
        );
    }

    /// The parent runs and checks an isolated child; the child loads the Part fixtures' types.
    pub(in crate::game) fn declared_part_root(name: &str) -> Option<std::path::PathBuf> {
        if run_fresh_cases(name, "HORIZON_DECLARED_PART_GAME_CHILD", &["valid"]) {
            return None;
        }
        let _mods = mod_platform::__private::part_tests::game_thread();
        let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let root = super::super::mods::content_root(&base, &Default::default())
            .unwrap()
            .prepared()
            .to_path_buf();
        // game_thread installs a small synthetic key table for API-only tests.
        // This fixture loads a real resource graph: resolve its complete keys first.
        let (registry, _) = horizon_world::level::data::load_registry_with_flags(&root).unwrap();
        let ids: std::collections::HashSet<_> = (0..registry.len())
            .map(|i| {
                registry
                    .state(horizon_world::registry::BlockStateId(i as u32))
                    .unwrap()
                    .name
                    .clone()
            })
            .collect();
        crate::game::api_values::register_game_keys(ids);
        Some(root)
    }

    pub(in crate::game) fn prepare_entity_ticking_neighborhood(
        game: &mut super::Game<'_>,
        center: horizon_core::position::ChunkPos,
    ) {
        for z in center.z - 2..=center.z + 2 {
            for x in center.x - 2..=center.x + 2 {
                if !game.world.chunks.contains_key(&(x, z)) {
                    game.world.chunks.insert(
                        (x, z),
                        std::sync::Arc::new(horizon_client::chunk::Chunk::empty(x, z)),
                    );
                    game.block_ticks
                        .add_empty_chunk(horizon_core::position::ChunkPos::new(x, z))
                        .unwrap();
                }
            }
        }
    }

    /// Bind the fixture body's canonical identity before configuring its acting context.
    pub(in crate::game) fn bind_local_player(game: &mut super::Game<'_>, id: u32, uuid: u128) {
        game.next_entity = game.next_entity.max(id);
        game.player.as_mut().expect("fixture player body").entity_id =
            i32::try_from(id).expect("fixture entity ID fits the protocol");
        game.level.player_uuid = Some(horizon_client::lan::playerdata::uuid_ints(uuid));
        game.bind_local_player_residency();
        game.lan_acting_entity = Some(id);
        game.lan_acting_uuid = Some(uuid);
    }

    /// Install the server record backing a fixture's already configured acting body.
    pub(in crate::game) fn bind_acting_guest(game: &mut super::Game<'_>) {
        use horizon_client::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
        use horizon_protocol::packets::configuration::ClientInformation;
        use std::sync::{
            Arc,
            atomic::{AtomicI32, AtomicUsize},
        };
        let conn = game.lan_acting.unwrap();
        let id = game.lan_acting_entity.unwrap();
        let uuid = game.lan_acting_uuid.unwrap();
        let name = game.lan_acting_name.clone().unwrap();
        let at = game.player.as_ref().unwrap().position;
        game.next_entity = game.next_entity.max(id);
        game.lan_host(
            super::super::lan::LanSettings {
                port: 0,
                guest_commands: false,
                force_game_mode: false,
            },
            NetConfig {
                listen: false,
                announce: false,
                lod_distance: 0,
                ..NetConfig::lan(0, String::new(), 2)
            },
            HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: game.dim.id().to_owned(),
                allow_flight: true,
                seed: 0,
                max_players: 2,
                integrated: true,
                levels: game.level_shapes,
            },
        )
        .unwrap();
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn,
                uuid,
                name,
                properties: Vec::new(),
                info: ClientInformation::new(8),
                brand: String::new(),
                sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:1".parse().unwrap(),
                expected: false,
            },
            id as i32,
            [at.x, at.y, at.z],
            10,
        );
        guest.placed = true;
        game.lan.as_mut().unwrap().hub.guests.push(guest);
        game.reset_entity_residency();
    }

    pub(in crate::game) fn with_game(test: impl FnOnce(&mut super::Game<'_>)) {
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        with_game_at(root, test);
    }

    pub(in crate::game) fn with_game_at(
        root: std::path::PathBuf,
        test: impl FnOnce(&mut super::Game<'_>),
    ) {
        with_game_role(
            root,
            super::super::execution_role::ExecutionRole::IntegratedServer,
            test,
        );
    }

    /// Keep a key that a test mod's `register` declared. The game fixture registers the
    /// installed mods again (`register_game_keys`), and a key's slot is stable for the process,
    /// so the repeat must declare the same key.
    pub(in crate::game) fn record_key<K: Copy + PartialEq + std::fmt::Debug>(
        slot: &std::sync::OnceLock<K>,
        key: K,
    ) {
        assert_eq!(
            *slot.get_or_init(|| key),
            key,
            "a repeat registration declared another key"
        );
    }

    pub(in crate::game) fn with_client_game(test: impl FnOnce(&mut super::Game<'_>)) {
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        with_game_role(
            root,
            super::super::execution_role::ExecutionRole::Client,
            test,
        );
    }

    pub(in crate::game) fn with_game_role(
        root: std::path::PathBuf,
        role: super::super::execution_role::ExecutionRole,
        test: impl FnOnce(&mut super::Game<'_>),
    ) {
        use super::super::{Audio, Game, Options, Progress, Settings, load_resources};
        use mod_api::Authority;

        // Parts registration replaces its metadata snapshot. Finish registration before
        // loading the resource graph, whose registry retains one immutable worker snapshot.
        let (registry, _) = horizon_world::level::data::load_registry_with_flags(&root).unwrap();
        let ids: std::collections::HashSet<_> = (0..registry.len())
            .map(|i| {
                registry
                    .state(horizon_world::registry::BlockStateId(i as u32))
                    .unwrap()
                    .name
                    .clone()
            })
            .collect();
        crate::game::api_values::register_game_keys(ids);
        // Rebuilding Parts replaces the entity-type lookup snapshot that content_root
        // completed. Publish the prepared registry order into this new fixture epoch.
        super::super::mods::register_iterated_registries(&root).unwrap();
        let saves = tempfile::tempdir().unwrap();
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            role,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = role;
        options.game_dir = Some(saves.path().to_path_buf());
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.to_path_buf(),
            )),
            None,
        )
        .unwrap();
        game.reset_entity_residency();
        game.native_lifecycle
            .begin_open(Authority::IntegratedServer);

        // A game that opened its world resolves its parts; the harness stops
        // short of the open, so do it here for the by-state part table.
        resolve_parts(&game).unwrap();
        // The title's save folder is under the working directory: give each test its own, so a
        // test that saves leaves the checkout clean and parallel tests don't share files.
        game.save = std::sync::Arc::new(horizon_client::save::SaveDir::new(
            saves.path().to_path_buf(),
            horizon_client::save::LevelShape::OVERWORLD,
        ));
        test(&mut game);
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn typed_uuid_and_nbt_reads_follow_the_entity_level() {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            // with_game prepares lifecycle, not world-open command registries.
            // execute in and summon must resolve real dimension and entity keys.
            game.build_registries();
            let words = [7, 0, 0, 3];
            let uuid = (7u128 << 96) | 3;
            // Harness commands use the console boundary; setting dedicated
            // options alone does not grant the command parser operator rights.
            assert!(game.as_console(|game| game.run_command(
                "execute in minecraft:the_nether run summon cow 8.5 70 8.5 {UUID:[I;7,0,0,3],NoAI:1b}"
            )));
            let original = game.dim;
            game.enter_command_level(super::super::Dim::Nether).unwrap();
            let id = *game
                .persist
                .uuids
                .iter()
                .find(|(_, value)| **value == words)
                .unwrap()
                .0;
            let entity = game.entity(id).unwrap();
            assert_eq!(game.entity_uuid(entity), Some(uuid));
            let nbt = game.entity_nbt(entity).unwrap();
            assert_eq!(
                nbt.get(&"NoAI".into()),
                Some(&horizon_core::nbt::Tag::Byte(1))
            );
            assert!(!nbt.contains_key(&"id".into()));
            game.enter_command_level(original).unwrap();
            assert_eq!(game.entity_nbt(entity), None);
            game.enter_command_level(super::super::Dim::Nether).unwrap();
            assert_eq!(game.entity_uuid(entity), Some(uuid));
            assert_eq!(game.entity_nbt(entity), Some(nbt));
            game.enter_command_level(original).unwrap();
            assert_eq!(game.dim, original);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn block_part_allows_weak_power_in_real_game_level() {
        use horizon_client::chunk::Chunk;
        use horizon_core::position::BlockPos;
        use std::sync::Arc;

        mod_platform::__private::part_tests::weak_power_game_thread();
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let glass = registry.default_state("minecraft:glass").unwrap();
            let source = registry
                .resolve(&horizon_world::chunk::BlockState {
                    name: "minecraft:lever".into(),
                    properties: [("face", "wall"), ("facing", "east"), ("powered", "true")]
                        .into_iter()
                        .map(|(key, value)| (key.into(), value.into()))
                        .collect(),
                })
                .unwrap();
            let pos = BlockPos::new(5, 64, 5);
            game.world.set(5, 64, 5, glass).unwrap();
            game.world.set(6, 64, 5, source).unwrap();
            let signal = game.with_level(|b, level, ticks| {
                b.best_neighbor_signal(level, ticks, BlockPos::new(pos.x - 1, pos.y, pos.z))
            });
            assert_eq!(signal, 15);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_soil_part_answers_for_the_plants_above_it_in_a_real_game_level() {
        use horizon_client::chunk::Chunk;
        use horizon_core::position::BlockPos;
        use std::sync::Arc;

        // The fixture's parts: glass sustains any plant, grass sustains none.
        mod_platform::__private::part_tests::sustain_plant_game_thread();
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let state = |name: &str| registry.default_state(name).unwrap();
            let dandelion = state("minecraft:dandelion");
            let survives = |game: &mut super::super::Game<'_>, soil: &str| {
                game.world.set(5, 64, 5, state(soil)).unwrap();
                game.with_level(|b, level, ticks| {
                    b.can_survive(level, ticks, BlockPos::new(5, 65, 5), dandelion)
                })
            };
            // Allow: vanilla refuses a dandelion on glass; the soil's part lets it stand.
            assert!(survives(game, "minecraft:glass"));
            // Deny: vanilla lets a dandelion stand on grass; the soil's part refuses it.
            assert!(!survives(game, "minecraft:grass_block"));
            // Default: soils with no part answer as vanilla's plant rule does.
            assert!(survives(game, "minecraft:dirt"));
            assert!(!survives(game, "minecraft:stone"));
            // A client's copy of the level does not ask the part.
            game.options.dedicated = None;
            assert!(!survives(game, "minecraft:glass"));
            assert!(survives(game, "minecraft:grass_block"));
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_block_part_decides_whether_the_tool_harvests() {
        use horizon_client::{chunk::Chunk, motion::PlayerMotion};
        use horizon_core::{geometry::Vec3, position::ChunkPos};
        use std::sync::Arc;

        // The fixture's parts: stone is harvested by any tool, dirt by none.
        mod_platform::__private::part_tests::harvest_game_thread();
        with_game(|game| {
            let scratch = std::env::temp_dir().join(format!(
                "horizon-harvest-part-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            std::fs::create_dir_all(&scratch).unwrap();
            game.world_root = scratch.clone();
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 62.0, 4.5)).unwrap());
            game.player.as_mut().unwrap().ground.on_ground = true;
            game.creative = false;
            game.build_registries();
            game.set_allow_commands(true);
            assert!(game.run_command("gamemode survival"));
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            let registry = game.pack.assets.registry;
            let pos = [5, 63, 4];
            // The dig's progress and the drops of a break by bare hand: (block, its hardness, what
            // vanilla drops by hand, and what the drop is). Vanilla's own rule: a block that
            // requires a tool drops nothing by hand, and the dig is 3.33 times slower for it.
            for (block, hardness, part, by_hand, drop) in [
                // Stone needs a pickaxe; the part harvests it anyway.
                (
                    "minecraft:stone",
                    1.5_f32,
                    Some(true),
                    false,
                    "minecraft:cobblestone",
                ),
                // Dirt needs none; the part refuses it.
                ("minecraft:dirt", 0.5, Some(false), true, "minecraft:dirt"),
                // No part: vanilla's rule alone, both ways.
                (
                    "minecraft:cobblestone",
                    2.0,
                    None,
                    false,
                    "minecraft:cobblestone",
                ),
                ("minecraft:grass_block", 0.6, None, true, "minecraft:dirt"),
            ] {
                let state = registry.default_state(block).unwrap();
                let harvest = !part.unwrap_or(by_hand);
                game.world.set(pos[0], pos[1], pos[2], state).unwrap();
                let divisor = if harvest { 30.0 } else { 100.0 };
                assert_eq!(
                    game.destroy_progress(pos),
                    1.0 / hardness / divisor,
                    "{block}"
                );
                game.dropped.clear();
                game.break_block(pos).unwrap();
                let dropped = game.dropped.iter().any(|item| &*item.stack.item == drop);
                assert_eq!(dropped, harvest, "{block}: drops {drop}");
            }
            std::fs::remove_dir_all(scratch).unwrap();
        });
    }

    /// A world with block `support` at (5, 64, 5) and a powered lever on its east face.
    fn lever_on(game: &mut super::super::Game<'_>, support: &str) {
        use horizon_client::chunk::Chunk;
        use std::sync::Arc;

        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let registry = game.pack.assets.registry;
        let support = registry.default_state(support).unwrap();
        let lever = registry
            .resolve(&horizon_world::chunk::BlockState {
                name: "minecraft:lever".into(),
                properties: [("face", "wall"), ("facing", "east"), ("powered", "true")]
                    .into_iter()
                    .map(|(key, value)| (key.into(), value.into()))
                    .collect(),
            })
            .unwrap();
        game.world.set(5, 64, 5, support).unwrap();
        game.world.set(6, 64, 5, lever).unwrap();
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_level_handle_reads_redstone_as_signal_getter_does() {
        use horizon_core::geometry::Direction::{East, North, West};

        with_game(|game| {
            lever_on(game, "minecraft:stone");
            let level = game.level(game.dim.id()).unwrap();
            let (stone, lever, beside, far) = ([5, 64, 5], [6, 64, 5], [4, 64, 5], [5, 64, 9]);
            // A powered lever: weak 15 every way, strong 15 only into the block it hangs on,
            // which lies the way it faces from that block (LeverBlock.getDirectSignal).
            assert_eq!(level.signal(game, lever, North), Some(15));
            assert_eq!(level.direct_signal(game, lever, East), Some(15));
            assert_eq!(level.direct_signal(game, lever, North), Some(0));
            // Stone conducts the strong power to the block west of it.
            assert_eq!(level.direct_signal_to(game, stone), Some(15));
            assert_eq!(level.signal(game, stone, West), Some(15));
            assert_eq!(level.best_neighbor_signal(game, beside), Some(15));
            assert!(level.has_neighbor_signal(game, beside));
            // Nothing gives strong power into `beside`, and nothing reaches `far`.
            assert_eq!(level.direct_signal_to(game, beside), Some(0));
            assert_eq!(level.best_neighbor_signal(game, far), Some(0));
            assert!(!level.has_neighbor_signal(game, far));
        });
        // Glass is no conductor: the lever's power stops at it.
        with_game(|game| {
            lever_on(game, "minecraft:glass");
            let level = game.level(game.dim.id()).unwrap();
            assert_eq!(level.best_neighbor_signal(game, [4, 64, 5]), Some(0));
            assert_eq!(level.signal(game, [5, 64, 5], West), Some(0));
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_level_bone_meals_a_block_and_sets_a_biome_cell() {
        use horizon_client::chunk::Chunk;
        use std::sync::Arc;

        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let farmland = registry.default_state("minecraft:farmland").unwrap();
            let wheat = registry.default_state("minecraft:wheat").unwrap();
            let stone = registry.default_state("minecraft:stone").unwrap();
            game.world.set(0, 63, 0, farmland).unwrap();
            game.world.set(0, 64, 0, wheat).unwrap();
            game.world.set(5, 64, 5, stone).unwrap();
            let level = game.level(game.dim.id()).unwrap();
            assert!(game.level_bone_meal(level, [0, 64, 0]));
            assert_ne!(game.world.block(0, 64, 0), wheat, "the crop grew");
            assert!(!game.level_bone_meal(level, [5, 64, 5]), "stone");
            assert!(!game.level_bone_meal(level, [100_000, 64, 0]), "unloaded");

            let current = usize::from(game.world.chunks[&(0, 0)].biome(0, 64, 0));
            let target = usize::from(current == 0);
            let name = world_biomes!(game).list[target].name.clone();
            assert!(game.level_set_biome(level, [0, 64, 0], &name));
            assert_eq!(
                usize::from(game.world.chunks[&(0, 0)].biome(3, 67, 3)),
                target,
                "the whole 4x4x4 cell changed"
            );
            assert!(game.world.modified.contains(&(0, 0)));
            assert!(
                !game.level_set_biome(level, [0, 64, 0], &name),
                "already set"
            );
            assert!(!game.level_set_biome(level, [0, 64, 0], "minecraft:nope"));
            assert!(!game.level_set_biome(level, [100_000, 64, 0], &name));
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_level_can_survive_asks_the_blocks_own_rule() {
        use horizon_client::chunk::Chunk;
        use std::sync::Arc;

        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let grass = registry.default_state("minecraft:grass_block").unwrap();
            let stone = registry.default_state("minecraft:stone").unwrap();
            let sapling = registry.default_state("minecraft:oak_sapling").unwrap();
            game.world.set(0, 64, 0, grass).unwrap();
            assert!(game.level_can_survive([0, 65, 0], sapling));
            game.world.set(0, 64, 0, stone).unwrap();
            assert!(!game.level_can_survive([0, 65, 0], sapling));
            assert!(
                !game
                    .level_can_survive([0, 65, 0], horizon_world::registry::BlockStateId(u32::MAX))
            );
            assert!(!game.level_can_survive([100, 65, 100], sapling));
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_level_handle_updates_neighbours_as_vanillas_writes_do() {
        use horizon_core::geometry::Direction::East;
        use mod_api::NeighbourUpdate::{Neighbours, NeighboursExcept, OutputSignal};

        let lit = |game: &super::super::Game<'_>, [x, y, z]: [i32; 3]| {
            let state = game.world.block(x, y, z);
            game.block_state_property(state, "lit") == Some("true")
        };
        let set = |game: &mut super::super::Game<'_>, [x, y, z]: [i32; 3], name: &str| {
            let state = game.pack.assets.registry.default_state(name).unwrap();
            game.world.set(x, y, z, state).unwrap();
        };
        // The powered lever at (6, 64, 5) on stone; unlit lamps beside it, east and above,
        // placed without updates, so they don't know it powers them yet.
        let (lever, east, above) = ([6, 64, 5], [7, 64, 5], [6, 65, 5]);
        with_game(|game| {
            lever_on(game, "minecraft:stone");
            let level = game.level(game.dim.id()).unwrap();
            let state = game.world.block(6, 64, 5);
            set(game, east, "minecraft:redstone_lamp");
            set(game, above, "minecraft:redstone_lamp");
            // `updateNeighborsAtExceptFromFacing(lever, East)`: every neighbour but the east one.
            assert_eq!(
                level.update_neighbors(
                    game,
                    lever,
                    state.try_into_api().unwrap(),
                    NeighboursExcept(East)
                ),
                Ok(true)
            );
            assert!(lit(game, above) && !lit(game, east));
            // `updateNeighborsAt(lever)`: the east lamp hears it too.
            assert_eq!(
                level.update_neighbors(game, lever, state.try_into_api().unwrap(), Neighbours),
                Ok(true)
            );
            assert!(lit(game, east));
        });
        // `Level.neighborChanged(east, lever)`: only that lamp hears.
        with_game(|game| {
            lever_on(game, "minecraft:stone");
            let level = game.level(game.dim.id()).unwrap();
            let state = game.world.block(6, 64, 5);
            set(game, east, "minecraft:redstone_lamp");
            set(game, above, "minecraft:redstone_lamp");
            assert_eq!(
                level.neighbor_changed(game, east, state.try_into_api().unwrap()),
                Ok(true)
            );
            assert!(lit(game, east) && !lit(game, above));
        });
        // A repeater in front of the lever, reading it, schedules its own tick when told (a
        // diode's `checkTickOnNeighbor`), through the level's queue; told again while that tick
        // is pending, it schedules no second one (`willTickThisTick` / `hasScheduledTick`).
        // Hooks that run inside the update get level hosts, not a `ServerHost`, so a mod
        // can't call `update_neighbors` from within one: re-entry goes through the queue.
        with_game(|game| {
            lever_on(game, "minecraft:stone");
            game.block_ticks
                .add_empty_chunk(horizon_core::position::ChunkPos::new(0, 0))
                .unwrap();
            let level = game.level(game.dim.id()).unwrap();
            set(game, [7, 63, 5], "minecraft:stone");
            let repeater = game
                .pack
                .assets
                .registry
                .resolve(&horizon_world::chunk::BlockState {
                    name: "minecraft:repeater".into(),
                    properties: [
                        ("facing", "west"),
                        ("delay", "1"),
                        ("locked", "false"),
                        ("powered", "false"),
                    ]
                    .into_iter()
                    .map(|(key, value)| (key.into(), value.into()))
                    .collect(),
                })
                .unwrap();
            game.world.set(7, 64, 5, repeater).unwrap();
            let block = game.res.behaviours().block(repeater);
            let at = horizon_core::position::BlockPos::new(7, 64, 5);
            let state = game.world.block(6, 64, 5);
            assert!(!game.block_ticks.has_block_tick(at, block));
            assert_eq!(
                level.update_neighbors(game, lever, state.try_into_api().unwrap(), Neighbours),
                Ok(true)
            );
            assert!(game.block_ticks.has_block_tick(at, block));
            assert_eq!(
                level.update_neighbors(game, lever, state.try_into_api().unwrap(), Neighbours),
                Ok(true)
            );
            assert!(game.block_ticks.has_block_tick(at, block));
            assert_eq!(
                game.world.block(7, 64, 5),
                repeater,
                "unpowered until its tick runs"
            );
        });
        // `updateNeighbourForOutputSignal(cake)`: the comparator reading the cake, which
        // outputs 14 when whole while the comparator still reads 0, schedules its tick.
        with_game(|game| {
            lever_on(game, "minecraft:stone");
            game.block_ticks
                .add_empty_chunk(horizon_core::position::ChunkPos::new(0, 0))
                .unwrap();
            let level = game.level(game.dim.id()).unwrap();
            // Both stand on stone, as they must to survive a neighbour update.
            set(game, [10, 63, 5], "minecraft:stone");
            set(game, [11, 63, 5], "minecraft:stone");
            set(game, [10, 64, 5], "minecraft:cake");
            let comparator = game
                .pack
                .assets
                .registry
                .resolve(&horizon_world::chunk::BlockState {
                    name: "minecraft:comparator".into(),
                    properties: [
                        ("facing", "west"),
                        ("mode", "compare"),
                        ("powered", "false"),
                    ]
                    .into_iter()
                    .map(|(key, value)| (key.into(), value.into()))
                    .collect(),
                })
                .unwrap();
            game.world.set(11, 64, 5, comparator).unwrap();
            let block = game.res.behaviours().block(comparator);
            let at = horizon_core::position::BlockPos::new(11, 64, 5);
            assert!(!game.block_ticks.has_block_tick(at, block));
            let cake = game.world.block(10, 64, 5);
            assert_eq!(
                level.update_neighbors(
                    game,
                    [10, 64, 5],
                    cake.try_into_api().unwrap(),
                    OutputSignal
                ),
                Ok(true)
            );
            assert!(game.block_ticks.has_block_tick(at, block));
            // Off the level in place: refused, nothing done.
            assert_eq!(
                level.update_neighbors(
                    game,
                    [10, 64, 500_000],
                    cake.try_into_api().unwrap(),
                    OutputSignal
                ),
                Ok(false)
            );
            // A source that is no block state: refused, not a panic.
            let known: mod_api::BlockState = cake.try_into_api().unwrap();
            let unknown: mod_api::BlockState =
                mod_api::BlockStateRecord::new(known.block(), u16::MAX).into();
            assert_eq!(
                level.update_neighbors(game, [10, 64, 5], unknown, Neighbours),
                Err(mod_api::LevelWriteError::InvalidState(
                    mod_api::BlockStateError::UnknownState
                ))
            );
            assert_eq!(
                level.neighbor_changed(game, [11, 64, 5], unknown),
                Err(mod_api::LevelWriteError::InvalidState(
                    mod_api::BlockStateError::UnknownState
                ))
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn neighbor_changed_reads_its_inputs_and_its_tick_fires_at_its_delay() {
        use mod_platform::__private::part_tests::{
            SignalRead, signal_reader_game_thread, signal_reads,
        };

        // The fixture's part on glass reads its inputs in `neighbor_changed` and schedules
        // its own tick 2 ticks later, as a diode does.
        signal_reader_game_thread();
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let registry = game.pack.assets.registry;
            let glass = registry.default_state("minecraft:glass").unwrap();
            let lever = |powered: &str| {
                registry
                    .resolve(&horizon_world::chunk::BlockState {
                        name: "minecraft:lever".into(),
                        properties: [("face", "wall"), ("facing", "east"), ("powered", powered)]
                            .into_iter()
                            .map(|(key, value)| (key.into(), value.into()))
                            .collect(),
                    })
                    .unwrap()
            };
            // A block-ticking chunk: lit, its neighbours loaded, with its tick list, as
            // loading gives one.
            for (x, z) in (-1..=1).flat_map(|x| (-1..=1).map(move |z| (x, z))) {
                let mut chunk = horizon_client::chunk::Chunk::empty(x, z);
                chunk.lit = true;
                game.world.chunks.insert((x, z), std::sync::Arc::new(chunk));
            }
            game.block_ticks
                .add_empty_chunk(horizon_core::position::ChunkPos::new(0, 0))
                .unwrap();
            // A lever on the east face of stone, and glass north of the lever: the lever's
            // weak power reaches it from the south.
            let stone = registry.default_state("minecraft:stone").unwrap();
            game.world.set(5, 64, 5, stone).unwrap();
            game.world.set(6, 64, 4, glass).unwrap();
            game.world.set(6, 64, 5, lever("false")).unwrap();
            let dimension = game.dim.id().to_owned();
            let start = game.world.game_time;
            assert!(signal_reads().is_empty());

            // Pulling the lever updates its neighbours (`Block.UPDATE_ALL`), the glass among them.
            assert!(game.native_set_block(&dimension, [6, 64, 5], lever("true"), 3));
            assert_eq!(
                signal_reads(),
                [SignalRead::Changed([6, 64, 4], true, 15, true)]
            );
            // The tick it scheduled runs through the level's own queue, 2 ticks later, and
            // reads the lever south of it from `tick_block` (`LevelWriteHost: LevelTicksHost`).
            for (elapsed, reads) in [(1, vec![]), (2, vec![SignalRead::Ticked([6, 64, 4], 15)])] {
                game.world.game_time = start + elapsed;
                game.tick_scheduled().unwrap();
                assert_eq!(signal_reads(), reads, "after {elapsed} ticks");
            }
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_level_handle_reads_redstone_through_should_check_weak_power() {
        use horizon_core::geometry::Direction::West;

        // The fixture's part lets glass conduct (`should_check_weak_power`).
        mod_platform::__private::part_tests::weak_power_game_thread();
        with_game(|game| {
            lever_on(game, "minecraft:glass");
            let level = game.level(game.dim.id()).unwrap();
            assert_eq!(level.signal(game, [5, 64, 5], West), Some(15));
            assert_eq!(level.best_neighbor_signal(game, [4, 64, 5]), Some(15));
            // Strong power into the glass is its own; the part changes only what it conducts.
            assert_eq!(level.direct_signal_to(game, [5, 64, 5]), Some(15));
            assert_eq!(level.direct_signal_to(game, [4, 64, 5]), Some(0));
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn block_part_answers_is_pathfindable_once_per_state_and_type() {
        use mod_api::entity::mobs::PathComputationType::{self, Air, Land, Water};

        // Before any part: every state answers as the vanilla export says.
        with_game(|game| {
            let data = &game.res.rest().mob_data;
            for (i, vanilla) in data.states.iter().enumerate() {
                for (kind, bit) in [(Land, 1), (Water, 2), (Air, 4)] {
                    let state = horizon_world::registry::BlockStateId(i as u32);
                    assert_eq!(data.pathfindable(state, kind), vanilla.flags & bit != 0);
                }
            }
        });
        mod_platform::__private::part_tests::pathfindable_game_thread();
        with_game(|game| {
            let registry = game.pack.assets.registry;
            let asks = |name: &str, kind: PathComputationType| {
                let state = registry.default_state(name).unwrap();
                game.res.rest().mob_data.pathfindable(state, kind)
            };
            // Glass: land and air answered, water left to the block (vanilla: none of them).
            assert!(asks("minecraft:glass", Land));
            assert!(asks("minecraft:glass", Air));
            assert!(!asks("minecraft:glass", Water));
            // Air: land refused, water allowed, air left to the block.
            assert!(!asks("minecraft:air", Land));
            assert!(asks("minecraft:air", Water));
            assert!(asks("minecraft:air", Air));
            // A block with no part answers as before.
            assert!(!asks("minecraft:stone", Land));
            // A reload without the mod's answers takes them back.
            game.res.mob_data().resolve_parts(
                game.res.facts(),
                &horizon_client::mobs::data::PartAnswers::default(),
            );
            assert!(!asks("minecraft:glass", Land));
            assert!(asks("minecraft:air", Land));
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn block_part_asks_neighbours_to_update_when_its_block_is_removed() {
        use horizon_client::chunk::Chunk;
        use horizon_core::position::BlockPos;
        use std::sync::Arc;

        mod_platform::__private::part_tests::removal_game_thread();
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let glass = registry.default_state("minecraft:glass").unwrap();
            let stone = registry.default_state("minecraft:stone").unwrap();
            let lamp_state = |lit: &str| {
                registry
                    .resolve(&horizon_world::chunk::BlockState {
                        name: "minecraft:redstone_lamp".into(),
                        properties: [("lit".into(), lit.into())].into_iter().collect(),
                    })
                    .unwrap()
            };
            let power = registry.default_state("minecraft:redstone_block").unwrap();
            // An unlit lamp beside a redstone block lights when told a neighbour changed, and
            // set into the world it hears nothing: the proof the update reached it. Moved by a
            // piston, a removal tells no neighbour itself, so only the part's request can.
            let lamp = BlockPos::new(6, 64, 5);
            for (state, heard) in [(stone, false), (glass, true)] {
                game.world.set(5, 64, 5, state).unwrap();
                game.world.set(6, 64, 5, lamp_state("false")).unwrap();
                game.world.set(7, 64, 5, power).unwrap();
                let lit = game.with_level(|b, level, ticks| {
                    // Flags 82: how a piston pushes a block out (`PistonBaseBlock.moveBlocks`).
                    use horizon_world::level::flags;
                    let flags = flags::UPDATE_MOVE_BY_PISTON
                        | flags::UPDATE_KNOWN_SHAPE
                        | flags::UPDATE_CLIENTS;
                    b.set_block(
                        level,
                        ticks,
                        BlockPos::new(5, 64, 5),
                        registry.default_state("minecraft:air").unwrap(),
                        flags,
                    );
                    horizon_world::level::LevelAccess::block_state(level, lamp)
                        == lamp_state("true")
                });
                assert_eq!(lit, heard);
            }
        });
    }

    /// A plain piston pushing east at (5, `y`, 5) with `block` at (6, `y`, 5) and `above` on top
    /// of it, powered from (4, `y`, 5): whether the block above moved with the one below. Moved,
    /// its old cell is air once the push started. Each scenario has its own `y`, so none meets
    /// the moving blocks of another.
    fn piston_pulls_above(game: &mut super::Game<'_>, y: i32, block: &str, above: &str) -> bool {
        use horizon_core::position::BlockPos;
        let registry = game.pack.assets.registry;
        let state = |name: &str| registry.default_state(name).unwrap();
        let air = state("minecraft:air");
        game.with_level(|b, level, ticks| {
            let piston = |extended: &str| {
                registry
                    .resolve(&horizon_world::chunk::BlockState {
                        name: "minecraft:piston".into(),
                        properties: [
                            ("facing".into(), "east".into()),
                            ("extended".into(), extended.into()),
                        ]
                        .into_iter()
                        .collect(),
                    })
                    .unwrap()
            };
            let mut place = |pos: [i32; 3], s| {
                let flags = horizon_world::level::flags::UPDATE_ALL;
                b.set_block(
                    level,
                    ticks,
                    BlockPos::new(pos[0], pos[1], pos[2]),
                    s,
                    flags,
                );
            };
            place([5, y, 5], piston("false"));
            place([6, y, 5], state(block));
            place([6, y + 1, 5], state(above));
            place([4, y, 5], state("minecraft:redstone_block"));
            b.run_block_events(level, ticks);
            assert_eq!(
                horizon_world::level::LevelAccess::block_state(level, BlockPos::new(5, y, 5)),
                piston("true"),
                "{block}: the piston extended"
            );
            horizon_world::level::LevelAccess::block_state(level, BlockPos::new(6, y + 1, 5)) == air
        })
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn block_parts_decide_what_a_piston_pulls_along() {
        use horizon_client::chunk::Chunk;
        use std::sync::Arc;

        mod_platform::__private::part_tests::sticky_game_thread();
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            // The block below, the block on top of it, and whether the top one moves.
            for ((block, above, pulled), y) in [
                // No part: vanilla's slime and honey, and nothing else sticks.
                ("minecraft:slime_block", "minecraft:stone", true),
                ("minecraft:honey_block", "minecraft:stone", true),
                ("minecraft:slime_block", "minecraft:honey_block", false),
                ("minecraft:stone", "minecraft:stone", false),
                // A sticky part pulls whatever its `can_stick_to` allows: diamond all of it,
                // gold only gold.
                ("minecraft:diamond_block", "minecraft:stone", true),
                ("minecraft:diamond_block", "minecraft:honey_block", true),
                ("minecraft:gold_block", "minecraft:gold_block", true),
                ("minecraft:gold_block", "minecraft:stone", false),
                ("minecraft:gold_block", "minecraft:honey_block", false),
                // Either side may refuse: honey is held back by gold's rule though it is sticky.
                ("minecraft:honey_block", "minecraft:gold_block", false),
                // A slime part is kept apart from honey as slime is.
                ("minecraft:emerald_block", "minecraft:honey_block", false),
                ("minecraft:emerald_block", "minecraft:stone", true),
            ]
            .into_iter()
            .zip((8..).step_by(4))
            {
                assert_eq!(
                    piston_pulls_above(game, y, block, above),
                    pulled,
                    "{above} on {block}"
                );
            }
        });
    }

    /// The restitution an entity landing on a block reads is asked of the block's parts in the
    /// entity's level, on the side that ticks it, with the entity and the block's position; a
    /// block without a part is not asked, and a non-finite answer is ignored.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_block_part_changes_the_bounce_restitution_an_entity_reads() {
        use horizon_client::block_contact::{Contact, bounce_restitution};
        use horizon_core::position::BlockPos;

        let heard = mod_platform::__private::part_tests::restitution_game_thread();
        crate::game::native_hooks::install_block_contact_hooks();
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            let registry = game.pack.assets.registry;
            let state = |name: &str| registry.default_state(name).unwrap();
            let (glass, stone, slime) = (
                state("minecraft:glass"),
                state("minecraft:stone"),
                state("minecraft:slime_block"),
            );
            let pos = BlockPos::new(3, 64, 5);
            let mut read = |block, own, client_side| {
                game.world.set(pos.x, pos.y, pos.z, block).unwrap();
                let contact = Contact {
                    entity: 7,
                    kind: "minecraft:pig",
                    state: block,
                    position: pos,
                };
                let level = horizon_client::health::DamageLevel::of(
                    &mut game.world,
                    registry,
                    None,
                    Some(&game.pack.block_registries),
                    &game.damage_hooks,
                );
                bounce_restitution(&mut level.on_client(client_side), contact, own)
            };
            // Glass's part answers whatever the block's own value: 0.5 on a server, 0.25 on a
            // client predicting its own bounce.
            assert_eq!(read(glass, 0.0, false), 0.5);
            assert_eq!(read(glass, 1.0, true), 0.25);
            // A block with no part keeps its own: slime bounces fully.
            assert_eq!(read(slime, 1.0, false), 1.0);
            // Stone's part answers not a number, which is ignored.
            assert_eq!(read(stone, 0.0, false), 0.0);
            let pig = |client| (client, 7, "minecraft:pig".to_owned(), pos, true);
            assert_eq!(*heard.lock().unwrap(), [pig(false), pig(true)]);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn block_part_connects_wire_in_a_real_game_level() {
        use horizon_client::chunk::Chunk;
        use horizon_core::{geometry::Direction, position::BlockPos};
        use std::sync::Arc;

        mod_platform::__private::part_tests::redstone_game_thread();
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let wire = registry.default_state("minecraft:redstone_wire").unwrap();
            let glass = registry.default_state("minecraft:glass").unwrap();
            let stone = registry.default_state("minecraft:stone").unwrap();
            let pos = BlockPos::new(5, 64, 5);
            let neighbour = BlockPos::new(6, 64, 5);
            game.world.set(5, 63, 5, stone).unwrap();
            game.world.set(6, 64, 5, glass).unwrap();
            let connected = game.with_level(|b, level, ticks| {
                b.update_shape(level, ticks, wire, pos, Direction::East, neighbour, glass)
            });
            assert_eq!(
                registry.state(connected).unwrap().properties["east"],
                "side"
            );
            game.world.set(6, 64, 5, stone).unwrap();
            let vanilla = game.with_level(|b, level, ticks| {
                b.update_shape(level, ticks, wire, pos, Direction::East, neighbour, stone)
            });
            assert_eq!(registry.state(vanilla).unwrap().properties["east"], "none");
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn block_part_flips_fence_wall_and_pane_connections_in_a_real_game_level() {
        use horizon_client::chunk::Chunk;
        use horizon_core::{geometry::Direction, position::BlockPos};
        use std::sync::Arc;

        mod_platform::__private::part_tests::connection_game_thread();
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let state = |name: &str| registry.default_state(name).unwrap();
            let stone = state("minecraft:stone");
            let torch = state("minecraft:torch");
            let pos = BlockPos::new(5, 64, 5);
            let neighbour = BlockPos::new(6, 64, 5);
            let side = |game: &mut crate::game::Game<'_>, block, beside| {
                game.world.set(6, 64, 5, beside).unwrap();
                let shaped = game.with_level(|b, level, ticks| {
                    b.update_shape(level, ticks, block, pos, Direction::East, neighbour, beside)
                });
                registry.state(shaped).unwrap().properties["east"].clone()
            };
            game.world.set(5, 63, 5, stone).unwrap();
            // Vanilla joins each to a sturdy face and not to a torch. The part on
            // the oak fence, cobblestone wall and glass pane flips both answers;
            // the spruce fence, brick wall and iron bars have none.
            for (block, with_part, yes, no) in [
                ("oak_fence", true, "true", "false"),
                ("spruce_fence", false, "true", "false"),
                ("cobblestone_wall", true, "low", "none"),
                ("brick_wall", false, "low", "none"),
                ("glass_pane", true, "true", "false"),
                ("iron_bars", false, "true", "false"),
            ] {
                let block_state = state(&format!("minecraft:{block}"));
                let (sturdy, not) = if with_part { (no, yes) } else { (yes, no) };
                assert_eq!(side(game, block_state, stone), sturdy, "{block} by stone");
                assert_eq!(side(game, block_state, torch), not, "{block} by a torch");
            }
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn side_guards_agree_across_integrated_dedicated_and_client() {
        use super::super::execution_role::ExecutionRole;
        use horizon_client::chunk::Chunk;
        use std::sync::Arc;
        with_game(|game| {
            game.world
                .chunks
                .insert((-2, 3), Arc::new(Chunk::empty(-2, 3)));
            let level = game.level(game.dim.id()).unwrap();
            // The integrated server thread (game/world/integrated.rs sets the role and
            // `dedicated` on the serving Game) and a dedicated server simulate;
            // a client never does. A remote client runs with the `Client` role
            // (connecting changes `remote`, not the role), and the guards read
            // only the role, so it answers like the title-screen game here.
            for (role, dedicated) in [
                (ExecutionRole::IntegratedServer, true),
                (ExecutionRole::DedicatedServer, true),
            ] {
                game.options.execution_role = role;
                game.options.dedicated = dedicated.then(Vec::new);
                assert!(game.native_simulates());
                assert!(game.level(game.dim.id()).is_some());
                assert!(game.level_in_place(level));
                assert!(game.native_level_write_allowed(level, [-32, 64, 48]));
            }
            game.options.execution_role = ExecutionRole::Client;
            game.options.dedicated = None;
            assert!(!game.native_simulates());
            assert!(game.level(game.dim.id()).is_none());
            assert!(!game.level_in_place(level));
            assert!(!game.native_level_write_allowed(level, [-32, 64, 48]));
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn chunk_handles_resolve_loaded_parked_and_stale_levels() {
        use horizon_client::chunk::Chunk;
        use mod_api::{Authority, ChunkStatus};
        use std::sync::Arc;
        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            game.world
                .chunks
                .insert((-2, 3), Arc::new(Chunk::empty(-2, 3)));
            let chunk = game.chunk(level, [-2, 3]).unwrap();
            assert_eq!(chunk.pos(), [-2, 3]);
            assert_eq!(chunk.level(), level);
            assert!(chunk.loaded(game));
            assert_eq!(chunk.status(game), Some(ChunkStatus::Full));
            assert!(game.chunk(level, [99, 99]).is_none());
            assert!(
                game.enter_command_level(super::super::Dim::Nether)
                    .is_some()
            );
            assert_eq!(chunk.status(game), Some(ChunkStatus::Full));
            assert_eq!(game.chunk(level, [-2, 3]), Some(chunk));
            assert!(
                game.enter_command_level(super::super::Dim::Overworld)
                    .is_some()
            );
            game.world.chunks.remove(&(-2, 3));
            assert!(!chunk.loaded(game));
            assert_eq!(chunk.status(game), None);
            game.world
                .chunks
                .insert((-2, 3), Arc::new(Chunk::empty(-2, 3)));
            game.native_lifecycle
                .begin_open(Authority::IntegratedServer);
            assert!(game.chunk(level, [-2, 3]).is_none());
            assert!(!chunk.loaded(game));
            assert_eq!(chunk.status(game), None);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn level_reads_and_writes_use_dimension_build_height() {
        use horizon_client::chunk::{Chunk, LevelShape};
        use mod_api::BlockUpdateFlags;
        use std::sync::Arc;
        with_game(|game| {
            game.world.shape = LevelShape::new(0, 128, 128, true, false).unwrap();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level(game.dim.id()).unwrap();
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(0, 127, 0, stone).unwrap();
            game.world.set(0, 128, 0, stone).unwrap();
            assert!(game.level_is_loaded(level, [0, 127, 0]));
            assert_eq!(game.level_block_state(level, [0, 127, 0]), Some(stone));
            assert_eq!(
                game.level_block(level, [0, 127, 0])
                    .and_then(mod_api::__private::block_key_id),
                Some("minecraft:stone")
            );
            assert!(game.native_level_write_allowed(level, [0, 127, 0]));
            assert!(!game.level_is_loaded(level, [0, 128, 0]));
            assert_eq!(game.level_block_state(level, [0, 128, 0]), None);
            assert_eq!(game.level_block(level, [0, 128, 0]), None);
            assert!(!game.level_is_loaded(level, [0, -1, 0]));
            assert_eq!(game.level_block_state(level, [0, -1, 0]), None);
            assert_eq!(
                game.level_set_block(level, [0, 128, 0], stone, BlockUpdateFlags::NEIGHBORS),
                Ok(false)
            );
            assert!(
                game.level_destroy_block(level, [0, 128, 0], false, None, false)
                    .is_none()
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn equipping_mobs_by_hand_keeps_one_complete_stack() {
        use horizon_client::{items::ItemStack, mobs::Mob};
        use horizon_core::geometry::Vec3;
        with_game(|game| {
            game.creative = false;
            for (id, kind, item) in [
                (12, "minecraft:horse", "minecraft:iron_horse_armor"),
                (13, "minecraft:llama", "minecraft:red_carpet"),
                (14, "minecraft:happy_ghast", "minecraft:red_harness"),
                (15, "minecraft:nautilus", "minecraft:iron_nautilus_armor"),
            ] {
                let kind = game.res.mob_types().get(kind).unwrap().clone();
                let mut mob = Mob::new(id, kind, Vec3::ZERO, 0.0);
                mob.extra.tame = true;
                let mut held = ItemStack::new(item, 3);
                held.damage = 7;
                held.data_mut().custom_name = Some(horizon_core::nbt::Tag::String("kept".into()));
                game.inventory[game.selected] = Some(held.clone());
                let index = game.mobs.len();
                game.mobs.push(mob);
                let used = match id {
                    12 => game.interact_mount(index, item),
                    13 => game.interact_llama(index, item),
                    14 => game.interact_happy_ghast(index, item),
                    _ => game.interact_nautilus(index, item),
                };
                assert!(used, "{item}");
                assert_eq!(game.inventory[game.selected].as_ref().unwrap().count, 2);
                held.count = 1;
                assert_eq!(game.mobs[index].equipment_stacks()[6], Some(held), "{item}");
            }
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn sulfur_cube_absorbs_one_complete_stack_by_hand_and_pickup() {
        use horizon_client::{items::ItemStack, mobs::Mob};
        use horizon_core::geometry::Vec3;
        with_game(|game| {
            game.creative = false;
            let kind = game
                .res
                .mob_types()
                .get("minecraft:sulfur_cube")
                .unwrap()
                .clone();
            let item = kind.tempt.first().expect("a swallowable item").to_string();
            let mut held = ItemStack::new(&item, 3);
            held.data_mut().custom_name = Some(horizon_core::nbt::Tag::String("kept".into()));
            game.inventory[game.selected] = Some(held.clone());
            game.mobs.push(Mob::new(12, kind.clone(), Vec3::ZERO, 0.0));
            assert!(game.interact_sulfur_cube(0, &item));
            assert_eq!(game.inventory[game.selected].as_ref().unwrap().count, 2);
            held.count = 1;
            assert_eq!(game.mobs[0].equipment_stacks()[6], Some(held.clone()));
            game.mobs.clear();
            game.mobs.push(Mob::new(13, kind, Vec3::ZERO, 0.0));
            game.creatures.ticked.insert(13);
            game.spawn_item(
                ItemStack {
                    count: 3,
                    ..held.clone()
                },
                [0.0, 0.0, 0.0],
                [0.0; 3],
                0,
            );
            game.sulfur_cube_pickups();
            assert_eq!(game.mobs[0].equipment_stacks()[6], Some(held));
            assert_eq!(game.dropped[0].stack.count, 2);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn shearing_llama_and_happy_ghast_returns_full_body_stacks() {
        use horizon_client::{items::ItemStack, mobs::Mob};
        use horizon_core::geometry::Vec3;
        with_game(|game| {
            for (id, kind, item) in [
                (12, "minecraft:llama", "minecraft:red_carpet"),
                (13, "minecraft:happy_ghast", "minecraft:red_harness"),
            ] {
                let kind = game.res.mob_types().get(kind).unwrap().clone();
                let mut mob = Mob::new(id, kind, Vec3::ZERO, 0.0);
                mob.extra.tame = true;
                let mut stack = ItemStack::new(item, 3);
                stack.damage = 7;
                stack.data_mut().custom_name = Some(horizon_core::nbt::Tag::String("kept".into()));
                mob.extra.set_body_stack(Some(stack.clone()));
                let index = game.mobs.len();
                game.mobs.push(mob);
                let used = if item.ends_with("carpet") {
                    game.interact_llama(index, "minecraft:shears")
                } else {
                    game.interact_happy_ghast(index, "minecraft:shears")
                };
                assert!(used);
                assert!(game.dropped.iter().any(|drop| drop.stack == stack));
                assert!(game.mobs[index].extra.body_id().is_none());
            }
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn acting_guest_handles_resolve_during_guest_turns() {
        use super::super::lan::LanSettings;
        use horizon_client::{
            items::ItemStack,
            lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig},
        };
        use horizon_protocol::packets::configuration::ClientInformation;
        use mod_api::{EquipmentSlot, GameMode};
        use std::sync::{
            Arc,
            atomic::{AtomicI32, AtomicUsize},
        };

        with_game(|game| {
            let settings = LanSettings {
                port: 0,
                guest_commands: false,
                force_game_mode: false,
            };
            let net = NetConfig {
                listen: false,
                announce: false,
                lod_distance: 0,
                ..NetConfig::lan(0, String::new(), 2)
            };
            let config = HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: game.dim.id().to_owned(),
                allow_flight: true,
                seed: 0,
                max_players: 2,
                integrated: true,
                levels: game.level_shapes,
            };
            game.lan_host(settings, net, config).unwrap();
            let host = game.lan.as_mut().unwrap();
            for id in [12, 13] {
                let (tx, _rx) = std::sync::mpsc::channel();
                let mut guest = Guest::new(
                    Joined {
                        conn: id,
                        uuid: id as u128,
                        name: format!("guest{id}"),
                        properties: Vec::new(),
                        info: ClientInformation::new(8),
                        brand: String::new(),
                        sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                        latency: Arc::new(AtomicI32::new(0)),
                        address: "127.0.0.1:1".parse().unwrap(),
                        expected: false,
                    },
                    id as i32,
                    [3.0, 64.0, 5.0],
                    10,
                );
                guest.placed = true;
                if id == 12 {
                    // A player loaded from a file saved with a walking speed.
                    guest.abilities.walking_speed = 0.25;
                    guest.loaded_body = Some(horizon_client::lan::guest::SavedBody {
                        fire_ticks: 0,
                        air: 300,
                        fall_distance: 0.0,
                        velocity: [0.0; 3],
                        food_exhaustion: 0.0,
                        food_tick_timer: 0,
                        ticks_frozen: 0,
                        invulnerable: false,
                        invulnerable_time: 0,
                    });
                }
                guest.inventory[0] = Some(ItemStack::new("minecraft:stone", 1));
                host.hub.guests.push(guest);
            }
            let motion = horizon_client::motion::PlayerMotion::new(
                game.motion,
                horizon_core::geometry::Vec3::new(3.0, 64.0, 5.0),
            )
            .unwrap();
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| guest.motion = Some(Box::new(motion)))
                .unwrap();
            let player = game.player(12).unwrap();
            assert_eq!(
                player.inventory(game),
                Some(vec![crate::game::api_values::test_snapshot(
                    &ItemStack::new("minecraft:stone", 1)
                )])
            );
            let guest_living = game.living(player.fields().entity).unwrap();
            let host: &dyn mod_api::Host = game;
            assert_eq!(
                guest_living.attribute_base(host, "minecraft:max_health"),
                Some(20.0)
            );
            assert_eq!(
                guest_living.attribute_value(host, "minecraft:max_health"),
                Some(20.0)
            );
            assert_eq!(guest_living.max_health(host), Some(20.0));
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| guest.motion = None)
                .unwrap();
            // Body and saddle are stored on a guest like on a mob.
            let living = game.living(player.fields().entity).unwrap();
            let saddle = ItemStack::new("minecraft:saddle", 1);
            assert!(game.living_set_equipment(living, EquipmentSlot::Saddle, Some(saddle.clone())));
            assert!(game.living_set_equipment(
                living,
                EquipmentSlot::Body,
                Some(ItemStack::new("minecraft:iron_horse_armor", 1))
            ));
            assert_eq!(
                game.living_equipment(living, EquipmentSlot::Saddle),
                Some(saddle.clone())
            );
            assert_eq!(
                game.lan.as_ref().unwrap().hub.guests[0]
                    .equipment
                    .get(horizon_items::equipment::BODY)
                    .map(|s| &*s.item),
                Some("minecraft:iron_horse_armor")
            );
            assert!(game.living_set_equipment(living, EquipmentSlot::Saddle, None));
            assert_eq!(game.living_equipment(living, EquipmentSlot::Saddle), None);
            // The host's own previous mode, which no guest's change may touch.
            game.previous_mode = Some(horizon_client::command::args::GameMode::Adventure);
            assert_eq!(game.player_previous_game_mode(player), Some(None));
            assert!(!game.player_set_game_mode(player, GameMode::Survival));
            assert_eq!(game.player_previous_game_mode(player), Some(None));
            assert!(game.player_set_game_mode(player, GameMode::Creative));
            assert_eq!(game.player_game_mode(player), Some(GameMode::Creative));
            assert_eq!(
                game.player_previous_game_mode(player),
                Some(Some(GameMode::Survival))
            );
            let abilities = game.player_abilities(player).unwrap();
            assert!(abilities.invulnerable && abilities.may_fly && abilities.instabuild);
            assert!(game.player_set_game_mode(player, GameMode::Survival));
            assert_eq!(game.player_game_mode(player), Some(GameMode::Survival));
            let abilities = game.player_abilities(player).unwrap();
            assert!(!abilities.invulnerable && !abilities.may_fly && !abilities.instabuild);
            // A write sticks, is refused for a speed that is not a number, and a
            // mode change sets the flags again (the speeds stay).
            let mut written = abilities;
            written.may_build = false;
            written.walk_speed = 0.2;
            written.fly_speed = 0.1;
            assert!(game.player_set_abilities(player, written));
            assert_eq!(game.player_abilities(player), Some(written));
            assert_eq!(game.player_may_build(player), Some(false));
            let mut bad = written;
            bad.walk_speed = f32::NAN;
            assert!(!game.player_set_abilities(player, bad));
            assert_eq!(game.player_abilities(player), Some(written));
            assert!(game.player_set_game_mode(player, GameMode::Creative));
            let abilities = game.player_abilities(player).unwrap();
            assert!(abilities.may_build && abilities.instabuild && abilities.may_fly);
            assert_eq!((abilities.walk_speed, abilities.fly_speed), (0.2, 0.1));
            assert!(game.player_set_game_mode(player, GameMode::Survival));
            let stale = mod_api::__private::game::player_ref(
                mod_api::EntityRef::new(mod_api::WorldEpoch::new(0), player.fields().entity.id()),
                player.uuid(),
            );
            assert!(!game.player_set_game_mode(stale, GameMode::Creative));
            let other = game
                .native_lifecycle
                .epoch()
                .map(|epoch| mod_api::EntityRef::new(epoch, 13))
                .unwrap();
            game.as_guest(12, |game| {
                let player = game.player(12).unwrap();
                let entity = game.entity(12).unwrap();
                assert_eq!(player.fields().entity, entity);
                assert_eq!(entity.position(game), Some([3.0, 64.0, 5.0]));
                assert_eq!(entity.uuid(game), Some(12));
                let living = game.living(entity).unwrap();
                assert_eq!(
                    living
                        .equipment(game, EquipmentSlot::MainHand)
                        .unwrap()
                        .item()
                        .id(game),
                    "minecraft:stone"
                );
                assert_eq!(game.player.as_ref().unwrap().health.health(), 20.0);
                // The loaded walking speed is the movement speed's base.
                assert_eq!(
                    game.player
                        .as_mut()
                        .unwrap()
                        .attribute("minecraft:movement_speed")
                        .unwrap(),
                    f64::from(0.25f32)
                );
                // In the player's own turn the write reaches the body as well.
                let mut a = game.player_abilities(player).unwrap();
                a.may_fly = true;
                a.walk_speed = 0.3;
                assert!(game.player_set_abilities(player, a));
                assert_eq!(game.player_abilities(player), Some(a));
                assert!(game.player.as_ref().unwrap().may_fly);
                assert_eq!(other.position(game), Some([3.0, 64.0, 5.0]));
                assert!(game.living(other).is_some());
            })
            .unwrap();
            // A mode change in a turn, then a write in the same turn: the write wins.
            game.as_guest(12, |game| {
                let player = game.player(12).unwrap();
                // The guest's own previous mode is swapped in for the turn.
                assert_eq!(
                    game.player_previous_game_mode(player),
                    Some(Some(GameMode::Creative))
                );
                assert_eq!(game.player_is_secondary_use_active(player), Some(false));
                game.player.as_mut().unwrap().shift_key_down = true;
                assert_eq!(game.player_is_secondary_use_active(player), Some(true));
                game.player.as_mut().unwrap().shift_key_down = false;
                assert!(game.player_set_game_mode(player, GameMode::Creative));
                assert_eq!(
                    game.player_previous_game_mode(player),
                    Some(Some(GameMode::Survival))
                );
                let mut a = game.player_abilities(player).unwrap();
                assert!(a.instabuild && a.may_fly);
                a.fly_speed = 0.4;
                a.walk_speed = 0.3;
                assert!(game.player_set_abilities(player, a));
            })
            .unwrap();
            let kept = game.lan.as_ref().unwrap().hub.guest(12).unwrap();
            assert_eq!(kept.game_mode, 1);
            assert_eq!(kept.previous_game_mode, Some(0));
            assert_eq!(
                game.previous_mode,
                Some(horizon_client::command::args::GameMode::Adventure)
            );
            assert!(kept.abilities.instabuild);
            assert_eq!(kept.abilities.flying_speed, 0.4);
            assert_eq!(kept.abilities.walking_speed, 0.3);
            // The turn's write is stored in the guest's record.
            let stored = game.lan.as_ref().unwrap().hub.guest(12).unwrap().abilities;
            assert!(stored.may_fly && stored.walking_speed == 0.3);
            let swap = game.enter_guest(12).unwrap();
            assert!(game.entity(12).is_some());
            let other = game.entity(13).unwrap();
            assert_eq!(other.position(game), Some([3.0, 64.0, 5.0]));
            assert_eq!(other.uuid(game), Some(13));
            assert_eq!(
                other.kind(game).map(|kind| kind.id(game)).as_deref(),
                Some("minecraft:player")
            );
            let living = game.living(other).unwrap();
            assert_eq!(
                living
                    .equipment(game, EquipmentSlot::MainHand)
                    .unwrap()
                    .item()
                    .id(game),
                "minecraft:stone"
            );
            let player = game.player(13).unwrap();
            assert_eq!(player.fields().entity, other);
            assert_eq!(player.game_mode(game), Some(GameMode::Survival));
            assert!(game.player_set_game_mode(player, GameMode::Creative));
            assert_eq!(player.game_mode(game), Some(GameMode::Creative));
            assert_eq!(game.lan_acting, Some(12));
            let acting = game.player(12).unwrap();
            assert!(game.player_set_game_mode(acting, GameMode::Adventure));
            assert_eq!(game.player_game_mode(acting), Some(GameMode::Adventure));
            let nested = game.enter_guest(13).unwrap();
            assert!(game.enter_guest(12).is_none());
            assert_eq!(game.lan_acting, Some(13));
            assert!(game.entity(12).is_none());
            assert!(game.player(12).is_none());
            assert!(
                game.living(mod_api::EntityRef::new(other.epoch(), 12))
                    .is_none()
            );
            assert!(game.entity(13).is_some());
            game.leave_guest(nested);
            assert_eq!(game.lan_acting, Some(12));
            game.leave_guest(swap);
            assert!(game.lan_guest_swaps.is_empty());
            // A player saved in creative (game mode 1, mayfly and instabuild)
            // joins again: without force they stay creative; forced into the
            // default mode (survival) they get its abilities.
            let root = std::env::temp_dir().join(format!("abilities-force-{}", std::process::id()));
            game.world_root = root.clone();
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(13, |g| {
                    g.game_mode = 1;
                    g.abilities.update_for_mode(1);
                    assert!(g.abilities.may_fly && g.abilities.instabuild);
                    horizon_client::lan::playerdata::save(g, &root);
                })
                .unwrap();
            let joined = |game: &mut super::super::Game<'_>, force| {
                let host = game.lan.as_mut().unwrap();
                host.hub.config.default_game_mode = 0;
                host.hub.config.force_game_mode = force;
                host.hub
                    .update_guest(13, |g| {
                        g.game_mode = 0;
                        g.abilities = Default::default();
                    })
                    .unwrap();
                game.lan_guest_joined(13, &[], [0.0, 64.0, 0.0]);
                let g = game.lan.as_ref().unwrap().hub.guest(13).unwrap();
                (
                    g.game_mode,
                    g.abilities.may_fly,
                    g.abilities.instabuild,
                    g.abilities.may_build,
                )
            };
            assert_eq!(joined(game, false), (1, true, true, true));
            assert_eq!(joined(game, true), (0, false, false, true));
            // A remote guest can mount through the same handle path as the acting player.
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs.push(horizon_client::mobs::Mob::new(
                21,
                kind,
                horizon_core::geometry::Vec3::ZERO,
                0.0,
            ));
            let guest = game.entity(12).unwrap();
            let vehicle = game.entity(21).unwrap();
            assert!(game.entity_start_riding(guest, vehicle, true));
            assert_eq!(game.command_guest(12).unwrap().vehicle, Some(21));
            assert!(game.entity_stop_riding(guest));
            assert_eq!(game.command_guest(12).unwrap().vehicle, None);
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| guest.dimension = "minecraft:the_nether".into())
                .unwrap();
            assert!(!game.entity_start_riding(guest, vehicle, true));
            assert!(!game.entity_start_riding(vehicle, guest, true));
            let _ = std::fs::remove_dir_all(root);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn entities_in_returns_readable_handles_and_intersects_boxes() {
        use horizon_client::entities::ArmorStand;
        use horizon_core::geometry::Vec3;
        use mod_api::{LevelRef, WorldEpoch};
        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            game.armor_stands
                .push(ArmorStand::new(0, Vec3::new(0.5, 0.0, 0.0), 0.0));
            assert!(game.entity(0).is_none());
            game.armor_stands
                .push(ArmorStand::new(12, Vec3::new(1.0, 0.0, 0.0), 0.0));
            let found = game
                .level_entities_in(level, [0.0, 0.0, -1.0], [0.8, 1.0, 1.0])
                .unwrap();
            assert_eq!(found.len(), 1);
            assert_eq!(game.entity_position(found[0]), Some([1.0, 0.0, 0.0]));
            assert_eq!(
                game.entity_kind(found[0])
                    .map(|kind| kind.id(game))
                    .as_deref(),
                Some("minecraft:armor_stand")
            );
            assert!(
                game.level_entities_in(level, [0.0, 0.0, -1.0], [0.75, 1.0, 1.0])
                    .unwrap()
                    .is_empty()
            );
            assert!(
                game.level_entities_in(
                    LevelRef::new(WorldEpoch::new(0), game.dim.key()),
                    [0.0; 3],
                    [2.0; 3]
                )
                .is_none()
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn connectionless_guest_has_no_transport() {
        use horizon_client::{
            lan::{
                Joined, Lan, NetConfig, PlayerSender,
                chunks::ChunkEncoder,
                conn::{Holding, flush_all},
                data::SyncData,
                guest::Guest,
                hub::{Hub, HubConfig},
            },
            motion::PlayerMotion,
        };
        use horizon_core::geometry::Vec3;
        use horizon_protocol::packets::{
            configuration::ClientInformation,
            play::{Clientbound, Distance},
        };
        use mod_api::HostTransaction;
        use std::sync::{Arc, atomic::Ordering};
        with_game(|game| {
            let sync = Arc::new(SyncData::empty());
            let net = NetConfig {
                announce: false,
                ..NetConfig::lan(0, "test".into(), 2)
            };
            let lan = Lan::start_with(sync.clone(), net).unwrap();
            let real_id = lan.shared.next_id().unwrap();
            let fake_id = lan.shared.next_id().unwrap();
            assert_ne!(real_id, fake_id);
            let encoder = ChunkEncoder::new(
                [] as [(Arc<str>, bool); 0],
                &horizon_protocol::registry::Builtin::default(),
                &[],
                &[],
            );
            let config = HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 10,
                simulation_distance: 10,
                dimension: "minecraft:overworld".into(),
                allow_flight: false,
                seed: 0,
                max_players: 20,
                integrated: false,
                levels: Default::default(),
            };
            let mut hub = Hub::new(
                lan,
                sync,
                encoder,
                config.clone(),
                std::sync::Arc::new(
                    crate::game::test_server_load()
                        .loop_context(horizon_world::native_load_context::LoopSide::Server, []),
                ),
            );
            let motion = PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
            let fake = Guest::connectionless(fake_id, 2, "fake".into(), 2, [0.0; 3], motion);
            assert!(fake.placed && fake.motion.is_some());
            assert!(fake.loaded && fake.client_loaded());
            assert!(!fake.participates_online() && !fake.participates_in_player_simulation());
            assert_eq!((fake.game_mode, fake.permission, fake.input), (0, 0, 0));
            assert!(fake.vehicle.is_none() && fake.bed.is_none() && fake.menu.is_none());
            let (tx, rx) = std::sync::mpsc::channel();
            let mut real = Guest::new(
                Joined {
                    conn: real_id,
                    uuid: 1,
                    name: "real".into(),
                    properties: Vec::new(),
                    info: ClientInformation::new(8),
                    brand: String::new(),
                    sender: PlayerSender::new(tx, Arc::default()),
                    latency: Arc::default(),
                    address: ([127, 0, 0, 1], 1).into(),
                    expected: false,
                },
                1,
                [0.0; 3],
                10,
            );
            real.placed = true;
            hub.guests.push(real);
            hub.guests.push(fake);
            assert!(hub.here(&hub.guests[0]));
            assert!(!hub.here(&hub.guests[1]));
            let packet = Clientbound::SetSimulationDistance(Distance { distance: 8 });
            let hold = Holding::start();
            hub.send_to(fake_id, &packet);
            hub.broadcast(&packet);
            hub.guests[1].sender.send_many([vec![1, 2, 3]]);
            flush_all(hub.guests.iter().map(|g| &g.sender));
            drop(hold);
            assert_eq!(hub.guests[1].sender.pending.load(Ordering::Relaxed), 0);
            assert!(rx.recv_timeout(std::time::Duration::from_secs(2)).is_ok());
            game.lan_host(
                super::super::lan::LanSettings {
                    port: 0,
                    guest_commands: false,
                    force_game_mode: false,
                },
                NetConfig {
                    announce: false,
                    ..NetConfig::lan(0, "swap".into(), 2)
                },
                config,
            )
            .unwrap();
            let host = game.lan.as_mut().unwrap();
            host.hub.guests = std::mem::take(&mut hub.guests);
            {
                let value = [1600.0, 64.0, 0.0];
                host.hub.guests.update(1, |guest| guest.position = value);
            }
            game.lan_update_streaming();
            assert!(
                game.streaming
                    .extra
                    .iter()
                    .any(|(chunk, _)| *chunk == (0, 0))
            );
            assert!(
                !game
                    .streaming
                    .extra
                    .iter()
                    .any(|(chunk, _)| *chunk == (100, 0))
            );
            game.next_entity = 100;
            game.lan.as_ref().unwrap().hub.lan.shared.next_id();
            game.lan.as_ref().unwrap().hub.lan.shared.next_id();
            let level = game.level(game.dim.id()).unwrap();
            let role = game.options.execution_role;
            game.options.execution_role = super::super::execution_role::ExecutionRole::Client;
            assert!(game.fake_player(level, 1234, "machine").is_none());
            game.options.execution_role = role;
            let made = game.fake_player(level, 1234, "machine").unwrap();
            assert_eq!(game.fake_player(level, 1234, "machine"), Some(made));
            assert!(game.fake_player(level, 1234, "other").is_none());
            assert!(game.fake_player(level, 1, "real").is_none());
            assert_eq!(game.player_is_fake(made), Some(true));
            assert!(game.player(1234).is_none());
            let mut abilities = game.player_abilities(made).unwrap();
            abilities.may_fly = true;
            assert!(game.player_set_abilities(made, abilities));
            assert!(game.player_abilities(made).unwrap().may_fly);
            assert!(game.player_set_food_level(made, 10));
            assert!(game.player_feed(made, 3, 0.0));
            assert_eq!(game.player_food(made).unwrap().level, 13);
            assert!(game.player_set_experience_levels(made, 7));
            assert_eq!(game.player_experience(made).unwrap().level, 7);
            assert!(game.player_set_item_cooldown(made, "minecraft:stone", 20));
            assert_eq!(
                game.player_has_item_cooldown(made, "minecraft:stone"),
                Some(true)
            );
            assert!(game.player_remove_fake(made));
            assert!(!game.player_remove_fake(made));
            assert_eq!(game.player_is_fake(made), None);
            let replacement = game.fake_player(level, 1234, "machine").unwrap();
            assert_ne!(made, replacement);
            let saved_root = game.world_root.clone();
            let scratch = std::env::current_dir()
                .unwrap()
                .join("target/fake-player-save-test");
            let _ = std::fs::remove_dir_all(&scratch);
            game.world_root = scratch.clone();
            let host = game.lan.as_mut().unwrap();
            let i = host.hub.guests.iter().position(|g| g.uuid == 1234).unwrap();
            let mut actor = host.hub.guests.remove(i);
            game.save_guest(&mut actor);
            game.lan.as_mut().unwrap().hub.guests.insert(i, actor);
            assert!(!scratch.exists());
            game.world_root = saved_root;
            let conn = game
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guests
                .iter()
                .find(|g| g.uuid == 1234)
                .unwrap()
                .conn;
            game.as_guest(conn, |game| {
                assert_eq!(game.player_permission_level(replacement), Some(0));
                assert!(!game.player_remove_fake(replacement));
                assert!(game.fake_player(level, 4321, "other").is_none());
            })
            .unwrap();
            assert!(game.player_remove_fake(replacement));
            let overworld_actor = game.fake_player(level, 2345, "parked").unwrap();
            let attachment = horizon_mod_runtime::attachments::AttachmentType {
                namespace: "test",
                name: "identity",
                default: || 0i32,
                codec: None,
                copy_on_death: false,
                sync: false,
            };
            let holder = horizon_mod_runtime::attachments::Holder::Player(2345);
            game.options.dedicated = Some(Vec::new());
            let real_holder = horizon_mod_runtime::attachments::Holder::Player(1);
            attachment
                .set(
                    game.attachment_map_quiet(real_holder, mod_api::AttachmentAccess::Existing)
                        .unwrap(),
                    horizon_mod_runtime::attachments::Holder::Level,
                    17,
                )
                .unwrap();
            attachment
                .set(
                    game.attachment_map_quiet(holder, mod_api::AttachmentAccess::Existing)
                        .unwrap(),
                    horizon_mod_runtime::attachments::Holder::Level,
                    3,
                )
                .unwrap();
            assert!(
                game.enter_command_level(super::super::Dim::Nether)
                    .is_some()
            );
            let nether = game.level("minecraft:the_nether").unwrap();
            assert!(game.fake_player(level, 3456, "other").is_none());
            let nether_actor = game.fake_player(nether, 2345, "parked").unwrap();
            attachment
                .set(
                    game.attachment_map_quiet(holder, mod_api::AttachmentAccess::Existing)
                        .unwrap(),
                    horizon_mod_runtime::attachments::Holder::Level,
                    8,
                )
                .unwrap();
            assert_eq!(
                attachment
                    .get(
                        game.attachment_map_quiet(holder, mod_api::AttachmentAccess::Existing)
                            .unwrap(),
                        horizon_mod_runtime::attachments::Holder::Level
                    )
                    .unwrap(),
                8
            );
            assert_ne!(overworld_actor, nether_actor);
            let host: &mut dyn mod_api::ServerHost = game;
            let aborted: Result<(), ()> = host.transaction(|host, _| {
                attachment.set(host, holder, 99).unwrap();
                Err(())
            });
            assert!(aborted.is_err());
            assert_eq!(
                attachment
                    .get(
                        game.attachment_map_quiet(holder, mod_api::AttachmentAccess::Existing)
                            .unwrap(),
                        horizon_mod_runtime::attachments::Holder::Level
                    )
                    .unwrap(),
                8
            );
            let place = mod_api::ItemPlace::PlayerSlot {
                player: 2345,
                slot: 0,
            };
            let sticks = mod_api::ItemResource::of(&crate::game::api_values::test_snapshot(
                &horizon_client::items::ItemStack::new("minecraft:stick", 1),
            ));
            let host: &mut dyn mod_api::ServerHost = game;
            let rolled_back: Result<(), ()> = host.transaction(|host, tx| {
                assert_eq!(place.insert(host, tx, &sticks, 1), 1);
                Err(())
            });
            assert!(rolled_back.is_err());
            assert!(place.stack(game).is_none());
            assert_eq!(
                game.lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guests
                    .iter()
                    .find(|g| g.entity_id as u32 == overworld_actor.entity().id())
                    .unwrap()
                    .inventory[0],
                None
            );
            let host: &mut dyn mod_api::ServerHost = game;
            assert_eq!(
                host.transaction(|host, tx| Ok::<_, ()>(place.insert(host, tx, &sticks, 1))),
                Ok(1)
            );
            assert_eq!(place.stack(game).unwrap().count(), 1);
            assert_eq!(game.fake_player(nether, 2345, "parked"), Some(nether_actor));
            assert!(game.player_set_experience_levels(nether_actor, 8));
            assert!(game.player_set_experience_levels(overworld_actor, 3));
            assert_eq!(game.player_experience(nether_actor).unwrap().level, 8);
            assert_eq!(game.player_experience(overworld_actor).unwrap().level, 3);
            assert!(
                game.enter_command_level(super::super::Dim::Overworld)
                    .is_some()
            );
            assert_eq!(
                attachment
                    .get(
                        game.attachment_map_quiet(holder, mod_api::AttachmentAccess::Existing)
                            .unwrap(),
                        horizon_mod_runtime::attachments::Holder::Level
                    )
                    .unwrap(),
                3
            );
            assert!(game.player_remove_fake(overworld_actor));
            assert_eq!(
                attachment
                    .get(
                        game.attachment_map_quiet(real_holder, mod_api::AttachmentAccess::Existing)
                            .unwrap(),
                        horizon_mod_runtime::attachments::Holder::Level
                    )
                    .unwrap(),
                17
            );
            game.enter_command_level(super::super::Dim::Nether).unwrap();
            assert_eq!(
                attachment
                    .get(
                        game.attachment_map_quiet(holder, mod_api::AttachmentAccess::Existing)
                            .unwrap(),
                        horizon_mod_runtime::attachments::Holder::Level
                    )
                    .unwrap(),
                8
            );
            assert!(game.player_remove_fake(nether_actor));
            game.enter_command_level(super::super::Dim::Overworld)
                .unwrap();
            game.options.dedicated = None;
            let players = game.native_server_players();
            assert_eq!(players.len(), 1);
            assert_eq!(players[0].uuid(), 1);
            // A fake record in the Nether does not keep that level ticking.
            {
                let value = "minecraft:the_nether".into();
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(1, |guest| guest.dimension = value);
            }
            let ticking = game.levels_to_tick();
            assert!(!ticking.contains(&super::super::Dim::Nether));
            {
                let value = "minecraft:the_nether".into();
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| guest.dimension = value);
            }
            assert!(game.levels_to_tick().contains(&super::super::Dim::Nether));
            {
                let value = "minecraft:overworld".into();
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| guest.dimension = value);
            }
            game.lan_acting = Some(real_id);
            let selector = horizon_client::command::args::Selector {
                kind: 'a',
                limit: usize::MAX,
                ..Default::default()
            };
            assert_eq!(game.select(&selector).len(), 1);
            let named = horizon_client::command::args::Selector {
                player_name: Some("fake".into()),
                ..Default::default()
            };
            assert!(game.select(&named).is_empty());
            assert!(game.run_command_quietly("list"));
            assert!(!game.run_command_quietly("tp fake 0 64 0"));
            game.lan_acting = None;
            for (conn, learns) in [(fake_id, false), (real_id, true)] {
                game.as_guest(conn, |game| {
                    let mut book =
                        horizon_client::items::ItemStack::new("minecraft:knowledge_book", 1);
                    book.data_mut().set_other(
                        "minecraft:recipes",
                        Some(horizon_core::nbt::Tag::List(vec![
                            horizon_core::nbt::Tag::String("minecraft:stick".into()),
                        ])),
                    );
                    let slot = game.held_slot();
                    *game.stack_in_mut(slot) = Some(book.clone());
                    assert!(game.use_knowledge_book(&book));
                    assert!(game.stack_in(slot).is_none());
                    assert_eq!(game.level.recipe_book.knows("minecraft:stick"), learns);
                })
                .unwrap();
            }
            game.guest_packets
                .push(Clientbound::SetSimulationDistance(Distance { distance: 4 }));
            game.as_guest(fake_id, |game| {
                assert!(game.lan_acting_fake);
                assert!(game.player.as_ref().is_some_and(|p| p.client_loaded));
                assert!(game.player_view().is_none());
                let mut visited = 0;
                game.for_each_player(|_| visited += 1);
                assert_eq!(visited, 0);
                let before = game.stats.get("custom", "minecraft:jump");
                game.award_stat("custom", "minecraft:jump", 1);
                assert_eq!(game.stats.get("custom", "minecraft:jump"), before);
                game.trigger("minecraft:tick", |_, _| panic!("fake criterion evaluated"));
                assert_eq!(game.award_recipes(&["minecraft:test".into()]), 0);
                let source = horizon_client::command::args::Selector {
                    kind: 's',
                    limit: 1,
                    ..Default::default()
                };
                assert_eq!(
                    game.select(&source),
                    vec![super::super::commands::Target::Player]
                );
                let named = horizon_client::command::args::Selector {
                    player_name: Some("fake".into()),
                    ..Default::default()
                };
                assert!(game.select(&named).is_empty());
                let uuid = horizon_client::command::args::Selector {
                    uuid: Some(2),
                    ..Default::default()
                };
                assert!(game.select(&uuid).is_empty());
                game.guest_packets
                    .push(Clientbound::SetSimulationDistance(Distance {
                        distance: 12,
                    }));
                game.guest_entity_events.push(3);
                game.guest_cooldowns.push(("minecraft:test".into(), 20));
            })
            .unwrap();
            assert_eq!(game.guest_packets.len(), 1);
            assert!(game.guest_entity_events.is_empty() && game.guest_cooldowns.is_empty());
            assert_eq!(
                game.lan.as_ref().unwrap().hub.guests[1]
                    .sender
                    .pending
                    .load(Ordering::Relaxed),
                0
            );
            assert!(rx.try_recv().is_err());
            let outer = game.enter_guest(real_id).unwrap();
            assert!(!game.lan_acting_fake);
            game.guest_cooldowns.push(("minecraft:real".into(), 7));
            game.guest_player_sounds
                .push(("outer".into(), [0.0; 3], 1.0, 1.0));
            game.as_guest(fake_id, |game| {
                assert!(game.lan_acting_fake);
                game.guest_cooldowns.push(("minecraft:fake".into(), 20));
            })
            .unwrap();
            assert!(!game.lan_acting_fake);
            assert_eq!(game.guest_cooldowns.len(), 1);
            assert_eq!(&*game.guest_cooldowns[0].0, "minecraft:real");
            assert_eq!(game.guest_player_sounds[0].0, "outer");
            game.guest_player_sounds.clear();
            game.leave_guest(outer);
            let out = rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
            let horizon_client::lan::Out::Packet(body) = out else {
                panic!("expected cooldown packet")
            };
            let mut reader = horizon_protocol::buf::Reader::new(&body);
            let id = reader.varint().unwrap();
            let packet = Clientbound::decode(id, reader.rest()).unwrap();
            let Clientbound::Cooldown(cooldown) = packet else {
                panic!("expected cooldown")
            };
            assert_eq!(cooldown.group, "minecraft:real");
            assert_eq!(cooldown.duration, 7);
            assert!(rx.try_recv().is_err());

            // A real turn nested in a fake must not receive the fake's cooldown.
            let outer = game.enter_guest(fake_id).unwrap();
            assert!(game.lan_acting_fake);
            game.guest_cooldowns
                .push(("minecraft:fake_outer".into(), 8));
            game.guest_player_sounds
                .push(("fake_outer".into(), [0.0; 3], 1.0, 1.0));
            game.as_guest(real_id, |game| {
                assert!(!game.lan_acting_fake);
                let before = game.stats.get("custom", "minecraft:jump");
                game.award_stat("custom", "minecraft:jump", 1);
                assert_eq!(game.stats.get("custom", "minecraft:jump"), before + 1);
                game.guest_cooldowns
                    .push(("minecraft:real_inner".into(), 9));
            })
            .unwrap();
            assert!(game.lan_acting_fake);
            assert_eq!(&*game.guest_cooldowns[0].0, "minecraft:fake_outer");
            assert_eq!(game.guest_player_sounds[0].0, "fake_outer");
            game.guest_player_sounds.clear();
            game.leave_guest(outer);
            let horizon_client::lan::Out::Packet(body) =
                rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap()
            else {
                panic!("expected cooldown")
            };
            let mut reader = horizon_protocol::buf::Reader::new(&body);
            let id = reader.varint().unwrap();
            let Clientbound::Cooldown(cooldown) = Clientbound::decode(id, reader.rest()).unwrap()
            else {
                panic!("expected cooldown")
            };
            assert_eq!(cooldown.group, "minecraft:real_inner");
            assert!(rx.try_recv().is_err());

            // Two real nested turns deliver each queue to its own connection.
            let (tx2, rx2) = std::sync::mpsc::channel();
            let mut second = Guest::new(
                Joined {
                    conn: 3,
                    uuid: 3,
                    name: "second".into(),
                    properties: Vec::new(),
                    info: ClientInformation::new(8),
                    brand: String::new(),
                    sender: PlayerSender::new(tx2, Arc::default()),
                    latency: Arc::default(),
                    address: ([127, 0, 0, 1], 3).into(),
                    expected: false,
                },
                3,
                [0.0; 3],
                10,
            );
            second.placed = true;
            game.lan.as_mut().unwrap().hub.guests.push(second);
            let outer = game.enter_guest(real_id).unwrap();
            game.guest_cooldowns
                .push(("minecraft:real_outer".into(), 10));
            game.guest_player_sounds
                .push(("real_outer".into(), [0.0; 3], 1.0, 1.0));
            game.as_guest(3, |game| {
                assert_eq!(game.lan_acting, Some(3));
                assert!(
                    game.as_guest(real_id, |_| panic!("active guest re-entered"))
                        .is_none()
                );
                assert_eq!(game.lan_acting, Some(3));
                game.guest_cooldowns
                    .push(("minecraft:real_second".into(), 11))
            })
            .unwrap();
            assert_eq!(&*game.guest_cooldowns[0].0, "minecraft:real_outer");
            assert_eq!(game.guest_player_sounds[0].0, "real_outer");
            game.guest_player_sounds.clear();
            game.leave_guest(outer);
            for (receiver, expected) in [
                (&rx, "minecraft:real_outer"),
                (&rx2, "minecraft:real_second"),
            ] {
                let horizon_client::lan::Out::Packet(body) = receiver
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .unwrap()
                else {
                    panic!("expected cooldown")
                };
                let mut reader = horizon_protocol::buf::Reader::new(&body);
                let id = reader.varint().unwrap();
                let Clientbound::Cooldown(cooldown) =
                    Clientbound::decode(id, reader.rest()).unwrap()
                else {
                    panic!("expected cooldown")
                };
                assert_eq!(cooldown.group, expected);
                assert!(receiver.try_recv().is_err());
            }
            let fake = game.fake_player(level, 5678, "joiner").unwrap();
            game.enter_command_level(super::super::Dim::Nether).unwrap();
            let nether = game.level("minecraft:the_nether").unwrap();
            let other_fake = game.fake_player(nether, 5678, "joiner").unwrap();
            game.enter_command_level(super::super::Dim::Overworld)
                .unwrap();
            let saved_root = game.world_root.clone();
            let scratch = std::env::current_dir()
                .unwrap()
                .join("target/fake-login-save-test");
            let _ = std::fs::remove_dir_all(&scratch);
            game.world_root = scratch.clone();
            let shared = &game.lan.as_ref().unwrap().hub.lan.shared;
            let conn = shared.next_id().unwrap();
            let (tx, _rx) = std::sync::mpsc::channel();
            shared
                .events
                .send(horizon_client::lan::NetEvent::Joined(Box::new(Joined {
                    conn,
                    uuid: 5678,
                    name: "joiner".into(),
                    properties: Vec::new(),
                    info: ClientInformation::new(8),
                    brand: String::new(),
                    sender: PlayerSender::new(tx, Arc::default()),
                    latency: Arc::default(),
                    address: ([127, 0, 0, 1], 1).into(),
                    expected: false,
                })))
                .unwrap();
            use mod_api;
            struct Logout(std::sync::Arc<std::sync::atomic::AtomicUsize>);
            #[mod_api::__subscriptions]
            impl mod_api::Hooks for Logout {
                fn player_logged_out(
                    &self,
                    _: &mut dyn mod_api::ServerHost,
                    _: &mut mod_api::PlayerLoggedOutEvent<'_>,
                ) {
                    self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
            let logouts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "logout-test",
                Box::new(Logout(logouts.clone())),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                game.native_player_logged_out(0, "control");
                assert_eq!(logouts.swap(0, std::sync::atomic::Ordering::Relaxed), 1);
                game.tick_lan_packets();
                assert_eq!(logouts.load(std::sync::atomic::Ordering::Relaxed), 0);
            });
            assert_eq!(game.player_is_fake(fake), None);
            assert_eq!(game.player_is_fake(other_fake), None);
            assert!(
                !horizon_client::lan::playerdata::path(&scratch, 5678).exists(),
                "fake login replacement saved player data"
            );
            game.world_root = saved_root;
            let matching: Vec<_> = game
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guests
                .iter()
                .filter(|g| g.uuid == 5678)
                .collect();
            assert_eq!(matching.len(), 1);
            assert!(matching[0].participates_online());
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "logout-shutdown-test",
                Box::new(Logout(logouts.clone())),
            )]
            .into();
            let real_count = game
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guests
                .iter()
                .filter(|g| g.placed && g.participates_online())
                .count();
            mod_platform::testing::with_mods(mods, || game.native_players_leaving());
            assert_eq!(logouts.load(Ordering::Relaxed), real_count);
            game.lan.as_mut().unwrap().play.entry(fake_id).or_default();
            game.award_leave_game_to_all();
            let host = game.lan.as_mut().unwrap();
            assert!(!host.play.get(&fake_id).unwrap().left_game);
            assert!(host.play.get(&real_id).unwrap().left_game);
            host.hub.ticks = 2401;
            assert_eq!(game.ban_ip_target("fake"), None);
            assert_eq!(game.ban_ip_target("real").as_deref(), Some("127.0.0.1"));
            game.kick_idle(1);
            let host = game.lan.as_ref().unwrap();
            assert!(host.hub.guest(fake_id).unwrap().left_reason.is_empty());
            assert!(!host.hub.guest(real_id).unwrap().left_reason.is_empty());
            let old = game.fake_player(level, 6789, "reopen").unwrap();
            game.native_lifecycle_world_closed();
            game.native_lifecycle_open_started();
            let reopened = game.level(game.dim.id()).unwrap();
            assert_ne!(level.epoch(), reopened.epoch());
            assert_eq!(game.player_is_fake(old), None);
            let fresh = game.fake_player(reopened, 6789, "reopen").unwrap();
            assert_ne!(fresh, old);
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn level_sturdy_uses_gameplay_support() {
        use horizon_client::chunk::Chunk;
        use mod_api::SupportType as Support;
        use mod_api::{BlockFace as Face, LevelRef, WorldEpoch};
        use std::sync::Arc;
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level(game.dim.id()).unwrap();
            let slab = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone_slab")
                .unwrap();
            game.world.set(0, 64, 0, slab).unwrap();
            assert_eq!(
                level.block_face_sturdy(game, [0, 64, 0], Face::Down, Support::Full),
                Some(true)
            );
            assert_eq!(
                level.block_face_sturdy(game, [0, 64, 0], Face::North, Support::Full),
                Some(false)
            );
            let wall = game
                .pack
                .assets
                .registry
                .default_state("minecraft:cobblestone_wall")
                .unwrap();
            game.world.set(0, 64, 0, wall).unwrap();
            assert_eq!(
                level.block_face_sturdy(game, [0, 64, 0], Face::Up, Support::Center),
                Some(true)
            );
            assert_eq!(
                level.block_face_sturdy(game, [16, 64, 0], Face::Down, Support::Full),
                None
            );
            assert_eq!(
                level.block_face_sturdy(game, [0, 10000, 0], Face::Down, Support::Full),
                None
            );
            assert!(
                game.enter_command_level(super::super::Dim::Nether)
                    .is_some()
            );
            assert_eq!(
                level.block_face_sturdy(game, [0, 64, 0], Face::Up, Support::Center),
                Some(true)
            );
            assert!(
                game.enter_command_level(super::super::Dim::Overworld)
                    .is_some()
            );
            let shulker = game
                .pack
                .assets
                .registry
                .default_state("minecraft:shulker_box")
                .unwrap();
            game.world.set(0, 64, 0, shulker).unwrap();
            use horizon_client::block_entity_anim::{Shulker, ShulkerStatus};
            // The early guest packet pass changes opener status before entity
            // hooks run, and before tick_block_entities synchronizes animations.
            // Exercise that producer with both an absent and a closed snapshot.
            for had_snapshot in [false, true] {
                game.be_anims.shulkers.remove(&[0, 64, 0]);
                game.world.shulkers.remove(&[0, 64, 0]);
                if had_snapshot {
                    game.world.shulkers.insert([0, 64, 0], Shulker::default());
                }
                for support in [Support::Full, Support::Center, Support::Rigid] {
                    assert_eq!(
                        level.block_face_sturdy(game, [0, 64, 0], Face::Up, support),
                        Some(true),
                    );
                }
                assert!(game.container_openers_changed((0, 64, 0), true));
                assert_eq!(
                    game.be_anims.shulkers[&[0, 64, 0]].status,
                    ShulkerStatus::Opening
                );
                for parked in [false, true] {
                    if parked {
                        assert!(
                            game.enter_command_level(super::super::Dim::Nether)
                                .is_some()
                        );
                    }
                    for support in [Support::Full, Support::Center, Support::Rigid] {
                        assert_eq!(
                            level.block_face_sturdy(game, [0, 64, 0], Face::Up, support),
                            Some(false),
                        );
                        assert_eq!(
                            level.block_face_sturdy(game, [0, 64, 0], Face::Down, support),
                            Some(true),
                        );
                    }
                }
                assert!(
                    game.enter_command_level(super::super::Dim::Overworld)
                        .is_some()
                );
                assert!(game.container_openers_changed((0, 64, 0), false));
                assert_eq!(
                    game.world.shulkers[&[0, 64, 0]].status,
                    ShulkerStatus::Closing
                );
                for support in [Support::Full, Support::Center, Support::Rigid] {
                    assert_eq!(
                        level.block_face_sturdy(game, [0, 64, 0], Face::Up, support),
                        Some(false),
                    );
                }
            }
            game.be_anims.shulkers.remove(&[0, 64, 0]);
            for status in [
                ShulkerStatus::Closed,
                ShulkerStatus::Opening,
                ShulkerStatus::Opened,
                ShulkerStatus::Closing,
            ] {
                game.world.shulkers.insert(
                    [0, 64, 0],
                    Shulker {
                        status,
                        ..Default::default()
                    },
                );
                for support in [Support::Full, Support::Center, Support::Rigid] {
                    assert_eq!(
                        level.block_face_sturdy(game, [0, 64, 0], Face::Up, support),
                        Some(status == ShulkerStatus::Closed),
                    );
                    assert_eq!(
                        level.block_face_sturdy(game, [0, 64, 0], Face::Down, support),
                        Some(true),
                    );
                }
            }
            assert!(
                game.enter_command_level(super::super::Dim::Nether)
                    .is_some()
            );
            assert_eq!(
                level.block_face_sturdy(game, [0, 64, 0], Face::Up, Support::Full),
                Some(false),
            );
            assert!(
                game.enter_command_level(super::super::Dim::Overworld)
                    .is_some()
            );
            game.world.shulkers.remove(&[0, 64, 0]);
            assert_eq!(
                level.block_face_sturdy(game, [0, 64, 0], Face::Up, Support::Full),
                Some(true),
            );
            let moving = game
                .pack
                .assets
                .registry
                .default_state("minecraft:moving_piston")
                .unwrap();
            game.world.set(0, 64, 0, moving).unwrap();
            assert_eq!(
                level.block_face_sturdy(game, [0, 64, 0], Face::Up, Support::Full),
                Some(false),
            );
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.moving_pistons.push((
                [0, 64, 0],
                horizon_world::piston_shape::MovingPistonState {
                    moved_state: stone,
                    direction: horizon_core::geometry::Direction::East,
                    extending: true,
                    source: false,
                    progress: 1.0,
                    previous_progress: 1.0,
                },
            ));
            assert_eq!(
                level.block_face_sturdy(game, [0, 64, 0], Face::Up, Support::Full),
                Some(true),
            );
            game.world.moving_pistons[0].1.progress = 0.5;
            game.world.moving_pistons[0].1.previous_progress = 0.5;
            assert_eq!(
                level.block_face_sturdy(game, [0, 64, 0], Face::Up, Support::Full),
                Some(false),
            );
            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            assert_eq!(
                stale.block_face_sturdy(game, [0, 64, 0], Face::Down, Support::Full),
                None
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn level_clip_reads_parked_world_and_refuses_unloaded() {
        use horizon_client::chunk::Chunk;
        use mod_api::{BlockFace as Face, ClipBlock as Block, ClipFluid as Fluid};
        use std::sync::Arc;
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level(game.dim.id()).unwrap();
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(1, 64, 0, stone).unwrap();
            let from = [0.5, 64.5, 0.5];
            let to = [2.5, 64.5, 0.5];
            for mode in [Block::Collider, Block::Outline] {
                let hit = level.clip(game, from, to, mode, Fluid::None).unwrap();
                assert!(!hit.miss);
                assert_eq!(hit.block, [1, 64, 0]);
                assert_eq!(hit.face, Face::West);
                assert_eq!(hit.location, [1.0, 64.5, 0.5]);
                assert!(!hit.inside && !hit.world_border);
            }
            assert!(
                level
                    .clip(
                        game,
                        [15.5, 64.5, 0.5],
                        [17.5, 64.5, 0.5],
                        Block::Collider,
                        Fluid::None
                    )
                    .is_none()
            );
            let miss = level
                .clip(
                    game,
                    [0.5, 67.5, 0.5],
                    [2.5, 67.5, 0.5],
                    Block::Collider,
                    Fluid::None,
                )
                .unwrap();
            assert!(miss.miss);
            let inside = level
                .clip(
                    game,
                    [1.5, 64.5, 0.5],
                    [2.5, 64.5, 0.5],
                    Block::Collider,
                    Fluid::None,
                )
                .unwrap();
            assert!(inside.inside && !inside.miss);
            assert!(
                level
                    .clip(
                        game,
                        [f64::NAN, 64.0, 0.0],
                        to,
                        Block::Collider,
                        Fluid::None
                    )
                    .is_none()
            );
            let stale = mod_api::LevelRef::new(
                mod_api::WorldEpoch::new(level.epoch().get() + 1),
                level.dimension(),
            );
            assert!(
                stale
                    .clip(game, from, to, Block::Collider, Fluid::None)
                    .is_none()
            );
            use horizon_world::world_ray::RayWorld;
            let mut ordinary = super::super::explosion::ColliderRays {
                scene: horizon_client::collisions::SceneCollisions {
                    world: &game.world,
                    binding: game.collision,
                    subject: None,
                },
                context: horizon_world::collision_context::CollisionContext::empty(false),
                clip_data: None,
            };
            assert!(
                ordinary
                    .interaction_shape(
                        horizon_world::registry::BlockStateId(u32::MAX),
                        horizon_core::position::BlockPos::new(0, 0, 0)
                    )
                    .unwrap()
                    .is_empty()
            );
            let plant = game
                .pack
                .assets
                .registry
                .default_state("minecraft:dandelion")
                .unwrap();
            game.world.set(4, 64, 4, plant).unwrap();
            let facts = game.res.facts().get(plant);
            assert!(facts.outline_offset);
            let offset = game.pack.context.tables.offset(plant, 4, 4).unwrap();
            assert_ne!(offset[0], 0.0);
            let b = facts.outline[0];
            let y = 64.0 + offset[1] + (b[1] + b[4]) / 2.0;
            let z = 4.0 + offset[2] + (b[2] + b[5]) / 2.0;
            let plant_hit = level
                .clip(game, [3.0, y, z], [5.0, y, z], Block::Outline, Fluid::None)
                .unwrap();
            assert_eq!(plant_hit.block, [4, 64, 4]);
            assert!((plant_hit.location[0] - (4.0 + offset[0] + b[0])).abs() < 1e-9);
            let cauldron = game
                .pack
                .assets
                .registry
                .default_state("minecraft:cauldron")
                .unwrap();
            game.world.set(0, 64, 2, cauldron).unwrap();
            let bowl = level
                .clip(
                    game,
                    [0.5, 65.27, 0.5],
                    [0.5, 64.85, 3.5],
                    Block::Outline,
                    Fluid::None,
                )
                .unwrap();
            assert!(!bowl.miss);
            assert_eq!(bowl.block, [0, 64, 2]);
            assert_eq!(bowl.face, Face::Up);
            assert!(
                game.enter_command_level(super::super::Dim::Nether)
                    .is_some()
            );
            assert_eq!(
                level
                    .clip(game, from, to, Block::Collider, Fluid::None)
                    .unwrap()
                    .block,
                [1, 64, 0]
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn living_attributes_answer_for_mobs_stands_and_the_dragon() {
        use horizon_client::{entities::ArmorStand, mobs::Mob};
        use horizon_core::geometry::Vec3;
        with_game(|game| {
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, zombie, Vec3::ZERO, 0.0));
            game.armor_stands.push(ArmorStand::new(13, Vec3::ZERO, 0.0));
            let mut dragon = super::super::dragon::Dragon::new(Vec3::ZERO, 0.0, 0);
            dragon.id = 14;
            game.dragon.replace(dragon);
            for (id, max) in [(12, 20.0), (13, 20.0), (14, 200.0)] {
                let living = game.living(game.entity(id).unwrap()).unwrap();
                assert_eq!(
                    living.attribute_base(game, "minecraft:max_health"),
                    Some(max)
                );
                assert_eq!(
                    living.attribute_value(game, "minecraft:max_health"),
                    Some(max)
                );
                assert_eq!(living.attribute_value(game, "missing:attribute"), None);
            }
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn level_shapes_use_empty_context() {
        use super::super::{Audio, Game, Options, Progress, Settings, load_resources};
        use horizon_client::chunk::Chunk;
        use mod_api::{Authority, BlockShapeKind};
        use std::{sync::Arc, time::Instant};

        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            super::super::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = super::super::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.to_path_buf(),
            )),
            None,
        )
        .unwrap();
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.native_lifecycle
            .begin_open(Authority::IntegratedServer);

        let level = game.level("minecraft:overworld").unwrap();
        assert!(
            game.enter_command_level(super::super::Dim::Nether)
                .is_some()
        );
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        game.world.set(0, 64, 0, stone).unwrap();
        assert!(
            game.enter_command_level(super::super::Dim::Overworld)
                .is_some()
        );
        let nether = game.level("minecraft:the_nether").unwrap();
        let host: &dyn mod_api::ServerHost = &game;
        assert_eq!(host.levels().list_levels(), vec![level, nether]);
        let cube = vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]];
        assert_eq!(
            game.level_block_shape(nether, stone, [0, 64, 0], BlockShapeKind::Outline),
            Some(cube.clone())
        );
        assert_eq!(
            game.level_block_shape(nether, stone, [0, 64, 0], BlockShapeKind::Collision),
            Some(cube)
        );
        let shape = |game: &mut Game<'_>, name: &str, kind| {
            let state = game.pack.assets.registry.default_state(name).unwrap();
            game.world.set(0, 64, 0, state).unwrap();
            game.level_block_shape(level, state, [0, 64, 0], kind)
                .unwrap()
        };
        game.inventory[game.selected] = Some(horizon_client::items::ItemStack {
            item: "minecraft:light".into(),
            count: 1,
            damage: 0,
            extra: None,
        });
        assert!(shape(&mut game, "minecraft:light", BlockShapeKind::Outline).is_empty());
        assert_eq!(
            game.outline_boxes([0, 64, 0]),
            vec![[0.0, 64.0, 0.0, 1.0, 65.0, 1.0]]
        );
        game.inventory[game.selected].as_mut().unwrap().item = "minecraft:scaffolding".into();
        let scaffolding = shape(&mut game, "minecraft:scaffolding", BlockShapeKind::Outline);
        assert!(!scaffolding.is_empty());
        assert_ne!(scaffolding, vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]]);
        assert_eq!(
            game.outline_boxes([0, 64, 0]),
            vec![[0.0, 64.0, 0.0, 1.0, 65.0, 1.0]]
        );
        assert_eq!(
            shape(&mut game, "minecraft:stone_slab", BlockShapeKind::Outline),
            vec![[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]]
        );
        assert_eq!(
            shape(&mut game, "minecraft:stone_slab", BlockShapeKind::Collision),
            vec![[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]]
        );
        let contains = |boxes: &[[f64; 6]], p: [f64; 3]| {
            boxes
                .iter()
                .any(|b| (0..3).all(|i| b[i] < p[i] && p[i] < b[i + 3]))
        };
        let stair = shape(&mut game, "minecraft:oak_stairs", BlockShapeKind::Outline);
        assert!(contains(&stair, [0.25, 0.25, 0.25]));
        let upper = [
            [0.25, 0.75, 0.25],
            [0.75, 0.75, 0.25],
            [0.25, 0.75, 0.75],
            [0.75, 0.75, 0.75],
        ];
        assert_eq!(
            upper.into_iter().filter(|p| contains(&stair, *p)).count(),
            2
        );
        let fence = shape(&mut game, "minecraft:oak_fence", BlockShapeKind::Outline);
        assert!(contains(&fence, [0.5, 0.5, 0.5]));
        assert!(!contains(&fence, [0.1, 0.5, 0.1]));
        let lectern = shape(&mut game, "minecraft:lectern", BlockShapeKind::Collision);
        assert!(contains(&lectern, [0.5, 0.5, 0.5]));
        assert!(!contains(&lectern, [0.1, 0.5, 0.1]));
    }

    #[test]
    fn uuid_words_are_most_significant_first() {
        let words = [0x0102_0304, -1, 0, 0x7fff_ffff];
        assert_eq!(
            super::uuid_from_words(words),
            0x0102_0304_ffff_ffff_0000_0000_7fff_ffff
        );
    }
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn level_block_writes_validate_and_change_world() {
        use super::super::{Audio, Game, Options, Progress, Settings, load_resources};
        use horizon_client::{chunk::Chunk, items::ItemStack};
        use mod_api::{Authority, BlockUpdateFlags, HostTransaction};
        use std::{sync::Arc, time::Instant};

        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            super::super::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = super::super::execution_role::ExecutionRole::IntegratedServer;
        options.dedicated = Some(Vec::new());
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.to_path_buf(),
            )),
            None,
        )
        .unwrap();
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.native_lifecycle
            .begin_open(Authority::IntegratedServer);

        let level = game.level("minecraft:overworld").unwrap();
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        let chest = game
            .pack
            .assets
            .registry
            .default_state("minecraft:chest")
            .unwrap();
        let air = game
            .pack
            .assets
            .registry
            .default_state("minecraft:air")
            .unwrap();
        // Exercise the generated typed boundary, not only the native engine-input helpers.
        use crate::game::api_values::TryIntoApi;
        let api_stone: mod_api::BlockState = stone.try_into_api().unwrap();
        let invalid: mod_api::BlockState =
            mod_api::BlockStateRecord::new(api_stone.block(), u16::MAX).into();
        assert_eq!(
            level.set_block(&mut game, [0, 64, 0], invalid, BlockUpdateFlags::NONE),
            Err(mod_api::LevelWriteError::InvalidState(
                mod_api::BlockStateError::UnknownState
            ))
        );
        assert_eq!(game.world.block(0, 64, 0), air);
        {
            let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::Setup);
            assert_eq!(
                level.set_block(&mut game, [0, 64, 0], api_stone, BlockUpdateFlags::NONE),
                Err(mod_api::LevelWriteError::Unavailable)
            );
        }
        assert_eq!(game.world.block(0, 64, 0), air);
        game.world.block_log = Some(Vec::new());
        assert_eq!(
            level.update_neighbors(
                &mut game,
                [0, 64, 0],
                invalid,
                mod_api::NeighbourUpdate::Neighbours
            ),
            Err(mod_api::LevelWriteError::InvalidState(
                mod_api::BlockStateError::UnknownState
            ))
        );
        assert_eq!(
            level.neighbor_changed(&mut game, [0, 64, 0], invalid),
            Err(mod_api::LevelWriteError::InvalidState(
                mod_api::BlockStateError::UnknownState
            ))
        );
        assert_eq!(game.world.block(0, 64, 0), air);
        assert!(game.world.block_log.as_ref().unwrap().is_empty());
        // Flag-aware writes log only client updates; ordinary World::set still logs by default.
        assert_eq!(
            game.level_set_block(level, [2, 64, 0], stone, BlockUpdateFlags::NONE),
            Ok(true)
        );
        assert!(game.world.block_log.as_ref().unwrap().is_empty());
        assert_eq!(
            game.level_set_block(level, [3, 64, 0], stone, BlockUpdateFlags::CLIENTS),
            Ok(true)
        );
        assert_eq!(game.world.block_log.as_ref().unwrap(), &vec![(3, 64, 0)]);
        assert_eq!(
            game.level_set_block(
                level,
                [4, 64, 0],
                stone,
                BlockUpdateFlags::CLIENTS | BlockUpdateFlags::INVISIBLE
            ),
            Ok(true)
        );
        assert_eq!(
            game.world.block_log.as_ref().unwrap(),
            &vec![(3, 64, 0), (4, 64, 0)]
        );
        // `sendBlockUpdated` queues the block again without writing; out of height it queues nothing.
        game.world.modified.clear();
        let dirty_before = game.world.modified.clone();
        let block_before = game.world.block(5, 64, 0);
        assert!(game.level_send_block_updated(level, [5, 64, 0]));
        assert!(game.level_send_block_updated(level, [5, 64, 0]));
        assert_eq!(game.world.modified.clone(), dirty_before);
        assert_eq!(game.world.block(5, 64, 0), block_before);
        let host: &mut dyn mod_api::ServerHost = &mut game;
        let aborted: Result<(), ()> = host.transaction(|host, _| {
            assert!(level.send_block_updated(host, [6, 64, 0]));
            Err(())
        });
        assert!(aborted.is_err());
        assert!(!game.world.block_log.as_ref().unwrap().contains(&(6, 64, 0)));
        assert!(!game.level_send_block_updated(level, [5, 10_000, 0]));
        assert_eq!(
            game.world.block_log.as_ref().unwrap(),
            &vec![(3, 64, 0), (4, 64, 0), (5, 64, 0)]
        );
        // A bare hand mines at the base speed; an unloaded position has none.
        assert_eq!(
            game.level_block_break_speed(level, [3, 64, 0], None),
            Some(1.0)
        );
        assert_eq!(
            game.level_block_break_speed(level, [3, 10_000, 0], None),
            None
        );
        let mut pickaxe = ItemStack::new("minecraft:iron_pickaxe", 1);
        assert_eq!(
            game.level_block_break_speed(level, [3, 64, 0], Some(&pickaxe)),
            Some(6.0)
        );
        pickaxe.enchant("minecraft:efficiency", 5);
        assert_eq!(
            game.level_block_break_speed(level, [3, 64, 0], Some(&pickaxe)),
            Some(6.0)
        );
        let mut custom_tool = ItemStack::new("minecraft:stick", 1);
        let components = horizon_client::item_data::command_components(
            r#"[tool={rules:[{blocks:"stone",speed:7.0f}],default_mining_speed:2.0f}]"#,
        )
        .unwrap();
        (custom_tool.damage, custom_tool.extra) =
            horizon_client::item_data::decode_components(Some(&components));
        assert_eq!(
            game.level_block_break_speed(level, [3, 64, 0], Some(&custom_tool)),
            Some(7.0)
        );
        assert_eq!(
            game.level_block_break_speed(level, [5, 64, 0], Some(&custom_tool)),
            Some(2.0)
        );
        // Server-side game: the client-only crack is refused.
        assert!(!game.level_set_client_break_progress(level, 1, [3, 64, 0], 4));
        game.world.block_log = None;
        let waterlogged = game
            .pack
            .assets
            .registry
            .resolve(&horizon_world::chunk::BlockState {
                name: "minecraft:oak_slab".into(),
                properties: [
                    ("type".into(), "bottom".into()),
                    ("waterlogged".into(), "true".into()),
                ]
                .into_iter()
                .collect(),
            })
            .unwrap();
        let water = game
            .pack
            .assets
            .registry
            .default_state("minecraft:water")
            .unwrap();
        for moved in [false, true] {
            game.world.set(5, 64, 0, waterlogged).unwrap();
            assert!(game.level_remove_block(level, [5, 64, 0], moved));
            assert_eq!(game.world.block(5, 64, 0), water);
            game.world.set(5, 64, 0, waterlogged).unwrap();
            let removed = game.with_level(|_, level, ticks| {
                let mut host = super::super::mod_api_host::LevelTicks { level, ticks };
                mod_api::LevelWriteHost::remove_block(&mut host, [5, 64, 0], moved)
            });
            assert!(removed);
            assert_eq!(game.world.block(5, 64, 0), water);
        }
        let flags = BlockUpdateFlags::NEIGHBORS;
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], stone, flags),
            Ok(true)
        );
        assert_eq!(game.world.block(0, 64, 0), stone);
        assert_eq!(
            game.level_set_block(level, [16, 64, 0], stone, flags),
            Ok(false)
        );
        assert_eq!(
            game.level_set_block(level, [0, 10000, 0], stone, flags),
            Ok(false)
        );
        let stale = mod_api::LevelRef::new(
            mod_api::WorldEpoch::new(level.epoch().get() + 1),
            level.dimension(),
        );
        assert_eq!(
            game.level_set_block(stale, [0, 64, 0], stone, flags),
            Err(mod_api::LevelWriteError::Stale)
        );
        assert!(
            game.level_destroy_block(stale, [0, 64, 0], true, None, false)
                .is_none()
        );
        let fill_chest = |game: &mut Game<'_>| {
            let Some(super::super::BlockEntity::Chest(items)) =
                game.world.block_entities.get_mut(&(0, 64, 0))
            else {
                panic!("chest block entity missing")
            };
            items[0] = Some(horizon_client::items::ItemStack {
                item: "minecraft:diamond".into(),
                count: 3,
                damage: 0,
                extra: None,
            });
        };
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], chest, flags),
            Ok(true)
        );
        fill_chest(&mut game);
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], air, flags),
            Ok(true)
        );
        assert_eq!(
            game.dropped
                .iter()
                .filter(|item| item.stack.item.as_ref() == "minecraft:diamond")
                .map(|item| item.stack.count)
                .sum::<u32>(),
            3
        );
        game.dropped.clear();
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], chest, flags),
            Ok(true)
        );
        fill_chest(&mut game);
        assert_eq!(
            game.level_set_block(
                level,
                [0, 64, 0],
                air,
                BlockUpdateFlags::SKIP_BLOCK_ENTITY_SIDE_EFFECTS
            ),
            Ok(true)
        );
        assert!(!game.world.block_entities.contains_key(&(0, 64, 0)));
        assert!(game.dropped.is_empty());
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], stone, flags),
            Ok(true)
        );
        assert!(
            game.level_destroy_block(level, [0, 64, 0], false, None, false)
                .is_some()
        );
        assert!(game.dropped.is_empty());
        assert_eq!(game.world.block(0, 64, 0), air);
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], stone, flags),
            Ok(true)
        );
        assert!(
            game.level_destroy_block(level, [0, 64, 0], true, None, false)
                .is_some()
        );
        assert!(
            game.dropped
                .iter()
                .any(|item| item.stack.item.as_ref() == "minecraft:cobblestone")
        );
        assert!(
            game.level_destroy_block(level, [0, 64, 0], true, None, false)
                .is_none()
        );
        game.dropped.clear();
        let copper = game
            .pack
            .assets
            .registry
            .default_state("minecraft:copper_chest")
            .unwrap();
        let exposed = game
            .pack
            .assets
            .registry
            .default_state("minecraft:exposed_copper_chest")
            .unwrap();
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], copper, flags),
            Ok(true)
        );
        fill_chest(&mut game);
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], exposed, flags),
            Ok(true)
        );
        let Some(super::super::BlockEntity::Chest(items)) =
            game.world.block_entities.get(&(0, 64, 0))
        else {
            panic!("copper chest entity missing")
        };
        assert_eq!(items[0].as_ref().unwrap().count, 3);
        assert!(game.dropped.is_empty());
        let sensor = game
            .pack
            .assets
            .registry
            .default_state("minecraft:sculk_sensor")
            .unwrap();
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], sensor, flags),
            Ok(true)
        );
        assert!(matches!(
            game.world.block_entities.get(&(0, 64, 0)),
            Some(super::super::BlockEntity::Sculk(_))
        ));
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], chest, flags),
            Ok(true)
        );
        assert!(matches!(
            game.world.block_entities.get(&(0, 64, 0)),
            Some(super::super::BlockEntity::Chest(_))
        ));
        assert!(
            game.enter_command_level(super::super::Dim::Nether)
                .is_some()
        );
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], stone, flags),
            Err(mod_api::LevelWriteError::Unavailable)
        );
        assert!(
            game.level_destroy_block(level, [0, 64, 0], true, None, false)
                .is_none()
        );
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_client_finds_the_entities_it_tracks_and_its_own_player_in_a_box() {
        use super::super::execution_role::ExecutionRole;
        use horizon_core::geometry::Vec3;
        use horizon_protocol::connection::Pipe;
        use mod_api::{Host, LevelRef, WorldEpoch};
        with_game(|game| {
            game.options.execution_role = ExecutionRole::Client;
            game.options.dedicated = None;
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "EntitiesInTest").unwrap();
            let remote = game.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.entity_id = 7;
            remote.profile = 42;
            game.player =
                Some(horizon_client::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            game.spawn_entity(15, "minecraft:pig", [2.0, 0.0, 0.0], [0.0; 3]);
            game.spawn_entity(16, "minecraft:pig", [40.0, 0.0, 0.0], [0.0; 3]);
            let level = game.level("minecraft:overworld").unwrap();
            let ids = |host: &dyn Host, level: LevelRef, min, max| {
                level
                    .entities_in(host, min, max)
                    .map(|found| found.into_iter().map(|e| e.id()).collect::<Vec<_>>())
            };
            // The tracked pig, then the window's own player; the other pig is far away.
            assert_eq!(
                ids(game, level, [-1.0, -1.0, -1.0], [3.0, 2.0, 1.0]),
                Some(vec![15, 7])
            );
            assert_eq!(
                ids(game, level, [39.0, -1.0, -1.0], [41.0, 2.0, 1.0]),
                Some(vec![16])
            );
            // The pig's box ends at x 2.45.
            assert_eq!(
                ids(game, level, [2.5, 0.0, -1.0], [4.0, 1.0, 1.0]),
                Some(vec![])
            );
            game.set_mode_id(3);
            assert_eq!(
                ids(game, level, [-1.0, -1.0, -1.0], [3.0, 2.0, 1.0]),
                Some(vec![15])
            );
            let nether = LevelRef::new(level.epoch(), mod_api::DimensionKey::NETHER);
            assert_eq!(ids(game, nether, [-1.0; 3], [3.0; 3]), None);
            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            assert_eq!(ids(game, stale, [-1.0; 3], [3.0; 3]), None);
            game.remote.as_mut().unwrap().logged_in = false;
            assert_eq!(ids(game, level, [-1.0; 3], [3.0; 3]), None);
        });
    }

    /// A client's `entities_in` lists another player standing in the box (added through the
    /// real `AddEntity` packet), after the entities the server tracks and before the window's
    /// own player; a player outside the box, and one the server removed, are not listed.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_client_lists_another_player_in_the_box_after_the_tracked_entities() {
        use super::super::execution_role::ExecutionRole;
        use horizon_core::geometry::Vec3;
        use horizon_protocol::{
            connection::Pipe,
            packets::play::{AddEntity, Clientbound as C, RemoveEntities},
        };
        use mod_api::{Host, LevelRef};
        with_game(|game| {
            game.options.execution_role = ExecutionRole::Client;
            game.options.dedicated = None;
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "EntitiesInTest").unwrap();
            let remote = game.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.entity_id = 7;
            remote.profile = 42;
            let player = remote
                .builtin
                .get("minecraft:entity_type")
                .unwrap()
                .id("minecraft:player")
                .unwrap();
            game.player =
                Some(horizon_client::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            game.spawn_entity(15, "minecraft:pig", [2.0, 0.0, 0.0], [0.0; 3]);
            let add_player = |game: &mut crate::game::Game<'_>, id, uuid, x| {
                game.handle_packet(C::AddEntity(AddEntity {
                    id,
                    uuid,
                    kind: player,
                    position: [x, 0.0, 0.0],
                    velocity: [0.0; 3],
                    pitch: 0,
                    yaw: 0,
                    head_yaw: 0,
                    data: 0,
                }))
                .unwrap();
            };
            // 18 stands in the box, 19 far outside it, 20 in it until the server removes it.
            add_player(game, 18, 9, 1.0);
            add_player(game, 19, 10, 40.0);
            add_player(game, 20, 11, 0.5);
            assert!(
                [18, 19, 20]
                    .iter()
                    .all(|id| game.mobs.iter().any(|m| m.id == *id))
            );
            let level = game.level("minecraft:overworld").unwrap();
            let ids = |host: &dyn Host, level: LevelRef, min, max| {
                level
                    .entities_in(host, min, max)
                    .map(|found| found.into_iter().map(|e| e.id()).collect::<Vec<_>>())
            };
            // Entity-list order: the pig, then the other players, then the window's own.
            assert_eq!(
                ids(game, level, [-1.0, -1.0, -1.0], [3.0, 2.0, 1.0]),
                Some(vec![15, 18, 20, 7])
            );
            // A box around only the far player finds it, and the near box did not.
            assert_eq!(
                ids(game, level, [39.0, -1.0, -1.0], [41.0, 2.0, 1.0]),
                Some(vec![19])
            );
            game.handle_packet(C::RemoveEntities(RemoveEntities { ids: vec![20] }))
                .unwrap();
            assert_eq!(
                ids(game, level, [-1.0, -1.0, -1.0], [3.0, 2.0, 1.0]),
                Some(vec![15, 18, 7])
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_client_refuses_a_box_with_a_bound_that_is_not_finite_and_swaps_an_inverted_one() {
        use super::super::execution_role::ExecutionRole;
        use horizon_core::geometry::Vec3;
        use horizon_protocol::connection::Pipe;
        use mod_api::{Host, LevelRef};
        with_game(|game| {
            game.options.execution_role = ExecutionRole::Client;
            game.options.dedicated = None;
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "EntitiesInTest").unwrap();
            let remote = game.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.entity_id = 7;
            remote.profile = 42;
            game.player =
                Some(horizon_client::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            game.spawn_entity(15, "minecraft:pig", [2.0, 0.0, 0.0], [0.0; 3]);
            let level = game.level("minecraft:overworld").unwrap();
            let ids = |host: &dyn Host, level: LevelRef, min, max| {
                level
                    .entities_in(host, min, max)
                    .map(|found| found.into_iter().map(|e| e.id()).collect::<Vec<_>>())
            };
            // Mod code chooses the bounds: one that is not finite is refused, never panics,
            // and nothing is walked by the box's size.
            for (min, max) in [
                ([f64::NAN, -1.0, -1.0], [3.0, 2.0, 1.0]),
                ([-1.0, -1.0, -1.0], [3.0, f64::NAN, 1.0]),
                ([f64::NEG_INFINITY, -1.0, -1.0], [3.0, 2.0, 1.0]),
                ([-1.0, -1.0, -1.0], [3.0, 2.0, f64::INFINITY]),
                ([f64::NEG_INFINITY; 3], [f64::INFINITY; 3]),
            ] {
                assert_eq!(ids(game, level, min, max), None, "{min:?} {max:?}");
            }
            // A minimum above its maximum on one axis is the same box swapped (vanilla's
            // AABB): the tracked pig, then the window's own player.
            assert_eq!(
                ids(game, level, [3.0, -1.0, -1.0], [-1.0, 2.0, 1.0]),
                Some(vec![15, 7])
            );
            assert_eq!(
                ids(game, level, [3.0, 2.0, 1.0], [-1.0, -1.0, -1.0]),
                Some(vec![15, 7])
            );
        });
    }
}

#[cfg(test)]
#[path = "native_entity_refs/restitution_replay_tests.rs"]
mod restitution_replay_tests;
