//! Fixed-rate development simulation driver. Rendering consumes elapsed time;
//! a per-frame work cap retains its backlog instead of dropping simulation time.
use std::time::Duration;

pub const TICK: Duration = Duration::from_millis(50);
#[derive(Default)]
pub struct FixedStep {
    pending: Duration,
}
impl FixedStep {
    pub fn advance(&mut self, elapsed: Duration) -> u32 {
        self.pending += elapsed;
        let steps = (self.pending.as_nanos() / TICK.as_nanos()).min(10) as u32;
        self.pending -= TICK * steps;
        steps
    }
    pub fn reset(&mut self) {
        self.pending = Duration::ZERO;
    }
    pub fn fraction(&self) -> f64 {
        (self.pending.as_secs_f64() / TICK.as_secs_f64()).min(1.0)
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
}
