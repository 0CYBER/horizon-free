//! `RandomStrollGoal` and its subclasses (`WaterAvoidingRandomStrollGoal`,
//! `RandomSwimmingGoal`, `WaterAvoidingRandomFlyingGoal`,
//! `Parrot.ParrotWanderGoal`) and `RandomStroll.fly` for an allay's brain
//! behaviour.
use super::*;

/// RandomStrollGoal / WaterAvoidingRandomStrollGoal.
#[derive(Clone, Debug)]
pub struct Stroll {
    pub speed: f64,
    pub interval: i32,
    pub check_no_action: bool,
    /// WaterAvoidingRandomStrollGoal probability of a plain stroll.
    pub water_avoiding: Option<f32>,
    /// RandomSwimmingGoal: BehaviorUtils.getRandomSwimmablePos.
    pub swim: bool,
    /// WaterAvoidingRandomFlyingGoal: a hover spot, else an air or water pos.
    pub flying: bool,
    /// `RandomStroll.fly` (`AllayAi`): an air or water spot within 10 by 7,
    /// 2 below, with no hover spot tried first.
    pub fly: bool,
    /// `Parrot.ParrotWanderGoal`: to a tree, else as `flying`.
    pub parrot: bool,
    /// `setFlags(MOVE, LOOK)`, which only `Guardian.registerGoals` adds.
    pub look: bool,
    /// `RandomStrollGoal.forceTrigger`: the next `canUse` skips the interval's
    /// draw (see `Goal::trigger`).
    pub force: bool,
    wanted: [f64; 3],
    /// A subclass's `getPosition` in place of the default pick
    /// (`GolemRandomStrollInVillageGoal`, `MoveBackToVillageGoal`).
    position: Option<fn(&mut Mob, &mut Cx) -> Option<[f64; 3]>>,
    /// A subclass's own `canUse` test, run before the interval's draw.
    gate: Option<fn(&Mob, &MobWorld) -> bool>,
    label: Option<&'static str>,
}
impl Stroll {
    /// `RandomStrollGoal.<init>` (interval 120, checks the no-action time).
    pub fn new(speed: f64) -> Self {
        Self {
            speed,
            interval: 120,
            check_no_action: true,
            water_avoiding: None,
            swim: false,
            flying: false,
            fly: false,
            parrot: false,
            look: false,
            force: false,
            wanted: [0.0; 3],
            position: None,
            gate: None,
            label: None,
        }
    }
    /// The stroll's spot comes from `f` (an overridden `getPosition`).
    pub fn position(mut self, f: fn(&mut Mob, &mut Cx) -> Option<[f64; 3]>) -> Self {
        self.position = Some(f);
        self
    }
    /// The stroll may only start while `f` holds (an overridden `canUse`).
    pub fn gate(mut self, f: fn(&Mob, &MobWorld) -> bool) -> Self {
        self.gate = Some(f);
        self
    }
    /// The goal's class name, for debugging.
    pub fn named(mut self, label: &'static str) -> Self {
        self.label = Some(label);
        self
    }
    pub fn avoiding_water(speed: f64) -> Self {
        Self {
            water_avoiding: Some(0.001),
            ..Self::new(speed)
        }
    }
    pub fn interval(mut self, i: i32) -> Self {
        self.interval = i;
        self
    }
    /// WaterAvoidingRandomFlyingGoal (a nearby air position).
    pub fn flying(speed: f64) -> Self {
        Self {
            flying: true,
            ..Self::new(speed)
        }
    }
    /// `RandomStroll.fly(speed)`: `getTargetFlyPos`, an air or water spot
    /// within 10 by 7 (`AirAndWaterRandomPos.getPos(body, 10, 7, -2, ...)`).
    pub fn fly(speed: f64) -> Self {
        Self {
            fly: true,
            ..Self::new(speed)
        }
    }
    /// `setFlags(MOVE, LOOK)`: the stroll keeps the head too (the guardian's).
    pub fn looking(mut self) -> Self {
        self.look = true;
        self
    }
    /// `Parrot.ParrotWanderGoal(speed)`, a WaterAvoidingRandomFlyingGoal.
    pub fn parrot(speed: f64) -> Self {
        Self {
            water_avoiding: Some(0.001),
            parrot: true,
            ..Self::new(speed)
        }
    }
    /// RandomSwimmingGoal(speed, interval)
    pub fn swimming(speed: f64, interval: i32) -> Self {
        Self {
            swim: true,
            ..Self::new(speed).interval(interval)
        }
    }
}
impl Goal for Stroll {
    goal_clone!();
    fn name(&self) -> &'static str {
        if let Some(label) = self.label {
            label
        } else if self.parrot {
            "ParrotWander"
        } else if self.water_avoiding.is_some() {
            "Stroll"
        } else if self.swim {
            "RandomSwimming"
        } else {
            "RandomStroll"
        }
    }
    fn flags(&self) -> u8 {
        if self.look { MOVE | LOOK } else { MOVE }
    }
    /// `RandomStrollGoal.trigger`.
    fn trigger(&mut self) {
        self.force = true;
    }
    /// RandomStrollGoal.canUse.
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if crate::mobs::riders::mob_controlled(m, cx.w) {
            return false;
        }
        if self.gate.is_some_and(|gate| !gate(m, cx.w)) {
            return false;
        }
        if !self.force {
            if self.check_no_action && m.ai.no_action_time >= 100 {
                return false;
            }
            if m.random.next_int_bound(reduced(self.interval)).unwrap_or(1) != 0 {
                return false;
            }
        }
        let pos = match self.water_avoiding {
            _ if self.position.is_some() => self.position.and_then(|pick| pick(m, cx)),
            Some(_) if self.parrot => crate::mobs::parrot::wander_pos(m, cx.w),
            Some(prob) => {
                if m.fluids.in_water {
                    m.land_random_pos(cx.w, 15, 7)
                        .or_else(|| m.default_random_pos(cx.w, 10, 7))
                } else if m.random.next_float() >= prob {
                    m.land_random_pos(cx.w, 10, 7)
                } else {
                    m.default_random_pos(cx.w, 10, 7)
                }
            }
            None if self.swim => {
                let mut p = m.default_random_pos(cx.w, 10, 7);
                let mut count = 0;
                while let Some(q) = p {
                    let s = cx.w.world.block(
                        q[0].floor() as i32,
                        q[1].floor() as i32,
                        q[2].floor() as i32,
                    );
                    if cx
                        .w
                        .data
                        .pathfindable(s, crate::mobs::data::PathComputationType::Water)
                        || count >= 10
                    {
                        break;
                    }
                    count += 1;
                    p = m.default_random_pos(cx.w, 10, 7);
                }
                p
            }
            None if self.flying => m.flying_stroll_pos(cx.w),
            None if self.fly => {
                let view = m.view_vector();
                m.air_and_water_random_pos(cx.w, 10, 7, -2, view[0], view[2])
            }
            None => m.default_random_pos(cx.w, 10, 7),
        };
        let Some(p) = pos else { return false };
        self.wanted = p;
        // `RandomStrollGoal.canUse` clears the trigger once it has a spot.
        self.force = false;
        true
    }
    /// RandomStrollGoal.canContinueToUse.
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        !m.ai.nav.is_done() && !crate::mobs::riders::mob_controlled(m, cx.w)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        m.nav_move_to(cx.w, self.wanted, self.speed);
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.nav_stop();
    }
}
