use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `CanContinueSleepingEvent`: a sleeping living entity's check that it may go on
    /// sleeping, run each tick for a server player the dead and health checks would not
    /// already wake, and for a sleeping villager or wandering trader. `entity` is its UUID,
    /// `name` a player's name (empty for a mob) and `kind` its type (`minecraft:player`).
    /// `problem` is what vanilla found (`getProblem`): `NotPossibleHere` when the bed at `pos`
    /// is gone, `NotPossibleNow` when a player's bed rule no longer lets them sleep (daytime),
    /// none when nothing is wrong. `may_continue` starts empty, meaning `problem.is_none()` (`mayContinueSleeping`);
    /// set it `Some(true)` to keep them asleep, `Some(false)` to wake them. The player's state is not swapped in
    /// on a guest tick.
    pub(in crate::game) fn native_can_continue_sleeping(
        &mut self,
        entity: u128,
        name: &str,
        kind: &str,
        pos: [i32; 3],
        problem: Option<SleepProblem>,
        may_continue: &mut Option<bool>,
    ) {
        self.fire_can_continue_sleeping(entity, name, kind, pos, problem, may_continue);
    }
}
