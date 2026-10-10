//! The warden (`Warden`, `WardenAi`, `AngerManagement`): a brain with the
//! emerge, dig, roar, fight, investigate, sniff and idle activities, anger
//! kept per entity, vibration listening, the sonic boom, the heartbeat and
//! tendril animations, and digging back into the ground when nothing has
//! disturbed it for a minute.
use super::{
    Damage, DamageKind, Mob, MobEvent, MobWorld, Species, Who,
    anim::{Poses, State},
    goals::{Cx, Float},
    keyframes::Animation,
    path::{Path, PathType},
};
use crate::sculk::{self, Context, Info};
use horizon_core::{
    geometry::{Vec3, trig::Mth},
    random::RandomSource,
};
use std::{collections::HashMap, f32::consts::PI};

pub const EMERGE_DURATION: i64 = 134;
pub const ROAR_DURATION: i64 = 84;
const ROAR_SOUND_DELAY: i64 = 25;
const SNIFFING_DURATION: i64 = 84;
const DIGGING_DURATION: i64 = 100;
pub const DIGGING_COOLDOWN: i64 = 1200;
const MELEE_ATTACK_COOLDOWN: i64 = 18;
const SONIC_DURATION: i64 = 60;
const SONIC_SOUND_DELAY: i64 = 34;
const SPEED_IDLING: f64 = 0.5;
const SPEED_INVESTIGATING: f64 = 0.7;
const SPEED_FIGHTING: f64 = 1.2;
const DEFAULT_ANGER: i32 = 35;
const PROJECTILE_ANGER: i32 = 10;
const ON_HURT_ANGER_BOOST: i32 = 20;
const MAX_ANGER: i32 = 150;
const DARKNESS_RADIUS: f64 = 20.0;
/// Darkness lasts 260 ticks and is renewed within the last 200.
pub const DARKNESS_DURATION: i32 = 260;
pub const DARKNESS_REAPPLY: i32 = 200;

// AnimState keyframe slots.
const ROAR: usize = 0;
const SNIFF: usize = 1;
const EMERGE: usize = 2;
const DIGGING: usize = 3;
const ATTACK: usize = 4;
const SONIC_BOOM: usize = 5;
// AnimState values: tendril and heart animation (ticks, 10 at the start).
const TENDRIL: usize = 0;
const HEART: usize = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Pose {
    #[default]
    Standing,
    Emerging,
    Digging,
    Roaring,
    Sniffing,
}
impl Pose {
    fn code(self) -> i32 {
        match self {
            Self::Standing => 0,
            Self::Emerging => 1,
            Self::Digging => 2,
            Self::Roaring => 3,
            Self::Sniffing => 4,
        }
    }
    fn from_code(code: i32) -> Self {
        match code {
            1 => Self::Emerging,
            2 => Self::Digging,
            3 => Self::Roaring,
            4 => Self::Sniffing,
            _ => Self::Standing,
        }
    }
}
/// The warden's pose (kept in `Extra.counter` so it can be read while the
/// brain is being ticked).
pub fn pose(m: &Mob) -> Pose {
    Pose::from_code(m.extra.counter)
}
pub fn digging_or_emerging(m: &Mob) -> bool {
    matches!(pose(m), Pose::Digging | Pose::Emerging)
}
/// `Entity.setPose`, with `onSyncedDataUpdated` starting the keyframes.
fn set_pose(m: &mut Mob, new: Pose) {
    if pose(m) == new {
        return;
    }
    m.extra.counter = new.code();
    let age = m.age as f32;
    match new {
        Pose::Emerging => m.anim.states[EMERGE].start(age),
        Pose::Digging => m.anim.states[DIGGING].start(age),
        Pose::Roaring => m.anim.states[ROAR].start(age),
        Pose::Sniffing => m.anim.states[SNIFF].start(age),
        Pose::Standing => {}
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AngerLevel {
    Calm,
    Agitated,
    Angry,
}
impl AngerLevel {
    pub fn by_anger(anger: i32) -> Self {
        if anger >= 80 {
            Self::Angry
        } else if anger >= 40 {
            Self::Agitated
        } else {
            Self::Calm
        }
    }
    pub fn minimum(self) -> i32 {
        match self {
            Self::Calm => 0,
            Self::Agitated => 40,
            Self::Angry => 80,
        }
    }
    pub fn is_angry(self) -> bool {
        self == Self::Angry
    }
    pub fn ambient_sound(self) -> &'static str {
        match self {
            Self::Calm => "entity.warden.ambient",
            Self::Agitated => "entity.warden.agitated",
            Self::Angry => "entity.warden.angry",
        }
    }
    pub fn listening_sound(self) -> &'static str {
        match self {
            Self::Calm => "entity.warden.listening",
            _ => "entity.warden.listening_angry",
        }
    }
}

/// `AngerManagement`: anger per suspect, highest first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AngerManagement {
    suspects: Vec<Who>,
    anger: HashMap<Who, i32>,
    highest: i32,
}
impl AngerManagement {
    /// One second: every suspect calms by one; the invalid are forgotten.
    pub fn tick(&mut self, valid: impl Fn(Who) -> bool) {
        let mut dropped = Vec::new();
        for (who, anger) in self.anger.iter_mut() {
            if *anger > 1 && valid(*who) {
                *anger -= 1;
            } else {
                dropped.push(*who);
            }
        }
        for who in dropped {
            self.anger.remove(&who);
            self.suspects.retain(|s| *s != who);
        }
        self.sort_and_update_highest();
    }
    fn sort_and_update_highest(&mut self) {
        // AngerManagement.Sorter: the angry first, then players, then the
        // angriest; a stable sort keeps the order of ties.
        let anger = self.anger.clone();
        let level = |w: &Who| anger.get(w).copied().unwrap_or(0);
        self.suspects.sort_by(|a, b| {
            let (aa, ab) = (level(a), level(b));
            let (angry_a, angry_b) = (
                AngerLevel::by_anger(aa).is_angry(),
                AngerLevel::by_anger(ab).is_angry(),
            );
            if angry_a != angry_b {
                return if angry_a {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                };
            }
            let (player_a, player_b) = (*a == Who::Player, *b == Who::Player);
            if player_a != player_b {
                return if player_a {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                };
            }
            ab.cmp(&aa)
        });
        self.highest = match self.suspects.len() {
            0 => 0,
            1 => level(&self.suspects[0]),
            _ => self.suspects.iter().map(&level).max().unwrap_or(0),
        };
    }
    /// `increaseAnger`: the entity's new anger (at most 150).
    pub fn increase(&mut self, who: Who, amount: i32) -> i32 {
        let new_suspect = !self.anger.contains_key(&who);
        let current = (self.anger.get(&who).copied().unwrap_or(0) + amount).min(MAX_ANGER);
        self.anger.insert(who, current);
        if new_suspect {
            self.suspects.push(who);
        }
        self.sort_and_update_highest();
        current
    }
    pub fn clear(&mut self, who: Who) {
        self.anger.remove(&who);
        self.suspects.retain(|s| *s != who);
        self.sort_and_update_highest();
    }
    /// `getActiveAnger`: the target's anger, or the highest when there is none.
    pub fn active_anger(&self, target: Option<Who>) -> i32 {
        match target {
            None => self.highest,
            Some(t) => self.anger.get(&t).copied().unwrap_or(0),
        }
    }
    /// `getActiveEntity`: the first suspect that can still be targeted.
    pub fn active_entity(&self, can_target: impl Fn(Who) -> bool) -> Option<Who> {
        self.suspects.iter().copied().find(|w| can_target(*w))
    }
    pub fn anger_of(&self, who: Who) -> i32 {
        self.anger.get(&who).copied().unwrap_or(0)
    }
    pub fn suspects(&self) -> &[Who] {
        &self.suspects
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Activity {
    Emerge,
    Dig,
    Roar,
    Fight,
    Investigate,
    Sniff,
    Idle,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Track {
    Block([i32; 3]),
    /// An entity, tracked at its eyes when the flag is set.
    Entity(Who, bool),
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct Walk {
    target: Track,
    speed: f32,
    close_enough: i32,
}
/// `MoveToTargetSink`'s running state.
#[derive(Clone, Debug, Default)]
struct MoveSink {
    end: i64,
    last_target: [i32; 3],
    speed: f32,
    path: Option<Path>,
}
/// `RunOne` over the idle stroll and `DoNothing`.
#[derive(Clone, Copy, Debug, Default)]
struct Pause {
    end: i64,
}

/// The warden's brain and sensors.
#[derive(Clone, Debug)]
pub struct Warden {
    pub anger: AngerManagement,
    pub vibration: sculk::Data,
    activity: Activity,
    in_brain: bool,
    // Memories: the game time they stop being present (None: absent).
    dig_cooldown: Option<i64>,
    is_emerging: Option<i64>,
    sniffing: bool,
    sniff_cooldown: Option<i64>,
    roar_target: Option<Who>,
    attack_target: Option<Who>,
    disturbance: Option<([i32; 3], i64)>,
    look: Option<(Track, i64)>,
    walk: Option<Walk>,
    cant_reach_since: Option<i64>,
    vibration_cooldown: Option<i64>,
    touch_cooldown: Option<i64>,
    recent_projectile: Option<i64>,
    roar_sound_delay: Option<i64>,
    roar_sound_cooldown: Option<i64>,
    sonic_cooldown: Option<i64>,
    sonic_sound_delay: Option<i64>,
    sonic_sound_cooldown: Option<i64>,
    attack_cooling_down: Option<i64>,
    nearest_living: Vec<Who>,
    nearest_attackable: Option<Who>,
    sensor_timer: i32,
    // Running behaviours (the game time they time out).
    emerging: Option<i64>,
    digging: Option<i64>,
    roaring: Option<i64>,
    sniffing_run: Option<i64>,
    sonic: Option<i64>,
    look_sink: Option<i64>,
    move_sink: Option<MoveSink>,
    move_cooldown: i32,
    pause: Option<Pause>,
    /// `getClientAngerLevel`, synchronised once a second.
    pub client_anger: i32,
    /// Attackers noted by `hurtServer` and not yet handled by the brain.
    hurt_by: Vec<(Option<Who>, bool)>,
}
const NEVER: i64 = i64::MAX;
impl Default for Warden {
    fn default() -> Self {
        Self {
            anger: AngerManagement::default(),
            vibration: sculk::Data::default(),
            activity: Activity::Idle,
            in_brain: false,
            dig_cooldown: None,
            is_emerging: None,
            sniffing: false,
            sniff_cooldown: None,
            roar_target: None,
            attack_target: None,
            disturbance: None,
            look: None,
            walk: None,
            cant_reach_since: None,
            vibration_cooldown: None,
            touch_cooldown: None,
            recent_projectile: None,
            roar_sound_delay: None,
            roar_sound_cooldown: None,
            sonic_cooldown: None,
            sonic_sound_delay: None,
            sonic_sound_cooldown: None,
            attack_cooling_down: None,
            nearest_living: Vec::new(),
            nearest_attackable: None,
            sensor_timer: 0,
            emerging: None,
            digging: None,
            roaring: None,
            sniffing_run: None,
            sonic: None,
            look_sink: None,
            move_sink: None,
            move_cooldown: 0,
            pause: None,
            client_anger: 0,
            hurt_by: Vec::new(),
        }
    }
}
fn present(memory: Option<i64>, now: i64) -> bool {
    memory.is_some_and(|until| now < until)
}
impl Warden {
    /// `Brain.setMemoryWithExpiry`: present for `ttl` brain ticks; set
    /// outside the brain's tick it loses the one it ticks at once.
    fn expiry(&self, now: i64, ttl: i64) -> Option<i64> {
        Some(now + ttl - i64::from(!self.in_brain))
    }
    pub fn target(&self) -> Option<Who> {
        self.attack_target
    }
}

pub fn register(m: &mut Mob) {
    m.extra.warden = Some(Box::default());
    m.ai.nav.can_float = true;
    m.set_malus(PathType::UnpassableRail, 0.0);
    m.set_malus(PathType::Damaging, 8.0);
    m.set_malus(PathType::PowderSnow, 8.0);
    m.set_malus(PathType::Lava, 8.0);
    m.set_malus(PathType::Fire, 0.0);
    m.set_malus(PathType::FireInNeighbor, 0.0);
    // Swim(0.8) is part of the brain's core activity.
    m.ai.goals.add(0, Float);
    // The sensors scan on a random phase.
    let phase = m.random.next_int_bound(20).unwrap_or(0);
    if let Some(w) = m.extra.warden.as_mut() {
        w.sensor_timer = phase;
    }
}
/// `Warden.finalizeSpawn`: a minute until it digs back down; a summoned
/// warden climbs out of the ground first.
pub fn finalize(m: &mut Mob, triggered: bool, events: &mut Vec<MobEvent>) {
    let now = i64::from(m.age);
    let Some(st) = m.extra.warden.as_mut() else {
        return;
    };
    st.dig_cooldown = Some(now + DIGGING_COOLDOWN);
    if triggered {
        st.is_emerging = Some(now + EMERGE_DURATION);
        set_pose(m, Pose::Emerging);
        sound(m, "entity.warden.agitated", 5.0, 1.0, events);
    }
}
fn at(m: &Mob) -> [f64; 3] {
    [m.position.x, m.position.y, m.position.z]
}
fn sound(m: &Mob, name: &'static str, volume: f32, pitch: f32, events: &mut Vec<MobEvent>) {
    events.push(MobEvent::Sound(name, at(m), volume, pitch));
}
/// `LivingEntity.getVoicePitch` for an adult.
fn voice_pitch(m: &mut Mob) -> f32 {
    (m.random.next_float() - m.random.next_float()) * 0.2 + 1.0
}
fn closer_than(m: &Mob, p: Vec3, horizontal: f64, vertical: f64) -> bool {
    let (dx, dy, dz) = (p.x - m.position.x, p.y - m.position.y, p.z - m.position.z);
    dx * dx + dz * dz < horizontal * horizontal && dy.abs() < vertical
}

// ---- entity facts ----

/// `Warden.canTargetEntity`.
pub fn can_target(w: &MobWorld, who: Who) -> bool {
    match who {
        Who::Player => w.player.as_ref().is_some_and(|p| p.alive && !p.creative),
        Who::Mob(id) => w
            .seen(id)
            .is_some_and(|s| s.alive && s.species != Species::Warden),
    }
}
fn position_of(w: &MobWorld, who: Who) -> Option<Vec3> {
    w.target_info(who).map(|t| t.position)
}
fn track_position(w: &MobWorld, track: Track) -> Option<[f64; 3]> {
    match track {
        Track::Block(p) => Some([
            f64::from(p[0]) + 0.5,
            f64::from(p[1]) + 0.5,
            f64::from(p[2]) + 0.5,
        ]),
        Track::Entity(who, eyes) => w.target_info(who).map(|t| {
            [
                t.position.x,
                if eyes { t.eye_y } else { t.position.y },
                t.position.z,
            ]
        }),
    }
}
fn track_block(w: &MobWorld, track: Track) -> Option<[i32; 3]> {
    match track {
        Track::Block(p) => Some(p),
        Track::Entity(who, _) => w.target_info(who).map(|t| {
            [
                t.position.x.floor() as i32,
                t.position.y.floor() as i32,
                t.position.z.floor() as i32,
            ]
        }),
    }
}
fn block_pos(m: &Mob) -> [i32; 3] {
    [
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    ]
}
fn manhattan(a: [i32; 3], b: [i32; 3]) -> i32 {
    (a[0] - b[0]).abs() + (a[1] - b[1]).abs() + (a[2] - b[2]).abs()
}
fn dist_sqr(a: [i32; 3], b: [i32; 3]) -> f64 {
    (0..3).map(|i| f64::from(a[i] - b[i]).powi(2)).sum()
}

// ---- anger ----

/// `Warden.increaseAngerAt`.
fn increase_anger(
    m: &mut Mob,
    st: &mut Warden,
    w: &MobWorld,
    who: Option<Who>,
    amount: i32,
    play_sound: bool,
    events: &mut Vec<MobEvent>,
) {
    let Some(who) = who else { return };
    if !can_target(w, who) {
        return;
    }
    set_dig_cooldown(st, i64::from(m.age));
    let maybe_switch = st.attack_target != Some(Who::Player);
    let new = st.anger.increase(who, amount);
    if who == Who::Player && maybe_switch && AngerLevel::by_anger(new).is_angry() {
        st.attack_target = None;
    }
    if play_sound && pose(m) != Pose::Roaring {
        let level = angry_level(st);
        let pitch = voice_pitch(m);
        sound(m, level.listening_sound(), 10.0, pitch, events);
    }
}
fn angry_level(st: &Warden) -> AngerLevel {
    AngerLevel::by_anger(st.anger.active_anger(st.attack_target))
}
/// `Warden.getEntityAngryAt`.
fn entity_angry_at(st: &Warden, w: &MobWorld) -> Option<Who> {
    if angry_level(st).is_angry() {
        st.anger.active_entity(|e| can_target(w, e))
    } else {
        None
    }
}
/// `Warden.getAngerLevel` of a warden (for its ambient sound).
pub fn anger_level(m: &Mob) -> AngerLevel {
    m.extra.warden.as_ref().map_or(AngerLevel::Calm, |s| {
        AngerLevel::by_anger(s.anger.active_anger(s.attack_target))
    })
}
/// `WardenAi.setDigCooldown`.
fn set_dig_cooldown(st: &mut Warden, now: i64) {
    if present(st.dig_cooldown, now) {
        st.dig_cooldown = st.expiry(now, DIGGING_COOLDOWN);
    }
}
/// `WardenAi.setDisturbanceLocation`.
fn set_disturbance(st: &mut Warden, w: &MobWorld, now: i64, position: [i32; 3]) {
    if entity_angry_at(st, w).is_none() && st.attack_target.is_none() {
        set_dig_cooldown(st, now);
        st.sniff_cooldown = st.expiry(now, 100);
        let until = st.expiry(now, 100).unwrap_or(now);
        st.look = Some((Track::Block(position), until));
        st.disturbance = Some((position, until));
        st.walk = None;
    }
}
/// `Warden.setAttackTarget`.
fn set_attack_target(st: &mut Warden, target: Who, now: i64) {
    st.roar_target = None;
    st.attack_target = Some(target);
    st.cant_reach_since = None;
    st.sonic_cooldown = st.expiry(now, 200);
}

// ---- damage ----

/// `Warden.isInvulnerableTo`: while digging or emerging.
pub fn accepts_damage(m: &Mob, source: &Damage) -> bool {
    !(digging_or_emerging(m) && source.kind != DamageKind::Generic)
}
/// `Warden.hurtServer`'s reaction, handled by the brain's next tick.
pub fn hurt(m: &mut Mob, source: &Damage) {
    if digging_or_emerging(m) {
        return;
    }
    let direct = !matches!(
        source.kind,
        DamageKind::Arrow | DamageKind::Thrown | DamageKind::Fireball
    );
    if let Some(st) = m.extra.warden.as_mut() {
        st.hurt_by.push((source.attacker, direct));
    }
}

// ---- vibrations ----

/// `Warden.VibrationUser.canReceiveVibration`.
pub fn can_receive_vibration(m: &Mob, now: i64, ctx: &Context) -> bool {
    let Some(st) = m.extra.warden.as_ref() else {
        return false;
    };
    m.alive()
        && !present(st.vibration_cooldown, now)
        && !digging_or_emerging(m)
        && !(ctx.living && !ctx.targetable)
}
/// `Warden.VibrationUser.onReceiveVibration`.
fn receive_vibration(
    m: &mut Mob,
    st: &mut Warden,
    w: &MobWorld,
    info: Info,
    events: &mut Vec<MobEvent>,
) {
    if !m.alive() {
        return;
    }
    let now = i64::from(m.age);
    st.vibration_cooldown = st.expiry(now, 40);
    // Entity event 61: the tendrils twitch.
    m.anim.v[TENDRIL] = 10.0;
    let pitch = voice_pitch(m);
    sound(m, "entity.warden.tendril_clicks", 5.0, pitch, events);
    let origin = [
        info.pos[0].floor() as i32,
        info.pos[1].floor() as i32,
        info.pos[2].floor() as i32,
    ];
    let mut suspicious = origin;
    if let Some(owner) = info.owner {
        if let Some(p) = position_of(w, owner)
            && closer_than_distance(m, p, 30.0)
        {
            if present(st.recent_projectile, now) {
                if can_target(w, owner)
                    && let Some(t) = w.target_info(owner)
                {
                    suspicious = [
                        t.position.x.floor() as i32,
                        t.position.y.floor() as i32,
                        t.position.z.floor() as i32,
                    ];
                }
                increase_anger(m, st, w, Some(owner), DEFAULT_ANGER, true, events);
            } else {
                increase_anger(m, st, w, Some(owner), PROJECTILE_ANGER, true, events);
            }
        }
        st.recent_projectile = st.expiry(now, 100);
    } else {
        increase_anger(m, st, w, info.source, DEFAULT_ANGER, true, events);
    }
    if !angry_level(st).is_angry() {
        let active = st.anger.active_entity(|e| can_target(w, e));
        if info.owner.is_some() || active.is_none() || active == info.source {
            set_disturbance(st, w, now, suspicious);
        }
    }
}
fn closer_than_distance(m: &Mob, p: Vec3, distance: f64) -> bool {
    m.distance_sqr(p) < distance * distance
}

// ---- the brain ----

/// `Warden.customServerAiStep`: the vibration ticker, the brain, darkness,
/// anger and the activity.
pub fn server_ai_step(m: &mut Mob, cx: &mut Cx) {
    let Some(mut st) = m.extra.warden.take() else {
        return;
    };
    let now = i64::from(m.age);
    // Warden.tick: vibrations arrive before the brain thinks.
    st.in_brain = false;
    if st.dig_cooldown.is_some() && m.custom_name.is_some() {
        set_dig_cooldown(&mut st, now);
    }
    let tick = st
        .vibration
        .tick(cx.w.game_time, true, false, |d| d.floor() as i32);
    if let Some((info, travel)) = tick.started {
        let eye = [m.position.x, m.position.y + m.kind.eye_height, m.position.z];
        cx.events.push(MobEvent::Vibration {
            from: info.pos,
            to: eye,
            ticks: travel,
        });
    }
    if let Some(info) = tick.received {
        receive_vibration(m, &mut st, cx.w, info, cx.events);
    }
    touch_entities(m, &mut st, cx);
    handle_hurt(m, &mut st, cx);
    st.in_brain = true;
    brain_tick(m, &mut st, cx);
    st.in_brain = false;
    // Warden.customServerAiStep after the brain.
    if (i64::from(m.age) + i64::from(m.id)) % 120 == 0 {
        cx.events.push(MobEvent::Darkness {
            at: [m.position.x, m.position.y, m.position.z],
            radius: DARKNESS_RADIUS,
        });
    }
    if m.age.is_multiple_of(20) {
        let w = cx.w;
        st.anger.tick(|who| can_target(w, who));
        st.client_anger = st.anger.active_anger(st.attack_target);
    }
    update_activity(m, &mut st, now);
    m.extra.warden = Some(st);
}
/// `Warden.doPush`: touching something angers it (every second).
fn touch_entities(m: &mut Mob, st: &mut Warden, cx: &mut Cx) {
    if digging_or_emerging(m) || !m.alive() {
        return;
    }
    let now = i64::from(m.age);
    if present(st.touch_cooldown, now) {
        return;
    }
    let bounds = m.bounds();
    let mut touched = None;
    if let Some(p) = cx.w.player.as_ref()
        && p.alive
        && !p.spectator
        && bounds.intersects(p.bounds)
    {
        touched = Some(Who::Player);
    }
    if touched.is_none() {
        touched =
            cx.w.others
                .iter()
                .find(|s| s.id != m.id && s.alive && bounds.intersects(s.bounds))
                .map(|s| Who::Mob(s.id));
    }
    if let Some(who) = touched {
        st.touch_cooldown = st.expiry(now, 20);
        increase_anger(m, st, cx.w, Some(who), DEFAULT_ANGER, true, cx.events);
        if let Some(t) = cx.w.target_info(who) {
            let block = [
                t.position.x.floor() as i32,
                t.position.y.floor() as i32,
                t.position.z.floor() as i32,
            ];
            set_disturbance(st, cx.w, now, block);
        }
    }
}
/// The rest of `Warden.hurtServer`.
fn handle_hurt(m: &mut Mob, st: &mut Warden, cx: &mut Cx) {
    for (attacker, direct) in std::mem::take(&mut st.hurt_by) {
        increase_anger(
            m,
            st,
            cx.w,
            attacker,
            AngerLevel::Angry.minimum() + ON_HURT_ANGER_BOOST,
            false,
            cx.events,
        );
        if st.attack_target.is_none()
            && let Some(who) = attacker
            && let Some(p) = position_of(cx.w, who)
            && (direct || closer_than_distance(m, p, 5.0))
        {
            set_attack_target(st, who, i64::from(m.age));
        }
    }
}
/// `Brain.setActiveActivityToFirstValid`, erasing the memories of the
/// activity being left.
fn update_activity(m: &mut Mob, st: &mut Warden, now: i64) {
    let _ = m;
    let next = if present(st.is_emerging, now) {
        Activity::Emerge
    } else if st.roar_target.is_none() && !present(st.dig_cooldown, now) {
        Activity::Dig
    } else if st.roar_target.is_some() {
        Activity::Roar
    } else if st.attack_target.is_some() {
        Activity::Fight
    } else if st.disturbance.is_some_and(|(_, until)| now < until) {
        Activity::Investigate
    } else if st.sniffing {
        Activity::Sniff
    } else {
        Activity::Idle
    };
    if next != st.activity {
        match st.activity {
            Activity::Emerge => st.is_emerging = None,
            Activity::Roar => st.roar_target = None,
            Activity::Fight => st.attack_target = None,
            Activity::Investigate => st.disturbance = None,
            Activity::Sniff => st.sniffing = false,
            Activity::Dig | Activity::Idle => {}
        }
        st.activity = next;
    }
}
fn active(st: &Warden, activity: Activity) -> bool {
    st.activity == activity
}
fn brain_tick(m: &mut Mob, st: &mut Warden, cx: &mut Cx) {
    let now = i64::from(m.age);
    // Memories that timed out are forgotten.
    if st.disturbance.is_some_and(|(_, until)| now >= until) {
        st.disturbance = None;
    }
    if st.look.is_some_and(|(_, until)| now >= until) {
        st.look = None;
    }
    sense(m, st, cx.w);
    start_behaviours(m, st, cx);
    tick_behaviours(m, st, cx);
}
/// `WardenEntitySensor` (and `NearestLivingEntitySensor`) every 20 ticks.
fn sense(m: &mut Mob, st: &mut Warden, w: &MobWorld) {
    st.sensor_timer -= 1;
    if st.sensor_timer > 0 {
        return;
    }
    st.sensor_timer = 20;
    let range = m.attrs.follow_range;
    let area = m
        .bounds()
        .inflate(horizon_core::geometry::Vec3::new(range, range, range));
    let mut found: Vec<(Who, f64)> = Vec::new();
    if let Some(p) = w.player.as_ref()
        && p.alive
        && area.intersects(p.bounds)
    {
        found.push((Who::Player, m.distance_sqr(p.position)));
    }
    for s in w.others {
        if s.id != m.id && s.alive && area.intersects(s.bounds) {
            found.push((Who::Mob(s.id), m.distance_sqr(s.position)));
        }
    }
    found.sort_by(|a, b| a.1.total_cmp(&b.1));
    st.nearest_living = found.into_iter().map(|(who, _)| who).collect();
    let closest = |players: bool| {
        st.nearest_living
            .iter()
            .copied()
            .find(|who| can_target(w, *who) && (*who == Who::Player) == players)
    };
    let attackable = closest(true).or_else(|| closest(false));
    st.nearest_attackable = attackable;
}
/// `NearestVisibleLivingEntities.contains`.
fn visible(m: &mut Mob, st: &Warden, w: &MobWorld, who: Who) -> bool {
    st.nearest_living.contains(&who) && m.sees(w, who)
}

fn start_behaviours(m: &mut Mob, st: &mut Warden, cx: &mut Cx) {
    let now = i64::from(m.age);
    // Core: SetWardenLookTarget.
    if st.attack_target.is_none() {
        let target = st
            .roar_target
            .and_then(|who| track_block(cx.w, Track::Entity(who, true)))
            .or(st.disturbance.map(|(p, _)| p));
        if let Some(p) = target {
            st.look = Some((Track::Block(p), NEVER));
        }
    }
    // Dig: Digging.
    if active(st, Activity::Dig)
        && st.digging.is_none()
        && st.attack_target.is_none()
        && st.walk.is_none()
        && (m.on_ground || m.is_in_liquid())
    {
        st.digging = Some(now + DIGGING_DURATION);
        if m.on_ground {
            set_pose(m, Pose::Digging);
            sound(m, "entity.warden.dig", 5.0, 1.0, cx.events);
        } else {
            sound(m, "entity.warden.agitated", 5.0, 1.0, cx.events);
            stop_digging(m, st);
        }
    }
    // Core: LookAtTargetSink.
    if st.look_sink.is_none() && st.look.is_some() {
        let duration = 45 + i64::from(m.random.next_int_bound(46).unwrap_or(0));
        st.look_sink = Some(now + duration);
    }
    // Core: MoveToTargetSink.
    if st.move_sink.is_none() && st.walk.is_some() {
        start_move_sink(m, st, cx);
    }
    // Emerge: Emerging.
    if active(st, Activity::Emerge)
        && st.emerging.is_none()
        && present(st.is_emerging, now)
        && st.walk.is_none()
    {
        st.emerging = Some(now + EMERGE_DURATION);
        set_pose(m, Pose::Emerging);
        sound(m, "entity.warden.emerge", 5.0, 1.0, cx.events);
    }
    // SetRoarTarget (investigate, sniff and idle).
    if matches!(
        st.activity,
        Activity::Investigate | Activity::Sniff | Activity::Idle
    ) && st.roar_target.is_none()
        && st.attack_target.is_none()
        && let Some(target) = entity_angry_at(st, cx.w).filter(|t| can_target(cx.w, *t))
    {
        st.roar_target = Some(target);
        st.cant_reach_since = None;
    }
    // Investigate: GoToTargetLocation.
    if active(st, Activity::Investigate)
        && st.attack_target.is_none()
        && st.walk.is_none()
        && let Some((location, until)) = st.disturbance
        && now < until
        && dist_sqr(location, block_pos(m)) >= 4.0
    {
        let ox = m.random.next_int_bound(3).unwrap_or(1) - 1;
        let oz = m.random.next_int_bound(3).unwrap_or(1) - 1;
        let target = [location[0] + ox, location[1], location[2] + oz];
        st.look = Some((Track::Block(target), NEVER));
        st.walk = Some(Walk {
            target: Track::Block(target),
            speed: SPEED_INVESTIGATING as f32,
            close_enough: 2,
        });
    }
    // Sniff: Sniffing.
    if active(st, Activity::Sniff)
        && st.sniffing_run.is_none()
        && st.sniffing
        && st.attack_target.is_none()
        && st.walk.is_none()
    {
        st.sniffing_run = Some(now + SNIFFING_DURATION);
        sound(m, "entity.warden.sniff", 5.0, 1.0, cx.events);
    }
    // Roar.
    if active(st, Activity::Roar)
        && st.roaring.is_none()
        && let Some(target) = st.roar_target
        && st.attack_target.is_none()
    {
        st.roaring = Some(now + ROAR_DURATION);
        st.roar_sound_delay = st.expiry(now, ROAR_SOUND_DELAY);
        st.walk = None;
        st.look = Some((Track::Entity(target, true), NEVER));
        set_pose(m, Pose::Roaring);
        increase_anger(m, st, cx.w, Some(target), 20, false, cx.events);
    }
    // Fight: the dig cooldown setter.
    if active(st, Activity::Fight) {
        set_dig_cooldown(st, now);
    }
    // Idle: TryToSniff.
    if active(st, Activity::Idle)
        && !present(st.sniff_cooldown, now)
        && st.nearest_attackable.is_some()
        && st.disturbance.is_none()
    {
        st.sniffing = true;
        let cooldown = 100 + i64::from(m.random.next_int_bound(101).unwrap_or(0));
        st.sniff_cooldown = st.expiry(now, cooldown);
        st.walk = None;
        set_pose(m, Pose::Sniffing);
    }
    // Fight: StopAttackingIfTargetInvalid.
    if active(st, Activity::Fight)
        && let Some(target) = st.attack_target
    {
        let tired = st.cant_reach_since.is_some_and(|since| now - since > 200);
        let valid = can_target(cx.w, target)
            && !tired
            && cx.w.target_info(target).is_some_and(|t| t.alive)
            && angry_level(st).is_angry();
        if !valid {
            if !can_target(cx.w, target) {
                st.anger.clear(target);
            }
            set_dig_cooldown(st, now);
            st.attack_target = None;
        }
    }
    // Idle: RunOne over the stroll and DoNothing.
    if active(st, Activity::Idle) && st.pause.is_none() && !st.sniffing {
        run_one(m, st, cx);
    }
    // Fight: SetEntityLookTarget.
    if active(st, Activity::Fight)
        && st.look.is_none()
        && let Some(target) = st.attack_target
        && visible(m, st, cx.w, target)
        && position_of(cx.w, target)
            .is_some_and(|p| m.distance_sqr(p) <= m.attrs.follow_range * m.attrs.follow_range)
    {
        st.look = Some((Track::Entity(target, true), NEVER));
    }
    // Fight: SetWalkTargetFromAttackTargetIfTargetOutOfReach.
    if active(st, Activity::Fight)
        && let Some(target) = st.attack_target
    {
        let in_reach = visible(m, st, cx.w, target)
            && cx
                .w
                .target_info(target)
                .is_some_and(|t| m.within_melee_range(&t));
        if in_reach {
            st.walk = None;
        } else {
            st.look = Some((Track::Entity(target, true), NEVER));
            st.walk = Some(Walk {
                target: Track::Entity(target, false),
                speed: SPEED_FIGHTING as f32,
                close_enough: 0,
            });
        }
    }
    // Fight: SonicBoom.
    if active(st, Activity::Fight)
        && st.sonic.is_none()
        && !present(st.sonic_cooldown, now)
        && let Some(target) = st.attack_target
        && position_of(cx.w, target).is_some_and(|p| closer_than(m, p, 15.0, 20.0))
    {
        st.sonic = Some(now + SONIC_DURATION);
        st.attack_cooling_down = st.expiry(now, SONIC_DURATION);
        st.sonic_sound_delay = st.expiry(now, SONIC_SOUND_DELAY);
        m.anim.states[SONIC_BOOM].start(m.age as f32);
        sound(m, "entity.warden.sonic_charge", 3.0, 1.0, cx.events);
    }
    // Fight: MeleeAttack.
    if active(st, Activity::Fight)
        && !present(st.attack_cooling_down, now)
        && let Some(target) = st.attack_target
        && visible(m, st, cx.w, target)
        && let Some(info) = cx.w.target_info(target)
        && m.within_melee_range(&info)
    {
        st.look = Some((Track::Entity(target, true), NEVER));
        m.swing();
        attack(m, target, cx);
        st.attack_cooling_down = st.expiry(now, MELEE_ATTACK_COOLDOWN);
        // SonicBoom.setCooldown(this, 40) from doHurtTarget.
        st.sonic_cooldown = st.expiry(now, 40);
        m.anim.states[ROAR].stop();
        m.anim.states[ATTACK].start(m.age as f32);
    }
}
/// `Warden.doHurtTarget`: the hit, its impact sound and the knockback.
fn attack(m: &mut Mob, target: Who, cx: &mut Cx) {
    let pitch = voice_pitch(m);
    sound(m, "entity.warden.attack_impact", 10.0, pitch, cx.events);
    super::ai::do_hurt_target(m, target, cx);
    // ATTACK_KNOCKBACK 1.5: half of it, along the warden's facing.
    let yaw = m.yaw.to_radians();
    cx.events.push(MobEvent::Push {
        target,
        velocity: [
            -f64::from(yaw.sin()) * 0.75,
            0.0,
            f64::from(yaw.cos()) * 0.75,
        ],
    });
    m.velocity.x *= 0.6;
    m.velocity.z *= 0.6;
}
/// `RunOne`: a stroll (weight 2) or a pause (weight 1), picked by shuffling.
fn run_one(m: &mut Mob, st: &mut Warden, cx: &mut Cx) {
    let now = i64::from(m.age);
    // ShufflingList.shuffle: each entry sorts by -ln(random) / weight.
    let stroll_key = -f64::from(m.random.next_float()).ln() / 2.0;
    let pause_key = -f64::from(m.random.next_float()).ln();
    let pause_first = pause_key < stroll_key;
    let order = if pause_first {
        [false, true]
    } else {
        [true, false]
    };
    for stroll in order {
        if stroll {
            // RandomStroll.stroll(0.5): only when there is no walk target.
            if st.walk.is_none() {
                st.walk = m.land_random_pos(cx.w, 10, 7).map(|p| Walk {
                    target: Track::Block([
                        p[0].floor() as i32,
                        p[1].floor() as i32,
                        p[2].floor() as i32,
                    ]),
                    speed: SPEED_IDLING as f32,
                    close_enough: 0,
                });
                return;
            }
        } else {
            let duration = 30 + i64::from(m.random.next_int_bound(31).unwrap_or(0));
            st.pause = Some(Pause {
                end: now + duration,
            });
            return;
        }
    }
}
fn stop_digging(m: &mut Mob, st: &mut Warden) {
    st.digging = None;
    // Digging.stop: the warden is removed.
    m.removed = true;
}

fn tick_behaviours(m: &mut Mob, st: &mut Warden, cx: &mut Cx) {
    let now = i64::from(m.age);
    // LookAtTargetSink.
    if let Some(end) = st.look_sink {
        let keep = now <= end
            && st.look.is_some_and(|(track, _)| match track {
                Track::Block(_) => true,
                Track::Entity(who, _) => visible(m, st, cx.w, who),
            });
        if keep {
            if let Some((track, _)) = st.look
                && let Some(p) = track_position(cx.w, track)
            {
                m.look_at(p);
            }
        } else {
            st.look_sink = None;
            st.look = None;
        }
    }
    // MoveToTargetSink.
    tick_move_sink(m, st, cx);
    // Emerging.
    if let Some(end) = st.emerging
        && now > end
    {
        st.emerging = None;
        if pose(m) == Pose::Emerging {
            set_pose(m, Pose::Standing);
        }
    }
    // Digging: removed when the time is up.
    if let Some(end) = st.digging
        && now > end
    {
        stop_digging(m, st);
    }
    // Sniffing.
    if let Some(end) = st.sniffing_run
        && now > end
    {
        st.sniffing_run = None;
        if pose(m) == Pose::Sniffing {
            set_pose(m, Pose::Standing);
        }
        st.sniffing = false;
        if let Some(entity) = st.nearest_attackable.filter(|e| can_target(cx.w, *e))
            && let Some(p) = position_of(cx.w, entity)
        {
            if sniff_close(m, p) {
                increase_anger(m, st, cx.w, Some(entity), DEFAULT_ANGER, true, cx.events);
            }
            if st.disturbance.is_none() {
                let block = [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32];
                set_disturbance(st, cx.w, now, block);
            }
        }
    }
    // Roar.
    if let Some(end) = st.roaring {
        if now > end {
            st.roaring = None;
            if pose(m) == Pose::Roaring {
                set_pose(m, Pose::Standing);
            }
            if let Some(target) = st.roar_target.take() {
                set_attack_target(st, target, now);
            }
        } else if !present(st.roar_sound_delay, now) && !present(st.roar_sound_cooldown, now) {
            st.roar_sound_cooldown = st.expiry(now, ROAR_DURATION - ROAR_SOUND_DELAY);
            sound(m, "entity.warden.roar", 3.0, 1.0, cx.events);
        }
    }
    // Sonic boom.
    if let Some(end) = st.sonic {
        if now > end {
            st.sonic = None;
            st.sonic_cooldown = st.expiry(now, 40);
        } else {
            tick_sonic_boom(m, st, cx);
        }
    }
    // The idle pause.
    if let Some(pause) = st.pause
        && now > pause.end
    {
        st.pause = None;
    }
}
/// `Warden.closerThan(entity, 6, 20)` for the sniff.
fn sniff_close(m: &Mob, p: Vec3) -> bool {
    closer_than(m, p, 6.0, 20.0)
}
fn tick_sonic_boom(m: &mut Mob, st: &mut Warden, cx: &mut Cx) {
    let now = i64::from(m.age);
    let Some(target) = st.attack_target else {
        return;
    };
    if let Some(t) = cx.w.target_info(target) {
        m.look_at([t.position.x, t.position.y, t.position.z]);
    }
    if present(st.sonic_sound_delay, now) || present(st.sonic_sound_cooldown, now) {
        return;
    }
    st.sonic_sound_cooldown = st.expiry(now, SONIC_DURATION - SONIC_SOUND_DELAY);
    if !can_target(cx.w, target) {
        return;
    }
    let Some(t) = cx.w.target_info(target) else {
        return;
    };
    if !closer_than(m, t.position, 15.0, 20.0) {
        return;
    }
    // The chest attachment: 1.6 up and 0.0 forward.
    let source = [m.position.x, m.position.y + 1.6, m.position.z];
    let eye = [t.position.x, t.eye_y, t.position.z];
    let delta = [eye[0] - source[0], eye[1] - source[1], eye[2] - source[2]];
    let length = (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt();
    if length < 1.0e-9 {
        return;
    }
    let normal = [delta[0] / length, delta[1] / length, delta[2] / length];
    let steps = length.floor() as i32 + 7;
    for i in 1..steps {
        let d = f64::from(i);
        cx.events.push(MobEvent::Particles {
            kind: "sonic_boom",
            at: [
                source[0] + normal[0] * d,
                source[1] + normal[1] * d,
                source[2] + normal[2] * d,
            ],
            count: 0,
        });
    }
    sound(m, "entity.warden.sonic_boom", 3.0, 1.0, cx.events);
    let from = at(m);
    // Players and the mobs hit here have no knockback resistance.
    let resistance = 0.0;
    match target {
        Who::Player => cx.events.push(MobEvent::DamagePlayer {
            amount: 10.0,
            from,
            kind: "minecraft:sonic_boom",
        }),
        Who::Mob(id) => cx.events.push(MobEvent::AttackMob {
            target: id,
            amount: 10.0,
            source: Damage {
                kind: DamageKind::SonicBoom,
                attacker: Some(Who::Mob(m.id)),
                from: Some(from),
            },
        }),
    }
    let horizontal = 2.5 * (1.0 - resistance);
    let vertical = 0.5 * (1.0 - resistance);
    cx.events.push(MobEvent::Push {
        target,
        velocity: [
            normal[0] * horizontal,
            normal[1] * vertical,
            normal[2] * horizontal,
        ],
    });
}

// ---- walking: MoveToTargetSink ----

fn reached_target(m: &Mob, w: &MobWorld, walk: &Walk) -> bool {
    track_block(w, walk.target).is_some_and(|p| manhattan(p, block_pos(m)) <= walk.close_enough)
}
/// `MoveToTargetSink.tryComputePath`.
fn try_compute_path(m: &mut Mob, st: &mut Warden, w: &MobWorld, walk: &Walk) -> Option<Path> {
    let target = track_block(w, walk.target)?;
    let now = i64::from(m.age);
    let mut path = m.create_path_to(w, target, 0);
    if reached_target(m, w, walk) {
        st.cant_reach_since = None;
        return None;
    }
    let can_reach = path.as_ref().is_some_and(|p| p.reached);
    if can_reach {
        st.cant_reach_since = None;
    } else if st.cant_reach_since.is_none() {
        st.cant_reach_since = Some(now);
    }
    if path.is_some() {
        return path;
    }
    let bottom_center = [
        f64::from(target[0]) + 0.5,
        f64::from(target[1]),
        f64::from(target[2]) + 0.5,
    ];
    if let Some(p) = m.land_random_pos_towards(w, 10, 7, bottom_center) {
        path = m.create_path_to_entity(w, p, 0);
    }
    path
}
fn start_move_sink(m: &mut Mob, st: &mut Warden, cx: &mut Cx) {
    let now = i64::from(m.age);
    let Some(walk) = st.walk else { return };
    if st.move_sink.is_some() {
        return;
    }
    if st.move_cooldown > 0 {
        st.move_cooldown -= 1;
        return;
    }
    let reached = reached_target(m, cx.w, &walk);
    if !reached && let Some(path) = try_compute_path(m, st, cx.w, &walk) {
        let duration = 150 + i64::from(m.random.next_int_bound(101).unwrap_or(0));
        let last = track_block(cx.w, walk.target).unwrap_or_default();
        m.nav_move_to_path(Some(path.clone()), f64::from(walk.speed));
        st.move_sink = Some(MoveSink {
            end: now + duration,
            last_target: last,
            speed: walk.speed,
            path: Some(path),
        });
        return;
    }
    st.walk = None;
    if reached {
        st.cant_reach_since = None;
    }
}
fn tick_move_sink(m: &mut Mob, st: &mut Warden, cx: &mut Cx) {
    let now = i64::from(m.age);
    let Some(mut sink) = st.move_sink.take() else {
        return;
    };
    let walk = st.walk;
    let still = now <= sink.end
        && walk.is_some_and(|walk| {
            let spectator = matches!(walk.target, Track::Entity(Who::Player, _))
                && cx.w.player.as_ref().is_some_and(|p| p.spectator);
            !m.ai.nav.is_done() && !reached_target(m, cx.w, &walk) && !spectator
        });
    if !still {
        // MoveToTargetSink.stop.
        if let Some(walk) = walk
            && !reached_target(m, cx.w, &walk)
            && m.ai.nav.stuck
        {
            st.move_cooldown = m.random.next_int_bound(40).unwrap_or(0);
        }
        m.nav_stop();
        st.walk = None;
        return;
    }
    if let Some(walk) = walk
        && let Some(target) = track_block(cx.w, walk.target)
        && dist_sqr(target, sink.last_target) > 4.0
        && let Some(path) = try_compute_path(m, st, cx.w, &walk)
    {
        sink.last_target = target;
        sink.speed = walk.speed;
        m.nav_move_to_path(Some(path.clone()), f64::from(walk.speed));
        sink.path = Some(path);
    }
    st.move_sink = Some(sink);
}

// ---- per-tick client behaviour ----

/// `Warden.tick` on the client: the heartbeat, the tendrils and the digging
/// crumbs; the keyframe states follow the pose.
pub fn ai_step(m: &mut Mob, events: &mut Vec<MobEvent>) {
    let Some(st) = m.extra.warden.as_ref() else {
        return;
    };
    let client_anger = st.client_anger;
    let delay = heart_beat_delay(client_anger);
    if i64::from(m.age) % i64::from(delay) == 0 {
        m.anim.v[HEART] = 10.0;
        let pitch = voice_pitch(m);
        sound(m, "entity.warden.heartbeat", 5.0, pitch, events);
    }
    // The animation values count down after the sync of the last tick.
    for slot in [TENDRIL, HEART] {
        if m.anim.v[slot] > 0.0 {
            m.anim.v[slot] -= 1.0;
        }
    }
    let digging_clock = |m: &Mob, slot: usize| {
        m.anim.states[slot]
            .millis(m.age as f32)
            .is_some_and(|ms| (ms as f32) < 4500.0)
    };
    match pose(m) {
        Pose::Emerging if digging_clock(m, EMERGE) => {
            events.push(MobEvent::DigParticles { at: at(m) })
        }
        Pose::Digging if digging_clock(m, DIGGING) => {
            events.push(MobEvent::DigParticles { at: at(m) })
        }
        _ => {}
    }
    // Poses that ended stop their keyframes.
    let p = pose(m);
    if p != Pose::Roaring {
        m.anim.states[ROAR].stop();
    }
    if p != Pose::Sniffing {
        m.anim.states[SNIFF].stop();
    }
    if p != Pose::Emerging {
        m.anim.states[EMERGE].stop();
    }
    if p != Pose::Digging {
        m.anim.states[DIGGING].stop();
    }
    if m.anim.states[ATTACK]
        .millis(m.age as f32)
        .is_some_and(|ms| ms > 1000)
    {
        m.anim.states[ATTACK].stop();
    }
    if m.anim.states[SONIC_BOOM]
        .millis(m.age as f32)
        .is_some_and(|ms| ms > 3500)
    {
        m.anim.states[SONIC_BOOM].stop();
    }
}
/// Test aid: the minute of calm ran out.
pub fn expire_dig_cooldown(m: &mut Mob) {
    if let Some(st) = m.extra.warden.as_mut() {
        st.dig_cooldown = None;
        st.attack_target = None;
        st.roar_target = None;
    }
}
/// The warden's state for the control socket.
pub fn describe(m: &Mob) -> Option<serde_json::Value> {
    let st = m.extra.warden.as_ref()?;
    let anger: Vec<String> = st
        .anger
        .suspects()
        .iter()
        .map(|w| format!("{w:?}={}", st.anger.anger_of(*w)))
        .collect();
    Some(serde_json::json!({
        "pose": format!("{:?}", pose(m)),
        "activity": format!("{:?}", st.activity),
        "anger_level": format!("{:?}", anger_level(m)),
        "anger": anger,
        "attack_target": st.attack_target.map(|t| format!("{t:?}")),
        "roar_target": st.roar_target.map(|t| format!("{t:?}")),
        "disturbance": st.disturbance.map(|(p, _)| p),
        "walk": st.walk.is_some(),
        "dig_cooldown": st.dig_cooldown.map(|t| t - i64::from(m.age)),
        "sniff_cooldown": st.sniff_cooldown.map(|t| t - i64::from(m.age)),
        "vibration": st.vibration.current.map(|i| i.event),
    }))
}
/// `Warden.getHeartBeatDelay`.
pub fn heart_beat_delay(client_anger: i32) -> i32 {
    let anger = client_anger as f32 / AngerLevel::Angry.minimum() as f32;
    40 - (anger.clamp(0.0, 1.0) * 30.0).floor() as i32
}
/// `Warden.getAmbientSound`: none while roaring, digging or emerging.
pub fn ambient_sound(m: &Mob) -> Option<&'static str> {
    if pose(m) == Pose::Roaring || digging_or_emerging(m) {
        None
    } else {
        Some(anger_level(m).ambient_sound())
    }
}
/// `getTendrilAnimation` / `getHeartAnimation` at the partial tick.
pub fn tendril_animation(m: &Mob, partial: f32) -> f32 {
    m.anim.lerp(TENDRIL, partial) / 10.0
}
pub fn heart_animation(m: &Mob, partial: f32) -> f32 {
    m.anim.lerp(HEART, partial) / 10.0
}

// ---- model ----

/// `WardenModel.setupAnim`.
pub fn anim(m: &Mob, s: &State, p: &mut Poses, animations: &HashMap<String, Animation>) {
    p.xrot("head", s.x_rot.to_radians());
    p.yrot("head", s.y_rot.to_radians());
    // animateWalk
    let speed_modifier = (3.0 * s.speed).min(0.5);
    let adjusted = s.pos * 0.8662;
    let (cos, sin) = (adjusted.mcos(), adjusted.msin());
    let speed_min = speed_modifier.min(0.35);
    let head_z = p.get("head")[5] + 0.3 * sin * speed_modifier;
    p.zrot("head", head_z);
    let head_x = p.get("head")[3] + 1.2 * (adjusted + PI / 2.0).mcos() * speed_min;
    p.xrot("head", head_x);
    p.zrot("body", 0.1 * sin * speed_modifier);
    p.xrot("body", cos * speed_min);
    p.xrot("left_leg", cos * speed_modifier);
    p.xrot("right_leg", (adjusted + PI).mcos() * speed_modifier);
    p.xrot("left_arm", -(0.8 * cos * speed_modifier));
    p.zrot("left_arm", 0.0);
    p.xrot("right_arm", -(0.8 * sin * speed_modifier));
    p.zrot("right_arm", 0.0);
    for (arm, x) in [("left_arm", 13.0), ("right_arm", -13.0)] {
        let a = p.get(arm);
        a[4] = 0.0;
        a[2] = 1.0;
        a[0] = x;
        a[1] = -13.0;
    }
    // animateIdlePose
    let scaled = s.age * 0.1;
    let (wobble_cos, wobble_sin) = (scaled.mcos(), scaled.msin());
    let head_z = p.get("head")[5] + 0.06 * wobble_cos;
    p.zrot("head", head_z);
    let head_x = p.get("head")[3] + 0.06 * wobble_sin;
    p.xrot("head", head_x);
    let body_z = p.get("body")[5] + 0.025 * wobble_sin;
    p.zrot("body", body_z);
    let body_x = p.get("body")[3] + 0.025 * wobble_cos;
    p.xrot("body", body_x);
    // animateTendrils
    let tendril = tendril_animation(m, s.partial)
        * ((f64::from(s.age) * 2.25).cos() * std::f64::consts::PI * f64::from(0.1f32)) as f32;
    p.xrot("left_tendril", tendril);
    p.xrot("right_tendril", -tendril);
    for (slot, name) in [
        (ATTACK, "WARDEN_ATTACK"),
        (SONIC_BOOM, "WARDEN_SONIC_BOOM"),
        (DIGGING, "WARDEN_DIG"),
        (EMERGE, "WARDEN_EMERGE"),
        (ROAR, "WARDEN_ROAR"),
        (SNIFF, "WARDEN_SNIFF"),
    ] {
        if let (Some(ms), Some(a)) = (
            m.anim.states[slot].millis(s.age),
            animations.get(&format!("WardenAnimation.{name}")),
        ) {
            a.apply(ms, 1.0, p);
        }
    }
}
/// The emissive layers of `WardenRenderer`: the bioluminescent skin, the
/// pulsating spots, the tendrils and the heart, as (model, texture, alpha).
pub fn emissive_layers(m: &Mob, partial: f32) -> [(&'static str, &'static str, f32); 5] {
    let age = m.age as f32 + partial;
    [
        (
            "warden#bioluminescent",
            "entity/warden/warden_bioluminescent_layer",
            1.0,
        ),
        (
            "warden#pulsating_spots",
            "entity/warden/warden_pulsating_spots_1",
            ((age * 0.045).mcos() * 0.25).max(0.0),
        ),
        (
            "warden#pulsating_spots",
            "entity/warden/warden_pulsating_spots_2",
            ((age * 0.045 + PI).mcos() * 0.25).max(0.0),
        ),
        (
            "warden#tendrils",
            "entity/warden/warden",
            tendril_animation(m, partial),
        ),
        (
            "warden#heart",
            "entity/warden/warden_heart",
            heart_animation(m, partial),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anger_levels_follow_the_thresholds() {
        assert_eq!(AngerLevel::by_anger(0), AngerLevel::Calm);
        assert_eq!(AngerLevel::by_anger(39), AngerLevel::Calm);
        assert_eq!(AngerLevel::by_anger(40), AngerLevel::Agitated);
        assert_eq!(AngerLevel::by_anger(79), AngerLevel::Agitated);
        assert_eq!(AngerLevel::by_anger(80), AngerLevel::Angry);
        assert!(AngerLevel::Angry.is_angry());
        assert_eq!(AngerLevel::Calm.ambient_sound(), "entity.warden.ambient");
        assert_eq!(
            AngerLevel::Agitated.listening_sound(),
            "entity.warden.listening_angry"
        );
    }

    #[test]
    fn anger_accumulates_to_a_cap_and_decays_each_second() {
        let mut a = AngerManagement::default();
        assert_eq!(a.increase(Who::Player, 35), 35);
        assert_eq!(a.increase(Who::Player, 35), 70);
        assert_eq!(a.increase(Who::Player, 100), 150);
        assert_eq!(a.active_anger(None), 150);
        a.tick(|_| true);
        assert_eq!(a.anger_of(Who::Player), 149);
        // An invalid suspect is forgotten.
        a.increase(Who::Mob(3), 10);
        a.tick(|w| w == Who::Player);
        assert_eq!(a.anger_of(Who::Mob(3)), 0);
        assert_eq!(a.suspects(), &[Who::Player]);
        // Anger of one point is gone after the next second.
        let mut b = AngerManagement::default();
        b.increase(Who::Mob(1), 1);
        b.tick(|_| true);
        assert!(b.suspects().is_empty());
    }

    #[test]
    fn the_angriest_player_comes_first() {
        let mut a = AngerManagement::default();
        a.increase(Who::Mob(1), 90);
        a.increase(Who::Player, 50);
        a.increase(Who::Mob(2), 120);
        // Angry suspects first, the angrier mob before the weaker one...
        assert_eq!(a.suspects(), &[Who::Mob(2), Who::Mob(1), Who::Player]);
        // ...until the player is angry as well: players lead ties of level.
        a.increase(Who::Player, 40);
        assert_eq!(a.suspects()[0], Who::Player);
        assert_eq!(a.active_entity(|w| w != Who::Player), Some(Who::Mob(2)));
        assert_eq!(a.active_anger(Some(Who::Mob(1))), 90);
        assert_eq!(a.active_anger(Some(Who::Mob(9))), 0);
        a.clear(Who::Player);
        assert_eq!(a.suspects(), &[Who::Mob(2), Who::Mob(1)]);
        assert_eq!(a.active_anger(None), 120);
    }

    #[test]
    fn a_single_suspect_sets_the_highest_anger() {
        let mut a = AngerManagement::default();
        a.increase(Who::Mob(5), 44);
        assert_eq!(a.active_anger(None), 44);
        a.clear(Who::Mob(5));
        assert_eq!(a.active_anger(None), 0);
    }

    #[test]
    fn the_heart_beats_faster_when_angry() {
        assert_eq!(heart_beat_delay(0), 40);
        assert_eq!(heart_beat_delay(40), 25);
        assert_eq!(heart_beat_delay(80), 10);
        assert_eq!(heart_beat_delay(150), 10);
    }

    #[test]
    fn memories_expire_with_their_ttl() {
        let mut st = Warden {
            in_brain: true,
            ..Warden::default()
        };
        // Set during the brain: present for exactly the ttl.
        let until = st.expiry(100, 5);
        assert!(present(until, 104));
        assert!(!present(until, 105));
        // Set before the brain's tick: that tick already counts.
        st.in_brain = false;
        let until = st.expiry(100, 5);
        assert!(present(until, 103));
        assert!(!present(until, 104));
    }

    #[test]
    fn the_pose_codes_round_trip() {
        for p in [
            Pose::Standing,
            Pose::Emerging,
            Pose::Digging,
            Pose::Roaring,
            Pose::Sniffing,
        ] {
            assert_eq!(Pose::from_code(p.code()), p);
        }
    }
}
