//! Mods' render pipelines on the GPU (`Hooks::register_render_pipelines`): the pipelines built
//! from a resource load's checked shaders (`crate::mod_pipelines`), and the quads a frame draws
//! with them. They draw with the world primitives, at their stages and into the same targets,
//! so they take the world's reversed depth, its draw resolution and TAA's jitter. Pipelines are
//! built with the renderer a load is applied with ([`super::Renderer::build_mod_pipelines`]),
//! never while quads are being submitted, so a required pipeline the GPU refuses fails that
//! load's renderer and the reload with it.

use super::world_primitives::Upload;
use crate::mod_pipelines::{Built, BuiltTexture, FRAGMENT_ENTRY, Pipelines, VERTEX_ENTRY};
use glam::Mat4;
use mod_api::{PipelineBlend, WorldDepth};

/// One corner of a pipeline quad, camera-relative.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PipelineVertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    /// Red in the low byte, then green, blue and alpha, as vanilla encodes them.
    pub color: u32,
}

/// Quads that follow each other in a frame's vertices and draw with one pipeline at one stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PipelineRun {
    /// The pipeline's slot in the frame's [`Pipelines`].
    pub slot: u32,
    /// The stage it draws at (`RenderLevelStage::ALL`'s index).
    pub stage: u32,
    pub quads: u32,
}

/// A frame's pipeline quads, as the game queued them: 4 vertices a quad, in the runs' order.
#[derive(Clone, Copy, Debug, Default)]
pub struct Batch<'a> {
    /// The pipelines of the resources the frame draws with; `None` draws nothing.
    pub set: Option<&'a Pipelines>,
    pub vertices: &'a [PipelineVertex],
    pub runs: &'a [PipelineRun],
    /// `PipelineFrame.time`.
    pub time: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FrameUniform {
    viewport: [f32; 2],
    time: f32,
    _pad: f32,
}

/// A run as uploaded: where its quads start in the vertex buffer.
struct Run {
    slot: usize,
    stage: usize,
    quads: std::ops::Range<u32>,
}

/// Where textured pipelines' textures come from when the renderer builds them
/// ([`mod_api::RenderPipeline::textured`]).
#[derive(Clone, Copy)]
pub struct TextureSources<'a> {
    /// The 2D atlas the blocks and items atlases' sprites are drawn from.
    pub atlas: &'a wgpu::TextureView,
    /// A sprite's `[u0, v0, u1, v1]` in `atlas`, by its name there; `None` for no such sprite.
    pub sprite: &'a dyn Fn(&str) -> Option<[f32; 4]>,
}

/// A built pipeline, with its texture's bind group (group 1) when it is textured.
type Compiled = (wgpu::RenderPipeline, Option<wgpu::BindGroup>);

/// The pipelines and the buffer that draw a [`Batch`].
pub struct ModPipelines {
    color_format: wgpu::TextureFormat,
    layout: wgpu::PipelineLayout,
    /// Group 0, then a texture, its sampler and where it lies in what is bound.
    textured_layout: wgpu::PipelineLayout,
    texture_layout: wgpu::BindGroupLayout,
    /// The atlas's one sampler binding; `pipeline_texture` loads integer texels itself.
    sampler: wgpu::Sampler,
    group: wgpu::BindGroup,
    view: wgpu::Buffer,
    frame: wgpu::Buffer,
    vertices: wgpu::Buffer,
    /// The load `pipelines` was built from; 0 before any.
    generation: u64,
    /// By slot; `None` for a pipeline the load refused.
    pipelines: Vec<Option<Compiled>>,
    runs: Vec<Run>,
}

impl ModPipelines {
    /// Storage for pipelines drawing into `color_format` over a reversed Depth32Float depth.
    pub fn new(device: &wgpu::Device, color_format: wgpu::TextureFormat) -> Self {
        let uniform = |binding, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Mod pipelines"),
            entries: &[
                uniform(0, wgpu::ShaderStages::VERTEX),
                uniform(1, wgpu::ShaderStages::FRAGMENT),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Mod pipelines"),
            bind_group_layouts: &[Some(&group_layout)],
            ..Default::default()
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Mod pipeline texture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                uniform(2, wgpu::ShaderStages::FRAGMENT),
            ],
        });
        let textured_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Mod textured pipelines"),
            bind_group_layouts: &[Some(&group_layout), Some(&texture_layout)],
            ..Default::default()
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Mod pipeline texture"),
            ..Default::default()
        });
        let buffer = |label, size, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let view = buffer("Mod pipeline view", 64, wgpu::BufferUsages::UNIFORM);
        let frame = buffer(
            "Mod pipeline frame",
            std::mem::size_of::<FrameUniform>() as u64,
            wgpu::BufferUsages::UNIFORM,
        );
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Mod pipelines"),
            layout: &group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: view.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: frame.as_entire_binding(),
                },
            ],
        });
        Self {
            color_format,
            layout,
            textured_layout,
            texture_layout,
            sampler,
            group,
            view,
            frame,
            vertices: buffer("Mod pipeline quads", 0, wgpu::BufferUsages::VERTEX),
            generation: 0,
            pipelines: Vec::new(),
            runs: Vec::new(),
        }
    }

    /// One load's pipeline: its checked shader with the fixed state it registered, and its
    /// texture's bind group when it has one, or the GPU's error.
    fn pipeline(
        &self,
        device: &wgpu::Device,
        built: &Built,
        textures: Option<TextureSources<'_>>,
    ) -> Result<Compiled, String> {
        let texture = match &built.texture {
            None => None,
            Some(texture) => {
                let textures = textures.ok_or("no texture can be bound here")?;
                Some(self.texture_group(device, texture, textures)?)
            }
        };
        // The shader text is a mod's or a pack's: a backend failing to compile it is an
        // internal error, and neither kind may reach the device's uncaptured handler.
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Mod pipeline"),
            source: wgpu::ShaderSource::Wgsl(built.shader.as_str().into()),
        });
        let attributes = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2,2=>Unorm8x4];
        // Blending happens in the frame's storage, as every vanilla draw's does. Neither mode
        // writes the frame's alpha past what covering it means.
        let blend = match built.blend {
            PipelineBlend::Additive => wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::SrcAlpha,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Zero,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
            },
            _ => wgpu::BlendState::ALPHA_BLENDING,
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Mod pipeline"),
            layout: Some(if texture.is_some() {
                &self.textured_layout
            } else {
                &self.layout
            }),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some(VERTEX_ENTRY),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<PipelineVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                // Two-sided, as world quads are.
                cull_mode: None,
                ..Default::default()
            },
            // No depth is written. Reversed depth: nearer is greater.
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(match built.depth {
                    WorldDepth::ThroughWalls => wgpu::CompareFunction::Always,
                    _ => wgpu::CompareFunction::GreaterEqual,
                }),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(FRAGMENT_ENTRY),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: self.color_format,
                    blend: Some(blend),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let internal = pollster::block_on(internal.pop());
        let validation = pollster::block_on(validation.pop());
        match internal.or(validation) {
            None => Ok((pipeline, texture)),
            Some(error) => Err(error.to_string()),
        }
    }

    /// The bind group of a pipeline's texture: the atlas and its sprite's rect. Made when the
    /// pipeline is built, never in a frame; a sprite the atlas lacks is refused by name.
    fn texture_group(
        &self,
        device: &wgpu::Device,
        texture: &BuiltTexture,
        sources: TextureSources<'_>,
    ) -> Result<wgpu::BindGroup, String> {
        use wgpu::util::DeviceExt;
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let BuiltTexture::Sprite(name) = texture;
        let Some(rect) = (sources.sprite)(name) else {
            drop(pollster::block_on(validation.pop()));
            return Err(format!(
                "its texture sprite {name}: {}",
                crate::mod_pipelines::texture_refusal(mod_api::PipelineTextureError::NoSuchSprite)
            ));
        };
        let view = sources.atlas.clone();
        let rect = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Mod pipeline texture rect"),
            contents: bytemuck::cast_slice(&rect),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Mod pipeline texture"),
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: rect.as_entire_binding(),
                },
            ],
        });
        match pollster::block_on(validation.pop()) {
            None => Ok(group),
            Some(error) => Err(format!("its texture: {error}")),
        }
    }

    /// Builds the pipelines of `set` unless they are built already. A required pipeline the GPU
    /// refuses is the error, by id, and leaves this renderer and `set` as they were; an optional
    /// one is logged once, draws nothing and resolves invalid.
    /// A textured pipeline whose texture can't be bound (`textures`) is refused the same way.
    fn build(
        &mut self,
        device: &wgpu::Device,
        set: &Pipelines,
        textures: Option<TextureSources<'_>>,
    ) -> Result<(), String> {
        self.build_with(set, |this, built| this.pipeline(device, built, textures))
    }

    fn build_with(
        &mut self,
        set: &Pipelines,
        compile: impl Fn(&Self, &Built) -> Result<Compiled, String>,
    ) -> Result<(), String> {
        if set.generation() == self.generation {
            return Ok(());
        }
        let mut refused = Vec::new();
        let pipelines: Vec<_> = set
            .built()
            .iter()
            .enumerate()
            .map(|(slot, built)| {
                let built = built.as_ref()?;
                compile(self, built)
                    .map_err(|error| refused.push((slot, built, error)))
                    .ok()
            })
            .collect();
        let required: Vec<_> = refused
            .iter()
            .filter(|(_, built, _)| !built.optional)
            .map(|(_, built, error)| format!("required {}", gpu_refusal(built, error)))
            .collect();
        if !required.is_empty() {
            return Err(required.join("; "));
        }
        for (slot, built, error) in refused {
            if set.invalidate(slot) {
                log::warn!("Horizon: {}", gpu_refusal(built, &error));
            }
        }
        for (slot, pipeline) in pipelines.iter().enumerate() {
            if pipeline.is_some() {
                set.revalidate(slot);
            }
        }
        self.generation = set.generation();
        self.pipelines = pipelines;
        Ok(())
    }

    /// Uploads the frame's quads, drawn with `view_proj` into a frame `viewport` pixels big. The
    /// number of quads to draw: the caller's quad indices must reach that far. A set this
    /// renderer was not built with draws nothing.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        upload: Upload<'_>,
        batch: &Batch<'_>,
        view_proj: Mat4,
        viewport: [u32; 2],
    ) -> u32 {
        self.runs.clear();
        if batch
            .set
            .is_none_or(|set| set.generation() != self.generation)
        {
            return 0;
        }
        let mut next = 0;
        for run in batch.runs {
            let quads = next..next + run.quads;
            next = quads.end;
            // A run past the vertices given, or of a pipeline this load has none for, has
            // nothing to draw with.
            if quads.end as usize * 4 > batch.vertices.len()
                || !matches!(self.pipelines.get(run.slot as usize), Some(Some(_)))
            {
                continue;
            }
            self.runs.push(Run {
                slot: run.slot as usize,
                stage: run.stage as usize,
                quads,
            });
        }
        let Some(last) = self.runs.last() else {
            return 0;
        };
        let quads = last.quads.end;
        let bytes: &[u8] = bytemuck::cast_slice(&batch.vertices[..quads as usize * 4]);
        if self.vertices.size() < bytes.len() as u64 {
            self.vertices = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Mod pipeline quads"),
                size: (bytes.len() as u64).next_power_of_two(),
                usage: self.vertices.usage(),
                mapped_at_creation: false,
            });
        }
        upload(&self.vertices, bytes);
        upload(&self.view, bytemuck::cast_slice(&view_proj.to_cols_array()));
        upload(
            &self.frame,
            bytemuck::bytes_of(&FrameUniform {
                viewport: [viewport[0] as f32, viewport[1] as f32],
                time: batch.time,
                _pad: 0.0,
            }),
        );
        quads
    }

    /// Draws the prepared runs of stages `first..=last` in the order queued, with `indices`
    /// (6 a quad, `u32`). Leaves the pass with its own pipeline, bind group 0 and buffers, and
    /// bind group 1 after a textured pipeline.
    pub fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        indices: &wgpu::Buffer,
        first: usize,
        last: usize,
    ) {
        let mut bound = false;
        for run in &self.runs {
            if run.stage < first || run.stage > last {
                continue;
            }
            let Some(Some((pipeline, texture))) = self.pipelines.get(run.slot) else {
                continue;
            };
            if !bound {
                bound = true;
                pass.set_bind_group(0, &self.group, &[]);
                pass.set_vertex_buffer(0, self.vertices.slice(..));
                pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
            }
            pass.set_pipeline(pipeline);
            // Only a textured pipeline has group 1; the others draw as they did before it.
            if let Some(texture) = texture {
                pass.set_bind_group(1, texture, &[]);
            }
            pass.draw_indexed(run.quads.start * 6..run.quads.end * 6, 0, 0..1);
        }
    }
}

fn gpu_refusal(built: &Built, error: &str) -> String {
    format!(
        "render pipeline {} from mod {} refused by the GPU: {error}",
        built.id, built.owner
    )
}

impl super::Renderer {
    /// Builds the mods' pipelines of the load this renderer is made for, before the load is
    /// published: the required ones the GPU refuses are the error, by id. A textured pipeline's
    /// sprite is looked up with `sprite` (its `[u0, v0, u1, v1]` in the entity atlas, the 2D
    /// atlas world primitives draw from), and a sprite it doesn't find refuses that pipeline.
    pub fn build_mod_pipelines(
        &mut self,
        set: &Pipelines,
        sprite: &dyn Fn(&str) -> Option<[f32; 4]>,
    ) -> Result<(), String> {
        let textures = TextureSources {
            atlas: &self.entity_atlas,
            sprite,
        };
        self.world_primitives
            .pipelines
            .build(&self.device, set, Some(textures))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::world_primitives::{self, STAGES, WorldPrimitives};
    use glam::Vec3;
    use mod_api::{Identifier, RenderPipeline};

    const SIZE: u32 = 64;
    /// The frame before any quad: a quarter grey, stored as 64 in 8 bits.
    const CLEAR: f64 = 64.0 / 255.0;

    const PASS: &str =
        "fn pipeline_fragment(input: PipelineInput) -> vec4<f32> { return input.color; }";
    const HALF: &str = "fn pipeline_fragment(input: PipelineInput) -> vec4<f32> { return vec4<f32>(vec3<f32>(0.5), 1.0); }";
    const INPUTS: &str = "fn pipeline_fragment(input: PipelineInput) -> vec4<f32> {
    return vec4<f32>(step(0.5, input.uv.x), frame.time, step(0.5, input.uv.y), 1.0);
}";

    fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = crate::gfx::test_gpu::instance();
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        pollster::block_on(adapter.request_device(&Default::default())).ok()
    }

    /// The load of `generation` with one optional pipeline for each shader, in slot order.
    fn load(generation: u64, shaders: &[(&str, PipelineBlend, WorldDepth)]) -> Pipelines {
        let registered: Vec<_> = (0..shaders.len())
            .map(|slot| {
                let id = Identifier::parse(&format!("test:p{slot}")).unwrap();
                RenderPipeline::new(id.clone(), id, shaders[slot].1, shaders[slot].2).optional()
            })
            .collect();
        Pipelines::build(&registered, generation, |file| {
            let slot: usize = file
                .strip_prefix("assets/test/shaders/pipeline/p")
                .and_then(|rest| rest.strip_suffix(".wgsl"))
                .unwrap()
                .parse()
                .unwrap();
            Ok(shaders[slot].0.to_owned())
        })
        .unwrap()
    }

    fn view_proj() -> Mat4 {
        crate::gfx::world_projection(70.0, 1.0, 512.0)
    }

    /// A quad facing the camera `z` blocks ahead of it, over the pixels `x0..x1`, `y0..y1` of
    /// the frame (y down), its texture coordinates spanning 0 to 1 and `color` as stored bytes.
    fn quad([x0, y0, x1, y1]: [f32; 4], z: f32, color: [u8; 4]) -> [PipelineVertex; 4] {
        let depth = view_proj().project_point3(Vec3::new(0.0, 0.0, -z)).z;
        let inverse = view_proj().inverse();
        let at = |x: f32, y: f32, uv: [f32; 2]| PipelineVertex {
            pos: inverse
                .project_point3(Vec3::new(
                    x / SIZE as f32 * 2.0 - 1.0,
                    1.0 - y / SIZE as f32 * 2.0,
                    depth,
                ))
                .to_array(),
            uv,
            color: u32::from_le_bytes(color),
        };
        [
            at(x0, y0, [0.0, 0.0]),
            at(x0, y1, [0.0, 1.0]),
            at(x1, y1, [1.0, 1.0]),
            at(x1, y0, [1.0, 0.0]),
        ]
    }

    /// Draws `batch`'s stages up to `last` over the cleared frame of `format`, behind a wall
    /// 5 blocks ahead, and reads the frame back as RGBA floats.
    fn render(
        (device, queue): &(wgpu::Device, wgpu::Queue),
        primitives: &mut WorldPrimitives,
        format: wgpu::TextureFormat,
        batch: Batch<'_>,
        last: usize,
    ) -> impl Fn(u32, u32) -> [f32; 4] + use<> {
        let texture = |size: u32, format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let color = texture(
            SIZE,
            format,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let depth = texture(
            SIZE,
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let atlas = texture(
            1,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let wall = view_proj().project_point3(Vec3::new(0.0, 0.0, -5.0)).z;
        if let Some(set) = batch.set {
            primitives.pipelines.build(device, set, None).unwrap();
        }
        let drawn = primitives.prepare(
            device,
            &mut |buffer, bytes| queue.write_buffer(buffer, 0, bytes),
            &world_primitives::Batch {
                pipelines: batch,
                ..Default::default()
            },
            view_proj(),
            [SIZE, SIZE],
            &atlas.create_view(&Default::default()),
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let color_view = color.create_view(&Default::default());
            let depth_view = depth.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: CLEAR,
                            g: CLEAR,
                            b: CLEAR,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wall),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            if drawn {
                primitives.draw_through(&mut pass, &mut 0, last);
            }
        }
        let float = format == wgpu::TextureFormat::Rgba16Float;
        let texel = if float { 8 } else { 4 };
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(SIZE * SIZE * texel),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            color.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(SIZE * texel),
                    rows_per_image: None,
                },
            },
            color.size(),
        );
        queue.submit([encoder.finish()]);
        readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let bytes = readback.slice(..).get_mapped_range().unwrap().to_vec();
        move |x: u32, y: u32| {
            let i = ((y * SIZE + x) * texel) as usize;
            std::array::from_fn(|channel| {
                if float {
                    let at = i + channel * 2;
                    half::f16::from_le_bytes([bytes[at], bytes[at + 1]]).to_f32()
                } else {
                    f32::from(bytes[i + channel])
                }
            })
        }
    }

    fn assert_near(got: [f32; 4], expected: [f32; 4], tolerance: f32, what: &str) {
        assert!(
            got.iter()
                .zip(expected)
                .all(|(got, expected)| (got - expected).abs() <= tolerance),
            "{what}: got {got:?}, expected {expected:?}"
        );
    }

    /// One frame with every part of a pipeline's contract in its own corner. Expected bytes
    /// are vanilla's 2.2 encoding worked out by hand: linear 0.5 is 0.5^(1/2.2) = 0.7297,
    /// stored as 186.
    #[test]
    fn gpu_validation_failure_is_contained_and_invalidates_handle() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, _)) = device() else {
            return;
        };
        let set = load(
            41,
            &[(PASS, PipelineBlend::Translucent, WorldDepth::Tested)],
        );
        let id = Identifier::parse("test:p0").unwrap();
        let handle = set.resolve(&id).unwrap();
        let renderer = ModPipelines::new(&device, wgpu::TextureFormat::Rgba8Unorm);
        let mut broken = set.built()[0].clone().unwrap();
        broken.owner = "owning_mod".into();
        assert_eq!(
            gpu_refusal(&broken, "compiler refusal"),
            "render pipeline test:p0 from mod owning_mod refused by the GPU: compiler refusal"
        );
        broken.shader = "not wgsl".into();
        assert!(renderer.pipeline(&device, &broken, None).is_err());
        assert!(set.invalidate(0));
        assert_eq!(set.resolve(&id), Err(mod_api::RenderPipelineError::Invalid));
        assert_eq!(set.slot(handle), Err(mod_api::RenderPipelineError::Invalid));
    }

    #[test]
    fn optional_gpu_refusal_is_published_only_once() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, _)) = device() else {
            return;
        };
        let set = load(
            41,
            &[(PASS, PipelineBlend::Translucent, WorldDepth::Tested)],
        );
        let id = Identifier::parse("test:p0").unwrap();
        let handle = set.resolve(&id).unwrap();
        let mut renderer = ModPipelines::new(&device, wgpu::TextureFormat::Rgba8Unorm);
        renderer
            .build_with(&set, |_, _| Err("injected refusal".into()))
            .unwrap();
        assert_eq!(renderer.generation, 41);
        assert!(renderer.pipelines[0].is_none());
        assert_eq!(set.slot(handle), Err(mod_api::RenderPipelineError::Invalid));
        assert!(!set.invalidate(0));
        renderer
            .build_with(&set, |_, _| panic!("already compiled"))
            .unwrap();
    }

    #[test]
    fn a_later_successful_build_makes_a_refused_slot_usable_again() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, _)) = device() else {
            return;
        };
        let set = load(
            41,
            &[(PASS, PipelineBlend::Translucent, WorldDepth::Tested)],
        );
        let id = Identifier::parse("test:p0").unwrap();
        let handle = set.resolve(&id).unwrap();
        let mut refusing = ModPipelines::new(&device, wgpu::TextureFormat::Rgba8Unorm);
        refusing
            .build_with(&set, |_, _| Err("injected refusal".into()))
            .unwrap();
        assert_eq!(set.slot(handle), Err(mod_api::RenderPipelineError::Invalid));
        let mut recovered = ModPipelines::new(&device, wgpu::TextureFormat::Rgba8Unorm);
        recovered.build(&device, &set, None).unwrap();
        assert!(recovered.pipelines[0].is_some());
        assert_eq!(set.slot(handle), Ok(0));
        assert!(set.resolve(&id).is_ok());
    }

    #[test]
    fn required_gpu_failure_does_not_publish_and_mixed_success_does() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, _)) = device() else {
            return;
        };
        let set = load(
            41,
            &[(PASS, PipelineBlend::Translucent, WorldDepth::Tested)],
        );
        let mut renderer = ModPipelines::new(&device, wgpu::TextureFormat::Rgba8Unorm);
        renderer.build(&device, &set, None).unwrap();
        let shaders = [
            (PASS, PipelineBlend::Translucent, WorldDepth::Tested),
            (PASS, PipelineBlend::Translucent, WorldDepth::Tested),
        ];
        let mut registered: Vec<_> = (0..shaders.len())
            .map(|slot| {
                let id = Identifier::parse(&format!("test:p{slot}")).unwrap();
                RenderPipeline::new(id.clone(), id, shaders[slot].1, shaders[slot].2).optional()
            })
            .collect();
        registered[0].optional = false;
        let next = Pipelines::build(&registered, 42, |_| Ok(PASS.to_owned())).unwrap();
        let id = Identifier::parse("test:p0").unwrap();
        let handle = next.resolve(&id).unwrap();
        let error = renderer
            .build_with(&next, |_, _| Err("injected refusal".into()))
            .unwrap_err();
        assert_eq!(
            error,
            "required render pipeline test:p0 from mod  refused by the GPU: injected refusal"
        );
        assert_eq!(renderer.generation, 41);
        assert_eq!(renderer.pipelines.len(), 1);
        assert_eq!(next.slot(handle), Ok(0));
        assert!(next.resolve(&Identifier::parse("test:p1").unwrap()).is_ok());
        renderer.build(&device, &next, None).unwrap();
        assert_eq!(renderer.generation, 42);
        assert_eq!(renderer.pipelines.len(), 2);
        assert!(renderer.pipelines.iter().all(Option::is_some));
    }

    #[test]
    fn pipelines_convert_colour_both_ways_and_keep_their_blend_depth_and_stage() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(gpu) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        use PipelineBlend::{Additive, Translucent};
        use WorldDepth::{Tested, ThroughWalls};
        let set = load(
            3,
            &[
                (PASS, Translucent, Tested),
                (HALF, Translucent, ThroughWalls),
                // Refused by the load: the pipelines after it still draw.
                ("fn nothing() {}", Translucent, Tested),
                (HALF, Additive, Tested),
                (INPUTS, Translucent, Tested),
            ],
        );
        assert!(set.built()[2].is_none());
        let grey = [128, 128, 128, 255];
        let white = [255; 4];
        let vertices = [
            // Top left: a grey quad returned as it came, in front of the wall and behind it.
            quad([0.0, 0.0, 16.0, 32.0], 2.0, grey),
            quad([16.0, 0.0, 32.0, 32.0], 10.0, grey),
            // Top right: linear 0.5 through walls, at a later stage.
            quad([32.0, 0.0, 64.0, 32.0], 10.0, white),
            // Bottom left: linear 0.5 added to the frame.
            quad([0.0, 32.0, 32.0, 64.0], 2.0, white),
            // Bottom right: the texture coordinate and the frame's time.
            quad([32.0, 32.0, 64.0, 64.0], 2.0, white),
        ]
        .concat();
        let run = |slot, stage, quads| PipelineRun { slot, stage, quads };
        let runs = [run(0, 0, 2), run(1, 2, 1), run(3, 0, 1), run(4, 0, 1)];
        let batch = Batch {
            set: Some(&set),
            vertices: &vertices,
            runs: &runs,
            time: 1.0,
        };
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&gpu.0, (format, false));

        let at = render(&gpu, &mut primitives, format, batch, STAGES - 1);
        let clear = [64.0, 64.0, 64.0, 255.0];
        assert_near(
            at(8, 16),
            [128.0, 128.0, 128.0, 255.0],
            1.0,
            "decoded on the way in and encoded on the way out, a colour comes back as it was",
        );
        assert_near(
            at(24, 16),
            clear,
            0.0,
            "depth tested: hidden behind the wall",
        );
        assert_near(
            at(48, 16),
            [186.0, 186.0, 186.0, 255.0],
            1.0,
            "linear 0.5 through walls",
        );
        assert_near(
            at(16, 48),
            [250.0, 250.0, 250.0, 255.0],
            1.0,
            "additive: 186 over the frame's 64",
        );
        assert_near(
            at(40, 40),
            [0.0, 255.0, 0.0, 255.0],
            0.0,
            "the quad's first quarter: uv under 0.5, time 1",
        );
        assert_near(
            at(56, 56),
            [255.0, 255.0, 255.0, 255.0],
            0.0,
            "the quad's last quarter: uv over 0.5",
        );

        // Only the first stage: the later stage's quad is not drawn yet.
        let at = render(&gpu, &mut primitives, format, batch, 0);
        assert_near(
            at(48, 16),
            clear,
            0.0,
            "a later stage's quad waits for its stage",
        );
        assert_near(
            at(8, 16),
            [128.0, 128.0, 128.0, 255.0],
            1.0,
            "the first stage draws",
        );
    }

    /// In a float frame light above white survives, stored as vanilla encodes it (linear 2.0
    /// is 2^(1/2.2) = 1.37035), and a new load's shader replaces the old one's pipeline.
    #[test]
    fn a_float_frame_keeps_light_above_white_and_a_reload_builds_the_new_shader() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(gpu) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let bright = "fn pipeline_fragment(input: PipelineInput) -> vec4<f32> { return vec4<f32>(2.0, 1.0, 0.0, 1.0); }";
        let green = "fn pipeline_fragment(input: PipelineInput) -> vec4<f32> { return vec4<f32>(0.0, 1.0, 0.0, 1.0); }";
        let state = (PipelineBlend::Translucent, WorldDepth::Tested);
        let vertices = quad([0.0, 0.0, 64.0, 64.0], 2.0, [255; 4]);
        let runs = [PipelineRun {
            slot: 0,
            stage: 4,
            quads: 1,
        }];
        let format = wgpu::TextureFormat::Rgba16Float;
        let mut primitives = WorldPrimitives::new(&gpu.0, (format, false));
        let mut draw = |set: &Pipelines| {
            let batch = Batch {
                set: Some(set),
                vertices: &vertices,
                runs: &runs,
                time: 0.0,
            };
            render(&gpu, &mut primitives, format, batch, STAGES - 1)(32, 32)
        };

        let before = load(1, &[(bright, state.0, state.1)]);
        assert_near(
            draw(&before),
            [1.37035, 1.0, 0.0, 1.0],
            0.002,
            "linear 2.0 in a float frame",
        );
        let after = load(2, &[(green, state.0, state.1)]);
        assert_near(
            draw(&after),
            [0.0, 1.0, 0.0, 1.0],
            0.002,
            "the reloaded shader",
        );
    }

    /// A textured pipeline samples a 2x1 sprite's two texels across its quad, and one naming
    /// half the atlas samples only that sprite's part, repeating it past 1. A sprite the
    /// atlas lacks refuses the optional pipeline, and an untextured one in the same load still
    /// draws as before. Expected bytes: stored red and green stay 255 through decode and encode.
    #[test]
    fn textured_pipelines_sample_their_file_or_their_sprite() {
        use mod_api::PipelineTexture;
        use wgpu::util::DeviceExt;
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(gpu) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let (device, queue) = (&gpu.0, &gpu.1);
        let sample = "fn pipeline_fragment(input: PipelineInput) -> vec4<f32> { return pipeline_texture(input.uv); }";
        let id = |slot: usize| Identifier::parse(&format!("test:p{slot}")).unwrap();
        let state = (PipelineBlend::Translucent, WorldDepth::Tested);
        let pipeline = |slot| RenderPipeline::new(id(slot), id(slot), state.0, state.1).optional();
        let registered = [
            // A 2x1 sprite, the whole atlas: red on the left, green on the right.
            pipeline(0).textured(PipelineTexture::Sprite(id(0))),
            // The atlas's right half, green.
            pipeline(1).textured(PipelineTexture::Sprite(id(1))),
            pipeline(2).textured(PipelineTexture::Sprite(
                Identifier::parse("test:none").unwrap(),
            )),
            pipeline(3),
        ];
        let red_green = vec![255, 0, 0, 255, 0, 255, 0, 255];
        let set = Pipelines::build(&registered, 7, |file| {
            Ok(if file.ends_with("p3.wgsl") {
                PASS
            } else {
                sample
            }
            .to_owned())
        })
        .unwrap();
        let atlas = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 2,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &red_green,
        );
        let atlas = atlas.create_view(&Default::default());
        let sprite = |name: &str| match name {
            "test:p0" => Some([0.0, 0.0, 1.0, 1.0]),
            "test:p1" => Some([0.5, 0.0, 1.0, 1.0]),
            _ => None,
        };
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(device, (format, false));
        let sources = TextureSources {
            atlas: &atlas,
            sprite: &sprite,
        };
        primitives
            .pipelines
            .build(device, &set, Some(sources))
            .unwrap();
        assert_eq!(
            set.resolve(&id(2)),
            Err(mod_api::RenderPipelineError::Invalid),
            "a sprite the atlas lacks"
        );
        assert!(
            primitives.pipelines.pipelines[0]
                .as_ref()
                .unwrap()
                .1
                .is_some()
        );
        assert!(
            primitives.pipelines.pipelines[3]
                .as_ref()
                .unwrap()
                .1
                .is_none()
        );
        let mut vertices = Vec::new();
        for (row, slot) in [(0.0, 0), (16.0, 1), (32.0, 3)] {
            let mut quad = quad([0.0, row, 64.0, row + 16.0], 2.0, [255, 255, 255, 255]);
            if slot == 1 {
                // Twice across: the sprite repeats.
                quad.iter_mut().for_each(|v| v.uv[0] *= 2.0);
            }
            vertices.extend(quad);
        }
        let runs = [0, 1, 3].map(|slot| PipelineRun {
            slot,
            stage: 4,
            quads: 1,
        });
        let pixel = render(
            &gpu,
            &mut primitives,
            format,
            Batch {
                set: Some(&set),
                vertices: &vertices,
                runs: &runs,
                time: 0.0,
            },
            STAGES - 1,
        );
        let (red, green) = ([255.0, 0.0, 0.0, 255.0], [0.0, 255.0, 0.0, 255.0]);
        assert_near(pixel(8, 8), red, 1.0, "the 2x1 sprite's left texel");
        assert_near(pixel(56, 8), green, 1.0, "the 2x1 sprite's right texel");
        assert_near(pixel(8, 24), green, 1.0, "the sprite, not the atlas's red");
        assert_near(pixel(40, 24), green, 1.0, "the sprite repeated");
        assert_near(pixel(8, 40), [255.0; 4], 1.0, "the untextured pipeline");
    }
}
