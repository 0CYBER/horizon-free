//! HDR output (Horizon setting, off by default): the world is drawn as light into a float
//! frame, in vanilla's gamma space, the HUD on a layer of its own, and the last pass
//! (`hdr.wgsl`) writes the picture to an HDR10 or scRGB swapchain at the player's paper white.
//! Under a `shaders/` pack the frame is the pack's 8-bit SDR picture, shown at paper white.
//!
//! On the float frame Horizon's world light is the same on every display (hdr_look.wgsl: its
//! tiers, up to its scene peak), and the tone curve alone fits it to the display's peak
//! (docs/design/hdr-linear-light.md, step 6a). Lit surfaces get no gain: diffuse white stays at
//! paper white, and HDR's range above it is the light sources' and the sun's. The sun's disc
//! alone follows the display past the scene's peak (`HDR_SUN_CAP`).

/// The swapchain HDR output presents to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Output {
    /// HDR10: 10-bit BT.2020 with the PQ curve, the display's own signal.
    Pq,
    /// scRGB: linear 16-bit float, BT.709 primaries, 1.0 = 80 nits (on Wayland, mostly the
    /// compositor's reference white: `surface_scale`).
    ScRgb,
}

/// Horizon's nits in the surface's for `output`: 1, but 80 / 203 for scRGB on a surface whose
/// reference white is 80 nits (`reference_80`), which the compositor shows at its own white,
/// Horizon's 203 nits (`gfx::reported`). The HDR metadata's nits are on that scale too.
pub fn surface_scale(output: Output, reference_80: bool) -> f32 {
    if output == Output::ScRgb && reference_80 {
        80.0 / 203.0
    } else {
        1.0
    }
}

/// Whether the Vulkan driver (the adapter's PCI `vendor` and `driver` name) leaves a Wayland scRGB
/// swapchain at wp_color_manager_v1's default reference white, 80 nits. Mesa (NVK included)
/// describes it as sRGB primaries with the linear transfer function and sets no luminances, so the
/// protocol's default reference of 80 applies; NVIDIA's own driver declares luminances 0..80 with
/// reference 203, which is Horizon's scale already.
pub fn scrgb_reference_80(vendor: u32, driver: &str) -> bool {
    !(vendor == 0x10de && driver == "NVIDIA")
}

/// The CIE 1931 xy chromaticities of a red, green and blue primary.
pub type Primaries = [[f32; 2]; 3];
/// A linear RGB to RGB matrix, columns first (WGSL's `mat3x3<f32>`): column `i` is where
/// channel `i` alone goes.
pub type Matrix = [[f32; 3]; 3];
const BT709: Primaries = [[0.640, 0.330], [0.300, 0.600], [0.150, 0.060]];
const BT2020: Primaries = [[0.708, 0.292], [0.170, 0.797], [0.131, 0.046]];
const DISPLAY_P3: Primaries = [[0.680, 0.320], [0.265, 0.690], [0.150, 0.060]];
/// Every colour here keeps D65 as white, whatever white point the display reports.
const D65: [f64; 2] = [0.3127, 0.3290];

/// Reported primaries as the display reading keeps them: `None` for a coordinate that isn't a
/// number, which would also never compare equal to the next reading.
pub(super) fn reported_primaries(p: Primaries) -> Option<Primaries> {
    p.as_flattened().iter().all(|c| c.is_finite()).then_some(p)
}

/// The primaries Color Boost stretches toward on a display that `reported` its own: those, or
/// Display P3 where none were reported, all are zero, or they don't hold BT.709's (nothing
/// would be stretched toward the display then, only away from it). Each one outside BT.2020
/// is moved onto BT.2020's edge, so the stretch never goes past it: Display P3's red too,
/// which lies a hair outside.
pub fn boost_target(reported: Option<Primaries>) -> Primaries {
    let clamp = |p: Primaries| p.map(|p| onto_triangle(p, BT2020));
    reported
        .filter(|p| p.as_flattened().iter().any(|&c| c != 0.0))
        .map(clamp)
        .filter(|clamped| BT709.iter().all(|&p| inside(p, *clamped, 2e-3)))
        .unwrap_or_else(|| clamp(DISPLAY_P3))
}

/// Whether `p` lies in `triangle`, or within `slack` of it (a quantised report of Display P3
/// holds BT.709's blue a hair outside). A triangle with no area holds nothing.
fn inside(p: [f32; 2], triangle: Primaries, slack: f32) -> bool {
    let cross = |a: [f32; 2], b: [f32; 2], p: [f32; 2]| {
        (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
    };
    let area = cross(triangle[0], triangle[1], triangle[2]);
    if area.abs() < 1e-6 {
        return false;
    }
    (0..3).all(|i| {
        let (a, b) = (triangle[i], triangle[(i + 1) % 3]);
        let length = (b[0] - a[0]).hypot(b[1] - a[1]);
        cross(a, b, p) * area.signum() / length >= -slack
    })
}

/// `p`, or outside `triangle` the nearest point on its edges.
fn onto_triangle(p: [f32; 2], triangle: Primaries) -> [f32; 2] {
    if inside(p, triangle, 0.0) {
        return p;
    }
    let nearest = |a: [f32; 2], b: [f32; 2]| {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let t = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
        [a[0] + t * dx, a[1] + t * dy]
    };
    (0..3)
        .map(|i| nearest(triangle[i], triangle[(i + 1) % 3]))
        .min_by(|a, b| {
            let d = |q: &[f32; 2]| (q[0] - p[0]).hypot(q[1] - p[1]);
            d(a).total_cmp(&d(b))
        })
        .unwrap_or(p)
}

/// HDR output's linear BT.709 to BT.2020 with Color Boost `boost` (0..=1) toward `target`
/// (`boost_target`): `None` at 0, where the shader keeps its exact constant.
pub fn gamut(boost: f32, target: Primaries) -> Option<Matrix> {
    (boost > 0.0).then(|| stretched_to_bt2020(boost.min(1.0), target))
}

/// Linear RGB in BT.709's primaries, each moved `boost` of the way toward `target`'s, D65
/// white kept, to BT.2020: RGB to XYZ by those primaries, then XYZ to BT.2020.
fn stretched_to_bt2020(boost: f32, target: Primaries) -> Matrix {
    let s = f64::from(boost);
    let stretched: [[f64; 2]; 3] = std::array::from_fn(|i| {
        std::array::from_fn(|c| {
            let from = f64::from(BT709[i][c]);
            from + s * (f64::from(target[i][c]) - from)
        })
    });
    let bt2020 = BT2020.map(|p| p.map(f64::from));
    let m = mul(inverse(rgb_to_xyz(bt2020)), rgb_to_xyz(stretched));
    // Rows here; the shader takes columns.
    std::array::from_fn(|col| std::array::from_fn(|row| m[row][col] as f32))
}

/// The RGB to XYZ matrix (rows) of `primaries` with D65 white: each primary's XYZ at Y = 1,
/// scaled so that RGB 1, 1, 1 is D65 at Y = 1.
fn rgb_to_xyz(primaries: [[f64; 2]; 3]) -> [[f64; 3]; 3] {
    let xyz = |[x, y]: [f64; 2]| [x / y, 1.0, (1.0 - x - y) / y];
    let columns = primaries.map(xyz);
    let p: [[f64; 3]; 3] = std::array::from_fn(|r| std::array::from_fn(|c| columns[c][r]));
    let white = xyz(D65);
    let inv = inverse(p);
    let scale: [f64; 3] =
        std::array::from_fn(|r| (0..3).map(|c| inv[r][c] * white[c]).sum::<f64>());
    std::array::from_fn(|r| std::array::from_fn(|c| p[r][c] * scale[c]))
}

fn mul(a: [[f64; 3]; 3], b: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    std::array::from_fn(|r| std::array::from_fn(|c| (0..3).map(|k| a[r][k] * b[k][c]).sum()))
}

fn inverse(m: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let cofactor = |r: usize, c: usize| {
        let (r1, r2) = ((r + 1) % 3, (r + 2) % 3);
        let (c1, c2) = ((c + 1) % 3, (c + 2) % 3);
        m[r1][c1] * m[r2][c2] - m[r1][c2] * m[r2][c1]
    };
    let det: f64 = (0..3).map(|c| m[0][c] * cofactor(0, c)).sum();
    // The adjugate is the cofactors transposed.
    std::array::from_fn(|r| std::array::from_fn(|c| cofactor(c, r) / det))
}

impl Output {
    pub fn name(self) -> &'static str {
        match self {
            Self::Pq => "HDR10 (PQ)",
            Self::ScRgb => "scRGB",
        }
    }
    pub fn format(self) -> wgpu::TextureFormat {
        match self {
            Self::Pq => wgpu::TextureFormat::Rgb10a2Unorm,
            Self::ScRgb => wgpu::TextureFormat::Rgba16Float,
        }
    }
    pub fn color_space(self) -> wgpu::SurfaceColorSpace {
        match self {
            Self::Pq => wgpu::SurfaceColorSpace::Bt2100Pq,
            Self::ScRgb => wgpu::SurfaceColorSpace::ExtendedSrgbLinear,
        }
    }
    fn space(self) -> wgpu::SurfaceColorSpaces {
        match self {
            Self::Pq => wgpu::SurfaceColorSpaces::BT2100_PQ,
            Self::ScRgb => wgpu::SurfaceColorSpaces::EXTENDED_SRGB_LINEAR,
        }
    }
}

/// `HORIZON_HDR_OUTPUT`'s names, in order of preference.
const NAMES: [(Output, &str); 2] = [(Output::Pq, "pq"), (Output::ScRgb, "scrgb")];

/// The HDR output `caps` can present: HDR10 where offered, else scRGB.
/// `HORIZON_HDR_OUTPUT=pq` or `scrgb` allows only that one.
pub fn output(caps: &wgpu::SurfaceCapabilities) -> Option<Output> {
    pick(caps, std::env::var("HORIZON_HDR_OUTPUT").ok().as_deref())
}
fn pick(caps: &wgpu::SurfaceCapabilities, only: Option<&str>) -> Option<Output> {
    NAMES
        .into_iter()
        .filter(|(_, name)| only.is_none_or(|o| o == *name))
        .map(|(output, _)| output)
        .find(|o| caps.color_spaces(o.format()).contains(o.space()))
}
/// The HDR output an offscreen renderer (`--offscreen`) offers: it has no display to ask, so
/// only the one `HORIZON_HDR_OUTPUT` names, for HDR screenshots without an HDR display.
pub fn offscreen_output() -> Option<Output> {
    named(std::env::var("HORIZON_HDR_OUTPUT").ok().as_deref())
}
/// What an offscreen renderer's display reports: nothing, or for a test of Auto without an
/// HDR display, the system's HDR as the file `HORIZON_HDR_DISPLAY` names says it now (`on` or
/// `off`), read at each poll as the system is.
pub(super) fn offscreen_display() -> super::DisplayReading {
    let said =
        std::env::var_os("HORIZON_HDR_DISPLAY").and_then(|file| std::fs::read_to_string(file).ok());
    super::DisplayReading {
        luminance: None,
        primaries: None,
        hdr: match said.as_deref().map(str::trim) {
            Some("on") => Some(true),
            Some("off") => Some(false),
            _ => None,
        },
        compositor: false,
    }
}
fn named(name: Option<&str>) -> Option<Output> {
    NAMES
        .into_iter()
        .find(|(_, n)| name == Some(*n))
        .map(|(output, _)| output)
}

struct Targets {
    size: [u32; 2],
    frame: wgpu::Texture,
    /// The layered frame's tone curve's input: the frame as the post effects left it.
    world: Option<wgpu::Texture>,
    coverage: Option<wgpu::Texture>,
    /// The layered frame's HUD target: what the HUD draws on.
    hud: Option<wgpu::Texture>,
    group: wgpu::BindGroup,
    /// The layered frame's bindings for `tonemap`, `hud_backdrop` and `composite`.
    passes: Option<[wgpu::BindGroup; 3]>,
}

/// The float frame with HUD coverage (`hud_coverage::supported`, docs/design/hdr-linear-light.md
/// "The HUD and menus"): the default world draws write light directly; the tone curve eases
/// it into the peak after the post effects (`tonemap`); the
/// camera overlays draw on that light; the HUD draws on a target of its own,
/// the world clipped to white (`hud_backdrop`); and the last pass composites the HUD over the
/// world's light by its coverage (`fs_hdr_layered`), as the menu blur does before it blurs
/// (`composite`).
struct Layered {
    tonemap: (wgpu::BindGroupLayout, wgpu::RenderPipeline),
    keep: (wgpu::BindGroupLayout, wgpu::RenderPipeline),
    composite: (wgpu::BindGroupLayout, wgpu::RenderPipeline),
    /// Compiled only when a screenshot needs it, for its readback format and HUD mode.
    screenshot: Option<(wgpu::TextureFormat, bool, bool, wgpu::RenderPipeline)>,
    /// What `tonemap` and `composite` read, written before they run (`begin`).
    params: wgpu::Buffer,
}

/// The look's scene peak and its sun's cap, in multiples of paper white, as the tests expect
/// them of the shaders (hdr_look.wgsl `HDR_SCENE_PEAK`, `HDR_SUN_CAP`): the renderer itself
/// reads neither.
#[cfg(test)]
pub const SCENE_PEAK: f32 = 1000.0 / 203.0;
#[cfg(test)]
pub const SUN_CAP: f32 = 2000.0 / 203.0;

/// Which pixels of a light source's sprite may shine (`hdr_stats.wgsl`), as a fraction of
/// the way from its darkest to its brightest opaque pixel: `parts` for one whose bright parts
/// shine (a torch's flame, not its handle), `whole` for one that shines all over
/// (`mesher::WHOLE`). The tier windows still decide how much.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Masks {
    pub parts: f32,
    pub whole: f32,
}
pub const MASKS: Masks = Masks {
    parts: 0.32,
    whole: 0.0,
};
/// A step of HDR calibration (`crate::hdr_calibration`): its test picture is drawn in nits
/// above the controls instead of the frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Calibration {
    /// 1 black level, 2 peak brightness, 3 paper white.
    pub step: u32,
    /// The slider's value, in nits.
    pub nits: f32,
    /// Where the controls start, as a share of the height.
    pub band: f32,
}

pub struct Hdr {
    layout: wgpu::BindGroupLayout,
    /// Horizon's logo as a 1-bit mask, for calibration's test pictures.
    logo: wgpu::TextureView,
    pipeline: wgpu::RenderPipeline,
    params: wgpu::Buffer,
    /// The same pass drawing HDR10 for a screenshot (`capture`), with its own parameters,
    /// so it can differ from the swapchain's in one frame, and without the swapchain's dither:
    /// a screenshot keeps the exact codes.
    capture_pipeline: wgpu::RenderPipeline,
    capture_params: wgpu::Buffer,
    pub output: Output,
    /// The frame's format: what every other pipeline draws in.
    format: wgpu::TextureFormat,
    targets: Option<Targets>,
    /// `HORIZON_HDR_DEBUG=1`: grey what isn't expanded, tint what is magenta.
    debug: bool,
    layered: Option<Layered>,
    /// Paper white and peak brightness, in nits.
    pub paper_white: f32,
    pub peak: f32,
    /// The display's black level, in nits: shadows are lifted onto it (`hdr.wgsl`).
    pub black: f32,
    /// The HUD and menus' brightness, as a share of paper white: above 1 brighter than it.
    pub ui: f32,
    /// Which pixels of a light source shine (`Masks`).
    pub masks: Masks,
    /// HDR lighting, whether Horizon lifts this frame's world: off under a Horizon shader pack
    /// that draws, while a `shaders/` pack's post or material stage runs (the pack's own),
    /// and with the Vanilla look (`look` off).
    pub lighting: bool,
    /// The Look setting, on for Horizon's: off (Vanilla), nothing in the world is lifted, light
    /// sources and the sun included, so the picture is SDR's on the HDR canvas.
    pub look: bool,
    /// Calibration's test picture while it runs.
    pub calibration: Option<Calibration>,
    /// An scRGB surface's reference white is 80 nits, the compositor's white (`surface_scale`):
    /// on Wayland, but for NVIDIA's own driver (`scrgb_reference_80`).
    pub reference_80: bool,
    /// Color Boost's BT.709 to BT.2020 (`gamut`) for the whole picture HDR output writes, the
    /// HDR screenshot included; `None` shows the colours exactly. Calibration's test pictures
    /// never take it.
    pub gamut: Option<Matrix>,
}

/// hdr.wgsl's `Params` in floats: 7 scalars and a pad, `calibration`, `gamut`'s three columns
/// of 16 bytes each, and `boost` padded to the struct's 16-byte alignment.
const PARAMS: usize = 28;

/// `Params`' last 16 floats, from `gamut` on: its columns, each padded to 16 bytes, and
/// `boost`, 1 with a matrix, 0 without.
fn gamut_params(gamut: Option<Matrix>) -> [f32; 16] {
    let mut floats = [0.0; 16];
    if let Some(gamut) = gamut {
        for (i, column) in gamut.iter().enumerate() {
            floats[4 * i..][..3].copy_from_slice(column);
        }
        floats[12] = 1.0;
    }
    floats
}

pub(super) fn shader() -> String {
    super::wgsl_prefix::HDR_LIGHT.to_owned() + include_str!("../hdr.wgsl")
}

impl Hdr {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        output: Output,
        coverage: bool,
    ) -> Self {
        let debug = std::env::var("HORIZON_HDR_DEBUG").ok();
        Self::with_debug(device, queue, format, output, coverage, debug.as_deref())
    }

    fn with_debug(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        output: Output,
        coverage: bool,
        debug: Option<&str>,
    ) -> Self {
        let coverage_debug = coverage && debug == Some("coverage");
        // The layered frame's last pass, else the one over a `shaders/` pack's picture.
        let entry_point = if coverage { "fs_hdr_layered" } else { "fs_hdr" };
        let mut source = shader();
        if coverage_debug {
            // The debug view wraps the last pass as it is, so the picture under the tint is
            // exactly the one without it.
            let entry = format!(
                "@fragment fn {entry_point}(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32>"
            );
            assert!(
                source.contains(&entry),
                "hdr.wgsl's {entry_point} changed its signature"
            );
            source = source.replace(&entry, "fn picture_hdr(pos: vec4<f32>) -> vec4<f32>");
            source.push_str(&entry);
            source.push_str(
                r#" {
    let picture = picture_hdr(pos);
    let c = textureLoad(coverage, vec2<i32>(pos.xy), 0).r;
    // Magenta at paper white, in the output's own encoding.
    let paper = vec3(params.paper, 0.0, params.paper);
    let magenta = select(paper, pq(BT709_TO_BT2020 * paper * 80.0), params.pq > 0.5);
    return vec4(mix(picture.rgb, magenta, c), picture.a);
}
"#,
            );
        }
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon HDR"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        // hdr.wgsl's bindings: the textures at `textures`, and its `Params` at 2 if `params`.
        let layout = |label, textures: &[u32], params: bool| {
            let texture = |binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            };
            let mut entries: Vec<_> = textures.iter().map(|&b| texture(b)).collect();
            if params {
                entries.push(wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                });
            }
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries: &entries,
            })
        };
        // One full-screen pass of hdr.wgsl's `entry` into `format`, reading `layout`; `dither`
        // for the last pass into the HDR10 swapchain (`pq_dither`).
        let pass = |label, layout: &wgpu::BindGroupLayout, entry, format, dither: bool| {
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(layout)],
                ..Default::default()
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_hdr"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &[("pq_dither", f64::from(u8::from(dither)))],
                        ..Default::default()
                    },
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let layout_bindings: &[u32] = if coverage { &[0, 3, 4, 5] } else { &[0, 3] };
        let display = layout("Horizon HDR bindings", layout_bindings, true);
        let pipeline_for =
            |format, dither| pass("Horizon HDR", &display, entry_point, format, dither);
        let params = || {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Horizon HDR parameters"),
                size: std::mem::size_of::<[f32; PARAMS]>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let layered = coverage.then(|| {
            let with = |label, textures: &[u32], params, entry, format| {
                let layout = layout(label, textures, params);
                let pipeline = pass(label, &layout, entry, format, false);
                (layout, pipeline)
            };
            let hud = super::hud_coverage::HUD_FORMAT;
            Layered {
                tonemap: with("Horizon HDR tone curve", &[1], true, "fs_tonemap", format),
                keep: with("Horizon HDR HUD backdrop", &[0], false, "fs_keep", hud),
                composite: with(
                    "Horizon HDR composite",
                    &[0, 4, 5],
                    true,
                    "fs_composite",
                    format,
                ),
                screenshot: None,
                params: params(),
            }
        });
        Self {
            logo: logo_mask(device, queue),
            pipeline: pipeline_for(output.format(), output == Output::Pq),
            capture_pipeline: pipeline_for(Output::Pq.format(), false),
            layout: display,
            params: params(),
            capture_params: params(),
            output,
            format,
            targets: None,
            debug: debug == Some("1"),
            layered,
            paper_white: 200.0,
            peak: 1000.0,
            black: 0.0,
            ui: 1.0,
            masks: MASKS,
            lighting: true,
            look: true,
            calibration: None,
            reference_80: false,
            gamut: None,
        }
    }

    /// The same HDR output presenting `output` instead (`output_apply`): the same frame and
    /// HUD layer, and the player's tuning as it is (the look, the world's lighting, the UI
    /// brightness, Color Boost, the masks, the debug view, calibration and the luminance). Its
    /// targets are made again on the next frame.
    pub fn retargeted(&self, device: &wgpu::Device, queue: &wgpu::Queue, output: Output) -> Self {
        Self {
            debug: self.debug,
            paper_white: self.paper_white,
            peak: self.peak,
            black: self.black,
            ui: self.ui,
            masks: self.masks,
            lighting: self.lighting,
            look: self.look,
            calibration: self.calibration,
            reference_80: self.reference_80,
            gamut: self.gamut,
            ..Self::new(device, queue, self.format, output, self.layered.is_some())
        }
    }

    /// The frame every pass draws into this frame, sized like the window.
    pub fn frame(&mut self, device: &wgpu::Device, size: [u32; 2]) -> wgpu::Texture {
        if self.targets.as_ref().is_none_or(|t| t.size != size) {
            let texture = |label, format, usage| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size[0].max(1),
                        height: size[1].max(1),
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: usage | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
            };
            let frame = texture(
                "Horizon HDR frame",
                self.format,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            );
            let layered = self.layered.is_some();
            let world = layered.then(|| {
                texture(
                    "Horizon HDR world",
                    self.format,
                    wgpu::TextureUsages::COPY_DST,
                )
            });
            let coverage = layered.then(|| {
                texture(
                    "HUD coverage",
                    super::hud_coverage::FORMAT,
                    wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                )
            });
            // 8-bit, so each HUD draw clips and rounds as on the frame with HDR off.
            let hud = layered.then(|| {
                texture(
                    "HUD",
                    super::hud_coverage::HUD_FORMAT,
                    wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                )
            });
            let group = self.group(
                device,
                &frame,
                coverage.as_ref(),
                hud.as_ref(),
                &self.params,
            );
            let passes = self
                .layered
                .as_ref()
                .zip(world.as_ref())
                .zip(coverage.as_ref().zip(hud.as_ref()))
                .map(|((l, world), (coverage, hud))| {
                    let view = |t: &wgpu::Texture| t.create_view(&Default::default());
                    let (frame, world, coverage, hud) =
                        (view(&frame), view(world), view(coverage), view(hud));
                    let entry = |binding, view| wgpu::BindGroupEntry {
                        binding,
                        resource: wgpu::BindingResource::TextureView(view),
                    };
                    let params = wgpu::BindGroupEntry {
                        binding: 2,
                        resource: l.params.as_entire_binding(),
                    };
                    let group = |layout, entries: &[wgpu::BindGroupEntry]| {
                        device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("Horizon HDR layered bindings"),
                            layout,
                            entries,
                        })
                    };
                    [
                        group(&l.tonemap.0, &[entry(1, &world), params.clone()]),
                        group(&l.keep.0, &[entry(0, &frame)]),
                        group(
                            &l.composite.0,
                            &[
                                entry(0, &frame),
                                params,
                                entry(4, &coverage),
                                entry(5, &hud),
                            ],
                        ),
                    ]
                });
            self.targets = Some(Targets {
                size,
                frame,
                world,
                coverage,
                hud,
                group,
                passes,
            });
        }
        self.targets.as_ref().unwrap().frame.clone()
    }

    /// Lets the targets go, a scaled screenshot's: the next `frame` makes them again.
    pub fn release_targets(&mut self) {
        self.targets = None;
    }

    /// The float frame with HUD coverage (`Layered`).
    pub fn layered(&self) -> bool {
        self.layered.is_some()
    }

    /// The layered frame's HUD target, which the HUD and menus draw on instead of the frame
    /// once `hud_backdrop` made it.
    pub fn hud(&self) -> Option<&wgpu::Texture> {
        self.targets.as_ref()?.hud.as_ref()
    }

    /// The layered frame's SDR screenshot, directly in the readback's texture; the icon
    /// maps only the world. Light above white keeps its hue; where Horizon lifted nothing
    /// (`lighting` off: the Vanilla look, or under a shader pack) each channel clips as SDR's
    /// does. Neither the display's targets nor its parameters are changed.
    pub fn sdr_screenshot(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        format: wgpu::TextureFormat,
        world_only: bool,
    ) -> Option<wgpu::Texture> {
        let keeps_hue = self.lighting;
        let (l, t) = (self.layered.as_mut()?, self.targets.as_ref()?);
        if l.screenshot
            .as_ref()
            .is_none_or(|(f, w, s, _)| (*f, *w, *s) != (format, world_only, keeps_hue))
        {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("HDR SDR screenshot"),
                source: wgpu::ShaderSource::Wgsl(shader().into()),
            });
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("HDR SDR screenshot"),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_hdr"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_sdr_screenshot"),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &[
                            ("screenshot_world_only", f64::from(u8::from(world_only))),
                            ("screenshot_keeps_hue", f64::from(u8::from(keeps_hue))),
                        ],
                        ..Default::default()
                    },
                    targets: &[Some(format.into())],
                }),
                multiview_mask: None,
                cache: None,
            });
            l.screenshot = Some((format, world_only, keeps_hue, pipeline));
        }
        let pipeline = &l.screenshot.as_ref()?.3;
        let views = [
            (0, &t.frame),
            (4, t.coverage.as_ref()?),
            (5, t.hud.as_ref()?),
        ]
        .map(|(binding, texture)| (binding, texture.create_view(&Default::default())));
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("HDR SDR screenshot"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &views
                .each_ref()
                .map(|(binding, view)| wgpu::BindGroupEntry {
                    binding: *binding,
                    resource: wgpu::BindingResource::TextureView(view),
                }),
        });
        let copy = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Horizon screenshot copy"),
            size: t.frame.size(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        Self::draw(
            encoder,
            pipeline,
            &group,
            &copy.create_view(&Default::default()),
            "HDR SDR screenshot",
            None,
        );
        Some(copy)
    }

    /// On the layered frame, before `tonemap` and `composite`: what they read, for a frame that
    /// drew a world (`world`) or not.
    pub fn begin(&self, queue: &wgpu::Queue, world: bool) {
        if let Some(l) = &self.layered {
            queue.write_buffer(
                &l.params,
                0,
                bytemuck::cast_slice(&self.params(world, false)),
            );
        }
    }

    /// On the layered frame, after the post effects and before the camera overlays: the tone
    /// curve eases the light into the peak (`fs_tonemap`).
    pub fn tonemap(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        timing: Option<&mut super::pack_timing::PassTiming>,
    ) {
        let (Some(l), Some(t)) = (&self.layered, &self.targets) else {
            return;
        };
        let (Some([group, ..]), Some(world)) = (&t.passes, &t.world) else {
            return;
        };
        encoder.copy_texture_to_texture(
            t.frame.as_image_copy(),
            world.as_image_copy(),
            t.frame.size(),
        );
        let view = t.frame.create_view(&Default::default());
        Self::draw(
            encoder,
            &l.tonemap.1,
            group,
            &view,
            "HDR tone curve",
            timing,
        );
    }

    /// On the layered frame: the picture so far (`fs_composite`), as the frame stores light,
    /// into `target`, for the menu blur to blur it and make it the world (`hud_backdrop` after).
    pub fn composite(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        timing: Option<&mut super::pack_timing::PassTiming>,
    ) {
        if let (
            Some(l),
            Some(Targets {
                passes: Some([_, _, group]),
                ..
            }),
        ) = (&self.layered, &self.targets)
        {
            Self::draw(
                encoder,
                &l.composite.1,
                group,
                target,
                "HDR composite",
                timing,
            );
        }
    }

    pub fn coverage_view(&self) -> Option<wgpu::TextureView> {
        Some(
            self.targets
                .as_ref()?
                .coverage
                .as_ref()?
                .create_view(&Default::default()),
        )
    }

    /// The peak the HDR metadata gives: while calibration's peak step runs, PQ's own
    /// 10 000 nits, so a compositor that clips at the declared peak (KWin) lets the test
    /// patch through as bright as the display goes and the logo under it shows.
    pub fn declared_peak(&self) -> f32 {
        match self.calibration {
            Some(c) if c.step == 2 => 10_000.0,
            _ => self.peak,
        }
    }

    /// The HDR metadata's peak (`declared_peak`) and black level, in the surface's nits
    /// (`surface_scale`).
    pub fn metadata(&self) -> (f32, f32) {
        let scale = surface_scale(self.output, self.reference_80);
        (self.declared_peak() * scale, self.black * scale)
    }

    /// On the layered frame, which holds the world, the hand and the camera overlays as light:
    /// makes the HUD's backdrop from it, clipped to white, alpha 0 (`fs_keep`). The HUD and the
    /// menus draw on it from here on.
    pub fn hud_backdrop(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        timing: Option<&mut super::pack_timing::PassTiming>,
    ) {
        if let (
            Some(l),
            Some(Targets {
                hud: Some(hud),
                passes: Some([_, group, _]),
                ..
            }),
        ) = (&self.layered, &self.targets)
        {
            let view = hud.create_view(&Default::default());
            Self::draw(encoder, &l.keep.1, group, &view, "HDR HUD backdrop", timing);
        }
    }

    /// Writes the finished frame to the swapchain image `target`.
    pub fn present(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::Texture,
        timing: Option<&mut super::pack_timing::PassTiming>,
    ) {
        let Some(t) = &self.targets else {
            return;
        };
        queue.write_buffer(
            &self.params,
            0,
            bytemuck::cast_slice(&self.params(false, self.output == Output::Pq)),
        );
        let view = target.create_view(&Default::default());
        Self::draw(
            encoder,
            &self.pipeline,
            &t.group,
            &view,
            "HDR output",
            timing,
        );
    }

    /// The frame `present` writes, as HDR10 (BT.2020, PQ, 10 bits a channel) whatever
    /// the swapchain is, in a texture of its own for a screenshot (`Rgb10a2Unorm`).
    pub fn capture(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Option<wgpu::Texture> {
        let t = self.targets.as_ref()?;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Horizon HDR screenshot"),
            size: t.frame.size(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Output::Pq.format(),
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let group = self.group(
            device,
            &t.frame,
            t.coverage.as_ref(),
            t.hud.as_ref(),
            &self.capture_params,
        );
        queue.write_buffer(
            &self.capture_params,
            0,
            bytemuck::cast_slice(&self.params(false, true)),
        );
        let view = texture.create_view(&Default::default());
        Self::draw(
            encoder,
            &self.capture_pipeline,
            &group,
            &view,
            "HDR output",
            None,
        );
        Some(texture)
    }

    fn group(
        &self,
        device: &wgpu::Device,
        frame: &wgpu::Texture,
        coverage: Option<&wgpu::Texture>,
        hud: Option<&wgpu::Texture>,
        params: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        let frame = view(frame);
        let layered = coverage.zip(hud).map(|(c, h)| (view(c), view(h)));
        let texture = |binding, view| wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::TextureView(view),
        };
        let mut entries = vec![
            texture(0, &frame),
            wgpu::BindGroupEntry {
                binding: 2,
                resource: params.as_entire_binding(),
            },
            texture(3, &self.logo),
        ];
        if let Some((coverage, hud)) = &layered {
            entries.extend([texture(4, coverage), texture(5, hud)]);
        }
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Horizon HDR bindings"),
            layout: &self.layout,
            entries: &entries,
        })
    }

    /// The display's headroom: its peak over paper white, at least 1.
    fn headroom(&self) -> f32 {
        (self.peak / self.paper_white.max(1.0)).max(1.0)
    }

    /// What the world's writers read (`hdr_written`): the display's headroom, an unused lane,
    /// world/look and debug. The look's own numbers are the shaders' (hdr_look.wgsl), and of
    /// the display only the sun's disc reads the headroom, for its peak (`hdr_sun_peak`): the
    /// other writers leave the display to the tone curve.
    pub(super) fn draw_params(&self, world: bool) -> [f32; 4] {
        let shows = f32::from(u8::from(world && self.look));
        let debug = f32::from(u8::from(self.debug));
        [self.headroom(), 0.0, shows, debug]
    }

    /// hdr.wgsl's `Params`.
    fn params(&self, world: bool, pq: bool) -> [f32; PARAMS] {
        let paper = self.paper_white.max(1.0);
        // Only Horizon's lifted world is known to end at the look's scene peak; elsewhere (the
        // Vanilla look, a pack's own light, the title) the curve's source follows the display.
        let look = f32::from(u8::from(world && self.lighting));
        let c = self.calibration;
        let mut params = [0.0; PARAMS];
        params[..12].copy_from_slice(&[
            paper / 80.0,
            self.headroom(),
            f32::from(u8::from(pq)),
            self.black.clamp(0.0, paper),
            self.ui,
            look,
            if pq {
                1.0
            } else {
                surface_scale(self.output, self.reference_80)
            },
            // Pad `calibration` to its 16-byte alignment.
            0.0,
            c.map_or(0.0, |c| c.step as f32),
            c.map_or(0.0, |c| c.nits),
            c.map_or(0.0, |c| c.band),
            0.0,
        ]);
        params[12..].copy_from_slice(&gamut_params(self.gamut));
        params
    }

    /// One full-screen pass of `pipeline` into `view`.
    fn draw(
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::RenderPipeline,
        group: &wgpu::BindGroup,
        view: &wgpu::TextureView,
        label: &'static str,
        timing: Option<&mut super::pack_timing::PassTiming>,
    ) {
        let attachments = [Some(wgpu::RenderPassColorAttachment {
            view,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })];
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            timestamp_writes: timing.and_then(|t| t.writes(label)),
            color_attachments: &attachments,
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// The windows' icon (`window::set_icon_png`: Horizon's in the official game) as a 1-bit mask:
/// its white strokes, one colour with hard edges, are the logo; everything else is not. No icon
/// given: an empty mask.
fn logo_mask(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
    use wgpu::util::DeviceExt;
    let (w, h, rgba) = horizon_client_input::window::icon_png()
        .and_then(|png| crate::hud::decode_bytes(png).ok())
        .unwrap_or((1, 1, vec![0; 4]));
    let mask: Vec<u8> = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| {
            if p[3] > 128 && p[..3].iter().all(|&c| c > 200) {
                255
            } else {
                0
            }
        })
        .collect();
    device
        .create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("Horizon HDR calibration logo"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &mask,
        )
        .create_view(&Default::default())
}

#[cfg(test)]
mod tests {
    use super::{Hdr, Output, named, pick};
    use wgpu::{SurfaceColorSpaces as S, TextureFormat as F};

    #[test]
    fn shared_light_matches_the_original_for_every_mark() {
        let _gpu = crate::gfx::test_gpu::lock();
        use wgpu::util::DeviceExt;
        let Some((device, queue)) = gpu() else {
            eprintln!("HDR equivalence GPU unavailable");
            return;
        };
        // All 8-bit marks, non-grey and over-white colours, look, debug, headroom and the unused
        // lane; half-float neighbours straddle mark boundaries. The reference's shares of its
        // peak give the look's gains (1.6, 2.4, 2.8 and 4).
        let source = super::super::wgsl_prefix::HDR_LIGHT.to_owned()
            + r#"
struct ReferenceParams { peak: f32, unused: f32, world: f32, debug: f32, share: vec4<f32> }
var<private> params: ReferenceParams;
@group(0) @binding(0) var<storage, read_write> errors: array<u32>;
"# + include_str!("hdr_light_reference.txt")
            + r#"
@compute @workgroup_size(64) fn check(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    let mark = f32(i % 256u) / 255.0;
    let mode = i / 256u;
    params = ReferenceParams(4.0, select(1.0, 4.0, (mode & 4u) != 0u),
        select(0.0, 1.0, (mode & 1u) != 0u), select(0.0, 1.0, (mode & 8u) != 0u),
        vec4(0.2, 1.4 / 3.0, 0.6, 1.0));
    // The headroom moves the sun's disc alone.
    let settings = vec4(select(1.0, 1000.0 / 203.0, (mode & 2u) != 0u), params.unused, params.world,
        params.debug);
    let color = select(vec3(0.9, 0.4, 0.02), vec3(1.4, 0.8, 0.2), (mode & 16u) != 0u);
    // The look's light, where it is on.
    let lifted = reference_lit(color, mark, true);
    let actual = hdr_lit(color, mark, true);
    errors[i] = select(0u, 1u, params.debug < 0.5
        && (any(abs(lifted.light - actual.light) > vec3(1e-6)) || lifted.lifted != actual.lifted));
    // From 0.75 up nothing is lifted: a lit surface's white stays white.
    let white = hdr_written(vec3(1.0), 0.76, vec4(5.0, 4.0, 1.0, 0.0));
    errors[i] |= select(0u, 4u, any(white != vec3(1.0)));
    // An unlifted texel sheds no shading: scaled by exactly 1.
    errors[i] |= select(0u, 8u, hdr_unshade(color, 0.0, 0.3) != 1.0);
    let boundary = (f32(i % 255u) + 0.5) / 255.0;
    for (var step = -1; step <= 1; step++) {
        let stored = unpack2x16float(pack2x16float(vec2(boundary + f32(step) / 65536.0))).x;
        let quantized = round(saturate(stored) * 255.0) / 255.0;
        let want = pow(reference_lit(color, quantized, params.world > 0.5).light, vec3(1.0 / 2.2));
        let got = hdr_written(color, stored, settings);
        errors[i] |= select(0u, 2u, any(abs(want - got) > vec3(1e-6)));
    }
}
"#;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shared HDR equivalence"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &shader,
            entry_point: Some("check"),
            compilation_options: Default::default(),
            cache: None,
        });
        let count = 256 * 32;
        let output = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: &vec![0; count * 4],
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (count * 4) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: output.as_entire_binding(),
            }],
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(count as u32 / 64, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, (count * 4) as u64);
        queue.submit([encoder.finish()]);
        readback.map_async(wgpu::MapMode::Read, .., |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let data = readback.get_mapped_range(..).unwrap();
        for (i, bytes) in data.as_chunks::<4>().0.iter().enumerate() {
            assert_eq!(u32::from_le_bytes(*bytes), 0, "case {i}");
        }
        eprintln!(
            "HDR equivalence GPU ran {count} cases and {} half-float boundary cases",
            count * 3
        );
    }

    #[test]
    fn coverage_target_resizes_resets_and_shows_in_the_debug_view() {
        let _gpu = crate::gfx::test_gpu::lock();
        let instance = crate::gfx::test_gpu::instance();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            return;
        };
        let Ok((device, queue)) = pollster::block_on(adapter.request_device(&Default::default()))
        else {
            return;
        };
        for enabled in [false, true] {
            for debug in [None, Some("coverage")] {
                let mut hdr =
                    Hdr::with_debug(&device, &queue, F::Rgba16Float, Output::Pq, enabled, debug);
                hdr.frame(&device, [2, 1]);
                assert_eq!(hdr.coverage_view().is_some(), enabled);
                hdr.frame(&device, [4, 2]);
                let t = hdr.targets.as_ref().unwrap();
                if let Some(coverage) = &t.coverage {
                    assert_eq!(coverage.size(), t.frame.size());
                }
                let mut encoder = device.create_command_encoder(&Default::default());
                let frame_view = t.frame.create_view(&Default::default());
                let mut color = super::super::hud_coverage::attachment(&frame_view);
                color.ops.load = wgpu::LoadOp::Clear(wgpu::Color::BLACK);
                encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    color_attachments: &[Some(color)],
                    ..Default::default()
                });
                hdr.hud_backdrop(&mut encoder, None);
                if let Some(view) = hdr.coverage_view() {
                    super::super::hud_coverage::clear(&mut encoder, &view);
                    let mut color = super::super::hud_coverage::attachment(&view);
                    color.ops.load = wgpu::LoadOp::Clear(wgpu::Color::WHITE);
                    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        color_attachments: &[Some(color)],
                        ..Default::default()
                    });
                }
                let capture = hdr.capture(&device, &queue, &mut encoder).unwrap();
                let buffer = super::super::copy_frame(&device, &mut encoder, &capture, [4, 2]);
                queue.submit([encoder.finish()]);
                buffer.map_async(wgpu::MapMode::Read, .., |r| r.unwrap());
                device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                let bytes = buffer.get_mapped_range(..).unwrap();
                let pixel = u32::from_le_bytes(bytes[..4].try_into().unwrap());
                let [r, g, b] = [0, 10, 20].map(|shift| (pixel >> shift) & 0x3ff);
                if enabled && debug.is_some() {
                    // Fully covered: BT.709 magenta at paper white (200 nits here) in BT.2020
                    // PQ, not PQ's 10 000 nits (1023, 0, 1023).
                    let near = |got: u32, want: u32| got.abs_diff(want) <= 4;
                    assert!(near(r, 550) && near(g, 346) && near(b, 583), "{r} {g} {b}");
                } else {
                    assert_eq!(pixel, 0xc0000000);
                }
                if let Some(view) = hdr.coverage_view() {
                    let mut encoder = device.create_command_encoder(&Default::default());
                    super::super::hud_coverage::clear(&mut encoder, &view);
                    let coverage = hdr.targets.as_ref().unwrap().coverage.as_ref().unwrap();
                    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                        label: None,
                        size: 512,
                        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                        mapped_at_creation: false,
                    });
                    encoder.copy_texture_to_buffer(
                        coverage.as_image_copy(),
                        wgpu::TexelCopyBufferInfo {
                            buffer: &buffer,
                            layout: wgpu::TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some(256),
                                rows_per_image: None,
                            },
                        },
                        coverage.size(),
                    );
                    queue.submit([encoder.finish()]);
                    buffer.map_async(wgpu::MapMode::Read, .., |r| r.unwrap());
                    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                    assert_eq!(&buffer.get_mapped_range(..).unwrap()[..4], &[0; 4]);
                }
            }
        }
    }

    pub(super) fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = crate::gfx::test_gpu::instance();
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        eprintln!("HDR GPU {:?}", adapter.get_info());
        pollster::block_on(adapter.request_device(&Default::default())).ok()
    }

    /// Clears all of `texture` to `color`.
    pub(super) fn fill(
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
        color: [f64; 4],
    ) {
        let view = texture.create_view(&Default::default());
        let mut attachment = super::super::hud_coverage::attachment(&view);
        let [r, g, b, a] = color;
        attachment.ops.load = wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a });
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(attachment)],
            ..Default::default()
        });
    }

    /// The first texel of `texture` once `encoder` ran: four half floats, 10-bit PQ codes
    /// for HDR10, or 8-bit values over 255 for the HUD's target.
    pub(super) fn texel(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mut encoder: wgpu::CommandEncoder,
        texture: &wgpu::Texture,
    ) -> [f32; 4] {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 256,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        buffer.map_async(wgpu::MapMode::Read, .., |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let bytes = buffer.get_mapped_range(..).unwrap();
        if texture.format() == Output::Pq.format() {
            let pixel = u32::from_le_bytes(bytes[..4].try_into().unwrap());
            return [0, 10, 20, 30].map(|shift| ((pixel >> shift) & 0x3ff) as f32);
        }
        if texture.format() == super::super::hud_coverage::HUD_FORMAT {
            return std::array::from_fn(|i| f32::from(bytes[i]) / 255.0);
        }
        std::array::from_fn(|i| half::f16::from_le_bytes([bytes[2 * i], bytes[2 * i + 1]]).to_f32())
    }

    /// KWin's target primaries for a wide-gamut panel, from a trace of its events.
    const KWIN: super::Primaries = [[0.690, 0.303], [0.241, 0.714], [0.144, 0.054]];

    /// Color Boost at 0 is BT.709 to BT.2020 as the shader's constant (ITU-R BT.2087, 4
    /// decimals) has it, and leaves the uniform's matrix and flag at zero: the shader's own path.
    #[test]
    fn no_color_boost_is_bt709_to_bt2020() {
        let constant = [
            [0.6274, 0.0691, 0.0164],
            [0.3293, 0.9195, 0.0880],
            [0.0433, 0.0114, 0.8956],
        ];
        let m = super::stretched_to_bt2020(0.0, KWIN);
        for (got, want) in m.as_flattened().iter().zip(constant.as_flattened()) {
            assert!((got - want).abs() < 1e-4, "{m:?}");
        }
        assert_eq!(super::gamut(0.0, KWIN), None);
        assert_eq!(super::gamut_params(None), [0.0; 16]);
        // Packed as WGSL takes a mat3x3: red's column first, each padded to 16 bytes.
        let packed = super::gamut_params(Some(m));
        let pad = [0.0];
        assert_eq!(
            packed,
            [
                &m[0][..],
                &pad,
                &m[1],
                &pad,
                &m[2],
                &pad,
                &[1.0, 0.0, 0.0, 0.0]
            ]
            .concat()[..]
        );
    }

    /// Full Color Boost shows pure red, green and blue at the display's own primaries (through
    /// ITU-R BT.2020's published RGB to XYZ), and white stays D65's white.
    #[test]
    fn full_color_boost_reaches_the_display_primaries() {
        let target = super::boost_target(Some(KWIN));
        assert_eq!(target, KWIN);
        let m = super::gamut(1.0, target).unwrap();
        let bt2020_to_xyz = [
            [0.636958, 0.144617, 0.168881],
            [0.262700, 0.677998, 0.059302],
            [0.000000, 0.028073, 1.060985],
        ];
        for (channel, want) in m.iter().zip(KWIN) {
            let xyz: [f32; 3] =
                std::array::from_fn(|r| (0..3).map(|k| bt2020_to_xyz[r][k] * channel[k]).sum());
            let sum: f32 = xyz.iter().sum();
            let xy = [xyz[0] / sum, xyz[1] / sum];
            assert!(
                (xy[0] - want[0]).abs() < 1e-4 && (xy[1] - want[1]).abs() < 1e-4,
                "{xy:?} {want:?}"
            );
        }
        let white: [f32; 3] = std::array::from_fn(|r| m.iter().map(|c| c[r]).sum());
        assert!(white.iter().all(|w| (w - 1.0).abs() < 1e-5), "{white:?}");
    }

    /// What Color Boost stretches toward: the display's primaries; Display P3 for none, zeros,
    /// or a gamut short of BT.709. Display P3 reported as DXGI quantises it holds BT.709's blue,
    /// which it shares. A red past BT.2020, Display P3's own included, is moved onto its edge,
    /// so no colour needs a negative BT.2020 channel.
    #[test]
    fn the_boost_target_stays_within_bt2020_and_falls_back_to_display_p3() {
        use super::{BT2020, DISPLAY_P3, boost_target, inside, reported_primaries};
        let fallback = boost_target(None);
        assert_eq!(fallback[1..], DISPLAY_P3[1..]);
        for reported in [
            Some([[0.0; 2]; 3]),
            Some([[0.600, 0.340], [0.320, 0.570], [0.160, 0.070]]),
            reported_primaries([[f32::NAN, 0.3], [0.2, 0.7], [0.15, 0.06]]),
        ] {
            assert_eq!(boost_target(reported), fallback, "{reported:?}");
        }
        let quantised = [[0.6797, 0.3203], [0.2647, 0.6904], [0.1503, 0.0605]];
        assert_eq!(boost_target(Some(quantised))[1..], quantised[1..]);
        let wide = [[0.730, 0.265], KWIN[1], KWIN[2]];
        let clamped = boost_target(Some(wide));
        assert_eq!(clamped[1..], wide[1..]);
        for target in [fallback, boost_target(Some(quantised)), clamped] {
            assert!(inside(target[0], BT2020, 1e-6), "{target:?}");
            let m = super::gamut(1.0, target).unwrap();
            assert!(m.as_flattened().iter().all(|&c| c > -1e-6), "{m:?}");
        }
        assert_ne!(fallback[0], DISPLAY_P3[0]);
        assert_ne!(clamped[0], wide[0]);
    }

    /// hdr.wgsl's `Params` is laid out as `params` writes it: `gamut` after `calibration`, its
    /// columns 16 bytes apart, then `boost`, the whole as large as the buffer.
    #[test]
    fn the_params_struct_matches_the_floats_written() {
        let module = naga::front::wgsl::parse_str(&super::shader()).unwrap();
        let (_, ty) = module
            .types
            .iter()
            .find(|(_, t)| t.name.as_deref() == Some("Params"))
            .unwrap();
        let naga::TypeInner::Struct { members, span } = &ty.inner else {
            panic!("Params is a struct");
        };
        assert_eq!(*span as usize, 4 * super::PARAMS);
        let offset = |name: &str| {
            members
                .iter()
                .find(|m| m.name.as_deref() == Some(name))
                .map(|m| m.offset as usize / 4)
        };
        assert_eq!(offset("calibration"), Some(8));
        assert_eq!(offset("gamut"), Some(super::PARAMS - 16));
        assert_eq!(offset("boost"), Some(super::PARAMS - 4));
    }

    /// Mesa's drivers (RADV, ANV, NVK on an NVIDIA card) leave a Wayland scRGB surface at the
    /// protocol's reference white of 80 nits; NVIDIA's own driver declares 203.
    #[test]
    fn only_nvidias_own_driver_declares_its_scrgb_reference() {
        for (vendor, driver, reference_80) in [
            (0x1002, "radv", true),
            (0x8086, "Intel open-source Mesa driver", true),
            (0x10de, "NVK", true),
            (0x10de, "NVIDIA", false),
        ] {
            assert_eq!(
                super::scrgb_reference_80(vendor, driver),
                reference_80,
                "{driver}"
            );
        }
    }

    /// On a surface at 80 nits' reference, scRGB writes paper white's 203 nits as 1.0 and the
    /// metadata's nits on that scale; HDR10, the screenshot's HDR10 copy and other surfaces
    /// are untouched.
    #[test]
    fn scrgb_at_an_80_nit_reference_writes_203_nits_as_one() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = gpu() else {
            return;
        };
        let gain = 80.0 / 203.0;
        for output in [Output::Pq, Output::ScRgb] {
            let mut hdr = Hdr::with_debug(&device, &queue, F::Rgba16Float, output, true, None);
            (hdr.peak, hdr.black) = (812.0, 0.5);
            let plain = (
                hdr.params(false, false),
                hdr.params(false, true),
                hdr.metadata(),
            );
            assert_eq!(plain.0[6], 1.0);
            assert_eq!(plain.2, (812.0, 0.5));
            hdr.reference_80 = true;
            let scrgb = output == Output::ScRgb;
            let params = hdr.params(false, false);
            assert_eq!(params[6], if scrgb { gain } else { 1.0 }, "{output:?}");
            assert_eq!(params[..6], plain.0[..6]);
            assert_eq!(params[7..], plain.0[7..]);
            // The PQ pass (present on HDR10, capture always) reads exactly what it did.
            assert_eq!(hdr.params(false, true), plain.1, "{output:?}");
            let want = if scrgb {
                (812.0 * gain, 0.5 * gain)
            } else {
                (812.0, 0.5)
            };
            assert_eq!(hdr.metadata(), want, "{output:?}");
        }
    }

    /// The world's writers read no gain from the display (docs/design/hdr-linear-light.md, step
    /// 6a). Of the display only its headroom reaches them, for the sun's disc's peak, and the
    /// tone curve, which holds light at that peak: the look works both out of the headroom
    /// (hdr_look.wgsl `hdr_sun_peak`), so the renderer says only whether the look is on.
    #[test]
    fn the_writers_light_is_the_same_on_every_display_but_the_suns_disc() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = gpu() else {
            return;
        };
        let mut hdr = Hdr::with_debug(&device, &queue, F::Rgba16Float, Output::Pq, true, None);
        for (paper, peak) in [
            (203.0, 300.0),
            (203.0, 600.0),
            (203.0, 1000.0),
            (400.0, 600.0),
            (480.0, 200.0),
            (203.0, 1500.0),
            (203.0, 2000.0),
            (203.0, 4000.0),
            (80.0, 10000.0),
        ] {
            (hdr.paper_white, hdr.peak) = (paper, peak);
            assert_eq!(
                hdr.draw_params(true),
                [(peak / paper).max(1.0), 0.0, 1.0, 0.0],
                "{paper}/{peak}"
            );
            let params = hdr.params(true, false);
            assert_eq!(
                (params[1], params[5]),
                ((peak / paper).max(1.0), 1.0),
                "{paper}/{peak}"
            );
        }
        // The title, and frames Horizon doesn't light (the Vanilla look, a pack drawing):
        // nothing says where their light ends, so the curve's source follows the display.
        assert_eq!(hdr.params(false, false)[5], 0.0);
        hdr.lighting = false;
        assert_eq!(hdr.params(true, false)[5], 0.0);
    }

    /// The tone curve maps the scene's peak S onto the display's peak T with ITU-R BT.2390's
    /// EETF, keeping hue, after holding light above S at S. Checkpoints are worked out apart
    /// from the shader, in f64 from SMPTE ST 2084 and BT.2390.
    #[test]
    fn the_tone_curve_maps_the_scene_peak_onto_the_display() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = gpu() else {
            return;
        };
        let mut hdr = Hdr::with_debug(&device, &queue, F::Rgba16Float, Output::Pq, true, None);
        let scene_peak = super::SCENE_PEAK;
        // The frame holding `color` (in gamma, as it stores light), after the curve if `curve`.
        let through = |hdr: &mut Hdr, color: [f64; 4], curve: bool| {
            let frame = hdr.frame(&device, [1, 1]);
            let mut encoder = device.create_command_encoder(&Default::default());
            fill(&mut encoder, &frame, color);
            hdr.begin(&queue, true);
            if curve {
                hdr.tonemap(&mut encoder, None);
            }
            texel(&device, &queue, encoder, &frame)
        };
        // `exact`, or within one half-float step: S's own gamma rounds up in half float, to
        // light just above S that the curve holds at S, and whether that comes back to the same
        // half float depends on the GPU's rounding (one step lower on an Apple M4 Pro).
        let unchanged = |hdr: &mut Hdr, color: [f64; 4], exact: bool| {
            let (got, stored) = (through(hdr, color, true), through(hdr, color, false));
            (0..4).all(|c| {
                got[c] == stored[c] || !exact && (got[c] - stored[c]).abs() <= stored[c] / 1024.0
            })
        };
        // Linear light in and out, in units of paper white.
        let mapped = |hdr: &mut Hdr, light: [f32; 3]| {
            let [r, g, b] = light.map(|c| f64::from(c.powf(1.0 / 2.2)));
            let [r, g, b, a] = through(hdr, [r, g, b, 1.0], true);
            assert_eq!(a, 1.0);
            [r, g, b].map(|c| c.powf(2.2))
        };
        let near = |got: f32, want: f32| (got - want).abs() <= 0.005 * want.max(1.0);
        // A display at or above the mastering one (T >= S): every stored value up to S comes
        // out exactly as it went in, grey or coloured, so 1500 and 10 000 nits show what 1000
        // does; so do the settings' extremes (Paper White 10 to 480 nits, Peak up to 10 000)
        // wherever T reaches S. Past S only the sun's disc is written, up to its peak (T, at
        // most `SUN_CAP`): that too comes out as it went in, and light above it is held at it
        // with its hue.
        for (setting, (paper, peak)) in [
            (203.0, 1000.0),
            (203.0, 1500.0),
            (203.0, 10000.0),
            (480.0, 10000.0),
            (10.0, 200.0),
        ]
        .into_iter()
        .enumerate()
        {
            (hdr.paper_white, hdr.peak) = (paper, peak);
            // The first setting is the identity curve, so it gets the whole ramp; the others
            // sample it densely at the ends and the knee region.
            let ramp: Vec<i32> = if setting == 0 {
                (0..=64).collect()
            } else {
                vec![0, 1, 2, 4, 8, 16, 24, 32, 40, 48, 56, 60, 62, 63, 64]
            };
            for i in ramp {
                let g = f64::from((scene_peak * i as f32 / 64.0).powf(1.0 / 2.2));
                let exact = i < 64;
                assert!(unchanged(&mut hdr, [g, g, g, 1.0], exact), "{peak}: {i}");
                assert!(
                    unchanged(&mut hdr, [g, g * 0.6, g * 0.1, 1.0], exact),
                    "{peak}: {i}"
                );
            }
            let sun_peak = (peak / paper).clamp(scene_peak, super::SUN_CAP);
            for i in 0..64 {
                let light = scene_peak + (sun_peak - scene_peak) * i as f32 / 64.0;
                let g = f64::from(light.powf(1.0 / 2.2));
                assert!(
                    unchanged(&mut hdr, [g, g * 0.6, g * 0.1, 1.0], light > scene_peak),
                    "{peak}: {light}"
                );
            }
            for light in [sun_peak, 8.0, 50.0] {
                let [r, g, b] = mapped(&mut hdr, [light, light / 4.0, 0.0]);
                assert!(
                    near(r, light.min(sun_peak)) && near(g * 4.0, r) && b == 0.0,
                    "{peak}: {light}: {r} {g} {b}"
                );
            }
        }
        // Dimmer displays: unchanged up to the knee, then the EETF into T, T at S and past it.
        // At 300 nits the knee falls just below white (161 nits): white shows at 198 nits
        // where the gates' curve, whose source followed the display, kept it at 203; the
        // highlights above it keep more of their steps.
        for (paper, peak, knee, checkpoints) in [
            (
                203.0,
                600.0,
                2.0,
                &[
                    (2.5, 2.47516),
                    (3.0, 2.75699),
                    (4.0, 2.9404),
                    (scene_peak, 600.0 / 203.0),
                    (8.0, 600.0 / 203.0),
                ][..],
            ),
            (
                203.0,
                300.0,
                0.75,
                &[
                    (0.9, 0.89289),
                    (1.0, 0.97365),
                    (1.2, 1.10406),
                    (1.4, 1.20134),
                    (2.0, 1.36833),
                    (4.0, 1.47641),
                    (8.0, 300.0 / 203.0),
                ],
            ),
            (
                400.0,
                600.0,
                0.8,
                &[
                    (1.0, 0.97973),
                    (1.2, 1.11501),
                    (1.5, 1.25607),
                    (2.0, 1.38818),
                    (scene_peak, 1.5),
                ],
            ),
            // No headroom (a peak under paper white): the scene is compressed into white.
            (
                203.0,
                150.0,
                0.4,
                &[
                    (1.0, 0.78346),
                    (1.5, 0.90244),
                    (2.0, 0.95589),
                    (4.0, 0.99944),
                    (scene_peak, 1.0),
                ],
            ),
        ] {
            (hdr.paper_white, hdr.peak) = (paper, peak);
            for light in [0.0f32, 0.3, knee] {
                let g = f64::from(light.powf(1.0 / 2.2));
                assert!(
                    unchanged(&mut hdr, [g, g * 0.5, 0.0, 1.0], true),
                    "{paper}/{peak}: {light}"
                );
            }
            for &(light, want) in checkpoints {
                let [r, ..] = mapped(&mut hdr, [light; 3]);
                assert!(near(r, want), "{paper}/{peak}: {light}: {r}, not {want}");
            }
            // It rises, stays under the peak, and the brightest channel scales the others; a
            // half float's step in gamma is 0.1% of the light near white.
            let headroom = (peak / paper).max(1.0);
            let mut last = 0.0;
            for i in 0..40 {
                let light = 0.5 * 1.08f32.powi(i);
                let [r, g, b] = mapped(&mut hdr, [light, light / 4.0, 0.0]);
                assert!(
                    r >= last * 0.998 && r <= headroom * 1.002,
                    "{peak}: {light}: {r}"
                );
                assert!(near(g * 4.0, r) && b == 0.0, "{peak}: {light}: {r} {g} {b}");
                last = r;
            }
        }
        // Light Horizon doesn't lift (the Vanilla look, a 0.2 pack's own) has no known peak:
        // the curve's source follows the display, the knee at 60% of its peak and at least
        // white, as before one peak. At 300 nits the source's peak is 645 nits.
        hdr.lighting = false;
        (hdr.paper_white, hdr.peak) = (203.0, 300.0);
        let headroom = 300.0 / 203.0;
        for light in [0.0f32, 0.5, 0.9, 1.0] {
            let g = f64::from(light.powf(1.0 / 2.2));
            assert!(unchanged(&mut hdr, [g, g * 0.5, 0.0, 1.0], true), "{light}");
        }
        for (light, want) in [
            (1.2, 1.16839),
            (1.4, 1.28356),
            (headroom, 1.31719),
            (2.0, 1.44024),
            (4.0, headroom),
        ] {
            let [r, ..] = mapped(&mut hdr, [light; 3]);
            assert!(near(r, want), "{light}: {r}, not {want}");
        }
        // With no headroom there, a clip at white, channel by channel.
        hdr.peak = 150.0;
        let [r, g, b] = mapped(&mut hdr, [1.5, 0.5, 0.2]);
        assert!(r == 1.0 && near(g, 0.5) && near(b, 0.2), "{r} {g} {b}");
    }

    /// HDR output switched from HDR10 to scRGB and back (`output_apply`): the player's tuning
    /// stays, each swapchain gets the same light in its own encoding, and the HDR10 capture
    /// and the HDR10 picture after the switch back are the ones before it.
    #[test]
    fn a_switched_encoding_keeps_the_tuning_and_shows_the_same_light() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = gpu() else {
            eprintln!("HDR encoding switch GPU skipped: no adapter");
            return;
        };
        let mut hdr = Hdr::with_debug(&device, &queue, F::Rgba16Float, Output::Pq, true, None);
        (hdr.paper_white, hdr.peak, hdr.black) = (250.0, 800.0, 0.0);
        (hdr.ui, hdr.look, hdr.lighting) = (0.5, false, false);
        hdr.masks.parts = 0.4;
        hdr.debug = true;
        hdr.calibration = Some(super::Calibration {
            step: 2,
            nits: 700.0,
            band: 0.6,
        });
        let tuning = |h: &Hdr| {
            (
                [h.paper_white, h.peak, h.black, h.ui],
                (h.look, h.lighting, h.debug, h.layered.is_some(), h.format),
                (h.masks, h.calibration),
            )
        };
        let switched = hdr.retargeted(&device, &queue, Output::ScRgb);
        assert_eq!(switched.output, Output::ScRgb);
        assert_eq!(tuning(&switched), tuning(&hdr));
        hdr.calibration = None;
        hdr.debug = false;
        // The swapchain's first texel (`capture`: the HDR10 screenshot's) for light 2 times
        // paper white, grey, as the layered frame stores it.
        let shot = |hdr: &mut Hdr, capture: bool| {
            let frame = hdr.frame(&device, [1, 1]);
            let mut encoder = device.create_command_encoder(&Default::default());
            let g = 2f64.powf(1.0 / 2.2);
            fill(&mut encoder, &frame, [g, g, g, 1.0]);
            hdr.begin(&queue, true);
            hdr.hud_backdrop(&mut encoder, None);
            super::super::hud_coverage::clear(&mut encoder, &hdr.coverage_view().unwrap());
            let target = if capture {
                hdr.capture(&device, &queue, &mut encoder).unwrap()
            } else {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: None,
                    size: frame.size(),
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: hdr.output.format(),
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                })
            };
            if !capture {
                hdr.present(&queue, &mut encoder, &target, None);
            }
            texel(&device, &queue, encoder, &target)
        };
        // BT.2100's PQ EOTF: a 10-bit code's nits.
        let pq_nits = |code: f32| {
            let (m1, m2) = (2610.0 / 16384.0, 2523.0 / 4096.0 * 128.0);
            let (c1, c2, c3) = (3424.0 / 4096.0, 2413.0 / 128.0, 2392.0 / 128.0);
            let p = (code / 1023.0f32).powf(1.0 / m2);
            ((p - c1).max(0.0) / (c2 - c3 * p)).powf(1.0 / m1) * 10000.0
        };
        let pq = shot(&mut hdr, false);
        let pq_capture = shot(&mut hdr, true);
        let mut scrgb = hdr.retargeted(&device, &queue, Output::ScRgb);
        let linear = shot(&mut scrgb, false);
        assert_eq!(shot(&mut scrgb, true), pq_capture);
        // Two times paper white in both: 500 nits, as HDR10's code and as scRGB's 80-nit units.
        let near = |got: f32| (got - 500.0).abs() <= 0.015 * 500.0;
        assert!(
            near(pq_nits(pq[0])) && pq[..3].iter().all(|&c| c == pq[0]),
            "{pq:?}"
        );
        assert!(near(linear[0] * 80.0), "{linear:?}");
        let mut back = scrgb.retargeted(&device, &queue, Output::Pq);
        assert_eq!(
            (shot(&mut back, false), shot(&mut back, true)),
            (pq, pq_capture)
        );
    }

    /// ITU-R BT.2390's black level lift, in HDR10 and in scRGB, against values worked out
    /// apart from the shader, in f64 from SMPTE ST 2084: E3 = E2 + b (1 - E2)^4 on the PQ
    /// signal of each BT.2020 channel, for a source from 0 to the peak and b the black level's
    /// share of it. Both outputs show the same light, grey or coloured: scRGB's, decoded to
    /// BT.2020, is HDR10's.
    #[test]
    fn the_black_level_lift_follows_bt2390_in_pq_and_scrgb() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = gpu() else {
            return;
        };
        let (m1, m2) = (2610.0 / 16384.0, 2523.0 / 4096.0 * 128.0);
        let (c1, c2, c3) = (3424.0 / 4096.0, 2413.0 / 128.0, 2392.0 / 128.0);
        let pq = |nits: f64| {
            let y = (nits / 10_000.0).powf(m1);
            ((c1 + c2 * y) / (1.0 + c3 * y)).powf(m2)
        };
        let nits = |e: f64| {
            let p = e.powf(1.0 / m2);
            10_000.0 * ((p - c1).max(0.0) / (c2 - c3 * p)).powf(1.0 / m1)
        };
        // Linear BT.709 to BT.2020, ITU-R BT.2087's matrix as hdr.wgsl rounds it.
        let to_bt2020 = |[r, g, b]: [f64; 3]| {
            [
                0.6274 * r + 0.3293 * g + 0.0433 * b,
                0.0691 * r + 0.9195 * g + 0.0114 * b,
                0.0164 * r + 0.0880 * g + 0.8956 * b,
            ]
        };
        let (paper, peak) = (203.0, 1000.0);
        let lifted = |light: f64, black: f64| {
            let top = pq(peak);
            let e2 = (pq(light) / top).min(1.0);
            nits((e2 + pq(black) / top * (1.0 - e2).powi(4)) * top)
        };
        for output in [Output::Pq, Output::ScRgb] {
            let mut hdr = Hdr::with_debug(&device, &queue, F::Rgba16Float, output, true, None);
            (hdr.paper_white, hdr.peak) = (paper as f32, peak as f32);
            let target = device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: output.format(),
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            for black in [0.0, 0.005, 0.05, 0.5] {
                hdr.black = black as f32;
                // Black, the shadows, paper white and the peak, in units of paper white, grey
                // and in BT.709's primaries and orange.
                for (light, colour) in [0.0, 0.001, 0.01, 0.1, 1.0, peak / paper]
                    .into_iter()
                    .flat_map(|light| {
                        // Black is the same in every colour: once.
                        let colours = if light == 0.0 { 1 } else { 5 };
                        [
                            [1.0, 1.0, 1.0],
                            [1.0, 0.0, 0.0],
                            [0.0, 1.0, 0.0],
                            [0.0, 0.0, 1.0],
                            [1.0, 0.5, 0.1],
                        ]
                        .into_iter()
                        .take(colours)
                        .map(move |colour| (light, colour))
                    })
                {
                    let [r, g, b] = colour.map(|c: f64| {
                        half::f16::from_f64(f64::powf(light * c, 1.0 / 2.2)).to_f64()
                    });
                    let shown = to_bt2020([r, g, b].map(|g| (g.powf(2.2) * paper).min(peak)));
                    let want = shown.map(|c| if black > 0.0 { lifted(c, black) } else { c });
                    let frame = hdr.frame(&device, [1, 1]);
                    let mut encoder = device.create_command_encoder(&Default::default());
                    fill(&mut encoder, &frame, [r, g, b, 1.0]);
                    hdr.hud_backdrop(&mut encoder, None);
                    super::super::hud_coverage::clear(&mut encoder, &hdr.coverage_view().unwrap());
                    hdr.begin(&queue, true);
                    hdr.present(&queue, &mut encoder, &target, None);
                    // HDR10's exact codes are a screenshot's copy: the swapchain's own get a
                    // code of dither either way.
                    let exact = (output == Output::Pq)
                        .then(|| hdr.capture(&device, &queue, &mut encoder).unwrap());
                    let presented = texel(&device, &queue, encoder, &target);
                    let exact = exact.map(|copy| {
                        let encoder = device.create_command_encoder(&Default::default());
                        texel(&device, &queue, encoder, &copy)
                    });
                    for (got, codes) in [(Some(presented), 2.0), (exact, 1.0)] {
                        let Some(got) = got else {
                            continue;
                        };
                        let got = [got[0], got[1], got[2]].map(f64::from);
                        let ok = match output {
                            // 10-bit codes, a step either way for rounding.
                            Output::Pq => {
                                (0..3).all(|i| (got[i] - 1023.0 * pq(want[i])).abs() <= codes)
                            }
                            // 80 nits a unit, in half floats, which BT.2020 mixes.
                            Output::ScRgb => {
                                let top = want.into_iter().fold(0.0, f64::max);
                                let shown = to_bt2020(got.map(|c| c * 80.0));
                                (0..3).all(|i| (shown[i] - want[i]).abs() <= 2e-3 * top + 1e-4)
                            }
                        };
                        assert!(
                            ok,
                            "{output:?} black {black}, {colour:?} at {light}: {got:?}, \
                             not {want:?} BT.2020 nits"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_scaled_screenshot_shoots_at_its_size_and_leaves_the_window_its_own_targets() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = gpu() else {
            eprintln!("scaled screenshot GPU unavailable");
            return;
        };
        let mut hdr = Hdr::with_debug(&device, &queue, F::Rgba16Float, Output::Pq, true, None);
        let extent = |t: &wgpu::Texture| [t.width(), t.height()];
        hdr.frame(&device, [4, 2]);
        // `Renderer::draw_scaled_frame`: the frame at the screenshot's size, its SDR picture
        // and HDR10 copy at that size too, then the targets let go.
        assert_eq!(extent(&hdr.frame(&device, [8, 4])), [8, 4]);
        let mut encoder = device.create_command_encoder(&Default::default());
        let sdr = hdr
            .sdr_screenshot(&device, &mut encoder, F::Bgra8Unorm, false)
            .unwrap();
        let pq = hdr.capture(&device, &queue, &mut encoder).unwrap();
        queue.submit([encoder.finish()]);
        assert_eq!((extent(&sdr), extent(&pq)), ([8, 4], [8, 4]));
        hdr.release_targets();
        assert!(hdr.hud().is_none() && hdr.coverage_view().is_none());
        // The window's next frame makes its own again.
        assert_eq!(extent(&hdr.frame(&device, [4, 2])), [4, 2]);
        assert_eq!(extent(hdr.hud().unwrap()), [4, 2]);
    }

    #[test]
    fn sdr_screenshots_keep_the_world_up_to_white_and_the_hud_as_drawn() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = gpu() else {
            eprintln!("SDR screenshot GPU unavailable");
            return;
        };
        let mut hdr = Hdr::with_debug(&device, &queue, F::Rgba16Float, Output::Pq, true, None);
        // These display settings must not scale either the SDR world or the HUD.
        (hdr.paper_white, hdr.peak, hdr.black, hdr.ui) = (300.0, 1500.0, 0.2, 0.25);
        let shoot = |hdr: &mut Hdr, world: [f64; 3], hud: [f64; 3], c: f64, icon| {
            let frame = hdr.frame(&device, [1, 1]);
            let t = hdr.targets.as_ref().unwrap();
            let mut encoder = device.create_command_encoder(&Default::default());
            fill(&mut encoder, &frame, [world[0], world[1], world[2], 1.0]);
            fill(
                &mut encoder,
                t.hud.as_ref().unwrap(),
                [hud[0], hud[1], hud[2], 0.0],
            );
            fill(&mut encoder, t.coverage.as_ref().unwrap(), [c; 4]);
            // Read half-floats so 8-bit quantization cannot conceal the curve or RGB ratios.
            let copy = hdr
                .sdr_screenshot(&device, &mut encoder, F::Rgba16Float, icon)
                .unwrap();
            texel(&device, &queue, encoder, &copy)
        };
        // The world rises to white and stays there (within the division's rounding), its
        // channels in step.
        let mut last = 0.0;
        for i in 0..65 {
            let g = f64::from(i) / 16.0;
            let got = shoot(
                &mut hdr,
                [g, g / 2.0, 0.0],
                [g.min(1.0), (g / 2.0).min(1.0), 0.0],
                0.0,
                false,
            );
            assert!(got[0] >= last - 0.001 && got[0] <= 1.0, "{g}: {got:?}");
            assert_eq!(got[0] > 0.999, g >= 1.0, "{g}: {got:?}");
            assert!((got[0] - 2.0 * got[1]).abs() < 0.002 && got[2] == 0.0);
            last = got[0];
        }
        // Fully covered is exactly the HUD as its 8-bit target holds it, regardless of the
        // world or UI Brightness.
        let hud = [64.0, 128.0, 192.0].map(|v: f64| v / 255.0);
        assert_eq!(
            shoot(&mut hdr, [2.0, 1.0, 0.5], hud, 1.0, false),
            [hud[0], hud[1], hud[2], 1.0].map(|v| half::f16::from_f64(v).to_f32())
        );
        // At zero coverage the mapped world; the icon also ignores a fully opaque HUD.
        for (hud, c, icon) in [([1.0, 1.0, 0.5], 0.0, false), ([0.25; 3], 1.0, true)] {
            let got = shoot(&mut hdr, [2.0, 1.0, 0.5], hud, c, icon);
            for (g, expected) in got.into_iter().zip([1.0, 0.5, 0.25, 1.0]) {
                assert!((g - expected).abs() < 0.001, "{got:?}");
            }
        }
        // R8's 128/255 coverage: recover the HUD against the clipped backdrop, then
        // composite in gamma. The original HUD target and world's light stay intact.
        let got = shoot(
            &mut hdr,
            [2.0, 1.0, 0.5],
            [0.75, 0.875, 0.4375],
            128.0 / 255.0,
            false,
        );
        for (g, expected) in got.into_iter().zip([0.75, 0.626961, 0.31348, 1.0]) {
            assert!((g - expected).abs() < 0.002, "{got:?}");
        }
        let t = hdr.targets.as_ref().unwrap();
        let encoder = device.create_command_encoder(&Default::default());
        assert_eq!(
            texel(&device, &queue, encoder, &t.frame),
            [2.0, 1.0, 0.5, 1.0]
        );
        // The HUD's 8-bit target rounds what is drawn on it.
        let encoder = device.create_command_encoder(&Default::default());
        let stored = [191.0, 223.0, 112.0, 0.0].map(|v| v / 255.0);
        let hud = t.hud.as_ref().unwrap();
        assert_eq!(texel(&device, &queue, encoder, hud), stored);
        // With the Vanilla look or under a pack, Horizon lifts nothing: the screenshot is the
        // SDR picture, clipped at white.
        hdr.lighting = false;
        for gamma in [0.5f32, 0.9, 1.0] {
            // fs_keep reads the stored half-float world before writing the 8-bit HUD
            // backdrop. Quantize first here too: 0.9 rounds to a half-float below
            // the 8-bit rounding boundary; the original f32 can round to a different code.
            let w = [gamma, gamma / 2.0, 0.0].map(|v| half::f16::from_f32(v).to_f32());
            // Uncovered, the HUD's target holds the world as drawn; the icon ignores the HUD.
            let got = shoot(&mut hdr, w.map(f64::from), w.map(f64::from), 0.0, false);
            assert_eq!(got[..3], w.map(|v| half::f16::from_f32(v).to_f32()));
            let got = shoot(&mut hdr, w.map(f64::from), [0.25; 3], 1.0, true);
            assert_eq!(got[..3], w.map(|v| half::f16::from_f32(v).to_f32()));
        }
        // Partly covered, the 8-bit screenshot is the HUD's target as drawn.
        let got = shoot(
            &mut hdr,
            [1.0, 0.5, 0.0],
            [0.75, 0.875, 0.4375],
            128.0 / 255.0,
            false,
        );
        assert_eq!(
            got.map(|v| (v * 255.0).round()),
            [191.0, 223.0, 112.0, 255.0]
        );
    }

    /// The SDR screenshot's world for `world` in the frame, with nothing of the HUD over it,
    /// read as half-floats so 8 bits can't hide a difference.
    fn sdr_world(
        hdr: &mut Hdr,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        world: [f32; 3],
    ) -> [f32; 4] {
        let [r, g, b] = world.map(f64::from);
        let frame = hdr.frame(device, [1, 1]);
        let t = hdr.targets.as_ref().unwrap();
        let mut encoder = device.create_command_encoder(&Default::default());
        fill(&mut encoder, &frame, [r, g, b, 1.0]);
        fill(
            &mut encoder,
            t.hud.as_ref().unwrap(),
            [r.min(1.0), g.min(1.0), b.min(1.0), 0.0],
        );
        fill(&mut encoder, t.coverage.as_ref().unwrap(), [0.0; 4]);
        let copy = hdr
            .sdr_screenshot(device, &mut encoder, F::Rgba16Float, false)
            .unwrap();
        texel(device, queue, encoder, &copy)
    }

    #[test]
    fn an_sdr_screenshot_returns_white_and_below_exactly() {
        let Some((device, queue)) = gpu() else {
            eprintln!("SDR screenshot GPU unavailable");
            return;
        };
        let mut hdr = Hdr::with_debug(&device, &queue, F::Rgba16Float, Output::Pq, true, None);
        // Every half-float the frame can hold from 0.99 to white, and every 8-bit step below:
        // each comes back bit for bit, on every channel.
        let white = half::f16::from_f32(1.0).to_bits();
        let near = (half::f16::from_f32(0.99).to_bits()..=white).map(half::f16::from_bits);
        let below = (0..=252u16).map(|i| half::f16::from_f32(f32::from(i) / 255.0));
        for top in near.chain(below).map(half::f16::to_f32) {
            for w in [[top; 3], [top / 4.0, top, top / 2.0]] {
                let got = sdr_world(&mut hdr, &device, &queue, w);
                assert_eq!(got[..3], w, "{w:?}");
            }
        }
    }

    #[test]
    fn an_sdr_screenshot_keeps_the_hue_of_light_above_white_within_0_to_1() {
        let Some((device, queue)) = gpu() else {
            eprintln!("SDR screenshot GPU unavailable");
            return;
        };
        let mut hdr = Hdr::with_debug(&device, &queue, F::Rgba16Float, Output::Pq, true, None);
        // Stored colours with their brightest channel from just above white to far above the
        // scene's peak: the brightest lands on white and the others keep their share of it,
        // which is the hue in gamma as in linear light.
        for shares in [
            [1.0f32, 0.5, 0.0],
            [0.3, 1.0, 0.6],
            [0.05, 0.2, 1.0],
            [1.0; 3],
        ] {
            for top in [1.001f32, 1.25, 2.0, 4.0, 16.0, 64.0] {
                let got = sdr_world(&mut hdr, &device, &queue, shares.map(|s| s * top));
                assert!(got.iter().all(|c| (0.0..=1.0).contains(c)), "{got:?}");
                for (g, share) in got.iter().zip(shares) {
                    assert!((g - share).abs() < 0.001, "{shares:?} at {top}: {got:?}");
                }
            }
        }
    }

    #[test]
    fn the_hud_lays_over_the_worlds_light_by_its_coverage() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = gpu() else {
            return;
        };
        let mut hdr = Hdr::with_debug(&device, &queue, F::Rgba16Float, Output::Pq, true, None);
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: F::Rgba16Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        // The world's light, the HUD's target over it (on the world clipped to white), its
        // coverage, UI Brightness, and the composite, all in gamma as the frame stores them.
        for (world, hud, coverage, ui, want) in [
            // Nothing over it: the world, light above white included.
            (3.0, 1.0, 0.0, 1.0, 3.0),
            // Half-transparent black: half of the world shows, as the HUD blends.
            (2.0, 0.5, 0.5, 1.0, 1.0),
            // Covered: the HUD as drawn, at UI Brightness.
            (2.0, 0.3, 1.0, 1.0, 0.3),
            (2.0, 0.3, 1.0, 0.5, 0.3 * 0.5f32.powf(1.0 / 2.2)),
            // UI Brightness above paper white: brighter than drawn.
            (2.0, 0.3, 1.0, 2.0, 0.3 * 2f32.powf(1.0 / 2.2)),
            // Added over the world (the text highlight), which it doesn't hide.
            (0.6, 0.8, 0.0, 1.0, 0.8),
            // At or below white, at paper white: the HUD's picture as drawn.
            (0.7, 0.45, 0.5, 1.0, 0.45),
        ] {
            hdr.ui = ui;
            let frame = hdr.frame(&device, [1, 1]);
            let t = hdr.targets.as_ref().unwrap();
            let mut encoder = device.create_command_encoder(&Default::default());
            fill(&mut encoder, &frame, [world, world, world, 1.0]);
            fill(&mut encoder, t.hud.as_ref().unwrap(), [hud, hud, hud, 0.0]);
            fill(&mut encoder, t.coverage.as_ref().unwrap(), [coverage; 4]);
            hdr.begin(&queue, true);
            hdr.composite(&mut encoder, &target.create_view(&Default::default()), None);
            let got = texel(&device, &queue, encoder, &target);
            assert!(
                (got[0] - want).abs() < 0.01 && got[3] == 1.0,
                "{world} {hud} {coverage} {ui}: {got:?}"
            );
        }
    }

    /// The frame without the HUD's layer, a `shaders/` pack's SDR picture: all of it shows at
    /// paper white, the HUD it drew included, whatever its alpha and UI Brightness.
    #[test]
    fn a_frame_without_the_huds_layer_shows_at_paper_white() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = gpu() else {
            return;
        };
        let mut hdr = Hdr::with_debug(&device, &queue, F::Rgba8Unorm, Output::ScRgb, false, None);
        (hdr.paper_white, hdr.peak, hdr.ui) = (200.0, 300.0, 2.0);
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Output::ScRgb.format(),
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        // White and a mid grey (128 of 255), in nits.
        let grey = 128.0 / 255.0;
        for (colour, want) in [(1.0, 200.0), (grey, 200.0 * f64::powf(grey, 2.2))] {
            for alpha in [0.0, 0.3, 1.0] {
                let frame = hdr.frame(&device, [1, 1]);
                let mut encoder = device.create_command_encoder(&Default::default());
                fill(&mut encoder, &frame, [colour, colour, colour, alpha]);
                hdr.present(&queue, &mut encoder, &target, None);
                let got = texel(&device, &queue, encoder, &target);
                for c in &got[..3] {
                    assert!(
                        (f64::from(*c) * 80.0 - want).abs() < 0.5,
                        "{colour} alpha {alpha}: {got:?}, not {want} nits"
                    );
                }
            }
        }
    }

    #[test]
    fn the_menu_blur_keeps_the_light_it_blurs() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = gpu() else {
            return;
        };
        let mut hdr = Hdr::with_debug(&device, &queue, F::Rgba16Float, Output::Pq, true, None);
        let size = [8, 8];
        let frame = hdr.frame(&device, size);
        let mut encoder = device.create_command_encoder(&Default::default());
        fill(&mut encoder, &frame, [4.0, 4.0, 4.0, 1.0]);
        hdr.hud_backdrop(&mut encoder, None);
        super::super::hud_coverage::clear(&mut encoder, &hdr.coverage_view().unwrap());
        let mut blur = crate::menu_blur::MenuBlur::new(&device, F::Rgba16Float, false);
        let (_, source) = blur.source(&device, size);
        hdr.begin(&queue, true);
        hdr.composite(&mut encoder, &source, None);
        let view = frame.create_view(&Default::default());
        blur.blur(&device, &queue, &mut encoder, &view, size, 3.0, None);
        hdr.hud_backdrop(&mut encoder, None);
        let hud = hdr.hud().unwrap().clone();
        // The blurred world keeps its light, and the HUD's backdrop starts again from it.
        let [r, ..] = texel(&device, &queue, encoder, &frame);
        assert!((r - 4.0).abs() < 0.01, "{r}");
        let encoder = device.create_command_encoder(&Default::default());
        assert_eq!(texel(&device, &queue, encoder, &hud), [1.0, 1.0, 1.0, 0.0]);
    }

    #[test]
    fn hdr_shaders_validate() {
        let _gpu = crate::gfx::test_gpu::lock();
        let entity = super::super::entity_shader();
        for source in [
            &super::shader(),
            &super::super::terrain_shader(),
            &super::super::hdr_stats::shader(),
            &(super::super::wgsl_prefix::HDR_LIGHT.to_owned() + include_str!("stars.wgsl")),
            &entity,
        ] {
            let module = naga::front::wgsl::parse_str(source).unwrap();
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
        }
    }

    fn caps(offered: &[(F, S)]) -> wgpu::SurfaceCapabilities {
        wgpu::SurfaceCapabilities {
            format_capabilities: offered
                .iter()
                .map(|&(format, color_spaces)| wgpu::SurfaceFormatCapabilities {
                    format,
                    color_spaces,
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn hdr10_first_then_scrgb() {
        let _gpu = crate::gfx::test_gpu::lock();
        let both = caps(&[
            (F::Bgra8UnormSrgb, S::SRGB),
            (F::Rgb10a2Unorm, S::BT2100_PQ | S::BT2100_HLG),
            (F::Rgba16Float, S::EXTENDED_SRGB_LINEAR),
        ]);
        assert_eq!(pick(&both, None), Some(Output::Pq));
        assert_eq!(pick(&both, Some("scrgb")), Some(Output::ScRgb));
        let scrgb = caps(&[(F::Rgba16Float, S::EXTENDED_SRGB_LINEAR)]);
        assert_eq!(pick(&scrgb, None), Some(Output::ScRgb));
        assert_eq!(pick(&scrgb, Some("pq")), None);
        assert_eq!(pick(&caps(&[(F::Bgra8UnormSrgb, S::SRGB)]), None), None);
    }

    #[test]
    fn offscreen_offers_only_the_named_output() {
        let _gpu = crate::gfx::test_gpu::lock();
        assert_eq!(named(Some("pq")), Some(Output::Pq));
        assert_eq!(named(Some("scrgb")), Some(Output::ScRgb));
        assert_eq!(named(Some("hdr10")), None);
        assert_eq!(named(None), None);
    }
}

#[cfg(test)]
#[path = "hdr_written_tests.rs"]
mod written_tests;

#[cfg(test)]
#[path = "hdr_dither_tests.rs"]
mod dither_tests;
