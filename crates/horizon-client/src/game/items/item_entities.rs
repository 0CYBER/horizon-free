//! Dropped items and experience orbs: their tick (`tick_items`, `tick_dropped_items`,
//! `tick_experience`), the stacks a destroyed container lets fall, and spawning and merging orbs.
use super::*;

impl Game<'_> {
    pub(super) fn tick_items(&mut self) {
        let mut round = self.item_update_round();
        self.tick_dropped_items(&mut round);
        horizon_threads::work::clear_work();
        self.drain_entity_inside();
        self.native_tick_post_batch(round);
    }
    /// An item that joined during this tick's entity pass: it first ticks on
    /// the next one.
    pub(super) fn joined_during_entity_pass(&self, id: u32) -> bool {
        id > self.ticked_entities_through || self.contact_drops.contains(&id)
    }
    pub(super) fn tick_dropped_items(&mut self, round: &mut native_ticks::TickRound) {
        let mut items = self.dropped.lift();
        // ServerLevel.tick: only entities in entity-ticking chunks tick.
        let ticking = self.entity_ticking_area();
        let mut removed = vec![false; items.len()];
        let mut cells = horizon_client::entities::ItemCells::new(&items);
        // A LAN or server guest only shows the host's items.
        let merging = self.remote.is_none();
        // `Entity.checkBelowWorld`: below `Level.getMinY() - 64`.
        let void = f64::from(self.level_min_y() - 64);
        let mut obstacles =
            horizon_client::entities::ObstacleGrid::new(self.item_entity_collisions());
        let mut nearby = Vec::new();
        let mut bubble_tops = Vec::new();
        let mut moves = ahead::Moves::work_out(self, &items, &obstacles, round, |item| {
            !self.joined_during_entity_pass(item.id)
                && ticking(
                    item.position.x.floor() as i32,
                    item.position.z.floor() as i32,
                )
        });
        // `ItemUtils.onContainerDestroyed`: what a destroyed item held, and
        // where it lay, for adding to the level once its own tick is done.
        let mut destroyed: Vec<(ItemStack, Vec3)> = Vec::new();
        let item_type = horizon_threads::work::entity_id("minecraft:item");
        for i in 0..items.len() {
            horizon_threads::work::set_entity(item_type);
            if removed[i] {
                // Merged into an earlier item before its own tick: it never ticked.
                round.not_ticked(items[i].id);
                continue;
            }
            // `addFreshEntity` during the entity pass: the item ticks from the
            // next tick on (`EntityTickList`).
            if merging && self.joined_during_entity_pass(items[i].id) {
                continue;
            }
            if !ticking(
                items[i].position.x.floor() as i32,
                items[i].position.z.floor() as i32,
            ) || round.canceled(items[i].id)
                || round.halted(items[i].id)
            {
                continue;
            }
            self.tick_saved_invulnerable_time(items[i].id);
            let item = &mut items[i];
            let mut at_turn = |item: &mut ItemEntity| {
                // `getEntityCollisions(entity, bounds.expandTowards(movement))`: the boxes around
                // where the item can reach this tick.
                obstacles.near(ahead::reach(item), &mut nearby);
                // ItemEntity.tick's noPhysics (servers only): out of a block, a solid entity
                // or the border, placed or closed on it, towards the closest open side (#1252).
                let no_physics = merging
                    && item.inside_block(
                        &self.world,
                        self.collision,
                        &nearby,
                        Some(self.border.geometry()),
                    );
                // `moveTowardsClosestSpace` draws the speed from the item's own random.
                let escape_speed = no_physics.then(|| item.random.next_float() * 0.2 + 0.1);
                let mut entered = Vec::new();
                let (alive, discarded) = self.tick_item_with_block_contacts(
                    item,
                    escape_speed,
                    &nearby,
                    false,
                    &mut |splash| entered.push(splash),
                );
                (no_physics, alive, discarded, entered)
            };
            let (no_physics, alive, discarded, entered) = match moves.take(i, item, &mut at_turn) {
                Some(moved) => moved,
                None => at_turn(item),
            };
            // Entity.doWaterSplashEffect: the server plays the sound (broadcast
            // to clients). Its addParticle is empty.
            for splash in entered {
                self.apply_item_splash(splash);
            }
            if discarded {
                removed[i] = true;
                cells.remove(i, item.previous);
                continue;
            }
            if merging && !no_physics {
                self.item_landing_event(item.id, item.previous, item.position, item.on_ground);
                self.entity_powder_snow_particles(
                    item.previous,
                    item.position,
                    item.bounds(),
                    item.velocity,
                    false,
                );
            }
            cells.moved(i, item.previous, item.position);
            // Entity.handleOnAboveBubbleColumn: the server sends the particles of
            // the surface of a bubble column the item is at the top of.
            if merging {
                bubble_tops.extend(horizon_client::motion::bubble_column_surfaces(
                    &self.world,
                    self.pack.assets.registry,
                    self.res.facts(),
                    self.collision,
                    item.bounds(),
                ));
            }
            // ItemEntity.tick: merge every 2 ticks while moving between
            // blocks, every 40 otherwise, before the lifetime check.
            let block = |v: Vec3| (v.x.floor(), v.y.floor(), v.z.floor());
            let rate = if block(item.previous) == block(item.position) {
                40
            } else {
                2
            };
            if merging
                && (alive || item.age == 6000 && item.position.y >= void)
                && item.ticks.is_multiple_of(rate)
            {
                let max_stack = |stack: &ItemStack| self.stack_max_size(stack).max(1);
                horizon_client::entities::merge_with_neighbours(
                    &mut items,
                    &mut removed,
                    &mut cells,
                    i,
                    true,
                    &max_stack,
                );
            }
            let item = &mut items[i];
            let mut alive = alive || (item.age < 6000 && item.position.y >= void);
            if merging && !removed[i] && item.age >= 6000 && item.position.y >= void {
                let mut extra_life = 0;
                self.native_item_expire(item.id, &item.stack, &mut extra_life);
                if extra_life > 0 {
                    item.age = 6000 - extra_life.min(32766);
                    alive = true;
                } else if self.serving.is_some() || self.options.dedicated.is_some() {
                    self.native_item_despawn(
                        item.id,
                        &item.stack.item,
                        item.stack.count as i32,
                        [item.position.x, item.position.y, item.position.z],
                    );
                }
            }
            if !alive || removed[i] {
                removed[i] = true;
                cells.remove(i, item.position);
                continue;
            }
            let from = item.previous;
            let to = item.position;
            if round.sections() {
                let live = self.entity_section_step_visible(
                    item.id,
                    [from.x, from.y, from.z],
                    [to.x, to.y, to.z],
                );
                if !live {
                    removed[i] = true;
                    cells.remove(i, to);
                    continue;
                }
            }
            let item_id = item.id;
            let saved = native_components::ITEM_ENVIRONMENT_DAMAGE
                .map(|damage| self.saved_refuses_damage(item_id, damage, false));
            let tags = world_items!(self);
            let registry = self.pack.assets.registry;
            let block_registries = &self.pack.block_registries;
            let hooks = &self.damage_hooks;
            let environment = item.environment_tick_with_hook(
                &mut self.world,
                self.res.facts(),
                registry,
                &native_components::item_damage_resolver(
                    &self.pack.item_renderer.models,
                    &self.health_resources.damage,
                ),
                |world, damage| {
                    let vanilla = match damage {
                        "minecraft:on_fire" => saved[0],
                        "minecraft:lava" => saved[1],
                        "minecraft:in_fire" => saved[2],
                        "minecraft:cactus" => saved[3],
                        _ => false,
                    };
                    let source = horizon_client::health::HurtSource {
                        damage_type: damage,
                        causing: 0,
                        direct: 0,
                        position: None,
                    };
                    horizon_client::health::DamageLevel::of(
                        world,
                        registry,
                        Some(tags),
                        Some(block_registries),
                        hooks,
                    )
                    .invulnerability_check(
                        horizon_client::health::Hurting::Entity(
                            item_id as i32,
                            "minecraft:item",
                            source,
                        ),
                        vanilla,
                    )
                },
            );
            if environment.should_play_lava_hurt_sound(item.ticks) {
                let pitch = 2.0 + self.world.random.next_float() * 0.4;
                self.audio.play(
                    "minecraft:entity.generic.burn",
                    Some([item.position.x, item.position.y, item.position.z]),
                    0.4,
                    pitch,
                );
            }
            if !environment.alive {
                removed[i] = true;
                cells.remove(i, item.position);
                for stack in environment.spilled {
                    destroyed.push((stack, item.position));
                }
            }
        }
        for pos in bubble_tops {
            self.bubble_column_particles(pos);
        }
        let mut removed = removed.into_iter();
        items.retain(|_| !removed.next().unwrap_or(false));
        let Some(p) = self.player.as_ref() else {
            self.put_back_dropped(items);
            self.spawn_destroyed_contents(destroyed);
            return;
        };
        if p.health.dead_or_dying() || self.spectator() {
            self.put_back_dropped(items);
            self.spawn_destroyed_contents(destroyed);
            return;
        }
        let w = f64::from(p.dimensions.width) / 2.0;
        let (min, max) = (
            [
                p.position.x - w - 1.0,
                p.position.y - 0.5,
                p.position.z - w - 1.0,
            ],
            [
                p.position.x + w + 1.0,
                p.position.y + f64::from(p.dimensions.height) + 0.5,
                p.position.z + w + 1.0,
            ],
        );
        let feet = [p.position.x, p.position.y, p.position.z];
        let own = self.level.ensure_player_uuid();
        self.native_player_pickups(&mut items, (min, max), feet, own);
        self.put_back_dropped(items);
        self.spawn_destroyed_contents(destroyed);
    }
    /// `ItemUtils.onContainerDestroyed`: spilled stacks use their own constructor
    /// motion (0.2 up and ±0.1 sideways), leaving the level random untouched.
    pub(super) fn spawn_destroyed_contents(&mut self, destroyed: Vec<(ItemStack, Vec3)>) {
        for (stack, at) in destroyed {
            self.spawn_item_own(stack, [at.x, at.y, at.z], 0);
        }
    }
    /// `ExperienceOrb.award`: orbs of the largest sizes; each draws the id
    /// `tryMergeToExisting` compares from the level random, then moves with
    /// its own random.
    pub(super) fn spawn_orbs(&mut self, at: [f64; 3], value: u32) {
        self.spawn_orbs_toward(at, [0.0; 3], value);
    }
    /// `ExperienceOrb.awardWithDirection`: `award` whose fresh orbs start
    /// `roughly` that way (`new ExperienceOrb(level, pos, roughly, value)`).
    pub(super) fn spawn_orbs_toward(&mut self, at: [f64; 3], roughly: [f64; 3], mut value: u32) {
        while value > 0 {
            let v = ExperienceOrb::split(value);
            value -= v;
            let merge_id = self.world.random.next_int_bound(40).unwrap_or(0);
            if !self.merge_new_orb(at, v, merge_id) {
                self.add_orb_toward(at, roughly, v);
            }
        }
    }
    /// `ExperienceOrb.tryMergeToExisting`: an orb of the same value within a
    /// 1 block box of `at`, whose id is a multiple of 40 from `merge_id`, takes
    /// the new one as one more in its count (and restarts its age); otherwise
    /// a fresh orb spawns.
    pub(super) fn award_orb(&mut self, at: [f64; 3], value: u32, merge_id: i32) {
        if !self.merge_new_orb(at, value, merge_id) {
            self.add_orb(at, value);
        }
    }
    /// `tryMergeToExisting`: true when an orb took the new one.
    pub(super) fn merge_new_orb(&mut self, at: [f64; 3], value: u32, merge_id: i32) -> bool {
        let centre = Vec3::new(at[0], at[1], at[2]);
        let half = Vec3::new(0.5, 0.5, 0.5);
        let area = Aabb::new(centre.subtract(half), centre.add_vector(half));
        // A server's orbs are its own; a LAN or server guest only shows them.
        if self.remote.is_none() {
            for i in 0..self.orbs.len() {
                let o = &self.orbs[i];
                if !o.bounds().intersects(area) {
                    continue;
                }
                let vanilla = o.can_merge(merge_id as u32, value as i32);
                let (join, id, count, age) = (o.join, o.id, o.count, o.age);
                let orb_value = o.value;
                if !self.orb_merges(id, orb_value, count, 0, value as i32, 1, vanilla) {
                    self.restore_orb(join, orb_value, count, age);
                    continue;
                }
                let Some(i) = self.orbs.locate(join) else {
                    continue;
                };
                let o = &mut self.orbs[i];
                // A mod's write in the merge hook is the merged orb; else one more in its count.
                if (o.value, o.count) == (orb_value, count) {
                    if count >= crate::orb_limits::MAX_COUNT {
                        continue;
                    }
                    o.count += 1;
                }
                o.age = 0;
                return true;
            }
        }
        false
    }
    /// Puts back what a merge hook may have written to the orb `join`: a refused merge
    /// leaves both orbs as they were.
    fn restore_orb(&mut self, join: crate::join_seq::JoinSeq, value: i32, count: u32, age: u32) {
        if let Some(i) = self.orbs.locate(join) {
            let orb = &mut self.orbs[i];
            (orb.value, orb.count, orb.age) = (value, count, age);
        }
    }
    /// Whether orb `into` takes orb `from` (0: a spawning orb of `from_value`),
    /// given vanilla's `canMerge` answer: the mods may flip it, then cancel the merge.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn orb_merges(
        &mut self,
        into: u32,
        into_value: i32,
        into_count: u32,
        from: u32,
        from_value: i32,
        from_count: u32,
        vanilla: bool,
    ) -> bool {
        let dimension = self.dim.id();
        let mut can_merge = vanilla;
        self.native_experience_orb_can_merge(
            dimension,
            into,
            from,
            into_value,
            from_value,
            &mut can_merge,
        );
        if !can_merge {
            return false;
        }
        let mut canceled = false;
        self.native_experience_orb_merge(mod_api::__private::ExperienceOrbMergeCall {
            dimension,
            into,
            from,
            into_value,
            into_count,
            from_value,
            from_count,
            canceled: &mut canceled,
        });
        !canceled
    }
    /// `ExperienceOrb.tick`'s scan before movement, once every twenty ticks.
    /// Hooks can change the resident list, so retain join identities, not indices.
    pub(super) fn merge_orb(&mut self, i: usize) {
        if !self.orbs[i].ticks.is_multiple_of(20) {
            return;
        }
        let into_join = self.orbs[i].join;
        let area = self.orbs[i].bounds().inflate(Vec3::new(0.5, 0.5, 0.5));
        let candidates: Vec<_> = self
            .orbs
            .iter()
            .filter(|o| o.join != into_join && o.bounds().intersects(area))
            .map(|o| o.join)
            .collect();
        for from_join in candidates {
            let Some(i) = self.orbs.locate(into_join) else {
                return;
            };
            let Some(j) = self.orbs.locate(from_join) else {
                continue;
            };
            let (a, b) = (&self.orbs[i], &self.orbs[j]);
            let (into, into_value, into_count, into_age) = (a.id, a.value, a.count, a.age);
            let (from, from_value, from_count) = (b.id, b.value, b.count);
            let vanilla = b.can_merge(a.id, a.value);
            if !self.orb_merges(
                into, into_value, into_count, from, from_value, from_count, vanilla,
            ) {
                self.restore_orb(into_join, into_value, into_count, into_age);
                continue;
            }
            let Some(i) = self.orbs.locate(into_join) else {
                return;
            };
            let written = (self.orbs[i].value, self.orbs[i].count) != (into_value, into_count);
            let fresh = self.orbs.locate(from_join).filter(|&j| {
                let (a, b) = (&self.orbs[i], &self.orbs[j]);
                (b.value, b.count) == (from_value, from_count)
                    && b.bounds()
                        .intersects(a.bounds().inflate(Vec3::new(0.5, 0.5, 0.5)))
            });
            // A mod's write in the merge hook is the merged orb; else `into` adds the
            // source's count. A stale source, or a count past what a save holds, refuses
            // the merge and puts a written `into` back.
            let count = if written {
                Some(self.orbs[i].count)
            } else {
                into_count
                    .checked_add(from_count)
                    .filter(|&c| c <= crate::orb_limits::MAX_COUNT)
            };
            let (Some(j), Some(count)) = (fresh, count) else {
                self.restore_orb(into_join, into_value, into_count, into_age);
                continue;
            };
            let age = self.orbs[j].age;
            self.orbs[i].count = count;
            self.orbs[i].age = self.orbs[i].age.min(age);
            self.orbs.remove(j);
        }
    }
    /// `Entity.unstuckIfPossible`, which `ExperienceOrb`'s constructor runs on an
    /// orb whose box is in a block: the free spot nearest its box's centre. An
    /// orb in open air keeps its position.
    pub(super) fn unstuck_orb(&self, at: [f64; 3]) -> [f64; 3] {
        let b = self.res.behaviours();
        let border = self.border_bounds();
        ExperienceOrb::unstuck(Vec3::new(at[0], at[1], at[2]), &border, |p| {
            b.collision_shape(self.world.block(p.x, p.y, p.z))
                .map(|s| s.moved(Vec3::new(f64::from(p.x), f64::from(p.y), f64::from(p.z))))
        })
        .map_or(at, |p| [p.x, p.y, p.z])
    }
    /// `new ExperienceOrb(level, pos, Vec3.ZERO, value)`: yaw and motion from
    /// the orb's own random.
    pub(super) fn add_orb(&mut self, at: [f64; 3], value: u32) {
        self.add_orb_toward(at, [0.0; 3], value);
    }
    /// `new ExperienceOrb(level, pos, roughly, value)`: a random motion that
    /// points against `roughly` is turned round, and the orb starts half its
    /// size (0.25) along `roughly`, unstuck if it collides there.
    pub(super) fn add_orb_toward(&mut self, at: [f64; 3], roughly: [f64; 3], value: u32) {
        let len_sqr = roughly.iter().map(|v| v * v).sum::<f64>();
        // Vec3.normalize: a vector shorter than 1e-5 has no direction.
        let len = len_sqr.sqrt();
        let unit = if len < 1.0e-5 {
            [0.0; 3]
        } else {
            roughly.map(|v| v / len)
        };
        let at = self.unstuck_orb([0, 1, 2].map(|a| at[a] + unit[a] * 0.25));
        let mut r = random_source::entity();
        let _yaw = r.next_float();
        let p = Vec3::new(at[0], at[1], at[2]);
        let mut velocity = Vec3::new(
            (r.next_double() * 0.2 - 0.1) * 2.0,
            r.next_double() * 0.2 * 2.0,
            (r.next_double() * 0.2 - 0.1) * 2.0,
        );
        let dot = roughly[0] * velocity.x + roughly[1] * velocity.y + roughly[2] * velocity.z;
        if len_sqr > 0.0 && dot < 0.0 {
            velocity = Vec3::new(-velocity.x, -velocity.y, -velocity.z);
        }
        self.next_entity += 1;
        self.orbs.push(ExperienceOrb {
            attachment_lifetime: Default::default(),
            fall_distance: 0.0,
            rotation: [0.0, 0.0],
            join: Default::default(),
            no_gravity: false,
            id: self.next_entity,
            position: p,
            previous: p,
            velocity,
            // `getExperienceValue` sizes: 1 to 2477.
            value: value as i32,
            age: 0,
            on_ground: false,
            count: 1,
            ticks: 0,
            health: 5,
            air: 300,
            following: None,
        });
        // ServerLevel.addFreshEntity.
        self.native_joins_flush(false, 0);
    }
    /// The orbs' tick, but for the movement: that is `run_ordered_ticks`'s,
    /// in join order with the other kinds.
    pub(in crate::game) fn tick_experience(&mut self) {
        if self.xp_delay > 0 {
            self.xp_delay -= 1;
        }
        let spectator = self.spectator();
        let player = self
            .player
            .as_ref()
            .filter(|p| !p.health.dead_or_dying() && !spectator);
        // Player.aiStep: standing, the pickup area is the player's box
        // inflated by (1, 0.5, 1); riding, the union with the vehicle's box
        // inflated by (1, 0, 1).
        let bounds = player.map(|p| {
            let w = f64::from(p.dimensions.width) / 2.0;
            let own = Aabb::new(
                Vec3::new(p.position.x - w, p.position.y, p.position.z - w),
                Vec3::new(
                    p.position.x + w,
                    p.position.y + f64::from(p.dimensions.height),
                    p.position.z + w,
                ),
            );
            let area = match self.riding_box() {
                Some(vehicle) => own.union_bounds(vehicle).inflate(Vec3::new(1.0, 0.0, 1.0)),
                None => own.inflate(Vec3::new(1.0, 0.5, 1.0)),
            };
            (
                [
                    area.min_position().x,
                    area.min_position().y,
                    area.min_position().z,
                ],
                [
                    area.max_position().x,
                    area.max_position().y,
                    area.max_position().z,
                ],
            )
        });
        let mut orbs = self.orbs.lift();
        #[cfg(test)]
        crate::game::entity::taken_pass_tests::after_take(self);
        let mut gained = 0;
        let mut taken = Vec::new();
        if let Some((min, max)) = bounds {
            orbs.retain_mut(|o| {
                let touching = o.position.x + 0.25 > min[0]
                    && o.position.x - 0.25 < max[0]
                    && o.position.y + 0.5 > min[1]
                    && o.position.y < max[1]
                    && o.position.z + 0.25 > min[2]
                    && o.position.z - 0.25 < max[2];
                if touching && self.xp_delay == 0 {
                    // ExperienceOrb.playerTouch: one orb every 2 ticks; a
                    // merged orb gives its value once per count.
                    self.xp_delay = 2;
                    gained += o.points();
                    taken.push(o.clone());
                    o.count -= 1;
                    return o.count > 0;
                }
                true
            });
        }
        self.orbs
            .restore(orbs, horizon_client::join_seq::RestoreOrder::SurvivorsFirst);
        if gained > 0 {
            gained = self.mending(gained);
        }
        for o in taken {
            // ClientPacketListener.handleTakeItemEntity: every orb taken sounds
            // at the orb, whether mending took its points or not.
            let pitch =
                (self.world.random.next_float() - self.world.random.next_float()) * 0.35 + 0.9;
            let at = [o.position.x, o.position.y, o.position.z];
            self.audio.play(
                "minecraft:entity.experience_orb.pickup",
                Some(at),
                0.1,
                pitch,
            );
            self.add_pickup(
                crate::game::pickups::Taken::Orb(Box::new(o)),
                crate::game::pickups::Collector::Player,
            );
        }
        if gained > 0 {
            self.give_experience_points(gained as i32);
        }
    }

    /// `Player.dropEquipment` for the player who died: unless keep_inventory
    /// is on, `destroyVanishingCursedItems`, then `Inventory.dropAll` from
    /// `at` (the eyes less 0.3): slots 0..36, armor feet to head (36..39),
    /// the off hand (40), each thrown in a random direction. The mods hear
    /// the drops (`native_living_drops`) before they join the level.
    pub(super) fn drop_inventory(&mut self, at: [f64; 3], dying: &creatures::DyingDrops<'_>) {
        let mut stacks: Vec<ItemStack> = Vec::new();
        if !self
            .ui
            .rules
            .bool_at(horizon_client::gamerules::vanilla::KEEP_INVENTORY)
        {
            stacks.extend(self.inventory.iter_mut().filter_map(Option::take));
            stacks.extend(self.armor.iter_mut().rev().filter_map(Option::take));
            stacks.extend(self.offhand.take());
            stacks.extend(self.equipment.take_all());
            stacks.retain(|s| s.enchantment_level("minecraft:vanishing_curse") == 0);
            self.apply_equipment();
        }
        let stacks = stacks.into_iter().map(|stack| (stack, at, 0)).collect();
        self.drop_death_items(dying, stacks, 40, |game| {
            let r = &mut game.world.random;
            let strength = f64::from(r.next_float()) * 0.5;
            let angle = f64::from(r.next_float()) * std::f64::consts::TAU;
            Some([-angle.sin() * strength, 0.2, angle.cos() * strength])
        });
    }

    /// Item entity `i` leaves the level (`Entity.discard`); mods hear it.
    pub(super) fn remove_dropped(&mut self, i: usize) -> ItemEntity {
        let e = self.dropped.remove(i);
        self.native_entity_left(e.id);
        e
    }

    /// Experience orb `i` leaves the level (`Entity.discard`); mods hear it.
    pub(super) fn remove_orb(&mut self, i: usize) -> ExperienceOrb {
        let o = self.orbs.remove(i);
        self.native_entity_left(o.id);
        o
    }

    /// The item entity `spawn_item` just added, unless a native mod kept it
    /// out of the level (`native_entity_join_level`).
    pub(super) fn just_spawned_item(&mut self) -> Option<&mut ItemEntity> {
        let id = self.next_entity;
        self.dropped.last_mut().filter(|d| d.id == id)
    }

    /// `new ItemEntity(level, x, y, z, stack, dx, dy, dz)`.
    pub(super) fn spawn_item(
        &mut self,
        stack: ItemStack,
        position: [f64; 3],
        velocity: [f64; 3],
        pickup_delay: u32,
    ) {
        self.add_item_entity(stack, position, Some(velocity), pickup_delay);
    }

    /// `new ItemEntity(level, x, y, z, stack)`: the item leaves with its own
    /// random's motion, which this returns.
    pub(super) fn spawn_item_own(
        &mut self,
        stack: ItemStack,
        position: [f64; 3],
        pickup_delay: u32,
    ) -> [f64; 3] {
        self.add_item_entity(stack, position, None, pickup_delay)
    }

    /// The dispenser constructs default motion before replacing it with the throw.
    pub(super) fn spawn_item_thrown(
        &mut self,
        stack: ItemStack,
        position: [f64; 3],
        velocity: [f64; 3],
    ) {
        let random = spill::prepare_item_random(None);
        self.add_item_entity_prepared(stack, position, Some(velocity), 0, Some(random));
    }

    pub(super) fn add_item_entity(
        &mut self,
        stack: ItemStack,
        position: [f64; 3],
        velocity: Option<[f64; 3]>,
        pickup_delay: u32,
    ) -> [f64; 3] {
        self.add_item_entity_prepared(stack, position, velocity, pickup_delay, None)
    }

    /// Block.popResource
    pub(super) fn pop_resource(&mut self, pos: [i32; 3], mut stack: ItemStack) {
        self.pending.attach(pos, &mut stack);
        // Block.popResource: Mth.nextDouble offsets from the level random; the
        // motion is `new ItemEntity(..)`'s own, from the item's random.
        let p = spill::pop_position(&mut self.world.random, pos);
        // Block.popResource draws its offsets before the private overload checks the gamerule.
        if !self
            .ui
            .rules
            .bool_at(horizon_client::gamerules::vanilla::BLOCK_DROPS)
        {
            return;
        }
        self.spawn_item_own(stack, p, 10);
    }

    /// `Block.popResourceFromFace`: the loot leaves the clicked face with the
    /// face's own motion instead of the item's random one.
    pub(super) fn pop_resource_from_face(
        &mut self,
        pos: [i32; 3],
        face: [i32; 3],
        mut stack: ItemStack,
    ) {
        self.pending.attach(pos, &mut stack);
        let (p, v) = spill::pop_position_from_face(&mut self.world.random, pos, face);
        // The private overload checks the gamerule after the offsets are drawn.
        if !self
            .ui
            .rules
            .bool_at(horizon_client::gamerules::vanilla::BLOCK_DROPS)
        {
            return;
        }
        self.spawn_item(stack, p, v, 10);
    }

    /// Player.drop: thrown from the eyes along the view direction. True when
    /// the selected stack's `on_dropped_by_player` Part kept it in the slot
    /// (`ServerPlayer.drop(boolean)`, as NeoForge asks it).
    pub(in crate::game) fn drop_held(&mut self, all: bool) -> bool {
        if self.eye_position().is_none() {
            return false;
        }
        let spectator = self.spectator();
        if !spectator && self.native_dropped_by_player_event(all) {
            return true;
        }
        let Some(thrown) = horizon_client::items::take_held_drop(
            &mut self.inventory[self.selected],
            all,
            spectator,
        ) else {
            return false;
        };
        self.player_drop(thrown);
        false
    }

    /// A guest's drop key. A drop the held item's Part refused sends the
    /// inventory back, undoing the removal the client predicted.
    pub(in crate::game) fn guest_drop_held(&mut self, gi: usize, conn: u32, all: bool) {
        let kept = self.as_guest(conn, |g| g.drop_held(all)) == Some(true);
        if let Some(host) = self.lan.as_mut() {
            if kept {
                host.hub.send_inventory(gi);
            } else {
                host.hub.guests.update(gi, |g| g.told_selected());
            }
        }
    }

    /// `Player.createItemStackToDrop`: an item entity thrown from the eyes
    /// along the view direction. `thrown_from_hand` (`drop(stack, true)`, the
    /// drop key) names the player as the thrower; `drop(stack, false)`, for
    /// what no longer fits in the inventory, does not.
    pub(super) fn throw_from_eyes(
        &mut self,
        thrown: ItemStack,
        eye: [f64; 3],
        randomly: bool,
        thrown_from_hand: bool,
    ) -> Option<u32> {
        let r = &mut self.world.random;
        let v = horizon_client::entities::drop_motion(r, self.yaw, self.pitch, randomly);
        let at = [eye[0], eye[1] - f64::from(0.3f32), eye[2]];
        let uuid = thrown_from_hand.then(|| self.acting_player_uuid());
        // Only `drop(.., false)` tosses ask `item_toss`: a death's scatter (`randomly`) does not.
        let id = if randomly {
            self.spawn_item(thrown, at, v, 40);
            self.next_entity
        } else {
            self.toss_item(thrown, at, v, uuid)?
        };
        let item = self.dropped.iter_mut().rev().find(|d| d.id == id)?;
        item.thrower = uuid;
        Some(item.id)
    }

    /// `FurnaceResultSlot.checkTakeAchievements`: the player took from the output of the
    /// furnace at `pos`, which pops the experience of what it made at them
    /// (`awardUsedRecipesAndPopExperience`) and forgets it.
    pub(super) fn award_furnace_recipes(&mut self, pos: (i32, i32, i32), at: [f64; 3]) {
        let used = match self.world.block_entities.get_mut(&pos) {
            Some(BlockEntity::Furnace(f)) => {
                let used = f.recipe_experience(world_recipes!(self));
                f.recipes_used.clear();
                used
            }
            _ => return,
        };
        for (count, experience) in used {
            let xp = horizon_client::containers::create_experience(
                count,
                experience,
                &mut self.world.random,
            );
            self.spawn_orbs(at, xp);
        }
    }
}

#[cfg(test)]
mod destroyed_contents_tests {
    use super::*;
    use crate::game::game_tests::tests::server_game;
    use horizon_client::items::ItemStack;
    use horizon_core::random::RandomSource;

    /// The motion of each spilled stack, in the order the spill made them.
    fn spilled_motions(game: &Game<'_>) -> Vec<[f64; 3]> {
        game.dropped
            .iter()
            .map(|item| [item.velocity.x, item.velocity.y, item.velocity.z])
            .collect()
    }

    /// `ItemUtils.onContainerDestroyed` makes each stack with
    /// `new ItemEntity(level, x, y, z, stack)`, whose motion is 0.2 up and
    /// ±0.1 sideways from the *item's* random (ItemEntity.java:61-66), and
    /// draws nothing from the level's. Every destroyed container funnels here:
    /// lava, fire and cactus in the item's environment tick, an explosion, and
    /// the damage command.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn spilled_stacks_rise_from_their_own_randoms() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        let at = Vec3::new(8.5, 70.0, 8.5);
        let stacks = ["minecraft:stone", "minecraft:dirt", "minecraft:glass"]
            .into_iter()
            .map(|id| (ItemStack::new(id, 1), at))
            .collect();
        let mut level = game.world.random.clone();
        game.spawn_destroyed_contents(stacks);
        let motions = spilled_motions(&game);
        assert_eq!(motions.len(), 3);
        for motion in &motions {
            assert_eq!(motion[1], 0.2, "the four-argument constructor rises 0.2");
            assert!((-0.1..0.1).contains(&motion[0]), "{motion:?}");
            assert!((-0.1..0.1).contains(&motion[2]), "{motion:?}");
        }
        // Three stacks, three different sideways motions: each drew its own.
        assert_ne!(motions[0], motions[1]);
        assert_ne!(motions[1], motions[2]);
        assert_ne!(motions[0], motions[2]);
        // The spill leaves the level's random alone.
        assert_eq!(game.world.random.next_double(), level.next_double());
        assert_eq!(game.world.random.next_double(), level.next_double());
    }

    /// The damage command is one of those callers: killing a shulker box item
    /// spills its contents with the same upward motion.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_damaged_shulker_box_spills_its_stacks_upward() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        game.build_registries();
        game.set_allow_commands(true);
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
        );
        let feet = Vec3::new(8.5, 70.0, 8.5);
        game.player = Some(horizon_client::motion::PlayerMotion::new(game.motion, feet).unwrap());
        game.update_chunk_tickets();
        let boxed = horizon_client::item_data::parse_stack(
            r#"{id:"minecraft:shulker_box",count:1,components:{"minecraft:container":[
                {slot:0,item:{id:"minecraft:stone",count:2}},
                {slot:4,item:{id:"minecraft:dirt",count:1}},
                {slot:8,item:{id:"minecraft:glass",count:3}}]}}"#,
        )
        .unwrap()
        .unwrap();
        game.spawn_item(boxed, [8.5, 70.0, 8.5], [0.0; 3], 0);
        assert_eq!(game.dropped.len(), 1);
        assert!(game.run_command("damage @e[type=minecraft:item,limit=1] 100 minecraft:generic"));
        let mut spilled: Vec<_> = game
            .dropped
            .iter()
            .map(|item| (item.stack.item.to_string(), item.stack.count))
            .collect();
        spilled.sort();
        assert_eq!(
            spilled,
            [
                ("minecraft:dirt".to_owned(), 1),
                ("minecraft:glass".to_owned(), 3),
                ("minecraft:stone".to_owned(), 2)
            ]
        );
        for item in &game.dropped {
            assert_eq!(item.velocity.y, 0.2, "{item:?}");
        }
    }
}

#[cfg(test)]
mod escape_tests {
    use crate::game::game_tests::tests::server_game;
    use horizon_client::items::ItemStack;
    use horizon_core::random::RandomSource;

    /// `Entity.moveTowardsClosestSpace` uses the item's own random, never the
    /// level's random. Seeded Java values pin vanilla's float arithmetic (#13043).
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn an_item_escapes_a_block_at_its_own_randoms_speed() {
        let mut game_resources = None;
        let mut game = server_game(&mut game_resources);
        let feet = horizon_core::geometry::Vec3::new(8.5, 70.0, 8.5);
        game.player = Some(horizon_client::motion::PlayerMotion::new(game.motion, feet).unwrap());
        super::native_entity_refs::tests::prepare_entity_ticking_neighborhood(
            &mut game,
            horizon_core::position::ChunkPos::new(0, 0),
        );
        game.update_chunk_tickets();
        assert!(game.entity_ticking_area()(8, 8));
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        game.world.set(8, 64, 8, stone).unwrap();
        let stack = ItemStack::new("minecraft:stone", 2);
        game.spawn_item(stack, [8.5, 64.4, 8.5], [0.0, 0.0, 0.0], 10);
        let item = game.dropped.last_mut().unwrap();
        item.random = horizon_core::random::LegacyRandom::new(12345);
        let mut level = game.world.random.clone();
        game.ticked_entities_through = u32::MAX;
        game.tick_items();
        let item = game.dropped.last().unwrap();
        // Up is the nearest open side. Independent Java Random(12345):
        // nextFloat()*0.2F+0.1F, widened before multiplying by widened 0.98F.
        assert_eq!(item.velocity.y, f64::from_bits(0x3fc5_9ef4_570f_5500));
        assert_eq!(item.random.clone().next_float().to_bits(), 0x3f03_61b3);
        assert_eq!(game.world.random.next_float(), level.next_float());
    }
}
