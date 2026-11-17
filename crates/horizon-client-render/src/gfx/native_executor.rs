//! Horizon's own shader packs as the renderer's external pipeline (#14680): each frame point
//! and lifecycle callback of `ExternalPipeline` is the pack's method of the same name.

use super::external::{
    Capabilities, DisplacementPrepare, EntityLayers, ExternalPipeline, FrameStart, FrameState,
    Near, OpaqueCopy, Refused, ShadowDraws, ShadowView, SkyPass, TerrainLayers, TimingSetup,
    WorldPost, WorldTargets,
};
use super::native_pack::{NativePack, install_kept};
use super::pack_timing::PassTiming;
use super::rt::scene::{RayScene, Traced};

impl ExternalPipeline for NativePack {
    fn capabilities(&self) -> Option<Capabilities> {
        Some(NativePack::capabilities(self))
    }

    /// A rebuild of the pack that ran keeps its kept resources (`NativePack::carry`); else
    /// they go before this pack's are made (`install_kept`), which may find no room for them.
    fn take_over(
        &mut self,
        device: &wgpu::Device,
        old: Option<&mut dyn ExternalPipeline>,
        size: [u32; 2],
    ) -> Result<(), Refused> {
        let mut old = old.and_then(|o| (o as &mut dyn std::any::Any).downcast_mut::<NativePack>());
        if let Some(old) = old.as_deref_mut() {
            self.carry(old);
        }
        install_kept(device, Some(self), old, size)
            .map_err(|(why, old_runs)| Refused { why, old_runs })
    }

    fn retire(&mut self, device: &wgpu::Device) {
        // Nothing is made, so nothing is refused, whatever the size.
        let freed = install_kept(device, None, Some(self), [1, 1]);
        debug_assert!(freed.is_ok(), "{freed:?}");
    }

    /// The kept resources start over.
    fn world_cleared(&mut self) {
        if let Some(epochs) = &mut self.frame.epochs {
            epochs.note(horizon_shader_api::kept::REASON_WORLD);
        }
    }

    fn reset_history(&mut self) {
        self.frame.reset_history();
        self.post.reset_history();
    }

    fn set_up_timing(&mut self, setup: TimingSetup<'_>) {
        NativePack::set_up_timing(self, setup);
    }

    fn timing_submitted(&mut self, queue: &wgpu::Queue) {
        NativePack::timing_submitted(self, queue);
    }

    fn timer(&mut self) -> Option<&mut Option<PassTiming>> {
        Some(&mut self.timing)
    }

    fn timing(&self) -> Option<&PassTiming> {
        self.timing.as_ref()
    }

    fn shadow_request(&self, state: &FrameState<'_>) -> Option<ShadowView> {
        NativePack::shadow_request(self, state.frame)
    }

    fn prepare_displacement(
        &mut self,
        prepare: DisplacementPrepare<'_, &mut dyn Iterator<Item = Near<'_>>>,
    ) {
        NativePack::prepare_displacement(self, prepare);
    }

    fn start_frame(&mut self, start: FrameStart<'_>) -> bool {
        NativePack::start_frame(self, start)
    }

    fn draw_shadow(&mut self, draws: ShadowDraws<'_>) {
        NativePack::draw_shadow(self, draws);
    }

    fn before_world(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        scene: Option<&Traced<'_>>,
    ) {
        NativePack::before_world(self, device, encoder, scene);
    }

    fn draw_sky_pass(&mut self, sky: SkyPass<'_>) {
        NativePack::draw_sky_pass(self, sky);
    }

    fn prepare_world_targets(&mut self, world: WorldTargets<'_>) {
        NativePack::prepare_world_targets(self, world);
    }

    fn bind_scene(&mut self, device: &wgpu::Device, scene: Option<&RayScene>, camera: glam::DVec3) {
        NativePack::bind_scene(self, device, scene, camera);
    }

    fn terrain_layers(&self) -> Option<TerrainLayers<'_>> {
        NativePack::terrain_layers(self)
    }

    fn entity_layers(&self) -> Option<EntityLayers<'_>> {
        NativePack::entity_layers(self)
    }

    fn draw_sky(&self, pass: &mut wgpu::RenderPass<'_>) -> bool {
        NativePack::draw_sky(self, pass)
    }

    fn draw_displaced(&self, pass: &mut wgpu::RenderPass<'_>, layers: TerrainLayers<'_>) {
        if let Some(grid) = self.displaced_grid(layers) {
            grid.draw(pass);
        }
    }

    fn copy_opaque(&self, copy: OpaqueCopy<'_>) {
        NativePack::copy_opaque(self, copy);
    }

    fn draw_weather(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        weather: &super::weather::Weather,
        entities: &wgpu::BindGroup,
    ) -> bool {
        NativePack::draw_weather(self, pass, weather, entities)
    }

    fn run_post(&mut self, post: WorldPost<'_>) {
        NativePack::run_post(self, post);
    }
}
