//! Resident client world: streamed chunk columns plus the bookkeeping the game
//! loop needs to relight and remesh after edits.
use crate::{
    chunk::{Chunk, ColumnLayout, LevelShape},
    fast_hash::{FxHashMap, FxHashSet},
    section_mask::SectionMask,
};
use horizon_core::random::{LegacyRandom, LevelRandom};
use horizon_world::registry::BlockStateId;
use std::sync::{Arc, OnceLock};

mod path_type_cache;
mod pending_entity;
pub use pending_entity::{EntityLoadReason, PendingEntity};

mod modified;
pub use modified::{ChunkMark, ModifiedChunks};

/// Sections (x, section index, z) waiting to be remeshed, kept by column
/// (a bit per section index), so the mesh scheduler looks at each column once
/// rather than at every section of it.
#[derive(Clone, Debug, Default)]
pub struct DirtySections {
    columns: crate::fast_hash::FxHashMap<(i32, i32), SectionMask>,
}
impl DirtySections {
    pub fn insert(&mut self, (x, sy, z): (i32, i32, i32)) {
        debug_assert!((0..SectionMask::CAPACITY as i32).contains(&sy));
        self.columns.entry((x, z)).or_default().insert(sy as usize);
    }
    /// Mark all sections in `mask` dirty for column `(x, z)`.
    pub fn insert_column_mask(&mut self, (x, z): (i32, i32), mask: SectionMask) {
        *self.columns.entry((x, z)).or_default() |= mask;
    }
    pub fn contains(&self, &(x, sy, z): &(i32, i32, i32)) -> bool {
        (0..SectionMask::CAPACITY as i32).contains(&sy)
            && self
                .columns
                .get(&(x, z))
                .is_some_and(|m| m.contains(sy as usize))
    }
    /// The dirty section indexes of column (x, z), removed.
    pub fn take_column(&mut self, x: i32, z: i32) -> SectionMask {
        self.columns.remove(&(x, z)).unwrap_or_default()
    }
    /// Keep the columns `keep` accepts.
    pub fn retain_columns(&mut self, mut keep: impl FnMut(i32, i32) -> bool) {
        self.columns.retain(|&(x, z), _| keep(x, z));
    }
    /// The columns holding dirty sections, with their section indexes.
    pub fn columns(&self) -> impl Iterator<Item = ((i32, i32), SectionMask)> + '_ {
        self.columns.iter().map(|(&c, &m)| (c, m))
    }
    pub fn clear(&mut self) {
        self.columns.clear();
    }
    pub fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }
    /// How many sections are dirty.
    pub fn len(&self) -> usize {
        self.columns.values().map(SectionMask::len).sum()
    }
}
pub struct World {
    // Initialized only for a logical server level, then borrowed by every AI worker.
    path_type_cache: OnceLock<path_type_cache::PathTypeCache>,
    /// The dimension type's fire-source tag, resolved at entry and data reload.
    pub infiniburn: Option<Arc<horizon_world::level::data::BlockSet>>,
    /// Server-owned mod data attached to this dimension.
    pub attachments: horizon_mod_runtime::attachments::Attachments,
    /// Mod-owned watered areas; never serialized with the world.
    pub water_tickets: horizon_mod_runtime::water_tickets::WaterTickets,
    /// The holders lent to mods since the last flush, whose synced changes
    /// `flush_attachment_sync` sends.
    pub attachment_syncs: FxHashSet<horizon_mod_runtime::attachments::Holder>,
    /// Block entities a moving piston landed since the last flush, whose
    /// synced attachments `flush_attachment_sync` sends whole (#3184).
    pub attachment_landings: FxHashSet<horizon_mod_runtime::attachments::Holder>,
    /// The capability generations mods' providers advanced here
    /// (`level_invalidate_capabilities`), which kept capabilities compare.
    pub capability_generations: horizon_mod_runtime::capability_generations::CapabilityGenerations,
    /// The level's build range and skylight (its dimension type); its
    /// chunks hold `shape.column()`.
    pub shape: LevelShape,
    pub fast_lava: bool,
    pub chunks: crate::chunk_map::ChunkMap,
    pub random: LevelRandom,
    pub revision: u64,
    pub health_rules: HealthRules,
    pub air: BlockStateId,
    /// Sections whose geometry changed and need remeshing.
    pub dirty_sections: DirtySections,
    /// Chunks whose light must be recomputed.
    pub dirty_light: crate::fast_hash::FxHashSet<(i32, i32)>,
    /// Edits on lit chunks (position, old state, new state), relit
    /// incrementally by streaming.
    pub light_edits: Vec<((i32, i32, i32), u32, u32)>,
    /// Where each light check queued since the last ask sits in `light_edits`, so a repeated
    /// check at the same position queues nothing, as `LevelLightEngine.checkBlock` queues into a
    /// set. An index that no longer holds its position (drained, rolled back) is ignored.
    pub light_checks: crate::fast_hash::FxHashMap<(i32, i32, i32), usize>,
    /// Saved entities of chunks that just loaded, spawned by the game.
    pub loaded_entities: Vec<((i32, i32), Vec<PendingEntity>)>,
    /// Chunks loaded since the level's last pass, their entity sections not
    /// ready yet (`PersistentEntitySectionManager`'s PENDING), empty ones too:
    /// the pass ends by readying them (`processPendingLoads`).
    pub entities_pending: FxHashSet<(i32, i32)>,
    /// Chunks that just came into this level from a worker, and whether each
    /// was generated rather than read from the save: the game tells native mods.
    pub loaded_chunks: Vec<(i32, i32, bool)>,
    /// Chunks changed since the last save.
    pub modified: ModifiedChunks,
    /// Block entities, and the order they tick in.
    pub block_entities: crate::block_entities::BlockEntities,
    /// Carrier transfer fences: the world cells and carriers locked
    /// while a carrier operation runs between prepared and committed
    /// (the game's carrier transfers). Empty, one load, when no
    /// operation is in flight.
    pub carrier_fences: crate::carrier_fences::Fences,
    /// Whether the world's `data/` folder already existed when the
    /// world opened — the carrier store's first-use gate. Captured at
    /// open, not at transfer time, so a folder created this session
    /// (by the first save, say) cannot pass for an old one.
    pub carrier_data_existed_at_open: bool,
    /// Each chest's `ContainerOpenersCounter`: opener count and largest
    /// interaction range.
    pub chest_openers: FxHashMap<(i32, i32, i32), (i32, f64)>,
    /// `BaseContainerBlockEntity.name`: custom names of container block entities.
    pub block_names: FxHashMap<(i32, i32, i32), horizon_core::nbt::Tag>,
    /// States (by id) whose writes are recorded in `watched_writes` (sculk
    /// blocks, which need a block entity).
    pub watched: Vec<bool>,
    /// Writes of watched states: position, previous and new state.
    pub watched_writes: Vec<((i32, i32, i32), u32, u32)>,
    /// `ChunkAccess.inhabitedTime` of resident chunks: ticks a player spent
    /// within spawning range (local difficulty).
    pub inhabited: FxHashMap<(i32, i32), i64>,
    pub game_time: i64,
    pub day_time: i64,
    /// The End's world clock (`WorldClocks.THE_END`, the End sky flash's), saved apart from
    /// the game time and, like every clock, still while `advance_time` is off.
    pub end_clock: i64,
    pub weather: Weather,
    /// Every block position written since the log was last drained, kept
    /// only while the world is open to the network (see `lan`).
    pub block_log: Option<Vec<(i32, i32, i32)>>,
    /// Writes of this tick's block events and block entities, held out of
    /// the log while it is sent: they go out with the next tick's.
    pub block_log_held: Vec<(i32, i32, i32)>,
    /// Every write since the mobs last ticked (position, old and new state),
    /// for `ServerLevel.sendBlockUpdated`'s path recomputing; kept only while
    /// the mobs of this world tick (`game/mobs/creatures/mod.rs`).
    pub path_writes: Option<Vec<((i32, i32, i32), u32, u32)>>,
    /// The chunks changed in bulk (`edit_runs`) since the log was last
    /// drained, kept while `block_log` is: sent to the players whole.
    pub bulk_log: FxHashSet<(i32, i32)>,
    /// The sections whose light changed since the log was last drained, as
    /// a bit per section of each column, kept while `block_log` is
    /// (`ChunkHolder.sectionLightChanged`).
    pub light_log: Option<crate::fast_hash::FxHashMap<(i32, i32), SectionMask>>,
    /// The villager POIs of each section (`PoiManager`'s sections).
    pub pois: crate::poi_sections::PoiSections<horizon_world::poi::PoiIdentity>,
    /// Moving-piston states removed during the last tick; retained until the
    /// final block mesh is visible.
    pub moving_piston_fallback: Vec<([i32; 3], horizon_world::piston_shape::MovingPistonState)>,
    /// The moving-piston block entities of the level, by position, copied
    /// from the block ticks after each tick: collision and rendering read them.
    pub moving_pistons: Vec<([i32; 3], horizon_world::piston_shape::MovingPistonState)>,
    /// The `ShulkerBoxBlockEntity` lid state of the level, by position, copied
    /// from the block entities after each tick: the collision queries read it
    /// (`ShulkerBoxBlockEntity.getBoundingBox`), as `moving_pistons` for pistons.
    pub shulkers: FxHashMap<[i32; 3], crate::block_entity_anim::Shulker>,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Weather {
    pub clear_time: i32,
    pub rain_time: i32,
    pub thunder_time: i32,
    pub raining: bool,
    pub thundering: bool,
    pub rain_level: (f32, f32),
    pub thunder_level: (f32, f32),
}
impl Weather {
    /// The timers of `advanceWeatherCycle`, which run only while
    /// `advance_weather` is on.
    pub fn tick_timers(&mut self, random: &mut horizon_core::random::LegacyRandom) {
        use horizon_core::random::RandomSource;
        let mut uniform =
            |min: i32, max: i32| min + random.next_int_bound(max - min + 1).unwrap_or(0);
        if self.clear_time > 0 {
            self.clear_time -= 1;
            self.thunder_time = if self.thundering { 0 } else { 1 };
            self.rain_time = if self.raining { 0 } else { 1 };
            self.thundering = false;
            self.raining = false;
        } else {
            if self.thunder_time > 0 {
                self.thunder_time -= 1;
                if self.thunder_time == 0 {
                    self.thundering = !self.thundering;
                }
            } else if self.thundering {
                self.thunder_time = uniform(3600, 15600);
            } else {
                self.thunder_time = uniform(12000, 180000);
            }
            if self.rain_time > 0 {
                self.rain_time -= 1;
                if self.rain_time == 0 {
                    self.raining = !self.raining;
                }
            } else if self.raining {
                self.rain_time = uniform(12000, 24000);
            } else {
                self.rain_time = uniform(12000, 180000);
            }
        }
    }
    /// The rain and thunder level fade at the end of `advanceWeatherCycle`,
    /// which runs whatever `advance_weather` says, so `/weather` still shows.
    pub fn tick_levels(&mut self) {
        self.thunder_level.0 = self.thunder_level.1;
        self.thunder_level.1 =
            (self.thunder_level.1 + if self.thundering { 0.01 } else { -0.01 }).clamp(0.0, 1.0);
        self.rain_level.0 = self.rain_level.1;
        self.rain_level.1 =
            (self.rain_level.1 + if self.raining { 0.01 } else { -0.01 }).clamp(0.0, 1.0);
    }
    /// The levels at their targets at once (rain 1 while it rains, else 0, and thunder the
    /// same), both halves, so the frame shows them at any partial tick: a frozen world under
    /// `HORIZON_STILL`, which would otherwise have to run about a hundred ticks to fade them.
    pub fn settle_levels(&mut self) {
        let target = |on: bool| if on { 1.0 } else { 0.0 };
        self.rain_level = (target(self.raining), target(self.raining));
        self.thunder_level = (target(self.thundering), target(self.thundering));
    }
    /// Level.getRainLevel / getThunderLevel (thunder scales with rain).
    pub fn levels(&self, partial: f32) -> (f32, f32) {
        let rain = self.rain_level.0 + (self.rain_level.1 - self.rain_level.0) * partial;
        let thunder =
            self.thunder_level.0 + (self.thunder_level.1 - self.thunder_level.0) * partial;
        (rain, thunder * rain)
    }
    /// `ClientPacketListener.handleGameEvent`'s weather events, the player's level's or a viewed
    /// level's: START_RAINING (1), STOP_RAINING (2), RAIN_LEVEL_CHANGE (7) and
    /// THUNDER_LEVEL_CHANGE (8), with the event's `param`. Any other event is not the weather's.
    pub fn game_event(&mut self, event: u8, param: f32) {
        match event {
            1 => {
                self.raining = true;
                self.rain_level = (0.0, 0.0);
            }
            2 => {
                self.raining = false;
                self.rain_level = (1.0, 1.0);
            }
            7 => {
                self.rain_level = (param, param);
                self.raining = param > 0.0;
            }
            8 => {
                self.thunder_level = (param, param);
                self.thundering = param > 0.0;
            }
            _ => {}
        }
    }
}
impl World {
    /// Install loaded instances, retiring an existing column's block entities even
    /// when the new payload omits their positions. Initial loads need no world scan.
    pub fn load_chunk_block_entities(
        &mut self,
        chunk: (i32, i32),
        entities: Vec<(
            crate::block_entities::Pos,
            crate::chunk::block_entity::BlockEntity,
        )>,
    ) {
        if self.chunks.contains_key(&chunk) {
            let incoming: FxHashSet<_> = entities.iter().map(|(pos, _)| *pos).collect();
            self.block_entities
                .unload(|p| (p.0 >> 4, p.2 >> 4) == chunk && !incoming.contains(p));
        }
        self.block_entities.load_chunk(entities);
    }
    /// A loaded column's block entities, as fresh instances, with the attachment maps its save
    /// kept for them.
    pub fn install_chunk_block_entities(&mut self, chunk: &mut crate::chunk::Chunk) {
        let key = (chunk.x, chunk.z);
        self.load_chunk_block_entities(key, std::mem::take(&mut chunk.block_entities));
        if let Some(extras) = &chunk.saved {
            for (pos, attachments) in &extras.block_entity_attachments {
                self.block_entities
                    .restore_attachments(*pos, attachments.clone());
            }
        }
    }
    /// Queue ordinary NBT; transient transfer state is never decoded from it.
    pub fn queue_entity_nbt(&mut self, key: (i32, i32), tags: Vec<horizon_core::nbt::Tag>) {
        self.loaded_entities
            .push((key, tags.into_iter().map(PendingEntity::from).collect()));
    }

    pub fn new(air: BlockStateId, seed: i64) -> Self {
        Self {
            path_type_cache: OnceLock::new(),
            infiniburn: None,
            shape: LevelShape::OVERWORLD,
            chunks: Default::default(),
            random: LevelRandom::for_level(LegacyRandom::new(seed)),
            revision: 0,
            health_rules: HealthRules::default(),
            air,
            fast_lava: false,
            dirty_sections: Default::default(),
            dirty_light: Default::default(),
            light_edits: Vec::new(),
            light_checks: Default::default(),
            loaded_entities: Vec::new(),
            entities_pending: FxHashSet::default(),
            loaded_chunks: Vec::new(),
            modified: ModifiedChunks::default(),
            attachments: Default::default(),
            water_tickets: Default::default(),
            attachment_syncs: FxHashSet::default(),
            attachment_landings: FxHashSet::default(),
            capability_generations: Default::default(),
            block_entities: Default::default(),
            carrier_fences: Default::default(),
            carrier_data_existed_at_open: false,
            chest_openers: FxHashMap::default(),
            block_names: FxHashMap::default(),
            watched: Vec::new(),
            watched_writes: Vec::new(),
            inhabited: FxHashMap::default(),
            game_time: 0,
            day_time: 1000,
            end_clock: 0,
            weather: Weather::default(),
            block_log: None,
            block_log_held: Vec::new(),
            path_writes: None,
            bulk_log: Default::default(),
            light_log: None,
            moving_piston_fallback: Vec::new(),
            moving_pistons: Vec::new(),
            shulkers: FxHashMap::default(),
            pois: Default::default(),
        }
    }
    /// A controlled blockstate scene for the native scene probes: material
    /// layers, a stepped pyramid and the model gallery. Not generated terrain.
    pub fn test_scene(
        assets: &crate::assets::Assets,
        render: &crate::assets::RenderAssets,
        radius: i32,
    ) -> crate::Result<Self> {
        let air = assets
            .registry
            .default_state("minecraft:air")
            .ok_or("missing air state")?;
        let mut world = Self::new(air, 0);
        for cz in -radius..=radius {
            for cx in -radius..=radius {
                world
                    .chunks
                    .insert((cx, cz), Arc::new(Chunk::empty(cx, cz)));
                for z in 0..16 {
                    for x in 0..16 {
                        for y in 0..4 {
                            let layer = match y {
                                0 => 0,
                                1 | 2 => 1,
                                _ if (cx + cz) & 1 == 0 => 2,
                                _ => 3,
                            };
                            world.set(
                                cx * 16 + x,
                                y,
                                cz * 16 + z,
                                assets.material_states[layer],
                            )?;
                        }
                    }
                }
            }
        }
        for y in 4..11 {
            for z in -10_i32..=10 {
                for x in -10_i32..=10 {
                    if x.abs().max(z.abs()) <= 10 - (y - 4) {
                        world.set(
                            x,
                            y,
                            z,
                            assets.material_states[if y & 1 == 0 { 4 } else { 5 }],
                        )?;
                    }
                }
            }
        }
        for (position, state) in &render.model_placements {
            world.set(position.x, position.y, position.z, *state)?;
        }
        world.revision = 0;
        Ok(world)
    }
    /// The column the level's chunks hold.
    pub fn layout(&self) -> ColumnLayout {
        self.shape.column()
    }
    /// The lowest block Y the level's chunks hold.
    pub fn bottom_y(&self) -> i32 {
        self.layout().min_y()
    }
    /// `Entity.checkBelowWorld`'s floor, `Level.getMinY() - 64`: the level's own
    /// lowest Y (0 in the Nether and the End), not the chunks' widened column.
    /// An entity below it is out of the world.
    pub fn below_world_y(&self) -> f64 {
        f64::from(self.shape.min_y - 64)
    }
    /// One above the highest block Y the level's chunks hold.
    pub fn top_y(&self) -> i32 {
        self.layout().max_y()
    }
    /// Writes the stored state at a resident position and nothing else: no remesh, relight,
    /// block log, path or watched write, and no block entity change. A transaction's tentative
    /// write, which `set` makes real at the root commit or which this takes back on an abort,
    /// so nothing outside sees it. False outside the resident chunks or the world's height.
    pub fn set_stored(&mut self, x: i32, y: i32, z: i32, state: BlockStateId) -> bool {
        let Some(chunk) = self.chunks.get_mut(&(x >> 4, z >> 4)) else {
            return false;
        };
        if chunk.section_index(y).is_none() {
            return false;
        }
        Arc::make_mut(chunk).set(x & 15, y, z & 15, state.0);
        true
    }
    /// Resident storage write; marks the touched sections (and neighbours on
    /// section borders) for remeshing and the surrounding chunks for relighting.
    /// Neighbour updates, drops and block entities belong to the gameplay layer.
    pub fn set(&mut self, x: i32, y: i32, z: i32, state: BlockStateId) -> crate::Result<()> {
        self.set_with_notification(x, y, z, state, true)
    }
    /// Store a state while controlling whether the change enters the client block log.
    pub fn set_with_notification(
        &mut self,
        x: i32,
        y: i32,
        z: i32,
        state: BlockStateId,
        notify: bool,
    ) -> crate::Result<()> {
        let Some(chunk) = self.chunks.get_mut(&(x >> 4, z >> 4)) else {
            return Err("write outside resident chunks".into());
        };
        let (min_y, sections) = (chunk.min_y(), chunk.sections.len() as i32);
        if chunk.section_index(y).is_none() {
            return Err("write outside world height".into());
        }
        let previous = Arc::make_mut(chunk).set(x & 15, y, z & 15, state.0);
        if previous != state.0 {
            self.block_entities
                .state_changed((x, y, z), previous, state.0);
            let watches = |id: u32| self.watched.get(id as usize).copied().unwrap_or(false);
            if watches(previous) || watches(state.0) {
                self.watched_writes.push(((x, y, z), previous, state.0));
            }
            self.revision = self.revision.wrapping_add(1);
            if notify && let Some(log) = &mut self.block_log {
                log.push((x, y, z));
            }
            if let Some(writes) = &mut self.path_writes {
                writes.push(((x, y, z), previous, state.0));
            }
            self.modified.insert((x >> 4, z >> 4));
            // The mesher reads a 3x3x3 neighbourhood for AO, so a write marks
            // the sections it can reach: this section, the one above/below
            // only on a section face, and a neighbour column only on a chunk
            // border (the diagonal only where both borders meet).
            let sy = (y - min_y) >> 4;
            let local_y = y & 15;
            let mark = |dirty: &mut DirtySections, bx: i32, bz: i32| {
                dirty.insert((bx >> 4, sy, bz >> 4));
                if local_y == 0 && sy > 0 {
                    dirty.insert((bx >> 4, sy - 1, bz >> 4));
                }
                if local_y == 15 && sy + 1 < sections {
                    dirty.insert((bx >> 4, sy + 1, bz >> 4));
                }
            };
            let (bx, bz) = (x & 15, z & 15);
            mark(&mut self.dirty_sections, x, z);
            if bx == 0 {
                mark(&mut self.dirty_sections, x - 1, z);
            } else if bx == 15 {
                mark(&mut self.dirty_sections, x + 1, z);
            }
            if bz == 0 {
                mark(&mut self.dirty_sections, x, z - 1);
            } else if bz == 15 {
                mark(&mut self.dirty_sections, x, z + 1);
            }
            if (bx == 0 || bx == 15) && (bz == 0 || bz == 15) {
                mark(
                    &mut self.dirty_sections,
                    if bx == 0 { x - 1 } else { x + 1 },
                    if bz == 0 { z - 1 } else { z + 1 },
                );
            }
            if self.chunks.get(&(x >> 4, z >> 4)).is_some_and(|c| c.lit) {
                self.light_edits.push(((x, y, z), previous, state.0));
            } else {
                for dz in -1..=1 {
                    for dx in -1..=1 {
                        self.dirty_light.insert(((x >> 4) + dx, (z >> 4) + dz));
                    }
                }
            }
        }
        Ok(())
    }
    /// `getHeight(Heightmap.Types.WORLD_SURFACE, x, z)`: one above the highest
    /// block that isn't `air`, or the column's minimum Y (also where the chunk
    /// isn't loaded).
    pub fn world_surface(&self, x: i32, z: i32, air: impl Fn(BlockStateId) -> bool) -> i32 {
        let bottom = self.layout().min_y();
        self.chunks.get(&(x >> 4, z >> 4)).map_or(bottom, |chunk| {
            chunk.world_surface(x, z, |v| air(BlockStateId(v)))
        })
    }
    /// `getHeight(Heightmap.Types.MOTION_BLOCKING, x, z)`: one above the
    /// highest block where `blocks` (motion blocking or holding a fluid),
    /// or the column's minimum Y (also where the chunk isn't loaded).
    pub fn motion_blocking(&self, x: i32, z: i32, blocks: impl Fn(BlockStateId) -> bool) -> i32 {
        let bottom = self.layout().min_y();
        self.chunks.get(&(x >> 4, z >> 4)).map_or(bottom, |chunk| {
            chunk.motion_blocking(x & 15, z & 15, |v| blocks(BlockStateId(v)))
        })
    }
    #[inline]
    pub fn block(&self, x: i32, y: i32, z: i32) -> BlockStateId {
        self.chunks
            .get(&(x >> 4, z >> 4))
            .map_or(self.air, |chunk| chunk.block(x & 15, y, z & 15))
    }
    /// Whether a block of the box `lo..=hi` may match, from the palettes of
    /// the sections it reaches (a palette of the whole registry may hold
    /// anything): `false` means [`Self::block`] matches in none of its cells,
    /// so a scan over them can stop before reading one.
    pub fn may_hold(
        &self,
        lo: [i32; 3],
        hi: [i32; 3],
        mut matches: impl FnMut(BlockStateId) -> bool,
    ) -> bool {
        let layout = self.layout();
        let (bottom, top) = (layout.min_y(), layout.max_y() - 1);
        // Heights outside the sections read as state 0.
        if (lo[1] < bottom || hi[1] > top) && matches(BlockStateId(0)) {
            return true;
        }
        let sections = (lo[1].max(bottom) - bottom) >> 4..=(hi[1].min(top) - bottom) >> 4;
        for cx in lo[0] >> 4..=hi[0] >> 4 {
            for cz in lo[2] >> 4..=hi[2] >> 4 {
                let Some(chunk) = self.chunks.get(&(cx, cz)) else {
                    if matches(self.air) {
                        return true;
                    }
                    continue;
                };
                for s in sections.clone() {
                    let blocks = &chunk.sections[s as usize].blocks;
                    if blocks.is_global() || blocks.maybe_has(|v| matches(BlockStateId(v))) {
                        return true;
                    }
                }
            }
        }
        false
    }
    /// The block entities to tick this tick, in order (`BlockEntities::tickers`).
    pub fn block_entity_tickers(
        &mut self,
    ) -> std::sync::Arc<Vec<(crate::block_entities::Pos, u32)>> {
        let (chunks, air) = (&self.chunks, self.air);
        self.block_entities.tickers(|(x, y, z)| {
            chunks
                .get(&(x >> 4, z >> 4))
                .map_or(air, |chunk| chunk.block(x & 15, y, z & 15))
                .0
        })
    }
    pub fn light(&self, x: i32, y: i32, z: i32) -> u8 {
        self.chunks
            .get(&(x >> 4, z >> 4))
            .map_or(0xF0, |chunk| chunk.light(x & 15, y, z & 15))
    }
    pub fn is_air(&self, state: BlockStateId) -> bool {
        state == self.air
    }
    pub fn loaded(&self, x: i32, z: i32) -> bool {
        self.chunks.contains_key(&(x >> 4, z >> 4))
    }
    /// Takes chunk `key` out of the level as it unloads, with the capability generations its
    /// positions held.
    pub fn unload_chunk(&mut self, key: (i32, i32)) -> Option<Arc<Chunk>> {
        self.capability_generations.unload_chunk(key);
        self.chunks.remove(&key)
    }
    /// `Level.isLoaded`: `y` is inside the build height and the chunk of `(x, z)` is loaded.
    /// Never loads a chunk.
    pub fn is_loaded_at(&self, x: i32, y: i32, z: i32) -> bool {
        self.shape.contains_y(y) && self.loaded(x, z)
    }
    /// Persisted container capacity override; absence preserves vanilla capacity.
    pub fn container_stack_limit(&self, key: (i32, i32, i32)) -> Option<u32> {
        self.chunks
            .get(&(key.0 >> 4, key.2 >> 4))
            .and_then(|c| c.container_stack_limits.get(&key).copied())
    }
    /// Store with the authoritative saved block entity fields.
    pub fn set_container_stack_limit(&mut self, key: (i32, i32, i32), limit: u32) {
        use horizon_core::nbt::{Compound, Tag};
        self.with_chunk_extras(key, |extras| {
            if !extras.block_entity_sources.iter().any(|(p, _)| *p == key) {
                extras
                    .block_entity_sources
                    .push((key, Tag::Compound(Compound::new())));
            }
            if let Some((_, Tag::Compound(tag))) = extras
                .block_entity_sources
                .iter_mut()
                .find(|(p, _)| *p == key)
            {
                tag.insert("horizon:stack_limit".into(), Tag::Int(limit as i32));
            }
        });
    }
    /// `LockCode`: the `lock` a container block entity refuses to open
    /// without, as the tag it was loaded or set with carries it (Horizon's
    /// model holds no field for it).
    pub fn block_lock(&self, key: (i32, i32, i32)) -> Option<&horizon_core::nbt::Tag> {
        self.chunks
            .get(&(key.0 >> 4, key.2 >> 4))?
            .saved
            .as_ref()?
            .block_entity_sources
            .iter()
            .find(|(p, _)| *p == key)?
            .1
            .get("lock")
    }
    /// `LockCode.addToTag` for a tag a command gives a container (`setblock`,
    /// `data merge`): the tag it was set with keeps the lock, so `data get`
    /// shows it, a player's open checks it and the save writes it.
    pub fn set_block_lock(&mut self, key: (i32, i32, i32), tag: &horizon_core::nbt::Tag) {
        use horizon_core::nbt::Tag;
        let lock = tag.get("lock").and_then(|lock| match lock {
            Tag::Compound(_) => Some(lock.clone()),
            _ => None,
        });
        let mut tag = tag.clone();
        if let Tag::Compound(map) = &mut tag {
            map.remove(&"LootTable".into());
            map.remove(&"LootTableSeed".into());
        }
        self.with_chunk_extras(key, |extras| {
            let at = extras
                .block_entity_sources
                .iter()
                .position(|(p, _)| *p == key);
            match at {
                Some(i) => {
                    if let Tag::Compound(source) = &mut extras.block_entity_sources[i].1 {
                        match lock {
                            Some(lock) => {
                                source.insert("lock".into(), lock);
                            }
                            None => {
                                source.remove(&"lock".into());
                            }
                        }
                    }
                }
                // A container set at runtime has no source yet: the tag the
                // command gave is the one it was set with, `id` and position
                // included (the save checks the id before merging).
                None if lock.is_some() => extras.block_entity_sources.push((key, tag.clone())),
                None => {}
            }
        });
    }
    /// The fields Horizon keeps beside a block entity's model, for a journal: the tag it was
    /// loaded or set with (its `lock`) and its pending loot metadata.
    pub fn block_entity_extras(&self, key: (i32, i32, i32)) -> BlockEntityExtras {
        let saved = self
            .chunks
            .get(&(key.0 >> 4, key.2 >> 4))
            .and_then(|chunk| chunk.saved.as_ref());
        BlockEntityExtras {
            source: saved.and_then(|s| {
                s.block_entity_sources
                    .iter()
                    .find(|(p, _)| *p == key)
                    .map(|(_, tag)| tag.clone())
            }),
            pending_loot: saved.and_then(|s| s.pending_loot.get(&key).cloned()),
        }
    }
    /// Puts back what [`World::block_entity_extras`] kept.
    pub fn set_block_entity_extras(&mut self, key: (i32, i32, i32), kept: BlockEntityExtras) {
        self.with_chunk_extras(key, |extras| {
            extras.block_entity_sources.retain(|(p, _)| *p != key);
            extras
                .block_entity_sources
                .extend(kept.source.map(|tag| (key, tag)));
            extras.pending_loot.remove(&key);
            extras
                .pending_loot
                .extend(kept.pending_loot.map(|loot| (key, loot)));
        });
    }
    /// `LevelChunk.removeBlockEntity`: the tag a block entity was loaded or set
    /// with goes with it, so a container placed there later starts unlocked.
    pub fn forget_block_entity_source(&mut self, key: (i32, i32, i32)) {
        self.with_chunk_extras(key, |extras| {
            extras.block_entity_sources.retain(|(p, _)| *p != key);
            extras.pending_loot.remove(&key);
        });
    }
    /// Run `f` on the keep-as-loaded data of the chunk holding `key`, put back
    /// for the next save (`block_entity_sources` is where Horizon restores the
    /// block entity fields its model does not hold).
    pub fn with_chunk_extras(
        &mut self,
        key: (i32, i32, i32),
        f: impl FnOnce(&mut crate::chunk::ChunkExtras),
    ) {
        let chunk_key = (key.0 >> 4, key.2 >> 4);
        let Some(chunk) = self.chunks.get_mut(&chunk_key) else {
            return;
        };
        let chunk = Arc::make_mut(chunk);
        let mut extras = chunk
            .saved
            .take()
            .map(Arc::unwrap_or_clone)
            .unwrap_or_default();
        f(&mut extras);
        let limit = extras
            .block_entity_sources
            .iter()
            .find(|(p, _)| *p == key)
            .and_then(|(_, tag)| tag.get("horizon:stack_limit"))
            .and_then(|tag| match tag {
                horizon_core::nbt::Tag::Int(n) if (1..=99).contains(n) => Some(*n as u32),
                _ => None,
            });
        match limit {
            Some(limit) => {
                chunk.container_stack_limits.insert(key, limit);
            }
            None => {
                chunk.container_stack_limits.remove(&key);
            }
        }
        chunk.saved = Some(Arc::new(extras));
        self.modified.insert(chunk_key);
    }
}

/// The game rules and difficulty a player's health reads (`Player.isInvulnerableTo`,
/// natural regeneration), kept with the world they belong to.
#[derive(Clone, Copy, Debug)]
pub struct HealthRules {
    pub difficulty: horizon_world::food::Difficulty,
    pub natural_regeneration: bool,
    /// The `drowning_damage`, `fall_damage`, `fire_damage` and
    /// `freeze_damage` rules `Player.isInvulnerableTo` reads.
    pub drowning_damage: bool,
    pub fall_damage: bool,
    pub fire_damage: bool,
    pub freeze_damage: bool,
}
impl Default for HealthRules {
    fn default() -> Self {
        Self {
            difficulty: horizon_world::food::Difficulty::Normal,
            natural_regeneration: true,
            drowning_damage: true,
            fall_damage: true,
            fire_damage: true,
            freeze_damage: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DirtySections, Weather, World};
    use crate::chunk::{Chunk, MIN_Y, SECTIONS};
    use horizon_world::registry::BlockStateId;
    use std::sync::Arc;

    #[test]
    fn block_entity_neighbor_far_read_keeps_chunk_unloaded() {
        let mut world = World::new(BlockStateId(0), 0);
        world.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let count = world.chunks.len();
        assert!(world.loaded(15, 0));
        assert!(!world.loaded(16, 0));
        assert_eq!(world.block(16, 64, 0), BlockStateId(0));
        assert_eq!(world.chunks.len(), count);
        assert!(!world.loaded(16, 0));
    }

    #[test]
    fn container_limit_index_tracks_load_rollback_removal_and_replacement() {
        use crate::chunk::ChunkExtras;
        use horizon_core::nbt::{Compound, Tag};
        let key = (3, 70, 4);
        let mut extras = ChunkExtras::default();
        for (pos, n) in [(key, 10), ((4, 70, 4), 100), ((5, 70, 4), 0)] {
            let mut tag = Compound::new();
            tag.insert("horizon:stack_limit".into(), Tag::Int(n));
            extras.block_entity_sources.push((pos, Tag::Compound(tag)));
        }
        let mut chunk = Chunk::empty(0, 0);
        chunk.saved = Some(Arc::new(extras));
        let mut world = World::new(BlockStateId(0), 0);
        world.chunks.insert((0, 0), Arc::new(chunk));
        assert_eq!(world.container_stack_limit(key), Some(10));
        assert_eq!(world.container_stack_limit((4, 70, 4)), None);
        assert_eq!(world.container_stack_limit((5, 70, 4)), None);
        let kept = world.block_entity_extras(key);
        world.set_container_stack_limit(key, 20);
        assert_eq!(world.container_stack_limit(key), Some(20));
        world.set_block_entity_extras(key, kept);
        assert_eq!(world.container_stack_limit(key), Some(10));
        world.forget_block_entity_source(key);
        assert_eq!(world.container_stack_limit(key), None);
        world.set_container_stack_limit(key, 30);
        world.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        assert_eq!(world.container_stack_limit(key), None);
    }

    /// `/weather thunder` then `/weather clear` in a frozen world: the levels sit at their
    /// targets at every partial tick, where a tick of fading would have moved them 0.01.
    #[test]
    fn settled_weather_shows_its_targets_at_any_partial_tick() {
        let mut weather = Weather {
            raining: true,
            thundering: true,
            rain_level: (0.25, 0.26),
            ..Weather::default()
        };
        weather.settle_levels();
        for partial in [0.0, 0.37, 1.0] {
            assert_eq!(weather.levels(partial), (1.0, 1.0), "{partial}");
        }
        weather.raining = false;
        weather.thundering = false;
        weather.settle_levels();
        assert_eq!(weather.levels(0.5), (0.0, 0.0));
    }

    /// Every block write marks the same section set the 27-cell loop did, but
    /// only the distinct keys: one in the interior, a neighbour section on a
    /// section face, a neighbour column on a chunk border and the diagonal at
    /// a corner (eight keys at a corner on a section face).
    #[test]
    fn writes_mark_exactly_the_reachable_sections() {
        let mut world = World::new(BlockStateId(0), 0);
        world.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        for (x, y, z) in [
            (3, 70, 3),
            (0, 70, 3),
            (15, 70, 3),
            (3, 70, 0),
            (0, 70, 0),
            (15, 70, 15),
            (0, MIN_Y, 0),
            (0, 80, 0),
            (15, MIN_Y + SECTIONS as i32 * 16 - 1, 15),
        ] {
            world.dirty_sections.clear();
            world.set(x, y, z, BlockStateId(1)).unwrap();
            let mut expected = DirtySections::default();
            for dz in -1..=1 {
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let (bx, by, bz) = (x + dx, y + dy, z + dz);
                        let s = (by - MIN_Y) >> 4;
                        if (0..SECTIONS as i32).contains(&s) {
                            expected.insert((bx >> 4, s, bz >> 4));
                        }
                    }
                }
            }
            let mut got: Vec<_> = world.dirty_sections.columns().collect();
            let mut want: Vec<_> = expected.columns().collect();
            got.sort_unstable_by_key(|&(column, _)| column);
            want.sort_unstable_by_key(|&(column, _)| column);
            assert_eq!(got, want, "write at {x} {y} {z}");
            if (x & 15, z & 15) == (3, 3) {
                assert_eq!(world.dirty_sections.len(), 1);
            }
            if (x & 15, z & 15, y & 15) == (0, 0, 0) && y > MIN_Y {
                // Four columns, each with the section below and this one.
                assert_eq!(world.dirty_sections.len(), 8);
            }
        }
    }

    /// A box misses a state only where no section it reaches lists it, and
    /// unloaded columns or heights past the world count as their air.
    #[test]
    fn may_hold_reads_the_palettes_of_the_reached_sections() {
        let mut world = World::new(BlockStateId(0), 0);
        let mut chunk = Chunk::empty(0, 0);
        chunk.set(3, 70, 3, 9);
        world.chunks.insert((0, 0), Arc::new(chunk));
        let nine = |s: BlockStateId| s.0 == 9;
        // The section of y 64..80 holds 9, the one below does not.
        assert!(world.may_hold([0, 64, 0], [1, 65, 1], nine));
        assert!(world.may_hold([10, 79, 10], [12, 80, 12], nine));
        assert!(!world.may_hold([0, 50, 0], [15, 63, 15], nine));
        // A box over the unloaded column next door sees only air.
        assert!(!world.may_hold([16, 70, 0], [17, 71, 1], nine));
        assert!(world.may_hold([16, 70, 0], [17, 71, 1], |s| s.0 == 0));
        // Past the top, and the palette of air below it.
        assert!(!world.may_hold([0, 400, 0], [0, 401, 0], nine));
        assert!(world.may_hold([0, 400, 0], [0, 401, 0], |s| s.0 == 0));
    }
}

/// A block entity's fields kept beside its model ([`World::block_entity_extras`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlockEntityExtras {
    pub source: Option<horizon_core::nbt::Tag>,
    pub pending_loot: Option<crate::chunk::PendingContainerLoot>,
}
