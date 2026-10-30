//! The stages a pack can fill, each a WGSL function with a fixed signature
//! that Horizon calls inside its own passes, and the WGSL put around a
//! pack's files to build a stage: the prelude before (the inputs and the
//! resources), the entry points after.
use crate::inputs::{Frame, MATERIAL_INPUT, POST_INPUT, SKY_INPUT, SURFACE, Struct};
use crate::package::Pack;

/// The bind group of a pack's resources; groups 0 to 2 are the renderer's.
pub const GROUP: u32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageKind {
    /// The sky's colour along each view ray, before the sun, moon and stars.
    Sky,
    /// The colour of block surfaces (terrain), before the alpha test and fog.
    Material,
    /// A full-screen pass over the world's picture, under the hand and HUD.
    Post,
}

/// A resource a stage reads, declared by its prelude.
#[derive(Debug)]
pub struct Resource {
    pub binding: u32,
    pub name: &'static str,
    pub ty: &'static str,
    pub doc: &'static str,
}

const FRAME: Resource = Resource {
    binding: 0,
    name: "frame",
    ty: "var<uniform> frame: Frame",
    doc: "The frame (`Frame`).",
};

const POST_RESOURCES: [Resource; 6] = [
    FRAME,
    Resource {
        binding: 1,
        name: "scene_color",
        ty: "var scene_color: texture_2d<f32>",
        doc: "The world's picture before any pass.",
    },
    Resource {
        binding: 2,
        name: "scene_depth",
        ty: "var scene_depth: texture_depth_2d",
        doc: "The world's depth, reversed (1 near, 0 far and sky).",
    },
    Resource {
        binding: 3,
        name: "previous",
        ty: "var previous: texture_2d<f32>",
        doc: "The previous pass's output (`scene_color` for the first pass).",
    },
    Resource {
        binding: 4,
        name: "linear_sampler",
        ty: "var linear_sampler: sampler",
        doc: "Bilinear filtering, clamped to the edges.",
    },
    Resource {
        binding: 5,
        name: "nearest_sampler",
        ty: "var nearest_sampler: sampler",
        doc: "Nearest texel, clamped to the edges.",
    },
];

/// Functions the post prelude gives packs.
const POST_HELPERS: &str = "\
// The world's depth at `uv` (reversed: 0 is the sky).
fn depth_at(uv: vec2<f32>) -> f32 {
    let size = vec2<f32>(textureDimensions(scene_depth));
    let texel = vec2<i32>(clamp(uv * size, vec2<f32>(0.0), size - 1.0));
    return textureLoad(scene_depth, texel, 0);
}
// The camera-relative world position, in blocks, of the point at `uv` and `depth`.
fn view_position(uv: vec2<f32>, depth: f32) -> vec3<f32> {
    let clip = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
    let view = frame.projection_inverse * clip;
    return (frame.view_inverse * vec4<f32>(view.xyz / view.w, 0.0)).xyz;
}
";

/// The post stage's entry points, as the renderer builds them: a full-screen
/// triangle, and a fragment that calls the pack's `post` for each pixel.
const POST_ENTRIES: &str = "\
struct HorizonPostOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}
@vertex fn horizon_vs_post(@builtin(vertex_index) i: u32) -> HorizonPostOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: HorizonPostOut;
    out.position = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(p.x, 1.0 - p.y);
    return out;
}
@fragment fn horizon_fs_post(in: HorizonPostOut) -> @location(0) vec4<f32> {
    let color = textureSampleLevel(previous, linear_sampler, in.uv, 0.0);
    return post(PostInput(in.uv, in.position.xy, color, depth_at(in.uv)));
}
";

/// Entry points with the material stage's inputs: the renderer calls
/// `material` from its own terrain shader in the same way.
const MATERIAL_ENTRIES: &str = "\
struct HorizonMaterialIn {
    @builtin(position) position: vec4<f32>,
    @location(0) albedo: vec4<f32>,
    @location(1) tint: vec4<f32>,
    @location(2) light: vec2<f32>,
    @location(3) lightmap: vec3<f32>,
    @location(4) world: vec3<f32>,
    @location(5) uv: vec2<f32>,
    @location(6) @interpolate(flat) translucent: u32,
}
@fragment fn horizon_fs_material(in: HorizonMaterialIn) -> @location(0) vec4<f32> {
    let input = MaterialInput(in.albedo, in.tint, in.light, in.lightmap, in.world, in.uv, in.translucent != 0u);
    return material(input).color;
}
";

/// Entry points with the sky stage's inputs: the renderer calls `sky` from
/// its own sky shader in the same way.
const SKY_ENTRIES: &str = "\
struct HorizonSkyIn {
    @builtin(position) position: vec4<f32>,
    @location(0) direction: vec3<f32>,
    @location(1) color: vec4<f32>,
}
@fragment fn horizon_fs_sky(in: HorizonSkyIn) -> @location(0) vec4<f32> {
    return sky(SkyInput(normalize(in.direction), in.color));
}
";

impl StageKind {
    pub const ALL: [StageKind; 3] = [StageKind::Sky, StageKind::Material, StageKind::Post];

    /// The manifest's name for the stage (`[sky]`, `[material]`, `[[post]]`).
    pub fn name(self) -> &'static str {
        match self {
            StageKind::Sky => "sky",
            StageKind::Material => "material",
            StageKind::Post => "post",
        }
    }

    /// The function a pack defines: it has the stage's name.
    pub fn function(self) -> &'static str {
        self.name()
    }

    pub fn input(self) -> &'static Struct {
        match self {
            StageKind::Sky => &SKY_INPUT,
            StageKind::Material => &MATERIAL_INPUT,
            StageKind::Post => &POST_INPUT,
        }
    }

    /// The type the function returns.
    pub fn output(self) -> &'static str {
        match self {
            StageKind::Material => SURFACE.name,
            StageKind::Sky | StageKind::Post => "vec4<f32>",
        }
    }

    /// `fn <name>(input: <Input>) -> <Output>`.
    pub fn signature(self) -> String {
        format!(
            "fn {}(input: {}) -> {}",
            self.function(),
            self.input().name,
            self.output()
        )
    }

    pub fn resources(self) -> &'static [Resource] {
        match self {
            StageKind::Post => &POST_RESOURCES,
            StageKind::Sky | StageKind::Material => std::slice::from_ref(&FRAME),
        }
    }

    /// Everything the pack's files may use, declared before them.
    pub fn prelude(self) -> String {
        let mut out =
            String::from("// Horizon shader API prelude (docs/api/shaders-reference.md)\n");
        out += &Frame::WGSL.wgsl();
        out += &self.input().wgsl();
        if self == StageKind::Material {
            out += &SURFACE.wgsl();
        }
        for r in self.resources() {
            out += &format!("@group({GROUP}) @binding({}) {};\n", r.binding, r.ty);
        }
        if self == StageKind::Post {
            out += POST_HELPERS;
        }
        out
    }

    /// The entry points put after the pack's files.
    pub fn entries(self) -> &'static str {
        match self {
            StageKind::Sky => SKY_ENTRIES,
            StageKind::Material => MATERIAL_ENTRIES,
            StageKind::Post => POST_ENTRIES,
        }
    }
}

/// A stage's WGSL, and where each part of it came from.
#[derive(Debug)]
pub struct Composed {
    pub text: String,
    /// (the file, or `<prelude>` / `<entries>`, its byte range in `text`).
    pub parts: Vec<(String, std::ops::Range<usize>)>,
}

impl Composed {
    /// The file, line and column (1-based) of a byte offset in `text`.
    pub fn locate(&self, offset: usize) -> (&str, usize, usize) {
        let (file, range) = self
            .parts
            .iter()
            .find(|(_, r)| r.contains(&offset))
            .or(self.parts.last())
            .expect("a composed stage has parts");
        let before = &self.text[range.start..offset.clamp(range.start, range.end)];
        let line = before.matches('\n').count() + 1;
        let column = before.len() - before.rfind('\n').map_or(0, |i| i + 1) + 1;
        (file, line, column)
    }
}

/// The WGSL of the stage `kind` whose own file is `file`: the prelude, the
/// pack's shared files in order, the file, then the entry points.
pub fn compose(pack: &Pack, kind: StageKind, file: &str) -> Composed {
    let mut composed = Composed {
        text: String::new(),
        parts: Vec::new(),
    };
    let mut push = |name: &str, text: &str| {
        let start = composed.text.len();
        composed.text += text;
        if !text.ends_with('\n') {
            composed.text.push('\n');
        }
        composed
            .parts
            .push((name.to_owned(), start..composed.text.len()));
    };
    push("<prelude>", &kind.prelude());
    for shared in pack.manifest.shared.iter().filter(|s| *s != file) {
        push(shared, pack.source(shared).unwrap_or_default());
    }
    push(file, pack.source(file).unwrap_or_default());
    push("<entries>", kind.entries());
    composed
}
