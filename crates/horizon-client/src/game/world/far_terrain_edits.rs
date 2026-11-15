use super::*;

impl Game<'_> {
    /// The columns this update's block edits touch, summarized again by far terrain when
    /// it is on.
    pub(super) fn observe_light_edits(&mut self) {
        if self.lod.is_some() {
            for &((x, _, z), _, _) in &self.world.light_edits {
                self.lod_observe.insert((x >> 4, z >> 4));
            }
        }
    }
}
