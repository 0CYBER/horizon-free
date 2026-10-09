//! `DeltaTracker.Timer` and the tick-rate managers of 26.3: how real
//! milliseconds turn into game ticks, partial ticks and real-time deltas, and
//! the `/tick` freeze, step and sprint state.

/// `DeltaTracker.Timer`: float arithmetic on integer milliseconds, exactly as
/// the client does it.
#[derive(Clone, Debug)]
pub struct Timer {
    delta_ticks: f32,
    delta_tick_residual: f32,
    realtime_delta_ticks: f32,
    paused_delta_tick_residual: f32,
    last_ms: i64,
    last_ui_ms: i64,
    ms_per_tick: f32,
    paused: bool,
    frozen: bool,
}

impl Timer {
    pub fn new(ticks_per_second: f32, current_ms: i64) -> Self {
        Self {
            delta_ticks: 0.0,
            delta_tick_residual: 0.0,
            realtime_delta_ticks: 0.0,
            paused_delta_tick_residual: 0.0,
            last_ms: current_ms,
            last_ui_ms: current_ms,
            ms_per_tick: 1000.0 / ticks_per_second,
            paused: false,
            frozen: false,
        }
    }

    /// `advanceGameTime`: the whole ticks to run now. `target_mspt` maps the
    /// default milliseconds per tick to the target in force (the tick-rate
    /// manager's, when the level runs normally). The caller runs at most ten.
    pub fn advance_game_time(&mut self, current_ms: i64, target_mspt: impl FnOnce(f32) -> f32) -> i32 {
        self.delta_ticks = (current_ms - self.last_ms) as f32 / target_mspt(self.ms_per_tick);
        self.last_ms = current_ms;
        self.delta_tick_residual += self.delta_ticks;
        let ticks = self.delta_tick_residual as i32;
        self.delta_tick_residual -= ticks as f32;
        ticks
    }

    /// `advanceRealTime`.
    pub fn advance_real_time(&mut self, current_ms: i64) {
        self.realtime_delta_ticks = (current_ms - self.last_ui_ms) as f32 / self.ms_per_tick;
        self.last_ui_ms = current_ms;
    }

    /// `updatePauseState`.
    pub fn update_pause_state(&mut self, pause: bool) {
        if pause {
            if !self.paused {
                self.paused_delta_tick_residual = self.delta_tick_residual;
            }
            self.paused = true;
        } else {
            if self.paused {
                self.delta_tick_residual = self.paused_delta_tick_residual;
            }
            self.paused = false;
        }
    }

    /// `updateFrozenState`.
    pub fn update_frozen_state(&mut self, frozen: bool) {
        self.frozen = frozen;
    }

    pub fn game_time_delta_ticks(&self) -> f32 {
        self.delta_ticks
    }

    /// `getGameTimeDeltaPartialTick`.
    pub fn game_time_delta_partial_tick(&self, ignore_frozen_game: bool) -> f32 {
        if !ignore_frozen_game && self.frozen {
            1.0
        } else if self.paused {
            self.paused_delta_tick_residual
        } else {
            self.delta_tick_residual
        }
    }

    /// `getRealtimeDeltaTicks`.
    pub fn realtime_delta_ticks(&self) -> f32 {
        if self.realtime_delta_ticks > 7.0 {
            0.5
        } else {
            self.realtime_delta_ticks
        }
    }
}

const NANOSECONDS_PER_SECOND: i64 = 1_000_000_000;
const NANOSECONDS_PER_MILLISECOND: i64 = 1_000_000;

/// `TickRateManager`.
#[derive(Clone, Debug)]
pub struct TickRateManager {
    tickrate: f32,
    nanoseconds_per_tick: i64,
    frozen_ticks_to_run: i32,
    run_game_elements: bool,
    is_frozen: bool,
}

impl Default for TickRateManager {
    fn default() -> Self {
        Self {
            tickrate: 20.0,
            nanoseconds_per_tick: NANOSECONDS_PER_SECOND / 20,
            frozen_ticks_to_run: 0,
            run_game_elements: true,
            is_frozen: false,
        }
    }
}

impl TickRateManager {
    /// `setTickRate` (`Math.max(rate, 1.0F)`, NaN propagates).
    pub fn set_tick_rate(&mut self, rate: f32) {
        self.tickrate = if rate.is_nan() || rate > 1.0 { rate } else { 1.0 };
        self.nanoseconds_per_tick = (NANOSECONDS_PER_SECOND as f64 / f64::from(self.tickrate)) as i64;
    }
    pub fn tickrate(&self) -> f32 {
        self.tickrate
    }
    pub fn milliseconds_per_tick(&self) -> f32 {
        self.nanoseconds_per_tick as f32 / NANOSECONDS_PER_MILLISECOND as f32
    }
    pub fn nanoseconds_per_tick(&self) -> i64 {
        self.nanoseconds_per_tick
    }
    pub fn runs_normally(&self) -> bool {
        self.run_game_elements
    }
    pub fn is_stepping_forward(&self) -> bool {
        self.frozen_ticks_to_run > 0
    }
    pub fn set_frozen_ticks_to_run(&mut self, ticks: i32) {
        self.frozen_ticks_to_run = ticks;
    }
    pub fn frozen_ticks_to_run(&self) -> i32 {
        self.frozen_ticks_to_run
    }
    pub fn set_frozen(&mut self, frozen: bool) {
        self.is_frozen = frozen;
    }
    pub fn is_frozen(&self) -> bool {
        self.is_frozen
    }
    /// `tick`: decides whether game elements run this tick.
    pub fn tick(&mut self) {
        self.run_game_elements = !self.is_frozen || self.frozen_ticks_to_run > 0;
        if self.frozen_ticks_to_run > 0 {
            self.frozen_ticks_to_run -= 1;
        }
    }
    /// `isEntityFrozen` for an entity that is not (and does not carry) a player.
    pub fn is_entity_frozen(&self, is_player_or_carries_player: bool) -> bool {
        !self.runs_normally() && !is_player_or_carries_player
    }
}

/// What `/tick sprint` reports when it ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SprintReport {
    pub ticks_per_second: i32,
    pub milliseconds_per_tick: String,
}

/// `ServerTickRateManager`: the freeze, step and sprint state kept by the
/// server (the broadcasts to clients are the caller's concern).
#[derive(Clone, Debug, Default)]
pub struct ServerTickRateManager {
    pub base: TickRateManager,
    remaining_sprint_ticks: i64,
    sprint_tick_start_time: i64,
    sprint_time_spend: i64,
    scheduled_current_sprint_ticks: i64,
    previous_is_frozen: bool,
}

impl ServerTickRateManager {
    pub fn is_sprinting(&self) -> bool {
        self.scheduled_current_sprint_ticks > 0
    }
    /// `stepGameIfPaused`.
    pub fn step_game_if_paused(&mut self, ticks: i32) -> bool {
        if !self.base.is_frozen() {
            return false;
        }
        self.base.set_frozen_ticks_to_run(ticks);
        true
    }
    /// `stopStepping`.
    pub fn stop_stepping(&mut self) -> bool {
        if self.base.frozen_ticks_to_run() > 0 {
            self.base.set_frozen_ticks_to_run(0);
            true
        } else {
            false
        }
    }
    /// `stopSprinting`.
    pub fn stop_sprinting(&mut self) -> Option<SprintReport> {
        if self.remaining_sprint_ticks > 0 {
            Some(self.finish_tick_sprint())
        } else {
            None
        }
    }
    /// `requestGameToSprint`: true when an earlier sprint was interrupted.
    pub fn request_game_to_sprint(&mut self, time: i32) -> bool {
        let interrupted = self.remaining_sprint_ticks > 0;
        self.sprint_time_spend = 0;
        self.scheduled_current_sprint_ticks = i64::from(time);
        self.remaining_sprint_ticks = i64::from(time);
        self.previous_is_frozen = self.base.is_frozen();
        self.base.set_frozen(false);
        interrupted
    }
    fn finish_tick_sprint(&mut self) -> SprintReport {
        let completed_ticks = self.scheduled_current_sprint_ticks - self.remaining_sprint_ticks;
        let milliseconds_to_complete =
            (self.sprint_time_spend.max(1)) as f64 / NANOSECONDS_PER_MILLISECOND as f64;
        let ticks_per_second = (1000.0 * completed_ticks as f64 / milliseconds_to_complete) as i32;
        let per_tick = if completed_ticks == 0 {
            f64::from(self.base.milliseconds_per_tick())
        } else {
            milliseconds_to_complete / completed_ticks as f64
        };
        self.scheduled_current_sprint_ticks = 0;
        self.sprint_time_spend = 0;
        self.remaining_sprint_ticks = 0;
        self.base.set_frozen(self.previous_is_frozen);
        SprintReport {
            ticks_per_second,
            milliseconds_per_tick: crate::java_text::format_fixed2(per_tick),
        }
    }
    /// `checkShouldSprintThisTick`: whether to run this tick without waiting,
    /// plus the report when the sprint just finished.
    pub fn check_should_sprint_this_tick(&mut self, now_ns: i64) -> (bool, Option<SprintReport>) {
        if !self.base.run_game_elements {
            (false, None)
        } else if self.remaining_sprint_ticks > 0 {
            self.sprint_tick_start_time = now_ns;
            self.remaining_sprint_ticks -= 1;
            (true, None)
        } else {
            (false, Some(self.finish_tick_sprint()))
        }
    }
    /// `endTickWork`.
    pub fn end_tick_work(&mut self, now_ns: i64) {
        self.sprint_time_spend += now_ns - self.sprint_tick_start_time;
    }
    /// `setTickRate`.
    pub fn set_tick_rate(&mut self, rate: f32) {
        self.base.set_tick_rate(rate);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer_carries_fractions_and_caps_nothing_itself() {
        let mut t = Timer::new(20.0, 0);
        assert_eq!(t.advance_game_time(49, |d| d), 0);
        assert_eq!(t.advance_game_time(50, |d| d), 1);
        assert_eq!(t.advance_game_time(1050, |d| d), 20);
    }
    #[test]
    fn pause_restores_the_residual() {
        let mut t = Timer::new(20.0, 0);
        t.advance_game_time(25, |d| d);
        t.update_pause_state(true);
        t.advance_game_time(1000, |d| d);
        assert_eq!(t.game_time_delta_partial_tick(false), 0.5);
        t.update_pause_state(false);
        assert_eq!(t.game_time_delta_partial_tick(false), 0.5);
    }
    #[test]
    fn frozen_step_runs_exactly_the_requested_ticks() {
        let mut m = ServerTickRateManager::default();
        m.base.set_frozen(true);
        m.base.tick();
        assert!(!m.base.runs_normally());
        assert!(m.step_game_if_paused(2));
        for _ in 0..2 {
            m.base.tick();
            assert!(m.base.runs_normally());
        }
        m.base.tick();
        assert!(!m.base.runs_normally());
    }
}
