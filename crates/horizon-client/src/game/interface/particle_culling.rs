//! Particle visibility checks and their worker, separate from entity culling.
use super::Game;
use horizon_client::graphics::ParticleCullingMode;
use horizon_client_gameplay::particle_culling::state_is_opaque;
pub(super) use horizon_client_gameplay::particle_culling::{
    ParticleAsyncState, ParticleOcclusionContext,
};

/// A `SectionScene` of the game's fields, so `async_particle_culler` can be borrowed beside it.
macro_rules! section_scene {
    ($game:expr) => {
        horizon_client_gameplay::section_scene::SectionScene {
            world: &$game.world,
            facts: $game.res.facts(),
            renderer: $game.renderer.as_ref(),
            streaming: &$game.streaming,
            pending_uploads: &$game.pending_uploads,
        }
    };
}

impl Game<'_> {
    pub(super) fn particle_occlusion_context(
        &self,
        camera: glam::DVec3,
    ) -> ParticleOcclusionContext<'_> {
        let at = camera.to_array().map(|value| value.floor() as i32);
        let camera_inside_opaque =
            state_is_opaque(self.res.facts().get(self.world.block(at[0], at[1], at[2])));
        let mode =
            if self.model_packs_are_vanilla() && self.player.is_some() && !camera_inside_opaque {
                self.settings.horizon.particle_culling
            } else {
                ParticleCullingMode::Off
            };
        ParticleOcclusionContext::new(
            section_scene!(self),
            &self.async_particle_culler,
            mode,
            camera,
        )
    }

    pub(super) fn finish_async_particle_culling(&self) {
        self.async_particle_culler
            .borrow_mut()
            .finish_frame(&self.world, self.res.facts());
    }
}
