//! Horizon's terrain drawn with a Java pack's `gbuffers_terrain` and
//! `gbuffers_water` (docs/shader-packs.md). The pack's vertex program runs
//! after a glue that rebuilds what Iris would feed it from Horizon's
//! `TerrainVertex`: the camera-relative position, the quad's normal and
//! tangent (from its four corners, read from the arena page as storage),
//! the colour without vanilla's directional shade, the lightmap
//! coordinates, the sprite's place in a block atlas laid out from Horizon's
//! sprite arrays, and `mc_Entity` from the pack's `block.properties`, by
//! sprite name.
use horizon_shaderpack::translate::Geometry;
use std::collections::BTreeMap;

/// `TerrainVertex` in 32-bit words.
const VERTEX_WORDS: u32 = 6;

/// The glue's declarations and `hz_vertex`; `HZ_ORIGIN_DECL` and
/// `HZ_ORIGIN` are the variant's (instanced or flat draws).
const GLUE: &str = r#"
layout(location = 0) in vec3 hz_position;
layout(location = 1) in uvec2 hz_uv;
layout(location = 2) in uint hz_layer;
layout(location = 3) in vec4 hz_color;
layout(location = 4) in uvec2 hz_light;
HZ_ORIGIN_DECL
layout(std430, set = 3, binding = 0) readonly buffer HzVertices { uint hz_vertices[]; };
// [0]: atlas columns, [1]: 1 in the Nether, then each sprite layer's block id.
layout(std430, set = 3, binding = 1) readonly buffer HzTable { int hz_table[]; };
vec4 hz_pos;
vec3 hz_normal;
vec4 hz_tangent;
vec4 hz_tint;
vec2 hz_atlas;
vec2 hz_mid;
vec2 hz_lightmap;
float hz_block;
vec3 hz_mid_block;
vec3 hz_corner(uint v) {
    uint b = v * HZ_WORDSu;
    return vec3(uintBitsToFloat(hz_vertices[b]), uintBitsToFloat(hz_vertices[b + 1u]),
        uintBitsToFloat(hz_vertices[b + 2u]));
}
vec2 hz_corner_uv(uint v) {
    uint w = hz_vertices[v * HZ_WORDSu + 3u];
    return vec2(float(w & 65535u), float(w >> 16u));
}
void hz_vertex() {
    vec4 origin = HZ_ORIGIN;
    hz_pos = vec4(hz_position + origin.xyz, 1.0);
    // The quad's corners: quads are four vertices from a multiple of four.
    uint q = uint(gl_VertexIndex) & 4294967292u;
    vec3 p0 = hz_corner(q);
    vec3 e1 = hz_corner(q + 1u) - p0;
    vec3 e2 = hz_corner(q + 2u) - p0;
    vec3 n = cross(e1, e2);
    if (dot(n, n) < 1e-12) {
        n = cross(e2, hz_corner(q + 3u) - p0);
    }
    hz_normal = dot(n, n) > 0.0 ? normalize(n) : vec3(0.0, 1.0, 0.0);
    vec2 t0 = hz_corner_uv(q);
    vec2 d1 = hz_corner_uv(q + 1u) - t0;
    vec2 d2 = hz_corner_uv(q + 2u) - t0;
    float s = d1.x * d2.y - d2.x * d1.y < 0.0 ? -1.0 : 1.0;
    vec3 t = (e1 * d2.y - e2 * d1.y) * s;
    vec3 b = (e2 * d1.x - e1 * d2.x) * s;
    t = t - hz_normal * dot(hz_normal, t);
    t = dot(t, t) > 0.0 ? normalize(t) : vec3(1.0, 0.0, 0.0);
    hz_tangent = vec4(t, dot(cross(hz_normal, t), b) < 0.0 ? -1.0 : 1.0);
    // Iris turns vanilla's directional shade off: undo it on axis faces.
    vec3 a = abs(hz_normal);
    bool nether = hz_table[1] != 0;
    float shade = 1.0;
    if (a.y > 0.999) {
        shade = nether ? 0.9 : (hz_normal.y > 0.0 ? 1.0 : 0.5);
    } else if (a.z > 0.999) {
        shade = 0.8;
    } else if (a.x > 0.999) {
        shade = 0.6;
    }
    hz_tint = vec4(min(hz_color.rgb / shade, vec3(1.0)), hz_color.a);
    // The sprite's cell in the atlas, columns of square cells.
    int layer = int(hz_layer & 16383u);
    int cols = max(hz_table[0], 1);
    vec2 cell = vec2(float(layer % cols), float(layer / cols));
    vec2 uv = clamp(vec2(hz_uv) * origin.w, 0.0, 1.0);
    hz_atlas = (cell + uv) / float(cols);
    hz_mid = (cell + 0.5) / float(cols);
    // Vanilla's UV2: block light, then sky light, 0..240.
    hz_lightmap = vec2(float(hz_light.y), float(hz_light.x));
    hz_block = float(hz_table[2 + layer]);
    hz_mid_block = (floor(hz_position - hz_normal * 0.01) + 0.5 - hz_position) * 64.0;
}
"#;

/// The glue of one draw path: instanced section draws (the origin is an
/// instance attribute) or flat ones (`gfx/flat.rs`: from the block's slot).
pub fn glue(flat: bool, alpha_test: Option<f32>) -> Geometry {
    let (decl, origin) = if flat {
        (
            "layout(std430, set = 1, binding = 0) readonly buffer HzBlockSlots { uint hz_block_slots[]; };\n\
             layout(std430, set = 1, binding = 1) readonly buffer HzSlotOrigins { vec4 hz_slot_origins[]; };",
            "hz_slot_origins[hz_block_slots[uint(gl_VertexIndex) >> 8u]]",
        )
    } else {
        ("layout(location = 5) in vec4 hz_origin;", "hz_origin")
    };
    let vertex_glue = GLUE
        .replace("HZ_ORIGIN_DECL", decl)
        .replace("HZ_ORIGIN", origin)
        .replace("HZ_WORDS", &VERTEX_WORDS.to_string());
    let lightmap = "vec4(hz_lightmap, 0.0, 1.0)";
    let attributes: BTreeMap<String, String> = [
        ("iris_Vertex", "hz_pos"),
        ("vaPosition", "hz_pos"),
        ("iris_Normal", "vec4(hz_normal, 0.0)"),
        ("vaNormal", "vec4(hz_normal, 0.0)"),
        ("iris_Color", "hz_tint"),
        ("vaColor", "hz_tint"),
        ("iris_MultiTexCoord0", "vec4(hz_atlas, 0.0, 1.0)"),
        ("vaUV0", "vec4(hz_atlas, 0.0, 1.0)"),
        ("iris_MultiTexCoord1", lightmap),
        ("iris_MultiTexCoord2", lightmap),
        ("vaUV2", lightmap),
        // OverlayTexture.NO_OVERLAY.
        ("vaUV1", "vec4(0.0, 10.0, 0.0, 0.0)"),
        ("mc_Entity", "vec4(hz_block, -1.0, 0.0, 0.0)"),
        ("mc_midTexCoord", "vec4(hz_mid, 0.0, 1.0)"),
        ("at_tangent", "hz_tangent"),
        ("at_midBlock", "vec4(hz_mid_block, 0.0)"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    Geometry {
        vertex_glue,
        attributes,
        alpha_test,
    }
}

/// The glue's table: atlas columns, the Nether flag, then each sprite
/// layer's block id from `block.properties` (-1 for blocks it doesn't
/// list, as Iris).
pub fn table(sprite_layers: &BTreeMap<String, u32>, block_ids: &BTreeMap<String, i32>) -> Vec<i32> {
    let layers = sprite_layers.values().max().map_or(0, |&m| m as usize + 1);
    let mut out = vec![-1; layers + 2];
    out[0] = atlas_columns(layers as u32) as i32;
    out[1] = 0;
    for (name, &layer) in sprite_layers {
        if let Some(id) = horizon_shaderpack::properties::sprite_block_id(name, block_ids) {
            out[layer as usize + 2] = id;
        }
    }
    out
}

/// The atlas's columns (and rows) of sprite cells for `layers` sprites.
pub fn atlas_columns(layers: u32) -> u32 {
    (f64::from(layers.max(1)).sqrt().ceil() as u32).max(1)
}

/// Whether `device` renders to and filters the 16-bit normalized formats
/// a Java pack's `RGBA16` targets become (`java_stages::target_format`).
pub fn norm16_targets(adapter: &wgpu::Adapter, device: &wgpu::Device) -> bool {
    use wgpu::TextureFormat as F;
    let wanted = wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
        | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
    device.features().contains(wanted)
        && [
            F::R16Unorm,
            F::Rg16Unorm,
            F::Rgba16Unorm,
            F::R16Snorm,
            F::Rg16Snorm,
            F::Rgba16Snorm,
        ]
        .into_iter()
        .all(|f| {
            let features = adapter.get_texture_format_features(f);
            features
                .allowed_usages
                .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
                && features
                    .flags
                    .contains(wgpu::TextureFormatFeatureFlags::FILTERABLE)
        })
}

/// The bind group layout of a page group: the page's vertices and the glue's
/// table, read by the vertex stage.
pub fn page_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let storage = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Java shader pack terrain page"),
        entries: &[storage(0), storage(1)],
    })
}

/// Draws Horizon's terrain with a pack's pipelines: the solid layer or the
/// translucent one. Set by the renderer, which owns the sections.
pub type DrawTerrain<'a> = dyn Fn(&mut wgpu::RenderPass<'_>, bool, &TerrainPipelines<'_>) + 'a;

/// What the renderer binds to draw with a pack's geometry program: the
/// pipeline of its draw path and the pack's group (group 2). Group 1 is the
/// flat path's, group 3 the page's ([`page_layout`]). `flat` is `None` when
/// the renderer has no flat draws.
pub struct TerrainPipelines<'a> {
    pub instanced: &'a wgpu::RenderPipeline,
    pub flat: Option<&'a wgpu::RenderPipeline>,
    pub group: &'a wgpu::BindGroup,
}

/// What a Java pack's geometry reads from the renderer each frame.
pub struct JavaWorld<'a> {
    /// The block sprite arrays, [`super::SPRITE_SPLIT`]-layer each.
    pub sprites: &'a [wgpu::Texture],
    pub split: u32,
    /// Mipmap levels the terrain samples.
    pub levels: u32,
    /// The sprite layers and mip levels animation rewrote this frame.
    pub animated: &'a [(u32, u32)],
    pub lightmap: &'a wgpu::TextureView,
    pub draw: &'a DrawTerrain<'a>,
}

const ATLAS: &str = r#"
struct Atlas { cols: u32, layers: u32, split: u32, pad: u32 }
@group(0) @binding(0) var<uniform> atlas: Atlas;
@group(0) @binding(1) var s0: texture_2d_array<f32>;
@group(0) @binding(2) var s1: texture_2d_array<f32>;
@group(0) @binding(3) var s2: texture_2d_array<f32>;
@group(0) @binding(4) var s3: texture_2d_array<f32>;
@group(0) @binding(5) var s4: texture_2d_array<f32>;
@group(0) @binding(6) var s5: texture_2d_array<f32>;
@group(0) @binding(7) var s6: texture_2d_array<f32>;
@group(0) @binding(8) var s7: texture_2d_array<f32>;

struct Out {
    @builtin(position) p: vec4f,
    @location(0) @interpolate(flat) mip: u32,
}

@vertex
fn vs(@builtin(vertex_index) i: u32, @builtin(instance_index) mip: u32) -> Out {
    let uv = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    return Out(vec4f(uv * 2.0 - 1.0, 0.0, 1.0), mip);
}

// Each sprite layer into its cell, at mip level `mip`.
@fragment
fn fs(in: Out) -> @location(0) vec4f {
    let cell = max(textureDimensions(s0, in.mip).x, 1u);
    let px = vec2u(in.p.xy);
    let c = px / cell;
    let layer = c.y * atlas.cols + c.x;
    if c.x >= atlas.cols || layer >= atlas.layers {
        return vec4f(0.0);
    }
    let at = px % cell;
    let l = i32(layer % atlas.split);
    let m = i32(in.mip);
    switch layer / atlas.split {
        case 0u: { return textureLoad(s0, at, l, m); }
        case 1u: { return textureLoad(s1, at, l, m); }
        case 2u: { return textureLoad(s2, at, l, m); }
        case 3u: { return textureLoad(s3, at, l, m); }
        case 4u: { return textureLoad(s4, at, l, m); }
        case 5u: { return textureLoad(s5, at, l, m); }
        case 6u: { return textureLoad(s6, at, l, m); }
        default: { return textureLoad(s7, at, l, m); }
    }
}
"#;

/// Horizon's block sprites as one 2D atlas, as Iris binds `gtexture`:
/// square cells, [`atlas_columns`] to a row. Drawn from the sprite arrays
/// once, then animated sprites are copied in as they change.
pub struct Atlas {
    texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    mips: Vec<wgpu::TextureView>,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    /// Layers, sprite size, mip levels it was made for.
    key: (u32, u32, u32),
    /// The first sprite array it was drawn from (resource reloads make
    /// new ones).
    source: wgpu::Texture,
    drawn: bool,
}

impl Atlas {
    /// The atlas for `world`'s sprites; `None` without any.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, world: &JavaWorld) -> Option<Self> {
        let key = Self::key(world)?;
        let (layers, size, levels) = key;
        let cols = atlas_columns(layers);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Java shader pack block atlas"),
            size: wgpu::Extent3d {
                width: cols * size,
                height: cols * size,
                depth_or_array_layers: 1,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mips = (0..levels)
            .map(|level| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    base_mip_level: level,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let mut entries = vec![wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }];
        entries.extend((1..=8).map(|binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2Array,
                multisampled: false,
            },
            count: None,
        }));
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Java shader pack block atlas"),
            entries: &entries,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Java shader pack block atlas"),
            source: wgpu::ShaderSource::Wgsl(ATLAS.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Java shader pack block atlas"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Java shader pack block atlas"),
                    bind_group_layouts: &[Some(&layout)],
                    ..Default::default()
                }),
            ),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Java shader pack block atlas"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(
            &uniform,
            0,
            bytemuck::cast_slice(&[cols, layers, world.split.max(1), 0]),
        );
        Some(Self {
            view: texture.create_view(&Default::default()),
            texture,
            mips,
            pipeline,
            layout,
            uniform,
            key,
            source: world.sprites[0].clone(),
            drawn: false,
        })
    }

    fn key(world: &JavaWorld) -> Option<(u32, u32, u32)> {
        let first = world.sprites.first()?;
        let layers = world
            .sprites
            .iter()
            .map(|t| t.depth_or_array_layers())
            .sum();
        let levels = (world.levels + 1).min(first.mip_level_count()).max(1);
        Some((layers, first.width(), levels))
    }

    /// Whether it was made for `world`'s sprites.
    pub fn fits(&self, world: &JavaWorld) -> bool {
        Self::key(world) == Some(self.key) && world.sprites.first() == Some(&self.source)
    }

    /// Bring it up to date: drawn the first time, then the sprites
    /// animation rewrote copied into their cells.
    pub fn update(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        world: &JavaWorld,
    ) {
        if !self.drawn {
            self.draw(device, encoder, world);
            self.drawn = true;
            return;
        }
        let (layers, size, levels) = self.key;
        let cols = atlas_columns(layers);
        let split = world.split.max(1);
        for &(layer, level) in world.animated {
            let Some(array) = world.sprites.get((layer / split) as usize) else {
                continue;
            };
            if level >= levels || layer >= layers {
                continue;
            }
            let edge = (size >> level).max(1);
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: array,
                    mip_level: level,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: layer % split,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &self.texture,
                    mip_level: level,
                    origin: wgpu::Origin3d {
                        x: (layer % cols) * edge,
                        y: (layer / cols) * edge,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width: edge,
                    height: edge,
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    /// The atlas's size in pixels (`atlasSize`).
    pub fn size(&self) -> [u32; 2] {
        [self.texture.width(), self.texture.height()]
    }

    /// Draw the sprites into the atlas, every mip level.
    fn draw(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, world: &JavaWorld) {
        let views: Vec<wgpu::TextureView> = (0..8)
            .map(|i| {
                let t = world.sprites.get(i).unwrap_or(&world.sprites[0]);
                t.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2Array),
                    mip_level_count: Some(self.key.2),
                    ..Default::default()
                })
            })
            .collect();
        let mut entries = vec![wgpu::BindGroupEntry {
            binding: 0,
            resource: self.uniform.as_entire_binding(),
        }];
        entries.extend(views.iter().enumerate().map(|(i, v)| wgpu::BindGroupEntry {
            binding: i as u32 + 1,
            resource: wgpu::BindingResource::TextureView(v),
        }));
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Java shader pack block atlas"),
            layout: &self.layout,
            entries: &entries,
        });
        for (mip, view) in self.mips.iter().enumerate() {
            let attachments = [Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Java shader pack block atlas"),
                color_attachments: &attachments,
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, mip as u32..mip as u32 + 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `module` validates and translates for Metal, Vulkan and DirectX 12.
    fn backends(module: &naga::Module, label: &str) {
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(module)
        .unwrap_or_else(|e| panic!("{label}: {e:?}"));
        naga::back::msl::write_string(
            module,
            &info,
            // wgpu's Metal backend asks for MSL 2.x.
            &naga::back::msl::Options {
                lang_version: (2, 4),
                ..Default::default()
            },
            &naga::back::msl::PipelineOptions::default(),
        )
        .unwrap_or_else(|e| panic!("{label} (MSL): {e}"));
        naga::back::spv::write_vec(module, &info, &Default::default(), None)
            .unwrap_or_else(|e| panic!("{label} (SPIR-V): {e}"));
        for entry in &module.entry_points {
            let mut hlsl = String::new();
            naga::back::hlsl::Writer::new(
                &mut hlsl,
                &naga::back::hlsl::Options::default(),
                &naga::back::hlsl::PipelineOptions {
                    entry_point: Some((entry.stage, entry.name.clone())),
                },
            )
            .write(module, &info, None)
            .unwrap_or_else(|e| panic!("{label} {} (HLSL): {e}", entry.name));
        }
    }

    /// A terrain program reading every attribute the glue fills, run after
    /// each draw path's glue, and the atlas shader, on every backend.
    #[test]
    fn the_glue_feeds_a_terrain_program_on_every_backend() {
        let program = "#version 120
attribute vec4 mc_Entity;
attribute vec4 mc_midTexCoord;
attribute vec4 at_tangent;
attribute vec3 at_midBlock;
varying vec4 color;
varying vec2 uv;
varying vec2 lm;
varying vec3 normal;
varying float id;
void main() {
    vec4 p = gl_ModelViewMatrix * gl_Vertex;
    p.xyz += at_midBlock / 64.0 * 0.0 + at_tangent.xyz * 0.0;
    gl_Position = gl_ProjectionMatrix * p;
    color = gl_Color;
    uv = (gl_TextureMatrix[0] * gl_MultiTexCoord0).xy + mc_midTexCoord.xy * 0.0;
    lm = (gl_TextureMatrix[1] * gl_MultiTexCoord1).xy;
    normal = gl_NormalMatrix * gl_Normal;
    id = mc_Entity.x;
}
";
        for flat in [false, true] {
            let mut link = horizon_shaderpack::translate::Link {
                set: 2,
                geometry: Some(glue(flat, Some(0.1))),
                ..Default::default()
            };
            let t = horizon_shaderpack::translate::translate(
                horizon_shaderpack::Stage::Vertex,
                program,
                &mut link,
                &|l| format!("line {l}"),
            )
            .unwrap_or_else(|e| panic!("flat {flat}: {e}"));
            for wanted in ["hz_vertex();", "mc_Entity = vec4(vec4(hz_block"] {
                assert!(t.glsl.contains(wanted), "{wanted}\n{}", t.glsl);
            }
            // The glue's inputs are Horizon's vertex layout.
            let inputs = t.module.entry_points[0].function.arguments.len();
            assert_eq!(inputs, if flat { 6 } else { 7 }, "{}", t.wgsl);
            let reparsed = naga::front::wgsl::parse_str(&t.wgsl).unwrap();
            backends(&reparsed, &format!("terrain glue (flat {flat})"));
        }
        backends(&naga::front::wgsl::parse_str(ATLAS).unwrap(), "atlas");
        backends(
            &naga::front::wgsl::parse_str(super::super::java_stages::BLIT).unwrap(),
            "Java shader pack blits",
        );
    }

    #[test]
    fn sprites_take_their_blocks_ids() {
        let layers: BTreeMap<String, u32> = [
            ("minecraft:block/oak_leaves", 0),
            ("minecraft:block/water_still", 1),
            ("minecraft:block/stone", 2),
            ("minecraft:block/oak_log_top", 3),
        ]
        .into_iter()
        .map(|(n, l)| (n.to_owned(), l))
        .collect();
        let ids: BTreeMap<String, i32> = [("oak_leaves", 10003), ("water", 8), ("oak_log", 7)]
            .into_iter()
            .map(|(n, i)| (n.to_owned(), i))
            .collect();
        assert_eq!(table(&layers, &ids), [2, 0, 10003, 8, -1, 7]);
        assert_eq!(atlas_columns(5), 3);
        assert_eq!(atlas_columns(0), 1);
    }
}
