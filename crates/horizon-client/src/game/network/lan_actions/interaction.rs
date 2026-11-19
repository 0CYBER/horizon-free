use super::*;

impl Game<'_> {
    pub(super) fn guest_tick_death(&mut self, gi: usize) {
        let removed = self.lan.as_mut().and_then(|host| {
            host.hub.guests.update(gi, |guest| {
                if !guest.tick_death() || guest.death_time != 20 {
                    return None;
                }
                let dimensions = guest.dimensions();
                Some((
                    guest.entity_id,
                    guest.position,
                    dimensions,
                    guest.death_canceled,
                ))
            })
        });
        if let Some((entity, position, dimensions, death_canceled)) = removed {
            self.lan_entity_event(entity, 60, position);
            self.death_poof(
                position,
                f64::from(dimensions.width),
                f64::from(dimensions.height),
            );
            if !death_canceled {
                self.guest_drop_carried(gi);
            }
        }
    }

    pub(super) fn guest_drop_carried(&mut self, gi: usize) {
        let carried = self
            .lan
            .as_mut()
            .and_then(|host| host.hub.guests.update(gi, |g| g.carried.take()));
        if let Some(c) = carried {
            self.drop_guest_items(gi, vec![c]);
        }
    }

    /// `AbstractContainerMenu.removed`: what a closing player held on the
    /// cursor goes back into their inventory (`placeItemBackInInventory`,
    /// dropped only if it doesn't fit), and their client's cursor empties
    /// with the inventory it is sent.
    pub(in crate::game) fn guest_carried_back(&mut self, gi: usize) {
        let carried = self
            .lan
            .as_mut()
            .and_then(|host| host.hub.guests.update(gi, |g| g.carried.take()));
        if let Some(c) = carried {
            self.lan_place_back(gi, vec![c]);
            if let Some(host) = self.lan.as_mut() {
                host.hub.send_inventory(gi);
            }
        }
    }

    pub(in crate::game) fn drop_guest_items(&mut self, gi: usize, stacks: Vec<ItemStack>) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let g = &host.hub.guests[gi];
        let eye = [
            g.position[0],
            g.position[1] + g.eye_height() - f64::from(0.3f32),
            g.position[2],
        ];
        let (yaw, pitch) = (g.yaw, g.pitch);
        let mut r = horizon_client::random_source::entity(); // the guest's Entity.random
        for stack in stacks {
            let v = horizon_client::entities::drop_motion(&mut r, yaw, pitch, false);
            self.spawn_item(stack, eye, v, 40);
        }
    }

    /// Whether a block outline occludes the straight line from `eye` to `point`.
    pub(super) fn block_between(&self, eye: [f64; 3], point: [f64; 3]) -> bool {
        let d = [point[0] - eye[0], point[1] - eye[1], point[2] - eye[2]];
        let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if dist < 1e-9 {
            return false;
        }
        let dir = [d[0] / dist, d[1] / dist, d[2] / dist];
        self.clip_ray(eye, dir, dist).is_some_and(|h| {
            let hd = [
                h.point[0] - eye[0],
                h.point[1] - eye[1],
                h.point[2] - eye[2],
            ];
            let hit_dist = (hd[0] * hd[0] + hd[1] * hd[1] + hd[2] * hd[2]).sqrt();
            hit_dist < dist - 1e-6
        })
    }

    /// Aim the swapped-in player at an entity; returns where it aims and the
    /// eye. A boat, minecart, armor stand, item frame or painting is aimed at
    /// the point the client hit (the interact packet's location) when that
    /// is on it, else at its centre.
    pub(super) fn look_at_entity(
        &mut self,
        gi: usize,
        id: i32,
        location: Option<[f64; 3]>,
    ) -> Option<([f64; 3], [f64; 3])> {
        let eye = {
            let g = &self.lan.as_ref()?.hub.guests[gi];
            [g.position[0], g.position[1] + g.eye_height(), g.position[2]]
        };
        let object = || {
            let (lo, hi, origin) = self.object_target(id)?;
            let middle = [0, 1, 2].map(|k| (lo[k] + hi[k]) / 2.0);
            let Some(l) = location else {
                // A boat's mobs sit over its middle: aim low at its near
                // side, so the ray meets the boat before them (the server
                // attacks the entity it is sent).
                let carries = u32::try_from(id).is_ok_and(|v| {
                    self.boats.iter().any(|b| b.id == v) && !self.boat_mobs(v).is_empty()
                });
                if carries {
                    let near = |k: usize| eye[k].clamp(lo[k] + 0.01, hi[k] - 0.01);
                    return Some([near(0), lo[1] + 0.05, near(2)]);
                }
                return Some(middle);
            };
            let point = [0, 1, 2].map(|k| origin[k] + l[k]);
            // A little inside the box, so the ray from the eye meets it.
            let inside = [0, 1, 2].map(|k| {
                let margin = ((hi[k] - lo[k]) / 2.0).min(0.01);
                point[k].clamp(lo[k] + margin, hi[k] - margin)
            });
            let on = (0..3).all(|k| (point[k] - inside[k]).abs() < 0.1);
            Some(if on { inside } else { middle })
        };
        let mob_box = self.mobs.iter().find(|m| m.id as i32 == id).map(|m| {
            let b = m.bounds();
            let (mn, mx) = (b.min_position(), b.max_position());
            ([mn.x, mn.y, mn.z], [mx.x, mx.y, mx.z])
        });
        let centre = match mob_box {
            Some((lo, hi)) => visible_aim_point(eye, lo, hi, |p| self.block_between(eye, p)),
            None if self.object_target(id).is_some() => object()?,
            // A dragon part or an end crystal (game/mobs/dragon.rs).
            None => {
                let b = self.tracked_special_box(id)?;
                let (lo, hi) = (b.min_position(), b.max_position());
                [
                    (lo.x + hi.x) / 2.0,
                    (lo.y + hi.y) / 2.0,
                    (lo.z + hi.z) / 2.0,
                ]
            }
        };
        let d = [centre[0] - eye[0], centre[1] - eye[1], centre[2] - eye[2]];
        let horizontal = (d[0] * d[0] + d[2] * d[2]).sqrt();
        self.lan.as_mut()?.hub.guests.update(gi, |g| {
            g.yaw = (-d[0].atan2(d[2])).to_degrees() as f32;
            g.pitch = (-d[1].atan2(horizontal)).to_degrees() as f32;
        });
        Some((centre, eye))
    }

    /// `ServerGamePacketListenerImpl.handlePlayerAction` for STAB: the spear in
    /// the guest's main hand jabs once its charge allows it (with five ticks
    /// of tolerance). The guest's client is sent the dash Lunge gave them.
    pub(super) fn guest_jab(&mut self, conn: u32) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let Some(g) = host.hub.guest(conn) else {
            return;
        };
        let at = g.position;
        let players = self.stab_players_for(conn);
        let sound = self
            .guest_hit_by(conn, |g| {
                if g.cannot_attack_with_item(g.attack_ticks as i32, 5) {
                    None
                } else {
                    let outer = std::mem::replace(&mut g.stab_players, players);
                    let sound = g.jab();
                    g.stab_players = outer;
                    sound
                }
            })
            .flatten();
        self.land_player_stabs();
        // PiercingWeapon.makeSound: `playSound(causer, ...)`, so the guest's
        // own client plays it.
        if let Some(name) = sound {
            self.send_level_sound(LevelSound::new(name, at, 1.0, 1.0), Some(conn));
        }
    }

    /// `Level.getEntityOrPart(id).getBoundingBox()` for a guest's attack or
    /// interaction: the host, another player, a mob, a vehicle or object, a
    /// dragon part or crystal, a projectile. None for an id the level doesn't
    /// have (the handlers then find nothing either).
    /// `WorldBorder.isWithinBounds(target.blockPosition())` for an entity of this box:
    /// the block its feet are in (the box's bottom centre).
    pub(super) fn inside_border(&self, target: &Aabb) -> bool {
        let feet = target.bottom_center();
        self.border.contains(feet.x.floor(), feet.z.floor())
    }

    pub(in crate::game) fn guest_target_box(&self, gi: usize, id: i32) -> Option<Aabb> {
        let host = self.lan.as_ref()?;
        let conn = host.hub.guests[gi].conn;
        if let Some(g) = host
            .hub
            .guests
            .simulation_iter()
            .find(|g| g.entity_id == id && g.conn != conn && host.hub.here(g))
        {
            return Some(g.bounds());
        }
        if let Some(m) = self.mobs.iter().find(|m| m.id as i32 == id) {
            return Some(m.bounds());
        }
        if let Some((lo, hi, _)) = self.object_target(id) {
            return Some(Aabb::new(
                Vec3::new(lo[0], lo[1], lo[2]),
                Vec3::new(hi[0], hi[1], hi[2]),
            ));
        }
        if let Some(b) = self.tracked_special_box(id) {
            return Some(b);
        }
        if let Some(knot) = self.knot_at_id(id) {
            let knot = horizon_client::mobs::leash::knot_state(knot);
            let p = knot.position;
            let half = knot.width / 2.0;
            return Some(Aabb::new(
                Vec3::new(p.x - half, p.y, p.z - half),
                Vec3::new(p.x + half, p.y + knot.height, p.z + half),
            ));
        }
        self.creatures
            .projectiles
            .iter()
            .find(|p| p.id != 0 && p.id as i32 == id)
            .map(|p| p.bounds())
    }

    /// The fence of the knot whose entity id is `id` (`lan_assign_ids`).
    pub(super) fn knot_at_id(&self, id: i32) -> Option<[i32; 3]> {
        self.knot_ids
            .iter()
            .find(|&(_, &knot)| knot as i32 == id)
            .map(|(&fence, _)| fence)
    }

    /// `ServerGamePacketListenerImpl.handleUseItemOn` for guest `conn` (at `gi`) clicking
    /// `hit` with `hand`, after the packet's sequence is acknowledged: reach, the hit's place in
    /// its block, the build height, spawn protection and the border, then
    /// `ServerPlayerGameMode.useItemOn`, and the blocks the client predicted sent back. True
    /// when the use consumed (`InteractionResult` a success or a consume). A fake player's
    /// `PlayerRef::perform_action` runs it as the packet does.
    pub(in crate::game) fn guest_use_item_on(
        &mut self,
        conn: u32,
        gi: usize,
        hit: Hit,
        hand: Hand,
    ) -> Result<bool, ActionRefused> {
        let mut consumed = false;
        let Some(host) = self.lan.as_mut() else {
            return Err(ActionRefused::Stale);
        };
        // handleUseItemOn: within reach, then the hit point within
        // a block of the block's centre (logged when it isn't).
        if !host.hub.guests.update(gi, |g| g.may_reach_block(hit.pos)) {
            return Err(ActionRefused::OutOfReach);
        }
        let inside = (0..3).all(|k| (hit.point[k] - f64::from(hit.pos[k]) - 0.5).abs() < 1.0000001);
        if !inside {
            let d = horizon_core::java_number::double_to_string;
            let at = |k: usize| d(hit.point[k]);
            let line = format!(
                "Rejecting UseItemOnPacket from {}: Location ({}, {}, {}) too far away from hit block BlockPos{{x={}, y={}, z={}}}.",
                host.hub.guests[gi].name,
                at(0),
                at(1),
                at(2),
                hit.pos[0],
                hit.pos[1],
                hit.pos[2]
            );
            self.lan_log_event(horizon_client::lan::hub::HostEvent::Warn(line));
            return Err(ActionRefused::OutOfReach);
        }
        let spectator = self
            .lan
            .as_ref()
            .is_some_and(|host| host.hub.guests[gi].spectator());
        let face = hit.face;
        // handleUseItemOn: above or below the world nothing is used and
        // the player is told the limit (the blocks are not resent);
        // under spawn protection nothing is used or placed; the player
        // is told and the blocks below go back. mayInteract: nothing is
        // used on a block outside the world border either.
        let min_y = self.world.shape.min_y;
        let max_y = self.world.shape.max_y() - 1;
        if hit.pos[1] > max_y {
            self.build_limit_message(conn, true, max_y);
            return Err(ActionRefused::Protected);
        }
        if hit.pos[1] < min_y {
            self.build_limit_message(conn, false, min_y);
            return Err(ActionRefused::Protected);
        }
        let mut refused = None;
        if self.under_spawn_protection(gi, hit.pos) {
            self.spawn_protection_message(conn, hit.pos);
            refused = Some(ActionRefused::Protected);
        } else if self
            .border
            .contains(f64::from(hit.pos[0]), f64::from(hit.pos[2]))
        {
            if spectator && self.spectator_use_portal(conn, gi, hit.pos) {
                return Ok(true);
            }
            let logged = self.world.block_log.as_ref().map_or(0, Vec::len);
            let heard = self.audio.capture.as_ref().map_or(0, Vec::len);
            let used = self
                .as_guest_at(gi, conn, |g| {
                    let attempt = g.was_block_placement_attempt(hand);
                    let saved = (g.forced_hit, g.guest_use);
                    g.forced_hit = Some(hit);
                    g.guest_use = Some(GuestUse::OnBlock(hand));
                    let swings = g.swings_started;
                    let predicted = g.predicted_swings;
                    let consumed = g.with_block_use_original(hand, |g| {
                        let consumed = matches!(g.use_item(), Ok(Some(_)));
                        if consumed && !g.use_failed {
                            g.finish_block_use(hit.pos, hand);
                        }
                        consumed
                    });
                    (g.forced_hit, g.guest_use) = saved;
                    reset_use_attack_strength(&mut g.attack_ticks, swings, g.swings_started);
                    (
                        g.swings_started != swings && g.predicted_swings == predicted,
                        consumed,
                        attempt,
                    )
                })
                .ok();
            consumed = used.is_some_and(|u| u.1);
            let swung = used.map(|u| u.0);
            self.lan_swing_back(conn, swung);
            self.guest_use_sounds(conn, logged, heard);
            // A placement that did nothing at the top or bottom of the
            // world says why. Vanilla asks twice for the top (once
            // before the swing, once in the check that also has the
            // bottom), so a player there is told twice.
            if let Some((_, false, true)) = used {
                if face == [0, 1, 0] && hit.pos[1] >= max_y {
                    self.build_limit_message(conn, true, max_y);
                    self.build_limit_message(conn, true, max_y);
                } else if face == [0, -1, 0] && hit.pos[1] <= min_y {
                    self.build_limit_message(conn, false, min_y);
                }
            }
        } else {
            self.build_limit_message(conn, true, max_y);
            refused = Some(ActionRefused::Protected);
        }
        // The client predicted a result: correct it with what really is.
        let Some(host) = self.lan.as_mut() else {
            return refused.map_or(Ok(consumed), Err);
        };
        for pos in [hit.pos, hit.beside()] {
            let state = self.world.block(pos[0], pos[1], pos[2]).0 as i32;
            host.hub
                .send_to(conn, &C::BlockUpdate(play::BlockUpdate { pos, state }));
        }
        refused.map_or(Ok(consumed), Err)
    }

    /// `ServerGamePacketListenerImpl.handleUseItem` for guest `conn` (at `gi`) using the item in
    /// `hand`, after the packet's sequence is acknowledged: the rotation the client sent, if any
    /// (`absSnapRotationTo`), a spawn egg's own `mayInteract`, then `ServerPlayerGameMode.useItem`.
    /// True when the use consumed (`InteractionResult` a success or a consume). A fake player's
    /// `PlayerRef::perform_action` runs it as the packet does, with no rotation.
    pub(in crate::game) fn guest_use_item(
        &mut self,
        conn: u32,
        gi: usize,
        hand: Hand,
        rotation: Option<(f32, f32)>,
    ) -> Result<bool, ActionRefused> {
        let Some(host) = self.lan.as_ref() else {
            return Err(ActionRefused::Stale);
        };
        // ServerPlayerGameMode.useItem: a spectator's use passes.
        if host.hub.guests[gi].spectator() {
            return Ok(false);
        }
        // handleUseItem: the client's rotation (absSnapRotationTo, which
        // sends nothing back), then the item alone. What it looked at a
        // moment ago (a block, a mob) is UseItemOn's or Interact's.
        let Some(host) = self.lan.as_mut() else {
            return Err(ActionRefused::Stale);
        };
        if let Some((yaw, pitch)) = rotation {
            host.hub
                .guests
                .update(gi, |g| (g.yaw, g.pitch) = (yaw, pitch));
        }
        let g = &host.hub.guests[gi];
        // SpawnEggItem.use calls ServerLevel.mayInteract at its own
        // fluid ray's position, unlike useOn's packet-supplied block.
        let held = if hand == Hand::Off {
            g.offhand.as_ref()
        } else {
            g.inventory.get(g.selected).and_then(Option::as_ref)
        };
        if held.is_some_and(|s| s.item.ends_with("_spawn_egg")) {
            let hit = self
                .as_guest_at(gi, conn, |g| g.with_hand(hand, |g| g.spawn_egg_fluid_hit()))
                .ok()
                .flatten();
            if hit.is_some_and(|h| self.under_spawn_protection(gi, h.pos)) {
                return Err(ActionRefused::Protected);
            }
        }
        let heard = self.audio.capture.as_ref().map_or(0, Vec::len);
        let used = self
            .as_guest_at(gi, conn, |g| {
                // A horn's sound, inline instruments included.
                let horn = g.with_hand(hand, |g| {
                    let held = g.held()?;
                    g.res.items().instrument(held).map(|i| i.sound)
                });
                let saved = g.guest_use;
                g.guest_use = Some(GuestUse::Item(hand));
                let swings = g.swings_started;
                let consumed = matches!(g.use_item(), Ok(Some(_)));
                g.guest_use = saved;
                reset_use_attack_strength(&mut g.attack_ticks, swings, g.swings_started);
                (g.swings_started != swings, horn, consumed)
            })
            .ok();
        let (swung, horn, consumed) = match used {
            Some((swung, horn, consumed)) => (Some(swung), horn, consumed),
            None => (None, None, false),
        };
        self.lan_swing_back(conn, swung);
        self.guest_instrument_sound(conn, heard, horn);
        Ok(consumed)
    }

    /// `handleInteract`'s entity use for a guest or the persistent fake: the border and
    /// interaction range, then that entity and hand only, never an air-use fallback.
    pub(in crate::game) fn guest_interact(
        &mut self,
        conn: u32,
        gi: usize,
        id: i32,
        hand: Hand,
        location: [f64; 3],
    ) -> Result<bool, ActionRefused> {
        if let Some(target) = self.guest_target_box(gi, id) {
            if !self.inside_border(&target) {
                return Err(ActionRefused::Protected);
            }
            if self.as_guest_at(gi, conn, |g| {
                g.within_entity_interaction_range(&target, 3.0)
            }) != Ok(true)
            {
                return Err(ActionRefused::OutOfReach);
            }
        } else {
            return Err(ActionRefused::InvalidTarget);
        }
        // LeashFenceKnotEntity.interact: a knot has no entity here, only its fence.
        if let Some(knot) = self.knot_at_id(id) {
            let used = self
                .as_guest_at(gi, conn, |g| {
                    let swings = g.swings_started;
                    let consumed = g.with_hand(hand, |g| g.use_knot(knot, true));
                    reset_use_attack_strength(&mut g.attack_ticks, swings, g.swings_started);
                    (g.swings_started != swings, consumed)
                })
                .ok();
            self.lan_swing_back(conn, used.map(|u| u.0));
            return Ok(used.is_some_and(|u| u.1));
        }
        let host = self.lan.as_ref().ok_or(ActionRefused::Stale)?;
        let before = host.hub.guests[gi].vehicle;
        // Packet interactions aim the server's ray at the clicked entity. A fake already
        // supplies its own viewpoint; its explicit target must not change that rotation.
        if !host.hub.guests[gi].is_fake() && self.look_at_entity(gi, id, Some(location)).is_none() {
            return Ok(false);
        }
        let target = InteractTarget { id, location };
        let used = self
            .as_guest_at(gi, conn, |g| {
                let saved = g.guest_use;
                let failed = std::mem::replace(&mut g.use_failed, false);
                g.guest_use = Some(GuestUse::Entity(hand, target));
                let swings = g.swings_started;
                let consumed = matches!(g.use_item(), Ok(Some(_))) && !g.use_failed;
                g.guest_use = saved;
                g.use_failed = failed;
                reset_use_attack_strength(&mut g.attack_ticks, swings, g.swings_started);
                (g.swings_started != swings, consumed)
            })
            .ok();
        self.lan_swing_back(conn, used.map(|u| u.0));
        self.guest_boarded(conn, before);
        Ok(used.is_some_and(|u| u.1))
    }

    /// `ServerGamePacketListenerImpl.handleInteract`'s ATTACK for guest `conn` (at `gi`) hitting
    /// entity `id`: the border and the attack range, the native mods' `attack_entity`, then
    /// `Player.attack` for a player, a mob or another entity, and the deflection of a fireball,
    /// a wind charge or a shulker bullet. True when the blow landed. A fake player's
    /// `PlayerRef::perform_action` runs it as the packet does.
    pub(in crate::game) fn guest_attack(
        &mut self,
        conn: u32,
        gi: usize,
        id: i32,
    ) -> Result<bool, ActionRefused> {
        let Some(host) = self.lan.as_ref() else {
            return Err(ActionRefused::Stale);
        };
        if host.hub.guests[gi].spectator() {
            return Ok(false);
        }
        // handleAttack: a target inside the world border, then
        // `isWithinAttackRange(mainHandItem, box, 3.0)` before anything else, players,
        // mobs, objects and fireballs alike.
        if let Some(target) = self.guest_target_box(gi, id) {
            if !self.inside_border(&target) {
                return Err(ActionRefused::Protected);
            }
            if self.as_guest_at(gi, conn, |g| g.within_attack_range(&target)) != Ok(true) {
                return Err(ActionRefused::OutOfReach);
            }
        }
        if self.guest_attack_stops(gi, id) {
            return Ok(false);
        }
        // Native mods, where Player.attack starts: the global `attack_entity`,
        // then the held item's `on_left_click_entity` Part. A canceled blow
        // does nothing (game/native_hooks.rs).
        let Some(host) = self.lan.as_ref() else {
            return Err(ActionRefused::Stale);
        };
        let aimed = if let Some(g) = host
            .hub
            .guests
            .iter()
            .find(|g| g.entity_id == id && g.conn != conn && host.hub.here(g))
        {
            Some(("minecraft:player".to_owned(), g.position))
        } else {
            // The mod hooks hear no natural fence knot (#13036).
            self.level_entity(id)
                .filter(|t| !matches!(t, crate::game::commands::Target::Knot(_)))
                .map(|t| {
                    (
                        self.target_type(t),
                        self.target_position(t).unwrap_or_default(),
                    )
                })
        };
        if let Some((kind, at)) = aimed
            && self.as_guest_at(gi, conn, |g| {
                let mut canceled = false;
                let dimension = g.dim.id();
                g.native_attack_entity(dimension, id, &kind, at, &mut canceled);
                canceled || g.native_left_click_entity_event(id, &kind)
            }) == Ok(true)
        {
            return Ok(false);
        }
        if let Some(victim) = self.pvp_victim(conn, id) {
            if let Ok(Some(strike)) = self.as_guest_at(gi, conn, |g| g.pvp_strike(id as u32))
                && self.pvp_land(victim, &strike)
            {
                let on_ground = self.pvp_victim_on_ground(victim);
                let stops_sprint = strike.stops_sprint();
                let sent = self
                    .as_guest_at(gi, conn, |g| g.pvp_hit_after(on_ground, stops_sprint))
                    .ok()
                    .flatten();
                self.send_guest_motion(conn, sent);
                return Ok(true);
            }
            return Ok(false);
        }
        // A ghast's fireball or a wind charge: turned along the guest's own
        // view (ServerGamePacketListenerImpl.handleInteract, then
        // Player.attack's AIM_DEFLECT).
        if let Some(i) = self
            .creatures
            .projectiles
            .iter()
            .position(|p| p.id != 0 && p.id as i32 == id && crate::game::deflect::attackable(p))
        {
            return Ok(self.as_guest_at(gi, conn, |g| {
                if !g.within_interaction_range(i) {
                    return false;
                }
                if matches!(
                    g.creatures.projectiles[i].kind,
                    horizon_client::projectiles::ProjectileKind::ShulkerBullet { .. }
                ) {
                    g.hit_shulker_bullet(i);
                } else {
                    g.deflect(i);
                }
                true
            }) == Ok(true));
        }
        if let Some(index) = self
            .mobs
            .iter()
            .position(|m| m.id as i32 == id && m.alive())
            .filter(|_| self.dragon.as_ref().is_none_or(|d| d.id as i32 != id))
        {
            let Some(host) = self.lan.as_ref() else {
                return Err(ActionRefused::Stale);
            };
            if host.hub.guests[gi].vehicle == Some(self.mobs[index].id) {
                return Ok(false);
            }
            // A mace's smash sets the attacker's velocity and sends it at once, before the
            // Wind Burst's push (MaceItem.hurtEnemy, then the post-attack effects).
            let sent = self
                .as_guest_at(gi, conn, |g| {
                    let marked = g
                        .player
                        .as_mut()
                        .is_some_and(|p| std::mem::take(&mut p.health.sync_velocity));
                    g.attack_mob(index);
                    g.swing_arm();
                    let p = g.player.as_mut()?;
                    let sent = std::mem::replace(&mut p.health.sync_velocity, marked);
                    sent.then_some(p.velocity)
                })
                .ok()
                .flatten();
            self.send_guest_motion(conn, sent);
            // The mob now answers to this guest (game/network/lan_mobs.rs).
            if let Ok(id) = u32::try_from(id)
                && let Some(host) = self.lan.as_mut()
                && host.hub.guests[gi].participates_in_player_simulation()
            {
                host.mob_players.insert(id, conn);
            }
            return Ok(true);
        }
        let Some(host) = self.lan.as_ref() else {
            return Err(ActionRefused::Stale);
        };
        let (old_yaw, old_pitch) = (host.hub.guests[gi].yaw, host.hub.guests[gi].pitch);
        let boat = self.boats.iter().position(|boat| boat.id as i32 == id);
        // A packet names its target already. Keep the attacker's actual yaw
        // for the boat's knockback instead of aiming a ray at its center.
        if boat.is_none() && self.look_at_entity(gi, id, None).is_none() {
            return Err(ActionRefused::InvalidTarget);
        }
        // A mace's smash sets the attacker's velocity and sends it at once, before the
        // Wind Burst's push (MaceItem.hurtEnemy, then the post-attack effects).
        let sent = self
            .as_guest_at(gi, conn, |g| {
                let marked = g
                    .player
                    .as_mut()
                    .is_some_and(|p| std::mem::take(&mut p.health.sync_velocity));
                if let Some(index) = boat {
                    g.strike_boat(index);
                    g.swing_arm();
                } else {
                    g.attack();
                }
                let p = g.player.as_mut()?;
                let sent = std::mem::replace(&mut p.health.sync_velocity, marked);
                sent.then_some(p.velocity)
            })
            .ok()
            .flatten();
        self.send_guest_motion(conn, sent);
        // The mob now answers to this guest (game/network/lan_mobs.rs).
        let Some(host) = self.lan.as_mut() else {
            return Err(ActionRefused::Stale);
        };
        if host.hub.guests[gi].participates_in_player_simulation()
            && let Ok(id) = u32::try_from(id)
        {
            host.mob_players.insert(id, conn);
        }
        {
            let value = old_yaw;
            host.hub.guests.update(gi, |guest| guest.yaw = value);
        }
        {
            let value = old_pitch;
            host.hub.guests.update(gi, |guest| guest.pitch = value);
        }
        Ok(true)
    }
}
