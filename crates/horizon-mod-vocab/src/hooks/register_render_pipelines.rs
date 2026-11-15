//! Client render pipeline registration.

pub trait Hooks {
    /// Register the mod's own render pipelines, once at client startup (NeoForge's
    /// `RegisterRenderPipelinesEvent`; 1.21.1 mods used `RegisterShadersEvent`): push a
    /// [`RenderPipeline`] for each into `event.pipelines`. The game builds them with its own at
    /// startup and again on every resource reload. An id outside the mod's namespaces, one an
    /// earlier registration took are logged with the mod and
    /// dropped. A required pipeline whose shader is missing or doesn't build fails the load: the
    /// game does not start, and a reload fails and keeps the previous resources. An
    /// [optional](RenderPipeline::optional()) one is logged and resolves to
    /// [`RenderPipelineError::Invalid`].
    #[notes = r#"## Render pipelines

`Hooks::register_render_pipelines` is the client registration-phase counterpart of NeoForge's
`RegisterRenderPipelinesEvent` (and of 1.21.1's `RegisterShadersEvent`, which it replaced). A
pipeline is plain data: its id, a WGSL fragment shader in the mod's assets, a blend mode and a
depth mode. No GPU object or shader handle crosses the mod API, so a native mod and a wasm mod
register the same way.

```rust,ignore
fn register_render_pipelines(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterRenderPipelinesEvent<'_>,
) {
    event.pipelines.push(RenderPipeline::new(
        Identifier::parse("example:glow").unwrap(),
        Identifier::parse("example:glow").unwrap(),
        PipelineBlend::Additive,
        WorldDepth::Tested,
    ));
}
```

The shader `example:glow` is `assets/example/shaders/pipeline/glow.wgsl`. It defines one function;
the game supplies the vertex stage, the bindings and the conversion to the frame:

```wgsl
fn pipeline_fragment(input: PipelineInput) -> vec4<f32> {
    let pulse = 1.0 + sin(frame.time * 4.0);
    return vec4(input.color.rgb * pulse, input.color.a * (1.0 - input.uv.y));
}
```

`PipelineInput` has `uv` (the quad's texture coordinate), `color` (the quad's colour) and
`position` (blocks, relative to the camera). `frame.time` is seconds, wrapping every hour, and
`frame.viewport` is the world picture's size in pixels. Colours follow the
mod render colour rule (`docs/design/mod-api/mod-render-colour.md`): the function reads and returns linear
light, 1.0 is paper white, more is brighter, and alpha is coverage. A file that declares its own
bindings or entry points is refused.

Resolve the pipeline with `host.render_assets().render_pipeline(&id)` in `client_setup` and again
in `client_resources_reloaded`, then draw with `WorldDrawContext::pipeline_quads` from
`submit_custom_geometry` or `render_level_stage`. A resource reload builds every registered
pipeline again, so a resource pack can replace a shader; a handle from before the reload is
`Stale`.

A pipeline is required unless it is registered with `.optional()` (NeoForge's
`registerPipeline` and `registerOptionalPipeline`). When a load cannot build a required pipeline,
because its file is missing, refused or turned down by the GPU, the whole load fails and names it:
the game does not start, and a reload keeps the previous resources and handles, then drops the
selected resource packs and tries again, as vanilla does with any failed reload. An optional
pipeline that cannot be built is logged once with its id and resolves to `Invalid`, and the other
pipelines draw. Pipeline quads share the view's 4,096 quads with `WorldDrawContext::quads`, and in each
stage they draw after that stage's plain quads and lines, in the order submitted. Under a shader
pack they follow the plain quads: a stage the pack draws itself is drawn with the next stage the
game reaches."#]
    #[hook(
        side = "client",
        contract = "collect",
        out = "pipelines",
        collect_validator = "crate::client::render_pipeline::validate_render_pipelines",
        context = "registration",
        neoforge = "RegisterRenderPipelinesEvent"
    )]
    fn register_render_pipelines();
}
