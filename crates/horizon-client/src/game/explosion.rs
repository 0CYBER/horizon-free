//! `ServerExplosion`: exploded positions (vanilla ray march over a Java
//! HashSet, so iteration and the following shuffle match vanilla's random
//! draws), entity damage with exposure and knockback, block interaction with
//! merged drops and TNT chain priming, and optional fire.
use super::Game;
use horizon_client::{Result, blocks::Fluid, items::ItemStack};
use horizon_core::{
    geometry::Vec3,
    position::BlockPos,
    random::{LegacyRandom, RandomSource},
};
use horizon_world::java_hash_map::{JavaHashMap, JavaKey};
use std::cmp::Ordering;

/// `BlockPos` with Java `Vec3i.hashCode` / `compareTo`.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Key([i32; 3]);
impl JavaKey for Key {
    fn java_hash(&self) -> i32 {
        let [x, y, z] = self.0;
        y.wrapping_add(z.wrapping_mul(31))
            .wrapping_mul(31)
            .wrapping_add(x)
    }
    fn comparable_compare(&self, other: &Self) -> Option<Ordering> {
        let ([x, y, z], [ox, oy, oz]) = (self.0, other.0);
        Some(y.cmp(&oy).then(z.cmp(&oz)).then(x.cmp(&ox)))
    }
    fn tie_break(&self, _other: &Self) -> Ordering {
        Ordering::Less
    }
}

/// `Explosion.BlockInteraction` for the explosions Horizon produces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BlockInteraction {
    Keep,
    Destroy,
    DestroyWithDecay,
}

thread_local! {
    /// Client-side randomness (explosion sound pitch), separate from the level random.
    static CLIENT_RANDOM: std::cell::RefCell<LegacyRandom> = std::cell::RefCell::new(LegacyRandom::new(0x5eed));
}
pub(super) fn client_float() -> f32 {
    CLIENT_RANDOM.with(|r| r.borrow_mut().next_float())
}

impl Game<'_> {
    /// `Level.clip(from, to, COLLIDER, NONE)` missing every block, with full
    /// collision cubes as obstacles.
    fn clear_line(&self, from: [f64; 3], to: [f64; 3]) -> bool {
        let d = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
        let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let steps = (length / 0.1).ceil().max(1.0) as i32;
        let mut last = [i32::MIN; 3];
        for i in 0..=steps {
            let t = f64::from(i) / f64::from(steps);
            let b = [
                (from[0] + d[0] * t).floor() as i32,
                (from[1] + d[1] * t).floor() as i32,
                (from[2] + d[2] * t).floor() as i32,
            ];
            if b == last {
                continue;
            }
            last = b;
            if self
                .res
                .facts
                .get(self.world.block(b[0], b[1], b[2]))
                .full_collision
            {
                return false;
            }
        }
        true
    }
    /// `ServerExplosion.getSeenPercent` for an entity box.
    pub(super) fn seen_percent(&self, center: [f64; 3], min: [f64; 3], max: [f64; 3]) -> f32 {
        let xs = 1.0 / ((max[0] - min[0]) * 2.0 + 1.0);
        let ys = 1.0 / ((max[1] - min[1]) * 2.0 + 1.0);
        let zs = 1.0 / ((max[2] - min[2]) * 2.0 + 1.0);
        let x_offset = (1.0 - (1.0 / xs).floor() * xs) / 2.0;
        let z_offset = (1.0 - (1.0 / zs).floor() * zs) / 2.0;
        if xs < 0.0 || ys < 0.0 || zs < 0.0 {
            return 0.0;
        }
        let (mut hits, mut count) = (0, 0);
        let mut xx = 0.0;
        while xx <= 1.0 {
            let mut yy = 0.0;
            while yy <= 1.0 {
                let mut zz = 0.0;
                while zz <= 1.0 {
                    let x = min[0] + (max[0] - min[0]) * xx;
                    let y = min[1] + (max[1] - min[1]) * yy;
                    let z = min[2] + (max[2] - min[2]) * zz;
                    if self.clear_line([x + x_offset, y, z + z_offset], center) {
                        hits += 1;
                    }
                    count += 1;
                    zz += zs;
                }
                yy += ys;
            }
            xx += xs;
        }
        hits as f32 / count as f32
    }
    /// `ServerExplosion.calculateExplodedPositions`.
    fn exploded_positions(&mut self, center: [f64; 3], radius: f32) -> Vec<[i32; 3]> {
        let (min_y, max_y) = self.with_level(|_, level, _| {
            use horizon_world::level::LevelAccess;
            (level.min_y(), level.max_y())
        });
        let mut set: JavaHashMap<Key, ()> = JavaHashMap::default();
        for xx in 0..16 {
            for yy in 0..16 {
                for zz in 0..16 {
                    if !(xx == 0 || xx == 15 || yy == 0 || yy == 15 || zz == 0 || zz == 15) {
                        continue;
                    }
                    let mut xd = f64::from(xx as f32 / 15.0 * 2.0 - 1.0);
                    let mut yd = f64::from(yy as f32 / 15.0 * 2.0 - 1.0);
                    let mut zd = f64::from(zz as f32 / 15.0 * 2.0 - 1.0);
                    let d = (xd * xd + yd * yd + zd * zd).sqrt();
                    xd /= d;
                    yd /= d;
                    zd /= d;
                    let mut power = radius * (0.7 + self.world.random.next_float() * 0.6);
                    let (mut xp, mut yp, mut zp) = (center[0], center[1], center[2]);
                    while power > 0.0 {
                        let pos = [xp.floor() as i32, yp.floor() as i32, zp.floor() as i32];
                        if pos[1] < min_y
                            || pos[1] >= max_y
                            || !(-30_000_000..30_000_000).contains(&pos[0])
                            || !(-30_000_000..30_000_000).contains(&pos[2])
                        {
                            break;
                        }
                        let state = self.world.block(pos[0], pos[1], pos[2]);
                        let f = self.res.facts.get(state);
                        let fluid = if f.fluid != Fluid::None {
                            100.0f32
                        } else {
                            0.0
                        };
                        if !self.res.facts.is_air(state) || f.fluid != Fluid::None {
                            power -= (f.explosion_resistance.max(fluid) + 0.3) * 0.3;
                        }
                        if power > 0.0 {
                            set.put(Key(pos), ());
                        }
                        xp += xd * f64::from(0.3f32);
                        yp += yd * f64::from(0.3f32);
                        zp += zd * f64::from(0.3f32);
                        power -= 0.225_000_01;
                    }
                }
            }
        }
        set.entries().map(|(k, _)| k.0).collect()
    }
    /// `ServerExplosion.hurtEntities` (player, mobs, items, primed TNT).
    fn explosion_hurt_entities(&mut self, center: [f64; 3], radius: f32) {
        if radius < 1.0e-5 {
            return;
        }
        self.explosion_hits_special(center, radius);
        let diameter = f64::from(radius * 2.0);
        let distance = |p: [f64; 3]| {
            ((p[0] - center[0]).powi(2) + (p[1] - center[1]).powi(2) + (p[2] - center[2]).powi(2))
                .sqrt()
        };
        let damage = |dist: f64, exposure: f32| {
            let pow = (1.0 - dist) * f64::from(exposure);
            ((pow * pow + pow) / 2.0 * 7.0 * diameter + 1.0) as f32
        };
        let direction = |origin: [f64; 3]| {
            let d = [
                origin[0] - center[0],
                origin[1] - center[1],
                origin[2] - center[2],
            ];
            let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            if l < 1.0e-4 {
                [0.0; 3]
            } else {
                [d[0] / l, d[1] / l, d[2] / l]
            }
        };
        // Player.
        if let Some(p) = self.player.as_ref() {
            let feet = [p.position.x, p.position.y, p.position.z];
            let dist = distance(feet) / diameter;
            if dist <= 1.0 && !p.health.dead_or_dying() {
                let w = f64::from(p.dimensions.width) / 2.0;
                let h = f64::from(p.dimensions.height);
                let eye = [
                    feet[0],
                    feet[1] + f64::from(p.dimensions.eye_height),
                    feet[2],
                ];
                let exposure = self.seen_percent(
                    center,
                    [feet[0] - w, feet[1], feet[2] - w],
                    [feet[0] + w, feet[1] + h, feet[2] + w],
                );
                let amount = damage(dist, exposure);
                let dir = direction(eye);
                let power = (1.0 - dist) * f64::from(exposure);
                let mut hurt = false;
                let creative = self.creative;
                if let Some(p) = self.player.as_mut()
                    && let Some(id) = self.motion.health.damage.id("minecraft:explosion")
                {
                    // Creative players are invulnerable to explosions.
                    if !creative {
                        hurt = p
                            .hurt(&self.motion.health, self.world.health_rules, id, amount)
                            .unwrap_or(false);
                    }
                    p.velocity = Vec3::new(
                        p.velocity.x + dir[0] * power,
                        p.velocity.y + dir[1] * power,
                        p.velocity.z + dir[2] * power,
                    );
                }
                if hurt && !self.creative {
                    self.hurt_armor(amount);
                }
            }
        }
        // Mobs.
        let mut mobs = std::mem::take(&mut self.mobs);
        for m in mobs.iter_mut().filter(|m| m.alive()) {
            let feet = [m.position.x, m.position.y, m.position.z];
            let dist = distance(feet) / diameter;
            if dist > 1.0 {
                continue;
            }
            let (w, h) = (m.kind.width / 2.0, m.kind.height);
            let eye = [feet[0], feet[1] + m.kind.eye_height, feet[2]];
            let exposure = self.seen_percent(
                center,
                [feet[0] - w, feet[1], feet[2] - w],
                [feet[0] + w, feet[1] + h, feet[2] + w],
            );
            let source = horizon_client::mobs::Damage {
                kind: horizon_client::mobs::DamageKind::Explosion,
                attacker: None,
                from: Some(center),
            };
            if !m.damage(damage(dist, exposure), source) {
                continue;
            }
            let dir = direction(eye);
            let power = (1.0 - dist) * f64::from(exposure) * (1.0 - m.kind.knockback_resistance);
            m.velocity = Vec3::new(
                m.velocity.x + dir[0] * power,
                m.velocity.y + dir[1] * power,
                m.velocity.z + dir[2] * power,
            );
        }
        self.mobs = mobs;
        // Dropped items: 5 health, destroyed by the explosion damage (nether stars resist).
        let mut items = std::mem::take(&mut self.dropped);
        items.retain_mut(|item| {
            let feet = [item.position.x, item.position.y, item.position.z];
            let dist = distance(feet) / diameter;
            if dist > 1.0 {
                return true;
            }
            let half = 0.125;
            let exposure = self.seen_percent(
                center,
                [feet[0] - half, feet[1], feet[2] - half],
                [feet[0] + half, feet[1] + 0.25, feet[2] + half],
            );
            if &*item.stack.item != "minecraft:nether_star" && damage(dist, exposure) >= 5.0 {
                return false;
            }
            let dir = direction([feet[0], feet[1] + 0.2125, feet[2]]);
            let power = (1.0 - dist) * f64::from(exposure);
            item.velocity = Vec3::new(
                item.velocity.x + dir[0] * power,
                item.velocity.y + dir[1] * power,
                item.velocity.z + dir[2] * power,
            );
            true
        });
        self.dropped = items;
        // Primed TNT: knockback only, from its position (not its eyes).
        let mut tnts = std::mem::take(&mut self.tnts);
        for t in &mut tnts {
            let feet = [t.position.x, t.position.y, t.position.z];
            let dist = distance(feet) / diameter;
            if dist > 1.0 {
                continue;
            }
            let half = 0.49;
            let exposure = self.seen_percent(
                center,
                [feet[0] - half, feet[1], feet[2] - half],
                [feet[0] + half, feet[1] + 0.98, feet[2] + half],
            );
            let dir = direction(feet);
            let power = (1.0 - dist) * f64::from(exposure);
            t.velocity = Vec3::new(
                t.velocity.x + dir[0] * power,
                t.velocity.y + dir[1] * power,
                t.velocity.z + dir[2] * power,
            );
        }
        self.tnts = tnts;
    }
    /// `Level.explode` on the server: positions, entities, blocks, fire; then
    /// the client's explosion sound.
    pub(super) fn server_explode(
        &mut self,
        center: [f64; 3],
        radius: f32,
        interaction: BlockInteraction,
        fire: bool,
    ) -> Result<()> {
        let mut targets = self.exploded_positions(center, radius);
        // Explosion.explode: the blast is heard.
        let ctx = horizon_client::sculk::Context::default();
        self.post_game_event("minecraft:explode", center, ctx);
        self.explosion_hurt_entities(center, radius);
        if interaction != BlockInteraction::Keep {
            // Util.shuffle(list, level.random)
            for i in (2..=targets.len()).rev() {
                let swap = self.world.random.next_int_bound(i as i32).unwrap_or(0) as usize;
                targets.swap(i - 1, swap);
            }
            let mut stacks: Vec<([i32; 3], ItemStack)> = Vec::new();
            for &pos in &targets {
                self.explosion_hit_block(pos, radius, interaction, &mut stacks)?;
            }
            for (pos, stack) in stacks {
                self.pop_resource(pos, stack);
            }
        }
        if fire {
            for &pos in &targets {
                if self.world.random.next_int_bound(3).unwrap_or(1) == 0
                    && self
                        .res
                        .facts
                        .is_air(self.world.block(pos[0], pos[1], pos[2]))
                    && self
                        .res
                        .facts
                        .get(self.world.block(pos[0], pos[1] - 1, pos[2]))
                        .solid_render
                    && let Some(fire) = self.res.assets.registry.default_state("minecraft:fire")
                {
                    self.level_set(pos, fire)?;
                }
            }
        }
        let pitch = (1.0 + (client_float() - client_float()) * 0.2) * 0.7;
        self.audio
            .play("minecraft:entity.generic.explode", Some(center), 4.0, pitch);
        let block_count = if interaction == BlockInteraction::Keep {
            0
        } else {
            targets.len()
        };
        let (world, facts) = (&self.world, &self.res.facts);
        self.particles.explosion(center, radius, block_count, &|b| {
            facts.is_air(world.block(b[0], b[1], b[2]))
        });
        Ok(())
    }
    /// `BlockBehaviour.onExplosionHit` (TNT: `wasExploded` primes a short fuse).
    fn explosion_hit_block(
        &mut self,
        pos: [i32; 3],
        radius: f32,
        interaction: BlockInteraction,
        stacks: &mut Vec<([i32; 3], ItemStack)>,
    ) -> Result<()> {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        if self.res.facts.is_air(state) {
            return Ok(());
        }
        let name = self.res.facts.get(state).name.clone();
        let is_tnt = &*name == "minecraft:tnt";
        // Block.onExplosionHit: the drops copy from the block entity, which
        // then spills its contents as the block turns to air.
        let key = (pos[0], pos[1], pos[2]);
        if let Some(entity) = self.world.block_entities.get(&key).cloned() {
            let custom = self.world.block_names.get(&key).cloned();
            self.pending.record(pos, &entity, &name, custom);
        } else if let Some(tag) = super::spill::saved_entity(&self.world, pos) {
            self.pending.record_saved(pos, &tag);
        }
        if !is_tnt {
            let dynamic = self.pending.dynamic(pos);
            let drops = self.res.items.block_drops_with(
                self.res.assets.registry,
                state,
                None,
                &dynamic,
                &mut self.world.random,
            );
            for mut stack in drops {
                if &*stack.item == "minecraft:air" || stack.count == 0 {
                    continue;
                }
                self.pending.attach(pos, &mut stack);
                // survives_explosion / explosion_decay: 1 / radius per stack.
                if interaction == BlockInteraction::DestroyWithDecay
                    && self.world.random.next_float() > 1.0 / radius
                {
                    continue;
                }
                add_or_append_stack(stacks, stack, pos);
            }
        }
        self.pending.clear(pos);
        self.level_set(pos, self.world.air)?;
        if is_tnt {
            self.spawn_primed_tnt(BlockPos::new(pos[0], pos[1], pos[2]), true);
        }
        Ok(())
    }
}

/// `ServerExplosion.addOrAppendStack` with `ItemEntity.merge(.., 16)`.
fn add_or_append_stack(
    stacks: &mut Vec<([i32; 3], ItemStack)>,
    mut stack: ItemStack,
    pos: [i32; 3],
) {
    for (_, existing) in stacks.iter_mut() {
        if existing.item == stack.item && existing.damage == stack.damage && existing.count < 16 {
            let moved = (16 - existing.count).min(stack.count);
            existing.count += moved;
            stack.count -= moved;
            if stack.count == 0 {
                return;
            }
        }
    }
    stacks.push((pos, stack));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_pos_hash_matches_vec3i() {
        // Vec3i.hashCode: (y + z * 31) * 31 + x
        assert_eq!(Key([1, 2, 3]).java_hash(), (2 + 3 * 31) * 31 + 1);
        assert_eq!(Key([-5, 64, -7]).java_hash(), (64 - 7 * 31) * 31 - 5);
        assert_eq!(
            Key([0, 1, 0]).comparable_compare(&Key([5, 0, 9])),
            Some(Ordering::Greater)
        );
        assert_eq!(
            Key([1, 0, 2]).comparable_compare(&Key([0, 0, 3])),
            Some(Ordering::Less)
        );
    }

    #[test]
    fn drops_merge_up_to_sixteen() {
        let stack = |item: &str| ItemStack {
            item: item.into(),
            count: 1,
            damage: 0,
            extra: None,
        };
        let mut stacks = Vec::new();
        for _ in 0..20 {
            add_or_append_stack(&mut stacks, stack("minecraft:dirt"), [0, 0, 0]);
        }
        add_or_append_stack(&mut stacks, stack("minecraft:stone"), [1, 0, 0]);
        let counts: Vec<u32> = stacks.iter().map(|(_, s)| s.count).collect();
        assert_eq!(counts, vec![16, 4, 1]);
    }
}
