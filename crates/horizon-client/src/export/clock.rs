use super::{SpecError, validate_fps};
use smallvec::SmallVec;
use std::fmt;
use std::time::Duration;

const NS_PER_SECOND: u128 = 1_000_000_000;
const SPEED_SCALE: u128 = 1_000_000;
// At 1 FPS, one frame can cover 20 seconds at maximum replay speed, plus up
// to one second carried from a tick at the slowest /tick rate. The fastest
// /tick rate is 10,000 TPS (horizon-command's tree_misc.rs).
const MAX_TICKS_PER_FRAME: u128 = (mod_api::REPLAY_MAX_SPEED as u128 + 1) * 10_000;

#[derive(Debug)]
pub struct FrameStep {
    pub index: u64,
    pub real: Duration,
    pub sim: Duration,
    pub ticks: u32,
    pub partial: f32,
    /// Each tick's real timestamp minus the frame's start, with both timestamps
    /// floored to whole nanoseconds.
    pub tick_offsets_ns: SmallVec<[i64; 4]>,
    first_sim_tick_ns: i128,
    tick_ns: u64,
}

impl FrameStep {
    /// Recording-time offsets. Converting the floored real offsets back through the playback
    /// speed could leave a tick just short of a packet timestamp.
    pub fn simulation_tick_offsets_ns(&self) -> impl Iterator<Item = u64> + '_ {
        (0..self.ticks).map(|n| {
            (self.first_sim_tick_ns + i128::from(n) * i128::from(self.tick_ns)).max(0) as u64
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockError {
    ZeroTickDuration,
    TooManyTicks,
    FrameIndexOverflow,
}

impl fmt::Display for ClockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ZeroTickDuration => "export tick duration must be positive",
            Self::TooManyTicks => "too many simulation ticks in an export frame",
            Self::FrameIndexOverflow => "export frame index overflow",
        })
    }
}

impl std::error::Error for ClockError {}

pub struct ExportClock {
    numerator: u128,
    ns_times_denominator: u128,
    index: u64,
    // Simulation time is in units of 1 / (numerator * SPEED_SCALE) nanoseconds.
    acc: u128,
    sim_fraction: u128,
}

impl ExportClock {
    pub fn new(fps: (u32, u32)) -> Result<Self, SpecError> {
        validate_fps(fps)?;
        Ok(Self {
            numerator: u128::from(fps.0),
            ns_times_denominator: u128::from(fps.1) * NS_PER_SECOND,
            index: 0,
            acc: 0,
            sim_fraction: 0,
        })
    }

    /// Start at an existing tick phase, such as the position of a replay after pre-roll.
    pub fn set_tick_phase(&mut self, elapsed_ns: u64) {
        self.acc = u128::from(elapsed_ns) * self.numerator * SPEED_SCALE;
    }

    /// Advance one frame. A shorter tick duration can make ticks due at offset zero.
    /// A speed of zero advances only real time and retains the simulation phase.
    pub fn step(&mut self, speed_micro: u32, tick_ns: u64) -> Result<FrameStep, ClockError> {
        self.step_until(speed_micro, tick_ns, u64::MAX)
    }

    /// Advance one output frame, clipping simulation to the remaining recording time.
    pub fn step_until(
        &mut self,
        speed_micro: u32,
        tick_ns: u64,
        remaining_ns: u64,
    ) -> Result<FrameStep, ClockError> {
        if tick_ns == 0 {
            return Err(ClockError::ZeroTickDuration);
        }
        let index = self.index;
        let next = index.checked_add(1).ok_or(ClockError::FrameIndexOverflow)?;
        let start = u128::from(index) * self.ns_times_denominator;
        let end = u128::from(next) * self.ns_times_denominator;
        let real_ns = end / self.numerator - start / self.numerator;
        let speed = u128::from(speed_micro);
        let sim_divisor = self.numerator * SPEED_SCALE;
        let remaining = (u128::from(remaining_ns) * sim_divisor).saturating_sub(self.sim_fraction);
        let sim = (self.ns_times_denominator * speed).min(remaining);
        let tick = u128::from(tick_ns) * sim_divisor;
        let elapsed = self.acc + sim;
        let ticks = elapsed / tick;
        if ticks > MAX_TICKS_PER_FRAME {
            return Err(ClockError::TooManyTicks);
        }

        let mut tick_offsets_ns = SmallVec::new();
        tick_offsets_ns
            .try_reserve_exact(ticks as usize)
            .map_err(|_| ClockError::TooManyTicks)?;
        for n in 1..=ticks {
            let until_tick = (tick * n).saturating_sub(self.acc);
            let offset = if until_tick == 0 {
                0
            } else {
                // Include the frame start's fractional real nanosecond before flooring.
                (start % self.numerator * speed + until_tick) / (self.numerator * speed)
            };
            tick_offsets_ns.push(offset as i64);
        }

        let first_sim_tick_ns = (self.sim_fraction as i128 + tick as i128 - self.acc as i128)
            .div_euclid(sim_divisor as i128);
        self.index = next;
        self.acc = elapsed % tick;
        let sim_output = self.sim_fraction + sim;
        self.sim_fraction = sim_output % sim_divisor;
        // Rounding a phase just below one to f32 can otherwise produce 1.0.
        let partial = (self.acc as f64 / tick as f64) as f32;
        Ok(FrameStep {
            index,
            real: Duration::from_nanos(real_ns as u64),
            sim: Duration::from_nanos((sim_output / sim_divisor) as u64),
            ticks: ticks as u32,
            partial: partial.min(1.0 - f32::EPSILON / 2.0),
            tick_offsets_ns,
            first_sim_tick_ns,
            tick_ns,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rational_frame_rates_and_speeds_do_not_drift() {
        for (num, den) in [(24, 1), (30, 1), (60, 1), (30_000, 1001)] {
            for speed in [500_000, 1_000_000, 2_000_000] {
                let mut clock = ExportClock::new((num, den)).unwrap();
                let mut real = Duration::ZERO;
                let mut sim = Duration::ZERO;
                let mut ticks = 0;
                for index in 0..num {
                    let step = clock.step(speed, 50_000_000).unwrap();
                    assert_eq!(step.index, u64::from(index));
                    assert!((0.0..1.0).contains(&step.partial));
                    assert_eq!(step.tick_offsets_ns.len(), step.ticks as usize);
                    assert!(step.tick_offsets_ns.is_sorted());
                    assert!(
                        step.tick_offsets_ns
                            .iter()
                            .all(|&ns| (0..=step.real.as_nanos() as i64).contains(&ns))
                    );
                    if (num, speed) == (60, 1_000_000) {
                        if index % 3 == 2 {
                            assert_eq!(
                                step.tick_offsets_ns.as_slice(),
                                [step.real.as_nanos() as i64]
                            );
                            assert_eq!(step.partial, 0.0);
                        } else {
                            assert_eq!(step.ticks, 0);
                        }
                    }
                    real += step.real;
                    sim += step.sim;
                    ticks += step.ticks;
                }
                let expected_sim = Duration::from_micros(u64::from(den) * u64::from(speed));
                assert_eq!(real, Duration::from_secs(u64::from(den)));
                assert_eq!(sim, expected_sim);
                assert_eq!(ticks as u128, expected_sim.as_nanos() / 50_000_000);
            }
        }
    }

    #[test]
    fn replay_end_clips_ticks_and_partial_without_shortening_output_frames() {
        for (end, ticks, partial) in [(49, 0, 0.98), (60, 1, 0.2)] {
            let mut clock = ExportClock::new((30, 1)).unwrap();
            let first = clock.step(1_000_000, 50_000_000).unwrap();
            let remaining_ns = end * 1_000_000 - first.sim.as_nanos() as u64;
            let last = clock
                .step_until(1_000_000, 50_000_000, remaining_ns)
                .unwrap();
            assert_eq!(last.sim, Duration::from_nanos(remaining_ns));
            assert_eq!(last.real, Duration::from_nanos(33_333_333));
            assert_eq!(last.ticks, ticks);
            assert_eq!(last.partial, partial);
        }
    }

    #[test]
    fn speed_and_tick_duration_changes_keep_simulation_time() {
        let mut clock = ExportClock::new((3, 1)).unwrap();
        // A third of a second at half speed leaves 166666666⅔ sim ns.
        let first = clock.step(500_000, 200_000_000).unwrap();
        assert_eq!(first.sim.as_nanos(), 166_666_666);
        assert_eq!(first.ticks, 0);

        // The shorter tick is due even with playback paused.
        let paused = clock.step(0, 125_000_000).unwrap();
        assert_eq!(paused.sim, Duration::ZERO);
        assert_eq!(paused.tick_offsets_ns.as_slice(), [0]);

        // This frame adds exactly half a simulation second and produces three ticks.
        let resumed = clock.step(1_500_000, 175_000_000).unwrap();
        assert_eq!(resumed.sim.as_nanos(), 500_000_000);
        assert_eq!(
            resumed.tick_offsets_ns.as_slice(),
            [88_888_889, 205_555_556, 322_222_222]
        );
        assert!((resumed.partial - (1.0 / 60.0) / 0.175).abs() < 1e-7);

        // The next half-speed frame still gets the carried fraction.
        let next = clock.step(500_000, 175_000_000).unwrap();
        assert_eq!(next.sim.as_nanos(), 166_666_667);
    }

    #[test]
    fn alternating_speeds_land_on_the_exact_sixty_fps_tick_boundaries() {
        let mut clock = ExportClock::new((60, 1)).unwrap();
        for _ in 0..60 {
            let first = clock.step(3_000_000, 50_000_000).unwrap();
            let second = clock.step(500_000, 50_000_000).unwrap();
            let third = clock.step(2_500_000, 50_000_000).unwrap();
            assert_eq!(first.sim.as_nanos(), 50_000_000);
            assert_eq!(second.sim.as_nanos(), 8_333_333);
            assert_eq!(third.sim.as_nanos(), 41_666_667);
            assert_eq!(second.ticks, 0);
            for step in [first, third] {
                assert_eq!(
                    step.tick_offsets_ns.as_slice(),
                    [step.real.as_nanos() as i64]
                );
                assert_eq!(step.partial, 0.0);
            }
        }
    }

    #[test]
    fn rejecting_an_excessive_tick_count_keeps_the_current_frame() {
        let mut clock = ExportClock::new((60, 1)).unwrap();
        let first = clock.step(1_000_000, 50_000_000).unwrap();
        assert_eq!(first.index, 0);
        assert!(matches!(
            clock.step(u32::MAX, 1),
            Err(ClockError::TooManyTicks)
        ));
        let second = clock.step(2_000_000, 50_000_000).unwrap();
        assert_eq!(second.index, 1);
        assert_eq!(
            second.tick_offsets_ns.as_slice(),
            [second.real.as_nanos() as i64]
        );
        assert_eq!(second.partial, 0.0);
    }
}
