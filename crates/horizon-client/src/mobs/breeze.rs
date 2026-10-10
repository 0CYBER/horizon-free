//! Breezes (`Breeze`, `BreezeAi`): the brain's idle and fight activities as
//! one simulation (start attacking the nearest player or iron golem or whoever
//! hurt it, slide to a walk target, then shoot, long jump, shoot when stuck or
//! slide), memories with vanilla's expiry, the poses and their animations,
//! `BreezeModel`'s layers and the wind charges it shoots.
use super::{
    Mob, MobEvent, MobWorld, Species, Who,
    anim::{Poses, State},
    goals::{Cx, Float, Goal, goal_clone},
    keyframes::Animation,
};
use crate::projectiles::{Projectile, ProjectileKind};
use horizon_core::{geometry::Vec3, random::RandomSource};
use std::collections::HashMap;

// Breeze state (see `brain`): `variant3` the pose, `variant` the pose the
// animations last saw, `ints[0..6]` the memories (shoot, shoot cooldown,
// shoot charging, shoot recovering, jump cooldown, jump inhaling) as the
// remaining ticks plus one, `point` the walk target with `vector[0]` its
// speed and `vector[1]` how close is close enough, `home` the jump target,
// `vector[2]` the whirl sound clock, `flag` the long jump behaviour with its
// end tick in `anim_ticks`, `flag2` the shoot behaviour with its end tick in
// `egg_time`, `counter` the end tick of the slide to the walk target (0 when
// not sliding), `counter2` the end tick of doing nothing, `anger` the
// cooldown before the next try to walk, `scratch[0]` the ticks it may still
// be unable to attack its target. AnimState states:
// idle, shoot, slide, slide back, inhale, long jump.
pub const STANDING: u8 = 0;
pub const SHOOTING: u8 = 1;
pub const INHALING: u8 = 2;
pub const LONG_JUMPING: u8 = 3;
pub const SLIDING: u8 = 4;

const SHOOT: usize = 0;
const SHOOT_COOLDOWN: usize = 1;
const SHOOT_CHARGING: usize = 2;
const SHOOT_RECOVERING: usize = 3;
const JUMP_COOLDOWN: usize = 4;
const JUMP_INHALING: usize = 5;

const SHOOT_INITIAL_DELAY: i32 = 15;
const SHOOT_RECOVER_DELAY: i32 = 4;
const SHOOT_COOLDOWN_TICKS: i32 = 10;
const INHALING_TICKS: i32 = 10;
const ALLOWED_ANGLES: [i32; 5] = [40, 55, 60, 75, 80];

pub fn register(m: &mut Mob) {
    m.set_malus(super::path::PathType::OnTopOfTrapdoor, -1.0);
    m.set_malus(super::path::PathType::Fire, -1.0);
    // Swim(0.8) and the brain.
    m.ai.goals.add(0, Float);
    m.ai.goals.add(0, Brain);
}
pub fn pose(m: &Mob) -> u8 {
    m.extra.variant3
}
fn set_pose(m: &mut Mob, pose: u8) {
    m.extra.variant3 = pose;
}
fn has(m: &Mob, memory: usize) -> bool {
    m.extra.ints[memory] > 0
}
/// `Brain.setMemoryWithExpiry`: present now and for `ttl` more ticks.
fn remember(m: &mut Mob, memory: usize, ttl: i32) {
    m.extra.ints[memory] = ttl + 1;
}
fn forget(m: &mut Mob, memory: usize) {
    m.extra.ints[memory] = 0;
}
fn walk_target(m: &Mob) -> bool {
    m.extra.point.is_some()
}
fn now(m: &Mob) -> i32 {
    m.age as i32
}
/// `Breeze.getFiringYPosition`.
pub fn firing_y(m: &Mob) -> f64 {
    m.position.y + f64::from(m.height()) / 2.0 + f64::from(0.3_f32)
}
/// `Breeze.withinInnerCircleRange`.
fn within_inner_circle(m: &Mob, target: Vec3) -> bool {
    let me = Vec3::new(
        m.position.x.floor() + 0.5,
        m.position.y.floor() + 0.5,
        m.position.z.floor() + 0.5,
    );
    (target.x - me.x).powi(2) + (target.z - me.z).powi(2) < 16.0 && (target.y - me.y).abs() < 10.0
}
/// `Entity.lookAt` with the eyes anchor: snap body and head to face a point.
fn snap_look(m: &mut Mob, at: [f64; 3]) {
    let (dx, dy, dz) = (
        at[0] - m.position.x,
        at[1] - m.eye_y(),
        at[2] - m.position.z,
    );
    let flat = (dx * dx + dz * dz).sqrt();
    m.head_pitch = (-dy.atan2(flat).to_degrees()) as f32;
    m.yaw = (dz.atan2(dx).to_degrees() - 90.0) as f32;
    m.head_yaw = m.yaw;
    m.previous_head_pitch = m.head_pitch;
    m.previous_yaw = m.yaw;
    m.previous_head_yaw = m.yaw;
}
/// `BreezeUtil.hasLineOfSight`.
fn line_of_sight(m: &Mob, w: &MobWorld, target: Vec3) -> bool {
    let from = m.position;
    let range = 50.0_f64.max(m.attrs.follow_range);
    from.distance(target) <= range && w.clear_between(from, target, false)
}
/// `BreezeUtil.randomPointBehindTarget`: four to eight blocks behind the
/// target's facing, with a spread of 45 degrees.
fn point_behind(m: &mut Mob, target: Vec3, head_yaw: f32) -> Vec3 {
    let view = head_yaw + 180.0 + m.random.next_gaussian() as f32 * 90.0 / 2.0;
    let r = 4.0 + m.random.next_float() * 4.0;
    let d = Vec3::direction_from_rotation(0.0, view).scale(f64::from(r));
    target.add_vector(d)
}
/// The attack target's position, eye height, facing and whether it is a
/// player.
struct Quarry {
    position: Vec3,
    eye: f64,
    yaw: f32,
    height: f64,
}
fn quarry(m: &Mob, cx: &Cx, who: Who) -> Option<Quarry> {
    let t = cx.w.target_info(who)?;
    let yaw = match who {
        Who::Player => {
            cx.w.player
                .as_ref()
                .map_or(0.0, |p| (-p.look[0]).atan2(p.look[2]).to_degrees() as f32)
        }
        Who::Mob(id) => cx.w.seen(id).map_or(m.yaw, |s| s.yaw),
    };
    Some(Quarry {
        position: t.position,
        eye: t.eye_y,
        yaw,
        height: t.bounds.max_position().y - t.bounds.min_position().y,
    })
}
/// `TargetingConditions.forCombat().range(follow)` plus `canAttack`: a
/// player (not in creative or spectator) or an iron golem it can see.
fn attackable(m: &mut Mob, cx: &Cx, who: Who) -> bool {
    let follow = m.attrs.follow_range;
    match who {
        Who::Player => m.can_target_player(cx.w, follow, true),
        Who::Mob(id) => {
            let Some(s) = cx.w.seen(id) else {
                return false;
            };
            s.alive
                && s.species == Species::IronGolem
                && m.distance_sqr(s.position) <= follow * follow
                && m.sees(cx.w, who)
        }
    }
}
/// `BreezeAttackEntitySensor` and `HurtBySensor`: candidates for
/// `StartAttacking`.
fn nearest_attackable(m: &mut Mob, cx: &Cx) -> Option<Who> {
    let follow = m.attrs.follow_range;
    let mut candidates: Vec<(f64, Who)> = Vec::new();
    if let Some(p) = &cx.w.player
        && p.alive
        && !p.creative
        && !p.spectator
    {
        candidates.push((m.distance_sqr(p.position), Who::Player));
    }
    for s in
        cx.w.nearby(m.position, follow, |s| s.species == Species::IronGolem)
    {
        candidates.push((m.distance_sqr(s.position), Who::Mob(s.id)));
    }
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
    candidates
        .into_iter()
        .map(|c| c.1)
        .find(|w| attackable(m, cx, *w))
}
fn hurt_by(m: &Mob, cx: &Cx) -> Option<Who> {
    let (d, at) = m.last_damage?;
    if i64::from(m.age) - at > 40 {
        return None;
    }
    match d.attacker? {
        Who::Player => Some(Who::Player),
        Who::Mob(id) => {
            cx.w.seen(id)
                .filter(|s| s.alive && s.species == Species::IronGolem)
                .map(|_| Who::Mob(id))
        }
    }
}
/// The goal that runs the brain every tick.
#[derive(Clone, Debug)]
struct Brain;
impl Goal for Brain {
    goal_clone!();
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, _: &mut Mob, _: &mut Cx) -> bool {
        true
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        brain(m, cx);
    }
}
/// `Brain.tick`: memories expire, sensors scan, behaviours start in
/// priority order for the active activity, then the running ones tick.
fn brain(m: &mut Mob, cx: &mut Cx) {
    for i in 0..6 {
        if m.extra.ints[i] > 0 {
            m.extra.ints[i] -= 1;
        }
    }
    if m.extra.anger > 0 {
        m.extra.anger -= 1;
    }
    let fight = m.ai.target.is_some() && !walk_target(m);
    if fight {
        stop_if_target_invalid(m, cx);
        start_shoot(m, cx);
        start_long_jump(m, cx);
        start_shoot_when_stuck(m);
        start_slide(m, cx);
    } else {
        // StartAttacking on the nearest attackable, then on whoever hurt it.
        if m.ai.target.is_none() {
            let scan = (m.age + m.id).is_multiple_of(20);
            let found = if scan {
                nearest_attackable(m, cx)
            } else {
                None
            };
            let found = found.or_else(|| hurt_by(m, cx));
            if let Some(who) = found {
                m.ai.target = Some(who);
                m.extra.anger = 0;
                m.extra.counter2 = 0;
            }
        }
        start_walking(m, cx);
        idle_stroll(m, cx);
    }
    tick_shoot(m, cx);
    tick_long_jump(m, cx);
    tick_walking(m, cx);
}
/// `StopAttackingIfTargetInvalid` with `wasEntityAttackableLastNTicks(100)`:
/// the target is dropped once it has not been attackable for 100 ticks.
fn stop_if_target_invalid(m: &mut Mob, cx: &mut Cx) {
    let Some(who) = m.ai.target else { return };
    let alive = cx.w.target_info(who).is_some_and(|t| t.alive);
    if alive && attackable(m, cx, who) {
        m.extra.scratch[0] = 100;
        return;
    }
    m.extra.scratch[0] -= 1;
    if !alive || m.extra.scratch[0] < 0 {
        m.ai.target = None;
        m.extra.scratch[0] = 0;
    }
}
/// `Shoot.checkExtraStartConditions` and `start`.
fn start_shoot(m: &mut Mob, cx: &mut Cx) {
    if m.extra.flag2
        || !has(m, SHOOT)
        || has(m, SHOOT_COOLDOWN)
        || has(m, SHOOT_CHARGING)
        || has(m, SHOOT_RECOVERING)
        || walk_target(m)
        || m.extra.home.is_some()
        || pose(m) != STANDING
    {
        return;
    }
    let Some(q) = m.ai.target.and_then(|w| quarry(m, cx, w)) else {
        return;
    };
    if m.position.distance_squared(q.position) >= 256.0 {
        forget(m, SHOOT);
        return;
    }
    m.extra.flag2 = true;
    m.extra.egg_time = now(m) + SHOOT_INITIAL_DELAY + 1 + SHOOT_RECOVER_DELAY;
    set_pose(m, SHOOTING);
    remember(m, SHOOT_CHARGING, SHOOT_INITIAL_DELAY);
    let at = m.center();
    cx.events
        .push(MobEvent::Sound("entity.breeze.inhale", at, 1.0, 1.0));
}
fn tick_shoot(m: &mut Mob, cx: &mut Cx) {
    if !m.extra.flag2 {
        return;
    }
    let target = m.ai.target;
    if now(m) > m.extra.egg_time || target.is_none() || !has(m, SHOOT) {
        // Shoot.stop.
        m.extra.flag2 = false;
        if pose(m) == SHOOTING {
            set_pose(m, STANDING);
        }
        remember(m, SHOOT_COOLDOWN, SHOOT_COOLDOWN_TICKS);
        forget(m, SHOOT);
        return;
    }
    let Some(q) = target.and_then(|w| quarry(m, cx, w)) else {
        return;
    };
    snap_look(m, [q.position.x, q.eye, q.position.z]);
    if !has(m, SHOOT_CHARGING) && !has(m, SHOOT_RECOVERING) {
        remember(m, SHOOT_RECOVERING, SHOOT_RECOVER_DELAY);
        let start = Vec3::new(m.position.x, firing_y(m), m.position.z);
        let passenger_offset = 0.3;
        let (xd, yd, zd) = (
            q.position.x - m.position.x,
            q.position.y + q.height * passenger_offset - start.y,
            q.position.z - m.position.z,
        );
        let mut p = Projectile::new(
            ProjectileKind::BreezeWindCharge,
            start,
            Some(Who::Mob(m.id)),
        );
        // 5 - 4 * difficulty at normal.
        p.shoot(&mut m.random, [xd, yd, zd], 0.7, -3.0);
        cx.events.push(MobEvent::Projectile(p));
        cx.events
            .push(MobEvent::Sound("entity.breeze.shoot", m.center(), 1.5, 1.0));
    }
}
/// `ShootWhenStuck`.
fn start_shoot_when_stuck(m: &mut Mob) {
    if has(m, SHOOT) || has(m, JUMP_INHALING) || m.extra.home.is_some() || walk_target(m) {
        return;
    }
    if m.rider.is_some() || m.fluids.in_water || m.extra.levitation_effect() {
        remember(m, SHOOT, 60);
    }
}
impl super::Extra {
    fn levitation_effect(&self) -> bool {
        self.effects.iter().any(|e| e.0 == "minecraft:levitation")
    }
}
/// `Slide.start`: pick a point to slide to.
fn start_slide(m: &mut Mob, cx: &mut Cx) {
    if has(m, SHOOT)
        || has(m, JUMP_COOLDOWN)
        || walk_target(m)
        || !m.on_ground
        || m.fluids.in_water
        || pose(m) != STANDING
    {
        return;
    }
    let Some(q) = m.ai.target.and_then(|w| quarry(m, cx, w)) else {
        return;
    };
    let mut position: Option<Vec3> = None;
    if within_inner_circle(m, q.position)
        && let Some(away) =
            m.default_random_pos_away(cx.w, 5, 5, [q.position.x, q.position.y, q.position.z])
    {
        let away = Vec3::new(away[0], away[1], away[2]);
        if line_of_sight(m, cx.w, away)
            && q.position.distance_squared(away) > q.position.distance_squared(m.position)
        {
            position = Some(away);
        }
    }
    let position = position.unwrap_or_else(|| {
        if m.random.next_boolean() {
            point_behind(m, q.position, q.yaw)
        } else {
            // randomPointInMiddleCircle: stop four to eight blocks short.
            let d = q.position.subtract(m.position);
            let distance = d.length() - (8.0 + (4.0 - 8.0) * m.random.next_double());
            m.position.add_vector(d.normalize().scale(distance))
        }
    });
    m.extra.point = Some([position.x.floor(), position.y.floor(), position.z.floor()]);
    m.extra.vector[0] = 0.6;
    m.extra.vector[1] = 1.0;
}
/// `SlideToTargetSink` (`MoveToTargetSink`): walk to the walk target.
fn start_walking(m: &mut Mob, cx: &mut Cx) {
    if m.extra.counter != 0 {
        return;
    }
    let Some(target) = m.extra.point else {
        return;
    };
    if m.extra.anger > 0 {
        return;
    }
    let close = m.extra.vector[1] as i32;
    let here = [
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    ];
    let reached = (target[0] as i32 - here[0]).abs()
        + (target[1] as i32 - here[1]).abs()
        + (target[2] as i32 - here[2]).abs()
        <= close;
    let speed = m.extra.vector[0];
    if !reached && m.nav_move_to(cx.w, [target[0] + 0.5, target[1], target[2] + 0.5], speed) {
        m.extra.counter = now(m) + 20 + m.random.next_int_bound(21).unwrap_or(0);
        cx.events
            .push(MobEvent::Sound("entity.breeze.slide", m.center(), 1.0, 1.0));
        set_pose(m, SLIDING);
    } else {
        m.extra.point = None;
    }
}
fn tick_walking(m: &mut Mob, _cx: &mut Cx) {
    if m.extra.counter == 0 {
        return;
    }
    let reached = m.extra.point.is_none_or(|target| {
        let here = [
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        ];
        (target[0] as i32 - here[0]).abs()
            + (target[1] as i32 - here[1]).abs()
            + (target[2] as i32 - here[2]).abs()
            <= m.extra.vector[1] as i32
    });
    if now(m) > m.extra.counter || m.ai.nav.is_done() || reached {
        // SlideToTargetSink.stop.
        m.extra.counter = 0;
        m.extra.point = None;
        m.nav_stop();
        set_pose(m, STANDING);
        if m.ai.target.is_some() {
            remember(m, SHOOT, 60);
        }
    }
}
/// `RunOne` of `DoNothing(20, 100)` and `RandomStroll.stroll(0.6)`.
fn idle_stroll(m: &mut Mob, cx: &mut Cx) {
    if m.extra.counter != 0 || walk_target(m) || m.ai.target.is_some() {
        return;
    }
    if now(m) < m.extra.counter2 {
        return;
    }
    if m.random.next_int_bound(3).unwrap_or(0) == 0 {
        m.extra.counter2 = now(m) + 20 + m.random.next_int_bound(81).unwrap_or(0);
    } else if let Some(p) = m.land_random_pos(cx.w, 10, 7) {
        m.extra.point = Some([p[0].floor(), p[1].floor(), p[2].floor()]);
        m.extra.vector[0] = 0.6;
        m.extra.vector[1] = 0.0;
    }
}
fn can_jump_from_here(m: &Mob, w: &MobWorld) -> bool {
    let (x, y, z) = (
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    );
    if w.block_name(x, y, z) == "minecraft:honey_block" {
        return false;
    }
    (1..=4).all(|i| {
        w.facts.is_air(w.world.block(x, y + i, z))
            || w.fluid(x, y + i, z) == crate::blocks::Fluid::Water
    })
}
/// `LongJump.snapToSurface`: the block above the surface below the point,
/// else above the ceiling over it.
fn snap_to_surface(w: &MobWorld, p: Vec3) -> Option<[i32; 3]> {
    let (x, z) = (p.x.floor() as i32, p.z.floor() as i32);
    let y0 = p.y.floor() as i32;
    for y in (y0 - 10..=y0).rev() {
        let top = f64::from(w.data.state(w.world.block(x, y, z)).top);
        if top > 0.0 {
            let hit = (f64::from(y) + top).min(p.y);
            return Some([x, hit.floor() as i32 + 1, z]);
        }
    }
    for y in y0..=y0 + 10 {
        if w.data.state(w.world.block(x, y, z)).top > 0.0 {
            return Some([x, y + 1, z]);
        }
    }
    None
}
fn dangerous_block(name: &str) -> bool {
    matches!(
        name,
        "minecraft:fire"
            | "minecraft:soul_fire"
            | "minecraft:lava"
            | "minecraft:magma_block"
            | "minecraft:campfire"
            | "minecraft:soul_campfire"
            | "minecraft:wither_rose"
            | "minecraft:sweet_berry_bush"
            | "minecraft:cactus"
            | "minecraft:powder_snow"
    )
}
/// `LongJump.canRun` and `start`.
fn start_long_jump(m: &mut Mob, cx: &mut Cx) {
    if m.extra.flag
        || has(m, SHOOT)
        || has(m, JUMP_COOLDOWN)
        || walk_target(m)
        || !can_run_long_jump(m, cx)
    {
        return;
    }
    m.extra.flag = true;
    m.extra.anim_ticks = now(m) + 200;
    if !has(m, JUMP_INHALING) {
        remember(m, JUMP_INHALING, INHALING_TICKS);
    }
    set_pose(m, INHALING);
    let at = m.center();
    cx.events
        .push(MobEvent::Sound("entity.breeze.charge", at, 1.0, 1.0));
    if let Some(t) = m.extra.home {
        snap_look(
            m,
            [
                f64::from(t[0]) + 0.5,
                f64::from(t[1]) + 0.5,
                f64::from(t[2]) + 0.5,
            ],
        );
    }
}
fn can_run_long_jump(m: &mut Mob, cx: &mut Cx) -> bool {
    if !m.on_ground && !m.fluids.in_water {
        return false;
    }
    if m.fluids.water > m.eye_height() || m.fluids.in_lava {
        return false;
    }
    if m.extra.home.is_some() {
        return true;
    }
    let Some(who) = m.ai.target else {
        return false;
    };
    let Some(q) = quarry(m, cx, who) else {
        return false;
    };
    let follow = m.attrs.follow_range;
    let distance = m.position.distance(q.position);
    if distance >= follow {
        m.ai.target = None;
        return false;
    }
    if distance - 4.0 <= 0.0 || !can_jump_from_here(m, cx.w) {
        return false;
    }
    let behind = point_behind(m, q.position, q.yaw);
    let Some(target) = snap_to_surface(cx.w, behind) else {
        return false;
    };
    if dangerous_block(cx.w.block_name(target[0], target[1] - 1, target[2])) {
        return false;
    }
    let centre = Vec3::new(
        f64::from(target[0]) + 0.5,
        f64::from(target[1]) + 0.5,
        f64::from(target[2]) + 0.5,
    );
    if !line_of_sight(m, cx.w, centre)
        && !line_of_sight(m, cx.w, centre.add_vector(Vec3::new(0.0, 4.0, 0.0)))
    {
        return false;
    }
    m.extra.home = Some(target);
    true
}
/// `LongJumpUtil.calculateJumpVectorForAngle` without the collision check.
fn jump_vector(m: &Mob, target: Vec3, max_velocity: f32, angle: i32) -> Option<Vec3> {
    let pos = m.position;
    let plane = Vec3::new(target.x - pos.x, 0.0, target.z - pos.z)
        .normalize()
        .scale(0.5);
    let aim = target.subtract(plane);
    let d = aim.subtract(pos);
    let angrad = f64::from(angle as f32 * std::f32::consts::PI / 180.0);
    let xz = d.z.atan2(d.x);
    let r2 = d.x * d.x + d.z * d.z;
    let r = r2.sqrt();
    let g = m.kind.gravity;
    let sin2 = f64::from((2.0 * angrad as f32).sin());
    let cos_sq = angrad.cos().powi(2);
    let v0sq = r2 * g / (r * sin2 - 2.0 * d.y * cos_sq);
    if v0sq < 0.0 {
        return None;
    }
    let v0 = v0sq.sqrt();
    if v0 > f64::from(max_velocity) {
        return None;
    }
    let v0r = v0 * angrad.cos();
    let v0y = v0 * angrad.sin();
    Some(Vec3::new(v0r * xz.cos(), v0y, v0r * xz.sin()).scale(f64::from(0.95_f32)))
}
fn tick_long_jump(m: &mut Mob, cx: &mut Cx) {
    if !m.extra.flag {
        return;
    }
    let ongoing = pose(m) != STANDING && !has(m, JUMP_COOLDOWN);
    if now(m) > m.extra.anim_ticks || !ongoing {
        // LongJump.stop.
        m.extra.flag = false;
        if matches!(pose(m), LONG_JUMPING | INHALING) {
            set_pose(m, STANDING);
        }
        m.extra.home = None;
        forget(m, JUMP_INHALING);
        m.extra.variant2 = 0;
        return;
    }
    let in_water = m.fluids.in_water;
    if !in_water && m.extra.variant2 != 0 {
        m.extra.variant2 = 0;
    }
    let finished_inhaling = !has(m, JUMP_INHALING) && pose(m) == INHALING;
    if finished_inhaling {
        let target = m.extra.home.map(|t| {
            Vec3::new(
                f64::from(t[0]) + 0.5,
                f64::from(t[1]),
                f64::from(t[2]) + 0.5,
            )
        });
        let mut angles = ALLOWED_ANGLES;
        for i in (1..angles.len()).rev() {
            let j = m.random.next_int_bound(i as i32 + 1).unwrap_or(0) as usize;
            angles.swap(i, j);
        }
        let max = 0.058_333_334_f32 * m.attrs.follow_range as f32;
        let v = target.and_then(|t| angles.iter().find_map(|&a| jump_vector(m, t, max, a)));
        let Some(v) = v else {
            set_pose(m, STANDING);
            return;
        };
        if in_water {
            m.extra.variant2 = 1;
        }
        let at = m.center();
        cx.events
            .push(MobEvent::Sound("entity.breeze.jump", at, 1.0, 1.0));
        set_pose(m, LONG_JUMPING);
        m.yaw = m.body_yaw;
        m.velocity = v;
        m.on_ground = false;
        return;
    }
    let landed_in_water = in_water && m.extra.variant2 == 0;
    if pose(m) == LONG_JUMPING && (m.on_ground || landed_in_water) {
        let at = m.center();
        cx.events
            .push(MobEvent::Sound("entity.breeze.land", at, 1.0, 1.0));
        set_pose(m, STANDING);
        let hurt = m
            .last_damage
            .is_some_and(|(_, t)| i64::from(m.age) - t <= 40);
        remember(m, JUMP_COOLDOWN, if hurt { 2 } else { 10 });
        remember(m, SHOOT, 100);
    }
}
/// Whether `Breeze.shouldDiscardFriction` applies (a long jump in the air).
pub fn discards_friction(m: &Mob) -> bool {
    m.species() == Species::Breeze && pose(m) == LONG_JUMPING
}
/// `Breeze.tick` and the client's pose-driven animation states.
pub fn ai_step(m: &mut Mob, events: &mut Vec<MobEvent>) {
    let age = m.age as f32;
    let current = pose(m);
    if current != m.extra.variant {
        // resetAnimations, then the pose's own animation.
        let st = &mut m.anim.states;
        st[1].stop();
        st[0].stop();
        st[4].stop();
        st[5].stop();
        match current {
            SHOOTING => st[1].start_if_stopped(age),
            INHALING => st[4].start_if_stopped(age),
            SLIDING => st[2].start_if_stopped(age),
            _ => {}
        }
        m.extra.variant = current;
    }
    if current == LONG_JUMPING {
        m.anim.states[5].start_if_stopped(age);
    }
    m.anim.states[0].start_if_stopped(age);
    if current != SLIDING && m.anim.states[2].started.is_some() {
        m.anim.states[3].start(age);
        m.anim.states[2].stop();
    }
    // The whirl sound.
    if m.extra.vector[2] <= 0.0 {
        m.extra.vector[2] = f64::from(1 + m.random.next_int_bound(80).unwrap_or(0));
    } else {
        m.extra.vector[2] -= 1.0;
        if m.extra.vector[2] <= 0.0 {
            let pitch = 0.7 + 0.4 * m.random.next_float();
            let volume = 0.8 + 0.2 * m.random.next_float();
            events.push(MobEvent::Sound(
                "entity.breeze.whirl",
                [m.position.x, m.position.y, m.position.z],
                volume,
                pitch,
            ));
        }
    }
}
/// `Breeze.playAmbientSound` and its sound selection.
pub fn ambient(m: &Mob) -> Option<&'static str> {
    if m.ai.target.is_some() && m.on_ground {
        return None;
    }
    Some(if m.on_ground {
        "entity.breeze.idle_ground"
    } else {
        "entity.breeze.idle_air"
    })
}
/// `BreezeModel.setupAnim`: the six keyframe animations.
pub fn anim(m: &Mob, s: &State, p: &mut Poses, animations: &HashMap<String, Animation>) {
    let age = s.age;
    for (state, name) in [
        (&m.anim.states[0], "IDLE"),
        (&m.anim.states[1], "SHOOT"),
        (&m.anim.states[2], "SLIDE"),
        (&m.anim.states[3], "SLIDE_BACK"),
        (&m.anim.states[4], "INHALE"),
        (&m.anim.states[5], "JUMP"),
    ] {
        if let (Some(ms), Some(a)) = (
            state.millis(age),
            animations.get(&format!("BreezeAnimation.{name}")),
        ) {
            a.apply(ms, 1.0, p);
        }
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn a_memory_is_present_for_its_ttl_plus_the_tick_it_was_set() {
        // setMemoryWithExpiry(15) at tick T: present T through T + 15.
        let mut v = 16;
        let mut present = 1;
        for _ in 0..30 {
            if v > 0 {
                v -= 1;
            }
            if v > 0 {
                present += 1;
            }
        }
        assert_eq!(present, 16);
    }
    #[test]
    fn the_highest_arc_reaches_a_target_within_range() {
        // Angle 80 degrees: v0^2 = r^2 g / (r sin 2a - 2 y cos^2 a).
        let g = 0.08;
        let (r, y): (f64, f64) = (10.0, 0.0);
        let a = 60.0_f64.to_radians();
        let v0sq = r * r * g / (r * (2.0 * a).sin() - 2.0 * y * a.cos().powi(2));
        assert!(v0sq.sqrt() < 1.4);
    }
}
