//! Iris's full-screen passes and the colour targets they draw to: which
//! programs run (`begin*`, `prepare*`, `deferred*`, `composite*`, `final`),
//! the targets each one writes (`/* DRAWBUFFERS:01 */`, `/* RENDERTARGETS:
//! 0,1 */`), the targets' formats and clearing (`const int colortex0Format
//! = RGBA16F;` and friends), and which side of each ping-ponged target a
//! pass reads (`BufferFlipper`). Read from the preprocessed sources with
//! their comments, as Iris's `ProgramDirectives`, `PackRenderTargetDirectives`
//! and `CompositeRenderer` do.

use crate::{
    JavaPack,
    options::OptionValues,
    preprocess::Environment,
    properties::{Blend, LEGACY_TARGETS, ShaderProperties},
    translate::{Link, Translated},
};
use std::collections::{BTreeMap, BTreeSet};

/// Colour targets Iris supports (`IrisLimits.MAX_COLOR_BUFFERS`).
pub const MAX_COLOR_BUFFERS: u32 = 32;

/// The formats `const int colortexNFormat` may name (`InternalTextureFormat`).
pub const TEXTURE_FORMATS: &[&str] = &[
    "RGBA",
    "R8",
    "RG8",
    "RGB8",
    "RGBA8",
    "R8_SNORM",
    "RG8_SNORM",
    "RGB8_SNORM",
    "RGBA8_SNORM",
    "R16",
    "RG16",
    "RGB16",
    "RGBA16",
    "R16_SNORM",
    "RG16_SNORM",
    "RGB16_SNORM",
    "RGBA16_SNORM",
    "R16F",
    "RG16F",
    "RGB16F",
    "RGBA16F",
    "R32F",
    "RG32F",
    "RGB32F",
    "RGBA32F",
    "R8I",
    "RG8I",
    "RGB8I",
    "RGBA8I",
    "R8UI",
    "RG8UI",
    "RGB8UI",
    "RGBA8UI",
    "R16I",
    "RG16I",
    "RGB16I",
    "RGBA16I",
    "R16UI",
    "RG16UI",
    "RGB16UI",
    "RGBA16UI",
    "R32I",
    "RG32I",
    "RGB32I",
    "RGBA32I",
    "R32UI",
    "RG32UI",
    "RGB32UI",
    "RGBA32UI",
    "RGBA2",
    "RGBA4",
    "R3_G3_B2",
    "RGB5_A1",
    "RGB565",
    "RGB10_A2",
    "RGB10_A2UI",
    "R11F_G11F_B10F",
    "RGB9_E5",
];

/// The value of the last `/* NAME:value */` in `text` and where it starts
/// (`CommentDirectiveParser.findDirective`).
pub fn comment_directive(text: &str, name: &str) -> Option<(usize, String)> {
    let prefix = format!("{name}:");
    let at = text.rfind(&prefix)?;
    if !text[..at].trim_end().ends_with("/*") {
        return None;
    }
    let rest = &text[at + prefix.len()..];
    let end = rest.find("*/")?;
    Some((at, rest[..end].trim().to_owned()))
}

/// A `const <type> <key> = <value>;` line (`ConstDirectiveParser`).
#[derive(Clone, Debug, PartialEq)]
pub struct ConstDirective {
    /// `int`, `float`, `vec2`, `ivec3`, `vec4` or `bool`.
    pub ty: &'static str,
    pub key: String,
    pub value: String,
}

/// Every line of `text` that starts with a `const` Iris reads, in order.
pub fn const_directives(text: &str) -> Vec<ConstDirective> {
    text.lines().filter_map(const_directive).collect()
}

fn const_directive(line: &str) -> Option<ConstDirective> {
    if !line.contains("const") || !line.contains('=') || !line.contains(';') {
        return None;
    }
    let rest = line.trim().strip_prefix("const")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim_start();
    // In Iris's order: `ivec3` before `vec4` doesn't matter, but `int` must
    // not take `ivec3`'s place, and it can't (`ivec3` starts with `i`, `v`).
    let ty = ["int", "float", "vec2", "ivec3", "vec4", "bool"]
        .into_iter()
        .find(|t| rest.starts_with(t))?;
    let rest = &rest[ty.len()..];
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let (key, value) = rest.split_once('=')?;
    let key = key.trim();
    if key.is_empty() || !key.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    let value = value.split_once(';')?.0.trim();
    Some(ConstDirective {
        ty,
        key: key.to_owned(),
        value: value.to_owned(),
    })
}

/// The targets a fragment program writes, by output location: the later of
/// its `DRAWBUFFERS` and `RENDERTARGETS` directives, or `None` without one
/// (Iris then takes the `gl_FragData` indices it writes).
pub fn draw_buffers(fragment: &str) -> Result<Option<Vec<u32>>, String> {
    let draw = comment_directive(fragment, "DRAWBUFFERS");
    let render = comment_directive(fragment, "RENDERTARGETS");
    let (digits, value) = match (draw, render) {
        (Some(d), Some(r)) if d.0 > r.0 => (true, d.1),
        (_, Some(r)) => (false, r.1),
        (Some(d), None) => (true, d.1),
        (None, None) => return Ok(None),
    };
    let bad = || format!("invalid render targets `{value}`");
    let targets: Vec<u32> = if digits {
        value
            .chars()
            .map(|c| c.to_digit(10).ok_or_else(bad))
            .collect::<Result<_, _>>()?
    } else {
        value
            .split(',')
            .map(|t| t.trim().parse().map_err(|_| bad()))
            .collect::<Result<_, _>>()?
    };
    match targets.iter().find(|&&t| t >= MAX_COLOR_BUFFERS) {
        Some(t) => Err(format!(
            "render target {t} is not supported, only {MAX_COLOR_BUFFERS} are"
        )),
        None => Ok(Some(targets)),
    }
}

/// The index of a target name, `colortex<N>` or a legacy one (`gaux1`).
pub fn target_index(name: &str) -> Option<u32> {
    match LEGACY_TARGETS.iter().position(|t| *t == name) {
        Some(i) => Some(i as u32),
        None => name
            .strip_prefix("colortex")?
            .parse()
            .ok()
            .filter(|&i| i < MAX_COLOR_BUFFERS),
    }
}

/// A key naming a target and a suffix (`colortex4Format`, `gaux1Clear`).
fn target_key(key: &str, suffix: &str) -> Option<u32> {
    target_index(key.strip_suffix(suffix)?)
}

/// A colour target's settings (`RenderTargetSettings`, `size.buffer.*`).
#[derive(Clone, Debug, PartialEq)]
pub struct TargetSettings {
    /// One of [`TEXTURE_FORMATS`]; `RGBA` (8 bits per channel) by default.
    pub format: String,
    /// Cleared every frame (the default), or kept for the next frame.
    pub clear: bool,
    /// Iris clears to black, or `colortex0` to the fog colour, without one.
    pub clear_color: Option<[f32; 4]>,
    /// `size.buffer.<name>`: a fraction of the screen or a size in pixels.
    pub size: Option<TargetSize>,
}

impl Default for TargetSettings {
    fn default() -> Self {
        Self {
            format: "RGBA".into(),
            clear: true,
            clear_color: None,
            size: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TargetSize {
    Relative(f32, f32),
    Pixels(u32, u32),
}

/// Every target's settings from the consts of `fragments`, read in order so
/// the last one wins, then `size.buffer.*` from the properties.
pub fn target_settings<'a>(
    fragments: impl IntoIterator<Item = &'a str>,
    properties: &ShaderProperties,
) -> Vec<TargetSettings> {
    let mut targets = vec![TargetSettings::default(); MAX_COLOR_BUFFERS as usize];
    for fragment in fragments {
        for c in const_directives(fragment) {
            match c.ty {
                "int" => {
                    if let Some(i) = target_key(&c.key, "Format") {
                        let format = c.value.to_uppercase();
                        if TEXTURE_FORMATS.contains(&format.as_str()) {
                            targets[i as usize].format = format;
                        }
                    }
                }
                "bool" => {
                    if let (Some(i), Ok(clear)) = (target_key(&c.key, "Clear"), c.value.parse()) {
                        targets[i as usize].clear = clear;
                    }
                }
                "vec4" => {
                    if let (Some(i), Some(color)) =
                        (target_key(&c.key, "ClearColor"), parse_vec4(&c.value))
                    {
                        targets[i as usize].clear_color = Some(color);
                    }
                }
                _ => {}
            }
        }
    }
    for (key, value) in properties.preprocessed.iter() {
        let Some(i) = key.strip_prefix("size.buffer.").and_then(target_index) else {
            continue;
        };
        let parts: Vec<&str> = value.split_whitespace().collect();
        if let [w, h] = parts[..] {
            targets[i as usize].size = if w.contains('.') || h.contains('.') {
                w.parse()
                    .ok()
                    .zip(h.parse().ok())
                    .map(|(w, h)| TargetSize::Relative(w, h))
            } else {
                w.parse()
                    .ok()
                    .zip(h.parse().ok())
                    .map(|(w, h)| TargetSize::Pixels(w, h))
            };
        }
    }
    targets
}

fn parse_vec4(value: &str) -> Option<[f32; 4]> {
    let args = value.strip_prefix("vec4")?.trim();
    let args = args.strip_prefix('(')?.strip_suffix(')')?;
    let parts: Vec<f32> = args
        .split(',')
        .map(|p| p.trim().parse().ok())
        .collect::<Option<_>>()?;
    parts.try_into().ok()
}

/// One full-screen pass.
#[derive(Clone, Debug, PartialEq)]
pub struct Pass {
    /// The program's path without extension (`world0/composite1`).
    pub program: String,
    /// What it writes, by output location; `None`: the outputs it writes.
    pub draw_buffers: Option<Vec<u32>>,
    /// Targets it reads from their alternate side (it writes the other side).
    pub reads_alt: BTreeSet<u32>,
    /// Targets whose mipmaps it wants generated before it runs.
    pub mipmapped: BTreeSet<u32>,
    /// `scale.<name>`: the fraction of the screen it draws.
    pub scale: f32,
    /// `blend.<program>` override for all outputs of this pass.
    pub blend: Option<Blend>,
    /// `blend.<program>.<buffer>` overrides per colour target index.
    pub buffer_blends: Vec<(u32, Blend)>,
}

/// The full-screen passes of one program folder, in order, and the
/// flipping state between them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Passes {
    pub targets: Vec<TargetSettings>,
    pub begin: Vec<Pass>,
    pub prepare: Vec<Pass>,
    /// The targets gbuffers programs (and Horizon's world) write on their
    /// alternate side, before and after translucent geometry.
    pub flipped_after_prepare: BTreeSet<u32>,
    /// The target Horizon's world goes into: the first one `gbuffers_terrain`
    /// (or the program Iris falls back to for terrain) draws to, `colortex0`
    /// without one.
    pub world_target: u32,
    pub deferred: Vec<Pass>,
    pub flipped_after_translucent: BTreeSet<u32>,
    pub composite: Vec<Pass>,
    /// `final`, which writes the screen; `None` without one (Iris then
    /// copies `colortex0` to the screen).
    pub final_pass: Option<Pass>,
    /// After `final`, these targets' alternate side is copied back so the
    /// next frame starts unflipped.
    pub flipped_at_end: BTreeSet<u32>,
    pub directives: Directives,
}

/// The `const` directives of a program folder that aren't about targets
/// (`PackDirectives`), read from the same programs in the same order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Directives {
    /// `sunPathRotation`, degrees: how far the sun and moon's path tilts
    /// toward the north or south (a Z rotation between the sky's Y -90 and
    /// its angle, `CelestialUniforms`).
    pub sun_path_rotation: f32,
    /// `eyeBrightnessHalflife`, tenths of a second: `eyeBrightnessSmooth`.
    pub eye_brightness_half_life: f32,
    /// `wetnessHalflife`, tenths of a second: `wetness` smoothing when raining.
    pub wetness_half_life: f32,
    /// `drynessHalflife`, tenths of a second: `wetness` smoothing when dry.
    pub dryness_half_life: f32,
}

impl Default for Directives {
    fn default() -> Self {
        Self {
            sun_path_rotation: 0.0,
            eye_brightness_half_life: 10.0,
            wetness_half_life: 600.0,
            dryness_half_life: 200.0,
        }
    }
}

impl Directives {
    /// Read from `fragments` in order, the last one winning.
    pub fn read<'a>(fragments: impl IntoIterator<Item = &'a str>) -> Self {
        let mut out = Self::default();
        for fragment in fragments {
            for c in const_directives(fragment) {
                // Java's `Float.parseFloat` takes a trailing `f`.
                let value = c.value.trim_end_matches(['f', 'F']).parse::<f32>();
                match (c.ty, c.key.as_str(), value) {
                    ("float", "sunPathRotation", Ok(v)) => out.sun_path_rotation = v,
                    ("float", "eyeBrightnessHalflife", Ok(v)) => out.eye_brightness_half_life = v,
                    ("float", "wetnessHalflife", Ok(v)) => out.wetness_half_life = v,
                    ("float", "drynessHalflife", Ok(v)) => out.dryness_half_life = v,
                    _ => {}
                }
            }
        }
        out
    }
}

/// Iris's single programs (`ProgramId`), in order.
const SINGLE_PROGRAMS: &[&str] = &[
    "shadow",
    "shadow_solid",
    "shadow_cutout",
    "shadow_water",
    "shadow_entities",
    "shadow_lightning",
    "shadow_block",
    "gbuffers_basic",
    "gbuffers_line",
    "gbuffers_textured",
    "gbuffers_textured_lit",
    "gbuffers_skybasic",
    "gbuffers_skytextured",
    "gbuffers_clouds",
    "gbuffers_terrain",
    "gbuffers_terrain_solid",
    "gbuffers_terrain_cutout",
    "gbuffers_damagedblock",
    "gbuffers_block",
    "gbuffers_block_translucent",
    "gbuffers_beaconbeam",
    "gbuffers_item",
    "gbuffers_entities",
    "gbuffers_entities_translucent",
    "gbuffers_lightning",
    "gbuffers_particles",
    "gbuffers_particles_translucent",
    "gbuffers_entities_glowing",
    "gbuffers_armor_glint",
    "gbuffers_spidereyes",
    "gbuffers_hand",
    "gbuffers_weather",
    "gbuffers_water",
    "gbuffers_hand_water",
    "dh_terrain",
    "dh_water",
    "dh_generic",
    "dh_shadow",
    "final",
];

/// The program groups that run as full-screen passes, in Iris's order.
const GROUPS: &[&str] = &["begin", "prepare", "deferred", "composite"];

impl JavaPack {
    /// The full-screen passes of the programs in `dir` (`world0`, or `""`
    /// for the pack's root), with the player's `values`.
    pub fn passes(
        &self,
        dir: &str,
        values: &OptionValues,
        env: &Environment,
        properties: &ShaderProperties,
    ) -> Result<Passes, String> {
        let path = |name: &str| {
            if dir.is_empty() {
                name.to_owned()
            } else {
                format!("{dir}/{name}")
            }
        };
        // A pass needs both halves, and its `program.*.enabled` to hold.
        let exists = |name: &str| {
            let p = path(name);
            self.programs.contains(&format!("{p}.vsh"))
                && self.programs.contains(&format!("{p}.fsh"))
                && properties.program_enabled(&p, &self.options, values)
        };
        let fragment = |name: &str| -> Result<String, String> {
            let file = format!("{}.fsh", path(name));
            Ok(self.preprocess(&file, values, env)?.with_comments())
        };
        let group = |base: &str| -> Vec<String> {
            (0..100)
                .map(|i| {
                    if i == 0 {
                        base.to_owned()
                    } else {
                        format!("{base}{i}")
                    }
                })
                .filter(|name| exists(name))
                .collect()
        };
        let groups: Vec<Vec<String>> = GROUPS.iter().map(|g| group(g)).collect();

        // Target settings come from the fragments of every pass, in Iris's
        // order (`ProgramSet.locateDirectives`): some packs declare them in
        // `final` only.
        let mut sources: Vec<String> = Vec::new();
        for name in group("shadowcomp")
            .iter()
            .chain(&groups[0])
            .chain(&groups[1])
        {
            sources.push(fragment(name)?);
        }
        for name in SINGLE_PROGRAMS {
            if exists(name) {
                sources.push(fragment(name)?);
            }
        }
        let mut fragments: BTreeMap<String, String> = BTreeMap::new();
        for name in groups[2].iter().chain(&groups[3]) {
            let text = fragment(name)?;
            sources.push(text.clone());
            fragments.insert(name.clone(), text);
        }
        for name in &groups[0..2].concat() {
            fragments.insert(name.clone(), fragment(name)?);
        }
        let mut out = Passes {
            targets: target_settings(sources.iter().map(String::as_str), properties),
            directives: Directives::read(sources.iter().map(String::as_str)),
            ..Passes::default()
        };

        let resolve_blends = |prog: &str| {
            let blend = properties.blend(prog).and_then(Result::ok);
            let buffer_blends: Vec<(u32, Blend)> = properties
                .buffer_blends(prog)
                .into_iter()
                .filter_map(|(i, b)| Some((i, b.ok()?)))
                .collect();
            (blend, buffer_blends)
        };

        let flips = explicit_flips(properties);
        let run = |group: &[String], pre: &str, flipped: &mut BTreeSet<u32>| {
            *flipped = flips_pre(&flips, pre, flipped);
            let mut passes = Vec::new();
            for name in group {
                let text = &fragments[name];
                let prog = path(name);
                let (blend, buffer_blends) = resolve_blends(&prog);
                let pass = Pass {
                    program: prog.clone(),
                    draw_buffers: draw_buffers(text).map_err(|e| format!("{prog}: {e}"))?,
                    reads_alt: flipped.clone(),
                    mipmapped: mipmapped(text),
                    scale: properties
                        .preprocessed
                        .get(&format!("scale.{name}"))
                        .and_then(|v| v.split_whitespace().next()?.parse().ok())
                        .unwrap_or(1.0),
                    blend,
                    buffer_blends,
                };
                *flipped = flip_set(&pass, &flips);
                passes.push(pass);
            }
            Ok::<_, String>(passes)
        };
        let mut flipped = BTreeSet::new();
        out.begin = run(&groups[0], "begin_pre", &mut flipped)?;
        out.prepare = run(&groups[1], "prepare_pre", &mut flipped)?;
        out.flipped_after_prepare = flipped.clone();
        // Iris's fallbacks for terrain (`ProgramFallbackResolver`).
        let terrain = [
            "gbuffers_terrain",
            "gbuffers_textured_lit",
            "gbuffers_textured",
            "gbuffers_basic",
        ]
        .into_iter()
        .find(|name| exists(name));
        if let Some(name) = terrain {
            let buffers =
                draw_buffers(&fragment(name)?).map_err(|e| format!("{}: {e}", path(name)))?;
            out.world_target = buffers.and_then(|b| b.first().copied()).unwrap_or(0);
        }
        out.deferred = run(&groups[2], "deferred_pre", &mut flipped)?;
        out.flipped_after_translucent = flipped.clone();
        out.composite = run(&groups[3], "composite_pre", &mut flipped)?;
        out.flipped_at_end = flipped;
        if exists("final") {
            let prog = path("final");
            let (blend, buffer_blends) = resolve_blends(&prog);
            out.final_pass = Some(Pass {
                program: prog,
                draw_buffers: None,
                reads_alt: out.flipped_at_end.clone(),
                mipmapped: mipmapped(&fragment("final")?),
                scale: 1.0,
                blend,
                buffer_blends,
            });
        }
        Ok(out)
    }
}

/// A pass's programs translated, with the targets it draws to.
pub struct Program {
    pub pass: Pass,
    pub vertex: Translated,
    pub fragment: Translated,
    /// The target each output location writes: the pass's directive, or
    /// as Iris does without one, location `i` to target `i`.
    pub draw_buffers: Vec<u32>,
    /// The bindings both programs share.
    pub link: Link,
}

impl JavaPack {
    /// Translate `pass`'s `.vsh` and `.fsh`.
    pub fn compile(
        &self,
        pass: &Pass,
        targets: &[TargetSettings],
        values: &OptionValues,
        env: &Environment,
        properties: &ShaderProperties,
    ) -> Result<Program, String> {
        let mut link = Link {
            image_formats: properties.image_formats(),
            shared_samplers: shared_samplers(targets),
            ..Link::default()
        };
        let vertex = self.translate(&format!("{}.vsh", pass.program), values, env, &mut link)?;
        let fragment = self.translate(&format!("{}.fsh", pass.program), values, env, &mut link)?;
        let draw_buffers = match &pass.draw_buffers {
            Some(buffers) => buffers.clone(),
            None => (0..fragment.interface.outputs.keys().max().map_or(1, |m| m + 1)).collect(),
        };
        Ok(Program {
            pass: pass.clone(),
            vertex,
            fragment,
            draw_buffers,
            link,
        })
    }
}

/// Iris's sampler states for the textures it binds: colour targets
/// linear (nearest for integer formats, and for 32-bit floats, which wgpu
/// can't filter everywhere), depth nearest, noise repeating. For
/// [`Link::shared_samplers`].
pub fn shared_samplers(targets: &[TargetSettings]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (i, t) in targets.iter().enumerate() {
        let f = &t.format;
        let class = if f.ends_with("32F") || f.ends_with('I') {
            "iris_nearest"
        } else {
            "iris_linear"
        };
        out.insert(format!("colortex{i}"), class.to_owned());
        if let Some(legacy) = LEGACY_TARGETS.get(i) {
            out.insert((*legacy).to_owned(), class.to_owned());
        }
    }
    for name in [
        "depthtex0",
        "depthtex1",
        "depthtex2",
        "gdepthtex",
        "shadowtex0",
        "shadowtex1",
        "shadow",
        "watershadow",
        "dhDepthTex",
        "dhDepthTex0",
        "dhDepthTex1",
        "vxDepthTexOpaque",
        "vxDepthTexTrans",
    ] {
        out.insert(name.to_owned(), "iris_nearest".to_owned());
    }
    for name in ["shadowcolor", "shadowcolor0", "shadowcolor1"] {
        out.insert(name.to_owned(), "iris_linear".to_owned());
    }
    out.insert("noisetex".to_owned(), "iris_repeat".to_owned());
    out
}

fn toggle(set: &mut BTreeSet<u32>, buffer: u32) {
    if !set.remove(&buffer) {
        set.insert(buffer);
    }
}

/// `flip.<pass>.<buffer> = true|false`, by pass and target index.
fn explicit_flips(properties: &ShaderProperties) -> BTreeMap<&str, BTreeMap<u32, bool>> {
    let mut out: BTreeMap<&str, BTreeMap<u32, bool>> = BTreeMap::new();
    for (key, value) in properties.preprocessed.iter() {
        let Some((pass, buffer)) = key.strip_prefix("flip.").and_then(|k| k.split_once('.')) else {
            continue;
        };
        if let (Some(i), Ok(flip)) = (target_index(buffer), value.trim().parse()) {
            out.entry(pass).or_default().insert(i, flip);
        }
    }
    out
}

/// The flipped set after `pass` ran: what it read, flipped by its writes
/// and its explicit flips.
fn flip_set(pass: &Pass, flips: &BTreeMap<&str, BTreeMap<u32, bool>>) -> BTreeSet<u32> {
    let name = pass.program.rsplit('/').next().unwrap_or(&pass.program);
    let explicit = flips.get(name);
    let mut set = pass.reads_alt.clone();
    for &buffer in pass.draw_buffers.as_deref().unwrap_or(&[0]) {
        if explicit.and_then(|f| f.get(&buffer)) != Some(&false) {
            toggle(&mut set, buffer);
        }
    }
    for (&buffer, &flip) in explicit.into_iter().flatten() {
        if flip {
            toggle(&mut set, buffer);
        }
    }
    set
}

fn flips_pre(
    flips: &BTreeMap<&str, BTreeMap<u32, bool>>,
    pre: &str,
    before: &BTreeSet<u32>,
) -> BTreeSet<u32> {
    let mut set = before.clone();
    for (&buffer, &flip) in flips.get(pre).into_iter().flatten() {
        if flip {
            toggle(&mut set, buffer);
        }
    }
    set
}

/// `const bool colortexNMipmapEnabled = true;` in a program's fragment.
fn mipmapped(fragment: &str) -> BTreeSet<u32> {
    let mut out = BTreeSet::new();
    for c in const_directives(fragment) {
        if c.ty != "bool" {
            continue;
        }
        if let Some(i) = target_key(&c.key, "MipmapEnabled") {
            match c.value.as_str() {
                "true" => out.insert(i),
                "false" => out.remove(&i),
                _ => false,
            };
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comment_directives_as_iris_finds_them() {
        // Iris's own cases (`CommentDirectiveParser.Tests`).
        let find = |text: &str, name: &str| comment_directive(text, name).map(|d| d.1);
        let draw = |text: &str| find(text, "DRAWBUFFERS");
        assert_eq!(
            draw("Some normal text that doesn't contain a DRAWBUFFERS directive"),
            None
        );
        assert_eq!(
            draw("doesn't contain a /* DRAWBUFFERS: directive of any sort"),
            None
        );
        assert_eq!(
            draw("/*DRAWBUFFERS:321*/ OptiFine will detect this").as_deref(),
            Some("321")
        );
        assert_eq!(
            draw("a directive: /* DRAWBUFFERS:321 */").as_deref(),
            Some("321")
        );
        assert_eq!(
            draw("This is a line /* DRAWBUFFERS:31 */ containing").as_deref(),
            Some("31")
        );
        assert_eq!(
            draw("/* DRAWBUFFERS: */ invalid but matching").as_deref(),
            Some("")
        );
        assert_eq!(draw("    /*DRAWBUFFERS:12*/").as_deref(), Some("12"));
        let lines = "/* RENDERTARGETS:Duplicate handling? */\nuniform sampler2D test;\n\
                     /* RENDERTARGETS:Within a line? */ Let's see /* RENDERTARGETS:It works */\n";
        assert_eq!(find(lines, "RENDERTARGETS").as_deref(), Some("It works"));

        // The later directive wins, whichever kind it is.
        assert_eq!(
            draw_buffers("/* RENDERTARGETS: 4,10 */ /* DRAWBUFFERS:02 */"),
            Ok(Some(vec![0, 2]))
        );
        assert_eq!(
            draw_buffers("/* DRAWBUFFERS:02 */ /* RENDERTARGETS: 4,10 */"),
            Ok(Some(vec![4, 10]))
        );
        assert_eq!(draw_buffers("void main() {}"), Ok(None));
        assert!(draw_buffers("/* RENDERTARGETS: 40 */").is_err());
        assert!(draw_buffers("/* DRAWBUFFERS:0a */").is_err());
    }

    #[test]
    fn sky_directives_last_one_wins() {
        assert_eq!(Directives::read([]), Directives::default());
        assert_eq!(Directives::default().eye_brightness_half_life, 10.0);
        let d = Directives::read([
            "const float sunPathRotation = 30.0;\nconst float eyeBrightnessHalflife = 4.0f;\nconst float wetnessHalflife = 300.0;",
            "const float sunPathRotation = -40.0; // Complementary Unbound",
            "const int sunPathRotation = 5;",
        ]);
        assert_eq!(d.sun_path_rotation, -40.0);
        assert_eq!(d.eye_brightness_half_life, 4.0);
        assert_eq!(d.wetness_half_life, 300.0);
        assert_eq!(d.dryness_half_life, 200.0, "the default");
    }

    #[test]
    fn const_directives_and_target_settings() {
        let text = "\
const int colortex4Format = rgba16f;
  const bool gaux2Clear = false; // kept between frames
const vec4 colortex0ClearColor = vec4(0.5, 0.25, 0.0, 1.0);
const int colortex5Format = NOT_A_FORMAT;
uniform const int x = 1;
const int colortex6Format=RGBA8;
const intx colortex7Format = RGBA8;
const ivec3 workGroups = ivec3(16, 8, 16);
";
        let consts = const_directives(text);
        assert_eq!(
            consts
                .iter()
                .map(|c| (c.ty, c.key.as_str(), c.value.as_str()))
                .collect::<Vec<_>>(),
            [
                ("int", "colortex4Format", "rgba16f"),
                ("bool", "gaux2Clear", "false"),
                ("vec4", "colortex0ClearColor", "vec4(0.5, 0.25, 0.0, 1.0)"),
                ("int", "colortex5Format", "NOT_A_FORMAT"),
                ("int", "colortex6Format", "RGBA8"),
                ("ivec3", "workGroups", "ivec3(16, 8, 16)"),
            ],
            "a type needs a space after it, the name doesn't"
        );
        let targets = target_settings([text], &ShaderProperties::default());
        assert_eq!(targets.len(), MAX_COLOR_BUFFERS as usize);
        assert_eq!(targets[4].format, "RGBA16F");
        assert_eq!(targets[5].format, "RGBA", "unknown formats are ignored");
        assert!(!targets[5].clear, "gaux2 is colortex5");
        assert_eq!(targets[0].clear_color, Some([0.5, 0.25, 0.0, 1.0]));
        assert_eq!(targets[1], TargetSettings::default());
    }
}
