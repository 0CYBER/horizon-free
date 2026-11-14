//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The host's methods both sides share of the `levels` area, implemented by the game and reached through [`Host`].
#[horizon_mod_api_macros::host_area]
pub trait LevelsHost {
    /// Side: both.
    /// Whether a mod with the `horizon-mod.toml` id `id` is installed: a
    /// native, wasm or content package (Fabric's `FabricLoader.isModLoaded`,
    /// NeoForge's `ModList.isLoaded`). The list is fixed at startup.
    #[access(Metadata)]
    #[static_call]
    fn is_loaded(&self, id: &str) -> bool;
    /// Side: both.
    /// Find a loaded chunk in an open server level.
    #[access(WorldOnly)]
    fn chunk(&self, level: LevelRef, pos: [i32; 2]) -> Option<ChunkRef>;
}
/// The handles' reads of the `levels` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait LevelsBackend {
    /// Side: server.
    /// Whether any watered-area ticket covers this block. Minimum bounds are inclusive and
    /// maximum bounds exclusive on all three axes. Reads active and parked levels, loads no
    /// chunks, and calls no mod predicates. False on clients or for a stale/unavailable level.
    #[access(WorldOnly)]
    fn level_has_water_ticket(&self, level: LevelRef, pos: [i32; 3]) -> bool;
    /// Side: server.
    /// `ServerLevel.findClosestBiome3d`: search this level's generator for an id or `#tag`.
    /// The result follows vanilla's sampled spiral and vertical order; it is not a search
    /// through edited biome cells. Does not load resident chunks or switch parked levels.
    /// None for no match, a stale/unavailable level, an unavailable generator, a negative
    /// radius, nonpositive sampling steps, or a potential sample count above
    /// [`BiomeSearch::MAX_SAMPLES`] (see [`BiomeSearch::new`] for the work formula).
    /// Positive vertical steps larger than the build height sample only the clamped origin.
    #[access(WorldOnly)]
    fn level_find_closest_biome(
        &self,
        level: LevelRef,
        biome: &str,
        parameters: BiomeSearch,
    ) -> Option<BiomeSearchResult>;
    /// Side: server.
    /// `ServerLevel.findNearestMapStructure` without skipping known starts: the
    /// nearest start of `structure`, an id or a `#tag`, searched in this
    /// level's generator as `/locate structure` does. Random-spread structure
    /// sets are searched in squares of placement cells out to
    /// [`StructureSearch::radius`]; concentric rings (strongholds) check every
    /// ring position whatever the radius. Starts are built as the search needs
    /// them, without loading or generating chunks or switching parked levels.
    /// None for no start, a structure that does not generate in this level,
    /// a world without structures, a stale level, a client, a negative radius,
    /// one above [`StructureSearch::MAX_RADIUS`], and an origin outside the
    /// world's 30,000,000-block bound.
    #[access(WorldOnly)]
    fn level_find_nearest_structure(
        &self,
        level: LevelRef,
        structure: &str,
        parameters: StructureSearch,
    ) -> Option<StructureSearchResult>;
    /// Side: both.
    /// `Level.getSkyDarken`: the sky-light reduction from dimension, time and weather.
    /// Reads any loaded server level, including parked levels, or the current client level.
    /// None for a stale or unavailable level. Does not load chunks.
    #[access(WorldOnly)]
    fn level_get_sky_darken(&self, level: LevelRef) -> Option<i32>;
    /// Side: both.
    /// `BlockGetter.getBlockFloorHeight` with the normal collision shapes and an empty
    /// entity context. The result is relative to the block: negative infinity means no floor.
    /// None for a stale level, an unloaded column or a position outside build height.
    /// Reads parked server levels and the current client level without loading chunks.
    #[access(WorldOnly)]
    fn level_block_floor_height(&self, level: LevelRef, pos: [i32; 3]) -> Option<f64>;
    /// Side: server.
    /// The biome id selected by `LevelReader.getBiome` and this level's biome zoom.
    /// Reads an active or parked server level. None for a
    /// stale handle or an unloaded selected noise column. Never generates or loads chunks.
    #[access(WorldOnly)]
    fn level_biome_at(&self, level: LevelRef, pos: [i32; 3]) -> Option<String>;
    /// Side: both.
    /// The entry ids (`addon:fire`) of a datapack registry a mod declared
    /// (`declare_datapack_registry`), sorted, as this world session last loaded them. On a client
    /// they are the ones its server sent at the join and after each reload, and only for a
    /// registry declared `Required` or `Optional`. None for a stale or parked level and a
    /// registry no mod declared (or, on a client, one the server did not send). Reads nothing
    /// from disk.
    #[access(WorldOnly)]
    fn level_datapack_entries(&self, level: LevelRef, registry: &str) -> Option<Vec<String>>;
    /// Side: both.
    /// A handle to `entry` of `registry` at the generation now loaded (on a client, the one its
    /// server last sent). A handle kept across a reload fails its next read with
    /// `StaleGeneration`, one from an earlier world session with `WrongSession`. A stale or parked
    /// level and a registry no mod declared are `UnknownRegistry`; an entry no pack defines is
    /// `MissingEntry`.
    #[access(WorldOnly)]
    fn level_datapack_handle(
        &self,
        level: LevelRef,
        registry: &str,
        entry: &str,
    ) -> Result<DatapackHandle, DatapackReadError>;
    /// Side: both.
    /// The decoded fields of the entry a handle names, in the registry's schema order, as plain
    /// values: a copy, which carries no authority after the read. The handle is checked first
    /// (session, then generation), then the entry; the errors are those of
    /// [`level_datapack_handle`](Self::level_datapack_handle).
    #[access(WorldOnly)]
    fn level_datapack_read(
        &self,
        level: LevelRef,
        handle: &DatapackHandle,
    ) -> Result<Vec<(String, DatapackValue)>, DatapackReadError>;
    /// Side: both.
    /// How fast `tool` (None is an empty hand) mines the block at `pos`: the tool's own mining
    /// speed on that block (`ItemStack.getDestroySpeed`, 1.0 for no tool; for a block it has no
    /// rule for, the tool's default mining speed), before the player's enchantments, attributes, water, ground and
    /// `player_break_speed` hooks, which `player_break_speed` itself hears. None for a stale
    /// handle or an unloaded or out-of-height position. Reads no chunks.
    #[access(WorldOnly)]
    fn level_block_break_speed(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        tool: Option<&ItemStack>,
    ) -> Option<f32>;
    /// Side: client.
    /// The biome in the client's current level (`LevelReader.getBiome`). None before
    /// login, for a stale or foreign dimension, or an unloaded selected column. Never loads chunks.
    #[access(WorldOnly)]
    fn level_client_biome(&self, level: LevelRef, pos: [i32; 3]) -> Option<String>;
    /// Side: both.
    /// A biome's natural spawners by category (`MobSpawnSettings.getMobs`), from the level's
    /// biome table, data packs and biome modifiers included, categories sorted by name. None for a
    /// stale handle or an unknown biome. This is the biome's own list; `potential_spawns` is
    /// what a position gets after mods edit it. Reads no chunks.
    #[access(WorldOnly)]
    fn level_biome_spawners(&self, level: LevelRef, biome: &str) -> Option<Vec<BiomeSpawners>>;
    /// Side: both.
    /// A biome's own sky, fog and water colours (`BiomeSpecialEffects`), from the level's biome
    /// table. None for a stale handle or an unknown biome. Reads no chunks.
    #[access(WorldOnly)]
    fn level_biome_effects(&self, level: LevelRef, biome: &str) -> Option<BiomeEffects>;
    /// Side: both.
    /// Whether a biome is in a biome tag (`Holder.is(TagKey)`), by the level's tag tables: the
    /// vanilla tags, the world's data packs and mods' biome tags merged. `tag` is a namespaced
    /// id (`minecraft:is_forest`), with or without a leading `#`. A tag no pack defines holds
    /// nothing, so false. None for a stale handle, an unknown biome or a level that has no tag
    /// tables loaded. Reads no chunks.
    #[access(WorldOnly)]
    fn level_biome_in_tag(&self, level: LevelRef, biome: &str, tag: &str) -> Option<bool>;
    /// Side: both.
    /// A biome's climate by id (`minecraft:plains`), from the level's biome table, data packs
    /// included: temperature, downfall, whether it has precipitation and the frozen modifier
    /// (`Biome.ClimateSettings`). None for a stale handle or an unknown biome. Reads no chunks.
    #[access(WorldOnly)]
    fn level_biome_properties(&self, level: LevelRef, biome: &str) -> Option<BiomeProperties>;
    /// Side: both.
    /// The dimension's logical height (`ServerLevel.getLogicalHeight`), which may differ
    /// from build height. Reads active and parked server levels or the client's current level.
    /// None for a stale or unavailable level. Does not load chunks.
    #[access(WorldOnly)]
    fn level_logical_height(&self, level: LevelRef) -> Option<i32>;
    /// Side: server.
    /// The level's world border as [min_x, min_z, max_x, max_z], using the
    /// previous size while it moves. Horizon currently shares one border
    /// across its dimensions. None for a stale handle or a closed level.
    #[access(WorldOnly)]
    fn level_world_border(&self, level: LevelRef) -> Option<[f64; 4]>;
    /// Side: both.
    /// The name of a dimension's key.
    #[access(Metadata)]
    fn dimension_key_id(&self, key: DimensionKey) -> Option<String>;
    /// Side: server.
    /// A handle for chunk `pos` of an open level, loaded or not, so a mod can
    /// force an unloaded chunk. None for a stale handle, on a client, and for
    /// a chunk outside the world (`ChunkPos` past 30 million blocks).
    #[access(WorldOnly)]
    fn level_chunk_ref(&self, level: LevelRef, pos: [i32; 2]) -> Option<ChunkRef>;
    /// Side: server.
    /// Whether `/forceload` holds the chunk (`ServerLevel.getForcedChunks`),
    /// loaded or not; a mod's own ticket on it does not count. None for a stale handle or on a client.
    #[access(WorldOnly)]
    fn chunk_forced(&self, chunk: ChunkRef) -> Option<bool>;
    /// Side: server.
    /// The level's force-loaded chunks (`/forceload query`), sorted as
    /// the command lists them. None for a stale handle or on a client.
    #[access(WorldOnly)]
    fn level_forced_chunks(&self, level: LevelRef) -> Option<Vec<[i32; 2]>>;
    /// Side: server.
    /// The chunk tickets `controller` holds in the level, as (owner, chunk
    /// position, natural_spawning), in order. None for a stale handle, on a
    /// client, outside a mod's hook and for another mod's controller.
    #[access(WorldOnly)]
    fn level_mod_forced_chunks(
        &self,
        level: LevelRef,
        controller: TicketController,
    ) -> Option<Vec<(TicketOwner, [i32; 2], bool)>>;
    /// Side: server.
    /// Whether this chunk is loaded; false on a client or for a stale handle.
    #[access(WorldOnly)]
    fn chunk_loaded(&self, chunk: ChunkRef) -> bool;
    /// Side: server.
    /// The status of a loaded chunk. Proto chunks are not exposed here.
    #[access(WorldOnly)]
    fn chunk_status(&self, chunk: ChunkRef) -> Option<ChunkStatus>;
    /// Side: server.
    /// Whether the chunk passes vanilla's seeded slime-chunk test, independent of
    /// dimension, height, biome and other spawn conditions. Works for unloaded chunks
    /// in active or parked open levels. None for a stale level, a client or a chunk
    /// outside world bounds. Does not load chunks or advance the world's random stream.
    #[access(WorldOnly)]
    fn chunk_is_slime_chunk(&self, chunk: ChunkRef) -> Option<bool>;
    /// Side: server.
    /// `ChunkMap.getPlayers(pos, false)`: the players watching this chunk, those
    /// it is inside the view of and was sent to, in the order the server keeps
    /// its players. A player watches it from its `chunk_watch` until it leaves
    /// the view; Horizon's far-ring columns past the view distance are not
    /// watched. Fake players watch nothing. Loaded or not, a chunk nobody
    /// watches gives an empty list. None on a client and for a stale handle.
    #[access(WorldOnly)]
    fn chunk_watchers(&self, chunk: ChunkRef) -> Option<Vec<PlayerRef>>;
    /// Side: server.
    /// Match and assemble through the engine's recipe matcher, without consuming inputs.
    /// Covers minecraft:crafting, smelting, blasting, smoking, campfire_cooking,
    /// stonecutting and smithing, including data-pack recipes of those types.
    /// Mod-owned types return RecipeLookupError::NotMatchable: cache recipes_of_type once
    /// per load/reload and match those in mod memory (no per-tick host crossing).
    /// Dynamic custom outputs and contextual predicates remain mod-owned.
    /// A valid input without a match returns Ok(None); invalid shapes/items and unavailable
    /// levels are typed refusals. Crafting dimensions are 1 through 3, with row-major slots.

    #[access(WorldOnly)]
    fn level_recipe_for(
        &self,
        level: LevelRef,
        recipe_type: &Identifier,
        inputs: &RecipeInput,
    ) -> Result<Option<RecipeMatch>, RecipeLookupError>;
    /// Side: server.
    /// Every available member of the server level's entity lookup
    /// (`ServerLevel.getAllEntities`), including spectators and entities outside
    /// any spatial query. Dragon parts are not lookup members.
    /// Returns None on a remote client, for a stale handle or a parked level.
    /// Returns None while any lookup member is temporarily extracted, rather
    /// than returning an incomplete list.
    /// Order is lookup admission order, preserved across temporary extraction;
    /// removal and re-admission moves an entity to the end. A chunk's entities
    /// join in the order it stored them: section by section, each in admission
    /// order, as vanilla stores them. An entity that moved into another section
    /// since it joined keeps its admission place there, where vanilla moves it
    /// to the end of that section. Zero runtime IDs fall back to stable storage
    /// join order, not runtime admission order. Unregistered entity kinds have
    /// no authoritative lookup handle and are omitted. These limitations mean
    /// saved-world and whole-level ordering parity is not guaranteed.
    /// Allocates a result buffer, not one allocation per entity; sorting is
    /// O(n log n), followed by one authoritative handle lookup per candidate.
    #[access(WorldOnly)]
    fn level_all_entities(&self, level: LevelRef) -> Option<Vec<EntityRef>>;
    /// Side: both.
    /// Entities whose bounding boxes intersect this box (`Level.getEntities`).
    /// Face-only contact does not intersect; spectator players are excluded.
    /// Returns None for a stale handle or a parked level: its entity handles
    /// cannot resolve until the level is in place, and for a bound that is not
    /// finite (NaN or infinite). A minimum above its maximum is the same box
    /// with the two swapped, as vanilla's `AABB`. On a client (`ClientLevel`),
    /// the entities the server tracks to it, other players among them, and the
    /// window's own player, with the same walk.
    /// None there before login and for a level it does not show. The walk
    /// is in entity-list order followed by players. Every entity list is walked
    /// once per call: O(entities) regardless of the box. Each overlapping mob
    /// resolves through a mob-list scan: O(K*N) for K overlapping mobs and N mobs. Tridents do not
    /// resolve; frames, paintings and other special entities
    /// are not on the entity-list walk. The dragon follows ordinary entities;
    /// its eight intersecting parts follow players in vanilla hash-table order (`Level.getEntities`).
    /// During `hears_projectile_impacts`, the projectile list is taken out of
    /// the game and is not visible; it is restored for `projectile_impact`.
    #[access(WorldOnly)]
    fn level_entities_in(
        &self,
        level: LevelRef,
        min: [f64; 3],
        max: [f64; 3],
    ) -> Option<Vec<EntityRef>>;
    /// Side: server.
    /// `ServerLevel.players`: the online players in this level, parked or in
    /// place, each once under their own UUID, also during a guest's turn. Fake
    /// players are not listed, as vanilla's list leaves out connectionless
    /// players; a player displaced by a nested guest turn is not listed either,
    /// since its handle cannot resolve until that turn ends. None on a client
    /// and for a stale handle.
    #[access(WorldOnly)]
    fn level_online_players(&self, level: LevelRef) -> Option<Vec<PlayerRef>>;
    /// Side: both.
    /// The block state ID at `pos` (`Level.getBlockState`), without formatting a string.
    /// None for a chunk that isn't loaded or a `y` outside the world. On a client, the
    /// state is the one the server sent for a chunk the client holds, in the level it shows
    /// or in one the server opened for viewing (`host.level`): a viewed level reads its own
    /// world, never the player's, and never loads a column.
    #[access(WorldOnly)]
    fn level_block_state(&self, level: LevelRef, pos: [i32; 3]) -> Option<BlockState>;
    /// Side: server.
    /// Preview a held tool's transformation without changing the block or stack. The existing
    /// block-tool hook fires with `simulated` true; listeners must not mutate the level in a
    /// simulation. The player must be live in the level in place, and the block must be loaded;
    /// parked levels cannot provide the hook's level context. A stale handle, invalid face,
    /// unavailable player or level, or no transformation returns None.
    #[access(WorldOnly)]
    fn level_block_transform(
        &self,
        level: LevelRef,
        player: PlayerRef,
        hand: Hand,
        stack: &ItemStack,
        pos: [i32; 3],
        hit: BlockHit,
    ) -> Option<BlockTransformPreview>;
    /// Side: both.
    /// The block at `pos` without its properties: `minecraft:oak_stairs`.
    /// None as for `block_state`. API 381.
    #[access(WorldOnly)]
    fn level_block(&self, level: LevelRef, pos: [i32; 3]) -> Option<BlockKey>;
    /// Side: both.
    /// Property `key` of the block state at `pos` (`BlockState.getValue`):
    /// `east` for `facing`. None for a block without it, or as for
    /// `block_state`. API 381.
    #[access(WorldOnly)]
    fn level_block_property(&self, level: LevelRef, pos: [i32; 3], key: &str) -> Option<String>;
    /// Side: server.
    /// Whether this is the level in place: the one the server ticks now, whose block entities
    /// the game's own capabilities read and write and whose capabilities
    /// `LevelRef::capability` looks up. False for a level parked while the server ticks
    /// another (its blocks can still be read), for a handle of an earlier open, and on a
    /// client.
    #[access(WorldOnly)]
    fn level_in_place(&self, level: LevelRef) -> bool;
    /// Side: both.
    /// Whether `pos` is inside the world's height and its chunk is loaded
    /// (`Level.isLoaded`, which is false outside `isInValidBounds`): a block
    /// read or write there finds the world. False on a client or for a stale
    /// handle: the position is not loaded here. On a client, a chunk it holds is loaded
    /// (`ClientLevel.hasChunkAt`). API 381.
    #[access(WorldOnly)]
    fn level_is_loaded(&self, level: LevelRef, pos: [i32; 3]) -> bool;
    /// Side: server.
    /// The stored light of one layer at `pos` (`BlockAndLightGetter.getBrightness`), 0 to 15.
    /// None as for `block_state`: a position that isn't loaded has no light to read.
    #[access(WorldOnly)]
    fn level_brightness(&self, level: LevelRef, layer: LightLayer, pos: [i32; 3]) -> Option<i32>;
    /// Side: both.
    /// The lowest block `y` and the height of the level
    /// (`LevelHeightAccessor.getMinY`, `getHeight`). Reads any loaded server level
    /// or the client's current level. None for a stale handle, before client login,
    /// or for a dimension unavailable on this side. Does not load chunks. API 381.
    #[access(WorldOnly)]
    fn level_height(&self, level: LevelRef) -> Option<(i32, i32)>;
    /// Side: both.
    /// `Level.isInWorldBounds`: `pos` within 30,000,000 blocks on x and z and inside the level's
    /// build height. None as for `native_level_height`. Does not load chunks.
    #[access(WorldOnly)]
    fn level_in_world_bounds(&self, level: LevelRef, pos: [i32; 3]) -> Option<bool>;
    /// Side: both.
    /// `Level.isInSpawnableBounds`: 30,000,000 blocks on x and z, 20,000,000 on y. None as for
    /// `native_level_height`. Does not load chunks.
    #[access(WorldOnly)]
    fn level_in_spawnable_bounds(&self, level: LevelRef, pos: [i32; 3]) -> Option<bool>;
    /// Side: server.
    /// `Some(true)` when this resident, in-bounds position has entity-ticking tickets.
    /// `Some(false)` for unloaded or out-of-bounds positions or tickets that forbid ticking.
    /// With no ticket system, resident in-bounds chunks tick; an active but empty ticket
    /// system permits none. This reads the current ticket decision, not a tick guarantee.
    /// `None` for stale handles, a client or a level not currently in place. Parked
    /// levels cannot answer because the live ticket state belongs to the level in place.
    /// Does not load chunks. Each call shares the ticket map through a reference count;
    /// it does not allocate a memo table or refresh a resident-chunk cache.
    #[access(WorldOnly)]
    fn level_entity_ticking(&self, level: LevelRef, pos: [i32; 3]) -> Option<bool>;
    /// Side: server.
    /// `Some(true)` when scheduled blocks, block events and block entities can tick here:
    /// a resident, in-bounds position with block-ticking tickets and no pending entities.
    /// `Some(false)` for unloaded or out-of-bounds positions, tickets that forbid ticking,
    /// or a chunk waiting for its entities. With no ticket system, resident in-bounds
    /// chunks with no pending entities tick; an active but empty ticket system permits none.
    /// `None` for stale handles, a client or a level not currently in place. Parked
    /// levels cannot answer because the live ticket state belongs to the level in place.
    /// Does not load chunks. Each call shares the ticket map through a reference count
    /// and borrows the pending-entity set without copying it or allocating a memo table.
    #[access(WorldOnly)]
    fn level_should_tick_blocks_at(&self, level: LevelRef, pos: [i32; 3]) -> Option<bool>;

    /// Side: both.
    /// The level's game time (`Level.getGameTime`). On clients this is the
    /// synchronized time advanced by the local level tick. API 381.
    #[access(WorldOnly)]
    fn level_game_time(&self, level: LevelRef) -> Option<i64>;
    /// Side: both.
    /// The time of day in ticks (`Level.getOverworldClockTime`), shared by
    /// every level; it keeps counting past 24000. On a client, the synchronized
    /// Overworld clock, advanced at its server-supplied rate, even when the
    /// displayed dimension has a different default clock. API 381.
    #[access(WorldOnly)]
    fn level_day_time(&self, level: LevelRef) -> Option<i64>;
    /// Side: server.
    /// The time of the level's own clock in ticks: the clock its dimension type names as
    /// `default_clock`, which its sky, spawning and beds go by and `/time` sets there. A mod's
    /// level with a `day_cycle` clock of its own counts it apart from the Overworld's; a level
    /// without a default clock, like the Nether, is None, as is a stale handle and a client.
    #[access(WorldOnly)]
    fn level_clock_time(&self, level: LevelRef) -> Option<i64>;
    /// Side: server.
    /// The next clock time after [`level_clock_time`](Self::level_clock_time) at which the
    /// level's own clock reaches `marker`, a time marker of its timelines (as `/time set <marker>`
    /// moves to it). Today only the Overworld's clock defines markers, so a level following any
    /// other clock (a mod dimension's own) answers None for every marker. None too for a marker
    /// the clock does not define, a time past `i64::MAX`, a level without a default clock, a stale
    /// handle and a client.
    #[access(WorldOnly)]
    fn level_clock_marker_time(&self, level: LevelRef, marker: &Identifier) -> Option<i64>;
    /// Side: both.
    /// `FluidState.getFlow` at `pos`: the current that pushes an entity there, after the
    /// `fluid_flow` hook, as entities feel it. Zero where no fluid flows. None as for
    /// `block_state`.
    #[access(WorldOnly)]
    fn level_fluid_flow(&self, level: LevelRef, pos: [i32; 3]) -> Option<Vec3>;
    /// Side: server.
    /// The supplied state's block-local outline or collision boxes at `pos`,
    /// with an empty context (`BlockState.getShape` / `getCollisionShape`).
    /// The state need not be the block stored there; level-dependent shapes
    /// still read the real level. Pass `block_state(host, pos)` to query the
    /// state at that position. Interaction, visual, occlusion and support shapes
    /// are not exposed. Returns None on a client, for an unknown state, a stale
    /// handle or a closed level. Collision errors return no boxes.
    #[access(WorldOnly)]
    fn level_block_shape(
        &self,
        level: LevelRef,
        state: BlockState,
        pos: [i32; 3],
        kind: BlockShapeKind,
    ) -> Option<Vec<[f64; 6]>>;
    /// Side: both.
    /// Clip an empty-context ray against collider or outline shapes, without fluids.
    /// None on failed reads, dynamic shapes, invalid rays or traversal limits, and for a ray
    /// that enters a column the level doesn't hold. On a client, the level it shows or a
    /// viewed one (`host.level`), each through its own world.
    #[access(WorldOnly)]
    fn level_clip(
        &self,
        level: LevelRef,
        from: [f64; 3],
        to: [f64; 3],
        blocks: ClipBlock,
        fluids: ClipFluid,
    ) -> Option<ClipHit>;
    /// Side: server.
    /// Gameplay's face support in the current or a parked server level, including
    /// live block entities for dynamic shapes. None for stale or closed levels,
    /// unloaded blocks, unsupported states or a shape query error.
    #[access(WorldOnly)]
    fn level_block_face_sturdy(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        face: BlockFace,
        support: SupportType,
    ) -> Option<bool>;
    /// Side: server.
    /// How strongly the block at `pos` powers the block on its `direction` side, power it
    /// conducts included (`SignalGetter.getSignal(pos, direction)`, as a block reads its
    /// neighbour `pos` that lies `direction` from it), 0 to 15. None for another level, a stale
    /// handle or a client. Takes a shared host and opens no transaction.
    #[access(WorldOnly)]
    fn level_signal(&self, level: LevelRef, pos: [i32; 3], direction: Direction) -> Option<i32>;
    /// Side: server.
    /// The strong power alone that the block at `pos` gives that way
    /// (`SignalGetter.getDirectSignal(pos, direction)`), 0 to 15. None as for `signal`.
    #[access(WorldOnly)]
    fn level_direct_signal(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        direction: Direction,
    ) -> Option<i32>;
    /// Side: server.
    /// The strongest strong power reaching the block at `pos` from its six neighbours
    /// (`SignalGetter.getDirectSignalTo`), 0 to 15. None as for `signal`.
    #[access(WorldOnly)]
    fn level_direct_signal_to(&self, level: LevelRef, pos: [i32; 3]) -> Option<i32>;
    /// Side: server.
    /// The strongest power reaching the block at `pos` from its six neighbours
    /// (`SignalGetter.getBestNeighborSignal`, what a dispenser or a piston reads), 0 to 15.
    /// None as for `signal`.
    #[access(WorldOnly)]
    fn level_best_neighbor_signal(&self, level: LevelRef, pos: [i32; 3]) -> Option<i32>;
    /// Side: server.
    /// Whether any neighbour powers the block at `pos` (`SignalGetter.hasNeighborSignal`),
    /// asking no side past the first that does. False where `signal` is None.
    #[access(WorldOnly)]
    fn level_has_neighbor_signal(&self, level: LevelRef, pos: [i32; 3]) -> bool;
    /// Side: both.
    /// Dimension shape of a loaded server level, including parked levels.
    #[access(WorldOnly)]
    fn level_dimension_shape(&self, level: LevelRef) -> Option<LevelDimensionShape>;
    /// Side: both.
    /// The level's dimension type (`Level.dimensionType()` and its gameplay attributes): coordinate
    /// scale, ambient light, fixed time, infiniburn, monster spawn light, sky and the gameplay
    /// switches a mod's `DimensionSettings` or a data pack sets. A level whose type neither gave
    /// fields for has the Overworld's. None for a stale handle; on a client, the level it shows.
    #[access(WorldOnly)]
    fn level_dimension_type(&self, level: LevelRef) -> Option<LevelDimensionType>;
    /// Side: server.
    /// `Level.getSeaLevel`: 63 in the Overworld, 32 in the Nether, 0 in the End; for a level of the
    /// noise generator the `sea_level` of its noise settings (a data pack's too, when it replaces
    /// them), for another level the `sea_level` its `DimensionSettings` gave, else 63. It is the sea
    /// level the game uses for the level: terrain and aquifers, mob spawning and mob behaviour, rain
    /// and snow, and maps. None for a stale handle and on a client.
    #[access(WorldOnly)]
    fn level_sea_level(&self, level: LevelRef) -> Option<i32>;
    /// Side: server.
    /// The world's seed (`WorldOptions.seed`), as `/seed` shows it.
    #[access(WorldOnly)]
    fn level_seed(&self, level: LevelRef) -> Option<i64>;
    /// Side: server.
    /// The difficulty (`Level.getDifficulty`): `peaceful`, `easy`, `normal`
    /// or `hard`, as `/difficulty` names it.
    #[access(WorldOnly)]
    fn level_difficulty(&self, level: LevelRef) -> Option<&'static str>;
    /// Side: server.
    /// The local difficulty at `pos` (`ServerLevel.getCurrentDifficultyAt`), as mobs spawned
    /// there are finalized with: the world difficulty scaled by the level's age, the chunk's
    /// inhabited time and the moon (both 0 for a chunk that isn't loaded). Its
    /// `special_multiplier` is vanilla's `getSpecialMultiplier`. None for a stale handle, a level
    /// the server has not opened, or a client.
    #[access(WorldOnly)]
    fn level_local_difficulty(&self, level: LevelRef, pos: [i32; 3]) -> Option<Difficulty>;
    /// Side: server.
    /// Read a registered boolean game rule.
    #[access(WorldOnly)]
    fn level_game_rule_bool(&self, level: LevelRef, key: BoolGameRuleKey) -> Option<bool>;
    /// Side: server.
    /// Game rule `rule` (`minecraft:keep_inventory` or `keep_inventory`), as
    /// `/gamerule` queries it: `Ok(None)` for an unknown rule. A gone/reopened level
    /// returns `Stale`; an unreadable side or phase returns `Unavailable`.
    #[access(WorldOnly)]
    fn level_game_rule(
        &self,
        level: LevelRef,
        rule: &str,
    ) -> Result<Option<GameRuleValue>, LevelReadError>;
    /// Side: server.
    /// Every game rule the world has (`GameRules`) with its current value, by `minecraft:` id
    /// (`max_minecart_speed` only with minecart improvements). A gone/reopened level
    /// returns `Stale`; an unreadable side or phase returns `Unavailable`.
    #[access(WorldOnly)]
    fn level_game_rules(
        &self,
        level: LevelRef,
    ) -> Result<Vec<(String, GameRuleValue)>, LevelReadError>;
    /// Side: both.
    /// The rain and thunder levels, each 0.0 to 1.0 (`Level.getRainLevel`,
    /// `getThunderLevel` at partial tick 1.0): both 0 in a level with no weather.
    /// Reads any open server level or the client's current synchronized level;
    /// None for a stale handle, an unavailable level or a client not logged in.
    #[access(WorldOnly)]
    fn level_weather(&self, level: LevelRef) -> Option<(f32, f32)>;
    /// Side: server.
    /// Whether it rains (`Level.isRaining`: a rain level above 0.2).
    #[access(WorldOnly)]
    fn level_is_raining(&self, level: LevelRef) -> Option<bool>;
    /// Side: server.
    /// `Level.isRainingAt(pos)` in the level a handle names, the one in place or a parked one:
    /// it rains there and `pos` is under open sky in a biome that rains. None for a stale
    /// handle, on a client, and where `pos` is not loaded or outside the world's height; a
    /// read never loads a chunk.
    #[access(WorldOnly)]
    fn level_is_raining_at(&self, level: LevelRef, pos: [i32; 3]) -> Option<bool>;
    /// Side: server.
    /// `LevelReader.enabledFeatures` of the level a handle names. Every level of a world shares
    /// the world's flags. None for a stale handle and on a client.
    #[access(WorldOnly)]
    fn level_enabled_features(&self, level: LevelRef) -> Option<FeatureFlags>;
    /// Side: server.
    /// Whether it thunders (`Level.isThundering`: a thunder level above 0.9).
    #[access(WorldOnly)]
    fn level_is_thundering(&self, level: LevelRef) -> Option<bool>;
    /// Side: server.
    /// The random of the server level `level` names: vanilla's `Level.random`
    /// (`level.getRandom()`), the one its explosions, block drops and other
    /// level draws use, so a mod's draws move vanilla's later ones as a Java
    /// mod's do. A parked level (the Nether while the server ticks the
    /// Overworld) answers with its own stream. This is a copy of it as it
    /// stands: draw with `LevelRef::with_random`, which stores the stream
    /// back, so the draws move the level's own. `None` on a client of a
    /// server, for a stale handle and for a level not open: the Nether and
    /// the End open when someone first goes there and close after 300 ticks
    /// with nobody in them (`close_idle_levels`).
    #[access(WorldOnly)]
    fn level_random(&self, level: LevelRef) -> Option<LegacyRandom>;
    /// Side: server.
    /// `Raids.get(id)`: the raid saved under `id` in the requested level, if
    /// `Raids` still holds it (a stopped raid goes at its next tick).
    #[access(WorldOnly)]
    fn level_raid(&self, level: LevelRef, id: i32) -> Option<RaidRef>;
    /// Side: server.
    /// `ServerLevel.getRaidAt(pos)`: the closest active raid whose centre is
    /// less than 96 blocks away (`Raids.getNearbyRaid`, 9216 squared).
    #[access(WorldOnly)]
    fn level_raid_at(&self, level: LevelRef, pos: [i32; 3]) -> Option<RaidRef>;
    /// Side: server.
    /// Every raid `Raids` holds, in its iteration order, stopped ones
    /// included until their next tick. None for a closed or stale level.
    #[access(WorldOnly)]
    fn level_raids(&self, level: LevelRef) -> Option<Vec<RaidRef>>;
    /// Side: server.
    /// The fluid state at `pos` of the level `level` names (`BlockGetter.getFluidState`). None
    /// for a stale handle, a chunk that isn't loaded or a `y` outside the world: a read never
    /// loads a chunk.
    #[access(WorldOnly)]
    fn level_fluid_state(&self, level: LevelRef, pos: [i32; 3]) -> Option<FluidState>;
    /// Side: both.
    /// Every loaded chunk of an open server level as handles, ordered by chunk x then z. None for
    /// a stale handle or on a client. Loads nothing.
    #[access(WorldOnly)]
    fn level_loaded_chunks(&self, level: LevelRef) -> Option<Vec<ChunkRef>>;
    /// Side: both.
    /// World-space block collision boxes under an empty collision context, without entities or border. Refuses a stale or nonresident level and invalid or overlarge boxes.
    /// On a client, the level it shows or a viewed one (`host.level`), each through its own
    /// world; None where the walk reaches a column the client doesn't hold.
    #[access(WorldOnly)]
    fn level_collision_boxes(&self, level: LevelRef, aabb: [f64; 6]) -> Option<Vec<[f64; 6]>>;
    /// Side: server.
    /// The world's respawn data (`LevelData.getRespawnData`): the world spawn and the way a
    /// player appearing there faces, as `/setworldspawn` sets them. Horizon keeps one world
    /// spawn, in the Overworld, so every level of a world answers the same. None on a client,
    /// for a stale handle, and before the world has chosen its spawn.
    #[access(WorldOnly)]
    fn level_respawn_data(&self, level: LevelRef) -> Option<RespawnData>;
    /// Side: server.
    /// Map `id`'s saved data (`ServerLevel.getMapData(new MapId(id))`): read from memory, or
    /// from `data/minecraft/maps/<id>.dat` when nothing has loaded it yet, without keeping it.
    /// Maps belong to the world, not to a level. None on a client, for a stale handle and for
    /// a map id nothing ever made.
    #[access(WorldOnly)]
    fn level_map_data(&self, level: LevelRef, id: i32) -> Option<MapState>;
    /// Side: server.
    /// `BlockAndTintGetter.canSeeSky`: the sky light at `pos` is full (15), so nothing above
    /// blocks it. None as for `brightness`.
    #[access(WorldOnly)]
    fn level_can_see_sky(&self, level: LevelRef, pos: [i32; 3]) -> Option<bool>;
    /// Side: server.
    /// Whether nothing solid lies on the segment `from`..`to`, as
    /// `LivingEntity.hasLineOfSight` clips between two eyes (`ClipContext.Block.COLLIDER`,
    /// `ClipContext.Fluid.NONE`): `clip`'s miss. Fluids, entities and the border don't block it.
    /// None as for `clip`.
    #[access(WorldOnly)]
    fn level_line_of_sight(&self, level: LevelRef, from: [f64; 3], to: [f64; 3]) -> Option<bool>;
    /// Side: both.
    /// Whether any block collision shape shares volume with `aabb` (`[min_x, min_y, min_z, max_x,
    /// max_y, max_z]`): the block half of `CollisionGetter.noCollision(aabb)`, negated. The same
    /// walk and limits as `collision_boxes`, stopping at the first shape it finds; entities and
    /// the world border are not counted. None as for `collision_boxes`.
    #[access(WorldOnly)]
    fn level_has_collisions(&self, level: LevelRef, aabb: [f64; 6]) -> Option<bool>;
    /// Side: both.
    /// The fluid's shape at `pos` (`FluidState.getShape`), as block-local boxes: none for no
    /// fluid, else one box from the floor to the fluid's height (`FluidState.getHeight`), a
    /// full block under the same fluid, else the fluid's own height (`amount / 9`). None as for
    /// `block_state`; an unloaded block above counts as no fluid there.
    #[access(WorldOnly)]
    fn level_fluid_shape(&self, level: LevelRef, pos: [i32; 3]) -> Option<Vec<[f64; 6]>>;
    /// Side: server.
    /// `Fluid.getTickDelay(level)`: the game ticks between a flow step of `fluid` at `pos` and the
    /// next. Water is 5, lava 30 (10 in the Nether, where it flows fast) and the empty fluid 0; a
    /// flowing form answers as its source. A mod's world fluid answers what it declared, as its
    /// `fluid_rules` parts change it at `pos`, floored at 1. None for a stale handle, a client,
    /// a level parked while the server ticks another, a position that isn't loaded or is outside
    /// the world, and a fluid with no world form.
    #[access(WorldOnly)]
    fn level_fluid_tick_delay(
        &self,
        level: LevelRef,
        fluid: FluidKey,
        pos: [i32; 3],
    ) -> Option<i32>;
    /// Side: both.
    /// `FluidState.getHeight(level, pos)` of the fluid at `pos`: 0 for none, a full block under
    /// the same fluid, else the fluid's own height (`amount / 9`). None as for `block_state`.
    /// An unloaded block above counts as no fluid there.
    #[access(WorldOnly)]
    fn level_fluid_height(&self, level: LevelRef, pos: [i32; 3]) -> Option<f32>;
    /// Side: both.
    /// The surface height of the fluid at `pos` that the camera's immersion test reads: a source
    /// with a block above whose underside is sturdy fills its cell (1), anything else is
    /// `fluid_height`. None as for `fluid_height`.
    #[access(WorldOnly)]
    fn level_fluid_height_for_camera(&self, level: LevelRef, pos: [i32; 3]) -> Option<f32>;
    /// Side: both.
    /// `FluidState.canBeReplacedWith(level, pos, fluid, direction)` for the fluid at `pos`: whether
    /// `fluid` flowing in `direction` takes its place. The empty fluid yields to anything, water
    /// only to a non-water fluid flowing down, lava to water once it stands 4/9 of a block high,
    /// and a mod's world fluid to nothing. False for a `fluid` with no world form. None as for
    /// `fluid_height`.
    #[access(WorldOnly)]
    fn level_fluid_can_be_replaced_with(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        fluid: FluidKey,
        direction: Direction,
    ) -> Option<bool>;
    /// Side: server.
    /// Every entity the segment from `from` to `to` passes through, nearest first: the test
    /// `ProjectileUtil.getEntityHitResult` makes on each candidate, each entity's box grown by
    /// `margin` on every side and clipped against the segment. A ray that starts inside a box
    /// does not hit that entity, as in vanilla. `exclude` (the shooter) is skipped. Vanilla's
    /// filter (`canHitEntity`, `isPickable`) is the caller's: take the first hit it accepts.
    /// Candidates are those `entities_in` finds, so spectators are not hit and the same kinds
    /// are left out. None for a negative or non-finite margin, a non-finite point, and where
    /// `entities_in` is None (a stale handle, a parked level).
    #[access(WorldOnly)]
    fn level_ray_trace_entities(
        &self,
        level: LevelRef,
        from: [f64; 3],
        to: [f64; 3],
        margin: f64,
        exclude: Option<EntityRef>,
    ) -> Option<Vec<EntityRayHit>>;
    /// Side: server.
    /// The level's natural spawning limits by category ([`SpawnLimits`]):
    /// [`SpawnLimits::VANILLA`] until a mod sets others. Reads parked levels.
    /// None on a client and for a stale handle.
    #[access(WorldOnly)]
    fn level_spawn_limits(&self, level: LevelRef) -> Option<SpawnLimits>;
    /// Side: server.
    /// Which categories the level's natural spawning may spawn ([`SpawnFlags`]):
    /// both on until a mod turns one off. Reads parked levels. None on a
    /// client and for a stale handle.
    #[access(WorldOnly)]
    fn level_spawn_flags(&self, level: LevelRef) -> Option<SpawnFlags>;
    /// Side: server.
    /// `ServerLevel.canSpawnEntitiesInChunk`: whether natural spawning may place
    /// a mob in chunk `pos` (`[x, z]`), the question it asks of every spawn
    /// position outside the chunk it started in. Horizon's answer is the one
    /// its spawner uses: the chunk is loaded. Reads parked levels and loads
    /// nothing. None on a client and for a stale handle.
    #[access(WorldOnly)]
    fn level_can_spawn_entities_in_chunk(&self, level: LevelRef, pos: [i32; 2]) -> Option<bool>;
    /// Side: server.
    /// The capability generation at `pos`: a number that changes when a provider says its
    /// answer there changed ([`LevelRef::invalidate_capabilities`]) and when the chunk of a
    /// position a provider invalidated unloads, and never comes back to a number it was while
    /// the level is open. [`BlockCapabilityCache`] compares it with the block entity and the
    /// block state there. Only the change means anything, not the number. Reads parked
    /// levels. None for a stale handle, a client, an unloaded position and one outside build
    /// height. Constant time, no allocation.
    #[access(WorldOnly)]
    fn level_capability_generation(&self, level: LevelRef, pos: [i32; 3]) -> Option<u64>;
}
/// The handles' writes of the `levels` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait LevelsBackendMut {
    /// Side: server.
    /// Mark the half-open box `min <= pos < max` as watered (`FarmlandWaterManager.addAABBTicket`).
    /// For 5x1x5 farmland starting at [0, 64, 0], use min [0, 64, 0], max [5, 65, 5].
    /// Farmland checks tickets before its vanilla water scan; rain and water still hydrate it.
    /// The calling mod owns this ticket; overlaps are independent. Remove it when the sprinkler
    /// stops. It is also removed on owner disable and level unload, never saved: re-add in
    /// `level_load` or the block entity's load listener. Reads/writes reach parked server levels.
    /// None outside an enabled mod call, on clients, for stale/unavailable levels, empty or
    /// reversed bounds, or a box spanning more than 65,536 x/z chunks. Does not load chunks.
    #[access(WorldOnly)]
    fn level_add_water_ticket(
        &mut self,
        level: LevelRef,
        min: [i32; 3],
        max: [i32; 3],
    ) -> Option<WaterTicket>;
    /// Side: server.
    /// Remove this watered area, leaving overlapping tickets intact. Only the owning enabled
    /// mod can remove it. False if already removed, stale, unloaded, or called on a client.
    #[access(WorldOnly)]
    fn water_ticket_remove(&mut self, water_ticket: WaterTicket) -> bool;
    /// Side: server.
    /// Spawns a mob of `kind` (`minecraft:zombie`) at `pos` in the level as
    /// `EntityType.spawn` does with `reason`: the entity type creates it (its
    /// Parts and attachments with it, and `entity_constructing`), with a random
    /// yaw from the level random, then `finalize_spawn` and `finalizeSpawn`
    /// with that reason (equipment, variants, jockeys), and it joins the level
    /// (`entity_join_level`), not persistent, as a natural spawn joins. The
    /// level's spawn limits, flags and spawn rules don't apply, as they don't
    /// to `/summon`. The new mob as a handle. None, with nothing spawned, for a
    /// kind that isn't a mob, a position that isn't finite or is outside the
    /// spawnable bounds (`Level.isInSpawnableBounds`), an unloaded chunk, a
    /// stale handle, a level not in place, a client, a `finalize_spawn` that
    /// cancelled the spawn or a mob a mod kept out.
    #[access(WorldOnly)]
    fn level_spawn_mob(
        &mut self,
        level: LevelRef,
        kind: &str,
        pos: [f64; 3],
        reason: SpawnReason,
    ) -> Option<EntityRef>;
    /// Side: server.
    /// Sets the level's natural spawning limits ([`SpawnLimits`]), from the next
    /// spawn pass on. They hold until the world closes, while the level is
    /// parked and across its closing and opening again, and are not saved: the
    /// world opens with vanilla's, so a mod sets its own in `level_load`. Mobs
    /// already in the level stay. False, with nothing set, on a client and for
    /// a stale handle or a level that isn't open.
    #[access(WorldOnly)]
    fn level_set_spawn_limits(&mut self, level: LevelRef, limits: SpawnLimits) -> bool;
    /// Side: server.
    /// Sets which categories the level's natural spawning may spawn
    /// ([`SpawnFlags`]), from the next spawn pass on. Kept and refused as
    /// `set_spawn_limits`.
    #[access(WorldOnly)]
    fn level_set_spawn_flags(&mut self, level: LevelRef, flags: SpawnFlags) -> bool;
    /// Side: server.
    /// Registers `passage` in the level ([`Passage`], #3775) and numbers it. The engine
    /// tells it to every Horizon client holding its chunk ([`Passage::chunk`]) at the end of
    /// the server's tick, and to a client that gets the chunk later as the chunk is sent
    /// (`chunk_sent`); a client forgets it with the chunk. Passages are not saved and a
    /// datapack reload does not bring them back: they hold until `remove_passage`, the
    /// level's close or the world's, so a mod registers its own again in `level_load`. An
    /// id is never given again while the game runs. A level holds 256 at most and a chunk 8.
    /// A passage removed before the tick's end is never told. Refused with nothing
    /// registered for a stale handle, a client, a passage that isn't a finite rectangle
    /// with unit axes at a right angle and sides above 0 and at most 32 blocks, one with a
    /// corner past vanilla's position limits, a full level or chunk, and once every id was
    /// given.
    #[access(WorldOnly)]
    fn level_register_passage(
        &mut self,
        level: LevelRef,
        passage: Passage,
    ) -> Result<PassageId, PassageRegistrationError>;
    /// Side: server.
    /// Removes the level's passage `id`: the clients that were told of it are told it is
    /// gone at the end of the server's tick. True when it was registered in this level.
    /// False for another level's or an unknown id, a stale handle and a client.
    #[access(WorldOnly)]
    fn level_remove_passage(&mut self, level: LevelRef, id: PassageId) -> bool;
    /// Side: server.
    /// `/forceload add` or `remove` of one chunk (`ServerLevel.setChunkForced`):
    /// true when the forced set changed. A forced chunk is kept loaded and
    /// entity ticking, saved with the level, and the level keeps ticking
    /// without a player. This is the world's own `/forceload` state, not a
    /// ticket the mod owns: the chunk stays forced across restarts, and after
    /// the mod is unloaded or removed, until `set_forced(false)` or
    /// `/forceload remove` clears it. Unlike vanilla, the chunk is not loaded before
    /// this returns: it loads over the next ticks, as after `/forceload`.
    /// False for a stale handle, on a client, and for a level the server
    /// has not opened.
    #[access(WorldOnly)]
    fn chunk_set_forced(&mut self, chunk: ChunkRef, forced: bool) -> bool;
    /// Side: server.
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
    #[access(WorldOnly)]
    fn chunk_set_mod_forced(
        &mut self,
        chunk: ChunkRef,
        controller: TicketController,
        owner: TicketOwner,
        forced: bool,
        natural_spawning: bool,
    ) -> bool;
    /// Side: server.
    /// Set a block (`Level.setBlock`). Stale/unavailable levels and invalid states return
    /// typed errors without mutation. Ok(false) means vanilla refused an unchanged,
    /// unloaded or out-of-height write. Notification follows the supplied update flags.
    #[access(WorldOnly)]
    fn level_set_block(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockState,
        flags: BlockUpdateFlags,
    ) -> Result<bool, LevelWriteError>;
    /// Side: server.
    /// Remove a block in place (`Level.removeBlock`), preserving its fluid.
    #[access(WorldOnly)]
    fn level_remove_block(&mut self, level: LevelRef, pos: [i32; 3], moved: bool) -> bool;
    /// Side: server.
    /// Check the block light at `pos` again (`LevelLightEngine.checkBlock`), at the level's
    /// next light update, as an edit there is checked; checking the same position again before
    /// then queues nothing. A block declaring `dynamic_light` is asked `light_emission` again
    /// first: the light engine never asks on its own, so a part whose answer reads more than
    /// the block's state (the block below, the depth) calls this when that changes. Clients take the new light from the server's light update and ask
    /// their own parts again. False, with nothing queued, off the level in place, for a stale
    /// handle, on a client and for a position that isn't loaded.
    #[access(WorldOnly)]
    fn level_check_light(&mut self, level: LevelRef, pos: [i32; 3]) -> bool;
    /// Side: server.
    /// Tells kept capabilities at `pos` that a provider's answer there changed with no change
    /// of block or block entity (NeoForge's `invalidateCapabilities`), such as a machine whose
    /// side configuration changed: the capability generation there moves on
    /// ([`LevelRef::capability_generation`]), so a [`BlockCapabilityCache`] there looks again
    /// at its next `get`. Inside a transaction it happens at the root commit, and an aborted
    /// one changes nothing. Block and block entity changes, chunk unloads and mod switches
    /// need no call: the cache sees them itself. False, with nothing changed, off the level in
    /// place, for a stale handle, on a client and for a position that isn't loaded.
    #[access(WorldOnly)]
    fn level_invalidate_capabilities(&mut self, level: LevelRef, pos: [i32; 3]) -> bool;
    /// Side: server.
    /// Destroy a block in the level in place (`Level.destroyBlock`), optionally dropping
    /// resources with `tool` (None is an empty hand). With `collect`, return the dropped
    /// stacks without spawning their item entities; otherwise return an empty list after
    /// spawning them. The one level drop path runs loot, block-drop hooks and experience
    /// once, in vanilla's random order. `drop` false suppresses resources entirely.
    /// None means nothing broke: air, a refused write, a stale handle, a client, an
    /// unloaded/out-of-height position, or a parked level.
    #[access(WorldOnly)]
    fn level_destroy_block(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        drop: bool,
        tool: Option<&ItemStack>,
        collect: bool,
    ) -> Result<Option<Vec<ItemStack>>, StackWriteError>;
    /// Side: server.
    /// `LevelData.setRespawnData(RespawnData.of(..))`, as `/setworldspawn` does: the world spawn
    /// moves to `pos`, facing `yaw` (wrapped to -180..180) and `pitch` (clamped to -90..90), a
    /// player whose own respawn point was the old world spawn follows it, and `spawn_change`
    /// fires. Only the Overworld's handle, where Horizon keeps the world spawn. False for
    /// another level, a stale handle, a client, a non-finite angle and a position outside the
    /// spawnable bounds.
    #[access(WorldOnly)]
    fn level_set_respawn_data(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        yaw: f32,
        pitch: f32,
    ) -> bool;
    /// Side: server.
    /// Sets the time of the level's own clock (its dimension type's `default_clock`, the one
    /// `level_clock_time` reads) to `time` ticks, as `/time set <time>` run in that level does:
    /// `time_skip` (cause `mod`) fires first and a mod can keep the old time, and the clock's
    /// fraction of a tick restarts. Players see the new time with the next time sync. A time
    /// the clock already shows is true and changes nothing. False for a negative time, a level
    /// without a default clock, a stale handle, a client, and when a mod kept the old time.
    #[access(WorldOnly)]
    fn level_set_clock_time(&mut self, level: LevelRef, time: i64) -> bool;
    /// Side: server.
    /// Sets game rule `rule` (`minecraft:keep_inventory` or `keep_inventory`) to `value`, as
    /// `/gamerule <rule> <value>` does (`GameRules.set` and `MinecraftServer.onGameRuleChanged`):
    /// the world follows it (`advance_time` freezes the clocks), `game_rule_changed` fires with
    /// no actor, and the players' clients get the rules they show. The rules are the world's,
    /// so any open level's handle sets them. True also when the rule already had the value.
    /// False for an unknown rule, a value of the other kind or outside the rule's range, a
    /// stale handle and a client.
    #[access(WorldOnly)]
    fn level_set_game_rule(&mut self, level: LevelRef, rule: &str, value: GameRuleValue) -> bool;
    /// Side: server.
    /// Repaint map `id` with `colors` (128 by 128 colour ids, row by row), pixel by pixel as
    /// `MapItemSavedData.setColor` does: each changed pixel marks the map to save, to redraw and
    /// to send to the players carrying or framing it. True when a pixel changed. False for
    /// another length, a stale handle, a client and a map id nothing made.
    #[access(WorldOnly)]
    fn level_set_map_data(&mut self, level: LevelRef, id: i32, colors: &[u8]) -> bool;
    /// Side: server.
    /// Run the block at `pos`'s scheduled tick now (`ServerLevel.tickBlock`): the path a due
    /// tick from the queue takes, the block's part first, else `BlockState.tick`, with the same
    /// random draws and level events. The queue is not read or changed; `schedule_block_tick`
    /// queues one. False for a refused write, a stale or parked handle, a client and an
    /// unloaded or out-of-height position.
    #[access(WorldOnly)]
    fn level_block_tick(&mut self, level: LevelRef, pos: [i32; 3]) -> bool;
    /// Side: server.
    /// Ask for the level to be saved: its level data, chunks and entities, as the autosave and
    /// `/save-all` save a level, with its chunks on disk when it ends. The save runs at the start
    /// of the next server tick, not inside the call, which may be in a hook of the very tick that
    /// is saving; a level asked for twice before then is saved once. True when the request was
    /// taken: the call cannot tell whether the save worked. `level_save` fires with the level's
    /// dimension once it was written; a save that fails fires nothing and is logged. False for a
    /// stale handle and on a client.
    #[access(WorldOnly)]
    fn level_request_save(&mut self, level: LevelRef) -> bool;
    /// Side: server.
    /// Moves the world border's centre (`WorldBorder.setCenter`), as `/worldborder center`
    /// does. Horizon keeps one border for every level, so this moves it everywhere; the
    /// players see it after the next sync. The centre it already has is true, with nothing
    /// heard. False for a centre that isn't finite or lies past 2.9999984E7 on x or z, a call
    /// from inside `world_border_center_changed`, a stale handle, a level the server has not
    /// opened or a client.
    #[access(WorldOnly)]
    fn level_set_world_border_center(&mut self, level: LevelRef, center: [f64; 2]) -> bool;
    /// Side: server.
    /// Sets the world border's size, as `/worldborder set`: at once when `ticks` is 0
    /// (`setSize`), otherwise growing or shrinking in a straight line over `ticks` game ticks
    /// from its size now (`lerpSizeBetween`). The one border every level shares. A still
    /// border set to the size it has at once is true, with nothing heard. False for a size
    /// outside 1 to 5.9999968E7, a negative `ticks`, a call from inside
    /// `world_border_bounds_changed`, a stale handle, a level the server has not opened or a
    /// client.
    #[access(WorldOnly)]
    fn level_set_world_border_size(&mut self, level: LevelRef, size: f64, ticks: i64) -> bool;
    /// Side: server.
    /// The world border's damage (`setDamagePerBlock`, `setSafeZone`), as `/worldborder
    /// damage amount` and `buffer`: a player outside the border and past `safe_zone` blocks
    /// takes `per_block` damage for each block beyond it. False for a value that is
    /// negative or not finite, a stale handle, a level the server has not opened or a client.
    #[access(WorldOnly)]
    fn level_set_world_border_damage(
        &mut self,
        level: LevelRef,
        per_block: f64,
        safe_zone: f64,
    ) -> bool;
    /// Side: server.
    /// The world border's warning (`setWarningBlocks`, `setWarningTime`), as `/worldborder
    /// warning distance` and `time`: a player's screen tints red within `distance` blocks of
    /// the border, or when a shrinking border will reach them within `ticks` game ticks.
    /// False for a negative value, a stale handle, a level the server has not opened or a
    /// client.
    #[access(WorldOnly)]
    fn level_set_world_border_warning(
        &mut self,
        level: LevelRef,
        distance: i32,
        ticks: i32,
    ) -> bool;
    /// Side: server.
    /// Sets the weather as `/weather` does (`ServerLevel.setWeatherParameters`): clear for
    /// `clear_ticks`, or raining (and thundering) for `weather_ticks`, after which the
    /// weather cycle goes on as usual. Rain and thunder then fade in or out as vanilla's do;
    /// `set_weather_levels` sets their strength at once. The `weather_change` hook may cancel
    /// all of it and `thunder_change` the thunder. The players see it after the next sync.
    /// False for a negative time, a call from inside `weather_change` or `thunder_change`, a
    /// parked level, a stale handle or a client.
    #[access(WorldOnly)]
    fn level_set_weather(
        &mut self,
        level: LevelRef,
        clear_ticks: i32,
        weather_ticks: i32,
        raining: bool,
        thundering: bool,
    ) -> bool;
    /// Side: server.
    /// How strong the rain and thunder are now (`Level.setRainLevel`, `setThunderLevel`),
    /// each clamped to 0 to 1 and set without fading. They still drift toward the weather
    /// the level has (`set_weather`) by 0.01 a tick, as vanilla's do. False for a value that
    /// isn't finite, a parked level, a stale handle or a client.
    #[access(WorldOnly)]
    fn level_set_weather_levels(&mut self, level: LevelRef, rain: f32, thunder: f32) -> bool;
    /// Side: server.
    /// Random-tick `pos` now, as `ServerLevel.tickChunk` does for a position it picked: the
    /// block's `randomTick` when it ticks randomly (its part first), then lava's fluid random
    /// tick, with the same random draws and level events. False as for `block_tick`.
    #[access(WorldOnly)]
    fn level_block_random_tick(&mut self, level: LevelRef, pos: [i32; 3]) -> bool;
    /// Side: server.
    /// Run the flow step of the fluid at `pos` now (`FluidState.tick`, which `ServerLevel.tickFluid`
    /// calls for a due fluid tick): the fluid levels, spreads and schedules its next step with
    /// the same random draws, hooks and level events as a queued one. Ticking the empty fluid does
    /// nothing. The queue is not read or changed; `schedule_fluid_tick` queues one. False for a
    /// refused write, a stale or parked handle, a client and an unloaded or out-of-height
    /// position.
    #[access(WorldOnly)]
    fn level_fluid_tick(&mut self, level: LevelRef, pos: [i32; 3]) -> bool;
    /// Side: server.
    /// `ServerLevel.findLightningTargetAround`: the closest lightning rod near the column at
    /// `pos`'s `x` and `z`, else a living entity under open sky within 3 blocks of it, picked
    /// with the level random as vanilla does (so this moves later draws), else the top of the
    /// column. Strikes nothing. None for a refused level, a stale or parked handle, a client and
    /// an unloaded column.
    #[access(WorldOnly)]
    fn level_lightning_target(&mut self, level: LevelRef, pos: [i32; 3]) -> Option<[i32; 3]>;
    /// Side: server.
    /// What the block at `pos` would drop (`Block.getDrops(state, level, pos, blockEntity,
    /// entity, tool)`), without breaking it or spawning anything: its loot table rolled with
    /// `tool` (None is an empty hand) and `entity` as the breaker, with what its block entity
    /// copies into the drops (a shulker box's contents, a banner's patterns, a pot's sherds).
    /// The roll draws from the table's random sequence, else the level random, as vanilla's
    /// does, so it moves later draws: that is why this is a write. No `block_drops` event,
    /// item entity or experience: that is `destroy_block`. None for a refused write, a stale or
    /// parked handle, a client, an unloaded or out-of-height position, and a stale `entity`.
    #[access(WorldOnly)]
    fn level_block_drops(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        tool: Option<&ItemStack>,
        entity: Option<EntityRef>,
    ) -> Result<Option<Vec<ItemStack>>, StackWriteError>;
    /// Side: server.
    /// One roll of the loot table `table` (`mymod:sifting/gravel`, the world's data packs
    /// merged), handed back instead of dropped: `LootTable.getRandomItems(LootParams)` with
    /// `origin` as ORIGIN, `tool` as TOOL when given and `entity` as THIS_ENTITY when given,
    /// the context `/loot … fish <table> <pos> <tool>` gives. The roll draws from the table's
    /// random sequence, else the level random, as vanilla's does, so it moves later draws: that
    /// is why this is a write. The stacks come split to their stack sizes, empties left out.
    /// Mods' loot kinds see `origin` and this level; no `loot_drops` event, since nothing
    /// drops. None for a table that isn't there, a stale or parked handle, a client, and a
    /// stale `entity`.
    #[access(WorldOnly)]
    fn level_roll_loot(
        &mut self,
        level: LevelRef,
        table: &str,
        origin: [f64; 3],
        tool: Option<&ItemStack>,
        entity: Option<EntityRef>,
    ) -> Result<Option<Vec<ItemStack>>, StackWriteError>;
    /// Side: server.
    /// Send `ServerLevel.destroyBlockProgress` in this level: stages 0..9 show a crack,
    /// any other stage removes it. A machine uses a stable negative breaker id.
    /// Inside a transaction, notices wait for the root commit in call order; abort publishes
    /// no notice. Outside a transaction this executes immediately. True means accepted or
    /// queued, not already delivered.
    #[access(WorldOnly)]
    fn level_set_break_progress(
        &mut self,
        level: LevelRef,
        breaker: i32,
        pos: [i32; 3],
        progress: i32,
    ) -> bool;
    /// Side: server.
    /// Tell the clients the block at `pos` changed (`Level.sendBlockUpdated`): its state and
    /// block entity data are sent again at the end of the tick, without writing anything, for
    /// a mod that changed what a block shows by other means than a block write. False, with
    /// nothing queued, for a stale or parked handle, a client, and an unloaded or out-of-height
    /// position. True with no player to notify (no one is in the level) when the position is
    /// valid: the chunk is marked changed and nothing is sent.
    /// Inside a transaction this waits for the root commit; abort publishes no notice.
    /// Outside a transaction this executes immediately. True means accepted or queued,
    /// not already delivered.
    #[access(WorldOnly)]
    fn level_send_block_updated(&mut self, level: LevelRef, pos: [i32; 3]) -> bool;
    /// Side: server.
    /// Plays sound event `sound` at `at` for the players in this level
    /// within range (16 blocks, times the volume above 1), as
    /// `ServerLevel.playSound(null, ...)` does, under `source`'s volume
    /// slider. A `minecraft:` event must be registered; a mod's goes by name
    /// and is heard where its resources define it. True means queued, not
    /// heard.
    #[access(WorldOnly)]
    fn level_play_sound(
        &mut self,
        level: LevelRef,
        sound: &str,
        at: [f64; 3],
        source: SoundSource,
        volume: f32,
        pitch: f32,
    ) -> bool;
    /// Side: server.
    /// Sends `count` particles around `at` to the players in this level
    /// within 32 blocks, as `ServerLevel.sendParticles` does: each offset is
    /// a Gaussian times `spread`, and `speed` scales their motion (a zero
    /// count sends one particle moving along `spread`). Block particle types
    /// need `state`; item particles need `options.item_stack` (validated against
    /// the item registry and stack limits). Trail and vibration remain unsupported.
    /// True means queued, not drawn.
    #[access(WorldOnly)]
    fn level_send_particles(
        &mut self,
        level: LevelRef,
        options: &ParticleOptions,
        state: Option<BlockState>,
        at: [f64; 3],
        spread: [f64; 3],
        speed: f64,
        count: i32,
    ) -> bool;
    /// Side: server.
    /// Plays level event `id` at `pos` with `data` for the players near it
    /// (`Level.levelEvent(null, id, pos, data)`): `1000`s are sounds, `2000`s
    /// particles, as vanilla's `LevelEvent` numbers them (`2001` breaks a
    /// block with state id `data`). No player is left out, the acting one
    /// included. False for an event that carries a block state (2001, 2014,
    /// 3008) when `data` isn't one.
    #[access(WorldOnly)]
    fn level_level_event(&mut self, level: LevelRef, id: i32, pos: [i32; 3], data: i32) -> bool;
    /// Side: server.
    /// Plays level event `id` at `pos` with `data` for every player on the
    /// server, in every level and at any distance (`ServerLevel.globalLevelEvent`).
    /// Vanilla's ids are the wither's spawn (`1023`), the dragon's death
    /// (`1028`) and an End portal opening (`1038`): the client has a sound for
    /// them only through this global form, played two blocks from the camera
    /// towards the position the player is sent. Each player gets a position
    /// of their own: `pos` in this level within 32 blocks of its centre, the
    /// block 32 blocks from them towards that centre further away, and the
    /// block they stand in for a player in another level.
    /// With the `global_sound_events` game rule off it is a plain
    /// [`level_event`](Self::level_level_event), as vanilla falls back: only
    /// the players of this level near `pos` get it. False for the same events
    /// and data `level_event` refuses, a parked level, a stale handle or a
    /// client.
    #[access(WorldOnly)]
    fn level_global_level_event(
        &mut self,
        level: LevelRef,
        id: i32,
        pos: [i32; 3],
        data: i32,
    ) -> bool;
    /// Side: server.
    /// Sends game event `event` (`minecraft:block_change`...) from `at`, as
    /// `Level.gameEvent(event, at, context)` does: sculk sensors, shriekers,
    /// wardens and allays receive the optional source and affected state.
    /// Returns false without emitting for an unknown event, invalid position,
    /// stale or foreign source, unregistered state, client or inactive level.
    #[access(WorldOnly)]
    fn level_game_event(
        &mut self,
        level: LevelRef,
        event: &str,
        at: [f64; 3],
        context: GameEventContext,
    ) -> bool;
    /// Side: server.
    /// Queues block event `a`, `b` for `block` at `pos`, as `ServerLevel.blockEvent(pos, block,
    /// a, b)` does: after this tick's scheduled ticks the level runs the block's
    /// `triggerEvent` with them (a powered piston's 0 extends it, a note block's plays its
    /// note, a bell's 1 rings it) if `pos` still holds `block`, and a handled event reaches the
    /// players' copy of the block. The same event queued twice runs once; one in a chunk
    /// that isn't ticking waits for it. True means queued, not handled. False for a block
    /// key no block has, a position outside the level's build height, a parked level, a stale
    /// handle or a client.
    #[access(WorldOnly)]
    fn level_block_event(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        block: BlockKey,
        a: i32,
        b: i32,
    ) -> bool;
    /// Side: server.
    /// Explodes at `center` as vanilla's full `Level.explode(source,
    /// damageSource, calculator, x, y, z, radius, fire, interaction,
    /// smallParticles, largeParticles, blockParticles, sound)` does
    /// (`native_explode`), `source` being the exploding entity, or none. The
    /// causing entity comes from it, never from the caller: a primed TNT's
    /// owner, a living entity itself, a projectile's owner when that is
    /// living, else none. With both the damage is
    /// `minecraft:player_explosion`, else `minecraft:explosion`, unless
    /// `settings` names a damage type, and a kill credits the causing entity.
    /// `interaction` says what happens to blocks (`Mob` asks the source's
    /// `entity_mob_griefing`, `Trigger` only works buttons, levers, doors and
    /// the like); `fire` lights some of the air around whatever it is.
    /// `settings` holds the damage calculator, particles and sound, each
    /// vanilla's default when `None`. There is no radius limit, as in
    /// vanilla: a big blast costs the blocks its rays find, and one of 0 or
    /// below finds none. Refused, with nothing exploded and no hook told, as
    /// [`ExplodeError`] says.
    #[access(WorldOnly)]
    fn level_explode(
        &mut self,
        level: LevelRef,
        source: Option<EntityRef>,
        center: [f64; 3],
        radius: f32,
        fire: bool,
        interaction: ExplosionInteraction,
        settings: ExplosionSettings,
    ) -> Result<(), ExplodeError>;
    /// Side: server.
    /// The resistance the blast `level.explode(host, source, center, radius, fire, interaction)`
    /// would set off would meet in the cell at `pos`, as its rays read it
    /// (`ExplosionDamageCalculator.getBlockExplosionResistance`), without exploding anything. The
    /// larger of the block's answer (its parts' `explosion_resistance` first, then its own value)
    /// and the fluid's (water and lava 100); `NoResistance` for air with no fluid. The blast's
    /// calculator is the one `explode` gives that source, read from the same selection: as
    /// vanilla's `EntityBasedExplosionDamageCalculator` asks the source, a primed TNT minecart's
    /// rails and the blocks under them resist 0 (and the blast spares them), and a dangerous
    /// wither skull caps a block it may destroy at 0.8; any other source has the default one.
    /// A respawn anchor's water cell
    /// belongs to the anchor's own sourceless blast, which no description names, so no read sees
    /// it. The causing entity and damage type come from `source` as for `explode`, and parts are
    /// asked with that `ExplosionStart`. `Unavailable` for a stale level or `source`, a chunk
    /// that isn't loaded (it is never loaded) or a state the game doesn't know; None only from a
    /// host without a game. The level does not change.
    #[access(WorldOnly)]
    fn level_explosion_resistance(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        source: Option<EntityRef>,
        center: [f64; 3],
        radius: f32,
        fire: bool,
        interaction: ExplosionInteraction,
    ) -> Option<ExplosionResistance>;
    /// Side: server.
    /// `level_explosion_resistance` for every cell of `cells`, one answer a cell in the order
    /// `cells` gives them. A state is asked once per call only when every part on its block that
    /// answers `explosion_resistance` declares the answer pure
    /// (`BlockPart::explosion_resistance_is_pure`); otherwise its parts are asked once a cell.
    /// Refused, with nothing read, for an inverted area or more than
    /// `ResistanceCells::MAX` (262,144) cells: it runs on the server tick, so read big areas in
    /// pieces.
    #[access(WorldOnly)]
    fn level_explosion_resistances(
        &mut self,
        level: LevelRef,
        cells: &ResistanceCells,
        source: Option<EntityRef>,
        center: [f64; 3],
        radius: f32,
        fire: bool,
        interaction: ExplosionInteraction,
    ) -> Result<Vec<ExplosionResistance>, ResistanceCellsError>;
    /// Side: server.
    /// Destroys the blocks at `positions` as the blast `level.explode(host, source, center,
    /// radius, fire, interaction)` would destroy them, without casting its rays, hurting or
    /// pushing entities, or any packet or sound: a mod that casts its own rays breaks what they
    /// found with it, spread over ticks if it likes. A Horizon addition. Each call is one
    /// explosion to the hooks: `explosion_start` first, which may cancel it (then nothing breaks
    /// and the answer is 0), then, for a radius of at least 0.00001, `explosion_detonate` with
    /// the positions as its block list, in the order given, a repeated one once; a listener may
    /// only take positions out (spare them), and one it adds is ignored. Then the block step in
    /// vanilla's order: the positions shuffled with the level's random, and each block hit as
    /// the blast hits one (air skipped; `can_drop_from_explosion`; loot with the radius under
    /// the decay interaction; experience when the causing entity is a player;
    /// `on_block_exploded`, whose default sets air and primes TNT owned by the causing entity),
    /// the drops of the whole call merged and popped after the last block; then, with `fire`, a
    /// third of the cells left air above a solid block catch fire, under `None` too. `None`
    /// breaks nothing else, and `Trigger` only works the blocks it may. Unlike vanilla's rays, a
    /// position in a chunk that isn't loaded is skipped, never loaded and not counted, and so is
    /// one outside the level's height. The number of blocks destroyed: those that were not air
    /// when their turn came. `Err`, with nothing changed, for more than 65,536 positions
    /// (`ExplosionDestroyError::MAX_POSITIONS`), a stale level or `source`, or a centre or
    /// radius that isn't finite or a negative radius.
    #[access(WorldOnly)]
    fn level_explosion_destroy_blocks(
        &mut self,
        level: LevelRef,
        positions: &[[i32; 3]],
        source: Option<EntityRef>,
        center: [f64; 3],
        radius: f32,
        fire: bool,
        interaction: ExplosionInteraction,
    ) -> Result<u32, ExplosionDestroyError>;
    /// Side: server.
    /// Strikes lightning at `at`, as summoning a `minecraft:lightning_bolt`
    /// does: it sets fire, hurts and converts what it hits (a pig to a
    /// zombified piglin...) and every player hears the thunder. With
    /// `visual_only` it only flashes and sounds (`setVisualOnly`). The
    /// `lightning_strike` hook hears it first with the cause `mod` and may
    /// cancel it. False for a chunk that isn't loaded or a canceled strike.
    #[access(WorldOnly)]
    fn level_strike_lightning(&mut self, level: LevelRef, at: [f64; 3], visual_only: bool) -> bool;
    /// Side: server.
    /// Drops `stack` as a broken block drops it (`Block.popResource`): an
    /// item entity at a random point within a quarter block of the middle of
    /// `pos` (three draws from the level random, made for an empty stack and
    /// with `block_drops` off too), with its own random motion and the
    /// default pickup delay of 10 ticks. None, with nothing spawned, for an
    /// empty stack or an unknown item, with the `block_drops` rule off, on a
    /// client, for a stale handle, a level not in place, or an item a mod
    /// kept out (`entity_join_level`).
    #[access(WorldOnly)]
    fn level_pop_resource(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        stack: ItemStack,
    ) -> Result<Option<EntityRef>, StackWriteError>;
    /// Side: server.
    /// Spawns `stack` as an item entity at `x y z` (`new ItemEntity(level, x,
    /// y, z, stack)` and `addFreshEntity`): its own random motion (up 0.2, up
    /// to 0.1 either way across), with `pickup_delay` ticks before it can be
    /// picked up, as `ItemEntity.setPickupDelay` at construction (vanilla
    /// passes 10 for a block's drop, 40 for a player's throw and 32767 for
    /// never). None, with nothing
    /// spawned, for an empty stack or an unknown item, a value that isn't
    /// finite, on a client, for a stale handle, a level not in place, or an
    /// item a mod kept out (`entity_join_level`).
    #[access(WorldOnly)]
    fn level_spawn_item(
        &mut self,
        level: LevelRef,
        x: f64,
        y: f64,
        z: f64,
        stack: ItemStack,
        pickup_delay: u32,
    ) -> Result<Option<EntityRef>, StackWriteError>;
    /// Side: server.
    /// `spawn_item` with the item entity's motion set to
    /// `motion` (blocks a tick), as `new ItemEntity(level, x, y, z, stack,
    /// deltaX, deltaY, deltaZ)`, and the same `pickup_delay` ticks before it
    /// can be picked up (`ItemEntity.setPickupDelay`). None in the same
    /// cases, and for a motion that isn't finite.
    #[access(WorldOnly)]
    fn level_spawn_item_with_motion(
        &mut self,
        level: LevelRef,
        x: f64,
        y: f64,
        z: f64,
        stack: ItemStack,
        motion: [f64; 3],
        pickup_delay: u32,
    ) -> Result<Option<EntityRef>, StackWriteError>;
    /// Side: server.
    /// Awards `amount` experience at `x y z` as vanilla's `ExperienceOrb.award`
    /// does: orbs of the largest sizes first (2477, 1237, ... 3, 1), each
    /// drawing the id `tryMergeToExisting` compares from the level random and
    /// merging into an orb of the same value whose box meets the 1 block box
    /// around `x y z`, or else a new orb with its own random motion. The
    /// `block_drops` rule doesn't apply (vanilla's blocks check it before
    /// they award). True once the orbs are awarded, even if a mod keeps a new
    /// one out (`entity_join_level`). False, with no orbs, for 0, a
    /// value that isn't finite, on a client, for a stale handle or a level
    /// not in place.
    #[access(WorldOnly)]
    fn level_award_experience(
        &mut self,
        level: LevelRef,
        x: f64,
        y: f64,
        z: f64,
        amount: u32,
    ) -> bool;
    /// Side: server.
    /// Tell the neighbours of `pos` that a block changed there, as vanilla's own writes do:
    /// `Neighbours` (`Level.updateNeighborsAt`: a lever's or a button's own block, and the one
    /// it is attached to, when the signal it gives changes), `NeighboursExcept(direction)`
    /// (`updateNeighborsAtExceptFromFacing`: a diode's front) or `OutputSignal`
    /// (`updateNeighbourForOutputSignal`: the comparators beside it, when its comparator output
    /// changes without its state changing). `source` is the state whose block vanilla passes as
    /// the one that changed. Typed errors refuse stale/unavailable levels or invalid states
    /// without mutation; Ok(false) means an unloaded or out-of-height position.
    #[access(WorldOnly)]
    fn level_update_neighbors(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        source: BlockState,
        update: NeighbourUpdate,
    ) -> Result<bool, LevelWriteError>;
    /// Side: server.
    /// Tell the block at `pos` that `source`'s block beside it changed (`Level.neighborChanged`),
    /// as a diode tells the block in front of it before updating that block's neighbours.
    /// Typed errors refuse stale/unavailable levels or invalid states; Ok(false) means an
    /// unloaded or out-of-height position. Errors never mutate the level.
    #[access(WorldOnly)]
    fn level_neighbor_changed(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        source: BlockState,
    ) -> Result<bool, LevelWriteError>;
    /// Side: server.
    /// Replace the random of the server level `level` names with `random`,
    /// a stream `native_level_random` gave and the mod drew from. False where
    /// that gives `None`.
    #[access(WorldOnly)]
    fn level_set_random(&mut self, level: LevelRef, random: LegacyRandom) -> bool;
    /// Side: both.
    /// Apply bone meal to the block at `pos` as a dispenser does (`BoneMealItem.growCrop`, then
    /// `growWaterPlant`), with no player and no item: the mods' `bonemeal` hook first, level event
    /// 1505 when something grew. True when something grew; false for a refused write, a stale or
    /// parked handle, a client, and a block that did not grow.
    #[access(WorldOnly)]
    fn level_bone_meal(&mut self, level: LevelRef, pos: [i32; 3]) -> bool;
    /// Side: both.
    /// Set the biome of the 4x4x4 cell holding `pos` to `biome` (`minecraft:plains`), as
    /// `/fillbiome` does for one cell. False for a refused write, a stale or parked handle, a
    /// client, an unknown biome and an unchanged cell.
    /// Tentative reads see the new cells inside a transaction. Abort restores the cells
    /// without publishing. Saving, remeshing and resending wait for the outermost commit,
    /// once per column. Outside a transaction they run immediately.
    #[access(WorldOnly)]
    fn level_set_biome(&mut self, level: LevelRef, pos: [i32; 3], biome: &str) -> bool;
}
/// The handles' client writes of the `levels` area, implemented by the game and reached through [`ClientBackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait LevelsClientBackendMut {
    /// Side: client.
    /// Show or clear the crack of `breaker` on the block at `pos` in this client's own view
    /// (`LevelRenderer.destroyBlockProgress`): stages 0..9 show a crack, any other stage removes
    /// it. Nothing is sent to the server or to other players, and a crack not updated for 400
    /// ticks goes away. False for a stale or foreign level or before login.
    ///
    /// ```compile_fail
    /// use mod_api::{LevelRef, ServerHost};
    /// fn wrong_side(level: LevelRef, host: &mut dyn ServerHost) {
    ///     level.set_client_break_progress(host, -1, [0, 64, 0], 3);
    /// }
    /// ```
    #[access(WorldOnly)]
    fn level_set_client_break_progress(
        &mut self,
        level: LevelRef,
        breaker: i32,
        pos: [i32; 3],
        progress: i32,
    ) -> bool;
}
