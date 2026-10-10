//! Pieces of vanilla's `Brain` that species implemented as brains share:
//! memories that expire, behaviour durations, walk targets and
//! `MoveToTargetSink`.
use super::{Mob, goals::Cx};
use horizon_core::random::RandomSource;

/// A memory with an expiry (`Brain.setMemoryWithExpiry`): present for the
/// tick it was set and `ttl` more ticks. `Ttl::default()` is absent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ttl(i32);
impl Ttl {
    pub fn set(&mut self, ttl: i32) {
        self.0 = ttl + 1;
    }
    pub fn erase(&mut self) {
        self.0 = 0;
    }
    pub fn present(self) -> bool {
        self.0 > 0
    }
    /// `Brain.forgetOutdatedMemories`, once at the start of each tick.
    pub fn tick(&mut self) {
        if self.0 > 0 {
            self.0 -= 1;
        }
    }
}
/// A random duration in `min..=max` (`Behavior`'s end time).
pub fn duration(m: &mut Mob, min: i32, max: i32) -> i32 {
    if max <= min {
        return min;
    }
    min + m.random.next_int_bound(max - min + 1).unwrap_or(0)
}
/// `WalkTarget`: where to go, how fast and how close counts as there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WalkTarget {
    pub pos: [i32; 3],
    pub speed: f64,
    pub close: i32,
}
impl WalkTarget {
    pub fn new(pos: [i32; 3], speed: f64, close: i32) -> Self {
        Self { pos, speed, close }
    }
    /// `MoveToTargetSink.reachedTarget`: Manhattan distance of blocks.
    pub fn reached(&self, m: &Mob) -> bool {
        let here = [
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        ];
        let d = (self.pos[0] - here[0]).abs()
            + (self.pos[1] - here[1]).abs()
            + (self.pos[2] - here[2]).abs();
        d <= self.close
    }
}
/// `MoveToTargetSink(min, max)`: walks to the walk target and gives up on it
/// after a random time.
#[derive(Clone, Debug)]
pub struct Sink {
    min: i32,
    max: i32,
    until: Option<i32>,
}
impl Sink {
    pub fn new(min: i32, max: i32) -> Self {
        Self {
            min,
            max,
            until: None,
        }
    }
    pub fn running(&self) -> bool {
        self.until.is_some()
    }
    /// `stop`: forget the walk target and the path.
    pub fn stop(&mut self, m: &mut Mob, walk: &mut Option<WalkTarget>) {
        if self.until.take().is_some() {
            m.nav_stop();
        }
        *walk = None;
    }
    /// One tick of `checkExtraStartConditions`, `canStillUse`, `tick` and
    /// `stop` for the walk target.
    pub fn tick(&mut self, m: &mut Mob, cx: &mut Cx, walk: &mut Option<WalkTarget>) {
        let now = m.age as i32;
        if let Some(until) = self.until {
            let still = !m.ai.nav.is_done() && walk.is_some_and(|w| !w.reached(m));
            if now > until || !still {
                self.stop(m, walk);
            }
            return;
        }
        let Some(target) = *walk else { return };
        if target.reached(m) {
            *walk = None;
            return;
        }
        let at = [
            f64::from(target.pos[0]) + 0.5,
            f64::from(target.pos[1]),
            f64::from(target.pos[2]) + 0.5,
        ];
        let mut moving = m.nav_move_to(cx.w, at, target.speed);
        if !moving {
            // A partial step toward it (`DefaultRandomPos.getPosTowards`).
            moving = m
                .default_random_pos_towards(cx.w, 10, 7, at, std::f64::consts::FRAC_PI_2)
                .is_some_and(|p| m.nav_move_to(cx.w, p, target.speed));
        }
        if moving {
            self.until = Some(now + duration(m, self.min, self.max));
        } else {
            *walk = None;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_memory_lasts_its_expiry_and_the_tick_it_was_set() {
        let mut t = Ttl::default();
        assert!(!t.present());
        t.set(2);
        let mut ticks = 0;
        while t.present() {
            ticks += 1;
            t.tick();
        }
        assert_eq!(ticks, 3);
    }
}
