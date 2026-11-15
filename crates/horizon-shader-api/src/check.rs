//! The check a pack passes before it runs: in the game when it loads (a pack
//! that fails keeps the last good one drawing), in the launcher before it
//! installs, in the registry's CI, and in `horizon-shader check`.
//!
//! Each stage is built as the renderer builds it, parsed and validated by
//! naga, checked against its signature and the API's rules (no bindings, no
//! entry points, no `horizon_` names; in 0.2 none of the structs Horizon hands it
//! constructed), then translated to SPIR-V, Metal and HLSL with its options at their
//! defaults, minimums and maximums. A post stage of a pack with `[ray_tracing]`, and its
//! material stage where that traces, is checked twice: as every device runs it, where `trace`
//! misses, and against the scene.
use crate::inputs::{COMPUTE_INPUT, DISPLACEMENT_INPUT, Frame, RAY_HIT};
use crate::manifest::{OptionKind, PackOption};
use crate::package::Pack;
use crate::stages::{
    Composed, GROUP, RAY_ENTITY_RETAINED_BINDING, RAY_ENTITY_STORE_BINDING, RAY_SCENE_BINDING,
    StageKind, compose_traced, identifiers,
};
use naga::valid::{Capabilities, ModuleInfo, ValidationFlags, Validator};
use std::collections::HashSet;
use std::fmt;

/// naga expressions a stage may hold, over all its functions.
pub const MAX_EXPRESSIONS: usize = 20_000;
/// Texture reads (samples, gathers and loads) a stage may write.
pub const MAX_TEXTURE_READS: usize = 64;

/// What is wrong, and where: a file of the pack, with the line and column
/// when known (0 when not).
#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.line == 0 {
            write!(f, "{}: {}", self.file, self.message)
        } else {
            write!(
                f,
                "{}:{}:{}: {}",
                self.file, self.line, self.column, self.message
            )
        }
    }
}

fn problem(file: &str, message: impl Into<String>) -> Problem {
    Problem {
        file: file.to_owned(),
        line: 0,
        column: 0,
        message: message.into(),
    }
}

/// One stage that passed, ready for the renderer.
#[derive(Debug)]
pub struct CheckedStage {
    pub kind: StageKind,
    pub file: String,
    /// The manifest's name for it, else its file.
    pub label: String,
    pub module: naga::Module,
    pub info: ModuleInfo,
    /// A post stage of a pack with `[ray_tracing]`, or its material stage where that calls
    /// a trace function, built against the scene (`compose_scene`): what the renderer runs
    /// while it binds the scene, on a device with ray queries. `module` is the same stage with a
    /// `trace` that misses, which every device takes.
    pub scene: Option<naga::Module>,
    /// The stage's WGSL without the entry points (the prelude, the shared
    /// files, the file), for a renderer that calls the stage function from
    /// its own shader (the sky).
    pub source: String,
    /// `source` built against the scene, with `scene`.
    pub scene_source: Option<String>,
    /// What the budgets hold: with `scene`, its counts, the larger ones.
    pub expressions: usize,
    pub texture_reads: usize,
}

impl CheckedStage {
    /// Whether the stage against the scene reads the entity layer too: its pack's files name
    /// `RAY_ENTITY` (`names_ray_entity`), so the renderer binds the layer for it.
    pub fn traces_entities(&self) -> bool {
        self.scene.as_ref().is_some_and(|module| {
            module.global_variables.iter().any(|(_, g)| {
                g.binding
                    .as_ref()
                    .is_some_and(|b| b.group == GROUP && b.binding == RAY_ENTITY_STORE_BINDING)
            })
        })
    }

    /// Whether the stage's function, or a function it calls, reads the prelude's `global`
    /// (a resource of the stage, such as `reflection`).
    pub fn reads(&self, global: &str) -> bool {
        let Some((function, _)) = self
            .module
            .functions
            .iter()
            .find(|(_, f)| f.name.as_deref() == Some(self.kind.function()))
        else {
            return false;
        };
        self.module
            .global_variables
            .iter()
            .filter(|(_, g)| g.name.as_deref() == Some(global))
            .any(|(handle, _)| !self.info[function][handle].is_empty())
    }
}

#[derive(Debug)]
pub struct Report {
    pub stages: Vec<CheckedStage>,
    /// The manifest's `[sky]`.
    pub sky: Option<crate::manifest::Stage>,
    /// The manifest's `[reflection]`.
    pub reflection: Option<crate::manifest::Reflection>,
    /// The manifest's `[shadow]`.
    pub shadow: Option<crate::manifest::Shadow>,
    /// The manifest's `[material] fog`: whether Horizon fogs the material's colour.
    pub material_fog: bool,
    /// The manifest's `[[textures]]`, and each image decoded, in that order.
    pub textures: Vec<(crate::manifest::PackTexture, Image)>,
    /// The manifest the stages were checked against.
    pub manifest: crate::manifest::Manifest,
}

#[derive(Clone, Debug)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// RGBA, 8 bits a channel, rows from the top.
    pub rgba: Vec<u8>,
}

/// Decodes the PNG `bytes` into RGBA8.
pub fn decode_png(bytes: &[u8]) -> Result<Image, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let (width, height) = (reader.info().width, reader.info().height);
    let max = crate::manifest::MAX_TEXTURE_SIZE;
    if width == 0 || height == 0 || width > max || height > max {
        return Err(format!(
            "{width}×{height}; a texture is at most {max}×{max}"
        ));
    }
    let mut buffer = vec![0; reader.output_buffer_size().ok_or("too large")?];
    let frame = reader.next_frame(&mut buffer).map_err(|e| e.to_string())?;
    let data = &buffer[..frame.buffer_size()];
    let pixels = (width * height) as usize;
    let rgba = match frame.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data
            .chunks(3)
            .flat_map(|c| [c[0], c[1], c[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => data
            .chunks(2)
            .flat_map(|c| [c[0], c[0], c[0], c[1]])
            .collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err("indexed colour was not expanded".into()),
    };
    if rgba.len() != pixels * 4 {
        return Err("unexpected pixel data".into());
    }
    Ok(Image {
        width,
        height,
        rgba,
    })
}

/// Checks every stage of `pack`, and that each option is an `override` of
/// its type in some stage, or one `[shadow]` sizes its map by. Returns every problem
/// found, not just the first.
pub fn check(pack: &Pack) -> Result<Report, Vec<Problem>> {
    let m = &pack.manifest;
    let stages = m
        .sky
        .iter()
        .map(|s| (StageKind::Sky, s))
        .chain(m.vertex.iter().map(|s| (StageKind::Vertex, s)))
        .chain(m.material.iter().map(|s| (StageKind::Material, s)))
        .chain(m.weather.iter().map(|s| (StageKind::Weather, s)))
        .map(|(kind, s)| (kind, s, None))
        .chain(
            (m.kept.before_world.iter().enumerate())
                .map(|(i, s)| (StageKind::BeforeWorld, s, Some(i))),
        )
        .chain(m.post.iter().enumerate().map(|(i, s)| match s.compute {
            true => (StageKind::Compute, s, Some(i)),
            // In a pack without kept resources or compute passes an ordinary pass keeps the first
            // entry with its file, as such packs were built before, so they compose
            // byte-identically; with them, each entry's own `reads` decide its declarations.
            false => (StageKind::Post, s, m.computes().then_some(i)),
        }))
        .chain(m.tonemap.iter().map(|s| (StageKind::Tonemap, s, None)))
        .chain(m.entity.iter().map(|s| (StageKind::Entity, s, None)));
    let (mut checked, mut problems) = (Vec::new(), Vec::new());
    for (kind, stage, at) in stages {
        match check_entry(pack, kind, &stage.file, at) {
            Ok(mut c) => {
                c.label = stage.name.clone().unwrap_or_else(|| stage.file.clone());
                checked.push(c);
            }
            Err(p) => problems.extend(p),
        }
    }
    // The options the shadow map's size follows need no `override`: the frame has the size.
    let sizes: Vec<&String> = m
        .shadow
        .iter()
        .flat_map(|s| s.resolution_option.iter().chain(&s.distance_option))
        .collect();
    if problems.is_empty() {
        for option in m.options.iter().filter(|o| !sizes.contains(&&o.id)) {
            let used = checked.iter().any(|c| {
                c.module
                    .overrides
                    .iter()
                    .any(|(_, o)| o.name.as_deref() == Some(&option.id))
            });
            if !used {
                problems.push(problem(
                    crate::MANIFEST,
                    format!(
                        "option {:?}: no stage declares `override {}: {}`",
                        option.id,
                        option.id,
                        option.kind.wgsl_type()
                    ),
                ));
            }
        }
    }
    let mut textures = Vec::new();
    for t in &m.textures {
        match decode_png(pack.source_bytes(&t.file).unwrap_or_default()) {
            Ok(image) => textures.push((t.clone(), image)),
            Err(e) => problems.push(problem(&t.file, e)),
        }
    }
    if problems.is_empty() {
        Ok(Report {
            sky: pack.manifest.sky.clone(),
            stages: checked,
            textures,
            reflection: m.reflection.clone(),
            shadow: m.shadow.clone(),
            material_fog: m.material.as_ref().and_then(|s| s.fog).unwrap_or(true),
            manifest: m.clone(),
        })
    } else {
        Err(problems)
    }
}

/// Builds and checks one stage of `pack` from its file.
pub fn check_stage(pack: &Pack, kind: StageKind, file: &str) -> Result<CheckedStage, Vec<Problem>> {
    check_entry(pack, kind, file, None)
}

/// `check_stage` of the entry `at` of the stage's list (`stages::entry`): two `[[post]]`
/// entries may share a file and differ in the rest.
fn check_entry(
    pack: &Pack,
    kind: StageKind,
    file: &str,
    at: Option<usize>,
) -> Result<CheckedStage, Vec<Problem>> {
    // First without the entry points, so a missing or mistyped stage function
    // is reported as such rather than as a call that doesn't resolve.
    let mut own = compose_traced(pack, kind, file, at, false);
    let entries = own.parts.pop().expect("entries are the last part");
    own.text.truncate(entries.1.start);
    if pack.manifest.ray_tracing.is_some() {
        let problems = ray_queries(&own);
        if !problems.is_empty() {
            return Err(problems);
        }
    }
    // The material prelude's LabPBR functions call into the renderer: a stand-in here.
    if kind == StageKind::Material {
        own.text += crate::stages::PBR_STUB;
    }
    let module = parse(&own).map_err(|mut problems| {
        // A motion guide name the pack didn't ask for: say how to ask.
        let names = ["`scene_motion`", "`motion_at`", "`MotionData`", "`MOTION_"];
        for p in problems
            .iter_mut()
            .filter(|_| pack.manifest.motion.is_none())
        {
            if kind == StageKind::Post && names.iter().any(|n| p.message.contains(n)) {
                p.message += "; the motion guide's names need `[motion]` in the manifest";
            }
        }
        undeclared(problems, &pack.manifest, kind)
    })?;
    own.text.truncate(entries.1.start);
    let mut problems = signature(&module, kind, file);
    if kind == StageKind::Material && pack.manifest.displacement.is_some() {
        problems.extend(function_signature(
            &module,
            "displacement",
            &["DisplacementInput"],
            "f32",
            crate::stages::DISPLACEMENT_SIGNATURE,
            file,
        ));
    }
    problems.extend(api_rules(&module, kind, &own));
    problems.extend(constructions(&module, &own));
    problems.extend(options(&module, &pack.manifest.options, file));
    if !problems.is_empty() {
        return Err(problems);
    }
    let composed = compose_traced(pack, kind, file, at, false);
    let module = parse(&composed)?;
    let info = validate(&module, &composed, Capabilities::default())?;
    let problems = sampled_data(&module, &info, &composed, &pack.manifest);
    if !problems.is_empty() {
        return Err(problems);
    }
    // A stage the scene changes nothing in (a material stage that doesn't trace) is built once.
    let traced =
        Some(compose_traced(pack, kind, file, at, true)).filter(|s| s.text != composed.text);
    let scene = match traced {
        Some(mut composed) => {
            let module = parse(&composed)?;
            let capabilities = Capabilities::default() | Capabilities::RAY_QUERY;
            let info = validate(&module, &composed, capabilities)?;
            if kind == StageKind::Material {
                let problems = vertex_rays(&module, &info, &composed, &pack.manifest);
                if !problems.is_empty() {
                    return Err(problems);
                }
            }
            composed
                .text
                .truncate(composed.parts.last().expect("entries").1.start);
            Some((module, info, composed.text))
        }
        None => None,
    };
    let (expressions, texture_reads) = cost(scene.as_ref().map_or(&module, |s| &s.0));
    let mut problems = Vec::new();
    let engine = match kind {
        StageKind::Material => Some((pack.manifest.material_coverage, "the texture")),
        StageKind::Entity => Some((
            pack.manifest.entity_coverage,
            "the texture and the layer's opacity",
        )),
        _ => None,
    }
    .and_then(|(coverage, test)| (coverage == crate::Coverage::Engine).then_some(test));
    if let Some(test) = engine
        && let Some(at) = reachable_discard(&module, kind.function())
    {
        let function = kind.function();
        problems.push(located(
            &composed,
            at.to_range().map(|r| r.start),
            format!(
                "`discard` reachable from `{function}`: under `[{function}] coverage = \"engine\"` \
                 (the default) Horizon's alpha test on {test} decides what is drawn; a pack that \
                 cuts its own holes sets `coverage = \"pack\"`"
            ),
        ));
    }
    if expressions > MAX_EXPRESSIONS {
        problems.push(problem(
            file,
            format!("{expressions} expressions; a stage may hold {MAX_EXPRESSIONS}"),
        ));
    }
    if texture_reads > MAX_TEXTURE_READS {
        problems.push(problem(
            file,
            format!("{texture_reads} texture reads; a stage may write {MAX_TEXTURE_READS}"),
        ));
    }
    let options = &pack.manifest.options;
    problems.extend(translate(&module, &info, options, file, (2, 2)));
    if let (Some((module, info, _)), true) = (&scene, problems.is_empty()) {
        // Metal has ray queries from 2.4.
        problems.extend(translate(module, info, options, file, (2, 4)));
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    Ok(CheckedStage {
        kind,
        file: file.to_owned(),
        label: file.to_owned(),
        module,
        info,
        source: own.text,
        scene_source: scene.as_ref().map(|s| s.2.clone()),
        scene: scene.map(|s| s.0),
        expressions,
        texture_reads,
    })
}

/// naga's validation of a composed stage, for a device with `capabilities`.
fn validate(
    module: &naga::Module,
    composed: &Composed,
    capabilities: Capabilities,
) -> Result<ModuleInfo, Vec<Problem>> {
    Validator::new(ValidationFlags::all(), capabilities)
        .validate(module)
        .map_err(|e| {
            let at = e.spans().next().and_then(|(span, _)| span.to_range());
            let mut message = e.as_inner().to_string();
            let mut source = std::error::Error::source(e.as_inner());
            while let Some(s) = source {
                message += &format!(": {s}");
                source = s.source();
            }
            vec![located(composed, at.map(|r| r.start), message)]
        })
}

/// The names of naga's ray queries: with `[ray_tracing]` the scene's prelude enables them for
/// itself, and a pack's files that used them would depend on the scene's layout and on wgpu.
const RAY_QUERY_NAMES: [&str; 5] = [
    "ray_query",
    "acceleration_structure",
    "RayDesc",
    "RayIntersection",
    "wgpu_ray_query",
];

/// Where the files of a pack with `[ray_tracing]` name a ray query themselves (a type, a
/// `rayQuery` function or the `enable` directive), each name once a file; comments don't count
/// (WGSL's block comments nest).
fn ray_queries(own: &Composed) -> Vec<Problem> {
    let mut problems = Vec::new();
    for (_, range) in &own.parts[1..] {
        let mut seen: Vec<&str> = Vec::new();
        for (at, name) in identifiers(&own.text[range.clone()]) {
            let query = RAY_QUERY_NAMES.contains(&name) || name.starts_with("rayQuery");
            if query && !seen.contains(&name) {
                seen.push(name);
                problems.push(located(
                    own,
                    Some(range.start + at),
                    format!(
                        "`{name}`: a pack traces rays with `trace` and `trace_layers`; ray queries and the scene are Horizon's"
                    ),
                ));
            }
        }
    }
    problems
}

/// A pass's prelude declares only the kept resources it lists, so naga finds no other: where the
/// name it doesn't resolve is one, `problems` say so.
fn undeclared(mut problems: Vec<Problem>, m: &crate::Manifest, kind: StageKind) -> Vec<Problem> {
    let kept = |n: &str| {
        (m.kept.resources.iter()).any(|r| r.name == n || format!("{}_previous", r.name) == n)
    };
    for p in &mut problems {
        let name = (p
            .message
            .strip_prefix("no definition in scope for identifier: `"))
        .and_then(|n| n.split('`').next())
        .filter(|n| kept(n));
        if let Some(name) = name {
            p.message = match kind {
                StageKind::Post | StageKind::Compute | StageKind::BeforeWorld => {
                    format!(
                        "`{name}` is a kept resource this pass doesn't list in `reads`/`writes`"
                    )
                }
                _ => format!(
                    "`{name}` is a kept resource, which the {} stage can't read yet",
                    kind.name()
                ),
            };
        }
    }
    problems
}

/// Where a pass samples or gathers a 32-bit kept texture (`r32float`, `rgba32float`; an
/// `rgba32uint` one, `texture_2d<u32>`, can't be sampled at all), this
/// frame's or `_previous`, itself or through a function's argument: the API reads them with
/// `textureLoad` only, unfiltered, which every device does.
fn sampled_data(
    module: &naga::Module,
    info: &ModuleInfo,
    composed: &Composed,
    m: &crate::Manifest,
) -> Vec<Problem> {
    use crate::kept::{Format, Shape};
    let data = |name: &str| {
        m.kept.resources.iter().any(|r| {
            let wide = match r.shape {
                Shape::Fixed { format, .. } | Shape::Scaled { format, .. } => {
                    format != Format::Rgba16Float
                }
                Shape::Buffer { .. } => false,
            };
            wide && (r.name == name || format!("{}_previous", r.name) == name)
        })
    };
    let mut names: Vec<&str> = (0..module.entry_points.len())
        .flat_map(|i| &info.get_entry_point(i).sampling_set)
        .filter_map(|key| module.global_variables[key.image].name.as_deref())
        .filter(|n| data(n))
        .collect();
    names.sort();
    names.dedup();
    let first = |name: &str| {
        (composed.parts[1..composed.parts.len() - 1].iter()).find_map(|(_, range)| {
            let at = identifiers(&composed.text[range.clone()])
                .into_iter()
                .find(|w| w.1 == name)?;
            Some(range.start + at.0)
        })
    };
    (names.into_iter())
        .map(|name| {
            let message = format!(
                "`{name}` is a 32-bit kept texture: read it with `textureLoad`, it isn't sampled \
                 or gathered"
            );
            located(composed, first(name), message)
        })
        .collect()
}

/// Where a material stage built against the scene reaches it from a function that runs in a
/// vertex shader: `displacement`, and the vertex stage's `vertex`, which shares its module. The
/// renderer binds the scene for the fragment stage only.
fn vertex_rays(
    module: &naga::Module,
    info: &ModuleInfo,
    composed: &Composed,
    manifest: &crate::manifest::Manifest,
) -> Vec<Problem> {
    // The entity layer's bindings come last.
    let scene: Vec<_> = (module.global_variables.iter())
        .filter(|(_, g)| {
            g.binding.as_ref().is_some_and(|b| {
                let bindings = RAY_SCENE_BINDING..=RAY_ENTITY_RETAINED_BINDING;
                b.group == GROUP && bindings.contains(&b.binding)
            })
        })
        .map(|(handle, _)| handle)
        .collect();
    let vertex = [
        ("vertex", manifest.vertex.is_some()),
        ("displacement", manifest.displacement.is_some()),
    ];
    let mut problems = Vec::new();
    for (handle, f) in module.functions.iter() {
        let name = f.name.as_deref().unwrap_or_default();
        if vertex.contains(&(name, true)) && scene.iter().any(|&g| !info[handle][g].is_empty()) {
            let at = module
                .functions
                .get_span(handle)
                .to_range()
                .map(|r| r.start);
            problems.push(located(
                composed,
                at,
                format!(
                    "`{name}` reaches a trace function: it runs in a vertex shader, and only `material` traces rays"
                ),
            ));
        }
    }
    problems
}

fn located(composed: &Composed, offset: Option<usize>, message: String) -> Problem {
    match offset {
        Some(at) => {
            let (file, line, column) = composed.locate(at);
            Problem {
                file: file.to_owned(),
                line,
                column,
                message,
            }
        }
        None => problem(&composed.parts[composed.parts.len() - 2].0, message),
    }
}

fn parse(composed: &Composed) -> Result<naga::Module, Vec<Problem>> {
    naga::front::wgsl::parse_str(&composed.text).map_err(|e| {
        let label = e.labels().next();
        let mut message = e.message().to_owned();
        if let Some((_, text)) = label.filter(|(_, t)| !t.is_empty() && !message.contains(*t)) {
            message += &format!(" ({text})");
        }
        let at = label.and_then(|(span, _)| span.to_range()).map(|r| r.start);
        vec![located(composed, at, message)]
    })
}

fn type_name(module: &naga::Module, ty: naga::Handle<naga::Type>) -> String {
    let t = &module.types[ty];
    if let Some(name) = &t.name {
        return name.clone();
    }
    match t.inner {
        naga::TypeInner::Scalar(s) => scalar_name(s).to_owned(),
        naga::TypeInner::Vector { size, scalar } => {
            format!("vec{}<{}>", size as u8, scalar_name(scalar))
        }
        _ => format!("{:?}", t.inner),
    }
}

fn scalar_name(s: naga::Scalar) -> &'static str {
    match (s.kind, s.width) {
        (naga::ScalarKind::Bool, _) => "bool",
        (naga::ScalarKind::Float, 4) => "f32",
        (naga::ScalarKind::Sint, 4) => "i32",
        (naga::ScalarKind::Uint, 4) => "u32",
        _ => "another scalar",
    }
}

fn signature(module: &naga::Module, kind: StageKind, file: &str) -> Vec<Problem> {
    let found = module
        .functions
        .iter()
        .find(|(_, f)| f.name.as_deref() == Some(kind.function()));
    let Some((_, f)) = found else {
        return vec![problem(file, format!("defines no `{}`", kind.signature()))];
    };
    let args: Vec<String> = f
        .arguments
        .iter()
        .map(|a| type_name(module, a.ty))
        .collect();
    let result = f.result.as_ref().map(|r| type_name(module, r.ty));
    // A compute stage's function returns nothing: its output is "".
    if args == [kind.input().name] && result.as_deref().unwrap_or_default() == kind.output() {
        return Vec::new();
    }
    let result = result.map_or(String::new(), |r| format!(" -> {r}"));
    vec![problem(
        file,
        format!(
            "`fn {}({}){result}` should be `{}`",
            kind.function(),
            args.join(", "),
            kind.signature()
        ),
    )]
}

/// The module defines `fn <name>(<args>) -> <output>`, written out as `expected`.
fn function_signature(
    module: &naga::Module,
    name: &str,
    args_expected: &[&str],
    output: &str,
    expected: &str,
    file: &str,
) -> Vec<Problem> {
    let found = module
        .functions
        .iter()
        .find(|(_, f)| f.name.as_deref() == Some(name));
    let Some((_, f)) = found else {
        return vec![problem(file, format!("defines no `{expected}`"))];
    };
    let args: Vec<String> = f
        .arguments
        .iter()
        .map(|a| type_name(module, a.ty))
        .collect();
    let result = f.result.as_ref().map(|r| type_name(module, r.ty));
    if args == args_expected && result.as_deref() == Some(output) {
        return Vec::new();
    }
    vec![problem(
        file,
        format!(
            "`fn {name}({}) -> {}` should be `{expected}`",
            args.join(", "),
            result.as_deref().unwrap_or("()"),
        ),
    )]
}

/// The names of everything a module defines at its top level.
fn names(module: &naga::Module) -> HashSet<String> {
    let types = module.types.iter().filter_map(|(_, t)| t.name.clone());
    let functions = module.functions.iter().filter_map(|(_, f)| f.name.clone());
    let globals = module
        .global_variables
        .iter()
        .filter_map(|(_, g)| g.name.clone());
    let constants = module.constants.iter().filter_map(|(_, c)| c.name.clone());
    let overrides = module.overrides.iter().filter_map(|(_, o)| o.name.clone());
    types
        .chain(functions)
        .chain(globals)
        .chain(constants)
        .chain(overrides)
        .collect()
}

/// A pack's files declare no bindings, no entry points and no `horizon_`
/// names: what the prelude doesn't define is the pack's.
fn api_rules(module: &naga::Module, kind: StageKind, own: &Composed) -> Vec<Problem> {
    // The prelude as composed: the stage's own, with the pack's `[[textures]]`. The material
    // prelude's LabPBR functions call into the renderer: its stand-in is ours too.
    let mut text = own.text[own.parts[0].1.clone()].to_owned();
    if kind == StageKind::Material {
        text += crate::stages::PBR_STUB;
    }
    let prelude = naga::front::wgsl::parse_str(&text).expect("the prelude parses");
    let ours = names(&prelude);
    let file = &own.parts[own.parts.len() - 1].0;
    let mut problems = Vec::new();
    for e in &module.entry_points {
        problems.push(problem(
            file,
            format!(
                "declares the entry point `{}`: a stage is a function, Horizon adds the entry points",
                e.name
            ),
        ));
    }
    for (_, g) in module.global_variables.iter() {
        let name = g.name.clone().unwrap_or_default();
        if g.binding.is_some() && !ours.contains(&name) {
            problems.push(problem(
                file,
                format!(
                    "declares the binding `{name}`: a stage reads only the resources of its prelude"
                ),
            ));
        }
    }
    let mut theirs: Vec<String> = names(module)
        .into_iter()
        .filter(|n| !ours.contains(n) && n.starts_with("horizon_"))
        .collect();
    theirs.sort();
    for name in theirs {
        problems.push(problem(
            file,
            format!("`{name}`: names starting with horizon_ are Horizon's"),
        ));
    }
    problems
}

/// The pack's files construct none of the structs Horizon hands a stage (`Frame`, the
/// stage inputs, the post helpers' `SurfaceData` and `RayHit`). They may gain fields, which would break a
/// positional `PostInput(...)`; reading them and passing them along stay as they are. `Surface`,
/// which `material` returns, is built.
fn constructions(module: &naga::Module, own: &Composed) -> Vec<Problem> {
    let provided: Vec<&str> = StageKind::ALL
        .iter()
        .map(|k| k.input().name)
        .chain([
            Frame::WGSL.name,
            DISPLACEMENT_INPUT.name,
            COMPUTE_INPUT.name,
            "SurfaceData",
            RAY_HIT.name,
            "MotionData",
        ])
        .collect();
    let arenas = module
        .functions
        .iter()
        .map(|(_, f)| &f.expressions)
        .chain([&module.global_expressions]);
    let in_pack = |at: Option<usize>| {
        at.is_some_and(|at| own.parts[1..].iter().any(|(_, r)| r.contains(&at)))
    };
    let mut problems = Vec::new();
    for arena in arenas {
        for (handle, e) in arena.iter() {
            let naga::Expression::Compose { ty, .. } = *e else {
                continue;
            };
            let Some(name) = module.types[ty].name.as_deref() else {
                continue;
            };
            let at = arena.get_span(handle).to_range().map(|r| r.start);
            // Built in the pack's files (the prelude's helpers aren't the pack's), as a struct the
            // prelude declares (a pack's own struct of the same name is its own).
            let horizon_s = (module.types.get_span(ty).to_range())
                .is_some_and(|r| own.parts[0].1.contains(&r.start));
            if provided.contains(&name) && horizon_s && in_pack(at) {
                problems.push(located(
                    own,
                    at,
                    format!("constructs `{name}`: it is Horizon's and may gain fields; read it, don't construct it"),
                ));
            }
        }
    }
    problems
}

/// Where a `discard` is that the function `name` can reach: in its own body, its nested blocks,
/// or a function it calls, however deep.
fn reachable_discard(module: &naga::Module, name: &str) -> Option<naga::Span> {
    fn walk(
        block: &naga::Block,
        calls: &mut Vec<naga::Handle<naga::Function>>,
    ) -> Option<naga::Span> {
        for (statement, span) in block.span_iter() {
            let found = match statement {
                naga::Statement::Kill => return Some(*span),
                naga::Statement::Block(b) => walk(b, calls),
                naga::Statement::If { accept, reject, .. } => {
                    walk(accept, calls).or_else(|| walk(reject, calls))
                }
                naga::Statement::Switch { cases, .. } => {
                    cases.iter().find_map(|c| walk(&c.body, calls))
                }
                naga::Statement::Loop {
                    body, continuing, ..
                } => walk(body, calls).or_else(|| walk(continuing, calls)),
                naga::Statement::Call { function, .. } => {
                    calls.push(*function);
                    None
                }
                _ => None,
            };
            if found.is_some() {
                return found;
            }
        }
        None
    }
    let (start, _) = module
        .functions
        .iter()
        .find(|(_, f)| f.name.as_deref() == Some(name))?;
    let (mut calls, mut seen) = (vec![start], HashSet::new());
    while let Some(f) = calls.pop() {
        if seen.insert(f)
            && let Some(span) = walk(&module.functions[f].body, &mut calls)
        {
            return Some(span);
        }
    }
    None
}

/// Each `override` is an option of its type, or has a default of its own.
fn options(module: &naga::Module, options: &[PackOption], file: &str) -> Vec<Problem> {
    let mut problems = Vec::new();
    for (_, o) in module.overrides.iter() {
        let name = o.name.as_deref().unwrap_or("?");
        let ty = type_name(module, o.ty);
        match options.iter().find(|opt| opt.id == name) {
            Some(opt) if ty != opt.kind.wgsl_type() => problems.push(problem(
                file,
                format!(
                    "`override {name}: {ty}`: option {name:?} is a {}, so its override is `{}`",
                    kind_name(&opt.kind),
                    opt.kind.wgsl_type()
                ),
            )),
            None if o.init.is_none() => problems.push(problem(
                file,
                format!("`override {name}` has no default and no [[options]] entry"),
            )),
            _ => {}
        }
    }
    problems
}

fn kind_name(kind: &OptionKind) -> &'static str {
    match kind {
        OptionKind::Bool { .. } => "bool",
        OptionKind::Float { .. } => "float",
        OptionKind::Int { .. } => "int",
        OptionKind::Choice { .. } => "choice",
    }
}

fn cost(module: &naga::Module) -> (usize, usize) {
    let functions = module
        .functions
        .iter()
        .map(|(_, f)| f)
        .chain(module.entry_points.iter().map(|e| &e.function));
    let (mut expressions, mut reads) = (0, 0);
    for f in functions {
        expressions += f.expressions.len();
        reads += f
            .expressions
            .iter()
            .filter(|(_, e)| {
                matches!(
                    e,
                    naga::Expression::ImageSample { .. } | naga::Expression::ImageLoad { .. }
                )
            })
            .count();
    }
    (expressions, reads)
}

/// The option values a pack is translated with: its defaults, then every
/// option at its minimum, then at its maximum.
fn option_sets(options: &[PackOption]) -> [(&'static str, Vec<(String, f64)>); 3] {
    let at = |pick: fn(&OptionKind) -> f64| {
        options
            .iter()
            .map(|o| (o.id.clone(), pick(&o.kind)))
            .collect()
    };
    [
        ("defaults", at(OptionKind::default_constant)),
        (
            "minimums",
            at(|k| match *k {
                OptionKind::Bool { .. } | OptionKind::Choice { .. } => 0.0,
                OptionKind::Float { min, .. } => f64::from(min),
                OptionKind::Int { min, .. } => f64::from(min),
            }),
        ),
        (
            "maximums",
            at(|k| match k {
                OptionKind::Bool { .. } => 1.0,
                OptionKind::Choice { values, .. } => (values.len() - 1) as f64,
                OptionKind::Float { max, .. } => f64::from(*max),
                OptionKind::Int { max, .. } => f64::from(*max),
            }),
        ),
    ]
}

/// Translates every entry point for Vulkan (SPIR-V), Metal and DirectX 12
/// (HLSL) with each option set, as wgpu does when it builds the pipelines; Metal's shading
/// language at the version `metal`.
fn translate(
    module: &naga::Module,
    info: &ModuleInfo,
    options: &[PackOption],
    file: &str,
    metal: (u8, u8),
) -> Vec<Problem> {
    let present: HashSet<&str> = module
        .overrides
        .iter()
        .filter_map(|(_, o)| o.name.as_deref())
        .collect();
    let mut problems = Vec::new();
    for (set, values) in option_sets(options) {
        let mut constants = naga::back::PipelineConstants::default();
        for (id, value) in values
            .into_iter()
            .filter(|(id, _)| present.contains(id.as_str()))
        {
            constants.insert(id, value);
        }
        for e in &module.entry_points {
            let fail = |backend: &str, err: String| {
                problem(
                    file,
                    format!("with the options at their {set}, {backend} refuses it: {err}"),
                )
            };
            let processed = naga::back::pipeline_constants::process_overrides(
                module,
                info,
                Some((e.stage, &e.name)),
                &constants,
            );
            let (m, i) = match processed {
                Ok(p) => p,
                Err(err) => {
                    problems.push(fail("the override step", err.to_string()));
                    return problems;
                }
            };
            let spv = naga::back::spv::write_vec(
                &m,
                &i,
                &naga::back::spv::Options::default(),
                Some(&naga::back::spv::PipelineOptions {
                    shader_stage: e.stage,
                    entry_point: e.name.clone(),
                }),
            );
            let msl = naga::back::msl::write_string(
                &m,
                &i,
                &naga::back::msl::Options {
                    lang_version: metal,
                    ..Default::default()
                },
                &naga::back::msl::PipelineOptions {
                    entry_point: Some((e.stage, e.name.clone())),
                    ..Default::default()
                },
            );
            let mut hlsl = String::new();
            let hlsl_result = naga::back::hlsl::Writer::new(
                &mut hlsl,
                &naga::back::hlsl::Options::default(),
                &naga::back::hlsl::PipelineOptions {
                    entry_point: Some((e.stage, e.name.clone())),
                },
            )
            .write(&m, &i, None);
            let errors = [
                ("Vulkan (SPIR-V)", spv.err().map(|e| e.to_string())),
                ("Metal", msl.err().map(|e| e.to_string())),
                (
                    "DirectX 12 (HLSL)",
                    hlsl_result.err().map(|e| e.to_string()),
                ),
            ];
            for (backend, err) in errors {
                if let Some(err) = err {
                    problems.push(fail(backend, err));
                }
            }
            if !problems.is_empty() {
                return problems;
            }
        }
    }
    problems
}
