//! Registries from the server, changing dimension and the chunk columns kept.

use super::*;

impl Game<'_> {
    pub(super) fn set_registries_body(&mut self, registries: Arc<Dynamic>) {
        let entries: Vec<(&str, serde_json::Value)> = registries
            .registries
            .get("minecraft:enchantment")
            .into_iter()
            .flatten()
            .filter_map(|(name, data)| {
                Some((
                    name.as_str(),
                    horizon_client::scoreboard::tag_to_json(data.as_ref()?),
                ))
            })
            .collect();
        let enchantments = match self
            .stations()
            .enchantments
            .with_entries(entries.iter().map(|(n, v)| (*n, v)))
        {
            Ok(table) => table.map(Arc::new),
            Err(error) => {
                log::warn!("Horizon: refused enchantment registry: {error}");
                return;
            }
        };
        // The server's biomes the game lacks (a world data pack's) join the
        // game's table, from the data it sent: colours, meshing and the
        // chunk format all go through that table. The window's own server
        // thread set the world's table already.
        if self.integrated.is_none() {
            let table = registries
                .registries
                .get(BIOME_REGISTRY)
                .and_then(|list| client_biomes(self.res.biomes(), list))
                .map(Arc::new);
            if table.is_some() || self.world_biomes.is_some() {
                self.world_biomes_base = table.clone();
                self.world_biomes = table;
                let context = self.worker_context();
                self.streaming.reload_resources(&mut self.world, context);
            }
        }
        let biomes: Vec<u16> = registries
            .registries
            .get(BIOME_REGISTRY)
            .map(|list| {
                list.iter()
                    .map(|(name, _)| world_biomes!(self).id(name))
                    .collect()
            })
            .unwrap_or_default();
        self.server_items = self.server_items(&registries);
        if let Some(r) = self.remote.as_mut() {
            r.enchantments = enchantments;
            let bits = ceil_log2(biomes.len());
            let column = r.chunk_format.layout();
            r.chunk_format =
                Arc::new(ChunkFormat::new(&r.dimension, biomes, bits).in_column(column));
            r.registries = registries;
            r.resolve_clocks();
        }
        self.invalidate_tagged_shader_ids();
    }
    /// Switch to a (possibly new) dimension: a fresh client level.
    pub(super) fn enter_remote_dimension_body(
        &mut self,
        spawn: &play::SpawnInfo,
        keep_world: bool,
        reason: horizon_client::level_loading::Reason,
    ) {
        // Every level has the sea level its server says (`CommonPlayerSpawnInfo.seaLevel`): the
        // table may still hold a local world's (a superflat Overworld's).
        if let Ok(key) = horizon_core::LevelKey::parse(&spawn.dimension) {
            horizon_world::level::register_sea_level(key, spawn.sea_level);
        }
        self.track_remote_levels(&spawn.dimension);
        // The replacement player starts with its inventory menu, even if the
        // server switched levels without closing the previous container.
        self.remote_close_local();
        self.set_screen(Screen::Playing);
        let Some(r) = self.remote.as_ref() else {
            return;
        };
        // handleRespawn keeps the ClientLevel only for the same level key:
        // two levels sharing a dimension type are still two levels.
        let same = keep_world && r.heard_level.as_deref() == Some(spawn.dimension.as_str());
        let transition_screen = horizon_client_render::dimension_transition::select(
            mod_api::__private::dimension_key(&spawn.dimension),
            r.heard_level
                .as_deref()
                .map(mod_api::__private::dimension_key),
        );
        if !same {
            self.forget_all_remote_columns();
        }
        let r = self.remote.as_mut().unwrap();
        // A Horizon server's bulk edit sends whole columns with the light
        // from before its relight; any other server's packet light is final.
        let relight_resent = r.quiet_commands;
        let format = r.level_format(spawn, self.res.environment());
        let (dimension, shape) = (format.dimension, format.shape);
        r.chunk_format = format.chunk_format;
        r.level_facts = format.facts;
        r.level_environment = format.environment;
        r.dimension = dimension;
        r.resolve_clocks();
        self.world.day_time = r.clock_time(r.default_clock);
        r.level = spawn.dimension.clone();
        r.level_key = mod_api::__private::dimension_key(&spawn.dimension);
        r.hashed_seed = spawn.seed;
        r.game_mode = spawn.game_type;
        r.flat = spawn.flat;
        r.loaded_sent = false;
        // A replacement LevelLoadingScreen starts its bar from nothing (the
        // first one carries on from the server's start).
        if self.player.is_some() {
            self.loading_progress = 0.0;
        }
        // startWaitingForNewLevel: `LevelLoadTracker.startClientLoad` and the
        // `LevelLoadingScreen`.
        r.level_load.start_client_load(client_millis(), reason);
        r.level_load.set_transition_screen(transition_screen);
        // A replacement LocalPlayer has a fresh ambient mood and sendPosition baseline.
        self.ambience.moodiness = 0.0;
        r.position_sender = Default::default();
        r.predictions.clear();
        self.set_mode_id(spawn.game_type);
        self.dim = Dim::from_id(&spawn.dimension);
        self.audio.set_world(&spawn.dimension);
        // ClientPacketListener.handleLogin / handleRespawn: the server's
        // Player.getLastDeathLocation, for the recovery compass.
        self.level.last_death = spawn
            .last_death
            .as_ref()
            .map(|d| (d.dimension.clone(), d.pos));
        if !same {
            // A new ClientLevel: Minecraft.setLevel stops every sound.
            self.stop_sounds_for_new_level();
            let mut streaming = Streaming::new(
                Arc::new(NoTerrain),
                self.worker_context(),
                self.res.threads(),
            );
            streaming.remote = true;
            streaming.radius = self.streaming.radius;
            self.streaming = streaming;
            let (time, day, weather) = (
                self.world.game_time,
                self.world.day_time,
                self.world.weather,
            );
            self.reset_client_world(spawn.seed);
            // Entity IDs belong to this ClientLevel, not the connection. A
            // same-level respawn keeps it; a replacement invalidates handles.
            self.native_lifecycle_open_started();
            self.native_lifecycle_world_ready();
            self.world.shape = shape;
            self.world.game_time = time;
            self.world.day_time = day;
            self.world.weather = weather;
            self.mobs.clear();
            self.clear_remote_objects();
            self.clear_remote_projectiles();
            self.dropped.clear();
            self.clear_client_particles();
            if let Some(r) = self.remote.as_mut() {
                r.entities.clear();
                r.passenger_vehicles.clear();
                r.lerps.clear();
                r.cart_lerps.clear();
                r.stepped.clear();
                r.head_lerps.clear();
                r.pending_items.clear();
                // The entity ids are the old world's; the new dimension's
                // player entities re-map to their profiles on AddEntity.
                r.level_players.clear();
                r.level_player_names.clear();
                r.level_player_textures.clear();
                // A new ClientLevel: a new chunk storage, centred on 0, 0.
                r.chunk_center = (0, 0);
            }
            self.clear_lod_for_world_entry();
            self.pending_uploads.clear();
            if let Some(renderer) = self.renderer.as_mut() {
                renderer.clear_sections(self.world.layout());
            }
            // The new ClientLevel is told before setLevel drops the old one.
            let old = self
                .remote
                .as_mut()
                .and_then(|r| r.heard_level.replace(spawn.dimension.clone()));
            self.native_client_level_load(&spawn.dimension);
            if let Some(old) = old {
                self.native_client_level_unload(&old);
            }
        }
        self.streaming.relight_resent = relight_resent;
        let has_sky = self
            .remote
            .as_ref()
            .is_none_or(|r| r.dimension.has_skylight);
        self.streaming.sky_light = has_sky;
        self.streaming.nether_shading = self.nether_lighting();
        // The seed in SpawnInfo is already the hashed one the client's BiomeManager holds.
        self.streaming.biome_zoom = horizon_core::biome::BiomeZoom::new(spawn.seed);
        let player = self.player.as_ref();
        self.particles
            .detach_tracking_target(TrackingTarget::LocalPlayer, || {
                player.map(player_tracking_body)
            });
        self.drop_remote_player();
        // handleLogin / handleRespawn: LocalPlayer.setPortalCooldown, so a
        // player who arrives standing in a portal gets no overlay from it.
        self.portals.player.cooldown = spawn.portal_cooldown;
        if let Some(profile) = self.remote.as_ref().map(|remote| remote.profile) {
            self.initialize_entity(Holder::Player(profile), "minecraft:player");
        }
        self.finish_remote_level();
    }

    /// `ClientChunkCache.Storage.inRange`: within the storage around the
    /// view centre, `calculateStorageRange` (the server's radius, at least
    /// two, plus three) chunks each way.
    pub(super) fn remote_chunk_in_range_body(&self, x: i32, z: i32) -> bool {
        if self.replay.playing() {
            return true;
        }
        self.remote
            .as_ref()
            .is_none_or(|r| in_storage_range(r.view_distance, r.chunk_center, x, z))
    }
    pub(super) fn chunk_inside_remote_range_body(
        center: (i32, i32),
        distance: i32,
        x: i32,
        z: i32,
    ) -> bool {
        in_storage_range(distance, center, x, z)
    }
    /// The columns a moved or resized storage no longer holds. Returns the
    /// positions it freed (`forget_remote_columns`).
    pub(super) fn drop_remote_chunks_out_of_range_body(
        &mut self,
        center: (i32, i32),
        distance: i32,
    ) -> Vec<(i32, i32)> {
        let mut gone: horizon_client::fast_hash::FxHashSet<(i32, i32)> = self
            .world
            .chunks
            .keys()
            .copied()
            .filter(|&(x, z)| !Self::chunk_inside_remote_range(center, distance, x, z))
            .collect();
        self.forget_remote_columns(&mut gone, true)
    }
    /// Every column goes with the client level. Returns the positions it
    /// freed (`forget_remote_columns`).
    pub(super) fn forget_all_remote_columns_body(&mut self) -> Vec<(i32, i32)> {
        let mut gone = self.world.chunks.keys().copied().collect();
        self.forget_remote_columns(&mut gone, false)
    }
    /// Forgets the `gone` columns (and empties the set): their chunks (freed on a thread
    /// of their own when there are many), their sections in the renderer and their block
    /// entities, each store once for all of them. When the level stays open,
    /// far terrain keeps their last state (`server_terrain`). Returns the
    /// positions it freed, in `gone`'s order: only those still resident,
    /// so a later forget of the same positions frees nothing.
    pub(super) fn forget_remote_columns_body(
        &mut self,
        gone: &mut horizon_client::fast_hash::FxHashSet<(i32, i32)>,
        keep_terrain: bool,
    ) -> Vec<(i32, i32)> {
        if gone.is_empty() {
            return Vec::new();
        }
        // The client chunk seam: one unload per column still resident, with
        // its data still there, before `take_remote_columns` frees it.
        let freed_at: Vec<(i32, i32)> = gone
            .iter()
            .filter(|&&(x, z)| self.world.chunks.contains_key(&(x, z)))
            .copied()
            .collect();
        for &(x, z) in &freed_at {
            self.client_chunk_unload(x, z, keep_terrain);
        }
        let freed =
            horizon_client::streaming::Streaming::take_remote_columns(&mut self.world, gone);
        self.streaming.forget_columns(&mut self.world, gone);
        if keep_terrain && let Some(lod) = &mut self.lod {
            for chunk in &freed {
                lod.observe(chunk.clone(), self.res.facts());
            }
        }
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.remove_columns(gone.iter().copied());
        }
        horizon_client::chunk::free_columns(freed, gone.len());
        let gone_chunks: Vec<(i32, i32)> = gone.iter().copied().collect();
        self.world.block_entities.unload_chunks(&gone_chunks);
        if let Some(r) = self.remote.as_mut() {
            r.attachments.retain(|holder, _| match *holder {
                Holder::Chunk([x, z]) => !gone.contains(&(x, z)),
                Holder::BlockEntity([x, _, z]) => !gone.contains(&(x >> 4, z >> 4)),
                _ => true,
            });
        }
        gone.clear();
        freed_at
    }
    /// `handleAnimate`: a player wakes up, or an entity gets the critical
    /// hit (`CRIT`) or magic critical hit (`ENCHANTED_HIT`) emitter.
    pub(super) fn remote_animate_body(&mut self, id: i32, action: u8) {
        use horizon_client::particles::{ParticleOptions, ParticleType};
        let Some((target, body)) = self.remote_tracking_body(id) else {
            return;
        };
        let particle = match action {
            // Player.stopSleepInBed.
            0 => {
                if target == TrackingTarget::LocalPlayer {
                    self.sleeping = None;
                    self.sleep_timer = 100;
                }
                return;
            }
            1 => ParticleType::Crit,
            2 => ParticleType::EnchantedHit,
            _ => return,
        };
        self.particles
            .tracking_emitter(ParticleOptions::simple(particle), target, body);
    }
    /// `handleServerData`: the server list entry of the server joined keeps
    /// the icon the server sends (`ServerList.saveSingleServer`). Single
    /// player has no entry.
    pub(super) fn remote_server_data_body(&mut self, icon: Option<&[u8]>) {
        let (Some(r), Some(png)) = (self.remote.as_ref(), icon) else {
            return;
        };
        if self.integrated.is_some() {
            return;
        }
        if let Ok(dir) = std::env::current_dir() {
            horizon_client::multiplayer::save_server_icon(
                &dir.join("saves"),
                &r.session.address,
                png,
            );
        }
    }
}
