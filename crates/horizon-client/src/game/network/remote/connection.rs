//! Sounds by wire id, connecting to a server, a replay's session and leaving.

use super::*;
// The point generator reads a signature's types through named imports only, not the glob.
use horizon_client::text::Text;

impl Game<'_> {
    /// A `Holder<SoundEvent>` off the wire: an inline event's location, or
    /// the name of a registered one.
    pub(super) fn remote_sound_name_body(
        &self,
        sound: &(i32, Option<(String, Option<f32>)>),
    ) -> Option<String> {
        match &sound.1 {
            Some((location, _)) => Some(location.clone()),
            None => self.remote.as_ref().and_then(|r| {
                r.builtin
                    .get("minecraft:sound_event")?
                    .name(sound.0)
                    .map(str::to_owned)
            }),
        }
    }
    pub(super) fn remote_sound_entity_state_body(
        &mut self,
        id: i32,
    ) -> Option<horizon_client::sound_engine::SoundEntityState> {
        let r = self.remote.as_ref()?;
        let player = self.player.as_ref().map(|p| {
            (
                r.entity_id,
                [p.position.x, p.position.y, p.position.z],
                r.local_entity_silent,
            )
        });
        // The local player is not in mobs, so avoid a guaranteed index miss.
        if player.is_some_and(|p| p.0 == id) {
            return entity_sound_state(id, player, None, &r.entities, None);
        }
        let mob = self.mob_index.get(&self.mobs, id as u32).map(|m| {
            horizon_client::sound_engine::SoundEntityState {
                position: [m.position.x, m.position.y, m.position.z],
                silent: m.silent,
                removed: m.removed,
            }
        });
        let dragon = self
            .dragon
            .as_ref()
            .map(|d| (d.id as i32, [d.position.x, d.position.y, d.position.z]));
        let mut state = entity_sound_state(id, player, mob, &r.entities, dragon)?;
        // Mob states already use the live position. In particular,
        // level_entity would find the dragon's stale Mob before its Dragon.
        if mob.is_some() || dragon.is_some_and(|d| d.0 == id) {
            return Some(state);
        }
        // Vehicles, projectiles and other simulated objects may have moved
        // since the last tracking packet.
        if let Some(position) = self
            .level_entity(id)
            .and_then(|target| self.target_position(target))
        {
            state.position = position;
        }
        Some(state)
    }
    /// The hurt sound the hurt player's own client plays from
    /// `ClientboundDamageEventPacket` (`LivingEntity.handleDamageEvent`):
    /// `Player.getHurtSound` is the damage type's effects, with the codec's
    /// `hurt` default when the type is unknown.
    pub(super) fn remote_hurt_sound_body(&self, source_type: i32) -> &'static str {
        match self.remote.as_ref() {
            Some(r) => hurt_sound_for(&r.registries, &self.health_resources.damage, source_type),
            None => horizon_client::health::hurt_sound("hurt"),
        }
    }
    /// Leave the title for a server (ConnectScreen): an empty world fed by the connection.
    pub(super) fn connect_server_body(&mut self, address: &str, name: &str) -> Result<()> {
        self.connect_server_with_cookies(address, name, None)
    }
    /// `connect_server` carrying a transfer's stored server cookies
    /// (`TransferState`); None on an ordinary join.
    pub(super) fn connect_server_with_cookies_body(
        &mut self,
        address: &str,
        name: &str,
        cookies: Option<horizon_client::net::Cookies>,
    ) -> Result<()> {
        self.connect_server_with_options(address, name, cookies, false, false)
    }
    /// `ConnectScreen.startConnecting`: keep LAN discovery's server kind through loading.
    pub(super) fn connect_server_with_options_body(
        &mut self,
        address: &str,
        name: &str,
        cookies: Option<horizon_client::net::Cookies>,
        lan: bool,
        realm: bool,
    ) -> Result<()> {
        super::super::world_loading::preload_server_loot(
            &self.options.resources,
            &self.res.core.item_data,
        );
        if self.defer_world_entry(super::super::PendingWorldEntry::Server {
            address: address.to_owned(),
            name: name.to_owned(),
            cookies: cookies.clone(),
            lan,
            realm,
        })? {
            return Ok(());
        }
        self.join_world_rest()?;
        self.client_logging_out();
        if let Err(error) = self.join_server(address, name, None, cookies, lan, realm) {
            log::warn!("Horizon: could not join server: {error}");
            // A failed transfer leaves no packs of the connection it replaced behind.
            self.server_packs_disconnect();
            let skip = self.settings.file.bool("skipMultiplayerWarning");
            match &mut self.title {
                Some(title) => {
                    title.skip_multiplayer_warning = skip;
                    title.failed(error);
                }
                None => {
                    let mut title = self
                        .new_title(std::env::current_dir()?.join("saves"))
                        .with_user(|| self.options.player_name());
                    title.skip_multiplayer_warning = skip;
                    title.failed(error);
                    self.show_title(title);
                }
            }
            self.capture(false);
        }
        Ok(())
    }
    pub(super) fn connect_remote_session_body(
        &mut self,
        address: &str,
        name: &str,
        local: Option<horizon_protocol::connection::Pipe>,
        cookies: Option<horizon_client::net::Cookies>,
        lan: bool,
        realm: bool,
    ) -> Result<()> {
        self.realm_session = realm;
        self.reset_tutorial();
        self.perf.world_opening();
        // A server's own biomes come over the connection; the last local
        // world's packed table must not name its ids. The window's own
        // server thread keeps the one it just set.
        if local.is_none() {
            self.world_biomes = None;
            self.world_biomes_base = None;
        }
        let root = self.options.resources.clone();
        let item_data = Arc::clone(&self.res.core.item_data);
        let builtin = builtin_registries(&root).ok_or("Resources are incomplete: registries.json is missing or invalid. Click \"Prepare again\" in the launcher (or run horizon-prepare), then try again.")?;
        let lang = Arc::new(horizon_client::net::load_language(&root));
        // Options.createInformation: what the server learns about this player,
        // sent again when one of those options changes.
        let info = self
            .settings
            .client_information(client_view_distance(self.settings.render_distance));
        if local.is_none() && !self.options.split_guest {
            mod_api::content::crash::set_detail(
                "Server",
                horizon_client::net::shown_address(address),
            );
        }
        // A signed-in account always plays under its own name; a splitscreen player signs in
        // with its own account when it picked one.
        let auth = self.auth();
        let name = auth.as_ref().map_or(name, |a| a.name.as_str());
        let user = auth.as_ref().map_or_else(
            || {
                (
                    name.to_owned(),
                    horizon_protocol::crypto::offline_uuid(name),
                )
            },
            |a| (a.name.clone(), a.uuid),
        );
        self.server_packs_connect_for(local.is_none().then_some(address), user, cookies.is_some());
        let session = match local {
            Some(pipe) => Session::connect_local(
                pipe,
                name,
                auth.clone(),
                info.clone(),
                lang.clone(),
                item_data.clone(),
            ),
            None => {
                self.extras_connecting(address, name, lan);
                match cookies {
                    Some(cookies) => Session::connect_transferred(
                        address,
                        name,
                        auth.clone(),
                        info.clone(),
                        lang.clone(),
                        cookies,
                        item_data.clone(),
                    ),
                    None => {
                        let connect = if lan {
                            Session::connect_lan
                        } else {
                            Session::connect
                        };
                        connect(
                            address,
                            name,
                            auth.clone(),
                            info.clone(),
                            lang.clone(),
                            item_data.clone(),
                        )
                    }
                }
            }
        };
        let player_type = super::super::perspective::player_type(&root);
        let mut streaming = Streaming::new(
            Arc::new(NoTerrain),
            self.worker_context(),
            self.res.threads(),
        );
        streaming.remote = true;
        streaming.radius = self.settings.render_distance;
        self.streaming = streaming;
        self.world = Game::world_for_mods(self.res.facts().air, 0);
        self.save = Arc::new(SaveDir::new(
            std::env::temp_dir().join("horizon-remote-unused"),
            horizon_client::save::LevelShape::OVERWORLD,
        ));
        self.inventory = vec![None; 36];
        self.offhand = None;
        self.armor = [None, None, None, None];
        self.craft_grid = vec![None; 9];
        self.craft_size = 2;
        self.remove_local_player_body();
        self.spawn = None;
        self.mobs.clear();
        self.clear_remote_objects();
        self.clear_remote_projectiles();
        self.creatures.projectiles.clear();
        self.dropped.clear();
        self.clear_client_particles();
        self.falling.clear();
        self.mining = None;
        self.container = None;
        self.carried = None;
        self.using = None;
        self.end_mod_use();
        self.clicks.miss_time = 0;
        // A new LocalPlayer has no effects: the server sends what it has
        // (ClientboundUpdateMobEffectPacket on placement).
        self.potions = Default::default();
        self.set_mode_id(0);
        self.experience = Experience {
            level: 0,
            progress: 0.0,
            total: 0,
            score: 0,
        };
        // The connected client never opens a server world (single player also
        // joins here), so load the registries its argument parsers and local
        // suggestions need before receiving the server's command tree.
        self.ui.registries = Default::default();
        self.build_registries();
        self.ui.registries.permission = 0;
        self.ui.chat.clear();
        self.set_screen(Screen::Playing);
        self.hide_title();
        self.dim = Dim::Overworld;
        self.clear_lod_for_world_entry();
        self.pending_uploads.clear();
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections(self.world.layout());
        }
        self.apply_settings();
        self.clear_waypoints();
        self.boss_overlay.reset();
        // The chat's tree until the server sends its own, or forever when it
        // sends none: the game's own, plus the commands native mods own.
        let chat_tree = self.chat_tree_for(None);
        // Minecraft.services() has the keys wherever the game is played, single player's own
        // server included: its guests' chat keys are checked against them.
        let services_keys = Arc::new(std::sync::Mutex::new(ServicesKeys::Pending));
        let slot = services_keys.clone();
        let _ = std::thread::Builder::new()
            .name("Services keys".into())
            .spawn(move || {
                use horizon_server_files::profiles;
                let keys = profiles::fetch_profile_key_keys(&profiles::discovery_url());
                *slot.lock().unwrap_or_else(|e| e.into_inner()) = match keys {
                    Ok(keys) if !keys.is_empty() => ServicesKeys::Ready(keys),
                    Ok(_) => ServicesKeys::Missing,
                    Err(why) => {
                        log::error!("Horizon: {why}");
                        ServicesKeys::Missing
                    }
                };
            });
        self.ui.registries.selector_tokens.clear();
        self.replay.connections += 1;
        self.remote = Some(Remote {
            selector_metadata_supported: true,
            professions: self.client_professions()?,
            session,
            brand: "unknown".into(),
            recipe_viewer: Default::default(),
            features: ["minecraft:vanilla".to_string()].into(),
            quiet_commands: false,
            custom_books: false,
            logged_out: false,
            attachments: HashMap::new(),
            far_terrain: None,
            splash_sound: Default::default(),
            builtin,
            status: lang
                .get("connect.connecting")
                .cloned()
                .unwrap_or_else(|| "Connecting to the server...".into()),
            lang,
            registries: Arc::new(Dynamic::default()),
            tags: None,
            enchantments: None,
            joined: false,
            reconfiguring: None,
            logged_in: false,
            entity_id: 0,
            local_entity_silent: false,
            camera: None,
            use_item: None,
            dimension: DimensionType::vanilla("minecraft:overworld"),
            level: "minecraft:overworld".into(),
            level_key: mod_api::DimensionKey::OVERWORLD,
            level_facts: None,
            level_environment: None,
            facts_cache: Default::default(),
            added_levels: Default::default(),
            viewed: Default::default(),
            hashed_seed: 0,
            chunk_format: Arc::new(ChunkFormat::new(
                &DimensionType::vanilla("minecraft:overworld"),
                Vec::new(),
                7,
            )),
            decoder: ChunkDecoder::new(),
            view_distance: self.settings.render_distance,
            simulation_distance: self.settings.simulation_distance,
            sequence: 0,
            last_teleport_sequence: -1,
            position_sender: Default::default(),
            last_input: 0,
            was_sprinting: false,
            carried: std::cell::Cell::new(None),
            was_shared_flags: 0,
            was_living_flags: 0,
            last_recorded_pose: None,
            abilities: None,
            loaded_sent: false,
            heard_level: None,
            level_load: horizon_client::level_loading::LevelLoadTracker::new(0),
            sent_info: info,
            predictions: HashMap::new(),
            menu: ClientMenu::default(),
            ticked_before_player: std::collections::HashSet::new(),
            craft_result: None,
            menu_title: None,
            menu_title_snapshot: None,
            state_id: 0,
            use_target: None,
            menu_extra: None,
            entities: HashMap::new(),
            tick_fluids: Vec::new(),
            tick_rain: Vec::new(),
            tick_standings: Vec::new(),
            tick_water_fx: Vec::new(),
            passenger_vehicles: HashMap::new(),
            lerps: HashMap::new(),
            cart_lerps: HashMap::new(),
            stepped: HashMap::new(),
            head_lerps: HashMap::new(),
            ghast_dismount: None,
            first_passenger: false,
            passenger_index: 0,
            pending_items: HashMap::new(),
            health: (20.0, 20, 5.0),
            flash_on_set_health: false,
            game_mode: 0,
            chat: Vec::new(),
            chat_received: 0,
            chat_queue: VecDeque::new(),
            signatures: Default::default(),
            chat_previous: None,
            next_chat_index: 0,
            enforces_secure_chat: false,
            services_keys,
            chat_sessions: HashMap::new(),
            chat_validators: HashMap::new(),
            joining_narrated: None,
            chat_tree,
            chat_tree_received: false,
            secure_chat: Default::default(),
            custom_completions: Default::default(),
            players: HashMap::new(),
            tab_players: HashMap::new(),
            skin_textures: HashMap::new(),
            own_profile: OwnProfile::Idle,
            skins: HashMap::new(),
            skins_revision: 0,
            published_skins: None,
            listed: Default::default(),
            online_mode: false,
            levels: Vec::new(),
            level_players: HashMap::new(),
            level_player_names: HashMap::new(),
            level_player_textures: HashMap::new(),
            seen_players: std::collections::HashSet::new(),
            glowing: Default::default(),
            own_invisible: false,
            own_on_fire: false,
            own_arrows: 0,
            own_stingers: 0,
            own_shoulders: [None; 2],
            own_pose: None,
            profile: 0,
            profile_name: String::new(),
            destroying: None,
            destroying_item: None,
            is_destroying: false,
            interaction_swing: [true; 2],
            destroy_delay: 0,
            input: Default::default(),
            pending_keys: InputQueue::new(queued_keys::PENDING_KEYS),
            swap_answer: [false; 2],
            stop_mining: false,
            placing: None,
            flat: false,
            death_message: None,
            show_death_screen: true,
            reduced_debug_info: false,
            do_limited_crafting: false,
            player_type,
            synced: vec![None; CREATIVE_SYNCED],
            decode_errors: 0,
            clocks: HashMap::new(),
            default_clock: None,
            overworld_clock: None,
            clock_game_time: 0,
            chunk_center: (0, 0),
            backlog: VecDeque::new(),
            backlog_since: None,
            server_links: Vec::new(),
            debug_query: Default::default(),
            transfer: None,
        });
        // The level's TickRateManager follows the server's ticking packets.
        self.clock = horizon_client::clock::FixedStep::client();
        // The player's advancements and statistics are the server's.
        self.adv = Default::default();
        self.stats = Default::default();
        Ok(())
    }
    /// Prepare a client session and remote state to receive packets from a studio replay.
    pub(super) fn init_remote_for_replay_body(&mut self) -> Result<()> {
        self.realm_session = false;
        self.reset_tutorial();
        self.perf.world_opening();
        self.world_biomes = None;
        self.world_biomes_base = None;
        let root = self.options.resources.clone();
        let builtin = builtin_registries(&root).ok_or("Resources are incomplete")?;
        let lang = Arc::new(horizon_client::net::load_language(&root));
        let info = self
            .settings
            .client_information(client_view_distance(self.settings.render_distance));
        let auth = self.auth();
        let name = auth.as_ref().map_or("ReplayPlayer", |a| a.name.as_str());
        let user = auth.as_ref().map_or_else(
            || {
                (
                    name.to_owned(),
                    horizon_protocol::crypto::offline_uuid(name),
                )
            },
            |a| (a.name.clone(), a.uuid),
        );
        self.server_packs_connect(None, user);
        let session = Session::replay(name);
        let player_type = super::super::perspective::player_type(&root);
        let mut streaming = Streaming::new(
            Arc::new(NoTerrain),
            self.worker_context(),
            self.res.threads(),
        );
        streaming.remote = true;
        streaming.radius = self.settings.render_distance;
        self.streaming = streaming;
        self.world = Game::world_for_mods(self.res.facts().air, 0);
        self.save = Arc::new(SaveDir::new(
            std::env::temp_dir().join("horizon-replay-unused"),
            horizon_client::save::LevelShape::OVERWORLD,
        ));
        self.inventory = vec![None; 36];
        self.offhand = None;
        self.armor = [None, None, None, None];
        self.craft_grid = vec![None; 9];
        self.craft_size = 2;
        self.remove_local_player_body();
        self.spawn = None;
        self.mobs.clear();
        self.clear_remote_objects();
        self.clear_remote_projectiles();
        self.creatures.projectiles.clear();
        self.dropped.clear();
        self.particles.clear();
        self.pickups.clear();
        self.orbs.clear();
        self.falling.clear();
        self.mining = None;
        self.container = None;
        self.carried = None;
        self.using = None;
        self.end_mod_use();
        self.set_mode_id(0);
        self.experience = Experience {
            level: 0,
            progress: 0.0,
            total: 0,
            score: 0,
        };
        self.ui.registries = Default::default();
        self.build_registries();
        self.ui.registries.permission = 0;
        self.ui.chat.clear();
        self.set_screen(Screen::Playing);
        self.title = None;
        self.dim = Dim::Overworld;
        self.clear_lod_for_world_entry();
        self.pending_uploads.clear();
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections(self.world.layout());
        }
        self.apply_settings();
        self.clear_waypoints();
        self.boss_overlay.reset();
        let chat_tree = self.chat_tree_for(None);
        let services_keys = Arc::new(std::sync::Mutex::new(ServicesKeys::Ready(Vec::new())));

        self.ui.registries.selector_tokens.clear();
        self.replay.connections += 1;
        // The session made here is the replay's (an open's, or a seek's rebuild).
        self.replay.connection = self.replay.connections;
        self.remote = Some(Remote {
            selector_metadata_supported: true,
            professions: self.client_professions()?,
            session,
            brand: "horizon-studio".into(),
            recipe_viewer: Default::default(),
            features: ["minecraft:vanilla".to_string()].into(),
            quiet_commands: false,
            custom_books: false,
            attachments: HashMap::new(),
            far_terrain: None,
            splash_sound: Default::default(),
            builtin: builtin.clone(),
            status: "Playing Studio Replay".into(),
            lang,
            // A recording holds no configuration phase: the clocks `SetTime` names resolve here.
            registries: Arc::new(super::super::replay::replay_dynamic_registries(
                &self.ui.registries.clocks,
                &builtin,
            )),
            tags: None,
            enchantments: None,
            joined: false,
            reconfiguring: None,
            logged_in: true,
            logged_out: false,
            local_entity_silent: false,
            entity_id: 0,
            camera: None,
            use_item: None,
            dimension: DimensionType::vanilla("minecraft:overworld"),
            level: "minecraft:overworld".into(),
            level_key: mod_api::DimensionKey::OVERWORLD,
            level_facts: None,
            level_environment: None,
            facts_cache: Default::default(),
            added_levels: Default::default(),
            viewed: Default::default(),
            hashed_seed: 0,
            chunk_format: Arc::new(ChunkFormat::new(
                &DimensionType::vanilla("minecraft:overworld"),
                Vec::new(),
                7,
            )),
            decoder: ChunkDecoder::new(),
            view_distance: self.settings.render_distance,
            simulation_distance: self.settings.simulation_distance,
            sequence: 0,
            last_teleport_sequence: -1,
            position_sender: Default::default(),
            last_input: 0,
            was_sprinting: false,
            was_shared_flags: 0,
            was_living_flags: 0,
            last_recorded_pose: None,
            carried: std::cell::Cell::new(None),
            abilities: None,
            loaded_sent: false,
            heard_level: None,
            level_load: horizon_client::level_loading::LevelLoadTracker::new(0),
            sent_info: info,
            predictions: HashMap::new(),
            menu: ClientMenu::default(),
            ticked_before_player: std::collections::HashSet::new(),
            craft_result: None,
            menu_title: None,
            menu_title_snapshot: None,
            state_id: 0,
            use_target: None,
            menu_extra: None,
            entities: HashMap::new(),
            tick_fluids: Vec::new(),
            tick_rain: Vec::new(),
            tick_standings: Vec::new(),
            tick_water_fx: Vec::new(),
            passenger_vehicles: HashMap::new(),
            lerps: HashMap::new(),
            cart_lerps: HashMap::new(),
            stepped: HashMap::new(),
            head_lerps: HashMap::new(),
            ghast_dismount: None,
            first_passenger: false,
            passenger_index: 0,
            pending_items: HashMap::new(),
            health: (20.0, 20, 5.0),
            flash_on_set_health: false,
            game_mode: 0,
            chat: Vec::new(),
            chat_received: 0,
            chat_queue: VecDeque::new(),
            signatures: Default::default(),
            chat_previous: None,
            next_chat_index: 0,
            enforces_secure_chat: false,
            services_keys,
            chat_sessions: HashMap::new(),
            chat_validators: HashMap::new(),
            joining_narrated: None,
            chat_tree,
            chat_tree_received: false,
            secure_chat: Default::default(),
            custom_completions: Default::default(),
            players: HashMap::new(),
            tab_players: HashMap::new(),
            skin_textures: HashMap::new(),
            own_profile: OwnProfile::Idle,
            skins: HashMap::new(),
            skins_revision: 0,
            published_skins: None,
            listed: Default::default(),
            online_mode: false,
            levels: Vec::new(),
            level_players: HashMap::new(),
            level_player_names: HashMap::new(),
            level_player_textures: HashMap::new(),
            seen_players: std::collections::HashSet::new(),
            glowing: Default::default(),
            own_invisible: false,
            own_on_fire: false,
            own_arrows: 0,
            own_stingers: 0,
            own_shoulders: [None; 2],
            own_pose: None,
            profile: 0,
            profile_name: String::new(),
            destroying: None,
            destroying_item: None,
            is_destroying: false,
            interaction_swing: [true; 2],
            destroy_delay: 0,
            input: Default::default(),
            pending_keys: InputQueue::new(queued_keys::PENDING_KEYS),
            swap_answer: [false; 2],
            stop_mining: false,
            placing: None,
            flat: false,
            death_message: None,
            show_death_screen: true,
            reduced_debug_info: false,
            do_limited_crafting: false,
            player_type,
            synced: vec![None; CREATIVE_SYNCED],
            decode_errors: 0,
            clocks: HashMap::new(),
            default_clock: None,
            overworld_clock: None,
            clock_game_time: 0,
            chunk_center: (0, 0),
            backlog: VecDeque::new(),
            backlog_since: None,
            server_links: Vec::new(),
            debug_query: Default::default(),
            transfer: None,
        });
        self.clock = horizon_client::clock::FixedStep::client();
        self.adv = Default::default();
        self.stats = Default::default();
        Ok(())
    }

    /// Disconnect and return to the multiplayer screen (DisconnectedScreen), whose Back
    /// leads to the title after a single player world.
    pub(super) fn leave_server_body(&mut self, reason: Option<Text>) -> Result<()> {
        self.leave_server_with(reason, false)
    }

    /// `Minecraft.disconnect` for a protocol transfer: the world and the connection go, the
    /// server packs stay, and no screen opens because the next connection follows at once.
    pub(super) fn leave_server_for_transfer_body(&mut self) -> Result<()> {
        self.leave_server_with(None, true)
    }

    fn leave_server_with(&mut self, reason: Option<Text>, transferring: bool) -> Result<()> {
        self.interrupt_export_for_world_exit("export interrupted because the client disconnected");
        // A replay ends with the session it plays on: one that owns its session, left without
        // a reason, closes as Escape closes it, which leaves that session; otherwise the replay
        // ends alone and the leave goes on (a reason to the disconnect screen).
        self.drop_stale_replay();
        if self.replay.playing() {
            let owns = self.replay.standalone
                || self
                    .remote
                    .as_ref()
                    .is_some_and(|remote| remote.brand == "horizon-studio");
            if owns && reason.is_none() {
                self.close_studio_replay();
                return Ok(());
            }
            self.end_studio_replay();
            if owns {
                // Left with a reason: the disconnect screen below says it, as without a
                // replay; the hands and the view the replay set go first.
                self.reset_studio_hand_state();
                self.camera_type = horizon_client::camera::CameraType::FirstPerson;
            }
        }
        if self.integrated.as_ref().is_some_and(|s| !s.stopped()) {
            return match reason {
                Some(reason) => self.begin_integrated_disconnect(reason),
                None => self.begin_closing_save(false),
            };
        }
        self.client_logging_out();
        self.finish_perf();
        if reason.is_none() {
            // Leaving on purpose: nothing for auto-reconnect to retry.
            self.extras_left();
        }
        self.clear_waypoints();
        self.hud_disconnected();
        // Vanilla's Hud also ticks on the title and connect screens, so the
        // action bar has run out by the next world; Horizon's ticks in one.
        self.titles.overlay_time = 0;
        // GameRenderer.resetData and narrator.clear (Minecraft.disconnect).
        self.item_activation.reset();
        // Kept mod block entity geometry belongs to the world leaving.
        self.kept_block_entities.get_mut().clear();
        self.mod_view_distances.take();
        self.clear_narrator();
        // updateLevelInEngines(null): every sound stops and the music manager
        // lets go of its track. Leaving a server comes straight here, not
        // through finish_close_world, so the biome's ambient loop would
        // otherwise play on over the title screen until the next world.
        self.ambience.clear(&mut self.audio);
        self.music.stop_playing(None, &mut self.audio);
        self.audio.stop_all();
        self.mod_sounds.clear();
        // A world is leaving either way: its icon attempt goes with it.
        self.world_icon = None;
        // Minecraft.disconnect: the client level goes.
        if self.remote.is_some() {
            self.forget_all_remote_columns();
        }
        if let Some(old) = self.remote.as_mut().and_then(|r| r.heard_level.take()) {
            self.native_client_level_unload(&old);
        }
        if self.remote.is_some() {
            self.native_client_disconnecting();
        }
        if let Some(mut r) = self.remote.take() {
            r.session.disconnect();
        }
        super::super::lan::sync::forget_sync_data();
        self.gui.atlas.clear_object_skins();
        if transferring {
            self.server_packs_disconnect_for(true);
        } else {
            self.server_packs_disconnect();
        }
        self.code_of_conduct = None;
        self.reset_tutorial();
        self.close_dialogs();
        // Gui.setScreen -> KeyMapping.releaseAll: the title or disconnect
        // screen releases every held key. Its release goes to that screen,
        // so a key kept here would still be down in the next world (arismoko/horizon#3246).
        self.keys.clear();
        self.left = false;
        self.right = false;
        // The shared world is closing: its invite code ends with it.
        self.friends_lan_closed();
        // Single player's server thread saves the world before the title shows.
        let single_player = match self.integrated.take() {
            Some(server) => {
                server.stop()?;
                true
            }
            None => false,
        };
        // A splitscreen player leaving says nothing of where P1 plays.
        if !self.options.split_guest {
            mod_api::content::crash::clear_detail("Server");
        }
        self.clock = Default::default();
        self.adv = Default::default();
        self.stats = Default::default();
        self.score.board = Default::default();
        self.streaming = Streaming::new(Arc::new(NoTerrain), self.worker_context(), 1);
        self.streaming.paused = true;
        self.reset_client_world(0);
        self.remove_local_player_body();
        // The vehicle went with the level: an id kept here would seat the
        // next login on an entity that is not there.
        self.riding = None;
        // Out of the server's bed: InBedChatScreen goes with the level.
        self.sleeping = None;
        self.sleep_timer = 0;
        self.clear_entities();
        self.score.board = Default::default();
        self.tab_list = Default::default();
        self.border = Default::default();
        self.set_screen(Screen::Playing);
        self.container = None;
        self.clear_lod();
        self.lod_first_load = false;
        self.pending_uploads.clear();
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections(self.world.layout());
        }
        if transferring {
            return Ok(());
        }
        let saves = std::env::current_dir()?.join("saves");
        let mut title = self
            .new_title(saves.clone())
            .with_user(|| self.options.player_name());
        title.last_server = self.settings.file.string("lastServer");
        title.skip_multiplayer_warning = self.settings.file.bool("skipMultiplayerWarning");
        if single_player && reason.is_none() {
            self.show_title(title);
            self.capture(false);
            return Ok(());
        }
        title.mode = horizon_client::title::Mode::Multiplayer(match reason {
            Some(reason) => horizon_client::multiplayer::Screen::Disconnected {
                reason,
                retry: None,
                lines: std::cell::Cell::new(1),
                to_title: single_player,
            },
            None => horizon_client::multiplayer::Screen::list(&saves),
        });
        self.show_title(title);
        self.capture(false);
        Ok(())
    }
}
