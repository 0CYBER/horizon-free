//! The scene in the post passes and the material stage of a shader pack with `[ray_tracing]`
//! (docs/shaders.md, spec #15624): where the device traces rays, each pass, and the terrain's
//! pipelines of a material stage that calls `trace` or `trace_any`, are built a second time
//! from what the check made against the scene (`CheckedStage::scene`, `scene_source`), and their
//! group 3 also holds the TLAS, the store of the sections' vertices (gfx/ray_scene.rs), the
//! block sprites and the pack's material id and tags of each sprite layer, which the prelude's
//! `trace` reads. Those draw in frames that have a scene; in the others, and on a device that
//! can't hold the bindings, the ones whose `trace` misses draw, as on a device without ray
//! queries.

use horizon_shader_api::stages::{
    GROUP, PBR_NORMALS_BINDING, PBR_SPECULAR_BINDING, RAY_LAYERS_BINDING, RAY_SCENE_BINDING,
    RAY_SPACE_BINDING, RAY_SPRITE_ARRAYS, RAY_SPRITES_BINDING, RAY_STORE_BINDING,
};

use super::super::native_material::{MaterialPipelines, TerrainShape, terrain_entries};
use super::{INTERMEDIATE, NativeFrame, NativePost, post_pipeline};
pub(in crate::gfx) use crate::gfx::surface_setup::ray_query::ray_scene::{RayScene, Traced};

/// What the scene's passes are built against besides the scene: the terrain's sprites and the
/// pack's tables of them.
pub(in crate::gfx) struct TraceShape<'a> {
    /// terrain.wgsl's sprite arrays, the unused ones a stand-in.
    pub sprites: &'a [wgpu::TextureView],
    /// Layers per sprite array (terrain.wgsl's `SPRITE_SPLIT`).
    pub split: u32,
    /// The pack's tags and material id of each sprite layer that has any, as `(layer, value)`:
    /// the lists the material stage's `input.tags` and `input.material` come from.
    pub tags: &'a [(u32, u32)],
    pub materials: &'a [(u32, u32)],
}

impl<'a> TraceShape<'a> {
    pub(in crate::gfx) fn of(terrain: &TerrainShape<'a>) -> Self {
        Self {
            sprites: terrain.sprite_views,
            split: terrain.sprites[1] as u32,
            tags: terrain.sprite_tags,
            materials: terrain.sprite_materials,
        }
    }
}

/// What a stage built against the scene reads besides the TLAS and the store.
pub(in crate::gfx) struct Rays {
    /// The prelude's `horizon_ray_space`: the camera in the scene's space, then `split`.
    space: wgpu::Buffer,
    /// The prelude's `horizon_ray_layers`.
    layers: wgpu::Buffer,
    sprites: Vec<wgpu::TextureView>,
    split: u32,
}

/// The post passes built against the scene, and what they read besides it.
pub(super) struct NativeTrace {
    /// The post group's layout with the scene's bindings.
    pub(super) layout: wgpu::BindGroupLayout,
    /// One pipeline a pass, in the passes' order.
    pub(super) pipelines: Vec<wgpu::RenderPipeline>,
    rays: Rays,
    /// The resource packs' LabPBR maps (`NativeFrame::pbr`), for `hit_normal_map` and its kin.
    pbr: [wgpu::TextureView; 2],
}

/// A material stage's terrain pipelines built against the scene, and its frame's group with
/// the scene in it.
pub(in crate::gfx) struct TerrainTrace {
    layout: wgpu::BindGroupLayout,
    pipelines: MaterialPipelines,
    rays: Rays,
    /// The group, made for the frame's group `.0` and the scene of generation `.1` (the TLAS and
    /// the store are replaced as they grow, and the frame's group as its targets change).
    group: Option<(wgpu::BindGroup, u64, wgpu::BindGroup)>,
    /// Whether this frame traces the scene.
    on: bool,
}

/// `horizon_ray_layers`: two words a sprite layer, its material id then its tags, up to the
/// last layer that has either (a layer past the end has neither).
fn layer_words(tags: &[(u32, u32)], materials: &[(u32, u32)]) -> Vec<u32> {
    let layers = tags
        .iter()
        .chain(materials)
        .map(|&(layer, _)| layer as usize + 1)
        .max()
        .unwrap_or(0);
    // Never empty: a buffer needs a size.
    let mut words = vec![0; layers.max(1) * 2];
    for &(layer, id) in materials {
        words[layer as usize * 2] = id;
    }
    for &(layer, bits) in tags {
        words[layer as usize * 2 + 1] = bits;
    }
    words
}

/// The scene's bindings in group 3, for the fragment stage.
fn scene_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty,
        count: None,
    };
    let buffer = |ty| wgpu::BindingType::Buffer {
        ty,
        has_dynamic_offset: false,
        min_binding_size: None,
    };
    let read = wgpu::BufferBindingType::Storage { read_only: true };
    let mut entries = vec![
        entry(
            RAY_SCENE_BINDING,
            wgpu::BindingType::AccelerationStructure {
                vertex_return: false,
            },
        ),
        entry(RAY_STORE_BINDING, buffer(read)),
        entry(RAY_SPACE_BINDING, buffer(wgpu::BufferBindingType::Uniform)),
        entry(RAY_LAYERS_BINDING, buffer(read)),
    ];
    entries.extend((0..RAY_SPRITE_ARRAYS).map(|i| {
        entry(
            RAY_SPRITES_BINDING + i,
            wgpu::BindingType::Texture {
                // Read with `textureLoad`.
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2Array,
                multisampled: false,
            },
        )
    }));
    entries
}

/// The LabPBR maps of a post pass built against the scene, at the bindings the material stage
/// has them (`NativeFrame`'s 12 and 13): the post group has none of its own there.
fn post_pbr_entries() -> [wgpu::BindGroupLayoutEntry; 2] {
    [PBR_NORMALS_BINDING, PBR_SPECULAR_BINDING].map(|binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            // Sampled with the post stage's `nearest_sampler`.
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2Array,
            multisampled: false,
        },
        count: None,
    })
}

/// What `limits` lacks for a fragment stage that reads the group `group` and the other groups'
/// entries `others`, if anything.
fn over_limits(
    group: &[wgpu::BindGroupLayoutEntry],
    others: &[wgpu::BindGroupLayoutEntry],
    limits: &wgpu::Limits,
) -> Option<String> {
    let fragment = || {
        (group.iter().chain(others)).filter(|e| e.visibility.contains(wgpu::ShaderStages::FRAGMENT))
    };
    let count = |wanted: fn(&wgpu::BindingType) -> bool| {
        fragment().filter(|e| wanted(&e.ty)).count() as u32
    };
    use wgpu::{BindingType::Buffer, BufferBindingType as Kind};
    let needs = [
        (
            "sampled textures",
            count(|t| matches!(t, wgpu::BindingType::Texture { .. })),
            limits.max_sampled_textures_per_shader_stage,
        ),
        (
            "storage buffers",
            count(|t| {
                matches!(
                    t,
                    Buffer {
                        ty: Kind::Storage { .. },
                        ..
                    }
                )
            }),
            limits.max_storage_buffers_per_shader_stage,
        ),
        (
            "uniform buffers",
            count(|t| {
                matches!(
                    t,
                    Buffer {
                        ty: Kind::Uniform,
                        ..
                    }
                )
            }),
            limits.max_uniform_buffers_per_shader_stage,
        ),
        (
            "acceleration structures",
            1,
            limits.max_acceleration_structures_per_shader_stage,
        ),
        (
            "bindings in a group",
            group.iter().map(|e| e.binding + 1).max().unwrap_or(0),
            limits.max_bindings_per_bind_group,
        ),
    ];
    needs
        .into_iter()
        .find(|&(_, need, limit)| need > limit)
        .map(|(what, need, limit)| format!("{need} {what}, the device takes {limit}"))
}

/// Builds with `build` what draws against the scene, in a group 3 of `entries` and the
/// scene's, where the device traces rays and its fragment stage holds them beside its other
/// groups' entries `others`. Else None, and the log says why: what doesn't trace keeps missing
/// every ray.
fn with_scene<T>(
    device: &wgpu::Device,
    (entries, others): (&[wgpu::BindGroupLayoutEntry], &[wgpu::BindGroupLayoutEntry]),
    shape: &TraceShape<'_>,
    build: impl FnOnce(&wgpu::BindGroupLayout) -> Result<T, String>,
) -> Option<(wgpu::BindGroupLayout, T, Rays)> {
    // Without ray queries there is no scene (`ray_query` said so if they were asked for).
    if !crate::gfx::surface_setup::ray_query::on(device) {
        return None;
    }
    let mut entries = entries.to_vec();
    entries.extend(scene_entries());
    let refused = if shape.sprites.len() < RAY_SPRITE_ARRAYS as usize {
        Some("the terrain's sprite arrays aren't there".to_owned())
    } else {
        over_limits(&entries, others, &device.limits())
    };
    if let Some(why) = refused {
        log::warn!("Horizon renderer: the shader pack's rays miss everything: {why}");
        return None;
    }
    // A scope of its own: a backend that refuses these leaves the pack the ones that miss.
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Native shader pack group with the scene"),
        entries: &entries,
    });
    let built = build(&layout);
    let refused = pollster::block_on(scope.pop()).map(|e| e.to_string());
    let built = match (built, refused) {
        (Ok(built), None) => built,
        (Err(e), _) | (_, Some(e)) => {
            log::warn!("Horizon renderer: the shader pack's rays miss everything: {e}");
            return None;
        }
    };
    let words = layer_words(shape.tags, shape.materials);
    let rays = Rays {
        space: device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Native shader pack ray space"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        layers: wgpu::util::DeviceExt::create_buffer_init(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("Native shader pack sprite layers"),
                contents: bytemuck::cast_slice(&words),
                usage: wgpu::BufferUsages::STORAGE,
            },
        ),
        sprites: shape.sprites[..RAY_SPRITE_ARRAYS as usize].to_vec(),
        split: shape.split.max(1),
    };
    Some((layout, built, rays))
}

impl Rays {
    /// Writes where the camera is in `scene`, and returns this frame's `frame.ray_distance`.
    fn write(&self, queue: &wgpu::Queue, scene: &Traced<'_>) -> f32 {
        let mut space = [0u8; 16];
        space[..12].copy_from_slice(bytemuck::cast_slice(&scene.camera));
        space[12..].copy_from_slice(&self.split.to_ne_bytes());
        queue.write_buffer(&self.space, 0, &space);
        scene.distance
    }

    /// The scene's entries of a group 3.
    fn entries<'a>(&'a self, scene: &Traced<'a>) -> Vec<wgpu::BindGroupEntry<'a>> {
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: RAY_SCENE_BINDING,
                resource: scene.tlas.as_binding(),
            },
            wgpu::BindGroupEntry {
                binding: RAY_STORE_BINDING,
                resource: scene.store.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: RAY_SPACE_BINDING,
                resource: self.space.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: RAY_LAYERS_BINDING,
                resource: self.layers.as_entire_binding(),
            },
        ];
        entries.extend(
            self.sprites
                .iter()
                .zip(RAY_SPRITES_BINDING..)
                .map(|(view, binding)| wgpu::BindGroupEntry {
                    binding,
                    resource: wgpu::BindingResource::TextureView(view),
                }),
        );
        entries
    }
}

impl NativePost {
    /// Builds the passes against the scene from `scenes` (each pass's label and its module
    /// with the scene, in the passes' order), where the device traces rays and takes the
    /// bindings; else the passes keep missing every ray, and the log says why once. Returns
    /// whether they trace. `pbr` is the frame's LabPBR maps (`NativeFrame::pbr`).
    pub(in crate::gfx) fn trace_scene(
        &mut self,
        device: &wgpu::Device,
        scenes: Vec<(String, naga::Module)>,
        constants: &[(String, f64)],
        shape: &TraceShape<'_>,
        pbr: &[wgpu::TextureView; 2],
    ) -> bool {
        if scenes.len() != self.passes.len() {
            return false;
        }
        let build = |layout: &wgpu::BindGroupLayout| {
            let mut groups: [Option<&wgpu::BindGroupLayout>; GROUP as usize + 1] =
                Default::default();
            groups[GROUP as usize] = Some(layout);
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Native shader pack post with the scene"),
                bind_group_layouts: &groups,
                ..Default::default()
            });
            // A `[ray_tracing]` pack is 0.2's: every pass draws a half-float target.
            let target = INTERMEDIATE;
            Ok(scenes
                .into_iter()
                .map(|(label, module)| {
                    post_pipeline(device, &label, &pipeline_layout, module, constants, target)
                })
                .collect())
        };
        let entries = [self.entries.as_slice(), &post_pbr_entries()].concat();
        let traced = with_scene(device, (&entries, &[]), shape, build);
        self.trace = traced.map(|(layout, pipelines, rays)| NativeTrace {
            layout,
            pipelines,
            rays,
            pbr: pbr.clone(),
        });
        self.trace.is_some()
    }

    /// This frame's `frame.ray_distance` when the passes trace `scene` (`run` with the same
    /// `scene` then draws them): writes where the camera is in it. None when they don't.
    pub(in crate::gfx) fn trace(
        &self,
        queue: &wgpu::Queue,
        scene: Option<&Traced<'_>>,
    ) -> Option<f32> {
        let (trace, scene) = self.trace.as_ref().zip(scene)?;
        Some(trace.rays.write(queue, scene))
    }
}

impl NativeTrace {
    /// The scene's entries of a pass's group, and the LabPBR maps'.
    pub(super) fn entries<'a>(&'a self, scene: &Traced<'a>) -> Vec<wgpu::BindGroupEntry<'a>> {
        let mut entries = self.rays.entries(scene);
        let maps = [PBR_NORMALS_BINDING, PBR_SPECULAR_BINDING].into_iter();
        entries.extend(
            maps.zip(&self.pbr)
                .map(|(binding, view)| wgpu::BindGroupEntry {
                    binding,
                    resource: wgpu::BindingResource::TextureView(view),
                }),
        );
        entries
    }
}

impl TerrainTrace {
    /// The terrain's pipelines against the scene, made by `build` with the frame's group
    /// joined with it, where the device traces rays and the terrain's fragment stage, which
    /// also reads terrain.wgsl's group 0, holds the bindings.
    pub(in crate::gfx) fn new(
        device: &wgpu::Device,
        frame: &NativeFrame,
        terrain: &TerrainShape<'_>,
        build: impl FnOnce(&wgpu::BindGroupLayout) -> Result<MaterialPipelines, String>,
    ) -> Option<Self> {
        let groups = (frame.entries.as_slice(), terrain_entries());
        let shape = TraceShape::of(terrain);
        let (layout, pipelines, rays) = with_scene(device, (groups.0, &groups.1), &shape, build)?;
        Some(Self {
            layout,
            pipelines,
            rays,
            group: None,
            on: false,
        })
    }

    /// This frame's `frame.ray_distance` when the terrain traces `scene`: writes where the
    /// camera is in it.
    pub(in crate::gfx) fn trace(
        &self,
        queue: &wgpu::Queue,
        scene: Option<&Traced<'_>>,
    ) -> Option<f32> {
        Some(self.rays.write(queue, scene?))
    }

    /// Joins the frame's group, as it is now, with `scene`, this frame's: the terrain draws
    /// with these pipelines while there is one.
    pub(in crate::gfx) fn bind(
        &mut self,
        device: &wgpu::Device,
        frame: &NativeFrame,
        scene: Option<&Traced<'_>>,
    ) {
        self.on = scene.is_some();
        let Some(scene) = scene else {
            return;
        };
        let made = |(group, generation, _): &(wgpu::BindGroup, u64, wgpu::BindGroup)| {
            *group == frame.group && *generation == scene.generation
        };
        if !self.group.as_ref().is_some_and(made) {
            let joined = frame.joined(device, &self.layout, &self.rays.entries(scene));
            self.group = Some((frame.group.clone(), scene.generation, joined));
        }
    }

    /// The pipelines and group 3 the terrain draws with this frame, if it traces.
    pub(in crate::gfx) fn drawn(&self) -> Option<(&MaterialPipelines, &wgpu::BindGroup)> {
        let (_, _, group) = self.group.as_ref().filter(|_| self.on)?;
        Some((&self.pipelines, group))
    }
}

impl NativeFrame {
    /// The frame's group as `group` makes it, with `scene`'s entries, of the layout `layout`.
    fn joined(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        scene: &[wgpu::BindGroupEntry<'_>],
    ) -> wgpu::BindGroup {
        NativeFrame::group(
            device,
            layout,
            &self.buffer,
            (&self.reflection, &self.reflection_sampler),
            &self.shadow_map,
            (&self.opaque_depth, &self.opaque_color),
            &self.textures,
            &self.block_light,
            &self.pbr,
            scene,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sprite_layer_s_words_are_its_material_id_and_its_tags() {
        let words = layer_words(&[(2, 0b101), (0, 1)], &[(1, 10), (2, 7)]);
        assert_eq!(words, [0, 1, 10, 0, 7, 0b101]);
        // A pack with neither table: one layer of zeros, since a buffer can't be empty.
        assert_eq!(layer_words(&[], &[]), [0, 0]);
    }

    #[test]
    fn a_device_that_can_t_hold_the_scene_s_bindings_is_told_apart() {
        let mut entries = vec![wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        }];
        entries.extend(scene_entries());
        let fits = wgpu::Limits {
            max_sampled_textures_per_shader_stage: 9,
            max_storage_buffers_per_shader_stage: 2,
            max_uniform_buffers_per_shader_stage: 1,
            max_acceleration_structures_per_shader_stage: 1,
            max_bindings_per_bind_group: 60,
            ..Default::default()
        };
        assert_eq!(over_limits(&entries, &[], &fits), None);
        // OpenGL's 16 sampled textures hold the post stage's own 12 or so, not eight more.
        let few = wgpu::Limits {
            max_sampled_textures_per_shader_stage: 8,
            ..fits.clone()
        };
        let why = over_limits(&entries, &[], &few).unwrap();
        assert_eq!(why, "9 sampled textures, the device takes 8");
        let none = wgpu::Limits {
            max_acceleration_structures_per_shader_stage: 0,
            ..fits
        };
        assert!(
            over_limits(&entries, &[], &none)
                .unwrap()
                .contains("acceleration structures")
        );
    }

    /// A traced post pass also reads the LabPBR maps: two more sampled textures in its stage,
    /// at bindings below the scene's that don't raise the group's count.
    #[test]
    fn a_traced_post_pass_s_maps_count_toward_its_limits() {
        let mut entries = scene_entries();
        entries.extend(post_pbr_entries());
        let fits = wgpu::Limits {
            max_sampled_textures_per_shader_stage: 10,
            max_storage_buffers_per_shader_stage: 2,
            max_uniform_buffers_per_shader_stage: 1,
            max_acceleration_structures_per_shader_stage: 1,
            max_bindings_per_bind_group: 60,
            ..Default::default()
        };
        assert_eq!(over_limits(&entries, &[], &fits), None);
        let few = wgpu::Limits {
            max_sampled_textures_per_shader_stage: 9,
            ..fits
        };
        let why = over_limits(&entries, &[], &few).unwrap();
        assert_eq!(why, "10 sampled textures, the device takes 9");
    }

    /// The terrain's fragment stage reads terrain.wgsl's group 0 besides the frame's group:
    /// its textures and uniform count toward the same per-stage limits, a binding number of its
    /// own doesn't count toward the group's.
    #[test]
    fn the_terrain_s_own_group_counts_toward_its_fragment_stage_s_limits() {
        let mut entries = scene_entries();
        let terrain = terrain_entries();
        // 8 sprite arrays, the lightmap and the emission thresholds; the globals' uniform.
        let textures =
            |e: &&wgpu::BindGroupLayoutEntry| matches!(e.ty, wgpu::BindingType::Texture { .. });
        assert_eq!(terrain.iter().filter(textures).count(), 10);
        // What only the vertex stage reads doesn't count.
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 2,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        });
        let fits = wgpu::Limits {
            max_sampled_textures_per_shader_stage: 18,
            max_storage_buffers_per_shader_stage: 2,
            max_uniform_buffers_per_shader_stage: 2,
            max_acceleration_structures_per_shader_stage: 1,
            max_bindings_per_bind_group: 60,
            ..Default::default()
        };
        assert_eq!(over_limits(&entries, &terrain, &fits), None);
        let few = wgpu::Limits {
            max_sampled_textures_per_shader_stage: 17,
            ..fits.clone()
        };
        let why = over_limits(&entries, &terrain, &few).unwrap();
        assert_eq!(why, "18 sampled textures, the device takes 17");
        // The scene's group without the terrain's would have fitted.
        assert_eq!(over_limits(&entries, &[], &few), None);
        let one = wgpu::Limits {
            max_uniform_buffers_per_shader_stage: 1,
            ..fits
        };
        assert!(
            over_limits(&entries, &terrain, &one)
                .unwrap()
                .contains("2 uniform buffers")
        );
    }
}
