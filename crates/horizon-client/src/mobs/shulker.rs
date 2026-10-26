//! Shulkers (`Shulker`): fixed to a block face, peeking out of their shell,
//! firing homing bullets at players within 20 blocks, teleporting when
//! their face is lost or when badly hurt, and shrugging off arrows while
//! closed.
use super::{
    Damage, DamageKind, Mob, MobEvent, MobWorld, Species, Who,
    anim::{Poses, State},
    goals::{
        Cx, Goal, HurtByTarget, LOOK, LookAtPlayer, MOVE, NearestTarget, RandomLookAround,
        goal_clone, reduced,
    },
};
use crate::projectiles::{Projectile, ProjectileWorld, STEPS};
use horizon_core::geometry::trig::Mth;
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::{LegacyRandom, RandomSource},
};
use std::f32::consts::PI;

// Shulker state: `variant3` the attach face (3D data value), `counter` the
// raw peek (0..100), `color` the shell colour (16 none). AnimState slot 0
// the current peek amount.
pub const PEEK: usize = 0;
pub const NO_COLOR: u8 = 16;

pub fn register(m: &mut Mob) {
    m.no_gravity = true;
    m.yaw = 0.0;
    m.body_yaw = 0.0;
    m.previous_body_yaw = 0.0;
    if m.extra.color == 0 && m.extra.variant == 0 {
        m.extra.color = NO_COLOR;
    }
    let g = &mut m.ai.goals;
    g.add(1, LookAtPlayer::new(8.0).probability(0.02));
    g.add(4, ShulkerAttack { attack_time: 0 });
    g.add(7, Peek { time: 0 });
    g.add(8, RandomLookAround::default());
    m.ai.targets.add(1, HurtByTarget::new().alert());
    // ShulkerNearestAttackGoal: the search box is only 4 deep along the
    // attach axis.
    m.ai.targets.add(
        2,
        NearestTarget::player(true).when(|m, w| {
            let Some(p) = &w.player else { return false };
            let d = p.position.subtract(m.position);
            let axis = match m.extra.variant3 {
                0 | 1 => d.y,
                2 | 3 => d.z,
                _ => d.x,
            };
            axis.abs() <= 4.0 + 0.5
        }),
    );
}
fn face_step(face: u8) -> [i32; 3] {
    STEPS[face.min(5) as usize]
}
fn block_pos(m: &Mob) -> [i32; 3] {
    [
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    ]
}
/// `Shulker.canStayAt`: an empty spot against a sturdy face.
fn can_stay_at(w: &MobWorld, p: [i32; 3], face: u8) -> bool {
    if !w.facts.is_air(w.world.block(p[0], p[1], p[2])) {
        return false;
    }
    let s = face_step(face);
    let n = w.world.block(p[0] + s[0], p[1] + s[1], p[2] + s[2]);
    w.facts.get(n).full_collision
}
/// `findAttachableSurface`: the first face (down, up, north, south, west,
/// east) to cling to.
fn attachable(w: &MobWorld, p: [i32; 3]) -> Option<u8> {
    (0..6).find(|&f| can_stay_at(w, p, f))
}
/// `Shulker.teleportSomewhere`: up to five tries within 8 blocks.
fn teleport(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) -> bool {
    if !m.alive() {
        return false;
    }
    let old = block_pos(m);
    for _ in 0..5 {
        let r = |m: &mut Mob| m.random.next_int_bound(17).unwrap_or(8) - 8;
        let t = [old[0] + r(m), old[1] + r(m), old[2] + r(m)];
        if t[1] <= crate::chunk::MIN_Y || !w.facts.is_air(w.world.block(t[0], t[1], t[2])) {
            continue;
        }
        let Some(face) = attachable(w, t) else {
            continue;
        };
        m.extra.variant3 = face;
        events.push(MobEvent::Sound(
            "entity.shulker.teleport",
            [m.position.x, m.position.y, m.position.z],
            1.0,
            1.0,
        ));
        m.position = Vec3::new(
            f64::from(t[0]) + 0.5,
            f64::from(t[1]),
            f64::from(t[2]) + 0.5,
        );
        m.previous = m.position;
        m.extra.counter = 0;
        m.ai.target = None;
        return true;
    }
    false
}
/// `Shulker.setRawPeekAmount`: the shell opens or closes with a sound.
fn set_peek(m: &mut Mob, amount: i32, events: &mut Vec<MobEvent>) {
    let sound = if amount == 0 {
        "entity.shulker.close"
    } else {
        "entity.shulker.open"
    };
    events.push(MobEvent::Sound(
        sound,
        [m.position.x, m.position.y, m.position.z],
        1.0,
        1.0,
    ));
    m.extra.counter = amount;
}
/// `Shulker.tick` additions: keep a face (or teleport), ease the peek,
/// stay still and square on its block.
pub fn tick(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    m.velocity = Vec3::ZERO;
    m.body_yaw = 0.0;
    m.yaw = 0.0;
    m.head_pitch = 0.0;
    let p = block_pos(m);
    if !can_stay_at(w, p, m.extra.variant3) {
        match attachable(w, p) {
            Some(f) => m.extra.variant3 = f,
            None => {
                teleport(m, w, events);
            }
        }
    }
    let a = &mut m.anim;
    let target = m.extra.counter as f32 * 0.01;
    let cur = a.v[PEEK];
    if cur > target {
        a.v[PEEK] = (cur - 0.05).clamp(target, 1.0);
    } else if cur < target {
        a.v[PEEK] = (cur + 0.05).clamp(0.0, target);
    }
}
/// `Shulker.hurtServer` before the damage: arrows bounce off a closed shell.
pub fn accepts(m: &Mob, source: &Damage) -> bool {
    !(m.extra.counter == 0 && source.kind == DamageKind::Arrow)
}
/// `Shulker.hurtServer` after the damage: below half health, a one in four
/// chance to teleport away.
pub fn after_hurt(m: &mut Mob) {
    if m.health < m.attrs.max_health * 0.5 && m.random.next_int_bound(4).unwrap_or(1) == 0 {
        // The teleport needs the world: ask for it on the next tick.
        m.extra.flag = true;
    }
}
/// `Shulker.hurtServer` after a shulker bullet's damage: unless the
/// low-health teleport was drawn instead (`else if`), `hitByShulkerBullet`
/// runs on the next tick. Whether the shell is open is read here, at hurt
/// time, so a goal opening it before that tick cannot change the outcome.
pub fn hit_by_bullet(m: &mut Mob) {
    if m.species() == Species::Shulker && !m.extra.flag && m.extra.counter != 0 {
        m.extra.flag2 = true;
    }
}
/// The deferred teleport of `after_hurt` and `hitByShulkerBullet`.
pub fn ai_step(
    m: &mut Mob,
    w: &MobWorld,
    level_random: &mut LegacyRandom,
    events: &mut Vec<MobEvent>,
) {
    if m.extra.flag {
        m.extra.flag = false;
        teleport(m, w, events);
    }
    if m.extra.flag2 {
        m.extra.flag2 = false;
        hit_by_shulker_bullet(m, w, level_random, events);
    }
}
/// `Shulker.hitByShulkerBullet` (only queued by an open shulker): it
/// teleports, and leaves a new one of its colour behind unless shulkers
/// crowd the place (one in five fewer chances for every other shulker
/// within 8 blocks).
fn hit_by_shulker_bullet(
    m: &mut Mob,
    w: &MobWorld,
    level_random: &mut LegacyRandom,
    events: &mut Vec<MobEvent>,
) {
    let old_position = m.position;
    let old_box = m.bounds();
    if !teleport(m, w, events) {
        return;
    }
    let area = old_box.inflate(Vec3::new(8.0, 8.0, 8.0));
    // The shulker itself stands within reach of its old box, so it counts.
    let others = w
        .others_near(area)
        .filter(|s| {
            s.alive && s.species == Species::Shulker && s.id != m.id && s.bounds.intersects(area)
        })
        .count();
    let failure_chance = others as f32 / 5.0;
    // The level's random, not the shulker's.
    if level_random.next_float() < failure_chance {
        return;
    }
    let mut baby = Mob::new(0, m.kind.clone(), old_position, 0.0);
    baby.extra.color = m.extra.color;
    events.push(MobEvent::Spawn(Box::new(baby)));
}
/// A ProjectileWorld over the mob world, for aiming a new bullet.
struct Probe<'a, 'w> {
    w: &'a MobWorld<'w>,
    targets: Vec<(Who, Aabb)>,
}
impl ProjectileWorld for Probe<'_, '_> {
    fn clip(&self, _: Vec3, _: Vec3) -> Option<(Vec3, [i32; 3], [i32; 3])> {
        None
    }
    fn in_water(&self, _: Vec3) -> bool {
        false
    }
    fn targets(&self) -> &[(Who, Aabb)] {
        &self.targets
    }
    fn empty(&self, p: [i32; 3]) -> bool {
        self.w.facts.is_air(self.w.world.block(p[0], p[1], p[2]))
    }
    fn solid(&self, p: [i32; 3]) -> bool {
        self.w
            .facts
            .get(self.w.world.block(p[0], p[1], p[2]))
            .full_collision
    }
}
/// `ShulkerAttackGoal`: open up and fire a bullet every one to five and a
/// half seconds at a target within 20 blocks.
#[derive(Clone, Debug)]
struct ShulkerAttack {
    attack_time: i32,
}
impl Goal for ShulkerAttack {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.target_info(cx.w).is_some_and(|t| t.alive)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.attack_time = 20;
        set_peek(m, 100, cx.events);
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        set_peek(m, 0, cx.events);
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.attack_time -= 1;
        let (Some(who), Some(t)) = (m.ai.target, m.target_info(cx.w)) else {
            return;
        };
        m.look_at_speed([t.position.x, t.eye_y, t.position.z], 180.0, 180.0);
        if t.position.distance_squared(m.position) >= 400.0 {
            m.ai.target = None;
            return;
        }
        if self.attack_time > 0 {
            return;
        }
        self.attack_time = 20 + m.random.next_int_bound(10).unwrap_or(0) * 20 / 2;
        let probe = Probe {
            w: cx.w,
            targets: vec![(who, t.bounds)],
        };
        let axis = match m.extra.variant3 {
            0 | 1 => 1,
            2 | 3 => 2,
            _ => 0,
        };
        let seed = m.random.next_long() as u64 | 1;
        let bullet =
            Projectile::shulker_bullet(&probe, block_pos(m), Who::Mob(m.id), who, axis, seed);
        cx.events.push(MobEvent::Projectile(bullet));
        let pitch = (m.random.next_float() - m.random.next_float()) * 0.2 + 1.0;
        cx.events.push(MobEvent::Sound(
            "entity.shulker.shoot",
            [m.position.x, m.position.y, m.position.z],
            2.0,
            pitch,
        ));
    }
}
/// `ShulkerPeekGoal`: idle peeks of one to three seconds.
#[derive(Clone, Debug)]
struct Peek {
    time: i32,
}
impl Goal for Peek {
    goal_clone!();
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.ai.target.is_none()
            && m.random.next_int_bound(reduced(40)).unwrap_or(1) == 0
            && can_stay_at(cx.w, block_pos(m), m.extra.variant3)
    }
    fn can_continue(&mut self, m: &mut Mob, _cx: &mut Cx) -> bool {
        m.ai.target.is_none() && self.time > 0
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.time = reduced(20 * (1 + m.random.next_int_bound(3).unwrap_or(0)));
        set_peek(m, 30, cx.events);
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        if m.ai.target.is_none() {
            set_peek(m, 0, cx.events);
        }
    }
    fn tick(&mut self, _m: &mut Mob, _cx: &mut Cx) {
        self.time -= 1;
    }
}
/// The shell's texture: plain or dyed.
pub fn texture(m: &Mob) -> String {
    const COLORS: [&str; 16] = [
        "white",
        "orange",
        "magenta",
        "light_blue",
        "yellow",
        "lime",
        "pink",
        "gray",
        "light_gray",
        "cyan",
        "purple",
        "blue",
        "brown",
        "green",
        "red",
        "black",
    ];
    match COLORS.get(m.extra.color as usize) {
        Some(c) => format!("entity/shulker/shulker_{c}"),
        None => "entity/shulker/shulker".into(),
    }
}
/// `ShulkerRenderer.setupRotations`: turned onto its attach face around the
/// block's centre (after a half turn of the body).
pub fn rotations(m: &Mob) -> glam::Mat4 {
    {
        let q = match m.extra.variant3 {
            // Attached below: upright.
            0 => glam::Quat::IDENTITY,
            1 => glam::Quat::from_rotation_x(PI),
            2 => glam::Quat::from_rotation_x(PI / 2.0),
            3 => glam::Quat::from_euler(glam::EulerRot::XYZ, PI / 2.0, 0.0, PI),
            4 => glam::Quat::from_euler(glam::EulerRot::XYZ, PI / 2.0, 0.0, -PI / 2.0),
            _ => glam::Quat::from_euler(glam::EulerRot::XYZ, PI / 2.0, 0.0, PI / 2.0),
        };
        glam::Mat4::from_rotation_y(PI)
            * glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.5, 0.0))
            * glam::Mat4::from_quat(q)
            * glam::Mat4::from_translation(glam::Vec3::new(0.0, -0.5, 0.0))
    }
}
/// `ShulkerModel.setupAnim`: the lid rises and twists with the peek, the
/// head turns.
pub fn anim(m: &Mob, s: &State, p: &mut Poses) {
    let peek = m.anim.lerp(PEEK, s.partial);
    let bs = (0.5 + peek) * PI;
    let q = -1.0 + bs.msin();
    let extra = if bs > PI {
        (s.age * 0.1).msin() * 0.7
    } else {
        0.0
    };
    let lid = p.get("lid");
    lid[1] = 16.0 + bs.msin() * 8.0 + extra;
    lid[4] = if peek > 0.3 {
        q * q * q * q * PI * 0.125
    } else {
        0.0
    };
    p.xrot("head", s.x_rot.to_radians());
    p.yrot("head", (s.y_rot - 180.0).to_radians());
}
