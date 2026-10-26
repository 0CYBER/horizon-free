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
    /// A gbuffers program reads `DRAW_UNIFORMS`: the block at set 2, binding 0, set per draw.
    pub draw: bool,
}

/// The uniforms a gbuffers program reads per draw (5.3's "D"), in a block of their own
/// (`HzDraw`, std140: three ints, a pad, a vec4; 32 bytes) the game sets for each draw.
pub const DRAW_UNIFORMS: [(&str, &str); 4] = [
    ("renderStage", "int"),
    ("entityId", "int"),
    ("blockEntityId", "int"),
    ("entityColor", "vec4"),
];

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
    /// GLSL statements run last in `main`, with `gl_Position` in wgpu's clip space.
    pub after: String,
    /// The alpha test applied to output 0 (`gl_FragData[0]`, `gl_FragColor`), the loader's
    /// (3.2 `alphaTest.<program>`): `None` keeps every fragment.
    pub alpha_test: Option<AlphaTest>,
}

/// A fragment stays when its output 0's alpha compares to `reference` as `op` says (a GLSL
/// comparison operator, `true` for GL's `ALWAYS`, `false` for `NEVER`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AlphaTest {
    pub op: &'static str,
    pub reference: f32,
}

impl AlphaTest {
    /// `alphaTest.<program>`'s value: `off`, or a GL comparison and a reference.
    pub fn parse(value: &str) -> Option<Option<Self>> {
        let mut words = value.split_whitespace();
        let func = words.next()?;
        if func == "off" {
            return Some(None);
        }
        let op = match func.trim_start_matches("GL_") {
            "NEVER" => "false",
            "LESS" => "<",
            "EQUAL" => "==",
            "LEQUAL" => "<=",
            "GREATER" => ">",
            "NOTEQUAL" => "!=",
            "GEQUAL" => ">=",
            "ALWAYS" => return Some(None),
            _ => return None,
        };
        let reference = words.next()?.parse().ok()?;
        Some(Some(Self { op, reference }))
    }
}

/// Translates a composite-style program (`composite`, `deferred`, `final`...).
pub fn translate_composite(
    vertex: &Preprocessed,
    fragment: &Preprocessed,
    target: Target,
) -> Result<Program, Vec<Error>> {
    translate(vertex, fragment, target, None, false)
}

/// Translates a gbuffers-style program (`gbuffers_terrain`...) drawing the game's geometry
/// described by `inputs`, into `target` (the pack's buffers).
pub fn translate_gbuffers(
    vertex: &Preprocessed,
    fragment: &Preprocessed,
    target: Target,
    inputs: &GbuffersInputs,
) -> Result<Program, Vec<Error>> {
    translate(vertex, fragment, target, Some(inputs), false)
}

/// Translates a shadow program (`shadow`, `shadow_water`...) drawing the game's geometry
/// described by `inputs` from the shadow light: its model-view and projection matrices are the
/// shadow camera's (5.2).
pub fn translate_shadow(
    vertex: &Preprocessed,
    fragment: &Preprocessed,
    inputs: &GbuffersInputs,
) -> Result<Program, Vec<Error>> {
    translate(vertex, fragment, Target::Buffers, Some(inputs), true)
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
    shadow: bool,
) -> Result<Program, Vec<Error>> {
    let matrices = if shadow {
        GBUFFERS_MATRICES.replace("gbuffer", "shadow")
    } else {
        GBUFFERS_MATRICES.to_owned()
    };
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
            if shadow {
                uniforms.insert(name.replace("gbuffer", "shadow"), "mat4".to_owned());
            }
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
    // Per-draw values move to their own block; another type than the documented one keeps
    // the frame's block (and reads zero).
    let mut draw = false;
    if gbuffers.is_some() {
        for (name, ty) in DRAW_UNIFORMS {
            if uniforms.get(name).is_some_and(|t| t == ty) {
                uniforms.remove(name);
                draw = true;
            }
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

    // Varyings by name: the vertex outputs, then fragment inputs the vertex stage lacks. A
    // matrix or array travels as one varying per column or element (`varying_parts`).
    let mut locations: BTreeMap<String, (u32, String)> = BTreeMap::new();
    let mut split: BTreeMap<String, (String, Vec<(String, String)>)> = BTreeMap::new();
    for var in v.outs.iter().chain(&f.ins) {
        if locations.contains_key(&var.name) || split.contains_key(&var.name) {
            continue;
        }
        match varying_parts(&var.decl) {
            Some((global, parts)) => {
                let mut copies = Vec::new();
                for (k, (decl, access)) in parts.into_iter().enumerate() {
                    let part = format!("hz_v_{}_{k}", var.name);
                    let next = locations.len() as u32;
                    locations.insert(part.clone(), (next, decl));
                    copies.push((part, access));
                }
                split.insert(var.name.clone(), (global, copies));
            }
            None => {
                let next = locations.len() as u32;
                locations.insert(var.name.clone(), (next, var.decl.clone()));
            }
        }
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
        if draw {
            h.push_str(
                "layout(set = 2, binding = 0) uniform HzDraw { int renderStage; int entityId; \
                 int blockEntityId; int hz_draw_pad; vec4 entityColor; };\n",
            );
        }
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
        for (name, (global, _)) in &split {
            writeln!(h, "{}", global.replace("@", name)).unwrap();
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
            vertex_main.push_str(&matrices);
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
    vertex_main.push_str("    hz_main();\n");
    for (name, (_, copies)) in &split {
        for (part, access) in copies {
            writeln!(vertex_main, "    {part} = {name}{access};").unwrap();
        }
    }
    write!(
        vertex_main,
        "    gl_Position = vec4(gl_Position.x, {flip}gl_Position.y, (gl_Position.z + gl_Position.w) * 0.5, gl_Position.w);\n{}}}\n",
        gbuffers.map_or("", |g| &g.after)
    )
    .unwrap();
    let mut fragment_main = String::from("\nvoid main() {\n");
    if gbuffers.is_some() {
        fragment_main.push_str(&matrices);
    }
    fragment_main.push_str(fog);
    if f.frag_coord {
        fragment_main.push_str(match target {
            Target::Window => "    hz_FragCoord = vec4(gl_FragCoord.x, hz_target.y - gl_FragCoord.y, gl_FragCoord.zw);\n",
            Target::Buffers => "    hz_FragCoord = gl_FragCoord;\n",
        });
    }
    for (name, (_, copies)) in &split {
        for (part, access) in copies {
            writeln!(fragment_main, "    {name}{access} = {part};").unwrap();
        }
    }
    for name in &f.inits {
        writeln!(fragment_main, "    {name} = hz_init_{name}();").unwrap();
    }
    fragment_main.push_str("    hz_main();\n");
    // With `out` variables the pack tests alpha itself.
    if let Some(t) = gbuffers.and_then(|g| g.alpha_test)
        && f.outputs.contains(&0)
        && !f.frag_outs.iter().any(|(n, _)| *n == 0)
    {
        match t.op {
            "false" => fragment_main.push_str("    discard;\n"),
            op => writeln!(
                fragment_main,
                "    if (!(hz_out0.a {op} {:?})) {{ discard; }}",
                t.reference
            )
            .unwrap(),
        }
    }
    fragment_main.push_str("}\n");

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
            draw,
        },
    })
}

/// A matrix or array varying (`flat mat3 @[2];`): wgpu's stage interfaces take only scalars
/// and vectors (WGSL), so it becomes a private global (`mat3 @[2];`) and one varying per
/// column or element, each with the qualifiers and how to reach it (`[1][2]`). `None` for
/// scalars and vectors, and for sizes that are not plain numbers.
fn varying_parts(decl: &str) -> Option<(String, Vec<(String, String)>)> {
    let (before, after) = decl.split_once('@')?;
    let mut words: Vec<&str> = before.split_whitespace().collect();
    let ty = words.pop()?;
    let qualifiers: String = words.iter().map(|w| format!("{w} ")).collect();
    let array = after.trim().trim_end_matches(';').trim();
    let mut sizes = Vec::new();
    let mut rest = array;
    while let Some(r) = rest.strip_prefix('[') {
        let (n, r) = r.split_once(']')?;
        sizes.push(n.trim().parse::<usize>().ok()?);
        rest = r.trim_start();
    }
    if !rest.is_empty() {
        return None;
    }
    // matC or matCxR: C columns of R rows.
    let column = ty.strip_prefix("mat").and_then(|d| {
        let (c, r) = d.split_once('x').unwrap_or((d, d));
        Some((c.parse::<usize>().ok()?, r.parse::<usize>().ok()?))
    });
    if sizes.is_empty() && column.is_none() {
        return None;
    }
    let part_ty = column.map_or(ty.to_owned(), |(_, r)| format!("vec{r}"));
    if let Some((c, _)) = column {
        sizes.push(c);
    }
    let count: usize = sizes.iter().product();
    let parts = (0..count)
        .map(|mut k| {
            let mut access = String::new();
            for &size in sizes.iter().rev() {
                access.insert_str(0, &format!("[{}]", k % size));
                k /= size;
            }
            (format!("{qualifiers}{part_ty} @;"), access)
        })
        .collect();
    Some((format!("{ty} @{array};"), parts))
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
        let frame = var
            .binding
            .as_ref()
            .is_some_and(|b| b.group == 0 && b.binding == 0);
        if var.space != naga::AddressSpace::Uniform || !frame {
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

    /// Code after an unconditional `discard` (BSL's `gbuffers_clouds` with its own clouds on)
    /// or `return;` still makes a valid module.
    #[test]
    fn code_after_discard_or_return_translates() {
        let vsh = "#version 120\nvoid main() { gl_Position = ftransform(); }\n";
        for fsh in [
            "#version 120\nuniform sampler2D texture;\nvoid main() {\n    discard;\n    vec4 c = texture2D(texture, vec2(0.5));\n    gl_FragData[0] = c;\n}\n",
            "#version 120\nvoid main() {\n    gl_FragData[0] = vec4(1.0);\n    return;\n    gl_FragData[0] = vec4(0.5);\n}\n",
        ] {
            let files = [
                ("shaders/gbuffers_clouds.vsh", vsh),
                ("shaders/gbuffers_clouds.fsh", fsh),
            ];
            let pack = Pack::from_zip_bytes("P.zip", zip(&files)).unwrap();
            let m = Macros::standard();
            let inputs = GbuffersInputs {
                body: "    hz_Vertex = vec4(0.0, 0.0, 0.0, 1.0);\n".into(),
                ..Default::default()
            };
            translate_gbuffers(
                &preprocess(&pack, "gbuffers_clouds.vsh", &m),
                &preprocess(&pack, "gbuffers_clouds.fsh", &m),
                Target::Buffers,
                &inputs,
            )
            .unwrap_or_else(|e| panic!("{fsh}: {e:?}"));
        }
    }

    #[test]
    fn the_alpha_test_discards_below_its_reference() {
        assert_eq!(
            AlphaTest::parse("GREATER 0.1"),
            Some(Some(AlphaTest {
                op: ">",
                reference: 0.1
            }))
        );
        assert_eq!(AlphaTest::parse("off"), Some(None));
        assert_eq!(AlphaTest::parse("GL_ALWAYS 0"), Some(None));
        assert_eq!(AlphaTest::parse("SOMETIMES 0.5"), None);
        let vsh = "#version 120\nvoid main() { gl_Position = ftransform(); }\n";
        let legacy = "#version 120\nvoid main() { gl_FragData[0] = vec4(0.5); }\n";
        let outs =
            "#version 330\nlayout(location = 0) out vec4 c;\nvoid main() { c = vec4(0.5); }\n";
        for (fsh, kills) in [(legacy, true), (outs, false)] {
            let files = [
                ("shaders/gbuffers_entities.vsh", vsh),
                ("shaders/gbuffers_entities.fsh", fsh),
            ];
            let pack = Pack::from_zip_bytes("P.zip", zip(&files)).unwrap();
            let m = Macros::standard();
            let inputs = GbuffersInputs {
                body: "    hz_Vertex = vec4(0.0, 0.0, 0.0, 1.0);\n".into(),
                alpha_test: AlphaTest::parse("GREATER 0.1").unwrap(),
                ..Default::default()
            };
            let p = translate_gbuffers(
                &preprocess(&pack, "gbuffers_entities.vsh", &m),
                &preprocess(&pack, "gbuffers_entities.fsh", &m),
                Target::Buffers,
                &inputs,
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
            let functions = p.fragment.functions.iter().map(|(_, f)| f);
            let kill = functions
                .chain([&p.fragment.entry_points[0].function])
                .any(|f| format!("{:?}", f.body).contains("Kill"));
            // With `out` variables the pack tests alpha itself.
            assert_eq!(kill, kills, "{fsh}");
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
                   uniform int renderStage;\nvarying vec2 uv;\nvarying vec2 light;\nvarying vec4 color;\nvarying vec3 normal;\nvarying float id;\n\
                   void main() {\n    vec4 c = texture2D(texture, uv) * texture2D(lightmap, light) * color;\n    \
                   /* DRAWBUFFERS:02 */\n    gl_FragData[0] = c * gl_FragCoord.z * float(renderStage);\n    \
                   gl_FragData[1] = vec4(normal * 0.5 + 0.5, id);\n}\n";
        let files: Vec<(String, &str)> =
            [("gbuffers_terrain.vsh", vsh), ("gbuffers_terrain.fsh", fsh)]
                .iter()
                .map(|(p, t)| (format!("shaders/{p}"), *t))
                .collect();
        let files: Vec<(&str, &str)> = files.iter().map(|(p, t)| (p.as_str(), *t)).collect();
        let pack = Pack::from_zip_bytes("P.zip", zip(&files)).unwrap();
        let m = Macros::standard();
        let game = GbuffersInputs {
            declarations: "layout(location = 0) in vec3 game_pos;\nlayout(location = 1) in uvec2 game_uv;".into(),
            body: "    hz_Vertex = vec4(game_pos, 1.0);\n    hz_MultiTexCoord0 = vec4(vec2(game_uv) / 32768.0, 0.0, 1.0);\n    \
                   hz_MultiTexCoord1 = vec4(240.0, 240.0, 0.0, 1.0);\n    hz_Color = vec4(1.0);\n    hz_Normal = vec3(0.0, 1.0, 0.0);\n"
                .into(),
            attributes: vec![("mc_midTexCoord".into(), "vec2(game_uv) / 32768.0".into())],
            ..Default::default()
        };
        let p = translate_gbuffers(
            &preprocess(&pack, "gbuffers_terrain.vsh", &m),
            &preprocess(&pack, "gbuffers_terrain.fsh", &m),
            Target::Buffers,
            &game,
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
        // renderStage is set per draw, in its own block.
        assert!(p.interface.draw && !names.contains(&"renderStage"));
        let per_draw = p.fragment.global_variables.iter().any(|(_, v)| {
            v.binding
                .as_ref()
                .is_some_and(|b| b.group == 2 && b.binding == 0)
        });
        assert!(per_draw);
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
        // From the light, the compatibility matrices are the shadow camera's.
        let shadow = translate_shadow(
            &preprocess(&pack, "gbuffers_terrain.vsh", &m),
            &preprocess(&pack, "gbuffers_terrain.fsh", &m),
            &game,
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
        let names: Vec<&str> = (shadow.interface.uniforms.iter())
            .map(|u| u.name.as_str())
            .collect();
        for name in ["shadowModelView", "shadowProjection", "gbufferModelView"] {
            assert!(names.contains(&name), "{name} missing from {names:?}");
        }
    }

    #[test]
    fn matrix_and_array_varyings_travel_in_parts() {
        let vsh = "#version 120\nvarying mat3 tbn;\nflat varying ivec2 ids[2];\nvarying mat2x3 m[2];\n\
                   void main() {\n    gl_Position = ftransform();\n    tbn = mat3(2.0);\n    \
                   ids[1] = ivec2(3);\n    m[1][0] = vec3(4.0);\n}\n";
        let fsh = "#version 120\nvarying mat3 tbn;\nflat varying ivec2 ids[2];\nvarying mat2x3 m[2];\n\
                   void main() { gl_FragColor = vec4(tbn[2] + m[1][0], float(ids[1].x)); }\n";
        let p = program(
            &[("composite.vsh", vsh), ("composite.fsh", fsh)],
            Target::Buffers,
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
        let inputs = p.fragment.entry_points[0].function.arguments.len();
        // 3 columns + 2 elements + 2 x 2 columns, and the fragment position.
        assert!(inputs >= 9, "{inputs} inputs");
        assert_eq!(
            varying_parts("flat ivec2 @[2];").unwrap().1,
            [
                ("flat ivec2 @;".to_owned(), "[0]".to_owned()),
                ("flat ivec2 @;".to_owned(), "[1]".to_owned())
            ]
        );
        assert_eq!(varying_parts("mat2x3 @[2];").unwrap().1[3].1, "[1][1]");
        assert!(varying_parts("vec4 @;").is_none());
        assert!(varying_parts("vec4 @[N];").is_none());
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
