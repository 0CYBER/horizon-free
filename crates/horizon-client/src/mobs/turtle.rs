//! Turtles (`Turtle`): the egg a bred mother carries (`TurtleBreedGoal`), her
//! walk home (`TurtleGoHomeGoal`) and the egg she digs into the sand
//! (`TurtleLayEggGoal`). `extra.flag` is `hasEgg`, `extra.flag2` `isLayingEgg`,
//! `extra.counter` `layEggCounter`, `extra.home` `homePos`, `extra.counter2`
//! `goingHome` and `extra.point` `travelPos`.
use super::{
    Mob, MobEvent, MobWorld,
    goals::{
        Breed, Cx, Goal, JUMP, LookAtPlayer, MOVE, MoveToBlock, Panic, Stroll, Tempt, goal_clone,
        reduced,
    },
    nav::NavKind,
    path::PathType,
};
use crate::blocks::Fluid;
use horizon_core::random::RandomSource;

/// `Turtle.registerGoals`, and the path maluses and navigation `Turtle` and
/// `TurtlePathNavigation` give it.
pub fn register(m: &mut Mob) {
    m.set_malus(PathType::Water, 0.0);
    for t in [
        PathType::DoorIronClosed,
        PathType::DoorWoodClosed,
        PathType::DoorOpen,
    ] {
        m.set_malus(t, -1.0);
    }
    m.ai.nav.kind = NavKind::Amphibious;
    let food = m.kind.food.clone();
    let g = &mut m.ai.goals;
    g.add(0, Panic::new(1.2).looking_for_water(7));
    g.add(1, Breed::new(1.0));
    g.add(1, LayEgg(MoveToBlock::new(1.0, 16, 1)));
    g.add(2, Tempt::new(1.1, food, false));
    g.add(
        3,
        GoToWater(MoveToBlock {
            vertical_start: -1,
            ..MoveToBlock::new(1.0, 24, 1)
        }),
    );
    g.add(4, GoHome::default());
    g.add(7, Travel::default());
    g.add(8, LookAtPlayer::new(8.0));
    g.add(9, RandomStroll(Stroll::new(1.0).interval(100)));
}
/// `Turtle.goingHome`.
fn going_home(m: &Mob) -> bool {
    m.extra.counter2 != 0
}
fn set_going_home(m: &mut Mob, on: bool) {
    m.extra.counter2 = i32::from(on);
}
/// `Turtle.getWalkTargetValue`; `light` is `getPathfindingCostFromLightLevels`.
pub fn walk_target_value(m: &Mob, w: &MobWorld, p: [i32; 3], light: f32) -> f32 {
    if !going_home(m) && w.fluid(p[0], p[1], p[2]) == Fluid::Water
        || is_sand(w, [p[0], p[1] - 1, p[2]])
    {
        10.0
    } else {
        light
    }
}
/// `TurtlePathNavigation.isStableDestination` while travelling: water only.
/// `None` without a `travelPos`, the amphibious rule then.
pub fn stable_destination(m: &Mob, w: &MobWorld, p: [i32; 3]) -> Option<bool> {
    m.extra
        .point
        .map(|_| w.block_name(p[0], p[1], p[2]) == "minecraft:water")
}
/// `Turtle.hasEgg`.
pub fn has_egg(m: &Mob) -> bool {
    m.extra.flag
}
/// `Turtle.setLayingEgg`.
fn set_laying_egg(m: &mut Mob, on: bool) {
    m.extra.counter = i32::from(on);
    m.extra.flag2 = on;
}
/// `homePos.closerToCenterThan(position, distance)`. Spawning, hatching and
/// loading all set a home; one made any other way is at home where it stands.
fn close_to_home(m: &Mob, distance: f64) -> bool {
    let home = m.extra.home.unwrap_or([
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    ]);
    let d = [
        f64::from(home[0]) + 0.5 - m.position.x,
        f64::from(home[1]) + 0.5 - m.position.y,
        f64::from(home[2]) + 0.5 - m.position.z,
    ];
    d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < distance * distance
}
fn block_of(m: &Mob) -> [i32; 3] {
    [
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    ]
}
/// `TurtleEggBlock.isSand`.
fn is_sand(w: &MobWorld, p: [i32; 3]) -> bool {
    w.has_tag("minecraft:sand", w.block_name(p[0], p[1], p[2]))
}
/// `Turtle.aiStep`: while digging, sand crumbs fly every fifth tick.
pub fn ai_step(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    if m.alive() && m.extra.flag2 && m.extra.counter >= 1 && m.extra.counter % 5 == 0 {
        let at = block_of(m);
        let below = [at[0], at[1] - 1, at[2]];
        if is_sand(w, below) {
            events.push(MobEvent::BlockBreakEffect {
                pos: at,
                block: below,
            });
        }
    }
}

/// `Turtle.TurtleLayEggGoal`: a mother with an egg within 9 blocks of home
/// walks to sand with air above, digs for 200 ticks and lays it.
#[derive(Clone, Debug)]
struct LayEgg(MoveToBlock);
impl LayEgg {
    /// `isValidTarget`: sand with air above it.
    fn valid(w: &MobWorld, p: [i32; 3]) -> bool {
        w.facts.is_air(w.world.block(p[0], p[1] + 1, p[2])) && is_sand(w, p)
    }
}
impl Goal for LayEgg {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | JUMP
    }
    fn name(&self) -> &'static str {
        "TurtleLayEggGoal"
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if !(has_egg(m) && close_to_home(m, 9.0)) {
            return false;
        }
        // MoveToBlockGoal.canUse.
        if self.0.next_start > 0 {
            self.0.next_start -= 1;
            return false;
        }
        self.0.next_start = reduced(200 + m.random.next_int_bound(200).unwrap_or(0));
        self.0.find_nearest(m, cx.w, Self::valid)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_continue(cx.w, Self::valid) && has_egg(m) && close_to_home(m, 9.0)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.start(m, cx.w);
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.tick(m, cx.w);
        if m.fluids.in_water || !self.0.reached {
            return;
        }
        if m.extra.counter < 1 {
            set_laying_egg(m, true);
        } else if m.extra.counter > 200 {
            let at = [m.position.x, m.position.y, m.position.z];
            let pitch = 0.9 + m.random.next_float() * 0.2;
            cx.events
                .push(MobEvent::Sound("entity.turtle.lay_egg", at, 0.3, pitch));
            // Blocks.TURTLE_EGG with EGGS = nextInt(4) + 1 above the sand.
            let eggs = m.random.next_int_bound(4).unwrap_or(0) + 1;
            let registry = cx.w.registry;
            let state = registry
                .default_state("minecraft:turtle_egg")
                .and_then(|d| registry.state(d).cloned())
                .and_then(|mut s| {
                    s.properties.insert("eggs".into(), eggs.to_string());
                    registry.resolve(&s).ok()
                });
            let b = self.0.block;
            if let Some(state) = state {
                cx.events.push(MobEvent::SetBlock {
                    pos: [b[0], b[1] + 1, b[2]],
                    state,
                });
            }
            m.extra.flag = false;
            set_laying_egg(m, false);
            m.extra.in_love = 600;
        }
        if m.extra.flag2 {
            m.extra.counter += 1;
        }
    }
}

/// `Turtle.TurtleGoHomeGoal`: a mother with an egg heads home; any adult
/// that strayed 64 blocks away does now and then.
#[derive(Clone, Debug, Default)]
struct GoHome {
    stuck: bool,
    close_to_home_try_ticks: i32,
}
impl Goal for GoHome {
    goal_clone!();
    fn flags(&self) -> u8 {
        0
    }
    fn name(&self) -> &'static str {
        "TurtleGoHomeGoal"
    }
    fn can_use(&mut self, m: &mut Mob, _cx: &mut Cx) -> bool {
        if m.is_baby() {
            false
        } else if has_egg(m) {
            true
        } else {
            m.random.next_int_bound(reduced(700)).unwrap_or(1) == 0 && !close_to_home(m, 64.0)
        }
    }
    fn start(&mut self, m: &mut Mob, _cx: &mut Cx) {
        set_going_home(m, true);
        self.stuck = false;
        self.close_to_home_try_ticks = 0;
    }
    fn stop(&mut self, m: &mut Mob, _cx: &mut Cx) {
        set_going_home(m, false);
    }
    fn can_continue(&mut self, m: &mut Mob, _cx: &mut Cx) -> bool {
        !close_to_home(m, 7.0) && !self.stuck && self.close_to_home_try_ticks <= reduced(600)
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let home = m.extra.home.unwrap_or(block_of(m));
        let close = close_to_home(m, 16.0);
        if close {
            self.close_to_home_try_ticks += 1;
        }
        if !m.ai.nav.is_done() {
            return;
        }
        let to = [
            f64::from(home[0]) + 0.5,
            f64::from(home[1]),
            f64::from(home[2]) + 0.5,
        ];
        let mut next = m
            .default_random_pos_towards(cx.w, 16, 3, to, std::f64::consts::PI / 10.0)
            .or_else(|| m.default_random_pos_towards(cx.w, 8, 7, to, std::f64::consts::FRAC_PI_2));
        if let Some(at) = next
            && !close
            && cx.w.block_name(
                at[0].floor() as i32,
                at[1].floor() as i32,
                at[2].floor() as i32,
            ) != "minecraft:water"
        {
            next = m.default_random_pos_towards(cx.w, 16, 5, to, std::f64::consts::FRAC_PI_2);
        }
        match next {
            Some(at) => {
                m.nav_move_to(cx.w, at, 1.0);
            }
            None => self.stuck = true,
        }
    }
}

/// `Turtle.TurtleGoToWaterGoal`: a turtle on land walks to water within 24
/// blocks, unless it is going home or carries an egg.
#[derive(Clone, Debug)]
struct GoToWater(MoveToBlock);
impl GoToWater {
    fn valid(w: &MobWorld, p: [i32; 3]) -> bool {
        w.block_name(p[0], p[1], p[2]) == "minecraft:water"
    }
}
impl Goal for GoToWater {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | JUMP
    }
    fn name(&self) -> &'static str {
        "TurtleGoToWaterGoal"
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let wanted = if m.is_baby() && !m.fluids.in_water {
            true
        } else {
            !going_home(m) && !m.fluids.in_water && !has_egg(m)
        };
        if !wanted {
            return false;
        }
        // MoveToBlockGoal.canUse.
        if self.0.next_start > 0 {
            self.0.next_start -= 1;
            return false;
        }
        self.0.next_start = reduced(200 + m.random.next_int_bound(200).unwrap_or(0));
        self.0.find_nearest(m, cx.w, Self::valid)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        !m.fluids.in_water && self.0.try_ticks <= 1200 && Self::valid(cx.w, self.0.block)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.start(m, cx.w);
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        // MoveToBlockGoal.tick, with shouldRecalculatePath every 160 ticks.
        let b = self.0.block;
        let d = [
            f64::from(b[0]) + 0.5 - m.position.x,
            f64::from(b[1] + 1) + 0.5 - m.position.y,
            f64::from(b[2]) + 0.5 - m.position.z,
        ];
        if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] >= 1.0 {
            self.0.reached = false;
            self.0.try_ticks += 1;
            if self.0.try_ticks % 160 == 0 {
                let at = [
                    f64::from(b[0]) + 0.5,
                    f64::from(b[1] + 1),
                    f64::from(b[2]) + 0.5,
                ];
                m.nav_move_to(cx.w, at, self.0.speed);
            }
        } else {
            self.0.reached = true;
            self.0.try_ticks -= 1;
        }
    }
}

/// `Turtle.TurtleTravelGoal`: a turtle in water swims towards a spot up to
/// 512 blocks away, a leg at a time.
#[derive(Clone, Debug, Default)]
struct Travel {
    stuck: bool,
}
impl Goal for Travel {
    goal_clone!();
    fn flags(&self) -> u8 {
        0
    }
    fn name(&self) -> &'static str {
        "TurtleTravelGoal"
    }
    fn can_use(&mut self, m: &mut Mob, _cx: &mut Cx) -> bool {
        !going_home(m) && !has_egg(m) && m.fluids.in_water
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        let xt = m.random.next_int_bound(1025).unwrap_or(0) - 512;
        let mut yt = m.random.next_int_bound(9).unwrap_or(0) - 4;
        let zt = m.random.next_int_bound(1025).unwrap_or(0) - 512;
        if f64::from(yt) + m.position.y > f64::from(cx.w.sea_level - 1) {
            yt = 0;
        }
        m.extra.point = Some([
            (f64::from(xt) + m.position.x).floor(),
            (f64::from(yt) + m.position.y).floor(),
            (f64::from(zt) + m.position.z).floor(),
        ]);
        self.stuck = false;
    }
    fn can_continue(&mut self, m: &mut Mob, _cx: &mut Cx) -> bool {
        !m.ai.nav.is_done() && !self.stuck && !going_home(m) && m.extra.in_love <= 0 && !has_egg(m)
    }
    fn stop(&mut self, m: &mut Mob, _cx: &mut Cx) {
        m.extra.point = None;
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(travel) = m.extra.point else {
            self.stuck = true;
            return;
        };
        if !m.ai.nav.is_done() {
            return;
        }
        // Vec3.atBottomCenterOf(travelPos).
        let to = [travel[0] + 0.5, travel[1], travel[2] + 0.5];
        let tenth = f64::from((std::f64::consts::PI / 10.0) as f32);
        let half = f64::from(std::f32::consts::FRAC_PI_2);
        let mut next = m
            .default_random_pos_towards(cx.w, 16, 3, to, tenth)
            .or_else(|| m.default_random_pos_towards(cx.w, 8, 7, to, half));
        if let Some(n) = next {
            let (xc, zc) = (n[0].floor() as i32, n[2].floor() as i32);
            // Level.hasChunksAt(xc - 34, zc - 34, xc + 34, zc + 34).
            let loaded = ((xc - 34) >> 4..=(xc + 34) >> 4).all(|chunk_x| {
                ((zc - 34) >> 4..=(zc + 34) >> 4)
                    .all(|chunk_z| cx.w.world.chunks.contains_key(&(chunk_x, chunk_z)))
            });
            if !loaded {
                next = None;
            }
        }
        match next {
            Some(at) => {
                m.nav_move_to(cx.w, at, 1.0);
            }
            None => self.stuck = true,
        }
    }
}

/// `Turtle.TurtleRandomStrollGoal`: no stroll in water, going home or with an
/// egg.
#[derive(Clone, Debug)]
struct RandomStroll(Stroll);
impl Goal for RandomStroll {
    goal_clone!();
    fn flags(&self) -> u8 {
        self.0.flags()
    }
    fn name(&self) -> &'static str {
        "TurtleRandomStrollGoal"
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        !m.fluids.in_water && !going_home(m) && !has_egg(m) && self.0.can_use(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_continue(m, cx)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.start(m, cx);
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.stop(m, cx);
    }
}
