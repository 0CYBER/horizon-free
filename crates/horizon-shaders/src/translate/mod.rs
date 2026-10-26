//! A program's preprocessed GLSL (any version from 110 to 460, compatibility or core) turned
//! into the Vulkan-flavoured GLSL 450 naga reads, then into validated naga modules (spec 1.6,
//! 2.2). The rewrite works on preprocessing tokens and never adds or removes a line of the pack's
//! code, so naga's line numbers map back through the preprocessor to the pack's files.
//!
//! What it does:
//! - Loose uniforms go into one uniform block shared by both stages (`bool` stored as `int`).
//! - Each sampler is split into a texture and a sampler object, combined again where it is used
//!   (`sampler2D(t, s)`), or passed as two arguments to the pack's own functions.
//! - The compatibility built-ins (`gl_Vertex`, `gl_MultiTexCoordN`, `gl_*Matrix`, `ftransform()`,
//!   `gl_FragData`, `gl_FragColor`, `gl_TexCoord`, `texture2D`...) become declarations of ours,
//!   named with the reserved prefix `hz_`; varyings get locations by name across the stages.
//! - The pack's `main` becomes `hz_main`, called by a `main` of ours that sets the inputs and
//!   adapts GL's conventions (spec 5.5): clip y is negated for every target but the window, so
//!   memory rows are GL's rows and `gl_FragCoord` needs no change, and clip z goes from GL's
//!   [-w, w] to [0, w], so stored depth is GL's window depth.
//!
//! Composite-style programs draw one quad from (0,0) to (1,1) with an identity model-view and an
//! orthographic projection onto it, the texture coordinate equal to the position (OQ-6).

use std::collections::BTreeMap;
use std::fmt::Write;

use naga::ShaderStage;
use naga::front::glsl::{Frontend, Options};
use naga::valid::{Capabilities, ValidationFlags, Validator};

use crate::Preprocessed;

mod rewrite;
mod scan;

use scan::Stage;

/// What a program's stages need from the game.
#[derive(Debug, Default, PartialEq)]
pub struct Interface {
    /// The uniform block at binding 0: its size and each value's name, GLSL type and offset.
    /// Ours come first: `hz_target` (the target's width, height, 1/width, 1/height), and the
    /// fixed-function fog the compatibility `gl_Fog` reads (OQ-28): `hz_fog_color` (vanilla's fog
    /// colour) and `hz_fog` (density, start, end, 1 / (end - start)).
    pub uniform_size: u32,
    pub uniforms: Vec<Uniform>,
    /// Each sampler: texture at `binding`, sampler object at `binding + 1`.
    pub samplers: Vec<Sampler>,
    /// The fragment stage's output locations.
    pub outputs: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Uniform {
    pub name: String,
    /// The type the pack declared (`float`, `ivec2`, `mat4`; `bool` is stored as an `int`).
    pub ty: String,
    pub offset: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sampler {
    pub name: String,
    /// The type the pack declared: `sampler2D`, `sampler2DShadow`, `usampler2D`, `sampler3D`...
    pub ty: String,
    pub binding: u32,
}

/// A program that failed, at a pack file and line.
#[derive(Clone, Debug, PartialEq)]
pub struct Error {
    pub file: String,
    pub line: u32,
    pub message: String,
}

pub struct Program {
    pub vertex: naga::Module,
    pub fragment: naga::Module,
    pub interface: Interface,
}

/// Where the program draws, which decides the conventions of 5.5.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    /// Pack buffers: rows stored as GL stores them.
    Buffers,
    /// The window (`final`).
    Window,
}

/// What a gbuffers-style program's vertex stage reads. The game writes it: it knows its vertex
/// formats, the translator only places it.
#[derive(Clone, Debug, Default)]
pub struct GbuffersInputs {
    /// GLSL declarations: the vertex inputs and any bindings of set 1.
    pub declarations: String,
    /// GLSL statements that set `hz_Vertex`, `hz_Color`, `hz_MultiTexCoord0`,
    /// `hz_MultiTexCoord1` and `hz_Normal` (the compatibility inputs), run first in `main`.
    pub body: String,
    /// The value of each attribute a pack may declare (`mc_Entity`, `vaPosition`...), as a GLSL
    /// expression of the declared type's size; an attribute not listed reads zero.
    pub attributes: Vec<(String, String)>,
}

/// Translates a composite-style program (`composite`, `deferred`, `final`...).
pub fn translate_composite(
    vertex: &Preprocessed,
    fragment: &Preprocessed,
    target: Target,
) -> Result<Program, Vec<Error>> {
    translate(vertex, fragment, target, None)
}

/// Translates a gbuffers-style program (`gbuffers_terrain`...) drawing the game's geometry
/// described by `inputs`, into `target` (the pack's buffers).
pub fn translate_gbuffers(
    vertex: &Preprocessed,
    fragment: &Preprocessed,
    target: Target,
    inputs: &GbuffersInputs,
) -> Result<Program, Vec<Error>> {
    translate(vertex, fragment, target, Some(inputs))
}

/// The matrices of the compatibility profile in a gbuffers-style program, from the frame's.
const GBUFFERS_MATRICES: &str = "    hz_ModelViewMatrix = gbufferModelView;\n    \
     hz_ProjectionMatrix = gbufferProjection;\n    \
     hz_ModelViewProjectionMatrix = gbufferProjection * gbufferModelView;\n    \
     hz_ModelViewMatrixInverse = gbufferModelViewInverse;\n    \
     hz_ProjectionMatrixInverse = gbufferProjectionInverse;\n    \
     hz_NormalMatrix = mat3(gbufferModelView);\n    \
     mat4 hz_light = mat4(vec4(1.0 / 256.0, 0.0, 0.0, 0.0), vec4(0.0, 1.0 / 256.0, 0.0, 0.0), vec4(0.0, 0.0, 1.0, 0.0), vec4(1.0 / 32.0, 1.0 / 32.0, 0.0, 1.0));\n    \
     hz_TextureMatrix = mat4[8](mat4(1.0), hz_light, hz_light, mat4(1.0), mat4(1.0), mat4(1.0), mat4(1.0), mat4(1.0));\n";

fn translate(
    vertex: &Preprocessed,
    fragment: &Preprocessed,
    target: Target,
    gbuffers: Option<&GbuffersInputs>,
) -> Result<Program, Vec<Error>> {
    let mut v = Stage::scan(vertex, ShaderStage::Vertex);
    let mut f = Stage::scan(fragment, ShaderStage::Fragment);
    v.gbuffers = gbuffers.is_some();
    f.gbuffers = gbuffers.is_some();
    let mut errors: Vec<Error> = v.errors.drain(..).chain(f.errors.drain(..)).collect();

    // One uniform block and one set of samplers for both stages.
    let mut uniforms: BTreeMap<String, String> = BTreeMap::new();
    if gbuffers.is_some() {
        for name in [
            "gbufferModelView",
            "gbufferModelViewInverse",
            "gbufferProjection",
            "gbufferProjectionInverse",
        ] {
            uniforms.insert(name.to_owned(), "mat4".to_owned());
        }
    }
    let mut samplers: BTreeMap<String, String> = BTreeMap::new();
    for stage in [&v, &f] {
        for (name, ty, at) in &stage.uniforms {
            if let Some(old) = uniforms.insert(name.clone(), ty.clone())
                && &old != ty
            {
                errors.push(stage.error(
                    *at,
                    &format!("`{name}` is `{old}` in one stage and `{ty}` in the other"),
                ));
            }
        }
        for (name, ty) in &stage.samplers {
            samplers.insert(name.clone(), ty.clone());
        }
    }
    let samplers: Vec<Sampler> = samplers
        .into_iter()
        .enumerate()
        .map(|(i, (name, ty))| Sampler {
            name,
            ty,
            binding: 1 + 2 * i as u32,
        })
        .collect();

    // Varyings by name: the vertex outputs, then fragment inputs the vertex stage lacks.
    let mut locations: BTreeMap<String, (u32, String)> = BTreeMap::new();
    for var in v.outs.iter().chain(&f.ins) {
        let next = locations.len() as u32;
        locations
            .entry(var.name.clone())
            .or_insert((next, var.decl.clone()));
    }

    let header = |stage: &Stage| {
        let mut h = String::from("#version 450\n");
        h.push_str("layout(set = 0, binding = 0) uniform HzUniforms { vec4 hz_target; vec4 hz_fog_color; vec4 hz_fog;");
        for (name, ty) in &uniforms {
            match ty.as_str() {
                "bool" => write!(h, " int hz_bool_{name};"),
                _ => match ty.split_once('[') {
                    Some((base, size)) => write!(h, " {base} {name}[{size};"),
                    None => write!(h, " {ty} {name};"),
                },
            }
            .unwrap();
        }
        h.push_str(" };\n");
        for s in &samplers {
            let (texture, sampler) = split_sampler(&s.ty);
            writeln!(
                h,
                "layout(set = 0, binding = {}) uniform {texture} {}; layout(set = 0, binding = {}) uniform {sampler} hz_s_{};",
                s.binding, s.name, s.binding + 1, s.name
            )
            .unwrap();
        }
        let direction = if stage.kind == ShaderStage::Vertex {
            "out"
        } else {
            "in"
        };
        for (name, (location, decl)) in &locations {
            writeln!(
                h,
                "layout(location = {location}) {direction} {}",
                decl.replace("@", name)
            )
            .unwrap();
        }
        if let (Some(inputs), ShaderStage::Vertex) = (gbuffers, stage.kind) {
            h.push_str(&inputs.declarations);
            h.push('\n');
        }
        h.push_str(&stage.prelude());
        h
    };
    let flip = if target == Target::Buffers { "-" } else { "" };
    let fog =
        "    hz_Fog = hz_FogParameters(hz_fog_color, hz_fog.x, hz_fog.y, hz_fog.z, hz_fog.w);\n";
    let mut vertex_main = String::from("\nvoid main() {\n");
    match gbuffers {
        None => vertex_main.push_str(
            "    vec2 hz_p = hz_quad[gl_VertexIndex];\n    hz_Vertex = vec4(hz_p, 0.0, 1.0);\n    \
             hz_MultiTexCoord0 = hz_Vertex;\n    hz_Color = vec4(1.0);\n    hz_Normal = vec3(0.0, 0.0, 1.0);\n",
        ),
        Some(inputs) => {
            vertex_main.push_str(GBUFFERS_MATRICES);
            vertex_main.push_str(&inputs.body);
        }
    }
    for (name, value, ty) in &v.attributes {
        let value = match gbuffers {
            // Our quad's values for a composite.
            None => value.clone(),
            // The game's, or zero for anything it doesn't provide.
            Some(inputs) => match inputs.attributes.iter().find(|a| &a.0 == name) {
                Some((_, given)) => format!("{ty}({given})"),
                None => format!("{ty}(0)"),
            },
        };
        writeln!(vertex_main, "    {name} = {value};").unwrap();
    }
    vertex_main.push_str(fog);
    for name in &v.inits {
        writeln!(vertex_main, "    {name} = hz_init_{name}();").unwrap();
    }
    write!(
        vertex_main,
        "    hz_main();\n    gl_Position = vec4(gl_Position.x, {flip}gl_Position.y, (gl_Position.z + gl_Position.w) * 0.5, gl_Position.w);\n}}\n"
    )
    .unwrap();
    let mut fragment_main = String::from("\nvoid main() {\n");
    if gbuffers.is_some() {
        fragment_main.push_str(GBUFFERS_MATRICES);
    }
    fragment_main.push_str(fog);
    if f.frag_coord {
        fragment_main.push_str(match target {
            Target::Window => "    hz_FragCoord = vec4(gl_FragCoord.x, hz_target.y - gl_FragCoord.y, gl_FragCoord.zw);\n",
            Target::Buffers => "    hz_FragCoord = gl_FragCoord;\n",
        });
    }
    for name in &f.inits {
        writeln!(fragment_main, "    {name} = hz_init_{name}();").unwrap();
    }
    fragment_main.push_str("    hz_main();\n}\n");

    let v_text = format!(
        "{}{}{vertex_main}",
        header(&v),
        v.rewrite(&samplers, &uniforms)
    );
    let f_text = format!(
        "{}{}{fragment_main}",
        header(&f),
        f.rewrite(&samplers, &uniforms)
    );
    if !errors.is_empty() {
        return Err(errors);
    }
    let vertex_module = compile(&v, &v_text, header(&v).lines().count());
    let fragment_module = compile(&f, &f_text, header(&f).lines().count());
    let (vertex, fragment) = match (vertex_module, fragment_module) {
        (Ok(v), Ok(f)) => (v, f),
        (a, b) => return Err(a.err().into_iter().chain(b.err()).flatten().collect()),
    };
    let (uniform_size, offsets) = block_layout(&fragment);
    let uniforms = offsets
        .into_iter()
        .filter_map(|(member, offset)| {
            let name = member
                .strip_prefix("hz_bool_")
                .unwrap_or(&member)
                .to_owned();
            let ty = if name.starts_with("hz_") {
                "vec4".to_owned()
            } else {
                uniforms.get(&name)?.clone()
            };
            Some(Uniform { name, ty, offset })
        })
        .collect();
    let outputs = f.outputs.iter().copied().collect();
    Ok(Program {
        vertex,
        fragment,
        interface: Interface {
            uniform_size,
            uniforms,
            samplers,
            outputs,
        },
    })
}

/// `sampler2DShadow` → (`texture2D`, `samplerShadow`); `isampler3D` → (`itexture3D`, `sampler`).
pub(super) fn split_sampler(ty: &str) -> (String, &'static str) {
    let shadow = ty.ends_with("Shadow");
    let base = ty
        .trim_end_matches("Shadow")
        .replacen("sampler", "texture", 1);
    (base, if shadow { "samplerShadow" } else { "sampler" })
}

/// Parses and validates one stage; errors at the pack's files and lines.
fn compile(stage: &Stage, text: &str, header_lines: usize) -> Result<naga::Module, Vec<Error>> {
    let at = |span: naga::Span, message: String| {
        let start = span.to_range().map_or(0, |r| r.start);
        let line = text[..start.min(text.len())].matches('\n').count();
        match line
            .checked_sub(header_lines)
            .and_then(|l| stage.pp.lines.get(l))
        {
            Some(o) => Error {
                file: stage
                    .pp
                    .files
                    .get(o.file as usize)
                    .cloned()
                    .unwrap_or_default(),
                line: o.line,
                message,
            },
            None => Error {
                file: "<horizon>".into(),
                line: line as u32 + 1,
                message,
            },
        }
    };
    let module = Frontend::default()
        .parse(&Options::from(stage.kind), text)
        .map_err(|e| {
            e.errors
                .into_iter()
                .map(|e| at(e.meta, e.kind.to_string()))
                .collect::<Vec<_>>()
        })?;
    Validator::new(ValidationFlags::all(), Capabilities::all())
        .validate(&module)
        .map_err(|e| {
            let span = e.spans().next().map_or(naga::Span::default(), |(s, _)| *s);
            // The validator nests its reasons: say them all.
            let inner = e.into_inner();
            let mut message = inner.to_string();
            let mut source = std::error::Error::source(&inner);
            while let Some(s) = source {
                write!(message, ": {s}").unwrap();
                source = s.source();
            }
            vec![at(span, message)]
        })?;
    Ok(module)
}

/// The uniform block's size and member offsets, from naga's layout.
fn block_layout(module: &naga::Module) -> (u32, Vec<(String, u32)>) {
    for (_, var) in module.global_variables.iter() {
        if var.space != naga::AddressSpace::Uniform {
            continue;
        }
        if let naga::TypeInner::Struct { members, span } = &module.types[var.ty].inner {
            let offsets = members
                .iter()
                .filter_map(|m| Some((m.name.clone()?, m.offset)))
                .collect();
            return (*span, offsets);
        }
    }
    (16, vec![("hz_target".into(), 0)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::tests::zip;
    use crate::{Macros, Pack, preprocess};

    fn program(files: &[(&str, &str)], target: Target) -> Result<Program, Vec<Error>> {
        let files: Vec<(String, &str)> = files
            .iter()
            .map(|(p, t)| (format!("shaders/{p}"), *t))
            .collect();
        let files: Vec<(&str, &str)> = files.iter().map(|(p, t)| (p.as_str(), *t)).collect();
        let pack = Pack::from_zip_bytes("P.zip", zip(&files)).unwrap();
        let m = Macros::standard();
        translate_composite(
            &preprocess(&pack, "composite.vsh", &m),
            &preprocess(&pack, "composite.fsh", &m),
            target,
        )
    }

    const VSH_120: &str = "#version 120\nvarying vec2 texcoord;\nvoid main() {\n    gl_Position = ftransform();\n    \
                           texcoord = (gl_TextureMatrix[0] * gl_MultiTexCoord0).xy;\n}\n";

    #[test]
    fn compatibility_programs_translate() {
        let fsh = "#version 120\nuniform sampler2D colortex0;\nuniform sampler2D depthtex0;\nuniform float viewWidth;\n\
                   uniform bool hideGUI;\nuniform int frameCounter;\nvarying vec2 texcoord;\n\
                   vec3 tint(sampler2D s, vec2 c) { return texture2D(s, c).rgb; }\nvoid main() {\n    \
                   vec3 c = tint(colortex0, texcoord) * texture2D(depthtex0, texcoord).r;\n    if (hideGUI) c *= 0.5;\n    \
                   /* DRAWBUFFERS:04 */\n    gl_FragData[0] = vec4(c, 1.0);\n    gl_FragData[1] = vec4(frameCounter / viewWidth);\n}\n";
        let p = program(
            &[("composite.vsh", VSH_120), ("composite.fsh", fsh)],
            Target::Buffers,
        )
        .unwrap();
        let i = &p.interface;
        let samplers: Vec<(&str, u32)> = i
            .samplers
            .iter()
            .map(|s| (s.name.as_str(), s.binding))
            .collect();
        assert_eq!(samplers, [("colortex0", 1), ("depthtex0", 3)]);
        let uniforms: Vec<(&str, &str)> = i
            .uniforms
            .iter()
            .map(|u| (u.name.as_str(), u.ty.as_str()))
            .collect();
        assert_eq!(
            uniforms,
            [
                ("hz_target", "vec4"),
                ("hz_fog_color", "vec4"),
                ("hz_fog", "vec4"),
                ("frameCounter", "int"),
                ("hideGUI", "bool"),
                ("viewWidth", "float")
            ]
        );
        assert_eq!(i.uniforms[0].offset, 0);
        assert!(i.uniform_size >= 28);
        assert_eq!(i.outputs, [0, 1]);
    }

    /// naga drops non-constant global initializers: they run in our `main`, one per declarator,
    /// converting as GLSL's initialization does.
    #[test]
    fn global_initializers_read_uniforms() {
        let fsh = "#version 120\nuniform float rainStrength;\nuniform ivec2 atlasSize;\n\
                   vec2 size = atlasSize;\nvec3 a = vec3(rainStrength), b = a * 2.0, c;\n\
                   const float k = rainStrength > 0.5 ? 1.0 : 0.0;\nfloat w[] = float[](rainStrength, 1.0);\n\
                   void main() { c = b; gl_FragColor = vec4(a + c, size.x * k + w[0]); }\n";
        let p = program(
            &[("composite.vsh", VSH_120), ("composite.fsh", fsh)],
            Target::Window,
        );
        assert!(p.is_ok(), "{:?}", p.err());
    }

    #[test]
    fn core_programs_translate() {
        let vsh = "#version 330 core\nin vec3 vaPosition;\nin vec2 vaUV0;\nuniform mat4 modelViewMatrix, projectionMatrix;\n\
                   out vec2 uv;\nflat out int flatId;\nvoid main() {\n    \
                   gl_Position = projectionMatrix * modelViewMatrix * vec4(vaPosition, 1.0);\n    uv = vaUV0;\n    flatId = 1;\n}\n";
        let fsh = "#version 330 core\nin vec2 uv;\nuniform sampler2D colortex0;\nuniform usampler2D colortex5;\n\
                   layout(location = 2) out vec4 color;\nvoid main() {\n    \
                   if (texture(colortex0, uv).a < 0.1) discard;\n    color = texture(colortex0, uv) + float(texelFetch(colortex5, ivec2(gl_FragCoord.xy), 0).r);\n}\n";
        let p = program(
            &[("composite.vsh", vsh), ("composite.fsh", fsh)],
            Target::Window,
        )
        .unwrap();
        assert_eq!(p.interface.outputs, [2]);
        assert!(
            p.interface
                .uniforms
                .iter()
                .any(|u| u.name == "projectionMatrix" && u.ty == "mat4")
        );
    }

    #[test]
    fn frag_coord_is_set_for_every_target() {
        let fsh = "#version 120\nvoid main() { gl_FragColor = vec4(gl_FragCoord.x); }\n";
        for target in [Target::Buffers, Target::Window] {
            let p = program(
                &[("composite.vsh", VSH_120), ("composite.fsh", fsh)],
                target,
            )
            .unwrap();
            let m = &p.fragment;
            let (handle, _) = m
                .global_variables
                .iter()
                .find(|(_, g)| g.name.as_deref() == Some("hz_FragCoord"))
                .unwrap();
            // naga calls our `main` from its entry point: look in every function.
            let functions = m.functions.iter().map(|(_, f)| f);
            let stores = functions.chain([&m.entry_points[0].function]).any(|f| {
                f.body.iter().any(|s| match s {
                    naga::Statement::Store { pointer, .. } => {
                        matches!(f.expressions[*pointer], naga::Expression::GlobalVariable(h) if h == handle)
                    }
                    _ => false,
                })
            });
            assert!(stores, "{target:?}: main does not set hz_FragCoord");
        }
    }

    #[test]
    fn gbuffers_programs_read_the_games_vertices() {
        let vsh = "#version 120\nattribute vec4 mc_Entity;\nattribute vec2 mc_midTexCoord;\nvarying vec2 uv;\nvarying vec2 light;\n\
                   varying vec4 color;\nvarying vec3 normal;\nvarying float id;\nvoid main() {\n    \
                   gl_Position = ftransform();\n    uv = (gl_TextureMatrix[0] * gl_MultiTexCoord0).xy;\n    \
                   light = (gl_TextureMatrix[1] * gl_MultiTexCoord1).xy;\n    color = gl_Color;\n    \
                   normal = normalize(gl_NormalMatrix * gl_Normal);\n    id = mc_Entity.x + mc_midTexCoord.x;\n}\n";
        let fsh = "#version 120\nuniform sampler2D texture;\nuniform sampler2D lightmap;\nuniform mat4 gbufferModelView;\n\
                   varying vec2 uv;\nvarying vec2 light;\nvarying vec4 color;\nvarying vec3 normal;\nvarying float id;\n\
                   void main() {\n    vec4 c = texture2D(texture, uv) * texture2D(lightmap, light) * color;\n    \
                   /* DRAWBUFFERS:02 */\n    gl_FragData[0] = c * gl_FragCoord.z;\n    \
                   gl_FragData[1] = vec4(normal * 0.5 + 0.5, id);\n}\n";
        let files: Vec<(String, &str)> =
            [("gbuffers_terrain.vsh", vsh), ("gbuffers_terrain.fsh", fsh)]
                .iter()
                .map(|(p, t)| (format!("shaders/{p}"), *t))
                .collect();
        let files: Vec<(&str, &str)> = files.iter().map(|(p, t)| (p.as_str(), *t)).collect();
        let pack = Pack::from_zip_bytes("P.zip", zip(&files)).unwrap();
        let m = Macros::standard();
        let inputs = GbuffersInputs {
            declarations: "layout(location = 0) in vec3 game_pos;\nlayout(location = 1) in uvec2 game_uv;".into(),
            body: "    hz_Vertex = vec4(game_pos, 1.0);\n    hz_MultiTexCoord0 = vec4(vec2(game_uv) / 32768.0, 0.0, 1.0);\n    \
                   hz_MultiTexCoord1 = vec4(240.0, 240.0, 0.0, 1.0);\n    hz_Color = vec4(1.0);\n    hz_Normal = vec3(0.0, 1.0, 0.0);\n"
                .into(),
            attributes: vec![("mc_midTexCoord".into(), "vec2(game_uv) / 32768.0".into())],
        };
        let p = translate_gbuffers(
            &preprocess(&pack, "gbuffers_terrain.vsh", &m),
            &preprocess(&pack, "gbuffers_terrain.fsh", &m),
            Target::Buffers,
            &inputs,
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
        let names: Vec<&str> = p
            .interface
            .uniforms
            .iter()
            .map(|u| u.name.as_str())
            .collect();
        for name in [
            "gbufferModelView",
            "gbufferProjection",
            "gbufferModelViewInverse",
        ] {
            assert!(names.contains(&name), "{name} missing from {names:?}");
        }
        assert_eq!(p.interface.outputs, vec![0, 1]);
        let inputs: Vec<&str> = p.vertex.entry_points[0]
            .function
            .arguments
            .iter()
            .filter_map(|a| a.name.as_deref())
            .collect();
        assert!(
            inputs.contains(&"game_pos") && inputs.contains(&"game_uv"),
            "{inputs:?}"
        );
    }

    #[test]
    fn locals_may_hide_a_sampler() {
        let fsh = "#version 120\nuniform sampler2D specular;\nstruct M { float specular; };\n\
                   float f(float x) { float specular = x * 2.0; return specular; }\n\
                   void main() { M m = M(0.5); gl_FragColor = texture2D(specular, vec2(0.5)) * f(m.specular); }\n";
        program(
            &[("composite.vsh", VSH_120), ("composite.fsh", fsh)],
            Target::Buffers,
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
    }

    #[test]
    fn errors_point_at_the_pack_file_and_line() {
        let fsh =
            "#version 120\n#include \"/lib/bad.glsl\"\nvoid main() { gl_FragColor = vec4(f()); }\n";
        let bad = "// fine\nfloat f() {\n    return undefinedThing;\n}\n";
        let errors = program(
            &[
                ("composite.vsh", VSH_120),
                ("composite.fsh", fsh),
                ("lib/bad.glsl", bad),
            ],
            Target::Buffers,
        )
        .err()
        .unwrap();
        assert_eq!(
            (errors[0].file.as_str(), errors[0].line),
            ("lib/bad.glsl", 3),
            "{errors:?}"
        );
    }
}
