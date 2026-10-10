//! Fixed-rate development simulation driver. Rendering consumes elapsed time;
//! a per-frame work cap retains its backlog instead of dropping simulation time.
//! The rate, freezing, stepping and sprinting are what `/tick` controls
//! (`ServerTickRateManager`).
use std::time::Duration;

pub const TICK: Duration = Duration::from_millis(50);
/// Ticks one sprint frame may run.
const SPRINT_PER_FRAME: u32 = 40;
const SAMPLES: usize = 100;

pub struct FixedStep {
    pending: Duration,
    /// Ticks per second (`tickrate`).
    rate: f32,
    frozen: bool,
    /// Ticks the frozen world still runs (`/tick step`).
    step: u32,
    /// Ticks left in a sprint.
    sprint: u32,
    /// How long recent ticks took (nanoseconds), newest last.
    samples: Vec<u64>,
}
impl Default for FixedStep {
    fn default() -> Self {
        Self {
            pending: Duration::ZERO,
            rate: 20.0,
            frozen: false,
            step: 0,
            sprint: 0,
            samples: Vec::new(),
        }
    }
}
impl FixedStep {
    /// The length of one tick at the current rate.
    pub fn tick_duration(&self) -> Duration {
        Duration::from_secs_f64(1.0 / f64::from(self.rate.max(0.01)))
    }
    pub fn advance(&mut self, elapsed: Duration) -> u32 {
        if self.sprint > 0 {
            let n = self.sprint.min(SPRINT_PER_FRAME);
            self.sprint -= n;
            self.pending = Duration::ZERO;
            return n;
        }
        self.pending += elapsed;
        let tick = self.tick_duration();
        let steps = (self.pending.as_nanos() / tick.as_nanos().max(1)).min(10) as u32;
        self.pending -= tick * steps;
        steps
    }
    pub fn reset(&mut self) {
        self.pending = Duration::ZERO;
    }
    pub fn fraction(&self) -> f64 {
        (self.pending.as_secs_f64() / self.tick_duration().as_secs_f64()).min(1.0)
    }
    /// Whether the world ticks this time (`ServerTickRateManager.runsNormally`
    /// or a pending step); called once per tick.
    pub fn world_runs(&mut self) -> bool {
        if !self.frozen {
            return true;
        }
        if self.step > 0 {
            self.step -= 1;
            return true;
        }
        false
    }
    pub fn rate(&self) -> f32 {
        self.rate
    }
    pub fn set_rate(&mut self, rate: f32) {
        self.rate = rate.clamp(1.0, 10000.0);
    }
    pub fn frozen(&self) -> bool {
        self.frozen
    }
    pub fn set_frozen(&mut self, frozen: bool) {
        self.frozen = frozen;
        if !frozen {
            self.step = 0;
        }
    }
    pub fn sprinting(&self) -> bool {
        self.sprint > 0
    }
    /// `requestGameToSprint`: true when a sprint was already running.
    pub fn start_sprint(&mut self, ticks: u32) -> bool {
        let was = self.sprint > 0;
        self.sprint = ticks;
        was
    }
    pub fn stop_sprint(&mut self) -> bool {
        let was = self.sprint > 0;
        self.sprint = 0;
        was
    }
    pub fn stepping(&self) -> bool {
        self.step > 0
    }
    /// `stepGameIfPaused`: only works while frozen.
    pub fn start_step(&mut self, ticks: u32) -> bool {
        if !self.frozen {
            return false;
        }
        self.step = ticks;
        true
    }
    pub fn stop_step(&mut self) -> bool {
        let was = self.step > 0;
        self.step = 0;
        was
    }
    pub fn record(&mut self, took: Duration) {
        if self.samples.len() == SAMPLES {
            self.samples.remove(0);
        }
        self.samples.push(took.as_nanos() as u64);
    }
    pub fn average_nanos(&self) -> u64 {
        if self.samples.is_empty() {
            return 0;
        }
        self.samples.iter().sum::<u64>() / self.samples.len() as u64
    }
    /// The sorted tick times (p50, p95, p99 inputs) and their count.
    pub fn sorted_samples(&self) -> Vec<u64> {
        let mut v = self.samples.clone();
        v.sort_unstable();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn render_schedules_preserve_ticks_and_stall_backlog() {
        for schedule in [&[7, 13, 18, 12][..], &[50][..], &[1, 999, 2700, 300][..]] {
            let mut clock = FixedStep::default();
            let mut elapsed = Duration::ZERO;
            let mut ticks = 0;
            for _ in 0..100 {
                for &ms in schedule {
                    let frame = Duration::from_millis(ms);
                    elapsed += frame;
                    let count = clock.advance(frame);
                    assert!(count <= 10);
                    ticks += count;
                }
            }
            loop {
                let count = clock.advance(Duration::ZERO);
                ticks += count;
                if count == 0 {
                    break;
                }
            }
            assert_eq!(u128::from(ticks), elapsed.as_nanos() / TICK.as_nanos());
        }
    }
    #[test]
    fn pause_discards_only_explicitly_reset_time() {
        let mut clock = FixedStep::default();
        assert_eq!(clock.advance(Duration::from_millis(49)), 0);
        assert_eq!(clock.advance(Duration::from_millis(1)), 1);
        assert_eq!(clock.advance(Duration::from_millis(24)), 0);
        clock.reset();
        assert_eq!(clock.advance(Duration::from_millis(26)), 0);
        assert_eq!(clock.advance(Duration::from_millis(24)), 1);
    }
    #[test]
    fn rate_changes_the_tick_length() {
        let mut clock = FixedStep::default();
        clock.set_rate(40.0);
        assert_eq!(clock.advance(Duration::from_millis(100)), 4);
        clock.set_rate(5.0);
        assert_eq!(clock.advance(Duration::from_millis(400)), 2);
        clock.set_rate(1.0e9);
        assert_eq!(clock.rate(), 10000.0);
    }
    #[test]
    fn freeze_step_and_sprint() {
        let mut clock = FixedStep::default();
        assert!(clock.world_runs());
        assert!(!clock.start_step(3), "stepping needs a frozen game");
        clock.set_frozen(true);
        assert!(!clock.world_runs());
        assert!(clock.start_step(2));
        assert!(clock.world_runs() && clock.world_runs() && !clock.world_runs());
        assert!(clock.start_step(5) && clock.stop_step() && !clock.stop_step());
        clock.set_frozen(false);
        assert!(!clock.start_sprint(100));
        assert!(clock.sprinting());
        assert_eq!(clock.advance(Duration::ZERO), SPRINT_PER_FRAME);
        assert!(clock.stop_sprint() && !clock.sprinting());
        for ns in [3, 1, 2] {
            clock.record(Duration::from_nanos(ns));
        }
        assert_eq!(clock.average_nanos(), 2);
        assert_eq!(clock.sorted_samples(), vec![1, 2, 3]);
    }
}
