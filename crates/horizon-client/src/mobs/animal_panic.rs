//! `AnimalPanic`, the panic of the mobs whose vanilla AI is a brain (the
//! frog, goat, allay, camel, nautilus and armadillo), and the Manhattan
//! water search it shares with `PanicGoal`.
use super::*;
use crate::mobs::armadillo;

/// `AnimalPanic`: a panic that runs `PANIC_MIN_DURATION` to
/// `PANIC_MAX_DURATION` ticks and picks a new spot to flee to every time the
/// navigation is done, where `PanicGoal` gives up as soon as it gets there.
#[derive(Clone, Debug)]
pub struct AnimalPanic {
    pub speed: f64,
    /// `ArmadilloAi.ArmadilloPanic`: only `#panic_environmental_causes`.
    pub environmental: bool,
    /// `Behavior`'s `endTimestamp`, on the mob's age.
    end: i64,
}
impl AnimalPanic {
    pub fn new(speed: f64) -> Self {
        Self {
            speed,
            environmental: false,
            end: 0,
        }
    }
    /// `checkExtraStartConditions`: `HURT_BY` (the last damage source, which
    /// `getLastDamageSource` keeps for 40 ticks) in the mob's tag, or already
    /// panicking (`IS_PANICKING`). Being on fire is not a start condition of
    /// its own here: the burning damage it deals sets `HURT_BY`.
    fn wants_panic(&self, m: &Mob) -> bool {
        m.last_damage.is_some_and(|(d, t)| {
            let wanted = if self.environmental {
                armadillo::environmental(d.kind)
            } else {
                d.causes_panic()
            };
            wanted && i64::from(m.age) - t <= 40
        }) || m.ai.panicking
    }
}
impl Goal for AnimalPanic {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn name(&self) -> &'static str {
        "Panic"
    }
    fn can_use(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        self.wants_panic(m)
    }
    fn can_continue(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        // `canStillUse` is true; only `Behavior.timedOut` ends it.
        i64::from(m.age) <= self.end
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        // `Behavior.tryStart`: 100 + level.getRandom().nextInt(21).
        self.end =
            i64::from(m.age) + 100 + i64::from(cx.level.random().next_int_bound(21).unwrap_or(0));
        m.ai.panicking = true;
        // `start`: `eraseMemory(WALK_TARGET)` and `getNavigation().stop()`.
        m.nav_stop();
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        // `Behavior.tickOrStop` checks `timedOut` before ticking, every tick;
        // a goal only hears about the timeout when its selector polls
        // `can_continue`, which is every other tick.
        if i64::from(m.age) > self.end {
            m.ai.panicking = false;
            return;
        }
        if m.ai.nav.is_done()
            && let Some(p) = panic_pos(m, cx.w)
        {
            m.nav_move_to(cx.w, p, self.speed);
        }
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.ai.panicking = false;
    }
    /// A brain behaviour ticks every tick; a goal only every other one.
    fn every_tick(&self) -> bool {
        true
    }
}

/// `AnimalPanic.getPanicPos`: the nearest water while burning, else a land
/// spot 5 blocks out and 4 up or down (`LandRandomPos.getPos`).
fn panic_pos(m: &mut Mob, w: &MobWorld) -> Option<[f64; 3]> {
    if m.fire_ticks > 0
        && let Some(p) = animal_water_pos(m, w, 5)
    {
        // `Vec3.atBottomCenterOf`.
        let (x, y, z) = (
            f64::from(p[0]) + 0.5,
            f64::from(p[1]),
            f64::from(p[2]) + 0.5,
        );
        return Some([x, y, z]);
    }
    m.land_random_pos(w, 5, 4)
}

/// `PanicGoal.lookForWater`: `findBlocksInBoxByManhattanDistance(
/// mob.blockPosition(), rangeXZ, 1)` filtered to water, its first match.
pub fn water_pos(m: &Mob, w: &MobWorld, range: i32) -> Option<[i32; 3]> {
    water_pos_where(m, w, range, |_, _, _| true)
}

/// `AnimalPanic.lookForWater`: a mob whose bounding box rounds up to two
/// blocks wide (`Mth.ceil(getBbWidth()) == 2`, the camel) wants the three
/// blocks south, east and south-east of the spot to be water too.
fn animal_water_pos(m: &Mob, w: &MobWorld, range: i32) -> Option<[i32; 3]> {
    if f64::from(m.width()).ceil() != 2.0 {
        return water_pos(m, w, range);
    }
    let water = |x, y, z| w.fluid(x, y, z) == crate::blocks::Fluid::Water;
    water_pos_where(m, w, range, |x, y, z| {
        water(x, y, z + 1) && water(x + 1, y, z) && water(x + 1, y, z + 1)
    })
}

fn water_pos_where(
    m: &Mob,
    w: &MobWorld,
    range: i32,
    extra: impl Fn(i32, i32, i32) -> bool,
) -> Option<[i32; 3]> {
    let (bx, by, bz) = (
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    );
    // `getCollisionShape(level, pos).isEmpty()`: a mob inside a block sees
    // no water.
    if w.data.state(w.world.block(bx, by, bz)).top != 0.0 {
        return None;
    }
    let water = |x: i32, y: i32, z: i32| w.fluid(x, y, z) == crate::blocks::Fluid::Water;
    // `withinBoxByManhattanDistance(pos, range, 1, range)`: `range` wide and
    // deep, one block tall, nearest first (`MobWorld::nearest_block`).
    w.nearest_block([bx, by, bz], range, 1, |p, _| {
        water(p[0], p[1], p[2]) && extra(p[0], p[1], p[2])
    })
}
