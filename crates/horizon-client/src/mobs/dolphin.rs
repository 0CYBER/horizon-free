//! Dolphin goals: `DolphinSwimToTreasureGoal`, `DolphinSwimWithPlayerGoal`
//! and `DolphinJumpGoal`. `extra.flag` is `gotFish` and `extra.home` the
//! `treasurePos` the game found for it (`MobEvent::DolphinTreasure`).
use super::{
    Mob, MobEvent, Who,
    goals::{Cx, Goal, JUMP, LOOK, MOVE, goal_clone, reduced},
};
use crate::blocks::Fluid;
use horizon_core::random::RandomSource;

/// `Entity.getMotionDirection`: the horizontal step of the yaw
/// (`Direction.fromYRot`: south, west, north, east).
fn motion_step(yaw: f32) -> [i32; 2] {
    match (yaw / 90.0 + 0.5).floor() as i32 & 3 {
        0 => [0, 1],
        1 => [-1, 0],
        2 => [0, -1],
        _ => [1, 0],
    }
}

/// `DolphinSwimToTreasureGoal`: a dolphin that was fed a fish leads the way to
/// a shipwreck or an ocean ruin within 50 sectors.
#[derive(Clone, Debug, Default)]
pub struct SwimToTreasure {
    stuck: bool,
}
impl SwimToTreasure {
    /// `BlockPos.containing(x, dolphin.getY(), z).closerToCenterThan(position, 4)`.
    fn arrived(m: &Mob) -> bool {
        let Some(t) = m.extra.home else { return false };
        let d = [
            f64::from(t[0]) + 0.5 - m.position.x,
            m.position.y.floor() + 0.5 - m.position.y,
            f64::from(t[2]) + 0.5 - m.position.z,
        ];
        d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < 16.0
    }
}
impl Goal for SwimToTreasure {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn interruptable(&self) -> bool {
        false
    }
    fn can_use(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        m.extra.flag && m.air >= 100
    }
    fn can_continue(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        m.extra.home.is_some() && !Self::arrived(m) && !self.stuck && m.air >= 100
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.stuck = false;
        m.nav_stop();
        // The game looks for the structure and sets the position, or none.
        m.extra.home = None;
        cx.events.push(MobEvent::DolphinTreasure { mob: m.id });
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        if m.extra.home.is_none() || Self::arrived(m) || self.stuck {
            m.extra.flag = false;
        }
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(t) = m.extra.home else { return };
        let close = m.ai.nav.target_pos.is_some_and(|p| {
            let d = [
                f64::from(p[0]) + 0.5 - m.position.x,
                f64::from(p[1]) + 0.5 - m.position.y,
                f64::from(p[2]) + 0.5 - m.position.z,
            ];
            d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < 144.0
        });
        if !close && !m.ai.nav.is_done() {
            return;
        }
        let treasure = [
            f64::from(t[0]) + 0.5,
            f64::from(t[1]) + 0.5,
            f64::from(t[2]) + 0.5,
        ];
        let eighth = f64::from(std::f32::consts::FRAC_PI_8);
        let half = f64::from(std::f32::consts::FRAC_PI_2);
        let mut next = m.default_random_pos_towards(cx.w, 16, 1, treasure, eighth);
        if next.is_none() {
            next = m.default_random_pos_towards(cx.w, 8, 4, treasure, half);
        }
        // isPathfindable(WATER) is a water fluid state.
        if let Some(n) = next
            && cx.w.fluid(
                n[0].floor() as i32,
                n[1].floor() as i32,
                n[2].floor() as i32,
            ) != Fluid::Water
        {
            next = m.default_random_pos_towards(cx.w, 8, 5, treasure, half);
        }
        let Some(next) = next else {
            self.stuck = true;
            return;
        };
        let (y, x) = (m.kind.max_head_y_rot + 20.0, m.kind.max_head_x_rot);
        m.look_at_speed(next, y, x);
        m.nav_move_to(cx.w, next, 1.3);
        if m.random.next_int_bound(80).unwrap_or(1) == 0 {
            // Entity event 38: happy villager particles about the dolphin.
            cx.events.push(MobEvent::AroundSelf {
                mob: m.id,
                event: 38,
                kind: "happy_villager",
                feet: [m.position.x, m.position.y, m.position.z],
                width: f64::from(m.width()),
                height: f64::from(m.height()),
                burst: crate::particles::Burst::DOLPHIN,
            });
        }
    }
}

/// `DolphinSwimWithPlayerGoal`: swim up to a swimming player and give them
/// Dolphin's Grace.
#[derive(Clone, Debug)]
pub struct SwimWithPlayer {
    speed: f64,
}
impl SwimWithPlayer {
    pub fn new(speed: f64) -> Self {
        Self { speed }
    }
}
impl Goal for SwimWithPlayer {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        // getNearestPlayer(forNonCombat().range(10).ignoreLineOfSight()).
        m.can_see_player_through_walls(cx.w, 10.0)
            && cx.w.player.as_ref().is_some_and(|p| p.swimming)
            && m.ai.target != Some(Who::Player)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        cx.w.player
            .as_ref()
            .is_some_and(|p| p.swimming && m.distance_sqr(p.position) < 256.0)
    }
    fn start(&mut self, _: &mut Mob, cx: &mut Cx) {
        cx.events.push(grace());
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.nav_stop();
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(p) = cx.w.player.clone() else { return };
        let (y, x) = (m.kind.max_head_y_rot + 20.0, m.kind.max_head_x_rot);
        m.look_at_speed(p.eye, y, x);
        if m.distance_sqr(p.position) < 6.25 {
            m.nav_stop();
        } else {
            m.nav_move_to_entity(
                cx.w,
                [p.position.x, p.position.y, p.position.z],
                1,
                self.speed,
            );
        }
        if p.swimming && m.random.next_int_bound(6).unwrap_or(1) == 0 {
            cx.events.push(grace());
        }
    }
}
/// `MobEffectInstance(DOLPHINS_GRACE, 100)`.
fn grace() -> MobEvent {
    MobEvent::EffectPlayer {
        effect: "minecraft:dolphins_grace",
        ticks: 100,
        amplifier: 0,
    }
}

/// `DolphinJumpGoal`: leap out of clear water and arc back in.
#[derive(Clone, Debug)]
pub struct Jump {
    interval: i32,
    breached: bool,
}
impl Jump {
    pub fn new(interval: i32) -> Self {
        Self {
            interval: reduced(interval),
            breached: false,
        }
    }
}
impl Goal for Jump {
    goal_clone!();
    fn flags(&self) -> u8 {
        JUMP | MOVE
    }
    fn interruptable(&self) -> bool {
        false
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if m.random.next_int_bound(self.interval).unwrap_or(1) != 0 {
            return false;
        }
        let [sx, sz] = motion_step(m.yaw);
        let (bx, by, bz) = (
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        );
        let w = cx.w;
        [0, 1, 4, 5, 6, 7].into_iter().all(|i| {
            let (x, z) = (bx + sx * i, bz + sz * i);
            // waterIsClear, then surfaceIsClear.
            w.fluid(x, by, z) == Fluid::Water
                && !w.has_tag("minecraft:blocks_dolphin_jump", w.block_name(x, by, z))
                && w.facts.is_air(w.world.block(x, by + 1, z))
                && w.facts.is_air(w.world.block(x, by + 2, z))
        })
    }
    fn can_continue(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        let yd = m.velocity.y;
        (yd * yd >= f64::from(0.03f32)
            || m.head_pitch == 0.0
            || m.head_pitch.abs() >= 10.0
            || !m.fluids.in_water)
            && !m.on_ground
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        let [sx, sz] = motion_step(m.yaw);
        m.velocity.x += f64::from(sx) * 0.6;
        m.velocity.y += 0.7;
        m.velocity.z += f64::from(sz) * 0.6;
        m.nav_stop();
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.head_pitch = 0.0;
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let already = self.breached;
        if !already {
            self.breached = cx.w.fluid(
                m.position.x.floor() as i32,
                m.position.y.floor() as i32,
                m.position.z.floor() as i32,
            ) == Fluid::Water;
        }
        if self.breached && !already {
            m.sound("entity.dolphin.jump", cx.events);
        }
        let v = m.velocity;
        if v.y * v.y < f64::from(0.03f32) && m.head_pitch != 0.0 {
            // Mth.rotLerp(0.2, xRot, 0).
            m.head_pitch += 0.2 * super::living::wrap(0.0 - m.head_pitch);
        } else if v.length() > f64::from(1.0e-5f32) {
            let horizontal = v.x.hypot(v.z);
            m.head_pitch = ((-v.y).atan2(horizontal) * f64::from(180.0f32)
                / f64::from(std::f32::consts::PI)) as f32;
        }
    }
}
