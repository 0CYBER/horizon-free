//! Render pipelines mods register (`Hooks::register_render_pipelines`, NeoForge's
//! `RegisterRenderPipelinesEvent`): the registrations, kept once at start, and what each
//! resource load makes of them. A load reads every registered pipeline's fragment file
//! (`assets/<namespace>/shaders/pipeline/<path>.wgsl`), puts Horizon's prelude before it and its
//! entry points after it, and checks the whole shader, so the renderer (`gfx::mod_pipelines`) is
//! only ever handed WGSL that compiles. A required pipeline the load cannot build fails the load;
//! an optional one is logged and left out. A frame reads none of this by name.
//!
//! Colour follows the mod render colour rule (docs/design/mod-api/mod-render-colour.md):
//! the mod's function reads and returns linear light, and the generated entry points convert
//! from the quad's vanilla-space colour and to the frame, which stores vanilla's 2.2 encoding
//! in every format the renderer draws today.
use mod_api::{
    Identifier, PipelineBlend, PipelineTexture, PipelineTextureError, RenderPipeline,
    RenderPipelineError, RenderPipelineRef, WorldDepth,
};
use naga::valid::{Capabilities, ValidationFlags, Validator};
use std::sync::OnceLock;

/// The folder under each namespace's `shaders` the fragment files are read from.
const FOLDER: &str = "shaders/pipeline";

/// The function a pipeline's file defines.
const FUNCTION: &str = "pipeline_fragment";

/// What a pipeline's file can use, before it in the shader. Names starting `horizon_` are the
/// renderer's own.
const PRELUDE_TYPES: &str = "\
// What `pipeline_fragment` is given for each fragment.
struct PipelineInput {
    // The quad's texture coordinate here, as its corners gave it.
    uv: vec2<f32>,
    // The quad's colour in linear light; alpha is coverage.
    color: vec4<f32>,
    // This point in blocks, relative to the camera.
    position: vec3<f32>,
}
// The frame's values, the same for every fragment.
struct PipelineFrame {
    // The world picture's size in pixels.
    viewport: vec2<f32>,
    // Seconds of game time, smooth between ticks, wrapping to 0 every hour.
    time: f32,
}
@group(0) @binding(0) var<uniform> horizon_pipeline_view: mat4x4<f32>;
@group(0) @binding(1) var<uniform> frame: PipelineFrame;
";

/// What a textured pipeline's file can use as well ([`RenderPipeline::textured`]): its texture,
/// bound by the renderer in group 1, and the function that samples it.
const TEXTURE_PRELUDE: &str = "\
@group(1) @binding(0) var horizon_pipeline_texture: texture_2d<f32>;
@group(1) @binding(1) var horizon_pipeline_sampler: sampler;
// Where the sprite lies in the atlas: [u0, v0, u1, v1].
@group(1) @binding(2) var<uniform> horizon_pipeline_rect: vec4<f32>;
// The pipeline's sprite at `uv`: 0 to 1 spans it once and beyond that it repeats. One texel by
// integer coordinates inside the sprite's rect at mip 0 (`repeat_texel` is the same arithmetic),
// in linear light, with its alpha.
fn pipeline_texture(uv: vec2<f32>) -> vec4<f32> {
    let size = vec2<f32>(textureDimensions(horizon_pipeline_texture, 0));
    let origin = round(horizon_pipeline_rect.xy * size);
    let extent = max(round(horizon_pipeline_rect.zw * size) - origin, vec2<f32>(1.0));
    let local = clamp(floor((uv - floor(uv)) * extent), vec2<f32>(0.0), extent - 1.0);
    let texel = textureLoad(horizon_pipeline_texture, vec2<i32>(origin + local), 0);
    return vec4<f32>(horizon_decode(texel.rgb), texel.a);
}
";

/// `pipeline_texture`'s texel arithmetic, for testing without a GPU: the integer texel of an
/// atlas `size` texels wide and high that `uv` picks inside the sprite at `rect` (`[u0, v0, u1,
/// v1]`). The fraction of `uv` (negative values wrap) spans the rect once; the texel is always
/// one of the rect's own, its far edge excluded.
pub fn repeat_texel(uv: [f32; 2], rect: [f32; 4], size: [u32; 2]) -> [u32; 2] {
    std::array::from_fn(|axis| {
        let size = size[axis] as f32;
        let origin = (rect[axis] * size).round();
        let extent = ((rect[axis + 2] * size).round() - origin).max(1.0);
        let f = uv[axis] - uv[axis].floor();
        let local = (f * extent).floor().clamp(0.0, extent - 1.0);
        (origin + local) as u32
    })
}

/// The entry points, after the pipeline's file: the vertex stage, and a fragment stage that
/// hands the mod linear light and stores its answer as the frame holds colour.
const ENTRIES: &str = "
struct HorizonPipelineVertex {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) position: vec3<f32>,
}
@vertex fn horizon_vs_pipeline(@location(0) position: vec3<f32>, @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>) -> HorizonPipelineVertex {
    var out: HorizonPipelineVertex;
    out.clip = horizon_pipeline_view * vec4<f32>(position, 1.0);
    out.uv = uv;
    out.color = color;
    out.position = position;
    return out;
}
@fragment fn horizon_fs_pipeline(in: HorizonPipelineVertex) -> @location(0) vec4<f32> {
    let result = pipeline_fragment(PipelineInput(in.uv, horizon_decode_rgba(in.color), in.position));
    // Coverage only: no light rides in the frame's alpha, and NaN covers nothing.
    let coverage = clamp(select(0.0, result.a, result.a > 0.0), 0.0, 1.0);
    return vec4<f32>(horizon_encode(result.rgb), coverage);
}
";

/// The renderer's entry points in a built pipeline's shader.
pub const VERTEX_ENTRY: &str = "horizon_vs_pipeline";
pub const FRAGMENT_ENTRY: &str = "horizon_fs_pipeline";

static REGISTERED: OnceLock<Vec<RenderPipeline>> = OnceLock::new();

/// Publish the mods' registrations once at startup, as `register_render_pipelines` left them:
/// each id once, in a namespace its mod owns. A pipeline's place in the list is its slot.
pub fn register(pipelines: Vec<RenderPipeline>) {
    if REGISTERED.set(pipelines).is_err() {
        log::warn!("Render pipelines already registered; keeping the first list");
    }
}

/// The registered pipelines; none until registration publishes them.
fn registered() -> &'static [RenderPipeline] {
    REGISTERED.get().map_or(&[], Vec::as_slice)
}

/// A registered pipeline one resource load built: a shader that compiles and its fixed state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Built {
    /// The owning mod, not necessarily the pipeline namespace.
    pub owner: String,
    /// The pipeline's id, for the log.
    pub id: Identifier,
    /// The whole shader: prelude, the mod's file, entry points.
    pub shader: String,
    pub blend: PipelineBlend,
    pub depth: WorldDepth,
    /// Whether the load may go on without it ([`RenderPipeline::optional()`]).
    pub optional: bool,
    /// What its shader samples, if anything.
    pub texture: Option<BuiltTexture>,
}

/// A textured pipeline's texture as a load found it ([`PipelineTexture`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuiltTexture {
    /// A sprite of the 2D atlas, by its name there (`block/stone`, `powah:block/energy_charge`),
    /// looked up when the renderer builds the pipeline.
    Sprite(String),
}

/// The registered pipelines as one resource load built them, by slot. An optional pipeline the
/// load refused has no shader here and resolves to [`RenderPipelineError::Invalid`].
#[derive(Clone, Debug, Default)]
pub struct Pipelines {
    ids: Vec<Identifier>,
    built: Vec<Option<Built>>,
    /// The render assets' generation this set was loaded for; 0 for none.
    generation: u64,
    invalid: std::sync::Arc<std::sync::Mutex<Vec<usize>>>,
}

impl Pipelines {
    /// Build every registered pipeline from `source`, for the load of `generation`. An optional
    /// pipeline whose file is missing or refused is logged with its reason and the others build;
    /// a required one fails the load, naming every required pipeline refused and why.
    pub fn load(
        source: &crate::resource_source::ResourceSource,
        generation: u64,
    ) -> Result<Self, String> {
        Self::build(registered(), generation, |file| {
            source.read_to_string(file).map_err(|e| e.to_string())
        })
    }

    /// Build every registered pipeline, reading fragment files with `read`. A pipeline's
    /// texture is named here; its sprite is looked up when the renderer builds it.
    pub(crate) fn build(
        registered: &[RenderPipeline],
        generation: u64,
        read: impl Fn(&str) -> Result<String, String>,
    ) -> Result<Self, String> {
        let mut required = Vec::new();
        let built = registered
            .iter()
            .map(|pipeline| {
                let mut file = fragment_file(&pipeline.fragment);
                let loaded = read(&file)
                    .and_then(|text| compose_with(&text, pipeline.texture.is_some()))
                    .and_then(|shader| {
                        let texture = match &pipeline.texture {
                            None => None,
                            Some(texture) => Some(load_texture(texture).map_err(|cause| {
                                file = "its texture".into();
                                texture_refusal(cause)
                            })?),
                        };
                        Ok((shader, texture))
                    });
                match loaded {
                    Ok((shader, texture)) => Some(Built {
                        owner: pipeline.owner().to_owned(),
                        id: pipeline.id.clone(),
                        shader,
                        blend: pipeline.blend,
                        depth: pipeline.depth,
                        optional: pipeline.optional,
                        texture,
                    }),
                    Err(reason) => {
                        let refusal = format!(
                            "render pipeline {} from mod {} refused: {file}: {reason}",
                            pipeline.id,
                            pipeline.owner()
                        );
                        if pipeline.optional {
                            log::warn!("Horizon: {refusal}");
                        } else {
                            required.push(format!("required {refusal}"));
                        }
                        None
                    }
                }
            })
            .collect();
        if !required.is_empty() {
            return Err(required.join("; "));
        }
        Ok(Self {
            ids: registered.iter().map(|p| p.id.clone()).collect(),
            built,
            generation,
            invalid: Default::default(),
        })
    }

    /// Marks `slot` as one the GPU refused; false when it already was, so it is reported once.
    pub(crate) fn invalidate(&self, slot: usize) -> bool {
        let mut invalid = self.invalid.lock().unwrap_or_else(|e| e.into_inner());
        let fresh = !invalid.contains(&slot);
        if fresh {
            invalid.push(slot);
        }
        fresh
    }

    /// Makes `slot` usable again: a renderer that built it took the refusal of another over.
    pub(crate) fn revalidate(&self, slot: usize) {
        self.invalid
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|&invalid| invalid != slot);
    }

    /// The load this set belongs to: a renderer holding another builds its pipelines again.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Every slot in order, with what the load built for it.
    pub fn built(&self) -> &[Option<Built>] {
        &self.built
    }

    /// The handle of the registered pipeline `id` in this load.
    pub fn resolve(&self, id: &Identifier) -> Result<RenderPipelineRef, RenderPipelineError> {
        let slot = self
            .ids
            .iter()
            .position(|registered| registered == id)
            .ok_or(RenderPipelineError::Unknown)?;
        if self.built[slot].is_none()
            || self
                .invalid
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains(&slot)
        {
            return Err(RenderPipelineError::Invalid);
        }
        Ok(mod_api::__private::built::render_pipeline_ref(
            self.generation,
            slot as u32,
        ))
    }

    /// The slot a handle draws with, checked once when a batch is submitted.
    pub fn slot(&self, pipeline: RenderPipelineRef) -> Result<u32, RenderPipelineError> {
        use mod_api::__private::game::Fields;
        let fields = pipeline.fields();
        if fields.generation != self.generation {
            return Err(RenderPipelineError::Stale);
        }
        if self
            .invalid
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&(fields.slot as usize))
        {
            return Err(RenderPipelineError::Invalid);
        }
        match self.built.get(fields.slot as usize) {
            Some(Some(_)) => Ok(fields.slot),
            Some(None) => Err(RenderPipelineError::Invalid),
            None => Err(RenderPipelineError::Unknown),
        }
    }
}

/// `mymod:beam/glow` is `assets/mymod/shaders/pipeline/beam/glow.wgsl`.
fn fragment_file(fragment: &Identifier) -> String {
    format!(
        "assets/{}/{FOLDER}/{}.wgsl",
        fragment.namespace(),
        fragment.path()
    )
}

/// `texture` as a load finds it, or its named refusal.
fn load_texture(texture: &PipelineTexture) -> Result<BuiltTexture, PipelineTextureError> {
    match texture {
        PipelineTexture::Sprite(id) => {
            // A file's path or name is not a sprite key.
            let path = id.path();
            if path.starts_with("textures/") || path.ends_with(".png") {
                return Err(PipelineTextureError::NotASpriteKey);
            }
            // Named as the 2D atlas names the blocks and items atlases' sprites.
            Ok(BuiltTexture::Sprite(match id.namespace() {
                "minecraft" => path.to_owned(),
                _ => id.to_string(),
            }))
        }
        // A texture kind a later API adds is one this game has no route for yet.
        _ => Err(PipelineTextureError::NoSuchTextureFile),
    }
}

/// The diagnostics line of a refused texture: its named cause, then what it means.
pub fn texture_refusal(cause: PipelineTextureError) -> String {
    format!("{cause:?}: {cause}")
}

/// The prelude a pipeline's file follows.
fn prelude(textured: bool) -> String {
    format!(
        "{PRELUDE_TYPES}{}{}",
        if textured { TEXTURE_PRELUDE } else { "" },
        horizon_shader_api::stages::COLOR_HELPERS
    )
}

/// The whole shader of an untextured pipeline whose file is `text` ([`compose_with`]).
pub fn compose(text: &str) -> Result<String, String> {
    compose_with(text, false)
}

/// The whole shader of a pipeline whose file is `text`, with `pipeline_texture` when it is
/// `textured`, or why the file is refused, with the file's own line where there is one.
pub fn compose_with(text: &str, textured: bool) -> Result<String, String> {
    let prelude = prelude(textured);
    let mut shader = format!("{prelude}{text}\n");
    let located = |at: Option<usize>, message: String| match at
        .and_then(|at| at.checked_sub(prelude.len()))
        .filter(|at| *at <= text.len())
    {
        Some(at) => format!(
            "line {}: {message}",
            text.as_bytes()[..at]
                .iter()
                .filter(|b| **b == b'\n')
                .count()
                + 1
        ),
        None => message,
    };
    // First without the entry points, so a missing or mistyped function is reported as such
    // rather than as a call that doesn't resolve.
    let module = naga::front::wgsl::parse_str(&shader).map_err(|e| {
        let at = e
            .labels()
            .next()
            .and_then(|(span, _)| span.to_range())
            .map(|range| range.start);
        let mut message = e.message().to_owned();
        if !textured && text.contains("pipeline_texture") {
            message += " (`pipeline_texture` needs a texture: RenderPipeline::textured)";
        }
        located(at, message)
    })?;
    for (_, function) in module.functions.iter() {
        if function.expressions.iter().any(|(_, expression)| matches!(expression,
            naga::Expression::GlobalVariable(handle) if module.global_variables[*handle].name.as_deref() == Some("horizon_pipeline_view"))) {
            return Err("horizon_pipeline_view is vertex-only".into());
        }
    }
    if !module.overrides.is_empty() {
        return Err("override declarations are not supported".into());
    }
    signature(&module)?;
    if !module.entry_points.is_empty() {
        return Err("it declares an entry point; the game supplies both stages".into());
    }
    if module
        .global_variables
        .iter()
        .filter(|(_, variable)| variable.binding.is_some())
        .count()
        != if textured { 5 } else { 2 }
    {
        return Err("it declares a binding; a pipeline reads only `frame` and its input".into());
    }
    shader.push_str(ENTRIES);
    let module = naga::front::wgsl::parse_str(&shader).map_err(|e| e.message().to_owned())?;
    Validator::new(ValidationFlags::all(), Capabilities::default())
        .validate(&module)
        .map_err(|e| {
            let at = e
                .spans()
                .next()
                .and_then(|(span, _)| span.to_range())
                .map(|range| range.start);
            let mut message = e.as_inner().to_string();
            let mut source = std::error::Error::source(e.as_inner());
            while let Some(inner) = source {
                message += &format!(": {inner}");
                source = inner.source();
            }
            located(at, message)
        })?;
    Ok(shader)
}

/// `fn pipeline_fragment(input: PipelineInput) -> vec4<f32>`, or what the file has instead.
fn signature(module: &naga::Module) -> Result<(), String> {
    let expected = format!("expected `fn {FUNCTION}(input: PipelineInput) -> vec4<f32>`");
    let (_, function) = module
        .functions
        .iter()
        .find(|(_, function)| function.name.as_deref() == Some(FUNCTION))
        .ok_or_else(|| format!("no function `{FUNCTION}`: {expected}"))?;
    let takes_input = matches!(
        function.arguments.as_slice(),
        [argument] if module.types[argument.ty].name.as_deref() == Some("PipelineInput")
    );
    let returns_color = function.result.as_ref().is_some_and(|result| {
        matches!(
            module.types[result.ty].inner,
            naga::TypeInner::Vector {
                size: naga::VectorSize::Quad,
                scalar: naga::Scalar::F32,
            }
        )
    });
    if takes_input && returns_color {
        Ok(())
    } else {
        Err(format!("`{FUNCTION}` has another signature: {expected}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragment_cannot_read_vertex_uniform() {
        assert!(compose("fn pipeline_fragment(input: PipelineInput) -> vec4<f32> { return horizon_pipeline_view * input.color; }").unwrap_err().contains("horizon_pipeline_view"));
    }

    #[test]
    fn overrides_without_defaults_are_refused() {
        assert!(compose("override strength: f32; fn pipeline_fragment(input: PipelineInput) -> vec4<f32> { return input.color * strength; }").unwrap_err().contains("override"));
    }

    const GLOW: &str = "\
fn pipeline_fragment(input: PipelineInput) -> vec4<f32> {
    return vec4<f32>(input.color.rgb * (1.0 + sin(frame.time)), input.color.a * input.uv.y);
}
";

    fn pipeline(id: &str, fragment: &str) -> RenderPipeline {
        RenderPipeline::new(
            Identifier::parse(id).unwrap(),
            Identifier::parse(fragment).unwrap(),
            PipelineBlend::Additive,
            WorldDepth::ThroughWalls,
        )
    }
    fn id(text: &str) -> Identifier {
        Identifier::parse(text).unwrap()
    }

    #[test]
    fn a_file_with_the_function_composes_into_a_shader_with_both_entry_points() {
        let shader = compose(GLOW).unwrap();
        let module = naga::front::wgsl::parse_str(&shader).unwrap();
        let entries: Vec<_> = module
            .entry_points
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(entries, [VERTEX_ENTRY, FRAGMENT_ENTRY]);
    }

    #[test]
    fn a_refused_file_says_what_is_wrong_and_where() {
        let refused = |text: &str| compose(text).unwrap_err();
        assert!(refused("").starts_with("no function `pipeline_fragment`"));
        assert!(
            refused("fn pipeline_fragment(uv: vec2<f32>) -> vec4<f32> { return vec4<f32>(1.0); }")
                .contains("another signature")
        );
        assert!(
            refused(
                "fn pipeline_fragment(input: PipelineInput) -> vec3<f32> { return input.position; }"
            )
            .contains("another signature")
        );
        // A syntax error is reported on the file's own line, not the composed shader's.
        let broken = "// a comment\nfn pipeline_fragment(input: PipelineInput) -> vec4<f32> {\n    return vec4<f32>(1.0)\n}\n";
        let message = refused(broken);
        assert!(
            message.starts_with("line 3:") || message.starts_with("line 4:"),
            "{message}"
        );
        // A type error is found by validation, with the file's line too.
        let mistyped = "fn pipeline_fragment(input: PipelineInput) -> vec4<f32> {\n    let x: f32 = input.uv;\n    return vec4<f32>(x);\n}\n";
        assert!(
            refused(mistyped).starts_with("line 2:"),
            "{}",
            refused(mistyped)
        );
        assert!(
            refused(&format!(
                "@group(0) @binding(2) var<uniform> mine: vec4<f32>;\n{GLOW}"
            ))
            .contains("declares a binding")
        );
        assert!(
            refused(&format!(
                "{GLOW}@fragment fn mine() -> @location(0) vec4<f32> {{ return vec4<f32>(1.0); }}\n"
            ))
            .contains("declares an entry point")
        );
    }

    #[test]
    fn an_optional_pipeline_that_fails_leaves_the_others_drawing_and_refuses_its_own_use() {
        let registered = [
            pipeline("example:glow", "example:glow"),
            pipeline("example:broken", "example:broken").optional(),
            pipeline("example:missing", "example:nested/missing").optional(),
            pipeline("example:driver", "example:glow").optional(),
        ];
        let read = |file: &str| match file {
            "assets/example/shaders/pipeline/glow.wgsl" => Ok(GLOW.to_owned()),
            "assets/example/shaders/pipeline/broken.wgsl" => Ok("fn nothing() {}".to_owned()),
            "assets/example/shaders/pipeline/nested/missing.wgsl" => Err("not found".to_owned()),
            other => panic!("read {other}"),
        };
        let set = Pipelines::build(&registered, 7, read).unwrap();

        let glow = set.resolve(&id("example:glow")).unwrap();
        assert_eq!(set.slot(glow), Ok(0));
        let built = set.built()[0].as_ref().unwrap();
        assert_eq!(
            (built.blend, built.depth),
            (PipelineBlend::Additive, WorldDepth::ThroughWalls)
        );
        assert_eq!(
            set.resolve(&id("example:broken")),
            Err(RenderPipelineError::Invalid)
        );
        assert_eq!(
            set.resolve(&id("example:missing")),
            Err(RenderPipelineError::Invalid)
        );
        assert_eq!(
            set.resolve(&id("example:never")),
            Err(RenderPipelineError::Unknown)
        );

        // The GPU turns down a pipeline the load built: it is reported once, a handle taken
        // before is refused, and the rest still draw.
        let driver = set.resolve(&id("example:driver")).unwrap();
        assert!(set.invalidate(3));
        assert!(!set.invalidate(3));
        assert_eq!(set.slot(driver), Err(RenderPipelineError::Invalid));
        assert_eq!(
            set.resolve(&id("example:driver")),
            Err(RenderPipelineError::Invalid)
        );
        assert_eq!(set.slot(glow), Ok(0));
    }

    #[test]
    fn a_required_pipeline_that_fails_fails_the_load_by_name() {
        let registered = [
            pipeline("example:glow", "example:glow"),
            pipeline("example:beam", "example:beam"),
            pipeline("example:extra", "example:extra").optional(),
        ];
        let before = Pipelines::build(&registered, 7, |_| Ok(GLOW.to_owned())).unwrap();
        let handle = before.resolve(&id("example:beam")).unwrap();

        // A pack broke the required beam and the optional extra: the load fails naming the
        // required one only, so nothing of it is published and the previous load still draws.
        let error = Pipelines::build(&registered, 8, |file| match file {
            "assets/example/shaders/pipeline/glow.wgsl" => Ok(GLOW.to_owned()),
            _ => Ok("garbage".to_owned()),
        })
        .unwrap_err();
        assert!(
            error.starts_with(
                "required render pipeline example:beam from mod  refused: assets/example/shaders/pipeline/beam.wgsl: "
            ),
            "{error}"
        );
        assert!(!error.contains("example:glow"), "{error}");
        assert!(!error.contains("example:extra"), "{error}");
        assert_eq!(before.slot(handle), Ok(1));
    }

    #[test]
    fn initial_required_failure_has_a_literal_message() {
        let registered = [pipeline("example:glow", "example:glow")];
        let error = Pipelines::build(&registered, 7, |_| Err("not found".to_owned())).unwrap_err();
        assert_eq!(
            error,
            "required render pipeline example:glow from mod  refused: assets/example/shaders/pipeline/glow.wgsl: not found"
        );
    }

    #[test]
    fn a_reload_makes_old_handles_stale_and_can_fix_or_break_a_pipeline() {
        let registered = [pipeline("example:glow", "example:glow").optional()];
        let before = Pipelines::build(&registered, 7, |_| Ok(GLOW.to_owned())).unwrap();
        let handle = before.resolve(&id("example:glow")).unwrap();

        // A pack replaced the file with one that doesn't compile: the pipeline is invalid in
        // the new load, and the old handle is stale there, not silently redirected.
        let broken = Pipelines::build(&registered, 8, |_| Ok("garbage".to_owned())).unwrap();
        assert_eq!(broken.slot(handle), Err(RenderPipelineError::Stale));
        assert_eq!(
            broken.resolve(&id("example:glow")),
            Err(RenderPipelineError::Invalid)
        );

        let fixed = Pipelines::build(&registered, 9, |_| Ok(GLOW.to_owned())).unwrap();
        assert_eq!(fixed.slot(handle), Err(RenderPipelineError::Stale));
        let again = fixed.resolve(&id("example:glow")).unwrap();
        assert_eq!(fixed.slot(again), Ok(0));
    }

    /// A textured pipeline's file can call `pipeline_texture`; an untextured one is told why
    /// it can't, and neither may declare a binding of its own.
    #[test]
    fn only_a_textured_pipeline_samples_and_neither_binds_its_own() {
        let sample = "fn pipeline_fragment(input: PipelineInput) -> vec4<f32> { return pipeline_texture(input.uv) * input.color; }";
        let shader = compose_with(sample, true).unwrap();
        assert!(shader.contains("@group(1) @binding(0) var horizon_pipeline_texture"));
        assert!(
            compose(sample)
                .unwrap_err()
                .contains("RenderPipeline::textured")
        );
        let own = "@group(1) @binding(3) var<uniform> mine: f32;
fn pipeline_fragment(input: PipelineInput) -> vec4<f32> { return pipeline_texture(input.uv) * mine; }";
        assert!(
            compose_with(own, true)
                .unwrap_err()
                .contains("declares a binding")
        );
        // Untextured pipelines compose as before: no group 1.
        assert!(!compose(GLOW).unwrap().contains("@group(1)"));
    }

    /// Sprites are named as the 2D atlas names them; an id naming a file is refused by name,
    /// and a required pipeline's refusal fails the load with the named cause.
    #[test]
    fn a_textured_pipeline_names_its_sprite_and_refuses_a_file_path() {
        use mod_api::PipelineTexture;
        let pipeline = |id: &str| {
            let name = Identifier::parse("test:glow").unwrap();
            RenderPipeline::new(
                name.clone(),
                name,
                PipelineBlend::Additive,
                WorldDepth::Tested,
            )
            .textured(PipelineTexture::Sprite(Identifier::parse(id).unwrap()))
        };
        let shader = |_: &str| Ok(GLOW.to_owned());
        let built = |id| {
            Pipelines::build(&[pipeline(id)], 1, shader)
                .map(|set| set.built()[0].as_ref().unwrap().texture.clone().unwrap())
        };
        assert_eq!(
            built("minecraft:block/stone"),
            Ok(BuiltTexture::Sprite("block/stone".into()))
        );
        assert_eq!(
            built("powah:block/energy_charge"),
            Ok(BuiltTexture::Sprite("powah:block/energy_charge".into()))
        );
        assert_eq!(
            load_texture(&PipelineTexture::Sprite(
                Identifier::parse("test:textures/glow").unwrap()
            )),
            Err(PipelineTextureError::NotASpriteKey)
        );
        assert_eq!(
            built("test:effect/glow.png"),
            Err("required render pipeline test:glow from mod  refused: its texture: NotASpriteKey: the id names a file, not a sprite key".into())
        );
    }

    /// `pipeline_texture`'s texel arithmetic: `uv` 0 to 1 spans the sprite's rect once, the far
    /// edge and every repeat (negative too) land back inside it, never on a neighbour's texel.
    #[test]
    fn the_repeat_picks_integer_texels_inside_the_rect_for_any_uv() {
        // A 16 x 16 sprite at texel (32, 16) of a 64 x 64 atlas.
        let rect = [0.5, 0.25, 0.75, 0.5];
        let at = |u: f32, v: f32| repeat_texel([u, v], rect, [64, 64]);
        assert_eq!(at(0.0, 0.0), [32, 16], "the near corner");
        assert_eq!(at(0.5, 0.5), [40, 24]);
        assert_eq!(
            at(0.999_999, 0.999_999),
            [47, 31],
            "just inside the far edge"
        );
        assert_eq!(at(1.0, 1.0), [32, 16], "the far edge repeats to the start");
        assert_eq!(at(2.5, 3.0625), [40, 17], "repeated");
        assert_eq!(
            at(-0.0625, -0.25),
            [47, 28],
            "negative wraps from the far side"
        );
        assert_eq!(at(-1.0, -2.0), [32, 16]);
        // So small a negative that its fraction rounds to 1.0 still stays inside.
        assert_eq!(at(-1.0e-9, -1.0e-9), [47, 31]);
        assert_eq!(at(-1000.5, 1000.75), [40, 28]);
        // The 2x1 atlas's right half: always its one texel.
        for u in [0.0, 0.25, 0.999, 1.0, -0.5, 7.75] {
            assert_eq!(
                repeat_texel([u, u], [0.5, 0.0, 1.0, 1.0], [2, 1]),
                [1, 0],
                "{u}"
            );
        }
        // The left half: never the right's.
        for u in [0.0, 0.999, -0.001, 3.5] {
            assert_eq!(
                repeat_texel([u, 0.0], [0.0, 0.0, 0.5, 1.0], [2, 1]),
                [0, 0],
                "{u}"
            );
        }
    }
}
