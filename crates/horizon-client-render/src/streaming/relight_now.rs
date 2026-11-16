//! The light of a `Job::Light` taken into its chunk.
use super::Streaming;
use crate::{
    chunk::Light,
    light::{LightChanges, apply_lighting, apply_relighting},
    world::World,
};
use std::sync::Arc;

impl Streaming {
    /// Takes a lit column's light into its chunk (`Done::Lit`), `snapshot` being
    /// the edit sequence the blocks it was lit from had reached.
    pub(super) fn apply_lit(
        &mut self,
        world: &mut World,
        x: i32,
        z: i32,
        sections: Vec<Light>,
        snapshot: u64,
    ) {
        if let Some(chunk) = world.chunks.get_mut(&(x, z)) {
            let chunk = Arc::make_mut(chunk);
            // Keep blocks edited since the snapshot; take only light.
            // A chunk relit (a neighbour loaded or changed) remeshes
            // only around the light that changed, as vanilla's light
            // updates dirty only their sections (#270).
            let first = self.lit.insert((x, z));
            let mut changed = LightChanges::default();
            if first {
                apply_lighting(chunk, sections);
            } else {
                apply_relighting(chunk, sections, &mut changed);
            }
            // Saved unlit (at the edge of what was loaded), or its light
            // changed: saved again.
            if (first && !chunk.light_loaded) || !changed.is_empty() {
                world.modified.insert((x, z));
            }
            chunk.lit = true;
            if first {
                for dz in -1..=1 {
                    for dx in -1..=1 {
                        for sy in 0..world.layout().sections() as i32 {
                            world.dirty_sections.insert((x + dx, sy, z + dz));
                        }
                    }
                }
            } else {
                Self::mark_light_changes(world, &changed);
            }
            self.replay_light_edits(world, x, z, snapshot);
        }
    }
}
