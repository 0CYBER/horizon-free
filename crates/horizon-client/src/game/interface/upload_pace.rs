//! How long a frame spends uploading finished chunk meshes when the player set
//! no Chunk Updates or Upload Budget.
//!
//! A fixed `AUTO_UPLOAD_TIME` (2 ms) is a small part of a 60 FPS frame but
//! three times a frame at 1,000+ FPS: while flying, every frame that took a
//! burst of meshes stood out as a 3 ms hitch, which set the 1% lows (the ocean
//! at render distance 32 most of all: its water sections are many and all
//! meshed). The budget follows the frame instead: a quarter of the recent
//! frame time, at least `AUTO_UPLOAD_MIN`, growing with the backlog so a
//! teleport's or a world's terrain still arrives at the full 2 ms a frame.
use std::time::Duration;

/// The most a frame spends uploading meshes, and what it spends while a
/// level loads.
pub const AUTO_UPLOAD_TIME: Duration = Duration::from_millis(2);
/// `AUTO_UPLOAD_TIME` while the view's terrain is still arriving (a world's
/// open, a teleport).
pub const AUTO_UPLOAD_TIME_ARRIVING: Duration = Duration::from_millis(6);
/// The least: at least one mesh goes every frame in any case.
pub const AUTO_UPLOAD_MIN: Duration = Duration::from_micros(250);
/// Waiting meshes that double the budget: past a few of these, the frame
/// takes the whole `AUTO_UPLOAD_TIME`.
const BACKLOG_STEP: usize = 256;

/// The recent frame time, smoothed so one slow frame doesn't change the
/// budget of the next ones much.
#[derive(Default)]
pub struct UploadPace {
    frame: Option<Duration>,
}

impl UploadPace {
    /// One frame's interval (start to start).
    pub fn frame(&mut self, elapsed: Duration) {
        // A frame longer than a tick (a hitch, the window hidden) says nothing of the pace.
        let elapsed = elapsed.min(Duration::from_millis(50));
        self.frame = Some(match self.frame {
            None => elapsed,
            Some(average) => (average * 15 + elapsed) / 16,
        });
    }

    /// This frame's upload time, with `backlog` meshes waiting; `arriving`: terrain arriving
    /// (world join, teleport) where uploads are prioritised.
    pub fn budget(&self, backlog: usize, arriving: bool) -> Duration {
        if arriving {
            return AUTO_UPLOAD_TIME_ARRIVING;
        }
        let Some(frame) = self.frame else {
            return AUTO_UPLOAD_TIME;
        };
        let scale = 1 + (backlog / BACKLOG_STEP) as u32;
        (frame / 4)
            .saturating_mul(scale)
            .clamp(AUTO_UPLOAD_MIN, AUTO_UPLOAD_TIME)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paced(frame_us: u64) -> UploadPace {
        let mut pace = UploadPace::default();
        for _ in 0..64 {
            pace.frame(Duration::from_micros(frame_us));
        }
        pace
    }

    #[test]
    fn slow_frame_rates_keep_the_fixed_budget() {
        // 60 FPS: a quarter of the frame is past the 2 ms cap.
        assert_eq!(paced(16_667).budget(0, false), AUTO_UPLOAD_TIME);
        // Before any frame was timed, and on an arriving screen at any rate.
        assert_eq!(UploadPace::default().budget(0, false), AUTO_UPLOAD_TIME);
        assert_eq!(paced(800).budget(0, true), AUTO_UPLOAD_TIME_ARRIVING);
    }

    #[test]
    fn fast_frames_upload_a_quarter_of_a_frame_at_least_the_floor() {
        // 250 FPS: 1 ms of a 4 ms frame.
        assert_eq!(paced(4_000).budget(0, false), Duration::from_millis(1));
        // 1,250 FPS: a quarter would be 0.2 ms, under the floor.
        assert_eq!(paced(800).budget(10, false), AUTO_UPLOAD_MIN);
    }

    #[test]
    fn a_backlog_raises_the_budget_up_to_the_cap() {
        let pace = paced(2_000);
        assert_eq!(pace.budget(0, false), Duration::from_micros(500));
        assert_eq!(pace.budget(BACKLOG_STEP, false), Duration::from_millis(1));
        assert_eq!(
            pace.budget(2 * BACKLOG_STEP, false),
            Duration::from_micros(1500)
        );
        assert_eq!(pace.budget(100 * BACKLOG_STEP, false), AUTO_UPLOAD_TIME);
    }

    #[test]
    fn one_hitch_moves_the_pace_a_little() {
        let mut pace = paced(1_000);
        pace.frame(Duration::from_millis(40));
        // (15 x 1 ms + 40 ms) / 16 = 3.4375 ms: a quarter is 0.86 ms, not 10.
        assert_eq!(pace.budget(0, false), Duration::from_nanos(859_375));
    }
}
