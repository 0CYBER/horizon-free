//! The GLSL → WGSL translation on hand-written compatibility-profile
//! programs, checked through naga's IR (locations, bindings, uniform block),
//! and on real packs when `HORIZON_TEST_SHADERPACKS` names a folder of them.

use horizon_shaderpack::{
    JavaPack, Stage,
    include::Inlined,
    preprocess::{Environment, preprocess},
    program_name,
    translate::{Link, Translated, translate},
};
use std::collections::{BTreeMap, BTreeSet};

fn env() -> Environment {
    Environment::iris("Apple", "Apple M4 Pro", 4)
}

fn run(stage: Stage, source: &str, link: &mut Link) -> Result<Translated, String> {
    let inlined = Inlined {
        text: source.to_owned(),
        origin: Vec::new(),
    };
    let pre = preprocess(&inlined, &env())?;
    translate(stage, &pre.text, link, &|line| format!("line {line}"))
}

/// `(name, location)` of an entry point's inputs (`result: false`) or outputs.
fn io(t: &Translated, result: bool) -> BTreeMap<String, u32> {
    let f = &t.module.entry_points[0].function;
    let mut out = BTreeMap::new();
    let mut add =
        |name: Option<&String>, binding: &Option<naga::Binding>, ty: naga::Handle<naga::Type>| {
            match (binding, &t.module.types[ty].inner) {
                (Some(naga::Binding::Location { location, .. }), _) => {
                    out.insert(name.cloned().unwrap_or_default(), *location);
                }
                (None, naga::TypeInner::Struct { members, .. }) => {
                    for m in members {
                        if let Some(naga::Binding::Location { location, .. }) = &m.binding {
                            out.insert(m.name.clone().unwrap_or_default(), *location);
                        }
                    }
                }
                _ => {}
            }
        };
    if result {
        if let Some(r) = &f.result {
            add(None, &r.binding, r.ty);
        }
    } else {
        for a in &f.arguments {
            add(a.name.as_ref(), &a.binding, a.ty);
        }
    }
    out
}

/// Group 0 bindings of the module's resources, by variable name.
fn bindings(t: &Translated) -> BTreeMap<String, u32> {
    t.module
        .global_variables
        .iter()
        .filter_map(|(_, v)| Some((v.name.clone()?, v.binding.as_ref()?.binding)))
        .collect()
}

/// The uniform block's member names, in order.
fn block_members(t: &Translated) -> Vec<String> {
    t.module
        .global_variables
        .iter()
        .find(|(_, v)| v.space == naga::AddressSpace::Uniform)
        .map(|(_, v)| match &t.module.types[v.ty].inner {
            naga::TypeInner::Struct { members, .. } => {
                members.iter().filter_map(|m| m.name.clone()).collect()
            }
            _ => Vec::new(),
        })
        .unwrap_or_default()
}

const VSH: &str = r#"
#version 120
attribute vec4 mc_Entity;
uniform mat4 gbufferModelView;
uniform bool hasSkylight;
varying vec2 texcoord;
varying vec4 color;
varying mat3 tbn;
varying float entity;
void main() {
    gl_Position = ftransform();
    texcoord = (gl_TextureMatrix[0] * gl_MultiTexCoord0).xy;
    color = gl_Color;
    gl_FrontColor = gl_Color;
    tbn = mat3(gbufferModelView) * gl_NormalMatrix;
    entity = hasSkylight ? mc_Entity.x : 0.0;
}
"#;

const FSH: &str = r#"
#version 120
uniform sampler2D texture;
uniform sampler2DShadow shadowtex0;
uniform float frameTimeCounter;
varying vec2 texcoord;
varying vec4 color;
varying mat3 tbn;
float phase = frameTimeCounter * 2.0;
const float weights[] = float[](0.25, 0.5, 0.25);
vec4 sampleTwice(sampler2D tex, vec2 uv) {
    return texture2D(tex, uv) + texture2D(tex, uv + vec2(phase));
}
float shade(vec3 p) {
    float texture = shadow2D(shadowtex0, p).x;
    return texture * weights[1];
}
void main() {
    vec4 albedo = sampleTwice(texture, texcoord) * color;
    if (isnan(albedo.r)) {
        discard;
        albedo = vec4(0.0);
    }
    gl_FragData[0] = albedo * shade(tbn[2]) * gl_Color;
    gl_FragData[1] = vec4(texture(shadowtex0, vec3(texcoord, 0.5)).x);
}
"#;

#[test]
fn translates_a_compatibility_profile_pass() {
    let mut link = Link::default();
    let vs = run(Stage::Vertex, VSH, &mut link).unwrap();
    let fs = run(Stage::Fragment, FSH, &mut link).unwrap();

    // The varyings meet at the same locations, the mat3 as three columns.
    let outs = io(&vs, true);
    let ins = io(&fs, false);
    for name in [
        "texcoord",
        "color",
        "tbn_iris0",
        "tbn_iris1",
        "tbn_iris2",
        "iris_FrontColor",
    ] {
        assert_eq!(
            outs.get(name),
            ins.get(name),
            "{name}: vs {outs:?} fs {ins:?}"
        );
        assert!(outs.contains_key(name), "{name} in {outs:?}");
    }
    assert!(outs.contains_key("entity") && !ins.contains_key("entity"));
    // Attributes: the pack's, then the compatibility ones it reads.
    let attributes: Vec<String> = io(&vs, false).into_keys().collect();
    assert_eq!(
        attributes,
        [
            "iris_Color",
            "iris_MultiTexCoord0",
            "iris_Vertex",
            "mc_Entity"
        ]
    );
    // gl_FragData[n] are the outputs at location n.
    assert_eq!(
        fs.interface.outputs.keys().copied().collect::<Vec<_>>(),
        [0, 1]
    );
    assert_eq!(
        io(&fs, true).values().copied().collect::<BTreeSet<_>>(),
        [0, 1].into()
    );

    // One block whose members extend across the pass: the fragment program's
    // starts with the vertex program's, so offsets agree.
    let vs_block = block_members(&vs);
    let fs_block = block_members(&fs);
    assert_eq!(
        vs_block,
        [
            "gbufferModelView",
            "iris_bool_hasSkylight",
            "iris_ModelViewProjectionMatrix",
            "iris_TextureMatrix",
            "iris_NormalMatrix"
        ]
    );
    assert_eq!(fs_block[..vs_block.len()], vs_block[..]);
    assert_eq!(fs_block[vs_block.len()..], ["frameTimeCounter"]);

    // Samplers split into texture + sampler bindings, shared by the pass.
    let b = bindings(&fs);
    let block = fs
        .module
        .global_variables
        .iter()
        .find(|(_, v)| v.space == naga::AddressSpace::Uniform)
        .unwrap()
        .1;
    assert_eq!(
        block.binding.as_ref().map(|b| (b.group, b.binding)),
        Some((0, 0))
    );
    for name in ["texture_t", "texture_s", "shadowtex0_t", "shadowtex0_s"] {
        assert_eq!(b[name], link.bindings[name], "{name}");
    }
    assert_eq!(
        fs.interface.samplers,
        [
            ("texture".into(), "sampler2D".into()),
            ("shadowtex0".into(), "sampler2DShadow".into())
        ]
    );
    // A local named `texture` hides the sampler; the sampler parameter is split.
    assert!(
        fs.glsl.contains(
            "float texture = vec4(texture(sampler2DShadow(shadowtex0_t, shadowtex0_s), p)).x;"
        ),
        "{}",
        fs.glsl
    );
    assert!(
        fs.glsl
            .contains("vec4 sampleTwice(texture2D tex_t, sampler tex_s, vec2 uv)"),
        "{}",
        fs.glsl
    );
    assert!(
        fs.glsl
            .contains("sampleTwice(texture_t, texture_s, texcoord)"),
        "{}",
        fs.glsl
    );
    // Global initialisers run first; the unsized array is sized.
    assert!(
        fs.glsl.contains("phase = frameTimeCounter * 2.;"),
        "{}",
        fs.glsl
    );
    assert!(
        fs.glsl
            .contains("const float weights[3] = float[3](0.25, 0.5, 0.25);"),
        "{}",
        fs.glsl
    );
    // The code after `discard` is gone, isnan is a helper.
    assert!(!fs.glsl.contains("albedo = vec4(0.);"), "{}", fs.glsl);
    assert!(fs.glsl.contains("iris_isnan(albedo.r)"), "{}", fs.glsl);
    assert!(fs.wgsl.contains("@fragment"));
    // GL's clip space becomes wgpu's after the program's own `main`.
    assert!(
        vs.glsl
            .contains("gl_Position.y = -gl_Position.y;\n    gl_Position.z = (gl_Position.z + gl_Position.w) * 0.5;\n}"),
        "{}",
        vs.glsl
    );
}

#[test]
fn reports_errors_with_their_place() {
    let mut link = Link::default();
    let err = run(
        Stage::Fragment,
        "void main() {\n  float x = ;\n}\n",
        &mut link,
    )
    .err()
    .unwrap();
    assert!(err.starts_with("line 2:"), "{err}");
    let err = run(Stage::Geometry, "void main() {}", &mut link)
        .err()
        .unwrap();
    assert_eq!(err, "Geometry programs are not supported");
    let mut link = Link::default();
    run(
        Stage::Vertex,
        "uniform float a;\nvoid main() { gl_Position = vec4(a); }",
        &mut link,
    )
    .unwrap();
    let err = run(
        Stage::Fragment,
        "uniform int a;\nvoid main() { gl_FragColor = vec4(a); }",
        &mut link,
    )
    .err()
    .unwrap();
    assert_eq!(err, "uniform a is declared as both `float a` and `int a`");
}

/// What naga refuses and drivers accept: `vec % scalar` (Noble), `sqrt` of
/// a negative constant (Bloop's NaN), constants naga can't fold (Bliss's
/// `?:`, `mix`), 16-bit types (Noble on NVIDIA), `mat2(vec4)` (Bloop),
/// `1 << 31` (Rethinking Voxels), vertex `texture2D` without LOD (Bliss),
/// nested array varyings (Noble), and struct varyings (Photon).
#[test]
fn legalises_what_naga_refuses() {
    let mut link = Link::default();
    let vs = run(
        Stage::Vertex,
        "uniform sampler2D colortex0;
struct CloudsParameters {
    vec3 color;
    float density;
};
varying CloudsParameters clouds;
varying vec3 grid[2][2];
void main() {
    vec4 s = texture2D(colortex0, vec2(0.5));
    clouds.color = s.rgb;
    clouds.density = s.a;
    grid[0][0] = s.rgb;
    grid[0][1] = s.gbr;
    grid[1][0] = s.brg;
    grid[1][1] = vec3(1.0);
    gl_Position = vec4(0.0, 0.0, 0.0, 1.0);
}
",
        &mut link,
    )
    .unwrap();
    reparse(&vs.wgsl).unwrap();

    let fs = run(
        Stage::Fragment,
        "uniform vec2 uv;
struct CloudsParameters {
    vec3 color;
    float density;
};
varying CloudsParameters clouds;
varying vec3 grid[2][2];
const int RES = 256 % 100;
const int SIGN_BIT = 1 << 31;
const vec3 TARGET = length(vec3(1.0)) < 0.001 ? vec3(0.0) : normalize(vec3(1.0));
const vec3 TINT = mix(TARGET, vec3(1.0), 0.5);
const vec3 HALF = TINT * 0.5;
u16vec2 pack(uint v) { return u16vec2(v & 65535u, v >> 16u); }
float nan() { return sqrt(-1.0); }
void main() {
    ivec2 texel = ivec2(uv) % RES;
    texel %= 7;
    uint packed = packFloat2x16(f16vec2(HALF.xy));
    mat2 rot = mat2(vec4(1.0, 0.0, 0.0, 1.0));
    vec2 r = rot * vec2(clouds.density, float(SIGN_BIT != 0));
    gl_FragColor = vec4(vec2(texel) + r + clouds.color.xy + grid[1][0].xy, float(pack(packed).x), nan());
}
",
        &mut link,
    )
    .unwrap();
    for wanted in [
        "const int RES = 256 % 100;",
        "vec3 TARGET;",
        "vec3 TINT;",
        "vec3 HALF;",
        "ivec2 texel = iris_mod(ivec2(uv), RES);",
        "texel = iris_mod(texel, 7);",
        "uvec2 pack(uint v)",
        "return iris_sqrt(-1.);",
        "uint packed = packHalf2x16(vec2(HALF.xy));",
        "iris_mat2(",
    ] {
        assert!(fs.glsl.contains(wanted), "{wanted}\n{}", fs.glsl);
    }
    reparse(&fs.wgsl).unwrap();
}

/// A fragment output narrower than its four-channel target (Steadfast's
/// `final` writes `out vec3 color`) is written through a `vec4` with alpha 1:
/// GL fills the missing channels, wgpu refuses the pipeline.
#[test]
fn narrow_fragment_outputs_fill_four_channels() {
    let mut link = Link::default();
    let fs = run(
        Stage::Fragment,
        "uniform vec3 tint;
layout(location = 0) out vec3 color;
layout(location = 1) out float depth;
out vec4 extra;
void main() {
    color = tint;
    depth = 0.5;
    extra = vec4(tint, 0.25);
}
",
        &mut link,
    )
    .unwrap();
    for wanted in [
        "layout(location = 0) out vec4 iris_out_color;",
        "iris_out_color = vec4(color, 1.0);",
        "layout(location = 1) out vec4 iris_out_depth;",
        "iris_out_depth = vec4(depth, 0.0, 0.0, 1.0);",
        "layout(location = 2) out vec4 extra;",
    ] {
        assert!(fs.glsl.contains(wanted), "{wanted}\n{}", fs.glsl);
    }
    assert_eq!(
        fs.interface.outputs.values().collect::<Vec<_>>(),
        ["color", "depth", "extra"]
    );
    let module = naga::front::wgsl::parse_str(&fs.wgsl).unwrap();
    let entry = &module.entry_points[0];
    let result = entry.function.result.as_ref().unwrap();
    let naga::TypeInner::Struct { members, .. } = &module.types[result.ty].inner else {
        panic!("{}", fs.wgsl);
    };
    for m in members {
        assert!(
            matches!(
                module.types[m.ty].inner,
                naga::TypeInner::Vector {
                    size: naga::VectorSize::Quad,
                    ..
                }
            ),
            "{}",
            fs.wgsl
        );
    }
    reparse(&fs.wgsl).unwrap();
}

/// The written WGSL, read back and validated: what wgpu is given.
fn reparse(wgsl: &str) -> Result<(), String> {
    let module = naga::front::wgsl::parse_str(wgsl).map_err(|e| e.emit_to_string(wgsl))?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map(|_| ())
    .map_err(|e| e.emit_to_string(wgsl))
}

/// Every program the packs in `HORIZON_TEST_SHADERPACKS` enable by default
/// (`program.*.enabled` in their shaders.properties)
/// translates to WGSL that reads back (each pass's `.vsh` before its `.fsh`). Skipped when the
/// variable isn't set. Distant Horizons, Colorwheel and Voxy programs are
/// left out: Iris loads them only with those mods.
#[test]
fn real_packs_translate() {
    let Some(dir) = std::env::var_os("HORIZON_TEST_SHADERPACKS") else {
        return;
    };
    let mut failures = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let Ok(pack) = JavaPack::open(&entry.path()) else {
            continue;
        };
        let values = pack.values(&BTreeMap::new());
        let properties = pack.properties(&values, &env()).unwrap();
        let mut passes: BTreeMap<&str, Vec<&String>> = BTreeMap::new();
        for p in &pack.programs {
            let file = p.rsplit('/').next().unwrap();
            if ["dh_", "clrwl_", "voxy"]
                .iter()
                .any(|m| file.starts_with(m))
                || !properties.program_enabled(program_name(p), &pack.options, &values)
            {
                continue;
            }
            passes
                .entry(p.rsplit_once('.').unwrap().0)
                .or_default()
                .push(p);
        }
        let (mut ok, mut total) = (0, 0);
        for programs in passes.values_mut() {
            programs.sort_by_key(|p| !p.ends_with(".vsh"));
            let mut link = Link {
                image_formats: properties.image_formats(),
                ..Link::default()
            };
            for p in programs.iter() {
                total += 1;
                match pack
                    .translate(p, &values, &env(), &mut link)
                    .and_then(|t| reparse(&t.wgsl))
                {
                    Ok(()) => ok += 1,
                    Err(e) => {
                        failures.push(format!("{}: {e}", entry.file_name().to_string_lossy()))
                    }
                }
            }
        }
        println!(
            "{}: {ok}/{total} programs translated",
            entry.file_name().to_string_lossy()
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn textures_share_samplers_by_name() {
    let mut link = Link {
        shared_samplers: [("colortex0", "iris_linear"), ("colortex1", "iris_linear")]
            .map(|(a, b)| (a.to_owned(), b.to_owned()))
            .into(),
        ..Link::default()
    };
    let fs = run(
        Stage::Fragment,
        "uniform sampler2D colortex0;\nuniform sampler2D colortex1;\nuniform sampler2D custom;\n\
         uniform sampler2DShadow shadowtex0;\nvarying vec2 uv;\n\
         void main() { gl_FragColor = texture2D(colortex0, uv) + texture2D(colortex1, uv)\n\
         + texture2D(custom, uv) + shadow2D(shadowtex0, vec3(uv, 0.5)); }",
        &mut link,
    )
    .unwrap();
    assert_eq!(
        fs.glsl.matches("uniform sampler iris_linear;").count(),
        1,
        "{}",
        fs.glsl
    );
    assert!(
        fs.glsl.contains("sampler2D(colortex1_t, iris_linear)"),
        "{}",
        fs.glsl
    );
    assert!(fs.glsl.contains("uniform sampler custom_s;"), "{}", fs.glsl);
    assert!(
        fs.glsl.contains("uniform samplerShadow iris_compare;"),
        "{}",
        fs.glsl
    );
    assert_eq!(fs.interface.sampler_vars["colortex0"], "iris_linear");
    // Three sampler bindings for four textures.
    assert_eq!(
        bindings(&fs).keys().filter(|k| !k.ends_with("_t")).count(),
        3,
        "{:?}",
        bindings(&fs)
    );
}
