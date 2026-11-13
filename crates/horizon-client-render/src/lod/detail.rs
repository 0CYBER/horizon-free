//! Reduced far detail.
use super::*;

impl Lod {
    /// Reduced far detail (`graphics::FarDetail::Reduced`): the next update
    /// replans when it changes, and tiles sampled from then on approximate
    /// wide cells (`GeneratorWorker::lod_grid`).
    pub fn set_reduced_detail(&mut self, reduced: bool) {
        if self.reduced_detail != reduced {
            let keys: Vec<_> = self
                .ready
                .keys()
                .chain(self.pending.iter())
                .copied()
                .collect();
            for key in keys {
                self.mark_stale(key);
            }
        }
        self.reduced_detail = reduced;
        self.shared.approximate.store(reduced, Ordering::Relaxed);
    }
}
