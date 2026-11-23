use super::*;

impl Game<'_> {
    /// The columns this update's block edits touch, summarized again by far terrain when
    /// it is on.
    pub(super) fn observe_light_edits(&mut self) {
        if self.lod.is_some() {
            let columns: Vec<_> = self
                .world
                .light_edits
                .iter()
                .map(|&((x, _, z), _, _)| (x >> 4, z >> 4))
                .collect();
            self.lod_observe_later(columns);
        }
    }
    /// Resident columns that arrived or changed, for far terrain to summarize (`lod_observe`):
    /// only those it draws now. A column inside the near area is drawn by its own sections,
    /// so its summary waits until it leaves (`Lod::take_summary_asks`, or its unload); a
    /// source that keeps the summaries (`Lod::summarizes_arrivals`) takes every one.
    pub(super) fn lod_observe_later(&mut self, columns: impl IntoIterator<Item = (i32, i32)>) {
        let Some(lod) = &self.lod else { return };
        let every = lod.summarizes_arrivals();
        let (center, radius) = (self.streaming.center, self.streaming.radius);
        self.lod_observe.extend(columns.into_iter().filter(|&(x, z)| {
            every || !horizon_client::streaming::in_view_distance(center, radius, x, z)
        }));
    }
}
