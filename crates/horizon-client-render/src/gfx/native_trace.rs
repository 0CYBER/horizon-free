//! The scene in the post passes, the compute passes and the material and entity stages of a shader pack with
//! `[ray_tracing]` (docs/shaders.md, spec #15624): where the device traces rays, each post pass,
//! each compute or `[[before_world]]` pass that calls a trace function, and the terrain and entity pipelines
//! of stages that do, are built a second time from what the check made
//! against the scene (`CheckedStage::scene`, `scene_source`), and their group 3 also holds the
//! TLAS, the store of the sections' vertices (gfx/rt/scene.rs), the block sprites and the
//! pack's material id and tags of each sprite layer, which the prelude's `trace` reads. Those
//! run in frames that have a scene; in the others, and on a device that can't hold the
//! bindings, the ones whose `trace` misses run, as on a device without ray queries. A pack whose
//! files name `RAY_ENTITY` also has the entity layer's vertices, atlas and sampler there, and
//! the retained block entities' vertices. A pass whose files name the light sources, in a pack with
//! `[ray_tracing] lights = true`, also has them and their grid (gfx/ray_lights.rs) at binding 80.

use horizon_shader_api::stages::{
    GROUP, PBR_NORMALS_BINDING, PBR_SPECULAR_BINDING, RAY_ENTITY_ATLAS_BINDING,
    RAY_ENTITY_RETAINED_BINDING, RAY_ENTITY_SAMPLER_BINDING, RAY_ENTITY_STORE_BINDING,
    RAY_LAYERS_BINDING, RAY_LIGHTS_BINDING, RAY_LOOKS_BINDING, RAY_SCENE_BINDING,
    RAY_SPACE_BINDING, RAY_SPRITE_ARRAYS, RAY_SPRITES_BINDING, RAY_STORE_BINDING,
};

use super::super::native_material::{MaterialPipelines, TerrainShape};
use super::kept::Reads;
use super::{NativeFrame, NativePost, post_pipeline};
pub(in crate::gfx) use crate::gfx::rt::scene::Traced;

/// What the scene's passes are built against besides the scene: the terrain's sprites and the
/// pack's tables of them.
#[derive(Clone)]
pub(in crate::gfx) struct TraceShape<'a> {
    /// terrain.wgsl's sprite arrays, the unused ones a stand-in.
    pub sprites: &'a [wgpu::TextureView],
    /// Layers per sprite array (terrain.wgsl's `SPRITE_SPLIT`).
    pub split: u32,
    /// The pack's tags and material id of each sprite layer that has any, as `(layer, value)`:
    /// the lists the material stage's `input.tags` and `input.material` come from.
    pub tags: &'a [(u32, u32)],
    pub materials: &'a [(u32, u32)],
    /// Whether the stages read the entity layer (`CheckedStage::traces_entities`).
    pub entities: bool,
    /// The light sources (`NativeFrame::ray_lights`), where a stage of the pack reads them
    /// (`CheckedStage::traces_lights`): each group that does binds them.
    pub lights: Option<wgpu::Buffer>,
}

impl<'a> TraceShape<'a> {
    pub(in crate::gfx) fn of(
        terrain: &TerrainShape<'a>,
        entities: bool,
        lights: Option<wgpu::Buffer>,
    ) -> Self {
        Self {
            sprites: terrain.sprite_views,
            split: terrain.sprites[1] as u32,
            tags: terrain.sprite_tags,
            materials: terrain.sprite_materials,
            entities,
            lights,
        }
    }
}

/// What a stage built against the scene reads besides the TLAS and the store.
pub(in crate::gfx) struct Rays {
    /// The prelude's `horizon_ray_space`: the camera in the scene's space, `split`, the
    /// scene's counts of solid and of terrain instances, and the entity layer's three bases.
    space: wgpu::Buffer,
    /// The prelude's `horizon_ray_layers`.
    layers: wgpu::Buffer,
    sprites: Vec<wgpu::TextureView>,
    split: u32,
    /// Whether the group has the entity layer's bindings.
    entities: bool,
    /// The light sources, where a group has them.
    lights: Option<wgpu::Buffer>,
    /// While the group has the terrain's looks, what it binds there before the scene has
    /// them: a quad without a look.
    no_looks: Option<wgpu::Buffer>,
}

/// The post and compute passes built against the scene, and what they read besides it.
pub(super) struct NativeTrace {
    /// Each pass's group layout with the scene's bindings, the post passes' then, in a chain
    /// with kept resources, the compute passes' (`KeptStore::passes`), none for one that
    /// doesn't trace. Post passes without kept resources share one when they need the same
    /// scene bindings.
    layouts: Vec<Option<wgpu::BindGroupLayout>>,
    /// Which pass layouts include binding 81, and which binding 80, in the same order as
    /// `layouts`.
    binds: Vec<Binds>,
    /// One pipeline a post pass, in the passes' order.
    pub(super) pipelines: Vec<wgpu::RenderPipeline>,
    /// One a compute pass that traces, in `KeptStore::computes`' order.
    pub(super) computes: Vec<Option<wgpu::ComputePipeline>>,
    rays: Rays,
    /// The resource packs' LabPBR maps (`NativeFrame::pbr`), for `hit_normal_map` and its kin.
    pbr: [wgpu::TextureView; 2],
}

/// A stage's pipelines `P` built against the scene (a material stage's terrain, an entity
/// stage's entities), and its frame's group with the scene in it.
pub(in crate::gfx) struct StageTrace<P> {
    layout: wgpu::BindGroupLayout,
    pipelines: P,
    rays: Rays,
    /// The group, made for the frame's group `.0`, the scene of generation `.1` (the TLAS and
    /// the store are replaced as they grow, and the frame's group as its targets change) and
    /// the kept resources the stage reads `.2` (`Reads::made`): one, or one a parity of them.
    group: Option<(wgpu::BindGroup, u64, Option<u32>, Vec<wgpu::BindGroup>)>,
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

/// Whether a group 3 has the terrain's looks (binding 81) and the light sources (binding 80):
/// only a pass whose module declares one has it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Binds {
    looks: bool,
    lights: bool,
}

impl Binds {
    fn of(module: &naga::Module) -> Self {
        Self {
            looks: has_binding(module, RAY_LOOKS_BINDING),
            lights: has_binding(module, RAY_LIGHTS_BINDING),
        }
    }
}

/// The scene's bindings in group 3, for the shader stage `visibility`, with the entity layer's
/// where `entities`, the terrain's looks where `looks` and the light sources where `lights`.
fn scene_entries(
    visibility: wgpu::ShaderStages,
    entities: bool,
    looks: bool,
    lights: bool,
) -> Vec<wgpu::BindGroupLayoutEntry> {
    let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
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
    if entities {
        entries.extend([
            entry(RAY_ENTITY_STORE_BINDING, buffer(read)),
            entry(
                RAY_ENTITY_ATLAS_BINDING,
                wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
            ),
            entry(
                RAY_ENTITY_SAMPLER_BINDING,
                wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            ),
            entry(RAY_ENTITY_RETAINED_BINDING, buffer(read)),
        ]);
    }
    if looks {
        entries.push(entry(RAY_LOOKS_BINDING, buffer(read)));
    }
    if lights {
        entries.push(entry(RAY_LIGHTS_BINDING, buffer(read)));
    }
    entries
}

/// The LabPBR maps of a post or compute pass built against the scene, for the shader stage
/// `visibility`, at the bindings the material stage has them (`NativeFrame`'s 12 and 13): the
/// post group has none of its own there.
fn post_pbr_entries(visibility: wgpu::ShaderStages) -> [wgpu::BindGroupLayoutEntry; 2] {
    [PBR_NORMALS_BINDING, PBR_SPECULAR_BINDING].map(|binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Texture {
            // Sampled with the post stage's `nearest_sampler`.
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2Array,
            multisampled: false,
        },
        count: None,
    })
}

/// What `limits` lacks for the shader stage `stage` that reads the group `group` and the other
/// groups' entries `others`, if anything.
pub(in crate::gfx) fn over_limits(
    group: &[wgpu::BindGroupLayoutEntry],
    others: &[wgpu::BindGroupLayoutEntry],
    limits: &wgpu::Limits,
    stage: wgpu::ShaderStages,
) -> Option<String> {
    let seen = || (group.iter().chain(others)).filter(|e| e.visibility.contains(stage));
    let count =
        |wanted: fn(&wgpu::BindingType) -> bool| seen().filter(|e| wanted(&e.ty)).count() as u32;
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
            "storage textures",
            count(|t| matches!(t, wgpu::BindingType::StorageTexture { .. })),
            limits.max_storage_textures_per_shader_stage,
        ),
        (
            "acceleration structures",
            count(|t| matches!(t, wgpu::BindingType::AccelerationStructure { .. })),
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

/// Builds with `build` what runs against the scene, in a group 3 of each of `groups`' entries
/// and the scene's. Each group names its shader stage and whether it reads looks and the light
/// sources. The device must hold these bindings beside the other groups' entries `others`. Else
/// None: what
/// doesn't trace keeps missing every ray, all of it, so that `frame.has_ray_tracing` holds for
/// every pass.
fn with_scene<T>(
    device: &wgpu::Device,
    (groups, others): (
        Vec<(Vec<wgpu::BindGroupLayoutEntry>, wgpu::ShaderStages, Binds)>,
        &[wgpu::BindGroupLayoutEntry],
    ),
    shape: &TraceShape<'_>,
    build: impl FnOnce(&[wgpu::BindGroupLayout]) -> Result<T, String>,
) -> Option<(Vec<wgpu::BindGroupLayout>, T, Rays)> {
    // Without ray queries there is no scene (`ray_query` said so if they were asked for).
    if !crate::gfx::rt::query::on(device) {
        return None;
    }
    let any_looks = groups.iter().any(|(_, _, binds)| binds.looks);
    let any_lights = groups.iter().any(|(_, _, binds)| binds.lights);
    let groups: Vec<_> = groups
        .into_iter()
        .map(|(mut entries, stage, binds)| {
            let (looks, lights) = (binds.looks, binds.lights);
            entries.extend(scene_entries(stage, shape.entities, looks, lights));
            (entries, stage)
        })
        .collect();
    let refused = if shape.sprites.len() < RAY_SPRITE_ARRAYS as usize {
        Some("the terrain's sprite arrays aren't there".to_owned())
    } else {
        let limits = device.limits();
        (groups.iter()).find_map(|(entries, stage)| over_limits(entries, others, &limits, *stage))
    };
    if let Some(why) = refused {
        log::warn!("Horizon renderer: the shader pack's rays miss everything: {why}");
        return None;
    }
    // A scope of its own: a backend that refuses these leaves the pack the ones that miss.
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let layouts: Vec<_> = (groups.iter())
        .map(|(entries, _)| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Native shader pack group with the scene"),
                entries,
            })
        })
        .collect();
    let built = build(&layouts);
    let internal = pollster::block_on(internal.pop());
    let refused = internal
        .or(pollster::block_on(validation.pop()))
        .map(|e| e.to_string());
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
            size: 48,
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
        entities: shape.entities,
        lights: shape.lights.clone().filter(|_| any_lights),
        no_looks: any_looks.then(|| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Native shader pack: no looks"),
                size: std::mem::size_of::<crate::mesher::QuadLook>() as u64,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            })
        }),
    };
    Some((layouts, built, rays))
}

impl Rays {
    /// Writes where the camera is in `scene`, which of its instances are solid and which
    /// translucent terrain's, and where the entity layer's geometries start, and returns this
    /// frame's `frame.ray_distance`.
    fn write(&self, queue: &wgpu::Queue, scene: &Traced<'_>) -> f32 {
        queue.write_buffer(
            &self.space,
            0,
            bytemuck::cast_slice(&space_words(self.split, scene)),
        );
        scene.distance
    }

    /// The scene's entries of a group 3.
    fn entries<'a>(&'a self, scene: &Traced<'a>, binds: Binds) -> Vec<wgpu::BindGroupEntry<'a>> {
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
        // The renderer has the layer whenever a native pack's rays read it (`ray_prepare`).
        if let Some(e) = scene.entities.as_ref().filter(|_| self.entities) {
            let bindings = [
                RAY_ENTITY_STORE_BINDING,
                RAY_ENTITY_ATLAS_BINDING,
                RAY_ENTITY_SAMPLER_BINDING,
                RAY_ENTITY_RETAINED_BINDING,
            ];
            let resources = [
                wgpu::BindingResource::Buffer(e.vertices.clone()),
                wgpu::BindingResource::TextureView(e.atlas),
                wgpu::BindingResource::Sampler(e.sampler),
                e.retained.as_entire_binding(),
            ];
            let entry = |(binding, resource)| wgpu::BindGroupEntry { binding, resource };
            entries.extend(bindings.into_iter().zip(resources).map(entry));
        }
        if let Some(none) = self.no_looks.as_ref().filter(|_| binds.looks) {
            entries.push(wgpu::BindGroupEntry {
                binding: RAY_LOOKS_BINDING,
                resource: scene.looks.unwrap_or(none).as_entire_binding(),
            });
        }
        if let Some(lights) = self.lights.as_ref().filter(|_| binds.lights) {
            entries.push(wgpu::BindGroupEntry {
                binding: RAY_LIGHTS_BINDING,
                resource: lights.as_entire_binding(),
            });
        }
        entries
    }
}

/// `horizon_RaySpace`'s twelve words for `scene`, its sprite arrays holding `split` layers each:
/// the retained block entities' first instance at word 6, the Nether at word 7, and the entity
/// layer's bases from word 8, where WGSL aligns a `vec3<u32>`.
pub(in crate::gfx) fn space_words(split: u32, scene: &Traced<'_>) -> [u32; 12] {
    let [x, y, z] = scene.camera.map(f32::to_bits);
    let [a, b, c] = scene.entities.as_ref().map_or([0; 3], |e| e.bases);
    let retained = scene.entities.as_ref().map_or(0, |e| e.retained_instances);
    let instances = [scene.solid_instances, scene.translucent_instances];
    [
        x,
        y,
        z,
        split,
        instances[0],
        instances[1],
        retained,
        u32::from(scene.nether),
        a,
        b,
        c,
        0,
    ]
}

impl NativePost {
    /// Builds the passes against the scene from `scenes` (each post pass's label and its
    /// module with the scene, in the passes' order) and, in a chain with kept resources, the
    /// compute passes whose files trace (`ComputePass::scene`), where the device traces rays and
    /// takes the bindings; else every pass keeps missing every ray, and the log says why once.
    /// Returns whether they trace. `pbr` is the frame's LabPBR maps (`NativeFrame::pbr`).
    pub(in crate::gfx) fn trace_scene(
        &mut self,
        device: &wgpu::Device,
        scenes: Vec<(String, naga::Module)>,
        constants: &[(String, f64)],
        shape: &TraceShape<'_>,
        pbr: &[wgpu::TextureView; 2],
    ) -> bool {
        let computes: Vec<_> = (self.kept.iter_mut())
            .flat_map(|k| k.computes.iter_mut())
            .map(|p| Some((p.label.clone(), p.scene.take()?)))
            .collect();
        let ordinary = self.passes.len();
        if scenes.len() != ordinary || scenes.is_empty() && computes.iter().all(Option::is_none) {
            return false;
        }
        let none = Binds {
            looks: false,
            lights: false,
        };
        let binds: Vec<_> = scenes
            .iter()
            .map(|(_, module)| Binds::of(module))
            .chain(
                computes
                    .iter()
                    .map(|pass| pass.as_ref().map_or(none, |(_, module)| Binds::of(module))),
            )
            .collect();
        let shared = self.kept.is_none() && binds.windows(2).all(|pair| pair[0] == pair[1]);
        let (fragment, compute) = (wgpu::ShaderStages::FRAGMENT, wgpu::ShaderStages::COMPUTE);
        // Only the passes that use looks or the light sources consume another storage-buffer
        // slot for each.
        let groups: Vec<_> = match &self.kept {
            Some(k) => (k.passes.iter().enumerate())
                .filter(|(i, _)| *i < ordinary || computes[i - ordinary].is_some())
                .map(|(i, p)| {
                    let stage = if i < ordinary { fragment } else { compute };
                    (
                        [p.entries.as_slice(), &post_pbr_entries(stage)].concat(),
                        stage,
                        binds[i],
                    )
                })
                .collect(),
            None => binds
                .iter()
                .take(if shared { 1 } else { ordinary })
                .map(|&binds| {
                    (
                        [self.entries.as_slice(), &post_pbr_entries(fragment)].concat(),
                        fragment,
                        binds,
                    )
                })
                .collect(),
        };
        let (plan, kept) = (&self.plan, &self.kept);
        let build = |layouts: &[wgpu::BindGroupLayout]| {
            let pipeline_layout = |layout: &wgpu::BindGroupLayout| {
                let mut groups: [Option<&wgpu::BindGroupLayout>; GROUP as usize + 1] =
                    Default::default();
                groups[GROUP as usize] = Some(layout);
                device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Native shader pack post with the scene"),
                    bind_group_layouts: &groups,
                    ..Default::default()
                })
            };
            let pipeline_layouts: Vec<_> = layouts.iter().map(pipeline_layout).collect();
            let pipelines: Vec<wgpu::RenderPipeline> = (scenes.into_iter().enumerate())
                .map(|(i, (label, module))| {
                    let layout = &pipeline_layouts[i.min(pipeline_layouts.len() - 1)];
                    let target = super::kept::target(kept, plan[i].writes);
                    post_pipeline(device, &label, layout, module, constants, target)
                })
                .collect();
            // The tracing compute passes' layouts come after the post passes'.
            let mut own = pipeline_layouts.iter().skip(ordinary);
            let computes: Vec<Option<wgpu::ComputePipeline>> = (computes.into_iter())
                .map(|scene: Option<(String, naga::Module)>| {
                    let (label, module) = scene?;
                    let layout = own.next()?;
                    Some(super::kept::compute_pipeline(
                        device, &label, layout, module, constants,
                    ))
                })
                .collect();
            Ok((pipelines, computes))
        };
        let traced = with_scene(device, (groups, &[]), shape, build);
        self.trace = traced.map(|(made, (pipelines, computes), rays)| {
            let mut made = made.into_iter();
            let layouts = match &self.kept {
                // Kept passes in order, a compute pass that doesn't trace without one.
                Some(k) => (0..k.passes.len())
                    .map(|i| {
                        (i < ordinary || computes_trace(&computes, i - ordinary))
                            .then(|| made.next())
                            .flatten()
                    })
                    .collect(),
                None if shared => vec![made.next(); ordinary],
                None => made.map(Some).collect(),
            };
            NativeTrace {
                layouts,
                binds,
                pipelines,
                computes,
                rays,
                pbr: pbr.clone(),
            }
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

    /// Whether the passes trace the entity layer.
    pub(in crate::gfx) fn traces_entities(&self) -> bool {
        self.trace.as_ref().is_some_and(|t| t.rays.entities)
    }

    /// Whether the passes' rays read the terrain's looks.
    pub(in crate::gfx) fn traces_looks(&self) -> bool {
        self.trace
            .as_ref()
            .is_some_and(|t| t.rays.no_looks.is_some())
    }

    /// Whether a pass built against the scene binds the light sources.
    pub(in crate::gfx) fn traces_lights(&self) -> bool {
        self.trace.as_ref().is_some_and(|t| t.rays.lights.is_some())
    }
}

fn has_binding(module: &naga::Module, binding: u32) -> bool {
    module.global_variables.iter().any(|(_, g)| {
        g.binding
            .as_ref()
            .is_some_and(|b| b.group == GROUP && b.binding == binding)
    })
}

/// Whether compute pass `c` has a pipeline against the scene.
fn computes_trace(computes: &[Option<wgpu::ComputePipeline>], c: usize) -> bool {
    computes.get(c).is_some_and(Option::is_some)
}

impl NativeTrace {
    /// Pass `pass`'s layout with the scene (its index in `KeptStore::passes`, or a post
    /// pass's), if it traces.
    pub(super) fn layout(&self, pass: usize) -> Option<&wgpu::BindGroupLayout> {
        self.layouts.get(pass)?.as_ref()
    }

    /// The scene's entries of a pass's group, and the LabPBR maps'.
    pub(super) fn entries<'a>(
        &'a self,
        pass: usize,
        scene: &Traced<'a>,
    ) -> Vec<wgpu::BindGroupEntry<'a>> {
        let mut entries = self.rays.entries(scene, self.binds[pass]);
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

/// A material stage's terrain against the scene.
pub(in crate::gfx) type TerrainTrace = StageTrace<MaterialPipelines>;

impl<P> StageTrace<P> {
    /// The stage's pipelines against the scene, made by `build` with the frame's group joined
    /// with it, where the device traces rays and the stage's fragment shader, which also reads
    /// its own groups' entries `others` (terrain.wgsl's group 0, the entities'), holds the
    /// bindings.
    pub(in crate::gfx) fn new(
        device: &wgpu::Device,
        frame: &NativeFrame,
        (others, shape, (looks, lights)): (
            &[wgpu::BindGroupLayoutEntry],
            &TraceShape<'_>,
            (bool, bool),
        ),
        kept: &[wgpu::BindGroupLayoutEntry],
        build: impl FnOnce(&wgpu::BindGroupLayout) -> Result<P, String>,
    ) -> Option<Self> {
        let fragment = wgpu::ShaderStages::FRAGMENT;
        let binds = Binds { looks, lights };
        let own = vec![([&frame.entries[..], kept].concat(), fragment, binds)];
        // The light sources only where the stage reads them.
        let lights = lights.then(|| frame.ray_lights.as_ref().map(|l| l.buffer.clone()));
        let shape = &TraceShape {
            lights: lights.flatten(),
            ..shape.clone()
        };
        let build = |layouts: &[wgpu::BindGroupLayout]| build(&layouts[0]);
        let (layouts, pipelines, rays) = with_scene(device, (own, others), shape, build)?;
        Some(Self {
            layout: layouts.into_iter().next()?,
            pipelines,
            rays,
            group: None,
            on: false,
        })
    }

    /// This frame's `frame.ray_distance` when the stage traces `scene`: writes where the
    /// camera is in it.
    pub(in crate::gfx) fn trace(
        &self,
        queue: &wgpu::Queue,
        scene: Option<&Traced<'_>>,
    ) -> Option<f32> {
        Some(self.rays.write(queue, scene?))
    }

    /// Joins the frame's group, as it is now, with `scene`, this frame's, and the kept
    /// resources the stage reads (`reads`): the stage draws with these pipelines while
    /// there is a scene.
    pub(in crate::gfx) fn bind(
        &mut self,
        device: &wgpu::Device,
        frame: &NativeFrame,
        scene: Option<&Traced<'_>>,
        reads: Option<Reads<'_>>,
    ) {
        self.on = scene.is_some();
        let Some(scene) = scene else {
            return;
        };
        let stamp = reads.map(|r| r.made());
        let made = |(group, generation, kept, _): &(_, u64, Option<u32>, _)| {
            *group == frame.group && *generation == scene.generation && *kept == stamp
        };
        if !self.group.as_ref().is_some_and(made) {
            let join = |parity| {
                let binds = Binds {
                    looks: self.traces_looks(),
                    lights: self.traces_lights(),
                };
                let mut entries = self.rays.entries(scene, binds);
                entries.extend(reads.map(|r| r.entries(parity)).unwrap_or_default());
                frame.joined(device, &self.layout, &entries)
            };
            let groups = match reads {
                Some(_) => vec![join(0), join(1)],
                None => vec![join(0)],
            };
            self.group = Some((frame.group.clone(), scene.generation, stamp, groups));
        }
    }

    /// Group 3's layout with the scene, for a test's own entry points.
    #[cfg(test)]
    pub(in crate::gfx) fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }

    /// Whether the stage traces the entity layer.
    pub(in crate::gfx) fn traces_entities(&self) -> bool {
        self.rays.entities
    }

    /// Whether the stage's rays read the terrain's looks.
    pub(in crate::gfx) fn traces_looks(&self) -> bool {
        self.rays.no_looks.is_some()
    }

    /// Whether the stage's rays read the light sources.
    pub(in crate::gfx) fn traces_lights(&self) -> bool {
        self.rays.lights.is_some()
    }

    /// The pipelines and group 3 the stage draws with this frame, if it traces, the kept
    /// resources' version `parity` this frame's.
    pub(in crate::gfx) fn drawn(&self, parity: usize) -> Option<(&P, &wgpu::BindGroup)> {
        let (_, _, _, groups) = self.group.as_ref().filter(|_| self.on)?;
        Some((&self.pipelines, &groups[parity.min(groups.len() - 1)]))
    }
}

impl NativeFrame {
    /// The frame's group as `group` makes it, with `scene`'s entries, of the layout `layout`.
    pub(in crate::gfx) fn joined(
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
    use super::super::super::native_material::terrain_entries;
    use super::*;
    const FRAGMENT: wgpu::ShaderStages = wgpu::ShaderStages::FRAGMENT;

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
        entries.extend(scene_entries(FRAGMENT, false, false, false));
        let fits = wgpu::Limits {
            max_sampled_textures_per_shader_stage: 9,
            max_storage_buffers_per_shader_stage: 2,
            max_uniform_buffers_per_shader_stage: 1,
            max_acceleration_structures_per_shader_stage: 1,
            max_bindings_per_bind_group: 60,
            ..Default::default()
        };
        assert_eq!(over_limits(&entries, &[], &fits, FRAGMENT), None);
        // OpenGL's 16 sampled textures hold the post stage's own 12 or so, not eight more.
        let few = wgpu::Limits {
            max_sampled_textures_per_shader_stage: 8,
            ..fits.clone()
        };
        let why = over_limits(&entries, &[], &few, FRAGMENT).unwrap();
        assert_eq!(why, "9 sampled textures, the device takes 8");
        let none = wgpu::Limits {
            max_acceleration_structures_per_shader_stage: 0,
            ..fits
        };
        assert!(
            over_limits(&entries, &[], &none, FRAGMENT)
                .unwrap()
                .contains("acceleration structures")
        );
    }

    /// Each stage counts what it sees: a compute pass's storage textures, not the fragment
    /// stage's textures, and no acceleration structure without the scene.
    #[test]
    fn each_stage_counts_its_own_bindings() {
        let (compute, both) = (
            wgpu::ShaderStages::COMPUTE,
            FRAGMENT | wgpu::ShaderStages::COMPUTE,
        );
        let at = |binding, visibility, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty,
            count: None,
        };
        let storage = wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: wgpu::TextureFormat::R32Float,
            view_dimension: wgpu::TextureViewDimension::D2,
        };
        let entries = [
            at(64, compute, storage),
            at(66, compute, storage),
            at(1, both, post_pbr_entries(FRAGMENT)[0].ty),
        ];
        let limits = wgpu::Limits {
            max_storage_textures_per_shader_stage: 1,
            max_sampled_textures_per_shader_stage: 1,
            max_acceleration_structures_per_shader_stage: 0,
            ..Default::default()
        };
        assert_eq!(over_limits(&entries, &[], &limits, FRAGMENT), None);
        let why = over_limits(&entries, &[], &limits, compute);
        assert_eq!(
            why.as_deref(),
            Some("2 storage textures, the device takes 1")
        );
    }

    /// A compute pass with everything the scene can bind that takes a storage buffer (the store,
    /// the layers, the entity layer's two, the looks and the light sources) and its kept
    /// buffers: two kept buffers fill a device of 8, a third is refused, which leaves the
    /// pack's rays missing everywhere (`with_scene`). Without the light sources it fits again.
    #[test]
    fn the_light_sources_count_toward_the_limits_with_the_rest() {
        let compute = wgpu::ShaderStages::COMPUTE;
        let kept = |n: u32| {
            (0..n).map(move |i| wgpu::BindGroupLayoutEntry {
                binding: horizon_shader_api::stages::KEPT_BINDING + i,
                visibility: compute,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            })
        };
        let group = |lights, kept_buffers| {
            let mut entries = scene_entries(compute, true, true, lights);
            entries.extend(post_pbr_entries(compute));
            entries.extend(kept(kept_buffers));
            entries
        };
        let eight = wgpu::Limits {
            max_storage_buffers_per_shader_stage: 8,
            max_acceleration_structures_per_shader_stage: 1,
            ..Default::default()
        };
        assert_eq!(over_limits(&group(true, 2), &[], &eight, compute), None);
        let why = over_limits(&group(true, 3), &[], &eight, compute).unwrap();
        assert_eq!(why, "9 storage buffers, the device takes 8");
        assert_eq!(over_limits(&group(false, 3), &[], &eight, compute), None);
    }

    /// The entity layer's store, atlas, sampler and retained store count too: two more storage
    /// buffers and a sampled texture, and the group's bindings up to the retained store's.
    #[test]
    fn the_entity_layer_counts_toward_the_limits() {
        let entries = scene_entries(FRAGMENT, true, false, false);
        let fits = wgpu::Limits {
            max_sampled_textures_per_shader_stage: 9,
            max_storage_buffers_per_shader_stage: 4,
            max_uniform_buffers_per_shader_stage: 1,
            max_acceleration_structures_per_shader_stage: 1,
            max_bindings_per_bind_group: 64,
            ..Default::default()
        };
        assert_eq!(over_limits(&entries, &[], &fits, FRAGMENT), None);
        let three = wgpu::Limits {
            max_storage_buffers_per_shader_stage: 3,
            ..fits.clone()
        };
        let why = over_limits(&entries, &[], &three, FRAGMENT).unwrap();
        assert_eq!(why, "4 storage buffers, the device takes 3");
        let sixty_three = wgpu::Limits {
            max_bindings_per_bind_group: 63,
            ..fits.clone()
        };
        let why = over_limits(&entries, &[], &sixty_three, FRAGMENT).unwrap();
        assert_eq!(why, "64 bindings in a group, the device takes 63");
        let two = wgpu::Limits {
            max_storage_buffers_per_shader_stage: 2,
            ..fits.clone()
        };
        let eight = wgpu::Limits {
            max_sampled_textures_per_shader_stage: 8,
            ..fits.clone()
        };
        assert!(over_limits(&entries, &[], &eight, FRAGMENT).is_some());
        // Without the layer the scene fits as before.
        assert_eq!(
            over_limits(
                &scene_entries(FRAGMENT, false, false, false),
                &[],
                &two,
                FRAGMENT
            ),
            None
        );
    }

    /// A traced post pass also reads the LabPBR maps: two more sampled textures in its stage,
    /// at bindings below the scene's that don't raise the group's count.
    #[test]
    fn a_traced_post_pass_s_maps_count_toward_its_limits() {
        let mut entries = scene_entries(FRAGMENT, false, false, false);
        entries.extend(post_pbr_entries(FRAGMENT));
        let fits = wgpu::Limits {
            max_sampled_textures_per_shader_stage: 10,
            max_storage_buffers_per_shader_stage: 2,
            max_uniform_buffers_per_shader_stage: 1,
            max_acceleration_structures_per_shader_stage: 1,
            max_bindings_per_bind_group: 60,
            ..Default::default()
        };
        assert_eq!(over_limits(&entries, &[], &fits, FRAGMENT), None);
        let few = wgpu::Limits {
            max_sampled_textures_per_shader_stage: 9,
            ..fits
        };
        let why = over_limits(&entries, &[], &few, FRAGMENT).unwrap();
        assert_eq!(why, "10 sampled textures, the device takes 9");
    }

    /// The terrain's fragment stage reads terrain.wgsl's group 0 besides the frame's group:
    /// its textures and uniform count toward the same per-stage limits, a binding number of its
    /// own doesn't count toward the group's.
    #[test]
    fn the_terrain_s_own_group_counts_toward_its_fragment_stage_s_limits() {
        let mut entries = scene_entries(FRAGMENT, false, false, false);
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
        assert_eq!(over_limits(&entries, &terrain, &fits, FRAGMENT), None);
        let few = wgpu::Limits {
            max_sampled_textures_per_shader_stage: 17,
            ..fits.clone()
        };
        let why = over_limits(&entries, &terrain, &few, FRAGMENT).unwrap();
        assert_eq!(why, "18 sampled textures, the device takes 17");
        // The scene's group without the terrain's would have fitted.
        assert_eq!(over_limits(&entries, &[], &few, FRAGMENT), None);
        let one = wgpu::Limits {
            max_uniform_buffers_per_shader_stage: 1,
            ..fits
        };
        assert!(
            over_limits(&entries, &terrain, &one, FRAGMENT)
                .unwrap()
                .contains("2 uniform buffers")
        );
    }
}
