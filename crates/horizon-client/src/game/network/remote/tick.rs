//! The client's tick while connected, and the local player's.

use super::*;

impl Game<'_> {
    /// `ClientPacketListener.tick`'s `LevelLoadTracker.tickClientLoad`: the
    /// player's section is shown once its column is lit; when the level is
    /// ready the loading screen closes and the server hears `PlayerLoaded`
    /// (`notifyPlayerLoaded`).
    pub(super) fn tick_client_load_body(&mut self) {
        let Some(player) = self.player.as_ref() else {
            return;
        };
        if !self
            .remote
            .as_ref()
            .is_some_and(|r| r.level_load.is_active())
        {
            return;
        }
        // `LevelRenderer`'s playerCompiledSectionCallback: the section holding
        // the camera is there (the column is lit, as meshing needs).
        let (cx, cz) = (
            (player.position.x.floor() as i32) >> 4,
            (player.position.z.floor() as i32) >> 4,
        );
        let section_ready = self.streaming.is_lit(cx, cz);
        let camera_y = (player.position.y + f64::from(self.eye)).floor() as i32;
        let shape = self.world.shape;
        let check = horizon_client::level_loading::PlayerCheck {
            outside_build_height: camera_y < shape.min_y || camera_y >= shape.min_y + shape.height,
            spectator: player.spectator,
            alive: player.health.health() > 0.0,
        };
        let now = client_millis();
        // LevelLoadingScreen.tick.
        self.tick_load_progress();
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        if !r.level_load.poll_client_load(now, section_ready, check) {
            return;
        }
        if !r.loaded_sent {
            r.loaded_sent = true;
            self.send(S::PlayerLoaded(Empty));
        }
        // LevelLoadingScreen.onClose.
        self.say_system_now(&horizon_client::text::Text::translate(
            "narrator.ready_to_play",
            &[],
        ));
        self.capture_loaded_world();
    }

    /// One client tick while connected (Minecraft.tick with a ClientLevel).
    pub(super) fn tick_remote_body(&mut self) -> Result<()> {
        self.tick_remote_level()?;
        // Its rockets tick after the player (even waiting for chunks), then ParticleEngine.tick.
        self.tick_own_rockets();
        self.tick_remote_particle_engine();
        self.tick_book_snapshot_retry();
        // Far tiles the samplers asked for (`lod_net`).
        if let Some(r) = &self.remote {
            r.session.send_far_terrain();
        }
        self.tick_chat_session();
        // Minecraft.tick sends this even when the player cannot tick yet,
        // including while waiting for chunks. Keep it outside early returns.
        if self.remote.as_ref().is_some_and(Remote::has_level) {
            self.send(S::ClientTickEnd(Empty));
        }
        Ok(())
    }

    pub(super) fn tick_remote_player_body(&mut self, running: bool, hold: bool) -> Result<()> {
        // Inventory.tick's ItemStack.inventoryTick: the pop time runs down.
        if !hold {
            for pop in &mut self.hotbar_pop {
                *pop = pop.saturating_sub(1);
            }
        }
        // ClientLevel.tick: the border's size change, while the level runs.
        if running {
            self.border.tick();
        }
        // ClientLevel.tick: the End's sky flash, frozen or not.
        self.tick_end_flash();
        self.player_border();
        // The entities added after the player tick after it: their pushes
        // (from where they ended last tick, onto where the player ended)
        // reach its next move, this one.
        let pushed_after = self.remote_pushes(false);
        self.tick_remote_entities(running);
        self.tick_remote_arm_poses();
        self.tick_random_textures();
        // Breeze.tick's block dust: drawn by each client for the breezes it sees.
        if running {
            self.tick_breeze_effects();
        }
        self.tick_remote_guardian_eyes(running);
        // Level.tickBlockEntities: a frozen level ticks no block entities.
        // A step advances the moving pistons created by the block event once,
        // rather than finishing them during the frozen ticks between steps.
        if running {
            self.tick_block_entity_anims();
            self.tick_remote_spawners();
            self.client_tick_beacons();
            self.tick_remote_potent_sulfur();
            self.with_level(|b, level, ticks| {
                b.client_tick_block_entities(level, ticks);
            });
            let pushes = std::mem::take(&mut self.block_ticks.piston_pushes);
            self.push_piston_entities(pushes);
            self.sync_moving_pistons();
        }
        self.tick_hand();
        if running {
            self.tick_particle_spawns(); // ParticleEngine.tick: tick_remote, after the rockets.
        } else if self.clock.still() {
            self.settle_rain_fog();
        }
        self.tick_frame_visuals(running);
        // Under `HORIZON_STILL` they wait for the world too: endermen's and blazes' particles
        // draw on the particles' random, and effects count down, on every tick a frozen
        // client runs, as many as the run took to get there.
        if !hold {
            self.tick_client_ambient();
            self.tick_client_effects();
        }
        // Gui.tick: the titles, the action bar and the held item's name.
        if !hold {
            self.tick_hud();
        } else {
            // Only its countdowns wait: lying down still opens the bed's chat, waking closes it.
            self.tick_in_bed_chat();
        }
        self.tick_subtitles();
        self.tick_client_load();
        let entity_collisions = self.player_entity_collisions();
        // Those added before it tick before it: their pushes reach this move.
        let pushed = pushed_after.add_vector(self.remote_pushes(true));
        let sprint_window = self.settings.sprint_window;
        let Some(player) = self.player.as_mut() else {
            return Ok(());
        };
        let (cx, cz) = (
            (player.position.x.floor() as i32) >> 4,
            (player.position.z.floor() as i32) >> 4,
        );
        if !self.streaming.is_lit(cx, cz) {
            // LocalPlayer.tick can wait for chunks, but Minecraft.tick still
            // calls sendChanges: rotations and the 20-tick position reminder
            // must continue, as well as moves into the unloaded area.
            if let Some(r) = self.remote.as_ref()
                && r.loaded_sent
            {
                let input = r.last_input;
                self.remote_send_state(input);
            }
            return Ok(());
        }
        // LocalPlayer.tick reaches Player.tick only once the client has loaded
        // and the player's chunk is there; its endpoints bracket that work
        // alone, also on the death screen, not the level pass before it.
        let player = self
            .remote
            .as_ref()
            .is_some_and(|r| r.loaded_sent)
            .then(|| {
                (mod_api::__private::listened(mod_api::HookId::PlayerTickPre)
                    || mod_api::__private::listened(mod_api::HookId::PlayerTickPost))
                .then(|| self.local_player())
                .flatten()
            })
            .flatten();
        let player_from = entity_section_steps::player_before(self);
        if let Some(player) = player {
            self.native_player_tick_pre(mod_api::__private::PlayerTickPreCall { player });
        }
        let result = self.tick_local_player(entity_collisions, pushed, sprint_window);
        entity_section_steps::player_after(self, player_from);
        // `EntityEvent.Size` for the pose the tick set.
        self.native_local_player_size();
        if let Some(player) = player {
            self.native_player_tick_post(mod_api::__private::PlayerTickPostCall { player });
        }
        result
    }

    /// `LocalPlayer.tick`'s `Player.tick` work: input, riding, motion, sleep
    /// and the state the server hears.
    pub(super) fn tick_local_player_body(
        &mut self,
        entity_collisions: Vec<horizon_core::geometry::Aabb>,
        pushed: Vec3,
        sprint_window: i32,
    ) -> Result<()> {
        // Player.tick's ItemCooldowns.tick, after Pre and after
        // handleKeybinds: this path replaces `tick_world`'s for a server's
        // client (multiplayer and the default single player), so the entries
        // advance here or the slot stays shaded until the server says the
        // entry ended. `onCooldownEnded` is the plain ItemCooldowns' no-op on
        // a client. Ticking it after the keybinds keeps a held use from
        // trying again a tick before the server's entry ends.
        self.cooldowns.tick();
        // Player.tick's sleep counter (the darkening and the wake-up fade).
        self.sleep_timer =
            super::super::tick_sleep_timer(self.sleep_timer, self.sleeping.is_some());
        let stick = self.pad_move();
        if self.player.is_none() {
            return Ok(());
        }
        // The LevelLoadingScreen takes the input until the level is ready.
        let loading = self
            .remote
            .as_ref()
            .is_some_and(|r| r.level_load.is_active());
        let playing = self.screen == Screen::Playing && !loading && !self.dialog_open();
        let held = |action| playing && self.keys.held(action);
        let mut keys = Keys {
            forward: held(Action::Forward),
            backward: held(Action::Back),
            left: held(Action::Left),
            right: held(Action::Right),
            jump: held(Action::Jump),
            shift: held(Action::Sneak),
            sprint: held(Action::Sprint),
            sprint_suppressed: false,
            analog: None,
        }
        .with_stick(stick.filter(|_| playing));
        // On a server (not this game's own server thread), the stick moves as the keys it sets do
        // (`KeyboardInput.tick`: 8 directions, full speed). The server only hears those keys
        // (`ServerboundPlayerInputPacket`), and an anticheat that predicts from them pulls back
        // a player moving at the stick's own angle and speed.
        if self.integrated.is_none() && self.server_address().is_some() {
            keys.analog = None;
        }
        let keys = self.mod_movement_keys(keys);
        let input = u8::from(keys.forward)
            | u8::from(keys.backward) << 1
            | u8::from(keys.left) << 2
            | u8::from(keys.right) << 3
            | u8::from(keys.jump) << 4
            | u8::from(keys.shift) << 5
            | u8::from(keys.sprint) << 6;
        // LocalPlayer.applyInput and aiStep: watching through another
        // entity, the player does not move itself; its keys still reach the
        // server, whose sneak check lets go of the camera.
        let keys = if self.controls_camera() {
            keys
        } else {
            Keys::default()
        };
        // The held lead's end eases from the look at the start of the tick
        // (`yRotO`/`xRotO`), as the local tick does; `tick` returns before it
        // reaches its own write when this window is a server client.
        self.rope_rot_old = (self.yaw, self.pitch);
        // Before the tick, and a rider's toggle in `remote_ride`.
        let was_flying = self.player.as_ref().is_some_and(|p| p.flying);
        // A passenger moves with its vehicle (Entity.rideTick), not by itself.
        let riding = self.screen != Screen::Dead && self.remote_ride(keys, sprint_window)?;
        // The server sends no passenger's position: they sit where their
        // vehicles are now.
        self.position_riders();
        // Entity.getFluidInteractionBox asks the vehicle: a boat afloat shelters
        // its passenger's eyes (AbstractBoat.modifyPassengerFluidInteractionBox).
        let boat = self
            .riding
            .and_then(|id| self.boats.iter().find(|b| b.id == id))
            .map(|b| horizon_client::motion::RiddenBoat {
                under_water: b.under_water(),
                max_y: b.bounds().max_position().y,
            });
        let Some(player) = self.player.as_mut() else {
            return Ok(());
        };
        player.fluids.boat = boat;
        if !riding && !self.replay.playing() {
            self.previous_position = player.position;
        }
        let before = (
            player.ground.on_ground,
            [player.velocity.x, player.velocity.y, player.velocity.z],
            [player.position.x, player.position.y, player.position.z],
        );
        self.sync_player_body();
        // LivingEntity.tickDeath: the local player counts its deathTime once
        // per client tick while dead (the camera's death roll and zoom).
        if self.screen == Screen::Dead {
            self.ui.death_time = self.ui.death_time.saturating_add(1);
        }
        let sleeping = self.sleeping;
        let sleep_position = sleeping.and_then(|head| self.sleeping_position(head));
        let Some(player) = self.player.as_mut() else {
            return Ok(());
        };
        let was_gliding = player.fall_flying;
        let dead = self.screen == Screen::Dead;
        // LivingEntity.isImmobile: a sleeper lies still, even if its bed's
        // shape is unavailable here; only the server decides when to wake.
        if sleeping.is_some() {
            if let Some(at) = sleep_position {
                player.position = at;
            }
            player.velocity = Vec3::ZERO;
            player.pitch = 0.0;
        } else if !dead && !riding {
            player.client_side = true;
            player.velocity = player.velocity.add_vector(pushed);
            // LivingEntity.checkAutoSpinAttack: a Riptide spin that touches a
            // mob ends and bounces back; the server deals the hit.
            player.spin_targets.clear();
            if player.auto_spin_attack_ticks > 0 {
                player.spin_targets.extend(
                    self.mobs
                        .iter()
                        .filter(|m| !m.removed)
                        .map(|m| (m.id, m.bounds())),
                );
            }
            MotionWorld {
                entities: &entity_collisions,
                player,
                scene: &mut self.world,
                assets: &self.pack.assets,
                collision_binding: self.collision,
                resources: self.motion,
                health: &self.health_resources,
                loop_context: &self.loop_context,
                damage_hooks: &self.damage_hooks,
                tag_items: None,
                block_registries: None,
            }
            .controlled_simulation_tick_with_sprint_window(keys, sprint_window)?;
        } else {
            // LivingEntity.baseTick counts hurtTime down for a dead or riding
            // player too (controlled_simulation_tick does it otherwise): left
            // at 10, the hurt tilt restarted every frame and shook the camera.
            player.health.advance_clock();
            if riding && !dead {
                player.client_side = true;
                let mut passenger = MotionWorld {
                    entities: &entity_collisions,
                    player: &mut *player,
                    scene: &mut self.world,
                    assets: &self.pack.assets,
                    collision_binding: self.collision,
                    resources: self.motion,
                    health: &self.health_resources,
                    loop_context: &self.loop_context,
                    damage_hooks: &self.damage_hooks,
                    tag_items: None,
                    block_registries: None,
                };
                passenger.passenger_pose_tick()?;
                // LivingEntity.aiStep: a passenger freezes in powder snow too, and the
                // frostbite overlay shows it (#8156).
                passenger.passenger_block_effects_tick()?;
            }
        }
        // Player.tick holds the player at the world's edge after the move: X
        // and Z clamp, Y does not (Player.java:264-265). A passenger is left
        // alone: Entity.rideTick runs Player.tick and only then seats the rider
        // on the vehicle (`Entity.java:2433-2439`), so the seat's position is
        // what survives the tick, and clamping here would pull a rider off a
        // seat that legally sits past the edge.
        if !riding {
            player.position.x = player.position.x.clamp(-2.9999999E7, 2.9999999E7);
            player.position.z = player.position.z.clamp(-2.9999999E7, 2.9999999E7);
        }
        // The server decides fire the window cannot predict (blaze fireballs,
        // fire aspect, lightning): Entity.isOnFire reads the synched flag, so
        // the predicted tick's countdown must not end it while it is set.
        player.fire_ticks = hold_server_fire(
            player.fire_ticks,
            self.remote.as_ref().is_some_and(|r| r.own_on_fire),
        );
        // LocalPlayer.onUpdateAbilities: flight turned on or off (a double
        // jump, a spectator, landing) is sent to the server.
        let flying = player.flying;
        // LocalPlayer.aiStep: a glide this client started is sent to the
        // server (ServerboundPlayerCommandPacket START_FALL_FLYING).
        let started_gliding = player.fall_flying && !was_gliding;
        let events = std::mem::take(&mut player.events);
        self.previous_eye = self.eye;
        self.eye += (player.dimensions.eye_height - self.eye) * 0.5;
        for event in events {
            match event {
                horizon_client::motion::MotionEvent::Emission {
                    position,
                    state,
                    muffled,
                } => {
                    let at = [position.x, position.y, position.z];
                    let sounds = self.own_sound_type(at, state, mod_api::BlockSoundCause::Step);
                    // Player.playStepSound: muffled under the swim sound in water.
                    let (volume, pitch) = if muffled {
                        (sounds.volume * 0.05, sounds.pitch * 0.8)
                    } else {
                        (sounds.volume * 0.15, sounds.pitch)
                    };
                    self.play_client_player_sound(&sounds.step, volume, pitch);
                }
                horizon_client::motion::MotionEvent::Swim { volume, pitch } => {
                    self.play_client_player_sound("minecraft:entity.player.swim", volume, pitch);
                }
                // Entity.doWaterSplashEffect on the client's own level: the
                // server's splash skips this player (Player.playSound), and
                // its particles are the client's own.
                horizon_client::motion::MotionEvent::Splash {
                    high_speed,
                    volume,
                    pitch,
                } => {
                    let name = if high_speed {
                        "minecraft:entity.player.splash.high_speed"
                    } else {
                        "minecraft:entity.player.splash"
                    };
                    self.play_client_player_sound(name, volume, pitch);
                }
                horizon_client::motion::MotionEvent::SplashParticle {
                    bubble,
                    position,
                    velocity,
                } => {
                    use horizon_client::particles::{ParticleOptions, ParticleType};
                    let kind = if bubble {
                        ParticleType::Bubble
                    } else {
                        ParticleType::Splash
                    };
                    self.particles.add(
                        &ParticleOptions::simple(kind),
                        [position.x, position.y, position.z],
                        [velocity.x, velocity.y, velocity.z],
                    );
                }
                horizon_client::motion::MotionEvent::Chime { volume, pitch } => {
                    self.play_client_player_sound(
                        "minecraft:block.amethyst_block.chime",
                        volume,
                        pitch,
                    );
                }
                // NetherPortalBlock.entityInside on the client's own level:
                // the portal overlay and its trigger sound (the server moves
                // the player).
                horizon_client::motion::MotionEvent::PowderSnowMelt { pos } => {
                    self.powder_snow_melt(pos);
                }
                horizon_client::motion::MotionEvent::InsidePortal { .. } => {
                    self.client_inside_nether_portal();
                }
                // HoneyBlock.maybeDoSlideEffects on the client's own level:
                // the slide sound, heard here (LocalPlayer.playSound); the
                // particles come from the server's event 53.
                horizon_client::motion::MotionEvent::HoneySlide { sound: true, .. } => {
                    self.play_client_player_sound("minecraft:block.honey_block.slide", 1.0, 1.0);
                }
                // LivingEntity.causeFallDamage and HoneyBlock.fallOn on the
                // client's own level: the fall and honey sounds are heard here
                // (LocalPlayer.playSound); the server's copy skips this player
                // (Player.playSound).
                horizon_client::motion::MotionEvent::Fall(event) => self.block_fall(event),
                _ => {}
            }
        }
        // A stand riding the player follows this move (tickPassenger).
        self.seat_own_stands();
        // Other players on this player (possibly through a plugin seat) follow this tick too;
        // the full pass only when something rides this player.
        if self.remote.as_ref().is_some_and(|r| {
            let me = r.entity_id as u32;
            r.passenger_vehicles.values().any(|&v| v == me)
        }) {
            self.position_riders();
        }
        // The server owns health and hunger.
        self.apply_remote_health();
        self.tick_avatar(before, false);
        // LivingEntity.tick: a sleeper's xRot is 0.
        if sleeping.is_some() {
            self.pitch = 0.0;
            self.snap_player_model_pitch();
        }
        // The player's own model (third person, the inventory's preview).
        if !self.replay.playing() {
            self.tick_player_model();
        }
        self.client_hooks_tick();
        if !dead {
            self.remote_tick_using();
        }
        if flying != was_flying {
            self.send(S::PlayerAbilities(play::PlayerAbilitiesRequest {
                flags: if flying { 2 } else { 0 },
            }));
        }
        if started_gliding {
            let id = self.remote.as_ref().map_or(0, |r| r.entity_id);
            self.send(S::PlayerCommand(play::PlayerCommand {
                id,
                action: 6,
                data: 0,
            }));
        }
        // LocalPlayer.sendChanges sends `input.keyPresses`, which an auto-jump's `makeJump` set to
        // jump for this tick: the server's prediction plays the jump from it. Sent as held, the jump
        // was the client's alone, and the server saw a jump its keys didn't allow.
        let auto_jumped = self.player.as_ref().is_some_and(|p| p.was_auto_jump);
        self.remote_send_state(input | u8::from(auto_jumped) << 4);
        Ok(())
    }
}
