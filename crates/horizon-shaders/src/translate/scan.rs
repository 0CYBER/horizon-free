//! The first pass over a stage's tokens: what it declares at global scope, which of its
//! functions take samplers, which outputs and built-ins it uses.

use std::collections::{BTreeSet, HashMap};
use std::fmt::Write;

use naga::ShaderStage;

use super::Error;
use super::rewrite::{attribute_value, is_zero_literal};
use crate::Preprocessed;
use crate::macros::lex;

/// A varying: its name and its declaration with `@` where the name goes (`flat ivec2 @[2];`).
pub(super) struct Var {
    pub(super) name: String,
    pub(super) decl: String,
    /// Of a struct type: the pack's declaration stays as a private global, and the members
    /// travel in varyings of ours (`struct_parts`).
    pub(super) structure: bool,
}

/// `layout(rgba16f) uniform writeonly image2D colorimg4;`
#[derive(Clone)]
pub(super) struct ImageDecl {
    pub(super) name: String,
    pub(super) ty: String,
    pub(super) format: String,
    /// `readonly`, `writeonly` or empty (both).
    pub(super) access: String,
    /// Declared `coherent` or `volatile`: invocations of a dispatch share it as it changes.
    pub(super) coherent: bool,
}

pub(super) struct Stage<'p> {
    pub(super) kind: ShaderStage,
    pub(super) pp: &'p Preprocessed,
    pub(super) tokens: Vec<&'p str>,
    /// Token ranges to blank out (declarations moved into the header).
    pub(super) removed: Vec<std::ops::Range<usize>>,
    /// Name, type with its array suffix, token of the type.
    pub(super) uniforms: Vec<(String, String, usize)>,
    pub(super) samplers: Vec<(String, String)>,
    /// Uniform names with constant initializers retained as compiler constants.
    pub(super) defaults: Vec<(String, usize)>,
    pub(super) images: Vec<ImageDecl>,
    /// The bindings of the SSBOs (`buffer` blocks), in set 4.
    pub(super) buffers: BTreeSet<u32>,
    pub(super) ins: Vec<Var>,
    pub(super) outs: Vec<Var>,
    /// Vertex inputs, now private globals, and the value our `main` gives each.
    /// Vertex inputs: name, our composite value, type.
    pub(super) attributes: Vec<(String, String, String)>,
    /// A gbuffers-style program: the matrices are the frame's, not the quad's.
    pub(super) gbuffers: bool,
    /// `normals` and `specular` are paged maps (`GbuffersInputs::paged_maps`).
    pub(super) paged_maps: bool,
    /// The pack's fragment outputs, declared again with their locations.
    pub(super) frag_outs: Vec<(u32, String)>,
    /// Narrow outputs copied into their vec4 after the pack's `main`.
    pub(super) out_copies: Vec<String>,
    /// Locations whose `hz_outN` is ours, filled from the pack's outputs: its vector type and
    /// the components written so far (a bit each).
    pub(super) wide_outs: HashMap<u32, (&'static str, u8)>,
    /// The stage calls `inverse`, which ours replaces, and doesn't define its own.
    pub(super) inverse: bool,
    /// The stage calls `textureGatherOffsets`, which naga lacks: ours is in the prelude.
    pub(super) gather_offsets: bool,
    /// The stage calls `textureQueryLod`, which naga lacks: ours is in the prelude.
    pub(super) query_lod: bool,
    /// The stage calls `dot` / `normalize` and defines neither: the scalar overloads GLSL has
    /// (`dot(float, float)`, `normalize(float)`) and naga lacks go in the prelude.
    pub(super) scalar_dot: bool,
    pub(super) scalar_normalize: bool,
    /// `min3`, `max3` and `mid3` the stage calls with GL_AMD_shader_trinary_minmax enabled and
    /// doesn't define itself: naga lacks them, so ours are in the prelude.
    pub(super) trinary: Vec<&'p str>,
    /// The pack's functions that take samplers: which parameters are samplers.
    pub(super) sampler_functions: HashMap<String, Vec<bool>>,
    /// The pack's structs: each member's type and name with its array suffix.
    pub(super) structs: HashMap<String, Vec<(String, String)>>,
    /// `out struct S { .. } s;`: the qualifiers and `S`, read with the declarators after `}`.
    struct_qualifiers: Vec<usize>,
    pub(super) outputs: BTreeSet<u32>,
    pub(super) frag_coord: bool,
    /// Tokens written differently, by index.
    pub(super) replaced: HashMap<usize, String>,
    /// Globals whose initializer runs in our `main` (`g = hz_init_g();`), in order.
    pub(super) inits: Vec<String>,
    /// Globals that keep their literal initializer (`initializer`): no constants, so a `const`
    /// that names one runs in our `main` too.
    pub(super) literal_globals: Vec<String>,
    /// A function was declared before its body (`float f();`): its body may read or write
    /// globals declared after an initializer that calls it.
    prototype: bool,
    /// An initializer run in our `main` called the pack's functions after a prototype: the
    /// globals of literals after it run in our `main` too, in file order (`initializer`).
    in_order: bool,
    /// Authored global names; automatic core matrix inputs must not hide them.
    pub(super) globals: BTreeSet<String>,
    /// Their captured array size constants and declarator ends: `.length()` stays type-only.
    pub(super) runtime_array_lengths: HashMap<String, (String, usize)>,
    pub(super) errors: Vec<Error>,
}

/// The format qualifiers of images (GLSL 4.60, 4.4.6.2).
const IMAGE_FORMATS: [&str; 39] = [
    "rgba32f",
    "rgba16f",
    "rg32f",
    "rg16f",
    "r11f_g11f_b10f",
    "r32f",
    "r16f",
    "rgba16",
    "rgb10_a2",
    "rgba8",
    "rg16",
    "rg8",
    "r16",
    "r8",
    "rgba16_snorm",
    "rgba8_snorm",
    "rg16_snorm",
    "rg8_snorm",
    "r16_snorm",
    "r8_snorm",
    "rgba32i",
    "rgba16i",
    "rgba8i",
    "rg32i",
    "rg16i",
    "rg8i",
    "r32i",
    "r16i",
    "r8i",
    "rgba32ui",
    "rgba16ui",
    "rgb10_a2ui",
    "rgba8ui",
    "rg32ui",
    "rg16ui",
    "rg8ui",
    "r32ui",
    "r16ui",
    "r8ui",
];

pub(super) const STORAGE: [&str; 5] = ["uniform", "in", "out", "attribute", "varying"];
pub(super) const QUALIFIERS: [&str; 11] = [
    "flat",
    "smooth",
    "noperspective",
    "centroid",
    "invariant",
    "highp",
    "mediump",
    "lowp",
    "precise",
    "sample",
    "const",
];
/// The compatibility profile's uniform state (with its `Inverse`/`Transpose` forms).
pub(super) const UNIFORM_STATE: [&str; 5] = [
    "gl_ModelViewMatrix",
    "gl_ProjectionMatrix",
    "gl_ModelViewProjectionMatrix",
    "gl_NormalMatrix",
    "gl_TextureMatrix",
];
/// The compatibility profile's vertex inputs.
pub(super) const VERTEX_INPUTS: [&str; 6] = [
    "gl_Vertex",
    "gl_Color",
    "gl_SecondaryColor",
    "gl_Normal",
    "gl_FogCoord",
    "gl_MultiTexCoord",
];
/// Built-in functions naga's constant evaluator lacks: a `const` that calls one is evaluated
/// where the program runs instead (GL drivers fold them; naga fails the program).
pub(super) const NOT_CONSTANT: [&str; 25] = [
    "mix",
    "smoothstep",
    "reflect",
    "refract",
    "faceforward",
    "transpose",
    "determinant",
    "inverse",
    "outerProduct",
    "matrixCompMult",
    "modf",
    "frexp",
    "ldexp",
    "packUnorm2x16",
    "packSnorm2x16",
    "packUnorm4x8",
    "packSnorm4x8",
    "packHalf2x16",
    "unpackUnorm2x16",
    "unpackSnorm2x16",
    "unpackUnorm4x8",
    "unpackSnorm4x8",
    "unpackHalf2x16",
    "bitfieldExtract",
    "bitfieldInsert",
];
/// Words that became keywords after GLSL 1.x and that old packs use as names.
pub(super) const LATER_KEYWORDS: [&str; 17] = [
    "sample",
    "buffer",
    "shared",
    "filter",
    "input",
    "output",
    "common",
    "partition",
    "active",
    "resource",
    "patch",
    "subroutine",
    "coherent",
    "volatile",
    "restrict",
    "readonly",
    "writeonly",
];

/// Vulkan GLSL's texture and sampler types, which naga's frontend reads as types, renamed.
const VULKAN_TYPES: [(&str, &str); 9] = [
    ("sampler", "hz_kw_sampler"),
    ("samplerShadow", "hz_kw_samplerShadow"),
    ("texture1D", "hz_kw_texture1D"),
    ("texture2D", "hz_kw_texture2D"),
    ("texture3D", "hz_kw_texture3D"),
    ("textureCube", "hz_kw_textureCube"),
    ("texture2DArray", "hz_kw_texture2DArray"),
    ("texture2DMS", "hz_kw_texture2DMS"),
    ("textureCubeArray", "hz_kw_textureCubeArray"),
];

pub(super) fn significant(t: &str) -> bool {
    !t.trim().is_empty()
}

/// A fragment output narrower than four components: the vec4 of its kind, and the components
/// that fill it (alpha 1).
fn widened(ty: &str) -> Option<(&'static str, &'static str)> {
    Some(match ty {
        "float" => ("vec4", ", 0.0, 0.0, 1.0"),
        "vec2" => ("vec4", ", 0.0, 1.0"),
        "vec3" => ("vec4", ", 1.0"),
        "int" => ("ivec4", ", 0, 0, 1"),
        "ivec2" => ("ivec4", ", 0, 1"),
        "ivec3" => ("ivec4", ", 1"),
        "uint" => ("uvec4", ", 0u, 0u, 1u"),
        "uvec2" => ("uvec4", ", 0u, 1u"),
        "uvec3" => ("uvec4", ", 1u"),
        _ => return None,
    })
}

/// A scalar or vector fragment output's vec4 of its kind, its width, and the value the
/// components no output writes start from (alpha 1, as `widened` fills them).
fn output_kind(ty: &str) -> Option<(&'static str, usize, &'static str)> {
    let (wide, fill) = match ty.trim_end_matches(['2', '3', '4']) {
        "float" | "vec" => ("vec4", "vec4(0.0, 0.0, 0.0, 1.0)"),
        "int" | "ivec" => ("ivec4", "ivec4(0, 0, 0, 1)"),
        "uint" | "uvec" => ("uvec4", "uvec4(0u, 0u, 0u, 1u)"),
        _ => return None,
    };
    let width = match ty.as_bytes().last() {
        Some(&n @ b'2'..=b'4') if ty.contains("vec") => (n - b'0') as usize,
        _ if ty.contains("vec") => return None,
        _ => 1,
    };
    Some((wide, width, fill))
}

/// `sampler2D`, `isampler3D`, `usampler2D`, `sampler2DShadow`...
pub(super) fn is_sampler_type(t: &str) -> bool {
    t.trim_start_matches(['i', 'u']).starts_with("sampler")
}

pub(super) fn is_ident(t: &str) -> bool {
    t.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
}

/// Whether a global of type `ty`, not an array, takes its initializer through a constructor of
/// the type, which converts it (`vec2 v = ivec2(1)`): scalars, vectors and matrices. An array or
/// a struct must match.
fn converts(ty: &str) -> bool {
    matches!(ty, "float" | "int" | "uint" | "bool") || ty.contains("vec") || ty.starts_with("mat")
}

/// The scalar and vector types a literal initializer may construct (`literal_value`).
const LITERAL_TYPES: [&str; 16] = [
    "float", "int", "uint", "bool", "vec2", "vec3", "vec4", "ivec2", "ivec3", "ivec4", "uvec2",
    "uvec3", "uvec4", "bvec2", "bvec3", "bvec4",
];

/// Whether `value`, the significant tokens of a global's initializer, is numbers (a sign before
/// one), `true` and `false` in constructors of scalars, vectors and arrays of one dimension
/// (`vec2[2](vec2(-0.5, 1.0), vec2(2.0))`): an expression naga's frontend folds into literals
/// and keeps as the global's own initializer.
fn literal_value(value: &[&str]) -> bool {
    let number = |t: &str| {
        let mut chars = t.chars();
        match chars.next() {
            Some('.') => chars.next().is_some_and(|c| c.is_ascii_digit()),
            first => first.is_some_and(|c| c.is_ascii_digit()),
        }
    };
    let mut arrays = 0;
    !value.is_empty()
        && value.iter().enumerate().all(|(k, &t)| match t {
            "(" | ")" | "," | "]" | "true" | "false" => true,
            "[" => {
                arrays += 1;
                arrays == 1
            }
            "-" | "+" => {
                (k == 0 || matches!(value[k - 1], "(" | ","))
                    && value.get(k + 1).is_some_and(|&n| number(n))
            }
            _ => number(t) || LITERAL_TYPES.contains(&t),
        })
}

fn outer_array_length(ty: &str) -> Option<&str> {
    let open = ty.find('[')?;
    let mut depth = 0;
    for (offset, c) in ty[open..].char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&ty[open + 1..open + offset]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Synthesized return types need the same dimension reversal as source type tokens.
fn reversed_array_dimensions(ty: &str) -> String {
    let Some(open) = ty.find('[') else {
        return ty.to_owned();
    };
    let mut tail = &ty[open..];
    let mut dimensions = Vec::new();
    while !tail.is_empty() {
        let Some(length) = outer_array_length(tail).filter(|_| tail.starts_with('[')) else {
            return ty.to_owned();
        };
        let end = length.len() + 2;
        dimensions.push(&tail[..end]);
        tail = &tail[end..];
    }
    let mut result = ty[..open].to_owned();
    for dimension in dimensions.into_iter().rev() {
        result.push_str(dimension);
    }
    result
}

impl<'p> Stage<'p> {
    pub(super) fn scan(pp: &'p Preprocessed, kind: ShaderStage) -> Self {
        let mut s = Stage {
            kind,
            pp,
            tokens: lex(&pp.code),
            removed: Vec::new(),
            uniforms: Vec::new(),
            defaults: Vec::new(),
            samplers: Vec::new(),
            images: Vec::new(),
            buffers: BTreeSet::new(),
            ins: Vec::new(),
            outs: Vec::new(),
            attributes: Vec::new(),
            frag_outs: Vec::new(),
            out_copies: Vec::new(),
            wide_outs: HashMap::new(),
            inverse: false,
            gather_offsets: false,
            query_lod: false,
            scalar_dot: false,
            scalar_normalize: false,
            trinary: Vec::new(),
            sampler_functions: HashMap::new(),
            structs: HashMap::new(),
            struct_qualifiers: Vec::new(),
            outputs: BTreeSet::new(),
            frag_coord: false,
            gbuffers: false,
            paged_maps: false,
            replaced: HashMap::new(),
            inits: Vec::new(),
            literal_globals: Vec::new(),
            prototype: false,
            in_order: false,
            globals: BTreeSet::new(),
            runtime_array_lengths: HashMap::new(),
            errors: Vec::new(),
        };
        s.rename_vulkan_types();
        let (mut start, mut depth, mut parens) = (0, 0usize, 0usize);
        let mut own_inverse = false;
        let (mut own_dot, mut own_normalize) = (false, false);
        let mut own_trinary = Vec::new();
        let trinary_enabled = pp.extensions.iter().any(|e| {
            e.split_once(':').is_some_and(|(name, behavior)| {
                name.trim() == "GL_AMD_shader_trinary_minmax" && behavior.trim() != "disable"
            })
        });
        for i in 0..s.tokens.len() {
            match s.tokens[i] {
                "(" => parens += 1,
                ")" => parens = parens.saturating_sub(1),
                "{" if depth == 0 && parens == 0 => {
                    s.global(start, i, true);
                    s.structure(start, i);
                    depth = 1;
                }
                "{" => depth += 1,
                "}" if depth > 0 => {
                    depth -= 1;
                    if depth == 0 {
                        start = i + 1;
                    }
                }
                ";" if depth == 0 && parens == 0 => {
                    s.global(start, i, false);
                    start = i + 1;
                }
                "gl_FragData" | "gl_FragColor" if kind == ShaderStage::Fragment => {
                    let n = if s.tokens[i] == "gl_FragColor" {
                        Some(0)
                    } else {
                        s.index_after(i).map(|(n, _)| n)
                    };
                    match n {
                        Some(n) => _ = s.outputs.insert(n),
                        None => s
                            .errors
                            .push(s.error(i, "`gl_FragData` needs a constant index")),
                    }
                }
                "gl_FragCoord" => s.frag_coord = true,
                "textureGatherOffsets" => s.gather_offsets = true,
                "textureQueryLod" => s.query_lod = true,
                "dot" | "normalize" => {
                    // A definition at the top level: `float dot(float a, ...)`.
                    let defines = depth == 0
                        && s.tokens[..i]
                            .iter()
                            .rfind(|t| significant(t))
                            .is_some_and(|t| is_ident(t));
                    if s.tokens[i] == "dot" {
                        s.scalar_dot = true;
                        own_dot |= defines;
                    } else {
                        s.scalar_normalize = true;
                        own_normalize |= defines;
                    }
                }
                name @ ("min3" | "max3" | "mid3") if trinary_enabled => {
                    if !s.trinary.contains(&name) {
                        s.trinary.push(name);
                    }
                    // A definition at the top level: `float max3(float a, ...)`.
                    if depth == 0
                        && s.tokens[..i]
                            .iter()
                            .rfind(|t| significant(t))
                            .is_some_and(|t| is_ident(t))
                    {
                        own_trinary.push(name);
                    }
                }
                "inverse" => {
                    s.inverse = true;
                    // `mat4 inverse(mat4 m)`: a pack for GLSL 1.20, which has none, wrote its own.
                    own_inverse |= depth == 0
                        && s.tokens[..i]
                            .iter()
                            .rfind(|t| significant(t))
                            .is_some_and(|t| is_ident(t));
                }
                "gl_TexCoord" => {
                    if let Some((n, _)) = s.index_after(i) {
                        s.builtin_varying(format!("hz_TexCoord{n}"), "vec4");
                    }
                }
                "gl_FrontColor" => s.builtin_varying("hz_FrontColor".into(), "vec4"),
                "gl_Color" if kind == ShaderStage::Fragment => {
                    s.builtin_varying("hz_FrontColor".into(), "vec4")
                }
                "gl_FogFragCoord" => s.builtin_varying("hz_FogFragCoord".into(), "float"),
                _ => {}
            }
        }
        s.inverse &= !own_inverse;
        s.scalar_dot &= !own_dot;
        s.scalar_normalize &= !own_normalize;
        s.trinary.retain(|name| !own_trinary.contains(name));
        if s.gather_offsets {
            // Its sampler is passed as a texture and a sampler, as to the pack's own functions.
            s.sampler_functions
                .entry("textureGatherOffsets".into())
                .or_insert_with(|| vec![true, false, false, false, false]);
        }
        if s.query_lod {
            s.sampler_functions
                .entry("textureQueryLod".into())
                .or_insert_with(|| vec![true, false]);
        }
        s
    }

    /// Names that desktop GLSL leaves free but Vulkan GLSL takes for its separate texture and
    /// sampler types (`sampler2D sampler`): renamed, unless called (`texture2D(...)`).
    fn rename_vulkan_types(&mut self) {
        for i in 0..self.tokens.len() {
            let Some(&(_, renamed)) = VULKAN_TYPES.iter().find(|(t, _)| *t == self.tokens[i])
            else {
                continue;
            };
            let called = self.tokens[i + 1..]
                .iter()
                .find(|t| significant(t))
                .is_some_and(|&t| t == "(");
            if !called {
                self.tokens[i] = renamed;
            }
        }
    }

    /// A built-in varying (`gl_TexCoord[n]`, `gl_FrontColor`, `gl_FogFragCoord`) of type `ty`: an
    /// output of a vertex stage, an input of a fragment stage.
    pub(super) fn builtin_varying(&mut self, name: String, ty: &str) {
        let list = if self.kind == ShaderStage::Vertex {
            &mut self.outs
        } else {
            &mut self.ins
        };
        if !list.iter().any(|v| v.name == name) {
            list.push(Var {
                name,
                decl: format!("{ty} @;"),
                structure: false,
            });
        }
    }

    /// `[ n ]` right after token `i`: the number and the index of the `]`.
    pub(super) fn index_after(&self, i: usize) -> Option<(u32, usize)> {
        let mut it = (i + 1..self.tokens.len()).filter(|&j| significant(self.tokens[j]));
        let open = it.next()?;
        let n = it.next()?;
        let close = it.next()?;
        (self.tokens[open] == "[" && self.tokens[close] == "]")
            .then(|| self.tokens[n].parse().ok().map(|n| (n, close)))
            .flatten()
    }

    pub(super) fn error(&self, token: usize, message: &str) -> Error {
        let line: usize = self.tokens[..token.min(self.tokens.len())]
            .iter()
            .map(|t| t.matches('\n').count())
            .sum();
        let o = self.pp.lines.get(line).copied().unwrap_or_default();
        Error {
            file: self
                .pp
                .files
                .get(o.file as usize)
                .cloned()
                .unwrap_or_default(),
            line: o.line,
            message: message.to_owned(),
        }
    }

    /// Core names share the compatibility state for geometry, including shadow transforms.
    /// Authored non-uniform globals (or uniforms of another type) keep their own meaning.
    /// `normalMatrix` is the core profile's only (ShaderDoc's `gl_NormalMatrix`, as #9136 quotes
    /// it): in a compatibility-profile stage it stays a uniform of the frame, read as zero.
    /// `textureMatrix` is `gl_TextureMatrix[0]` in the vertex stage only: our probes read the
    /// identity in Iris 1.11.7's terrain and shadow vertex stages, and zero in the terrain
    /// fragment stage (#9080).
    pub(super) fn core_matrix(&self, name: &str) -> Option<&'static str> {
        let (matrix, ty) = match name {
            "projectionMatrix" => ("hz_ProjectionMatrix", "mat4"),
            "modelViewMatrix" => ("hz_ModelViewMatrix", "mat4"),
            "normalMatrix" if self.core_profile() => ("hz_NormalMatrix", "mat3"),
            "textureMatrix" if self.kind == ShaderStage::Vertex => ("hz_TextureMatrix[0]", "mat4"),
            _ => return None,
        };
        if !self.gbuffers
            || (self.globals.contains(name)
                && !self.uniforms.iter().any(|(n, t, _)| n == name && t == ty))
        {
            return None;
        }
        Some(matrix)
    }

    /// The stage is in the core profile: a `#version` of 150 or later that doesn't name the
    /// compatibility profile. A fit of our #9136 probe's Iris 1.11.7 pictures (`normalMatrix` set
    /// at `330 core`, `330` and `150`, zero at `330 compatibility`), not Iris's rule. Below 150 is
    /// unmeasured (Iris drew no geometry for the `140` probe) and stays as before.
    fn core_profile(&self) -> bool {
        matches!(&self.pp.version, Some((version, profile))
            if *version >= 150 && profile.as_deref() != Some("compatibility"))
    }

    /// Remember names without interpreting their values. Commas inside constructors and array
    /// suffixes do not introduce another declarator, nor do references in an initializer.
    fn global_names(&mut self, words: &[usize], mut type_at: usize) {
        while words
            .get(type_at)
            .is_some_and(|&i| self.tokens[i] == "const" || QUALIFIERS.contains(&self.tokens[i]))
        {
            type_at += 1;
        }
        let (mut depth, mut name) = (0i32, true);
        for &i in words.iter().skip(type_at + 1) {
            let token = self.tokens[i];
            if depth == 0 && name && is_ident(token) {
                self.globals.insert(token.to_owned());
                name = false;
            }
            match token {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" => depth -= 1,
                "," if depth == 0 => name = true,
                _ => {}
            }
        }
    }

    /// A statement at global scope, tokens `from..to` (without its `;`, or before a `{`).
    pub(super) fn global(&mut self, from: usize, to: usize, body: bool) {
        let mut words: Vec<usize> = (from..to)
            .filter(|&i| significant(self.tokens[i]))
            .collect();
        // A struct defined in an interface declaration (GLSL 4.60, 4.1.8): `flat out struct S
        // { .. } s;` declares `s` as `flat out S s;` would, after the definition.
        let pending = std::mem::take(&mut self.struct_qualifiers);
        if body {
            if let Some(s) = words.iter().position(|&i| self.tokens[i] == "struct")
                && let Some(&name) = words.get(s + 1)
                && words[..s]
                    .iter()
                    .any(|&i| matches!(self.tokens[i], "in" | "out" | "varying"))
            {
                self.struct_qualifiers = words[..s].to_vec();
                self.struct_qualifiers.push(name);
                return;
            }
        } else {
            words.splice(0..0, pending);
        }
        // A prototype: a `(` before any `=`, and no storage qualifier.
        let paren = words
            .iter()
            .position(|&i| matches!(self.tokens[i], "(" | "="));
        let prototype = paren.is_some_and(|p| self.tokens[words[p]] == "(")
            && !words.iter().any(|&i| STORAGE.contains(&self.tokens[i]));
        if body && words.iter().any(|&i| self.tokens[i] == "buffer") {
            return self.buffer_block(&words);
        }
        if body || prototype {
            self.prototype |= prototype && !body;
            return self.function(&words);
        }
        let mut k = 0;
        let mut storage = None;
        let mut location = None;
        let mut component = None;
        let mut format = None;
        let mut access = "";
        let mut coherent = false;
        let mut interpolation = String::new();
        while let Some(&i) = words.get(k) {
            let w = self.tokens[i];
            if w == "layout" {
                // layout(location = n, ...), an image's layout(rgba16f)
                let mut j = k + 1;
                while let Some(&t) = words.get(j) {
                    j += 1;
                    match self.tokens[t] {
                        "location" => {
                            location = words
                                .get(j + 1)
                                .and_then(|&n| self.tokens[n].parse::<u32>().ok());
                        }
                        "component" => {
                            component = words
                                .get(j + 1)
                                .and_then(|&n| self.tokens[n].parse::<usize>().ok());
                        }
                        f if IMAGE_FORMATS.contains(&f) => format = Some(f),
                        ")" => break,
                        _ => {}
                    }
                }
                k = j;
                continue;
            }
            if STORAGE.contains(&w) {
                storage = Some(w);
            } else if matches!(w, "readonly" | "writeonly") {
                access = w;
            } else if QUALIFIERS.contains(&w) || matches!(w, "coherent" | "volatile" | "restrict") {
                coherent |= matches!(w, "coherent" | "volatile");
                if matches!(
                    w,
                    "flat" | "smooth" | "noperspective" | "centroid" | "sample"
                ) {
                    write!(interpolation, "{w} ").unwrap();
                }
            } else {
                break;
            }
            k += 1;
        }
        // `centroid` and `sample` choose where a varying is interpolated; a `flat` one is not
        // interpolated, so GLSL (4.60, 4.5) lets them stand beside it and means nothing by
        // them. naga rejects the pair: `flat` alone says all there is.
        if interpolation.split_whitespace().any(|w| w == "flat") {
            interpolation = "flat ".to_owned();
        }
        self.global_names(&words, k);
        let Some(storage) = storage else {
            return self.initializer(&words, to);
        };
        let Some(&ty_at) = words.get(k) else { return };
        let ty = self.tokens[ty_at].to_owned();
        if words[k..].iter().any(|&i| self.tokens[i] == "{") {
            return; // An interface block: left as the pack wrote it.
        }
        // Declarators: name [array] [= init], separated by commas.
        let mut declarators: Vec<(String, String)> = Vec::new();
        let mut defaults = String::new();
        let mut j = k + 1;
        while let Some(&n) = words.get(j) {
            let name = self.tokens[n].to_owned();
            let mut array = String::new();
            j += 1;
            let mut nesting = 0;
            while let Some(&t) = words.get(j) {
                match self.tokens[t] {
                    "," if nesting == 0 => break,
                    "(" | "[" => nesting += 1,
                    ")" | "]" => nesting -= 1,
                    _ => {}
                }
                if self.tokens[t] == "=" && nesting == 0 {
                    let start = j + 1;
                    j += 1;
                    let mut depth = 0;
                    while let Some(&token) = words.get(j) {
                        match self.tokens[token] {
                            "," if depth == 0 => break,
                            "(" | "[" => depth += 1,
                            ")" | "]" => depth -= 1,
                            _ => {}
                        }
                        j += 1;
                    }
                    if storage == "uniform" && !is_sampler_type(&ty) && !ty.contains("image") {
                        let value = words[start..j]
                            .iter()
                            .map(|&i| self.tokens[i])
                            .collect::<Vec<_>>()
                            .join(" ");
                        write!(defaults, "const {ty} hz_default_{name}{array} = {value}; ")
                            .unwrap();
                        self.defaults.push((name.clone(), n));
                    }
                    break;
                }
                array.push_str(self.tokens[t]);
                j += 1;
            }
            j += 1;
            declarators.push((name, array));
        }
        let fragment = self.kind == ShaderStage::Fragment;
        let input = matches!(storage, "in" | "attribute") || (storage == "varying" && fragment);
        self.removed.push(from..to + 1);
        if !defaults.is_empty() {
            self.replaced.insert(words[0], defaults);
        }
        for (name, array) in declarators {
            let decl = format!("{interpolation}{ty} @{array};");
            match storage {
                "uniform" if is_sampler_type(&ty) => self.samplers.push((name, ty.clone())),
                "uniform" if ty.contains("image") => match format {
                    Some(format) => self.images.push(ImageDecl {
                        name,
                        ty: ty.clone(),
                        format: format.to_owned(),
                        access: access.to_owned(),
                        coherent,
                    }),
                    None => self
                        .errors
                        .push(self.error(ty_at, &format!("`{name}`: an image needs a format"))),
                },
                "uniform" => self.uniforms.push((name, format!("{ty}{array}"), ty_at)),
                _ if input && !fragment => {
                    let value = attribute_value(&name, &ty);
                    self.attributes.push((name, value, ty.clone()));
                    self.removed.pop();
                    // A private global of ours now: `in`/`attribute` and qualifiers go.
                    for &w in &words[..k] {
                        self.removed.push(w..w + 1);
                    }
                }
                _ if self.structs.contains_key(&ty) => {
                    self.removed.pop();
                    for &w in &words[..k] {
                        self.removed.push(w..w + 1);
                    }
                    let var = Var {
                        name,
                        decl,
                        structure: true,
                    };
                    if input {
                        self.ins.push(var);
                    } else {
                        self.outs.push(var);
                    }
                }
                _ if input => self.ins.push(Var {
                    name,
                    decl,
                    structure: false,
                }),
                _ if fragment => {
                    let n = location.unwrap_or(self.outputs.len() as u32);
                    let shared = self.wide_outs.contains_key(&n);
                    if component.is_some_and(|c| c > 0) || shared {
                        self.output_part(n, component.unwrap_or(0), &ty, &name, &array, ty_at);
                        continue;
                    }
                    self.outputs.insert(n);
                    // wgpu wants every component of the target's format written; GL fills
                    // what a narrower output leaves out. Ours is a vec4, theirs a private global.
                    match widened(&ty).filter(|_| array.is_empty()) {
                        Some((wide, fill)) => {
                            self.frag_outs.push((
                                n,
                                format!(
                                    "layout(location = {n}) out {wide} hz_out{n}; {ty} {name};"
                                ),
                            ));
                            self.out_copies
                                .push(format!("hz_out{n} = {wide}({name}{fill});"));
                            let width = output_kind(&ty).map_or(4, |(_, width, _)| width);
                            self.wide_outs.insert(n, (wide, (1 << width) - 1));
                        }
                        None => self
                            .frag_outs
                            .push((n, format!("layout(location = {n}) out {ty} {name}{array};"))),
                    }
                }
                _ => self.outs.push(Var {
                    name,
                    decl,
                    structure: false,
                }),
            }
        }
    }

    /// A fragment output with a `component` offset, or one sharing its location with another
    /// (GLSL 4.60, 4.4.2): it fills components `component..` of the location's vector. Ours is
    /// that vector, the pack's a private global copied into its components after `main`.
    fn output_part(
        &mut self,
        n: u32,
        component: usize,
        ty: &str,
        name: &str,
        array: &str,
        ty_at: usize,
    ) {
        let kind = output_kind(ty).filter(|_| array.is_empty());
        let Some((wide, width, fill)) = kind.filter(|&(_, width, _)| component + width <= 4) else {
            let message = format!(
                "`{name}`: a `{ty}{array}` output at component {component} is not supported"
            );
            return self.errors.push(self.error(ty_at, &message));
        };
        let mask = ((1u8 << width) - 1) << component;
        match self.wide_outs.get_mut(&n) {
            Some((other, _)) if *other != wide => {
                let message =
                    format!("`{name}`: a `{ty}` output beside a `{other}` at location {n}");
                return self.errors.push(self.error(ty_at, &message));
            }
            Some((_, written)) if *written & mask != 0 => {
                let message = format!("`{name}`: components of location {n} written twice");
                return self.errors.push(self.error(ty_at, &message));
            }
            Some((_, written)) => {
                *written |= mask;
                self.frag_outs.push((n, format!("{ty} {name};")));
            }
            None => {
                self.wide_outs.insert(n, (wide, mask));
                self.outputs.insert(n);
                self.frag_outs.push((
                    n,
                    format!("layout(location = {n}) out {wide} hz_out{n}; {ty} {name};"),
                ));
                self.out_copies.push(format!("hz_out{n} = {fill};"));
            }
        }
        let swizzle = &"xyzw"[component..component + width];
        self.out_copies
            .push(format!("hz_out{n}.{swizzle} = {name};"));
    }

    /// A global `T name = a ? b : c;` or, without `const`, any initialized global but one of
    /// literals (`literal_value`): naga only evaluates constant initializers at global scope (a
    /// conditional is not one; GL drivers also take uniforms and other globals there), so each
    /// initializer becomes a function of ours on the same line (`T a; T hz_init_a() { return
    /// T(...); } T b; ...`), called by our `main` before the pack's, in file order. The return
    /// converts as GLSL's initialization does (`vec2 v = ivec2(...)`); a global of literals keeps
    /// them in the same constructor (`int n = int(8.0);`), which naga folds into a constant.
    pub(super) fn initializer(&mut self, words: &[usize], to: usize) {
        let text = |i: usize| self.tokens[i];
        let Some(eq) = words.iter().position(|&i| text(i) == "=") else {
            return;
        };
        // Declarators, split at top-level commas: (first word, `=` if any, end).
        let mut declarators = Vec::new();
        let (mut begin, mut nesting, mut assign) = (0, 0i32, None);
        for (n, &i) in words.iter().enumerate() {
            match text(i) {
                "(" | "[" | "{" => nesting += 1,
                ")" | "]" | "}" => nesting -= 1,
                "=" if nesting == 0 && assign.is_none() => assign = Some(n),
                "," if nesting == 0 => {
                    declarators.push((begin, assign.take(), n));
                    begin = n + 1;
                }
                _ => {}
            }
        }
        declarators.push((begin, assign, words.len()));
        let constant = words[..eq].iter().any(|&i| text(i) == "const");
        // The first declarator's name ends the type.
        let first_end = declarators[0].1.unwrap_or(declarators[0].2);
        let mut brackets = 0usize;
        let Some(name_at) = words[..first_end].iter().rposition(|&i| match text(i) {
            "]" => {
                brackets += 1;
                false
            }
            "[" => {
                brackets = brackets.saturating_sub(1);
                false
            }
            t => brackets == 0 && is_ident(t),
        }) else {
            return;
        };
        // The type, with a `vec3[]`'s count written in.
        let mut ty = String::new();
        for &i in &words[..name_at] {
            match text(i) {
                "const" => {}
                "]" if ty.ends_with('[') => match self.array_length(i) {
                    Some(n) => write!(ty, "{n}]").unwrap(),
                    None => return,
                },
                // `T[6]`: the size and its `]` belong to the type.
                t if ty.is_empty() || t == "[" || ty.contains('[') && !ty.ends_with(']') => {
                    ty.push_str(t)
                }
                // Qualifiers before the type (`highp`...): the type is the last word.
                t => ty = t.to_owned(),
            }
        }
        if ty.is_empty() {
            return;
        }
        // The zero-division rewrite reads hz_zero, so even a literal-only expression must run
        // at invocation time. Its value is unspecified in GLSL; keep the existing arithmetic
        // policy rather than inventing a finite replacement. Later dependent constants must
        // run after it, while unrelated constants (including comma declarators) stay constant.
        let mut runtime_names: BTreeSet<&str> = (self.inits.iter())
            .chain(&self.literal_globals)
            .map(String::as_str)
            .collect();
        // A global of literals (`vec2 o[2] = vec2[2](vec2(-0.5, 1.0), vec2(2.0))`) keeps them as
        // its initializer, as a GL driver takes it: nothing of ours stores it again in every
        // invocation, so a table read at a dynamic index (TAA jitter, AO offsets) is the module's
        // constant data. A scalar, vector or matrix, or an array of one dimension (more are
        // written reversed for naga); a global of another type (a struct, a `double`) runs in
        // our `main`, as does a `shared` one, which takes no initializer.
        // After a prototype, an initializer that calls the pack's functions may reach globals
        // declared after it (`float f(); float a = f(); float k = 2.0; float f() { return k; }`):
        // those run in our `main` too, so that `a` still reads `k` before `k` is set.
        let type_brackets = words[..name_at].iter().filter(|&&i| text(i) == "[").count();
        let shared = words[..name_at].iter().any(|&i| text(i) == "shared");
        let mut in_order = self.in_order;
        let mut literal = Vec::new();
        let runtime: Vec<bool> = declarators
            .iter()
            .enumerate()
            .map(|(n, &(begin, assign, end))| {
                let at = if n == 0 { name_at } else { begin };
                let run = assign.is_some_and(|a| {
                    if !constant {
                        let brackets = (words[at..a].iter()).filter(|&&i| text(i) == "[").count();
                        let value: Vec<&str> = words[a + 1..end].iter().map(|&i| text(i)).collect();
                        let shape = match type_brackets + brackets {
                            0 => converts(&ty),
                            1 => true,
                            _ => false,
                        };
                        return in_order || shared || !shape || !literal_value(&value);
                    }
                    (a + 1..end).any(|n| {
                        let t = text(words[n]);
                        let next = words.get(n + 1).map(|&i| text(i));
                        t == "?"
                            || (NOT_CONSTANT.contains(&t) && next == Some("("))
                            || (t == "/" && next.is_some_and(is_zero_literal))
                            || (runtime_names.contains(t)
                                && text(words[n - 1]) != "."
                                && next != Some("(")
                                // A sized array's length depends on its type, not its values.
                                && ![".", "length", "(", ")"].iter().enumerate().all(
                                    |(offset, &t)| {
                                        words.get(n + offset + 1).is_some_and(|&i| text(i) == t)
                                    },
                                ))
                    })
                });
                in_order |= run
                    && self.prototype
                    && assign.is_some_and(|a| {
                        (a + 1..end).any(|k| {
                            self.globals.contains(text(words[k]))
                                && words.get(k + 1).is_some_and(|&i| text(i) == "(")
                        })
                    });
                if run || (!constant && assign.is_some()) {
                    runtime_names.insert(text(words[at]));
                }
                if !run && !constant && assign.is_some() {
                    literal.push(text(words[at]).to_owned());
                }
                run
            })
            .collect();
        self.in_order = in_order;
        // A declarator run in our `main` splits the declaration around its function.
        let split = runtime.contains(&true);
        if !split && literal.is_empty() {
            return;
        }
        let mut replaced = Vec::new();
        let mut names = Vec::new();
        for (n, &(begin, assign, end)) in declarators.iter().enumerate() {
            let name_at = if n == 0 { name_at } else { begin };
            let name = text(words[name_at]).to_owned();
            if !is_ident(&name) {
                return;
            }
            let stop = assign.unwrap_or(end);
            let mut array: String = words[name_at + 1..stop].iter().map(|&i| text(i)).collect();
            if array == "[]" {
                // `T x[] = T[](a, b)`: the count is written in, as the rewrite does for the name.
                match assign.and_then(|a| self.array_length(words[a - 1])) {
                    Some(n) => array = format!("[{n}]"),
                    None => return,
                }
            }
            let next = if n + 1 < declarators.len() {
                let qualifier = if constant && !runtime[n + 1] {
                    "const "
                } else {
                    ""
                };
                format!(" {qualifier}{ty}")
            } else {
                String::new()
            };
            let end_token = words.get(end).copied().unwrap_or(to);
            // Scalars, vectors and matrices convert; arrays and structs must match.
            let scalar = array.is_empty() && !ty.contains('[') && converts(&ty);
            match assign {
                Some(a) if runtime[n] => {
                    let dimensions = if array.is_empty() { &ty } else { &array };
                    let capture = if let Some(length) = outer_array_length(dimensions) {
                        // Bind the size in declaration scope: a use site's local `count` must
                        // not change the length of a global declared with `[count]`.
                        let mut bound = format!("hz_array_length_{name}");
                        while self.tokens.contains(&bound.as_str())
                            || self
                                .runtime_array_lengths
                                .values()
                                .any(|(n, _)| n == &bound)
                        {
                            bound.push('_');
                        }
                        self.runtime_array_lengths
                            .insert(name.clone(), (bound.clone(), end_token));
                        format!(" const int {bound} = int({length});")
                    } else {
                        String::new()
                    };
                    let (open, close) = if scalar {
                        (format!("{ty}("), ")")
                    } else {
                        (String::new(), "")
                    };
                    let return_ty = format!(
                        "{}{}",
                        reversed_array_dimensions(&ty),
                        reversed_array_dimensions(&array),
                    );
                    replaced.push((
                        words[a],
                        format!(";{capture} {return_ty} hz_init_{name}() {{ return {open}"),
                    ));
                    replaced.push((end_token, format!("{close}; }}{next}")));
                    names.push(name);
                }
                // Kept in the constructor the return above would convert them with: naga folds
                // `int(8.0)` into the global's constant initializer, where `8.0` alone mismatches.
                Some(a) if scalar && !constant => {
                    replaced.push((words[a], format!("= {ty}(")));
                    let rest = if split && n + 1 < declarators.len() {
                        format!(";{next}")
                    } else {
                        text(end_token).to_owned()
                    };
                    replaced.push((end_token, format!("){rest}")));
                }
                _ if split && n + 1 < declarators.len() => {
                    replaced.push((end_token, format!(";{next}")))
                }
                _ => {}
            }
        }
        for &i in &words[..name_at] {
            if runtime[0] && text(i) == "const" {
                self.removed.push(i..i + 1);
            }
        }
        self.replaced.extend(replaced);
        self.inits.extend(names);
        self.literal_globals.extend(literal);
    }

    /// `layout(std430, binding = n) buffer Name {`: an SSBO, in set 4 at its binding. Its
    /// access qualifiers go, so that every stage binds it alike (read and write).
    fn buffer_block(&mut self, words: &[usize]) {
        let mut binding = None;
        for (k, &i) in words.iter().enumerate() {
            match self.tokens[i] {
                // An extension adds real buffer syntax even in a GLSL 1.x stage.
                // It must not take the legacy identifier rename for `buffer`.
                "buffer" => {
                    self.replaced.insert(i, "buffer".into());
                }
                "binding" => {
                    binding = words
                        .get(k + 2)
                        .and_then(|&n| self.tokens[n].parse::<u32>().ok());
                    self.replaced.insert(i, "set = 4, binding".into());
                }
                "readonly" | "writeonly" | "restrict" | "coherent" | "volatile" => {
                    self.removed.push(i..i + 1)
                }
                _ => {}
            }
        }
        match binding {
            Some(b) => _ = self.buffers.insert(b),
            None => {
                let at = words.first().copied().unwrap_or(0);
                self.errors
                    .push(self.error(at, "a buffer block needs a binding"));
            }
        }
    }

    /// `struct Name {` at token `open`, whose words start at `from`: its members, to `}`.
    fn structure(&mut self, from: usize, open: usize) {
        let words: Vec<&str> = self.tokens[from..open]
            .iter()
            .copied()
            .filter(|t| significant(t))
            .collect();
        let [.., "struct", name] = words[..] else {
            return;
        };
        let mut members = Vec::new();
        let mut decl: Vec<&str> = Vec::new();
        for &t in &self.tokens[open + 1..] {
            match t {
                "}" => break,
                ";" => {
                    // `T a, b[2]`, after any precision or other qualifier.
                    let words: Vec<&str> =
                        decl.drain(..).filter(|w| !QUALIFIERS.contains(w)).collect();
                    let Some((ty, rest)) = words.split_first() else {
                        continue;
                    };
                    for d in rest.split(|w| *w == ",") {
                        if let Some((n, array)) = d.split_first() {
                            members.push((ty.to_string(), format!("{n}{}", array.concat())));
                        }
                    }
                }
                t if significant(t) => decl.push(t),
                _ => {}
            }
        }
        self.structs.insert(name.to_owned(), members);
    }

    /// A function definition or prototype: remember which parameters are samplers.
    pub(super) fn function(&mut self, words: &[usize]) {
        let Some(open) = words.iter().position(|&i| self.tokens[i] == "(") else {
            return;
        };
        let Some(&name) = open.checked_sub(1).and_then(|n| words.get(n)) else {
            return;
        };
        self.globals.insert(self.tokens[name].to_owned());
        let mut params = Vec::new();
        let mut is_sampler = false;
        for &i in &words[open + 1..] {
            match self.tokens[i] {
                "," | ")" => {
                    params.push(is_sampler);
                    is_sampler = false;
                }
                t if is_sampler_type(t) => is_sampler = true,
                _ => {}
            }
        }
        if params.iter().any(|&p| p) {
            self.sampler_functions
                .insert(self.tokens[name].to_owned(), params);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::pack::tests::zip;
    use crate::translate::tests::program;
    use crate::translate::{Target, translate_compute};
    use crate::{Macros, Pack, preprocess};

    #[test]
    fn a_struct_defined_in_a_varyings_declaration_is_a_varying() {
        // GLSL 4.60, 4.1.8: the qualifiers before `struct` apply to the declarators after `}`.
        let vsh = "#version 120\nflat varying struct P { vec2 a; vec2 b; } p, q;\nvoid main() {\n    \
                   gl_Position = ftransform();\n    p.a = vec2(1.0);\n    q.b = vec2(2.0);\n}\n";
        let fsh = "#version 120\nflat varying struct P { vec2 a; vec2 b; } p, q;\n\
                   void main() { gl_FragData[0] = vec4(p.a, q.b); }\n";
        let p = program(
            &[("composite.vsh", vsh), ("composite.fsh", fsh)],
            Target::Buffers,
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
        // `p` and `q`, each packed into one slot.
        let inputs = p.fragment.entry_points[0].function.arguments.len();
        assert_eq!(inputs, 2, "{inputs} inputs");
    }

    /// A global of literals is the module's initializer, as a GL driver takes it, with nothing of
    /// ours storing it again in every invocation: a table read at a dynamic index (TAA jitter,
    /// AO offsets) is constant data, not an array copied for each pixel. One that reads a uniform
    /// still runs in our `main`, and so does a `const` that names a global of literals.
    #[test]
    fn a_global_of_literals_is_initialized_by_the_module() {
        let m = fragment(
            "#version 120\nuniform float viewWidth;\nuniform int frameCounter;\n\
             vec2 offsets[3] = vec2[3](vec2(-0.5, 1.0), vec2(0.25, -2.0), vec2(1.5));\n\
             float bias = -1.0, scaled = viewWidth * 2.0;\nconst float twice = bias * 2.0;\n\
             void main() { gl_FragData[0] = vec4(offsets[frameCounter], bias + twice, scaled); }\n",
        );
        assert_eq!(
            init(&m, "offsets"),
            floats(&[-0.5, 1.0, 0.25, -2.0, 1.5, 1.5])
        );
        assert_eq!(init(&m, "bias"), floats(&[-1.0]));
        assert_eq!(init(&m, "scaled"), None);
        assert_eq!(init(&m, "twice"), None);
        assert_eq!(initializers(&m), ["hz_init_scaled", "hz_init_twice"]);
    }

    /// GL drivers take literals of another type than the global's (`int steps = 8.0;`) and
    /// convert them as the type's constructor does: each global's constant is a value of its own
    /// type, as it is for literals of the global's type.
    #[test]
    fn a_global_of_literals_converts_to_its_type() {
        use naga::Literal::{Bool, I32, U32};
        let m = fragment(
            "#version 330\nout vec4 c;\n\
             int steps = 8.0;\nbool on = 1;\nint i = 1u;\nmat3 m = 1.0;\nvec3 tint = 1.0;\n\
             vec3 fog = vec4(0.5, 0.6, 0.7, 1.0);\nfloat f = vec2(1.0, 2.0);\n\
             int taps[2] = int[2](1u, -2);\nuint mask = 3u;\nbool off = false;\n\
             bvec2 flags = bvec2(true, false);\n\
             void main() { c = vec4(tint * m * fog * f, float(steps + i + taps[1]) * float(on)\n\
             + float(mask) + float(off) + float(flags.x)); }\n",
        );
        assert_eq!(init(&m, "steps"), Some(vec![I32(8)]));
        assert_eq!(init(&m, "on"), Some(vec![Bool(true)]));
        assert_eq!(init(&m, "i"), Some(vec![I32(1)]));
        let identity = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        assert_eq!(init(&m, "m"), floats(&identity));
        assert_eq!(init(&m, "tint"), floats(&[1.0, 1.0, 1.0]));
        assert_eq!(init(&m, "fog"), floats(&[0.5, 0.6, 0.7]));
        assert_eq!(init(&m, "f"), floats(&[1.0]));
        assert_eq!(init(&m, "taps"), Some(vec![I32(1), I32(-2)]));
        assert_eq!(init(&m, "mask"), Some(vec![U32(3)]));
        assert_eq!(init(&m, "off"), Some(vec![Bool(false)]));
        assert_eq!(init(&m, "flags"), Some(vec![Bool(true), Bool(false)]));
        assert!(initializers(&m).is_empty(), "{:?}", initializers(&m));
    }

    /// A function declared before its body may read a global declared after an initializer
    /// that calls it (GLSL 4.60, 4.1.11 lets one call the pack's functions). Such globals run in
    /// our `main` after that initializer, in file order, so `a` reads `k` before `k` is set. A
    /// global of literals before the call keeps its constant.
    #[test]
    fn globals_after_a_call_before_a_body_run_in_file_order() {
        let m = fragment(
            "#version 120\nfloat f();\nfloat base = 1.0;\nfloat a = f();\nfloat k = 2.0;\n\
             float f() { return base + k; }\n\
             void main() { gl_FragData[0] = vec4(a, k, 0.0, 1.0); }\n",
        );
        assert_eq!(init(&m, "base"), floats(&[1.0]));
        assert_eq!(init(&m, "k"), None);
        let entry = &m.entry_points[0].function;
        let mut stored = Vec::new();
        stores(&m, entry, &entry.body, &mut stored);
        stored.retain(|&name| matches!(name, "a" | "k"));
        assert_eq!(stored, ["a", "k"]);
    }

    /// Workgroup memory takes no initializer in naga: a `shared` global of literals, which GLSL
    /// forbids and a driver may still take, runs in our `main`.
    #[test]
    fn a_shared_global_of_literals_runs_in_main() {
        let csh = "#version 430\nlayout(local_size_x = 1) in;\nshared float k = 2.0;\n\
                   layout(std430, binding = 0) buffer B { float v; } b;\nvoid main() { b.v = k; }\n";
        let pack = Pack::from_zip_bytes("P.zip", zip(&[("shaders/composite.csh", csh)])).unwrap();
        let p = translate_compute(&preprocess(&pack, "composite.csh", &Macros::standard()))
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(initializers(&p.module), ["hz_init_k"]);
    }

    /// The fragment stage of a composite program whose fragment source is `fsh`.
    fn fragment(fsh: &str) -> naga::Module {
        let vsh = "#version 120\nvoid main() { gl_Position = ftransform(); }\n";
        program(
            &[("composite.vsh", vsh), ("composite.fsh", fsh)],
            Target::Buffers,
        )
        .unwrap_or_else(|e| panic!("{e:?}"))
        .fragment
    }

    /// The literals of global `name`'s initializer, component by component; `None` without one.
    fn init(m: &naga::Module, name: &str) -> Option<Vec<naga::Literal>> {
        let (_, global) = (m.global_variables.iter())
            .find(|(_, g)| g.name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("no global {name}"));
        global.init.map(|e| {
            let mut values = Vec::new();
            literals(m, e, &mut values);
            values
        })
    }

    fn floats(values: &[f32]) -> Option<Vec<naga::Literal>> {
        Some(values.iter().map(|&v| naga::Literal::F32(v)).collect())
    }

    /// Our functions that run a global's initializer in `main`, in the module's order.
    fn initializers(m: &naga::Module) -> Vec<&str> {
        (m.functions.iter())
            .filter_map(|(_, f)| f.name.as_deref())
            .filter(|name| name.starts_with("hz_init_"))
            .collect()
    }

    /// The globals `body` of `f` stores to, in order, through the functions it calls.
    fn stores<'m>(
        m: &'m naga::Module,
        f: &naga::Function,
        body: &naga::Block,
        out: &mut Vec<&'m str>,
    ) {
        for statement in body {
            match *statement {
                naga::Statement::Store { pointer, .. } => {
                    if let naga::Expression::GlobalVariable(g) = f.expressions[pointer]
                        && let Some(name) = m.global_variables[g].name.as_deref()
                    {
                        out.push(name);
                    }
                }
                naga::Statement::Call { function, .. } => {
                    let callee = &m.functions[function];
                    stores(m, callee, &callee.body, out);
                }
                naga::Statement::Block(ref block) => stores(m, f, block, out),
                _ => {}
            }
        }
    }

    /// The literals of a constant expression of `m`, component by component.
    fn literals(m: &naga::Module, e: naga::Handle<naga::Expression>, out: &mut Vec<naga::Literal>) {
        match m.global_expressions[e] {
            naga::Expression::Literal(v) => out.push(v),
            naga::Expression::Compose { ref components, .. } => {
                for &c in components {
                    literals(m, c, out);
                }
            }
            naga::Expression::Splat { size, value } => {
                for _ in 0..size as u8 {
                    literals(m, value, out);
                }
            }
            ref other => panic!("not a literal: {other:?}"),
        }
    }
}
