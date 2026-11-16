use super::*;

impl Game<'_> {
    /// Per tick, for every guest: dig, use items, pick up drops, die.
    pub(in crate::game) fn tick_guests(&mut self) {
        building_cache(self.lan.as_ref());
        self.tick_guests_inner();
        building_cache(None);
        self.pickup_columns = None;
    }

    fn tick_guests_inner(&mut self) {
        // `EntityEvent.Size` for the poses the guests' moves set since the last tick.
        self.native_guest_sizes();
        self.tick_guest_cameras();
        self.lan_merchant_tick();
        self.lan_stations_tick();
        self.lan_tables_tick();
        self.lan_mod_crafting_tick();
        self.lan_menu_links_tick();
        self.lan_containers_tick();
        self.tick_lan_mount_menus();
        self.native_players_equipment();
        self.announce_guest_advancements();
        // Entity.commonTick belongs to each player, including spectators and
        // players whose client has not loaded yet, not to the server clock.
        if let Some(host) = self.lan.as_mut() {
            for g in &host.hub.guests {
                if g.participates_in_player_simulation()
                    && host.hub.here(g)
                    && !self.server_trips.iter().any(|t| t.is(g.conn))
                {
                    host.play.entry(g.conn).or_default().level_up_sound.tick();
                }
            }
        }
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let conns: Vec<u32> = host
            .hub
            .guests
            .iter()
            // hasClientLoaded gates incoming actions, not ServerPlayer.doTick.
            .filter(|g| host.hub.here(g))
            .map(|g| g.conn)
            .collect();
        // The players' pickups ask which columns items reach before
        // scanning every item: most players stand on none.
        self.pickup_columns = Some(Default::default());
        // The players the guests' hooks can catch.
        if host.play.values().any(|p| p.hook.is_some()) {
            self.refresh_hook_targets();
        }
        for conn in conns {
            let Some(host) = self.lan.as_mut() else {
                return;
            };
            let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
                continue;
            };
            // ServerGamePacketListenerImpl.tick: the chat and command throttlers
            // decay one a tick, dead or alive.
            let play = host.play.entry(conn).or_default();
            play.chat_spam = play.chat_spam.saturating_sub(1);
            play.command_spam = play.command_spam.saturating_sub(1);
            play.recipe_checks = play.recipe_checks.saturating_sub(1);
            // commandSuggestionsProvider.tick: one answer a tick, to the latest request.
            play.suggestion_budget = true;
            self.guest_try_suggestions(conn);
            self.guest_player_tick(gi, conn, mod_api::HookId::PlayerTickPre, |game, player| {
                game.native_player_tick_pre(mod_api::__private::PlayerTickPreCall { player })
            });
            self.guest_sleep_tick(gi);
            let Some(host) = self.lan.as_mut() else {
                return;
            };
            if host.hub.guests[gi].dead
                || (host.hub.guests[gi].health <= 0.0 && host.hub.guests[gi].death_canceled)
            {
                self.guest_tick_death(gi);
                if let Some(host) = self.lan.as_ref() {
                    refresh_building_player(host, conn);
                }
                self.guest_player_tick(
                    gi,
                    conn,
                    mod_api::HookId::PlayerTickPost,
                    |game, player| {
                        game.native_player_tick_post(mod_api::__private::PlayerTickPostCall {
                            player,
                        })
                    },
                );
                continue;
            }
            if self.on_server_trip(conn) {
                // LivingEntity.hurt kills at once (die), whether the level
                // has loaded around the player or not: its respawn request
                // is taken as soon as it comes.
                self.guest_die(gi, conn);
                if let Some(host) = self.lan.as_ref() {
                    refresh_building_player(host, conn);
                }
                self.guest_player_tick(
                    gi,
                    conn,
                    mod_api::HookId::PlayerTickPost,
                    |game, player| {
                        game.native_player_tick_post(mod_api::__private::PlayerTickPostCall {
                            player,
                        })
                    },
                );
                continue;
            }
            // ServerPlayerGameMode.tick stops an active destroy when its block is air.
            let Some(host) = self.lan.as_mut() else {
                return;
            };
            if let Some((pos, _)) = host.play.get(&conn).and_then(|play| play.digging)
                && self
                    .res
                    .facts()
                    .is_air(self.world.block(pos[0], pos[1], pos[2]))
                && let Some(play) = host.play.get_mut(&conn)
            {
                play.digging = None;
                play.mining = None;
            }
            let play = host.play.entry(conn).or_default();
            // ServerGamePacketListenerImpl.tick's dropSpamThrottler.
            play.creative_drops = play.creative_drops.saturating_sub(1);
            let digging = play.digging;
            let using = play.using.is_some();
            let delayed = play.destroy_delay > 0 || play.use_delay > 0 || play.attack_ticks < 100;
            if digging.is_some() || using || delayed {
                let players = if using {
                    self.stab_players_for(conn)
                } else {
                    Vec::new()
                };
                self.as_guest(conn, |g| {
                    let outer = std::mem::replace(&mut g.stab_players, players);
                    g.attack_ticks = g.attack_ticks.saturating_add(1);
                    if let Some((pos, face)) = digging {
                        g.forced_hit = Some(Hit {
                            pos,
                            face,
                            point: [
                                f64::from(pos[0]) + 0.5,
                                f64::from(pos[1]) + 0.5,
                                f64::from(pos[2]) + 0.5,
                            ],
                            inside: false,
                        });
                    }
                    let _ = g.tick_mining();
                    g.forced_hit = None;
                    g.right = g.using.is_some();
                    g.tick_using();
                    g.stab_players = outer;
                    if g.use_delay > 0 {
                        g.use_delay -= 1;
                    }
                });
                self.land_player_stabs();
                // ServerPlayerGameMode.handleBlockBreakAction: creative and
                // instant survival STARTs finish without a STOP from the client.
                // tick_mining clears mining on completion; forget the START too,
                // so a flower or crop planted here later is not broken again.
                if digging.is_some()
                    && let Some(host) = self.lan.as_mut()
                    && let Some(play) = host.play.get_mut(&conn)
                    && (host.hub.guests[gi].creative() || play.mining.is_none())
                {
                    play.digging = None;
                }
            }
            // Player.tick's ItemCooldowns.tick, once a tick for each player
            // (the shield an axe disabled, a thrown pearl), after
            // LivingEntity.tick's use: an entry a finished use starts has 20
            // ticks left when the next tick's packets are handled, as the
            // guest's own client counts.
            let Some(host) = self.lan.as_mut() else {
                return;
            };
            let play = host.play.entry(conn).or_default();
            play.shield_cooldown = play.shield_cooldown.saturating_sub(1);
            // ServerItemCooldowns.onCooldownEnded: a finished entry is told to
            // the guest (a duration of 0 removes it).
            for group in play.cooldowns.tick() {
                host.hub.send_to(
                    conn,
                    &C::Cooldown(play::Cooldown {
                        group: group.to_string(),
                        duration: 0,
                    }),
                );
            }
            // Player.tick: a different main-hand item resets the attack
            // strength, and detectEquipmentUpdates gives the guest its modifiers.
            let g = &host.hub.guests[gi];
            let held = crate::game::held_items::held_attack_item(&g.inventory, g.selected);
            let play = host.play.entry(conn).or_default();
            if play.attack_item.as_ref() != held {
                play.attack_ticks = 0;
                play.attack_item = held.cloned();
            }
            self.guest_delayed_destroy(conn);
            self.guest_server_tick(conn);
            // Player.tick's statistics and ServerPlayer.tick's triggers, then
            // PlayerAdvancements.flushDirty.
            let Some(host) = self.lan.as_mut() else {
                return;
            };
            let sleeping = host.hub.guests[gi].sleeping;
            // ServerPlayer.doTick's updateScoreForCriteria: the health, food,
            // armor, experience and level objectives follow the guest.
            let mut recorded =
                std::mem::take(&mut host.play.entry(conn).or_default().score_recorded);
            self.as_guest(conn, |g| {
                g.tick_fishing();
                g.tick_maps();
                g.tick_lodestone_compasses();
                g.native_inventory_ticks();
                g.count_tick_stats(sleeping);
                g.tick_advancements();
                g.update_player_attributes();
                g.record_player_scores(&mut recorded);
            });
            if let Some(host) = self.lan.as_mut() {
                host.play.entry(conn).or_default().score_recorded = recorded;
            }
            self.lan_map_updates(conn);
            if let Some(host) = self.lan.as_mut() {
                host.hub.flush_progress(conn);
            }
            self.guest_dig_feedback(conn);
            // A hand-over freezes this player's inventory until its record is released.
            if self
                .lan
                .as_ref()
                .is_some_and(|host| host.hub.guests[gi].handover.is_none())
            {
                self.guest_pickups(gi);
                self.guest_arrows(gi);
                self.guest_orbs(gi);
            }
            self.guest_die(gi, conn);
            if let Some(host) = self.lan.as_ref() {
                refresh_building_player(host, conn);
            }
            self.guest_player_tick(gi, conn, mod_api::HookId::PlayerTickPost, |game, player| {
                game.native_player_tick_post(mod_api::__private::PlayerTickPostCall { player })
            });
        }
    }

    /// Only listeners pay for a guest swap; reuse the loop's resolved index.
    pub(super) fn guest_player_tick(
        &mut self,
        gi: usize,
        conn: u32,
        hook: mod_api::HookId,
        fire: fn(&mut Self, mod_api::PlayerRef),
    ) {
        if !mod_api::__private::listened(hook) {
            return;
        }
        let (Some(epoch), Some(host)) = (self.native_lifecycle.epoch(), self.lan.as_ref()) else {
            return;
        };
        let guest = &host.hub.guests[gi];
        let player = mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(epoch, guest.entity_id as u32),
            guest.uuid,
        );
        let _ = self.as_guest_at(gi, conn, |game| fire(game, player));
    }

    /// `LivingEntity.checkAutoSpinAttack` for a server's copy of a Riptide
    /// spinner (it runs before `aiStep`'s countdown clears the damage): the
    /// first mob touching the player's box takes `Player.attack` with the
    /// spin's damage and the spin ends. A `ServerPlayer` is client
    /// authoritative, so its `aiStep` does not travel and both boxes vanilla
    /// passes are the current one: no sweep over the tick's moves.
    pub(in crate::game) fn server_spin_attack(&mut self) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        if p.auto_spin_attack_ticks <= 0 {
            return;
        }
        let swept = p.bounds();
        let riding = self.riding;
        let Some(index) = first_spin_target(
            swept,
            riding,
            self.mobs.iter().map(|m| (m.id, m.removed, m.bounds())),
        ) else {
            return;
        };
        self.attack_mob(index);
        if let Some(p) = self.player.as_mut() {
            p.auto_spin_attack_ticks = 0;
            p.auto_spin_attack_dmg = 0.0;
            p.velocity = p.velocity.scale(-0.2);
        }
    }

    /// `handlePlayerPositionChange` jumps from the pre-move position and yaw.
    /// A later attack packet reads the resulting velocity even before travel.
    pub(in crate::game) fn guest_jump(&mut self, conn: u32, from: [f64; 3], yaw: f32) {
        let identity = if !mod_api::__private::listened(mod_api::HookId::PlayerJump) {
            None
        } else {
            self.lan
                .as_ref()
                .and_then(|host| host.hub.guest(conn))
                .map(|g| (g.uuid, g.name.clone()))
        };
        self.as_guest(conn, |game| {
            let Some(player) = game.player.as_mut() else {
                return;
            };
            let (position, yaw_after, ground) =
                (player.position, player.yaw, player.ground.on_ground);
            player.position = Vec3::new(from[0], from[1], from[2]);
            player.yaw = yaw;
            player.ground.on_ground = true;
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
            let _ = w.jump_from_ground();
            (w.player.position, w.player.yaw, w.player.ground.on_ground) =
                (position, yaw_after, ground);
            if let Some((uuid, name)) = identity {
                game.native_player_jump(uuid, &name);
            }
        });
    }
}

#[cfg(test)]
#[path = "tests/building_cache.rs"]
mod building_cache_tests;

#[cfg(test)]
#[path = "tests/fall_sound_tests.rs"]
mod fall_sound_tests;
