//! Object-owned passenger memory and typed transfer loading.
use super::*;

impl Game<'_> {
    pub(super) fn mob_at(&self, object: MobObject) -> &Mob {
        match object {
            MobObject::Resident(index) => &self.mobs[index],
            MobObject::OffLevel(id) => self.persist.off_level[&id]
                .as_mob()
                .expect("mob object slot"),
        }
    }

    pub(super) fn mob_at_mut(&mut self, object: MobObject) -> &mut Mob {
        match object {
            MobObject::Resident(index) => &mut self.mobs[index],
            MobObject::OffLevel(id) => self
                .persist
                .off_level
                .get_mut(&id)
                .and_then(OffLevel::as_mob_mut)
                .expect("mob object slot"),
        }
    }

    /// Internal object lookup, including a passenger refused level residency.
    /// This is deliberately separate from `level_entity` and native handle queries.
    pub(in crate::game) fn mob_object(&self, id: u32) -> Option<&Mob> {
        self.mobs
            .iter()
            .find(|mob| mob.id == id)
            .or_else(|| self.persist.off_level.get(&id).and_then(OffLevel::as_mob))
    }

    /// Mutate an extant mob object without making it resident.
    pub(in crate::game) fn mob_object_mut(&mut self, id: u32) -> Option<&mut Mob> {
        if let Some(index) = self.mobs.iter().position(|mob| mob.id == id) {
            return Some(&mut self.mobs[index]);
        }
        self.persist
            .off_level
            .get_mut(&id)
            .and_then(OffLevel::as_mob_mut)
    }

    /// A boat object, resident or retained by a passenger graph.
    /// Selectors, tracking and ticking keep iterating `boats` instead.
    pub(in crate::game) fn boat_object(&self, id: u32) -> Option<&crate::game::boats::Boat> {
        self.boats
            .iter()
            .find(|boat| boat.id == id)
            .or_else(|| self.persist.off_level.get(&id).and_then(OffLevel::as_boat))
    }

    /// A minecart object, resident or retained by a passenger graph.
    /// Selectors, tracking and ticking keep iterating `carts` instead.
    pub(in crate::game) fn cart_object(
        &self,
        id: u32,
    ) -> Option<&crate::game::minecarts::Minecart> {
        self.carts
            .iter()
            .find(|cart| cart.id == id)
            .or_else(|| self.persist.off_level.get(&id).and_then(OffLevel::as_cart))
    }

    /// Internal topology only: resident objects followed by retained passengers.
    /// Callers needing level membership must continue to iterate `mobs` instead.
    pub(in crate::game) fn mob_objects(&self) -> impl Iterator<Item = &Mob> {
        self.mobs
            .iter()
            .chain(self.persist.off_level.values().filter_map(OffLevel::as_mob))
    }

    pub(in crate::game) fn load_item_transferred(
        &mut self,
        tag: Tag,
        uuid: Option<[i32; 4]>,
        transferred: Option<crate::game::entity_initialization::TransferredEntityState>,
    ) {
        self.next_entity += 1;
        // `ItemEntity(EntityType, Level)` first: its own random draws UUID, bobOffs and yaw.
        let mut own = horizon_core::random::EntityRandom::own(random_source::entity());
        let fresh = horizon_client::save::insecure_uuid(&mut own);
        let bob = own.next_float() * std::f32::consts::PI * 2.0;
        own.next_float();
        if let Some(mut e) = entity_nbt::item_entity_from_nbt(&tag, self.next_entity, bob) {
            self.clean_stack(&mut e.stack);
            e.random = own;
            if let Some(state) = transferred {
                self.adopt_transferred_entity_state(e.id, state);
            }
            self.remember(e.id, Some(uuid.unwrap_or(fresh)), tag);
            self.bind_saved_entity_attachments(e.id, &mut e.attachment_lifetime);
            self.dropped.push(e);
        }
    }

    pub(in crate::game) fn load_orb_transferred(
        &mut self,
        tag: Tag,
        uuid: Option<[i32; 4]>,
        mut transferred: Option<crate::game::entity_initialization::TransferredEntityState>,
    ) {
        for (i, mut orb) in entity_nbt::orbs_from_nbt(&tag).into_iter().enumerate() {
            self.next_entity += 1;
            orb.id = self.next_entity;
            let id = orb.id;
            // Horizon splits a saved `count` into orbs: the first is the saved one.
            if i == 0 {
                if let Some(state) = transferred.take() {
                    self.adopt_transferred_entity_state(id, state);
                }
                self.remember_uuid(id, uuid, &tag);
                self.bind_saved_entity_attachments(id, &mut orb.attachment_lifetime);
            }
            // An orb that came through a portal keeps its cooldown.
            self.restore_portal_cooldown(id, &tag);
            self.orbs.push(orb);
            self.remember_tags(id, &tag);
        }
    }

    pub(in crate::game) fn load_arrow_transferred(
        &mut self,
        tag: Tag,
        uuid: Option<[i32; 4]>,
        player_uuid: Option<[i32; 4]>,
        transferred: Option<crate::game::entity_initialization::TransferredEntityState>,
    ) {
        if let Some((mut p, in_block)) =
            entity_nbt::arrow_from_nbt(&tag, player_uuid, crate::game::dispenser::tipped_effect)
        {
            if let Some(a) = p.arrow.as_mut() {
                a.last_state = in_block.and_then(|t| self.state_from_nbt(&t));
                self.clean_stack(&mut a.pickup_item);
            }
            self.claim_owner(&mut p);
            // An arrow that came through a portal keeps its
            // cooldown: it does not go straight back through.
            self.next_entity += 1;
            p.id = self.next_entity;
            self.restore_portal_cooldown(p.id, &tag);
            self.remember_tags(p.id, &tag);
            if let Some(state) = transferred {
                self.adopt_transferred_entity_state(p.id, state);
            }
            let own_uuid = p.own_uuid(uuid);
            self.remember_uuid(p.id, Some(own_uuid), &tag);
            self.bind_saved_entity_attachments(p.id, &mut p.attachment_lifetime);
            self.add_projectile(p);
        }
    }

    pub(in crate::game) fn load_spit_or_bullet_transferred(
        &mut self,
        tag: Tag,
        uuid: Option<[i32; 4]>,
        player_uuid: Option<[i32; 4]>,
        transferred: Option<crate::game::entity_initialization::TransferredEntityState>,
    ) {
        let target = self.bullet_target(&tag);
        if let Some(mut p) = entity_nbt::spit_or_bullet_from_nbt(&tag, player_uuid, target) {
            self.claim_owner(&mut p);
            self.next_entity += 1;
            p.id = self.next_entity;
            self.restore_portal_cooldown(p.id, &tag);
            if let Some(state) = transferred {
                self.adopt_transferred_entity_state(p.id, state);
            }
            let own_uuid = p.own_uuid(uuid);
            self.remember_uuid(p.id, Some(own_uuid), &tag);
            self.bind_saved_entity_attachments(p.id, &mut p.attachment_lifetime);
            self.add_projectile(p);
            self.remember_last_projectile_tags(&tag);
        }
    }

    pub(in crate::game) fn load_fireball_transferred(
        &mut self,
        tag: Tag,
        uuid: Option<[i32; 4]>,
        player_uuid: Option<[i32; 4]>,
        transferred: Option<crate::game::entity_initialization::TransferredEntityState>,
    ) {
        if let Some(mut p) = entity_nbt::fireball_from_nbt(&tag, player_uuid) {
            self.claim_owner(&mut p);
            // A projectile that came through a portal keeps its cooldown.
            self.next_entity += 1;
            p.id = self.next_entity;
            self.restore_portal_cooldown(p.id, &tag);
            if let Some(state) = transferred {
                self.adopt_transferred_entity_state(p.id, state);
            }
            let own_uuid = p.own_uuid(uuid);
            self.remember_uuid(p.id, Some(own_uuid), &tag);
            self.bind_saved_entity_attachments(p.id, &mut p.attachment_lifetime);
            self.add_projectile(p);
            self.remember_last_projectile_tags(&tag);
        }
    }

    pub(in crate::game) fn load_throwable_transferred(
        &mut self,
        tag: Tag,
        uuid: Option<[i32; 4]>,
        player_uuid: Option<[i32; 4]>,
        transferred: Option<crate::game::entity_initialization::TransferredEntityState>,
    ) {
        if let Some(mut p) = entity_nbt::throwable_from_nbt(&tag, player_uuid) {
            self.claim_owner(&mut p);
            // A projectile that came through a portal keeps its cooldown.
            self.next_entity += 1;
            p.id = self.next_entity;
            self.restore_portal_cooldown(p.id, &tag);
            if let Some(state) = transferred {
                self.adopt_transferred_entity_state(p.id, state);
            }
            let own_uuid = p.own_uuid(uuid);
            self.remember_uuid(p.id, Some(own_uuid), &tag);
            self.bind_saved_entity_attachments(p.id, &mut p.attachment_lifetime);
            self.add_projectile(p);
            self.remember_last_projectile_tags(&tag);
        }
    }

    pub(in crate::game) fn load_falling_transferred(
        &mut self,
        tag: Tag,
        uuid: Option<[i32; 4]>,
        transferred: Option<crate::game::entity_initialization::TransferredEntityState>,
    ) {
        if let Some((state, position, velocity, age, fall_distance)) =
            entity_nbt::falling_block_from_nbt(&tag)
            && let Some(state) =
                entity_nbt::falling_state_from_nbt(self.pack.assets.registry, &state)
        {
            let (no_gravity, drop_item) = entity_nbt::falling_block_flags(&tag);
            self.next_entity += 1;
            // A summoned one has no UUID yet: `Entity` makes a random one.
            if let Some(state) = transferred {
                self.adopt_transferred_entity_state(self.next_entity, state);
            }
            self.remember_uuid(self.next_entity, uuid, &tag);
            let uuid = self.uuid_for(self.next_entity);
            self.falling.push(FallingBlock {
                attachment_lifetime: Default::default(),
                // Entity.load: the saved Rotation, as the save writes it back.
                rotation: entity_nbt::loaded_rotation(&tag),
                join: Default::default(),
                id: self.next_entity,
                state,
                // DATA_START_POS is not persisted by vanilla.
                start_pos: horizon_core::position::BlockPos::default(),
                position,
                previous: position,
                velocity,
                age,
                no_gravity,
                portal: horizon_client::nether_portal::PortalState {
                    cooldown: horizon_client::save::number(tag.get("PortalCooldown")).unwrap_or(0)
                        as i32,
                    process: None,
                },
                drop_item,
                fall_distance,
                on_ground: entity_nbt::flag(&tag, "OnGround"),
                stuck: Vec3::ZERO,
                damage: horizon_client::falling_block::Damage::from_nbt(
                    &tag,
                    self.res.facts().get(state).name.ends_with("anvil"),
                ),
                saved: Some(Box::new((uuid, tag.clone()))),
            });
        }
    }

    pub(in crate::game) fn load_trident_transferred(
        &mut self,
        tag: Tag,
        uuid: Option<[i32; 4]>,
        transferred: Option<crate::game::entity_initialization::TransferredEntityState>,
    ) {
        if let Some(n) = entity_nbt::trident_from_nbt(&tag) {
            let mut t =
                crate::game::trident::ThrownTrident::from_nbt(n, &tag, self.pack.assets.registry);
            t.read_loyalty(&world_stations!(self).enchantments, &mut self.world.random);
            t.id = self.take_entity_id();
            // One that came through a portal keeps its cooldown.
            let id = t.id;
            self.tridents.push(t);
            if let Some(state) = transferred {
                self.adopt_transferred_entity_state(id, state);
            }
            self.remember(id, uuid, tag);
        }
    }

    pub(in crate::game) fn load_rocket_transferred(
        &mut self,
        tag: Tag,
        uuid: Option<[i32; 4]>,
        player_uuid: Option<[i32; 4]>,
        transferred: Option<crate::game::entity_initialization::TransferredEntityState>,
    ) {
        if let Some(r) = entity_nbt::firework_rocket_from_nbt(&tag, player_uuid) {
            let id = self.take_entity_id();
            let mut rocket = crate::game::fireworks::Rocket::from_nbt(r);
            rocket.id = id;
            // One that came through a portal keeps its cooldown.
            self.restore_portal_cooldown(id, &tag);
            self.rockets.push(rocket);
            if let Some(state) = transferred {
                self.adopt_transferred_entity_state(id, state);
            }
            self.remember(id, uuid, tag);
        }
    }

    pub(in crate::game) fn load_tnt_transferred(
        &mut self,
        tag: Tag,
        uuid: Option<[i32; 4]>,
        transferred: Option<crate::game::entity_initialization::TransferredEntityState>,
    ) {
        self.load_tnt_with_portal(tag, uuid, transferred, false);
    }

    pub(in crate::game) fn load_tnt_with_portal(
        &mut self,
        tag: Tag,
        uuid: Option<[i32; 4]>,
        transferred: Option<crate::game::entity_initialization::TransferredEntityState>,
        used_portal: bool,
    ) {
        if uuid.is_some_and(|u| self.target_with_uuid(u).is_some()) {
            return;
        }
        let mut tag = tag;
        let riders = passenger_graph::take_passengers(&mut tag);
        if let Some(position) = entity_nbt::position(&tag) {
            self.next_entity += 1;
            let id = self.next_entity;
            // getShortOr uses NumericTag.shortValue: floor floating values
            // to int before short narrowing; integer tags narrow directly.
            let fuse = tnt_fuse(&tag);
            // A fresh load: Entity.load's angles. A portal arrival:
            // teleportSetPosition's `setYRot`/`setXRot` of the turned angles,
            // with no `% 360` on the yaw.
            let rotation = if used_portal {
                let [yaw, pitch] = entity_nbt::rotation(&tag);
                [
                    if yaw.is_finite() { yaw } else { 0.0 },
                    if pitch.is_finite() {
                        (pitch % 360.0).clamp(-90.0, 90.0)
                    } else {
                        0.0
                    },
                ]
            } else {
                entity_nbt::loaded_rotation(&tag)
            };
            self.tnts.push(crate::game::tnt::PrimedTnt {
                used_portal,
                attachment_lifetime: Default::default(),
                rotation,
                join: Default::default(),
                no_gravity: entity_nbt::flag(&tag, "NoGravity"),
                id,
                position,
                previous: position,
                velocity: entity_nbt::motion(&tag),
                fuse,
                on_ground: entity_nbt::flag(&tag, "OnGround"),
                fall_distance: horizon_client::save::float(tag.get("fall_distance")).unwrap_or(0.0),
                stuck: Vec3::ZERO,
                in_water: false,
                owner: None,
                guest: None,
                // PrimedTnt.readAdditionalSaveData: 4 by default,
                // kept to 0..128.
                power: entity_nbt::tnt_power(&tag),
                block_state: tag
                    .get("block_state")
                    .and_then(|v| entity_nbt::state_from_nbt(self.pack.assets.registry, v)),
            });
            // `Entity.load`: the `PortalCooldown` of one that just came through,
            // which `handlePortal` counts down every tick.
            self.restore_portal_cooldown(id, &tag);
            if let Some(state) = transferred {
                self.adopt_transferred_entity_state(id, state);
            }
            self.remember(id, uuid, tag);
            if let Some(riders) = riders {
                self.load_passengers(id, riders);
            }
        }
    }

    pub(in crate::game) fn load_boat_transferred(
        &mut self,
        tag: Tag,
        uuid: Option<[i32; 4]>,
        k: &str,
        transferred: Option<crate::game::entity_initialization::TransferredEntityState>,
    ) -> Option<u32> {
        // A runtime transfer owns a distinct live object: like a Mob, the copy
        // refused residency stays below the passengers admitted before it.
        // A disk copy with a taken UUID is still not constructed.
        if transferred.is_none() && uuid.is_some_and(|u| self.target_with_uuid(u).is_some()) {
            return None;
        }
        let refused = transferred.is_some() && self.uuid_admission_refused(uuid);
        if let Some((position, velocity, yaw)) = entity_nbt::boat_from_nbt(&tag) {
            let mut tag = tag;
            let riders = passenger_graph::take_passengers(&mut tag);
            self.next_entity += 1;
            let id = self.next_entity;
            let (wood, chest) = crate::game::boats::any_boat_wood(k).unwrap_or(("oak", false));
            let mut b =
                crate::game::boats::Boat::new(id, std::sync::Arc::from(wood), position, yaw);
            b.no_gravity = entity_nbt::flag(&tag, "NoGravity");
            b.on_ground = entity_nbt::flag(&tag, "OnGround");
            b.fire_ticks = crate::game::boats::saved_fire(&tag);
            b.ticks_frozen = crate::game::boats::saved_ticks_frozen(&tag);
            b.pitch = entity_nbt::rotation(&tag)[1];
            // Entity.load: OnGround and fall_distance, as saved.
            (b.on_ground, b.fall_distance) = entity_nbt::ground_from_nbt(&tag);
            if chest {
                b = b.with_chest();
                b.items = horizon_client::save::decode_items(
                    tag.get("Items"),
                    crate::game::boats::CHEST_SLOTS,
                );
                b.items
                    .iter_mut()
                    .flatten()
                    .for_each(|stack| self.clean_stack(stack));
            }
            b.velocity = velocity;
            // AbstractBoat.readAdditionalSaveData: the lead's holder.
            b.leash_pending = horizon_client::mobs::leash::pending_from_nbt(tag.get("leash"));
            if let Some(state) = transferred {
                self.adopt_transferred_entity_state(id, state);
            }
            self.remember(id, uuid, tag);
            if refused {
                self.persist.off_level.insert(id, OffLevel::Boat(b));
            } else {
                self.bind_saved_entity_attachments(id, &mut b.attachment_lifetime);
                self.boats.push(b);
            }
            if let Some(riders) = riders {
                self.load_passengers(id, riders);
            }
            return Some(id);
        }
        None
    }

    pub(in crate::game) fn load_cart_transferred(
        &mut self,
        tag: Tag,
        uuid: Option<[i32; 4]>,
        k: &str,
        transferred: Option<crate::game::entity_initialization::TransferredEntityState>,
    ) -> Option<u32> {
        // As `load_boat_transferred`: only a runtime transfer retains a refused copy.
        if transferred.is_none() && uuid.is_some_and(|u| self.target_with_uuid(u).is_some()) {
            return None;
        }
        let refused = transferred.is_some() && self.uuid_admission_refused(uuid);
        let kind = crate::game::minecarts::CartKind::from_id(k).unwrap();
        let mut tag = tag;
        let riders = passenger_graph::take_passengers(&mut tag);
        let id = self.next_entity + 1;
        if let Some(mut c) = crate::game::minecarts::Minecart::from_nbt(&tag, id, kind) {
            c.items
                .iter_mut()
                .flatten()
                .for_each(|stack| self.clean_stack(stack));
            crate::game::minecarts::forget_unreadable_display(&mut c, |t| {
                self.state_from_nbt(t).is_some()
            });
            self.next_entity = id;
            if let Some(state) = transferred {
                self.adopt_transferred_entity_state(id, state);
            }
            self.remember(id, uuid, tag);
            if refused {
                self.persist.off_level.insert(id, OffLevel::Cart(c));
            } else {
                self.bind_saved_entity_attachments(id, &mut c.attachment_lifetime);
                self.carts.push(c);
            }
            if let Some(riders) = riders {
                self.load_passengers(id, riders);
            }
            return Some(id);
        }
        None
    }

    /// `PersistentEntitySectionManager.addEntityUuid`: another resident object
    /// already holds `uuid`, so this one is refused level residency.
    pub(in crate::game) fn uuid_admission_refused(&self, uuid: Option<[i32; 4]>) -> bool {
        uuid.is_some_and(|uuid| {
            self.target_with_uuid(uuid)
                .is_some_and(|target| target != crate::game::commands::Target::Player)
        })
    }

    pub(in crate::game) fn load_mob_transferred(
        &mut self,
        tag: Tag,
        mount: Option<u32>,
        transferred: Option<crate::game::entity_initialization::TransferredEntityState>,
    ) -> Option<u32> {
        let uuid = entity_nbt::uuid(&tag);
        // Only disk reconstruction coalesces the player's saved copy with a
        // chunk copy. An explicit runtime token owns a distinct live object.
        if transferred.is_none()
            && let Some(id) = uuid.and_then(|u| self.restored_vehicle(u))
        {
            return Some(id);
        }
        self.next_entity += 1;
        // Structure mobs that generation left to finish: saves from before
        // they were finalized at placement, or a placement that could not.
        let mut tag = tag;
        Self::normalize_loaded_mob_tags(&mut tag);
        let mut finalize = false;
        if let Tag::Compound(c) = &mut tag {
            finalize = c.remove(&"horizon:finalize".into()).is_some();
        }
        let riders = passenger_graph::take_passengers(&mut tag);
        // The mob's own random makes its UUID, kept unless the tag has one.
        let (own, fresh) = horizon_client::mobs::living_random();
        let uuid = Some(uuid.unwrap_or(fresh));
        let mut m = entity_nbt::mob_from_nbt_with_random(
            &tag,
            self.res.mob_types(),
            self.next_entity,
            Some(self.pack.assets.registry),
            own,
        )?;
        m.clean_saved_stacks(&tag, &self.items().component_defaults);
        self.resolve_loaded_profession(&mut m);
        if horizon_client::mobs::camel::is_camel(m.species()) {
            horizon_client::mobs::camel::finish_load(&mut m, self.world.game_time);
        }
        // `NeutralMob.readPersistentAngerSaveData`: the anger runs until its
        // end time, and the mob is angry at the target again, through
        // `Mob.setTarget` -> `asValidTarget`: a player the golem cannot attack
        // (`IronGolem.canAttack`) comes back as no target.
        m.extra.anger = horizon_client::mobs::anger_left(m.extra.anger_end, self.world.game_time);
        let anger_uuid = match tag.get("angry_at") {
            Some(Tag::IntArray(words)) if words.len() == 4 => {
                Some([words[0], words[1], words[2], words[3]])
            }
            _ => None,
        };
        // These are the vanilla NeutralMob implementors; each calls
        // readPersistentAngerSaveData, which invokes setTarget even when
        // `angry_at` is absent or cannot resolve to a living entity.
        let neutral = matches!(
            m.species(),
            horizon_client::mobs::Species::Wolf
                | horizon_client::mobs::Species::IronGolem
                | horizon_client::mobs::Species::Bee
                | horizon_client::mobs::Species::PolarBear
                | horizon_client::mobs::Species::Enderman
                | horizon_client::mobs::Species::ZombifiedPiglin
        );
        // Restore on the server whether or not a mod listens. The saved UUID
        // can name an unloaded entity, an online player, or another mob;
        // substituting the local player when there is no listener both changes
        // vanilla behavior and makes loading depend on mod subscriptions.
        let restore_target =
            neutral && self.is_logical_server() && self.native_lifecycle.epoch().is_some();
        m.extra.mount = mount;
        m.extra.passenger = mount.is_some();
        // The object id preserves load order even when some passengers are
        // retained in the off-level store rather than the resident list.
        m.extra.vehicle_order = i64::from(m.id);
        if finalize {
            let at = [m.position.x, m.position.y, m.position.z];
            let difficulty = self.local_difficulty(at);
            self.finalize_plain(
                &mut m,
                &difficulty,
                horizon_client::mobs::spawn_rules::Reason::Structure,
            );
            // `Shulker.finalizeSpawn` ends in yaw 0 here as on the other spawn paths.
            horizon_client::mobs::shulker::finalize_spawn(&mut m);
        }
        horizon_client::mobs::armor::prepare(&m, &self.motion.attributes);
        let id = m.id;
        // PersistentEntitySectionManager.addEntityUuid refuses residency, not
        // the already-constructed object or its passenger links. Descendants
        // are still considered individually in preorder.
        let refused = self.uuid_admission_refused(uuid);

        if let Some(mut state) = transferred {
            state.restore_mob(&mut m);
            self.adopt_transferred_entity_state(id, state);
        } else if m.health <= 0.0 {
            // A saved dying body has already passed die; loading only resumes tickDeath.
            self.died.insert(id);
        }
        self.remember(id, uuid, tag);
        if refused {
            self.persist.off_level.insert(id, OffLevel::Mob(m));
        } else {
            self.bind_saved_entity_attachments(id, &mut m.attachment_lifetime);
            self.mobs.push(m);
        }
        if !refused && restore_target {
            self.native_restore_loaded_target(id, anger_uuid);
        }
        if let Some(riders) = riders {
            self.load_passengers(id, riders);
        }
        Some(id)
    }

    /// `Entity.shouldBeSaved`: the mobs riding entity `id` under `tag`'s
    /// `Passengers`, after the passengers it kept from loading. Discarded
    /// passengers cannot save even before their list entry is swept.
    pub(super) fn mob_passenger_objects(&self, id: u32) -> Vec<MobObject> {
        let mut riders: Vec<(i64, u32, MobObject)> = self
            .mobs
            .iter()
            .enumerate()
            .filter(|(_, mob)| mob.extra.mount == Some(id) && !mob.removed)
            .map(|(index, mob)| (mob.extra.vehicle_order, mob.id, MobObject::Resident(index)))
            .chain(
                self.persist
                    .off_level
                    .iter()
                    .filter_map(|(&object_id, object)| {
                        let mob = object.as_mob()?;
                        (mob.extra.mount == Some(id) && !mob.removed).then_some((
                            mob.extra.vehicle_order,
                            mob.id,
                            MobObject::OffLevel(object_id),
                        ))
                    }),
            )
            .collect();
        riders.sort_by_key(|&(order, id, _)| (order, id));
        if let Some(driver) = self.mob_object(id).and_then(|mob| mob.extra.rider)
            && let Some(position) = riders.iter().position(|&(_, id, _)| id == driver)
            && position != 0
        {
            let driver = riders.remove(position);
            riders.insert(0, driver);
        }
        riders.into_iter().map(|(_, _, object)| object).collect()
    }

    pub(in crate::game) fn mob_passenger_ids(&self, root: u32) -> Vec<u32> {
        self.mob_passenger_objects(root)
            .into_iter()
            .map(|object| self.mob_at(object).id)
            .collect()
    }

    /// `Entity.teleportCrossDimension`: pair each serialized node with that
    /// exact object's runtime ownership in one walk, never by UUID or Vec order.
    pub(in crate::game) fn take_vehicle_tree_transfer(
        &mut self,
        root: u32,
    ) -> Option<(Tag, TransferredVehicleTree)> {
        let (kind, mut tag) = if let Some(object) = self.mob_object_slot(root) {
            (VehicleTreeKind::Mob, self.mob_object_nbt(object))
        } else if self.boat_object(root).is_some() {
            let uuid = self.uuid_for(root);
            let mut tag = self.boat_object(root)?.to_nbt(uuid, self.persist.raw.get(&root));
            if let Some(index) = self.boats.iter().position(|boat| boat.id == root) {
                self.write_boat_leash(&mut tag, index);
            }
            self.save_portal_cooldown(root, &mut tag);
            (VehicleTreeKind::Boat, tag)
        } else if self.cart_object(root).is_some() {
            let uuid = self.uuid_for(root);
            let mut tag = self.cart_object(root)?.to_nbt(uuid, self.persist.raw.get(&root));
            self.save_portal_cooldown(root, &mut tag);
            (VehicleTreeKind::Cart, tag)
        } else {
            let target = self.level_entity(root as i32)?;
            (VehicleTreeKind::Other, self.command_target_nbt(target)?)
        };
        let name = entity_nbt::kind(&tag)?;
        passenger_graph::take_passengers(&mut tag);
        write_attachments(&mut tag, root, &self.entity_attachments);
        let mut tags = Vec::new();
        let mut passengers = Vec::new();
        for slot in self.ordered_passenger_slots(root) {
            match slot {
                PassengerSlot::Live(id) => {
                    let (tag, state) = self.take_vehicle_tree_transfer(id)?;
                    tags.push(tag);
                    passengers.push(Some(state));
                }
                PassengerSlot::Saved(tag) => {
                    tags.push(tag);
                    passengers.push(None);
                }
            }
        }
        add_passengers(&mut tag, tags);
        Some((
            tag,
            TransferredVehicleTree {
                source_id: root,
                kind,
                name,
                state: self.take_transferred_entity_state(root),
                passengers,
            },
        ))
    }

    pub(super) fn prepare_vehicle_transfer(
        &mut self,
        mut tag: Tag,
        tree: TransferredVehicleTree,
    ) -> Option<PreparedVehicleTransfer> {
        if entity_nbt::kind(&tag).as_deref() != Some(tree.name.as_str()) {
            log::warn!(
                "Horizon: transferred vehicle type does not match its captured object state"
            );
            return None;
        }
        let tags = match passenger_graph::take_passengers(&mut tag) {
            Some(Tag::List(tags)) => tags,
            _ => Vec::new(),
        };
        if tags.len() != tree.passengers.len() {
            log::warn!(
                "Horizon: transferred vehicle topology does not match its captured object state"
            );
            return None;
        }
        let passengers = tags
            .into_iter()
            .zip(tree.passengers)
            .map(|(tag, child)| match child {
                Some(child) => self
                    .prepare_vehicle_transfer(tag, child)
                    .map(PreparedVehiclePassenger::Object),
                None => Some(PreparedVehiclePassenger::Saved(tag)),
            })
            .collect::<Option<Vec<_>>>()?;
        Some(PreparedVehicleTransfer {
            source_id: tree.source_id,
            tag,
            kind: tree.kind,
            name: tree.name,
            state: tree.state,
            passengers,
        })
    }

    /// Reconstruct using actual object IDs, even when the copied root is refused
    /// residency. `Entity.teleportCrossDimension` admits every passenger before
    /// its copied vehicle, whether that root is a mob, boat or minecart.
    pub(in crate::game) fn restore_vehicle_tree_state(
        &mut self,
        tag: Tag,
        tree: TransferredVehicleTree,
    ) -> Option<RestoredVehicleTree> {
        let prepared = self.prepare_vehicle_transfer(tag, tree)?;
        let mut by_source = HashMap::new();
        let root = self.restore_vehicle_transfer(prepared, &mut by_source)?;
        Some(RestoredVehicleTree { root, by_source })
    }

    pub(super) fn restore_vehicle_transfer(
        &mut self,
        tree: PreparedVehicleTransfer,
        by_source: &mut HashMap<u32, u32>,
    ) -> Option<u32> {
        let PreparedVehicleTransfer {
            source_id,
            tag,
            kind,
            name,
            state,
            passengers,
        } = tree;
        let riders = passengers
            .into_iter()
            .map(|child| match child {
                PreparedVehiclePassenger::Object(child) => self
                    .restore_vehicle_transfer(child, by_source)
                    .map(PassengerSlot::Live),
                PreparedVehiclePassenger::Saved(tag) => Some(PassengerSlot::Saved(tag)),
            })
            .collect::<Option<Vec<_>>>()?;
        let root = self.restore_vehicle_object(tag, kind, &name, state)?;
        for slot in riders {
            match slot {
                PassengerSlot::Live(id) => self.link_loaded_passenger(id, root),
                // A slot with no source object still takes ordinary admission in
                // the destination, as main's NBT reconstruction did.
                PassengerSlot::Saved(tag) => self.load_passengers(root, Tag::List(vec![tag])),
            }
        }
        by_source.insert(source_id, root);
        Some(root)
    }

    pub(super) fn restore_vehicle_object(
        &mut self,
        tag: Tag,
        kind: VehicleTreeKind,
        name: &str,
        state: crate::game::entity_initialization::TransferredEntityState,
    ) -> Option<u32> {
        let uuid = entity_nbt::uuid(&tag);
        match kind {
            VehicleTreeKind::Mob => self.load_mob_transferred(tag, None, Some(state)),
            VehicleTreeKind::Boat => self.load_boat_transferred(tag, uuid, name, Some(state)),
            VehicleTreeKind::Cart => self.load_cart_transferred(tag, uuid, name, Some(state)),
            VehicleTreeKind::Other => {
                let before = self.next_entity;
                self.load_listed_transferred(name, uuid, self.level.player_uuid, tag, state)
                    .ok()?;
                ((before + 1)..=self.next_entity).find(|id| self.passenger_object_present(*id))
            }
        }
    }

    pub(super) fn mob_object_slot(&self, id: u32) -> Option<MobObject> {
        self.mobs
            .iter()
            .position(|mob| mob.id == id)
            .map(MobObject::Resident)
            .or_else(|| {
                self.persist
                    .off_level
                    .get(&id)
                    .and_then(OffLevel::as_mob)
                    .map(|_| MobObject::OffLevel(id))
            })
    }

    pub(super) fn mob_object_slots(&self) -> impl Iterator<Item = MobObject> + '_ {
        (0..self.mobs.len()).map(MobObject::Resident).chain(
            self.persist
                .off_level
                .iter()
                .filter_map(|(&id, object)| object.as_mob().map(|_| MobObject::OffLevel(id))),
        )
    }

    pub(super) fn mob_object_tree_nbt(&mut self, object: MobObject) -> Tag {
        let id = self.mob_at(object).id;
        let mut tag = self.mob_object_nbt(object);
        self.put_riding(&mut tag, object, id);
        tag
    }

    /// `Entity.saveWithoutId`'s riding parts for a mob object (id `id`): a passenger saves `Pos`
    /// at its vehicle's x and z with its own y, then the mobs riding it go under `Passengers`.
    /// Resident and off-level mobs, and riders saved through their mount, all come here.
    fn put_riding(&mut self, tag: &mut Tag, object: MobObject, id: u32) {
        use crate::game::commands::Target;
        let vehicle = self.passenger_vehicle_target(Target::Mob(id));
        if let (Some(at), Tag::Compound(c)) =
            (vehicle.and_then(|v| self.target_position(v)), &mut *tag)
        {
            let pos = [at[0], self.mob_at(object).position.y, at[2]];
            c.insert(
                "Pos".into(),
                Tag::List(pos.map(|v| Tag::Double(v.to_bits())).into()),
            );
        }
        self.put_mob_passengers(tag, id);
    }

    pub(super) fn mob_object_nbt(&mut self, object: MobObject) -> Tag {
        let id = self.mob_at(object).id;
        let uuid = self.uuid_for(id);
        // A warden's anger names each suspect by UUID. A mob that has never been
        // loaded or saved has no UUID yet, so give it one, or the save would
        // drop the anger at it (`who_uuid` writes nothing for a mob the map does
        // not know).
        let suspects: Vec<u32> = self
            .mob_at(object)
            .extra
            .warden
            .as_ref()
            .map(|st| {
                st.anger
                    .suspects()
                    .iter()
                    .filter_map(|who| match who {
                        Who::Mob(id) => Some(*id),
                        Who::Player => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        for suspect in suspects {
            self.uuid_for(suspect);
        }
        // `NeutralMob.addPersistentAngerSaveData`: the anger ends at an
        // absolute game time, so it goes on while the chunk is unloaded, and
        // the target is the entity the mob is angry at (`settle_anger_end`).
        let anger = self.mob_at(object).extra.anger;
        if horizon_client::mobs::persistent_anger(self.mob_at(object).species()) {
            let game_time = self.world.game_time;
            horizon_client::mobs::settle_anger_end(self.mob_at_mut(object), game_time);
        } else {
            self.mob_at_mut(object).extra.anger_end = if anger > 0 {
                self.world.game_time + i64::from(anger)
            } else {
                -1
            };
        }
        self.mob_at_mut(object).extra.angry_at = if anger > 0 {
            // A wolf and an iron golem keep their anger's target themselves,
            // also once the target is gone (`tame::update_persistent_anger`).
            if matches!(
                self.mob_at(object).species(),
                horizon_client::mobs::Species::Wolf | horizon_client::mobs::Species::IronGolem
            ) {
                self.mob_at(object).extra.angry_at
            } else {
                self.mob_at(object).ai.target
            }
        } else {
            None
        };
        // A mob gets its UUID when it is first saved (`uuid_for`): the anger
        // target still in the world gets it now, so `uuid_of` can name it.
        if let Some(horizon_client::mobs::Who::Mob(target)) = self.mob_at(object).extra.angry_at
            && target != 0
            && self.mobs.iter().any(|m| m.id == target)
        {
            self.uuid_for(target);
        }
        // The mob that last hurt it, still resident, gets its UUID now, as the
        // anger target does, so `put_last_hurt_by_mob` can name it.
        if let Some(horizon_client::mobs::Who::Mob(attacker)) = self.mob_at(object).ai.last_hurt_by
            && self.mobs.iter().any(|m| m.id == attacker)
        {
            self.uuid_for(attacker);
        }
        let mut tag = entity_nbt::mob_to_nbt(
            self.mob_at(object),
            uuid,
            self.persist.raw.get(&id),
            Some(self.pack.assets.registry),
            |other| self.persist.uuid_of(other),
        );
        self.put_last_hurt_by_mob(self.mob_at(object), &mut tag);
        if horizon_client::mobs::camel::is_camel(self.mob_at(object).species())
            && let Tag::Compound(c) = &mut tag
        {
            let tick = horizon_client::mobs::camel::synced_pose_tick(
                self.mob_at(object),
                self.world.game_time,
            );
            c.insert("LastPoseTick".into(), Tag::Long(tick));
        }
        self.write_leash(&mut tag, id);
        write_attachments(&mut tag, id, &self.entity_attachments);
        // A mod's AI pause beside the mods' attachments, whose map would drop it.
        entity_nbt::write_ai_paused(self.mob_at(object).ai.ai_paused, &mut tag);
        tag
    }

    /// Trace only edges touching off-level objects. A rejected leaf is owned
    /// by its resident vehicle; an orphaned rejected ancestor can instead be
    /// owned by a resident descendant or a player riding it.
    pub(super) fn retain_off_level_objects(&mut self, retained: &mut HashSet<u32>) {
        let mut links: HashMap<u32, Vec<u32>> = HashMap::new();
        for mob in self.mob_objects() {
            if let Some(vehicle) = mob.extra.mount
                && (self.persist.off_level.contains_key(&mob.id)
                    || self.persist.off_level.contains_key(&vehicle)
                    || self.persist.removed_vehicles.contains_key(&vehicle))
            {
                links.entry(mob.id).or_default().push(vehicle);
                links.entry(vehicle).or_default().push(mob.id);
            }
        }
        for (&vehicle, slots) in &self.persist.passenger_slots {
            for slot in slots {
                if let PassengerSlot::Live(id) = slot
                    && self.passenger_object_vehicle(*id) == Some(vehicle)
                    && (self.persist.off_level.contains_key(id)
                        || self.persist.off_level.contains_key(&vehicle)
                        || self.persist.removed_vehicles.contains_key(&vehicle))
                {
                    links.entry(*id).or_default().push(vehicle);
                    links.entry(vehicle).or_default().push(*id);
                }
            }
        }
        for vehicle in self.riding.into_iter().chain(
            self.lan
                .iter()
                .flat_map(|host| host.hub.guests.iter().filter_map(|guest| guest.vehicle)),
        ) {
            if self.persist.off_level.contains_key(&vehicle)
                || self.persist.removed_vehicles.contains_key(&vehicle)
            {
                retained.insert(vehicle);
            }
        }
        let mut pending: Vec<_> = links
            .keys()
            .copied()
            .filter(|id| retained.contains(id))
            .collect();
        while let Some(id) = pending.pop() {
            for &neighbor in links.get(&id).into_iter().flatten() {
                if (self.persist.off_level.contains_key(&neighbor)
                    || self.persist.removed_vehicles.contains_key(&neighbor))
                    && retained.insert(neighbor)
                {
                    pending.push(neighbor);
                }
            }
        }
        let gone: Vec<_> = self
            .persist
            .off_level
            .keys()
            .copied()
            .filter(|id| !retained.contains(id))
            .collect();
        for id in gone {
            self.persist.off_level.remove(&id);
            self.forget_entity(id);
        }
        let gone: Vec<_> = self
            .persist
            .removed_vehicles
            .keys()
            .copied()
            .filter(|id| !retained.contains(id))
            .collect();
        for id in gone {
            self.persist.removed_vehicles.remove(&id);
            self.forget_entity(id);
        }
    }
}

#[cfg(test)]
mod namespace_review_tests {
    use super::*;

    #[test]
    #[ignore = "daily"]
    fn captured_content_vehicle_type_matches_its_actual_save_id() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            let mut kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            Arc::make_mut(&mut kind).id = "example:mob".into();
            game.mobs.push(Mob::new(151, kind, Vec3::ZERO, 0.0));
            let (tag, tree) = game.take_vehicle_tree_transfer(151).unwrap();
            assert_eq!(entity_nbt::kind(&tag).as_deref(), Some("example:mob"));
            assert_eq!(tree.name, "example:mob");
            assert!(game.prepare_vehicle_transfer(tag, tree).is_some());
        });
    }
}
