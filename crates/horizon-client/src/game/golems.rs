//! Golems built from blocks: a carved pumpkin on snow blocks, an iron T or a
//! copper block (`CarvedPumpkinBlock.trySpawnGolem`).
use super::Game;

impl Game<'_> {
    /// Brings a freshly built golem to life at the base block of its pattern.
    pub(super) fn spawn_built_golem(&mut self, pos: [i32; 3], kind: &str) {
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.05,
            f64::from(pos[2]) + 0.5,
        ];
        let _ = self.spawn_mob(kind, at, true);
    }
}
