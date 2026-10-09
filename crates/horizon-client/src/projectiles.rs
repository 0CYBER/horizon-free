//! Projectiles fired by mobs and thrown by the player: arrows
//! (`AbstractArrow`, gravity 0.05), throwables (`ThrowableProjectile`:
//! snowballs, eggs, ender pearls, gravity 0.03) and fireballs
//! (`AbstractHurtingProjectile`: no gravity, 0.1 acceleration, 0.95 inertia).
//! The game applies the effects of a hit.
use crate::mobs::Who;
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::RandomSource,
};

#[derive(Clone, Debug, PartialEq)]
pub enum ProjectileKind {
    /// Arrow with its base damage, critical flag and tipped/stray/bogged effect.
    Arrow {
        base_damage: f64,
        critical: bool,
        effect: Option<(&'static str, i32)>,
    },
    Snowball,
    Egg,
    EnderPearl,
    SmallFireball,
    LargeFireball {
        power: i32,
    },
    LlamaSpit,
    /// `ShulkerBullet`: homing on `target` in axis-aligned steps.
    ShulkerBullet {
        target: Who,
        /// The current step direction (3D data value), if any.
        dir: Option<u8>,
        steps: i32,
        delta: [f64; 3],
        seed: u64,
    },
    /// `WitherSkull`: blue (dangerous) skulls keep their speed better.
    WitherSkull {
        dangerous: bool,
    },
    /// Splash potion thrown by witches: effect id, duration.
    Potion {
        effect: &'static str,
        ticks: i32,
        amplifier: i32,
        color: u32,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub struct Projectile {
    pub kind: ProjectileKind,
    pub owner: Option<Who>,
    pub position: Vec3,
    pub previous: Vec3,
    pub velocity: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub age: u32,
    pub stuck: bool,
    /// Whether the projectile has left its owner's box (Projectile.leftOwner).
    pub left_owner: bool,
}
/// What a projectile struck this tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    None,
    Block {
        pos: [i32; 3],
        at: Vec3,
        face: [i32; 3],
    },
    Entity {
        who: Who,
        at: Vec3,
    },
}
/// The world a projectile flies through: block ray clipping, water, and
/// the boxes of entities it could hit.
pub trait ProjectileWorld {
    fn clip(&self, from: Vec3, to: Vec3) -> Option<(Vec3, [i32; 3], [i32; 3])>;
    fn in_water(&self, at: Vec3) -> bool;
    fn targets(&self) -> &[(Who, Aabb)];
    /// `Level.isEmptyBlock`.
    fn empty(&self, _p: [i32; 3]) -> bool {
        true
    }
    /// `loadedAndEntityCanStandOn`: a sturdy top face.
    fn solid(&self, _p: [i32; 3]) -> bool {
        false
    }
}
/// Direction steps by 3D data value: down, up, north, south, west, east.
pub const STEPS: [[i32; 3]; 6] = [
    [0, -1, 0],
    [0, 1, 0],
    [0, 0, -1],
    [0, 0, 1],
    [-1, 0, 0],
    [1, 0, 0],
];
fn axis_of(d: u8) -> u8 {
    match d {
        0 | 1 => 1,
        2 | 3 => 2,
        _ => 0,
    }
}
impl Projectile {
    pub fn new(kind: ProjectileKind, position: Vec3, owner: Option<Who>) -> Self {
        Self {
            kind,
            owner,
            position,
            previous: position,
            velocity: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            age: 0,
            stuck: false,
            left_owner: false,
        }
    }
    pub fn is_arrow(&self) -> bool {
        matches!(self.kind, ProjectileKind::Arrow { .. })
    }
    pub fn is_fireball(&self) -> bool {
        matches!(
            self.kind,
            ProjectileKind::SmallFireball
                | ProjectileKind::LargeFireball { .. }
                | ProjectileKind::WitherSkull { .. }
        )
    }
    fn gravity(&self) -> f64 {
        match self.kind {
            ProjectileKind::Arrow { .. } => 0.05,
            ProjectileKind::SmallFireball
            | ProjectileKind::LargeFireball { .. }
            | ProjectileKind::WitherSkull { .. } => 0.0,
            ProjectileKind::LlamaSpit => 0.06,
            ProjectileKind::Potion { .. } => 0.05,
            _ => 0.03,
        }
    }
    /// Projectile.shoot: normalized direction plus triangular noise, times speed.
    pub fn shoot(
        &mut self,
        random: &mut impl RandomSource,
        dir: [f64; 3],
        speed: f32,
        uncertainty: f32,
    ) {
        let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2])
            .sqrt()
            .max(1.0e-9);
        let spread = 0.017_227_5 * f64::from(uncertainty);
        let v = [
            (dir[0] / len + random.triangle(0.0, spread)) * f64::from(speed),
            (dir[1] / len + random.triangle(0.0, spread)) * f64::from(speed),
            (dir[2] / len + random.triangle(0.0, spread)) * f64::from(speed),
        ];
        self.velocity = Vec3::new(v[0], v[1], v[2]);
        self.update_rotation();
    }
    /// Projectile.shootFromRotation for player throws.
    pub fn shoot_from_rotation(
        &mut self,
        random: &mut impl RandomSource,
        pitch: f32,
        yaw: f32,
        offset: f32,
        speed: f32,
        uncertainty: f32,
        source_velocity: Vec3,
        source_on_ground: bool,
    ) {
        let (p, y, o) = (
            pitch.to_radians(),
            yaw.to_radians(),
            (pitch + offset).to_radians(),
        );
        let dir = [
            f64::from(-y.sin() * p.cos()),
            f64::from(-o.sin()),
            f64::from(y.cos() * p.cos()),
        ];
        self.shoot(random, dir, speed, uncertainty);
        self.velocity = Vec3::new(
            self.velocity.x + source_velocity.x,
            self.velocity.y
                + if source_on_ground {
                    0.0
                } else {
                    source_velocity.y
                },
            self.velocity.z + source_velocity.z,
        );
    }
    /// AbstractHurtingProjectile: movement along the direction at the acceleration power.
    pub fn fireball(
        kind: ProjectileKind,
        position: Vec3,
        owner: Option<Who>,
        dir: [f64; 3],
    ) -> Self {
        let mut p = Self::new(kind, position, owner);
        let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2])
            .sqrt()
            .max(1.0e-9);
        p.velocity = Vec3::new(dir[0] / len * 0.1, dir[1] / len * 0.1, dir[2] / len * 0.1);
        p.update_rotation();
        p
    }
    fn update_rotation(&mut self) {
        let v = self.velocity;
        let sd = (v.x * v.x + v.z * v.z).sqrt();
        self.yaw = v.x.atan2(v.z).to_degrees() as f32;
        self.pitch = v.y.atan2(sd).to_degrees() as f32;
    }
    /// A shulker's bullet from its block toward `target`, first leaving
    /// along an axis other than the one it is attached on.
    pub fn shulker_bullet(
        w: &impl ProjectileWorld,
        owner_block: [i32; 3],
        owner: Who,
        target: Who,
        attach_axis: u8,
        seed: u64,
    ) -> Self {
        let at = Vec3::new(
            f64::from(owner_block[0]) + 0.5,
            f64::from(owner_block[1]) + 0.5,
            f64::from(owner_block[2]) + 0.5,
        );
        let mut p = Self::new(
            ProjectileKind::ShulkerBullet {
                target,
                dir: Some(1),
                steps: 0,
                delta: [0.0; 3],
                seed,
            },
            at,
            Some(owner),
        );
        p.select_next_move(w, Some(attach_axis));
        p
    }
    fn bullet_random(&mut self, bound: i32) -> i32 {
        let ProjectileKind::ShulkerBullet { seed, .. } = &mut self.kind else {
            return 0;
        };
        // xorshift64*: the bullet's own random stream.
        *seed ^= *seed >> 12;
        *seed ^= *seed << 25;
        *seed ^= *seed >> 27;
        let v = seed.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33;
        (v % bound.max(1) as u64) as i32
    }
    fn block_pos(&self) -> [i32; 3] {
        [
            self.position.x.floor() as i32,
            self.position.y.floor() as i32,
            self.position.z.floor() as i32,
        ]
    }
    fn target_box(&self, w: &impl ProjectileWorld) -> Option<Aabb> {
        let ProjectileKind::ShulkerBullet { target, .. } = self.kind else {
            return None;
        };
        w.targets()
            .iter()
            .find(|(who, _)| *who == target)
            .map(|(_, b)| *b)
    }
    /// `ShulkerBullet.selectNextMoveDirection`.
    fn select_next_move(&mut self, w: &impl ProjectileWorld, excluded: Option<u8>) {
        let (target, y_offset) = match self.target_box(w) {
            Some(b) => {
                let (lo, hi) = (b.min_position(), b.max_position());
                let half = (hi.y - lo.y) * 0.5;
                let c = [(lo.x + hi.x) / 2.0, lo.y + half, (lo.z + hi.z) / 2.0];
                (c.map(|v| v.floor() as i32), half)
            }
            None => {
                let p = self.block_pos();
                ([p[0], p[1] - 1, p[2]], 0.5)
            }
        };
        let mut goal = [
            f64::from(target[0]) + 0.5,
            f64::from(target[1]) + y_offset,
            f64::from(target[2]) + 0.5,
        ];
        let mut selection = None;
        let centre = [
            f64::from(target[0]) + 0.5,
            f64::from(target[1]) + 0.5,
            f64::from(target[2]) + 0.5,
        ];
        let d2 = (0..3)
            .map(|i| ([self.position.x, self.position.y, self.position.z][i] - centre[i]).powi(2))
            .sum::<f64>();
        if d2 >= 4.0 {
            let cur = self.block_pos();
            let mut options = Vec::new();
            let step = |d: u8| {
                let s = STEPS[d as usize];
                [cur[0] + s[0], cur[1] + s[1], cur[2] + s[2]]
            };
            for (axis, (lo, hi)) in [(0u8, (4u8, 5u8)), (1, (0, 1)), (2, (2, 3))] {
                if excluded == Some(axis) {
                    continue;
                }
                let i = axis as usize;
                if cur[i] < target[i] && w.empty(step(hi)) {
                    options.push(hi);
                } else if cur[i] > target[i] && w.empty(step(lo)) {
                    options.push(lo);
                }
            }
            let mut pick = self.bullet_random(6) as u8;
            if options.is_empty() {
                let mut i = 5;
                while !w.empty(step(pick)) && i > 0 {
                    pick = self.bullet_random(6) as u8;
                    i -= 1;
                }
            } else {
                pick = options[self.bullet_random(options.len() as i32) as usize];
            }
            let s = STEPS[pick as usize];
            goal = [
                self.position.x + f64::from(s[0]),
                self.position.y + f64::from(s[1]),
                self.position.z + f64::from(s[2]),
            ];
            selection = Some(pick);
        }
        let d = [
            goal[0] - self.position.x,
            goal[1] - self.position.y,
            goal[2] - self.position.z,
        ];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let delta = if len == 0.0 {
            [0.0; 3]
        } else {
            d.map(|v| v / len * 0.15)
        };
        let steps_next = 10 + self.bullet_random(5) * 10;
        if let ProjectileKind::ShulkerBullet {
            dir,
            steps,
            delta: dl,
            ..
        } = &mut self.kind
        {
            *dir = selection;
            *dl = delta;
            *steps = steps_next;
        }
    }
    /// `ShulkerBullet.tick`.
    fn tick_bullet(&mut self, w: &impl ProjectileWorld) -> (bool, Hit) {
        let target = self.target_box(w);
        let ProjectileKind::ShulkerBullet { delta, .. } = &mut self.kind else {
            return (false, Hit::None);
        };
        if target.is_some() {
            for d in delta.iter_mut() {
                *d = (*d * 1.025).clamp(-1.0, 1.0);
            }
            let dl = *delta;
            let v = self.velocity;
            self.velocity = Vec3::new(
                v.x + (dl[0] - v.x) * 0.2,
                v.y + (dl[1] - v.y) * 0.2,
                v.z + (dl[2] - v.z) * 0.2,
            );
        } else {
            self.velocity.y -= 0.04;
        }
        let from = self.position;
        let to = from.add_vector(self.velocity);
        let block = w.clip(from, to);
        let end = block.map_or(to, |b| b.0);
        let mut best: Option<(Who, Vec3, f64)> = None;
        for (who, b) in w.targets() {
            if Some(*who) == self.owner && !self.left_owner {
                continue;
            }
            if let Some(at) = b.inflate(Vec3::new(0.3, 0.3, 0.3)).clip(from, end) {
                let d = (at.x - from.x).powi(2) + (at.y - from.y).powi(2) + (at.z - from.z).powi(2);
                if best.is_none_or(|bb| d < bb.2) {
                    best = Some((*who, at, d));
                }
            }
        }
        if !self.left_owner {
            let owner_box = w
                .targets()
                .iter()
                .find(|(who, _)| Some(*who) == self.owner)
                .map(|(_, b)| *b);
            self.left_owner =
                owner_box.is_none_or(|b| !b.inflate(Vec3::new(1.0, 1.0, 1.0)).contains(from));
        }
        if let Some((who, at, _)) = best {
            self.position = at;
            return (false, Hit::Entity { who, at });
        }
        if let Some((at, pos, face)) = block {
            self.position = at;
            return (false, Hit::Block { pos, at, face });
        }
        self.position = to;
        self.update_rotation();
        if let Some(b) = target {
            let (dir, steps) = match self.kind {
                ProjectileKind::ShulkerBullet { dir, steps, .. } => (dir, steps),
                _ => (None, 0),
            };
            if steps > 0 {
                let left = steps - 1;
                if let ProjectileKind::ShulkerBullet { steps, .. } = &mut self.kind {
                    *steps = left;
                }
                if left == 0 {
                    self.select_next_move(w, dir.map(axis_of));
                }
            }
            let dir = match self.kind {
                ProjectileKind::ShulkerBullet { dir, .. } => dir,
                _ => None,
            };
            if let Some(d) = dir {
                let p = self.block_pos();
                let s = STEPS[d as usize];
                let axis = axis_of(d);
                let (lo, hi) = (b.min_position(), b.max_position());
                let t = [
                    ((lo.x + hi.x) / 2.0).floor() as i32,
                    lo.y.floor() as i32,
                    ((lo.z + hi.z) / 2.0).floor() as i32,
                ];
                if w.solid([p[0] + s[0], p[1] + s[1], p[2] + s[2]])
                    || p[axis as usize] == t[axis as usize]
                {
                    self.select_next_move(w, Some(axis));
                }
            }
        }
        (self.position.y > -128.0, Hit::None)
    }
    /// One tick: returns whether to keep the projectile and what it hit.
    pub fn tick(&mut self, w: &impl ProjectileWorld) -> (bool, Hit) {
        self.previous = self.position;
        self.age += 1;
        if matches!(self.kind, ProjectileKind::ShulkerBullet { .. }) {
            return self.tick_bullet(w);
        }
        if self.stuck {
            // AbstractArrow despawns 1200 ticks after sticking.
            return (self.age < 1200, Hit::None);
        }
        let water = w.in_water(self.position);
        match self.kind {
            ProjectileKind::SmallFireball
            | ProjectileKind::LargeFireball { .. }
            | ProjectileKind::WitherSkull { .. } => {
                // WitherSkull.getInertia: 0.73 for dangerous skulls.
                let inertia = match self.kind {
                    ProjectileKind::WitherSkull { dangerous: true } => 0.73,
                    _ if water => 0.8,
                    _ => 0.95,
                };
                let v = self.velocity;
                let len = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt().max(1.0e-9);
                self.velocity = Vec3::new(
                    (v.x + v.x / len * 0.1) * inertia,
                    (v.y + v.y / len * 0.1) * inertia,
                    (v.z + v.z / len * 0.1) * inertia,
                );
            }
            ProjectileKind::Arrow { .. } => {}
            _ => {
                // ThrowableProjectile: gravity, then inertia.
                self.velocity.y -= self.gravity();
                let inertia = if water { 0.8 } else { 0.99 };
                self.velocity = Vec3::new(
                    self.velocity.x * inertia,
                    self.velocity.y * inertia,
                    self.velocity.z * inertia,
                );
            }
        }
        let from = self.position;
        let to = Vec3::new(
            from.x + self.velocity.x,
            from.y + self.velocity.y,
            from.z + self.velocity.z,
        );
        let block = w.clip(from, to);
        let end = block.map_or(to, |b| b.0);
        // ProjectileUtil.getEntityHitResult: boxes inflated by 0.3 along the path.
        let mut best: Option<(Who, Vec3, f64)> = None;
        for (who, b) in w.targets() {
            if Some(*who) == self.owner && !self.left_owner {
                continue;
            }
            let inflated = b.inflate(Vec3::new(0.3, 0.3, 0.3));
            if let Some(at) = inflated.clip(from, end) {
                let d = (at.x - from.x).powi(2) + (at.y - from.y).powi(2) + (at.z - from.z).powi(2);
                if best.is_none_or(|bb| d < bb.2) {
                    best = Some((*who, at, d));
                }
            }
        }
        if !self.left_owner {
            // Leave the owner once no target box (the owner's) contains us.
            let owner_box = w
                .targets()
                .iter()
                .find(|(who, _)| Some(*who) == self.owner)
                .map(|(_, b)| *b);
            self.left_owner = owner_box
                .is_none_or(|b| !b.inflate(Vec3::new(1.0, 1.0, 1.0)).contains(self.position));
        }
        if let Some((who, at, _)) = best {
            self.position = at;
            return (false, Hit::Entity { who, at });
        }
        if let Some((at, pos, face)) = block {
            self.position = at;
            if self.is_arrow() {
                // AbstractArrow.onHitBlock: stick, nudged back out of the face.
                let v = self.velocity;
                let len = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt().max(1.0e-9);
                self.position = Vec3::new(
                    at.x - v.x / len * 0.05,
                    at.y - v.y / len * 0.05,
                    at.z - v.z / len * 0.05,
                );
                self.stuck = true;
                self.age = 0;
                return (true, Hit::Block { pos, at, face });
            }
            return (false, Hit::Block { pos, at, face });
        }
        self.position = to;
        if self.is_arrow() {
            // AbstractArrow.tick: inertia 0.99 (0.6 in water) after moving, then gravity.
            let inertia = if water { 0.6 } else { 0.99 };
            self.velocity = Vec3::new(
                self.velocity.x * inertia,
                self.velocity.y * inertia - self.gravity(),
                self.velocity.z * inertia,
            );
            self.update_rotation();
        } else if !self.is_fireball() {
            self.update_rotation();
        }
        let alive =
            self.position.y > -128.0 && self.age < if self.is_fireball() { 1200 } else { 6000 };
        (alive, Hit::None)
    }
    /// AbstractArrow.onHitEntity damage: ceil(speed * base) plus crit bonus.
    pub fn arrow_damage(&self, random: &mut impl RandomSource) -> f32 {
        let ProjectileKind::Arrow {
            base_damage,
            critical,
            ..
        } = self.kind
        else {
            return 0.0;
        };
        let v = self.velocity;
        let speed = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
        let mut damage = (speed * base_damage).clamp(0.0, f64::from(i32::MAX)).ceil() as i32;
        if critical {
            let bonus = random.next_int_bound(damage / 2 + 2).unwrap_or(0);
            damage = damage.saturating_add(bonus);
        }
        damage as f32
    }
    /// The item sprite drawn for thrown projectiles (ThrownItemRenderer).
    pub fn item_sprite(&self) -> Option<&'static str> {
        Some(match self.kind {
            ProjectileKind::Snowball => "item/snowball",
            ProjectileKind::Egg => "item/egg",
            ProjectileKind::EnderPearl => "item/ender_pearl",
            ProjectileKind::SmallFireball | ProjectileKind::LargeFireball { .. } => {
                "item/fire_charge"
            }
            ProjectileKind::Potion { .. } => "item/splash_potion",
            ProjectileKind::LlamaSpit => "entity/llama/llama_spit",
            ProjectileKind::ShulkerBullet { .. } => "entity/shulker/spark",
            ProjectileKind::WitherSkull { dangerous: false } => "entity/wither/wither",
            ProjectileKind::WitherSkull { dangerous: true } => "entity/wither/wither_invulnerable",
            ProjectileKind::Arrow { .. } => return None,
        })
    }
    /// Render scale of the item billboard (fireball renderers).
    pub fn sprite_scale(&self) -> f32 {
        match self.kind {
            ProjectileKind::LargeFireball { .. } => 3.0,
            ProjectileKind::SmallFireball => 0.75,
            _ => 1.0,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::random::LegacyRandom;

    struct Empty(Vec<(Who, Aabb)>);
    impl ProjectileWorld for Empty {
        fn clip(&self, _: Vec3, _: Vec3) -> Option<(Vec3, [i32; 3], [i32; 3])> {
            None
        }
        fn in_water(&self, _: Vec3) -> bool {
            false
        }
        fn targets(&self) -> &[(Who, Aabb)] {
            &self.0
        }
    }
    #[test]
    fn snowball_falls_with_throwable_gravity_and_drag() {
        let mut p = Projectile::new(ProjectileKind::Snowball, Vec3::new(0.0, 100.0, 0.0), None);
        p.velocity = Vec3::new(1.0, 0.0, 0.0);
        let w = Empty(vec![]);
        p.tick(&w);
        // ThrowableProjectile: gravity before inertia, then move.
        assert!((p.velocity.y - (-0.03 * 0.99)).abs() < 1e-9);
        assert!((p.position.x - 0.99).abs() < 1e-9);
    }
    #[test]
    fn arrow_applies_drag_then_gravity_after_moving() {
        let mut p = Projectile::new(
            ProjectileKind::Arrow {
                base_damage: 2.0,
                critical: false,
                effect: None,
            },
            Vec3::new(0.0, 100.0, 0.0),
            None,
        );
        p.velocity = Vec3::new(3.0, 0.0, 0.0);
        let w = Empty(vec![]);
        p.tick(&w);
        assert!((p.position.x - 3.0).abs() < 1e-9);
        assert!((p.velocity.x - 2.97).abs() < 1e-9);
        assert!((p.velocity.y + 0.05).abs() < 1e-9);
    }
    #[test]
    fn arrow_hits_entity_box_inflated_by_margin() {
        let target = Aabb::new(Vec3::new(2.0, 99.0, -0.3), Vec3::new(2.6, 100.8, 0.3));
        let mut p = Projectile::new(
            ProjectileKind::Arrow {
                base_damage: 2.0,
                critical: false,
                effect: None,
            },
            Vec3::new(0.0, 100.0, 0.0),
            None,
        );
        p.velocity = Vec3::new(3.0, 0.0, 0.0);
        let w = Empty(vec![(Who::Mob(7), target)]);
        let (keep, hit) = p.tick(&w);
        assert!(!keep);
        assert!(matches!(
            hit,
            Hit::Entity {
                who: Who::Mob(7),
                ..
            }
        ));
        let mut r = LegacyRandom::new(1);
        // ceil(3 * 2) = 6 for a non-critical arrow.
        assert_eq!(p.arrow_damage(&mut r), 6.0);
    }
    #[test]
    fn fireball_accelerates_along_its_direction() {
        let mut p = Projectile::fireball(
            ProjectileKind::SmallFireball,
            Vec3::new(0.0, 100.0, 0.0),
            None,
            [1.0, 0.0, 0.0],
        );
        let w = Empty(vec![]);
        p.tick(&w);
        // (0.1 + 0.1) * 0.95
        assert!((p.velocity.x - 0.19).abs() < 1e-9);
    }
}
