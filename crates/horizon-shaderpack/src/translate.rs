//! Preprocessed pack GLSL to WGSL. Packs are written for `#version 120`..`460
//! compatibility` and lean on the compatibility profile; naga's GLSL front end
//! takes Vulkan-style GLSL 450. The legaliser parses the program into a
//! syntax tree (glsl-lang), rewrites it the way Iris's glsl-transformer
//! patches do for the core profile, prints GLSL 450 and has naga turn that
//! into a validated module and WGSL:
//!
//! - `attribute`/`varying` become `in`/`out` with locations; matrix, nested
//!   array and struct varyings are split into vectors (WGSL can pass none of
//!   them directly between stages);
//! - loose uniforms go into one std140 block (`bool`s stored as `int`);
//! - `sampler2D` & co. become a texture and a sampler binding, rebuilt as
//!   `sampler2D(t, s)` where used, also through function parameters, and not
//!   where a local hides them;
//! - compatibility built-ins (`gl_Vertex`, `gl_Color`, `gl_FragData[n]`,
//!   `gl_ModelViewMatrix`, `ftransform()`, ...) and Iris's core-profile names
//!   (`vaPosition`, `modelViewMatrix`, ...) are declared;
//! - legacy texture calls are renamed, `shadow2D` returns a `vec4` again and a
//!   swizzled shadow lookup is widened first (naga has no scalar swizzle);
//! - outside fragment programs, `texture`, `textureProj` and their `Offset`
//!   forms become their `Lod` forms at level 0 (no derivatives there; naga
//!   refuses the implicit ones), a bias dropped;
//! - global initialisers run at the start of `main` (drivers accept
//!   non-constant ones), unsized global arrays are sized from them;
//! - `isnan`, `isinf` and `modf` are helper functions (WGSL has none);
//! - what naga refuses and drivers accept: `%` goes through overloads (naga
//!   won't take `ivec2 % int`), `mat2(vec4)` goes through `iris_mat2`, `1 << 31`
//!   shifts as unsigned before casting back to `int`, `sqrt` of a negative
//!   constant isn't folded, constants naga can't fold (`?:`, `mix`, `<<`)
//!   become globals set in `main`, and 16/8-bit types are their 32-bit ones.
//!
//! The programs of one pass share a [`Link`], so their locations, bindings and
//! uniform block agree.

use crate::Stage;
use glsl_lang::{
    ast::{self, NodeContent},
    parse::DefaultParse,
    transpiler::glsl::{
        FormattingState, show_declaration, show_expr, show_external_declaration,
        show_type_specifier,
    },
};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// Locations, bindings and uniforms shared by the programs of one pass: a
/// pass's vertex program is translated first, its fragment program second.
#[derive(Default, Debug)]
pub struct Link {
    /// Varying name → location (split matrices and arrays as `name#i`).
    pub varyings: BTreeMap<String, u32>,
    /// Vertex attribute name → location.
    pub attributes: BTreeMap<String, u32>,
    /// Texture, sampler, image and block name → binding in group 0 (the
    /// uniform block is binding 0).
    pub bindings: BTreeMap<String, u32>,
    /// The uniform block's members in order (`"mat4 gbufferModelView"`); a
    /// later program's block extends an earlier one's, so offsets agree.
    pub uniforms: Vec<(String, String)>,
    /// Storage formats of images whose declaration has none.
    pub image_formats: BTreeMap<String, String>,
    /// Samplers shared by texture name (`colortex0` → `iris_linear`), so a
    /// pass binds a few samplers rather than one per texture (Metal allows
    /// 16). Once any is named, comparison samplers share `iris_compare` and
    /// integer ones `iris_nearest`; textures not named keep `{name}_s`.
    pub shared_samplers: BTreeMap<String, String>,
}

impl Link {
    fn location(map: &mut BTreeMap<String, u32>, name: &str) -> u32 {
        let next = map.len() as u32;
        *map.entry(name.to_owned()).or_insert(next)
    }

    fn binding(&mut self, name: &str) -> u32 {
        let next = self.bindings.len() as u32 + 1;
        *self.bindings.entry(name.to_owned()).or_insert(next)
    }

    fn uniform(&mut self, name: &str, decl: String) -> Result<(), String> {
        match self.uniforms.iter().find(|(n, _)| n == name) {
            Some((_, d)) if *d != decl => Err(format!(
                "uniform {name} is declared as both `{d}` and `{decl}`"
            )),
            Some(_) => Ok(()),
            None => {
                self.uniforms.push((name.to_owned(), decl));
                Ok(())
            }
        }
    }
}

/// What a translated program binds and passes, for the pipeline that runs it.
#[derive(Default, Debug)]
pub struct Interface {
    /// `(name, kind)` of each sampler the program reads (`colortex0`,
    /// `sampler2D`); its bindings are `{name}_t` and its sampler's in the [`Link`].
    pub samplers: Vec<(String, String)>,
    /// Each sampler's sampler variable: `{name}_s`, or a shared one
    /// ([`Link::shared_samplers`]).
    pub sampler_vars: BTreeMap<String, String>,
    /// `(name, format)` of each image.
    pub images: Vec<(String, String)>,
    /// Uniform names the program declares or uses.
    pub uniforms: BTreeSet<String>,
    /// Vertex attributes read (vertex programs).
    pub attributes: BTreeSet<String>,
    /// Fragment outputs: location → name (`iris_FragData0` for `gl_FragData[0]`).
    pub outputs: BTreeMap<u32, String>,
}

pub struct Translated {
    /// The legalised GLSL 450 naga read (kept for diagnostics).
    pub glsl: String,
    pub module: naga::Module,
    pub info: naga::valid::ModuleInfo,
    pub wgsl: String,
    pub interface: Interface,
}

/// Combined sampler types: (texture type, sampler type, compares depth).
const OPAQUE: &[(&str, &str, &str, bool)] = &[
    ("sampler1D", "texture1D", "sampler", false),
    ("sampler2D", "texture2D", "sampler", false),
    ("sampler3D", "texture3D", "sampler", false),
    ("samplerCube", "textureCube", "sampler", false),
    ("sampler2DArray", "texture2DArray", "sampler", false),
    ("sampler2DShadow", "texture2D", "samplerShadow", true),
    (
        "sampler2DArrayShadow",
        "texture2DArray",
        "samplerShadow",
        true,
    ),
    ("samplerCubeShadow", "textureCube", "samplerShadow", true),
    ("isampler2D", "itexture2D", "sampler", false),
    ("usampler2D", "utexture2D", "sampler", false),
    ("isampler3D", "itexture3D", "sampler", false),
    ("usampler3D", "utexture3D", "sampler", false),
    ("isampler2DArray", "itexture2DArray", "sampler", false),
    ("usampler2DArray", "utexture2DArray", "sampler", false),
];

fn opaque(ty: &str) -> Option<&'static (&'static str, &'static str, &'static str, bool)> {
    OPAQUE.iter().find(|o| o.0 == ty)
}

/// Compatibility-profile and ARB calls and their core names.
const LEGACY_CALLS: &[(&str, &str)] = &[
    ("texture1D", "texture"),
    ("texture2D", "texture"),
    ("texture3D", "texture"),
    ("textureCube", "texture"),
    ("texture2DLod", "textureLod"),
    ("texture2DLodARB", "textureLod"),
    ("texture3DLod", "textureLod"),
    ("textureCubeLod", "textureLod"),
    ("texture2DGrad", "textureGrad"),
    ("texture2DGradARB", "textureGrad"),
    ("texture2DProj", "textureProj"),
    ("texture2DProjLod", "textureProjLod"),
    ("texelFetch2D", "texelFetch"),
    ("isnan", "iris_isnan"),
    ("isinf", "iris_isinf"),
    ("modf", "iris_modf"),
];

/// Legacy shadow lookups, which return a `vec4`.
const SHADOW_CALLS: &[(&str, &str)] = &[
    ("shadow2D", "texture"),
    ("shadow2DLod", "textureLod"),
    ("shadow2DProj", "textureProj"),
];

/// Compatibility-profile uniforms: (name, member declaration).
const LEGACY_UNIFORMS: &[(&str, &str)] = &[
    ("gl_ModelViewMatrix", "mat4 iris_ModelViewMatrix"),
    ("gl_ProjectionMatrix", "mat4 iris_ProjectionMatrix"),
    (
        "gl_ModelViewProjectionMatrix",
        "mat4 iris_ModelViewProjectionMatrix",
    ),
    ("gl_NormalMatrix", "mat3 iris_NormalMatrix"),
    (
        "gl_ModelViewMatrixInverse",
        "mat4 iris_ModelViewMatrixInverse",
    ),
    (
        "gl_ProjectionMatrixInverse",
        "mat4 iris_ProjectionMatrixInverse",
    ),
    ("gl_TextureMatrix", "mat4 iris_TextureMatrix[8]"),
    ("gl_Fog", "iris_FogParameters iris_Fog"),
];

/// Compatibility-profile vertex attributes: (name, type).
const LEGACY_ATTRIBUTES: &[(&str, &str)] = &[
    ("gl_Vertex", "vec4"),
    ("gl_Normal", "vec3"),
    ("gl_Color", "vec4"),
    ("gl_MultiTexCoord0", "vec4"),
    ("gl_MultiTexCoord1", "vec4"),
    ("gl_MultiTexCoord2", "vec4"),
    ("gl_MultiTexCoord3", "vec4"),
    ("gl_FogCoord", "float"),
];

/// Iris's core-profile names, declared when a program uses them without
/// declaring them: (name, type, is an attribute).
const IRIS_IMPLICIT: &[(&str, &str, bool)] = &[
    ("vaPosition", "vec3", true),
    ("vaColor", "vec4", true),
    ("vaUV0", "vec2", true),
    ("vaUV1", "ivec2", true),
    ("vaUV2", "ivec2", true),
    ("vaNormal", "vec3", true),
    ("modelViewMatrix", "mat4", false),
    ("modelViewMatrixInverse", "mat4", false),
    ("projectionMatrix", "mat4", false),
    ("projectionMatrixInverse", "mat4", false),
    ("normalMatrix", "mat3", false),
    ("textureMatrix", "mat4", false),
    ("chunkOffset", "vec3", false),
    ("alphaTestRef", "float", false),
];

/// Built-ins naga lacks. `isnan` and `isinf` test the bits: Metal compiles
/// with fast math, which folds `x != x` to false.
const HELPERS: &str = "\
struct iris_FogParameters { vec4 color; float density; float start; float end; float scale; };
bool iris_isnan(float x) { return (floatBitsToUint(x) & 0x7fffffffu) > 0x7f800000u; }
bvec2 iris_isnan(vec2 x) { return greaterThan(floatBitsToUint(x) & 0x7fffffffu, uvec2(0x7f800000u)); }
bvec3 iris_isnan(vec3 x) { return greaterThan(floatBitsToUint(x) & 0x7fffffffu, uvec3(0x7f800000u)); }
bvec4 iris_isnan(vec4 x) { return greaterThan(floatBitsToUint(x) & 0x7fffffffu, uvec4(0x7f800000u)); }
bool iris_isinf(float x) { return (floatBitsToUint(x) & 0x7fffffffu) == 0x7f800000u; }
bvec2 iris_isinf(vec2 x) { return equal(floatBitsToUint(x) & 0x7fffffffu, uvec2(0x7f800000u)); }
bvec3 iris_isinf(vec3 x) { return equal(floatBitsToUint(x) & 0x7fffffffu, uvec3(0x7f800000u)); }
bvec4 iris_isinf(vec4 x) { return equal(floatBitsToUint(x) & 0x7fffffffu, uvec4(0x7f800000u)); }
float iris_modf(float x, out float i) { i = trunc(x); return x - i; }
vec2 iris_modf(vec2 x, out vec2 i) { i = trunc(x); return x - i; }
vec3 iris_modf(vec3 x, out vec3 i) { i = trunc(x); return x - i; }
vec4 iris_modf(vec4 x, out vec4 i) { i = trunc(x); return x - i; }
";

/// Overloaded `mat2` constructor helper: drivers accept `mat2(vec4)` (Bloop),
/// which naga's GLSL front end refuses.
const MAT2_HELPERS: &str = "\
mat2 iris_mat2(float x) { return mat2(x); }
mat2 iris_mat2(vec2 a, vec2 b) { return mat2(a, b); }
mat2 iris_mat2(float a, float b, float c, float d) { return mat2(a, b, c, d); }
mat2 iris_mat2(vec4 v) { return mat2(v.xy, v.zw); }
mat2 iris_mat2(mat2 m) { return m; }
mat2 iris_mat2(mat3 m) { return mat2(m); }
mat2 iris_mat2(mat4 m) { return mat2(m); }
";

/// A fragment output narrower than four channels (`out vec3 color`,
/// Steadfast's `final`): the four-channel type it is written through and the
/// epilogue line that fills it, with alpha 1 and the rest 0. `None` when the
/// output already has four channels.
fn widen_output(ty: &str, name: &str) -> Option<(String, String)> {
    let (wide, one, zero) = match ty {
        "float" | "vec2" | "vec3" => ("vec4", "1.0", "0.0"),
        "int" | "ivec2" | "ivec3" => ("ivec4", "1", "0"),
        "uint" | "uvec2" | "uvec3" => ("uvec4", "1u", "0u"),
        _ => return None,
    };
    let channels = match ty.chars().last() {
        Some('2') => 2,
        Some('3') => 3,
        _ => 1,
    };
    let mut parts = vec![name.to_owned()];
    parts.extend((channels..3).map(|_| zero.to_owned()));
    parts.push(one.to_owned());
    Some((
        wide.to_owned(),
        format!("iris_out_{name} = {wide}({});", parts.join(", ")),
    ))
}

/// `%` as overloads: naga's GLSL front end splats a scalar operand for
/// `+ - / & ^ |` but not for `%`, and rejects `ivec2(uv) % 256` (Noble).
fn modulo_helpers() -> String {
    let mut out = String::new();
    for (s, v) in [("int", "ivec"), ("uint", "uvec")] {
        out.push_str(&format!("{s} iris_mod({s} a, {s} b) {{ return a % b; }}\n"));
        for n in 2..=4 {
            let v = format!("{v}{n}");
            out.push_str(&format!("{v} iris_mod({v} a, {v} b) {{ return a % b; }}\n"));
            out.push_str(&format!(
                "{v} iris_mod({v} a, {s} b) {{ return a % {v}(b); }}\n"
            ));
            out.push_str(&format!(
                "{v} iris_mod({s} a, {v} b) {{ return {v}(a) % b; }}\n"
            ));
        }
    }
    out
}

/// 16- and 8-bit types (`GL_NV_gpu_shader5`, `GL_EXT_shader_explicit_arithmetic_types`)
/// and their built-ins, as the 32-bit ones WGSL has: Noble declares
/// `u16vec2` & co. when Iris reports an NVIDIA GPU, and falls back to these
/// same aliases elsewhere.
const SMALL_TYPES: &[(&str, &str)] = &[
    ("float16_t", "float"),
    ("f16vec2", "vec2"),
    ("f16vec3", "vec3"),
    ("f16vec4", "vec4"),
    ("int16_t", "int"),
    ("i16vec2", "ivec2"),
    ("i16vec3", "ivec3"),
    ("i16vec4", "ivec4"),
    ("uint16_t", "uint"),
    ("u16vec2", "uvec2"),
    ("u16vec3", "uvec3"),
    ("u16vec4", "uvec4"),
    ("int8_t", "int"),
    ("i8vec2", "ivec2"),
    ("i8vec3", "ivec3"),
    ("i8vec4", "ivec4"),
    ("uint8_t", "uint"),
    ("u8vec2", "uvec2"),
    ("u8vec3", "uvec3"),
    ("u8vec4", "uvec4"),
    ("packFloat2x16", "packHalf2x16"),
    ("unpackFloat2x16", "unpackHalf2x16"),
];

/// `source` with [`SMALL_TYPES`] replaced, word by word.
fn widen_small_types(source: &str) -> std::borrow::Cow<'_, str> {
    if !SMALL_TYPES.iter().any(|(t, _)| source.contains(t)) {
        return source.into();
    }
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(c) = rest.chars().next() {
        let len = if c.is_ascii_alphanumeric() || c == '_' {
            rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(rest.len())
        } else {
            c.len_utf8()
        };
        let word = &rest[..len];
        out.push_str(
            SMALL_TYPES
                .iter()
                .find(|(t, _)| *t == word)
                .map_or(word, |(_, w)| w),
        );
        rest = &rest[len..];
    }
    out.into()
}

/// Translate one preprocessed program. `locate` turns a line of `source`
/// into `file:line` for errors.
pub fn translate(
    stage: Stage,
    source: &str,
    link: &mut Link,
    locate: &dyn Fn(u32) -> String,
) -> Result<Translated, String> {
    let naga_stage = match stage {
        Stage::Vertex => naga::ShaderStage::Vertex,
        Stage::Fragment => naga::ShaderStage::Fragment,
        Stage::Compute => naga::ShaderStage::Compute,
        _ => return Err(format!("{stage:?} programs are not supported")),
    };
    let source = widen_small_types(source);
    let mut unit = ast::TranslationUnit::parse(&source).map_err(|e| {
        let line = e.line() + 1;
        format!("{}: {}", locate(line), e.inner())
    })?;
    let mut legaliser = Legaliser::new(stage, link);
    legaliser.unit(&mut unit)?;
    let glsl = legaliser.finish(&unit)?;
    let options = naga::front::glsl::Options {
        stage: naga_stage,
        defines: Default::default(),
    };
    let mut module = naga::front::glsl::Frontend::default()
        .parse(&options, &glsl)
        .map_err(|e| first_lines(&e.emit_to_string(&glsl)))?;
    unpad_private_arrays(&mut module);
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map_err(|e| first_lines(&e.emit_to_string(&glsl)))?;
    let wgsl =
        naga::back::wgsl::write_string(&module, &info, naga::back::wgsl::WriterFlags::empty())
            .map_err(|e| e.to_string())?;
    Ok(Translated {
        glsl,
        module,
        info,
        wgsl,
        interface: legaliser.interface,
    })
}

/// Built-in functions naga folds in `const` initialisers.
const FOLDABLE: &[&str] = &[
    "radians",
    "degrees",
    "sin",
    "cos",
    "tan",
    "asin",
    "acos",
    "atan",
    "pow",
    "exp",
    "log",
    "exp2",
    "log2",
    "sqrt",
    "inversesqrt",
    "abs",
    "sign",
    "floor",
    "trunc",
    "round",
    "ceil",
    "fract",
    "mod",
    "min",
    "max",
    "clamp",
    "step",
    "length",
    "distance",
    "dot",
    "cross",
    "normalize",
];

fn constructor(name: &str) -> bool {
    ["bool", "int", "uint", "float", "double"].contains(&name)
        || ["vec", "ivec", "uvec", "bvec", "dvec", "mat", "dmat"]
            .iter()
            .any(|p| name.starts_with(p))
}

/// Whether `list` has an initialiser naga's constant folder refuses (`?:`,
/// `mix`, `smoothstep`, `1 << 31`, or a reference to a constant already turned
/// into a global).
fn needs_runtime(list: &ast::InitDeclaratorList, runtime: &HashSet<String>) -> bool {
    let expr = |e: &ast::Expr| {
        fn walk(e: &ast::Expr, f: &dyn Fn(&ast::Expr) -> bool) -> bool {
            use ast::ExprData as E;
            f(e) || match &e.content {
                E::Unary(_, a) | E::PostInc(a) | E::PostDec(a) | E::Dot(a, _) => walk(a, f),
                E::Binary(_, a, b) | E::Assignment(a, _, b) | E::Comma(a, b) | E::Bracket(a, b) => {
                    walk(a, f) || walk(b, f)
                }
                E::Ternary(a, b, c) => walk(a, f) || walk(b, f) || walk(c, f),
                E::FunCall(_, args) => args.iter().any(|a| walk(a, f)),
                _ => false,
            }
        }
        walk(e, &|e| match &e.content {
            ast::ExprData::Ternary(..) => true,
            ast::ExprData::Binary(op, _, _)
                if matches!(
                    op.content,
                    ast::BinaryOpData::LShift | ast::BinaryOpData::RShift
                ) =>
            {
                true
            }
            ast::ExprData::Variable(v) => runtime.contains(v.as_str()),
            ast::ExprData::FunCall(f, _) => {
                callee(f).is_some_and(|c| !constructor(&c) && !FOLDABLE.contains(&c.as_str()))
            }
            _ => false,
        })
    };
    fn init(i: &ast::Initializer, expr: &dyn Fn(&ast::Expr) -> bool) -> bool {
        match &i.content {
            ast::InitializerData::Simple(e) => expr(e),
            ast::InitializerData::List(items) => items.iter().any(|i| init(i, expr)),
        }
    }
    let tail = list.tail.iter().filter_map(|t| t.initializer.as_ref());
    list.head
        .initializer
        .iter()
        .chain(tail)
        .any(|i| init(i, &expr))
}

/// A negative number: `-1.0`, `-1`.
fn negative_constant(e: &ast::Expr) -> bool {
    use ast::ExprData as E;
    match &e.content {
        E::Unary(op, a) if op.content == ast::UnaryOpData::Minus => {
            matches!(
                a.content,
                E::FloatConst(v) if v > 0.0
            ) || matches!(a.content, E::IntConst(v) if v > 0)
                || matches!(a.content, E::DoubleConst(v) if v > 0.0)
        }
        E::FloatConst(v) => *v < 0.0,
        _ => false,
    }
}

/// naga's GLSL front end lays every struct out as std140, so an array field
/// of a struct that never reaches a buffer (Photon's ACES spline parameters,
/// `float[6]`) gets a stride of 16, and constructing it from a `float[6](..)`
/// (stride 4) fails validation. Give such fields their natural stride.
fn unpad_private_arrays(module: &mut naga::Module) {
    use naga::TypeInner as T;
    let mut shared = HashSet::new();
    let mut stack: Vec<_> = module
        .global_variables
        .iter()
        .filter(|(_, v)| {
            matches!(
                v.space,
                naga::AddressSpace::Uniform
                    | naga::AddressSpace::Storage { .. }
                    | naga::AddressSpace::Immediate
            )
        })
        .map(|(_, v)| v.ty)
        .collect();
    while let Some(ty) = stack.pop() {
        if !shared.insert(ty) {
            continue;
        }
        match &module.types[ty].inner {
            T::Struct { members, .. } => stack.extend(members.iter().map(|m| m.ty)),
            T::Array { base, .. } | T::BindingArray { base, .. } => stack.push(*base),
            _ => {}
        }
    }
    let mut layouter = naga::proc::Layouter::default();
    if layouter.update(module.to_ctx()).is_err() {
        return;
    }
    let structs: Vec<_> = module
        .types
        .iter()
        .filter(|(h, t)| !shared.contains(h) && matches!(t.inner, T::Struct { .. }))
        .map(|(h, _)| h)
        .collect();
    for ty in structs {
        let mut new = module.types[ty].clone();
        let T::Struct { members, .. } = &mut new.inner else {
            continue;
        };
        let mut changed = false;
        for m in members.iter_mut() {
            let T::Array { base, size, stride } = module.types[m.ty].inner else {
                continue;
            };
            let natural = layouter[base].to_stride();
            if stride != natural {
                let span = module.types.get_span(m.ty);
                m.ty = module.types.insert(
                    naga::Type {
                        name: None,
                        inner: T::Array {
                            base,
                            size,
                            stride: natural,
                        },
                    },
                    span,
                );
                changed = true;
            }
        }
        if changed {
            module.types.replace(ty, new);
        }
    }
}

fn first_lines(s: &str) -> String {
    s.lines().take(8).collect::<Vec<_>>().join("\n")
}

/// What a name means inside a function.
#[derive(Clone, Copy)]
enum Local {
    Plain,
    /// A sampler parameter, split into `{name}_t`, `{name}_s`.
    Sampler(&'static str),
}

/// A struct field `(type_name, field_name, array_suffix)`.
type StructField = (String, String, String);

struct Legaliser<'a> {
    stage: Stage,
    link: &'a mut Link,
    interface: Interface,
    /// Global samplers: name → combined type.
    /// Sampler uniforms: their combined type and sampler variable.
    samplers: HashMap<String, (&'static str, String)>,
    /// Sampler variables declared so far (shared ones once).
    sampler_vars: HashSet<String>,
    /// User functions → indices of their sampler parameters.
    sampler_params: HashMap<String, BTreeSet<usize>>,
    functions: HashSet<String>,
    /// User structs → their fields `(ty, name, array)` (for struct varyings).
    structs: HashMap<String, Vec<StructField>>,
    scopes: Vec<HashMap<String, Local>>,
    /// Declarations before the program (bindings, block, inputs, outputs).
    header: Vec<String>,
    /// Statements run at the start and end of `main`.
    prologue: Vec<String>,
    epilogue: Vec<String>,
    /// Names the program declares at global scope.
    declared: HashSet<String>,
    /// Identifiers used, for the implicit declarations.
    used: HashSet<String>,
    errors: Vec<String>,
    /// Inside a constant initialiser: no helper calls there.
    constant: bool,
    /// The program uses `%` ([`modulo_helpers`]), `iris_sqrt` or `iris_mat2`:
    /// their helpers go in.
    modulo: bool,
    nan: bool,
    /// `textureGather` was used: [`GATHER`] is added.
    gather: bool,
    mat2_ctor: bool,
    /// Constants turned into globals ([`needs_runtime`]).
    runtime_constants: HashSet<String>,
}

fn var(name: &str) -> ast::Expr {
    ast::ExprData::Variable(ast::IdentifierData::from(name).into_node()).into_node()
}

fn call(name: &str, args: Vec<ast::Expr>) -> ast::Expr {
    ast::ExprData::FunCall(
        ast::FunIdentifierData::Expr(Box::new(var(name))).into_node(),
        args,
    )
    .into_node()
}

/// `textureGather(sampler2D, uv, comp)`: component `comp` of the four texels
/// bilinear filtering at `uv` would blend, in GL's order, read with
/// `texelFetch` (edges clamped).
const GATHER: &str = "\
vec4 iris_gather(texture2D t, sampler s, vec2 uv, int c) {
    ivec2 hi = textureSize(sampler2D(t, s), 0) - 1;
    ivec2 i = ivec2(floor(uv * vec2(hi + 1) - 0.5));
    vec4 a = texelFetch(sampler2D(t, s), clamp(i + ivec2(0, 1), ivec2(0), hi), 0);
    vec4 b = texelFetch(sampler2D(t, s), clamp(i + ivec2(1, 1), ivec2(0), hi), 0);
    vec4 d = texelFetch(sampler2D(t, s), clamp(i + ivec2(1, 0), ivec2(0), hi), 0);
    vec4 e = texelFetch(sampler2D(t, s), clamp(i, ivec2(0), hi), 0);
    return vec4(a[c], b[c], d[c], e[c]);
}
";

/// GL converts a `bool` given to a numeric constructor (`vec3(r < radius)`,
/// Photon's sun disc); naga refuses. A comparison or logical expression
/// passed to one becomes `(cond ? 1.0 : 0.0)` (or `1`/`1u`).
fn bool_to_number(e: &mut ast::Expr) {
    use ast::BinaryOpData as B;
    let ast::ExprData::FunCall(f, args) = &mut e.content else {
        return;
    };
    let Some(name) = callee(f) else { return };
    let base = name.trim_start_matches('d');
    let (one, zero) = if base.starts_with("ivec") || name == "int" {
        (ast::ExprData::IntConst(1), ast::ExprData::IntConst(0))
    } else if base.starts_with("uvec") || name == "uint" {
        (ast::ExprData::UIntConst(1), ast::ExprData::UIntConst(0))
    } else if base.starts_with("vec") || base.starts_with("mat") || name == "float" {
        (
            ast::ExprData::FloatConst(1.0),
            ast::ExprData::FloatConst(0.0),
        )
    } else {
        return;
    };
    for a in args.iter_mut() {
        let boolean = match &a.content {
            ast::ExprData::Binary(op, ..) => matches!(
                op.content,
                B::Or | B::Xor | B::And | B::Equal | B::NonEqual | B::Lt | B::Gt | B::Lte | B::Gte
            ),
            ast::ExprData::Unary(op, _) => op.content == ast::UnaryOpData::Not,
            _ => false,
        };
        if boolean {
            let cond = std::mem::replace(a, var("_"));
            *a = ast::ExprData::Ternary(
                Box::new(cond),
                Box::new(one.clone().into_node()),
                Box::new(zero.clone().into_node()),
            )
            .into_node();
        }
    }
}

/// Outside fragment shaders there are no derivatives, so GL samples level 0
/// where `texture` and its kin would pick one; naga refuses them there (the
/// vertex programs of Chocapic13', MakeUp and Bliss). Turn them into their
/// `Lod` forms at level 0 (a bias is dropped), also inside `vec4(..)` as
/// legacy shadow lookups are wrapped.
fn explicit_lod(e: &mut ast::Expr) {
    let ast::ExprData::FunCall(f, args) = &mut e.content else {
        return;
    };
    let Some(name) = callee(f) else { return };
    if name == "vec4" && args.len() == 1 {
        return explicit_lod(&mut args[0]);
    }
    // (implicit, explicit, where the level goes, how many arguments without a bias)
    let (lod, at, plain) = match name.as_str() {
        "texture" => ("textureLod", 2, 2),
        "textureProj" => ("textureProjLod", 2, 2),
        "textureOffset" => ("textureLodOffset", 2, 3),
        "textureProjOffset" => ("textureProjLodOffset", 2, 3),
        _ => return,
    };
    if args.len() < plain {
        return;
    }
    args.truncate(plain);
    let zero = ast::ExprData::FloatConst(0.0).into_node();
    args.insert(at, zero);
    *f = ast::FunIdentifierData::Expr(Box::new(var(lod))).into_node();
}

/// The name a call is made by: a function, or a constructor's type.
fn callee(f: &ast::FunIdentifier) -> Option<String> {
    match &f.content {
        ast::FunIdentifierData::Expr(e) => match &e.content {
            ast::ExprData::Variable(id) => Some(id.as_str().to_owned()),
            _ => None,
        },
        ast::FunIdentifierData::TypeSpecifier(t) if t.array_specifier.is_none() => {
            Some(type_text(t))
        }
        ast::FunIdentifierData::TypeSpecifier(_) => None,
    }
}

fn type_text(t: &ast::TypeSpecifier) -> String {
    let mut s = String::new();
    let _ = show_type_specifier(&mut s, t, &mut FormattingState::default());
    s
}

fn expr_text(e: &ast::Expr) -> String {
    let mut s = String::new();
    let _ = show_expr(&mut s, e, &mut FormattingState::default());
    s
}

fn storage(q: &Option<ast::TypeQualifier>) -> Vec<ast::StorageQualifierData> {
    q.iter()
        .flat_map(|q| &q.qualifiers)
        .filter_map(|s| match &s.content {
            ast::TypeQualifierSpecData::Storage(s) => Some(s.content.clone()),
            _ => None,
        })
        .collect()
}

/// The interpolation and `centroid` words of a declaration.
fn interpolation(q: &Option<ast::TypeQualifier>) -> String {
    let mut out = String::new();
    for s in q.iter().flat_map(|q| &q.qualifiers) {
        match &s.content {
            ast::TypeQualifierSpecData::Interpolation(i) => out.push_str(match i.content {
                ast::InterpolationQualifierData::Flat => "flat ",
                ast::InterpolationQualifierData::Smooth => "smooth ",
                ast::InterpolationQualifierData::NoPerspective => "noperspective ",
            }),
            ast::TypeQualifierSpecData::Storage(s)
                if s.content == ast::StorageQualifierData::Centroid =>
            {
                out.push_str("centroid ")
            }
            _ => {}
        }
    }
    out
}

/// `layout(...)` entries of a declaration, as text (`rgba16f`, `location = 0`).
fn layout_ids(q: &Option<ast::TypeQualifier>) -> Vec<(String, Option<String>)> {
    q.iter()
        .flat_map(|q| &q.qualifiers)
        .filter_map(|s| match &s.content {
            ast::TypeQualifierSpecData::Layout(l) => Some(l),
            _ => None,
        })
        .flat_map(|l| &l.ids)
        .map(|id| match &id.content {
            ast::LayoutQualifierSpecData::Identifier(i, e) => {
                (i.as_str().to_owned(), e.as_ref().map(|e| expr_text(e)))
            }
            ast::LayoutQualifierSpecData::Shared => ("shared".to_owned(), None),
        })
        .collect()
}

/// Array sizes as text (`[3]`), `[]` for unsized ones.
fn array_text(a: &Option<ast::ArraySpecifier>) -> String {
    a.iter()
        .flat_map(|a| &a.dimensions)
        .map(|d| match &d.content {
            ast::ArraySpecifierDimensionData::Unsized => "[]".to_owned(),
            ast::ArraySpecifierDimensionData::ExplicitlySized(e) => format!("[{}]", expr_text(e)),
        })
        .collect()
}

/// Parse one or more `[N]` dimensions (`[9][9]` → `Some(vec![9, 9])`).
fn array_dims(array: &str) -> Option<Vec<usize>> {
    if array.is_empty() {
        return Some(Vec::new());
    }
    let mut dims = Vec::new();
    let mut rest = array.trim();
    while !rest.is_empty() {
        let inner = rest.strip_prefix('[')?;
        let (num, tail) = inner.split_once(']')?;
        dims.push(num.trim().parse::<usize>().ok()?);
        rest = tail.trim();
    }
    Some(dims)
}

/// Expand multi-dimensional indices `dims` into `(key_suffix, expr_suffix)`
/// pairs (`[2, 2]` → `("#0#0", "[0][0]")`, `("#0#1", "[0][1]")`, ...).
fn expand_dims(dims: &[usize]) -> Vec<(String, String)> {
    let mut out = vec![(String::new(), String::new())];
    for &n in dims {
        let mut next = Vec::with_capacity(out.len() * n);
        for (k, e) in &out {
            for i in 0..n {
                next.push((format!("{k}#{i}"), format!("{e}[{i}]")));
            }
        }
        out = next;
    }
    out
}

/// The element count of `T[](a, b, c)`.
fn constructor_len(init: &Option<ast::Initializer>) -> Option<usize> {
    match &init.as_ref()?.content {
        ast::InitializerData::Simple(e) => match &e.content {
            ast::ExprData::FunCall(f, args) => match &f.content {
                ast::FunIdentifierData::TypeSpecifier(t) if t.array_specifier.is_some() => {
                    Some(args.len())
                }
                _ => None,
            },
            _ => None,
        },
        ast::InitializerData::List(items) => Some(items.len()),
    }
}

/// Size `[]` in a declaration and in its `T[](...)` initialiser.
fn size_arrays(
    len: usize,
    decl_array: &mut Option<ast::ArraySpecifier>,
    ty: &mut ast::TypeSpecifier,
    init: &mut Option<ast::Initializer>,
) {
    let sized = || {
        ast::ArraySpecifierData {
            dimensions: vec![
                ast::ArraySpecifierDimensionData::ExplicitlySized(Box::new(
                    ast::ExprData::IntConst(len as i32).into_node(),
                ))
                .into_node(),
            ],
        }
        .into_node()
    };
    let is_unsized = |a: &Option<ast::ArraySpecifier>| {
        a.as_ref().is_some_and(|a| {
            a.dimensions.len() == 1
                && a.dimensions[0].content == ast::ArraySpecifierDimensionData::Unsized
        })
    };
    if is_unsized(decl_array) {
        *decl_array = Some(sized());
    }
    if is_unsized(&ty.array_specifier) {
        ty.array_specifier = Some(sized());
    }
    if let Some(ast::InitializerData::Simple(e)) = init.as_mut().map(|i| &mut i.content) {
        if let ast::ExprData::FunCall(f, _) = &mut e.content {
            if let ast::FunIdentifierData::TypeSpecifier(t) = &mut f.content {
                if is_unsized(&t.array_specifier) {
                    t.array_specifier = Some(sized());
                }
            }
        }
    }
}

/// Columns of a matrix type (`mat3` → 3 × `vec3`, `mat2x4` → 2 × `vec4`).
fn matrix_columns(ty: &str) -> Option<(usize, String)> {
    let rest = ty.strip_prefix("mat").or_else(|| ty.strip_prefix("dmat"))?;
    let (cols, rows) = match rest.split_once('x') {
        Some((c, r)) => (c.parse().ok()?, r.parse::<usize>().ok()?),
        None => (rest.parse().ok()?, rest.parse().ok()?),
    };
    Some((cols, format!("vec{rows}")))
}

fn is_integer(ty: &str) -> bool {
    ty.starts_with("int")
        || ty.starts_with("uint")
        || ty.starts_with("ivec")
        || ty.starts_with("uvec")
}

impl<'a> Legaliser<'a> {
    fn new(stage: Stage, link: &'a mut Link) -> Self {
        Self {
            stage,
            link,
            interface: Interface::default(),
            samplers: HashMap::new(),
            sampler_vars: HashSet::new(),
            sampler_params: HashMap::new(),
            functions: HashSet::new(),
            structs: HashMap::new(),
            scopes: Vec::new(),
            header: Vec::new(),
            prologue: Vec::new(),
            epilogue: Vec::new(),
            declared: HashSet::new(),
            used: HashSet::new(),
            errors: Vec::new(),
            constant: false,
            modulo: false,
            nan: false,
            gather: false,
            mat2_ctor: false,
            runtime_constants: HashSet::new(),
        }
    }

    fn record_struct(&mut self, spec: &ast::StructSpecifier) {
        let Some(name) = &spec.name else { return };
        let mut fields = Vec::new();
        for field in &spec.fields {
            let base_ty = type_text(&field.ty);
            let ty_arr = array_text(&field.ty.array_specifier);
            for id in &field.identifiers {
                let arr = format!("{ty_arr}{}", array_text(&id.array_spec));
                fields.push((base_ty.clone(), id.ident.as_str().to_owned(), arr));
            }
        }
        self.structs.insert(name.as_str().to_owned(), fields);
    }

    fn unit(&mut self, unit: &mut ast::TranslationUnit) -> Result<(), String> {
        // Functions, their sampler parameters, and struct definitions first.
        for ed in &unit.0 {
            if let ast::ExternalDeclarationData::Declaration(d) = &ed.content
                && let ast::DeclarationData::InitDeclaratorList(list) = &d.content
                && let ast::TypeSpecifierNonArrayData::Struct(s) = &list.head.ty.ty.ty.content
            {
                self.record_struct(s);
            }
            let proto = match &ed.content {
                ast::ExternalDeclarationData::FunctionDefinition(f) => &f.prototype,
                ast::ExternalDeclarationData::Declaration(d) => match &d.content {
                    ast::DeclarationData::FunctionPrototype(p) => p,
                    _ => continue,
                },
                _ => continue,
            };
            let name = proto.name.as_str().to_owned();
            for (i, p) in proto.parameters.iter().enumerate() {
                let ty = match &p.content {
                    ast::FunctionParameterDeclarationData::Named(_, d) => &d.ty,
                    ast::FunctionParameterDeclarationData::Unnamed(_, t) => t,
                };
                if opaque(&type_text(ty)).is_some() {
                    self.sampler_params
                        .entry(name.clone())
                        .or_default()
                        .insert(i);
                }
            }
            self.functions.insert(name);
        }
        let mut kept = Vec::with_capacity(unit.0.len());
        for mut ed in std::mem::take(&mut unit.0) {
            let keep = match &mut ed.content {
                ast::ExternalDeclarationData::Preprocessor(_) => false,
                ast::ExternalDeclarationData::FunctionDefinition(f) => {
                    self.function(f);
                    true
                }
                ast::ExternalDeclarationData::Declaration(d) => self.global(d)?,
            };
            if keep {
                kept.push(ed);
            }
        }
        unit.0 = kept;
        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(self.errors.join("\n"))
        }
    }

    /// A global declaration; `false` when it moves into the header.
    fn global(&mut self, d: &mut ast::Declaration) -> Result<bool, String> {
        match &mut d.content {
            ast::DeclarationData::FunctionPrototype(p) => {
                self.prototype(p);
                Ok(true)
            }
            ast::DeclarationData::Precision(..) => Ok(false),
            ast::DeclarationData::Block(b) => {
                let binding = self.link.binding(b.name.as_str());
                let buffer = storage(&Some(b.qualifier.clone()))
                    .contains(&ast::StorageQualifierData::Buffer);
                let kind = if buffer { "std430" } else { "std140" };
                b.qualifier
                    .qualifiers
                    .retain(|q| !matches!(q.content, ast::TypeQualifierSpecData::Layout(_)));
                let mut text = String::new();
                show_declaration(&mut text, d, &mut FormattingState::default())
                    .map_err(|e| e.to_string())?;
                self.header.push(format!(
                    "layout({kind}, set = 0, binding = {binding}) {}",
                    text.trim()
                ));
                Ok(false)
            }
            ast::DeclarationData::Invariant(_) | ast::DeclarationData::TypeOnly(_) => Ok(true),
            ast::DeclarationData::InitDeclaratorList(list) => self.global_list(list),
        }
    }

    fn global_list(&mut self, list: &mut ast::InitDeclaratorList) -> Result<bool, String> {
        // If a standalone `struct S { ... };` has no declarator names, keep it
        // in the header so varyings of type `S` can declare `S varname;` there.
        if list.head.name.is_none()
            && list.tail.is_empty()
            && matches!(
                list.head.ty.ty.ty.content,
                ast::TypeSpecifierNonArrayData::Struct(_)
            )
        {
            let mut text = String::new();
            let decl = ast::DeclarationData::InitDeclaratorList(list.clone()).into_node();
            show_declaration(&mut text, &decl, &mut FormattingState::default())
                .map_err(|e| e.to_string())?;
            self.header.push(text.trim().to_owned());
            return Ok(false);
        }
        let qualifier = list.head.ty.qualifier.clone();
        let storage = storage(&qualifier);
        let has = |s: ast::StorageQualifierData| storage.contains(&s);
        let ty = type_text(&list.head.ty.ty);
        let ty_array = array_text(&list.head.ty.ty.array_specifier);
        let mut names: Vec<(String, String)> = Vec::new();
        if let Some(n) = &list.head.name {
            names.push((
                n.as_str().to_owned(),
                array_text(&list.head.array_specifier),
            ));
        }
        for t in &list.tail {
            names.push((
                t.ident.ident.as_str().to_owned(),
                array_text(&t.ident.array_spec),
            ));
        }
        self.declared.extend(names.iter().map(|(n, _)| n.clone()));
        let input = has(ast::StorageQualifierData::Attribute)
            || has(ast::StorageQualifierData::In) && self.stage == Stage::Vertex;
        let varying = has(ast::StorageQualifierData::Varying)
            || has(ast::StorageQualifierData::Out) && self.stage == Stage::Vertex
            || has(ast::StorageQualifierData::In) && self.stage == Stage::Fragment;
        if has(ast::StorageQualifierData::Uniform) {
            if let Some(&(combined, texture, sampler, compare)) = opaque(&ty) {
                for (name, _) in &names {
                    let t = self.link.binding(&format!("{name}_t"));
                    self.header.push(format!(
                        "layout(set = 0, binding = {t}) uniform {texture} {name}_t;"
                    ));
                    let shared = &self.link.shared_samplers;
                    let var = if shared.is_empty() {
                        format!("{name}_s")
                    } else if compare {
                        "iris_compare".to_owned()
                    } else if combined.starts_with(['i', 'u']) {
                        "iris_nearest".to_owned()
                    } else {
                        shared
                            .get(name)
                            .cloned()
                            .unwrap_or_else(|| format!("{name}_s"))
                    };
                    if self.sampler_vars.insert(var.clone()) {
                        let s = self.link.binding(&var);
                        self.header.push(format!(
                            "layout(set = 0, binding = {s}) uniform {sampler} {var};"
                        ));
                    }
                    self.interface
                        .sampler_vars
                        .insert(name.clone(), var.clone());
                    self.samplers.insert(name.clone(), (combined, var));
                    self.interface
                        .samplers
                        .push((name.clone(), combined.to_owned()));
                }
            } else if ["image", "iimage", "uimage"]
                .iter()
                .any(|p| ty.starts_with(p))
            {
                let mut layout: Vec<String> = Vec::new();
                let ids = layout_ids(&qualifier);
                let mut format = ids
                    .iter()
                    .find(|(k, v)| v.is_none() && k != "shared")
                    .map(|(k, _)| k.clone());
                let memory: String = storage
                    .iter()
                    .filter_map(|s| match s {
                        ast::StorageQualifierData::ReadOnly => Some("readonly "),
                        ast::StorageQualifierData::WriteOnly => Some("writeonly "),
                        ast::StorageQualifierData::Coherent => Some("coherent "),
                        ast::StorageQualifierData::Restrict => Some("restrict "),
                        ast::StorageQualifierData::Volatile => Some("volatile "),
                        _ => None,
                    })
                    .collect();
                for (name, _) in &names {
                    let f = format
                        .take()
                        .or_else(|| self.link.image_formats.get(name).cloned())
                        .unwrap_or_else(|| "rgba8".to_owned());
                    let binding = self.link.binding(name);
                    layout.push(format!(
                        "layout({f}, set = 0, binding = {binding}) {memory}uniform {ty} {name};"
                    ));
                    self.interface.images.push((name.clone(), f));
                }
                self.header.extend(layout);
            } else {
                if ty.starts_with("struct") || ty_array.contains("[]") {
                    return Err(format!(
                        "uniform {}: unsupported type {ty}{ty_array}",
                        names[0].0
                    ));
                }
                for (name, array) in &names {
                    if ty == "bool" {
                        self.link
                            .uniform(name, format!("int iris_bool_{name}{array}"))?;
                        self.header.push(format!("bool {name}{array};"));
                        self.prologue
                            .push(format!("{name} = iris_bool_{name} != 0;"));
                    } else {
                        self.link
                            .uniform(name, format!("{ty}{ty_array} {name}{array}"))?;
                    }
                    self.interface.uniforms.insert(name.clone());
                }
            }
            return Ok(false);
        }
        if input || varying || has(ast::StorageQualifierData::Out) && self.stage == Stage::Fragment
        {
            let dir = if input || varying && self.stage == Stage::Fragment {
                "in"
            } else {
                "out"
            };
            let interp = interpolation(&qualifier);
            let explicit = layout_ids(&qualifier)
                .into_iter()
                .find(|(k, _)| k == "location")
                .and_then(|(_, v)| v?.parse().ok());
            for (i, (name, array)) in names.iter().enumerate() {
                let full_array = format!("{ty_array}{array}");
                if input {
                    let at = Link::location(&mut self.link.attributes, name);
                    self.header.push(format!(
                        "layout(location = {at}) in {ty} {name}{full_array};"
                    ));
                    self.interface.attributes.insert(name.clone());
                } else if !varying {
                    let at = explicit
                        .map(|l: u32| l + i as u32)
                        .unwrap_or_else(|| self.next_output());
                    if let (Some(widened), "") = (widen_output(&ty, name), full_array.as_str()) {
                        // GL fills the missing channels of a narrow output;
                        // wgpu wants every channel of the target written.
                        let (wide, fill) = widened;
                        self.header.push(format!("{ty} {name};"));
                        self.header.push(format!(
                            "layout(location = {at}) out {wide} iris_out_{name};"
                        ));
                        self.epilogue.push(fill);
                    } else {
                        self.header.push(format!(
                            "layout(location = {at}) out {ty} {name}{full_array};"
                        ));
                    }
                    self.interface.outputs.insert(at, name.clone());
                } else {
                    self.varying(dir, &interp, &ty, name, &full_array);
                }
            }
            return Ok(false);
        }
        // A constant naga can't fold becomes a global set at the start of `main`.
        if has(ast::StorageQualifierData::Const) && needs_runtime(list, &self.runtime_constants) {
            self.runtime_constants
                .extend(names.iter().map(|(n, _)| n.clone()));
            let q = &mut list.head.ty.qualifier;
            if let Some(q) = q {
                q.qualifiers.retain(|q| {
                    !matches!(&q.content, ast::TypeQualifierSpecData::Storage(s)
                        if s.content == ast::StorageQualifierData::Const)
                });
            }
            if q.as_ref().is_some_and(|q| q.qualifiers.is_empty()) {
                *q = None;
            }
        } else if has(ast::StorageQualifierData::Const) || has(ast::StorageQualifierData::Shared) {
            self.sized(list);
            self.constant = true;
            self.global_initialisers(list, false);
            self.constant = false;
            return Ok(true);
        }
        self.sized(list);
        self.global_initialisers(list, true);
        Ok(true)
    }

    fn next_output(&self) -> u32 {
        (0..)
            .find(|l| !self.interface.outputs.contains_key(l))
            .unwrap()
    }

    /// Emit scalar/vector/matrix varying leaves for an expression `expr_path`
    /// with key `key_prefix`.
    fn varying_leaf(&mut self, dir: &str, interp: &str, ty: &str, key: &str, expr_path: &str) {
        if let Some(fields) = self.structs.get(ty).cloned() {
            for (f_ty, f_name, f_arr) in fields {
                let sub_key = format!("{key}#{f_name}");
                let sub_expr = format!("{expr_path}.{f_name}");
                self.varying_composite(dir, interp, &f_ty, &sub_key, &sub_expr, &f_arr);
            }
            return;
        }
        if let Some((cols, column)) = matrix_columns(ty) {
            let flat = if is_integer(&column) && !interp.contains("flat") {
                "flat "
            } else {
                ""
            };
            let mut columns = Vec::new();
            for c in 0..cols {
                let c_key = format!("{key}#{c}");
                let at = Link::location(&mut self.link.varyings, &c_key);
                let part = c_key.replace('#', "_iris");
                self.header.push(format!(
                    "layout(location = {at}) {flat}{interp}{dir} {column} {part};"
                ));
                if dir == "out" {
                    self.epilogue.push(format!("{part} = {expr_path}[{c}];"));
                }
                columns.push(part);
            }
            if dir == "in" {
                self.prologue
                    .push(format!("{expr_path} = {ty}({});", columns.join(", ")));
            }
            return;
        }
        let flat = if is_integer(ty) && !interp.contains("flat") {
            "flat "
        } else {
            ""
        };
        let at = Link::location(&mut self.link.varyings, key);
        let part = key.replace('#', "_iris");
        self.header.push(format!(
            "layout(location = {at}) {flat}{interp}{dir} {ty} {part};"
        ));
        if dir == "out" {
            self.epilogue.push(format!("{part} = {expr_path};"));
        } else {
            self.prologue.push(format!("{expr_path} = {part};"));
        }
    }

    fn varying_composite(
        &mut self,
        dir: &str,
        interp: &str,
        ty: &str,
        key: &str,
        expr_path: &str,
        array: &str,
    ) {
        let Some(dims) = array_dims(array) else {
            self.errors
                .push(format!("varying {expr_path}: unsupported array `{array}`"));
            return;
        };
        if dims.is_empty() {
            self.varying_leaf(dir, interp, ty, key, expr_path);
        } else {
            for (k_suf, e_suf) in expand_dims(&dims) {
                self.varying_leaf(
                    dir,
                    interp,
                    ty,
                    &format!("{key}{k_suf}"),
                    &format!("{expr_path}{e_suf}"),
                );
            }
        }
    }

    /// One varying: matrices, arrays (including multi-dimensional `vec3[9][9]`)
    /// and structs (`CloudsParameters`) split into vectors that a private
    /// global of the original type is packed from or into.
    fn varying(&mut self, dir: &str, interp: &str, ty: &str, name: &str, array: &str) {
        if array.is_empty() && matrix_columns(ty).is_none() && !self.structs.contains_key(ty) {
            let flat = if is_integer(ty) && !interp.contains("flat") {
                "flat "
            } else {
                ""
            };
            let at = Link::location(&mut self.link.varyings, name);
            self.header.push(format!(
                "layout(location = {at}) {flat}{interp}{dir} {ty} {name};"
            ));
            return;
        }
        self.header.push(format!("{ty} {name}{array};"));
        self.varying_composite(dir, interp, ty, name, name, array);
    }

    fn sized(&mut self, list: &mut ast::InitDeclaratorList) {
        let head_ty = list.head.ty.ty.clone();
        let head = &mut list.head.content;
        if let Some(len) = constructor_len(&head.initializer) {
            size_arrays(
                len,
                &mut head.array_specifier,
                &mut head.ty.content.ty,
                &mut head.initializer,
            );
        }
        for t in &mut list.tail {
            let t = &mut t.content;
            if let Some(len) = constructor_len(&t.initializer) {
                let mut ty = head_ty.clone();
                size_arrays(
                    len,
                    &mut t.ident.content.array_spec,
                    &mut ty,
                    &mut t.initializer,
                );
            }
        }
    }

    /// Rewrite global initialisers; with `hoist`, move simple ones to the
    /// start of `main`.
    fn global_initialisers(&mut self, list: &mut ast::InitDeclaratorList, hoist: bool) {
        let take = |this: &mut Self, name: &str, init: &mut Option<ast::Initializer>| {
            let Some(i) = init else { return };
            match &mut i.content {
                ast::InitializerData::Simple(e) => {
                    this.expr(e);
                    if hoist {
                        this.prologue.push(format!("{name} = {};", expr_text(e)));
                        *init = None;
                    }
                }
                ast::InitializerData::List(_) => {}
            }
        };
        if let Some(n) = list.head.name.clone() {
            take(self, n.as_str(), &mut list.head.initializer);
        }
        for t in &mut list.tail {
            let n = t.ident.ident.as_str().to_owned();
            take(self, &n, &mut t.initializer);
        }
    }

    fn prototype(&mut self, p: &mut ast::FunctionPrototype) -> Vec<(String, Local)> {
        if p.name.as_str() == "main" {
            p.name = ast::IdentifierData::from("iris_main").into_node();
        }
        let mut locals = Vec::new();
        let mut params = Vec::with_capacity(p.parameters.len());
        for mut param in std::mem::take(&mut p.parameters) {
            // `const in float x` (Bliss): naga's front end takes no `const`
            // on parameters, and it changes nothing for the caller.
            match &mut param.content {
                ast::FunctionParameterDeclarationData::Named(q, _)
                | ast::FunctionParameterDeclarationData::Unnamed(q, _) => {
                    if let Some(list) = q {
                        list.qualifiers.retain(|q| {
                            !matches!(&q.content, ast::TypeQualifierSpecData::Storage(s)
                                if s.content == ast::StorageQualifierData::Const)
                        });
                        if list.qualifiers.is_empty() {
                            *q = None;
                        }
                    }
                }
            }
            let (ty, name) = match &param.content {
                ast::FunctionParameterDeclarationData::Named(_, d) => {
                    (type_text(&d.ty), Some(d.ident.ident.as_str().to_owned()))
                }
                ast::FunctionParameterDeclarationData::Unnamed(_, t) => (type_text(t), None),
            };
            match (opaque(&ty), name) {
                (Some(&(combined, texture, sampler, _)), Some(name)) => {
                    for (t, n) in [
                        (texture, format!("{name}_t")),
                        (sampler, format!("{name}_s")),
                    ] {
                        let spec = ast::TypeSpecifierData {
                            ty: ast::TypeSpecifierNonArrayData::TypeName(
                                ast::TypeNameData::from(t).into_node(),
                            )
                            .into_node(),
                            array_specifier: None,
                        }
                        .into_node();
                        let ident = ast::ArrayedIdentifierData {
                            ident: ast::IdentifierData::from(n.as_str()).into_node(),
                            array_spec: None,
                        }
                        .into_node();
                        let decl =
                            ast::FunctionParameterDeclaratorData { ty: spec, ident }.into_node();
                        params.push(
                            ast::FunctionParameterDeclarationData::Named(None, decl).into_node(),
                        );
                    }
                    locals.push((name, Local::Sampler(combined)));
                }
                (_, name) => {
                    locals.extend(name.map(|n| (n, Local::Plain)));
                    params.push(param);
                }
            }
        }
        p.parameters = params;
        locals
    }

    fn function(&mut self, f: &mut ast::FunctionDefinition) {
        let locals = self.prototype(&mut f.prototype);
        self.scopes.push(locals.into_iter().collect());
        self.statements(&mut f.statement.statement_list);
        self.scopes.pop();
    }

    fn declare(&mut self, name: &str) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_owned(), Local::Plain);
        }
    }

    fn local_declaration(&mut self, d: &mut ast::Declaration) {
        let ast::DeclarationData::InitDeclaratorList(list) = &mut d.content else {
            return;
        };
        self.sized(list);
        self.constant =
            storage(&list.head.ty.qualifier).contains(&ast::StorageQualifierData::Const);
        if let Some(i) = &mut list.head.initializer {
            self.initializer(i);
        }
        if let Some(n) = &list.head.name {
            let n = n.as_str().to_owned();
            self.declare(&n);
        }
        for t in &mut list.tail {
            if let Some(i) = &mut t.initializer {
                self.initializer(i);
            }
            let n = t.ident.ident.as_str().to_owned();
            self.declare(&n);
        }
        self.constant = false;
    }

    fn initializer(&mut self, i: &mut ast::Initializer) {
        match &mut i.content {
            ast::InitializerData::Simple(e) => self.expr(e),
            ast::InitializerData::List(items) => items.iter_mut().for_each(|i| self.initializer(i)),
        }
    }

    /// A list of statements, without the dead code after an unconditional
    /// `return`, `discard`, `break` or `continue` (up to the next `case`):
    /// naga leaves such code unvisited and rejects the function.
    fn statements(&mut self, list: &mut Vec<ast::Statement>) {
        let mut dead = false;
        list.retain_mut(|s| {
            match s.content {
                ast::StatementData::CaseLabel(_) => dead = false,
                _ if dead => return false,
                ast::StatementData::Jump(_) => dead = true,
                _ => {}
            }
            true
        });
        list.iter_mut().for_each(|s| self.statement(s));
    }

    fn scoped(&mut self, s: &mut ast::Statement) {
        self.scopes.push(HashMap::new());
        self.statement(s);
        self.scopes.pop();
    }

    fn statement(&mut self, s: &mut ast::Statement) {
        match &mut s.content {
            ast::StatementData::Declaration(d) => self.local_declaration(d),
            ast::StatementData::Expression(e) => {
                if let Some(e) = &mut e.0 {
                    self.expr(e);
                }
            }
            ast::StatementData::Selection(sel) => {
                self.expr(&mut sel.cond);
                match &mut sel.rest.content {
                    ast::SelectionRestStatementData::Statement(a) => self.scoped(a),
                    ast::SelectionRestStatementData::Else(a, b) => {
                        self.scoped(a);
                        self.scoped(b);
                    }
                }
            }
            ast::StatementData::Switch(sw) => {
                self.expr(&mut sw.head);
                self.scopes.push(HashMap::new());
                self.statements(&mut sw.body);
                self.scopes.pop();
            }
            ast::StatementData::CaseLabel(c) => {
                if let ast::CaseLabelData::Case(e) = &mut c.content {
                    self.expr(e);
                }
            }
            ast::StatementData::Iteration(it) => {
                self.scopes.push(HashMap::new());
                match &mut it.content {
                    ast::IterationStatementData::While(c, body) => {
                        self.condition(c);
                        self.scoped(body);
                    }
                    ast::IterationStatementData::DoWhile(body, e) => {
                        self.scoped(body);
                        self.expr(e);
                    }
                    ast::IterationStatementData::For(init, rest, body) => {
                        match &mut init.content {
                            ast::ForInitStatementData::Expression(Some(e)) => self.expr(e),
                            ast::ForInitStatementData::Expression(None) => {}
                            ast::ForInitStatementData::Declaration(d) => self.local_declaration(d),
                        }
                        if let Some(c) = &mut rest.condition {
                            self.condition(c);
                        }
                        if let Some(e) = &mut rest.post_expr {
                            self.expr(e);
                        }
                        self.scoped(body);
                    }
                }
                self.scopes.pop();
            }
            ast::StatementData::Jump(j) => {
                if let ast::JumpStatementData::Return(Some(e)) = &mut j.content {
                    self.expr(e);
                }
            }
            ast::StatementData::Compound(c) => {
                self.scopes.push(HashMap::new());
                self.statements(&mut c.statement_list);
                self.scopes.pop();
            }
        }
    }

    fn condition(&mut self, c: &mut ast::Condition) {
        match &mut c.content {
            ast::ConditionData::Expr(e) => self.expr(e),
            ast::ConditionData::Assignment(_, name, init) => {
                self.initializer(init);
                let n = name.as_str().to_owned();
                self.declare(&n);
            }
        }
    }

    fn local(&self, name: &str) -> Option<Local> {
        self.scopes.iter().rev().find_map(|s| s.get(name).copied())
    }

    /// What a variable reads, if it must change.
    fn variable(&mut self, name: &str) -> Option<ast::Expr> {
        let combined = match self.local(name) {
            Some(Local::Sampler(c)) => Some(c),
            Some(Local::Plain) => return None,
            None => self.samplers.get(name).map(|(c, _)| *c),
        };
        if let Some(c) = combined {
            let sampler = match self.samplers.get(name) {
                Some((_, v)) if self.local(name).is_none() => v.clone(),
                _ => format!("{name}_s"),
            };
            return Some(call(c, vec![var(&format!("{name}_t")), var(&sampler)]));
        }
        self.used.insert(name.to_owned());
        if let Some((_, decl)) = LEGACY_UNIFORMS.iter().find(|(n, _)| *n == name) {
            let member = decl.split(' ').nth(1).unwrap().split('[').next().unwrap();
            if let Err(e) = self.link.uniform(member, (*decl).to_owned()) {
                self.errors.push(e);
            }
            self.interface.uniforms.insert(member.to_owned());
            return Some(var(member));
        }
        let renamed = match (name, self.stage) {
            ("gl_FragColor", Stage::Fragment) => return Some(self.frag_data(0)),
            ("gl_VertexID", _) => "gl_VertexIndex".to_owned(),
            ("gl_InstanceID", _) => "gl_InstanceIndex".to_owned(),
            ("gl_FrontColor", Stage::Vertex) | ("gl_Color", Stage::Fragment) => {
                self.legacy_varying("iris_FrontColor", "vec4");
                "iris_FrontColor".to_owned()
            }
            ("gl_FogFragCoord", Stage::Vertex | Stage::Fragment) => {
                self.legacy_varying("iris_FogFragCoord", "float");
                "iris_FogFragCoord".to_owned()
            }
            (n, Stage::Vertex) if LEGACY_ATTRIBUTES.iter().any(|(a, _)| *a == n) => {
                let ty = LEGACY_ATTRIBUTES.iter().find(|(a, _)| *a == n).unwrap().1;
                let renamed = n.replacen("gl_", "iris_", 1);
                if self.interface.attributes.insert(renamed.clone()) {
                    let at = Link::location(&mut self.link.attributes, &renamed);
                    self.header
                        .push(format!("layout(location = {at}) in {ty} {renamed};"));
                }
                renamed
            }
            _ => return None,
        };
        Some(var(&renamed))
    }

    fn legacy_varying(&mut self, name: &str, ty: &str) {
        if self.declared.insert(name.to_owned()) {
            let dir = if self.stage == Stage::Vertex {
                "out"
            } else {
                "in"
            };
            let at = Link::location(&mut self.link.varyings, name);
            self.header
                .push(format!("layout(location = {at}) {dir} {ty} {name};"));
        }
    }

    fn frag_data(&mut self, n: u32) -> ast::Expr {
        let name = format!("iris_FragData{n}");
        if self.interface.outputs.insert(n, name.clone()).is_none() {
            self.header
                .push(format!("layout(location = {n}) out vec4 {name};"));
        }
        var(&name)
    }

    fn expr(&mut self, e: &mut ast::Expr) {
        use ast::ExprData as E;
        match &mut e.content {
            E::Variable(id) => {
                let name = id.as_str().to_owned();
                if let Some(new) = self.variable(&name) {
                    *e = new;
                }
            }
            E::IntConst(_)
            | E::UIntConst(_)
            | E::BoolConst(_)
            | E::FloatConst(_)
            | E::DoubleConst(_) => {}
            E::Unary(_, a) | E::PostInc(a) | E::PostDec(a) => self.expr(a),
            E::Binary(op, a, b) if op.content == ast::BinaryOpData::Mod && !self.constant => {
                self.expr(a);
                self.expr(b);
                self.modulo = true;
                let lhs = std::mem::replace(&mut **a, var("_"));
                let rhs = std::mem::replace(&mut **b, var("_"));
                *e = call("iris_mod", vec![lhs, rhs]);
            }
            E::Binary(op, a, b)
                if op.content == ast::BinaryOpData::LShift
                    && matches!(a.content, E::IntConst(_))
                    && matches!(b.content, E::IntConst(n) if n >= 31) =>
            {
                self.expr(a);
                self.expr(b);
                let lhs = std::mem::replace(&mut **a, var("_"));
                let rhs = std::mem::replace(&mut **b, var("_"));
                let shifted = ast::ExprData::Binary(
                    ast::BinaryOpData::LShift.into_node(),
                    Box::new(call("uint", vec![lhs])),
                    Box::new(call("uint", vec![rhs])),
                )
                .into_node();
                *e = call("int", vec![shifted]);
            }
            E::Assignment(a, op, b)
                if op.content == ast::AssignmentOpData::Mod && !self.constant =>
            {
                self.expr(a);
                self.expr(b);
                self.modulo = true;
                let rhs = std::mem::replace(&mut **b, var("_"));
                **b = call("iris_mod", vec![(**a).clone(), rhs]);
                *op = ast::AssignmentOpData::Equal.into_node();
            }
            E::Binary(_, a, b) | E::Assignment(a, _, b) | E::Comma(a, b) => {
                self.expr(a);
                self.expr(b);
            }
            E::Ternary(a, b, c) => {
                self.expr(a);
                self.expr(b);
                self.expr(c);
            }
            E::Bracket(a, i) => {
                let array = match &a.content {
                    E::Variable(id) if self.local(id.as_str()).is_none() => id.as_str().to_owned(),
                    _ => String::new(),
                };
                let index = match i.content {
                    E::IntConst(n) => Some(n as u32),
                    E::UIntConst(n) => Some(n),
                    _ => None,
                };
                match (array.as_str(), index) {
                    ("gl_FragData", Some(n)) if self.stage == Stage::Fragment => {
                        *e = self.frag_data(n)
                    }
                    ("gl_TexCoord", Some(n)) => {
                        let name = format!("iris_TexCoord{n}");
                        self.legacy_varying(&name, "vec4");
                        *e = var(&name);
                    }
                    ("gl_FragData" | "gl_TexCoord", None) => self.errors.push(format!(
                        "{array} indexed by a non-constant: {}",
                        expr_text(e)
                    )),
                    _ => {
                        self.expr(a);
                        self.expr(i);
                    }
                }
            }
            E::FunCall(_, args) => {
                args.iter_mut().for_each(|a| self.expr(a));
                self.call(e);
            }
            E::Dot(a, _) => {
                self.expr(a);
                if self.shadow_lookup(a) {
                    let inner = std::mem::replace(&mut **a, var("_"));
                    **a = call("vec4", vec![inner]);
                }
            }
        }
    }

    /// Whether `e` is a `texture*` call on a depth-comparing sampler (a float).
    fn shadow_lookup(&self, e: &ast::Expr) -> bool {
        let ast::ExprData::FunCall(f, args) = &e.content else {
            return false;
        };
        let Some(name) = callee(f) else { return false };
        const LOOKUPS: &[&str] = &[
            "texture",
            "textureLod",
            "textureOffset",
            "textureLodOffset",
            "textureProj",
            "textureProjLod",
            "textureProjOffset",
            "textureGrad",
            "textureGradOffset",
        ];
        if !LOOKUPS.contains(&name.as_str()) || self.functions.contains(&name) {
            return false;
        }
        let Some(first) = args.first() else {
            return false;
        };
        match &first.content {
            ast::ExprData::FunCall(f, _) => callee(f).and_then(|c| opaque(&c)).is_some_and(|o| o.3),
            _ => false,
        }
    }

    fn call(&mut self, e: &mut ast::Expr) {
        let ast::ExprData::FunCall(f, args) = &mut e.content else {
            return;
        };
        let Some(name) = callee(f) else { return };
        if name == "ftransform" && args.is_empty() && !self.functions.contains(&name) {
            let mvp = self.variable("gl_ModelViewProjectionMatrix").unwrap();
            let vertex = self
                .variable("gl_Vertex")
                .unwrap_or_else(|| var("gl_Vertex"));
            *e = ast::ExprData::Binary(
                ast::BinaryOpData::Mult.into_node(),
                Box::new(mvp),
                Box::new(vertex),
            )
            .into_node();
            return;
        }
        if let Some(params) = self.sampler_params.get(&name) {
            // sampler2D(a_t, a_s) passed to a split parameter: a_t, a_s.
            let mut spliced = Vec::with_capacity(args.len() + params.len());
            for (i, a) in std::mem::take(args).into_iter().enumerate() {
                match &a.content {
                    ast::ExprData::FunCall(g, inner)
                        if params.contains(&i)
                            && inner.len() == 2
                            && callee(g).is_some_and(|c| opaque(&c).is_some()) =>
                    {
                        spliced.extend(inner.iter().cloned())
                    }
                    _ => spliced.push(a),
                }
            }
            *args = spliced;
            return;
        }
        if self.functions.contains(&name) {
            return;
        }
        // `mat2(vec4)` (Bloop): naga refuses 1-argument `mat2` from a `vec4`.
        if name == "mat2"
            && !self.constant
            && args.len() == 1
            && !matches!(
                args[0].content,
                ast::ExprData::FloatConst(_)
                    | ast::ExprData::IntConst(_)
                    | ast::ExprData::UIntConst(_)
                    | ast::ExprData::DoubleConst(_)
            )
        {
            *f = ast::FunIdentifierData::Expr(Box::new(var("iris_mat2"))).into_node();
            self.mat2_ctor = true;
            return;
        }
        // `sqrt(-1.0)`, Bloop's NaN: naga folds it and fails, drivers give NaN.
        if name == "sqrt" && !self.constant && args.len() == 1 && negative_constant(&args[0]) {
            *f = ast::FunIdentifierData::Expr(Box::new(var("iris_sqrt"))).into_node();
            self.nan = true;
            return;
        }
        if name == "textureGather"
            && (2..=3).contains(&args.len())
            && let ast::ExprData::FunCall(g, inner) = &args[0].content
            && inner.len() == 2
            && callee(g).as_deref() == Some("sampler2D")
        {
            // naga's GLSL front end has no `textureGather`.
            let mut call_args = inner.clone();
            call_args.push(args[1].clone());
            call_args.push(
                args.get(2)
                    .cloned()
                    .unwrap_or_else(|| ast::ExprData::IntConst(0).into_node()),
            );
            *e = call("iris_gather", call_args);
            self.gather = true;
            return;
        }
        if name == "modf"
            && args.len() == 2
            && !matches!(args[1].content, ast::ExprData::Variable(_))
        {
            // `modf(x, v.y)` (Photon): WGSL can't point at a vector
            // component, so no `out` parameter: `+(v.y = trunc(x), x - trunc(x))`
            // (the unary `+` has the comma printed in brackets, also in an
            // initialiser).
            let (x, whole) = (args[0].clone(), args[1].clone());
            let assign = ast::ExprData::Assignment(
                Box::new(whole),
                ast::AssignmentOpData::Equal.into_node(),
                Box::new(call("trunc", vec![x.clone()])),
            )
            .into_node();
            let fraction = ast::ExprData::Binary(
                ast::BinaryOpData::Sub.into_node(),
                Box::new(x.clone()),
                Box::new(call("trunc", vec![x])),
            )
            .into_node();
            let comma = ast::ExprData::Comma(Box::new(assign), Box::new(fraction)).into_node();
            *e = ast::ExprData::Unary(ast::UnaryOpData::Add.into_node(), Box::new(comma))
                .into_node();
            return;
        }
        if let Some((_, new)) = LEGACY_CALLS.iter().find(|(old, _)| *old == name) {
            *f = ast::FunIdentifierData::Expr(Box::new(var(new))).into_node();
        } else if let Some((_, new)) = SHADOW_CALLS.iter().find(|(old, _)| *old == name) {
            let args = std::mem::take(args);
            *e = call("vec4", vec![call(new, args)]);
        }
        if self.stage != Stage::Fragment {
            explicit_lod(e);
        }
        bool_to_number(e);
    }

    /// The legalised source: header, the rewritten program, then a `main`
    /// that runs the prologue, the program's own `main` and the epilogue.
    fn finish(&mut self, unit: &ast::TranslationUnit) -> Result<String, String> {
        for &(name, ty, attribute) in IRIS_IMPLICIT {
            if !self.used.contains(name) || self.declared.contains(name) {
                continue;
            }
            if attribute && self.stage == Stage::Vertex {
                let at = Link::location(&mut self.link.attributes, name);
                self.header
                    .push(format!("layout(location = {at}) in {ty} {name};"));
                self.interface.attributes.insert(name.to_owned());
            } else if !attribute {
                self.link.uniform(name, format!("{ty} {name}"))?;
                self.interface.uniforms.insert(name.to_owned());
            }
        }
        if !self.errors.is_empty() {
            return Err(self.errors.join("\n"));
        }
        if self.stage == Stage::Vertex {
            // GL's clip space to wgpu's: y flipped, so texture row 0 stays
            // the bottom of the picture as in GL (texture coordinates and
            // `gl_FragCoord` then agree with the pack's), and z from
            // [-w, w] to [0, w].
            self.epilogue.push("gl_Position.y = -gl_Position.y;".into());
            self.epilogue
                .push("gl_Position.z = (gl_Position.z + gl_Position.w) * 0.5;".into());
        }
        let mut out = String::from("#version 450 core\n");
        out.push_str(HELPERS);
        if self.modulo {
            out.push_str(&modulo_helpers());
        }
        if self.gather {
            out.push_str(GATHER);
        }
        if self.nan {
            out.push_str("float iris_sqrt(float x) { return sqrt(x); }\n");
        }
        if self.mat2_ctor {
            out.push_str(MAT2_HELPERS);
        }
        if !self.link.uniforms.is_empty() {
            out.push_str("layout(std140, set = 0, binding = 0) uniform IrisUniforms {\n");
            for (_, decl) in &self.link.uniforms {
                out.push_str(&format!("    {decl};\n"));
            }
            out.push_str("};\n");
        }
        for h in &self.header {
            out.push_str(h);
            out.push('\n');
        }
        let mut state = FormattingState::default();
        for ed in &unit.0 {
            show_external_declaration(&mut out, ed, &mut state).map_err(|e| e.to_string())?;
        }
        out.push_str("\nvoid main() {\n");
        for s in self
            .prologue
            .iter()
            .chain(["iris_main();".to_owned()].iter())
            .chain(&self.epilogue)
        {
            out.push_str("    ");
            out.push_str(s);
            out.push('\n');
        }
        out.push_str("}\n");
        Ok(out)
    }
}
