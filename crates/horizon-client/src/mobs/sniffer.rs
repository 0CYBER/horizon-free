//! Sniffers (`Sniffer`, `SnifferAi`): the brain's core, idle, sniff and dig
//! activities, scenting and sniffing for diggable ground, walking there,
//! digging up a seed, the explored positions, laying eggs when bred, and
//! `SnifferModel`'s keyframe animations.
use super::{
    Mob, MobEvent, MobWorld,
    anim::{Poses, State},
    brain::{Sink, Ttl, WalkTarget, duration},
    goals::{Breed, Cx, Float, Goal, Panic, Tempt, goal_clone},
    keyframes::Animation,
};
use horizon_core::random::RandomSource;
use std::collections::HashMap;

// Sniffer state: `variant` the state (below), `scratch[0]` the state the
// animations last saw, `positions` the explored positions (newest first),
// `ints[3]` the tick the dug seed drops. AnimState states: feeling happy,
// scenting, sniffing, digging, rising.
pub const IDLING: u8 = 0;
pub const FEELING_HAPPY: u8 = 1;
pub const SCENTING: u8 = 2;
pub const SNIFFING: u8 = 3;
pub const SEARCHING: u8 = 4;
pub const DIGGING: u8 = 5;
pub const RISING: u8 = 6;

const SNIFF_COOLDOWN: i32 = 9600;
/// `Sniffer.getBabyStartAge`.
pub const BABY_START_AGE: i32 = -48000;

pub fn state(m: &Mob) -> u8 {
    m.extra.variant
}
fn set_state(m: &mut Mob, s: u8) {
    m.extra.variant = s;
}
/// `Sniffer.getDefaultDimensions`: lower while digging.
pub fn digging(m: &Mob) -> bool {
    state(m) == DIGGING
}
pub fn register(m: &mut Mob) {
    use super::path::PathType;
    m.ai.nav.can_float = true;
    m.set_malus(PathType::Water, -1.0);
    m.set_malus(PathType::OnTopOfPowderSnow, -1.0);
    m.set_malus(PathType::DamageCautious, -1.0);
    let tempt = m.kind.tempt.clone();
    let g = &mut m.ai.goals;
    g.add(0, Float);
    g.add(0, SnifferPanic(Panic::new(2.0)));
    g.add(
        0,
        Brain {
            walk: None,
            sink: Sink::new(500, 700),
            look: None,
            look_until: 0,
            digging: false,
            happy: false,
            cooldown: Ttl::default(),
            sniff_target: None,
            run: Run::None,
            until: 0,
            breed: Breed::new(1.0),
            tempt: Tempt::new(1.25, tempt, false),
            love_on: false,
            tempt_on: false,
        },
    );
}
/// The core's `AnimalPanic`, forgetting any sniffing when it starts.
#[derive(Clone, Debug)]
struct SnifferPanic(Panic);
impl Goal for SnifferPanic {
    goal_clone!();
    fn flags(&self) -> u8 {
        self.0.flags()
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_use(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_continue(m, cx)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        set_state(m, IDLING);
        self.0.start(m, cx)
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.stop(m, cx)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Run {
    None,
    Digging,
    FinishedDigging,
    Searching,
    Happy,
    Scenting,
    Sniffing,
    DoNothing,
}
#[derive(Clone, Debug)]
struct Brain {
    walk: Option<WalkTarget>,
    sink: Sink,
    /// `LOOK_TARGET` and the end of `LookAtTargetSink`'s run (0 when not running).
    look: Option<[f64; 3]>,
    look_until: i32,
    /// `SNIFFER_DIGGING`, `SNIFFER_HAPPY`, `SNIFF_COOLDOWN`, `SNIFFER_SNIFFING_TARGET`.
    digging: bool,
    happy: bool,
    cooldown: Ttl,
    sniff_target: Option<[i32; 3]>,
    /// The timed behaviour running and when it times out.
    run: Run,
    until: i32,
    breed: Breed,
    tempt: Tempt,
    love_on: bool,
    tempt_on: bool,
}
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
        self.brain(m, cx);
    }
}
fn now(m: &Mob) -> i32 {
    m.age as i32
}
fn sound(m: &Mob, cx: &mut Cx, name: &'static str, pitch: f32) {
    cx.events
        .push(MobEvent::Sound(name, m.center(), 1.0, pitch));
}
/// `Sniffer.getOnPos`: the block under its feet.
fn on_pos(m: &Mob) -> [i32; 3] {
    [
        m.position.x.floor() as i32,
        (m.position.y - 0.2).floor() as i32,
        m.position.z.floor() as i32,
    ]
}
/// `Sniffer.getHeadBlock`: 2.25 blocks ahead of it, a little above its feet.
pub fn head_block(m: &Mob) -> [i32; 3] {
    let (yaw, pitch) = (m.yaw.to_radians(), m.head_pitch.to_radians());
    let forward = [
        f64::from(-yaw.sin() * pitch.cos()),
        f64::from(-pitch.sin()),
        f64::from(yaw.cos() * pitch.cos()),
    ];
    [
        (m.position.x + forward[0] * 2.25).floor() as i32,
        (m.position.y + f64::from(0.2_f32)).floor() as i32,
        (m.position.z + forward[2] * 2.25).floor() as i32,
    ]
}
impl Brain {
    fn tempted(&self) -> bool {
        self.tempt_on
    }
    /// `Sniffer.canSniff`.
    fn can_sniff(&self, m: &Mob) -> bool {
        !self.tempted()
            && !m.ai.panicking
            && !m.fluids.in_water
            && m.extra.in_love <= 0
            && m.on_ground
            && m.rider.is_none()
            && m.extra.leash.is_none()
    }
    /// `Sniffer.canDig(BlockPos)`: diggable, unexplored and reachable.
    fn can_dig_at(&self, m: &mut Mob, w: &MobWorld, p: [i32; 3]) -> bool {
        w.has_tag(
            "minecraft:sniffer_diggable_block",
            w.block_name(p[0], p[1], p[2]),
        ) && !m.extra.positions.contains(&p)
            && m.create_path_to(w, p, 1).is_some_and(|path| path.reached)
    }
    /// `Sniffer.canDig`.
    fn can_dig(&self, m: &mut Mob, w: &MobWorld) -> bool {
        if m.ai.panicking
            || self.tempted()
            || m.is_baby()
            || m.fluids.in_water
            || !m.on_ground
            || m.rider.is_some()
        {
            return false;
        }
        let head = head_block(m);
        self.can_dig_at(m, w, [head[0], head[1] - 1, head[2]])
    }
    /// `Sniffer.calculateDigPosition`.
    fn dig_position(&self, m: &mut Mob, w: &MobWorld) -> Option<[i32; 3]> {
        for idx in 0..5 {
            let Some(p) = m.land_random_pos(w, 10 + 2 * idx, 3) else {
                continue;
            };
            let below = [
                p[0].floor() as i32,
                p[1].floor() as i32 - 1,
                p[2].floor() as i32,
            ];
            if self.can_dig_at(m, w, below) {
                return Some(below);
            }
        }
        None
    }
    fn transition(&mut self, m: &mut Mob, cx: &mut Cx, to: u8) {
        match to {
            FEELING_HAPPY => sound(m, cx, "entity.sniffer.happy", 1.0),
            SCENTING => {
                let pitch = if m.is_baby() { 1.3 } else { 1.0 };
                sound(m, cx, "entity.sniffer.scenting", pitch)
            }
            SNIFFING => sound(m, cx, "entity.sniffer.sniffing", 1.0),
            DIGGING => m.extra.ints[3] = now(m) + 120,
            RISING => sound(m, cx, "entity.sniffer.digging_stop", 1.0),
            _ => {}
        }
        set_state(m, to);
    }
    /// `SnifferAi.resetSniffing`.
    fn reset_sniffing(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.digging = false;
        self.sniff_target = None;
        self.transition(m, cx, IDLING);
    }
    fn brain(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.cooldown.tick();
        // Tempted: the tempting player memory while the goal runs.
        if m.ai.panicking {
            self.walk = None;
            self.sink.stop(m, &mut self.walk);
            self.love_on = false;
            self.tempt_on = false;
        } else {
            self.start_behaviours(m, cx);
        }
        self.sink.tick(m, cx, &mut self.walk);
        self.tick_running(m, cx);
    }
    /// `Brain.startEachNonRunningBehavior` for the first valid activity.
    fn start_behaviours(&mut self, m: &mut Mob, cx: &mut Cx) {
        let n = now(m);
        // DIG, SNIFF, IDLE.
        let dig = self.digging && self.walk.is_none();
        let sniff = self.sniff_target.is_some() && self.walk.is_some();
        if dig {
            if self.run == Run::None && !self.cooldown.present() && self.can_sniff(m) {
                self.run = Run::Digging;
                self.until = n + duration(m, 160, 180);
                self.transition(m, cx, DIGGING);
            }
            if self.run == Run::None && self.cooldown.present() {
                self.run = Run::FinishedDigging;
                self.until = n + 40;
                self.transition(m, cx, RISING);
            }
            return;
        }
        if sniff {
            if self.run == Run::None && self.can_sniff(m) {
                self.run = Run::Searching;
                self.until = n + 600;
                self.transition(m, cx, SEARCHING);
            }
            return;
        }
        if self.digging {
            return;
        }
        // IDLE: love, tempt, look at the target, feeling happy, one of the rest.
        if !self.love_on && !self.tempt_on && self.breed.can_use(m, cx) {
            self.reset_sniffing(m, cx);
            self.breed.start(m, cx);
            self.love_on = true;
        }
        if !self.love_on && !self.tempt_on && self.tempt.can_use(m, cx) {
            self.reset_sniffing(m, cx);
            self.tempt.start(m, cx);
            self.tempt_on = true;
        }
        if self.look.is_some() && self.look_until == 0 {
            self.look_until = n + duration(m, 45, 90);
        }
        if self.run == Run::None && self.happy {
            self.run = Run::Happy;
            self.until = n + duration(m, 40, 100);
            self.transition(m, cx, FEELING_HAPPY);
        }
        if self.run == Run::None {
            self.run_one(m, cx);
        }
    }
    /// The idle `RunOne`: a weighted random pick among what can start.
    fn run_one(&mut self, m: &mut Mob, cx: &mut Cx) {
        let n = now(m);
        // (weight, option)
        let mut options: Vec<(i32, u8)> = Vec::new();
        if self.look.is_some() && self.walk.is_none() {
            options.push((2, 0));
        }
        if !self.happy && !self.love_on && !self.tempted() && self.sniff_target.is_none() {
            options.push((1, 1));
        }
        if self.walk.is_none()
            && self.sniff_target.is_none()
            && !self.cooldown.present()
            && !m.is_baby()
            && self.can_sniff(m)
        {
            options.push((1, 2));
        }
        if cx.w.player.as_ref().is_some_and(|p| {
            p.alive && !p.creative && m.distance_sqr(p.position) <= 36.0 && !p.spectator
        }) {
            options.push((1, 3));
        }
        if self.walk.is_none() {
            options.push((1, 4));
        }
        options.push((2, 5));
        let total: i32 = options.iter().map(|o| o.0).sum();
        // A weighted pick from the high bits (`WeightedList.getRandom`).
        let mut roll = ((m.random.next_float() * total as f32) as i32).min(total - 1);
        let mut pick = options[0].1;
        for (w, o) in &options {
            if roll < *w {
                pick = *o;
                break;
            }
            roll -= w;
        }
        match pick {
            // SetWalkTargetFromLookTarget(1.0, 3).
            0 => {
                if let Some(l) = self.look {
                    self.walk = Some(WalkTarget::new(
                        [
                            l[0].floor() as i32,
                            l[1].floor() as i32,
                            l[2].floor() as i32,
                        ],
                        1.0,
                        3,
                    ));
                }
            }
            1 => {
                self.run = Run::Scenting;
                self.until = n + duration(m, 40, 80);
                self.transition(m, cx, SCENTING);
            }
            2 => {
                self.run = Run::Sniffing;
                self.until = n + duration(m, 40, 80);
                self.transition(m, cx, SNIFFING);
            }
            // SetEntityLookTarget(PLAYER, 6).
            3 => {
                if let Some(p) = &cx.w.player {
                    self.look = Some(p.eye);
                    self.look_until = 0;
                }
            }
            // RandomStroll.stroll(1.0).
            4 => {
                if let Some(p) = m.land_random_pos(cx.w, 10, 7) {
                    self.walk = Some(WalkTarget::new(
                        [
                            p[0].floor() as i32,
                            p[1].floor() as i32,
                            p[2].floor() as i32,
                        ],
                        1.0,
                        0,
                    ));
                }
            }
            _ => {
                self.run = Run::DoNothing;
                self.until = n + duration(m, 5, 20);
            }
        }
    }
    /// `Brain.tickRunningBehaviors`.
    fn tick_running(&mut self, m: &mut Mob, cx: &mut Cx) {
        let n = now(m);
        let timed_out = n > self.until;
        // The delegated idle behaviours.
        if self.love_on {
            if self.breed.can_continue(m, cx) {
                self.breed.tick(m, cx);
            } else {
                self.breed.stop(m, cx);
                self.love_on = false;
            }
        }
        if self.tempt_on {
            self.tempt.stop_distance = if m.is_baby() { 2.5 } else { 3.5 };
            if self.tempt.can_continue(m, cx) {
                self.tempt.tick(m, cx);
            } else {
                self.tempt.stop(m, cx);
                self.tempt_on = false;
            }
        }
        // LookAtTargetSink.
        if self.look_until != 0 {
            if let Some(l) = self.look
                && n <= self.look_until
            {
                m.look_at(l);
            } else {
                self.look = None;
                self.look_until = 0;
            }
        }
        match self.run {
            Run::None => {}
            Run::Digging => {
                let ok = self.digging && self.can_dig(m, cx.w) && m.extra.in_love <= 0;
                if timed_out || !ok {
                    if timed_out {
                        self.cooldown.set(SNIFF_COOLDOWN);
                    } else {
                        self.reset_sniffing(m, cx);
                    }
                    self.run = Run::None;
                }
            }
            Run::FinishedDigging => {
                if timed_out || !self.digging {
                    self.transition(m, cx, IDLING);
                    // onDiggingComplete: remember the spot when it finished.
                    if timed_out {
                        let p = on_pos(m);
                        m.extra.positions.insert(0, p);
                        m.extra.positions.truncate(21);
                    }
                    self.digging = false;
                    self.happy = true;
                    self.run = Run::None;
                }
            }
            Run::Searching => {
                let still = if !self.can_sniff(m) {
                    self.transition(m, cx, IDLING);
                    false
                } else {
                    match (self.walk, self.sniff_target) {
                        (Some(w), Some(t)) => w.pos == t,
                        _ => false,
                    }
                };
                if timed_out || !still {
                    if self.can_dig(m, cx.w) && self.can_sniff(m) {
                        self.digging = true;
                    }
                    self.sink.stop(m, &mut self.walk);
                    self.sniff_target = None;
                    self.run = Run::None;
                }
            }
            Run::Happy => {
                if timed_out {
                    self.transition(m, cx, IDLING);
                    self.happy = false;
                    self.run = Run::None;
                }
            }
            Run::Scenting => {
                if timed_out {
                    self.transition(m, cx, IDLING);
                    self.run = Run::None;
                }
            }
            Run::Sniffing => {
                if timed_out || !self.can_sniff(m) {
                    self.transition(m, cx, IDLING);
                    if timed_out && let Some(p) = self.dig_position(m, cx.w) {
                        self.sniff_target = Some(p);
                        self.walk = Some(WalkTarget::new(p, 1.25, 0));
                    }
                    self.run = Run::None;
                }
            }
            Run::DoNothing => {
                if timed_out {
                    self.run = Run::None;
                }
            }
        }
    }
}
/// `Sniffer.tick` and `Sniffer.aiStep` additions: the animation states,
/// the dropped seed and the water malus.
pub fn ai_step(m: &mut Mob, events: &mut Vec<MobEvent>) {
    use super::path::PathType;
    let age = m.age as f32;
    let current = state(m);
    if current != m.extra.scratch[0] as u8 {
        // resetAnimations, then the state's own.
        for s in &mut m.anim.states[..5] {
            s.stop();
        }
        match current {
            FEELING_HAPPY => m.anim.states[0].start_if_stopped(age),
            SCENTING => m.anim.states[1].start_if_stopped(age),
            SNIFFING => m.anim.states[2].start_if_stopped(age),
            DIGGING => m.anim.states[3].start_if_stopped(age),
            RISING => m.anim.states[4].start_if_stopped(age),
            _ => {}
        }
        m.extra.scratch[0] = i32::from(current);
    }
    if current == DIGGING && now(m) == m.extra.ints[3] {
        let h = head_block(m);
        events.push(MobEvent::GiftLoot {
            table: "minecraft:gameplay/sniffer_digging",
            at: [f64::from(h[0]), f64::from(h[1]), f64::from(h[2])],
        });
        events.push(MobEvent::Sound(
            "entity.sniffer.drop_seed",
            m.center(),
            1.0,
            1.0,
        ));
    }
    if current == SEARCHING && m.age.is_multiple_of(20) {
        events.push(MobEvent::Sound(
            "entity.sniffer.searching",
            m.center(),
            1.0,
            1.0,
        ));
    }
    // onPathfindingStart / Done: water only blocks the way while it is dry.
    let wet = m.fluids.in_water || m.fire_ticks > 0;
    m.set_malus(PathType::Water, if wet { 0.0 } else { -1.0 });
}
/// The brain memory holding the explored positions
/// (`SNIFFER_EXPLORED_POSITIONS`).
pub fn to_nbt(m: &Mob, map: &mut horizon_world::nbt::Compound) {
    use horizon_world::nbt::Tag;
    let list: Vec<Tag> = m
        .extra
        .positions
        .iter()
        .map(|p| {
            Tag::compound([
                ("dimension", Tag::String("minecraft:overworld".into())),
                ("pos", Tag::IntArray(p.to_vec())),
            ])
        })
        .collect();
    let mut memories = horizon_world::nbt::Compound::new();
    if !list.is_empty() {
        memories.insert(
            "minecraft:sniffer_explored_positions".into(),
            Tag::compound([("value", Tag::List(list))]),
        );
    }
    map.insert(
        "Brain".into(),
        Tag::compound([("memories", Tag::Compound(memories))]),
    );
}
pub fn from_nbt(m: &mut Mob, tag: &horizon_world::nbt::Tag) {
    use horizon_world::nbt::Tag;
    let Some(Tag::List(list)) = tag
        .get("Brain")
        .and_then(|b| b.get("memories"))
        .and_then(|b| b.get("minecraft:sniffer_explored_positions"))
        .and_then(|b| b.get("value"))
    else {
        return;
    };
    m.extra.positions = list
        .iter()
        .filter_map(|e| match e.get("pos") {
            Some(Tag::IntArray(p)) if p.len() == 3 => Some([p[0], p[1], p[2]]),
            _ => None,
        })
        .collect();
}
/// `Sniffer.canMate`'s state test.
pub fn can_mate_state(s: u8) -> bool {
    matches!(s, IDLING | SCENTING | FEELING_HAPPY)
}
/// Ambient sound: none while digging or searching.
pub fn ambient(m: &Mob) -> Option<&'static str> {
    (!matches!(state(m), DIGGING | SEARCHING)).then_some("entity.sniffer.idle")
}
/// `SnifferModel.setupAnim` (and `SniffletModel`'s baby transform).
pub fn anim(m: &Mob, s: &State, p: &mut Poses, animations: &HashMap<String, Animation>) {
    let get = |n: &str| animations.get(&format!("SnifferAnimation.{n}"));
    p.xrot("head", s.x_rot.to_radians());
    p.yrot("head", s.y_rot.to_radians());
    let walk = if state(m) == SEARCHING {
        "SNIFFER_SNIFF_SEARCH"
    } else {
        "SNIFFER_WALK"
    };
    if let Some(a) = get(walk) {
        a.apply_walk(s.pos, s.speed, 9.0, 100.0, p);
    }
    let age = s.age;
    for (st, name) in [
        (&m.anim.states[3], "SNIFFER_DIG"),
        (&m.anim.states[2], "SNIFFER_LONGSNIFF"),
        (&m.anim.states[4], "SNIFFER_STAND_UP"),
        (&m.anim.states[0], "SNIFFER_HAPPY"),
        (&m.anim.states[1], "SNIFFER_SNIFFSNIFF"),
    ] {
        if let (Some(ms), Some(a)) = (st.millis(age), get(name)) {
            a.apply(ms, 1.0, p);
        }
    }
    // The baby layer is SniffletModel geometry, but SnifferRenderer wraps it
    // in a plain SnifferModel, so SnifferAnimation.BABY_TRANSFORM never runs.
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_calm_states_can_mate() {
        assert!(can_mate_state(IDLING));
        assert!(can_mate_state(SCENTING));
        assert!(can_mate_state(FEELING_HAPPY));
        assert!(!can_mate_state(SNIFFING));
        assert!(!can_mate_state(DIGGING));
    }
}
