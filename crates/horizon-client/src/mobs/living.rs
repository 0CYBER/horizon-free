//! `LivingEntity` movement for mobs: `aiStep` input decay and jumping,
//! `travel` in air, water and lava (friction-scaled acceleration, gravity,
//! fluid slowdown and falling adjustment), fluid immersion heights, fall
//! damage, fire and the walk animation.
use super::{Mob, MobEvent, MobWorld};
use crate::{blocks::Fluid, collisions::SceneCollisions};
use horizon_core::geometry::{Aabb, Vec3};

#[derive(Clone, Copy, Debug, Default)]
pub struct Fluids {
    pub water: f64,
    pub lava: f64,
    pub in_water: bool,
    pub in_lava: bool,
    pub eye_in_water: bool,
}
pub fn wrap(a: f32) -> f32 {
    let mut a = a % 360.0;
    if a >= 180.0 {
        a -= 360.0;
    }
    if a < -180.0 {
        a += 360.0;
    }
    a
}
impl MobWorld<'_> {
    /// FluidState.getHeight: a full block under the same fluid, else amount / 9.
    pub fn fluid_height(&self, x: i32, y: i32, z: i32) -> (Fluid, f64) {
        let f = self.facts.get(self.world.block(x, y, z));
        if f.fluid == Fluid::None {
            return (Fluid::None, 0.0);
        }
        let above = self.facts.get(self.world.block(x, y + 1, z));
        if above.fluid == f.fluid {
            return (f.fluid, 1.0);
        }
        (f.fluid, f64::from(f.fluid_level) / 9.0)
    }
    /// Entity.updateFluidHeightAndDoFluidPushing heights over the deflated box.
    pub fn fluids(&self, bounds: Aabb, eye_y: f64) -> Fluids {
        let b = bounds.deflate(Vec3::new(0.001, 0.001, 0.001));
        let (lo, hi) = (b.min_position(), b.max_position());
        let mut out = Fluids::default();
        for x in lo.x.floor() as i32..=(hi.x.ceil() as i32 - 1) {
            for y in lo.y.floor() as i32..=(hi.y.ceil() as i32 - 1) {
                for z in lo.z.floor() as i32..=(hi.z.ceil() as i32 - 1) {
                    let (fluid, h) = self.fluid_height(x, y, z);
                    if fluid == Fluid::None {
                        continue;
                    }
                    let top = f64::from(y) + h;
                    if top >= lo.y {
                        let depth = top - lo.y;
                        match fluid {
                            Fluid::Water => {
                                out.water = out.water.max(depth);
                                out.in_water = true;
                            }
                            Fluid::Lava => {
                                out.lava = out.lava.max(depth);
                                out.in_lava = true;
                            }
                            Fluid::None => {}
                        }
                    }
                }
            }
        }
        let (fx, fz) = ((lo.x + hi.x) / 2.0, (lo.z + hi.z) / 2.0);
        let ey = eye_y.floor() as i32;
        let (fluid, h) = self.fluid_height(fx.floor() as i32, ey, fz.floor() as i32);
        out.eye_in_water = fluid == Fluid::Water && eye_y < f64::from(ey) + h + 0.111_111_11;
        out
    }
    pub fn no_collision(&self, b: Aabb) -> bool {
        let mut scene = SceneCollisions {
            world: self.world,
            binding: self.collision,
        };
        scene.unobstructed(b).unwrap_or(false)
    }
}
impl Mob {
    pub fn eye_y(&self) -> f64 {
        self.position.y + self.eye_height()
    }
    /// Entity dimensions: the type's, a class's BABY_DIMENSIONS, or the
    /// slime size scaling.
    pub fn dims(&self) -> (f64, f64, f64) {
        let k = &self.kind;
        let (mut w, mut h, mut e) = (k.width, k.height, k.eye_height);
        if self.is_baby() {
            match super::species::def(k.species).baby {
                super::species::Baby::Abs(bw, bh, be) => (w, h, e) = (bw, bh, be),
                super::species::Baby::Scale(s, eye) => {
                    (w, h, e) = (w * s, h * s, eye.unwrap_or(e * s))
                }
                super::species::Baby::None => {}
            }
        }
        if self.kind.species == super::Species::Pufferfish {
            let s = super::aquatic::puff_scale(self.extra.size);
            (w, h, e) = (w * s, h * s, e * s);
        } else if self.extra.size > 0 {
            let s = f64::from(self.extra.size);
            (w, h, e) = (w * s, h * s, e * s);
        }
        (w, h, e)
    }
    pub fn eye_height(&self) -> f64 {
        self.dims().2
    }
    pub fn width(&self) -> f32 {
        self.dims().0 as f32
    }
    pub fn height(&self) -> f32 {
        self.dims().1 as f32
    }
    pub fn is_baby(&self) -> bool {
        self.extra.age < 0
    }
    pub fn fluid_jump_threshold(&self) -> f64 {
        if self.eye_height() < 0.4 { 0.0 } else { 0.4 }
    }
    pub fn is_in_liquid(&self) -> bool {
        self.fluids.in_water || self.fluids.in_lava
    }
    fn affected_by_fluids(&self) -> bool {
        true
    }
    /// Entity.moveRelative: input rotated by yaw and scaled to `speed`.
    pub fn move_relative(&mut self, speed: f32, input: Vec3) {
        let len_sq = input.x * input.x + input.y * input.y + input.z * input.z;
        if len_sq < 1.0e-7 {
            return;
        }
        let v = if len_sq > 1.0 {
            let l = len_sq.sqrt();
            Vec3::new(input.x / l, input.y / l, input.z / l)
        } else {
            input
        };
        let v = Vec3::new(
            v.x * f64::from(speed),
            v.y * f64::from(speed),
            v.z * f64::from(speed),
        );
        let (s, c) = (
            f64::from(self.yaw.to_radians().sin() as f32),
            f64::from(self.yaw.to_radians().cos() as f32),
        );
        self.velocity = Vec3::new(
            self.velocity.x + (v.x * c - v.z * s),
            self.velocity.y + v.y,
            self.velocity.z + (v.z * c + v.x * s),
        );
    }
    /// Entity.move with block collisions and step-up; updates collision flags
    /// and fall distance (Entity.checkFallDamage).
    pub fn do_move(&mut self, w: &MobWorld, events: &mut Vec<MobEvent>) {
        let wanted = self.velocity;
        let step = self.kind.step_height as f32;
        let mut scene = SceneCollisions {
            world: w.world,
            binding: w.collision,
        };
        let moved = scene
            .resolve(self.bounds(), wanted, step, self.on_ground)
            .unwrap_or(Vec3::ZERO);
        self.position = Vec3::new(
            self.position.x + moved.x,
            self.position.y + moved.y,
            self.position.z + moved.z,
        );
        let x_hit = (moved.x - wanted.x).abs() > 1.0e-5;
        let z_hit = (moved.z - wanted.z).abs() > 1.0e-5;
        self.horizontal_collision = x_hit || z_hit;
        let vertical = (moved.y - wanted.y).abs() > 1.0e-5;
        self.vertical_collision_below = vertical && wanted.y < 0.0;
        self.on_ground = self.vertical_collision_below;
        if x_hit {
            self.velocity.x = 0.0;
        }
        if z_hit {
            self.velocity.z = 0.0;
        }
        if vertical {
            self.velocity.y = 0.0;
        }
        // Entity.checkFallDamage
        if self.on_ground {
            if self.fall_distance > 0.0 {
                self.cause_fall_damage(events);
            }
            self.fall_distance = 0.0;
        } else if moved.y < 0.0 {
            self.fall_distance -= moved.y;
        }
        if self.fluids.in_water {
            self.fall_distance = 0.0;
        }
    }
    fn cause_fall_damage(&mut self, events: &mut Vec<MobEvent>) {
        if self.kind.flags.fall_immune {
            return;
        }
        let power = self.fall_distance + 1.0e-6 - self.kind.safe_fall_distance;
        let damage = (power * self.kind.fall_damage_multiplier).floor();
        if damage > 0.0 {
            let at = [self.position.x, self.position.y, self.position.z];
            let sound = if damage > 4.0 {
                "entity.generic.big_fall"
            } else {
                "entity.generic.small_fall"
            };
            events.push(MobEvent::Sound(sound, at, 1.0, 1.0));
            self.damage(damage as f32, super::Damage::fall());
        }
    }
    /// LivingEntity.getFrictionInfluencedSpeed.
    fn friction_speed(&self, friction: f32) -> f32 {
        if self.on_ground {
            if friction > 0.6 {
                self.ai.speed * (0.216_000_02 / (friction * friction * friction))
            } else {
                self.ai.speed
            }
        } else {
            self.flying_speed()
        }
    }
    fn flying_speed(&self) -> f32 {
        0.02
    }
    fn gravity(&self) -> f64 {
        self.kind.gravity
    }
    /// LivingEntity.travel.
    pub fn travel(&mut self, w: &MobWorld, input: Vec3, events: &mut Vec<MobEvent>) {
        if super::ai::custom_travel(self, w, input, events) {
            return;
        }
        if self.kind.flags.flying {
            return self.travel_flying(w, input, events);
        }
        let stands_on_fluid = self.kind.flags.lava_walker && self.fluids.in_lava;
        if self.is_in_liquid() && self.affected_by_fluids() && !stands_on_fluid {
            self.travel_in_fluid(w, input, events);
        } else {
            self.travel_in_air(w, input, events);
        }
    }
    /// LivingEntity.travelFlying (ghast 0.02, phantom 0.2): no gravity, 0.91 drag.
    fn travel_flying(&mut self, w: &MobWorld, input: Vec3, events: &mut Vec<MobEvent>) {
        let (speed, drag) = if self.fluids.in_water {
            (0.02, 0.8)
        } else if self.fluids.in_lava {
            (0.02, 0.5)
        } else {
            (self.kind.air_speed, 0.91)
        };
        self.move_relative(speed, input);
        self.do_move(w, events);
        self.velocity = Vec3::new(
            self.velocity.x * drag,
            self.velocity.y * drag,
            self.velocity.z * drag,
        );
    }
    fn block_friction(&self, w: &MobWorld) -> f32 {
        // Entity.getBlockPosBelowThatAffectsMyMovement: 0.500001 below.
        let x = self.position.x.floor() as i32;
        let z = self.position.z.floor() as i32;
        let y = (self.position.y - 0.500_001).floor() as i32;
        w.facts.get(w.world.block(x, y, z)).friction
    }
    fn travel_in_air(&mut self, w: &MobWorld, input: Vec3, events: &mut Vec<MobEvent>) {
        let friction = if self.on_ground {
            self.block_friction(w)
        } else {
            1.0
        };
        let speed = self.friction_speed(friction);
        self.move_relative(speed, input);
        if self.kind.flags.climber && self.horizontal_collision {
            // Spider.onClimbable: handleOnClimbable clamps and resets the fall.
            self.fall_distance = 0.0;
            self.velocity = Vec3::new(
                self.velocity.x.clamp(-0.15, 0.15),
                self.velocity.y.max(-0.15),
                self.velocity.z.clamp(-0.15, 0.15),
            );
        }
        self.do_move(w, events);
        let mut movement = self.velocity;
        if self.kind.flags.climber
            && (self.horizontal_collision || self.ai.jumping)
            && self.climbing
        {
            movement.y = 0.2;
        }
        let mut y = movement.y;
        if self.extra.levitation > 0 {
            y += (0.05 * 1.0 - movement.y) * 0.2;
        } else {
            y -= self.gravity();
        }
        let drag = 0.91f32;
        let f = friction * drag;
        let vertical = if self.kind.flags.omnidirectional {
            drag
        } else {
            0.98
        };
        self.velocity = Vec3::new(
            movement.x * f64::from(f),
            y * f64::from(vertical),
            movement.z * f64::from(f),
        );
    }
    fn travel_in_fluid(&mut self, w: &MobWorld, input: Vec3, events: &mut Vec<MobEvent>) {
        let falling = self.velocity.y <= 0.0;
        let old_y = self.position.y;
        let gravity = self.gravity();
        if self.fluids.in_water {
            let mut slow = self.kind.water_slowdown;
            let mut speed = 0.02f32;
            let mut efficiency = self.kind.water_efficiency as f32;
            if !self.on_ground {
                efficiency *= 0.5;
            }
            if efficiency > 0.0 {
                slow += (0.546_000_06 - slow) * efficiency;
                speed += (self.ai.speed - speed) * efficiency;
            }
            self.move_relative(speed, input);
            self.do_move(w, events);
            let mut m = self.velocity;
            if self.horizontal_collision && self.kind.flags.climber {
                m.y = 0.2;
            }
            m = Vec3::new(m.x * f64::from(slow), m.y * 0.8, m.z * f64::from(slow));
            self.velocity = fluid_falling(gravity, falling, m);
        } else {
            self.move_relative(0.02, input);
            self.do_move(w, events);
            if self.fluids.lava <= self.fluid_jump_threshold() {
                self.velocity = Vec3::new(
                    self.velocity.x * 0.5,
                    self.velocity.y * 0.8,
                    self.velocity.z * 0.5,
                );
                self.velocity = fluid_falling(gravity, falling, self.velocity);
            } else {
                self.velocity = Vec3::new(
                    self.velocity.x * 0.5,
                    self.velocity.y * 0.5,
                    self.velocity.z * 0.5,
                );
            }
            if gravity != 0.0 {
                self.velocity.y -= gravity / 4.0;
            }
        }
        // jumpOutOfFluid
        let m = self.velocity;
        if self.horizontal_collision
            && w.no_collision(self.bounds().move_by(Vec3::new(
                m.x,
                m.y + 0.6 - self.position.y + old_y,
                m.z,
            )))
        {
            self.velocity.y = 0.3;
        }
    }
    /// The jump part of LivingEntity.aiStep.
    pub fn handle_jumping(&mut self) {
        if self.ai.jumping && self.affected_by_fluids() {
            let (fluid_height, in_water_with_height) = if self.fluids.in_lava {
                (self.fluids.lava, false)
            } else {
                (
                    self.fluids.water,
                    self.fluids.in_water && self.fluids.water > 0.0,
                )
            };
            let threshold = self.fluid_jump_threshold();
            if !in_water_with_height || self.on_ground && fluid_height <= threshold {
                if !self.fluids.in_lava || self.on_ground && self.fluids.lava <= threshold {
                    if (self.on_ground || in_water_with_height && fluid_height <= threshold)
                        && self.ai.no_jump_delay == 0
                    {
                        self.jump_from_ground();
                        self.ai.no_jump_delay = 10;
                    }
                } else {
                    self.jump_in_liquid();
                }
            } else {
                self.jump_in_liquid();
            }
        } else {
            self.ai.no_jump_delay = 0;
        }
    }
    /// Mob.jumpInLiquid: floaters rise 0.04 per tick, others 0.3.
    fn jump_in_liquid(&mut self) {
        self.velocity.y += if self.ai.nav.can_float { 0.04 } else { 0.3 };
    }
    pub fn jump_from_ground(&mut self) {
        let power = self.jump_power();
        if power > 1.0e-5 {
            self.velocity.y = f64::from(power).max(self.velocity.y);
        }
    }
    pub fn jump_power(&self) -> f32 {
        self.kind.jump_strength as f32
    }
    /// LivingEntity.updateWalkAnimation (client calculateEntityAnimation).
    pub fn update_walk_animation(&mut self) {
        let dy = if self.kind.flags.omnidirectional {
            self.position.y - self.previous.y
        } else {
            0.0
        };
        let dx = self.position.x - self.previous.x;
        let dz = self.position.z - self.previous.z;
        let distance = (dx * dx + dy * dy + dz * dz).sqrt() as f32;
        self.previous_limb_speed = self.limb_speed;
        if self.alive() {
            let target = (distance * 4.0).min(1.0);
            self.limb_speed += (target - self.limb_speed) * 0.4;
            self.limb_position += self.limb_speed * if self.is_baby() { 3.0 } else { 1.0 };
        } else {
            self.limb_speed = 0.0;
        }
    }
}
/// LivingEntity.getFluidFallingAdjustedMovement.
fn fluid_falling(gravity: f64, falling: bool, m: Vec3) -> Vec3 {
    if gravity != 0.0 {
        let y = if falling && (m.y - 0.005).abs() >= 0.003 && (m.y - gravity / 16.0).abs() < 0.003 {
            -0.003
        } else {
            m.y - gravity / 16.0
        };
        Vec3::new(m.x, y, m.z)
    } else {
        m
    }
}
