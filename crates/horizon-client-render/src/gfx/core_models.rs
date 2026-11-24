//! Pack core stages for the native GUI item and ordinary world model paths.
use std::{collections::HashMap, ops::Range};

use crate::core_shader::{CoreProgram, FragmentGlue, Glue, other_binding, translate_core_mapped};
use crate::hud::{HudAtlas, Rect, TextureFlags};
use crate::resource_source::ResourceSource;
use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt as _;

const INPUTS: &[(&str, &str)] = &[
    ("Position", "vec3"),
    ("Color", "vec4"),
    ("UV0", "vec2"),
    ("UV1", "ivec2"),
    ("UV2", "ivec2"),
    ("Normal", "vec3"),
];

/// `GuiItemAtlas.drawToSlot`: the complete tint, posed normal and full-bright light coordinates.
const ITEM: Glue = Glue {
    inputs: INPUTS,
    prelude: "layout(location=0) in vec2 hz_xy;
              layout(location=1) in vec2 hz_uv;
              layout(location=2) in vec4 hz_tint;
              layout(location=3) in float hz_z;
              layout(location=4) in vec2 hz_aux;",
    setup: "Position = vec3(hz_xy, hz_z); Color = hz_tint; UV0 = hz_uv;
            UV1 = ivec2(0, 10); UV2 = ivec2(240);
            int bits = int(hz_aux.x);
            Normal = vec3((ivec3(bits, bits >> 8, bits >> 16) & 255) - 127) / 127.0;",
    // The GUI path sorts faces itself and has no depth attachment.
    finish: "",
    // `HudBatch::quad_3d` emits corners 0, 1, 2, 0, 2, 3.
    vertex_id: "(int(gl_VertexIndex) / 6) * 4 + (int(gl_VertexIndex) % 6 < 3 ? int(gl_VertexIndex) % 6 : (int(gl_VertexIndex) % 6 == 3 ? 0 : int(gl_VertexIndex) % 6 - 2))",
};

/// `ModelFeatureRenderer.prepareModel`: pose and colour before the cardinal shade, and
/// coordinates relative to the actual `RenderType` entity image rather than to our packing.
const ENTITY: Glue = Glue {
    inputs: INPUTS,
    prelude: "layout(location=0) in vec3 hz_pos;
              layout(location=1) in vec2 hz_uv;
              layout(location=2) in uint hz_layer;
              layout(location=3) in vec4 hz_tint;
              layout(location=4) in uvec4 hz_light;
              layout(location=5) in vec4 hz_normal;
              layout(std140,set=0,binding=11) uniform HorizonTextureBounds { vec4 hz_rect; };",
    setup: "Position = hz_pos; Color = hz_tint; Normal = hz_normal.xyz;
            UV0 = (hz_uv - hz_rect.xy) / hz_rect.zw;
            UV2 = ivec2(hz_light.y, hz_light.x);
            uint overlay = hz_layer & 65535u;
            UV1 = overlay == 1u ? ivec2(0, 3) : ivec2(overlay == 5u ? 15 :
                  (overlay >= 16u && overlay < 32u ? int(overlay) - 16 : 0), 10);",
    finish: "",
    vertex_id: "int(gl_VertexIndex)",
};

const WORLD_COLOR: FragmentGlue = FragmentGlue {
    prelude: "layout(std140,set=0,binding=10) uniform HorizonModelFrame { vec4 hz_frame; };",
    // An `Rgba16Float` frame holds at most 65504; neither branch writes a negative colour.
    color: "hz_frame.x != 0.0 ? vec4(min(pow(max(hz_color.rgb, vec3(0.0)), vec3(2.2)), vec3(65504.0)), hz_color.a) : vec4(max(hz_color.rgb, vec3(0.0)), hz_color.a)",
};

#[derive(Default)]
pub struct ModelShaders {
    pub(crate) item: Option<CoreProgram>,
    entity: Option<CoreProgram>,
    entity_cull: Option<CoreProgram>,
}

impl ModelShaders {
    pub(crate) fn load(root: &ResourceSource, defaults: &ResourceSource) -> Self {
        Self {
            item: load(root, defaults, "item", &["ALPHA_CUTOUT 0.1"], &ITEM, None),
            entity: load(
                root,
                defaults,
                "entity",
                &["ALPHA_CUTOUT 0.1", "PER_FACE_LIGHTING"],
                &ENTITY,
                Some(&WORLD_COLOR),
            ),
            entity_cull: load(
                root,
                defaults,
                "entity",
                &["ALPHA_CUTOUT 0.1"],
                &ENTITY,
                Some(&WORLD_COLOR),
            ),
        }
    }
}

/// `ShaderManager.CompilationCache.getOrCompilePipeline`: both winning stages and their
/// imports come from the same generation. A resolved index's omissions are pack filters;
/// reading its underlay for a missing stage or include would undo those filters.
fn load(
    root: &ResourceSource,
    defaults: &ResourceSource,
    name: &str,
    defines: &[&str],
    glue: &Glue,
    output: Option<&FragmentGlue<'_>>,
) -> Option<CoreProgram> {
    let base = root.underlay().unwrap_or(defaults);
    let vertex_path = format!("assets/minecraft/shaders/core/{name}.vsh");
    let fragment_path = format!("assets/minecraft/shaders/core/{name}.fsh");
    let read = |path: &str| {
        root.read_to_string(path).ok().or_else(|| {
            (!root.contains(path) && root.underlay().is_none())
                .then(|| defaults.read_to_string(path).ok())
                .flatten()
        })
    };
    let vertex = read(&vertex_path)?;
    let fragment = read(&fragment_path)?;
    let include = |id: &str| {
        let (ns, path) = id.split_once(':')?;
        read(&format!("assets/{ns}/{path}"))
    };
    // Stock only when both stages match the base with their imports resolved: a pack may
    // replace an include alone.
    let base_include = |id: &str| {
        let (ns, path) = id.split_once(':')?;
        base.read_to_string(&format!("assets/{ns}/{path}")).ok()
    };
    let stock = |path: &str, text: &str| {
        const CORE: &str = "minecraft:shaders/core/";
        let Ok(base_text) = base.read_to_string(path) else {
            return false;
        };
        match (
            crate::core_shader::expand(text, CORE, &include),
            crate::core_shader::expand(&base_text, CORE, &base_include),
        ) {
            (Ok(ours), Ok(theirs)) => ours == theirs,
            _ => false,
        }
    };
    if stock(&vertex_path, &vertex) && stock(&fragment_path, &fragment) {
        return None;
    }
    match translate_core_mapped(
        &vertex,
        &fragment,
        defines,
        &include,
        glue,
        &|n| n == "Lighting",
        output,
    ) {
        Ok(program) => Some(program),
        Err(error) => {
            log::warn!("resource pack core/{name} ({defines:?}) is not used: {error}");
            None
        }
    }
}

const OFFSETS: [u64; 6] = [0, 256, 512, 768, 1024, 1280];
const SIZES: [u64; 6] = [48, 64, 160, 48, 32, 16];
const WORDS: usize = 324;

/// `DynamicUniforms.writeTransform`, `FogRenderer` and `Lighting` as std140 slices.
pub(crate) struct Uniforms([f32; WORDS]);

impl Uniforms {
    pub(crate) fn new(
        globals: [u8; 48],
        projection: Mat4,
        model_view: Mat4,
        fog: [f32; 4],
        distances: [f32; 6],
        lights: [Vec3; 2],
        linear: bool,
    ) -> Self {
        let mut data = [0.0; WORDS];
        bytemuck::cast_slice_mut(&mut data[..12]).copy_from_slice(&globals);
        data[64..80].copy_from_slice(&projection.to_cols_array());
        data[128..144].copy_from_slice(&model_view.to_cols_array());
        data[144..160].copy_from_slice(&Mat4::IDENTITY.to_cols_array());
        data[160..164].fill(1.0);
        data[192..196].copy_from_slice(&fog);
        data[196..202].copy_from_slice(&distances);
        data[256..259].copy_from_slice(&lights[0].to_array());
        data[260..263].copy_from_slice(&lights[1].to_array());
        data[320] = u8::from(linear).into();
        Self(data)
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        bytemuck::cast_slice(&self.0)
    }
}

/// The layout states the size `Resources::group` binds, so a program whose uniform block is
/// larger fails at pipeline creation (and falls back to native) rather than at draw time.
fn uniform_entry(binding: u32, size: u64) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(size),
        },
        count: None,
    }
}

fn material_sampler(device: &wgpu::Device, flags: TextureFlags) -> wgpu::Sampler {
    let address = if flags.clamp {
        wgpu::AddressMode::ClampToEdge
    } else {
        wgpu::AddressMode::Repeat
    };
    let filter = if flags.blur {
        wgpu::FilterMode::Linear
    } else {
        wgpu::FilterMode::Nearest
    };
    device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: address,
        address_mode_v: address,
        min_filter: filter,
        mag_filter: filter,
        ..Default::default()
    })
}

pub(crate) fn texture(
    device: &wgpu::Device,
    label: &str,
    [width, height]: [u32; 2],
) -> wgpu::Texture {
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
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

/// `OverlayTexture`'s neutral and hurt rows are material inputs, not the diffuse texture.
fn overlay(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
    let t = texture(device, "Core model overlay", [16, 16]);
    let pixels: [[u8; 4]; 256] = std::array::from_fn(|i| {
        if i < 128 {
            crate::overlay::HURT_TEXEL
        } else {
            [255, 255, 255, crate::overlay::white_alpha((i % 16) as u32)]
        }
    });
    queue.write_texture(
        t.as_image_copy(),
        bytemuck::cast_slice(&pixels),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(64),
            rows_per_image: None,
        },
        t.size(),
    );
    t.create_view(&Default::default())
}

pub(crate) struct Resources {
    layout: wgpu::BindGroupLayout,
    pub(crate) uniform: wgpu::Buffer,
    overlay: wgpu::TextureView,
    nearest: wgpu::Sampler,
    lighting: Option<u32>,
    world: bool,
}

impl Resources {
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        program: &CoreProgram,
        world: bool,
    ) -> Self {
        let lighting = program
            .other
            .iter()
            .position(|n| n == "Lighting")
            .map(other_binding);
        let mut entries: Vec<_> = (0..4)
            .map(|i| uniform_entry(i, SIZES[i as usize]))
            .chain(lighting.map(|binding| uniform_entry(binding, SIZES[4])))
            .collect();
        if world {
            entries.extend([uniform_entry(10, SIZES[5]), uniform_entry(11, 16)]);
        }
        for (_, binding) in crate::core_shader::SAMPLERS {
            entries.extend([
                wgpu::BindGroupLayoutEntry {
                    binding,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: binding + 1,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ]);
        }
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Core model bindings"),
            entries: &entries,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Core model uniforms"),
            size: if world {
                (WORDS * 4) as u64
            } else {
                GUI_LIGHT_STRIDE * 2
            },
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            layout,
            uniform,
            lighting,
            world,
            overlay: overlay(device, queue),
            nearest: device.create_sampler(&Default::default()),
        }
    }

    pub(crate) fn group(
        &self,
        device: &wgpu::Device,
        diffuse: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        lightmap: (&wgpu::TextureView, &wgpu::Sampler),
        bounds: Option<[f32; 4]>,
        base: u64,
    ) -> wgpu::BindGroup {
        let block = |binding, i: usize| wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &self.uniform,
                offset: base + OFFSETS[i],
                size: wgpu::BufferSize::new(SIZES[i]),
            }),
        };
        let mut entries: Vec<_> = (0..4).map(|i| block(i as u32, i)).collect();
        if let Some(binding) = self.lighting {
            entries.push(block(binding, 4));
        }
        let bounds = bounds.map(|rect| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Core model texture bounds"),
                contents: bytemuck::bytes_of(&rect),
                usage: wgpu::BufferUsages::UNIFORM,
            })
        });
        if self.world {
            entries.push(block(10, 5));
            entries.push(wgpu::BindGroupEntry {
                binding: 11,
                resource: bounds.as_ref().unwrap().as_entire_binding(),
            });
        }
        for ((_, binding), (view, sampler)) in crate::core_shader::SAMPLERS.into_iter().zip([
            (diffuse, sampler),
            (&self.overlay, &self.nearest),
            lightmap,
        ]) {
            entries.push(wgpu::BindGroupEntry {
                binding,
                resource: wgpu::BindingResource::TextureView(view),
            });
            entries.push(wgpu::BindGroupEntry {
                binding: binding + 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            });
        }
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Core model material"),
            layout: &self.layout,
            entries: &entries,
        })
    }

    pub(crate) fn pipeline(
        &self,
        device: &wgpu::Device,
        program: &CoreProgram,
        format: wgpu::TextureFormat,
        buffer: wgpu::VertexBufferLayout<'_>,
        cull: bool,
        coverage: bool,
    ) -> Option<super::hud_coverage::Pipeline> {
        let label = Some("Resource pack core model program");
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = |source: &naga::Module| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label,
                source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(source.clone())),
            })
        };
        let vertex = module(&program.vertex);
        let fragment = module(&program.fragment);
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label,
            bind_group_layouts: &[Some(&self.layout)],
            ..Default::default()
        });
        let desc = wgpu::RenderPipelineDescriptor {
            label,
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &vertex,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                buffers: &[Some(buffer)],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: cull.then_some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: self.world.then_some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: wgpu::DepthBiasState {
                    constant: 128,
                    ..Default::default()
                },
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &fragment,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        };
        let result = super::hud_coverage::Pipeline::new(
            device,
            &desc,
            wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(program.fragment.clone())),
            coverage,
        );
        if let Some(error) = pollster::block_on(scope.pop()) {
            log::warn!("resource pack model pipeline is not used: {error}");
            return None;
        }
        result
    }
}

fn key([u, v]: [f32; 2]) -> [u32; 2] {
    [u.to_bits(), v.to_bits()]
}

struct Variant {
    resources: Resources,
    pipeline: super::hud_coverage::Pipeline,
}

struct Material {
    groups: [Option<wgpu::BindGroup>; 2],
}

pub(super) struct World {
    variants: [Option<Variant>; 2],
    materials: HashMap<[u32; 2], Material>,
    runs: Vec<(Range<u32>, Option<([u32; 2], usize)>)>,
}

fn ordinary(layer: u32) -> bool {
    // Glows, OIT, enchantments and other special pipelines aren't equivalent to ENTITY_CUTOUT.
    layer & !0xffff == 0 && matches!(layer & 65535, 0 | 1 | 5 | 16..=31)
}

impl World {
    pub(super) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        source: &wgpu::Texture,
        atlas: &HudAtlas,
        lightmap: (&wgpu::TextureView, &wgpu::Sampler),
    ) -> Option<Self> {
        let shaders = &atlas.model_shaders;
        let variants = std::array::from_fn(|i| {
            let program = [&shaders.entity, &shaders.entity_cull][i].as_ref()?;
            let resources = Resources::new(device, queue, program, true);
            let pipeline = resources.pipeline(
                device,
                program,
                format,
                wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<super::Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &super::ENTITY_ATTRIBUTES,
                },
                i != 0,
                false,
            )?;
            Some(Variant {
                resources,
                pipeline,
            })
        });
        if variants.iter().all(Option::is_none) {
            return None;
        }
        let oom = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Core entity textures"),
        });
        let mut materials = HashMap::new();
        for (name, &rect) in &atlas.sprites {
            let path = name.split_once(':').map_or(name.as_str(), |(_, p)| p);
            if !(path.starts_with("entity/") || path.starts_with("skins/")) {
                continue;
            }
            let size = (atlas.width, atlas.height);
            let (w, h) = rect.texels(size);
            if w == 0 || h == 0 {
                continue;
            }
            let t = texture(device, "Core entity material", [w, h]);
            copy_sprite(&mut encoder, source, rect, &t, [0, 0]);
            let view = t.create_view(&Default::default());
            let sampler =
                material_sampler(device, atlas.flags.get(name).copied().unwrap_or_default());
            let bounds = [rect.u0, rect.v0, rect.u1 - rect.u0, rect.v1 - rect.v0];
            let groups = variants.each_ref().map(|variant| {
                variant.as_ref().map(|v| {
                    v.resources
                        .group(device, &view, &sampler, lightmap, Some(bounds), 0)
                })
            });
            materials.insert(key([rect.u0, rect.v0]), Material { groups });
        }
        queue.submit([encoder.finish()]);
        if pollster::block_on(oom.pop()).is_some() {
            return None;
        }
        Some(Self {
            variants,
            materials,
            runs: Vec::new(),
        })
    }

    pub(super) fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        uniforms: &Uniforms,
        skin: Option<&super::entity_skin::SkinFrame>,
        posed: u32,
        cpu: &[super::Vertex],
    ) {
        for variant in self.variants.iter().flatten() {
            queue.write_buffer(&variant.resources.uniform, 0, uniforms.bytes());
        }
        self.runs.clear();
        let lookup = |uv, cull: bool| {
            let index = usize::from(cull);
            let id = key(uv);
            self.materials.get(&id)?.groups[index].as_ref()?;
            Some((id, index))
        };
        if let Some(skin) = skin.filter(|_| posed != 0) {
            for job in &skin.jobs {
                let material = (job.mode == super::entity_skin::SkinJob::MODEL
                    && ordinary(job.overlay))
                .then(|| lookup([job.rect[0], job.rect[1]], job.cull != 0))
                .flatten();
                push_run(
                    &mut self.runs,
                    job.dst / 4..(job.dst + job.count) / 4,
                    material,
                );
            }
        }
        for (i, quad) in cpu.as_chunks::<4>().0.iter().enumerate() {
            let v = &quad[0];
            let [u, tagged_v] = v.glint_uv;
            let material = (tagged_v.is_sign_negative()
                && ordinary(v.layer)
                && v.normal[3].unsigned_abs() >= 126)
                .then(|| lookup([u, -tagged_v], v.normal[3] == -126))
                .flatten();
            let q = posed / 4 + i as u32;
            push_run(&mut self.runs, q..q + 1, material);
        }
    }

    pub(super) fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        native: &wgpu::RenderPipeline,
        group: &wgpu::BindGroup,
    ) {
        self.draw_range(pass, native, group, 0..u32::MAX);
    }

    pub(super) fn draw_range(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        native: &wgpu::RenderPipeline,
        group: &wgpu::BindGroup,
        quads: Range<u32>,
    ) {
        for (run, material) in &self.runs {
            let start = run.start.max(quads.start);
            let end = run.end.min(quads.end);
            if start >= end {
                continue;
            }
            match material {
                Some((key, i)) => {
                    pass.set_pipeline(self.variants[*i].as_ref().unwrap().pipeline.get(false));
                    pass.set_bind_group(0, self.materials[key].groups[*i].as_ref().unwrap(), &[]);
                }
                None => {
                    pass.set_pipeline(native);
                    pass.set_bind_group(0, group, &[]);
                }
            }
            pass.draw_indexed(start * 6..end * 6, 0, 0..1);
        }
    }
}

fn push_run<K: Eq>(runs: &mut Vec<(Range<u32>, K)>, range: Range<u32>, key: K) {
    match runs.last_mut() {
        Some((before, current)) if *current == key && before.end == range.start => {
            before.end = range.end
        }
        _ => runs.push((range, key)),
    }
}

pub(crate) fn copy_sprite(
    encoder: &mut wgpu::CommandEncoder,
    source: &wgpu::Texture,
    rect: Rect,
    dest: &wgpu::Texture,
    [x, y]: [u32; 2],
) {
    let size = (source.width(), source.height());
    let (width, height) = rect.texels(size);
    let origin = wgpu::Origin3d {
        x: (rect.u0 * size.0 as f32).round() as u32,
        y: (rect.v0 * size.1 as f32).round() as u32,
        z: 0,
    };
    encoder.copy_texture_to_texture(
        wgpu::TexelCopyTextureInfo {
            origin,
            ..source.as_image_copy()
        },
        wgpu::TexelCopyTextureInfo {
            origin: wgpu::Origin3d { x, y, z: 0 },
            ..dest.as_image_copy()
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
}

const GUI_LIGHT_STRIDE: u64 = 1536;

pub(crate) struct Gui {
    variant: Variant,
    groups: [wgpu::BindGroup; 2],
    lights: [[Vec3; 2]; 2],
    sheet: wgpu::Texture,
    animated: Vec<(Rect, [u32; 2])>,
}

impl Gui {
    /// `Sheets.cutoutItemSheet`: bind the stitched items image, not the GUI texture or an
    /// isolated sprite. Share the current sprite uploads with that sheet for animated icons.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        coverage: bool,
        source: &wgpu::Texture,
        atlas: &HudAtlas,
        layout: &HashMap<String, Rect>,
        lights: [[Vec3; 2]; 2],
        lightmap: (&wgpu::TextureView, &wgpu::Sampler),
    ) -> Option<(Self, HashMap<String, Rect>)> {
        let program = atlas.model_shaders.item.as_ref()?;
        let missing = layout.get("missingno")?;
        let size = [
            (missing.w as f32 / (missing.u1 - missing.u0)).round() as u32,
            (missing.h as f32 / (missing.v1 - missing.v0)).round() as u32,
        ];
        if size.contains(&0) || size.into_iter().max()? > device.limits().max_texture_dimension_2d {
            return None;
        }
        let resources = Resources::new(device, queue, program, false);
        let pipeline = resources.pipeline(device, program, format, wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<crate::hud::HudVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0=>Float32x2,1=>Float32x2,2=>Unorm8x4,3=>Float32,4=>Float32x2],
        }, false, coverage)?;
        let oom = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let sheet = texture(device, "Core item sheet", size);
        let sampler = material_sampler(
            device,
            TextureFlags {
                clamp: true,
                blur: false,
            },
        );
        let view = sheet.create_view(&Default::default());
        let groups = std::array::from_fn(|i| {
            resources.group(
                device,
                &view,
                &sampler,
                lightmap,
                None,
                i as u64 * GUI_LIGHT_STRIDE,
            )
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Copy core item sheet"),
        });
        let mut accepted = HashMap::new();
        let mut animated = Vec::new();
        let mut occupied = std::collections::HashSet::new();
        for (name, &to) in layout {
            let from = if name == "missingno" {
                atlas.sprites.get(crate::hud::MISSING_SPRITE)
            } else {
                atlas.sprites.get(name)
            };
            let Some(&from) = from else {
                continue;
            };
            let dst = [
                (to.u0 * size[0] as f32).round() as u32,
                (to.v0 * size[1] as f32).round() as u32,
            ];
            if (name != "missingno" && to.u0 == missing.u0 && to.v0 == missing.v0)
                || from.texels((atlas.width, atlas.height)) != to.texels((size[0], size[1]))
                || !occupied.insert(dst)
            {
                continue;
            }
            copy_sprite(&mut encoder, source, from, &sheet, dst);
            if atlas
                .sprite_animations
                .iter()
                .any(|(r, _)| r.u0 == from.u0 && r.v0 == from.v0)
            {
                animated.push((from, dst));
            }
            accepted.insert(name.clone(), to);
        }
        queue.submit([encoder.finish()]);
        if pollster::block_on(oom.pop()).is_some() {
            return None;
        }
        Some((
            Self {
                variant: Variant {
                    resources,
                    pipeline,
                },
                groups,
                lights,
                sheet,
                animated,
            },
            accepted,
        ))
    }

    pub(crate) fn animate(&self, encoder: &mut wgpu::CommandEncoder, source: &wgpu::Texture) {
        for &(from, to) in &self.animated {
            copy_sprite(encoder, source, from, &self.sheet, to);
        }
    }

    pub(crate) fn prepare(
        &self,
        uploads: &mut super::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        batch: &crate::hud::HudBatch,
        size: [u32; 2],
        scale: f32,
    ) {
        if !batch
            .blends
            .iter()
            .any(|(_, b)| matches!(b, crate::hud::Blend::ItemModel(_)))
        {
            return;
        }
        let mut globals = [0; 48];
        for (at, value) in [
            (12, batch.glint_alpha),
            (28, batch.game_time),
            (32, size[0] as f32),
            (36, size[1] as f32),
        ] {
            globals[at..at + 4].copy_from_slice(&value.to_ne_bytes());
        }
        let projection = Mat4::orthographic_rh(
            0.0,
            size[0] as f32 / scale,
            size[1] as f32 / scale,
            0.0,
            -1000.0,
            1000.0,
        );
        for (i, lights) in self.lights.into_iter().enumerate() {
            let uniforms = Uniforms::new(
                globals,
                projection,
                Mat4::IDENTITY,
                [0.0; 4],
                [f32::MAX; 6],
                lights,
                false,
            );
            uploads.buffer(
                encoder,
                &self.variant.resources.uniform,
                i as u64 * GUI_LIGHT_STRIDE,
                uniforms.bytes(),
            );
        }
    }

    pub(crate) fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        range: Range<u32>,
        block_light: bool,
        coverage: bool,
    ) {
        pass.set_pipeline(self.variant.pipeline.get(coverage));
        pass.set_bind_group(0, &self.groups[usize::from(block_light)], &[]);
        pass.draw(range, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_assets::resource_source::Resource;
    const VSH: &str = "assets/minecraft/shaders/core/item.vsh";
    const FSH: &str = "assets/minecraft/shaders/core/item.fsh";
    const INCLUDE: &str = "assets/minecraft/shaders/include/tint.glsl";

    fn indexed(entries: &[(&str, &str)]) -> std::collections::BTreeMap<String, Resource> {
        entries
            .iter()
            .map(|(path, text)| ((*path).into(), Resource::bytes(text.as_bytes().to_vec())))
            .collect()
    }

    #[test]
    fn core_model_stages_and_imports_respect_resolved_filters() {
        let vertex = "in vec3 Position;\nvoid main() { gl_Position = vec4(Position, 1.0); }";
        let stock = "out vec4 fragColor;\nvoid main() { fragColor = vec4(1.0); }";
        let pack = "#include <minecraft:tint.glsl>\nout vec4 fragColor;\nvoid main() { fragColor = tint; }";
        let include = "const vec4 tint = vec4(0.2, 0.4, 0.6, 1.0);";
        let base =
            ResourceSource::indexed(indexed(&[(VSH, vertex), (FSH, stock), (INCLUDE, include)]));
        let resolved =
            |entries: &[(&str, &str)]| ResourceSource::layered(indexed(entries), base.clone());
        assert!(ModelShaders::load(&base, &base).item.is_none());
        let complete = resolved(&[(VSH, vertex), (FSH, pack), (INCLUDE, include)]);
        assert!(ModelShaders::load(&complete, &complete).item.is_some());
        let no_vertex = resolved(&[(FSH, pack), (INCLUDE, include)]);
        assert!(ModelShaders::load(&no_vertex, &no_vertex).item.is_none());
        let no_import = resolved(&[(VSH, vertex), (FSH, pack)]);
        assert!(ModelShaders::load(&no_import, &no_import).item.is_none());
        // A pack that replaces only an include the stock stages read still runs.
        let base =
            ResourceSource::indexed(indexed(&[(VSH, vertex), (FSH, pack), (INCLUDE, include)]));
        let tint = "const vec4 tint = vec4(1.0, 0.0, 0.0, 1.0);";
        let include_only = ResourceSource::layered(
            indexed(&[(VSH, vertex), (FSH, pack), (INCLUDE, tint)]),
            base.clone(),
        );
        assert!(ModelShaders::load(&base, &base).item.is_none());
        assert!(
            ModelShaders::load(&include_only, &include_only)
                .item
                .is_some()
        );
    }
}
