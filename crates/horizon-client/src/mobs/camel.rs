//! Camels (`Camel`): ridden by two, sitting down and standing up now and
//! then, dashing instead of jumping, eating cactus, always "tame".
use super::{
    Mob, MobEvent,
    anim::{Poses, State},
    goals::{
        Breed, Cx, Float, FollowParent, Goal, LookAtPlayer, MOVE, Panic, RandomLookAround, Stroll,
        Tempt, goal_clone,
    },
};
use horizon_core::geometry::trig::Mth;
use horizon_core::random::RandomSource;
use std::collections::HashMap;

// Camel state: `egg_time` the last pose change tick (negative while
// sitting), `anim_ticks` the dash cooldown, `flag2` dashing, `ints[0]` the
// idle animation timeout. AnimState states: sit, sit pose, stand up, idle,
// dash.
const SIT_DOWN: i64 = 40;
const STAND_UP: i64 = 52;
pub const DASH_COOLDOWN: i32 = 55;

pub fn register(m: &mut Mob) {
    m.extra.tame = true;
    let tempt = m.kind.tempt.clone();
    let g = &mut m.ai.goals;
    // CamelAi speed modifiers (times the 0.09 movement speed attribute).
    g.add(0, Float);
    g.add(1, CamelPanic(Panic::new(4.0)));
    g.add(2, Breed::new(1.0));
    g.add(3, Tempt::new(2.5, tempt, false));
    g.add(4, FollowParent::new(2.5));
    g.add(5, Stroll::new(2.0));
    g.add(5, RandomSitting);
    g.add(6, LookAtPlayer::new(6.0));
    g.add(7, RandomLookAround::default());
    // finalizeSpawn: fully standing.
    m.extra.egg_time = (m.age as i64 - STAND_UP - 1).max(0) as i32;
}
fn pose_time(m: &Mob) -> i64 {
    i64::from(m.age) - i64::from(m.extra.egg_time).abs()
}
pub fn sitting(m: &Mob) -> bool {
    m.extra.egg_time < 0
}
fn in_transition(m: &Mob) -> bool {
    pose_time(m) < if sitting(m) { SIT_DOWN } else { STAND_UP }
}
/// `Camel.getPoseTime` of a client mob from the `LAST_POSE_CHANGE_TICK` it
/// was sent and the client's game time, and whether that value says it sits.
fn client_pose(m: &Mob, game_time: i64) -> (i64, bool) {
    let tick = m.extra.pose_change_tick;
    (game_time - tick.abs(), tick < 0)
}
/// The camel's `LAST_POSE_CHANGE_TICK` as the server sends it: in game time
/// (a mob counts its own age), negative while sitting.
pub fn synced_pose_tick(m: &Mob, game_time: i64) -> i64 {
    let tick = (game_time - pose_time(m)).max(0);
    if sitting(m) { -tick.max(1) } else { tick }
}
/// `Entity.onSyncedDataUpdated` for the pose tick of a client mob: its sign
/// is whether it sits. `client_tick` keeps `egg_time` (in the mob's age, for
/// `sitting` and `refuse_to_move`) with it.
pub fn set_synced_pose_tick(m: &mut Mob, tick: i64) {
    m.extra.pose_change_tick = tick;
    m.extra.egg_time = if tick < 0 { -1 } else { 0 };
}
/// `Camel.refuseToMove`: sitting or getting up or down.
pub fn refuse_to_move(m: &Mob) -> bool {
    sitting(m) || in_transition(m)
}
fn sit_down(m: &mut Mob, events: &mut Vec<MobEvent>) {
    if !sitting(m) {
        sound(m, voice(m, 0), events);
        m.extra.egg_time = -(m.age as i32).max(1);
    }
}
pub fn stand_up(m: &mut Mob, events: &mut Vec<MobEvent>) {
    if sitting(m) {
        sound(m, voice(m, 1), events);
        m.extra.egg_time = m.age as i32;
    }
}
/// `standUpInstantly`.
pub fn stand_up_instantly(m: &mut Mob) {
    m.extra.egg_time = (i64::from(m.age) - STAND_UP - 1).max(0) as i32;
}
/// `is_camel`: camels and camel husks (`CamelHusk extends Camel`).
pub fn is_camel(s: super::Species) -> bool {
    matches!(s, super::Species::Camel | super::Species::CamelHusk)
}
/// The sit, stand, dash ready, dash and eat sounds (`CamelHusk` overrides each).
pub fn voice(m: &Mob, which: usize) -> &'static str {
    let husk = m.species() == super::Species::CamelHusk;
    match (which, husk) {
        (0, false) => "entity.camel.sit",
        (0, true) => "entity.camel_husk.sit",
        (1, false) => "entity.camel.stand",
        (1, true) => "entity.camel_husk.stand",
        (2, false) => "entity.camel.dash_ready",
        (2, true) => "entity.camel_husk.dash_ready",
        (3, false) => "entity.camel.dash",
        (3, true) => "entity.camel_husk.dash",
        (_, false) => "entity.camel.eat",
        (_, true) => "entity.camel_husk.eat",
    }
}
fn sound(m: &Mob, name: &'static str, events: &mut Vec<MobEvent>) {
    events.push(MobEvent::Sound(
        name,
        [m.position.x, m.position.y, m.position.z],
        1.0,
        1.0,
    ));
}
/// `CamelAi.CamelPanic`: standing up at once to flee.
#[derive(Clone, Debug)]
struct CamelPanic(Panic);
impl Goal for CamelPanic {
    goal_clone!();
    fn flags(&self) -> u8 {
        self.0.flags()
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        // CamelHusk.isMobControlled: not with a mob on its back either.
        m.rider.is_none()
            && !(m.species() == super::Species::CamelHusk && m.extra.rider.is_some())
            && self.0.can_use(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_continue(m, cx)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        stand_up_instantly(m);
        self.0.start(m, cx);
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.stop(m, cx);
    }
}
/// `CamelAi.RandomSitting`: after 20 seconds in a pose, sit or stand.
#[derive(Clone, Debug)]
struct RandomSitting;
impl Goal for RandomSitting {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, _cx: &mut Cx) -> bool {
        !m.fluids.in_water
            && pose_time(m) >= 400
            && m.on_ground
            && m.rider.is_none()
            && m.random.next_int_bound(4).unwrap_or(1) == 0
    }
    fn can_continue(&mut self, _m: &mut Mob, _cx: &mut Cx) -> bool {
        false
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        if sitting(m) {
            stand_up(m, cx.events);
        } else if !super::goals::should_panic(m) {
            sit_down(m, cx.events);
            m.ai.nav.stop();
        }
    }
}
/// `Camel.tick`: the dash clock, water stands it up; keyframe states.
pub fn ai_step(m: &mut Mob, events: &mut Vec<MobEvent>) {
    if m.extra.flag2 && m.extra.anim_ticks < 50 && (m.on_ground || m.fluids.in_water) {
        m.extra.flag2 = false;
    }
    if m.extra.anim_ticks > 0 {
        m.extra.anim_ticks -= 1;
        if m.extra.anim_ticks == 0 {
            sound(m, voice(m, 2), events);
        }
    }
    if sitting(m) && m.fluids.in_water {
        stand_up_instantly(m);
    }
    if refuse_to_move(m) {
        m.velocity.x = 0.0;
        m.velocity.z = 0.0;
        m.ai.xxa = 0.0;
        m.ai.zza = 0.0;
        m.ai.nav.stop();
    }
    animation_states(m, pose_time(m), sitting(m));
}
/// `Camel.tick` on a client that is not the server: the dash cooldown, which
/// the dash flag starts, and the keyframe states from the synced pose tick.
pub fn client_tick(m: &mut Mob, game_time: i64) {
    if m.extra.anim_ticks > 0 {
        m.extra.anim_ticks -= 1;
    }
    let (pose_time, sitting) = client_pose(m, game_time);
    let since = (i64::from(m.age) - pose_time).max(0) as i32;
    m.extra.egg_time = if sitting { -since.max(1) } else { since };
    animation_states(m, pose_time, sitting);
}
/// `Camel.onSyncedDataUpdated(DASH)`: a dash starts the cooldown unless it
/// runs. Not on the first tick.
pub fn set_dashing(m: &mut Mob, dashing: bool) {
    if m.age > 0 && m.extra.anim_ticks == 0 {
        m.extra.anim_ticks = DASH_COOLDOWN;
    }
    m.extra.flag2 = dashing;
}
/// `Camel.setupAnimationStates` for a pose time and whether it sits.
fn animation_states(m: &mut Mob, pose_time: i64, sitting: bool) {
    let age = m.age as f32;
    if m.extra.ints[0] <= 0 {
        m.extra.ints[0] = m.random.next_int_bound(40).unwrap_or(0) + 80;
        m.anim.states[3].start(age);
    } else {
        m.extra.ints[0] -= 1;
    }
    // isCamelVisuallySitting: (poseTime < 0) != sitting.
    let visually_sitting = (pose_time < 0) != sitting;
    let sitting_down = sitting && (0..SIT_DOWN).contains(&pose_time);
    let transition = (0..if sitting { SIT_DOWN } else { STAND_UP }).contains(&pose_time);
    let dashing = m.extra.flag2;
    let st = &mut m.anim.states;
    if visually_sitting {
        st[2].stop();
        st[4].stop();
        if sitting_down {
            st[0].start_if_stopped(age);
            st[1].stop();
        } else {
            st[0].stop();
            st[1].start_if_stopped(age);
        }
    } else {
        st[0].stop();
        st[1].stop();
        st[4].animate_when(dashing, age);
        st[2].animate_when(transition, age);
    }
}
/// `Camel.executeRidersJump`: the dash along the look, once cooled down.
pub fn dash(m: &mut Mob, amount: f32, events: &mut Vec<MobEvent>) {
    if m.extra.anim_ticks > 0 || !m.on_ground || refuse_to_move(m) {
        return;
    }
    let yaw = m.yaw.to_radians();
    let (dx, dz) = (f64::from(-yaw.msin()), f64::from(yaw.mcos()));
    let speed = m.movement_speed();
    let a = f64::from(amount);
    m.velocity.x += dx * 22.2222 * a * speed;
    m.velocity.z += dz * 22.2222 * a * speed;
    m.velocity.y += 1.4285 * a * 0.42;
    m.extra.anim_ticks = DASH_COOLDOWN;
    m.extra.flag2 = true;
    sound(m, voice(m, 3), events);
}
/// `CamelModel.setupAnim`.
pub fn anim(
    m: &Mob,
    s: &State,
    p: &mut Poses,
    animations: &HashMap<String, super::keyframes::Animation>,
) {
    let mut y = s.y_rot.clamp(-30.0, 30.0);
    let mut x = s.x_rot.clamp(-25.0, 45.0);
    let cooldown = (m.extra.anim_ticks as f32 - s.partial).max(0.0);
    if cooldown > 0.0 {
        x = (x + 45.0 * cooldown / 55.0).clamp(-25.0, 70.0);
    }
    y = y.to_radians();
    p.yrot("head", y);
    p.xrot("head", x.to_radians());
    let prefix = if m.is_baby() {
        "CamelBabyAnimation.CAMEL_BABY_"
    } else {
        "CamelAnimation.CAMEL_"
    };
    let get = |n: &str| animations.get(&format!("{prefix}{n}"));
    if let Some(walk) = get("WALK") {
        walk.apply_walk(s.pos, s.speed, 2.0, 2.5, p);
    }
    let age = s.age;
    for (state, name) in [
        (&m.anim.states[0], "SIT"),
        (&m.anim.states[1], "SIT_POSE"),
        (&m.anim.states[2], "STANDUP"),
        (&m.anim.states[3], "IDLE"),
        (&m.anim.states[4], "DASH"),
    ] {
        if let (Some(ms), Some(a)) = (state.millis(age), get(name)) {
            a.apply(ms, 1.0, p);
        }
    }
}
