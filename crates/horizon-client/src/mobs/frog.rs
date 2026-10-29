//! Frogs (`Frog`, `FrogAi`): the `Croak` and `ShootTongue` behaviours as
//! goals, the poses they set and the client's animation states.
//! AnimState states: jump, croak, tongue, swim idle.
//!
//! The brain's idle `RunOne` picks a croak three times as often as a stroll
//! (weights 3 and 1; it also holds a do-nothing option that only repeats
//! the pick), so a frog that is not walking croaks in three of four idle
//! picks: `Croak` takes those, and the frog's stroll then follows at once.
use super::{
    Mob, MobEvent, MobWorld, Species, Who,
    goals::{
        Cx, Float, Goal, LOOK, LookAtPlayer, MOVE, Panic, RandomLookAround, Stroll, goal_clone,
    },
    sync,
};
use horizon_core::{geometry::Vec3, random::RandomSource};

const JUMP: usize = 0;
const CROAK: usize = 1;
const TONGUE: usize = 2;
const SWIM_IDLE: usize = 3;

/// `Croak.CROAK_TICKS`.
const CROAK_TICKS: i32 = 60;
/// `ShootTongue`'s duration and eating distance.
const TONGUE_TIME_OUT: i32 = 100;
const EATING_DISTANCE: f64 = 1.75;
/// `FrogAttackablesSensor.TARGET_DETECTION_DISTANCE` and the sensor's scan
/// rate.
const DETECTION_DISTANCE: f64 = 10.0;
const SCAN_RATE: i32 = 20;

/// `ShootTongue.State`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Phase {
    MoveToTarget,
    CatchAnimation,
    EatAnimation,
    #[default]
    Done,
}

/// The frog's brain memories and `DATA_TONGUE_TARGET_ID`.
#[derive(Clone, Debug, Default)]
pub struct State {
    croak: i32,
    /// Ticks to the attackables sensor's next scan, and `NEAREST_ATTACKABLE`.
    scan: i32,
    nearest: Option<u32>,
    /// `UNREACHABLE_TONGUE_TARGETS`: the mob and the ticks it stays in mind.
    unreachable: Vec<(u32, i32)>,
    /// `ATTACK_TARGET` while `ShootTongue` runs, and the synched tongue target.
    target: Option<u32>,
    pub tongue_target: Option<u32>,
    phase: Phase,
    eat_timer: i32,
    path_counter: i32,
    time: i32,
}

pub fn register(m: &mut Mob) {
    // `Frog()`: SmoothSwimmingMoveControl(85, 10, 0.02, 0.1, true). The
    // frog's movement speed is 1.0, so the controller's 0.1 outside-water
    // speed is what keeps a walking frog at vanilla's pace; the plain ground
    // controller walks ten times too fast (#2213).
    m.ai.mov.kind = super::control::MoveKind::SmoothSwimming {
        max_turn_x: 85.0,
        max_turn_y: 10.0,
        in_water: 0.02,
        outside: 0.1,
        gravity: true,
    };
    let g = &mut m.ai.goals;
    g.add(0, Float);
    g.add(1, Panic::new(1.25));
    g.add(2, ShootTongue);
    g.add(4, Croak);
    g.add(5, Stroll::avoiding_water(1.0).interval(1));
    g.add(6, LookAtPlayer::new(6.0));
    g.add(7, RandomLookAround::default());
}

/// `Entity.setPose` and `Frog.onSyncedDataUpdated(DATA_POSE)`.
pub fn set_pose(m: &mut Mob, id: u8) {
    let old = std::mem::replace(&mut m.extra.pose, id);
    if old != id {
        pose_changed(m);
    }
}
/// The animation states of the pose the frog has taken.
pub fn pose_changed(m: &mut Mob) {
    let age = m.age as f32;
    let pose = m.extra.pose;
    let states = &mut m.anim.states;
    for (state, on) in [
        (JUMP, pose == sync::LONG_JUMPING),
        (CROAK, pose == sync::CROAKING),
        (TONGUE, pose == sync::USING_TONGUE),
    ] {
        if on {
            states[state].start(age);
        } else {
            states[state].stop();
        }
    }
}
/// `Frog.tick` on a client: the idle swim while it floats still. The server
/// runs it too, for a window that shows its own mobs.
pub fn tick(m: &mut Mob) {
    let age = m.age as f32;
    let still = m.fluids.in_water && m.limb_speed <= 1.0e-5;
    m.anim.states[SWIM_IDLE].animate_when(still, age);
}

/// `Frog.canEat`: a slime or magma cube of size 1.
fn can_eat(species: Species, size: i32) -> bool {
    matches!(species, Species::Slime | Species::MagmaCube) && size == 1
}

/// `FrogAttackablesSensor` (every 20 ticks) and the memory timers.
pub fn server_ai_step(m: &mut Mob, w: &MobWorld) {
    let f = &mut m.extra.frog;
    f.unreachable.retain_mut(|(_, ttl)| {
        *ttl -= 1;
        *ttl > 0
    });
    f.scan -= 1;
    if f.scan > 0 {
        return;
    }
    f.scan = SCAN_RATE;
    let unreachable: Vec<u32> = f.unreachable.iter().map(|u| u.0).collect();
    let eye = Vec3::new(m.position.x, m.eye_y(), m.position.z);
    let nearest = w
        .nearby(m.position, DETECTION_DISTANCE, |s| {
            can_eat(s.species, s.size)
                && !unreachable.contains(&s.id)
                && w.line_of_sight(eye, Vec3::new(s.position.x, s.eye_y, s.position.z))
        })
        .find(|s| s.position.distance_squared(m.position) < DETECTION_DISTANCE * DETECTION_DISTANCE)
        .map(|s| s.id);
    m.extra.frog.nearest = nearest;
}

/// `Croak`: the pose sync::CROAKING for 60 ticks (the croak is the frog's ambient
/// sound).
#[derive(Clone, Debug)]
struct Croak;
impl Goal for Croak {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, _cx: &mut Cx) -> bool {
        // checkExtraStartConditions (standing), no walk target, and not in
        // a liquid (start does nothing there).
        m.extra.pose == sync::STANDING
            && m.on_ground
            && !m.fluids.in_water
            && !m.fluids.in_lava
            && m.ai.nav.is_done()
            && m.random.next_int_bound(4).unwrap_or(0) != 0
    }
    fn can_continue(&mut self, m: &mut Mob, _cx: &mut Cx) -> bool {
        m.extra.frog.croak < CROAK_TICKS
    }
    fn start(&mut self, m: &mut Mob, _cx: &mut Cx) {
        m.extra.frog.croak = 0;
        set_pose(m, sync::CROAKING);
    }
    fn stop(&mut self, m: &mut Mob, _cx: &mut Cx) {
        set_pose(m, sync::STANDING);
    }
    fn tick(&mut self, m: &mut Mob, _cx: &mut Cx) {
        m.extra.frog.croak += 1;
    }
    fn every_tick(&self) -> bool {
        true
    }
}

/// `ShootTongue` with the tongue activity's `StartAttacking` and
/// `StopAttackingIfTargetInvalid`: walk up to a small slime or magma cube,
/// shoot the tongue, pull it in and eat it.
#[derive(Clone, Debug)]
struct ShootTongue;
impl Goal for ShootTongue {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some(id) = m.extra.frog.nearest else {
            return false;
        };
        // The start conditions: no walk target, no panic, not croaking.
        if m.ai.panicking || m.extra.pose == sync::CROAKING || !m.ai.nav.is_done() {
            return false;
        }
        let Some(t) = cx.w.seen(id).filter(|t| t.alive) else {
            m.extra.frog.nearest = None;
            return false;
        };
        // canPathfindToTarget: a path that ends at the target.
        let at = [t.position.x, t.position.y, t.position.z];
        let ends = m
            .create_path_to_entity(cx.w, at, 0)
            .is_some_and(|p| f64::from(p.dist_to_target) < EATING_DISTANCE);
        if !ends {
            // The target is forgotten and not taken for 100 ticks.
            let f = &mut m.extra.frog;
            f.nearest = None;
            if !f.unreachable.iter().any(|u| u.0 == id) {
                if f.unreachable.len() == 5 {
                    f.unreachable.remove(0);
                }
                f.unreachable.push((id, 100));
            }
            return false;
        }
        m.extra.frog.target = Some(id);
        true
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let f = &m.extra.frog;
        f.phase != Phase::Done
            && f.time <= TONGUE_TIME_OUT
            && !m.ai.panicking
            && f.target
                .is_some_and(|id| cx.w.seen(id).is_some_and(|t| t.alive))
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(t) = m.extra.frog.target.and_then(|id| cx.w.seen(id)) else {
            return;
        };
        let at = [t.position.x, t.position.y, t.position.z];
        let eye = [t.position.x, t.eye_y, t.position.z];
        m.look_at(eye);
        m.extra.frog.tongue_target = m.extra.frog.target;
        m.nav_move_to_entity(cx.w, at, 0, 2.0);
        let f = &mut m.extra.frog;
        f.path_counter = 10;
        f.time = 0;
        f.phase = Phase::MoveToTarget;
    }
    fn stop(&mut self, m: &mut Mob, _cx: &mut Cx) {
        let f = &mut m.extra.frog;
        f.target = None;
        f.tongue_target = None;
        f.phase = Phase::Done;
        set_pose(m, sync::STANDING);
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(id) = m.extra.frog.target else {
            return;
        };
        let Some(t) = cx.w.seen(id) else {
            return;
        };
        let (at, pos) = ([t.position.x, t.position.y, t.position.z], t.position);
        m.look_at([t.position.x, t.eye_y, t.position.z]);
        m.extra.frog.tongue_target = Some(id);
        m.extra.frog.time += 1;
        let here = [m.position.x, m.position.y, m.position.z];
        match m.extra.frog.phase {
            Phase::MoveToTarget => {
                if pos.distance_squared(m.position) < EATING_DISTANCE * EATING_DISTANCE {
                    cx.events
                        .push(MobEvent::Sound("entity.frog.tongue", here, 2.0, 1.0));
                    set_pose(m, sync::USING_TONGUE);
                    // The target is drawn in at 0.75 a tick.
                    let pull = m.position.subtract(pos).normalize().scale(0.75);
                    cx.events.push(MobEvent::Push {
                        target: Who::Mob(id),
                        velocity: [pull.x, pull.y, pull.z],
                    });
                    let f = &mut m.extra.frog;
                    f.eat_timer = 0;
                    f.phase = Phase::CatchAnimation;
                } else if m.extra.frog.path_counter <= 0 {
                    m.nav_move_to_entity(cx.w, at, 0, 2.0);
                    m.extra.frog.path_counter = 10;
                } else {
                    m.extra.frog.path_counter -= 1;
                }
            }
            Phase::CatchAnimation => {
                let f = &mut m.extra.frog;
                f.eat_timer += 1;
                if f.eat_timer > 6 {
                    f.phase = Phase::EatAnimation;
                    // eatEntity.
                    cx.events
                        .push(MobEvent::Sound("entity.frog.eat", here, 2.0, 1.0));
                    super::ai::do_hurt_target(m, Who::Mob(id), cx);
                }
            }
            Phase::EatAnimation => {
                let f = &mut m.extra.frog;
                if f.eat_timer >= 10 {
                    f.phase = Phase::Done;
                } else {
                    f.eat_timer += 1;
                }
            }
            Phase::Done => {}
        }
    }
}
