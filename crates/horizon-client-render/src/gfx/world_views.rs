//! Native mods' world views (`native_render::world_views`): the frame's world drawn again
//! from a camera of the mod's own into a target of its own, as the water mirror is: its own
//! section list (`view_sections`), the frame's entity geometry (built for every view's frustum
//! too), the frame's sky and fog, through the frame's pipelines with the view's globals. A
//! view's clip plane is its projection's near plane; a mirrored view's projection flips x, and
//! it draws through pipelines that cull the front faces instead (`Flipped`).

use super::{EntityPipelineOptions, Frame, Globals, Renderer, SolidPipeline, native_render};
use glam::{DVec3, Mat4, Vec3, Vec4};
use std::collections::HashMap;

/// A view's camera in the frame's camera-relative world: the frame's geometry is placed relative
/// to the player's camera, so the view's matrices take the offset to its own camera in. A
/// mirror flips the eye's x (the picture left to right, every winding round); a clip plane
/// becomes the near plane (`near_plane_on`).
pub(super) struct ViewCamera {
    /// The frame's camera-relative world to the view's clip space.
    pub view_proj: Mat4,
    /// The view's own camera-relative world to its clip space (`WorldViewDrawn::view_proj`).
    pub own_view_proj: Mat4,
    /// The view's eye, relative to the frame's camera.
    pub offset: Vec3,
    /// Its globals' `world_view`: x -1 mirrored, else 1; y the clip-space depth entities come
    /// forward (`ENTITY_MARGIN`); z 1 with a clip plane, which the shaders take from there
    /// rather than from y (that can round to 0 for a camera very near the plane).
    pub world_view: [f32; 4],
    view: Mat4,
    aspect: f32,
}

/// How far forward a clipped view's entities come in depth, as a share of their distance. A
/// clip plane leaves depth changing little with distance (`near_plane_on`), so the entities'
/// constant bias (gfx.rs `entity_bias`, 128 steps of the depth's float) would bring one blocks
/// forward through the wall it stands behind; a clipped view draws them without it and brings
/// each pixel forward by this share after clipping (`fs_entity_clipped`), so the plane cuts
/// entities where it cuts terrain. The tolerance is this share of the distance: an entity wins
/// against terrain up to 1/1024 of its distance in front of it (about 1 block at 1024 blocks,
/// 1/16 at 64), and one farther behind stays hidden. A surface on terrain's plane (a boat's
/// floor on a trapdoor) wins wherever that share is more than the depth's rounding (roughly,
/// while the camera is more than 1/300 of the surface's distance behind the plane).
const ENTITY_MARGIN: f32 = 1.0 / 1024.0;

impl ViewCamera {
    pub fn new(view: &native_render::WorldView, camera: DVec3, far: f32) -> Self {
        let aspect = view.size[0] as f32 / view.size[1] as f32;
        let mut rotation = crate::camera::view_matrix_rolled(view.yaw, view.pitch, view.roll);
        if view.mirror {
            rotation = Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0)) * rotation;
        }
        let projection = super::world_projection(view.fov_degrees, aspect, far);
        let offset = (view.camera - camera).as_vec3();
        let mut own_view_proj = projection * rotation;
        let mut world_view = [if view.mirror { -1.0 } else { 1.0 }, 0.0, 0.0, 0.0];
        if let Some(plane) = view.camera_clip_plane() {
            let k = corner_cosine((view.fov_degrees.to_radians() / 2.0).tan(), aspect);
            own_view_proj = near_plane_on(own_view_proj, plane, k);
            // Depth at `w` is a constant plus `k * -plane.w / w`: a shift of that term's share
            // moves a point the same share of its distance.
            world_view[1] = ENTITY_MARGIN * k * -plane.w;
            world_view[2] = 1.0;
        }
        Self {
            view_proj: own_view_proj * Mat4::from_translation(-offset),
            own_view_proj,
            offset,
            world_view,
            view: rotation * Mat4::from_translation(-offset),
            aspect,
        }
    }
}

/// The cosine of the angle out to the corners of a frustum with the vertical half-angle's
/// tangent `tan_half` and `aspect`.
fn corner_cosine(tan_half: f32, aspect: f32) -> f32 {
    1.0 / (1.0 + tan_half * tan_half * (1.0 + aspect * aspect)).sqrt()
}

/// `view_proj`, a reversed-Z perspective from the origin, with its near plane turned onto
/// `plane` (a unit normal, the origin behind it): the clip z row becomes `w - k * plane`, so
/// `z <= w` keeps what is in front of the plane and depth still falls with distance. With `k`
/// the frustum's `corner_cosine`, nothing in the frustum is farther in front of the plane than
/// its `w` over `k`, so `z >= 0` leaves none of it out (the far plane goes).
fn near_plane_on(view_proj: Mat4, plane: Vec4, k: f32) -> Mat4 {
    let rows = view_proj.transpose();
    Mat4::from_cols(
        rows.x_axis,
        rows.y_axis,
        rows.w_axis - k * plane,
        rows.w_axis,
    )
    .transpose()
}

/// The frame's solid and translucent terrain pipelines as a mirrored view draws them: culling
/// the front faces, since its flipped picture turns every winding round, so the faces it drops
/// are the ones the view unmirrored drops (gfx.rs builds the per-section ones, `flat::Flat` the
/// flat ones).
pub(super) struct Flipped {
    pub solid: SolidPipeline,
    pub translucent: wgpu::RenderPipeline,
}

/// A view's targets, kept while it is asked for each frame.
pub(super) struct Target {
    size: [u32; 2],
    format: wgpu::TextureFormat,
    color: wgpu::Texture,
    color_view: wgpu::TextureView,
    depth: wgpu::TextureView,
}

/// The views' targets by id, the pipelines mirrored views draw with, the entities' one clipped
/// views draw with (`ENTITY_MARGIN`), and this frame's views and their sections.
pub(super) struct Kept {
    targets: HashMap<u64, Target>,
    flipped: Flipped,
    clipped_entities: wgpu::RenderPipeline,
    pub sections: super::view_sections::Sections,
}

impl Kept {
    /// `entity` is gfx.rs's `entity_pipeline_with`.
    pub fn new(
        solid: SolidPipeline,
        translucent: wgpu::RenderPipeline,
        entity: impl Fn(EntityPipelineOptions<'_>) -> wgpu::RenderPipeline,
    ) -> Self {
        // gfx.rs's `entity_pipeline` without its constant bias, through `fs_entity_clipped`
        let clipped_entities = entity(EntityPipelineOptions {
            label: "Entities (clipped world view)",
            cull_mode: None,
            depth_write: true,
            compare: wgpu::CompareFunction::GreaterEqual,
            bias: Default::default(),
            blend: wgpu::BlendState::ALPHA_BLENDING,
            clouds: false,
            vertex: "vs_entity",
            fragment: "fs_entity_clipped",
            write_mask: wgpu::ColorWrites::ALL,
        });
        Self {
            targets: HashMap::new(),
            flipped: Flipped { solid, translucent },
            clipped_entities,
            sections: Default::default(),
        }
    }
}

/// What of the frame the views draw.
struct Lists {
    pub globals: Globals,
    pub clear: wgpu::Color,
    pub sky: bool,
    pub flat: bool,
    pub hdr: bool,
    pub entity_quads: u32,
}

impl Renderer {
    /// Draws each of this frame's views (`view_sections::Sections`) and hands it to
    /// `native_world_view_drawn`. The views' globals go in the frame's own buffer, its bind
    /// groups with it, and the frame's are put back after them, all in `encoder`'s order.
    /// `globals` are the frame's, `flat` whether it drew flat, `entity_quads` its entity quads.
    pub(super) fn draw_world_views(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        frame: &Frame,
        globals: Globals,
        flat: bool,
        entity_quads: u32,
    ) {
        let views = self.world_views.sections.take_views();
        let [r, g, b, _] = globals.fog_color.map(f64::from);
        let lists = &Lists {
            globals,
            // as the world pass clears, but for a shader pack's own sky
            clear: if frame.end_sky && !frame.underwater && super::sky_open(frame) {
                wgpu::Color::BLACK
            } else {
                wgpu::Color { r, g, b, a: 1.0 }
            },
            sky: super::sky_visible(frame),
            flat,
            hdr: self.hdr.is_some(),
            entity_quads,
        };
        let format = self.output.frame.format;
        let mut targets = std::mem::take(&mut self.world_views.targets);
        targets.retain(|id, _| views.views().iter().any(|v| v.id == *id));
        // children first (`WorldViews::order`): the encoder's order makes each one's picture
        // this frame's in its parent's `world_view_drawn`
        for (k, view) in views.views().iter().enumerate() {
            let target = targets
                .entry(view.id)
                .and_modify(|t| {
                    if t.size != view.size || t.format != format {
                        *t = Target::new(&self.device, view.size, format);
                    }
                })
                .or_insert_with(|| Target::new(&self.device, view.size, format));
            let camera = ViewCamera::new(view, frame.camera, frame.depth_far);
            let globals = Globals {
                view_proj: camera.view_proj.to_cols_array_2d(),
                inverse_view: camera.view.inverse().to_cols_array_2d(),
                misc: [
                    lists.globals.misc[0],
                    lists.globals.misc[1],
                    camera.aspect,
                    (view.fov_degrees.to_radians() / 2.0).tan(),
                ],
                outline_params: super::outline_params(view.size[0], view.size[1]),
                view_origin: [camera.offset.x, camera.offset.y, camera.offset.z, 0.0],
                world_view: camera.world_view,
                ..lists.globals
            };
            self.uploads
                .buffer(encoder, &self.globals, 0, bytemuck::bytes_of(&globals));
            self.draw_world_view(encoder, target, frame, lists, k, view);
            native_render::world_view_drawn(&mut native_render::WorldViewDrawn {
                id: view.id,
                parent: view.parent,
                nesting_depth: views.depth(k),
                device: &self.device,
                queue: &self.queue,
                encoder,
                color: &target.color,
                color_view: &target.color_view,
                color_format: format,
                size: view.size,
                depth: &target.depth,
                view_proj: camera.own_view_proj,
                camera: view.camera,
            });
        }
        if !views.views().is_empty() {
            self.uploads.buffer(
                encoder,
                &self.globals,
                0,
                bytemuck::bytes_of(&lists.globals),
            );
        }
        self.world_views.targets = targets;
        self.world_views.sections.put_back(views);
    }

    /// The sky, the solid terrain, the entities and the translucent terrain, in the world
    /// pass's order, without what follows one camera alone (the sunrise's mesh, the sun and
    /// moon, stars, clouds, particles, weather, the block outline, the hand), of view `k`'s own
    /// section draws (`view_sections::Sections::draws`, flat: `Flat::view_calls`). A mirrored
    /// view's terrain draws through the `Flipped` pipelines (the sky, entities and block entities
    /// cull no faces); a clipped view's entities without the constant bias (`ENTITY_MARGIN`).
    fn draw_world_view(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &Target,
        frame: &Frame,
        lists: &Lists,
        k: usize,
        view: &native_render::WorldView,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Native world view"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.color_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(lists.clear),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &target.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_bind_group(0, &self.group, &[]);
        if lists.sky {
            pass.set_pipeline(&self.sky);
            pass.draw(0..3, 0..1);
        }
        let flat = self.flat.as_ref().filter(|_| lists.flat);
        let flipped = view
            .mirror
            .then(|| flat.map_or(&self.world_views.flipped, |f| &f.flipped));
        let vertices = || super::flat_vertices(&self.arena, self.translucent_arena.as_ref());
        let [solid, translucent, end] = self.world_views.sections.draws[k];
        match flat {
            Some(flat) => {
                pass.set_pipeline(
                    flipped.map_or(flat.solid.get(lists.hdr), |f| f.solid.get(lists.hdr)),
                );
                flat.draw(&mut pass, flat.view_calls(k)[0], vertices());
                pass.set_bind_group(0, &self.group, &[]);
            }
            None => {
                pass.set_pipeline(
                    flipped.map_or(self.solid.get(lists.hdr), |f| f.solid.get(lists.hdr)),
                );
                pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.set_vertex_buffer(1, self.instances.slice(..));
                self.draw_solid_sections(&mut pass, &self.indirect, 0, solid, translucent - solid);
            }
        }
        // The game's block entity sections, which take in what the views see.
        let static_entities = frame
            .visible_static_block_entity_sections
            .iter()
            .filter_map(|section| self.static_entity_sections.get(section));
        if !frame.visible_static_block_entity_sections.is_empty() {
            pass.set_pipeline(&self.static_entity_pipeline);
            pass.set_bind_group(0, &self.entity_group, &[]);
            pass.set_bind_group(1, &self.static_entity_group, &[]);
            pass.set_index_buffer(
                self.static_entity_indices.slice(..),
                wgpu::IndexFormat::Uint32,
            );
            for gpu in static_entities {
                let quads = gpu.vertices / 4;
                if quads > 0 {
                    pass.set_vertex_buffer(0, gpu.buffer.slice(..));
                    pass.set_vertex_buffer(1, gpu.origin_buffer.slice(..));
                    pass.draw_indexed(0..quads * 6, 0, 0..1);
                }
            }
        }
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        if lists.entity_quads > 0 {
            pass.set_pipeline(match view.clip_plane {
                Some(_) => &self.world_views.clipped_entities,
                None => &self.entity_pipeline,
            });
            pass.set_bind_group(0, &self.entity_group, &[]);
            pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
            pass.draw_indexed(0..lists.entity_quads * 6, 0, 0..1);
        }
        pass.set_bind_group(0, &self.group, &[]);
        match flat {
            Some(flat) => {
                pass.set_pipeline(flipped.map_or(&flat.translucent, |f| &f.translucent));
                flat.draw(&mut pass, flat.view_calls(k)[1], vertices());
            }
            None => {
                pass.set_pipeline(flipped.map_or(&self.translucent, |f| &f.translucent));
                pass.set_vertex_buffer(1, self.instances.slice(..));
                self.draw_sections(&mut pass, &self.indirect, 0, translucent, end - translucent);
            }
        }
    }
}

impl Target {
    fn new(device: &wgpu::Device, [width, height]: [u32; 2], format: wgpu::TextureFormat) -> Self {
        let texture = |label, format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width,
                    height,
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
        let sampled = wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC;
        let color = texture("Native world view", format, sampled);
        let depth = texture(
            "Native world view depth",
            wgpu::TextureFormat::Depth32Float,
            sampled,
        );
        Self {
            size: [width, height],
            format,
            color_view: color.create_view(&Default::default()),
            color,
            depth: depth.create_view(&Default::default()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A view at the frame's camera sees as a screen view would; one elsewhere sees a point
    /// where its own camera-relative matrix does.
    #[test]
    fn a_view_sees_the_world_from_its_own_camera() {
        let frame_camera = DVec3::new(100.5, 70.0, -20.25);
        let at = |camera, yaw, pitch| {
            native_render::WorldView::new(1, camera, yaw, pitch, 70.0, [320, 180])
        };
        let here = ViewCamera::new(&at(frame_camera, 30.0, 10.0), frame_camera, 512.0);
        let screen = super::super::View::screen(30.0, 10.0, 70.0, 320.0 / 180.0, 512.0);
        assert!(
            here.view_proj
                .abs_diff_eq(screen.view_proj(Mat4::IDENTITY), 1e-6)
        );
        // a camera 40 blocks east looking west (yaw 90) sees the frame camera's point dead ahead
        let east = ViewCamera::new(
            &at(frame_camera + DVec3::X * 40.0, 90.0, 0.0),
            frame_camera,
            512.0,
        );
        let clip = east.view_proj * glam::Vec4::new(0.0, 0.0, 0.0, 1.0);
        assert!(
            clip.x.abs() / clip.w < 1e-5 && clip.y.abs() / clip.w < 1e-5,
            "{clip}"
        );
        assert!(clip.w > 39.9 && clip.w < 40.1, "40 blocks in front: {clip}");
        // and the same point relative to its own camera through `own_view_proj`
        let own = east.own_view_proj * glam::Vec4::new(-40.0, 0.0, 0.0, 1.0);
        assert!((own - clip).abs().max_element() < 1e-3);
        // a roll turns the picture about the view direction: 90 counterclockwise puts up on the left
        let rolled = ViewCamera::new(
            &at(frame_camera, 0.0, 0.0).with_roll(90.0),
            frame_camera,
            512.0,
        );
        let up = rolled.view_proj * glam::Vec4::new(0.0, 1.0, 10.0, 1.0);
        assert!(up.x < -0.05 * up.w && (up.y / up.w).abs() < 1e-5, "{up}");
    }

    /// A mirrored view sees a point where the unmirrored one does, with x the other way, and
    /// its matrix's determinant changes sign: every winding turns round (what `Flipped` makes
    /// up for).
    #[test]
    fn a_mirrored_view_flips_the_picture_left_to_right() {
        let camera = DVec3::new(100.5, 70.0, -20.25);
        let view = native_render::WorldView::new(1, camera, 0.0, 0.0, 70.0, [320, 180]);
        let plain = ViewCamera::new(&view, camera, 512.0);
        let mirrored = ViewCamera::new(&view.with_mirror(), camera, 512.0);
        // looking south (+z), a point a block east (+x) is left of the middle; mirrored, right
        let east = Vec4::new(1.0, 0.0, 10.0, 1.0);
        let x = |m: Mat4| (m * east).x / (m * east).w;
        assert!((x(plain.view_proj) + 0.08033).abs() < 1e-4);
        assert!((x(mirrored.view_proj) - 0.08033).abs() < 1e-4);
        assert!(plain.view_proj.determinant() * mirrored.view_proj.determinant() < 0.0);
    }

    /// A clip plane is the view's near plane: depth 1 on it, past 1 (left out) behind it,
    /// falling with distance in front of it, and above 0 as far as the frustum reaches.
    #[test]
    fn a_clip_plane_is_the_near_plane() {
        // a camera at z 40 looking north (-z), and the plane z = 20 facing north
        let frame_camera = DVec3::new(8.0, 104.0, 0.5);
        let view = native_render::WorldView::new(
            1,
            DVec3::new(8.0, 104.0, 40.0),
            180.0,
            0.0,
            70.0,
            [200, 120],
        )
        .with_clip_plane([0.0, 0.0, -2.0, 40.0]);
        let camera = ViewCamera::new(&view, frame_camera, 1000.0);
        let depth = |z: f64| {
            let p = (DVec3::new(8.0, 104.0, z) - frame_camera).as_vec3();
            let clip = camera.view_proj * p.extend(1.0);
            clip.z / clip.w
        };
        assert!((depth(20.0) - 1.0).abs() < 1e-5, "{}", depth(20.0));
        assert!((depth(28.0) - 1.39475).abs() < 1e-4, "{}", depth(28.0));
        assert!((depth(12.0) - 0.83082).abs() < 1e-4, "{}", depth(12.0));
        assert!((depth(-300.0) - 0.44271).abs() < 1e-4, "{}", depth(-300.0));
        // A plane facing the frustum's top right corner, a block out along it: 1000 blocks out
        // along that corner is still in front of the far end, at depth 1 / 1000.
        let tan_half = 35f32.to_radians().tan();
        let corner = Vec3::new(2.0 * tan_half, tan_half, -1.0).normalize();
        let lens = super::super::world_projection(70.0, 2.0, 512.0);
        let clipped = near_plane_on(lens, corner.extend(-1.0), corner_cosine(tan_half, 2.0));
        let far = clipped * (corner * 1000.0).extend(1.0);
        assert!((far.z / far.w - 0.001).abs() < 1e-5, "{far}");
        // A camera 1e-43 behind the plane: the entities' shift rounds to 0, the clip flag stays.
        let near = native_render::WorldView::new(1, DVec3::ZERO, 0.0, 0.0, 70.0, [64, 64])
            .with_clip_plane([0.0, 0.0, 1.0, -1e-43]);
        let camera = ViewCamera::new(&near, DVec3::ZERO, 1000.0);
        assert_eq!(camera.world_view, [1.0, 0.0, 1.0, 0.0]);
    }
}
