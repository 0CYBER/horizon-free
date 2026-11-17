//! Shader API 0.2's kept resources and compute passes (#16011 item 4): `[kept]`, `[[resource]]`,
//! `[[before_world]]`, `[[post]] compute = true` and options' `resets`, which the game runs.
use crate::manifest::{POST_SCALES, Stage};
use serde::Deserialize;

/// `[[resource]]` entries a pack may declare.
pub const MAX_RESOURCES: usize = 8;
/// `[[before_world]]` entries a pack may declare.
pub const MAX_BEFORE_WORLD: usize = 4;
/// `[[post]]` entries with `compute = true` a pack may declare.
pub const MAX_COMPUTE_POST: usize = 8;
/// The bytes of every buffer of a pack together, per view.
pub const MAX_BUFFER_BYTES: u64 = 32 << 20;
/// The bytes of every fixed-size kept texture of a pack together, both versions counted: as
/// much as its buffers, so a trail, a cache or a lookup table can be a texture, not a buffer.
pub const MAX_FIXED_TEXTURE_BYTES: u64 = 32 << 20;
/// A workgroup's largest size along each axis, and its invocations: wgpu's default limits,
/// which every device with compute has.
pub const MAX_WORKGROUP: [u32; 3] = [256, 256, 64];
pub const MAX_WORKGROUP_INVOCATIONS: u32 = 256;
/// The workgroups a fixed dispatch may need along an axis (wgpu's default limit).
pub const MAX_WORKGROUPS: u32 = 65535;
/// The invocations of one dispatch, and of every compute pass of a frame (per view) together,
/// counted as whole workgroups (`invocations`).
pub const MAX_DISPATCH_INVOCATIONS: u64 = 1 << 25;
pub const MAX_FRAME_INVOCATIONS: u64 = 1 << 26;
/// The size a `"pixels"` dispatch counts as: R isn't known at load, and such a dispatch scales
/// with R as a fragment pass does, so the caps measure the pack, not the window.
pub const PIXELS_COUNTED_AS: [u32; 2] = [3840, 2160];

/// What clears a kept resource (docs/api/shaders-reference-0.2.md, "Kept resources").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keep {
    /// Screen history: cleared on every camera cut, resize and screen-history reset.
    View,
    /// A world cache: survives resizes and cuts within `[kept] reach`.
    World,
}

/// `frame.reset_reasons`' bits: why `view_epoch` or `world_epoch` changed this frame
/// (docs/api/shaders-reference-0.2.md, "Kept resources", says which and when).
pub const REASON_LOAD: u32 = 1;
pub const REASON_RESET: u32 = 1 << 1;
pub const REASON_WORLD: u32 = 1 << 2;
pub const REASON_TELEPORT: u32 = 1 << 3;
pub const REASON_PAUSE: u32 = 1 << 4;
pub const REASON_SIZE: u32 = 1 << 5;
pub const REASON_DISPLAY: u32 = 1 << 6;
pub const REASON_OPTION: u32 = 1 << 7;

/// The reasons by name: a pack with `[[resource]]` has each as a WGSL `const` (`reasons_wgsl`).
pub const REASONS: [(&str, u32); 8] = [
    ("REASON_LOAD", REASON_LOAD),
    ("REASON_RESET", REASON_RESET),
    ("REASON_WORLD", REASON_WORLD),
    ("REASON_TELEPORT", REASON_TELEPORT),
    ("REASON_PAUSE", REASON_PAUSE),
    ("REASON_SIZE", REASON_SIZE),
    ("REASON_DISPLAY", REASON_DISPLAY),
    ("REASON_OPTION", REASON_OPTION),
];

/// `REASONS` as the WGSL constants a kept pack's prelude declares.
pub(crate) fn reasons_wgsl() -> String {
    let consts = REASONS
        .iter()
        .map(|(name, bit)| format!("const {name}: u32 = {bit}u;\n"));
    consts.collect()
}

/// A kept buffer's element type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Element {
    U32,
    AtomicU32,
    F32,
    Vec4U32,
    Vec4F32,
}

impl Element {
    pub const ALL: [Element; 5] = [
        Element::U32,
        Element::AtomicU32,
        Element::F32,
        Element::Vec4U32,
        Element::Vec4F32,
    ];

    /// Its WGSL type, as the manifest writes it.
    pub const fn wgsl(self) -> &'static str {
        match self {
            Element::U32 => "u32",
            Element::AtomicU32 => "atomic<u32>",
            Element::F32 => "f32",
            Element::Vec4U32 => "vec4<u32>",
            Element::Vec4F32 => "vec4<f32>",
        }
    }

    pub const fn bytes(self) -> u64 {
        match self {
            Element::U32 | Element::AtomicU32 | Element::F32 => 4,
            Element::Vec4U32 | Element::Vec4F32 => 16,
        }
    }
}

/// A kept texture's format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Rgba16Float,
    R32Float,
    Rgba32Float,
    /// Integers, read as `texture_2d<u32>`; only a compute pass writes one.
    Rgba32Uint,
    /// One integer a texel, as `Rgba32Uint` otherwise.
    R32Uint,
}

impl Format {
    pub const ALL: [Format; 5] = [
        Format::Rgba16Float,
        Format::R32Float,
        Format::Rgba32Float,
        Format::Rgba32Uint,
        Format::R32Uint,
    ];

    /// Its WGSL texel format, as the manifest writes it.
    pub const fn wgsl(self) -> &'static str {
        match self {
            Format::Rgba16Float => "rgba16float",
            Format::R32Float => "r32float",
            Format::Rgba32Float => "rgba32float",
            Format::Rgba32Uint => "rgba32uint",
            Format::R32Uint => "r32uint",
        }
    }

    /// Whether it holds integers: read as `texture_2d<u32>` with `textureLoad`, written only by
    /// a compute pass (a pass draws floats).
    pub const fn is_uint(self) -> bool {
        matches!(self, Format::Rgba32Uint | Format::R32Uint)
    }

    /// The scalar a read of it gives (`texture_2d<_>`).
    pub const fn texel(self) -> &'static str {
        if self.is_uint() { "u32" } else { "f32" }
    }

    pub const fn bytes(self) -> u64 {
        match self {
            Format::R32Float | Format::R32Uint => 4,
            Format::Rgba16Float => 8,
            Format::Rgba32Float | Format::Rgba32Uint => 16,
        }
    }
}

/// A kept resource's shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    /// A storage buffer of `bytes`, one version.
    Buffer { bytes: u64, element: Element },
    /// A texture of fixed texels, two versions.
    Fixed { format: Format, size: [u32; 2] },
    /// A texture at `scale` of the render size R, two versions.
    Scaled { format: Format, scale: f32 },
}

/// `[[resource]]`: storage the pack keeps from frame to frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Resource {
    /// A free WGSL name; a texture's last-frame version is `<name>_previous`.
    pub name: String,
    pub shape: Shape,
    pub keep: Keep,
}

impl Resource {
    pub fn is_texture(&self) -> bool {
        !matches!(self.shape, Shape::Buffer { .. })
    }

    /// A texture's format; none for a buffer.
    pub fn format(&self) -> Option<Format> {
        match self.shape {
            Shape::Buffer { .. } => None,
            Shape::Fixed { format, .. } | Shape::Scaled { format, .. } => Some(format),
        }
    }
}

/// `dispatch`: how many invocations a compute entry runs.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(try_from = "toml::Value")]
pub enum Dispatch {
    /// `"pixels"`: one per texel of the render size R.
    Pixels,
    /// `[x, y, z]` cells.
    Cells([u32; 3]),
}

impl TryFrom<toml::Value> for Dispatch {
    type Error = String;

    fn try_from(v: toml::Value) -> Result<Self, String> {
        match v {
            toml::Value::String(s) if s == "pixels" => Ok(Dispatch::Pixels),
            v => v
                .try_into()
                .map(Dispatch::Cells)
                .map_err(|_| "dispatch is \"pixels\" or [x, y, z] cells".into()),
        }
    }
}

/// The pack's kept resources, the passes before the world, and the options that clear them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Kept {
    /// `[kept] reach`: a teleport further than this, in blocks, clears the world resources.
    pub reach: Option<u32>,
    pub resources: Vec<Resource>,
    /// `[[before_world]]`: compute passes after the ray scene and shadow inputs are prepared,
    /// before the world draws, in order.
    pub before_world: Vec<Stage>,
    /// Options whose change clears kept resources (`resets`), by id.
    pub resets: Vec<(String, Keep)>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawKept {
    reach: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawResource {
    name: String,
    buffer: Option<u64>,
    element: Option<String>,
    format: Option<String>,
    size: Option<[u32; 2]>,
    scale: Option<f32>,
    keep: String,
}

fn keep(value: &str) -> Result<Keep, String> {
    match value {
        "view" => Ok(Keep::View),
        "world" => Ok(Keep::World),
        k => Err(format!("keep {k:?}: view or world")),
    }
}

fn resource(r: RawResource, ray_tracing: bool, motion: bool) -> Result<Resource, String> {
    let name = &r.name;
    let previous = format!("{name}_previous");
    // WGSL's own functions and types too: a global by such a name hides them from the prelude.
    let builtin = |n: &str| naga::keywords::wgsl::BUILTIN_IDENTIFIERS.contains(&n);
    let free = |n: &str| !crate::manifest::taken(n, ray_tracing, motion) && !builtin(n);
    if !crate::manifest::target_name(name) || !free(name) || !free(&previous) {
        return Err(format!(
            "resource {name:?} is not a free WGSL name ([a-z][a-z0-9_]*, at most 32, and \
             {previous:?} free too)"
        ));
    }
    // `compute` is the function a compute file defines.
    if name == "compute" {
        return Err("resource \"compute\": the name is the compute function's".into());
    }
    let fail = |e: String| format!("resource {name:?}: {e}");
    let keep = keep(&r.keep).map_err(fail)?;
    let shape = match (r.buffer, &r.format) {
        (Some(bytes), None) => {
            if r.size.is_some() || r.scale.is_some() {
                return Err(fail("a buffer has no `size` or `scale`".into()));
            }
            let element = r
                .element
                .as_deref()
                .ok_or_else(|| fail("needs an `element`".into()))?;
            let Some(element) = Element::ALL.into_iter().find(|e| e.wgsl() == element) else {
                return Err(fail(format!(
                    "element {element:?}: u32, atomic<u32>, f32, vec4<u32> or vec4<f32>"
                )));
            };
            if bytes == 0 || bytes % element.bytes() != 0 {
                return Err(fail(format!(
                    "buffer = {bytes}: a whole number of {element}s, at least one",
                    element = element.wgsl()
                )));
            }
            Shape::Buffer { bytes, element }
        }
        (None, Some(format)) => {
            if r.element.is_some() {
                return Err(fail("`element` is a buffer's".into()));
            }
            let Some(format) = Format::ALL.into_iter().find(|f| f.wgsl() == format) else {
                return Err(fail(format!(
                    "format {format:?}: rgba16float, r32float, rgba32float, rgba32uint or r32uint"
                )));
            };
            match (r.size, r.scale) {
                (Some(size), None) if size.contains(&0) => {
                    return Err(fail(format!("size {size:?}: at least 1 texel a side")));
                }
                (Some(size), None) => Shape::Fixed { format, size },
                (None, Some(_)) if keep == Keep::World => {
                    return Err(fail(
                        "keep = \"world\" with `scale`: a world resource has a fixed `size`, \
                         since it has nothing to do with the window"
                            .into(),
                    ));
                }
                (None, Some(scale)) if POST_SCALES.contains(&scale) => {
                    Shape::Scaled { format, scale }
                }
                (None, Some(scale)) => {
                    return Err(fail(format!("scale {scale} is not one of {POST_SCALES:?}")));
                }
                _ => return Err(fail("a texture has a `size` or a `scale`".into())),
            }
        }
        _ => return Err(fail("needs either `buffer` (bytes) or `format`".into())),
    };
    Ok(Resource {
        name: r.name,
        shape,
        keep,
    })
}

/// The invocations a compute entry of a valid shape runs, as whole workgroups (every invocation
/// of a partly filled one runs `compute`), its `"pixels"` at `PIXELS_COUNTED_AS`.
pub fn invocations(s: &Stage) -> u64 {
    let (Some(workgroup), Some(dispatch)) = (s.workgroup, s.dispatch) else {
        return 0;
    };
    let [w, h] = PIXELS_COUNTED_AS;
    let cells = match dispatch {
        Dispatch::Pixels => [w, h, 1],
        Dispatch::Cells(cells) => cells,
    };
    (cells.iter().zip(workgroup))
        .map(|(c, w)| u64::from(c.div_ceil(w)) * u64::from(w))
        .product()
}

/// The frame's compute entries `computes` (`[[before_world]]` and compute `[[post]]`), together
/// within `MAX_FRAME_INVOCATIONS`.
fn frame_invocations<'a>(computes: impl Iterator<Item = &'a Stage>) -> Result<(), String> {
    let total: u64 = computes.map(invocations).sum();
    if total > MAX_FRAME_INVOCATIONS {
        return Err(format!(
            "the compute passes run {total} invocations a frame (whole workgroups, \"pixels\" \
             counted at 3840×2160); a pack may run {MAX_FRAME_INVOCATIONS}"
        ));
    }
    Ok(())
}

/// A compute entry's `workgroup` and `dispatch`.
fn compute_shape(s: &Stage) -> Result<(), String> {
    let (Some(workgroup), Some(dispatch)) = (s.workgroup, s.dispatch) else {
        return Err(format!(
            "{}: a compute pass needs `dispatch` and `workgroup`",
            s.file
        ));
    };
    let fits = workgroup
        .iter()
        .zip(MAX_WORKGROUP)
        .all(|(w, max)| (1..=max).contains(w));
    if !fits || workgroup.iter().product::<u32>() > MAX_WORKGROUP_INVOCATIONS {
        return Err(format!(
            "{}: workgroup {workgroup:?}: 1 to {MAX_WORKGROUP:?} a side, at most \
             {MAX_WORKGROUP_INVOCATIONS} invocations",
            s.file
        ));
    }
    if let Dispatch::Cells(cells) = dispatch {
        let groups = cells.iter().zip(workgroup).map(|(c, w)| c.div_ceil(w));
        if cells.contains(&0) || groups.clone().any(|g| g > MAX_WORKGROUPS) {
            return Err(format!(
                "{}: dispatch {cells:?}: at least 1 cell a side, at most {MAX_WORKGROUPS} \
                 workgroups a side",
                s.file
            ));
        }
    }
    let n = invocations(s);
    if n > MAX_DISPATCH_INVOCATIONS {
        return Err(format!(
            "{}: {n} invocations (whole workgroups, \"pixels\" counted at 3840×2160); a \
             dispatch may run {MAX_DISPATCH_INVOCATIONS}",
            s.file
        ));
    }
    Ok(())
}

/// `reads`, `writes` and `output` of one pass name declared resources: `x` or a texture's
/// `x_previous` read, `x` written once, and nothing both the pass's attachment and read, nor a
/// texture a compute pass writes read back.
fn access(s: &Stage, resources: &[Resource]) -> Result<(), String> {
    let find = |n: &str| resources.iter().find(|r| r.name == n);
    for (i, read) in s.reads.iter().enumerate() {
        let previous = read.strip_suffix("_previous").and_then(find);
        if find(read).is_none() && !previous.is_some_and(Resource::is_texture) {
            return Err(format!(
                "{}: reads {read:?}, no [[resource]] or texture's _previous",
                s.file
            ));
        }
        if s.reads[..i].contains(read) {
            return Err(format!("{}: reads {read:?} twice", s.file));
        }
    }
    for (i, write) in s.writes.iter().enumerate() {
        let Some(r) = find(write) else {
            return Err(format!("{}: writes {write:?}, no [[resource]]", s.file));
        };
        if s.writes[..i].contains(write) {
            return Err(format!(
                "{}: writes {write:?} twice: one writer a pass",
                s.file
            ));
        }
        if r.is_texture() && s.reads.contains(write) {
            return Err(format!(
                "{}: writes {write:?} and reads it back: read {write}_previous, or a buffer",
                s.file
            ));
        }
    }
    if let Some(output) = s.output.as_ref().filter(|o| find(o).is_some())
        && (s.reads.contains(output) || s.writes.contains(output))
    {
        return Err(format!(
            "{}: {output:?} is its output: a pass's attachment is no texture it reads, and \
             the output is its write",
            s.file
        ));
    }
    Ok(())
}

/// The keys `[[post]]` gains: on a compute entry `dispatch`, `workgroup` and `writes`, no
/// `output` or `scale`, never last; elsewhere none of the three.
fn post_entries(post: &[Stage]) -> Result<(), String> {
    let computes = post.iter().filter(|p| p.compute).count();
    if computes > MAX_COMPUTE_POST {
        return Err(format!(
            "{computes} compute [[post]] entries; a pack may have {MAX_COMPUTE_POST}"
        ));
    }
    for (i, p) in post.iter().enumerate() {
        if !p.compute {
            if p.dispatch.is_some() || p.workgroup.is_some() || !p.writes.is_empty() {
                return Err(format!(
                    "{}: `dispatch`, `workgroup` and `writes` are a compute pass's: an \
                     ordinary pass writes its `output`",
                    p.file
                ));
            }
            continue;
        }
        if p.output.is_some() || p.scale.is_some() {
            return Err(format!(
                "{}: a compute pass has no `output` or `scale`",
                p.file
            ));
        }
        if i + 1 == post.len() {
            return Err(format!(
                "{}: a compute pass can't be the last [[post]] entry: that one draws the picture",
                p.file
            ));
        }
        compute_shape(p)?;
    }
    Ok(())
}

/// Every rule of the keys above but the targets `check_post_targets` counts.
pub(crate) fn check(
    kept: Option<RawKept>,
    raw: Vec<RawResource>,
    before_world: Vec<Stage>,
    post: &[Stage],
    ray_tracing: bool,
    motion: bool,
    resets: Vec<(String, String)>,
) -> Result<Kept, String> {
    if raw.len() > MAX_RESOURCES {
        return Err(format!(
            "{} [[resource]] entries; a pack may declare {MAX_RESOURCES}",
            raw.len()
        ));
    }
    let mut resources: Vec<Resource> = Vec::new();
    for r in raw {
        let r = resource(r, ray_tracing, motion)?;
        let clash = |o: &Resource| {
            [&o.name, &format!("{}_previous", o.name)].contains(&&r.name)
                || format!("{}_previous", r.name) == o.name
        };
        if resources.iter().any(clash) {
            return Err(format!(
                "resource {:?} is declared twice, or as another's _previous",
                r.name
            ));
        }
        resources.push(r);
    }
    // The totals, `None` past `u64`: a manifest's sizes are the pack's, and may be any.
    let total = |pick: fn(&Shape) -> Option<u64>| {
        (resources.iter()).try_fold(0u64, |sum, r| sum.checked_add(pick(&r.shape)?))
    };
    let shown = |bytes: Option<u64>| bytes.map_or("over 2^64".to_owned(), |b| b.to_string());
    let buffers = total(|s| match *s {
        Shape::Buffer { bytes, .. } => Some(bytes),
        _ => Some(0),
    });
    if buffers.is_none_or(|b| b > MAX_BUFFER_BYTES) {
        return Err(format!(
            "[[resource]] buffers take {} bytes; a pack may keep {MAX_BUFFER_BYTES}",
            shown(buffers)
        ));
    }
    let fixed = total(|s| match *s {
        Shape::Fixed { format, size } => (2 * format.bytes())
            .checked_mul(u64::from(size[0]))?
            .checked_mul(u64::from(size[1])),
        _ => Some(0),
    });
    if fixed.is_none_or(|b| b > MAX_FIXED_TEXTURE_BYTES) {
        return Err(format!(
            "fixed-size [[resource]] textures take {} bytes with both versions; a pack may \
             keep {MAX_FIXED_TEXTURE_BYTES}",
            shown(fixed)
        ));
    }
    let world = resources.iter().any(|r| r.keep == Keep::World);
    let reach = match kept {
        Some(k) if !world => {
            return Err(format!(
                "[kept] reach = {}: no keep = \"world\" resource for it to clear",
                k.reach
            ));
        }
        Some(k) if k.reach == 0 => return Err("[kept] reach is at least 1 block".into()),
        Some(k) => Some(k.reach),
        None if world => {
            return Err(
                "a keep = \"world\" resource needs [kept] reach: how far a teleport \
                        clears it"
                    .into(),
            );
        }
        None => None,
    };
    if before_world.len() > MAX_BEFORE_WORLD {
        return Err(format!(
            "{} [[before_world]] entries; a pack may have {MAX_BEFORE_WORLD}",
            before_world.len()
        ));
    }
    for s in &before_world {
        if s.compute {
            return Err(format!(
                "{}: a [[before_world]] entry computes: no `compute` key",
                s.file
            ));
        }
        compute_shape(s)?;
    }
    post_entries(post)?;
    frame_invocations(before_world.iter().chain(post.iter().filter(|p| p.compute)))?;
    for s in before_world.iter().chain(post) {
        access(s, &resources)?;
    }
    let mut kept_resets = Vec::new();
    for (id, value) in resets {
        if resources.is_empty() {
            return Err(format!(
                "option {id:?}: resets, with no [[resource]] to clear"
            ));
        }
        kept_resets.push((
            id,
            keep(&value).map_err(|_| format!("option resets {value:?}: view or world"))?,
        ));
    }
    Ok(Kept {
        reach,
        resources,
        before_world,
        resets: kept_resets,
    })
}

/// `reads` of a stage that draws the world (`[material]`, `[entity]`, `[sky]`; none has
/// `writes`): declared resources, as a pass's are, and a texture's this-frame version only where
/// a `[[before_world]]` pass writes it, before the stage `draws`; a `[[post]]` pass writes it
/// after, so the stage reads it as `_previous`. A buffer has one version, read as it is.
pub(crate) fn drawn(m: &Stage, kept: &Kept, draws: &str) -> Result<(), String> {
    access(m, &kept.resources)?;
    let written = |name: &String| kept.before_world.iter().any(|w| w.writes.contains(name));
    let late = (kept.resources.iter())
        .find(|r| r.is_texture() && m.reads.contains(&r.name) && !written(&r.name));
    match late {
        Some(r) => Err(format!(
            "{}: reads {:?}, this frame's, which no [[before_world]] pass writes before {draws}: \
             read {}_previous",
            m.file, r.name, r.name
        )),
        None => Ok(()),
    }
}

/// The kept textures of the post target budget, by name, format and scale: an `output` naming a
/// kept texture writes it, at its own size (no `scale`), never its `_previous` nor an integer one
/// (`Format::is_uint`), which only a compute pass writes; each R-relative kept texture counts.
pub(crate) fn kept_targets<'a>(
    post: &[Stage],
    resources: &'a [Resource],
) -> Result<Vec<(&'a str, Format, f32)>, String> {
    for p in post {
        let Some(output) = &p.output else { continue };
        if let Some(r) = resources.iter().find(|r| r.name == *output)
            && (!r.is_texture() || p.scale.is_some())
        {
            return Err(format!(
                "{}: output {output:?} is a kept {}: an `output` writes a texture, at its size, \
                 with no `scale`",
                p.file,
                if r.is_texture() { "texture" } else { "buffer" }
            ));
        }
        if let Some(format) = (resources.iter().find(|r| r.name == *output))
            .and_then(Resource::format)
            .filter(|f| f.is_uint())
        {
            return Err(format!(
                "{}: output {output:?} is an {} texture: a pass draws floats, so only a compute \
                 pass writes one",
                p.file,
                format.wgsl()
            ));
        }
        if resources
            .iter()
            .any(|r| format!("{}_previous", r.name) == *output)
        {
            return Err(format!(
                "{}: output {output:?} is last frame's: nothing writes it",
                p.file
            ));
        }
    }
    Ok(resources
        .iter()
        .filter_map(|r| match r.shape {
            Shape::Scaled { format, scale } => Some((r.name.as_str(), format, scale)),
            _ => None,
        })
        .collect())
}
