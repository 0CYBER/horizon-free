//! The sky's celestial pass (`RenderPipelines.CELESTIAL`): the sun, the moon and the End sky
//! added onto the sky, and on the float frame, in Horizon's lifted world, the sun's disc in
//! vanilla's colour at the scene's peak (entity.wgsl `fs_sky_sun`).

pub(super) struct Celestial {
    /// `fs_sky_celestial`, added onto the sky: the End sky, the moon, and the sun wherever the
    /// world isn't lifted.
    pub(super) added: wgpu::RenderPipeline,
    /// `fs_sky_sun` (src + frame × alpha) and `fs_sky_sun_clip` (the min): only on the float
    /// frame, which writes light.
    sun: Option<[wgpu::RenderPipeline; 2]>,
}

impl Celestial {
    pub(super) fn new(
        kept: &super::reuse::On<'_>,
        layout: &wgpu::PipelineLayout,
        shader: &wgpu::ShaderModule,
        vertex: &[Option<wgpu::VertexBufferLayout<'_>>],
        (format, linear): (wgpu::TextureFormat, bool),
    ) -> Self {
        let pipeline = |label, entry, blend, write_mask| {
            kept.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some("vs_entity"),
                    compilation_options: Default::default(),
                    buffers: vertex,
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::Always),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some(entry),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &super::linear_blend::frame_constants(format, linear),
                        ..Default::default()
                    },
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(blend),
                        write_mask,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let component = |src_factor, dst_factor, operation| wgpu::BlendComponent {
            src_factor,
            dst_factor,
            operation,
        };
        use wgpu::{BlendFactor as F, BlendOperation as Op};
        // RenderPipelines.CELESTIAL: additive (src alpha, one), with the colour premultiplied
        // in fs_sky_celestial. Its alpha is how much a texel adds: HDR output (hdr.wgsl) lets
        // the sun's and moon's discs shine; nothing else reads the frame's alpha.
        let added = pipeline(
            "Celestial bodies",
            "fs_sky_celestial",
            wgpu::BlendState {
                color: component(F::One, F::One, Op::Add),
                alpha: component(F::Zero, F::OneMinusSrcAlpha, Op::Add),
            },
            wgpu::ColorWrites::ALL,
        );
        // The float frame clamps neither the fragment's alpha nor the factor, so the sky's
        // weight may pass 1. The frame's alpha is left alone: nothing reads it here.
        let sun = (format == super::LINEAR_FRAME).then(|| {
            let affine = component(F::One, F::SrcAlpha, Op::Add);
            let min = component(F::One, F::One, Op::Min);
            [
                pipeline(
                    "The sun onto the sky",
                    "fs_sky_sun",
                    wgpu::BlendState {
                        color: affine,
                        alpha: affine,
                    },
                    wgpu::ColorWrites::COLOR,
                ),
                pipeline(
                    "The sun's clip",
                    "fs_sky_sun_clip",
                    wgpu::BlendState {
                        color: min,
                        alpha: min,
                    },
                    wgpu::ColorWrites::COLOR,
                ),
            ]
        });
        Self { added, sun }
    }

    /// The Overworld's sun and moon (`Game::celestial_geometry`: the sun's quad first, unless a
    /// pack's sky draws its own), the indices in `range`, with `added` bound. In Horizon's
    /// `lifted` world (HDR lighting on the float frame, no pack, not the title) the sun's quad
    /// takes `fs_sky_sun`'s two passes instead.
    pub(super) fn draw_bodies(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        lifted: bool,
        mut range: std::ops::Range<u32>,
    ) {
        if let Some(sun) = self.sun.as_ref().filter(|_| lifted && range.start == 0) {
            for pipeline in sun {
                pass.set_pipeline(pipeline);
                pass.draw_indexed(0..6, 0, 0..1);
            }
            pass.set_pipeline(&self.added);
            range.start = 6;
        }
        if !range.is_empty() {
            pass.draw_indexed(range, 0, 0..1);
        }
    }
}

#[cfg(test)]
#[path = "celestial_tests.rs"]
mod tests;
