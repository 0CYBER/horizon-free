//! Pack visibility controls for the native fallback, retaining the native HDR body passes.
use super::{Frame, Renderer};
use horizon_shaders::SkyVisibility;

impl Renderer {
    pub(super) fn pack_sky_visibility(&self, active: bool) -> SkyVisibility {
        self.pack_post
            .as_ref()
            .filter(|_| active)
            .and_then(|p| p.1.as_ref())
            .map_or_else(SkyVisibility::default, |p| p.sky_visibility())
    }

    pub(super) fn draw_visible_sky_disc(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        frame: &Frame<'_>,
        visibility: SkyVisibility,
    ) {
        if !visibility.sky {
            return;
        }
        match self.core_sky.as_ref().filter(|c| c.draws_disc()) {
            Some(core) => {
                core.draw_disc(pass);
                pass.set_bind_group(0, &self.group, &[]);
            }
            None => {
                draw_sky_part(pass, visibility, &self.sky_base, None, 3);
            }
        }
        if frame.sky.sunrise_color[3] > 0.001 {
            draw_sky_part(
                pass,
                visibility,
                &self.sunrise_pipeline,
                Some(&self.sunrise_buffer),
                super::sunrise_fan::VERTICES as u32,
            );
        }
    }

    pub(super) fn draw_visible_void(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        visibility: SkyVisibility,
    ) {
        draw_sky_part(pass, visibility, &self.dark_disc, None, 3);
    }
}

/// The celestial quads the native pass draws: `celestial::bodies`, with the sun's dropped where a
/// pack's sky draws its own (`own_sun`) or its `shaders.properties` sets `sun=false`, and the
/// moon's likewise. The End's flashes are neither and keep their whole range.
pub(super) fn visible_bodies(
    quads: &[crate::mesher::Vertex],
    end_sky: bool,
    (own_sun, own_moon): (bool, bool),
    visibility: SkyVisibility,
) -> std::ops::Range<u32> {
    let hide = |own: bool, shown: bool| !end_sky && (own || !shown);
    super::celestial::bodies(
        quads,
        hide(own_sun, visibility.sun),
        hide(own_moon, visibility.moon),
    )
}

/// Native sunrise and void submission share the vanilla sky visibility request. Keeping
/// the check with the draw also covers the fallback when a pack has no sky program.
pub(super) fn draw_sky_part(
    pass: &mut wgpu::RenderPass<'_>,
    visibility: SkyVisibility,
    pipeline: &wgpu::RenderPipeline,
    vertices: Option<&wgpu::Buffer>,
    count: u32,
) {
    if visibility.sky {
        pass.set_pipeline(pipeline);
        if let Some(vertices) = vertices {
            pass.set_vertex_buffer(0, vertices.slice(..));
        }
        pass.draw(0..count, 0..1);
    }
}
