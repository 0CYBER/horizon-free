use crate::game::Game;
use horizon_gameplay::commands::time::TimeHost;

impl TimeHost for Game<'_> {
    fn game_time(&self) -> i64 {
        self.world.game_time
    }
    fn default_clock(&self) -> Option<String> {
        self.dimension_type().default_clock.clone()
    }
    fn dimension_id(&self) -> &'static str {
        self.dim.id()
    }
    fn clock_time(&self, clock: &str) -> i64 {
        self.command_clock_time(clock)
    }
    fn set_clock_time(&mut self, clock: &str, total: i64) {
        self.set_command_clock_time(clock, total);
    }
    fn clock_paused(&mut self, clock: &str) -> bool {
        self.command_clock_state(clock).paused
    }
    fn set_clock_paused(&mut self, clock: &str, paused: bool) {
        self.command_clock_state(clock).paused = paused;
    }
    fn clock_rate(&mut self, clock: &str) -> f32 {
        self.command_clock_state(clock).rate
    }
    fn set_clock_rate(&mut self, clock: &str, rate: f32) {
        self.command_clock_state(clock).rate = rate;
    }
    fn reset_clock_partial(&mut self, clock: &str) {
        self.command_clock_state(clock).partial = 0.0;
    }
    fn time_skip_canceled(&mut self, from: i64, to: i64) -> bool {
        let mut cancel = false;
        self.native_time_skip(self.dim.id(), from, to, "command", &mut cancel);
        cancel
    }
    fn time_marker(&self, marker: &str, total: i64) -> Option<(i64, bool)> {
        let m = self.ui.markers.get(marker).copied()?;
        Some((m.resolve(total)?, m.occurs_at(total)))
    }
    fn timeline_period(&self, timeline: &str) -> Option<Option<i64>> {
        self.ui.timelines.get(timeline).copied()
    }
}
