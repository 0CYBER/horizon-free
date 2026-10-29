//! Slimes and magma cubes (`AbstractCubeMob`): sized bodies, hopping
//! movement (`CubeMobMoveControl`), random directions, contact damage,
//! squish animation and splitting.
use super::{
    Mob, MobEvent, MobWorld, Species,
    anim::{Poses, State},
    control::{MoveKind, Operation, rotlerp},
    goals::{Cx, Goal, JUMP, LOOK, MOVE, NearestTarget, Prey, goal_clone, reduced},
};
use horizon_core::{
    geometry::{Vec3, trig},
    random::RandomSource,
};

// AnimState slots.
const SQUISH: usize = 0;
const TARGET_SQUISH: usize = 1;

pub fn register(m: &mut Mob) {
    if m.species() == Species::SulfurCube {
        return super::sulfur_cube::register(m);
    }
    m.ai.goals.add(1, CubeFloat);
    m.ai.goals.add(2, CubeAttack { tired: 0 });
    m.ai.goals.add(
        4,
        CubeRandomDirection {
            degrees: 0.0,
            next: 0,
        },
    );
    m.ai.goals.add(5, CubeKeepJumping);
    m.ai.targets.add(
        1,
        NearestTarget::new(Prey::Player, true)
            .interval(10)
            .when(|m, w| {
                w.player
                    .as_ref()
                    .is_some_and(|p| (p.position.y - m.position.y).abs() <= 4.0)
            }),
    );
    m.ai.targets
        .add(3, NearestTarget::species(&[Species::IronGolem], true));
    m.ai.mov.kind = MoveKind::Custom;
    m.ai.nav.can_float = true;
    m.extra.vector[0] = f64::from(180.0 * m.yaw / std::f32::consts::PI);
    set_size(m, 1);
}
/// AbstractCubeMob.setSize(size, true): the max health (size^2), movement
/// speed (0.2 + 0.1 * size, in floats) and attack damage (size) bases, a
/// magma cube's armor (size * 3), then the health back to the max.
pub fn set_size(m: &mut Mob, size: i32) {
    use super::finalize::{Attr, set_base};
    let size = size.clamp(1, 127);
    m.extra.size = size;
    set_base(m, Attr::MaxHealth, f64::from(size * size));
    set_base(
        m,
        Attr::MovementSpeed,
        f64::from(0.2f32 + 0.1f32 * size as f32),
    );
    set_base(m, Attr::AttackDamage, f64::from(size));
    if m.species() == super::Species::MagmaCube {
        set_base(m, Attr::Armor, f64::from(size * 3));
    }
    m.health = super::finalize::max_health(m);
}
/// AbstractCubeMob.setSpawnSize: 1, 2 or 4 (normal difficulty special multiplier 0).
pub fn finalize(m: &mut Mob) {
    let scale = m.random.next_int_bound(3).unwrap_or(0);
    set_size(m, 1 << scale);
}
fn tiny(m: &Mob) -> bool {
    m.extra.size <= 1
}
fn can_deal_damage(m: &Mob) -> bool {
    // SulfurCube.canDealDamage: never.
    m.species() == Species::MagmaCube || !tiny(m) && m.species() != Species::SulfurCube
}
fn jump_delay(m: &mut Mob) -> i32 {
    let base = m.random.next_int_bound(20).unwrap_or(0) + 10;
    if m.species() == Species::MagmaCube {
        base * 4
    } else {
        base
    }
}
fn sound_pitch(m: &mut Mob) -> f32 {
    let adjust = if tiny(m) { 1.4 } else { 0.8 };
    ((m.random.next_float() - m.random.next_float()) * 0.2 + 1.0) * adjust
}
/// CubeMobMoveControl.tick
pub fn move_control(m: &mut Mob, events: &mut Vec<MobEvent>) {
    // SulfurCubeMobMoveControl: still while holding a block.
    if m.species() == Species::SulfurCube && super::sulfur_cube::has_body(m) {
        return;
    }
    let want = m.extra.vector[0] as f32;
    m.yaw = rotlerp(m.yaw, want, 90.0);
    m.head_yaw = m.yaw;
    m.body_yaw = m.yaw;
    if m.ai.mov.operation != Operation::MoveTo {
        m.ai.zza = 0.0;
        return;
    }
    m.ai.mov.operation = Operation::Wait;
    let speed = (m.ai.mov.speed_modifier * m.movement_speed()) as f32;
    if m.on_ground {
        m.set_speed(speed);
        let delay = m.extra.counter;
        m.extra.counter -= 1;
        if delay <= 0 {
            m.extra.counter = jump_delay(m);
            if m.extra.flag {
                m.extra.counter /= 3;
            }
            m.ai.jump = true;
            let jump = match (m.species(), tiny(m)) {
                (Species::MagmaCube, _) => "entity.magma_cube.jump",
                (Species::SulfurCube, _) => super::sulfur_cube::jump_sound(m),
                (_, true) => "entity.slime.jump_small",
                _ => "entity.slime.jump",
            };
            let pitch = sound_pitch(m);
            let vol = super::ai::sound_volume(m);
            events.push(MobEvent::Sound(jump, m.center(), vol, pitch));
        } else {
            m.ai.xxa = 0.0;
            m.ai.zza = 0.0;
            m.set_speed(0.0);
        }
    } else {
        m.set_speed(speed);
    }
}
pub(super) fn set_direction(m: &mut Mob, y_rot: f32, aggressive: bool) {
    m.extra.vector[0] = f64::from(y_rot);
    m.extra.flag = aggressive;
}
pub(super) fn set_wanted(m: &mut Mob, speed: f64) {
    m.ai.mov.speed_modifier = speed;
    m.ai.mov.operation = Operation::MoveTo;
}
/// AbstractCubeMob.jumpFromGround (and MagmaCube's size boost).
pub fn jump(m: &mut Mob) {
    let boost = if m.species() == Species::MagmaCube {
        m.extra.size as f32 * 0.1
    } else {
        0.0
    };
    m.velocity.y = f64::from(m.jump_power() + boost);
}
/// `AbstractCubeMob.tick`'s landing splash: crumbs of goo (or flames) around
/// the cube's feet, drawn by the side that ticks the mob.
fn landing_splash(m: &mut Mob, events: &mut Vec<MobEvent>) {
    let size = m.width() * 2.0;
    let radius = size / 2.0;
    let kind = match m.species() {
        Species::MagmaCube => "flame",
        Species::SulfurCube => "sulfur_cube_goo",
        _ => "item_slime",
    };
    let mut i = 0;
    while (i as f32) < size * 16.0 {
        let dir = m.random.next_float() * std::f64::consts::TAU as f32;
        let d = m.random.next_float() * 0.5 + 0.5;
        let xd = trig::sin(f64::from(dir)) * radius * d;
        let zd = trig::cos(f64::from(dir)) * radius * d;
        events.push(MobEvent::Particles {
            kind,
            at: [
                m.position.x + f64::from(xd),
                m.position.y,
                m.position.z + f64::from(zd),
            ],
            count: 0,
        });
        i += 1;
    }
}
/// `AbstractCubeMob.tick` on a client of a server: the landing splash follows
/// the on-ground flag the server sends.
pub fn client_tick(m: &mut Mob, events: &mut Vec<MobEvent>) {
    if m.on_ground && !m.extra.flag2 {
        landing_splash(m, events);
    }
    m.extra.flag2 = m.on_ground;
}
/// AbstractCubeMob.tick squish and landing sounds, plus contact damage.
pub fn ai_step(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    let v = &mut m.anim.v;
    v[SQUISH] += (v[TARGET_SQUISH] - v[SQUISH]) * 0.5;
    let was = m.extra.flag2;
    if m.on_ground && !was {
        landing_splash(m, events);
        let squish = match (m.species(), tiny(m)) {
            (Species::SulfurCube, _) => super::sulfur_cube::squish_sound(m),
            (Species::MagmaCube, true) => "entity.magma_cube.squish_small",
            (Species::MagmaCube, false) => "entity.magma_cube.squish",
            (_, true) => "entity.slime.squish_small",
            _ => "entity.slime.squish",
        };
        let pitch = ((m.random.next_float() - m.random.next_float()) * 0.2 + 1.0) / 0.8;
        let vol = super::ai::sound_volume(m);
        events.push(MobEvent::Sound(squish, m.center(), vol, pitch));
        m.anim.v[TARGET_SQUISH] = -0.5;
    } else if !m.on_ground && was {
        m.anim.v[TARGET_SQUISH] = 1.0;
    }
    m.extra.flag2 = m.on_ground;
    m.anim.v[TARGET_SQUISH] *= if m.species() == Species::MagmaCube {
        0.9
    } else {
        0.6
    };
    // playerTouch -> dealDamage
    if can_deal_damage(m)
        && m.alive()
        && let Some(p) = &w.player
        && p.alive
        && !p.creative
        // Player.touch: entities within the player's box grown by (1, 0.5, 1).
        && m.bounds().intersects(p.bounds.inflate(Vec3::new(1.0, 0.5, 1.0)))
    {
        let reach = 2.04f32.sqrt() as f64 - 0.6;
        if m.bounds()
            .inflate(Vec3::new(reach, 0.0, reach))
            .intersects(p.bounds)
        {
            let damage = m.attrs.attack_damage
                + if m.species() == Species::MagmaCube {
                    2.0
                } else {
                    0.0
                };
            events.push(MobEvent::AttackPlayer(
                damage,
                [m.position.x, m.position.y, m.position.z],
            ));
            if m.species() == Species::Slime {
                let pitch = (m.random.next_float() - m.random.next_float()) * 0.2 + 1.0;
                events.push(MobEvent::Sound(
                    "entity.slime.attack",
                    m.center(),
                    1.0,
                    pitch,
                ));
            }
        }
    }
}
/// MagmaCubeModel.setupAnim (slimes have no animation).
pub fn anim(m: &Mob, s: &State, p: &mut Poses) {
    if m.species() == Species::MagmaCube {
        let squish = m.anim.lerp(SQUISH, s.partial).max(0.0);
        for i in 0..8 {
            p.get(&format!("cube{i}"))[1] = -((4 - i) as f32) * squish * 1.7;
        }
    }
}
/// AbstractCubeMobRenderer.scale: size with squash and stretch.
pub fn scale(m: &Mob, alpha: f32) -> glam::Mat4 {
    let size = m.extra.size.max(1) as f32;
    let squish = m.anim.lerp(SQUISH, alpha) / (size * 0.5 + 1.0);
    let w = 1.0 / (squish + 1.0);
    let mut t = glam::Mat4::IDENTITY;
    if m.species() == Species::Slime {
        t = glam::Mat4::from_scale(glam::Vec3::splat(0.999))
            * glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.001, 0.0));
    }
    t * glam::Mat4::from_scale(glam::Vec3::new(w * size, 1.0 / w * size, w * size))
}
/// CubeMobFloatGoal
#[derive(Clone, Debug)]
pub(super) struct CubeFloat;
impl Goal for CubeFloat {
    goal_clone!();
    fn flags(&self) -> u8 {
        JUMP | MOVE
    }
    fn can_use(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        m.is_in_liquid()
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, _: &mut Cx) {
        if m.random.next_float() < 0.8 {
            m.ai.jump = true;
        }
        set_wanted(m, 1.2);
    }
}
/// CubeMobAttackGoal: face the target and hop at it for up to 300 ticks.
#[derive(Clone, Debug)]
struct CubeAttack {
    tired: i32,
}
impl Goal for CubeAttack {
    goal_clone!();
    fn flags(&self) -> u8 {
        LOOK
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.target_info(cx.w)
            .is_some_and(|t| t.alive && !(t.is_player && t.creative))
    }
    fn start(&mut self, _: &mut Mob, _: &mut Cx) {
        self.tired = reduced(300);
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if !m
            .target_info(cx.w)
            .is_some_and(|t| t.alive && !(t.is_player && t.creative))
        {
            return false;
        }
        self.tired -= 1;
        self.tired > 0
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some(t) = m.target_info(cx.w) {
            m.turn_to([t.position.x, t.eye_y, t.position.z], 10.0, 10.0);
        }
        let (yaw, aggressive) = (m.yaw, can_deal_damage(m));
        set_direction(m, yaw, aggressive);
    }
}
/// CubeMobRandomDirectionGoal
#[derive(Clone, Debug, Default)]
pub(super) struct CubeRandomDirection {
    degrees: f32,
    next: i32,
}
impl Goal for CubeRandomDirection {
    goal_clone!();
    fn flags(&self) -> u8 {
        LOOK
    }
    fn can_use(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        m.ai.target.is_none()
            && (m.on_ground || m.is_in_liquid() || m.has_effect("minecraft:levitation"))
    }
    fn tick(&mut self, m: &mut Mob, _: &mut Cx) {
        self.next -= 1;
        if self.next <= 0 {
            self.next = reduced(40 + m.random.next_int_bound(60).unwrap_or(0));
            self.degrees = m.random.next_int_bound(360).unwrap_or(0) as f32;
        }
        set_direction(m, self.degrees, false);
    }
}
/// CubeMobKeepOnJumpingGoal
#[derive(Clone, Debug)]
pub(super) struct CubeKeepJumping;
impl Goal for CubeKeepJumping {
    goal_clone!();
    fn flags(&self) -> u8 {
        JUMP | MOVE
    }
    fn can_use(&mut self, _: &mut Mob, _: &mut Cx) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, _: &mut Cx) {
        set_wanted(m, 1.0);
    }
}
