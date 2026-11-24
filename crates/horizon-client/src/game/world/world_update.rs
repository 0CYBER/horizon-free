//! The frame's world update (`update_world`): what arrived from the server, the chunks that
//! streamed in and the meshes handed to the renderer, the columns that unload (their entities
//! and block entities saved out first); and far terrain (`prepare_lod`, `clear_lod`).
use super::*;

impl Game<'_> {
    /// A saved stack as this world takes it in (`ItemStack.CODEC`, `applyPatch`):
    /// a value equal to its item's prototype in this world's item table leaves the
    /// patch.
    pub(in crate::game) fn clean_stack(&self, stack: &mut horizon_client::items::ItemStack) {
        horizon_client::item_data::clean_stack(stack, &self.items().component_defaults);
    }
    /// `ItemStack.CODEC` (`applyPatch`) on the stacks of the block entities in the
    /// chunks loaded this frame: a value equal to its item's prototype in this
    /// world's item table leaves the patch (`BlockEntity::clean_stacks`).
    pub(in crate::game) fn clean_loaded_stacks(&mut self) {
        if self.remote.is_some() {
            return;
        }
        let prototypes = self.items().component_defaults.clone();
        for &(x, z, _) in &self.world.loaded_chunks {
            let Some(saved) = self
                .world
                .chunks
                .get(&(x, z))
                .and_then(|chunk| chunk.saved.as_deref())
            else {
                continue;
            };
            for (at, _) in &saved.block_entity_sources {
                // `get_mut` wakes a sleeping entity: only one with a stack to clean.
                let stale = self
                    .world
                    .block_entities
                    .get(at)
                    .is_some_and(|entity| entity.clone().clean_stacks(&prototypes));
                if stale && let Some(entity) = self.world.block_entities.get_mut(at) {
                    entity.clean_stacks(&prototypes);
                }
            }
        }
    }
    pub(super) fn clear_lod(&mut self) {
        // The samplers waiting on the server stop first: dropping the far
        // terrain joins them.
        if let Some(remote) = &self.remote {
            remote.session.cancel_far_terrain();
        }
        self.lod = None;
        self.lod_source_attempted = false;
        self.lod_observe.clear();
        self.far_beacons.clear();
        self.lod_covered_at = None;
        if let Some(renderer) = &mut self.renderer {
            renderer.clear_lods();
        }
    }
    /// Clears far terrain for a world or dimension change, arming the next
    /// far terrain's world-entry first-load hold.
    pub(super) fn clear_lod_for_world_entry(&mut self) {
        self.clear_lod();
        self.lod_first_load = true;
    }
    pub(super) fn prepare_lod(&mut self) {
        let in_world = self.renderer.is_some() && self.player.is_some() && self.title.is_none();
        let enabled = in_world && self.far_terrain_choice().0 > 0;
        if !enabled {
            if self.lod.is_some() || self.lod_source_attempted {
                self.clear_lod();
            }
            if in_world {
                // Far terrain is off in an already-entered world: toggling it
                // on later must not start a world-entry first-load hold.
                self.lod_first_load = false;
            }
            return;
        }
        // Another level: its own kept chunks. (Another world at the same
        // address, a proxy's next server, comes through a new login; a seed
        // changed at a respawn of the same level is a plugin hiding it.)
        if self
            .lod_level
            .as_ref()
            .is_some_and(|level| self.remote.as_ref().is_some_and(|r| r.level != *level))
        {
            self.clear_lod_for_world_entry();
        }
        if self.lod_source_attempted || self.export_server_calls_blocked() {
            return;
        }
        self.lod_source_attempted = true;
        let arm_first_load = std::mem::take(&mut self.lod_first_load);
        self.lod_level = None;
        let dim = self.dim;
        let source = match (&self.integrated, self.server_far_terrain(), &self.remote) {
            (Some(server), ..) => server.call(move |game| game.lod_source_in(dim)).flatten(),
            (None, Some(offer), Some(remote)) => {
                let states = self.res.facts().states.len() as u32;
                let biomes = self.res.biomes().index.clone();
                let source: Arc<dyn horizon_client::streaming::LodSource> =
                    remote.session.far_terrain(offer, biomes, states);
                Some(source)
            }
            // A friend's world (a new code at every share) or a splitscreen
            // guest's (the player's own world, its other guests writing the
            // same folder): nothing to keep.
            (None, None, Some(remote))
                if self.options.split_guest
                    || remote
                        .session
                        .address
                        .starts_with(horizon_client::net::FRIEND) =>
            {
                None
            }
            // No offer from a server: draw what it sent, on this visit and before.
            (None, None, Some(remote)) => std::env::current_dir().ok().map(|home| {
                use horizon_client::server_terrain as terrain;
                let address = &remote.session.address;
                let root = home.join("server-terrain");
                let dir = terrain::level_dir(&root, address, remote.hashed_seed, &remote.level);
                self.lod_level = Some(remote.level.clone());
                // Past them, the player's Bobby cache of the level, only read.
                let bobby = terrain::bobby_level_dir(
                    &terrain::minecraft_dir().join(".bobby"),
                    address,
                    remote.hashed_seed,
                    &remote.level,
                )
                .map(|bobby| {
                    log::info!(
                        "Horizon: far terrain also reads Bobby's chunks of {}",
                        remote.level
                    );
                    let registry = self.pack.assets.registry;
                    let biomes = self.res.biomes().clone();
                    let dimension = &remote.dimension;
                    let layout = horizon_client::chunk::ColumnLayout::of_level(
                        dimension.min_y,
                        dimension.height,
                    );
                    let sky_light = dimension.has_skylight;
                    let decode: terrain::DecodeChunk =
                        Box::new(move |root: &horizon_core::nbt::NamedTag| {
                            horizon_client::save::decode_lit(
                                root, registry, &biomes, layout, sky_light,
                            )
                        });
                    terrain::Bobby::new(bobby, decode, self.res.facts())
                });
                terrain::ServerTerrain::with_bobby(dir, bobby) as _
            }),
            _ => Some(self.streaming.lod_source()),
        };
        if let Some(source) = source {
            let mut lod = horizon_client::lod::Lod::new(source, self.worker_context());
            if arm_first_load {
                lod.arm_first_load();
            }
            // Far tiles are never in the ray scene: the arenas' looks alone.
            lod.set_looks(self.streaming.looks, self.streaming.nether_shading);
            lod.set_shader_pack(self.streaming.shader_pack);
            self.lod = Some(lod);
            let resident: Vec<_> = self.world.chunks.keys().copied().collect();
            self.lod_observe_later(resident);
        }
    }
    pub(in crate::game) fn update_world(&mut self) {
        if let Err(e) = horizon_client::spans::time("poll_remote", || self.poll_remote()) {
            log::warn!("Horizon: {e}");
        }
        if self.export.is_none()
            && let Err(e) = self.advance_studio_replay()
        {
            log::warn!("Horizon Studio replay: {e}");
        }
        // The renderer laid translucent quads out again (a shader pack's flat terrain came
        // or went), or its pages couldn't take the quads' looks it turned on, and dropped every
        // section and far tile: meshed again, as for a new one, with the looks it takes now
        // (`shader_pack_inputs` would otherwise mesh them all again for the looks).
        if self
            .renderer
            .as_mut()
            .is_some_and(Renderer::take_dropped_sections)
        {
            let r = self.renderer.as_ref();
            self.streaming.looks = r.is_some_and(|r| r.terrain_looks());
            self.streaming.ray_looks = r.is_some_and(|r| r.ray_looks());
            self.clear_lod();
            self.all_sections_changed();
        }
        horizon_client::spans::time("prepare_lod", || self.prepare_lod());
        // The columns the single-player server changed in bulk without sending them, which far terrain still drew
        // as this window last saw them: summarized again (taken every tick, far terrain on or off, so none waits).
        if self.remote.is_some() {
            let changed = horizon_client::lan::handoff::take_changed();
            if let Some(lod) = &mut self.lod {
                for chunk in changed {
                    let column = (chunk.x, chunk.z);
                    if self.world.chunks.contains_key(&column) {
                        continue;
                    }
                    lod.observe(chunk, self.res.facts());
                }
            }
        }
        self.observe_light_edits();
        self.ask_edited_light_emission();
        if let Some(p) = &self.player {
            let own = super::player_chunk(p.position);
            self.streaming.center = self
                .remote
                .as_ref()
                .map_or(own, |r| view_center(own, r.chunk_center));
        }
        horizon_client::spans::time("tick_pregen", || self.tick_pregen());
        // A headless game has nothing to draw sections with: like a server's, they aren't
        // meshed while it has no renderer.
        if self.options.headless {
            self.streaming.mesh = self.renderer.is_some();
        }
        let areas = self.streaming.extra.len();
        let waiting = std::mem::take(&mut self.spawn_wait);
        self.streaming
            .extra
            .extend(waiting.into_iter().map(|chunk| (chunk, 0)));
        self.streaming.upload_backlog = self.pending_uploads.len();
        let plan = horizon_client::spans::time("streaming_plan", || {
            let mut step = self.streaming.begin_plan_update(&mut self.world);
            loop {
                match step {
                    horizon_client::streaming::PlanStep::Ready(plan) => break plan,
                    horizon_client::streaming::PlanStep::Save { chunk, resume } => {
                        self.native_late_full_chunk_snapshot(&chunk);
                        self.streaming.save_finished(chunk);
                        step = self.streaming.resume_plan_update(&mut self.world, resume);
                    }
                }
            }
        });
        let (commit, protos) = self.streaming.prepare_commit(&mut self.world, plan);
        self.native_proto_snapshots(&protos);
        self.streaming.submit_protos(protos);
        let (uploads, unloaded) = horizon_client::spans::time("streaming", || {
            self.streaming.complete_commit(&mut self.world, commit)
        });
        self.streaming.extra.truncate(areas);
        // Its saved stacks are built against this world's item prototypes as the
        // chunk arrives: the worker that decoded it had no item table.
        self.clean_loaded_stacks();
        // A lectern read from a save resolves its book pages as its chunk
        // arrives (`LecternBlockEntity.loadAdditional`): the chunk was decoded
        // on a worker, which has no game to read a selector or a score with.
        horizon_client::spans::time("resolve_loaded_lecterns", || self.resolve_loaded_lecterns());
        // The viewed levels' meshing, queued after the player's level's (nothing while no view
        // names one).
        if self.streaming.mesh
            && let Some(remote) = &mut self.remote
        {
            let streaming = &self.streaming;
            remote
                .viewed
                .mesh_frame(|| streaming.viewed_mesh_source(), std::time::Instant::now());
        }
        let dirty: Vec<_> = unloaded
            .iter()
            .filter(|c| self.world.modified.remove(&(c.x, c.z)))
            .cloned()
            .collect();
        let modified = horizon_client::spans::time("with_entities", || self.with_entities(&dirty));
        self.native_full_chunk_snapshots(&modified);
        if !modified.is_empty()
            && let Err(e) = horizon_client::spans::time("save_chunks", || {
                self.save
                    .save_later(&modified, self.pack.assets.registry, world_biomes!(self))
            })
        {
            log::warn!("Horizon: failed to save unloaded chunks: {e}");
        }
        horizon_client::spans::time("spawn_fresh_chunks", || self.spawn_fresh_chunks());
        self.ask_loaded_light_emission();
        // Vanilla saves the chunk before its unload hook and drops its block entities afterwards.
        horizon_client::spans::time("mods_chunks_changed", || {
            self.native_chunks_loaded();
            self.native_chunks_unloaded(&unloaded);
        });
        for chunk in &unloaded {
            self.world.modified.unloaded(&(chunk.x, chunk.z));
        }
        self.lod_observe_later(uploads.iter().map(|(key, _)| (key.0, key.2)));
        if let Some(lod) = &mut self.lod {
            for chunk in &unloaded {
                self.lod_observe.remove(&(chunk.x, chunk.z));
            }
            // Unloaded columns first (their summaries are all that remains),
            // then resident ones that left the full-detail area (their
            // sections are held until their summary is drawn), then a few
            // other resident ones (an edit copies a chunk the worker still
            // holds). The worker summarizes them.
            horizon_client::spans::time("lod_observe", || {
                for chunk in &unloaded {
                    lod.observe(chunk.clone(), self.res.facts());
                }
                let mut remaining =
                    horizon_client::lod::OBSERVE_AHEAD.saturating_sub(lod.observing());
                while remaining > 0 {
                    let asks = lod.take_summary_asks(remaining);
                    if asks.is_empty() {
                        break;
                    }
                    for key in asks {
                        if let Some(chunk) = self.world.chunks.get(&key) {
                            self.lod_observe.remove(&key);
                            lod.observe(chunk.clone(), self.res.facts());
                            remaining -= 1;
                        }
                    }
                }
                while lod.observing() < horizon_client::lod::OBSERVE_AHEAD
                    && let Some(&key) = self.lod_observe.iter().next()
                {
                    self.lod_observe.remove(&key);
                    if let Some(chunk) = self.world.chunks.get(&key) {
                        lod.observe(chunk.clone(), self.res.facts());
                    }
                }
            });
        }
        horizon_client::spans::time("unload_entities", || self.unload_entities(&unloaded));
        if !unloaded.is_empty() {
            let _span = horizon_client::spans::guard("unload_block_entities");
            let gone_chunks: Vec<(i32, i32)> = unloaded.iter().map(|c| (c.x, c.z)).collect();
            self.world.block_entities.unload_chunks(&gone_chunks);
            if !self.world.block_names.is_empty() {
                let gone: horizon_client::fast_hash::FxHashSet<(i32, i32)> =
                    gone_chunks.iter().copied().collect();
                self.world
                    .block_names
                    .retain(|p, _| !gone.contains(&(p.0 >> 4, p.2 >> 4)));
            }
        }
        // LevelChunk.registerTickContainerInLevel: a chunk that became
        // resident has its tick containers before a command can schedule into
        // it, even in a frozen level that runs no world tick. The unloaded
        // chunks above were saved with their ticks first.
        if self.remote.is_none() {
            horizon_client::spans::time("sync_tick_chunks", || self.sync_tick_chunks());
        }
        let (far_distance, far_reduced) =
            horizon_client::spans::time("far_terrain_choice", || self.far_terrain_choice());
        // Terrain still on its way from the server (single player's integrated one
        // too): the view's outermost columns not all in, or its events not all handled.
        let radius = match &self.remote {
            // As far as the server fills it: its view distance and one.
            Some(r) => self.streaming.radius.min(r.view_distance + 1),
            None => self.streaming.radius,
        };
        let view = (self.streaming.center, radius);
        let has = |column: (i32, i32)| self.world.chunks.contains_key(&column);
        let now = Instant::now();
        let view_arriving = {
            let _span = horizon_client::spans::guard("view_arrival");
            self.view_arrival.arriving(view, now, has)
        };
        self.view_arriving = view_arriving;
        let arriving = view_arriving || self.remote_backlog_delays_frame();
        // Which far tiles a zoom magnifies (`lod::ZoomAim`), read before
        // `self.renderer.as_mut()` below borrows self mutably for the rest of
        // this function. Cheap: trigonometry on one angle, or None when the
        // view is not zoomed.
        let lod_aim = self
            .player
            .as_ref()
            .map(|p| [p.position.x, p.position.y, p.position.z])
            .and_then(|camera| self.lod_aim(camera));
        if let Some(renderer) = self.renderer.as_mut() {
            let g = &self.settings.horizon;
            let budgeted = g.chunk_updates > 0 || g.upload_budget_kib > 0;
            // The meshes, once copied, are freed by a worker (`Streaming::free`).
            if !budgeted && self.pending_uploads.is_empty() && uploads.len() <= AUTO_UPLOADS_AT_ONCE
            {
                horizon_client::spans::time("upload", || {
                    for (key, mesh) in &uploads {
                        renderer.upload(*key, mesh);
                    }
                });
                self.streaming.free(uploads);
            } else {
                // Horizon's chunk-update and upload budgets: the nearest
                // sections go first, the rest wait for later frames; a newer
                // mesh of a waiting section replaces it, and sections of
                // chunks no longer loaded are dropped.
                let pending = &mut self.pending_uploads;
                let chunks = &self.world.chunks;
                let mut spent = Vec::new();
                let centre = self.streaming.center;
                // Section keys count from the world's bottom (`origin`'s min_y + y * 16).
                let bottom = f64::from(self.world.bottom_y());
                let cam_y = self
                    .player
                    .as_ref()
                    .map_or(4, |p| ((p.position.y - bottom) / 16.0).floor() as i32);
                pending.push_batch_for_chunks(uploads, chunks, (centre, cam_y), &mut spent);
                let (limit, budget) = if budgeted {
                    (
                        g.chunk_updates as usize,
                        u64::from(g.upload_budget_kib) * 1024,
                    )
                } else {
                    (0, 0)
                };
                let (mut count, mut bytes) = (0, 0u64);
                // Without the player's budgets, uploads stop once they took this
                // frame's share (`UploadPace::budget`: a quarter of the recent frame
                // time, more with a backlog, at most 2 ms; at least one goes): a burst
                // of finished meshes spreads over frames, nearest first.
                let auto = (!budgeted).then(|| self.upload_pace.budget(pending.len(), arriving));
                let started = Instant::now();
                let upload_span = horizon_client::spans::guard("upload_queue");
                while !pending.is_empty() {
                    if (limit > 0 && count >= limit)
                        || auto.is_some_and(|time| count > 0 && started.elapsed() >= time)
                    {
                        break;
                    }
                    let Some((_, mesh)) = pending.peek_nearest() else {
                        break;
                    };
                    let size = horizon_client::streaming::upload_mesh_bytes(mesh);
                    if budget > 0 && bytes > 0 && bytes + size > budget {
                        break;
                    }
                    let (key, mesh) = pending.pop_nearest().expect("checked above");
                    renderer.upload(key, &mesh);
                    spent.push((key, mesh));
                    count += 1;
                    bytes += size;
                }
                drop(upload_span);
                self.streaming.free(spent);
            }
            renderer.remove_columns(unloaded.iter().map(|c| (c.x, c.z)));
            // Sections whose mesh the arena refused: meshed again, as an edit's section
            // is, once their retry comes due. The mesh is dropped with the uploads above,
            // so this is the only way back for them.
            renderer.queue_upload_retries(&mut self.world.dirty_sections, now);
            // Meshes in the works are the view's terrain while it is new, then edits'.
            let meshing = self.streaming.busy() || !self.pending_uploads.is_empty();
            let sampling = self.lod.as_ref().is_some_and(|lod| lod.sampling());
            renderer.set_terrain_loading(
                arriving || (meshing && self.view_arrival.recent(now)) || sampling,
            );
            if let (Some(lod), Some(player)) = (&mut self.lod, &self.player) {
                let lod_distance = far_distance.min(horizon_client::lod::MAX_CHUNKS);
                lod.set_near_loading(self.streaming.terrain_loading());
                lod.set_reduced_detail(far_reduced);
                // The covered columns change only with the complete ones and
                // the render area: rebuilt then, not every frame.
                let at = (
                    renderer.lod_detail_revision(),
                    self.streaming.center,
                    self.streaming.radius,
                );
                let covered = (self.lod_covered_at != Some(at)).then(|| {
                    self.lod_covered_at = Some(at);
                    horizon_client::spans::time("lod_covered_set", || {
                        renderer
                            .lod_detail_columns()
                            .filter(|&(x, z)| {
                                horizon_client::streaming::in_view_distance(at.1, at.2, x, z)
                            })
                            .collect()
                    })
                });
                // Far terrain's finest tiles stand where full-detail terrain
                // ends: on a server, where it stops sending chunks, often well
                // inside the render distance. Thousands of chunk-sized tiles
                // between the two filled in long after the coarser ones.
                let detail = self.remote.as_ref().map_or(self.streaming.radius, |r| {
                    self.streaming.radius.min(r.sent_radius())
                });
                let camera = [player.position.x, player.position.y, player.position.z];
                let meshes = horizon_client::spans::time("lod_update", || {
                    lod.update(camera, detail, lod_distance, covered, lod_aim)
                });
                renderer.queue_lods(meshes, lod.removed(), lod.take_cutout());
                // A finished job can hold hundreds of tiles: spread them
                // over frames rather than stall one.
                horizon_client::spans::time("lod_upload", || {
                    renderer.upload_lods(std::time::Duration::from_millis(2))
                });
            }
        }
    }
    /// Run `f` inside a local player block-edit scope so writes record `World::immediate_sections`.
    pub(in crate::game) fn with_player_block_edit<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> T,
    ) -> T {
        self.world.begin_player_edit();
        let out = f(self);
        self.world.end_player_edit();
        out
    }

    /// Rebuild sections dirtied by player block edits and upload them before the current frame
    /// renders, matching vanilla's `SectionRenderDispatcher.rebuildSectionSync`. With
    /// `prioritizeChunkUpdates == 2` nearby dirty sections follow, nearest first, within
    /// `NEARBY_MESH_BUDGET` (paused for `NEARBY_COOLDOWN_FRAMES` after an overrun); the rest
    /// stay with the workers. Under Threaded (`0`, default), no main-thread meshing occurs.
    pub(in crate::game) fn flush_immediate_meshes(&mut self) {
        use horizon_client::streaming::immediate_mesh::{
            IMMEDIATE_MESH_LIMIT, NEARBY_IMMEDIATE_DIST_SQ,
        };
        let mode = self.settings.prioritize_chunk_updates;
        if mode == 0 || self.title.is_some() || self.renderer.is_none() {
            self.world.immediate_sections.clear();
            return;
        }
        let nearby = match self.player.as_ref() {
            Some(p) if mode >= 2 && self.streaming.nearby_meshing_due() => {
                self.world.nearby_dirty_sections(
                    [
                        p.position.x.floor() as i32,
                        p.position.y.floor() as i32,
                        p.position.z.floor() as i32,
                    ],
                    NEARBY_IMMEDIATE_DIST_SQ,
                    IMMEDIATE_MESH_LIMIT,
                )
            }
            _ => Vec::new(),
        };
        if self.world.immediate_sections.is_empty() && nearby.is_empty() {
            return;
        }
        self.observe_light_edits();
        self.ask_edited_light_emission();
        let uploads = self
            .streaming
            .mesh_immediate(&mut self.world, mode, &nearby);
        if uploads.is_empty() {
            return;
        }
        if !self.pending_uploads.is_empty() {
            let mut spent = Vec::new();
            self.pending_uploads
                .remove_keys(uploads.iter().map(|(k, _)| *k), &mut spent);
            self.streaming.free(spent);
        }
        self.lod_observe_later(uploads.iter().map(|(key, _)| (key.0, key.2)));
        if let Some(renderer) = self.renderer.as_mut() {
            for (key, mesh) in &uploads {
                renderer.upload(*key, mesh);
            }
        }
        self.streaming.free(uploads);
    }
}

/// The chunk the window meshes and draws around: the player's own, unless the server centred
/// their chunk cache well away from them (a mod's view centre, horizon-rust/horizon#19219), where
/// the chunks it sends are. Within two chunks it is the player's: the cache centre lags a move
/// across a chunk border by a tick, and vanilla's view area follows the camera there anyway.
pub(in crate::game) fn view_center(own: (i32, i32), cache: (i32, i32)) -> (i32, i32) {
    // A server may send any `i32` centre: measured in `i64`, which no pair of them overflows.
    let away = (i64::from(cache.0) - i64::from(own.0))
        .abs()
        .max((i64::from(cache.1) - i64::from(own.1)).abs());
    if away > 2 { cache } else { own }
}

#[cfg(test)]
mod view_center_tests {
    use super::view_center;

    #[test]
    fn follows_the_cache_centre_only_when_it_moved_away() {
        assert_eq!(view_center((0, 0), (0, 0)), (0, 0));
        // A border crossing the server has not answered yet.
        assert_eq!(view_center((5, 5), (4, 5)), (5, 5));
        assert_eq!(view_center((5, 5), (7, 3)), (5, 5));
        // A mod's view centre far from the player.
        assert_eq!(view_center((5, 5), (8, 5)), (8, 5));
        assert_eq!(view_center((0, 0), (-40, 120)), (-40, 120));
        // Any centre a server sends, however far, measures without overflowing.
        assert_eq!(view_center((0, 0), (i32::MIN, 0)), (i32::MIN, 0));
        assert_eq!(
            view_center((-1, 0), (i32::MAX, i32::MIN)),
            (i32::MAX, i32::MIN)
        );
    }
}
