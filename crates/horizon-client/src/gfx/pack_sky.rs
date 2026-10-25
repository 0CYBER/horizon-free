//! The vanilla sky's geometry for a shader pack's `gbuffers_skybasic` (spec 2.3): the sky disc,
//! the sunset fan, the stars and the void disc, as Horizon's own sky draws them (terrain.wgsl,
//! stars.rs), built once into a vertex buffer. The frame's colours and rotations sit in a small
//! uniform (set 1) that the vertex inputs read.

use std::ops::Range;

use wgpu::util::DeviceExt;

/// A sky vertex: a position and what it belongs to (`SKY`...).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    pos: [f32; 3],
    kind: u32,
}

const SKY: u32 = 0;
const SUNSET_APEX: u32 = 1;
/// Rim of the sunset fan: `pos.xy` is (sin, cos) of its angle.
const SUNSET_RIM: u32 = 2;
const STAR: u32 = 3;
const VOID: u32 = 4;

/// The frame's sky (std140).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    /// Camera-relative world from star space.
    stars: [[f32; 4]; 4],
    /// Sky colour, alpha 1.
    color: [f32; 4],
    /// Sunset colour and alpha.
    sunrise: [f32; 4],
    /// cos and sin of the sunset fan's turn, star brightness.
    sun: [f32; 4],
}

pub(super) const ATTRIBUTES: [wgpu::VertexAttribute; 2] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Uint32];

/// What `gbuffers_skybasic` reads from these vertices.
pub fn inputs() -> horizon_shaders::GbuffersInputs {
    let declarations =
        "layout(location = 0) in vec3 hz_v_pos;\nlayout(location = 1) in uint hz_v_kind;\n\
         layout(set = 1, binding = 0) uniform HzSky { mat4 hz_sky_stars; vec4 hz_sky_color; \
         vec4 hz_sky_sunrise; vec4 hz_sky_sun; };\n"
            .to_owned();
    let body = format!(
        "    vec3 hz_p = hz_v_pos;\n    \
         hz_Color = hz_sky_color;\n    \
         if (hz_v_kind == {SUNSET_APEX}u || hz_v_kind == {SUNSET_RIM}u) {{\n        \
             vec3 hz_l = vec3(0.0, 100.0, 0.0);\n        \
             hz_Color = hz_sky_sunrise;\n        \
             if (hz_v_kind == {SUNSET_RIM}u) {{\n            \
                 hz_l = vec3(hz_v_pos.xy * 120.0, -hz_v_pos.y * 40.0 * hz_sky_sunrise.a);\n            \
                 hz_Color.a = 0.0;\n        \
             }}\n        \
             vec3 hz_r = vec3(hz_l.x * hz_sky_sun.x - hz_l.y * hz_sky_sun.y, hz_l.x * hz_sky_sun.y + hz_l.y * hz_sky_sun.x, hz_l.z);\n        \
             hz_p = vec3(hz_r.x, -hz_r.z, hz_r.y);\n    \
         }} else if (hz_v_kind == {STAR}u) {{\n        \
             hz_p = (hz_sky_stars * vec4(hz_v_pos, 1.0)).xyz;\n        \
             hz_Color = vec4(hz_sky_sun.z);\n    \
         }} else if (hz_v_kind == {VOID}u) {{\n        \
             hz_Color = vec4(0.0, 0.0, 0.0, 1.0);\n    \
         }}\n    \
         hz_Vertex = vec4(hz_p, 1.0);\n    \
         hz_MultiTexCoord0 = vec4(0.0, 0.0, 0.0, 1.0);\n    \
         hz_MultiTexCoord1 = vec4(0.0, 240.0, 0.0, 1.0);\n    \
         hz_MultiTexCoord2 = hz_MultiTexCoord1;\n    \
         hz_Normal = vec3(0.0, 1.0, 0.0);\n"
    );
    horizon_shaders::GbuffersInputs {
        declarations,
        body,
        attributes: vec![
            ("vaPosition".into(), "hz_Vertex.xyz".into()),
            ("vaColor".into(), "hz_Color".into()),
            ("vaNormal".into(), "hz_Normal".into()),
        ],
        // At the far plane: the sky shows only where nothing else drew, and it writes no depth.
        after: "    gl_Position.z = gl_Position.w;\n".into(),
    }
}

/// A disc fan at height `y`, radius 512, as triangles (SkyRenderer's sky and void discs).
fn disc(y: f32, kind: u32, out: &mut Vec<Vertex>) {
    let sign = y.signum();
    let rim = |i: i32| {
        let a = (i as f32).to_radians();
        Vertex {
            pos: [sign * 512.0 * a.cos(), y, 512.0 * a.sin()],
            kind,
        }
    };
    let centre = Vertex {
        pos: [0.0, y, 0.0],
        kind,
    };
    for i in (-180..180).step_by(45) {
        out.extend([centre, rim(i), rim(i + 45)]);
    }
}

pub(super) struct PackSky {
    vertices: wgpu::Buffer,
    uniform: wgpu::Buffer,
    pub layout: wgpu::BindGroupLayout,
    group: wgpu::BindGroup,
    /// Vertex ranges of the sky disc, the sunset fan, the stars and the void.
    ranges: [Range<u32>; 4],
}

impl PackSky {
    pub fn new(device: &wgpu::Device) -> Self {
        let mut v = Vec::new();
        disc(16.0, SKY, &mut v);
        let sky = 0..v.len() as u32;
        let apex = Vertex {
            pos: [0.0; 3],
            kind: SUNSET_APEX,
        };
        let rim = |i: u32| {
            let a = i as f32 * std::f32::consts::TAU / 16.0;
            Vertex {
                pos: [a.sin(), a.cos(), 0.0],
                kind: SUNSET_RIM,
            }
        };
        for i in 0..16 {
            v.extend([apex, rim(i), rim(i + 1)]);
        }
        let sunset = sky.end..v.len() as u32;
        v.extend(
            super::stars::star_triangles()
                .into_iter()
                .map(|pos| Vertex { pos, kind: STAR }),
        );
        let stars = sunset.end..v.len() as u32;
        disc(-4.0, VOID, &mut v);
        let void = stars.end..v.len() as u32;
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Shader pack sky"),
            contents: bytemuck::cast_slice(&v),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Shader pack sky"),
            size: std::mem::size_of::<Uniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Shader pack sky"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Shader pack sky"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        Self {
            vertices,
            uniform,
            layout,
            group,
            ranges: [sky, sunset, stars, void],
        }
    }

    pub fn buffers() -> [Option<wgpu::VertexBufferLayout<'static>>; 1] {
        [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &ATTRIBUTES,
        })]
    }

    /// Draws the frame's sky, each part with its render stage: the disc, the sunset fan while
    /// the sun is near the horizon, the stars and, with the camera low, the void.
    pub fn draw(
        &self,
        queue: &wgpu::Queue,
        pass: &mut wgpu::RenderPass<'_>,
        frame: &super::Frame,
        stage: &dyn Fn(&mut wgpu::RenderPass<'_>, &str),
    ) {
        let sun_angle = frame.sky.sun_angle.to_radians();
        // The fan turns to the other horizon in the afternoon (terrain.wgsl's sunrise_weight).
        let flip = if sun_angle.sin() < 0.0 { 180f32 } else { 0.0 };
        let theta = (flip + 90.0).to_radians();
        let (rotation, brightness) = frame.stars.unwrap_or_default();
        let [r, g, b] = frame.sky.sky_color;
        let uniform = Uniform {
            stars: rotation.to_cols_array_2d(),
            color: [r, g, b, 1.0],
            sunrise: frame.sky.sunrise_color,
            sun: [theta.cos(), theta.sin(), brightness.clamp(0.0, 1.0), 0.0],
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
        pass.set_bind_group(1, &self.group, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        let [sky, sunset, stars, void] = self.ranges.clone();
        let parts = [
            ("SKY", sky, true),
            ("SUNSET", sunset, frame.sky.sunrise_color[3] > 0.001),
            ("STARS", stars, frame.stars.is_some()),
            ("VOID", void, frame.dark_disc),
        ];
        for (name, range, shown) in parts {
            if shown {
                stage(pass, name);
                pass.draw(range, 0..1);
            }
        }
    }
}
