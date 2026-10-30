//! Sulfur cubes (`SulfurCube` on `AbstractCubeMob`): hopping like a slime
//! but harmless, size 1 babies growing into size 2 adults, swallowing a
//! block (by hand, from the ground or a dispenser) that stops it in place
//! and gives it the block's archetype (`SulfurCubeArchetype`: buoyancy,
//! knockback, sounds, contact damage, a TNT fuse), shearing the block out,
//! buckets, and the outer/inner models and their render scale.
use super::{
    Damage, DamageKind, Mob, MobEvent, MobWorld,
    control::MoveKind,
    goals::{Cx, Goal, JUMP, LOOK, MOVE, goal_clone},
};
use horizon_core::{geometry::Vec3, random::RandomSource};
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

// Sulfur cube state (on top of `cube`'s `size`, `counter` jump delay,
// `flag` aggressive, `flag2` was on ground, `vector[0]` direction):
// `head` the swallowed block, `counter2` the fuse plus one (0 unprimed),
// `ints[3]` the pickup timer, `scratch[1]` a fuse just lit (sound pending),
// `scratch[2]` the push sound cooldown, `scratch[3]` from a bucket,
// `point` the item it is after.
pub const PICKUP_TIMER: i32 = 100;

/// One `SulfurCubeArchetype` (the parts Horizon's physics can use).
#[derive(Clone, Copy, Debug)]
pub struct Archetype {
    pub name: &'static str,
    pub buoyant: bool,
    /// `explosion`: fuse ticks, power, causes fire.
    pub explosion: Option<(i32, f32, bool)>,
    /// `contact_damage` amount (`sulfur_cube_hot`).
    pub contact_damage: Option<f32>,
    /// The knockback resistance modifier (the attribute clamps to 0..1).
    pub knockback_resistance: f64,
    /// `Attributes.BOUNCINESS` (the modifier added to a base of 0).
    pub bounciness: f32,
    /// `Attributes.FRICTION_MODIFIER` and `AIR_DRAG_MODIFIER` (base 1 times
    /// the archetype's factor).
    pub friction_modifier: f32,
    pub air_drag_modifier: f32,
    pub horizontal_power: f32,
    pub vertical_power: f32,
    pub hit_sound: &'static str,
    pub push_sound: &'static str,
    pub push_cooldown: f32,
    pub push_threshold: f32,
}
macro_rules! arch {
    ($n:literal, $b:expr, $e:expr, $c:expr, $k:expr, $phys:expr, $h:expr, $v:expr, $cd:expr, $t:expr) => {
        Archetype {
            name: $n,
            buoyant: $b,
            explosion: $e,
            contact_damage: $c,
            knockback_resistance: $k,
            bounciness: $phys.0,
            friction_modifier: $phys.1,
            air_drag_modifier: $phys.2,
            horizontal_power: $h,
            vertical_power: $v,
            hit_sound: concat!("entity.sulfur_cube.", $n, ".hit"),
            push_sound: concat!("entity.sulfur_cube.", $n, ".push"),
            push_cooldown: $cd,
            push_threshold: $t,
        }
    };
}
/// `data/minecraft/sulfur_cube_archetype/*.json`.
pub const ARCHETYPES: [Archetype; 12] = [
    arch!(
        "bouncy",
        true,
        None,
        None,
        -2.0,
        (0.9, 0.3, 0.01),
        0.4125,
        0.105,
        0.7,
        0.3
    ),
    arch!(
        "explosive",
        true,
        Some((120, 3.0, false)),
        None,
        -1.0,
        (0.5, 0.3, 0.3),
        0.4125,
        0.09,
        0.7,
        0.1
    ),
    arch!(
        "fast_flat",
        false,
        None,
        None,
        -1.0,
        (0.5, 0.2, 0.01),
        0.9125,
        0.09,
        0.9,
        0.03
    ),
    arch!(
        "fast_sliding",
        false,
        None,
        None,
        0.5,
        (0.1, 0.05, 0.01),
        0.6625,
        0.09,
        1.0,
        0.05
    ),
    arch!(
        "high_resistance",
        false,
        None,
        None,
        0.7,
        (0.2, 1.0, 0.01),
        0.4125,
        0.09,
        0.7,
        0.03
    ),
    arch!(
        "hot",
        true,
        None,
        Some(1.0),
        -1.0,
        (0.5, 0.3, 0.1),
        0.4125,
        0.09,
        0.7,
        0.2
    ),
    arch!(
        "light",
        true,
        None,
        None,
        -1.0,
        (1.0, 0.3, 1.8),
        0.4125,
        0.18,
        0.7,
        0.2
    ),
    arch!(
        "regular",
        true,
        None,
        None,
        -1.0,
        (0.5, 0.3, 0.1),
        0.4125,
        0.09,
        0.5,
        0.2
    ),
    arch!(
        "slow_bouncy",
        false,
        None,
        None,
        0.4,
        (0.6, 0.3, 0.05),
        0.4125,
        0.24,
        0.5,
        0.05
    ),
    arch!(
        "slow_flat",
        false,
        None,
        None,
        0.5,
        (0.4, 0.4, 0.1),
        0.4125,
        0.105,
        0.9,
        0.03
    ),
    arch!(
        "slow_sliding",
        false,
        None,
        None,
        0.8,
        (0.1, 0.05, 0.01),
        0.4125,
        0.09,
        1.0,
        0.02
    ),
    arch!(
        "sticky",
        false,
        None,
        None,
        -2.0,
        (0.0, 2.0, 0.01),
        0.4125,
        0.09,
        0.5,
        0.05
    ),
];
/// `SulfurCubeArchetype.DEFAULT_*`: no block swallowed.
const DEFAULT_PUSH: (&str, f32, f32) = ("entity.sulfur_cube.regular.push", 0.5, 0.2);
static ITEMS: OnceLock<Vec<HashSet<String>>> = OnceLock::new();
/// The items of each archetype's `#sulfur_cube_archetype/<name>` tag.
pub fn load_archetype_items(tags: &HashMap<String, HashSet<String>>) {
    let _ = ITEMS.set(
        ARCHETYPES
            .iter()
            .map(|a| {
                tags.get(&format!("minecraft:sulfur_cube_archetype/{}", a.name))
                    .cloned()
                    .unwrap_or_default()
            })
            .collect(),
    );
}
/// `matchingArchetypes`: the last one wins (as `collectEquipmentChanges`
/// leaves it), buoyancy if any.
pub fn archetype_of(item: &str) -> Option<&'static Archetype> {
    let items = ITEMS.get()?;
    ARCHETYPES
        .iter()
        .zip(items)
        .rev()
        .find(|(_, set)| set.contains(item))
        .map(|(a, _)| a)
}
/// `isSwallowableItem`: the `sulfur_cube_swallowable` tag.
pub fn swallowable(m: &Mob, item: &str) -> bool {
    m.kind.tempt.iter().any(|t| t == item)
}
pub fn has_body(m: &Mob) -> bool {
    m.extra.head.is_some()
}
pub fn archetype(m: &Mob) -> Option<&'static Archetype> {
    m.extra.head.as_deref().and_then(archetype_of)
}
/// The archetype whose bounciness, friction and air drag attributes a
/// sulfur cube's physics use (no other mob has those modifiers).
pub fn physics(m: &Mob) -> Option<&'static Archetype> {
    if m.species() == super::Species::SulfurCube {
        archetype(m)
    } else {
        None
    }
}
pub fn fuse(m: &Mob) -> i32 {
    m.extra.counter2 - 1
}
pub fn primed(m: &Mob) -> bool {
    fuse(m) >= 0
}
fn can_explode(m: &Mob) -> bool {
    archetype(m).is_some_and(|a| a.explosion.is_some()) && m.alive() && !primed(m)
}
pub fn register(m: &mut Mob) {
    use super::cube;
    m.ai.goals.add(1, cube::CubeFloat);
    m.ai.goals.add(2, SulfurTempt { player: None });
    m.ai.goals.add(3, SearchForItems);
    m.ai.goals.add(4, cube::CubeRandomDirection::default());
    m.ai.goals.add(5, cube::CubeKeepJumping);
    m.ai.mov.kind = MoveKind::Custom;
    m.ai.nav.can_float = true;
    m.extra.vector[0] = f64::from(180.0 * m.yaw / std::f32::consts::PI);
    // Created at size 1 (finalizeSpawn sizes it); not made a baby here.
    set_size(m, 1, false);
    m.health = m.attrs.max_health;
}
/// `setSize`: health 4 per size, speed 0.2 + 0.1 * size; size 1 makes an
/// adult a baby.
pub fn set_size(m: &mut Mob, size: i32, update_health: bool) {
    let size = size.clamp(1, 127);
    m.extra.size = size;
    m.attrs.max_health = (4 * size) as f32;
    m.attrs.speed = f64::from(0.2f32 + 0.1f32 * size as f32);
    if update_health {
        m.health = m.attrs.max_health;
        if size == 1 && !m.is_baby() {
            m.extra.age = -24000;
        }
    }
}
/// `setSpawnSize`: babies are size 1, adults size 2.
pub fn finalize(m: &mut Mob) {
    let size = if m.is_baby() { 1 } else { 2 };
    set_size(m, size, true);
}
/// `getSplitCount`: two babies, none from a primed cube.
pub fn split_count(m: &Mob) -> i32 {
    if primed(m) { 0 } else { 2 }
}
pub fn tiny(m: &Mob) -> bool {
    m.extra.size <= 1
}
pub fn jump_sound(m: &Mob) -> &'static str {
    if tiny(m) {
        "entity.small_sulfur_cube.jump"
    } else {
        "entity.sulfur_cube.jump"
    }
}
pub fn squish_sound(m: &Mob) -> &'static str {
    if tiny(m) {
        "entity.small_sulfur_cube.squish"
    } else if has_body(m) {
        "entity.sulfur_cube.bounce"
    } else {
        "entity.sulfur_cube.squish"
    }
}
/// The hurt and death sounds by size.
pub fn sounds(m: &Mob) -> [&'static str; 3] {
    if tiny(m) {
        [
            "",
            "entity.small_sulfur_cube.hurt",
            "entity.small_sulfur_cube.death",
        ]
    } else {
        ["", "entity.sulfur_cube.hurt", "entity.sulfur_cube.death"]
    }
}
/// `primeTime`: light the fuse (short when an explosion did it).
pub fn prime(m: &mut Mob, imminent: bool) -> bool {
    let Some((fuse, _, _)) = archetype(m).and_then(|a| a.explosion) else {
        return false;
    };
    if !m.alive() || primed(m) {
        return false;
    }
    let t = if imminent {
        m.random.next_int_bound((fuse / 4).max(1)).unwrap_or(0) + fuse / 8
    } else {
        fuse
    };
    m.invulnerable_flag = true;
    m.extra.counter2 = t + 1;
    m.extra.scratch[1] = 1;
    true
}
/// `hurtServer`: fire or explosions light an explosive cube; with a block
/// it shrugs off blows, falls and explosions and is only knocked about.
pub fn accepts_damage(m: &mut Mob, source: &Damage, amount: f32) -> bool {
    if !has_body(m) {
        return true;
    }
    if can_explode(m) && !primed(m) {
        if source.is_fire() {
            prime(m, false);
        } else if source.kind == DamageKind::Explosion {
            prime(m, true);
        }
    }
    let immune = matches!(
        source.kind,
        DamageKind::PlayerAttack
            | DamageKind::MaceSmash
            | DamageKind::Spear
            | DamageKind::MobAttack
            | DamageKind::Arrow
            | DamageKind::Thrown
            | DamageKind::Explosion
            | DamageKind::Fall
            | DamageKind::Sting
            | DamageKind::Cactus
            | DamageKind::Freeze
            | DamageKind::SweetBerryBush
            | DamageKind::HotFloor
    );
    if immune {
        if let Some(from) = source.from {
            knockback(m, from, amount);
        }
        return false;
    }
    true
}
/// `SulfurCube.knockback` with a block: the archetype's powers, turned by
/// where the blow came from (the attacker taken to aim at its centre).
pub fn knockback(m: &mut Mob, from: [f64; 3], damage: f32) -> bool {
    let Some(a) = archetype(m) else {
        return false;
    };
    let (h0, v0) = (a.horizontal_power, a.vertical_power);
    let (xd, zd) = (from[0] - m.position.x, from[2] - m.position.z);
    // applyVerticalPositionAnglePowerRotation from feet to feet.
    let d = [
        m.position.x - from[0],
        m.position.y - from[1],
        m.position.z - from[2],
    ];
    let angle = (-d[1]).atan2((d[0] * d[0] + d[2] * d[2]).sqrt()) as f32;
    let rot = -angle * 0.8;
    let (s, c) = (rot.sin(), rot.cos());
    let (mut h, mut v) = (h0 * c - v0 * s, h0 * s + v0 * c);
    let hr = if h0 > 0.0 { h.abs() / h0 } else { 0.0 };
    let vr = if v0 > 0.0 { v.abs() / v0 } else { 0.0 };
    let max = hr.max(vr);
    if max > 1.0 {
        h /= max;
        v /= max;
    }
    let k = damage.sqrt();
    let resist = a.knockback_resistance.clamp(0.0, 1.0) as f32;
    h *= k * (1.0 - resist) * 0.4;
    v *= k * (1.0 - resist);
    let (h, v) = (h.clamp(-128.0, 128.0), v.clamp(-128.0, 128.0));
    let len = (xd * xd + zd * zd).sqrt().max(1.0e-4);
    m.velocity = Vec3::new(
        m.velocity.x - xd / len * f64::from(h),
        m.velocity.y + f64::from(v) * 1.2,
        m.velocity.z - zd / len * f64::from(h),
    );
    // The hit sound plays on the next tick (no event queue here).
    m.extra.scratch[1] = 2;
    true
}
/// `SulfurCube.tick` and `customServerAiStep` additions after the cube's
/// own: the fuse, the timers, a swallowed block's stillness, buoyancy and
/// facing, growing up, and players pushing it.
pub fn ai_step(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    match m.extra.scratch[1] {
        1 => events.push(MobEvent::Sound("entity.tnt.primed", m.center(), 1.0, 1.0)),
        2 => {
            if let Some(a) = archetype(m) {
                events.push(MobEvent::Sound(a.hit_sound, m.center(), 1.0, 1.0));
            }
        }
        _ => {}
    }
    m.extra.scratch[1] = 0;
    // tickFuse.
    if m.extra.counter2 > 1 {
        m.extra.counter2 -= 1;
    }
    if fuse(m) == 0
        && let Some((_, power, _)) = archetype(m).and_then(|a| a.explosion)
    {
        events.push(MobEvent::Explode(
            [m.position.x, m.position.y + 0.0625, m.position.z],
            power,
            m.species(),
        ));
        m.removed = true;
        return;
    }
    if m.extra.ints[3] > 0 {
        m.extra.ints[3] -= 1;
    }
    if m.extra.scratch[2] > 0 {
        m.extra.scratch[2] -= 1;
    }
    // ageBoundaryReached: a grown baby is size 2 (and a baby made by
    // feeding, summoning or the egg is size 1, as `getBreedOffspring` makes it).
    if !m.is_baby() && m.extra.size == 1 && m.extra.age == 0 && m.age > 1 {
        set_size(m, 2, true);
    } else if m.is_baby() && m.extra.size > 1 {
        set_size(m, 1, true);
    }
    let body = has_body(m);
    // collectEquipmentChanges: a swallowed block takes every goal away.
    if body {
        m.ai.goals.disabled |= MOVE | LOOK | JUMP;
        m.ai.speed = 0.0;
        // SulfurCubeLookControl: square to the nearest quarter turn.
        let close = (m.yaw + 45.0).rem_euclid(90.0) - 45.0;
        m.yaw -= close;
        m.head_yaw = m.yaw;
    } else {
        m.ai.goals.disabled &= !(MOVE | LOOK | JUMP);
    }
    // travelInFluid: a buoyant block bobs up.
    if body && m.is_in_liquid() && archetype(m).is_some_and(|a| a.buoyant) {
        let vibe = 0.2 * (m.age as f32 * 0.4).sin();
        let height = if m.fluids.in_water {
            m.fluids.water
        } else {
            m.fluids.lava
        };
        let immersion = height - f64::from(m.height()) * 0.2 + f64::from(vibe);
        if immersion > 0.0 {
            m.velocity.y += immersion.min(1.0) * f64::from(0.04f32);
        }
    }
    if body && let Some(p) = &w.player {
        player_push(m, p, events);
    }
}
/// `playerTouch` / `playerPush`: a cube holding a block is shoved by a
/// player walking into it (and burns them when hot).
fn player_push(m: &mut Mob, p: &super::PlayerView, events: &mut Vec<MobEvent>) {
    if !p.alive || p.spectator {
        return;
    }
    let touch = p.bounds.inflate(Vec3::new(1.0, 0.5, 1.0));
    if !m.bounds().intersects(touch) {
        return;
    }
    let (dx, dz) = (m.position.x - p.position.x, m.position.z - p.position.z);
    let horizontal = (dx * dx + dz * dz).sqrt();
    let (bottom, top) = (m.position.y, m.position.y + f64::from(m.height()));
    let player_top = p.bounds.max_position().y;
    if !(horizontal < f64::from(1.3f32) && p.position.y <= top && player_top > bottom) {
        return;
    }
    let a = archetype(m);
    let kb = (1.0 - a.map_or(0.0, |a| a.knockback_resistance.clamp(0.0, 1.0))).max(0.0);
    let len = horizontal.max(1.0e-4);
    let speed = (p.velocity.length() * 2.0 * f64::from(0.3f32)).clamp(0.0, 0.5);
    let push = Vec3::new(
        dx / len * kb * speed,
        if m.on_ground {
            kb * f64::from(0.3f32) * speed
        } else {
            0.0
        },
        dz / len * kb * speed,
    );
    let (sound, cooldown, threshold) = a.map_or(DEFAULT_PUSH, |a| {
        (a.push_sound, a.push_cooldown, a.push_threshold)
    });
    if push.length_squared() > f64::from(threshold * threshold) && m.extra.scratch[2] <= 0 {
        m.extra.scratch[2] = (cooldown * 20.0) as i32;
        events.push(MobEvent::Sound(sound, m.center(), 1.0, 1.0));
    }
    m.velocity = m.velocity.add_vector(push);
    if let Some(amount) = a.and_then(|a| a.contact_damage) {
        events.push(MobEvent::DamagePlayer {
            amount,
            from: [m.position.x, m.position.y, m.position.z],
            kind: "minecraft:sulfur_cube_hot",
        });
    }
}
/// `SulfurCubeRenderer.scale` (after the cube's size and squish, flat when
/// holding a block): the fuse swell, half size for an adult and the lift.
pub fn scale(m: &Mob, alpha: f32) -> glam::Mat4 {
    let size = m.extra.size.max(1) as f32;
    let squish = if has_body(m) {
        0.0
    } else {
        m.anim.lerp(0, alpha) / (size * 0.5 + 1.0)
    };
    let w = 1.0 / (squish + 1.0);
    let mut t = glam::Mat4::from_scale(glam::Vec3::splat(0.999))
        * glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.001, 0.0))
        * glam::Mat4::from_scale(glam::Vec3::new(w * size, 1.0 / w * size, w * size));
    let fuse = if primed(m) {
        fuse(m) as f32 - alpha + 1.0
    } else {
        0.0
    };
    if fuse < 10.0 && fuse > 0.0 {
        let g = (1.0 - fuse / 10.0).clamp(0.0, 1.0);
        let s = 1.0 + g * g * g * g * 0.3;
        t *= glam::Mat4::from_scale(glam::Vec3::splat(s));
    }
    let baby = m.is_baby();
    let (offset, down) = if baby { (1.24, 1.0) } else { (0.98, 0.5) };
    t * glam::Mat4::from_scale(glam::Vec3::splat(down))
        * glam::Mat4::from_translation(glam::Vec3::new(0.0, offset - 1.0 / 16.0, 0.0))
}
/// The outer body model and texture.
pub fn body(m: &Mob) -> (String, String) {
    if m.is_baby() {
        (
            "sulfur_cube_small#main".into(),
            "entity/sulfur_cube/sulfur_cube_outer_small".into(),
        )
    } else {
        (
            "sulfur_cube#main".into(),
            "entity/sulfur_cube/sulfur_cube_outer".into(),
        )
    }
}
/// `SulfurCubeInnerLayer`: the inner cube when no block is held.
pub fn inner(m: &Mob) -> Option<(&'static str, &'static str)> {
    if has_body(m) {
        return None;
    }
    Some(if m.is_baby() {
        (
            "sulfur_cube_small#inner",
            "entity/sulfur_cube/sulfur_cube_inner_small",
        )
    } else {
        ("sulfur_cube#inner", "entity/sulfur_cube/sulfur_cube_inner")
    })
}
/// `SulfurCubeTemptGoal` (`TemptGoal.ForNonPathfinders`): face a player
/// holding slime balls (babies) or something to swallow (adults) and hop.
#[derive(Clone, Debug)]
struct SulfurTempt {
    player: Option<[f64; 3]>,
}
impl Goal for SulfurTempt {
    goal_clone!();
    fn flags(&self) -> u8 {
        LOOK
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some(p) = &cx.w.player else { return false };
        let wants = |i: &Option<std::sync::Arc<str>>| {
            i.as_deref().is_some_and(|i| {
                if m.is_baby() {
                    m.kind.food.iter().any(|f| f == i)
                } else {
                    swallowable(m, i)
                }
            })
        };
        // Attributes.TEMPT_RANGE 8.
        let ok = p.alive
            && !p.spectator
            && (wants(&p.main_hand) || wants(&p.off_hand))
            && m.distance_sqr(p.position) <= 64.0;
        self.player = ok.then_some(p.eye);
        ok
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(p) = &cx.w.player else { return };
        if m.distance_sqr(p.position) < 1.0 {
            super::cube::set_wanted(m, 0.0);
            return;
        }
        m.turn_to(p.eye, 10.0, 10.0);
        let yaw = m.yaw;
        super::cube::set_direction(m, yaw, true);
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        super::cube::set_wanted(m, 0.0);
    }
}
/// `SulfurCubeSearchForItemsGoal`: hop toward the nearest thing it could
/// swallow (the game marks it in `point`).
#[derive(Clone, Debug)]
struct SearchForItems;
impl Goal for SearchForItems {
    goal_clone!();
    fn flags(&self) -> u8 {
        LOOK
    }
    fn can_use(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        !m.is_baby() && m.extra.ints[3] <= 0 && m.extra.point.is_some()
    }
    fn tick(&mut self, m: &mut Mob, _: &mut Cx) {
        let Some(p) = m.extra.point else { return };
        m.turn_to(p, 10.0, 10.0);
        let yaw = m.yaw;
        super::cube::set_direction(m, yaw, true);
    }
}
/// `addAdditionalSaveData`: the cube's size, the timers and the fuse.
pub fn to_nbt(m: &Mob, map: &mut horizon_world::nbt::Compound) {
    use horizon_world::nbt::Tag;
    map.insert("Size".into(), Tag::Int(m.extra.size.max(1) - 1));
    map.insert("wasOnGround".into(), Tag::Byte(i8::from(m.extra.flag2)));
    map.insert("pickup_timer".into(), Tag::Int(m.extra.ints[3]));
    map.insert(
        "from_bucket".into(),
        Tag::Byte(i8::from(m.extra.scratch[3] != 0)),
    );
    map.insert("fuse".into(), Tag::Int(fuse(m)));
    for (key, value) in [
        ("Age", Tag::Int(m.extra.age)),
        ("AgeLocked", Tag::Byte(i8::from(m.extra.age_locked))),
    ] {
        map.insert(key.into(), value);
    }
    map.entry("ForcedAge".into()).or_insert(Tag::Int(0));
}
pub fn from_nbt(m: &mut Mob, tag: &horizon_world::nbt::Tag) {
    use horizon_world::nbt::Tag;
    let int = |k: &str| match tag.get(k) {
        Some(Tag::Int(v)) => Some(*v),
        Some(Tag::Byte(v)) => Some(i32::from(*v)),
        _ => None,
    };
    m.extra.age = int("Age").unwrap_or(0);
    m.extra.age_locked = int("AgeLocked").unwrap_or(0) != 0;
    let health = m.health;
    set_size(m, int("Size").unwrap_or(0) + 1, false);
    m.health = health;
    m.extra.flag2 = int("wasOnGround").unwrap_or(0) != 0;
    m.extra.ints[3] = int("pickup_timer").unwrap_or(0);
    m.extra.scratch[3] = int("from_bucket").unwrap_or(0);
    m.extra.counter2 = int("fuse").unwrap_or(-1).max(-1) + 1;
    if primed(m) {
        m.invulnerable_flag = true;
    }
}
