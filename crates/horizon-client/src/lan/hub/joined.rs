//! Entities that join a level between tracking passes (`ChunkMap.addEntity`): paired as they
//! joined, before their first tick.
use super::{Hub, entity_type};
use crate::lan::entity::EntityState;

impl Hub {
    /// Fresh entities that joined during the current tick before the tracking pass:
    /// paired from their state as they joined (ChunkMap.addEntity), before their first tick.
    pub fn note_fresh_joined(&mut self, entities: &[EntityState]) {
        let joined: Vec<_> = entities
            .iter()
            .filter(|e| !self.tracks(e.id))
            .cloned()
            .collect();
        if joined.is_empty() {
            return;
        }
        self.fresh_joined
            .entry(self.config.dimension.clone())
            .or_default()
            .extend(joined);
    }

    /// `ChunkMap.addEntity` for late arrivals, before their sounds are sent.
    /// Pairing does not run another tracking pass or advance entity clocks.
    pub fn note_joined(&mut self, entities: &[EntityState]) {
        if entities.is_empty() {
            return;
        }
        let dimension = &self.config.dimension;
        let mut outs: Vec<Vec<crate::lan::Body>> = self
            .guests
            .iter()
            .filter(|g| self.here(g))
            .map(|_| Vec::new())
            .collect();
        let mut viewers = self.guests.tracking_viewers(
            |g| g.placed && g.dimension == *dimension,
            outs.iter_mut().map(|out| (out, None)),
            false,
        );
        let level = self.entities.entry(dimension.clone()).or_default();
        level.range_percent = Some(self.entity_range_percent);
        let (types, sync) = (&mut self.entity_types, &self.sync);
        level.pair_after_tick(entities, &self.riding, &mut viewers, |kind| {
            entity_type(types, sync, kind)
        });
        drop(viewers);
        for (g, out) in self.guests.iter().filter(|g| self.here(g)).zip(&mut outs) {
            g.sender.send_many(out.drain(..));
        }
    }
}
