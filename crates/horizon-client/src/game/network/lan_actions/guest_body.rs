//! Restore the guest's first body without borrowing another player's RNG.
use super::*;
use horizon_client::{
    lan::guest::Guest,
    motion::{MotionResources, PlayerMotion},
};
use horizon_core::geometry::Vec3;

pub(super) fn initialize(resources: &MotionResources, guest: &mut Guest) -> bool {
    let p = Vec3::new(guest.position[0], guest.position[1], guest.position[2]);
    let Ok(motion) = PlayerMotion::new(resources, p) else {
        return false;
    };
    guest.motion = Some(Box::new(motion));
    let entity_id = guest.entity_id as u32;
    let accepted_hit = guest.carried_hit.take();
    if let Some(motion) = guest.motion.as_mut() {
        motion.health.accepted_hit = accepted_hit;
        motion.random = horizon_client::mobs::entity_random(entity_id, p);
    }
    // The first body after joining burns, breathes and falls on as saved.
    let g = &mut *guest;
    if let Some(saved) = g.loaded_body.take()
        && let Some(m) = g.motion.as_mut()
    {
        saved.apply(m);
        let _ = m.set_walking_speed_base(g.abilities.walking_speed);
        // LivingEntity.readAdditionalSaveData: the saved attributes.
        let _ = m.apply_saved_attributes(g.saved.as_ref().and_then(|t| t.get("attributes")));
    }
    // ServerPlayer.restoreFrom: a body that follows another takes its
    // attributes (a scale of 0.1 stays 0.1 after a respawn or a trip
    // to another level).
    let g = &mut *guest;
    if let Some(carried) = g.carried_attributes.take()
        && let Some(m) = g.motion.as_mut()
    {
        let _ = m.apply_saved_attributes(Some(&carried));
    }
    // The saved golden hearts go on the first body of a player who
    // has been here before.
    let saved = guest.saved_absorption.take();
    if let (Some(m), Some(amount)) = (guest.motion.as_mut(), saved) {
        let _ = m.internal_set_absorption(amount);
    }
    let kept = guest.logs_kept.take();
    if let (Some(m), Some((hurts, attacks))) = (guest.motion.as_mut(), kept) {
        m.hurt_log = hurts;
        m.attack_log = attacks;
    }
    true
}

impl Game<'_> {
    /// The guest's body on the server each tick: ServerPlayer.tick's damage
    /// clock and Entity.baseTick's fluid phase (burning), then for each move
    /// ServerPlayer.doCheckFallDamage (the fall distance grows while they go
    /// down and hurts them where they land, as the host's own landing does;
    /// water resets it, lava halves it) and checkMovementStatistics (the
    /// exhaustion of sprinting and swimming), then LivingEntity.baseTick's
    /// hazards (drowning, lava, fire blocks, suffocation, the void), and
    /// regeneration and the food tick (ServerPlayer.doTick). Their hurt
    /// sounds are heard at the guest.
    pub(super) fn guest_server_tick(&mut self, conn: u32) {
        self.guest_replay(conn);
        self.guest_vehicle_tick(conn);
        // The last move's fall stands in for the server's delta movement.
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let fell = host
            .hub
            .guest(conn)
            .and_then(|g| g.moves.last().map(|m| m.0[1]))
            .unwrap_or(0.0);
        self.guest_glide_tick(conn, fell);
        let Some((moves, from_ground)) = self.lan.as_mut().and_then(|host| {
            host.hub.update_guest(conn, |g| {
                let moves = std::mem::take(&mut g.moves);
                let from_ground = if moves.is_empty() {
                    g.on_ground
                } else {
                    g.moves_from_ground
                };
                (moves, from_ground)
            })
        }) else {
            return;
        };
        let g = self.lan.as_ref().unwrap().hub.guest(conn).unwrap();
        // Each move the player sent put it somewhere (`handleMovePlayer`'s `setPos`).
        let (id, end) = (g.entity_id as u32, g.position);
        self.entity_section_walked(id, end, moves.iter().map(|m| m.0));
        let g = self.lan.as_ref().unwrap().hub.guest(conn).unwrap();
        if g.spectator() {
            // ServerPlayer.doTick runs the player's whole tick for a spectator
            // too, unless its box touches an unloaded chunk
            // (`touchingUnloadedChunk`): its effects (LivingEntity.baseTick),
            // Player.aiStep's tickRegeneration and foodData.tick go on while
            // it watches (#11312). The hazards, the travel and the movement
            // statistics below are what a spectator's body skips.
            let (at, body) = (g.position, g.dimensions());
            let loaded =
                !touching_unloaded_chunk(at, body, |x, z| self.world.chunks.contains_key(&(x, z)));
            if !g.effects.list.is_empty() {
                self.as_guest(conn, |game| game.tick_mob_effects());
            }
            self.as_guest(conn, |game| {
                game.handle_shoulder_entities();
                if !loaded {
                    return;
                }
                let rules = game.world.health_rules;
                let Some(player) = game.player.as_mut() else {
                    return;
                };
                // Entity.baseTick's clocks (hurtTime, invulnerableTime) run too.
                player.health.advance_clock();
                let _ = player.peaceful_regeneration(&game.loop_context, rules);
                let _ = player.food_tick(
                    &game.loop_context,
                    crate::game::entity::damage_level_of!(game),
                    &game.health_resources,
                    rules,
                );
                let events = std::mem::take(&mut player.events);
                game.record_combat(&events);
            });
            return;
        }
        let at = g.position;
        let dimensions = g.dimensions();
        let (sneaking, sleeping) = (g.sneaking, g.sleeping);
        let riding = g.vehicle.is_some();
        if g.motion
            .as_ref()
            .is_some_and(|m| m.auto_spin_attack_ticks > 0)
        {
            self.as_guest(conn, |game| game.server_spin_attack());
        }
        let in_wall = !sleeping && self.in_wall(conn, at, dimensions);
        let mut soul = None;
        let events = self.as_guest(conn, |game| {
            let (rules, motion) = (game.world.health_rules, game.motion);
            let health = &motion.health;
            let leather_boots = super::super::worn_boots::leather_boots(&game.armor);
            let freeze_immune = game.wears_freeze_immune();
            let burn_immune = game.wears_burn_from_stepping_immunity();
            let boat = game.riding_boat_fluid_box();
            let Some(player) = game.player.as_mut() else {
                return (Vec::new(), None);
            };
            player.fluids.boat = boat;
            let mut w = MotionWorld {
                entities: &[],
                player,
                scene: &mut game.world,
                assets: &game.pack.assets,
                collision_binding: game.collision,
                resources: game.motion,
                health: &game.health_resources,
                tag_items: Some(world_items!(game)),
                loop_context: &game.loop_context,
                damage_hooks: &game.damage_hooks,
                block_registries: Some(&game.pack.block_registries),
            };
            w.player.health.advance_clock();
            let _ = w.fluid_base_tick();
            w.player.ground.on_ground = from_ground;
            // Player.aiStep: a flying player that isn't riding starts every tick
            // with no fall distance, so flying down in creative and switching to
            // survival lands without the flight's distance.
            if w.player.flying && !riding {
                w.player.fall_distance = 0.0;
            }
            let travelled: Vec<[f64; 3]> = moves.iter().map(|m| m.0).collect();
            // The tick's whole movement, for the delta movement a glide's wall
            // hit reads (`handleMovePlayer` gives the server copy the client's
            // own movement).
            let tick_movement = travelled.iter().fold(Vec3::ZERO, |sum, d| {
                sum.add_vector(Vec3::new(d[0], d[1], d[2]))
            });
            // Entity.move's movementThisTick: each move, from where the
            // tick's first one started.
            let end = w.player.position;
            let start = travelled
                .iter()
                .fold(end, |p, d| p.subtract(Vec3::new(d[0], d[1], d[2])));
            w.player.history.clear();
            let mut from = start;
            for d in &travelled {
                let delta = Vec3::new(d[0], d[1], d[2]);
                let to = from.add_vector(delta);
                w.player
                    .history
                    .add(horizon_world::entity_move::MovementRecord {
                        from,
                        to,
                        original: Some(delta),
                    });
                from = to;
            }
            let _ = w.server_stuck_in_blocks(start);
            // Where the first move that went up ended, if one did, and whether the player
            // was alive then: the hazards below may kill it later in this tick.
            let mut rose = None;
            let mut at = start;
            for (moved, on_ground) in moves {
                let movement = Vec3::new(moved[0], moved[1], moved[2]);
                at = at.add_vector(movement);
                let landed = (|| -> horizon_client::Result<()> {
                    w.set_on_ground_with_movement(on_ground, false, movement)?;
                    let pos = w.legacy_on_position()?;
                    let state = w.block_state(pos)?;
                    w.check_fall_damage(moved[1], on_ground, state, pos)?;
                    // handlePlayerPositionChange: a move that goes up ends the fall
                    // (`resetFallDistance`), after that move's own fall check.
                    if moved[1] > 0.0 {
                        w.player.fall_distance = 0.0;
                        rose.get_or_insert((at, !w.player.health.dead_or_dying()));
                    }
                    w.check_movement_statistics(movement)
                })();
                if landed.is_err() {
                    break;
                }
            }
            // A ServerPlayer travels on the server too
            // (MoveSimulationType.AUTHORITATIVE_SIDE_AND_SERVER), so every tick
            // on a ladder or vine LivingEntity.handleOnClimbable resets its fall
            // distance: climbing down doesn't hurt where it lands (arismoko/horizon#1255).
            if w.climbable().unwrap_or(false) {
                w.player.fall_distance = 0.0;
            }
            // ServerGamePacketListenerImpl.handlePlayerPositionChange ends the
            // impulse context on LivingEntity.hasLandedInLiquid: a move into a
            // liquid that does not rise, or a wind charge that lands in water
            // would waive the fall after it.
            let liquid = w.player.fluids.was_touching_water || w.player.is_in_lava();
            let delta_y = w.player.velocity.y;
            w.player.impulse.landed_in_liquid(delta_y, liquid);
            // Entity.applyEffectsFromBlocks along those moves: magma, cactus,
            // lit campfires and grown sweet berry bushes hurt (arismoko/horizon#1305).
            w.player.shift_key_down = sneaking;
            let _ = w.player.set_sleeping(sleeping);
            w.player.leather_boots = leather_boots;
            w.player.freeze_immune = freeze_immune;
            w.player.burn_from_stepping_immune = burn_immune;
            let _ = w.server_block_damage(start);
            let _ = w.update_soul_speed_block();
            let _ = w.tick_environment_damage();
            let _ = w.apply_fire_contact();
            let _ = w.server_hazards_tick(in_wall);
            // LivingEntity.aiStep: a Riptide spin counts down on the server's
            // copy too, or it would stay in the spin pose (eye 0.4) for good.
            // LivingEntity.tick: a changed scale rebuilds the box. Player.tick
            // ends with updatePlayerPose — or, for a passenger, the rideTick
            // path that clears a swim (arismoko/horizon#4509 / arismoko/horizon#3308).
            let _ = if riding {
                w.passenger_pose_tick()
            } else {
                w.server_player_pose_tick()
            };
            let mut events = std::mem::take(&mut w.player.events);
            if let Some((at, alive)) = rose {
                game.end_tracked_fall(at, alive);
            }
            // Powder snow and cauldrons put the fire out along those moves.
            game.block_extinguish_player(&events);
            // Entity.baseTick: rain puts out a burning player.
            game.rain_extinguish_player();
            // WitherRoseBlock.entityInside along those moves, and an
            // `entity_inside` part's typed effect writes.
            for event in &events {
                match event {
                    MotionEvent::InsideWitherRose => game.wither_rose_contact(),
                    MotionEvent::InsideEffect {
                        id,
                        duration,
                        amplifier,
                    } if game.remote.is_none() && game.ui.difficulty != 0 => {
                        game.give_player_effect(id, *duration, i32::from(*amplifier));
                    }
                    _ => {}
                }
            }
            // ServerGamePacketListenerImpl.handleMovePlayer:
            // Player.checkMovementStatistics for each move.
            for d in travelled {
                game.movement_stats(Vec3::new(d[0], d[1], d[2]));
            }
            // LivingEntity.baseTick: a new block position runs the worn
            // enchantments' location_changed effects (frost walker).
            game.location_changed_effects([start.x, start.y, start.z], from_ground);
            // LivingEntity.baseTick: outside the world border, then tickEffects.
            game.border_damage();
            game.tick_mob_effects();
            // Soul Speed's effects (LivingEntity.baseTick's location_changed
            // and tick effects); its particle goes to the players around.
            soul = game.tick_soul_speed();
            game.handle_shoulder_entities();
            let Some(player) = game.player.as_mut() else {
                return (Vec::new(), None);
            };
            let _ = player.peaceful_regeneration(&game.loop_context, rules);
            let _ = player.food_tick(
                &game.loop_context,
                crate::game::entity::damage_level_of!(game),
                health,
                rules,
            );
            events.append(&mut player.events);
            player.carry_client_movement(tick_movement);
            let _ = MotionWorld {
                entities: &[],
                player,
                scene: &mut game.world,
                assets: &game.pack.assets,
                collision_binding: game.collision,
                resources: game.motion,
                health: &game.health_resources,
                tag_items: Some(world_items!(game)),
                loop_context: &game.loop_context,
                damage_hooks: &game.damage_hooks,
                block_registries: Some(&game.pack.block_registries),
            }
            .server_travel();
            // LivingEntity.tick, after aiStep: the post-impulse grace counts
            // down, so a wind charge spares the moved-wrongly check and the
            // impulse reset for 40 ticks, not for good.
            player.impulse.tick_grace_time();
            // Player.aiStep's entity contacts run with the player's body,
            // even when the level and its mobs are frozen. Flush their damage
            // events and marked velocity through the existing guest path.
            game.touch_pufferfish();
            let Some(player) = game.player.as_mut() else {
                return (events, None);
            };
            // Landing damage from this travel reaches the tracker too.
            events.append(&mut player.events);
            // A hurt since the last tick marked the velocity (Entity.markHurt):
            // ServerEntity.sendChanges sends it as it is after this travel.
            let marked = std::mem::take(&mut player.health.sync_velocity);
            let velocity = marked.then_some(player.velocity);
            // Player.causeFallDamage: a fall of 2 blocks or more counts.
            for event in &events {
                if let MotionEvent::Fall(horizon_client::landing::FallEvent::Statistic(cm)) = event
                {
                    game.award_custom_stat("fall_one_cm", *cm);
                }
            }
            game.record_combat(&events);
            game.wear_armor(&events);
            (events, velocity)
        });
        let Some((events, marked)) = events else {
            return;
        };
        if let Some((at, velocity)) = soul
            && let Some(host) = self.lan.as_ref()
        {
            crate::game::lan_particles::send_soul_particle(host, at, velocity);
        }
        self.as_guest(conn, |game| {
            game.sculk_step_on_player();
            game.tick_player_sculk_events();
            // Entity.restituteMovementAfterCollisions: GameEvent.BOUNCE.
            for event in &events {
                if matches!(event, MotionEvent::Bounce { .. }) {
                    game.post_entity_event("minecraft:bounce", horizon_client::mobs::Who::Player);
                }
            }
        });
        for event in events {
            match event {
                MotionEvent::Health(horizon_client::health::HealthEvent::DamageBroadcast {
                    source,
                }) => self.guest_damage_broadcast(conn, source),
                // LivingEntity.playHurtSound: the server's `Entity.playSound`
                // reaches every player around but the hurt one, whose own
                // client plays it from handleDamageEvent (arismoko/horizon#4149).
                MotionEvent::HurtSound(sound) => self.guest_hurt_sound(conn, sound),
                // LivingEntity.checkFallDamage: the crumbs reach every player.
                MotionEvent::Fall(horizon_client::landing::FallEvent::Particles {
                    state,
                    position,
                    count,
                    speed,
                }) => self.landing_dust(state, position, count, speed),
                // A block's own sound names no player: the guest hears it too.
                MotionEvent::Fall(horizon_client::landing::FallEvent::Sound {
                    sound,
                    position: Some(pos),
                    volume,
                    pitch,
                }) if sound.block_sound().is_some() => {
                    if let Some(name) = sound.block_sound() {
                        let at = [
                            f64::from(pos.x) + 0.5,
                            f64::from(pos.y) + 0.5,
                            f64::from(pos.z) + 0.5,
                        ];
                        self.audio.play_unshared(name, Some(at), volume, pitch);
                        self.send_level_sound(LevelSound::new(name, at, volume, pitch), None);
                    }
                }
                // LivingEntity.causeFallDamage, and HoneyBlock.fallOn's
                // `entity.playSound`: Player.playSound skips the guest, whose
                // own client plays it; everyone else hears it.
                MotionEvent::Fall(horizon_client::landing::FallEvent::Sound {
                    sound,
                    volume,
                    pitch,
                    ..
                }) => {
                    if let Some(name) = sound.player_fall_sound() {
                        self.audio.play_unshared(name, Some(at), volume, pitch);
                        self.send_level_sound(LevelSound::new(name, at, volume, pitch), Some(conn));
                    }
                }
                // Player.causeFallDamage, mayfly: the mods hear the landing.
                MotionEvent::Fall(horizon_client::landing::FallEvent::FlyableFall {
                    distance,
                    multiplier,
                }) => {
                    if let Some((uuid, id, name)) = self
                        .lan
                        .as_ref()
                        .and_then(|host| host.hub.guest(conn))
                        .map(|g| (g.uuid, g.entity_id, g.name.clone()))
                    {
                        self.as_guest(conn, |game| {
                            game.native_player_flyable_fall(uuid, id, &name, distance, multiplier);
                        });
                    }
                }
                // HoneyBlock.fallOn: entity event 54, including the player who
                // landed (`broadcastEntityEvent` has no excluded player).
                MotionEvent::Fall(horizon_client::landing::FallEvent::Broadcast(event))
                    if horizon_client::landing::honey_particle_count(event as i8).is_some() =>
                {
                    if let Some(host) = self.lan.as_ref()
                        && let Some(id) = host.hub.guest(conn).map(|g| g.entity_id)
                    {
                        host.hub.entity_event(id, event as i8);
                    }
                }
                MotionEvent::Fall(horizon_client::landing::FallEvent::BlockFallSound {
                    position,
                    state,
                }) => {
                    let entity = self
                        .lan
                        .as_ref()
                        .and_then(|host| host.hub.guest(conn))
                        .map(|g| (g.entity_id as u32, "minecraft:player"));
                    let sound = self.block_sound_type(
                        [position.x, position.y, position.z],
                        state,
                        entity,
                        mod_api::BlockSoundCause::Fall,
                    );
                    self.audio.play_unshared(
                        &sound.fall,
                        Some(at),
                        sound.volume * 0.5,
                        sound.pitch * 0.75,
                    );
                    self.send_level_sound(
                        // `Player.playSound` plays in the players' source.
                        LevelSound::new(&sound.fall, at, sound.volume * 0.5, sound.pitch * 0.75)
                            .in_source(mod_api::SoundSource::Players),
                        Some(conn),
                    );
                }
                MotionEvent::Fall(event) => {
                    let player = self
                        .lan
                        .as_ref()
                        .and_then(|host| host.hub.guest(conn))
                        .map(|g| g.uuid);
                    // The rule's own game event goes out as the player whose
                    // landing it was (`FarmlandBlock.turnToBaseBlock`'s change,
                    // `TurtleEggBlock.decreaseEggs`' destroy): a sculk sensor, a
                    // calibrated sensor and a warden hear it.
                    if let Some(vibration) = self.landing_block_updates(player, &event) {
                        self.as_guest(conn, |game| game.post_landing_vibration(vibration));
                    }
                }
                // PowderSnowBlock.entityInside: every player around sees the
                // snowflake (`ServerLevel.sendParticles`).
                MotionEvent::Snowflake { position, velocity } => {
                    if let Some(host) = self.lan.as_ref() {
                        crate::game::lan_particles::send_snowflake_particle(
                            host,
                            [position.x, position.y, position.z],
                            [velocity.x, velocity.y, velocity.z],
                        );
                    }
                }
                // LivingEntity.baseTick's entity event 67, to every client
                // tracking the guest and its own; this window draws it too.
                // A mod's other bubble count goes out as particles instead.
                MotionEvent::Drown(bubbles) => {
                    if !self.drown_bubbles(at, [0.0; 3], bubbles) {
                        continue;
                    }
                    if let Some(host) = self.lan.as_ref()
                        && let Some(g) = host.hub.guest(conn)
                    {
                        host.hub
                            .entity_event(g.entity_id, crate::game::combat::DROWN_EVENT);
                    }
                    self.drown_particles(at, [0.0; 3], 8);
                }
                // ServerPlayer.onInsideBlock: the enter_block trigger, for the
                // player who moved.
                MotionEvent::EnterBlock { state } => {
                    self.as_guest(conn, |game| game.trigger_enter_block(state));
                }
                // HoneyBlock.maybeDoSlideEffects: the sound reaches the
                // others (Player.playSound excludes the player, whose client
                // plays it), the entity event 53 everyone around.
                MotionEvent::HoneySlide {
                    pos,
                    sound,
                    particles,
                } => {
                    if self.world.game_time % 20 == 0 {
                        self.as_guest(conn, |game| game.trigger_honey_slide([pos.x, pos.y, pos.z]));
                    }
                    let Some((id, at)) = self
                        .lan
                        .as_ref()
                        .and_then(|host| host.hub.guest(conn))
                        .map(|g| (g.entity_id, g.position))
                    else {
                        continue;
                    };
                    if sound {
                        let name = "minecraft:block.honey_block.slide";
                        self.audio.play_unshared(name, Some(at), 1.0, 1.0);
                        self.send_level_sound(LevelSound::new(name, at, 1.0, 1.0), Some(conn));
                    }
                    if particles {
                        self.honey_slide_particles(at);
                        if let Some(host) = self.lan.as_ref() {
                            host.hub.entity_event(id, 53);
                        }
                    }
                }
                _ => {}
            }
        }
        if let Some(v) = marked {
            if mod_api::__private::listened(mod_api::HookId::PlayerVelocity)
                && let Some((uuid, name)) = self
                    .lan
                    .as_ref()
                    .and_then(|host| host.hub.guest(conn))
                    .map(|g| (g.uuid, g.name.clone()))
            {
                self.native_player_velocity(uuid, &name, [v.x, v.y, v.z]);
            }
            if let Some(g) = self.lan.as_ref().and_then(|host| host.hub.guest(conn)) {
                g.send(&C::SetEntityMotion(play::SetEntityMotion {
                    id: g.entity_id,
                    velocity: [v.x, v.y, v.z],
                }));
            }
        }
    }
}

impl Game<'_> {
    /// `ServerPlayer.resetFallDistance` for a move that went up: the tracked fall ends, and
    /// `fall_from_height` measures it from where it began to `at`, where that move ended. A
    /// later move of the same tick may have taken the player further. `alive` is the player's
    /// state at that move (`getHealth() > 0`): a death later in the tick does not undo it.
    fn end_tracked_fall(&mut self, at: Vec3, alive: bool) {
        let Some(start) = self.adv.marks.fall.take() else {
            return;
        };
        if !alive {
            return;
        }
        let Some(player) = self.player.as_mut() else {
            return;
        };
        let end = std::mem::replace(&mut player.position, at);
        self.trigger_distance("minecraft:fall_from_height", start);
        if let Some(player) = self.player.as_mut() {
            player.position = end;
        }
    }
}

#[cfg(test)]
#[path = "guest_body/tests.rs"]
mod tests;
