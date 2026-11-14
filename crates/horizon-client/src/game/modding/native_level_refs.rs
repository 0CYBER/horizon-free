//! A level's blocks, height and time through a `LevelRef`
//! (docs/modding/src/how-to/native-mods.md, "Entity and level handles", #7387). Server reads
//! reach any loaded level; both-side reads also reach the client's current level.
//! A handle of an earlier open or an unloaded block reads `None` (or `false`).
use super::Game;
use horizon_client::{streaming::Streaming, world::World};
use horizon_core::{
    geometry::Direction,
    position::{BlockPos, ChunkPos},
};
use horizon_items::items::ItemStack;
use horizon_world::registry::BlockStateId;
use mod_api::world::mod_chunk_tickets::ModTicket;
use mod_api::{
    BlockHit, BlockTransformPreview, ChunkRef, ChunkStatus, ClipBlock, ClipFluid, DimensionKey,
    Hand, LevelRef, LightLayer, MapState, PlayerRef, RespawnData, TicketController, Vec3,
};

fn block_pos([x, y, z]: [i32; 3]) -> BlockPos {
    BlockPos::new(x, y, z)
}

/// Whether chunk `pos` lies inside the world (`/forceload`'s 30 million block bound).
fn chunk_in_world(pos: [i32; 2]) -> bool {
    const LIMIT: i32 = 30_000_000 >> 4;
    pos.iter().all(|c| (-LIMIT..LIMIT).contains(c))
}

/// Whether the mod running owns `controller`: a mod uses only the controllers it registered.
fn calling_mod_owns(controller: TicketController) -> bool {
    controller
        .owner()
        .is_some_and(|owner| mod_api::content::crash::current_native_mod() == Some(owner))
}

/// The stored light of `layer` at `pos` in `world`, as the game's own light query reads it.
pub(super) fn world_brightness(world: &World, layer: LightLayer, pos: [i32; 3]) -> i32 {
    let light = world.light(pos[0], pos[1], pos[2]);
    i32::from(match layer {
        LightLayer::Block => light & 15,
        LightLayer::Sky => light >> 4,
    })
}

/// A mod's registry id or `#tag`, namespaced as a command argument is.
fn normalized_selector(id: &str) -> String {
    use horizon_client::command::args::normalized;
    match id.strip_prefix('#') {
        Some(tag) => format!("#{}", normalized(tag)),
        None => normalized(id).into_owned(),
    }
}

/// Bound the synchronous generator work before constructing its spiral or height vector.
fn biome_search_within_budget(parameters: mod_api::BiomeSearch, height: i32) -> bool {
    if parameters.radius < 0
        || parameters.horizontal_step <= 0
        || parameters.vertical_step <= 0
        || height <= 0
    {
        return false;
    }
    let rings = parameters.radius as u64 / parameters.horizontal_step as u64;
    let width = 2 * rings + 1;
    // Clamping the origin into the build interval cannot increase this height count.
    let heights = (height as u64 - 1) / parameters.vertical_step as u64 + 1;
    width
        .checked_mul(width)
        .and_then(|columns| columns.checked_mul(heights))
        .is_some_and(|samples| samples <= mod_api::BiomeSearch::MAX_SAMPLES)
}

impl Game<'_> {
    pub(super) fn level_add_water_ticket(
        &mut self,
        level: LevelRef,
        min: [i32; 3],
        max: [i32; 3],
    ) -> Option<mod_api::WaterTicket> {
        let owner = mod_api::content::crash::current_native_mod()?;
        let owner_index = mod_api::__private::mod_index(owner)?;
        if !mod_api::__private::mod_index_on(owner_index) {
            return None;
        }
        self.native_level_world(level)?;
        let world = if level.dimension() == self.dim.key() {
            &mut self.world
        } else {
            self.levels.parked_world_mut_of(level.dimension_id())?
        };
        world
            .water_tickets
            .add(owner, min, max, || {
                mod_api::__private::mod_index_on(owner_index)
            })
            .map(|id| mod_api::WaterTicket::new(level, id))
    }

    pub(super) fn water_ticket_remove(&mut self, ticket: mod_api::WaterTicket) -> bool {
        let Some(owner) = mod_api::content::crash::current_native_mod() else {
            return false;
        };
        let Some(owner_index) = mod_api::__private::mod_index(owner) else {
            return false;
        };
        if !mod_api::__private::mod_index_on(owner_index) {
            return false;
        }
        let level = ticket.level();
        if self.native_level_world(level).is_none() {
            return false;
        }
        let world = if level.dimension() == self.dim.key() {
            &mut self.world
        } else if let Some(world) = self.levels.parked_world_mut_of(level.dimension_id()) {
            world
        } else {
            return false;
        };
        world.water_tickets.remove(owner, ticket.id())
    }

    pub(super) fn level_has_water_ticket(&self, level: LevelRef, pos: [i32; 3]) -> bool {
        self.native_level_world(level)
            .is_some_and(|world| world.water_tickets.contains(pos))
    }

    /// `ServerLevel.findClosestBiome3d`, using the same generator query as `/locate biome`.
    pub(super) fn level_find_closest_biome(
        &self,
        level: LevelRef,
        biome: &str,
        parameters: mod_api::BiomeSearch,
    ) -> Option<mod_api::BiomeSearchResult> {
        let world = self.native_level_world(level)?;
        if !biome_search_within_budget(parameters, world.shape.max_y() - world.shape.min_y) {
            return None;
        }
        let terrain = if level.dimension() == self.dim.key() {
            self.end.locator()
        } else {
            self.levels.parked_locator_of(level.dimension_id())
        }?;
        let selector = normalized_selector(biome);
        let allowed = |name: &str| self.biome_matches(&selector, name);
        let query = horizon_world::worldgen::generator::BiomeSearch {
            allowed: &allowed,
            origin: parameters.origin,
            radius: parameters.radius,
            horizontal: parameters.horizontal_step,
            vertical: parameters.vertical_step,
            // Resident columns are padded to the Overworld's storage span.
            // BiomeSource searches use the dimension's build range instead.
            min_y: world.shape.min_y,
            max_y: world.shape.max_y() - 1,
        };
        terrain
            .find_closest_biome(&query)
            .map(|(biome, position)| mod_api::BiomeSearchResult::new(biome, position))
    }

    /// `ServerLevel.findNearestMapStructure` without skipping known starts, through
    /// the generator search `/locate structure` runs.
    pub(super) fn level_find_nearest_structure(
        &self,
        level: LevelRef,
        structure: &str,
        parameters: mod_api::StructureSearch,
    ) -> Option<mod_api::StructureSearchResult> {
        self.native_level_world(level)?;
        // Inside the world the placement cells' chunks stay far from overflowing.
        if !(0..=mod_api::StructureSearch::MAX_RADIUS).contains(&parameters.radius)
            || !horizontal_in_bounds(parameters.origin)
        {
            return None;
        }
        let terrain = if level.dimension() == self.dim.key() {
            self.end.locator()
        } else {
            self.levels.parked_locator_of(level.dimension_id())
        }?;
        let names = self.structure_names(&normalized_selector(structure));
        terrain
            .locate(&names, parameters.origin, parameters.radius)
            .map(|(structure, position)| mod_api::StructureSearchResult::new(structure, position))
    }

    /// `LevelReader.getBiome`: the noise cell selected by this level's BiomeManager zoom.
    /// Unlike a generating server query, an absent selected column returns None.
    fn native_read_biome(&self, level: LevelRef, pos: [i32; 3]) -> Option<String> {
        let world = self.native_level_read_world(level)?;
        let streaming = if level.dimension() == self.native_level_key() {
            &self.streaming
        } else {
            self.levels.parked_streaming_of(level.dimension_id())?
        };
        let [qx, qy, qz] = streaming.biome_zoom.quart_at_block(block_pos(pos));
        let chunk = world.chunks.get(&(qx >> 2, qz >> 2))?;
        // ChunkAccess clamps to the dimension's height, not our padded storage.
        // Clamp before conversion to preserve signed block-coordinate extremes.
        let min_qy = world.shape.min_y >> 2;
        let max_qy = (world.shape.max_y() - 1) >> 2;
        let y = qy.clamp(min_qy, max_qy) << 2;
        let biome = chunk.biome(qx << 2, y, qz << 2);
        Some(world_biomes!(self).get(biome).name.clone())
    }

    /// The authoritative server's `LevelReader.getBiome`, including parked levels.
    pub(super) fn level_biome_at(&self, level: LevelRef, pos: [i32; 3]) -> Option<String> {
        if !self.is_logical_server() {
            return None;
        }
        self.native_read_biome(level, pos)
    }

    /// The client's current copy of `LevelReader.getBiome`, never an authoritative server read.
    pub(super) fn level_client_biome(&self, level: LevelRef, pos: [i32; 3]) -> Option<String> {
        if self.is_logical_server() {
            return None;
        }
        self.native_read_biome(level, pos)
    }

    /// A biome's natural spawners by category from the world's biome table.
    pub(super) fn level_biome_spawners(
        &self,
        level: LevelRef,
        biome: &str,
    ) -> Option<Vec<mod_api::world::biome::BiomeSpawners>> {
        self.native_level_read_world(level)?;
        world_biomes!(self).spawners(biome)
    }

    /// A biome's own sky, fog and water colours from the world's biome table.
    pub(super) fn level_biome_effects(
        &self,
        level: LevelRef,
        biome: &str,
    ) -> Option<mod_api::world::biome::BiomeEffects> {
        self.native_level_read_world(level)?;
        world_biomes!(self).effects(biome)
    }

    /// Whether a biome is in a biome tag, by the world's live tag tables.
    pub(super) fn level_biome_in_tag(
        &self,
        level: LevelRef,
        biome: &str,
        tag: &str,
    ) -> Option<bool> {
        self.native_level_read_world(level)?;
        if !world_biomes!(self).contains(biome) {
            return None;
        }
        let features = self.res.behaviours().live_features()?;
        Some(features.biome_in_tag(biome, tag.strip_prefix('#').unwrap_or(tag)))
    }

    /// A biome's `Biome.ClimateSettings` from the world's biome table, data packs included.
    pub(super) fn level_biome_properties(
        &self,
        level: LevelRef,
        biome: &str,
    ) -> Option<mod_api::BiomeProperties> {
        self.native_level_read_world(level)?;
        world_biomes!(self).properties(biome)
    }

    /// `ServerLevel.getLogicalHeight`: the dimension's logical height, not its build height.
    pub(super) fn level_logical_height(&self, level: LevelRef) -> Option<i32> {
        Some(self.native_level_read_world(level)?.shape.logical_height)
    }

    /// `BlockGetter.getBlockFloorHeight` with an empty collision context.
    /// Uses the normal dynamic shapes, querying below only when this shape is empty.
    pub(super) fn level_block_floor_height(
        &self,
        level: LevelRef,
        [x, y, z]: [i32; 3],
    ) -> Option<f64> {
        let world = self.native_level_read_world(level)?;
        if !world.loaded(x, z) || !world.shape.contains_y(y) {
            return None;
        }
        let here = self.block_collision_shape_in(world, [x, y, z]);
        let below = if here.is_empty() {
            self.block_collision_shape_in(world, [x, y.checked_sub(1)?, z])
        } else {
            horizon_core::geometry::VoxelShape::empty()
        };
        Some(horizon_core::geometry::block_floor_height(&here, || &below))
    }

    /// The border of a resident server level, through the handle's lifetime check.
    pub(super) fn level_world_border(&self, level: LevelRef) -> Option<[f64; 4]> {
        self.native_level_world(level)?;
        Some(self.border.bounds())
    }
    /// The world a handle names now, or None for a stale or foreign one.
    pub(super) fn native_level_world(&self, level: LevelRef) -> Option<&World> {
        self.native_level_world_result(level).ok()
    }
    /// The single residency boundary for typed server-level reads.
    pub(super) fn native_level_world_result(
        &self,
        level: LevelRef,
    ) -> Result<&World, mod_api::LevelReadError> {
        if !self.is_logical_server() {
            return Err(mod_api::LevelReadError::Unavailable);
        }
        if !self.level_handle_current(level) {
            return Err(mod_api::LevelReadError::Stale);
        }
        if level.dimension() == self.dim.key() {
            return Ok(&self.world);
        }
        self.levels
            .parked_world_of(level.dimension_id())
            .ok_or(mod_api::LevelReadError::Stale)
    }
    /// The world a handle names for a read that works on both sides: the
    /// server's level, or on a client the one it shows (`Minecraft.level`, its
    /// copy of the server's). None for a stale handle or another dimension.
    pub(super) fn native_level_read_world(&self, level: LevelRef) -> Option<&World> {
        if self.is_logical_server() {
            return self.native_level_world(level);
        }
        (self.remote.as_ref().is_some_and(|r| r.logged_in)
            && self.level_handle_current(level)
            && level.dimension() == self.native_level_key())
        .then_some(&self.world)
    }
    /// Find a loaded chunk in an open server level.
    pub(super) fn chunk(&self, level: LevelRef, pos: [i32; 2]) -> Option<ChunkRef> {
        self.native_level_world(level)?
            .chunks
            .contains_key(&(pos[0], pos[1]))
            .then_some(mod_api::__private::game::chunk_ref(level, pos))
    }
    /// Every loaded chunk of an open server level as handles, ordered by chunk x then z. None
    /// for a stale handle or on a client. Loads nothing.
    pub(super) fn level_loaded_chunks(&self, level: LevelRef) -> Option<Vec<ChunkRef>> {
        let mut positions: Vec<(i32, i32)> = self
            .native_level_world(level)?
            .chunks
            .keys()
            .copied()
            .collect();
        positions.sort_unstable();
        Some(
            positions
                .into_iter()
                .map(|(x, z)| mod_api::__private::game::chunk_ref(level, [x, z]))
                .collect(),
        )
    }
    /// Whether this chunk is loaded; false on a client or for a stale handle.
    pub(super) fn chunk_loaded(&self, chunk: ChunkRef) -> bool {
        self.chunk(chunk.level(), chunk.pos()).is_some()
    }
    /// The status of a loaded chunk. Proto chunks are not exposed here.
    pub(super) fn chunk_status(&self, chunk: ChunkRef) -> Option<ChunkStatus> {
        self.chunk(chunk.level(), chunk.pos())
            .map(|_| ChunkStatus::Full)
    }
    /// A handle for chunk `pos` of an open level, loaded or not, so a mod can
    /// force an unloaded chunk. None for a stale handle, on a client, and for
    /// a chunk outside the world (`ChunkPos` past 30 million blocks).
    pub(super) fn level_chunk_ref(&self, level: LevelRef, pos: [i32; 2]) -> Option<ChunkRef> {
        self.native_level_world(level)?;
        chunk_in_world(pos).then_some(ChunkRef::new(level, pos))
    }
    /// The streaming of the level a handle names now, or None for a stale or
    /// foreign one.
    fn native_level_streaming(&self, level: LevelRef) -> Option<&Streaming> {
        self.native_level_world(level)?;
        if level.dimension() == self.dim.key() {
            return Some(&self.streaming);
        }
        self.levels.parked_streaming_of(level.dimension_id())
    }
    /// Whether `/forceload` holds the chunk (`ServerLevel.getForcedChunks`),
    /// loaded or not; a mod's own ticket on it does not count. None for a stale handle or on a client.
    pub(super) fn chunk_forced(&self, chunk: ChunkRef) -> Option<bool> {
        let [x, z] = chunk.pos();
        Some(
            self.native_level_streaming(chunk.level())?
                .commanded()
                .contains(&(x, z)),
        )
    }
    /// `/forceload add` or `remove` of one chunk (`ServerLevel.setChunkForced`):
    /// true when the forced set changed. A forced chunk is kept loaded and
    /// entity ticking, saved with the level, and the level keeps ticking
    /// without a player. This is the world's own `/forceload` state, not a
    /// ticket the mod owns: the chunk stays forced across restarts, and after
    /// the mod is unloaded or removed, until `set_forced(false)` or
    /// `/forceload remove` clears it. Adding a newly forced chunk loads it
    /// before returning, as vanilla does.
    /// False for a stale handle, on a client, and for a level the server
    /// has not opened.
    pub(super) fn chunk_set_forced(&mut self, chunk: ChunkRef, forced: bool) -> bool {
        if self.native_level_streaming(chunk.level()).is_none() {
            return false;
        }
        let [x, z] = chunk.pos();
        if chunk.level().dimension() == self.dim.key() {
            self.streaming
                .set_forced_loaded(&mut self.world, x, z, forced)
        } else {
            self.levels
                .force_parked_chunk(chunk.level().dimension_id(), x, z, forced)
                .unwrap_or(false)
        }
    }
    /// The mods validate the chunk tickets `dim`'s level loaded from its save,
    /// controller by controller (`LoadingValidationCallback.validateTickets`),
    /// and the survivors take hold. Tickets a mod added since the load (in
    /// `level_load`, say) are not candidates, and a controller registered
    /// without a validator keeps all of its tickets. Nothing is built while no
    /// mod listens.
    pub(super) fn validate_mod_chunk_tickets(&mut self, dim: super::Dim) {
        let mut loaded = self.streaming.take_loaded_tickets();
        if !loaded.is_empty() && mod_api::__private::listened(mod_api::HookId::ValidateTickets) {
            let mut controllers: Vec<u32> = loaded.iter().map(|t| t.controller).collect();
            controllers.sort_unstable();
            controllers.dedup();
            for slot in controllers {
                let Some(controller) = TicketController::from_slot(slot) else {
                    continue;
                };
                let Some(owner) = controller.owner().filter(|_| controller.validates()) else {
                    continue;
                };
                let (own, rest): (Vec<_>, Vec<_>) =
                    loaded.into_iter().partition(|t| t.controller == slot);
                let mut validation =
                    mod_api::world::mod_chunk_tickets::ControllerTickets::new(owner, own.clone());
                self.native_validate_tickets(dim.id(), controller, &mut validation);
                let removed = validation.into_removed();
                loaded = rest;
                loaded.extend(own.into_iter().filter(|t| !removed.contains(t)));
            }
        }
        for ticket in loaded {
            self.streaming.set_mod_forced(ticket, true);
        }
    }
    /// The chunk ticket of `controller` for `owner` on `chunk`
    /// (`TicketController.forceChunk`): true when the controller's tickets
    /// changed. The controller must be the calling mod's own.
    /// A chunk stays forced, entity ticking and saved, and its level keeps
    /// ticking, while any ticket or `/forceload` holds it. With
    /// `natural_spawning` the chunk joins the spawning chunks (inhabited
    /// time, thunder, a spawn attempt) while it lies in the spawning area of a
    /// player (`ChunkMap.collectSpawningChunks`); the local mob cap still lets
    /// mobs spawn only with a non-spectator within 128 blocks. One
    /// owner holds a plain and a natural-spawning ticket on a chunk apart. Tickets are
    /// scoped by (controller, owner, chunk), so two owners in one chunk, or
    /// two controllers, release separately. They are saved in
    /// `data/horizon/mod_chunk_tickets.dat` under the controller's id; when
    /// the world loads, those of a controller no loaded mod registers are
    /// discarded. False outside a mod's hook, for another mod's controller, on
    /// a client, for a chunk outside the world and for a level the server has
    /// not opened.
    pub(super) fn chunk_set_mod_forced(
        &mut self,
        chunk: ChunkRef,
        controller: TicketController,
        owner: mod_api::TicketOwner,
        forced: bool,
        natural_spawning: bool,
    ) -> bool {
        if !calling_mod_owns(controller) {
            return false;
        }
        if self.native_level_streaming(chunk.level()).is_none() || !chunk_in_world(chunk.pos()) {
            return false;
        }
        let [x, z] = chunk.pos();
        let ticket = ModTicket {
            controller: controller.slot(),
            owner,
            chunk: (x, z),
            natural_spawning,
        };
        let dimension = chunk.level().dimension_id();
        let streaming = if dimension == self.dim.id() {
            &mut self.streaming
        } else if let Some(streaming) = self.levels.parked_streaming_of_mut(dimension) {
            streaming
        } else {
            return false;
        };
        streaming.set_mod_forced(ticket, forced)
    }
    /// The chunk tickets `controller` holds in the level, as (owner, chunk
    /// position, natural_spawning), in order. None for a stale handle, on a
    /// client, outside a mod's hook and for another mod's controller.
    pub(super) fn level_mod_forced_chunks(
        &self,
        level: LevelRef,
        controller: TicketController,
    ) -> Option<Vec<(mod_api::TicketOwner, [i32; 2], bool)>> {
        if !calling_mod_owns(controller) {
            return None;
        }
        let tickets = self.native_level_streaming(level)?.mod_tickets();
        Some(
            tickets
                .of_controller(controller.slot())
                .map(|t| (t.owner, [t.chunk.0, t.chunk.1], t.natural_spawning))
                .collect(),
        )
    }
    /// The level's force-loaded chunks (`/forceload query`), sorted as
    /// the command lists them. None for a stale handle or on a client.
    pub(super) fn level_forced_chunks(&self, level: LevelRef) -> Option<Vec<[i32; 2]>> {
        let mut forced: Vec<(i32, i32)> = self
            .native_level_streaming(level)?
            .commanded()
            .iter()
            .copied()
            .collect();
        forced.sort_unstable_by_key(|&(x, z)| ChunkPos::new(x, z).pack());
        Some(forced.into_iter().map(|(x, z)| [x, z]).collect())
    }
    /// The block state ID at `pos` (`Level.getBlockState`), without formatting a string.
    /// None for a chunk that isn't loaded or a `y` outside the world. On a client, the state
    /// the server sent for a column the client holds, of its own level or a viewed one.
    pub(super) fn level_block_state(&self, level: LevelRef, pos: [i32; 3]) -> Option<BlockStateId> {
        let world = self.native_level_view_world(level)?;
        let [x, y, z] = pos;
        world.is_loaded_at(x, y, z).then(|| world.block(x, y, z))
    }
    /// Preview a held tool's transformation without changing the block or stack. The existing
    /// block-tool hook fires with `simulated` true; listeners must not mutate the level in a
    /// simulation. The player must be live in the level in place, and the block must be loaded;
    /// parked levels cannot provide the hook's level context. A stale handle, invalid face,
    /// unavailable player or level, or no transformation returns None.
    pub(super) fn level_block_transform(
        &self,
        level: LevelRef,
        player: PlayerRef,
        hand: Hand,
        stack: &ItemStack,
        pos: [i32; 3],
        hit: BlockHit,
    ) -> Option<BlockTransformPreview> {
        use super::native_server::PlayerRead;
        let world = self.native_level_read_world(level)?;
        if level.dimension() != self.dim.key()
            || !world.is_loaded_at(pos[0], pos[1], pos[2])
            || !matches!(
                hit.face,
                [1, 0, 0] | [-1, 0, 0] | [0, 1, 0] | [0, -1, 0] | [0, 0, 1] | [0, 0, -1]
            )
        {
            return None;
        }
        let (offhand, sneaking) = match self.player_handle_read(player)? {
            PlayerRead::Acting { .. } if self.dim.key() == level.dimension() => {
                (self.offhand.as_ref(), self.keys.held(super::Mapping::Sneak))
            }
            PlayerRead::Guest(guest) if guest.dimension == level.dimension_id() => {
                (guest.offhand.as_ref(), guest.sneaking)
            }
            _ => return None,
        };
        let blocking = !sneaking && offhand.is_some_and(|s| self.blocks_attacks(s).is_some());
        let hit = super::Hit {
            pos,
            face: hit.face,
            point: hit.point,
            inside: hit.inside,
        };
        let (state, sound, loot, durability, particle, _) = self.block_transform_in(
            stack,
            &hit,
            super::TransformContext {
                world,
                player: Some(player.entity().id()),
                hand,
                blocking,
                simulated: true,
            },
        )?;
        use crate::game::api_values::TryIntoApi;
        Some(BlockTransformPreview::new(
            state.try_into_api().ok()?,
            sound,
            loot,
            durability,
            particle,
        ))
    }
    /// `FluidState.getFlow` at `pos`: the current that pushes an entity there, after the
    /// `fluid_flow` hook, as entities feel it. Zero where no fluid flows. None as for
    /// `block_state`.
    pub(super) fn level_fluid_flow(&self, level: LevelRef, pos: [i32; 3]) -> Option<Vec3> {
        let world = self.native_level_read_world(level)?;
        let [x, y, z] = pos;
        world.is_loaded_at(x, y, z).then(|| {
            let flow = horizon_client::entities::fluid_flow(
                world,
                &self.res.motion().fluids,
                &self.loop_context,
                block_pos(pos),
            );
            Vec3::new(flow.x, flow.y, flow.z)
        })
    }
    /// The block at `pos` without its properties: `minecraft:oak_stairs`.
    /// None as for `block_state`. API 381.
    pub(super) fn level_block(&self, level: LevelRef, pos: [i32; 3]) -> Option<mod_api::BlockKey> {
        self.level_block_state(level, pos)
            .and_then(|state| mod_api::__private::block_key(&self.res.facts().get(state).name))
    }
    /// Property `key` of the block state at `pos` (`BlockState.getValue`):
    /// `east` for `facing`. None for a block without it, or as for
    /// `block_state`. API 381.
    pub(super) fn level_block_property(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        key: &str,
    ) -> Option<String> {
        let state = self.level_block_state(level, pos)?;
        self.pack
            .assets
            .registry
            .state(state)?
            .properties
            .get(key)
            .cloned()
    }
    /// Whether this is the level in place: the one the server ticks now, whose block entities
    /// the game's own capabilities read and write and whose capabilities
    /// `LevelRef::capability` looks up. False for a level parked while the server ticks
    /// another (its blocks can still be read), for a handle of an earlier open, and on a
    /// client.
    pub(super) fn level_in_place(&self, level: LevelRef) -> bool {
        self.is_logical_server()
            && self.level_handle_current(level)
            && self.dim.key() == level.dimension()
    }
    /// `f` over the game's own `SignalGetter` for the level in place, as the level's blocks
    /// and mods' parts answer it; `None` for another level, a stale handle or a client.
    /// A shared borrow also lends Parts their restricted read-only level host. Comparator
    /// outputs and moving-piston state come from this level's live tick state.
    fn native_level_signals<R>(
        &self,
        level: LevelRef,
        f: impl FnOnce(
            &horizon_world::level::Signals<'_, super::native_level_signals::ReadLevel<'_, '_>>,
        ) -> R,
    ) -> Option<R> {
        if !self.level_in_place(level) {
            return None;
        }
        let view = super::native_level_signals::ReadLevel { game: self, level };
        Some(f(&self.res.behaviours().signals(&view, &self.block_ticks)))
    }
    /// How strongly the block at `pos` powers the block on its `direction` side, power it
    /// conducts included (`SignalGetter.getSignal(pos, direction)`, as a block reads its
    /// neighbour `pos` that lies `direction` from it), 0 to 15. None for another level, a stale
    /// handle or a client. Takes a shared host and opens no transaction.
    pub(super) fn level_signal(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        direction: Direction,
    ) -> Option<i32> {
        self.native_level_signals(level, |s| s.signal(block_pos(pos), direction))
    }
    /// The strong power alone that the block at `pos` gives that way
    /// (`SignalGetter.getDirectSignal(pos, direction)`), 0 to 15. None as for `signal`.
    pub(super) fn level_direct_signal(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        direction: Direction,
    ) -> Option<i32> {
        self.native_level_signals(level, |s| s.direct_signal(block_pos(pos), direction))
    }
    /// The strongest strong power reaching the block at `pos` from its six neighbours
    /// (`SignalGetter.getDirectSignalTo`), 0 to 15. None as for `signal`.
    pub(super) fn level_direct_signal_to(&self, level: LevelRef, pos: [i32; 3]) -> Option<i32> {
        self.native_level_signals(level, |s| s.direct_signal_to(block_pos(pos)))
    }
    /// The strongest power reaching the block at `pos` from its six neighbours
    /// (`SignalGetter.getBestNeighborSignal`, what a dispenser or a piston reads), 0 to 15.
    /// None as for `signal`.
    pub(super) fn level_best_neighbor_signal(&self, level: LevelRef, pos: [i32; 3]) -> Option<i32> {
        self.native_level_signals(level, |s| s.best_neighbor_signal(block_pos(pos)))
    }
    /// Whether any neighbour powers the block at `pos` (`SignalGetter.hasNeighborSignal`),
    /// asking no side past the first that does. False where `signal` is None.
    pub(super) fn level_has_neighbor_signal(&self, level: LevelRef, pos: [i32; 3]) -> bool {
        self.native_level_signals(level, |s| s.has_neighbor_signal(block_pos(pos)))
            .unwrap_or(false)
    }
    /// Whether `pos` is inside the world's height and its chunk is loaded
    /// (`Level.isLoaded`, which is false outside `isInValidBounds`): a block
    /// read or write there finds the world. False on a client or for a stale
    /// handle: the position is not loaded here. On a client, a chunk it holds is loaded
    /// (`ClientLevel.hasChunkAt`). API 381.
    pub(super) fn level_is_loaded(&self, level: LevelRef, pos: [i32; 3]) -> bool {
        self.native_level_read_world(level)
            .is_some_and(|w| w.is_loaded_at(pos[0], pos[1], pos[2]))
    }
    /// The stored light of one layer at `pos` (`BlockAndLightGetter.getBrightness`), 0 to 15.
    /// None as for `block_state`: a position that isn't loaded has no light to read.
    pub(super) fn level_brightness(
        &self,
        level: LevelRef,
        layer: LightLayer,
        pos: [i32; 3],
    ) -> Option<i32> {
        let world = self.native_level_world(level)?;
        world
            .is_loaded_at(pos[0], pos[1], pos[2])
            .then(|| world_brightness(world, layer, pos))
    }
    /// `BlockAndTintGetter.canSeeSky`: the sky light at `pos` is full. None as for
    /// `level_brightness`.
    pub(super) fn level_can_see_sky(&self, level: LevelRef, pos: [i32; 3]) -> Option<bool> {
        Some(self.level_brightness(level, LightLayer::Sky, pos)? >= 15)
    }
    /// `LevelLightEngine.checkBlock` at `pos`, queued with the level's light edits: the next
    /// update asks a `dynamic_light` block's parts again (`ask_edited_light_emission`) and
    /// relights the cell, which `LightTables::differs` lets through for such a state set to
    /// itself. A check already queued there since the last ask queues nothing, as vanilla's
    /// queue is a set. False off the level in place, for a stale handle, on a client and for a
    /// position that isn't loaded.
    pub(super) fn level_check_light(&mut self, level: LevelRef, pos: [i32; 3]) -> bool {
        if !self.native_level_write_allowed(level, pos) || self.dim.key() != level.dimension() {
            return false;
        }
        let [x, y, z] = pos;
        let edits = &self.world.light_edits;
        if let Some(&i) = self.world.light_checks.get(&(x, y, z))
            && edits.get(i).is_some_and(|edit| edit.0 == (x, y, z))
        {
            return true;
        }
        self.world.light_checks.insert((x, y, z), edits.len());
        let state = self.world.block(x, y, z).0;
        self.world.light_edits.push(((x, y, z), state, state));
        true
    }
    /// The capability generation at `pos`, in an active or parked level. None for a stale
    /// handle, a client, an unloaded position and one outside build height.
    pub(super) fn level_capability_generation(
        &self,
        level: LevelRef,
        [x, y, z]: [i32; 3],
    ) -> Option<u64> {
        let world = self.native_level_world(level)?;
        (world.loaded(x, z) && world.shape.contains_y(y))
            .then(|| world.capability_generations.get((x, y, z)))
    }
    /// Moves the capability generation at `pos` on, at the root commit inside a transaction.
    /// False off the level in place, for a stale handle, on a client and for a position that
    /// isn't loaded.
    pub(super) fn level_invalidate_capabilities(
        &mut self,
        level: LevelRef,
        [x, y, z]: [i32; 3],
    ) -> bool {
        if !self.level_in_place(level)
            || !self.world.loaded(x, z)
            || !self.world.shape.contains_y(y)
        {
            return false;
        }
        self.after_commit(super::transactions::After::Capabilities((x, y, z)));
        true
    }
    /// `LivingEntity.hasLineOfSight`'s clip between two points (collider shapes, no fluids):
    /// whether it misses. None as for `level_clip`.
    pub(super) fn level_line_of_sight(
        &self,
        level: LevelRef,
        from: [f64; 3],
        to: [f64; 3],
    ) -> Option<bool> {
        let hit = self.level_clip(level, from, to, ClipBlock::Collider, ClipFluid::None)?;
        Some(hit.miss)
    }
    /// Whether `level` is a current handle on the server: what the world's own data (its
    /// respawn data and maps, shared by its levels) needs.
    fn native_world_data_allowed(&self, level: LevelRef) -> bool {
        self.is_logical_server() && self.level_handle_current(level)
    }
    /// `LevelData.getRespawnData`: the world spawn and its facing. None on a client, for a
    /// stale handle, and before the world has chosen its spawn.
    pub(super) fn level_respawn_data(&self, level: LevelRef) -> Option<RespawnData> {
        if !self.native_world_data_allowed(level) {
            return None;
        }
        let [yaw, pitch] = self.level.world_spawn_angle;
        Some(RespawnData {
            pos: self.level.world_spawn?,
            yaw,
            pitch,
        })
    }
    /// `/setworldspawn`'s write (`set_world_spawn`), for the Overworld's handle. False for
    /// another level, a stale handle, a client, a non-finite angle and a position outside the
    /// spawnable bounds.
    pub(super) fn level_set_respawn_data(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        yaw: f32,
        pitch: f32,
    ) -> bool {
        if !self.native_world_data_allowed(level)
            || level.dimension() != DimensionKey::OVERWORLD
            || !(yaw.is_finite() && pitch.is_finite())
            || self.level_in_spawnable_bounds(level, pos) != Some(true)
        {
            return false;
        }
        self.set_world_spawn(pos, [yaw, pitch]);
        true
    }
    /// Map `id`'s saved data as it stands (`with_map`). None on a client, for a stale handle
    /// and for a map nothing made.
    pub(super) fn level_map_data(&self, level: LevelRef, id: i32) -> Option<MapState> {
        if !self.native_world_data_allowed(level) {
            return None;
        }
        self.with_map(id, |map| MapState {
            center: map.center,
            scale: map.scale,
            dimension: map.dimension.clone(),
            locked: map.locked,
            tracking: map.tracking,
            unlimited: map.unlimited,
            colors: map.colors.clone(),
        })
    }
    /// Repaint map `id` (`MapData::paint`). False for another length, a stale handle, a
    /// client, a map nothing made and a map whose pixels are all as given.
    pub(super) fn level_set_map_data(&mut self, level: LevelRef, id: i32, colors: &[u8]) -> bool {
        if !self.native_world_data_allowed(level) {
            return false;
        }
        self.map(id)
            .and_then(|map| map.paint(colors))
            .unwrap_or(false)
    }
    /// Queue the level to be saved at the start of the next server tick. False for a stale
    /// handle and on a client.
    pub(super) fn level_request_save(&mut self, level: LevelRef) -> bool {
        if self.native_level_world(level).is_none() {
            return false;
        }
        self.levels
            .request_save(super::Dim::from_id(level.dimension_id()));
        true
    }
    /// The due tick path for the block at `pos` now (`Behaviours::due_block_tick`). False for
    /// a refused write, a stale or parked handle, a client and an unloaded or out-of-height
    /// position.
    pub(super) fn level_block_tick(&mut self, level: LevelRef, pos: [i32; 3]) -> bool {
        if !self.native_level_tick_allowed(level, pos) {
            return false;
        }
        let at = block_pos(pos);
        self.with_level(|b, level, ticks| b.due_block_tick(level, ticks, at));
        let _ = self.process_level_events();
        true
    }
    /// The flow step of the fluid at `pos` now (`Behaviours::scheduled_fluid_tick`, the body of
    /// a due fluid tick). False as for `level_block_tick`.
    pub(super) fn level_fluid_tick(&mut self, level: LevelRef, pos: [i32; 3]) -> bool {
        if !self.native_level_tick_allowed(level, pos) {
            return false;
        }
        let at = block_pos(pos);
        self.with_level(|b, level, ticks| b.scheduled_fluid_tick(level, ticks, at));
        let _ = self.process_level_events();
        true
    }
    /// `tickChunk`'s random tick of `pos` now (`Behaviours::random_tick`). False as for
    /// `level_block_tick`.
    pub(super) fn level_block_random_tick(&mut self, level: LevelRef, pos: [i32; 3]) -> bool {
        if !self.native_level_tick_allowed(level, pos) {
            return false;
        }
        let at = block_pos(pos);
        self.with_level(|b, level, ticks| b.random_tick(level, ticks, at));
        let _ = self.process_level_events();
        true
    }
    /// Whether a tick may run at `pos`: a current handle of the level in place on the server,
    /// a loaded position inside the build height.
    fn native_level_tick_allowed(&self, level: LevelRef, pos: [i32; 3]) -> bool {
        self.native_world_data_allowed(level)
            && self.dim.key() == level.dimension()
            && self.world.loaded(pos[0], pos[2])
            && self.world.shape.contains_y(pos[1])
    }
    /// `ServerLevel.findLightningTargetAround` (`lightning_target`) for the column at `pos`,
    /// in the level in place. None for another level, a stale handle, a client and an
    /// unloaded column.
    pub(super) fn level_lightning_target(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
    ) -> Option<[i32; 3]> {
        if !self.native_world_data_allowed(level) || self.dim.key() != level.dimension() {
            return None;
        }
        self.lightning_target(pos[0], pos[2])
    }
    /// The lowest block `y` and the height of the level
    /// (`LevelHeightAccessor.getMinY`, `getHeight`). Reads any loaded server level
    /// or the client's current level. None for a stale handle, before client login,
    /// or for a dimension unavailable on this side. Does not load chunks. API 381.
    pub(super) fn level_height(&self, level: LevelRef) -> Option<(i32, i32)> {
        self.native_level_read_world(level)
            .map(|w| (w.shape.min_y, w.shape.height))
    }
    /// `Level.isInWorldBounds`: `pos` within 30,000,000 blocks on x and z and inside the level's
    /// build height. None as for `level_height`. Does not load chunks.
    pub(in crate::game) fn level_in_world_bounds(
        &self,
        level: LevelRef,
        pos: [i32; 3],
    ) -> Option<bool> {
        let (min_y, height) = self.level_height(level)?;
        Some(horizontal_in_bounds(pos) && pos[1] >= min_y && pos[1] < min_y.saturating_add(height))
    }
    /// `Level.isInSpawnableBounds`: 30,000,000 blocks on x and z, 20,000,000 on y. None as for
    /// `level_height`. Does not load chunks.
    pub(in crate::game) fn level_in_spawnable_bounds(
        &self,
        level: LevelRef,
        pos: [i32; 3],
    ) -> Option<bool> {
        self.native_level_read_world(level)?;
        Some(horizontal_in_bounds(pos) && (-20_000_000..20_000_000).contains(&pos[1]))
    }
    /// The ticks the server has run (`Level.getGameTime`), shared by every
    /// level as in vanilla. API 381.
    pub(super) fn level_game_time(&self, level: LevelRef) -> Option<i64> {
        self.native_level_read_world(level)?;
        Some(self.world.game_time)
    }
    /// The time of day in ticks (`Level.getOverworldClockTime`), shared by
    /// every level; it keeps counting past 24000. On a client, the synchronized
    /// Overworld clock, advanced at its server-supplied rate, even when the
    /// displayed dimension has a different default clock. API 381.
    pub(super) fn level_day_time(&self, level: LevelRef) -> Option<i64> {
        self.native_level_read_world(level)?;
        if self.is_logical_server() {
            Some(self.command_clock_time("minecraft:overworld"))
        } else {
            self.remote.as_ref().map(|r| r.overworld_clock_time())
        }
    }
    /// The time of the level's own clock (its dimension type's `default_clock`) in ticks: what the
    /// level's sky, spawning and beds go by, and what `/time` sets there. None for a level without
    /// a default clock, a stale handle and a client.
    pub(in crate::game) fn level_clock_time(&self, level: LevelRef) -> Option<i64> {
        self.native_level_read_world(level)?;
        if !self.is_logical_server() {
            return None;
        }
        let dim = super::Dim::from_id(level.dimension_id());
        self.dimension_type_of(dim).default_clock.as_ref()?;
        Some(self.command_clock_time(&self.level_clock_name(dim)))
    }
}

/// `Level.isInWorldBoundsHorizontal`: x and z inside the 30,000,000-block limit.
fn horizontal_in_bounds(pos: [i32; 3]) -> bool {
    (-30_000_000..30_000_000).contains(&pos[0]) && (-30_000_000..30_000_000).contains(&pos[2])
}

#[cfg(test)]
mod tests {
    use super::super::api_values::TryIntoApi;
    use super::super::{Dim, native_entity_refs::tests::with_game};
    use horizon_core::position::BlockPos;
    use mod_api::{ChunkRef, LevelRef, LightLayer, WorldEpoch};

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn review_climate_literals() {
        with_game(|game| {
            let level = game.level("minecraft:overworld").unwrap();
            for (name, temperature, downfall, precipitation, frozen) in [
                ("plains", 0.8, 0.4, true, false),
                ("desert", 2.0, 0.0, false, false),
                ("snowy_plains", 0.0, 0.5, true, false),
                ("ocean", 0.5, 0.5, true, false),
                ("frozen_ocean", 0.0, 0.5, true, true),
            ] {
                let actual = game
                    .level_biome_properties(level, &format!("minecraft:{name}"))
                    .unwrap();
                assert_eq!(
                    actual,
                    mod_api::BiomeProperties {
                        temperature,
                        downfall,
                        has_precipitation: precipitation,
                        frozen
                    },
                    "{name}"
                );
            }
            for id in ["minecraft:no_such_biome", "", "!malformed::id"] {
                assert_eq!(game.level_biome_properties(level, id), None);
            }
            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            assert_eq!(game.level_biome_properties(stale, "minecraft:plains"), None);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_level_with_its_own_clock_counts_apart_from_the_overworlds() {
        with_game(|game| {
            game.world.day_time = 100;
            game.enter_level(Dim::End).unwrap();
            assert_eq!(game.active_clock(), "minecraft:the_end");
            assert_eq!(game.world.day_time, 0, "the End's clock has its own total");
            game.world.day_time = 50;
            assert_eq!(game.command_clock_time("minecraft:overworld"), 100);
            game.enter_level(Dim::Nether).unwrap();
            assert_eq!(
                game.world.day_time, 100,
                "a level without a clock follows the Overworld's"
            );
            assert_eq!(game.command_clock_time("minecraft:the_end"), 50);
            game.enter_level(Dim::Overworld).unwrap();
            assert_eq!(game.world.day_time, 100);
        });
    }

    #[test]
    fn biome_search_budget_bounds_columns_and_heights_before_sampling() {
        use mod_api::BiomeSearch;
        let allowed = |radius, horizontal, vertical, height| {
            super::biome_search_within_budget(
                BiomeSearch::new([i32::MIN, i32::MAX, i32::MAX], radius, horizontal, vertical),
                height,
            )
        };
        assert!(allowed(0, 1, i32::MAX, 384));
        assert!(allowed(i32::MAX, i32::MAX, i32::MAX, 384));
        assert!(allowed(127, 1, i32::MAX, 256));
        assert!(!allowed(128, 1, i32::MAX, 256));
        assert!(allowed(15, 1, 4, 256));
        assert!(!allowed(16, 1, 4, 256));
        assert!(!allowed(i32::MAX, 1, 1, i32::MAX));
        for (radius, horizontal, vertical, height) in [
            (-1, 1, 1, 384),
            (0, 0, 1, 384),
            (0, -1, 1, 384),
            (0, 1, 0, 384),
            (0, 1, -1, 384),
            (0, 1, 1, 0),
        ] {
            assert!(!allowed(radius, horizontal, vertical, height));
        }
    }

    /// A logical server with its worker queue running rather than the
    /// title-screen streamer's paused placeholder.
    fn with_chunk_streaming(test: impl FnOnce(&mut super::Game<'_>)) {
        with_game(|game| {
            game.options.headless = true;
            game.streaming.paused = false;
            game.streaming.mesh = false;
            game.streaming.radius = 0;
            test(game);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn block_transform_preview_preserves_state_and_marks_simulation() {
        use super::super::native_guest_turn_tests::with_guests;
        use horizon_client::{chunk::Chunk, items::ItemStack};
        use mod_api::{BlockHit, BlockToolModificationEvent, Flow, Hand, Hooks, Host};
        use std::sync::{Arc, Mutex};

        struct Observer(Arc<Mutex<Vec<bool>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Observer {
            fn block_tool_modification(
                &self,
                _: &dyn Host,
                event: &mut BlockToolModificationEvent<'_>,
            ) -> Flow {
                self.0.lock().unwrap().push(event.tool.simulated);
                Flow::Continue
            }
        }
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![mod_api::ModEntry::new(
            "preview_test",
            Box::new(Observer(heard.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_guests(|game| {
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let pos = [0, 64, 0];
                let log = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:oak_log")
                    .unwrap();
                game.world.set(pos[0], pos[1], pos[2], log).unwrap();
                let tool = ItemStack::new("minecraft:iron_axe", 1);
                {
                    let value = Some(tool.clone());
                    game.lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .update(0, |g| g.inventory[0] = value);
                }
                let player = game.player(12).unwrap();
                let level = game.level(game.dim.id()).unwrap();
                let hit = BlockHit::new([0.5, 65.0, 0.5], [0, 1, 0], false);
                let preview = level
                    .block_transform(
                        game,
                        player,
                        Hand::Main,
                        &tool.try_into_api().unwrap(),
                        pos,
                        hit,
                    )
                    .unwrap();
                let stripped = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stripped_oak_log")
                    .unwrap();
                assert_eq!(preview.state, stripped.try_into_api().unwrap());
                assert_eq!(preview.durability, 1);
                assert_eq!(game.world.block(pos[0], pos[1], pos[2]), log);
                assert_eq!(
                    game.lan.as_ref().unwrap().hub.guests[0].inventory[0],
                    Some(tool.clone())
                );
                assert!(
                    level
                        .block_transform(
                            game,
                            player,
                            Hand::Main,
                            &tool.try_into_api().unwrap(),
                            [16, 64, 0],
                            hit
                        )
                        .is_none()
                );
                game.as_guest(12, |game| {
                    let hit = super::super::Hit {
                        pos,
                        face: hit.face,
                        point: hit.point,
                        inside: hit.inside,
                    };
                    assert!(game.use_block_transformer(&tool, &hit).unwrap());
                })
                .unwrap();
                assert_eq!(game.world.block(pos[0], pos[1], pos[2]), stripped);
                assert_eq!(
                    game.lan.as_ref().unwrap().hub.guests[0].inventory[0]
                        .as_ref()
                        .unwrap()
                        .damage,
                    1
                );
            })
        });
        assert_eq!(*heard.lock().unwrap(), vec![true, false]);
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn shared_level_signal_dispatches_and_clamps_part_output() {
        use horizon_client::chunk::Chunk;
        use horizon_core::geometry::Direction;
        use mod_api::Host;
        use std::sync::Arc;

        mod_platform::__private::part_tests::signal_game_thread();
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let y = game.world.bottom_y() + 1;
            let glass = game
                .pack
                .assets
                .registry
                .default_state("minecraft:glass")
                .unwrap();
            game.world.set(1, y, 1, glass).unwrap();
            let level = game.level(game.dim.id()).unwrap();
            let host: &dyn Host = game;
            assert_eq!(level.signal(host, [1, y, 1], Direction::East), Some(15));
            assert_eq!(
                level.direct_signal(host, [1, y, 1], Direction::East),
                Some(0)
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn shared_level_signal_reads_live_comparator_and_refuses_other_levels() {
        use horizon_client::chunk::Chunk;
        use horizon_core::geometry::Direction;
        use mod_api::Host;
        use std::sync::Arc;

        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let y = game.world.bottom_y() + 1;
            let registry = &game.pack.assets.registry;
            let redstone = registry.default_state("minecraft:redstone_block").unwrap();
            game.world.set(1, y, 1, redstone).unwrap();
            let comparator = registry.default_state("minecraft:comparator").unwrap();
            let mut comparator_state = registry.state(comparator).unwrap().clone();
            comparator_state
                .properties
                .insert("powered".into(), "true".into());
            comparator_state
                .properties
                .insert("facing".into(), "east".into());
            let comparator = registry.resolve(&comparator_state).unwrap();
            game.world.set(4, y, 4, comparator).unwrap();
            game.block_ticks
                .comparator_outputs
                .insert(BlockPos::new(4, y, 4), 9);
            let level = game.level(game.dim.id()).unwrap();
            let host: &dyn Host = game;
            assert_eq!(level.signal(host, [1, y, 1], Direction::East), Some(15));
            assert_eq!(level.best_neighbor_signal(host, [2, y, 1]), Some(15));
            assert!(level.has_neighbor_signal(host, [2, y, 1]));
            assert_eq!(level.signal(host, [4, y, 4], Direction::East), Some(9));
            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            let other = LevelRef::new(level.epoch(), mod_api::DimensionKey::NETHER);
            for unavailable in [stale, other] {
                assert_eq!(unavailable.signal(host, [1, y, 1], Direction::East), None);
                assert!(!unavailable.has_neighbor_signal(host, [2, y, 1]));
            }
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn respawn_data_maps_and_ticks_write_through_the_handle_and_refuse_others() {
        use horizon_client::chunk::Chunk;
        use std::sync::Arc;

        with_game(|game| {
            let level = game.level("minecraft:overworld").unwrap();
            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            let nether = LevelRef::new(level.epoch(), mod_api::DimensionKey::NETHER);

            // RespawnData.of: the yaw wraps to -180..180 and the pitch clamps to -90..90.
            assert!(level.set_respawn_data(game, [10, 70, -4], 270.0, 120.0));
            let data = level.respawn_data(game).unwrap();
            assert_eq!(data.pos, [10, 70, -4]);
            assert_eq!((data.yaw, data.pitch), (-90.0, 90.0));
            assert_eq!(
                game.level.world_spawn,
                Some([10, 70, -4]),
                "level.dat's spawn"
            );
            assert!(
                !nether.set_respawn_data(game, [0, 64, 0], 0.0, 0.0),
                "Overworld only"
            );
            assert!(!level.set_respawn_data(game, [0, 64, 0], f32::NAN, 0.0));
            assert!(!level.set_respawn_data(game, [30_000_001, 64, 0], 0.0, 0.0));
            assert!(!stale.set_respawn_data(game, [0, 64, 0], 0.0, 0.0));
            assert_eq!(stale.respawn_data(game), None);
            assert_eq!(
                level.respawn_data(game).unwrap().pos,
                [10, 70, -4],
                "refusals change nothing"
            );

            // A map repainted through the handle reads back and is marked to save and redraw.
            let fresh = crate::game::maps::MapData::fresh(0.0, 0.0, 0, "minecraft:overworld");
            let id = game.new_map_id(fresh);
            let mut colors = level.map_data(game, id).unwrap().colors;
            assert_eq!(colors.len(), 128 * 128);
            colors[5 + 2 * 128] = 34;
            assert!(level.set_map_data(game, id, &colors));
            assert!(
                !level.set_map_data(game, id, &colors),
                "nothing left to change"
            );
            assert!(!level.set_map_data(game, id, &colors[..10]));
            let read = level.map_data(game, id).unwrap();
            assert_eq!(read.colors[5 + 2 * 128], 34);
            assert_eq!((read.center, read.scale), ([0, 0], 0));
            assert_eq!(stale.map_data(game, id), None);
            assert!(!stale.set_map_data(game, id, &colors));
            assert_eq!(level.map_data(game, i32::MAX), None, "a map nothing made");

            // Ticks run only in the level in place, at a loaded position inside its height.
            let height = level.height(game).unwrap();
            assert!(!level.block_tick(game, [0, height.0, 0]));
            assert!(!level.block_random_tick(game, [0, height.0, 0]));
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            assert!(level.block_tick(game, [0, height.0, 0]));
            assert!(level.block_random_tick(game, [0, height.0, 0]));
            assert!(
                !level.block_tick(game, [0, height.0 - 1, 0]),
                "below the world"
            );
            assert!(
                !level.block_random_tick(game, [1_000_000, 64, 1_000_000]),
                "unloaded"
            );
            assert!(!stale.block_tick(game, [0, height.0, 0]));
            assert!(
                !nether.block_random_tick(game, [0, height.0, 0]),
                "a level not in place"
            );
            assert_eq!(stale.can_see_sky(game, [0, height.0, 0]), None);
            assert_eq!(stale.line_of_sight(game, [0.5; 3], [4.5, 0.5, 0.5]), None);
            assert_eq!(stale.lightning_target(game, [0, 64, 0]), None);
            assert_eq!(nether.lightning_target(game, [0, 64, 0]), None);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn loaded_chunks_lists_the_level_in_order_without_loading() {
        use horizon_client::chunk::Chunk;
        use std::sync::Arc;

        with_game(|game| {
            let level = game.level("minecraft:overworld").unwrap();
            let before = level.loaded_chunks(game).unwrap().len();
            for pos in [(2, -1), (0, 5), (0, -3)] {
                game.world
                    .chunks
                    .insert(pos, Arc::new(Chunk::empty(pos.0, pos.1)));
            }
            let chunks = level.loaded_chunks(game).unwrap();
            assert_eq!(chunks.len(), before + 3);
            let positions: Vec<[i32; 2]> = chunks.iter().map(|chunk| chunk.pos()).collect();
            assert!(
                positions.windows(2).all(|pair| pair[0] <= pair[1]),
                "ordered"
            );
            assert!(positions.contains(&[0, -3]) && positions.contains(&[2, -1]));
            let nether = LevelRef::new(level.epoch(), mod_api::DimensionKey::NETHER);
            assert_eq!(nether.loaded_chunks(game), None, "not opened yet");
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn bounds_reads_follow_the_level_without_loading_chunks() {
        with_game(|game| {
            let level = game.level("minecraft:overworld").unwrap();
            let nether = LevelRef::new(level.epoch(), mod_api::DimensionKey::NETHER);
            assert_eq!(level.in_world_bounds(game, [0, 64, 0]), Some(true));
            assert_eq!(level.in_world_bounds(game, [0, -65, 0]), Some(false));
            assert_eq!(level.in_world_bounds(game, [0, 320, 0]), Some(false));
            assert_eq!(
                level.in_world_bounds(game, [30_000_000, 64, 0]),
                Some(false)
            );
            assert_eq!(level.in_world_bounds(game, [0, i32::MAX, 0]), Some(false));
            assert_eq!(level.in_spawnable_bounds(game, [0, 320, 0]), Some(true));
            assert_eq!(
                level.in_spawnable_bounds(game, [29_999_999, 19_999_999, -30_000_000]),
                Some(true)
            );
            assert_eq!(
                level.in_spawnable_bounds(game, [0, 20_000_000, 0]),
                Some(false)
            );
            assert_eq!(
                nether.in_world_bounds(game, [0, 64, 0]),
                None,
                "not opened yet"
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn world_border_reads_refuse_stale_and_unopened_levels() {
        with_game(|game| {
            let level = game.level("minecraft:overworld").unwrap();
            game.border.set_center([12.0, -8.0]);
            game.border.set_size(40.0);
            assert_eq!(level.world_border(game), Some([-8.0, -28.0, 32.0, 12.0]));
            let stale_epoch = WorldEpoch::new(level.epoch().get() + 1);
            let stale = LevelRef::new(stale_epoch, level.dimension());
            assert_eq!(stale.world_border(game), None);
            let unopened = LevelRef::new(level.epoch(), mod_api::DimensionKey::NETHER);
            assert_eq!(unopened.world_border(game), None);
            game.border.set_center([0.0, 0.0]);
            assert_eq!(level.world_border(game), Some([-20.0, -20.0, 20.0, 20.0]));
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn height_reads_server_levels_and_the_joined_client_level() {
        use super::super::execution_role::ExecutionRole;
        use horizon_client::chunk::LevelShape;
        use horizon_protocol::connection::Pipe;
        use mod_api::Host;

        with_game(|game| {
            let overworld = game.level("minecraft:overworld").unwrap();
            let nether = LevelRef::new(overworld.epoch(), mod_api::DimensionKey::NETHER);
            assert_eq!(overworld.height(game), Some((-64, 384)));
            assert_eq!(nether.height(game), None, "not opened yet");
            game.enter_level(Dim::Nether).unwrap();
            assert_eq!(nether.height(game), Some((0, 256)));
            assert_eq!(overworld.height(game), Some((-64, 384)), "parked level");
            game.enter_level(Dim::Overworld).unwrap();

            game.options.execution_role = ExecutionRole::Client;
            game.options.dedicated = None;
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "HeightReadTest").unwrap();
            assert!(game.level("minecraft:overworld").is_none());
            assert_eq!(overworld.height(game), None, "not logged in");
            game.remote.as_mut().unwrap().logged_in = true;
            game.world.shape = LevelShape::new(-128, 512, 512, true, false).unwrap();
            let level = game.level("minecraft:overworld").unwrap();
            let host: &dyn Host = game;
            assert_eq!(level.height(host), Some((-128, 512)), "client's own shape");
            assert_eq!(
                nether.height(host),
                None,
                "parked server level is not visible"
            );
            assert!(host.level("minecraft:the_nether").is_none());
            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            assert_eq!(stale.height(host), None);
            game.remote.as_mut().unwrap().logged_in = false;
            assert_eq!(level.height(game), None, "logged out");
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn block_state_reads_stored_ids_and_refuses_unloaded_positions() {
        use horizon_client::chunk::Chunk;
        use std::sync::Arc;

        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level(game.dim.id()).unwrap();
            let bottom = game.world.bottom_y();
            let top = game.world.top_y();
            let registry = &game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let stairs = registry.default_state("minecraft:oak_stairs").unwrap();
            let air = registry.default_state("minecraft:air").unwrap();
            for (pos, expected) in [([0, bottom, 0], stone), ([15, top - 1, 15], stairs)] {
                game.world.set(pos[0], pos[1], pos[2], expected).unwrap();
                assert_eq!(
                    level.block_state(game, pos),
                    Some(expected.try_into_api().unwrap())
                );
            }
            assert_eq!(
                level.block_state(game, [1, bottom, 1]),
                Some(air.try_into_api().unwrap())
            );
            assert_eq!(level.block_state(game, [16, bottom, 0]), None);
            assert_eq!(level.block_state(game, [0, bottom - 1, 0]), None);
            assert_eq!(level.block_state(game, [0, top, 0]), None);
        });
    }

    /// `FlowingFluid.getFlow`: water one level below a source beside it flows away from the
    /// source, normalized; so does the source, toward it; a source with only air on stone
    /// around it, and air, have none.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn fluid_flow_reads_the_current_that_pushes_entities() {
        use horizon_client::chunk::Chunk;
        use mod_api::Vec3;
        use std::sync::Arc;

        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level(game.dim.id()).unwrap();
            let registry = &game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let water = registry.default_state("minecraft:water").unwrap();
            let mut state = registry.state(water).unwrap().clone();
            state.properties.insert("level".into(), "1".into());
            let flowing = registry.resolve(&state).unwrap();
            let y = game.world.bottom_y() + 1;
            for x in 0..16 {
                for z in 0..16 {
                    game.world.set(x, y - 1, z, stone).unwrap();
                }
            }
            game.world.set(1, y, 1, water).unwrap();
            game.world.set(2, y, 1, flowing).unwrap();
            game.world.set(8, y, 8, water).unwrap();
            let east = Some(Vec3::new(1.0, 0.0, 0.0));
            assert_eq!(level.fluid_flow(game, [2, y, 1]), east);
            assert_eq!(level.fluid_flow(game, [1, y, 1]), east);
            assert_eq!(level.fluid_flow(game, [8, y, 8]), Some(Vec3::ZERO));
            assert_eq!(level.fluid_flow(game, [5, y, 5]), Some(Vec3::ZERO));
            assert_eq!(level.fluid_flow(game, [16, y, 0]), None);
        });
    }

    /// A stone floor under chunk (0, 0) at `y - 1` and the standing level `y` returned, with
    /// `fluids` placed `dy` blocks above it: each a block id and its `level` property (0 is a
    /// source).
    fn pool(game: &mut super::super::Game<'_>, fluids: &[([i32; 3], &str, &str)]) -> i32 {
        use horizon_client::chunk::Chunk;
        use std::sync::Arc;

        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let registry = game.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        let y = game.world.bottom_y() + 1;
        for x in 0..16 {
            for z in 0..16 {
                game.world.set(x, y - 1, z, stone).unwrap();
            }
        }
        for &([x, dy, z], name, level) in fluids {
            let mut state = registry
                .state(registry.default_state(name).unwrap())
                .unwrap()
                .clone();
            state.properties.insert("level".into(), level.into());
            game.world
                .set(x, y + dy, z, registry.resolve(&state).unwrap())
                .unwrap();
        }
        y
    }

    /// Vanilla's fluid facts at a position, each against a value worked out by hand: water's 5
    /// and lava's 30 ticks (10 where lava flows fast), an empty fluid's 0, a source 8/9 high
    /// under air and a full block under water, flowing water of level 5 a third high, the
    /// camera's full block under a sturdy ceiling, and what water and lava replace.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn fluid_queries_answer_vanilla_values_at_a_position() {
        use mod_api::{Direction, Host};

        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let y = pool(
                game,
                &[
                    ([1, 0, 1], "minecraft:water", "0"),
                    ([1, 0, 4], "minecraft:water", "0"),
                    ([1, 1, 4], "minecraft:water", "0"),
                    ([1, 0, 7], "minecraft:water", "5"),
                    ([1, 0, 10], "minecraft:lava", "0"),
                    ([1, 0, 12], "minecraft:lava", "5"),
                ],
            );
            let stone = game.pack.assets.registry.default_state("minecraft:stone");
            game.world.set(1, y + 1, 1, stone.unwrap()).unwrap();
            let (water, flowing_water, lava, flowing_lava, empty) = {
                let host: &dyn Host = game;
                let key = |id| host.fluid_key(id).unwrap();
                (
                    key("minecraft:water"),
                    key("minecraft:flowing_water"),
                    key("minecraft:lava"),
                    key("minecraft:flowing_lava"),
                    key("minecraft:empty"),
                )
            };
            let tank_only = game.fluid_key("horizon:definition_test").unwrap();
            let host: &dyn Host = game;
            let at = |x, y, z| [x, y, z];

            // `Fluid.getTickDelay`: water 5 and lava 30 in the Overworld, a flowing form as its source.
            for (fluid, delay) in [
                (water, 5),
                (flowing_water, 5),
                (lava, 30),
                (flowing_lava, 30),
                (empty, 0),
            ] {
                assert_eq!(
                    level.fluid_tick_delay(host, fluid, at(1, y, 1)),
                    Some(delay)
                );
            }
            assert_eq!(level.fluid_tick_delay(host, tank_only, at(1, y, 1)), None);
            assert_eq!(level.fluid_tick_delay(host, water, at(16, y, 0)), None);
            let height = level.height(host).unwrap();
            assert_eq!(
                level.fluid_tick_delay(host, water, at(1, height.0, 1)),
                Some(5)
            );
            assert_eq!(
                level.fluid_tick_delay(host, water, at(1, height.0 - 1, 1)),
                None
            );

            // The surface: 8/9 under air, a full block under the same fluid, `amount / 9` flowing.
            let ninth = |n: f32| n / 9.0;
            assert_eq!(level.fluid_height(host, at(1, y, 4)), Some(1.0));
            assert_eq!(level.fluid_height(host, at(1, y + 1, 4)), Some(ninth(8.0)));
            assert_eq!(level.fluid_height(host, at(1, y, 7)), Some(ninth(3.0)));
            assert_eq!(level.fluid_height(host, at(1, y, 9)), Some(0.0));
            assert_eq!(level.fluid_height(host, at(16, y, 0)), None);
            // The camera sees a source under a sturdy block as full, and nothing else differently.
            assert_eq!(level.fluid_height_for_camera(host, at(1, y, 1)), Some(1.0));
            assert_eq!(level.fluid_height(host, at(1, y, 1)), Some(ninth(8.0)));
            assert_eq!(
                level.fluid_height_for_camera(host, at(1, y + 1, 4)),
                Some(ninth(8.0))
            );
            assert_eq!(
                level.fluid_height_for_camera(host, at(1, y, 7)),
                Some(ninth(3.0))
            );

            // Water gives way to lava flowing down only, lava to water from 4/9 of a block high.
            let replaced = |fluid, x, z, direction| {
                level.fluid_can_be_replaced_with(host, at(x, y, z), fluid, direction)
            };
            assert_eq!(replaced(lava, 1, 1, Direction::Down), Some(true));
            assert_eq!(replaced(lava, 1, 1, Direction::East), Some(false));
            assert_eq!(replaced(water, 1, 1, Direction::Down), Some(false));
            assert_eq!(replaced(flowing_water, 1, 10, Direction::East), Some(true));
            assert_eq!(replaced(flowing_water, 1, 12, Direction::East), Some(false));
            assert_eq!(replaced(lava, 1, 10, Direction::East), Some(false));
            assert_eq!(replaced(water, 1, 9, Direction::Up), Some(true));
            assert_eq!(replaced(tank_only, 1, 9, Direction::Up), Some(false));
            assert_eq!(
                level.fluid_can_be_replaced_with(host, at(16, y, 0), water, Direction::Down),
                None
            );
        });
    }

    /// Lava flows fast in the Nether: the delay follows the level's own dimension.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn lava_ticks_every_ten_in_a_fast_lava_level() {
        use mod_api::Host;

        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let y = pool(game, &[]);
            game.world.fast_lava = true;
            let host: &dyn Host = game;
            let delay = |id| level.fluid_tick_delay(host, host.fluid_key(id).unwrap(), [1, y, 1]);
            assert_eq!(delay("minecraft:lava"), Some(10));
            assert_eq!(delay("minecraft:flowing_lava"), Some(10));
            assert_eq!(delay("minecraft:water"), Some(5));
        });
    }

    /// An explicit fluid tick runs the flow step: a water source on a floor fills its four
    /// neighbours with flowing water of level 1 (amount 7), once; an empty cell, an unloaded
    /// column, a position below the world, a stale handle and a level not in place do nothing.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn an_explicit_fluid_tick_spreads_a_source_and_refuses_other_positions() {
        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            let nether = LevelRef::new(level.epoch(), mod_api::DimensionKey::NETHER);
            let y = pool(game, &[([8, 0, 8], "minecraft:water", "0")]);
            let amounts = |game: &mut super::super::Game<'_>| {
                [[9, 8], [7, 8], [8, 9], [8, 7], [8, 8]]
                    .map(|[x, z]| level.fluid_state(game, [x, y, z]).map(|f| f.amount))
            };
            assert_eq!(amounts(game), [Some(0), Some(0), Some(0), Some(0), Some(8)]);
            assert!(level.fluid_tick(game, [8, y, 8]));
            assert_eq!(amounts(game), [Some(7), Some(7), Some(7), Some(7), Some(8)]);
            let flowing = level.fluid_state(game, [9, y, 8]).unwrap();
            assert!(!flowing.is_source() && !flowing.falling);

            // Ticking the empty fluid is allowed and changes nothing.
            assert!(level.fluid_tick(game, [3, y, 3]));
            assert_eq!(
                level.fluid_state(game, [3, y, 3]).map(|f| f.amount),
                Some(0)
            );
            assert!(
                !level.fluid_tick(game, [1_000_000, y, 1_000_000]),
                "unloaded"
            );
            let below = game.world.bottom_y() - 1;
            assert!(!level.fluid_tick(game, [8, below, 8]), "below the world");
            assert!(!stale.fluid_tick(game, [8, y, 8]));
            assert!(!nether.fluid_tick(game, [8, y, 8]), "a level not in place");
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn forced_chunks_set_clear_and_query_like_forceload() {
        with_chunk_streaming(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let far = game.level_chunk_ref(level, [40, -7]).unwrap();
            let near = game.level_chunk_ref(level, [-3, 2]).unwrap();
            assert_eq!(game.chunk_forced(far), Some(false));
            assert_eq!(game.level_forced_chunks(level), Some(vec![]));
            // A synchronous load leaves existing columns for the normal owner
            // update to save and unload, including their entity data.
            let old = std::sync::Arc::new(horizon_client::chunk::Chunk::empty(-40, 40));
            game.world.chunks.insert((-40, 40), old.clone());
            game.world.modified.insert((-40, 40));
            assert!(!game.chunk_loaded(far));
            assert!(game.chunk_set_forced(far, true));
            assert!(game.chunk_loaded(far), "loaded before returning");
            assert!(std::sync::Arc::ptr_eq(&old, &game.world.chunks[&(-40, 40)]));
            assert!(game.world.modified.contains(&(-40, 40)));
            assert!(!game.chunk_set_forced(far, true), "already forced");
            assert!(game.chunk_set_forced(near, true));
            assert_eq!(game.chunk_forced(far), Some(true));
            // The command's own set and order.
            assert!(game.streaming.forced.contains(&(40, -7)));
            let listed = game.level_forced_chunks(level).unwrap();
            // Signed packed positions: z first, then x.
            assert_eq!(listed, vec![[40, -7], [-3, 2]]);
            assert!(game.chunk_set_forced(far, false));
            assert!(!game.chunk_set_forced(far, false), "already clear");
            assert_eq!(game.level_forced_chunks(level), Some(vec![[-3, 2]]));
            assert!(!game.streaming.forced.contains(&(40, -7)));
            // Outside the world.
            // Unchanged adds and removals do not load a missing column.
            let pending = game.level_chunk_ref(level, [200, 200]).unwrap();
            assert!(game.streaming.set_forced(200, 200, true));
            assert!(!game.chunk_loaded(pending));
            assert!(!game.chunk_set_forced(pending, true));
            assert!(!game.chunk_loaded(pending));
            assert!(game.chunk_set_forced(pending, false));
            assert!(!game.chunk_loaded(pending));
            assert!(game.level_chunk_ref(level, [1_875_000, 0]).is_none());
            assert!(game.level_chunk_ref(level, [0, -1_875_001]).is_none());
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn forced_chunk_changes_the_ticket_sources_and_the_saved_set() {
        with_chunk_streaming(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let chunk = game.level_chunk_ref(level, [5, 6]).unwrap();
            assert!(game.chunk_set_forced(chunk, true));
            assert!(game.levels_to_tick().contains(&game.dim));
            game.save_chunk_tickets().unwrap();
            assert!(game.save.load_forced_chunks().contains(&(5, 6)));
            assert!(game.chunk_set_forced(chunk, false));
            game.save_chunk_tickets().unwrap();
            assert!(game.save.load_forced_chunks().is_empty());
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn forced_chunks_of_a_parked_level_and_stale_handles() {
        with_chunk_streaming(|game| {
            let stale_epoch = WorldEpoch::new(game.native_lifecycle.epoch().unwrap().get() + 1);
            let level = game.level(game.dim.id()).unwrap();
            let stale_level = mod_api::LevelRef::new(stale_epoch, level.dimension());
            let stale = ChunkRef::new(stale_level, [0, 0]);
            assert_eq!(game.chunk_forced(stale), None);
            assert!(!game.chunk_set_forced(stale, true));
            assert_eq!(game.level_forced_chunks(stale_level), None);
            assert!(game.level_chunk_ref(stale_level, [0, 0]).is_none());
            // A level the server has not opened.
            let other = if game.dim == Dim::Nether {
                Dim::End
            } else {
                Dim::Nether
            };
            let parked = mod_api::LevelRef::new(level.epoch(), other.key());
            let chunk = ChunkRef::new(parked, [1, 1]);
            assert_eq!(game.chunk_forced(chunk), None);
            assert!(!game.chunk_set_forced(chunk, true));
            // Opened, then parked.
            let back = game.dim;
            game.enter_level(other).unwrap();
            game.enter_level(back).unwrap();
            assert_eq!(game.chunk_forced(chunk), Some(false));
            assert!(!game.chunk_loaded(chunk));
            assert!(game.chunk_set_forced(chunk, true));
            assert!(
                game.chunk_loaded(chunk),
                "parked level loaded before returning"
            );
            assert_eq!(game.dim, back, "forcing must not switch the acting level");
            assert_eq!(game.level_forced_chunks(parked), Some(vec![[1, 1]]));
            assert_eq!(game.level_forced_chunks(level), Some(vec![]));
            assert!(game.levels_to_tick().contains(&other));
            assert!(game.chunk_set_forced(chunk, false));
            assert!(!game.levels_to_tick().contains(&other));
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_level_ref_draws_from_its_levels_own_stream() {
        use horizon_core::random::RandomSource;
        use mod_api::{LevelRef, ServerHost};

        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            // The level in place: the draws advance `World.random` itself.
            let mut expected = game.world.random.clone();
            let host: &mut dyn ServerHost = game;
            let drawn: Vec<i32> = (0..3)
                .map(|_| level.with_random(&mut *host, |r| r.next_int()).unwrap())
                .collect();
            let want: Vec<i32> = (0..3).map(|_| expected.next_int()).collect();
            assert_eq!(drawn, want);
            assert_eq!(game.world.random.next_int(), expected.next_int());

            // A parked level answers with its own stream, not the one in place.
            let other = if game.dim == Dim::Nether {
                Dim::End
            } else {
                Dim::Nether
            };
            let parked = LevelRef::new(level.epoch(), other.key());
            let host: &mut dyn ServerHost = game;
            assert!(parked.random(&*host).is_none(), "not open yet");
            let back = game.dim;
            game.enter_level(other).unwrap();
            game.enter_level(back).unwrap();
            let mut expected = game
                .levels
                .parked_world_of(other.id())
                .unwrap()
                .random
                .clone();
            let in_place = game.world.random.clone();
            let host: &mut dyn ServerHost = game;
            let drawn = parked.with_random(&mut *host, |r| r.next_long()).unwrap();
            assert_eq!(drawn, expected.next_long());
            let mut after = game
                .levels
                .parked_world_of(other.id())
                .unwrap()
                .random
                .clone();
            assert_eq!(after.next_long(), expected.next_long());
            let mut untouched = in_place;
            assert_eq!(game.world.random.next_long(), untouched.next_long());

            // A handle of an earlier open names nothing.
            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), game.dim.key());
            let host: &mut dyn ServerHost = game;
            assert!(stale.with_random(&mut *host, |r| r.next_int()).is_none());
        });
    }

    fn in_mod(id: &'static str) -> mod_api::content::crash::ModGuard {
        mod_platform::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
            id,
            hook: "test",
            part: None,
        })
    }

    use horizon_client::mod_chunk_tickets::{ModTicket, TicketOwner};
    use mod_api::{
        self, Hooks, ModEntry, Registrar, ServerHost, TicketController, ValidateTicketsEvent,
        testing,
    };

    /// Registers three controllers: `quarry` and `drill` validate, `plain` doesn't. `quarry`'s
    /// validation prunes the block at [1, 64, 1]; each validation tells what it was shown.
    struct Machines;
    #[mod_api::__subscriptions]
    impl Hooks for Machines {
        fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            r.register_ticket_controller("quarry", true);
            r.register_ticket_controller("machines:drill", true);
            r.register_ticket_controller("plain", false);
        }
        fn validate_tickets(&self, _: &mut dyn ServerHost, e: &mut ValidateTicketsEvent<'_>) {
            SEEN.with_borrow_mut(|seen| {
                seen.push((
                    e.controller.id(),
                    e.validation
                        .block_tickets()
                        .into_iter()
                        .map(|(pos, _)| pos)
                        .collect(),
                ))
            });
            if e.controller.id() == "machines:quarry" {
                e.validation.remove_block_tickets([1, 64, 1]);
            }
        }
    }
    /// Registers one validating controller, and prunes everything it is shown.
    struct Other;
    #[mod_api::__subscriptions]
    impl Hooks for Other {
        fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            r.register_ticket_controller("vault", true);
        }
        fn validate_tickets(&self, _: &mut dyn ServerHost, e: &mut ValidateTicketsEvent<'_>) {
            SEEN.with_borrow_mut(|seen| {
                seen.push((
                    e.controller.id(),
                    e.validation
                        .block_tickets()
                        .into_iter()
                        .map(|(pos, _)| pos)
                        .collect(),
                ))
            });
            e.validation.remove_block_tickets([1, 64, 1]);
        }
    }
    thread_local! {
        /// What each validation was shown: its controller, then the block positions.
        static SEEN: std::cell::RefCell<Vec<(&'static str, Vec<[i32; 3]>)>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    fn mods() -> mod_api::Mods {
        vec![
            ModEntry::new("machines", Box::new(Machines)),
            ModEntry::new("other", Box::new(Other)),
        ]
        .into()
    }
    fn controller(id: &str) -> TicketController {
        testing::ticket_controller(id).unwrap()
    }
    /// The slot of a registered controller id.
    fn known_slot(id: &str) -> Option<u32> {
        TicketController::from_id(id).map(TicketController::slot)
    }
    fn quarry() -> TicketController {
        controller("machines:quarry")
    }
    /// A game whose mods are `Machines` and `Other`, with their controllers registered.
    fn with_machines(test: impl FnOnce(&mut super::super::Game<'_>)) {
        mod_platform::testing::with_mods(mods(), || with_chunk_streaming(test));
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn water_tickets_hydrate_real_farmland_and_preserve_vanilla_water_and_rain() {
        use horizon_client::chunk::Chunk;
        use std::sync::Arc;

        with_machines(|game| {
            game.install_live_features();
            let level = game.level(game.dim.id()).unwrap();
            let registry = game.pack.assets.registry;
            let dry = registry.default_state("minecraft:farmland").unwrap();
            let wheat = registry.default_state("minecraft:wheat").unwrap();
            let air = registry.default_state("minecraft:air").unwrap();
            let water = registry.default_state("minecraft:water").unwrap();
            let stone = registry.default_state("minecraft:stone").unwrap();
            let plains = game.res.biomes().id("minecraft:plains");
            let mut chunk = Chunk::empty(0, 0);
            for qy in 0..game.world.shape.height / 4 {
                for qx in 0..4 {
                    for qz in 0..4 {
                        chunk.set_biome_cell(qx, qy, qz, plains);
                    }
                }
            }
            game.world.chunks.insert((0, 0), Arc::new(chunk));
            game.block_ticks
                .add_empty_chunk(horizon_core::position::ChunkPos::new(0, 0))
                .unwrap();
            assert!(level.set_weather_levels(game, 0.0, 0.0));
            let moisture = |game: &super::super::Game<'_>, pos| {
                level
                    .block_property(game, pos, "moisture")
                    .unwrap()
                    .parse::<i32>()
                    .unwrap()
            };
            for x in 4..9 {
                for z in 4..9 {
                    game.world.set(x, 64, z, dry).unwrap();
                    game.world.set(x, 65, z, wheat).unwrap();
                }
            }
            game.world.set(9, 64, 4, dry).unwrap();
            game.world.set(9, 65, 4, wheat).unwrap();
            let guard = in_mod("machines");
            let ticket = level
                .add_water_ticket(game, [4, 64, 4], [9, 65, 9])
                .unwrap();
            for x in 4..9 {
                for z in 4..9 {
                    assert!(level.has_water_ticket(game, [x, 64, z]));
                    assert!(level.block_random_tick(game, [x, 64, z]));
                    assert_eq!(moisture(game, [x, 64, z]), 7);
                }
            }
            assert!(!level.has_water_ticket(game, [9, 64, 4]));
            assert!(level.block_random_tick(game, [9, 64, 4]));
            assert_eq!(moisture(game, [9, 64, 4]), 0);
            assert!(ticket.remove(game));
            assert!(!ticket.remove(game));
            for expected in (0..7).rev() {
                for x in 4..9 {
                    for z in 4..9 {
                        assert!(level.block_random_tick(game, [x, 64, z]));
                        assert_eq!(moisture(game, [x, 64, z]), expected);
                    }
                }
            }
            // A real water block at vanilla's four-block edge still hydrates without a ticket.
            game.world.set(0, 64, 4, water).unwrap();
            assert!(level.block_random_tick(game, [4, 64, 4]));
            assert_eq!(moisture(game, [4, 64, 4]), 7);
            game.world.set(0, 64, 4, air).unwrap();
            game.world.set(4, 64, 4, dry).unwrap();
            game.world.set(4, 65, 4, air).unwrap();
            assert!(level.set_weather_levels(game, 1.0, 0.0));
            assert_eq!(level.is_raining_at(game, [4, 65, 4]), Some(true));
            assert!(level.block_random_tick(game, [4, 64, 4]));
            assert_eq!(moisture(game, [4, 64, 4]), 7);
            // A roof still excludes rain: tickets have not replaced the rain predicate.
            game.world.set(4, 66, 4, stone).unwrap();
            assert_eq!(level.is_raining_at(game, [4, 65, 4]), Some(false));
            assert!(level.block_random_tick(game, [4, 64, 4]));
            assert_eq!(moisture(game, [4, 64, 4]), 6);
            drop(guard);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn water_ticket_handles_isolate_dimensions_owners_and_unload_lifetimes() {
        with_machines(|game| {
            let overworld = game.level(game.dim.id()).unwrap();
            assert!(
                overworld
                    .add_water_ticket(game, [0, 64, 0], [5, 65, 5])
                    .is_none()
            );
            let guard = in_mod("machines");
            let a = overworld
                .add_water_ticket(game, [-17, 64, -1], [18, 65, 2])
                .unwrap();
            let stale = LevelRef::new(
                WorldEpoch::new(overworld.epoch().get() + 1),
                overworld.dimension(),
            );
            assert!(!stale.has_water_ticket(game, [0, 64, 0]));
            assert!(
                stale
                    .add_water_ticket(game, [0, 64, 0], [5, 65, 5])
                    .is_none()
            );
            for pos in [[-17, 64, -1], [17, 64, 1]] {
                assert!(overworld.has_water_ticket(game, pos));
            }
            for pos in [
                [-18, 64, 0],
                [18, 64, 0],
                [0, 63, 0],
                [0, 65, 0],
                [0, 64, -2],
                [0, 64, 2],
            ] {
                assert!(!overworld.has_water_ticket(game, pos), "{pos:?}");
            }
            game.enter_level(Dim::Nether).unwrap();
            let nether = game.level(game.dim.id()).unwrap();
            assert!(!nether.has_water_ticket(game, [0, 64, 0]));
            assert!(overworld.has_water_ticket(game, [0, 64, 0]));
            let b = nether
                .add_water_ticket(game, [0, 64, 0], [5, 65, 5])
                .unwrap();
            assert!(
                overworld
                    .add_water_ticket(game, [0, 64, 0], [5, 65, 5])
                    .is_some()
            );
            drop(guard);
            let guard = in_mod("other");
            assert!(!a.remove(game), "only its owner removes a ticket");
            let overlap = nether
                .add_water_ticket(game, [3, 64, 3], [4, 65, 4])
                .unwrap();
            drop(guard);
            assert!(mod_api::__private::set_mod_enabled("machines", false));
            assert!(!overworld.has_water_ticket(game, [0, 64, 0]));
            assert!(!nether.has_water_ticket(game, [0, 64, 0]));
            assert!(nether.has_water_ticket(game, [3, 64, 3]));
            assert!(mod_api::__private::set_mod_enabled("machines", true));
            assert!(
                !overworld.has_water_ticket(game, [0, 64, 0]),
                "re-enable never restores old tickets"
            );
            let guard = in_mod("machines");
            assert!(!b.remove(game));
            let fresh = overworld
                .add_water_ticket(game, [0, 64, 0], [5, 65, 5])
                .unwrap();
            game.native_levels_closing();
            assert!(!overworld.has_water_ticket(game, [0, 64, 0]));
            assert!(!nether.has_water_ticket(game, [3, 64, 3]));
            assert!(!fresh.remove(game));
            drop(guard);
            let guard = in_mod("other");
            assert!(!overlap.remove(game));
            drop(guard);
            // The client cannot query or mutate the server's in-memory marks.
            game.options.execution_role = super::super::execution_role::ExecutionRole::Client;
            let guard = in_mod("machines");
            assert!(!nether.has_water_ticket(game, [0, 64, 0]));
            assert!(
                nether
                    .add_water_ticket(game, [0, 64, 0], [5, 65, 5])
                    .is_none()
            );
            assert!(!b.remove(game));
            drop(guard);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn water_tickets_are_not_restored_with_saved_farmland() {
        use horizon_client::{chunk::Chunk, save::SaveDir, world::World};
        use std::sync::Arc;

        with_machines(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let registry = game.pack.assets.registry;
            let dry = registry.default_state("minecraft:farmland").unwrap();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(horizon_core::position::ChunkPos::new(0, 0))
                .unwrap();
            game.world.set(4, 64, 4, dry).unwrap();
            let guard = in_mod("machines");
            let old = level
                .add_water_ticket(game, [4, 64, 4], [9, 65, 9])
                .unwrap();
            assert!(level.block_random_tick(game, [4, 64, 4]));
            assert_eq!(
                level
                    .block_property(game, [4, 64, 4], "moisture")
                    .as_deref(),
                Some("7")
            );
            game.save
                .save(
                    &[game.world.chunks.get(&(0, 0)).unwrap().clone()],
                    registry,
                    game.res.biomes(),
                )
                .unwrap();
            let root = game.save.root.clone();
            let shape = game.world.shape;
            game.native_levels_closing();
            let save = SaveDir::new(root, shape);
            let chunk = save.load(0, 0, registry, game.res.biomes()).unwrap();
            game.world = World::new(registry.default_state("minecraft:air").unwrap(), 0);
            game.world.chunks.insert((0, 0), Arc::new(chunk));
            game.save = Arc::new(save);
            game.native_lifecycle
                .begin_open(mod_api::Authority::IntegratedServer);
            let reopened = game.level(game.dim.id()).unwrap();
            assert_ne!(level.epoch(), reopened.epoch());
            assert!(!reopened.has_water_ticket(game, [4, 64, 4]));
            assert!(!old.remove(game));
            assert_eq!(
                reopened
                    .block_property(game, [4, 64, 4], "moisture")
                    .as_deref(),
                Some("7")
            );
            assert!(reopened.block_random_tick(game, [4, 64, 4]));
            assert_eq!(
                reopened
                    .block_property(game, [4, 64, 4], "moisture")
                    .as_deref(),
                Some("6")
            );
            drop(guard);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn mod_tickets_are_scoped_by_owner_and_share_the_forceload_store() {
        use mod_api::TicketOwner::{Block, Entity};
        with_machines(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let chunk = game.level_chunk_ref(level, [5, 6]).unwrap();
            let (a, b) = (Block([80, 64, 96]), Entity(7));
            // Outside a mod's hook there is no caller to own a ticket.
            assert!(!game.chunk_set_mod_forced(chunk, quarry(), a, true, false));
            let guard = in_mod("machines");
            assert!(game.chunk_set_mod_forced(chunk, quarry(), a, true, false));
            assert!(
                !game.chunk_set_mod_forced(chunk, quarry(), a, true, false),
                "held"
            );
            assert!(game.chunk_set_mod_forced(chunk, quarry(), b, true, false));
            assert!(game.streaming.forced.contains(&(5, 6)));
            assert_eq!(game.chunk_forced(chunk), Some(false), "not /forceload's");
            assert_eq!(game.level_forced_chunks(level), Some(vec![]));
            // Two owners in one chunk release separately.
            assert!(game.chunk_set_mod_forced(chunk, quarry(), a, false, false));
            assert!(game.streaming.forced.contains(&(5, 6)));
            assert_eq!(
                game.level_mod_forced_chunks(level, quarry()),
                Some(vec![(b, [5, 6], false)])
            );
            // Another mod cannot use our controller, however it asks; its own is empty.
            drop(guard);
            let guard = in_mod("other");
            assert_eq!(game.level_mod_forced_chunks(level, quarry()), None);
            assert!(!game.chunk_set_mod_forced(chunk, quarry(), b, false, false));
            assert!(!game.chunk_set_mod_forced(chunk, quarry(), b, true, false));
            let vault = controller("other:vault");
            assert_eq!(game.level_mod_forced_chunks(level, vault), Some(vec![]));
            assert!(game.chunk_set_mod_forced(chunk, vault, b, true, false));
            assert!(game.chunk_set_mod_forced(chunk, vault, b, false, false));
            drop(guard);
            // Our other controller is apart from the first: it neither lists nor releases its tickets.
            let guard = in_mod("machines");
            let drill = controller("machines:drill");
            assert_eq!(game.level_mod_forced_chunks(level, drill), Some(vec![]));
            assert!(!game.chunk_set_mod_forced(chunk, drill, b, false, false));
            assert!(game.chunk_set_mod_forced(chunk, drill, b, true, false));
            assert!(game.chunk_set_mod_forced(chunk, drill, b, false, false));
            assert_eq!(
                game.level_mod_forced_chunks(level, quarry()),
                Some(vec![(b, [5, 6], false)])
            );
            drop(guard);
            // /forceload and a ticket on one chunk: the chunk stays until both go.
            assert!(game.chunk_set_forced(chunk, true));
            assert!(game.chunk_set_forced(chunk, false));
            assert!(
                game.streaming.forced.contains(&(5, 6)),
                "the ticket holds it"
            );
            assert!(game.chunk_set_forced(chunk, true));
            let _guard = in_mod("machines");
            assert!(game.chunk_set_mod_forced(chunk, quarry(), b, false, false));
            assert!(
                game.streaming.forced.contains(&(5, 6)),
                "/forceload holds it"
            );
            game.streaming.clear_forced();
            assert!(game.streaming.forced.is_empty());
            // The world's bounds, as /forceload checks them.
            let outside = mod_api::ChunkRef::new(level, [1_875_000, 0]);
            assert!(!game.chunk_set_mod_forced(outside, quarry(), a, true, false));
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn mod_tickets_save_apart_from_vanillas_tickets_and_release_with_the_world() {
        use mod_api::TicketOwner::{Block, Entity};
        with_machines(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let chunk = game.level_chunk_ref(level, [-9, 4]).unwrap();
            let _guard = in_mod("machines");
            let drill = controller("machines:drill");
            let (block, entity) = (Block([-140, 70, 70]), Entity(u128::MAX - 3));
            assert!(game.chunk_set_mod_forced(chunk, quarry(), block, true, false));
            assert!(game.chunk_set_mod_forced(chunk, quarry(), entity, true, false));
            assert!(game.chunk_set_mod_forced(chunk, drill, block, true, false));
            game.save_chunk_tickets().unwrap();
            let vanilla = game.save.load_forced_chunks();
            assert!(vanilla.is_empty(), "vanilla's file has no mod ticket");
            // The world closes: its streaming goes, the next opening reads the file.
            let reopen = |game: &mut super::super::Game<'_>, slot_of: fn(&str) -> Option<u32>| {
                game.streaming.load_forced(
                    game.save.load_forced_chunks(),
                    game.save.load_mod_tickets(slot_of),
                );
                game.validate_mod_chunk_tickets(game.dim);
            };
            reopen(game, known_slot);
            // Each ticket comes back under the controller that held it.
            let listed =
                |game: &super::super::Game<'_>, c| game.level_mod_forced_chunks(level, c).unwrap();
            assert_eq!(listed(game, quarry()).len(), 2);
            assert_eq!(listed(game, drill), vec![(block, [-9, 4], false)]);
            assert!(game.streaming.forced.contains(&(-9, 4)));
            // No loaded mod registers the controller: its tickets are discarded, nothing
            // is forced, and the next save drops them from the file.
            reopen(game, |_| None);
            assert!(game.streaming.forced.is_empty());
            game.save_chunk_tickets().unwrap();
            reopen(game, known_slot);
            assert!(game.streaming.forced.is_empty());
            assert!(game.save.load_mod_tickets(known_slot).is_empty());
            // Released tickets leave the file.
            assert!(game.chunk_set_mod_forced(chunk, quarry(), Block([-140, 70, 70]), true, false));
            assert!(game.chunk_set_mod_forced(
                chunk,
                quarry(),
                Block([-140, 70, 70]),
                false,
                false
            ));
            game.save_chunk_tickets().unwrap();
            assert!(game.save.load_mod_tickets(known_slot).is_empty());
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_mod_ticket_keeps_a_parked_level_active() {
        with_machines(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let other = if game.dim == Dim::Nether {
                Dim::End
            } else {
                Dim::Nether
            };
            let back = game.dim;
            game.enter_level(other).unwrap();
            game.enter_level(back).unwrap();
            let parked = mod_api::LevelRef::new(level.epoch(), other.key());
            let chunk = ChunkRef::new(parked, [2, 2]);
            let _guard = in_mod("machines");
            let owner = mod_api::TicketOwner::Block([32, 64, 32]);
            assert!(!game.levels_to_tick().contains(&other));
            assert!(game.chunk_set_mod_forced(chunk, quarry(), owner, true, false));
            assert!(game.levels_to_tick().contains(&other));
            assert!(game.chunk_set_mod_forced(chunk, quarry(), owner, false, false));
            assert!(!game.levels_to_tick().contains(&other));
        });
    }

    /// The result `/forceload query` gives: the chunks listed, or 0 when it reports none.
    fn query_result(game: &mut super::super::Game<'_>, command: &str) -> Option<i32> {
        let d = game.ui.dispatcher.0.clone();
        let parse = d.parse(command, &game.ui.registries);
        let (ctx, _) = parse.executable().ok()?;
        let (ctx, root) = (ctx.clone(), parse.context.clone());
        game.run_chain(&root, &ctx).ok()
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn forceload_add_returns_with_each_new_column_loaded() {
        with_chunk_streaming(|game| {
            game.ui.registries.permission = 4;
            assert!(!game.world.chunks.contains_key(&(8, -4)));
            assert!(!game.world.chunks.contains_key(&(9, -4)));
            assert_eq!(query_result(game, "forceload add 128 -64 144 -64"), Some(2));
            for key in [(8, -4), (9, -4)] {
                assert!(
                    game.world.chunks.contains_key(&key),
                    "loaded {key:?} before command return"
                );
                assert!(game.streaming.commanded().contains(&key));
            }
            assert_eq!(query_result(game, "forceload add 128 -64 144 -64"), None);
            assert_eq!(
                query_result(game, "forceload remove 128 -64 144 -64"),
                Some(2)
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn forceload_query_lists_only_command_tickets() {
        with_machines(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let chunk = game.level_chunk_ref(level, [3, 3]).unwrap();
            game.ui.registries.permission = 4;
            let _guard = in_mod("machines");
            let owner = mod_api::TicketOwner::Block([48, 64, 48]);
            assert!(game.chunk_set_mod_forced(chunk, quarry(), owner, true, false));
            assert!(game.streaming.forced.contains(&(3, 3)));
            assert_eq!(query_result(game, "forceload query"), Some(0), "list form");
            assert_eq!(
                query_result(game, "forceload query 48 48"),
                None,
                "position form"
            );
            assert!(game.chunk_set_forced(chunk, true));
            assert_eq!(query_result(game, "forceload query"), Some(1));
            assert_eq!(query_result(game, "forceload query 48 48"), Some(1));
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn natural_spawning_tickets_and_plain_ones_save_and_load_apart() {
        use mod_api::TicketOwner::Block;
        with_machines(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let plain = game.level_chunk_ref(level, [11, 12]).unwrap();
            let spawning = game.level_chunk_ref(level, [13, 12]).unwrap();
            let _guard = in_mod("machines");
            let owner = Block([176, 64, 192]);
            assert!(game.chunk_set_mod_forced(plain, quarry(), owner, true, false));
            assert!(game.chunk_set_mod_forced(spawning, quarry(), owner, true, true));
            let tickets = game.streaming.mod_tickets();
            assert!(!tickets.forces_natural_spawning((11, 12)));
            assert!(tickets.forces_natural_spawning((13, 12)));
            // Both kinds load and tick: they are forced chunks.
            assert!(game.streaming.forced.contains(&(11, 12)));
            assert!(game.streaming.forced.contains(&(13, 12)));
            assert!(game.levels_to_tick().contains(&game.dim));
            assert_eq!(
                game.level_mod_forced_chunks(level, quarry()),
                Some(vec![(owner, [11, 12], false), (owner, [13, 12], true)])
            );
            // One owner holds both kinds on one chunk apart.
            assert!(game.chunk_set_mod_forced(plain, quarry(), owner, true, true));
            assert!(game.chunk_set_mod_forced(plain, quarry(), owner, false, true));
            assert!(game.streaming.forced.contains(&(11, 12)));
            assert!(
                !game
                    .streaming
                    .mod_tickets()
                    .forces_natural_spawning((11, 12))
            );
            game.save_chunk_tickets().unwrap();
            let saved = game.save.load_mod_tickets(known_slot);
            game.streaming.load_forced(Default::default(), saved);
            game.validate_mod_chunk_tickets(game.dim);
            let tickets = game.streaming.mod_tickets();
            assert!(!tickets.forces_natural_spawning((11, 12)));
            assert!(tickets.forces_natural_spawning((13, 12)));
            assert!(tickets.holds((11, 12)));
        });
    }

    /// The flag adds a chunk to the spawning chunks (inhabited time, a spawn
    /// attempt); the local mob cap still wants a non-spectator within 128
    /// blocks of its centre.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_natural_spawning_ticket_far_from_every_player_runs_the_attempt_but_the_local_cap_refuses()
    {
        use horizon_client::chunk::Chunk;
        use horizon_client::mobs::spawn_selection::{Category, SpawnPlayer, SpawnState};
        use horizon_client::motion::PlayerMotion;
        use horizon_core::geometry::Vec3;
        use mod_api::TicketOwner::Block;
        use std::sync::Arc;
        with_machines(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            for cx in 6..=10 {
                for cz in 6..=10 {
                    let mut chunk = Chunk::empty(cx, cz);
                    chunk.lit = true;
                    game.world.chunks.insert((cx, cz), Arc::new(chunk));
                }
            }
            let level = game.level(game.dim.id()).unwrap();
            // Both chunks lie in the player's 8-chunk area and past 128 blocks of it.
            let flagged = game.level_chunk_ref(level, [8, 8]).unwrap();
            let plain = game.level_chunk_ref(level, [8, 7]).unwrap();
            {
                let _guard = in_mod("machines");
                let owner = Block([128, 64, 128]);
                assert!(game.chunk_set_mod_forced(flagged, quarry(), owner, true, true));
                assert!(game.chunk_set_mod_forced(plain, quarry(), owner, true, false));
            }
            game.spawn_natural();
            assert_eq!(game.world.inhabited.get(&(8, 8)), Some(&1));
            assert!(
                !game.world.inhabited.contains_key(&(8, 7)),
                "without the flag it is no spawning chunk"
            );
            assert!(game.mobs.is_empty(), "the local cap refuses");
            // The cap itself: no player within 128 blocks of the chunk.
            let players: Vec<SpawnPlayer> = game
                .level_players()
                .iter()
                .map(|p| SpawnPlayer {
                    position: [p.position.x, p.position.y, p.position.z],
                    spectator: p.spectator,
                })
                .collect();
            let natural = game.natural_level();
            let mut state = SpawnState::new(&natural, 1, players, &[], |_, _| true);
            assert!(!state.can_spawn_local(Category::Monster, 8, 8));
            assert!(state.can_spawn_local(Category::Monster, 0, 0));
        });
    }

    fn saved(controller: &str, pos: [i32; 3], chunk: (i32, i32)) -> ModTicket {
        ModTicket {
            controller: controller_slot(controller),
            owner: TicketOwner::Block(pos),
            chunk,
            natural_spawning: false,
        }
    }
    fn controller_slot(id: &str) -> u32 {
        controller(id).slot()
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn validation_drops_a_removed_ticket_and_leaves_the_rest() {
        with_machines(|game| {
            // `quarry` and `vault` validate; `plain` doesn't. All of them hold the pruned block.
            let tickets = vec![
                saved("machines:quarry", [1, 64, 1], (7, 7)),
                saved("machines:quarry", [9, 64, 9], (8, 8)),
                saved("machines:drill", [1, 64, 1], (9, 9)),
                saved("machines:plain", [1, 64, 1], (10, 10)),
                saved("other:vault", [1, 64, 1], (11, 11)),
            ];
            game.streaming.load_forced(Default::default(), tickets);
            assert!(
                game.streaming.forced.is_empty(),
                "loaded, not yet validated"
            );
            game.validate_mod_chunk_tickets(game.dim);
            let forced = &game.streaming.forced;
            assert!(!forced.contains(&(7, 7)), "quarry's prune");
            assert!(forced.contains(&(8, 8)));
            assert!(
                forced.contains(&(9, 9)),
                "drill's validation prunes nothing"
            );
            assert!(forced.contains(&(10, 10)), "no validator, no validation");
            assert!(!forced.contains(&(11, 11)), "other's own controller");
            // A validation is shown one controller's tickets, whichever mod hears it.
            let seen = SEEN.take();
            let quarry_seen = vec![[1, 64, 1], [9, 64, 9]];
            assert!(seen.contains(&("machines:quarry", quarry_seen.clone())));
            assert!(seen.contains(&("machines:drill", vec![[1, 64, 1]])));
            assert!(seen.contains(&("other:vault", vec![[1, 64, 1]])));
            assert!(
                seen.iter().all(|(id, _)| *id != "machines:plain"),
                "never asked"
            );
            // The mod that doesn't own the controller saw an empty view.
            assert!(
                seen.iter()
                    .any(|(id, tickets)| *id == "machines:quarry" && tickets.is_empty())
            );
            game.save_chunk_tickets().unwrap();
            let file = game.save.load_mod_tickets(known_slot);
            assert_eq!(file.len(), 3);
            assert!(
                !file
                    .iter()
                    .any(|t| t.chunk == (7, 7) || t.chunk == (11, 11))
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn validation_runs_when_a_parked_level_opens_and_not_without_a_listener() {
        mod_platform::testing::with_mods(mods(), || {
            with_game(|game| {
                let other = if game.dim == Dim::Nether {
                    Dim::End
                } else {
                    Dim::Nether
                };
                let world =
                    std::env::temp_dir().join(format!("horizon-validate-{}", std::process::id()));
                game.world_root = world.clone();
                let root = horizon_client::save::dimension_dir(&world, other.id());
                let save = horizon_client::save::SaveDir::new(root, other.shape());
                let mut tickets = horizon_client::mod_chunk_tickets::ModTickets::default();
                tickets.insert(saved("machines:quarry", [1, 64, 1], (7, 7)));
                tickets.insert(saved("machines:quarry", [9, 64, 9], (8, 8)));
                save.save_mod_tickets(&tickets, |slot| {
                    TicketController::from_slot(slot).map(TicketController::id)
                })
                .unwrap();
                let back = game.dim;
                game.enter_level(other).unwrap();
                game.enter_level(back).unwrap();
                let parked = game.levels.parked_streaming_of(other.id()).unwrap();
                assert!(!parked.forced.contains(&(7, 7)));
                assert!(parked.forced.contains(&(8, 8)));
                assert!(game.levels_to_tick().contains(&other));
                let _ = std::fs::remove_dir_all(&world);
            });
        });
        // No mod listens: the tickets stay as loaded.
        struct Idle;
        #[mod_api::__subscriptions]
        impl Hooks for Idle {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                r.register_ticket_controller("quarry", true);
            }
        }
        let mods: mod_api::Mods = vec![ModEntry::new("idle", Box::new(Idle))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let tickets = vec![saved("idle:quarry", [1, 64, 1], (7, 7))];
                game.streaming.load_forced(Default::default(), tickets);
                game.validate_mod_chunk_tickets(game.dim);
                assert!(game.streaming.forced.contains(&(7, 7)));
            });
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn validation_sees_the_saved_tickets_not_ones_added_after_the_load() {
        with_machines(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let tickets = vec![saved("machines:quarry", [1, 64, 1], (7, 7))];
            game.streaming.load_forced(Default::default(), tickets);
            // `level_load` runs after the load: its hold is the same owner's.
            let fresh = game.level_chunk_ref(level, [6, 6]).unwrap();
            {
                let _guard = in_mod("machines");
                let owner = mod_api::TicketOwner::Block([1, 64, 1]);
                assert!(game.chunk_set_mod_forced(fresh, quarry(), owner, true, false));
            }
            game.validate_mod_chunk_tickets(game.dim);
            assert!(!game.streaming.forced.contains(&(7, 7)), "saved one pruned");
            assert!(game.streaming.forced.contains(&(6, 6)), "fresh one kept");
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn light_and_loaded_reads_match_the_games_query_and_load_nothing() {
        use horizon_client::chunk::{Chunk, Light};
        use mod_api::LevelHost;
        use std::sync::Arc;

        with_game(|game| {
            let mut chunk = Chunk::empty(0, 0);
            // Block light 5 and sky light 12 in the lowest section.
            Arc::make_mut(&mut chunk.sections[0]).light = Light::Uniform(0xC5);
            game.world.chunks.insert((0, 0), Arc::new(chunk));
            let level = game.level(game.dim.id()).unwrap();
            let lit = [3, -60, 5];
            let sky = game.world.light(3, -60, 5) >> 4;
            let block = game.world.light(3, -60, 5) & 15;
            assert_eq!((sky, block), (12, 5));
            let chunks = game.world.chunks.len();
            let darken = game.sky_darken();

            game.with_level(|_, host, _| {
                assert_eq!(host.brightness(LightLayer::Block, lit), 5);
                assert_eq!(host.brightness(LightLayer::Sky, lit), 12);
                assert_eq!(host.raw_brightness(lit, 0), 12);
                assert_eq!(host.raw_brightness(lit, 9), 5);
                let pos = BlockPos::new(lit[0], lit[1], lit[2]);
                assert_eq!(
                    host.raw_brightness(lit, darken),
                    horizon_world::level::raw_brightness(&*host, pos, darken)
                );
                assert!(host.is_loaded([0, 319, 0]));
                assert!(!host.is_loaded([0, 320, 0]));
                assert!(!host.is_loaded([0, -65, 0]));
                assert!(!host.is_loaded([16, -60, 0]));
            });
            assert_eq!(
                game.level_brightness(level, LightLayer::Block, lit),
                Some(5)
            );
            assert_eq!(game.level_brightness(level, LightLayer::Sky, lit), Some(12));
            assert_eq!(
                game.level_brightness(level, LightLayer::Sky, [16, -60, 0]),
                None
            );
            assert_eq!(
                game.level_brightness(level, LightLayer::Sky, [0, 320, 0]),
                None
            );
            let stale = LevelRef::new(WorldEpoch::new(u64::MAX), game.dim.key());
            assert_eq!(game.level_brightness(stale, LightLayer::Sky, lit), None);
            assert_eq!(game.world.chunks.len(), chunks, "nothing was loaded");
        });
    }

    /// `check_light` queues the cell with the level's light edits, its state set to itself,
    /// where the next update asks a `dynamic_light` block again and relights it; a stale
    /// handle and a position that isn't loaded queue nothing.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn check_light_queues_the_cell_with_the_light_edits() {
        use horizon_client::chunk::Chunk;
        use std::sync::Arc;

        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.world.light_edits.clear();
            let level = game.level(game.dim.id()).unwrap();
            let state = game.world.block(3, 64, 5).0;
            assert!(game.level_check_light(level, [3, 64, 5]));
            assert_eq!(game.world.light_edits, [((3, 64, 5), state, state)]);
            let stale = LevelRef::new(WorldEpoch::new(u64::MAX), game.dim.key());
            assert!(!game.level_check_light(stale, [3, 64, 5]));
            assert!(!game.level_check_light(level, [16, 64, 0]), "not loaded");
            assert!(
                !game.level_check_light(level, [3, 320, 5]),
                "above the world"
            );
            assert_eq!(game.world.light_edits.len(), 1);
        });
    }
}

#[cfg(test)]
mod environment_read_tests {
    use crate::game::{Dim, native_entity_refs::tests::with_game};
    use horizon_client::chunk::Chunk;
    use mod_api::{BiomeSearch, LevelRef, WorldEpoch};
    use std::sync::Arc;

    fn paint_biome(game: &mut crate::game::Game<'_>, name: &str) {
        let biome = world_biomes!(game).id(name);
        let mut chunk = Chunk::empty(0, 0);
        for y in 0..chunk.sections.len() as i32 * 4 {
            for x in 0..4 {
                for z in 0..4 {
                    chunk.set_biome_cell(x, y, z, biome);
                }
            }
        }
        game.world.chunks.insert((0, 0), Arc::new(chunk));
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn biome_reads_follow_live_cells_parked_levels_and_client_login_without_loading() {
        use crate::game::execution_role::ExecutionRole;
        use horizon_protocol::connection::Pipe;

        with_game(|game| {
            let overworld = game.level("minecraft:overworld").unwrap();
            paint_biome(game, "minecraft:plains");
            let before = (game.world.chunks.len(), game.streaming.pending());
            assert_eq!(
                overworld.biome_at(game, [8, 64, 8]).as_deref(),
                Some("minecraft:plains")
            );
            assert_eq!(overworld.biome_at(game, [31, 64, 31]), None);
            assert_eq!(before, (game.world.chunks.len(), game.streaming.pending()));
            paint_biome(game, "minecraft:desert");
            assert_eq!(
                overworld.biome_at(game, [8, 64, 8]).as_deref(),
                Some("minecraft:desert")
            );
            game.enter_level(Dim::Nether).unwrap();
            assert_eq!(
                overworld.biome_at(game, [8, 64, 8]).as_deref(),
                Some("minecraft:desert"),
                "parked level's cells"
            );
            game.enter_level(Dim::Overworld).unwrap();
            let stale = LevelRef::new(
                WorldEpoch::new(overworld.epoch().get() + 1),
                overworld.dimension(),
            );
            assert_eq!(stale.biome_at(game, [8, 64, 8]), None);

            game.options.execution_role = ExecutionRole::Client;
            game.options.dedicated = None;
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "BiomeReadTest").unwrap();
            assert_eq!(
                overworld.client_biome(game, [8, 64, 8]),
                None,
                "not logged in"
            );
            game.remote.as_mut().unwrap().logged_in = true;
            paint_biome(game, "minecraft:snowy_plains");
            let client_level = game.level("minecraft:overworld").unwrap();
            let before = (game.world.chunks.len(), game.streaming.pending());
            assert_eq!(
                client_level.client_biome(game, [8, 64, 8]).as_deref(),
                Some("minecraft:snowy_plains")
            );
            assert_eq!(client_level.client_biome(game, [31, 64, 31]), None);
            let foreign = LevelRef::new(client_level.epoch(), mod_api::DimensionKey::NETHER);
            assert_eq!(foreign.client_biome(game, [8, 64, 8]), None);
            assert_eq!(before, (game.world.chunks.len(), game.streaming.pending()));
            game.remote.as_mut().unwrap().logged_in = false;
            assert_eq!(
                client_level.client_biome(game, [8, 64, 8]),
                None,
                "logged out"
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn floor_reads_real_collision_shapes_and_refuses_unloaded_positions() {
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let slab = registry.default_state("minecraft:oak_slab").unwrap();
            let fence = registry.default_state("minecraft:oak_fence").unwrap();
            for (x, block, height, above) in [
                (2, stone, 1.0, 0.0),
                (5, slab, 0.5, f64::NEG_INFINITY),
                (8, fence, 1.5, 0.5),
            ] {
                game.world.set(x, 100, 8, block).unwrap();
                assert_eq!(level.block_floor_height(game, [x, 100, 8]), Some(height));
                assert_eq!(level.block_floor_height(game, [x, 101, 8]), Some(above));
            }
            assert_eq!(
                level.block_floor_height(game, [12, 100, 8]),
                Some(f64::NEG_INFINITY)
            );
            let before = (game.world.chunks.len(), game.streaming.pending());
            assert_eq!(level.block_floor_height(game, [32, 100, 8]), None);
            assert_eq!(
                level.block_floor_height(game, [8, game.world.top_y(), 8]),
                None
            );
            assert_eq!(before, (game.world.chunks.len(), game.streaming.pending()));
            game.enter_level(Dim::Nether).unwrap();
            assert_eq!(
                level.block_floor_height(game, [5, 100, 8]),
                Some(0.5),
                "parked collision shapes"
            );
            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            assert_eq!(stale.block_floor_height(game, [5, 100, 8]), None);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn sky_and_logical_height_read_the_named_dimension_and_client_state() {
        use crate::game::execution_role::ExecutionRole;
        use horizon_client::chunk::LevelShape;
        use horizon_protocol::connection::Pipe;

        with_game(|game| {
            let overworld = game.level("minecraft:overworld").unwrap();
            assert_eq!(overworld.logical_height(game), Some(384));
            for time in [0, 6000, 12000, 18000, 24000] {
                game.world.day_time = time;
                assert_eq!(overworld.get_sky_darken(game), Some(game.sky_darken()));
            }
            game.world.day_time = 6000;
            game.world.weather.rain_level = (0.0, 0.0);
            game.world.weather.thunder_level = (0.0, 0.0);
            game.enter_level(Dim::Nether).unwrap();
            let nether = game.level("minecraft:the_nether").unwrap();
            assert_eq!(nether.logical_height(game), Some(128));
            assert_eq!(
                nether.height(game),
                Some((0, 256)),
                "logical and build heights differ"
            );
            assert_eq!(nether.get_sky_darken(game), Some(11));
            // The vanilla fixture records clear midnight as 11 and half-rain noon as 1.
            // Change the shared state while the Overworld snapshot stays parked at noon.
            game.world.day_time = 18000;
            assert_eq!(overworld.day_time(game), Some(18000));
            assert_eq!(overworld.get_sky_darken(game), Some(11));
            game.world.day_time = 6000;
            game.world.weather.rain_level = (0.5, 0.5);
            assert_eq!(overworld.get_sky_darken(game), Some(1));
            assert_eq!(nether.get_sky_darken(game), Some(11));
            assert_eq!(game.dim, Dim::Nether, "a read does not swap levels");
            let parked = game.levels.parked_world_of("minecraft:overworld").unwrap();
            assert_eq!(
                parked.day_time, 6000,
                "the parked snapshot is not rewritten"
            );
            assert_eq!(parked.weather.levels(1.0), (0.0, 0.0));
            game.enter_level(Dim::Overworld).unwrap();
            game.world.day_time = 6000;
            assert_eq!(nether.logical_height(game), Some(128));
            assert_eq!(
                nether.get_sky_darken(game),
                Some(11),
                "parked level keeps its dimension's sky"
            );
            let stale = LevelRef::new(
                WorldEpoch::new(overworld.epoch().get() + 1),
                overworld.dimension(),
            );
            assert_eq!(stale.logical_height(game), None);
            assert_eq!(stale.get_sky_darken(game), None);

            game.options.execution_role = ExecutionRole::Client;
            game.options.dedicated = None;
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "SkyReadTest").unwrap();
            assert_eq!(overworld.logical_height(game), None, "not logged in");
            game.remote.as_mut().unwrap().logged_in = true;
            game.world.shape = LevelShape::new(-128, 512, 256, true, false).unwrap();
            let level = game.level("minecraft:overworld").unwrap();
            assert_eq!(
                level.logical_height(game),
                Some(256),
                "client's own dimension shape"
            );
            assert_eq!(level.get_sky_darken(game), Some(game.sky_darken()));
            assert_eq!(
                nether.logical_height(game),
                None,
                "another dimension isn't visible to a client"
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn closest_biome_uses_the_named_generator_without_loading_resident_chunks() {
        use horizon_client::vanillagen::{TerrainResources, VanillaTerrain};

        with_game(|game| {
            game.enter_level(Dim::Nether).unwrap();
            let directory = game.options.resources.join("worldgen");
            let terrain = VanillaTerrain::for_dimension(
                TerrainResources {
                    data: Some(&game.pack.data_changes),
                    dir: &directory,
                    registry: game.pack.assets.registry,
                    biomes: world_biomes!(game),
                    item_data: &game.res.core.item_data,
                },
                12345,
                "minecraft:the_nether",
            )
            .unwrap();
            game.end.set_locator(Some(Arc::new(terrain)));
            let nether = game.level("minecraft:the_nether").unwrap();
            let parameters = BiomeSearch::new([8, 64, 8], 0, 32, 64);
            let before = (game.world.chunks.len(), game.streaming.pending());
            let found = nether
                .find_closest_biome(game, "#minecraft:is_nether", parameters)
                .expect("all Nether source biomes match");
            assert_eq!(
                found.position,
                [8, 64, 8],
                "vanilla samples the origin first"
            );
            assert!(
                [
                    "minecraft:nether_wastes",
                    "minecraft:soul_sand_valley",
                    "minecraft:crimson_forest",
                    "minecraft:warped_forest",
                    "minecraft:basalt_deltas"
                ]
                .contains(&found.biome.as_str())
            );
            assert_eq!(
                nether.find_closest_biome(game, "minecraft:plains", parameters),
                None
            );
            assert_eq!(
                nether.find_closest_biome(game, "minecraft:no_such_biome", parameters),
                None
            );
            assert_eq!(
                nether.find_closest_biome(
                    game,
                    "#minecraft:is_nether",
                    BiomeSearch::new([0, 64, 0], -1, 32, 64)
                ),
                None
            );
            assert_eq!(
                nether.find_closest_biome(
                    game,
                    "#minecraft:is_nether",
                    BiomeSearch::new([0, 64, 0], 32, 0, 64)
                ),
                None
            );
            // The old public boundary accepted these calls and could panic or hang.
            let huge_step = nether
                .find_closest_biome(
                    game,
                    "#minecraft:is_nether",
                    BiomeSearch::new([0, 64, 0], 0, 32, i32::MAX),
                )
                .expect("a step larger than the build height samples only the origin");
            assert_eq!(huge_step.position, [0, 64, 0]);
            for parameters in [
                BiomeSearch::new([0, 64, 0], i32::MAX, 1, 1),
                BiomeSearch::new([0, 64, 0], 128, 1, i32::MAX),
                BiomeSearch::new([0, 64, 0], 16, 1, 1),
                BiomeSearch::new([0, 64, 0], 0, 32, 0),
                BiomeSearch::new([0, 64, 0], 0, 32, -1),
            ] {
                assert_eq!(
                    nether.find_closest_biome(game, "#minecraft:is_nether", parameters),
                    None,
                    "invalid or over-budget input must be refused even if the origin matches"
                );
            }
            assert_eq!(before, (game.world.chunks.len(), game.streaming.pending()));
            game.enter_level(Dim::Overworld).unwrap();
            let parked = nether
                .find_closest_biome(game, "#minecraft:is_nether", parameters)
                .unwrap();
            assert_eq!(parked, found, "parked level retains its generator and seed");
            let stale = LevelRef::new(
                WorldEpoch::new(nether.epoch().get() + 1),
                nether.dimension(),
            );
            assert_eq!(
                stale.find_closest_biome(game, "#minecraft:is_nether", parameters),
                None
            );
        });
    }

    /// The nearest stronghold lies on vanilla's first ring: 3 strongholds at
    /// `4 * 32 + (nextDouble - 0.5) * 32 * 2.5` chunks from the origin, 88 to 168
    /// (`ConcentricRingsStructurePlacement` with distance 32), each then moved
    /// to a biome it may start in within 112 blocks.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn nearest_structure_uses_the_named_generator_without_loading_resident_chunks() {
        use horizon_client::vanillagen::{TerrainResources, VanillaTerrain};
        use mod_api::StructureSearch;

        with_game(|game| {
            let directory = game.options.resources.join("worldgen");
            let terrain = VanillaTerrain::for_dimension(
                TerrainResources {
                    data: Some(&game.pack.data_changes),
                    dir: &directory,
                    registry: game.pack.assets.registry,
                    biomes: world_biomes!(game),
                    item_data: &game.res.core.item_data,
                },
                12345,
                "minecraft:overworld",
            )
            .unwrap();
            game.end.set_locator(Some(Arc::new(terrain)));
            let overworld = game.level("minecraft:overworld").unwrap();
            let before = (game.world.chunks.len(), game.streaming.pending());
            let search = StructureSearch::new([0, 64, 0], 100);
            let found = overworld
                .find_nearest_structure(game, "minecraft:stronghold", search)
                .expect("every Overworld has strongholds");
            assert_eq!(found.structure, "minecraft:stronghold");
            let [x, y, z] = found.position;
            assert_eq!(y, 0, "the start chunk's corner at y 0");
            assert_eq!((x.rem_euclid(16), z.rem_euclid(16)), (0, 0));
            // `findBiomeHorizontal` then moves each ring position up to 112 blocks.
            let chunks = f64::from(x.div_euclid(16)).hypot(f64::from(z.div_euclid(16)));
            assert!((81.0..=175.0).contains(&chunks), "{chunks} chunks out");
            assert_eq!(
                overworld.find_nearest_structure(game, "stronghold", search),
                Some(found.clone()),
                "an id without a namespace is minecraft's"
            );
            assert_eq!(
                overworld.find_nearest_structure(game, "#minecraft:eye_of_ender_located", search),
                Some(found.clone()),
                "a tag searches its members"
            );
            assert_eq!(
                overworld.find_nearest_structure(
                    game,
                    "minecraft:stronghold",
                    StructureSearch::new([x, 64, z], 0)
                ),
                Some(found.clone()),
                "from the start itself"
            );
            for structure in ["minecraft:fortress", "minecraft:no_such_structure"] {
                assert_eq!(
                    overworld.find_nearest_structure(game, structure, search),
                    None,
                    "{structure} does not generate here"
                );
            }
            for parameters in [
                StructureSearch::new([0, 64, 0], -1),
                StructureSearch::new([0, 64, 0], StructureSearch::MAX_RADIUS + 1),
                StructureSearch::new([0, 64, 0], i32::MAX),
                StructureSearch::new([i32::MAX, 64, 0], 100),
                StructureSearch::new([0, 64, i32::MIN], 100),
                StructureSearch::new([30_000_000, 64, 0], 0),
            ] {
                assert_eq!(
                    overworld.find_nearest_structure(game, "minecraft:stronghold", parameters),
                    None,
                    "{parameters:?} is refused"
                );
            }
            assert!(
                overworld
                    .find_nearest_structure(
                        game,
                        "minecraft:stronghold",
                        StructureSearch::new([29_999_999, i32::MIN, -30_000_000], 0)
                    )
                    .is_some(),
                "the world's edge and any height are searched"
            );
            assert_eq!(before, (game.world.chunks.len(), game.streaming.pending()));
            let stale = LevelRef::new(
                WorldEpoch::new(overworld.epoch().get() + 1),
                overworld.dimension(),
            );
            assert_eq!(
                stale.find_nearest_structure(game, "minecraft:stronghold", search),
                None
            );
        });
    }

    /// API results over original fixture worlds, compared with independently recorded vanilla calls.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn level_environment_reads_match_vanilla_vectors() {
        use horizon_client::vanillagen::{TerrainResources, VanillaTerrain};
        use serde_json::{Value, json};
        use std::collections::HashMap;

        let fixture = |name: &str, variable: &str| {
            let path = std::env::var_os(variable)
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| {
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../../tests/fixtures")
                        .join(name)
                });
            std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| serde_json::from_str::<Value>(line).unwrap())
                .collect::<Vec<_>>()
        };
        let inputs = fixture("level_environment_reads.jsonl", "HORIZON_LEVEL_READ_CASES");
        let expected = fixture(
            "level_environment_reads_expected.jsonl",
            "HORIZON_LEVEL_READ_EXPECTED",
        );
        assert!(inputs.len() >= 476);
        assert_eq!(inputs.len(), expected.len());
        let position =
            |value: &Value| std::array::from_fn::<_, 3, _>(|i| value[i].as_i64().unwrap() as i32);
        with_game(|game| {
            let mut generators = HashMap::new();
            for (input, expected) in inputs.iter().zip(&expected) {
                assert_eq!(input["name"], expected["name"]);
                let dimension = input["dimension"].as_str().unwrap_or("minecraft:overworld");
                let dim = Dim::from_id(dimension);
                if game.dim != dim {
                    game.enter_level(dim).unwrap();
                }
                let kind = input["kind"].as_str().unwrap();
                match kind {
                    "biome" => {
                        game.world.chunks.clear();
                        let ids = ["minecraft:plains", "minecraft:desert", "minecraft:forest"]
                            .map(|name| world_biomes!(game).id(name));
                        for column in input["loaded"].as_array().unwrap() {
                            let (cx, cz) = (
                                column[0].as_i64().unwrap() as i32,
                                column[1].as_i64().unwrap() as i32,
                            );
                            let mut chunk = Chunk::empty(cx, cz);
                            for y in 0..chunk.sections.len() as i32 * 4 {
                                for x in 0..4 {
                                    for z in 0..4 {
                                        let qy = (chunk.min_y() >> 2) + y;
                                        let index =
                                            ((cx * 4 + x) * 31 + qy * 7 + (cz * 4 + z) * 17)
                                                .rem_euclid(3);
                                        chunk.set_biome_cell(x, y, z, ids[index as usize]);
                                    }
                                }
                            }
                            game.world.chunks.insert((cx, cz), Arc::new(chunk));
                        }
                        game.streaming.biome_zoom = horizon_core::biome::BiomeZoom::new(
                            input["zoom_seed"].as_i64().unwrap(),
                        );
                    }
                    "floor" => {
                        game.world.chunks.clear();
                        game.world
                            .chunks
                            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                        for (field, y) in [("here", 100), ("below", 99)] {
                            let state = game
                                .pack
                                .assets
                                .registry
                                .default_state(input[field].as_str().unwrap())
                                .unwrap();
                            game.world.set(8, y, 8, state).unwrap();
                        }
                    }
                    "sky" => {
                        game.world.day_time = input["time"].as_i64().unwrap();
                        let rain = input["rain"].as_f64().unwrap() as f32;
                        let thunder = input["thunder"].as_f64().unwrap() as f32;
                        game.world.weather.rain_level = (rain, rain);
                        game.world.weather.thunder_level = (thunder, thunder);
                    }
                    "search" => {
                        let seed = input["seed"].as_i64().unwrap();
                        let key = (dimension.to_owned(), seed);
                        let generator = generators.entry(key).or_insert_with(|| {
                            let directory = game.options.resources.join("worldgen");
                            let terrain = VanillaTerrain::for_dimension(
                                TerrainResources {
                                    data: Some(&game.pack.data_changes),
                                    dir: &directory,
                                    registry: game.pack.assets.registry,
                                    biomes: world_biomes!(game),
                                    item_data: &game.res.core.item_data,
                                },
                                seed,
                                dimension,
                            )
                            .unwrap();
                            Arc::new(terrain)
                        });
                        game.end.set_locator(Some(generator.clone()));
                    }
                    "logical_height" => {}
                    other => panic!("unknown fixture {other}"),
                }
                let level = game.level(dimension).unwrap();
                let before = (game.world.chunks.len(), game.streaming.pending());
                let value = match kind {
                    "biome" => json!(level.biome_at(game, position(&input["position"]))),
                    "floor" => json!(level.block_floor_height(game, [8, 100, 8]).map(|height| {
                        if height == f64::NEG_INFINITY {
                            json!("-Infinity")
                        } else {
                            json!(height)
                        }
                    })),
                    "sky" => json!(level.get_sky_darken(game)),
                    "logical_height" => json!(level.logical_height(game)),
                    "search" => {
                        let parameters = BiomeSearch::new(
                            position(&input["position"]),
                            input["radius"].as_i64().unwrap() as i32,
                            input["horizontal"].as_i64().unwrap() as i32,
                            input["vertical"].as_i64().unwrap() as i32,
                        );
                        json!(
                            level
                                .find_closest_biome(
                                    game,
                                    input["selector"].as_str().unwrap(),
                                    parameters
                                )
                                .map(
                                    |found| json!({"biome":found.biome,"position":found.position})
                                )
                        )
                    }
                    _ => unreachable!(),
                };
                assert_eq!(
                    json!({"name":input["name"],"value":value}),
                    *expected,
                    "{}",
                    input["name"]
                );
                assert_eq!(
                    before,
                    (game.world.chunks.len(), game.streaming.pending()),
                    "read queued or loaded a chunk: {}",
                    input["name"]
                );
            }
        });
    }
}
