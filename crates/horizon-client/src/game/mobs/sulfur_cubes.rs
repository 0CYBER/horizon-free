//! Sulfur cubes on the game side (`SulfurCube.mobInteract`, `pickUpItem`,
//! `remove`): feeding babies, swallowing a held block, shearing it back
//! out, lighting an explosive one, swallowing blocks from the ground and
//! splitting into two babies.
use super::Game;
use horizon_client::{
    items::ItemStack,
    mesher::Vertex,
    mobs::{Mob, Species, Who, sulfur_cube},
    projectiles::Projectile,
    text::Text,
};
use horizon_core::position::BlockPos;
use horizon_world::registry::BlockStateId;

fn block_transform(base: glam::Mat4, baby: bool) -> glam::Mat4 {
    base * glam::Mat4::from_scale(glam::Vec3::new(1.0, -1.0, -1.0))
        * glam::Mat4::from_scale(glam::Vec3::splat(if baby { 0.5 } else { 1.0 }))
        * glam::Mat4::from_translation(glam::Vec3::new(-0.5, -0.518, -0.5))
}

fn block_overlay(fuse_ticks: i32, alpha: f32, hurt: bool) -> u32 {
    let fuse = if fuse_ticks >= 0 {
        fuse_ticks as f32 - alpha + 1.0
    } else {
        0.0
    };
    if fuse > 0.0 && (fuse / 5.0) as i32 % 2 == 0 {
        horizon_client::overlay::living(1.0, false)
    } else {
        horizon_client::overlay::living(0.0, hurt)
    }
}

fn block_light(light: u32, emission: u8) -> u32 {
    let block = (light >> 8 & 0xFF).max(u32::from(emission) * 16);
    (light & 0xFF) | block << 8
}

impl Game<'_> {
    /// `SulfurCubeInnerLayer.submit`: the renderable block state inside a sulfur cube, if any.
    pub(super) fn sulfur_cube_contained_block(&self, mob: &Mob) -> Option<BlockStateId> {
        let item = mob.extra.body_id()?;
        if super::armor_stands::skull_kind(item).is_some() {
            return None;
        }
        // shortcut: the body slot retains only an item id; preserve the stack
        // before supporting its block-state components here.
        self.items().get(item).and_then(|item| item.block)
    }

    /// `SulfurCubeInnerLayer.submit`: the swallowed block replaces the opaque core and precedes the shell.
    pub(super) fn sulfur_cube_body_geometry(
        &self,
        mob: &Mob,
        alpha: f32,
        base: glam::Mat4,
        light: u32,
        out: &mut Vec<Vertex>,
    ) -> bool {
        let Some(state) = self.sulfur_cube_contained_block(mob) else {
            return false;
        };
        let start = out.len();
        self.block_model_vertices(
            state,
            42,
            block_transform(base, mob.is_baby()),
            block_light(light, self.res.facts().get(state).emission),
            true,
            out,
        );
        let overlay = block_overlay(
            sulfur_cube::fuse(mob),
            alpha,
            mob.hurt_time > 0 || mob.death_time > 0,
        );
        if overlay != 0 {
            for vertex in &mut out[start..] {
                vertex.layer = overlay;
            }
        }
        out.len() > start
    }

    fn sulfur_sound(&mut self, index: usize, sound: &str) {
        let at = self.mobs[index].center();
        self.audio
            .play(&format!("minecraft:{sound}"), Some(at), 1.0, 1.0);
    }
    /// Spit the swallowed block out on top (`EntityAttachment.PASSENGER`).
    fn eject_sulfur_block(&mut self, index: usize) {
        let m = &mut self.mobs[index];
        let Some(item) = m.take_body_equipment() else {
            return;
        };
        let at = [
            m.position.x,
            m.position.y + f64::from(m.height()),
            m.position.z,
        ];
        self.spawn_item(item, at, [0.0, 0.2, 0.0], 10);
    }
    /// `SulfurCube.mobInteract`. Returns whether the click was used.
    pub(super) fn interact_sulfur_cube(&mut self, index: usize, item: &str) -> bool {
        let m = &self.mobs[index];
        if m.species() != Species::SulfurCube || !m.alive() {
            return false;
        }
        if m.is_baby() {
            // Slime balls grow a baby up by a tenth (`canAgeUp`: not while an
            // age lock holds it).
            if !m.kind.food.iter().any(|f| f == item) || !horizon_client::mobs::ai::can_age_up(m) {
                return false;
            }
            let m = &mut self.mobs[index];
            let secs = (-m.extra.age / 20) as f32 * 0.1;
            // AgeableMob.ageUp: a grown cube is size 2 (SulfurCube.ageBoundaryReached).
            horizon_client::mobs::ai::age_up(m, secs as i32, false);
            self.consume_held();
            self.sulfur_sound(index, "entity.small_sulfur_cube.eat");
            return true;
        }
        if sulfur_cube::primed(m) {
            return false;
        }
        let explosive = sulfur_cube::archetype(m).is_some_and(|a| a.explosion.is_some());
        if explosive && matches!(item, "minecraft:flint_and_steel" | "minecraft:fire_charge") {
            if !self
                .ui
                .rules
                .bool_at(horizon_client::gamerules::vanilla::TNT_EXPLODES)
            {
                let message = Text::translate("block.minecraft.tnt.disabled", &[]);
                self.send_overlay_message(message);
                return false;
            }
            sulfur_cube::prime(&mut self.mobs[index], false);
            if item == "minecraft:flint_and_steel" {
                self.damage_held_tool();
            } else {
                self.consume_held();
            }
            self.award_stat("used", item, 1);
            return true;
        }
        if item == "minecraft:shears" && sulfur_cube::has_body(m) {
            // shear: out comes the block, and nothing is swallowed for 5 s.
            let sheared = self.mobs[index].extra.body_stack().map(|s| s.item.clone());
            self.eject_sulfur_block(index);
            self.mobs[index].extra.ints[3] = sulfur_cube::PICKUP_TIMER;
            self.sulfur_sound(index, "entity.sulfur_cube.eject");
            self.damage_held_tool();
            // gameEvent(SHEAR, player).
            let at = self.mobs[index].position;
            let ctx = self.who_context(Some(Who::Player), None);
            self.post_game_event("minecraft:shear", [at.x, at.y, at.z], ctx);
            // CriteriaTriggers.PLAYER_SHEARED_EQUIPMENT with the body block.
            if let Some(body) = sheared {
                let target = self.mob_entity(&self.mobs[index]);
                self.trigger_item_entity(
                    "minecraft:player_sheared_equipment",
                    &ItemStack::new(&body, 1),
                    &target,
                );
            }
            return true;
        }
        if sulfur_cube::swallowable(m, item) {
            // equipItem: not the same block again; the old one comes out.
            if m.extra.body_id() == Some(item) {
                return false;
            }
            self.eject_sulfur_block(index);
            let held = self.inventory[self.selected]
                .clone()
                .map(|s| ItemStack { count: 1, ..s });
            self.mobs[index].extra.set_body_stack(held);
            self.consume_held();
            self.sulfur_sound(index, "entity.sulfur_cube.absorb");
            // Entity.gameEvent(ENTITY_INTERACT): the cube is the source.
            let (id, at) = (self.mobs[index].id, self.mobs[index].position);
            let ctx = self.who_context(Some(Who::Mob(id)), None);
            self.post_game_event("minecraft:entity_interact", [at.x, at.y, at.z], ctx);
            return true;
        }
        if item == "minecraft:bucket" {
            self.bucket_sulfur_cube(index);
            return true;
        }
        false
    }
    /// `Bucketable.bucketMobPickup` for a sulfur cube
    /// (`SulfurCube.canBePickedUpWithBucket`: an empty bucket), as the water
    /// bucket's mobs are picked up in `interact_mob`.
    fn bucket_sulfur_cube(&mut self, index: usize) {
        let held = self.held().cloned();
        let m = &self.mobs[index];
        self.audio.play(
            "minecraft:item.bucket.fill_sulfur_cube",
            Some(m.center()),
            1.0,
            1.0,
        );
        // getBucketItemStack, then saveToBucketTag.
        let mut filled = ItemStack::new("minecraft:sulfur_cube_bucket", 1);
        filled.extra = Some(Box::new(horizon_client::entity_nbt::bucket_components(m)));
        self.trigger_item("minecraft:filled_bucket", &filled);
        // PlayerInteractTrigger: the bucket as it was, and the cube picked up.
        if let Some(stack) = &held {
            let target = self.mob_entity(&self.mobs[index]);
            self.trigger_item_entity("minecraft:player_interacted_with_entity", stack, &target);
        }
        let id = self.mobs[index].id;
        // Leashable.dropLeash, then discard.
        self.break_leash(id, true, false);
        self.mobs[index].removed = true;
        // ItemUtils.createFilledResult.
        self.filled_result(filled);
        if let (Some(player), Some(name)) = (self.lan_acting_uuid, self.lan_acting_name.clone()) {
            self.native_player_bucket_entity(player, &name, id as i32, "minecraft:sulfur_cube");
        }
    }
    /// The resident sulfur cubes whose adopted body (`adopted_body`, the
    /// body `collectEquipmentChanges` last took) explodes, with its fuse:
    /// their cached `explosionData`, which `primeWhenOnPoweredPosition` may
    /// use this tick although the body has changed since.
    pub(super) fn sulfur_cubes_adopted_explosive(&self) -> Vec<(u32, i32)> {
        self.mobs
            .iter()
            .filter(|m| m.species() == Species::SulfurCube)
            .filter_map(|m| Some((m.id, sulfur_cube::adopted_explosion(m)?.0)))
            .collect()
    }
    /// `SulfurCube.primeWhenOnPoweredPosition`: an explosive cube standing in
    /// or next to redstone power lights its fuse (`getBestOwnOrNeighbourSignal`;
    /// no block has an own signal, so the neighbour signal is all there is).
    /// Only the cubes `primable` names (an adopted explosive body when the
    /// tick started) can: `canExplode` and `primeTime` both read the cached
    /// `explosionData`, so the fuse is that body's, not the live one's.
    pub(super) fn sulfur_cube_power_priming(&mut self, primable: &[(u32, i32)]) {
        if !self
            .ui
            .rules
            .bool_at(horizon_client::gamerules::vanilla::TNT_EXPLODES)
        {
            return;
        }
        // Vanilla calls primeWhenOnPoweredPosition from SulfurCube.tick,
        // not from a block-change callback. Frozen descendants never poll power.
        let blocked = self.mobs_below_off_level();
        for i in 0..self.mobs.len() {
            let m = &self.mobs[i];
            if blocked.contains(&m.id)
                || m.species() != Species::SulfurCube
                || !m.alive()
                || sulfur_cube::primed(m)
            {
                continue;
            }
            let Some(&(_, fuse)) = primable.iter().find(|(id, _)| *id == m.id) else {
                continue;
            };
            let at = BlockPos::new(
                m.position.x.floor() as i32,
                m.position.y.floor() as i32,
                m.position.z.floor() as i32,
            );
            let signal =
                self.with_level(|b, level, ticks| b.best_neighbor_signal(level, ticks, at));
            if signal != 0 {
                sulfur_cube::prime_with_fuse(&mut self.mobs[i], false, fuse);
            }
        }
    }
    /// `SulfurCube.hurtServer`: an arrow or a trident on fire that hits an
    /// explosive cube with a block lights its fuse (with `tnt_explodes`, as
    /// `primeTime` asks), called after the hit's hurt, which the priming
    /// leaves as it is, so its knock and the fuse both go ahead.
    pub(super) fn burning_arrow_primes(&mut self, p: &Projectile, who: Who) {
        if !p.arrow.as_ref().is_some_and(|a| a.on_fire())
            || !self
                .ui
                .rules
                .bool_at(horizon_client::gamerules::vanilla::TNT_EXPLODES)
        {
            return;
        }
        if let Who::Mob(id) = who
            && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id)
            && m.species() == Species::SulfurCube
        {
            sulfur_cube::burning_arrow_hit(m);
        }
    }
    /// `SulfurCubeSearchForItemsGoal` and `pickUpItem`: an adult with nothing
    /// inside hops to a block it can swallow within 8 and takes it in.
    pub(super) fn sulfur_cube_pickups(&mut self) {
        let griefing = self
            .ui
            .rules
            .bool_at(horizon_client::gamerules::vanilla::MOB_GRIEFING);
        for i in 0..self.mobs.len() {
            let m = &self.mobs[i];
            if m.species() != Species::SulfurCube || !self.creatures.ticked.contains(&m.id) {
                continue;
            }
            if !m.alive() || m.is_baby() || sulfur_cube::has_body(m) || m.extra.ints[3] > 0 {
                self.mobs[i].extra.point = None;
                continue;
            }
            let reach = m
                .bounds()
                .inflate(horizon_core::geometry::Vec3::new(8.0, 8.0, 8.0));
            let pos = m.position;
            let best = self
                .dropped
                .iter()
                .enumerate()
                .filter(|(_, e)| {
                    e.pickup_delay == 0
                        && reach.contains(e.position)
                        && sulfur_cube::swallowable(m, &e.stack.item)
                })
                .map(|(j, e)| (j, e.position.distance_squared(pos)))
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let Some((j, _)) = best else {
                self.mobs[i].extra.point = None;
                continue;
            };
            // Mob.aiStep: items within the box grown by (1, 0, 1).
            let touch = m
                .bounds()
                .inflate(horizon_core::geometry::Vec3::new(1.0, 0.0, 1.0));
            let e = &self.dropped[j];
            let close = touch.contains(e.position);
            self.mobs[i].extra.point = Some([e.position.x, e.position.y, e.position.z]);
            if !griefing || !close {
                continue;
            }
            let thrown_by_player = e
                .thrower
                .is_some_and(|uuid| Some(uuid) == self.level.player_uuid);
            let mut swallowed = e.stack.clone();
            swallowed.count = 1;
            let (taken_item, id) = (e.clone(), self.mobs[i].id);
            self.add_pickup(
                super::pickups::Taken::Item(Box::new(taken_item)),
                super::pickups::Collector::Mob(id),
            );
            let e = &mut self.dropped[j];
            e.stack.count -= 1;
            if e.stack.count == 0 {
                self.remove_dropped(j);
            }
            let m = &mut self.mobs[i];
            m.extra.set_body_stack(Some(swallowed.clone()));
            m.extra.point = None;
            self.sulfur_sound(i, "entity.sulfur_cube.absorb");
            if thrown_by_player {
                let target = self.mob_entity(&self.mobs[i]);
                self.trigger_item_entity(
                    "minecraft:thrown_item_picked_up_by_entity",
                    &swallowed,
                    &target,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swallowed_block_uses_vanilla_lift_and_baby_scale() {
        let base = glam::Mat4::from_translation(glam::Vec3::new(3.0, 4.0, -2.0))
            * glam::Mat4::from_rotation_y(0.7)
            * glam::Mat4::from_scale(glam::Vec3::new(-1.0, -1.0, 1.0));
        for baby in [false, true] {
            let size = if baby { 0.5 } else { 1.0 };
            let block = block_transform(base, baby);
            for (point, expected) in [
                (glam::Vec3::ZERO, glam::Vec3::new(-0.5, 0.518, 0.5)),
                (glam::Vec3::ONE, glam::Vec3::new(0.5, -0.482, -0.5)),
            ] {
                let actual = block.transform_point3(point);
                let expected = base.transform_point3(expected * size);
                assert!(actual.abs_diff_eq(expected, 1e-5), "baby={baby}");
            }
            assert!(block.transform_vector3(glam::Vec3::Y).y > 0.0);
        }
    }

    #[test]
    fn swallowed_tnt_flashes_white_over_hurt_using_the_interpolated_fuse() {
        let white = horizon_client::overlay::WHITE + 15;
        assert_eq!(block_overlay(-1, 0.0, false), 0);
        assert_eq!(block_overlay(-1, 0.0, true), 1);
        assert_eq!(block_overlay(9, 0.0, true), white);
        assert_eq!(block_overlay(9, 0.5, false), 0);
        assert_eq!(block_overlay(9, 0.5, true), 1);
        assert_eq!(block_overlay(0, 0.5, false), white);
    }

    #[test]
    fn contained_block_emission_preserves_brighter_light_and_the_sky_channel() {
        assert_eq!(block_light(0x2050, 1), 0x2050);
        assert_eq!(block_light(0x2050, 7), 0x7050);
        assert_eq!(block_light(0x0070, 0), 0x0070);
    }
}

#[cfg(test)]
mod off_level_tests {
    use super::*;
    use horizon_core::geometry::Vec3;

    #[test]
    #[ignore = "daily"]
    fn power_does_not_prime_a_frozen_sulfur_cube() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            let power = game
                .pack
                .assets
                .registry
                .default_state("minecraft:redstone_block")
                .unwrap();
            let kind = game
                .res
                .mob_types()
                .get("minecraft:sulfur_cube")
                .unwrap()
                .clone();
            for (id, x, mount) in [(73, 1, Some(72)), (74, 4, None)] {
                game.world.set(x, 79, 1, power).unwrap();
                let mut m = horizon_client::mobs::Mob::new(
                    id,
                    kind.clone(),
                    Vec3::new(f64::from(x), 80.0, 1.0),
                    0.0,
                );
                m.extra.set_body(Some("minecraft:tnt".into()));
                m.extra.armor_attributes = game.motion.attributes.create(&m.kind.id).map(Box::new);
                sulfur_cube::adopt_body(&mut m);
                m.extra.mount = mount;
                game.mobs.push(m);
            }
            let ghost = horizon_client::mobs::Mob::new(72, kind, Vec3::ZERO, 0.0);
            game.persist
                .off_level
                .insert(72, crate::game::persist::OffLevel::Mob(ghost));
            let primable = game.sulfur_cubes_adopted_explosive();
            game.sulfur_cube_power_priming(&primable);
            assert!(!sulfur_cube::primed(game.mob_object(73).unwrap()));
            assert!(sulfur_cube::primed(game.mob_object(74).unwrap()));
        });
    }
}
