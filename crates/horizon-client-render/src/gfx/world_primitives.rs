//! Native mods' world primitives (`native_world_primitives`, docs/native-mods.md): lines, boxes,
//! beams and text a mod hands the game each frame, drawn here with Horizon's own pipelines in
//! the world pass at the stages they were given (`render_level_stage`), the last one after the
//! entities and before the first-person hand. They draw
//! into the world's targets, so they take the world's reversed depth, its draw resolution and
//! TAA's jitter; a mod never touches wgpu for them.

use super::Vertex;
use glam::Mat4;

/// Initial per-view line and quad limits, shared by CPU submission and GPU setup.
pub const DEFAULT_CAPACITY: u32 = 4096;

/// The stages a frame draws primitives at, in frame order (`RenderLevelStage`).
pub const STAGES: usize = 5;
/// world_primitives.wgsl after what it shares with the frame's other shaders (`frame_stored`).
pub(super) fn shader() -> String {
    include_str!("../hdr_light.wgsl").to_owned() + include_str!("world_primitives.wgsl")
}

/// A frame's primitives, camera-relative, as the game tessellated them: quads of 4 vertices
/// from the shared atlas (plain colours on its white texel, text on its glyphs, beams on the
/// beacon beam sprite) and lines as `gizmos::camera_relative` gives them. `*_through` ones are
/// seen through walls: no depth test.
#[derive(Clone, Copy, Debug, Default)]
pub struct Batch<'a> {
    pub quads: &'a [Vertex],
    pub quads_through: &'a [Vertex],
    pub lines: &'a [[f32; 8]],
    pub lines_through: &'a [[f32; 8]],
    /// Where each stage but the last ends, as counts of quads, quads through walls, lines and
    /// lines through walls: each slice holds its stages in frame order, and what follows the
    /// last end is the last stage's. All zero, everything draws at the last stage.
    pub stage_ends: [[u32; 4]; STAGES - 1],
    /// The quads mods draw with their own pipelines (`gfx::mod_pipelines`), with their stages.
    pub pipelines: super::mod_pipelines::Batch<'a>,
}

impl Batch<'_> {
    pub fn is_empty(&self) -> bool {
        self.pipelines.runs.is_empty()
            && self.quads.is_empty()
            && self.quads_through.is_empty()
            && self.lines.is_empty()
            && self.lines_through.is_empty()
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    viewport: [f32; 4],
}

/// The pipelines and buffers that draw a [`Batch`].
pub struct WorldPrimitives {
    /// Depth tested, then seen through walls.
    quad_pipelines: [wgpu::RenderPipeline; 2],
    line_pipelines: [wgpu::RenderPipeline; 2],
    layout: wgpu::BindGroupLayout,
    globals: wgpu::Buffer,
    sampler: wgpu::Sampler,
    quads: wgpu::Buffer,
    lines: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_quads: u32,
    group: Option<AtlasGroup>,
    quad_vertices: Vec<Vertex>,
    line_instances: Vec<[f32; 8]>,
    index_data: Vec<u32>,
    /// Quads depth tested, quads through walls, lines depth tested, lines through walls.
    counts: [u32; 4],
    /// The batch's `stage_ends`, clamped to `counts`.
    stage_ends: [[u32; 4]; STAGES - 1],
    /// Mods' own pipelines and their quads, drawn after each stage's primitives.
    pub(super) pipelines: super::mod_pipelines::ModPipelines,
}

struct AtlasGroup {
    view: wgpu::TextureView,
    group: wgpu::BindGroup,
}

/// Copies bytes into a buffer as part of the frame (the renderer's staging uploads, or the
/// queue in a test). The callback consumes the bytes before returning; they may be reused.
pub type Upload<'u> = &'u mut dyn FnMut(&wgpu::Buffer, &[u8]);

impl WorldPrimitives {
    /// Pipelines drawing into `color_format` over a reversed Depth32Float depth, decoding their
    /// colours where the frame holds linear light (`linear`).
    pub fn new(device: &wgpu::Device, (color_format, linear): (wgpu::TextureFormat, bool)) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("World primitives"),
            source: wgpu::ShaderSource::Wgsl(shader().into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("World primitives"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("World primitives"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let quad_attributes =
            wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2,2=>Uint32,3=>Unorm8x4,4=>Uint32];
        let line_attributes =
            wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Unorm8x4,3=>Float32];
        let quad_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &quad_attributes,
        })];
        let line_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: 32,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &line_attributes,
        })];
        let pipeline = |label: &str,
                        entry: [&str; 2],
                        buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
                        topology,
                        through: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(entry[0]),
                    compilation_options: Default::default(),
                    buffers,
                },
                primitive: wgpu::PrimitiveState {
                    topology,
                    // Seen from inside or behind as well: a box around the camera, a beam.
                    cull_mode: None,
                    ..Default::default()
                },
                // Blended over what is drawn, writing no depth: a translucent box doesn't hide
                // the primitives behind it. Reversed depth: nearer is greater.
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(if through {
                        wgpu::CompareFunction::Always
                    } else {
                        wgpu::CompareFunction::GreaterEqual
                    }),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry[1]),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &super::linear_blend::frame_constants(color_format, linear),
                        ..Default::default()
                    },
                    targets: &[Some(wgpu::ColorTargetState {
                        format: color_format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let quads = ["vs_quad", "fs_quad"];
        let lines = ["vs_line", "fs_line"];
        let list = wgpu::PrimitiveTopology::TriangleList;
        let strip = wgpu::PrimitiveTopology::TriangleStrip;
        let quad_pipelines = [
            pipeline("World primitives", quads, &quad_buffers, list, false),
            pipeline(
                "World primitives through walls",
                quads,
                &quad_buffers,
                list,
                true,
            ),
        ];
        let line_pipelines = [
            pipeline("World primitive lines", lines, &line_buffers, strip, false),
            pipeline(
                "World primitive lines through walls",
                lines,
                &line_buffers,
                strip,
                true,
            ),
        ];
        let buffer = |label, size, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        Self {
            quad_pipelines,
            line_pipelines,
            globals: buffer(
                "World primitive globals",
                std::mem::size_of::<Globals>() as u64,
                wgpu::BufferUsages::UNIFORM,
            ),
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("World primitives"),
                ..Default::default()
            }),
            layout,
            quads: buffer("World primitive quads", 0, wgpu::BufferUsages::VERTEX),
            lines: buffer("World primitive lines", 0, wgpu::BufferUsages::VERTEX),
            indices: buffer("World primitive indices", 0, wgpu::BufferUsages::INDEX),
            index_quads: 0,
            group: None,
            quad_vertices: Vec::new(),
            line_instances: Vec::new(),
            index_data: Vec::new(),
            counts: [0; 4],
            stage_ends: [[0; 4]; STAGES - 1],
            pipelines: super::mod_pipelines::ModPipelines::new(device, color_format),
        }
    }

    /// Reserves CPU and GPU geometry storage for `quads` and `lines`, total across both depth
    /// modes. Call before preparing a frame, at setup to avoid growth during submission. Only
    /// index growth uploads the full index array; prepare geometry again after buffer growth.
    pub fn reserve(&mut self, device: &wgpu::Device, upload: Upload<'_>, quads: u32, lines: u32) {
        self.quad_vertices
            .reserve((quads as usize * 4).saturating_sub(self.quad_vertices.len()));
        self.line_instances
            .reserve((lines as usize).saturating_sub(self.line_instances.len()));
        let grow = |buffer: &mut wgpu::Buffer, bytes: u64| {
            if buffer.size() < bytes {
                *buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("World primitives"),
                    size: bytes.next_power_of_two(),
                    usage: buffer.usage(),
                    mapped_at_creation: false,
                });
            }
        };
        grow(
            &mut self.quads,
            quads as u64 * 4 * std::mem::size_of::<Vertex>() as u64,
        );
        grow(&mut self.lines, lines as u64 * 32);
        self.reserve_indices(device, upload, quads);
    }

    /// Reserves the shared quad indices for `quads` quads; growth uploads the full array.
    fn reserve_indices(&mut self, device: &wgpu::Device, upload: Upload<'_>, quads: u32) {
        if quads > self.index_quads {
            let capacity = quads.next_power_of_two();
            self.index_data.extend(
                (self.index_quads..capacity).flat_map(|q| [0, 1, 2, 0, 2, 3].map(|i| q * 4 + i)),
            );
            self.index_quads = capacity;
            let bytes: &[u8] = bytemuck::cast_slice(&self.index_data);
            if self.indices.size() < bytes.len() as u64 {
                self.indices = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("World primitives"),
                    size: (bytes.len() as u64).next_power_of_two(),
                    usage: self.indices.usage(),
                    mapped_at_creation: false,
                });
            }
            upload(&self.indices, bytes);
        }
    }

    /// Uploads this frame's `batch`, drawn with `view_proj` (camera-relative world to clip,
    /// jittered as the world is) into a frame `viewport` pixels big, sampling `atlas`. Whether
    /// there is anything to draw. CPU and GPU storage grows at its high-water mark; subsequent
    /// frames within that capacity reuse it. Bindings are rebuilt only when the atlas view changes.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        upload: Upload<'_>,
        batch: &Batch<'_>,
        view_proj: Mat4,
        viewport: [u32; 2],
        atlas: &wgpu::TextureView,
    ) -> bool {
        let quads = [batch.quads, batch.quads_through].map(|q| (q.len() / 4) as u32);
        let lines = [batch.lines, batch.lines_through].map(|l| l.len() as u32);
        self.counts = [quads[0], quads[1], lines[0], lines[1]];
        self.stage_ends = clamped_ends(batch.stage_ends, self.counts);
        // Before the empty check, which counts the pipeline runs this queues; the pipelines
        // themselves were built with the renderer (`Renderer::build_mod_pipelines`).
        let pipeline_quads =
            self.pipelines
                .prepare(device, upload, &batch.pipelines, view_proj, viewport);
        if batch.is_empty() {
            return false;
        }
        let total_quads = quads[0] + quads[1];
        self.reserve(device, upload, total_quads, lines[0] + lines[1]);
        self.reserve_indices(device, upload, pipeline_quads);
        if total_quads > 0 {
            self.quad_vertices.clear();
            for q in [batch.quads, batch.quads_through] {
                self.quad_vertices.extend_from_slice(&q[..q.len() / 4 * 4]);
            }
            let bytes: &[u8] = bytemuck::cast_slice(&self.quad_vertices);
            upload(&self.quads, bytes);
        }
        if lines[0] + lines[1] > 0 {
            self.line_instances.clear();
            self.line_instances.extend_from_slice(batch.lines);
            self.line_instances.extend_from_slice(batch.lines_through);
            let bytes: &[u8] = bytemuck::cast_slice(&self.line_instances);
            upload(&self.lines, bytes);
        }
        let globals = Globals {
            view_proj: view_proj.to_cols_array_2d(),
            viewport: [viewport[0] as f32, viewport[1] as f32, 0.0, 0.0],
        };
        upload(&self.globals, bytemuck::bytes_of(&globals));
        if self.group.as_ref().is_none_or(|g| g.view != *atlas) {
            self.group = Some(AtlasGroup {
                view: atlas.clone(),
                group: device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("World primitives"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: self.globals.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(atlas),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                    ],
                }),
            });
        }
        true
    }

    /// Draws the stages from `*next` to `stage` that [`Self::prepare`] uploaded into a pass
    /// over the world's colour and depth, and moves `next` past them: a stage the frame did
    /// not reach in this pass draws here, after those before it. Within the stages, the
    /// depth-tested quads and lines, then those seen through walls over everything, then the
    /// mods' pipeline quads in the order queued. Leaves the pass with its own pipeline, bind
    /// group 0 and buffers.
    pub fn draw_through(&self, pass: &mut wgpu::RenderPass<'_>, next: &mut usize, stage: usize) {
        let (first, last) = (*next, stage.min(STAGES - 1));
        if first > last {
            return;
        }
        *next = last + 1;
        let Some(AtlasGroup { group, .. }) = &self.group else {
            return;
        };
        let [quads, quads_through, lines, lines_through] =
            spans(self.counts, &self.stage_ends, first, last);
        if !(quads.is_empty()
            && quads_through.is_empty()
            && lines.is_empty()
            && lines_through.is_empty())
        {
            pass.set_bind_group(0, group, &[]);
        }
        for (through, (quad_range, line_range)) in [(quads, lines), (quads_through, lines_through)]
            .into_iter()
            .enumerate()
        {
            if !quad_range.is_empty() {
                pass.set_pipeline(&self.quad_pipelines[through]);
                pass.set_vertex_buffer(0, self.quads.slice(..));
                pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(quad_range.start * 6..quad_range.end * 6, 0, 0..1);
            }
            if !line_range.is_empty() {
                pass.set_pipeline(&self.line_pipelines[through]);
                pass.set_vertex_buffer(0, self.lines.slice(..));
                pass.draw(0..4, line_range);
            }
        }
        self.pipelines.draw(pass, &self.indices, first, last);
    }
}

/// A batch's stage ends made safe to draw from: never decreasing, never past `counts`.
fn clamped_ends(ends: [[u32; 4]; STAGES - 1], counts: [u32; 4]) -> [[u32; 4]; STAGES - 1] {
    let mut floor = [0; 4];
    ends.map(|end| {
        floor = std::array::from_fn(|i| end[i].clamp(floor[i], counts[i]));
        floor
    })
}

/// The uploaded quads and lines of stages `first..=last`: quads depth tested, quads through
/// walls, lines depth tested, lines through walls, each as a range of its buffer, where the
/// through-walls ones follow all the depth-tested ones.
fn spans(
    counts: [u32; 4],
    ends: &[[u32; 4]; STAGES - 1],
    first: usize,
    last: usize,
) -> [std::ops::Range<u32>; 4] {
    let start = first.checked_sub(1).map_or([0; 4], |stage| ends[stage]);
    let end = ends.get(last).copied().unwrap_or(counts);
    let [quads, _, lines, _] = counts;
    [
        start[0]..end[0],
        quads + start[1]..quads + end[1],
        start[2]..end[2],
        lines + start[3]..lines + end[3],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    const SIZE: u32 = 64;
    /// RGBA, as `Vertex::color` holds it.
    const RED: u32 = 0xFF00_00FF;
    const GREEN: u32 = 0xFF00_FF00;
    const BLUE: u32 = 0xFFFF_0000;
    /// ARGB, as a line carries it.
    const MAGENTA_ARGB: u32 = 0xFFFF_00FF;

    /// A camera-relative quad of plain colour facing the camera, `z` blocks ahead of it, over
    /// the pixels `x0..x1`, `y0..y1` of the frame (y down).
    fn quad(view_proj: Mat4, [x0, y0, x1, y1]: [f32; 4], z: f32, rgba: u32) -> [Vertex; 4] {
        let depth = view_proj.project_point3(Vec3::new(0.0, 0.0, -z)).z;
        let inverse = view_proj.inverse();
        let at = |x: f32, y: f32| {
            let ndc = Vec3::new(
                x / SIZE as f32 * 2.0 - 1.0,
                1.0 - y / SIZE as f32 * 2.0,
                depth,
            );
            inverse.project_point3(ndc).to_array()
        };
        [at(x0, y0), at(x0, y1), at(x1, y1), at(x1, y0)].map(|pos| Vertex {
            pos,
            uv: [0.5, 0.5],
            layer: 0,
            color: rgba,
            light: 0,
            normal: [0; 4],
            glint_uv: [0.0, 0.0],
        })
    }

    /// A horizontal line, 4 pixels wide, over the pixels `x0..x1` of row `y`, `z` blocks ahead.
    fn line(view_proj: Mat4, [x0, x1]: [f32; 2], y: f32, z: f32, argb: u32) -> [f32; 8] {
        let q = quad(view_proj, [x0, y, x1, y], z, 0);
        let [a, b] = [q[0].pos, q[3].pos];
        [
            a[0],
            a[1],
            a[2],
            b[0],
            b[1],
            b[2],
            f32::from_bits(argb),
            4.0,
        ]
    }

    fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = crate::gfx::test_gpu::instance();
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        pollster::block_on(adapter.request_device(&Default::default())).ok()
    }

    #[test]
    fn each_stage_draws_its_own_part_of_every_slice() {
        // Quads, quads through walls, lines, lines through walls: 6, 2, 4, 3 in all.
        let counts = [6, 2, 4, 3];
        // AfterSky has 1 quad; AfterOpaqueBlocks nothing; AfterTranslucentBlocks 2 quads and
        // a line; AfterWeather a quad through walls; AfterLevel the rest.
        let ends = [[1, 0, 0, 0], [1, 0, 0, 0], [3, 0, 1, 0], [3, 1, 1, 0]];
        assert_eq!(spans(counts, &ends, 0, 0), [0..1, 6..6, 0..0, 4..4]);
        assert_eq!(spans(counts, &ends, 1, 1), [1..1, 6..6, 0..0, 4..4]);
        assert_eq!(spans(counts, &ends, 2, 2), [1..3, 6..6, 0..1, 4..4]);
        assert_eq!(spans(counts, &ends, 3, 3), [3..3, 6..7, 1..1, 4..4]);
        assert_eq!(spans(counts, &ends, 4, 4), [3..6, 7..8, 1..4, 4..7]);
        // A pass that skipped the middle stages draws them with the next one, in order.
        assert_eq!(spans(counts, &ends, 1, 3), [1..3, 6..7, 0..1, 4..4]);
        // Everything, as a batch without stages draws.
        assert_eq!(spans(counts, &ends, 0, 4), [0..6, 6..8, 0..4, 4..7]);
        assert_eq!(
            spans(counts, &[[0; 4]; STAGES - 1], 4, 4),
            [0..6, 6..8, 0..4, 4..7]
        );
    }

    #[test]
    fn stage_ends_past_the_counts_or_going_back_are_clamped() {
        let ends = clamped_ends(
            [[9, 0, 2, 0], [1, 0, 1, 0], [9, 9, 9, 9], [0; 4]],
            [6, 2, 4, 3],
        );
        assert_eq!(
            ends,
            [[6, 0, 2, 0], [6, 0, 2, 0], [6, 2, 4, 3], [6, 2, 4, 3]]
        );
    }

    #[test]
    fn uploads_preserve_ranges_and_reuse_high_water_storage() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let atlas_view = atlas.create_view(&Default::default());
        let mut primitives =
            WorldPrimitives::new(&device, (wgpu::TextureFormat::Rgba8Unorm, false));
        let view_proj = Mat4::IDENTITY;
        let red = quad(view_proj, [0.0, 0.0, 32.0, 32.0], 2.0, RED);
        let blue = quad(view_proj, [32.0, 32.0, 64.0, 64.0], 2.0, BLUE);
        // Each slice drops its own trailing partial quad, not just the combined tail.
        let tested = [&red[..], &red[..1]].concat();
        let through = [&blue[..], &blue[..3]].concat();
        let lines = [line(view_proj, [0.0, 64.0], 16.0, 2.0, MAGENTA_ARGB)];
        let lines_through = [line(view_proj, [0.0, 64.0], 48.0, 2.0, MAGENTA_ARGB)];
        let batch = Batch {
            quads: &tested,
            quads_through: &through,
            lines: &lines,
            lines_through: &lines_through,
            ..Default::default()
        };
        assert!(!primitives.prepare(
            &device,
            &mut |_, _| panic!("empty frames must not upload"),
            &Batch::default(),
            view_proj,
            [SIZE, SIZE],
            &atlas_view,
        ));
        assert!(primitives.group.is_none());
        let mut writes = Vec::new();
        primitives.reserve(
            &device,
            &mut |buffer, bytes| writes.push((buffer.clone(), bytes.to_vec())),
            2,
            2,
        );
        let buffers = [
            primitives.quads.clone(),
            primitives.lines.clone(),
            primitives.indices.clone(),
        ];
        let storage = |p: &WorldPrimitives| {
            [
                (
                    p.quad_vertices.as_ptr() as usize,
                    p.quad_vertices.capacity(),
                ),
                (
                    p.line_instances.as_ptr() as usize,
                    p.line_instances.capacity(),
                ),
                (p.index_data.as_ptr() as usize, p.index_data.capacity()),
            ]
        };
        let original_storage = storage(&primitives);
        assert_eq!(writes.len(), 1, "setup uploads only indices");
        assert!(primitives.prepare(
            &device,
            &mut |buffer, bytes| writes.push((buffer.clone(), bytes.to_vec())),
            &batch,
            view_proj,
            [SIZE, SIZE],
            &atlas_view,
        ));
        assert_eq!(primitives.counts, [1, 1, 1, 1]);
        assert_eq!(writes.len(), 4);
        assert_eq!(
            writes[1].1,
            bytemuck::cast_slice::<_, u8>(&[red, blue].concat())
        );
        assert_eq!(
            writes[0].1,
            bytemuck::cast_slice::<_, u8>(&[0u32, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7])
        );
        assert_eq!(
            writes[2].1,
            bytemuck::cast_slice::<_, u8>(&[lines[0], lines_through[0]])
        );
        assert_eq!(writes[3].0, primitives.globals);
        assert_eq!(
            [&primitives.quads, &primitives.lines, &primitives.indices],
            [&buffers[0], &buffers[1], &buffers[2]],
            "first active frame uses setup buffers",
        );
        assert_eq!(storage(&primitives), original_storage);
        let group = primitives.group.as_ref().unwrap().group.clone();
        // Same view via a cloned handle, smaller frame, empty frame, then full frame again.
        // Empty frames neither upload nor discard the reusable resources.
        for batch in [
            batch,
            Batch {
                quads: &red,
                ..Default::default()
            },
            Batch::default(),
            batch,
        ] {
            let mut uploads = 0;
            let indices = primitives.indices.clone();
            let active = primitives.prepare(
                &device,
                &mut |buffer, bytes| {
                    assert_ne!(*buffer, indices, "indices must not be regenerated");
                    queue.write_buffer(buffer, 0, bytes);
                    uploads += 1;
                },
                &batch,
                view_proj,
                [SIZE, SIZE],
                &atlas_view.clone(),
            );
            assert_eq!(active, !batch.is_empty());
            let expected_uploads = if batch.is_empty() {
                0
            } else if batch.lines.is_empty() {
                2
            } else {
                3
            };
            assert_eq!(uploads, expected_uploads);
            assert_eq!(
                [&primitives.quads, &primitives.lines, &primitives.indices],
                [&buffers[0], &buffers[1], &buffers[2]],
            );
            assert_eq!(primitives.group.as_ref().unwrap().group, group);
            assert_eq!(storage(&primitives), original_storage);
            if batch.is_empty() {
                assert_eq!(primitives.counts, [0; 4]);
            }
        }
        // Growth appends valid indices beyond the previous capacity.
        let larger = [red, blue, red].concat();
        let mut index_upload = Vec::new();
        let old_indices = primitives.indices.clone();
        primitives.prepare(
            &device,
            &mut |buffer, bytes| {
                if buffer.usage().contains(wgpu::BufferUsages::INDEX) {
                    index_upload.extend_from_slice(bytemuck::cast_slice::<u8, u32>(bytes));
                }
            },
            &Batch {
                quads: &larger,
                ..Default::default()
            },
            view_proj,
            [SIZE, SIZE],
            &atlas_view,
        );
        assert_ne!(primitives.indices, old_indices);
        assert_eq!(
            index_upload,
            [
                0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7, 8, 9, 10, 8, 10, 11, 12, 13, 14, 12, 14, 15
            ]
        );
        assert_eq!(primitives.group.as_ref().unwrap().group, group);
    }

    /// A wall 5 blocks ahead fills the depth (reversed: nearer is greater). Behind it, a
    /// depth-tested quad and line stay hidden while the same seen through walls show; a
    /// depth-tested quad in front of the wall shows.
    #[test]
    fn depth_tested_primitives_hide_behind_walls_and_see_through_ones_do_not() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let view_proj = super::super::world_projection(70.0, 1.0, 512.0);
        let wall = view_proj.project_point3(Vec3::new(0.0, 0.0, -5.0)).z;
        // Reversed depth: 10 blocks away is less than the wall, 2 blocks away more.
        assert!(view_proj.project_point3(Vec3::new(0.0, 0.0, -10.0)).z < wall);
        assert!(view_proj.project_point3(Vec3::new(0.0, 0.0, -2.0)).z > wall);

        let format = wgpu::TextureFormat::Rgba8Unorm;
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
        // The atlas: one white texel, what plain colours sample.
        let atlas = texture(
            1,
            format,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        queue.write_texture(
            atlas.as_image_copy(),
            &[255; 4],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: None,
            },
            atlas.size(),
        );

        let (half, full) = (SIZE as f32 / 2.0, SIZE as f32);
        // Top left: behind the wall, depth tested. Top right: behind it, through walls.
        // Bottom left: in front of it, depth tested. Bottom right: lines behind it, the
        // depth-tested one along row 40, the one through walls along row 56.
        let quads = [
            quad(view_proj, [0.0, 0.0, half, half], 10.0, RED),
            quad(view_proj, [0.0, half, half, full], 2.0, BLUE),
        ]
        .concat();
        let quads_through = quad(view_proj, [half, 0.0, full, half], 10.0, GREEN);
        let lines = [
            line(view_proj, [half, full], 40.0, 10.0, MAGENTA_ARGB),
            line(view_proj, [half, full], 16.0, 2.0, MAGENTA_ARGB),
        ];
        let lines_through = [
            line(view_proj, [half, full], 56.0, 10.0, MAGENTA_ARGB),
            line(view_proj, [half, full], 24.0, 10.0, MAGENTA_ARGB),
        ];
        let batch = Batch {
            quads: &quads,
            quads_through: &quads_through,
            lines: &lines,
            lines_through: &lines_through,
            ..Default::default()
        };

        let mut primitives = WorldPrimitives::new(&device, (format, false));
        let atlas_view = atlas.create_view(&Default::default());
        assert!(primitives.prepare(
            &device,
            &mut |buffer, bytes| queue.write_buffer(buffer, 0, bytes),
            &batch,
            view_proj,
            [SIZE, SIZE],
            &atlas_view,
        ));
        let original_group = primitives.group.as_ref().unwrap().group.clone();
        // A resource reload replaces the atlas, even at the same dimensions. Blue is now
        // half intensity, making stale bindings visible in the pixel assertions below.
        let replacement = texture(
            1,
            format,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        queue.write_texture(
            replacement.as_image_copy(),
            &[255, 255, 128, 255],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: None,
            },
            replacement.size(),
        );
        let replacement_view = replacement.create_view(&Default::default());
        primitives.prepare(
            &device,
            &mut |buffer, bytes| queue.write_buffer(buffer, 0, bytes),
            &batch,
            view_proj,
            [SIZE, SIZE],
            &replacement_view,
        );
        let replacement_group = primitives.group.as_ref().unwrap().group.clone();
        assert_ne!(replacement_group, original_group);
        primitives.prepare(
            &device,
            &mut |buffer, bytes| queue.write_buffer(buffer, 0, bytes),
            &batch,
            view_proj,
            [SIZE, SIZE],
            &replacement_view.clone(),
        );
        assert_eq!(primitives.group.as_ref().unwrap().group, replacement_group);
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
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
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
            primitives.draw_through(&mut pass, &mut 0, STAGES - 1);
        }
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(SIZE * SIZE * 4),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            color.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(SIZE * 4),
                    rows_per_image: None,
                },
            },
            color.size(),
        );
        queue.submit([encoder.finish()]);
        readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let texels = readback.slice(..).get_mapped_range().unwrap().to_vec();
        let at = |x: u32, y: u32| {
            let i = ((y * SIZE + x) * 4) as usize;
            [texels[i], texels[i + 1], texels[i + 2], texels[i + 3]]
        };
        let black = [0, 0, 0, 255];
        assert_eq!(
            at(16, 16),
            black,
            "a depth-tested quad behind the wall is hidden"
        );
        assert_eq!(
            at(48, 16),
            [0, 255, 0, 255],
            "a through-wall quad covers a depth-tested line in front of the wall"
        );
        assert_eq!(
            at(16, 48),
            [0, 0, 128, 255],
            "a depth-tested quad before it shows, sampling the replacement atlas"
        );
        assert_eq!(
            at(48, 40),
            black,
            "a depth-tested line behind the wall is hidden"
        );
        assert_eq!(
            at(48, 56),
            [255, 0, 255, 255],
            "a line through walls shows behind it"
        );
        assert_eq!(
            at(48, 24),
            [255, 0, 255, 255],
            "a through-wall line covers a through-wall quad",
        );
        // The line is its width (4 pixels) thick, not more.
        assert_eq!(at(48, 52), black);
    }
}
