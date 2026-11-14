use super::*;

impl Game<'_> {
    pub(in crate::game) fn guest_respawn(&mut self, conn: u32) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let Some(i) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        // The guest's own bed, else the world spawn: never the host's bed.
        let world = self.world_spawn_position();
        let spawn = host.hub.guests[i].spawn.unwrap_or(world);
        if !host.hub.guests[i].dead {
            return;
        }
        let (uuid, entity_id) = (host.hub.guests[i].uuid, host.hub.guests[i].entity_id);
        let original = self.capture_original_player(conn);
        // PlayerList.respawn replaces the player, retaining only opted-in data.
        if let Some(attachments) = self
            .lan
            .as_mut()
            .and_then(|host| host.hub.guests.attachments_mut(conn))
            && let Ok(copy) = attachments.copy_on_death()
        {
            attachments.replace_contents(copy);
        }
        // NeoForge's clone keeps only the `PlayerPersisted` part of a mod's saved compound.
        self.keep_only_player_persisted(conn);
        self.reinitialize_server_player(uuid, conn);
        // The new `ServerPlayer` is filed where it is placed, untold.
        self.entity_section_forget(entity_id as u32);
        // PlayerList.respawn removes the dead player (Player.remove).
        self.guest_drop_carried(i);
        // AbstractContainerScreen.tick / ServerPlayer.closeContainer: the dead
        // player's open container goes with it, so a chest's lid (its opener
        // count) falls instead of staying up for a player who is gone.
        self.guest_close_menus_and_tell(conn);
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        // PlayerList.respawn builds a new `ServerPlayer`, whose `lastEquipmentItems` starts
        // empty: the mods hear each worn item again from empty (`native_players_equipment`).
        if let Some(play) = host.play.get_mut(&conn) {
            play.equipment_heard = None;
            play.inventory_heard = None;
            play.level_up_sound = Default::default();
        }
        // PlayerList.respawn: a server's player comes back in its respawn
        // point's level: the Overworld, where the world spawn and beds are,
        // or another one's anchor or forced point, which its block resolves
        // once the level has loaded around it.
        let server = self.options.dedicated.is_some();
        let own = host.hub.guests[i]
            .respawn_elsewhere
            .as_ref()
            .map(RespawnConfig::from_saved);
        let elsewhere = own.filter(|_| server);
        // In the window's world a guest has no level of its own to wait in:
        // its bed is resolved at once when its chunk is there, else it
        // comes back on the bed.
        let in_frame_bed = own.filter(|c| !server && c.dim == crate::game::Dim::Overworld);
        let spawn = in_frame_bed.map_or(spawn, |c| {
            [
                f64::from(c.pos[0]) + 0.5,
                f64::from(c.pos[1]) + 0.6,
                f64::from(c.pos[2]) + 0.5,
            ]
        });
        let level = elsewhere.map_or(crate::game::Dim::Overworld, |c| c.dim);
        let spawn = elsewhere.map_or(spawn, |c| {
            [
                f64::from(c.pos[0]) + 0.5,
                f64::from(c.pos[1]),
                f64::from(c.pos[2]) + 0.5,
            ]
        });
        // PlayerRespawnPositionEvent: an Overworld forced point needs no
        // block, so it is final now, before the Respawn packet goes out, and
        // a mod's other level or point costs no extra one. A bed or anchor,
        // or the world spawn's column, is final only once its trip arrives
        // (`arrive_server_trips`).
        let forced_spawn = host.hub.guests[i].spawn;
        let point_level = level;
        // The fresh player faces where `hub.respawn` turns it by default,
        // not the way the dead one faced.
        let found = crate::game::respawn_position::Respawn {
            dim: level,
            at: horizon_core::geometry::Vec3::new(spawn[0], spawn[1], spawn[2]),
            yaw: 0.0,
            pitch: 0.0,
        };
        let decision = (server && elsewhere.is_none() && forced_spawn.is_some())
            .then(|| self.respawn_position(conn, found, None))
            .flatten();
        let keep_point = decision.is_none_or(|d| d.copy);
        let (level, spawn) = match decision {
            Some(d) => (d.to.dim, [d.to.at.x, d.to.at.y, d.to.at.z]),
            None => (level, spawn),
        };
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let rotation = decision.map(|d| [d.to.yaw, d.to.pitch]);
        let back = self.dim;
        let left = crate::game::Dim::from_id(&host.hub.guests[i].dimension);
        let away = server && host.hub.guests[i].dimension != level.id();
        if server {
            self.server_trips.retain(|t| !t.is(conn));
        }
        if away && let Err(e) = self.enter_level(level) {
            log::error!("could not open {}: {e}", level.id());
            return;
        }
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let (cloned, name) = host.hub.guests.update(i, |g| {
            // restartClientLoadTimerAfterRespawn.
            g.dead = false;
            g.load_timeout = horizon_client::lan::guest::CLIENT_LOADED_TIMEOUT;
            g.loading = true;
            g.death_time = 0;
            g.food = 20;
            g.saturation = 5.0;
            g.sent_health = None;
            // ServerPlayer.restoreFrom(old, false): the base values come back,
            // and the new player has the max health they give.
            if let Some(m) = g.new_body(false) {
                let m = *m;
                g.logs_kept = Some((m.hurt_log, m.attack_log));
            }
            g.health = g.respawn_health(self.motion);
            let mut xp = Experience {
                level: g.xp_level,
                progress: g.xp_progress,
                total: g.xp_total,
            };
            let keep_inventory = self
                .ui
                .rules
                .bool_at(horizon_client::gamerules::vanilla::KEEP_INVENTORY)
                || g.game_mode == SPECTATOR;
            xp.respawn(keep_inventory);
            g.effects = Default::default();
            (g.xp_level, g.xp_progress, g.xp_total) = (xp.level, xp.progress, xp.total);
            // PlayerEvent.Clone, at the end of `restoreFrom`: the data is copied,
            // the new player not yet placed.
            (g.uuid, g.name.clone())
        });
        if let Some(original) = original {
            self.with_original_player(original, |game, original| {
                game.native_player_clone(cloned, &name, true, original);
            });
        }
        let clock_rate = self.network_clock_rate();
        let ctx = TickCtx {
            world: &mut self.world,
            entities: &[],
            next_entity: &mut self.next_entity,
            // PlayerList.respawn / sendLevelInfo send level respawn data,
            // independently of the personal destination passed to Hub::respawn.
            spawn: world,
            spawn_angle: self.level.world_spawn_angle,
            sea_level: self.end.locator().map_or(63, |terrain| terrain.sea_level()),
            difficulty: i32::from(self.ui.difficulty),
            difficulty_locked: self.level.difficulty_locked,
            clock_rate,
            tick_rate: self.clock.rate(),
            tick_frozen: self.clock.frozen(),
            tick_steps: self.clock.step_state(),
            collision: Some(self.collision),
            border: &self.border,
            streaming: Some(&self.streaming),
        };
        if let Some(host) = self.lan.as_mut() {
            host.hub.respawn(i, spawn, rotation, &ctx);
        }
        self.bind_player_attachments(conn, false);
        // ServerGamePacketListenerImpl.handleClientCommand: a hardcore world's
        // respawned player only watches.
        if self.level.hardcore {
            let (uuid, name, from) = {
                let Some(host) = self.lan.as_ref() else {
                    return;
                };
                let g = &host.hub.guests[i];
                (
                    g.uuid,
                    g.name.clone(),
                    crate::game::game_mode::mode_from_id(g.game_mode),
                )
            };
            let (mut cancel, mut to) = (false, GameMode::Spectator);
            if from != GameMode::Spectator {
                self.native_player_game_mode_changed(uuid, &name, from, &mut to, &mut cancel);
                // A redirect to the mode the player has is no change: nothing is sent.
                cancel |= to == from;
            }
            if !cancel && let Some(host) = self.lan.as_mut() {
                host.hub
                    .set_game_mode(conn, to.id(), &self.world, Some(self.collision));
            }
        }
        // PlayerList.respawn: `copyRespawnPosition` hands the new player the
        // old point through `setRespawnPosition`: none, or an Overworld forced
        // point (`spawn`). A point with a block is copied once the block is
        // found. A cancel leaves the new player with none; it still comes
        // back here this time, as vanilla placed it before the copy.
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        if !keep_point {
            // copyOriginalSpawnPosition false: the fresh player has none.
            {
                let value = None;
                host.hub.guests.update(i, |guest| guest.spawn = value);
            }
        } else if let Some(point) = copied_respawn_point(own.is_some(), forced_spawn) {
            let g = &host.hub.guests[i];
            let (uuid, id, name) = (g.uuid, g.entity_id, g.name.clone());
            let mut cancel = false;
            self.native_player_set_spawn(mod_api::__private::PlayerSetSpawnCall {
                player: uuid,
                player_entity_id: id,
                name: &name,
                dimension: point_level.id(),
                pos: point,
                forced: point.is_some(),
                cancel: &mut cancel,
            });
            if cancel && let Some(host) = self.lan.as_mut() {
                {
                    let value = None;
                    host.hub.guests.update(i, |guest| guest.spawn = value);
                }
            }
        }
        if let Some(config) = elsewhere {
            self.respawn_trip(conn, config);
        } else if in_frame_bed.is_none() && forced_spawn.is_none() {
            // TeleportTransition.createDefault: no point of its own, so the
            // world spawn's free column once its chunks have loaded.
            self.world_spawn_trip(conn, None);
        }
        if let Some(c) = in_frame_bed
            && self.dim == crate::game::Dim::Overworld
            && self.world.loaded(c.pos[0], c.pos[2])
        {
            let found = self
                .as_guest(conn, |g| g.resolve_server_respawn(c))
                .flatten();
            let world = self
                .level
                .world_spawn
                .map_or([0.5, 80.0, 0.5], |[x, y, z]| {
                    [f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5]
                });
            let Some(host) = self.lan.as_mut() else {
                return;
            };
            let at = found.map_or(world, |(at, yaw, pitch)| {
                {
                    let value = yaw;
                    host.hub.guests.update(i, |guest| {
                        guest.yaw = value;
                        guest.pitch = pitch;
                    });
                }
                [at.x, at.y, at.z]
            });
            if found.is_none() {
                // TeleportTransition.missingRespawnBlock.
                host.hub.missing_respawn_block(conn);
                self.world_spawn_trip(conn, None);
            }
            if let Some(host) = self.lan.as_mut() {
                host.hub.teleport(i, Some(at));
            }
        }
        // PlayerRespawnEvent, at the end of PlayerList.respawn: the player is
        // placed, in their respawn level, with their state swapped in so the
        // helpers reach them (as for native_run_mod_command).
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let (respawned, name) = (host.hub.guests[i].uuid, host.hub.guests[i].name.clone());
        self.as_guest(conn, |g| g.native_player_respawned(respawned, &name));
        // The player left another level for the respawn point's: placed for
        // good they tell the mods now, else the trip that is still carrying
        // them (a bed or anchor in another level, the world spawn's free
        // column) does when it arrives (`arrive_server_trips`).
        if away {
            if let Some(trip) = self.server_trips.iter_mut().find(|t| t.is(conn)) {
                trip.left = Some(left);
            } else {
                self.tell_mods_changed_dimension(conn, Some(left), level);
            }
        }
        if away && let Err(e) = self.enter_level(back) {
            log::error!("could not reopen {}: {e}", back.id());
        }
    }

    /// `ServerPlayer.die` for a guest whose health has run out: the death
    /// message, the loot and experience, the death screen.
    pub(in crate::game) fn guest_die(&mut self, gi: usize, conn: u32) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let dying = host.hub.guests[gi].health <= 0.0 && !host.hub.guests[gi].dead;
        // The killing blow, for the death hooks.
        let blow = (dying && !mod_api::__private::mods().is_empty())
            .then(|| self.guest_damage_source(conn));
        if dying && self.guest_death_canceled(gi, conn) {
            return;
        }
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        if host.hub.guests[gi].health <= 0.0 && !host.hub.guests[gi].dead {
            self.guest_forget_digging(conn);
            // FishingHook.shouldStopFishing: its owner is dead.
            if let Some(play) = self.lan.as_mut().and_then(|host| host.play.get_mut(&conn)) {
                play.hook.discard();
            }
        }
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let here = host.hub.here(&host.hub.guests[gi]);
        let g = &host.hub.guests[gi];
        if g.health <= 0.0 && !g.dead {
            host.hub.guests.update(gi, |g| {
                g.dead = true;
                g.last_death = Some((g.dimension.clone(), g.position.map(|v| v.floor() as i32)));
            });
            let g = &host.hub.guests[gi];
            let (id, at) = (g.entity_id, g.position);
            let reward = Experience {
                level: g.xp_level,
                progress: g.xp_progress,
                total: g.xp_total,
            }
            .death_reward(
                self.ui
                    .rules
                    .bool_at(horizon_client::gamerules::vanilla::KEEP_INVENTORY)
                    || g.game_mode == SPECTATOR,
            );
            let (name, uuid) = (g.name.clone(), g.uuid);
            // ServerPlayer.die: dropAllDeathLoot unless a spectator, from the eyes less 0.3.
            let drops = (g.game_mode != SPECTATOR)
                .then(|| [at[0], at[1] + g.eye_height() - f64::from(0.3f32), at[2]]);
            let cause = host.play.get_mut(&conn).and_then(|p| p.death_cause.take());
            // ServerPlayer.die opens with `gameEvent(ENTITY_DIE)` at the player:
            // sculk sensors and wardens hear the death.
            if here {
                self.as_guest(conn, |game| {
                    let ctx = game.player_context(None);
                    game.post_game_event("minecraft:entity_die", at, ctx);
                });
            }
            // `getKillCredit`, as the death finds it.
            let killer = self.guest_kill_credit_type(conn);
            self.native_player_died(uuid, cause.as_deref());
            // ServerPlayer.die: the neutral mobs near a guest in this level
            // forget their anger at them.
            if here {
                self.tell_neutral_mobs_player_died(at, Some(conn));
            }
            // The death's last blow, read before the death message empties the
            // combat record: ServerPlayer.die's triggers read it from a record
            // that CombatTracker.recheckStatus clears only at the end of die.
            let killing = self.guest_combat.get(&conn).and_then(|c| c.killing_blow());
            // ServerPlayer.die: with showDeathMessages, the message goes to
            // the dying player's death screen and to everyone the team lets
            // see it; without it, the death screen gets an empty one.
            let message = if self
                .ui
                .rules
                .bool_at(horizon_client::gamerules::vanilla::SHOW_DEATH_MESSAGES)
            {
                let me = self.player_text(&name, uuid);
                let text = match cause {
                    Some(key) => horizon_client::text::Text::translate(&key, &[me]),
                    None if at[1] < self.world.below_world_y() => {
                        horizon_client::text::Text::translate("death.attack.outOfWorld", &[me])
                    }
                    None => self.guest_death_message(conn, me),
                };
                let message = horizon_client::lan::data::component_nbt(&text.component());
                let board = &self.score.board;
                let Some(host) = self.lan.as_ref() else {
                    return;
                };
                host.hub.system_message_to(message.clone(), |other| {
                    horizon_client::scoreboard::death_message_reaches(board, &name, other)
                });
                if self.is_dedicated_server() {
                    crate::game::dedicated::events::record(
                        "death",
                        &name,
                        uuid,
                        serde_json::json!({ "message": text.plain() }),
                    );
                    // Only a message for everyone is logged (`broadcastSystemMessage`).
                    if board.players_team(&name).is_none_or(|t| {
                        t.death_message_visibility == horizon_client::scoreboard::Visibility::Always
                    }) {
                        crate::game::dedicated::log(
                            "INFO",
                            &format!("System chat: {}", text.plain()),
                        );
                    }
                } else if self.player.is_some()
                    && horizon_client::scoreboard::death_message_reaches(
                        board,
                        &name,
                        crate::game::chat::PLAYER_NAME,
                    )
                {
                    self.system_message(text);
                }
                message
            } else {
                literal("")
            };
            let Some(host) = self.lan.as_ref() else {
                return;
            };
            host.hub.send_to(
                conn,
                &C::PlayerCombatKill(play::PlayerCombatKill { id, message }),
            );
            // LivingEntity.die: the death animation, the dying player included.
            host.hub.entity_event(id, 3);
            let recently_hit = killing.is_some()
                && self
                    .guest_combat
                    .get(&conn)
                    .is_some_and(|c| c.recently_hit_by_player(self.world.game_time));
            let dropping = crate::game::creatures::DyingDrops {
                entity: id as u32,
                kind: "minecraft:player",
                player: Some(uuid),
                source: killing.as_ref().map(|(damage_type, attacker)| {
                    crate::game::creatures::death_hurt_source(damage_type, attacker.unwrap_or(0))
                }),
                recently_hit,
            };
            // LivingEntity.die: the death is heard, and a sculk catalyst
            // nearby takes the experience (before the loot is dropped), as it
            // does for a mob.
            let mut catalyst_xp = 0;
            let consumed = self.catalyst_takes_experience(at, |_| {
                catalyst_xp = reward;
                catalyst_xp
            });
            // CatalystListener: with experience to take, the player who last
            // hurt them gets KILL_MOB_NEAR_SCULK_CATALYST.
            if consumed && catalyst_xp > 0 {
                self.guest_catalyst_trigger(gi, conn, at);
            }
            // ServerPlayer.die: the drops, then the deathCount objectives, then
            // the `deaths` statistic and the two timers it resets.
            self.as_guest(conn, |game| {
                if let Some(eyes) = drops {
                    game.drop_inventory(eyes, &dropping);
                }
                game.score_player_died();
                if let Some(kind) = &killer {
                    game.award_stat("killed_by", kind, 1);
                }
            });
            self.guest_killed_player_trigger(conn, at, killing);
            self.as_guest(conn, |game| {
                game.award_custom_stat("deaths", 1);
                game.stats.reset_death_timers();
            });
            // ServerPlayer.die: a spectator never reaches dropAllDeathLoot.
            if drops.is_some() {
                // LivingEntity.dropExperience: Player.isAlwaysExperienceDropper.
                let now = self.world.game_time;
                let attacker = if mod_api::__private::mods().is_empty() {
                    None
                } else {
                    self.guest_combat
                        .get_mut(&conn)
                        .and_then(|c| c.take_hurt_by_player(now))
                        .and_then(|c| self.resolve_player_credit(c).map(|_| c.0))
                };
                let reward = self.living_experience_drop(
                    id as u32,
                    "minecraft:player",
                    attacker,
                    reward as i32,
                );
                if !consumed {
                    self.spawn_orbs(at, reward as u32);
                }
            }
            // `ServerPlayer.die`'s `broadcastEntityEvent`, after the message,
            // the loot and the experience: Fabric's AFTER_DEATH. The blow is
            // the one `living_death` heard, so a death it cancelled returns
            // above and is never told here.
            if let Some((damage_type, attacker)) = &blow
                && mod_api::__private::listened(mod_api::HookId::LivingDeathPost)
            {
                self.native_living_death_post(id as u32, damage_type, *attacker);
            }
        }
    }

    /// `SculkCatalystBlockEntity.CatalystListener`'s trigger on a player's
    /// death: for the player the dying guest's combat record credits
    /// (`LivingEntity.getKillCredit`), with the death's `killing_blow`.
    pub(super) fn guest_catalyst_trigger(&mut self, gi: usize, conn: u32, at: [f64; 3]) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let now = self.world.game_time;
        let combat = self.guest_combat.get(&conn);
        let player_credit = combat.and_then(|c| c.player_credit(now));
        let who = if let Some(credit) = player_credit {
            self.resolve_player_credit(credit)
        } else {
            combat.and_then(|c| c.kill_credit(now)).and_then(|id| {
                if self.lan_acting.is_none()
                    && self
                        .player
                        .as_ref()
                        .is_some_and(|p| p.entity_id as u32 == id)
                {
                    Some(None)
                } else {
                    host.hub
                        .guests
                        .iter()
                        .find(|g| g.entity_id as u32 == id)
                        .map(|g| Some(g.conn))
                }
            })
        };
        let Some(who) = who else { return };
        let (damage, attacker) = combat
            .and_then(|c| c.killing_blow())
            .unwrap_or_else(|| ("minecraft:player_attack".into(), None));
        let g = &host.hub.guests[gi];
        let view = horizon_client::advancements::EntityView {
            kind: "minecraft:player".into(),
            position: at,
            sneaking: g.sneaking,
            input_sneak: g.input & 32 != 0,
            gamemode: Some(
                match g.game_mode {
                    1 => "creative",
                    2 => "adventure",
                    3 => "spectator",
                    _ => "survival",
                }
                .into(),
            ),
            ..Default::default()
        };
        // The damage's causing entity: the player in place, another guest, or
        // a mob (`DamageSource.getEntity`).
        let source = attacker.and_then(|id| {
            if self.lan_acting.is_none()
                && self
                    .player
                    .as_ref()
                    .is_some_and(|p| p.entity_id as u32 == id)
            {
                Some(self.player_entity_for_any(&["minecraft:kill_mob_near_sculk_catalyst"]))
            } else {
                host.hub
                    .guests
                    .iter()
                    .find(|g| g.entity_id as u32 == id)
                    .map(|g| horizon_client::advancements::EntityView {
                        kind: "minecraft:player".into(),
                        position: g.position,
                        sneaking: g.sneaking,
                        input_sneak: g.input & 32 != 0,
                        ..Default::default()
                    })
                    .or_else(|| {
                        self.mobs
                            .iter()
                            .find(|m| m.id == id)
                            .map(|m| self.mob_entity(m))
                    })
            }
        });
        let projectile = match damage.as_str() {
            "minecraft:arrow" => Some("minecraft:arrow"),
            "minecraft:fireball" | "minecraft:small_fireball" => Some("minecraft:fireball"),
            "minecraft:wither_skull" => Some("minecraft:wither_skull"),
            "minecraft:fireworks" => Some("minecraft:firework_rocket"),
            _ => None,
        };
        let blow = horizon_client::advancements::DamageView {
            kind: damage,
            // DamageSource.isDirect: the causing entity is the direct one.
            is_direct: projectile.is_none(),
            direct: match projectile {
                Some(kind) => Some(horizon_client::advancements::EntityView {
                    kind: kind.into(),
                    position: at,
                    ..Default::default()
                }),
                None => source.clone(),
            },
            source,
            ..Default::default()
        };
        // CatalystListener: trigger(player, killedEntity, damageSource).
        match who {
            None => {
                self.trigger_killed_as("minecraft:kill_mob_near_sculk_catalyst", &view, Some(&blow))
            }
            Some(kconn) => {
                self.as_guest(kconn, |g| {
                    g.trigger_killed_as(
                        "minecraft:kill_mob_near_sculk_catalyst",
                        &view,
                        Some(&blow),
                    );
                });
            }
        }
    }

    /// `ServerPlayer.die` → `Entity.awardKillScore` on the entity the dying
    /// guest's combat record credits (`LivingEntity.getKillCredit`): the
    /// guest's `entity_killed_player`, with that entity and the death's
    /// `killing_blow`. Nothing credited, nothing fires.
    pub(super) fn guest_killed_player_trigger(
        &mut self,
        conn: u32,
        at: [f64; 3],
        blow: Option<(String, Option<u32>)>,
    ) {
        const TRIGGER: &str = "minecraft:entity_killed_player";
        let now = self.world.game_time;
        let Some(id) = self
            .guest_combat
            .get(&conn)
            .and_then(|c| c.kill_credit(now))
        else {
            return;
        };
        let Some(killer) = self.guest_entity_view(id, TRIGGER) else {
            return;
        };
        let blow = self.guest_killing_blow(blow, at, TRIGGER);
        self.as_guest(conn, |g| {
            g.trigger_killed_as(TRIGGER, &killer, blow.as_ref());
        });
    }

    /// The type of the entity a dying guest's combat record credits with the
    /// kill (`LivingEntity.getKillCredit`): a player (the one in place or a
    /// guest), a mob (one gone already credits nobody, as vanilla's reference
    /// no longer resolves), a mob whose own blast this tick killed the guest,
    /// or the ender dragon.
    fn guest_kill_credit_type(&self, conn: u32) -> Option<String> {
        let id = self
            .guest_combat
            .get(&conn)?
            .kill_credit(self.world.game_time)?;
        let in_place = self.lan_acting.is_none()
            && self
                .player
                .as_ref()
                .is_some_and(|p| p.entity_id as u32 == id);
        if in_place
            || self
                .lan
                .as_ref()
                .is_some_and(|host| host.hub.guests.iter().any(|g| g.entity_id as u32 == id))
        {
            return Some("minecraft:player".into());
        }
        if let Some(m) = self.mobs.iter().find(|m| m.id == id) {
            return Some(m.kind.id.clone());
        }
        // A mob that blew up this tick has left the list, but vanilla's player
        // dies inside that blast, before the source is discarded.
        if let Some((_, kind)) = self.combat.exploded_kinds.iter().rfind(|(m, _)| *m == id) {
            return Some(kind.clone());
        }
        self.dragon
            .as_ref()
            .filter(|d| d.id == id)
            .map(|_| "minecraft:ender_dragon".into())
    }

    /// The dying guest's last blow (`LivingEntity.getLastDamageSource`), its
    /// damage type and causing entity as the combat record held them, as a
    /// damage source: the projectile as the direct entity.
    pub(super) fn guest_killing_blow(
        &self,
        blow: Option<(String, Option<u32>)>,
        at: [f64; 3],
        trigger: &str,
    ) -> Option<horizon_client::advancements::DamageView> {
        let (damage, attacker) = blow?;
        // The damage's causing entity: the player in place, another guest, or
        // a mob (`DamageSource.getEntity`).
        let source = attacker.and_then(|id| self.guest_entity_view(id, trigger));
        Some(horizon_client::advancements::recorded_blow(
            damage, source, at,
        ))
    }

    /// The entity behind a combat record's id, as `trigger`'s predicates see
    /// it: the player in place, a guest, or a mob.
    pub(super) fn guest_entity_view(
        &self,
        id: u32,
        trigger: &str,
    ) -> Option<horizon_client::advancements::EntityView> {
        if self.lan_acting.is_none()
            && self
                .player
                .as_ref()
                .is_some_and(|p| p.entity_id as u32 == id)
        {
            // `player_entity_for` takes `&mut self` (the compound for `nbt`);
            // this view of the killer has the player's records without it.
            return Some(self.player_entity_for_any(&[trigger]));
        }
        self.lan
            .as_ref()?
            .hub
            .guests
            .iter()
            .find(|g| g.entity_id as u32 == id)
            .map(|g| horizon_client::advancements::EntityView {
                kind: "minecraft:player".into(),
                position: g.position,
                sneaking: g.sneaking,
                input_sneak: g.input & 32 != 0,
                ..Default::default()
            })
            .or_else(|| {
                self.mobs
                    .iter()
                    .find(|m| m.id == id)
                    .map(|m| self.mob_entity(m))
            })
    }

    /// The scalar death-tail/death-message source. `living_death` uses the
    /// full accepted hit retained on the body instead.
    pub(super) fn guest_damage_source(&self, conn: u32) -> (String, u32) {
        let (damage_type, attacker) = match self
            .lan
            .as_ref()
            .and_then(|host| host.play.get(&conn))
            .and_then(|p| p.death_cause.as_deref())
        {
            Some("death.attack.genericKill") => ("minecraft:generic_kill", 0),
            _ => self
                .guest_combat
                .get(&conn)
                .and_then(|tracker| tracker.last_blow())
                .unwrap_or(("minecraft:generic", 0)),
        };
        (damage_type.to_owned(), attacker)
    }

    /// `ServerPlayer.die`'s message from a guest's own `CombatTracker`, which
    /// sits in `guest_combat` between turns.
    pub(super) fn guest_death_message(
        &mut self,
        conn: u32,
        me: horizon_client::text::Text,
    ) -> horizon_client::text::Text {
        let mut tracker = self.guest_combat.remove(&conn).unwrap_or_default();
        std::mem::swap(&mut self.combat, &mut tracker);
        let text = self.combat_death_message_for(me);
        std::mem::swap(&mut self.combat, &mut tracker);
        self.guest_combat.insert(conn, tracker);
        text
    }
}
