use crate::game::Game;
use horizon_client::{
    clock::{FixedStep, SprintReport},
    stopwatches::Stopwatches,
};
use horizon_gameplay::commands::misc::MiscHost;

impl MiscHost for Game<'_> {
    fn stopwatches(&mut self) -> &mut Stopwatches {
        &mut self.score.stopwatches
    }
    fn tick_clock(&mut self) -> &mut FixedStep {
        &mut self.clock
    }
    fn millis(&self, nanos: u64) -> String {
        super::super::misc::millis(nanos)
    }
    fn apply_tick_rate(&mut self, rate: f32) {
        Game::apply_tick_rate(self, rate);
    }
    fn sprint_report(&mut self, report: &SprintReport) {
        Game::sprint_report(self, report);
    }
    fn on_tick_rate_changed(&mut self) {
        Game::on_tick_rate_changed(self);
    }
}
