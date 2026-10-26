//! The second pass: the stage's code written again for naga, line for line, and the
//! declarations of ours it needs.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;

use naga::ShaderStage;

use super::scan::{LATER_KEYWORDS, Stage, UNIFORM_STATE, VERTEX_INPUTS, is_ident, significant};
use super::{Sampler, split_sampler};

impl Stage<'_> {
    /// Declarations of ours this stage uses (after the uniform block, samplers and varyings).
    pub(super) fn prelude(&self) -> String {
        let mut p = String::from(
            "float hz_modf(float x, out float i) { i = trunc(x); return x - i; }\n\
             vec2 hz_modf(vec2 x, out vec2 i) { i = trunc(x); return x - i; }\n\
             vec3 hz_modf(vec3 x, out vec3 i) { i = trunc(x); return x - i; }\n\
             vec4 hz_modf(vec4 x, out vec4 i) { i = trunc(x); return x - i; }\n\
             struct hz_FogParameters { vec4 color; float density; float start; float end; float scale; };\n\
             hz_FogParameters hz_Fog;\n",
        );
        if self.gbuffers {
            // Set by our `main` from the frame's matrices (gbuffers.rs).
            p.push_str(
                "mat4 hz_ModelViewMatrix; mat4 hz_ProjectionMatrix; mat4 hz_ModelViewProjectionMatrix;\n\
                 mat4 hz_ModelViewMatrixInverse; mat4 hz_ProjectionMatrixInverse; mat3 hz_NormalMatrix; mat4 hz_TextureMatrix[8];\n",
            );
        } else {
            p.push_str(
                "const mat4 hz_ModelViewMatrix = mat4(1.0); const mat4 hz_ProjectionMatrix = mat4(2.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, -1.0, -1.0, 0.0, 1.0);\n\
                 const mat4 hz_ModelViewProjectionMatrix = hz_ProjectionMatrix; const mat4 hz_ModelViewMatrixInverse = mat4(1.0); const mat3 hz_NormalMatrix = mat3(1.0);\n\
                 const mat4 hz_ProjectionMatrixInverse = mat4(0.5, 0.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.5, 0.5, 0.0, 1.0);\n\
                 const mat4 hz_TextureMatrix[8] = mat4[8](mat4(1.0), mat4(1.0), mat4(1.0), mat4(1.0), mat4(1.0), mat4(1.0), mat4(1.0), mat4(1.0));\n",
            );
        }
        if self.kind == ShaderStage::Vertex {
            p.push_str(
                "const vec2 hz_quad[6] = vec2[6](vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0));\n\
                 vec4 hz_Vertex; vec4 hz_Color; vec4 hz_SecondaryColor; vec3 hz_Normal; float hz_FogCoord;\n\
                 vec4 hz_MultiTexCoord0; vec4 hz_MultiTexCoord1; vec4 hz_MultiTexCoord2; vec4 hz_MultiTexCoord3;\n\
                 vec4 hz_MultiTexCoord4; vec4 hz_MultiTexCoord5; vec4 hz_MultiTexCoord6; vec4 hz_MultiTexCoord7;\n",
            );
        } else {
            for n in &self.outputs {
                if !self.frag_outs.iter().any(|(at, _)| at == n) {
                    writeln!(p, "layout(location = {n}) out vec4 hz_out{n};").unwrap();
                }
            }
            for (_, decl) in &self.frag_outs {
                writeln!(p, "{decl}").unwrap();
            }
            p.push_str("vec4 hz_FragCoord;\n");
        }
        p
    }

    /// The pack's code with the rewrites above; the same number of lines.
    pub(super) fn rewrite(
        &self,
        samplers: &[Sampler],
        uniforms: &BTreeMap<String, String>,
    ) -> String {
        let removed = |i: usize| self.removed.iter().any(|r| r.contains(&i));
        let sampler_ty: HashMap<&str, &str> = samplers
            .iter()
            .map(|s| (s.name.as_str(), s.ty.as_str()))
            .collect();
        let old = self.pp.version.as_ref().is_none_or(|(v, _)| *v < 400);
        let fragment = self.kind == ShaderStage::Fragment;
        let mut out = String::with_capacity(self.pp.code.len() + 1024);
        // Per open bracket: the function called, and the closing text to add.
        let mut calls: Vec<(&str, &str, usize)> = Vec::new();
        // Sampler parameters of the function being written: name → type.
        let mut params: HashMap<String, String> = HashMap::new();
        // Names of samplers a local variable or parameter of the function hides.
        let mut hidden: Vec<&str> = Vec::new();
        let mut depth = 0;
        let mut skip_until = 0;
        let next = |i: usize| (i + 1..self.tokens.len()).find(|&j| significant(self.tokens[j]));
        let prev = |i: usize| (0..i).rev().find(|&j| significant(self.tokens[j]));
        for (i, &t) in self.tokens.iter().enumerate() {
            if i < skip_until {
                continue;
            }
            if let Some(text) = self.replaced.get(&i) {
                out.push_str(text);
                continue;
            }
            if removed(i) {
                out.extend(t.chars().filter(|&c| c == '\n'));
                if t.contains(' ') && !t.contains('\n') {
                    out.push(' ');
                }
                continue;
            }
            let called = next(i).is_some_and(|j| self.tokens[j] == "(");
            match t {
                "{" => depth += 1,
                "}" => {
                    depth -= 1;
                    if depth == 0 {
                        params.clear();
                        hidden.clear();
                    }
                }
                "(" => {
                    let callee = prev(i).map_or("", |j| self.tokens[j]);
                    let close = if matches!(callee, "shadow2D" | "shadow2DLod" | "shadow2DProj") {
                        ")"
                    } else {
                        ""
                    };
                    calls.push((callee, close, 0));
                }
                ")" => {
                    if let Some((_, close, _)) = calls.pop() {
                        out.push_str(close);
                    }
                }
                "," => {
                    if let Some(c) = calls.last_mut() {
                        c.2 += 1;
                    }
                }
                _ => {}
            }
            // `T x[] = T[](a, b)`: naga gives unsized arrays no size, so the count is written in.
            if t == "["
                && let Some(close) = next(i).filter(|&j| self.tokens[j] == "]")
            {
                if let Some(n) = self.array_length(close) {
                    write!(out, "[{n}]").unwrap();
                    skip_until = close + 1;
                    continue;
                }
            }
            if !is_ident(t) {
                out.push_str(t);
                continue;
            }
            // A function definition's sampler parameters: `sampler2D s` → `texture2D s, sampler hz_s_s`.
            if depth == 0 && t.contains("sampler") && calls.len() == 1 {
                if let Some(n) = next(i).filter(|&n| is_ident(self.tokens[n])) {
                    let name = self.tokens[n];
                    let (texture, sampler) = split_sampler(t);
                    write!(out, "{texture} {name}, {sampler} hz_s_{name}").unwrap();
                    params.insert(name.to_owned(), t.to_owned());
                    skip_until = n + 1;
                    continue;
                }
            }
            // `float specular`: a variable of the same name as a sampler, from here to the
            // function's end; `.specular` a member.
            if sampler_ty.contains_key(t) && !params.contains_key(t) {
                let before = prev(i).map(|j| self.tokens[j]);
                if before == Some(".") {
                    out.push_str(t);
                    continue;
                }
                if before.is_some_and(|b| is_ident(b) && !matches!(b, "return" | "case" | "else")) {
                    hidden.push(t);
                }
            }
            if hidden.contains(&t) {
                out.push_str(t);
                continue;
            }
            let sampler = params
                .get(t)
                .map(String::as_str)
                .or_else(|| sampler_ty.get(t).copied());
            if let Some(ty) = sampler {
                let as_argument = calls.last().is_some_and(|(callee, _, arg)| {
                    self.sampler_functions
                        .get(*callee)
                        .is_some_and(|p| p.get(*arg) == Some(&true))
                }) && prev(i)
                    .is_some_and(|j| matches!(self.tokens[j], "(" | ","));
                if as_argument {
                    write!(out, "{t}, hz_s_{t}").unwrap();
                } else {
                    write!(out, "{ty}({t}, hz_s_{t})").unwrap();
                }
                continue;
            }
            if uniforms.get(t).is_some_and(|ty| ty == "bool") {
                write!(out, "(hz_bool_{t} != 0)").unwrap();
                continue;
            }
            let replaced: Option<String> = match t {
                "main" if depth == 0 && called => Some("hz_main".into()),
                // naga's frontend leaves the code after an unconditional `discard` or `return;`
                // unreachable but half-built (a module that fails validation): behind an `if`
                // it stays reachable as far as naga knows.
                "discard" => Some("if (true) discard".into()),
                "return" if next(i).is_some_and(|j| self.tokens[j] == ";") => {
                    Some("if (true) return".into())
                }
                // GLSL's `modf`, which naga's frontend lacks.
                "modf" if called => Some("hz_modf".into()),
                "texture2D" | "texture1D" | "texture3D" | "textureCube" if called => {
                    Some("texture".into())
                }
                "texture2DLod" | "texture3DLod" | "texture2DLodARB" | "textureCubeLod"
                    if called =>
                {
                    Some("textureLod".into())
                }
                "texture2DProj" if called => Some("textureProj".into()),
                "texture2DGrad" | "texture2DGradARB" if called => Some("textureGrad".into()),
                "shadow2D" if called => Some("vec4(texture".into()),
                "shadow2DLod" if called => Some("vec4(textureLod".into()),
                "shadow2DProj" if called => Some("vec4(textureProj".into()),
                "ftransform" if called => {
                    // `ftransform()` → the product it stands for; its `()` stays as a no-op call... so skip it.
                    let close = next(i).and_then(next);
                    skip_until = close.map_or(i + 1, |c| c + 1);
                    Some("(hz_ModelViewProjectionMatrix * hz_Vertex)".into())
                }
                "gl_FragColor" => Some("hz_out0".into()),
                "gl_FragData" | "gl_TexCoord" => match self.index_after(i) {
                    Some((n, close)) => {
                        skip_until = close + 1;
                        Some(match t {
                            "gl_FragData" => format!("hz_out{n}"),
                            _ => format!("hz_TexCoord{n}"),
                        })
                    }
                    None => None,
                },
                "gl_Color" if fragment => Some("hz_FrontColor".into()),
                "gl_FragCoord" if !fragment => None,
                "gl_FragCoord" => Some("hz_FragCoord".into()),
                "gl_FrontColor" => Some("hz_FrontColor".into()),
                "gl_Fog" => Some("hz_Fog".into()),
                _ if UNIFORM_STATE.iter().any(|b| t.starts_with(b)) => {
                    Some(format!("hz_{}", &t[3..]))
                }
                _ if !fragment && VERTEX_INPUTS.iter().any(|b| t.starts_with(b)) => {
                    Some(format!("hz_{}", &t[3..]))
                }
                "gl_VertexID" => Some("gl_VertexIndex".into()),
                "gl_InstanceID" => Some("gl_InstanceIndex".into()),
                _ if old && LATER_KEYWORDS.contains(&t) => Some(format!("hz_kw_{t}")),
                _ => None,
            };
            out.push_str(replaced.as_deref().unwrap_or(t));
        }
        out
    }
}

impl Stage<'_> {
    /// The element count of an unsized array whose `[]` ends at `close`: the arguments of the
    /// constructor that follows (`[] = T[](...)` or `[](...)`).
    pub(super) fn array_length(&self, close: usize) -> Option<usize> {
        let mut j = close + 1;
        let mut seen_eq = false;
        while let Some(&t) = self.tokens.get(j) {
            match t {
                "(" => break,
                "=" if !seen_eq => seen_eq = true,
                ";" | "," | ")" | "{" => return None,
                _ => {}
            }
            j += 1;
        }
        let (mut depth, mut args, mut any) = (0, 1, false);
        for &t in &self.tokens[j + 1..] {
            match t {
                "(" | "[" | "{" => depth += 1,
                ")" if depth == 0 => return any.then_some(args),
                ")" | "]" | "}" => depth -= 1,
                "," if depth == 0 => args += 1,
                _ if significant(t) => any = true,
                _ => {}
            }
        }
        None
    }
}

/// What our vertex `main` gives a vertex input of a composite-style program.
pub(super) fn attribute_value(name: &str, ty: &str) -> String {
    match name {
        "vaPosition" => "vec3(hz_p, 0.0)".into(),
        "vaUV0" => "hz_p".into(),
        "vaColor" => "vec4(1.0)".into(),
        "vaNormal" => "vec3(0.0, 0.0, 1.0)".into(),
        _ => format!("{ty}(0)"),
    }
}
