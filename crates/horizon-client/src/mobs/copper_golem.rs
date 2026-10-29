//! Copper golems (`CopperGolem`): their brain (`CopperGolemAi`, in
//! `animal_brain`) carries items from copper chests to chests
//! (`transport_items`); here the chest interaction animations, the idle head
//! spin, and weathering until they turn into statues.
use super::{
    Mob, MobEvent, MobWorld,
    anim::{Poses, State},
    keyframes::Animation,
};
use horizon_core::random::RandomSource;
use std::collections::HashMap;

// Copper golem state: `variant` the weathering (0 unaffected .. 3 oxidized),
// `variant3` the `CopperGolemState`, `main_hand` with `ints[1]` the carried
// stack, the saddle slot the antenna (`EQUIPMENT_SLOT_ANTENNA`), `vector[0]`
// the next weathering tick (-1 unset, -2 waxed), `ints[0]` the idle animation
// start tick, `opened_chest` the `openedChestPos`. AnimationState slots: idle
// (0), getting an item (1), getting nothing (2), dropping an item (3),
// dropping nothing (4).
pub const IDLE: u8 = 0;
pub const GETTING_ITEM: u8 = 1;
pub const GETTING_NO_ITEM: u8 = 2;
pub const DROPPING_ITEM: u8 = 3;
pub const DROPPING_NO_ITEM: u8 = 4;
/// `WeatheringCopper.WeatherState` names.
pub const WEATHER: [&str; 4] = ["unaffected", "exposed", "weathered", "oxidized"];

pub fn register(m: &mut Mob) {
    m.set_required_path_length(48.0);
    m.ai.nav.can_open_doors = true;
    m.persistent = true;
    // nextWeatheringTick starts unset (-1); waxed is -2.
    m.extra.vector[0] = -1.0;
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
/// `CopperGolem.setupAnimationStates` (client) and the sound of the head spin.
pub fn ai_step(m: &mut Mob, _w: &MobWorld, events: &mut Vec<MobEvent>) {
    if animate(m) {
        let at = [m.position.x, m.position.y, m.position.z];
        events.push(MobEvent::LocalSound(sound(m, "spin"), at, 1.0, 1.0));
    }
}
/// `CopperGolem.setupAnimationStates`, for the server and a client alike;
/// returns whether the head spin sound plays now (a client's sound is the
/// server's `LocalSound`).
pub fn animate(m: &mut Mob) -> bool {
    let mut spin = false;
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
                // The client entity's own random, not the server's.
                let mut r =
                    horizon_core::random::LegacyRandom::new(i64::from(m.id) ^ i64::from(tick));
                m.extra.ints[0] = tick + 200 + r.next_int_bound(40).unwrap_or(0);
            }
            if tick == m.extra.ints[0] + 10 {
                spin = true;
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
    spin
}
/// `CopperGolem.updateWeathering` on the level random: the first tick sets
/// the next weathering tick, which advances the weather state; once
/// oxidized, a golem on an air block may turn into a statue
/// (`MobEvent::BecomeStatue`, done by the host).
pub fn update_weathering(
    m: &mut Mob,
    w: &MobWorld,
    lr: &mut impl RandomSource,
    events: &mut Vec<MobEvent>,
) {
    if !m.alive() || m.removed {
        return;
    }
    let now = w.game_time;
    let next = m.extra.vector[0] as i64;
    if next == -2 {
        return;
    }
    let interval = |r: &mut dyn FnMut() -> i32| 504_000 + i64::from(r());
    let mut draw = || lr.next_int_bound(48_001).unwrap_or(0);
    if next == -1 {
        m.extra.vector[0] = (now + interval(&mut draw)) as f64;
        return;
    }
    let fully = m.extra.variant >= 3;
    if now >= next && !fully {
        let new = weather_next(m.extra.variant);
        m.extra.variant = new;
        m.extra.vector[0] = if new >= 3 {
            0.0
        } else {
            (next + interval(&mut draw)) as f64
        };
    }
    if fully {
        let p = [
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        ];
        if w.facts.is_air(w.world.block(p[0], p[1], p[2])) && lr.next_float() <= 0.0058 {
            events.push(MobEvent::BecomeStatue { mob: m.id });
        }
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
