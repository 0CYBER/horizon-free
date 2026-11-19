//! Horizon's HDR look as a shader pack (`horizon-shader-api`'s `packs/horizon_hdr`) against the
//! look the renderer draws itself. Each test builds one module with both: the pack's stage in
//! the renderer's wrapper, and the prefixed copy of Horizon's own shader the wrapper carries. Both
//! are handed the same vertex, on every device here (Vulkan, and DirectX 12 with each compiler),
//! and their pixels are compared as the frame stores them: light in vanilla's gamma, so a step
//! of 255 there is one in the SDR picture.

use super::native_entity::{EntityKind, kind_bits};
use crate::mesher::{WHOLE, as_fluid, emitting, giving_off, hdr_tier};
use horizon_shader_api::{Pack, StageKind, check, check::CheckedStage};
use wgpu::util::DeviceExt;

/// The pack's folder, whose look file the renderer's own shaders start with.
fn look_pack() -> Pack {
    let dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../horizon-shader-api/packs/horizon_hdr"
    );
    Pack::read(std::path::Path::new(dir)).unwrap()
}

/// The pack's checked stage of `kind`.
fn look_stage(kind: StageKind) -> CheckedStage {
    let mut report = check(&look_pack()).unwrap();
    let at = report.stages.iter().position(|s| s.kind == kind).unwrap();
    report.stages.swap_remove(at)
}

/// The target's width: 16 texels of 16 bytes, the 256 bytes a copy's rows align to.
const WIDTH: u32 = 16;

/// A display at the scene's peak, where the look's world shows as written: the headroom, then
/// paper white in nits.
const DISPLAY: (f32, f32) = (1000.0 / 203.0, 203.0);

/// What the look tolerates between the two routes, as the frame stores light (vanilla's gamma):
/// a share of the brighter of the two and a floor. The pack's route has more 2.2 powers per
/// pixel (a decode of every input, an encode of the stage's light); measured, they leave the
/// pixels within 4e-7 of each other on the GPUs tried; the tolerance leaves room for others
/// and is still a fortieth of a step of 255 (0.004).
const TOLERANCE: (f32, f32) = (0.0, 1e-4);

/// The renderer's lightmap at sky light `s` and block light `b` (0..15): no light dark but not
/// black, block light warm and sky light cool, white where both are full. Every level differs,
/// so a stage that read the wrong one would show.
fn lightmap() -> [u8; 16 * 16 * 4] {
    let mut out = [255u8; 16 * 16 * 4];
    for s in 0..16 {
        for b in 0..16 {
            let (bl, sl) = ((b as f32 / 15.0).powf(1.5), (s as f32 / 15.0).powf(1.5));
            let at = (s * 16 + b) * 4;
            for (c, (warm, cool)) in [(1.0, 0.8), (0.85, 0.9), (0.7, 1.0)]
                .into_iter()
                .enumerate()
            {
                let v = (0.04 + 0.6 * bl * warm + 0.6 * sl * cool).min(1.0);
                out[at + c] = (v * 255.0).round() as u8;
            }
        }
    }
    out
}

/// A 1-high Rgba8 texture of `layers` layers holding `bytes`, as `dimension`.
fn texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    [width, height, layers]: [u32; 3],
    bytes: &[u8],
    dimension: wgpu::TextureViewDimension,
) -> wgpu::TextureView {
    device
        .create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: layers,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            bytes,
        )
        .create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(dimension),
            ..Default::default()
        })
}

/// The renderer's globals for a world with the look on the float frame. Its fog is black and
/// starts 8 blocks out, past every test's surface but the one put in it.
fn globals() -> super::Globals {
    super::Globals {
        fog_color: [0.0, 0.0, 0.0, 1.0],
        fog: [8.0, 24.0, 8.0, 24.0],
        fog_ends: [24.0, 24.0, 0.0, 0.0],
        filtering: [0.0, 1.0, 0.0, 0.0],
        hdr_light: [0.0, 0.0, 1.0, 1.0],
        // `Hdr::draw_params` with the look on: the headroom, an unused lane, the look, debug.
        hdr_settings: [DISPLAY.0, 0.0, 1.0, 0.0],
        ..bytemuck::Zeroable::zeroed()
    }
}

/// The pack's frame for `globals`, HDR on at `DISPLAY`, the lightmap's full light from
/// `lightmap`, over a target `rows` high.
fn pack_frame(
    queue: &wgpu::Queue,
    frame: &super::native_post::NativeFrame,
    globals: &super::Globals,
    lightmap: &[u8; 16 * 16 * 4],
    rows: u32,
) {
    let inputs = horizon_shader_api::Frame {
        resolution: [WIDTH as f32, rows as f32],
        hdr: 1,
        headroom: DISPLAY.0,
        paper_white: DISPLAY.1,
        peak: DISPLAY.0 * DISPLAY.1,
        fog_color: [
            globals.fog_color[0],
            globals.fog_color[1],
            globals.fog_color[2],
        ],
        fog_environment: [globals.fog[0], globals.fog[1]],
        fog_render: [globals.fog[2], globals.fog[3]],
        full_lightmap: super::native_post::full_lightmap(lightmap),
        ..Default::default()
    };
    queue.write_buffer(&frame.buffer, 0, bytemuck::bytes_of(&inputs));
}

/// One draw of `vertices` with `module`'s `vertex` and `fragment` into a float target `rows`
/// high, as `topology`, its texels read back.
struct Draw<'a> {
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    module: &'a wgpu::ShaderModule,
    layout: &'a wgpu::PipelineLayout,
    groups: &'a [(u32, &'a wgpu::BindGroup)],
    constants: &'a [(&'a str, f64)],
    topology: wgpu::PrimitiveTopology,
    vertices: u32,
    rows: u32,
}

impl Draw<'_> {
    fn texels(&self, vertex: &str, fragment: &str, more: &[(&str, f64)]) -> Vec<[f32; 4]> {
        let constants: Vec<_> = self.constants.iter().chain(more).copied().collect();
        let options = wgpu::PipelineCompilationOptions {
            constants: &constants,
            ..Default::default()
        };
        let format = wgpu::TextureFormat::Rgba32Float;
        let pipeline = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: Some(self.layout),
                vertex: wgpu::VertexState {
                    module: self.module,
                    entry_point: Some(vertex),
                    compilation_options: options.clone(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    topology: self.topology,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: self.module,
                    entry_point: Some(fragment),
                    compilation_options: options,
                    targets: &[Some(format.into())],
                }),
                multiview_mask: None,
                cache: None,
            });
        let target = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: WIDTH,
                height: self.rows,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let read = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(256 * self.rows),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: -1.0,
                        g: -1.0,
                        b: -1.0,
                        a: -1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&pipeline);
        for &(index, group) in self.groups {
            pass.set_bind_group(index, group, &[]);
        }
        pass.draw(0..self.vertices, 0..1);
        drop(pass);
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &read,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: None,
                },
            },
            target.size(),
        );
        self.queue.submit([encoder.finish()]);
        read.slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.unwrap());
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        bytemuck::pod_collect_to_vec(&read.slice(..).get_mapped_range().unwrap())
    }
}

/// The worst difference between `own` and `staged` pixels in the frame's colour channels: the
/// largest absolute one, and the largest share of the brighter of the two, each with its case.
#[derive(Default, Debug)]
struct Worst {
    abs: (f32, String),
    rel: (f32, String),
}

impl Worst {
    fn see(&mut self, own: [f32; 4], staged: [f32; 4], case: impl Fn() -> String) {
        // A NaN difference compares false and would never count as the worst.
        assert!(
            own[..3].iter().chain(&staged[..3]).all(|v| v.is_finite()),
            "a pixel isn't finite: {own:?} {staged:?} ({})",
            case()
        );
        for c in 0..3 {
            let d = (own[c] - staged[c]).abs();
            let r = d / own[c].max(staged[c]).max(1e-6);
            if d > self.abs.0 {
                self.abs = (d, case());
            }
            if r > self.rel.0 && d > TOLERANCE.1 {
                self.rel = (r, case());
            }
        }
    }
}

/// Whether two pixels are the same light within `TOLERANCE`.
fn same_light(own: [f32; 4], staged: [f32; 4]) -> bool {
    (0..3).all(|c| (own[c] - staged[c]).abs() <= TOLERANCE.0 * own[c].max(staged[c]) + TOLERANCE.1)
}

/// A device on `adapter` with every limit it has: the terrain's fragments sample more textures
/// than the default allows.
fn device(adapter: &wgpu::Adapter) -> Option<(wgpu::Device, wgpu::Queue)> {
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .ok()
}

/// The texels of the tests' sprites before lighting, by their brightness (hdr_look.wgsl
/// `brightness`): one past every tier's window, one inside the bright tiers', one before them all.
const TEXELS: [[u8; 3]; 3] = [[250, 240, 200], [150, 120, 60], [70, 50, 40]];

/// `hdr_look.wgsl`'s `brightness` of an 8-bit colour.
fn brightness([r, g, b]: [u8; 3]) -> f32 {
    let l = [r, g, b].map(|c| (f32::from(c) / 255.0).powf(2.2));
    0.5 * (0.2126 * l[0] + 0.7152 * l[1] + 0.0722 * l[2] + l[0].max(l[1]).max(l[2]))
}

/// The brightness a sprite of `TEXELS` must reach where its bright parts shine, as
/// `hdr_stats.wgsl` measures it (`hdr::MASKS.parts` of the way from its darkest opaque texel to
/// its brightest), in the thresholds texture's 8 bits.
fn measured_threshold() -> u8 {
    let ys = TEXELS.map(brightness);
    let (lo, hi) = (
        ys.iter().copied().fold(1.0, f32::min),
        ys.iter().copied().fold(0.0, f32::max),
    );
    let t = lo + (hi - lo) * super::hdr::MASKS.parts;
    (t * 255.0).round() as u8
}

/// The terrain's draw for the material `stage` (the look pack's unless a test has its own), its
/// sprites `sprites` (one texel and one threshold a layer), and `entries` after the module: its
/// group 0, the pack's frame and the module, built for `rows` rows, with the renderer's
/// `globals`.
fn material_module(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    (stage, globals): (&CheckedStage, &super::Globals),
    sprites: &[([u8; 3], u8)],
    entries: &str,
    rows: u32,
) -> (
    wgpu::ShaderModule,
    wgpu::PipelineLayout,
    [wgpu::BindGroup; 2],
    super::native_post::NativeFrame,
) {
    let source = super::native_material::stage_test_shader(stage) + entries;
    let lightmap = lightmap();
    let (terrain, empty) = super::native_pack::tests::terrain_layouts(device);
    let frame = super::native_post::NativeFrame::new(
        device,
        None,
        Vec::new(),
        Default::default(),
        None,
        false,
    );
    pack_frame(queue, &frame, globals, &lightmap, rows);
    let n = sprites.len() as u32;
    let texels: Vec<u8> = sprites
        .iter()
        .flat_map(|(t, _)| [t[0], t[1], t[2], 255])
        .collect();
    let thresholds: Vec<u8> = sprites
        .iter()
        .flat_map(|&(_, t)| [t, 0, 255, 255])
        .collect();
    let array = texture(
        device,
        queue,
        [1, 1, n],
        &texels,
        wgpu::TextureViewDimension::D2Array,
    );
    let plain = wgpu::TextureViewDimension::D2;
    let lightmap_view = texture(device, queue, [16, 16, 1], &lightmap, plain);
    let thresholds_view = texture(device, queue, [n, 1, 1], &thresholds, plain);
    let sampler = device.create_sampler(&Default::default());
    let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::bytes_of(globals),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let resource = |binding: u32| match binding {
        0 => uniform.as_entire_binding(),
        2 | 4 => wgpu::BindingResource::Sampler(&sampler),
        3 => wgpu::BindingResource::TextureView(&lightmap_view),
        12 => wgpu::BindingResource::TextureView(&thresholds_view),
        _ => wgpu::BindingResource::TextureView(&array),
    };
    let entries: Vec<_> = (0..=12)
        .map(|binding| wgpu::BindGroupEntry {
            binding,
            resource: resource(binding),
        })
        .collect();
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &terrain,
        entries: &entries,
    });
    let none = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &empty,
        entries: &[],
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[
            Some(&terrain),
            Some(&empty),
            Some(&empty),
            Some(&frame.layout),
        ],
        ..Default::default()
    });
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    (module, layout, [group, none], frame)
}

/// The terrain's two routes for a test's vertices: `test_corner(i)` gives vertex i's position on
/// the target (clip space), its sprite layer, its vertex colour (alpha the mesher's byte) and its
/// sky and block light (16 a level). Horizon's own runs its terrain vertex and solid fragment
/// with the look on; the pack's runs the material wrapper's vertex and solid fragment.
const TERRAIN_ROUTES: &str = "
struct TestCorner {
    clip: vec4<f32>,
    position: vec3<f32>,
    layer: u32,
    color: vec4<f32>,
    light: vec2<u32>,
}
@vertex fn test_engine_vs(@builtin(vertex_index) i: u32) -> horizon_terrain_TerrainWire {
    let c = test_corner(i);
    var o = horizon_terrain_terrain_vertex(c.position, vec2(0.5), c.layer, c.color, c.light,
        vec3(0.0), 1.0);
    o.clip = c.clip;
    return horizon_terrain_to_wire(o);
}
@fragment fn test_engine_fs(w: horizon_terrain_TerrainWire) -> @location(0) vec4<f32> {
    return horizon_terrain_solid_color(horizon_terrain_from_wire(w));
}
@vertex fn test_pack_vs(@builtin(vertex_index) i: u32) -> HorizonMaterialOut {
    let c = test_corner(i);
    var o = horizon_material_out(c.position, vec2(0.5), c.layer, c.color, vec2<f32>(c.light),
        vec2(0u), 1.0);
    o.clip = c.clip;
    return o;
}
@fragment fn test_pack_fs(in: HorizonMaterialOut) -> @location(0) vec4<f32> {
    return horizon_material_solid(in).color;
}
";

/// Point `i` of a `WIDTH`-wide grid `rows` high, in clip space.
fn grid_point(rows: u32) -> String {
    format!(
        "fn test_point(i: u32) -> vec4<f32> {{
    let at = (vec2(f32(i % {WIDTH}u), f32(i / {WIDTH}u)) + 0.5) / vec2({WIDTH}.0, {rows}.0);
    return vec4(at.x * 2.0 - 1.0, 1.0 - at.y * 2.0, 0.5, 1.0);
}}
"
    )
}

/// `items` as a WGSL array of `ty`.
fn wgsl_array<T>(ty: &str, items: &[T], f: impl Fn(&T) -> String) -> String {
    let items: Vec<String> = items.iter().map(f).collect();
    format!("array<{ty}, {}>({})", items.len(), items.join(", "))
}

/// The pack's material stage draws a block as Horizon's own terrain shader does with the look
/// on: blocks of every tier the mesher gives a block, by their bright parts and all over (a
/// froglight), lava's surface among them; a texel past every tier's window, one inside it and one
/// before it; a sprite threshold of 0, the one `hdr_stats.wgsl` measures for those texels, and one
/// above most of them; a face in full light and one shaded and dim, whose shading and light level
/// a lifted texel leaves.
#[test]
fn the_look_packs_material_stage_draws_horizons_blocks() {
    let _gpu = super::test_gpu::lock();
    let thresholds = [0, measured_threshold(), 200];
    let sprites: Vec<([u8; 3], u8)> = TEXELS
        .iter()
        .flat_map(|&t| thresholds.map(|h| (t, h)))
        .collect();
    // (block, its light level, a fluid's surface).
    let blocks = [
        ("stone", 0, false),
        ("ochre_froglight", 15, false),
        ("torch", 14, false),
        ("glowstone", 15, false),
        ("campfire", 15, false),
        ("redstone_torch", 7, false),
        ("lava", 15, true),
    ];
    // A top face in full light, a side face in an occluded corner at block light 7, and the top
    // face halfway into the fog (black, so lifting before or after it gives the same light).
    let faces = [
        (1.0f32, [240u32, 240], 0.0f32),
        (0.45, [0, 112], 0.0),
        (1.0, [240, 240], 16.0),
    ];
    let mut cases = Vec::new();
    for (layer, _) in sprites.iter().enumerate() {
        for &(name, level, fluid) in &blocks {
            let tier = hdr_tier(name, level);
            let color = giving_off(emitting(0xFF80_8080, tier), level);
            let alpha = (if fluid { as_fluid(color) } else { color }) >> 24;
            for &(shade, light, distance) in &faces {
                cases.push((layer as u32, name, tier, alpha, shade, light, distance));
            }
        }
    }
    let n = cases.len() as u32;
    let rows = n.div_ceil(WIDTH);
    let entries = TERRAIN_ROUTES.to_owned()
        + &grid_point(rows)
        + &format!(
            "fn test_corner(i: u32) -> TestCorner {{
    var layers = {};
    var alphas = {};
    var shades = {};
    var lights = {};
    var distances = {};
    // A biome's tint under the face's shade and ambient occlusion.
    let color = vec4(vec3(0.8, 1.0, 0.7) * shades[i], f32(alphas[i]) / 255.0);
    let position = vec3(0.0, 0.0, -distances[i]);
    return TestCorner(test_point(i), position, layers[i], color, lights[i]);
}}
",
            wgsl_array("u32", &cases, |c| format!("{}u", c.0)),
            wgsl_array("u32", &cases, |c| format!("{}u", c.3)),
            wgsl_array("f32", &cases, |c| format!("{:?}", c.4)),
            wgsl_array("vec2<u32>", &cases, |c| format!(
                "vec2({}u, {}u)",
                c.5[0], c.5[1]
            )),
            wgsl_array("f32", &cases, |c| format!("{:?}", c.6)),
        );
    let mut tested = Vec::new();
    for (adapter, backend) in super::test_gpu::each_adapter() {
        let Some((device, queue)) = device(&adapter) else {
            continue;
        };
        let stage = (&look_stage(StageKind::Material), &globals());
        let (module, layout, [group, none], frame) =
            material_module(&device, &queue, stage, &sprites, &entries, rows);
        let groups = [(0, &group), (1, &none), (2, &none), (3, &frame.group)];
        let draw = Draw {
            device: &device,
            queue: &queue,
            module: &module,
            layout: &layout,
            groups: &groups,
            constants: &[
                ("horizon_terrain_HDR_WRITE_LIGHT", 1.0),
                ("horizon_terrain_HDR_LIGHTING", 1.0),
                (
                    "horizon_terrain_SPRITE_SPLIT",
                    f64::from(sprites.len() as u32),
                ),
            ],
            topology: wgpu::PrimitiveTopology::PointList,
            vertices: n,
            rows,
        };
        let own = draw.texels("test_engine_vs", "test_engine_fs", &[]);
        let staged = draw.texels("test_pack_vs", "test_pack_fs", &[]);
        let mut worst = Worst::default();
        let (mut lifted, mut unshaded, mut gated) = (0, 0, 0);
        for (i, &(layer, name, tier, alpha, shade, light, distance)) in cases.iter().enumerate() {
            let (a, b) = (own[i], staged[i]);
            let (texel, threshold) = sprites[layer as usize];
            let case = || {
                format!(
                    "{backend}: {name} (alpha {alpha:#04x}), texel {texel:?}, threshold \
                     {threshold}, shade {shade}, light {light:?}, {distance} blocks out"
                )
            };
            worst.see(a, b, case);
            assert!(same_light(a, b), "{}: {b:?} against {a:?}", case());
            // The cases aren't all a lit surface's: a light source lifted past white, on a
            // shaded dim face too, and one its sprite's threshold keeps from shining.
            let past_white = a[0].max(a[1]).max(a[2]) > 1.0;
            lifted += u32::from(past_white);
            unshaded += u32::from(past_white && shade < 1.0);
            let parts = tier != 0 && tier & WHOLE == 0;
            gated += u32::from(parts && brightness(texel) < f32::from(threshold) / 255.0);
        }
        assert!(
            lifted > 20 && unshaded > 10 && gated > 10,
            "{backend}: {lifted} lifted, {unshaded} on a shaded face, {gated} gated"
        );
        eprintln!("{backend}: material stage, {n} cases, worst {worst:?}");
        tested.push(backend);
    }
    eprintln!("the look pack's material stage ran on {tested:?}");
}

/// Where the vertex colour and the light level both change across a face, Horizon's terrain
/// smooths their product, and so does the pack, which reads it as `lit_tint`: an occluded corner
/// (vertex colour 0.2, about vanilla's darkest ambient occlusion) beside a torch (block light
/// 14), its far corners open at block light 11, under this file's lightmap, over a whole face of
/// plain stone, is Horizon's within `TOLERANCE`. Multiplying `shaded_tint` by `lightmap`, each
/// smoothed apart, left it 6.6 steps of 255 off, midway along the edges from the dark corner.
#[test]
fn an_occluded_corner_beside_a_torch_is_horizons() {
    let _gpu = super::test_gpu::lock();
    let rows = 16;
    let stone = giving_off(emitting(0xFF80_8080, 0), 0) >> 24;
    let entries = TERRAIN_ROUTES.to_owned()
        + &format!(
            "fn test_corner(i: u32) -> TestCorner {{
    // Two triangles of a face over the target: the corner at its bottom left occluded and by
    // the torch.
    var at = array<vec2<f32>, 6>(vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(-1.0, 1.0),
        vec2(-1.0, 1.0), vec2(1.0, -1.0), vec2(1.0, 1.0));
    var shades = array<f32, 6>(0.2, 1.0, 1.0, 1.0, 1.0, 1.0);
    var blocks = array<u32, 6>(224u, 176u, 176u, 176u, 176u, 176u);
    let color = vec4(vec3(shades[i]), f32({stone}u) / 255.0);
    return TestCorner(vec4(at[i], 0.5, 1.0), vec3(0.0), 0u, color, vec2(0u, blocks[i]));
}}
"
        );
    let sprites = [([200, 200, 200], 0)];
    let mut tested = Vec::new();
    for (adapter, backend) in super::test_gpu::each_adapter() {
        let Some((device, queue)) = device(&adapter) else {
            continue;
        };
        let stage = (&look_stage(StageKind::Material), &globals());
        let (module, layout, [group, none], frame) =
            material_module(&device, &queue, stage, &sprites, &entries, rows);
        let groups = [(0, &group), (1, &none), (2, &none), (3, &frame.group)];
        let draw = Draw {
            device: &device,
            queue: &queue,
            module: &module,
            layout: &layout,
            groups: &groups,
            constants: &[
                ("horizon_terrain_HDR_WRITE_LIGHT", 1.0),
                ("horizon_terrain_HDR_LIGHTING", 1.0),
                ("horizon_terrain_SPRITE_SPLIT", 1.0),
            ],
            topology: wgpu::PrimitiveTopology::TriangleList,
            vertices: 6,
            rows,
        };
        let own = draw.texels("test_engine_vs", "test_engine_fs", &[]);
        let staged = draw.texels("test_pack_vs", "test_pack_fs", &[]);
        let mut worst = Worst::default();
        for (i, (&a, &b)) in own.iter().zip(&staged).enumerate() {
            let (x, y) = (i as u32 % WIDTH, i as u32 / WIDTH);
            assert!(
                a[3] >= 0.0 && b[3] >= 0.0,
                "{backend}: ({x}, {y}) not drawn"
            );
            worst.see(a, b, || {
                format!("{backend}: ({x}, {y}), {b:?} against {a:?}")
            });
            assert!(
                same_light(a, b),
                "{backend}: ({x}, {y}), {b:?} against {a:?}"
            );
        }
        let steps = worst.abs.0 * 255.0;
        eprintln!(
            "{backend}: occluded corner by a torch, worst {steps:.5} steps of 255: {worst:?}"
        );
        tested.push(backend);
    }
    eprintln!("the occluded corner ran on {tested:?}");
}

/// `MaterialInput.lit_tint` is the colour Horizon's terrain smooths over a face (terrain.wgsl's
/// `terrain_vertex` `color`, its vertex colour lit at each corner, interpolated): on a face of
/// near terrain whose corners differ in ambient occlusion and light, on water's surface (a fluid
/// quad's colour byte), and on a far tile as the LOD mesher emits it, each with a dynamic light
/// and a coloured light reaching across it unevenly, so the product changes across every face.
/// Read through a stage that returns it, as the entries decode it and encode the result.
#[test]
fn lit_tint_is_the_colour_horizon_smooths_over_a_face() {
    let _gpu = super::test_gpu::lock();
    let rgba = |[r, g, b]: [f32; 3], alpha: u32| {
        let byte = |c: f32| (c * 255.0).round() as u32;
        byte(r) | byte(g) << 8 | byte(b) << 16 | alpha << 24
    };
    let plain = giving_off(emitting(0xFF80_8080, 0), 0) >> 24;
    let tinted = |tint: [f32; 3], shades: [f32; 4], alpha: u32| {
        shades.map(|s| rgba(tint.map(|c| c * s), alpha))
    };
    let far = crate::lod::far_top_vertex((0, 0), 0);
    let far_light = [u32::from(far.light & 0xff), u32::from(far.light >> 8)];
    // (what, its four corners' colours and lights: sky, block, 16 a level).
    let cases: [(&str, [u32; 4], [[u32; 2]; 4]); 3] = [
        (
            "near terrain",
            tinted([0.5, 0.8, 0.4], [0.2, 0.6, 1.0, 0.8], plain),
            [[240, 224], [160, 176], [64, 0], [240, 96]],
        ),
        (
            "water's surface",
            tinted(
                [0.25, 0.46, 0.89],
                [1.0, 0.9, 0.8, 1.0],
                as_fluid(plain << 24) >> 24,
            ),
            [[240, 0], [200, 48], [120, 160], [240, 240]],
        ),
        ("a far tile", [far.color; 4], [far_light; 4]),
    ];
    let rows = 3 * 4;
    // Each case two triangles of a quad over its band of rows, its corners 0 to 3 from the
    // bottom left, counterclockwise, 4 blocks by 2 in front of the camera.
    let corners = [0usize, 1, 2, 0, 2, 3];
    let list = |f: &dyn Fn(&(&str, [u32; 4], [[u32; 2]; 4]), usize) -> String| {
        let items: Vec<String> = (cases.iter())
            .flat_map(|c| corners.map(|k| f(c, k)))
            .collect();
        items.join(", ")
    };
    let n = 6 * cases.len();
    let entries = TERRAIN_ROUTES.to_owned()
        + &format!(
            "fn test_corner(i: u32) -> TestCorner {{
    var colors = array<u32, {n}>({colors});
    var lights = array<vec2<u32>, {n}>({lights});
    var corners = array<u32, 6>(0u, 1u, 2u, 0u, 2u, 3u);
    let k = corners[i % 6u];
    let band = f32(i / 6u);
    let x = select(-1.0, 1.0, k == 1u || k == 2u);
    let y = 1.0 - 2.0 * (band + select(1.0, 0.0, k >= 2u)) / {cases}.0;
    let position = vec3(select(0.0, 4.0, x > 0.0), select(0.0, 2.0, k >= 2u), -2.0);
    return TestCorner(vec4(x, y, 0.5, 1.0), position, 0u, unpack4x8unorm(colors[i]), lights[i]);
}}
@fragment fn test_engine_lit(w: horizon_terrain_TerrainWire) -> @location(0) vec4<f32> {{
    return vec4(horizon_terrain_from_wire(w).color.rgb, 1.0);
}}
@fragment fn test_pack_lit(in: HorizonMaterialOut) -> @location(0) vec4<f32> {{
    return horizon_material_color(in, false);
}}
",
            cases = cases.len(),
            colors = list(&|c, k| format!("{}u", c.1[k])),
            lights = list(&|c, k| format!("vec2({}u, {}u)", c.2[k][0], c.2[k][1])),
        );
    let manifest = "schema = 1\nid = \"t\"\nname = \"T\"\nversion = \"1\"\napi = \"0.2\"\n\
        [material]\nfile = \"material.wgsl\"\n";
    let material = "fn material(input: MaterialInput) -> Surface {\n    \
        return Surface(vec4<f32>(input.lit_tint, 1.0));\n}\n";
    let files = [
        (horizon_shader_api::MANIFEST, manifest),
        ("material.wgsl", material),
    ]
    .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
    .into();
    let mut report = check(&Pack::from_files(files).unwrap()).unwrap();
    let stage = report.stages.remove(0);
    assert!(stage.lit_tint());
    // A dynamic light at the face's bottom left, level 12, and a warm coloured light reaching
    // its top right.
    let mut globals = globals();
    globals.time[2] = 1.0;
    globals.dynamic_lights[0] = [0.0, 0.0, -2.0, 12.0];
    globals.colored_count[0] = 1;
    globals.colored_lights[0] = [[4.0, 2.0, -2.0, 4.0], [1.0, 0.6, 0.2, 0.9]];
    let sprites = [([200, 200, 200], 0)];
    let mut tested = Vec::new();
    for (adapter, backend) in super::test_gpu::each_adapter() {
        let Some((device, queue)) = device(&adapter) else {
            continue;
        };
        let (module, layout, [group, none], frame) = material_module(
            &device,
            &queue,
            (&stage, &globals),
            &sprites,
            &entries,
            rows,
        );
        let groups = [(0, &group), (1, &none), (2, &none), (3, &frame.group)];
        let draw = Draw {
            device: &device,
            queue: &queue,
            module: &module,
            layout: &layout,
            groups: &groups,
            constants: &[("horizon_terrain_SPRITE_SPLIT", 1.0)],
            topology: wgpu::PrimitiveTopology::TriangleList,
            vertices: n as u32,
            rows,
        };
        let own = draw.texels("test_engine_vs", "test_engine_lit", &[]);
        let staged = draw.texels("test_pack_vs", "test_pack_lit", &[]);
        let mut worst = Worst::default();
        let mut spread = vec![(f32::MAX, 0.0f32); cases.len()];
        for (i, (&a, &b)) in own.iter().zip(&staged).enumerate() {
            let (x, y) = (i as u32 % WIDTH, i as u32 / WIDTH);
            let (what, ..) = cases[(y / 4) as usize];
            assert!(
                a[3] >= 0.0 && b[3] >= 0.0,
                "{backend}: {what} ({x}, {y}) not drawn"
            );
            worst.see(a, b, || {
                format!("{backend}: {what} ({x}, {y}), {b:?} against {a:?}")
            });
            assert!(
                (0..3).all(|c| (a[c] - b[c]).abs() <= 1e-5),
                "{backend}: {what} ({x}, {y}), {b:?} against {a:?}"
            );
            let s = &mut spread[(y / 4) as usize];
            let sum = a[0] + a[1] + a[2];
            *s = (s.0.min(sum), s.1.max(sum));
        }
        // The lights change the product across every face, the far tile's too.
        for (s, (what, ..)) in spread.iter().zip(&cases) {
            assert!(s.1 - s.0 > 0.05, "{backend}: {what} is flat: {s:?}");
        }
        eprintln!("{backend}: lit_tint, worst {worst:?}");
        tested.push(backend);
    }
    eprintln!("lit_tint ran on {tested:?}");
}

/// The pack's entity stage draws an entity as Horizon's own entity shader does with the look
/// on: every tier, a texel past every tier's window, one inside and one before them, a whole
/// layer and one fading out, a face toward the light and one shaded, in full light and dim.
#[test]
fn the_look_packs_entity_stage_draws_horizons_entities() {
    let _gpu = super::test_gpu::lock();
    let mut cases = Vec::new();
    for texel in 0..TEXELS.len() as u32 {
        for tier in 0..=4u32 {
            for layer_alpha in [1.0f32, 0.5] {
                for normal in [[0.0f32, 1.0, 0.0], [1.0, 0.0, 0.0]] {
                    for light in [[240u32, 240], [0, 96]] {
                        cases.push((texel, tier, layer_alpha, normal, light));
                    }
                }
            }
        }
    }
    let n = cases.len() as u32;
    let rows = n.div_ceil(WIDTH);
    let source = {
        let stage = look_stage(StageKind::Entity);
        super::native_entity::shader_source(&stage.source, horizon_shader_api::Fog::Engine, true)
    };
    let source = source
        + &grid_point(rows)
        + &format!(
            "override test_kind: u32 = 0u;
@vertex fn test_vs(@builtin(vertex_index) i: u32) -> horizon_entity_out {{
    var texels = {};
    var tiers = {};
    var alphas = {};
    var normals = {};
    var lights = {};
    let layer = (tiers[i] << 16u) | test_kind;
    let uv = vec2((f32(texels[i]) + 0.5) / 4.0, 0.5);
    let light = vec4(lights[i], 0u, 0u);
    var out = horizon_entity_vertex(vec3(0.0, 0.0, -2.0), uv, layer,
        vec4(0.9, 0.8, 1.0, alphas[i]), light, vec4(normals[i], 1.0), vec2(0.0),
        (layer >> horizon_entity_kind_shift) & 7u);
    out.clip = test_point(i);
    return out;
}}
",
            wgsl_array("u32", &cases, |c| format!("{}u", c.0)),
            wgsl_array("u32", &cases, |c| format!("{}u", c.1)),
            wgsl_array("f32", &cases, |c| format!("{:?}", c.2)),
            wgsl_array("vec3<f32>", &cases, |c| format!(
                "vec3({:?}, {:?}, {:?})",
                c.3[0], c.3[1], c.3[2]
            )),
            wgsl_array("vec2<u32>", &cases, |c| format!(
                "vec2({}u, {}u)",
                c.4[0], c.4[1]
            )),
        );
    let lightmap = lightmap();
    let globals = globals();
    let mut tested = Vec::new();
    for (adapter, backend) in super::test_gpu::each_adapter() {
        let Some((device, queue)) = device(&adapter) else {
            continue;
        };
        let entities = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &super::native_entity::entity_entries(),
        });
        let frame = super::native_post::NativeFrame::new(
            &device,
            None,
            Vec::new(),
            Default::default(),
            None,
            false,
        );
        pack_frame(&queue, &frame, &globals, &lightmap, rows);
        let mut atlas: Vec<u8> = TEXELS
            .iter()
            .flat_map(|t| [t[0], t[1], t[2], 255])
            .collect();
        atlas.extend([0, 0, 0, 0]);
        let plain = wgpu::TextureViewDimension::D2;
        let atlas = texture(&device, &queue, [4, 1, 1], &atlas, plain);
        let lightmap_view = texture(&device, &queue, [16, 16, 1], &lightmap, plain);
        let sampler = device.create_sampler(&Default::default());
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::bytes_of(&globals),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let entries: Vec<_> = (0..=7)
            .map(|binding| wgpu::BindGroupEntry {
                binding,
                resource: match binding {
                    0 => uniform.as_entire_binding(),
                    2 | 4 | 6 => wgpu::BindingResource::Sampler(&sampler),
                    3 => wgpu::BindingResource::TextureView(&lightmap_view),
                    _ => wgpu::BindingResource::TextureView(&atlas),
                },
            })
            .collect();
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &entities,
            entries: &entries,
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&entities), None, None, Some(&frame.layout)],
            ..Default::default()
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(source.as_str().into()),
        });
        let groups = [(0, &group), (3, &frame.group)];
        let draw = Draw {
            device: &device,
            queue: &queue,
            module: &module,
            layout: &layout,
            groups: &groups,
            constants: &[("horizon_vanilla_HDR_WRITE_LIGHT", 1.0)],
            topology: wgpu::PrimitiveTopology::PointList,
            vertices: n,
            rows,
        };
        // Horizon's own draw (kind 0), then the stage's (an entity).
        let own = draw.texels("test_vs", "horizon_entity_fs", &[("test_kind", 0.0)]);
        let staged = draw.texels(
            "test_vs",
            "horizon_entity_fs",
            &[("test_kind", f64::from(kind_bits(EntityKind::Entity)))],
        );
        let mut worst = Worst::default();
        let mut lifted = 0;
        for (i, &(texel, tier, alpha, normal, light)) in cases.iter().enumerate() {
            let (a, b) = (own[i], staged[i]);
            let case = || {
                format!(
                    "{backend}: texel {:?}, tier {tier}, layer alpha {alpha}, normal \
                     {normal:?}, light {light:?}",
                    TEXELS[texel as usize]
                )
            };
            worst.see(a, b, case);
            assert!(
                same_light(a, b) && (a[3] - b[3]).abs() < 1e-3,
                "{}: {b:?} against {a:?}",
                case()
            );
            lifted += u32::from(a[0].max(a[1]).max(a[2]) > 1.0);
        }
        assert!(lifted >= 6, "{backend}: {lifted} lifted");
        eprintln!("{backend}: entity stage, {n} cases, worst {worst:?}");
        tested.push(backend);
    }
    eprintln!("the look pack's entity stage ran on {tested:?}");
}

/// The pack's tone curve builds where the renderer builds a pack's `[tonemap]` (`NativePost`):
/// the look file's names, which the pack shares unprefixed, meet nothing the renderer adds.
#[test]
fn the_look_packs_tone_curve_builds_in_the_renderer() {
    let _gpu = super::test_gpu::lock();
    let report = check(&look_pack()).unwrap();
    let tonemap = report.stages.iter().find(|s| s.kind == StageKind::Tonemap);
    assert!(tonemap.is_some());
    let mut tested = Vec::new();
    for (adapter, backend) in super::test_gpu::each_adapter() {
        let Some((device, _queue)) = device(&adapter) else {
            continue;
        };
        let frame = super::native_post::NativeFrame::new(
            &device,
            None,
            Vec::new(),
            Default::default(),
            None,
            false,
        );
        let format = wgpu::TextureFormat::Rgba16Float;
        let built = super::native_post::NativePost::new(
            &device,
            Vec::new(),
            &report.manifest,
            &[],
            format,
            &frame,
            tonemap,
        );
        assert!(built.is_ok(), "{backend}: {:?}", built.err());
        tested.push(backend);
    }
    eprintln!("the look pack's tone curve built on {tested:?}");
}
