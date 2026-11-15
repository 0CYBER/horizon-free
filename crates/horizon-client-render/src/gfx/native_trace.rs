//! The scene in the post passes of a shader pack with `[ray_tracing]` (docs/shaders.md, spec
//! #15624): where the device traces rays, each pass is built a second time from the module the
//! check made against the scene (`CheckedStage::scene`), and its group 3 also holds the TLAS,
//! the store of the sections' vertices (gfx/ray_scene.rs), the block sprites and the pack's
//! material id and tags of each sprite layer, which the prelude's `trace` reads. Those passes
//! draw in frames that have a scene; in the others, and on a device that can't hold the
//! bindings, the passes whose `trace` misses draw, as on a device without ray queries.

use horizon_shader_api::stages::{
    GROUP, RAY_LAYERS_BINDING, RAY_SCENE_BINDING, RAY_SPACE_BINDING, RAY_SPRITE_ARRAYS,
    RAY_SPRITES_BINDING, RAY_STORE_BINDING,
};

use super::{INTERMEDIATE, NativePost, post_pipeline};
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

/// The post passes built against the scene, and what they read besides it.
pub(super) struct NativeTrace {
    /// The post group's layout with the scene's bindings.
    pub(super) layout: wgpu::BindGroupLayout,
    /// One pipeline a pass, in the passes' order.
    pub(super) pipelines: Vec<wgpu::RenderPipeline>,
    /// The prelude's `horizon_ray_space`: the camera in the scene's space, then `split`.
    space: wgpu::Buffer,
    /// The prelude's `horizon_ray_layers`.
    layers: wgpu::Buffer,
    sprites: Vec<wgpu::TextureView>,
    split: u32,
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

/// The scene's bindings in the post group, for the fragment stage.
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

/// What `limits` lacks to hold a fragment stage's `entries`, if anything.
fn over_limits(entries: &[wgpu::BindGroupLayoutEntry], limits: &wgpu::Limits) -> Option<String> {
    let count = |wanted: fn(&wgpu::BindingType) -> bool| {
        entries.iter().filter(|e| wanted(&e.ty)).count() as u32
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
            entries.iter().map(|e| e.binding + 1).max().unwrap_or(0),
            limits.max_bindings_per_bind_group,
        ),
    ];
    needs
        .into_iter()
        .find(|&(_, need, limit)| need > limit)
        .map(|(what, need, limit)| format!("{need} {what}, the device takes {limit}"))
}

impl NativePost {
    /// Builds the passes against the scene from `scenes` (each pass's label and its module
    /// with the scene, in the passes' order), where the device traces rays and takes the
    /// bindings; else the passes keep missing every ray, and the log says why once.
    pub(in crate::gfx) fn trace_scene(
        &mut self,
        device: &wgpu::Device,
        scenes: Vec<(String, naga::Module)>,
        constants: &[(String, f64)],
        shape: &TraceShape<'_>,
    ) {
        // Without ray queries there is no scene (`ray_query` said so if they were asked for).
        if !crate::gfx::surface_setup::ray_query::on(device) || scenes.len() != self.passes.len() {
            return;
        }
        let mut entries = self.entries.clone();
        entries.extend(scene_entries());
        let refused = if shape.sprites.len() < RAY_SPRITE_ARRAYS as usize {
            Some("the terrain's sprite arrays aren't there".to_owned())
        } else {
            over_limits(&entries, &device.limits())
        };
        if let Some(why) = refused {
            log::warn!("Horizon renderer: the shader pack's rays miss everything: {why}");
            return;
        }
        // A scope of its own: a backend that refuses these passes leaves the pack its others.
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Native shader pack post with the scene"),
            entries: &entries,
        });
        let mut groups: [Option<&wgpu::BindGroupLayout>; GROUP as usize + 1] = Default::default();
        groups[GROUP as usize] = Some(&layout);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Native shader pack post with the scene"),
            bind_group_layouts: &groups,
            ..Default::default()
        });
        // A `[ray_tracing]` pack is 0.2's: every pass draws a half-float target.
        let pipelines = scenes
            .into_iter()
            .map(|(label, module)| {
                let target = INTERMEDIATE;
                post_pipeline(device, &label, &pipeline_layout, module, constants, target)
            })
            .collect();
        if let Some(e) = pollster::block_on(scope.pop()) {
            log::warn!("Horizon renderer: the shader pack's rays miss everything: {e}");
            return;
        }
        let words = layer_words(shape.tags, shape.materials);
        self.trace = Some(NativeTrace {
            layout,
            pipelines,
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
        });
    }

    /// This frame's `frame.ray_distance` when the passes trace `scene` (`run` with the same
    /// `scene` then draws them): writes where the camera is in it. None when they don't.
    pub(in crate::gfx) fn trace(
        &self,
        queue: &wgpu::Queue,
        scene: Option<&Traced<'_>>,
    ) -> Option<f32> {
        let (trace, scene) = self.trace.as_ref().zip(scene)?;
        let mut space = [0u8; 16];
        space[..12].copy_from_slice(bytemuck::cast_slice(&scene.camera));
        space[12..].copy_from_slice(&trace.split.to_ne_bytes());
        queue.write_buffer(&trace.space, 0, &space);
        Some(scene.distance)
    }
}

impl NativeTrace {
    /// The scene's entries of a pass's group.
    pub(super) fn entries<'a>(&'a self, scene: &Traced<'a>) -> Vec<wgpu::BindGroupEntry<'a>> {
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
        assert_eq!(over_limits(&entries, &fits), None);
        // OpenGL's 16 sampled textures hold the post stage's own 12 or so, not eight more.
        let few = wgpu::Limits {
            max_sampled_textures_per_shader_stage: 8,
            ..fits.clone()
        };
        let why = over_limits(&entries, &few).unwrap();
        assert_eq!(why, "9 sampled textures, the device takes 8");
        let none = wgpu::Limits {
            max_acceleration_structures_per_shader_stage: 0,
            ..fits
        };
        assert!(
            over_limits(&entries, &none)
                .unwrap()
                .contains("acceleration structures")
        );
    }
}
