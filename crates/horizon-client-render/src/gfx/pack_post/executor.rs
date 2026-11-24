//! Frame points for a shader pack loaded from shaders/ (#14679).

use super::super::external::{ExternalPipeline, FrameState, ShadowView};
use super::{PackPost, pack_uniforms};

impl ExternalPipeline for PackPost {
    /// What the shadow camera draws of the terrain this frame, when the pack draws it.
    fn shadow_request(&self, state: &FrameState<'_>) -> Option<ShadowView> {
        if !self.has_shadow() {
            return None;
        }
        let shadow = &self.uniforms.shadow;
        Some(ShadowView {
            cull: pack_uniforms::shadow_culling(state.frame, shadow),
            light: pack_uniforms::shadow_light(state.frame, shadow),
            distance: (shadow.render_mul > 0.0).then_some(shadow.distance * shadow.render_mul),
            unculled: match self.shadow_culling {
                horizon_shaders::ShadowCulling::On => None,
                horizon_shaders::ShadowCulling::Off => Some(f32::INFINITY),
                horizon_shaders::ShadowCulling::Reversed => Some(shadow.voxel_distance),
            },
            every_face: true,
            view_casters: true,
            depth_planes: false,
        })
    }
}
