//! Copper golems (`CopperGolem`, `CopperGolemAi`): they carry items from
//! copper chests to chests (see `game::copper_golems` for the container side
//! of `TransportItemsBetweenContainers`), play the chest interaction
//! animations, spin their heads when idle, oxidise over time and finally turn
//! into statues.
use super::{
    Mob, MobEvent, MobWorld,
    anim::{Poses, State},
    goals::{Cx, Goal, JUMP, LOOK, MOVE, Panic, goal_clone},
    keyframes::Animation,
};
use horizon_core::random::RandomSource;
use std::collections::HashMap;

// Copper golem state: `variant` the weathering (0 unaffected .. 3 oxidized),
// `variant3` the `CopperGolemState`, `main_hand` with `ints[1]` the carried
// stack, the saddle slot the antenna (`EQUIPMENT_SLOT_ANTENNA`), `egg_time` the next weathering tick (-1
// unset, -2 waxed), `ints[0]` the idle animation start tick, `home` the chest
// being served with `positions` the visited chests, `point` where it walks,
// `scratch` = [transport phase, ticks since reaching the target, interaction
// state, transport cooldown ticks]. AnimationState slots: idle (0), getting an
// item (1), getting nothing (2), dropping an item (3), dropping nothing (4).
pub const IDLE: u8 = 0;
pub const GETTING_ITEM: u8 = 1;
pub const GETTING_NO_ITEM: u8 = 2;
pub const DROPPING_ITEM: u8 = 3;
pub const DROPPING_NO_ITEM: u8 = 4;
/// `WeatheringCopper.WeatherState` names.
pub const WEATHER: [&str; 4] = ["unaffected", "exposed", "weathered", "oxidized"];

pub fn register(m: &mut Mob) {
    m.ai.nav.required_path_length = 48.0;
    m.ai.nav.can_open_doors = true;
    m.persistent = true;
    m.extra.egg_time = -1;
    m.extra.scratch[3] = 60 + m.random.next_int_bound(40).unwrap_or(0);
    let g = &mut m.ai.goals;
    g.add(0, Panic::new(1.5));
    g.add(1, Serve);
    g.add(2, super::goals::LookAtPlayer::new(6.0).probability(0.02));
    g.add(3, Idle::default());
}
/// `CopperGolemOxidationLevels`: the body and eye textures.
pub fn texture(m: &Mob) -> String {
    match m.extra.variant {
        0 => "entity/copper_golem/copper_golem".into(),
        n => format!(
            "entity/copper_golem/copper_golem_{}",
            WEATHER[n.min(3) as usize]
        ),
    }
}
pub fn eye_texture(m: &Mob) -> String {
    match m.extra.variant {
        0 => "entity/copper_golem/copper_golem_eyes".into(),
        n => format!(
            "entity/copper_golem/copper_golem_eyes_{}",
            WEATHER[n.min(3) as usize]
        ),
    }
}
/// The sound of the oxidation level: `spin`, `hurt`, `death` or `step`.
pub fn sound(m: &Mob, what: &str) -> &'static str {
    match (m.extra.variant, what) {
        (2, "spin") => "entity.copper_golem_weathered.spin",
        (2, "hurt") => "entity.copper_golem_weathered.hurt",
        (2, "death") => "entity.copper_golem_weathered.death",
        (2, _) => "entity.copper_golem_weathered.step",
        (3, "spin") => "entity.copper_golem_oxidized.spin",
        (3, "hurt") => "entity.copper_golem_oxidized.hurt",
        (3, "death") => "entity.copper_golem_oxidized.death",
        (3, _) => "entity.copper_golem_oxidized.step",
        (_, "spin") => "entity.copper_golem.spin",
        (_, "hurt") => "entity.copper_golem.hurt",
        (_, "death") => "entity.copper_golem.death",
        _ => "entity.copper_golem.step",
    }
}
/// `CopperGolemAi`'s transport behaviour is running a target.
fn serving(m: &Mob) -> bool {
    m.extra.home.is_some()
}
/// `TransportItemsBetweenContainers` walking and looking at its chest, and
/// standing still while interacting with it.
#[derive(Clone, Debug)]
struct Serve;
impl Goal for Serve {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        serving(m) && !m.ai.panicking
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.can_use(m, cx)
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(h) = m.extra.home else { return };
        m.look_at([
            f64::from(h[0]) + 0.5,
            f64::from(h[1]) + 0.5,
            f64::from(h[2]) + 0.5,
        ]);
        match m.extra.point {
            Some(p) => {
                if m.ai.nav.is_done() || m.age % 10 == 0 {
                    let block = [
                        p[0].floor() as i32,
                        p[1].floor() as i32,
                        p[2].floor() as i32,
                    ];
                    let path = m.create_path_to(cx.w, block, 0);
                    m.nav_move_to_path(path, 1.0);
                }
            }
            None => {
                // stopInPlace.
                m.ai.nav.stop();
                m.velocity.x = 0.0;
                m.velocity.z = 0.0;
            }
        }
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.ai.nav.stop();
    }
}
/// The idle `RunOne`: `RandomStroll.stroll(1.0, 2, 2)` or `DoNothing(30, 60)`,
/// only while the transport cooldown runs.
#[derive(Clone, Debug, Default)]
struct Idle {
    wait: i32,
    stroll: bool,
}
impl Goal for Idle {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | JUMP
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if serving(m) || m.extra.scratch[3] <= 0 || !m.ai.nav.is_done() {
            return false;
        }
        if m.random.next_int_bound(2).unwrap_or(0) == 0 {
            let Some(p) = m.land_random_pos(cx.w, 2, 2) else {
                return false;
            };
            self.stroll = true;
            m.nav_move_to(cx.w, p, 1.0);
        } else {
            self.stroll = false;
            self.wait = 30 + m.random.next_int_bound(30).unwrap_or(0);
        }
        true
    }
    fn can_continue(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        if serving(m) {
            return false;
        }
        if self.stroll {
            !m.ai.nav.is_done()
        } else {
            self.wait > 0
        }
    }
    fn tick(&mut self, _: &mut Mob, _: &mut Cx) {
        self.wait -= 1;
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.ai.nav.stop();
    }
}
/// `CopperGolem.setupAnimationStates` (client) and the sound of the head spin.
pub fn ai_step(m: &mut Mob, _w: &MobWorld, events: &mut Vec<MobEvent>) {
    let age = m.age as f32;
    let state = m.extra.variant3;
    let st = &mut m.anim.states;
    let interaction = |st: &mut [super::keyframes::AnimationState; 6], keep: usize| {
        for (i, s) in st.iter_mut().enumerate().take(5).skip(1) {
            if i != keep {
                s.stop();
            }
        }
    };
    match state {
        IDLE => {
            interaction(st, 0);
            let tick = m.age as i32;
            if m.extra.ints[0] == tick {
                st[0].start(age);
            } else if m.extra.ints[0] == 0 {
                m.extra.ints[0] = tick + 200 + m.random.next_int_bound(40).unwrap_or(0);
            }
            if tick == m.extra.ints[0] + 10 {
                let at = [m.position.x, m.position.y, m.position.z];
                events.push(MobEvent::Sound(sound(m, "spin"), at, 1.0, 1.0));
                m.extra.ints[0] = 0;
            }
        }
        s @ GETTING_ITEM..=DROPPING_NO_ITEM => {
            st[0].stop();
            m.extra.ints[0] = 0;
            interaction(st, s as usize);
            st[s as usize].start_if_stopped(age);
        }
        _ => {}
    }
}
/// `CopperGolemModel.setupAnim`.
pub fn anim(m: &Mob, s: &State, p: &mut Poses, animations: &HashMap<String, Animation>) {
    p.xrot("head", s.x_rot.to_radians());
    p.yrot("head", s.y_rot.to_radians());
    let get = |n: &str| animations.get(&format!("CopperGolemAnimation.COPPER_GOLEM_{n}"));
    let holding = m.extra.main_hand.is_some() || m.extra.off_hand.is_some();
    if holding {
        if let Some(a) = get("WALK_ITEM") {
            a.apply_walk(s.pos, s.speed, 2.0, 2.5, p);
        }
        // poseHeldItemArmsIfStill.
        let right = p.get("right_arm");
        right[3] = right[3].min(-0.872_664_6);
        right[4] = right[4].min(-0.113_446_4);
        right[5] = right[5].min(-0.064_577_185);
        let left = p.get("left_arm");
        left[3] = left[3].min(-0.872_664_6);
        left[4] = left[4].max(0.113_446_4);
        left[5] = left[5].max(0.064_577_185);
    } else if let Some(a) = get("WALK") {
        a.apply_walk(s.pos, s.speed, 2.0, 2.5, p);
    }
    for (state, name) in [
        (&m.anim.states[0], "IDLE"),
        (&m.anim.states[1], "CHEST_INTERACTION_NOITEM_GET"),
        (&m.anim.states[2], "CHEST_INTERACTION_NOITEM_NOGET"),
        (&m.anim.states[3], "CHEST_INTERACTION_ITEM_DROP"),
        (&m.anim.states[4], "CHEST_INTERACTION_ITEM_NODROP"),
    ] {
        if let (Some(ms), Some(a)) = (state.millis(s.age), get(name)) {
            a.apply(ms, 1.0, p);
        }
    }
}
/// `WeatheringCopper.WeatherState.next`/`previous` on the variant.
pub fn weather_next(v: u8) -> u8 {
    (v + 1).min(3)
}
pub fn weather_previous(v: u8) -> u8 {
    v.saturating_sub(1)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn weathering_stops_at_both_ends() {
        assert_eq!(weather_next(3), 3);
        assert_eq!(weather_previous(0), 0);
        assert_eq!(weather_next(1), 2);
        assert_eq!(weather_previous(2), 1);
    }
}
