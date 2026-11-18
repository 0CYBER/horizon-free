//! The world, renderer and mesher state a section-readiness test reads, borrowed field by field.
use crate::occlusion_culling::opaque;
use horizon_client::{
    blocks::BlockFacts, gfx::Renderer, mesher::SectionMesh, streaming::Streaming, world::World,
};

#[derive(Clone, Copy)]
pub struct SectionScene<'a> {
    pub world: &'a World,
    pub facts: &'a BlockFacts,
    pub renderer: Option<&'a Renderer>,
    pub streaming: &'a Streaming,
    pub pending_uploads: &'a [((i32, i32, i32), SectionMesh)],
}

impl SectionScene<'_> {
    /// An opaque block whose section has a settled mesh, so it can hide what is behind it.
    pub fn occluder_ready(&self, pos: [i32; 3]) -> bool {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        if !opaque(self.facts.get(state)) {
            return false;
        }
        let Some(y) = pos[1].checked_sub(self.world.bottom_y()) else {
            return false;
        };
        self.section_settled((pos[0] >> 4, y.div_euclid(16), pos[2] >> 4))
    }

    /// Drawn, not dirty, not being meshed and with no mesh waiting to upload.
    pub fn section_settled(&self, section: (i32, i32, i32)) -> bool {
        self.renderer
            .is_some_and(|renderer| renderer.has_section(section))
            && !self.world.dirty_sections.contains(&section)
            && !self.streaming.is_meshing(section)
            && !self.pending_uploads.iter().any(|(key, _)| *key == section)
    }
}
