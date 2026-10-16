//! Entity occlusion affects mesh submission, never entity ticking or interaction.
use super::{Game, Vertex};
use horizon_client::chunk::MIN_Y;
use horizon_client::{
    blocks::{RenderShape, StateFacts},
    entity_culling::Culler,
};

fn opaque(facts: &StateFacts) -> bool {
    facts.render_shape == RenderShape::Model
        && facts.can_occlude
        && facts.solid_render
        && facts.full_collision
}

impl Game<'_> {
    pub(super) fn prepare_entity_culling(&mut self, camera: glam::DVec3) {
        let at = camera.to_array().map(|v| v.floor() as i32);
        self.entity_culler = (self.settings.horizon.entity_culling
            // Packs can replace an opaque block's model or texture.
            && self.settings.resource_packs.iter().all(|id| id == "vanilla")
            && self.player.is_some()
            && !self.spectator()
            && !opaque(self.res.facts.get(self.world.block(at[0], at[1], at[2]))))
        .then(|| Culler::new(camera.to_array(), 8_192));
    }

    /// Use complete generated geometry so equipment, animation and custom
    /// models cannot stick out of an assumed collision-sized culling box.
    pub(super) fn cull_entity_vertices(&self, vertices: &mut Vec<Vertex>, start: usize) {
        if let Some(culler) = &self.entity_culler {
            culler.cull_vertices(vertices, start, |pos| {
                let state = self.world.block(pos[0], pos[1], pos[2]);
                if !opaque(self.res.facts.get(state)) {
                    return false;
                }
                let Some(y) = pos[1].checked_sub(MIN_Y) else {
                    return false;
                };
                let section = (pos[0] >> 4, y.div_euclid(16), pos[2] >> 4);
                // Newly edited or not-yet-uploaded terrain is not an occluder.
                self.renderer
                    .as_ref()
                    .is_some_and(|r| r.has_section(section))
                    && !self.world.dirty_sections.contains(&section)
                    && !self.streaming.is_meshing(section)
                    && !self.pending_uploads.iter().any(|(key, _)| *key == section)
            });
        }
    }
}
