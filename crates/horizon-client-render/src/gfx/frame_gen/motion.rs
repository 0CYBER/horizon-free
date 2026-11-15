//! Motion vectors for what moves on its own, for frame generation and other passes: the
//! camera's motion vectors (from the depth) see mobs and dropped items as still. The game
//! tags each entity's vertices with its id (`Frame::entity_motion`); an entity whose
//! vertices have the same shape as last frame's (the same count, UVs and layers) gets each
//! vertex's own motion, limbs and spin included, drawn into a texture at the render size
//! (`motion.wgsl`) that the interpolation reads before the camera's.

use crate::mesher::Vertex;
use glam::{DVec3, Mat4, Vec3};
use std::collections::HashMap;

/// `motion.wgsl`'s `Params`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    jittered: [[f32; 4]; 4],
    current: [[f32; 4]; 4],
    previous: [[f32; 4]; 4],
}

/// A vertex this frame and where it was last frame, both relative to this frame's camera.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[cfg_attr(test, derive(Debug, PartialEq))]
struct MotionVertex {
    position: [f32; 3],
    previous: [f32; 3],
    uv: [f32; 2],
    /// `TRANSLUCENT` and `NEW`.
    flags: u32,
}

/// A translucent item, whose texture says where it shows.
const TRANSLUCENT: u32 = 1;
/// Nothing to match last frame: `motion.wgsl` writes `w = 1` and nothing else.
const NEW: u32 = 2;

pub(in crate::gfx) struct ObjectMotion {
    pipeline: wgpu::RenderPipeline,
    /// The entities' atlas, or a blank texel without one (`translucent_items` false).
    atlas: wgpu::TextureView,
    /// Translucent items are drawn, where their atlas alpha shows them; without an atlas
    /// they are left out.
    translucent_items: bool,
    /// Stage what has no last frame as `NEW` instead of leaving it out.
    new_runs: bool,
    sampler: wgpu::Sampler,
    params: wgpu::Buffer,
    vertices: wgpu::Buffer,
    /// Triangles of the entities matched this frame.
    staged: Vec<MotionVertex>,
    /// Last frame's vertices by entity, and its camera.
    tracked: HashMap<u64, Vec<Vertex>>,
    camera: DVec3,
    target: Option<([u32; 2], wgpu::TextureView)>,
}

pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

impl ObjectMotion {
    /// `atlas`: the entities' texture atlas; `None` leaves translucent items out.
    /// `new_runs`: also draw what has no last frame (new or reshaped tagged entities, and
    /// untagged geometry that writes depth) as `w = 1`; frame generation reads only `z`.
    pub(in crate::gfx) fn new(
        device: &wgpu::Device,
        atlas: Option<wgpu::TextureView>,
        new_runs: bool,
    ) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Frame generation motion"),
            source: wgpu::ShaderSource::Wgsl(include_str!("motion.wgsl").into()),
        });
        let attributes =
            wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Uint32];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Frame generation motion"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_motion"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<MotionVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                })],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_motion"),
                compilation_options: Default::default(),
                targets: &[Some(FORMAT.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let translucent_items = atlas.is_some();
        let atlas = atlas.unwrap_or_else(|| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("Frame generation motion blank atlas"),
                    size: wgpu::Extent3d::default(),
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        });
        Self {
            pipeline,
            atlas,
            translucent_items,
            new_runs,
            sampler: device.create_sampler(&Default::default()),
            params: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Frame generation motion"),
                size: std::mem::size_of::<Params>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            vertices: vertex_buffer(device, 1 << 12),
            staged: Vec::new(),
            tracked: HashMap::new(),
            camera: DVec3::ZERO,
            target: None,
        }
    }

    /// Forget last frame's entities: the next frame has no motion of its own.
    pub(in crate::gfx) fn reset(&mut self) {
        self.tracked.clear();
    }

    /// Match this frame's `entities` (relative to `camera`) to last frame's by the ids
    /// `spans` gives their vertices (start, id; 0 for none, each up to the next start), and
    /// keep them for the next frame.
    pub(in crate::gfx) fn track(
        &mut self,
        entities: &[Vertex],
        spans: &[(u32, u64)],
        camera: DVec3,
    ) {
        self.staged.clear();
        let shift = (self.camera - camera).as_vec3();
        let mut tracked = HashMap::with_capacity(self.tracked.len());
        for (i, &(start, id)) in spans.iter().enumerate() {
            let end = spans.get(i + 1).map_or(entities.len(), |s| s.0 as usize);
            let Some(now) = entities
                .get(start as usize..end)
                .filter(|v| (id != 0 || self.new_runs) && !v.is_empty())
            else {
                continue;
            };
            // Id 0 is never tracked: it has no last frame.
            let then = self.tracked.remove(&id).filter(|then| {
                then.len() == now.len()
                    && then
                        .iter()
                        .zip(now)
                        .all(|(a, b)| a.uv == b.uv && a.layer == b.layer)
            });
            if then.is_some() || self.new_runs {
                let new = if then.is_some() { 0 } else { NEW };
                let pair = |i: usize| MotionVertex {
                    position: now[i].pos,
                    previous: then
                        .as_ref()
                        .map_or(now[i].pos, |then| (Vec3::from(then[i].pos) + shift).into()),
                    uv: now[i].uv,
                    // The whole layer, unmasked, as frame generation has always tested it.
                    flags: new
                        | if translucent(now[i].layer) {
                            TRANSLUCENT
                        } else {
                            0
                        },
                };
                for q in (0..now.len() / 4).map(|q| q * 4) {
                    if !self.translucent_items && translucent(now[q].layer & 0xffff)
                        || new != 0 && !writes_depth(now[q].layer)
                    {
                        continue;
                    }
                    self.staged
                        .extend([q, q + 1, q + 2, q, q + 2, q + 3].map(pair));
                }
            }
            drop(then);
            if id != 0 {
                tracked.insert(id, now.to_vec());
            }
        }
        self.tracked = tracked;
        self.camera = camera;
    }

    /// Upload what `track` staged and the projections for `draw`, at `size` (the world's
    /// render size): `jittered` the world's (TAA's jitter in it), `current` the same without
    /// the jitter, `previous` last frame's from this frame's camera.
    pub(in crate::gfx) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: [u32; 2],
        jittered: Mat4,
        current: Mat4,
        previous: Mat4,
    ) {
        if self.target.as_ref().is_none_or(|t| t.0 != size) {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Frame generation motion"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            self.target = Some((size, texture.create_view(&Default::default())));
        }
        let bytes: &[u8] = bytemuck::cast_slice(&self.staged);
        if bytes.len() as u64 > self.vertices.size() {
            self.vertices = vertex_buffer(device, (bytes.len() as u64).next_power_of_two());
        }
        if !bytes.is_empty() {
            queue.write_buffer(&self.vertices, 0, bytes);
        }
        let params = Params {
            jittered: jittered.to_cols_array_2d(),
            current: current.to_cols_array_2d(),
            previous: previous.to_cols_array_2d(),
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
    }

    /// Draw the staged entities' motion vectors where `depth` (the world's, at `prepare`'s
    /// size) shows them; the rest of the texture is 0.
    pub(in crate::gfx) fn draw(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        depth: &wgpu::TextureView,
    ) -> wgpu::TextureView {
        let target = self.target.as_ref().expect("prepare() ran").1.clone();
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Frame generation motion"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.atlas),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let attachments = [Some(wgpu::RenderPassColorAttachment {
            view: &target,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })];
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Frame generation motion"),
            color_attachments: &attachments,
            ..Default::default()
        });
        if !self.staged.is_empty() {
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &group, &[]);
            let bytes = std::mem::size_of_val(self.staged.as_slice()) as u64;
            pass.set_vertex_buffer(0, self.vertices.slice(..bytes));
            pass.draw(0..self.staged.len() as u32, 0..1);
        }
        drop(pass);
        target
    }
}

fn translucent(layer: u32) -> bool {
    matches!(
        layer,
        super::super::entity_batch::TRANSLUCENT_ITEM_OVERLAY
            | super::super::entity_batch::TRANSLUCENT_GLINT_ITEM_OVERLAY
    )
}

/// Drawn without depth writes, so `depth` can't say where it shows: breaking cracks (11,
/// 12), shadows and energy swirls.
fn writes_depth(layer: u32) -> bool {
    use super::super::entity_batch::{ENERGY_SWIRL_OVERLAY, SHADOW_OVERLAY};
    !matches!(
        layer & 0xffff,
        11 | 12 | SHADOW_OVERLAY | ENERGY_SWIRL_OVERLAY
    )
}

fn vertex_buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Frame generation motion vertices"),
        size,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shader_validates() {
        let module = naga::front::wgsl::parse_str(include_str!("motion.wgsl")).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap();
    }

    fn quad(x: f32, uv: f32) -> Vec<Vertex> {
        (0..4)
            .map(|i| Vertex {
                pos: [x + i as f32, 0.0, -5.0],
                uv: [uv, 0.0],
                ..bytemuck::Zeroable::zeroed()
            })
            .collect()
    }

    /// An entity seen last frame gets its vertices' own motion, in this frame's camera
    /// space; a new one, one whose shape changed and untagged geometry get none.
    #[test]
    fn tracks_each_entity_by_id() {
        let Some((device, queue)) = super::super::optical_flow::tests::device() else {
            return;
        };
        let atlas = super::super::optical_flow::tests::upload(&device, &queue, [1, 1], &[0; 4]);
        let mut motion = ObjectMotion::new(&device, Some(atlas), false);
        let mut entities = quad(0.0, 0.0);
        entities.extend(quad(10.0, 0.5));
        motion.track(&entities, &[(0, 7), (4, 8)], DVec3::ZERO);
        assert!(motion.staged.is_empty());
        // Entity 7 moved 1 block right, the camera 2 blocks right; entity 8 became
        // another item; 9 is new; then untagged geometry.
        let mut entities = quad(-1.0, 0.0);
        entities.extend(quad(10.0, 0.25));
        entities.extend(quad(20.0, 0.0));
        entities.extend(quad(30.0, 0.0));
        motion.track(
            &entities,
            &[(0, 7), (4, 8), (8, 9), (12, 0)],
            DVec3::new(2.0, 0.0, 0.0),
        );
        assert_eq!(motion.staged.len(), 6);
        let v = motion.staged[0];
        assert_eq!(v.position, [-1.0, 0.0, -5.0]);
        assert_eq!(v.previous, [-2.0, 0.0, -5.0]);
        assert_eq!(motion.tracked.len(), 3);
    }

    use super::super::super::entity_batch::{SHADOW_OVERLAY, TRANSLUCENT_ITEM_OVERLAY};

    const CORNERS: [[f32; 2]; 4] = [[0.0, 0.0], [2.0, 0.0], [2.0, 4.0], [0.0, 4.0]];

    /// A 2 by 4 block quad at `x`, 5 blocks in front of the camera.
    fn square(x: f32, layer: u32) -> Vec<Vertex> {
        CORNERS
            .map(|[dx, y]| Vertex {
                pos: [x + dx, y, -5.0],
                layer,
                ..bytemuck::Zeroable::zeroed()
            })
            .to_vec()
    }

    /// Without `new_runs`, what is staged is what frame generation always staged: each
    /// matched quad as two triangles, translucent items flagged, nothing else.
    #[test]
    fn without_new_runs_only_matched_entities_are_staged() {
        let Some((device, queue)) = super::super::optical_flow::tests::device() else {
            return;
        };
        let atlas = super::super::optical_flow::tests::upload(&device, &queue, [1, 1], &[0; 4]);
        let mut motion = ObjectMotion::new(&device, Some(atlas), false);
        let mut entities = square(0.5, 0);
        entities.extend(square(2.0, TRANSLUCENT_ITEM_OVERLAY));
        motion.track(&entities, &[(0, 7), (4, 8)], DVec3::ZERO);
        let mut entities = square(0.0, 0);
        entities.extend(square(2.0, TRANSLUCENT_ITEM_OVERLAY));
        entities.extend(square(4.0, 0));
        entities.extend(square(6.0, 0));
        motion.track(&entities, &[(0, 7), (4, 8), (8, 9), (12, 0)], DVec3::ZERO);
        let corner = |x: f32, i: usize| [x + CORNERS[i][0], CORNERS[i][1], -5.0];
        let quad = |now: f32, then: f32, flags: u32| {
            [0, 1, 2, 0, 2, 3].map(|i| MotionVertex {
                position: corner(now, i),
                previous: corner(then, i),
                uv: [0.0; 2],
                flags,
            })
        };
        let mut expected = quad(0.0, 0.5, 0).to_vec();
        expected.extend(quad(2.0, 2.0, TRANSLUCENT));
        assert_eq!(motion.staged, expected);
    }

    /// Without the atlas, nothing can say where a translucent item shows: it is left out.
    #[test]
    fn translucent_items_need_the_atlas() {
        let Some((device, queue)) = super::super::optical_flow::tests::device() else {
            return;
        };
        for with_atlas in [false, true] {
            let atlas = with_atlas.then(|| {
                super::super::optical_flow::tests::upload(&device, &queue, [1, 1], &[0; 4])
            });
            let mut motion = ObjectMotion::new(&device, atlas, false);
            for layer in [TRANSLUCENT_ITEM_OVERLAY, TRANSLUCENT_ITEM_OVERLAY | 3 << 16] {
                let item = square(0.0, layer);
                motion.track(&item, &[(0, 7)], DVec3::ZERO);
                motion.track(&item, &[(0, 7)], DVec3::ZERO);
                assert_eq!(motion.staged.len(), if with_atlas { 6 } else { 0 });
            }
        }
    }

    /// A depth texture at `size` holding `value` everywhere.
    fn depth(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: [u32; 2],
        value: f32,
    ) -> wgpu::TextureView {
        let view = device
            .create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(value),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        queue.submit([encoder.finish()]);
        view
    }

    /// With `new_runs`, a new entity and untagged geometry that writes depth read
    /// `(0, 0, 0, 1)`; a matched one keeps its vector and `z = 1`; an untagged shadow (no
    /// depth writes) stays 0. Without it, only the matched one is drawn.
    #[test]
    fn new_runs_write_w_only_when_asked() {
        let Some((device, queue)) = super::super::optical_flow::tests::device() else {
            return;
        };
        let size = [8, 4];
        let projection = Mat4::orthographic_rh(0.0, 8.0, 0.0, 4.0, 1.0, 10.0);
        let world = projection.project_point3(Vec3::new(0.0, 0.0, -5.0)).z;
        let depth = depth(&device, &queue, size, world);
        for new_runs in [false, true] {
            let mut motion = ObjectMotion::new(&device, None, new_runs);
            motion.track(&square(0.5, 0), &[(0, 7)], DVec3::ZERO);
            // 7 moved half a block (half a pixel) left; 9 is new; then untagged geometry
            // and an untagged shadow.
            let mut entities = square(0.0, 0);
            entities.extend(square(2.0, 0));
            entities.extend(square(4.0, 0));
            entities.extend(square(6.0, SHADOW_OVERLAY));
            motion.track(&entities, &[(0, 7), (4, 9), (8, 0), (12, 0)], DVec3::ZERO);
            motion.prepare(&device, &queue, size, projection, projection, projection);
            let mut encoder = device.create_command_encoder(&Default::default());
            let view = motion.draw(&device, &mut encoder, &depth);
            queue.submit([encoder.finish()]);
            let texels: Vec<[f32; 4]> =
                super::super::optical_flow::tests::read(&device, &queue, view.texture(), 8)
                    .chunks(8)
                    .map(|t| {
                        std::array::from_fn(|c| {
                            half::f16::from_le_bytes([t[2 * c], t[2 * c + 1]]).to_f32()
                        })
                    })
                    .collect();
            let at = |x: usize| texels[2 * size[0] as usize + x];
            let [x, y, z, w] = at(1);
            assert!((x - 0.5 / 8.0).abs() < 1e-4 && y.abs() < 1e-4, "{x} {y}");
            assert_eq!([z, w], [1.0, 0.0], "a matched entity");
            let new = [0.0, 0.0, 0.0, if new_runs { 1.0 } else { 0.0 }];
            assert_eq!(at(3), new, "a new entity");
            assert_eq!(at(5), new, "untagged geometry");
            assert_eq!(at(7), [0.0; 4], "an untagged shadow");
        }
    }
}
