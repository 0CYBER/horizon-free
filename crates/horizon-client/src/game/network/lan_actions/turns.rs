use super::*;

impl Game<'_> {
    /// Run `f` as the guest `conn`: the host's player state is swapped for theirs.
    /// Returns `None` if the guest is missing or already active in a nested turn.
    pub(in crate::game) fn as_guest<R>(
        &mut self,
        conn: u32,
        f: impl FnOnce(&mut Self) -> R,
    ) -> Option<R> {
        let gi = self.guest_index(conn)?;
        self.as_guest_at(gi, conn, f).ok()
    }

    /// `as_guest` for the guest at `gi` in the hub's list, which a caller that already found it
    /// passes with its `conn`: the turn starts without another scan, and is refused when the
    /// guest there isn't `conn`.
    pub(in crate::game) fn as_guest_at<R>(
        &mut self,
        gi: usize,
        conn: u32,
        f: impl FnOnce(&mut Self) -> R,
    ) -> Result<R, TurnRefused> {
        let swap = self.enter_guest_at(gi, conn, self.dim)?;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::game::dedicated::crash_fixture("guest", || {
                self.lan
                    .as_ref()
                    .is_some_and(|host| host.hub.guest(conn).is_some_and(|g| g.placed))
            });
            let dimension = (self.serving.is_some() || self.options.dedicated.is_some())
                .then_some(self.dim.id());
            let result = horizon_client::block_contact::in_level(dimension, || f(self));
            self.drain_entity_inside();
            result
        }));
        // Commit the guest's progress and restore the outer player before propagating a panic.
        self.leave_guest(swap);
        match result {
            Ok(result) => Ok(result),
            Err(payload) => {
                // The record now holds what the turn did before the panic; the restore above
                // also hid the turn from `lan_acting`, so crash teardown is told here.
                self.lan_interrupted_turns.push(conn);
                std::panic::resume_unwind(payload)
            }
        }
    }

    /// Where guest `conn` is in the hub's list.
    pub(in crate::game) fn guest_index(&self, conn: u32) -> Option<usize> {
        self.lan.as_ref()?.hub.guests.connection_index(conn)
    }

    /// `as_guest`, with the system messages the guest's action earns (the
    /// `player::sendSystemMessage` of a packet handler) sent to that guest.
    pub(in crate::game) fn as_guest_messaged<R>(
        &mut self,
        conn: u32,
        f: impl FnOnce(&mut Self) -> R,
    ) -> Option<R> {
        // The outer capture comes back even when `f` panics (a native callback can), so later
        // feedback isn't trapped in this guest's capture. What the guest was already told still
        // reaches them before the panic goes on, after `as_guest` has restored the swap.
        let (result, lines) = self.as_guest(conn, |game| {
            let outer = game.chat_capture.replace(Vec::new());
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(game)));
            let lines = std::mem::replace(&mut game.chat_capture, outer).unwrap_or_default();
            (result, lines)
        })?;
        if let Some(host) = self.lan.as_ref() {
            for line in lines {
                host.hub.send_to(
                    conn,
                    &C::SystemChat(play::SystemChat {
                        content: horizon_client::lan::data::component_nbt(&line.component()),
                        overlay: false,
                    }),
                );
            }
        }
        match result {
            Ok(result) => self.lan.is_some().then_some(result),
            Err(payload) => {
                // `as_guest` saw this turn end normally: the panic was caught inside it.
                self.lan_interrupted_turns.push(conn);
                std::panic::resume_unwind(payload)
            }
        }
    }

    /// The first half of `as_guest`: the guest's state in place of the
    /// game's, which the returned swap keeps until `leave_guest`. The host
    /// stays in `self.lan`, so the guest can reach other players. Guest turns
    /// can nest: the acting guest and their queued packets are restored.
    pub(in crate::game) fn enter_guest(&mut self, conn: u32) -> Option<GuestSwap> {
        self.enter_guest_in_dimension(conn, self.dim)
    }

    /// Enter a guest after changing levels: the outer body is still in
    /// `outer_dimension`, not necessarily the level now in place.
    pub(in crate::game) fn enter_guest_in_dimension(
        &mut self,
        conn: u32,
        outer_dimension: Dim,
    ) -> Option<GuestSwap> {
        let i = self.guest_index(conn)?;
        self.enter_guest_at(i, conn, outer_dimension).ok()
    }

    /// `enter_guest` for the guest at `i`, which must be `conn`.
    pub(super) fn enter_guest_at(
        &mut self,
        i: usize,
        conn: u32,
        outer_dimension: Dim,
    ) -> Result<GuestSwap, TurnRefused> {
        let spawn_protection = self.lan.as_ref().and_then(|host| {
            host.hub
                .guests
                .get(i)
                .and_then(|g| self.spawn_protection_for(host, g.uuid))
        });
        let outer = BuildingPlayer {
            conn: self.lan_acting,
            dimension: outer_dimension,
            bounds: self
                .player
                .as_ref()
                .filter(|p| !self.lan_acting_fake && !self.spectator() && !p.health.dead_or_dying())
                .map(PlayerMotion::bounds),
            borrowed: true,
        };
        let Some(host) = self.lan.as_mut() else {
            return Err(TurnRefused::Gone);
        };
        if host.hub.guests.get(i).is_none_or(|g| g.conn != conn) {
            return Err(TurnRefused::Gone);
        }
        // An active guest's record holds an outer player's state, not theirs.
        if self.lan_guest_swaps.contains(&conn) {
            return Err(TurnRefused::Nested);
        }
        let other_boxes =
            self.lan_building_boxes
                .for_turn(conn, Some(outer), building_players(host));
        let fresh = host.hub.guests[i].motion.is_none();
        if fresh {
            let initialized = host
                .hub
                .guests
                .update(i, |guest| super::guest_body::initialize(self.motion, guest));
            if !initialized {
                return Err(TurnRefused::Gone);
            }
        }
        // Refusal must leave canonical records and loan state untouched.
        let Some(mut motion) = host.hub.guests.update(i, |g| g.motion.take()) else {
            return Err(TurnRefused::Gone);
        };
        host.hub.guests[i].residency.extract();
        self.lan_guest_swaps.push(conn);
        // `ServerPlayer.permissions()`: the guest's own command level for its
        // turn, so game-master checks see it (not the host's).
        let permission = std::mem::replace(
            &mut self.ui.registries.permission,
            host.hub.guests[i].permission,
        );
        let mut play = host.play.remove(&conn).unwrap_or_default();
        play.hook.begin_loan();
        if fresh {
            // A new body (joined, respawned, changed level): nothing on it yet,
            // its effects' modifiers and armor protection included.
            play.equipped = None;
            play.potions = Default::default();
            play.sculk = Default::default();
            // A new ServerPlayer has everything to record again.
            play.score_recorded = Default::default();
        }
        let attachments = std::mem::replace(
            &mut self.player_state.attachments,
            std::mem::take(host.hub.guests.attachments_mut(conn).unwrap()),
        );
        self.sculk.swap_player(&mut play.sculk);
        // The guest's own CombatTracker: damage to them is not the host's.
        // A new body starts with no record of the last one.
        let guest_combat = if fresh {
            Default::default()
        } else {
            self.guest_combat.remove(&conn).unwrap_or_default()
        };
        let host_combat = std::mem::replace(&mut self.combat, guest_combat);
        host.hub.guests.update(i, |g| {
            self.potions.swap_player(&mut g.effects, &mut play.potions)
        });
        let g = &host.hub.guests[i];
        motion.position = Vec3::new(g.position[0], g.position[1], g.position[2]);
        motion.entity_id = g.entity_id;
        motion.ground.on_ground = g.on_ground;
        motion.yaw = g.yaw;
        motion.pitch = g.pitch;
        motion.sprinting = g.sprinting;
        let abilities = g.abilities;
        motion.may_fly = abilities.may_fly;
        motion.flying = abilities.flying;
        motion.flying_speed = abilities.flying_speed;
        // FireworkRocketItem.use boosts only a gliding player.
        motion.fall_flying = g.fall_flying;
        // A player frozen for a hand-over must keep its saved record unchanged.
        motion.invulnerable = abilities.invulnerable || g.handover.is_some();
        // `ServerPlayer.isInvulnerableTo`'s connection term, as of now.
        motion.client_loaded = g.client_loaded();
        // The mods' collision exclusions are the guest's (arismoko/horizon#3717).
        motion
            .collision_exclusions
            .clone_from(&g.collision_exclusions);
        let _ = motion.set_health(g.health, false);
        motion.health.food.food = g.food;
        motion.health.food.saturation = g.saturation;
        let mut player = Some(*motion);
        match self.lan_acting {
            Some(outer) => {
                if let Some(guest) = host.hub.guest(outer) {
                    guest.residency.extract();
                }
            }
            None => self.resident_entities.local_player.extract(),
        }
        std::mem::swap(&mut self.player, &mut player);
        let eye = (g.eye_height() as f32, self.eye);
        self.eye = eye.0;
        let (yaw, pitch) = (self.yaw, self.pitch);
        self.yaw = g.yaw;
        self.pitch = g.pitch;
        host.hub.guests.update(i, |g| {
            std::mem::swap(&mut self.inventory, &mut g.inventory);
            std::mem::swap(&mut self.armor, &mut g.armor);
            std::mem::swap(&mut self.location_armor, &mut g.location_armor);
            std::mem::swap(&mut self.offhand, &mut g.offhand);
            std::mem::swap(&mut self.equipment, &mut g.equipment);
            std::mem::swap(&mut self.carried, &mut g.carried);
        });
        let g = &host.hub.guests[i];
        let selected = std::mem::replace(&mut self.selected, g.selected);
        let (mode, creative) = (self.mode, self.creative);
        let outer_abilities = self.acting_abilities.replace(g.abilities);
        let (entity_reach, block_reach) = (self.entity_reach, self.block_reach);
        let (game_mode, guest_previous) = (g.game_mode, g.previous_game_mode);
        self.set_mode_raw(game_mode);
        let previous_mode = std::mem::replace(
            &mut self.previous_mode,
            guest_previous.map(crate::game::game_mode::mode_from_id),
        );
        let Some(host) = self.lan.as_mut() else {
            unreachable!("LanHost dropped inside a guest turn; closes are deferred");
        };
        let g = &host.hub.guests[i];
        let xp = std::mem::replace(
            &mut self.experience,
            Experience {
                level: g.xp_level,
                progress: g.xp_progress,
                total: g.xp_total,
                score: g.score,
            },
        );
        let level_up_sound = std::mem::replace(&mut self.level_up_sound, play.level_up_sound);
        // The host's held keys stay theirs: the guest holds only sneak, and a
        // screen the guest opens (`keys.clear()`) leaves the host walking.
        let host_keys = std::mem::take(&mut self.keys);
        if g.sneaking {
            self.keys.insert(KeyCode::ShiftLeft);
        }
        // The window's screens are the host's: a sign, book, command block or
        // mount menu the guest's action opens is not drawn over the host's
        // world (arismoko/horizon#687). The guest's client opens its own.
        let screen = std::mem::replace(&mut self.screen, Screen::Playing);
        let host_sign = self.sign_edit.take();
        let host_book = self.book.take();
        let host_command_block = self.command_block_screen.take();
        let host_mount_menu = std::mem::take(&mut self.mount_menu);
        // The host's crafting grid stays theirs; a crafting table the guest
        // opens shows as the 3x3 size.
        let craft_size = std::mem::replace(&mut self.craft_size, 2);
        let craft_table = self.craft_table.take();
        play.craft_grid.clear();
        play.craft_grid.resize(9, None);
        let craft_grid =
            std::mem::replace(&mut self.craft_grid, std::mem::take(&mut play.craft_grid));
        let container = self.container.take();
        let mining = std::mem::replace(&mut self.mining, play.mining.take());
        let destroy_delay = std::mem::replace(&mut self.destroy_delay, play.destroy_delay);
        let using = std::mem::replace(&mut self.using, play.using.take());
        let use_duration = std::mem::replace(&mut self.use_duration, play.use_duration);
        let use_start = std::mem::replace(&mut self.use_start, std::mem::take(&mut play.use_start));
        let use_delay = std::mem::replace(&mut self.use_delay, play.use_delay);
        let shield_cooldown = std::mem::replace(&mut self.shield_cooldown, play.shield_cooldown);
        let cooldowns = std::mem::replace(&mut self.cooldowns, std::mem::take(&mut play.cooldowns));
        let xp_delay = std::mem::replace(&mut self.xp_delay, play.xp_delay);
        let attack_ticks = std::mem::replace(&mut self.attack_ticks, play.attack_ticks);
        let attack_item = std::mem::replace(&mut self.attack_item, play.attack_item.take());
        let swing = std::mem::replace(&mut self.swing, play.swing);
        std::mem::swap(&mut self.mount, &mut play.mount);
        // `Entity.portalCooldown` and `portalProcess` are the player's own.
        std::mem::swap(&mut self.portals.player, &mut play.portal);
        let (left, right) = (self.left, self.right);
        self.left = play.digging.is_some();
        self.right = play.using.is_some() || self.using.is_some();
        // The guest's vehicle (Entity.vehicle) for its turn.
        let riding = std::mem::replace(&mut self.riding, g.vehicle);
        self.lan_outer_ridings.push(riding);
        // `Entity.isPassenger()` for that turn, from the same vehicle: the
        // guest's body keeps no swim (`Entity.updateSwimming` needs it false).
        // Taken from `g.vehicle` each turn, so boarding and getting out land.
        if let Some(p) = self.player.as_mut() {
            p.is_passenger = g.vehicle.is_some();
        }
        // Entity.collide: the guest's body collides with the level's world
        // border too, as it is this turn (it moves with /worldborder).
        self.player_border();
        let Some(host) = self.lan.as_mut() else {
            unreachable!("LanHost dropped inside a guest turn; closes are deferred");
        };
        let g = &host.hub.guests[i];
        let host_station = self.station.take();
        // The guest's bed, sleep counter and respawn point; the host's
        // respawn point, anchor and action bar stay theirs.
        let sleeping = std::mem::replace(&mut self.sleeping, g.bed);
        let sleep_timer = std::mem::replace(&mut self.sleep_timer, g.sleep_timer);
        let spawn = std::mem::replace(
            &mut self.spawn,
            g.spawn.map(|s| Vec3::new(s[0], s[1], s[2])),
        );
        let respawn = (self.respawn_config.take(), self.level.spawn);
        self.respawn_config = g.respawn_elsewhere.as_ref().map(RespawnConfig::from_saved);
        let message = self.message.take();
        // The guest's own ender inventory; one the host has open goes back
        // into the host's while the guest acts.
        let host_ender = self.ender_open;
        self.close_ender_chest();
        let Some(host) = self.lan.as_mut() else {
            unreachable!("LanHost dropped inside a guest turn; closes are deferred");
        };
        let simulated = host.hub.guests.update(i, |g| {
            std::mem::swap(&mut self.ender_items, &mut g.ender_items);
            g.participates_in_player_simulation()
        });
        // The guest's advancements, recipe book and statistics.
        if simulated {
            self.advancement_tree();
        }
        let Some(host) = self.lan.as_mut() else {
            unreachable!("LanHost dropped inside a guest turn; closes are deferred");
        };
        host.hub.guests.update(i, |g| {
            std::mem::swap(&mut self.adv.progress, &mut g.advancements);
            std::mem::swap(&mut self.adv.changed, &mut g.advancements_changed);
            std::mem::swap(&mut self.adv.unlocked, &mut g.recipes_unlocked);
            std::mem::swap(&mut self.level.recipe_book, &mut g.recipe_book);
            std::mem::swap(&mut self.level.player_extra, &mut g.saved);
            std::mem::swap(&mut self.level.last_death, &mut g.last_death);
        });
        let player_save_projection = (
            self.level.player_impulse_grace.take(),
            self.level.player_fall_particles.take(),
        );
        let g = &host.hub.guests[i];
        std::mem::swap(&mut self.stats, &mut play.stats);
        self.hook.begin_loan();
        std::mem::swap(&mut self.hook, &mut play.hook);
        let adv_loaded = std::mem::replace(&mut self.adv.loaded, true);
        let last_inventory = std::mem::replace(&mut self.adv.last_inventory, play.last_inventory);
        let levitation = std::mem::replace(&mut self.adv.levitation, play.levitation);
        let marks = std::mem::replace(&mut self.adv.marks, std::mem::take(&mut play.marks));
        let death_cause = std::mem::replace(&mut self.ui.death_cause, play.death_cause.take());
        let [x, y, z] = g.known_movement;
        let known_movement = std::mem::replace(&mut self.known_movement, Vec3::new(x, y, z));
        let acting = (
            self.lan_acting.replace(conn),
            self.lan_acting_name.replace(g.name.clone()),
            self.lan_acting_uuid.replace(g.uuid),
            self.lan_acting_permission.replace(g.permission),
            self.lan_acting_entity.replace(g.entity_id as u32),
            std::mem::replace(&mut self.lan_acting_spawn_protection, spawn_protection),
        );
        let acting_fake = std::mem::replace(&mut self.lan_acting_fake, !g.participates_online());
        g.residency.put_back();
        self.hook.finish_loan();
        self.bind_current_hook_residency();
        let packets = std::mem::take(&mut self.guest_packets);
        let entity_events = std::mem::take(&mut self.guest_entity_events);
        let cooldown_events = std::mem::take(&mut self.guest_cooldowns);
        let player_sounds = std::mem::take(&mut self.guest_player_sounds);
        let block_sounds = std::mem::take(&mut self.guest_block_sounds);
        let equipment_breaks = std::mem::take(&mut self.guest_equipment_breaks);
        let swings = self.swings_started;
        let projectiles = self.creatures.projectiles.len();
        let modifiers = std::mem::replace(
            &mut self.applied_modifiers,
            std::mem::take(&mut play.modifiers),
        );
        // Its equipment on its attributes, when it changed (once its tick saw a new hand item).
        let selected_item = self.inventory.get(self.selected).and_then(Option::as_ref);
        let unchanged = play.equipped.as_deref().is_some_and(|eq| {
            eq.len() == self.armor.len() + 1
                && eq[..self.armor.len()] == self.armor[..]
                && eq[self.armor.len()].as_ref() == selected_item
        });
        if !unchanged && (play.equipped.is_none() || self.held_item_ticked()) {
            let mut equipped: Vec<Option<ItemStack>> = self.armor.to_vec();
            equipped.push(selected_item.cloned());
            self.apply_equipment();
            play.equipped = Some(equipped);
        }
        // The player's effects are on the new body before anything it does
        // (the movement that brought it, a landing): vanilla keeps the same
        // entity through a level change, with its attributes and effects.
        if fresh {
            self.apply_effect_status();
        }
        self.update_totem_ready();
        self.entity_reach = self.interaction_range("minecraft:entity_interaction_range", 3.0, 5.0);
        self.block_reach = self.interaction_range("minecraft:block_interaction_range", 4.5, 5.0);
        let building_boxes = std::mem::replace(&mut self.lan_building_boxes, other_boxes);
        Ok(GuestSwap {
            attachments,
            conn,
            i,
            play,
            player,
            player_save_projection,
            eye,
            yaw,
            pitch,
            selected,
            mode,
            creative,
            previous_mode,
            abilities: outer_abilities,
            entity_reach,
            block_reach,
            permission,
            xp,
            level_up_sound,
            host_keys,
            screen,
            host_sign,
            host_book,
            host_command_block,
            host_mount_menu,
            craft_size,
            craft_table,
            craft_grid,
            container,
            mining,
            destroy_delay,
            using,

            use_duration,
            use_start,
            use_delay,
            shield_cooldown,
            cooldowns,
            xp_delay,
            attack_ticks,
            attack_item,
            swing,
            left,
            right,
            riding,
            host_station,
            sleeping,
            sleep_timer,
            spawn,
            respawn,
            message,
            host_ender,
            adv_loaded,
            last_inventory,
            levitation,
            marks,
            death_cause,
            acting,
            acting_fake,
            known_movement,
            packets,
            entity_events,
            equipment_breaks,
            swings,
            projectiles,
            modifiers,
            cooldown_events,
            player_sounds,
            block_sounds,
            combat: host_combat,
            building_boxes,
        })
    }

    /// The second half of `as_guest`: the guest's state back into their
    /// record, the game's back in place, and what the turn changed sent.
    pub(in crate::game) fn leave_guest(&mut self, swap: GuestSwap) {
        let Some(host) = self.lan.as_ref() else {
            unreachable!("LanHost dropped inside a guest turn; closes are deferred");
        };
        debug_assert_eq!(host.hub.guests.get(swap.i).map(|g| g.conn), Some(swap.conn));
        let turn_events = self
            .player
            .as_mut()
            .map(|p| std::mem::take(&mut p.events))
            .unwrap_or_default();
        // The callbacks a turn ends with (combat, armor, the totem, the food hook) can reach a
        // mod and panic. The swap is still restored below and the guest's progress written
        // back; the panic resumes at the end (arismoko/horizon#2240). `as_guest_at` calls this after catching a
        // panic of the turn itself, so a panic here then goes on in place of that one.
        let callbacks = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if !turn_events.is_empty() {
                self.record_combat(&turn_events);
                self.wear_armor(&turn_events);
            }
            // A totem that saved the guest this turn (its hurt set it off).
            let totem = self.apply_totem();
            // The mods hear the food level the turn changed (eating, hunger,
            // starving...), once for the change it made as a whole, with the
            // guest's state still swapped in; they may change or undo it before it
            // reaches the guest.
            let now = self.player.as_ref().map(|p| p.health.food.food);
            let food = self
                .lan
                .as_ref()
                .and_then(|host| host.hub.guests.get(swap.i))
                .filter(|g| now.is_some_and(|now| g.food != now))
                .map(|g| (g.uuid, g.name.clone(), g.food, g.saturation));
            if let Some(now) = now
                && let Some((uuid, name, from, saturation)) = food
            {
                let (mut to, mut cancel) = (now, false);
                self.native_food_level_changed(uuid, &name, from, &mut to, &mut cancel);
                if let Some(p) = self.player.as_mut() {
                    if cancel {
                        p.health.food.food = from;
                        p.health.food.saturation = saturation;
                    } else if to != now {
                        p.health.food.food = to.clamp(0, 20);
                        p.health.food.saturation =
                            p.health.food.saturation.min(p.health.food.food as f32);
                    }
                }
            }
            totem
        }));
        let (totem, panic) = match callbacks {
            Ok(totem) => (totem, None),
            Err(payload) => (None, Some(payload)),
        };
        // Crash teardown treats the turn as interrupted until it has been put back without a
        // panic: the callbacks below run after the restore and can panic too.
        self.lan_interrupted_turns.push(swap.conn);
        // A turn opened inside this one by a raw `enter_guest` that a panic left open: its
        // player's record is displaced too, and the pop below would forget them.
        if let Some(at) = self.lan_guest_swaps.iter().rposition(|&c| c == swap.conn) {
            let above = self.lan_guest_swaps[at + 1..].to_vec();
            self.lan_interrupted_turns.extend(above);
        }
        let GuestSwap {
            attachments,
            conn,
            i,
            mut play,
            mut player,
            player_save_projection,
            eye,
            yaw,
            pitch,
            selected,
            mode,
            creative,
            previous_mode: outer_previous,
            abilities: outer_abilities,
            entity_reach,
            block_reach,
            permission,
            xp,
            level_up_sound,
            host_keys,
            screen,
            host_sign,
            host_book,
            host_command_block,
            host_mount_menu,
            craft_size,
            craft_table,
            craft_grid,
            container,
            mining,
            destroy_delay,
            using,

            use_duration,
            use_start,
            use_delay,
            shield_cooldown,
            cooldowns,
            xp_delay,
            attack_ticks,
            attack_item,
            swing,
            left,
            right,
            riding,
            host_station,
            sleeping,
            sleep_timer,
            spawn,
            respawn,
            message,
            host_ender,
            adv_loaded,
            last_inventory,
            levitation,
            marks,
            death_cause,
            combat,
            building_boxes,
            acting,
            acting_fake,
            known_movement,
            packets,
            entity_events,
            equipment_breaks,
            swings,
            projectiles,
            modifiers,
            cooldown_events,
            player_sounds,
            block_sounds,
        } = swap;
        // `ServerPlayer.permissions()` back to the host's after the turn.
        self.ui.registries.permission = permission;
        self.sculk.swap_player(&mut play.sculk);
        // The guest's damage record back into their slot, the host's in place.
        let guest_combat = std::mem::replace(&mut self.combat, combat);
        self.guest_combat.insert(conn, guest_combat);
        play.modifiers = std::mem::replace(&mut self.applied_modifiers, modifiers);
        // What the guest shot is theirs (Projectile.setOwner).
        for i in projectiles..self.creatures.projectiles.len() {
            self.creatures.projectiles.update(i, |p| {
                if p.owner == Some(horizon_client::mobs::Who::Player) && p.guest.is_none() {
                    p.guest = Some(conn);
                }
            });
        }
        let listening = mod_api::__private::listened(mod_api::HookId::PlayerLaunchProjectile);
        for projectile in projectiles..self.creatures.projectiles.len() {
            if !listening {
                break;
            }
            let item = match &self.creatures.projectiles[projectile].kind {
                horizon_client::projectiles::ProjectileKind::Snowball => {
                    Some("minecraft:snowball".to_owned())
                }
                horizon_client::projectiles::ProjectileKind::Egg => {
                    Some("minecraft:egg".to_owned())
                }
                horizon_client::projectiles::ProjectileKind::EnderPearl => {
                    Some("minecraft:ender_pearl".to_owned())
                }
                horizon_client::projectiles::ProjectileKind::Trident => {
                    Some("minecraft:trident".to_owned())
                }
                horizon_client::projectiles::ProjectileKind::Potion(stack, _) => {
                    Some(stack.item.to_string())
                }
                _ => None,
            };
            if let Some(item) = item {
                let Some(host) = self.lan.as_ref() else {
                    unreachable!("LanHost dropped inside a guest turn; closes are deferred");
                };
                let guest = &host.hub.guests[i];
                let (uuid, name) = (guest.uuid, guest.name.clone());
                self.native_player_launch_projectile(uuid, &name, &item);
            }
        }
        // The guest's swings are theirs: the host's own count stays the host's.
        play.swings += self.swings_started.wrapping_sub(swings);
        self.swings_started = swings;
        // Player.playSound: everyone near but the guest, in the players' source,
        // through `ServerLevel.playSeededSound` like any level sound.
        for (name, at, volume, pitch) in
            std::mem::replace(&mut self.guest_player_sounds, player_sounds)
        {
            let sound = LevelSound::new(&name, at, volume, pitch).in_source(SoundSource::Players);
            self.send_level_sound(sound, Some(conn));
        }
        // Block-source sounds exclude the acting player, like player sounds.
        for (name, at, volume, pitch) in
            std::mem::replace(&mut self.guest_block_sounds, block_sounds)
        {
            let sound = LevelSound::new(&name, at, volume, pitch).in_source(SoundSource::Blocks);
            self.send_level_sound(sound, Some(conn));
        }
        let outgoing = std::mem::replace(&mut self.guest_packets, packets);
        let Some(host) = self.lan.as_mut() else {
            unreachable!("LanHost dropped inside a guest turn; closes are deferred");
        };
        if host.hub.guests[i].participates_online() {
            for packet in outgoing {
                host.hub.send_to(conn, &packet);
            }
        }
        // ServerPlayer.connection.send: the guest's own entity events.
        let entity_id = host.hub.guests[i].entity_id;
        let outgoing_events = std::mem::replace(&mut self.guest_entity_events, entity_events);
        if host.hub.guests[i].participates_online() {
            for event in outgoing_events {
                host.hub.send_to(
                    conn,
                    &C::EntityEvent(play::EntityEvent {
                        id: entity_id,
                        event,
                    }),
                );
            }
        }
        // LivingEntity.onEquippedItemBroken: `broadcastEntityEvent`, the guest too.
        for event in std::mem::replace(&mut self.guest_equipment_breaks, equipment_breaks) {
            host.hub.entity_event(entity_id, event);
        }
        // ServerItemCooldowns.onCooldownStarted: the guest's own cooldown.
        let outgoing_cooldowns = std::mem::replace(&mut self.guest_cooldowns, cooldown_events);
        if host.hub.guests[i].participates_online() {
            for (group, duration) in outgoing_cooldowns {
                host.hub.send_to(
                    conn,
                    &C::Cooldown(play::Cooldown {
                        group: group.to_string(),
                        duration,
                    }),
                );
            }
        }
        host.hub.guests[i].residency.extract();
        self.hook.begin_loan();
        (
            self.lan_acting,
            self.lan_acting_name,
            self.lan_acting_uuid,
            self.lan_acting_permission,
            self.lan_acting_entity,
            self.lan_acting_spawn_protection,
        ) = acting;
        self.lan_acting_fake = acting_fake;
        self.known_movement = known_movement;
        self.lan_building_boxes = building_boxes;
        play.death_cause = std::mem::replace(&mut self.ui.death_cause, death_cause);
        // An ender chest the guest opened: the menu shows their inventory
        // itself (lan_open_container), not a stand-in block entity.
        self.close_ender_chest();

        // What the action opened: handed to the guest instead of the host.
        let opened = self.container.take();
        let guest_table = std::mem::replace(&mut self.craft_table, craft_table);
        let opened_table = (std::mem::replace(&mut self.craft_size, craft_size) == 3)
            .then_some(guest_table)
            .flatten();
        play.craft_grid = std::mem::replace(&mut self.craft_grid, craft_grid);
        let opened_station = std::mem::replace(&mut self.station, host_station);
        let bed = std::mem::replace(&mut self.sleeping, sleeping);
        let guest_sleep_timer = std::mem::replace(&mut self.sleep_timer, sleep_timer);
        let guest_spawn = std::mem::replace(&mut self.spawn, spawn).map(|s| [s.x, s.y, s.z]);
        let guest_respawn = std::mem::replace(&mut self.respawn_config, respawn.0);
        self.level.spawn = respawn.1;
        // ServerPlayer.displayClientMessage(_, true): the guest's action bar.
        let guest_message = std::mem::replace(&mut self.message, message);
        self.lan_outer_ridings.pop();
        let guest_vehicle = std::mem::replace(&mut self.riding, riding);
        self.left = left;
        self.right = right;
        play.mining = std::mem::replace(&mut self.mining, mining);
        play.destroy_delay = std::mem::replace(&mut self.destroy_delay, destroy_delay);
        play.using = std::mem::replace(&mut self.using, using);
        play.use_duration = std::mem::replace(&mut self.use_duration, use_duration);
        play.use_start = std::mem::replace(&mut self.use_start, use_start);
        play.use_delay = std::mem::replace(&mut self.use_delay, use_delay);
        play.shield_cooldown = std::mem::replace(&mut self.shield_cooldown, shield_cooldown);
        play.cooldowns = std::mem::replace(&mut self.cooldowns, cooldowns);
        play.xp_delay = std::mem::replace(&mut self.xp_delay, xp_delay);
        play.attack_ticks = std::mem::replace(&mut self.attack_ticks, attack_ticks);
        play.attack_item = std::mem::replace(&mut self.attack_item, attack_item);
        play.swing = std::mem::replace(&mut self.swing, swing);
        std::mem::swap(&mut self.mount, &mut play.mount);
        std::mem::swap(&mut self.portals.player, &mut play.portal);
        self.container = container;
        self.screen = screen;
        // A sign the guest's placement opened (SignBlock.openTextEdit).
        let guest_sign = std::mem::replace(&mut self.sign_edit, host_sign);
        // A lectern the guest's use opened (LecternBlock.openScreen).
        let guest_lectern = match std::mem::replace(&mut self.book, host_book) {
            Some(crate::game::books::BookScreen::View {
                lectern: Some(key), ..
            }) => Some(key),
            _ => None,
        };
        // A command block the guest's use opened (Player.openCommandBlock).
        let guest_command_block =
            std::mem::replace(&mut self.command_block_screen, host_command_block).map(|s| s.pos());
        // A mount inventory the guest's action opened (shift + use, or the
        // inventory key while riding).
        let guest_mount = std::mem::replace(&mut self.mount_menu, host_mount_menu).opened_mob();
        self.keys = host_keys;
        let new_mode = self.mode_id();
        let turn_previous = std::mem::replace(&mut self.previous_mode, outer_previous);
        let turn_abilities = std::mem::replace(&mut self.acting_abilities, outer_abilities);
        let Some(host) = self.lan.as_mut() else {
            unreachable!("LanHost dropped inside a guest turn; closes are deferred");
        };
        enum TurnTeleport {
            Command([f64; 3], [bool; 3]),
            Position([f64; 3]),
            Rider([f64; 3]),
        }
        let released_vehicle = host.hub.guests[i]
            .vehicle
            .filter(|id| Some(*id) != guest_vehicle);
        let (stat_events, name, teleport) = host.hub.guests.update(i, |g| {
            let mut teleport = None;
            g.attachments = std::mem::replace(&mut self.player_state.attachments, attachments);
            let left_vehicle = g.vehicle.filter(|_| guest_vehicle.is_none());
            if guest_vehicle.is_none() {
                g.vehicle_good = None;
            }
            g.vehicle = guest_vehicle;
            let fell_asleep = bed.is_some() && g.bed.is_none();
            (g.bed, g.sleep_timer, g.spawn) = (bed, guest_sleep_timer, guest_spawn);
            g.respawn_elsewhere = guest_respawn.map(|c| c.saved());
            g.sleeping = g.bed.is_some();
            if fell_asleep {
                g.woke_from = None;
            }
            if let Some((text, _)) = guest_message {
                g.send(&C::SystemChat(play::SystemChat {
                    content: literal(&text),
                    overlay: true,
                }));
            }
            std::mem::swap(&mut self.ender_items, &mut g.ender_items);
            std::mem::swap(&mut self.adv.progress, &mut g.advancements);
            std::mem::swap(&mut self.adv.changed, &mut g.advancements_changed);
            std::mem::swap(&mut self.adv.unlocked, &mut g.recipes_unlocked);
            std::mem::swap(&mut self.level.recipe_book, &mut g.recipe_book);
            std::mem::swap(&mut self.level.player_extra, &mut g.saved);
            std::mem::swap(&mut self.level.last_death, &mut g.last_death);
            self.level.player_impulse_grace = player_save_projection.0;
            self.level.player_fall_particles = player_save_projection.1;
            std::mem::swap(&mut self.stats, &mut play.stats);
            std::mem::swap(&mut self.hook, &mut play.hook);
            self.adv.loaded = adv_loaded;
            play.last_inventory = std::mem::replace(&mut self.adv.last_inventory, last_inventory);
            play.levitation = std::mem::replace(&mut self.adv.levitation, levitation);
            play.marks = std::mem::replace(&mut self.adv.marks, marks);
            let stat_events = std::mem::take(&mut play.stats.events);
            let name = g.name.clone();
            self.potions.swap_player(&mut g.effects, &mut play.potions);
            if let Some(key) = host_ender {
                self.world
                    .block_entities
                    .insert(key, BlockEntity::Chest(self.ender_items.clone()));
                self.ender_open = Some(key);
            }
            play.level_up_sound = std::mem::replace(&mut self.level_up_sound, level_up_sound);
            let new_xp = std::mem::replace(&mut self.experience, xp);
            g.xp_level = new_xp.level;
            g.xp_progress = new_xp.progress;
            g.xp_total = new_xp.total;
            g.score = new_xp.score;
            self.mode = mode;
            self.creative = creative;
            self.entity_reach = entity_reach;
            self.block_reach = block_reach;
            g.selected = std::mem::replace(&mut self.selected, selected);
            std::mem::swap(&mut self.inventory, &mut g.inventory);
            std::mem::swap(&mut self.armor, &mut g.armor);
            std::mem::swap(&mut self.location_armor, &mut g.location_armor);
            std::mem::swap(&mut self.offhand, &mut g.offhand);
            std::mem::swap(&mut self.equipment, &mut g.equipment);
            std::mem::swap(&mut self.carried, &mut g.carried);
            // A teleport with a rotation (commands) turns the guest. `/rotate`
            // (ServerPlayer.forceSetRotation) sends the rotation alone, rider or not.
            let forced = std::mem::take(&mut self.rotation_forced);
            let turned = !forced && [self.yaw, self.pitch] != [g.yaw, g.pitch];
            [g.yaw, g.pitch] = [self.yaw, self.pitch];
            if forced {
                g.send(&C::PlayerRotation(play::PlayerRotation {
                    yaw: g.yaw,
                    relative_yaw: false,
                    pitch: g.pitch,
                    relative_pitch: false,
                }));
            }
            self.yaw = yaw;
            self.pitch = pitch;
            self.eye = eye.1;
            std::mem::swap(&mut self.player, &mut player);
            if let Some(mut m) = player {
                // Travel can end a glide on a climbable before the next action.
                g.fall_flying = m.fall_flying;
                g.health = m.health.health();
                g.food = m.health.food.food;
                g.saturation = m.health.food.saturation;
                // A teleport (commands, a pearl) moves the guest; a pearl's is
                // sent even to where they stand (ServerPlayer.teleport).
                let at = [m.position.x, m.position.y, m.position.z];
                // A hit with extra knockback stopped the guest's sprint
                // (Player.causeExtraKnockback's setSprinting(false)).
                if !m.sprinting {
                    g.sprinting = false;
                }
                m.sprinting = false;
                // The acting body's glide flag is authoritative for this turn.
                // In a nested turn this is the inner guest, never the suspended
                // outer actor whose body the swap restores above.
                g.fall_flying = m.fall_flying;
                // ServerPlayer.indicateDamage: the hit's direction to the player.
                if let Some(yaw) = m.health.indicated_dir.take() {
                    g.send(&C::HurtAnimation(play::HurtAnimation {
                        id: g.entity_id,
                        yaw,
                    }));
                }
                let teleported = std::mem::take(&mut m.teleported);
                let teleported_delta = m.teleported_delta;
                m.teleported_delta = [false; 3];
                let moved: f64 = (0..3).map(|k| (at[k] - g.position[k]).powi(2)).sum();
                g.motion = Some(Box::new(m));
                // The pearl and the bed took the rider out of its vehicle at their
                // source (`stop_riding`), so `left_vehicle` already says so.
                // ServerPlayer.dismountTo: the client hears it is out of the
                // vehicle before the teleport beside it. A rider sits where its
                // vehicle puts it (lan_vehicles.rs), with no teleport.
                if let Some(vehicle) = left_vehicle {
                    g.send(&C::SetPassengers(play::SetPassengers {
                        vehicle: vehicle as i32,
                        passengers: Vec::new(),
                    }));
                }
                if g.vehicle.is_none() && (moved > 0.0001 || turned || teleported) {
                    // A command's teleport carries its relative axes as the
                    // packet's DELTA flags; any other move is absolute.
                    if teleported {
                        teleport = Some(TurnTeleport::Command(at, teleported_delta));
                    } else {
                        teleport = Some(TurnTeleport::Position(at));
                    }
                } else if teleported {
                    // ServerPlayer.startRiding and an asPassenger transition also
                    // teleport the rider. Retain its mount and publish its body.
                    teleport = Some(TurnTeleport::Rider(at));
                }
            }
            (stat_events, name, teleport)
        });
        match teleport {
            Some(TurnTeleport::Command(at, delta)) => host.hub.teleport_command(i, at, delta),
            Some(TurnTeleport::Position(at)) => host.hub.teleport(i, Some(at)),
            Some(TurnTeleport::Rider(at)) => host.hub.teleport_rider(i, at),
            None => {}
        }
        let g = &host.hub.guests[i];
        if new_mode != g.game_mode {
            let conn = g.conn;
            host.hub
                .set_game_mode(conn, new_mode, &self.world, Some(self.collision));
        }
        // The turn's previous mode, as `Game::set_mode` kept it: the mode left
        // by the turn's last change, when it changed more than once.
        host.hub.guests.update(i, |g| {
            g.previous_game_mode = turn_previous.map(GameMode::id);
            // The turn's abilities: a mode change made in it set the flags first
            // (`Game::set_mode`), then a mod's writes won, as in vanilla.
            if let Some(now) = turn_abilities
                && now != g.abilities
            {
                g.abilities = now;
                g.send(&g.abilities_packet());
            }
        });
        // LivingEntity.checkTotemDeathProtection's entity event, to every
        // client tracking the guest and its own; this window draws it too.
        if let Some(body) = totem {
            let id = host.hub.guests[i].entity_id;
            host.hub.entity_event(id, crate::game::totem::TOTEM_EVENT);
            if self.player.is_some() {
                self.totem_event(horizon_client::particles::TrackingTarget::Entity(id), body);
            }
        }
        let Some(host) = self.lan.as_mut() else {
            unreachable!("LanHost dropped inside a guest turn; closes are deferred");
        };
        host.play.insert(conn, play);
        let finished = self.lan_guest_swaps.pop();
        debug_assert_eq!(finished, Some(conn));
        host.hub.guests[i].residency.put_back();
        host.play[&conn].hook.finish_loan();
        match self.lan_acting {
            Some(outer) => {
                if let Some(guest) = host.hub.guest(outer) {
                    guest.residency.put_back();
                }
            }
            None => self.resident_entities.local_player.put_back(),
        }
        self.hook.finish_loan();
        if let Some(vehicle) = released_vehicle {
            self.release_removed_vehicle(vehicle);
        }
        self.bind_current_hook_residency();
        for event in turn_events {
            match event {
                MotionEvent::Health(horizon_client::health::HealthEvent::DamageBroadcast {
                    source,
                }) => self.guest_damage_broadcast(conn, source),
                MotionEvent::HurtSound(sound) => self.guest_hurt_sound(conn, sound),
                _ => {}
            }
        }
        self.stat_criteria_changed(&name, stat_events);
        if let Some(pos) = opened
            && !self.lan_open_container(conn, pos)
            && self
                .lan
                .as_ref()
                .is_some_and(|host| !host.hub.guests[i].spectator())
        {
            // Nothing the guest can use opened: its lid closes again.
            self.container_openers_changed(pos, false);
        }
        if let Some(station) = opened_station {
            self.lan_station_opened(conn, station);
        }
        if let Some(edit) = guest_sign {
            self.lan_open_sign_editor(conn, edit);
        }
        if let Some(key) = guest_lectern {
            self.lan_open_lectern(conn, key);
        }
        if let Some(pos) = guest_command_block {
            self.lan_open_command_block(conn, pos);
        }
        if let Some(pos) = opened_table {
            self.lan_open_crafting(conn, pos);
        }
        if let Some(mob) = guest_mount {
            self.lan_open_mount(conn, mob);
        }
        // The turn's teleports are in the hub now: the mods hear them.
        self.flush_teleport_notes();
        if let Some(host) = self.lan.as_ref() {
            refresh_building_player(host, conn);
        }
        // An inner turn can move, kill, or change the mode of a hub-owned body.
        if let (Some(actor), Some(host)) = (self.lan_acting, self.lan.as_ref()) {
            self.lan_building_boxes =
                self.lan_building_boxes
                    .for_turn(actor, None, building_players(host));
        }
        if let Some(payload) = panic {
            std::panic::resume_unwind(payload);
        }
        if let Some(i) = self.lan_interrupted_turns.iter().rposition(|&c| c == conn) {
            self.lan_interrupted_turns.remove(i);
        }
    }

    /// `ServerLevel.broadcastDamageEvent` for a guest that took an unblocked hit.
    pub(super) fn guest_damage_broadcast(
        &mut self,
        conn: u32,
        source: horizon_world::damage_source::DamageTypeId,
    ) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let Some(g) = host.hub.guest(conn) else {
            return;
        };
        let (id, at) = (g.entity_id, g.position);
        let kind = self
            .health_resources
            .damage
            .definition(source)
            .map_or("minecraft:generic", |d| d.name.as_str());
        host.hub.damage_event(id, kind, None, None, at);
    }

    /// `LivingEntity.playHurtSound` for a guest: the server's `Entity.playSound`
    /// reaches every player around but the hurt one, whose own client plays it
    /// from `handleDamageEvent` (arismoko/horizon#4149).
    pub(super) fn guest_hurt_sound(&mut self, conn: u32, sound: &str) {
        let Some(at) = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .map(|g| g.position)
        else {
            return;
        };
        self.audio.play_unshared(sound, Some(at), 1.0, 1.0);
        self.send_level_sound(LevelSound::new(sound, at, 1.0, 1.0), Some(conn));
    }

    /// `ServerLevel.players()` in turn: the host's own player, then each
    /// guest in the level in place with their state swapped in (`as_guest`).
    /// What reaches the players around a point (a beacon's powers, a splash
    /// potion, a warden's darkness, an elder guardian's curse) lands on each
    /// of them, not only on the host. A server has no player of its own.
    pub(in crate::game) fn for_each_player(&mut self, mut f: impl FnMut(&mut Self)) {
        if self.player.is_some() && !self.lan_acting_fake {
            f(self);
        }
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let conns: Vec<u32> = host
            .hub
            .guests
            .iter()
            .filter(|g| host.hub.here(g) && g.loaded && !g.dead)
            .map(|g| g.conn)
            .collect();
        for conn in conns {
            self.as_guest(conn, &mut f);
        }
    }

    /// The game mode fields without `set_mode`'s side effects on the host.
    pub(super) fn set_mode_raw(&mut self, id: i32) {
        self.mode = crate::game::game_mode::mode_from_id(id);
        self.creative = self.mode == GameMode::Creative;
    }
}
