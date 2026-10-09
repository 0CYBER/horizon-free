//! Tamable animals (`TamableAnimal`): wolves with their variants, taming,
//! sitting, following and defending the owner, begging, anger and shaking
//! off water (`Wolf`, `WolfModel`).
use super::{
    Damage, Mob, MobEvent, MobWorld, Species, Who,
    anim::{Poses, State},
    goals::{
        Breed, Cx, Float, Goal, HurtByTarget, JUMP, LOOK, LeapAtTarget, LookAtPlayer, MOVE,
        MeleeAttack, NearestTarget, Panic, Prey, RandomLookAround, Stroll, TARGET, goal_clone,
        reduced,
    },
};
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::RandomSource,
};
use std::f32::consts::PI;

/// Wolf variants: registry name and texture file (`WolfVariants`).
pub const WOLF_VARIANTS: [(&str, &str); 9] = [
    ("pale", "wolf"),
    ("spotted", "wolf_spotted"),
    ("snowy", "wolf_snowy"),
    ("black", "wolf_black"),
    ("ashen", "wolf_ashen"),
    ("rusty", "wolf_rusty"),
    ("woods", "wolf_woods"),
    ("chestnut", "wolf_chestnut"),
    ("striped", "wolf_striped"),
];
/// The wolf variant a biome spawns (its high-priority `BiomeCheck`).
pub fn wolf_variant_for(biome: &str, has_tag: &dyn Fn(&str) -> bool) -> u8 {
    if has_tag("minecraft:is_savanna") {
        1
    } else if biome == "minecraft:grove" {
        2
    } else if biome == "minecraft:old_growth_pine_taiga" {
        3
    } else if biome == "minecraft:snowy_taiga" {
        4
    } else if has_tag("minecraft:is_jungle") {
        5
    } else if biome == "minecraft:forest" {
        6
    } else if biome == "minecraft:old_growth_spruce_taiga" {
        7
    } else if has_tag("minecraft:is_badlands") {
        8
    } else {
        0
    }
}
/// The default collar: red.
pub const RED: u8 = 14;

// AnimState slots: begging interest and the shake animation.
const INTEREST: usize = 0;
const SHAKE: usize = 1;

pub fn register(m: &mut Mob) {
    m.extra.color = RED;
    let food = m.kind.food.clone();
    let g = &mut m.ai.goals;
    g.add(1, Float);
    g.add(1, TamePanic(Panic::new(1.5)));
    g.add(2, SitWhenOrdered);
    g.add(3, AvoidLlamas);
    g.add(4, LeapAtTarget::new(0.4));
    g.add(5, MeleeAttack::new(1.0, true));
    g.add(6, FollowOwner::new(1.0, 10.0, 2.0));
    g.add(7, Breed::new(1.0));
    g.add(8, Stroll::avoiding_water(1.0));
    g.add(9, Beg { food, ticks: 0 });
    g.add(10, LookAtPlayer::new(8.0));
    g.add(10, RandomLookAround::default());
    m.ai.targets.add(1, OwnerHurtBy { timestamp: 0 });
    m.ai.targets.add(2, OwnerHurt { timestamp: 0 });
    m.ai.targets.add(3, HurtByTarget::new().alert());
    m.ai.targets
        .add(4, NearestTarget::player(true).when(angry_at_player));
    m.ai.targets.add(
        5,
        NearestTarget::new(Prey::Filter(wolf_prey), false).when(|m, _| !m.extra.tame),
    );
    m.ai.targets.add(
        6,
        NearestTarget::new(Prey::Filter(baby_turtle_on_land), false).when(|m, _| !m.extra.tame),
    );
    m.ai.targets.add(
        7,
        NearestTarget::species(
            &[
                Species::Skeleton,
                Species::Stray,
                Species::Bogged,
                Species::WitherSkeleton,
            ],
            false,
        ),
    );
}
/// `Wolf.PREY_SELECTOR`: sheep, rabbits and foxes.
fn wolf_prey(s: &super::Seen) -> bool {
    matches!(s.species, Species::Sheep | Species::Rabbit | Species::Fox)
}
fn baby_turtle_on_land(s: &super::Seen) -> bool {
    s.species == Species::Turtle && s.baby
}
fn angry_at_player(m: &Mob, _: &MobWorld) -> bool {
    m.extra.anger > 0
}
/// `Wolf.isAngry`.
pub fn angry(m: &Mob) -> bool {
    m.extra.anger > 0
}
/// `TamableAnimal.applyTamingSideEffects` for wolves: 40 health when tame.
pub fn set_tame(m: &mut Mob, tame: bool) {
    m.extra.tame = tame;
    if tame {
        m.attrs.max_health = 40.0;
        m.health = 40.0;
    } else {
        m.attrs.max_health = 8.0;
    }
}
/// The owner (the local player) when tame.
fn owner(m: &Mob, w: &MobWorld) -> Option<(Vec3, f64, bool)> {
    if !m.extra.tame {
        return None;
    }
    let p = w.player.as_ref()?;
    Some((p.position, p.eye[1], p.spectator))
}
/// `TamableAnimal.unableToMoveToOwner`.
fn unable_to_move_to_owner(m: &Mob, w: &MobWorld) -> bool {
    m.extra.sitting || w.player.as_ref().is_some_and(|p| p.spectator)
}
/// `TamableAnimal.tryToTeleportToOwner`: ten tries around the owner.
fn teleport_to_owner(m: &mut Mob, w: &MobWorld) {
    let Some((at, _, _)) = owner(m, w) else {
        return;
    };
    let base = [
        at.x.floor() as i32,
        at.y.floor() as i32,
        at.z.floor() as i32,
    ];
    for _ in 0..10 {
        let xd = m.random.next_int_bound(7).unwrap_or(0) - 3;
        let zd = m.random.next_int_bound(7).unwrap_or(0) - 3;
        if xd.abs() < 2 && zd.abs() < 2 {
            continue;
        }
        let yd = m.random.next_int_bound(3).unwrap_or(0) - 1;
        let p = [base[0] + xd, base[1] + yd, base[2] + zd];
        if can_teleport_to(m, w, p) {
            m.position = Vec3::new(
                f64::from(p[0]) + 0.5,
                f64::from(p[1]),
                f64::from(p[2]) + 0.5,
            );
            m.previous = m.position;
            m.nav_stop();
            return;
        }
    }
}
/// `TamableAnimal.canTeleportTo`: walkable, not on leaves, no collision.
fn can_teleport_to(m: &Mob, w: &MobWorld, p: [i32; 3]) -> bool {
    let below = w.facts.get(w.world.block(p[0], p[1] - 1, p[2]));
    if !below.full_collision || below.name.ends_with("_leaves") {
        return false;
    }
    let here = w.facts.get(w.world.block(p[0], p[1], p[2]));
    if here.fluid != crate::blocks::Fluid::None {
        return false;
    }
    let half = f64::from(m.width()) / 2.0;
    let (x, y, z) = (
        f64::from(p[0]) + 0.5,
        f64::from(p[1]),
        f64::from(p[2]) + 0.5,
    );
    w.no_collision(Aabb::new(
        Vec3::new(x - half, y, z - half),
        Vec3::new(x + half, y + f64::from(m.height()), z + half),
    ))
}
fn owner_distance_sqr(m: &Mob, w: &MobWorld) -> Option<f64> {
    owner(m, w).map(|(at, _, _)| m.distance_sqr(at))
}

/// `TamableAnimal.TamableAnimalPanicGoal`: panic from fire and freezing,
/// teleporting to a far owner.
#[derive(Clone, Debug)]
struct TamePanic(Panic);
impl Goal for TamePanic {
    goal_clone!();
    fn flags(&self) -> u8 {
        self.0.flags()
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        // PanicGoal with PANIC_ENVIRONMENTAL_CAUSES: only burning or freezing.
        m.fire_ticks > 0 && self.0.can_use(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_continue(m, cx)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.start(m, cx)
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.stop(m, cx)
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if !unable_to_move_to_owner(m, cx.w)
            && owner_distance_sqr(m, cx.w).is_some_and(|d| d >= 144.0)
        {
            teleport_to_owner(m, cx.w);
        }
        self.0.tick(m, cx);
    }
}
/// `SitWhenOrderedToGoal`.
#[derive(Clone, Debug)]
struct SitWhenOrdered;
impl Goal for SitWhenOrdered {
    goal_clone!();
    fn flags(&self) -> u8 {
        JUMP | MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if !m.extra.sitting && !m.extra.tame {
            return false;
        }
        if m.fluids.in_water || !m.on_ground {
            return false;
        }
        match owner_distance_sqr(m, cx.w) {
            None => true,
            Some(d) => {
                let attacked = cx.w.player.as_ref().is_some_and(|p| p.hurt_by.is_some());
                !(d < 144.0 && attacked) && m.extra.sitting
            }
        }
    }
    fn can_continue(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        m.extra.sitting
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        m.nav_stop();
        m.extra.flag = false;
    }
}
/// `Wolf.WolfAvoidEntityGoal`: untamed wolves flee llamas.
#[derive(Clone, Debug)]
struct AvoidLlamas;
impl Goal for AvoidLlamas {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if m.extra.tame {
            return false;
        }
        let pos = m.position;
        let llama =
            cx.w.nearby(pos, 24.0, |s| {
                matches!(s.species, Species::Llama | Species::TraderLlama) && s.alive
            })
            .map(|s| s.position)
            .next();
        let Some(from) = llama else {
            return false;
        };
        // DefaultRandomPos.getPosAway(16, 7, from).
        let away = Vec3::new(pos.x - from.x, 0.0, pos.z - from.z);
        let len = (away.x * away.x + away.z * away.z).sqrt().max(1e-6);
        let target = [
            pos.x + away.x / len * 16.0,
            pos.y,
            pos.z + away.z / len * 16.0,
        ];
        m.nav_move_to(cx.w, target, 1.5)
    }
    fn can_continue(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        !m.ai.nav.is_done()
    }
}
/// `FollowOwnerGoal(speed, startDistance, stopDistance)`.
#[derive(Clone, Debug)]
struct FollowOwner {
    speed: f64,
    start: f64,
    stop: f64,
    recalc: i32,
}
impl FollowOwner {
    fn new(speed: f64, start: f64, stop: f64) -> Self {
        Self {
            speed,
            start,
            stop,
            recalc: 0,
        }
    }
}
impl Goal for FollowOwner {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some(d) = owner_distance_sqr(m, cx.w) else {
            return false;
        };
        !unable_to_move_to_owner(m, cx.w) && d >= self.start * self.start
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if m.ai.nav.is_done() || unable_to_move_to_owner(m, cx.w) {
            return false;
        }
        owner_distance_sqr(m, cx.w).is_some_and(|d| d > self.stop * self.stop)
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        self.recalc = 0;
        m.set_malus(super::path::PathType::Water, 0.0);
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.nav_stop();
        let water = super::path::PathType::Water.malus();
        m.set_malus(super::path::PathType::Water, water);
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some((at, eye, _)) = owner(m, cx.w) else {
            return;
        };
        let far = m.distance_sqr(at) >= 144.0;
        if !far {
            let max = max_head_x_rot(m);
            m.look_at_speed([at.x, eye, at.z], 10.0, max);
        }
        self.recalc -= 1;
        if self.recalc <= 0 {
            self.recalc = reduced(10);
            if far {
                teleport_to_owner(m, cx.w);
            } else {
                m.nav_move_to_entity(cx.w, [at.x, at.y, at.z], 0, self.speed);
            }
        }
    }
}
/// `OwnerHurtByTargetGoal`: attack what hurt the owner.
#[derive(Clone, Debug)]
struct OwnerHurtBy {
    timestamp: i64,
}
impl Goal for OwnerHurtBy {
    goal_clone!();
    fn flags(&self) -> u8 {
        TARGET
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if !m.extra.tame || m.extra.sitting {
            return false;
        }
        let Some((who, at)) = cx.w.player.as_ref().and_then(|p| p.hurt_by) else {
            return false;
        };
        at != self.timestamp && wants_to_attack(m, cx.w, who)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.ai.target
            .and_then(|t| cx.w.target_info(t))
            .is_some_and(|t| t.alive)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some((who, at)) = cx.w.player.as_ref().and_then(|p| p.hurt_by) {
            m.ai.target = Some(who);
            self.timestamp = at;
        }
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.ai.target = None;
    }
}
/// `OwnerHurtTargetGoal`: attack what the owner attacks.
#[derive(Clone, Debug)]
struct OwnerHurt {
    timestamp: i64,
}
impl Goal for OwnerHurt {
    goal_clone!();
    fn flags(&self) -> u8 {
        TARGET
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if !m.extra.tame || m.extra.sitting {
            return false;
        }
        let Some((who, at)) = cx.w.player.as_ref().and_then(|p| p.last_hurt) else {
            return false;
        };
        at != self.timestamp && wants_to_attack(m, cx.w, who)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.ai.target
            .and_then(|t| cx.w.target_info(t))
            .is_some_and(|t| t.alive)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some((who, at)) = cx.w.player.as_ref().and_then(|p| p.last_hurt) {
            m.ai.target = Some(who);
            self.timestamp = at;
        }
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.ai.target = None;
    }
}
/// `Wolf.wantsToAttack`: never creepers, ghasts or tame wolves and cats.
fn wants_to_attack(m: &Mob, w: &MobWorld, who: Who) -> bool {
    let Who::Mob(id) = who else {
        return false;
    };
    if id == m.id {
        return false;
    }
    let Some(s) = w.seen(id) else {
        return false;
    };
    match s.species {
        Species::Creeper | Species::Ghast => false,
        Species::Wolf | Species::Cat | Species::Parrot => !s.tame,
        Species::Horse | Species::Donkey | Species::Mule | Species::Llama => !s.tame,
        _ => true,
    }
}
/// `BegGoal`: tilt the head at a player holding something interesting.
#[derive(Clone, Debug)]
struct Beg {
    food: std::sync::Arc<[String]>,
    ticks: i32,
}
impl Beg {
    fn interesting(&self, m: &Mob, w: &MobWorld) -> bool {
        let Some(p) = &w.player else {
            return false;
        };
        let held = |i: &Option<std::sync::Arc<str>>| {
            i.as_deref().is_some_and(|i| {
                (!m.extra.tame && i == "minecraft:bone")
                    || (m.extra.tame && self.food.iter().any(|f| f == i))
            })
        };
        p.alive && !p.spectator && (held(&p.main_hand) || held(&p.off_hand))
    }
}
impl Goal for Beg {
    goal_clone!();
    fn flags(&self) -> u8 {
        LOOK
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        cx.w.player
            .as_ref()
            .is_some_and(|p| m.distance_sqr(p.position) <= 64.0)
            && self.interesting(m, cx.w)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        cx.w.player
            .as_ref()
            .is_some_and(|p| p.alive && m.distance_sqr(p.position) <= 64.0)
            && self.ticks > 0
            && self.interesting(m, cx.w)
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        m.extra.flag = true;
        self.ticks = reduced(40 + m.random.next_int_bound(40).unwrap_or(0));
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.extra.flag = false;
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some(p) = &cx.w.player {
            let max = max_head_x_rot(m);
            m.look_at_speed([p.position.x, p.eye[1], p.position.z], 10.0, max);
        }
        self.ticks -= 1;
    }
}
/// `Wolf.tick` / `aiStep`: anger countdown, begging tilt and shaking off water.
pub fn ai_step(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    if m.extra.anger > 0 {
        m.extra.anger -= 1;
        if m.extra.anger == 0 && m.ai.target == Some(Who::Player) {
            m.ai.target = None;
        }
    }
    if !m.alive() {
        return;
    }
    // interestedAngle eases toward 1 while begging.
    let goal = if m.extra.flag { 1.0 } else { 0.0 };
    let v = &mut m.anim.v;
    v[INTEREST] += (goal - v[INTEREST]) * 0.4;
    // isWet (extra.flag2), isShaking (counter2 > 0).
    let rain = w.raining
        && w.can_see_sky(
            m.position.x.floor() as i32,
            (m.position.y + f64::from(m.height())).floor() as i32,
            m.position.z.floor() as i32,
        );
    let shaking = m.extra.counter2 > 0;
    if m.fluids.in_water || rain {
        m.extra.flag2 = true;
        if shaking {
            m.extra.counter2 = 0;
            m.anim.v[SHAKE] = 0.0;
            m.anim.vo[SHAKE] = 0.0;
        }
    } else if shaking {
        if m.anim.v[SHAKE] == 0.0 {
            let pitch = (m.random.next_float() - m.random.next_float()) * 0.2 + 1.0;
            events.push(MobEvent::Sound("entity.wolf.shake", m.center(), 0.4, pitch));
        }
        m.anim.v[SHAKE] += 0.05;
        if m.anim.vo[SHAKE] >= 2.0 {
            m.extra.flag2 = false;
            m.extra.counter2 = 0;
            m.anim.v[SHAKE] = 0.0;
            m.anim.vo[SHAKE] = 0.0;
        }
        if m.anim.v[SHAKE] > 0.4 {
            let count = (((m.anim.v[SHAKE] - 0.4) * PI).sin() * 7.0) as u32;
            if count > 0 {
                events.push(MobEvent::Particles {
                    kind: "splash",
                    at: [m.position.x, m.position.y + 0.8, m.position.z],
                    count,
                });
            }
        }
    } else if m.extra.flag2 && m.ai.nav.is_done() && m.on_ground {
        // Wolf.aiStep: start shaking when wet, standing and not pathing.
        m.extra.counter2 = 1;
        m.anim.v[SHAKE] = 0.0;
        m.anim.vo[SHAKE] = 0.0;
    }
}
/// `Wolf.hurtServer` and `NeutralMob` anger: stop sitting; untamed wolves
/// hit by the player stay angry for 20 to 39 seconds.
pub fn after_hurt(m: &mut Mob, source: &Damage) {
    m.extra.sitting = false;
    if source.attacker == Some(Who::Player) && !m.extra.tame {
        m.extra.anger = 400 + m.random.next_int_bound(380).unwrap_or(0);
    }
}
/// `Wolf.getTailAngle`.
fn tail_angle(m: &Mob) -> f32 {
    if angry(m) {
        1.539_380_4
    } else if m.extra.tame {
        let max = m.attrs.max_health;
        let ratio = (max - m.health) / max;
        (0.55 - ratio * 0.4) * PI
    } else {
        PI / 5.0
    }
}
/// `WolfRenderState.getBodyRollAngle`.
fn body_roll(shake: f32, offset: f32) -> f32 {
    let p = ((shake + offset) / 1.8).clamp(0.0, 1.0);
    (p * PI).sin() * (p * PI * 11.0).sin() * 0.15 * PI
}
/// `WolfModel.setupAnim` with the adult and baby overrides.
pub fn anim(m: &Mob, s: &State, p: &mut Poses) {
    let (pos, speed) = (s.pos, s.speed);
    let baby = m.is_baby();
    if angry(m) {
        p.yrot("tail", 0.0);
    } else {
        p.yrot("tail", (pos * 0.6662).cos() * 1.4 * speed);
    }
    if m.extra.sitting {
        let age = if baby { 0.5 } else { 1.0 };
        let add = |p: &mut Poses, part: &str, x: f32, y: f32, z: f32| {
            let v = p.get(part);
            v[0] += x;
            v[1] += y;
            v[2] += z;
        };
        add(p, "body", 0.0, 4.0 * age, -2.0 * age);
        p.xrot("body", PI / 4.0);
        add(p, "tail", 0.0, 9.0 * age, -2.0 * age);
        add(p, "right_hind_leg", 0.0, 6.7 * age, -5.0 * age);
        p.xrot("right_hind_leg", PI * 3.0 / 2.0);
        add(p, "left_hind_leg", 0.0, 6.7 * age, -5.0 * age);
        p.xrot("left_hind_leg", PI * 3.0 / 2.0);
        p.xrot("right_front_leg", 5.811_947);
        add(p, "right_front_leg", 0.01 * age, 1.0 * age, 0.0);
        p.xrot("left_front_leg", 5.811_947);
        add(p, "left_front_leg", -0.01 * age, 1.0 * age, 0.0);
        if baby {
            let x = p.get("body")[3] - PI / 2.0;
            p.xrot("body", x);
        } else if p.has("upper_body") {
            p.get("upper_body")[1] += 2.0;
            p.xrot("upper_body", PI * 2.0 / 5.0);
            p.yrot("upper_body", 0.0);
        }
    } else {
        p.xrot("right_hind_leg", (pos * 0.6662).cos() * 1.4 * speed);
        p.xrot("left_hind_leg", (pos * 0.6662 + PI).cos() * 1.4 * speed);
        p.xrot("right_front_leg", (pos * 0.6662 + PI).cos() * 1.4 * speed);
        p.xrot("left_front_leg", (pos * 0.6662).cos() * 1.4 * speed);
    }
    // shakeOffWater
    let shake = m.anim.lerp(SHAKE, s.partial);
    let roll = m.anim.lerp(INTEREST, s.partial) * 0.15 * PI;
    p.zrot("body", body_roll(shake, -0.16));
    if baby {
        p.zrot("head", roll + body_roll(shake, 0.0));
        p.zrot("tail", body_roll(shake, -0.2));
    } else {
        if p.has("real_head") {
            p.zrot("real_head", roll + body_roll(shake, 0.0));
        }
        if p.has("upper_body") {
            p.zrot("upper_body", body_roll(shake, -0.08));
        }
        if p.has("real_tail") {
            p.zrot("real_tail", body_roll(shake, -0.2));
        }
    }
    p.xrot("head", s.x_rot.to_radians());
    p.yrot("head", s.y_rot.to_radians());
    p.xrot("tail", tail_angle(m));
}
/// `Wolf.getWetShade` as a vertex colour.
pub fn tint(m: &Mob, partial: f32) -> u32 {
    if !m.extra.flag2 {
        return 0xFFFF_FFFF;
    }
    let shade = (0.75 + m.anim.lerp(SHAKE, partial) / 2.0 * 0.25).min(1.0);
    let c = (shade * 255.0) as u32;
    0xFF00_0000 | c << 16 | c << 8 | c
}
/// `Wolf.getTexture`.
pub fn texture(m: &Mob) -> String {
    let file = WOLF_VARIANTS[(m.extra.variant as usize).min(8)].1;
    let state = if m.extra.tame {
        "_tame"
    } else if angry(m) {
        "_angry"
    } else {
        ""
    };
    let baby = if m.is_baby() { "_baby" } else { "" };
    format!("entity/wolf/{file}{state}{baby}")
}
/// `Wolf.getMaxHeadXRot`: 20 degrees while sitting.
fn max_head_x_rot(m: &Mob) -> f32 {
    if m.extra.sitting {
        20.0
    } else {
        m.kind.max_head_x_rot
    }
}
