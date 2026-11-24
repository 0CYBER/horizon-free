//! A resource pack's `core/lightmap` shader: the 16x16 lightmap texture the world, the entities
//! and the items are lit with, drawn on the GPU with the pack's GLSL (`RenderPipelines.LIGHTMAP`,
//! `Lightmap.render`). A fullbright pack's lightmap fragment shader lights everything, which the
//! CPU-built vanilla formula can never show. Without a pack shader the lightmap stays the vanilla
//! formula built on the CPU (`Renderer::update_lightmap`).

use crate::core_shader::{CoreProgram, Glue};

/// `RenderPipelines.LIGHTMAP`'s vertex stage, `core/screenquad.vsh`: three
/// vertices from the vertex index alone, one triangle over the target.
pub(crate) const GLUE: Glue = Glue {
    inputs: &[],
    prelude: "",
    setup: "",
    // The lightmap has no depth attachment; the quad's z is irrelevant.
    finish: "",
    vertex_id: "int(gl_VertexIndex)",
};

/// The `LightmapInfo` block's std140 layout (`Lightmap.render`'s UBO): six floats, then the four
/// vec3s at their 16-byte alignments.
pub(super) const UNIFORM_SIZE: u64 = 96;
/// `LightmapInfo`'s slot in the uniform buffer, past the shared blocks' zeroed ranges
/// (256-byte aligned, as a uniform buffer binding's offset must be).
const UNIFORM_OFFSET: u64 = 1024;

/// `LightmapInfo`'s fields, as `Lightmap.render` builds them: the factors and colours the vanilla
/// fragment shader mixes the lightmap from. `SkyFactor` is the sky's light factor, `BlockFactor`
/// the block light's (1.4 plus the flicker), `BrightnessFactor` the brightness (gamma) option and
/// `NightVisionColor` vanilla's default 0x999999.
pub(super) fn uniforms(
    sky: &super::Sky,
    darkness: f32,
    gamma: f32,
    night_vision: f32,
    flicker: f32,
    boss_darken: f32,
) -> [u8; 96] {
    let mut data = [0f32; 24];
    data[0] = sky.sky_light_factor;
    data[1] = 1.4 + flicker;
    data[2] = night_vision;
    data[3] = darkness;
    data[4] = boss_darken;
    data[5] = gamma;
    data[8..11].copy_from_slice(&sky.block_light_tint);
    data[12..15].copy_from_slice(&sky.sky_light_color);
    data[16..19].copy_from_slice(&sky.ambient_light_color);
    data[20..23].copy_from_slice(&[0.6, 0.6, 0.6]);
    bytemuck::cast_slice(&data).try_into().unwrap()
}

/// The pack's lightmap pipeline: one triangle over the 16x16 texture, its uniforms in `LightmapInfo`.
pub(super) struct PackLightmap {
    pipeline: wgpu::RenderPipeline,
    group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
}

impl PackLightmap {
    pub(super) fn new(device: &wgpu::Device, program: &CoreProgram) -> Option<Self> {
        // A pack's shader is untrusted: the backend failing to compile it is an internal error.
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let module = |stage, module: &naga::Module| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(&format!("Resource pack lightmap ({stage})")),
                source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module.clone())),
            })
        };
        let vertex = module("vertex", &program.vertex);
        let fragment = module("fragment", &program.fragment);
        // The four blocks vanilla's other pipelines share (`BLOCKS`), whether or not this shader
        // reads them, and the pack's own blocks (`LightmapInfo`, the `other` list's first). One
        // buffer holds them all: the shared blocks' zeroed slots, then `LightmapInfo` past them.
        let mut entries: Vec<wgpu::BindGroupLayoutEntry> = (0..4)
            .map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            })
            .collect();
        for index in 0..program.other.len() {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: crate::core_shader::other_binding(index),
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            });
        }
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Resource pack lightmap"),
            entries: &entries,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Resource pack lightmap"),
            size: UNIFORM_OFFSET + UNIFORM_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut resources: Vec<wgpu::BindGroupEntry> = (0..4)
            .map(|i| wgpu::BindGroupEntry {
                binding: i as u32,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &uniform,
                    offset: 0,
                    size: None,
                }),
            })
            .collect();
        for index in 0..program.other.len() {
            resources.push(wgpu::BindGroupEntry {
                binding: crate::core_shader::other_binding(index),
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &uniform,
                    offset: UNIFORM_OFFSET,
                    size: Some(wgpu::BufferSize::new(UNIFORM_SIZE).unwrap()),
                }),
            });
        }
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Resource pack lightmap"),
            layout: &layout,
            entries: &resources,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Resource pack lightmap"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Resource pack lightmap"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vertex,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                // `core/screenquad` needs no vertices.
                buffers: &[],
            },
            primitive: Default::default(),
            // The pass has a colour attachment only: no depth test, no blending.
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &fragment,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let internal = pollster::block_on(internal.pop());
        if let Some(e) = internal.or(pollster::block_on(validation.pop())) {
            log::warn!("the resource pack's lightmap shader is not used: {e}");
            return None;
        }
        Some(Self {
            pipeline,
            group,
            uniform,
        })
    }

    /// The frame's `LightmapInfo` (`Lightmap.render`'s mapped UBO).
    pub(super) fn prepare(
        &self,
        uploads: &mut crate::gfx::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        data: &[u8; 96],
    ) {
        uploads.buffer(encoder, &self.uniform, UNIFORM_OFFSET, data);
    }

    /// The one triangle over the lightmap, in place of the CPU-built one.
    pub(super) fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Switching fullbright on has the CPU path overwrite the pack's draw with
    /// white; switching it off must redraw the pack's shader even with every
    /// input unchanged (the PR's review), and the steady states do nothing.
    #[test]
    fn toggling_fullbright_redraws_the_pack_lightmap() {
        let sky = super::super::Sky {
            sky_light_factor: 0.25,
            ..Default::default()
        };
        let key = super::super::LightmapKey::new(&sky, 0.05, 0.5, 0.0, 0.0, 0.0, false);
        let vanilla = [7u8; 16 * 16 * 4];
        let white = [255u8; 16 * 16 * 4];
        // The pack's draw is in, fullbright off: steady state draws nothing.
        let (uploaded, mut from_cpu, drawn_key) = (true, false, Some(key));
        assert!(!super::super::Renderer::pack_lightmap_needs_draw(
            uploaded, from_cpu, drawn_key, &key
        ));
        // Fullbright on: the CPU overwrites the texture with white.
        assert!(super::super::Renderer::cpu_lightmap_needs_upload(
            &white, &vanilla, uploaded
        ));
        from_cpu = true;
        // Fullbright off, every input unchanged: the pack redraws.
        assert!(super::super::Renderer::pack_lightmap_needs_draw(
            uploaded, from_cpu, drawn_key, &key
        ));
        from_cpu = false;
        // And the steady states do nothing again: the pack is drawn and the
        // CPU's table is unchanged.
        assert!(!super::super::Renderer::pack_lightmap_needs_draw(
            uploaded, from_cpu, drawn_key, &key
        ));
        assert!(!super::super::Renderer::cpu_lightmap_needs_upload(
            &vanilla, &vanilla, uploaded
        ));
    }

    /// The std140 layout: the six floats, then the vec3s at 16-byte strides, the last one ending
    /// the block at 96 bytes.
    #[test]
    fn the_uniforms_match_vanillas_lightmap_info_block() {
        let sky = super::super::Sky {
            ambient_light_color: [0.1, 0.2, 0.3],
            sky_light_color: [0.4, 0.5, 0.6],
            block_light_tint: [0.7, 0.8, 0.9],
            sky_light_factor: 0.25,
            ..Default::default()
        };
        let data = uniforms(&sky, 0.05, 0.5, 0.75, 0.1, 0.2);
        assert_eq!(data.len() as u64, UNIFORM_SIZE);
        let f = |i: usize| f32::from_le_bytes(data[i * 4..i * 4 + 4].try_into().unwrap());
        assert_eq!(f(0), 0.25);
        assert!((f(1) - 1.5).abs() < 1e-6);
        assert_eq!(f(2), 0.75);
        assert_eq!(f(3), 0.05);
        assert_eq!(f(4), 0.2);
        assert_eq!(f(5), 0.5);
        let v = |i: usize| [f(i), f(i + 1), f(i + 2)];
        assert_eq!(v(8), [0.7, 0.8, 0.9]);
        assert_eq!(v(12), [0.4, 0.5, 0.6]);
        assert_eq!(v(16), [0.1, 0.2, 0.3]);
        assert_eq!(v(20), [0.6, 0.6, 0.6]);
    }

    /// A fullbright pack's lightmap shader paints the whole texture white: vanilla's lightmap.vsh
    /// with a fragment shader that writes one. Rendered into a 16x16 texture and read back.
    #[test]
    fn a_fullbright_lightmap_shader_paints_the_texture_white() {
        let _gpu = crate::gfx::test_gpu::lock();
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        let Ok(vertex) =
            std::fs::read_to_string(root.join("assets/minecraft/shaders/core/screenquad.vsh"))
        else {
            eprintln!("skipping: no prepared resources at {}", root.display());
            return;
        };
        let include = |id: &str| {
            let (namespace, path) = id.split_once(':')?;
            std::fs::read_to_string(root.join(format!("assets/{namespace}/{path}"))).ok()
        };
        let fragment = "#version 330\n\
            layout(location = 0) in vec2 texCoord;\n\
            layout(location = 0) out vec4 fragColor;\n\
            void main() {\n    fragColor = vec4(1.0);\n}\n";
        let program = crate::core_shader::translate_core_with(
            &vertex,
            fragment,
            &[],
            &include,
            &GLUE,
            &|name| name == "LightmapInfo",
        )
        .expect("the lightmap shaders translate");
        let instance = crate::gfx::test_gpu::instance();
        let Some((device, queue)) =
            pollster::block_on(instance.request_adapter(&Default::default()))
                .ok()
                .and_then(|adapter| {
                    pollster::block_on(adapter.request_device(&Default::default())).ok()
                })
        else {
            eprintln!("no GPU: skipping");
            return;
        };
        let lightmap = PackLightmap::new(&device, &program).expect("the lightmap pipeline");
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 16,
                height: 16,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        // The readback rows sit 256 bytes apart: a copy's row stride must be
        // a multiple of 256.
        const STRIDE: u32 = 256;
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(STRIDE * 16),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        let view = target.create_view(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
            lightmap.draw(&mut pass);
        }
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &read,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(STRIDE),
                    rows_per_image: Some(16),
                },
            },
            target.size(),
        );
        queue.submit([encoder.finish()]);
        read.slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let pixels = read.slice(..).get_mapped_range().unwrap().to_vec();
        let row = |y: usize| &pixels[y * STRIDE as usize..][..16 * 4];
        assert!(
            (0..16).all(|y| row(y)
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| p.iter().all(|&b| b == 255))),
            "the fullbright shader lights every texel: {:?}",
            &row(0)[..16]
        );
    }
}
