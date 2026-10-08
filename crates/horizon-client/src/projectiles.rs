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
            ProjectileKind::SmallFireball | ProjectileKind::LargeFireball { .. }
        )
    }
    fn gravity(&self) -> f64 {
        match self.kind {
            ProjectileKind::Arrow { .. } => 0.05,
            ProjectileKind::SmallFireball | ProjectileKind::LargeFireball { .. } => 0.0,
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
    /// One tick: returns whether to keep the projectile and what it hit.
    pub fn tick(&mut self, w: &impl ProjectileWorld) -> (bool, Hit) {
        self.previous = self.position;
        self.age += 1;
        if self.stuck {
            // AbstractArrow despawns 1200 ticks after sticking.
            return (self.age < 1200, Hit::None);
        }
        let water = w.in_water(self.position);
        match self.kind {
            ProjectileKind::SmallFireball | ProjectileKind::LargeFireball { .. } => {
                let inertia = if water { 0.8 } else { 0.95 };
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
