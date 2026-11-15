# Drawing with your own render pipeline

Draw world quads with your own fragment shader. Register a `RenderPipeline`, resolve
its handle, and submit quads through a `WorldDrawContext`. This is the restricted
counterpart of NeoForge's `RegisterRenderPipelinesEvent`: you supply the fragment
stage; the game fixes the vertex stage, primitive type and bindings.

## 1. Write the shader

Put a WGSL file at `assets/<namespace>/shaders/pipeline/<path>.wgsl`. It defines one function:

```wgsl
fn pipeline_fragment(input: PipelineInput) -> vec4<f32> {
    let pulse = 1.0 + sin(frame.time * 4.0);
    return vec4(input.color.rgb * pulse, input.color.a * (1.0 - input.uv.y));
}
```

The game puts this before your file:

| Name | Type | Meaning |
| --- | --- | --- |
| `input.uv` | `vec2<f32>` | The quad's texture coordinate at this fragment, as its corners gave it. |
| `input.color` | `vec4<f32>` | The quad's colour in linear light; alpha is coverage. |
| `input.position` | `vec3<f32>` | This point in blocks, relative to the camera. |
| `frame.time` | `f32` | Seconds of game time, smooth between ticks, wrapping to 0 every hour. |
| `frame.viewport` | `vec2<f32>` | The world picture's size in pixels. |

The function reads and returns linear light: 1.0 is paper white, and more is brighter where the frame can hold it. The
alpha you return is coverage. The game supplies the vertex stage, the bindings and the conversion
to the frame, so a file that declares a binding or an entry point is refused. Names that start with
`horizon_` belong to the game.

## 2. Register the pipeline

```rust,ignore
fn register_render_pipelines(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterRenderPipelinesEvent<'_>,
) {
    event.pipelines.push(RenderPipeline::new(
        Identifier::parse("example:glow").unwrap(), // the pipeline's id
        Identifier::parse("example:glow").unwrap(), // assets/example/shaders/pipeline/glow.wgsl
        PipelineBlend::Additive,
        WorldDepth::Tested,
    ));
}
```

`PipelineBlend::Translucent` lays the result over the frame by its alpha, and `Additive` adds it,
scaled by its alpha. `WorldDepth::Tested` hides the quads behind nearer world surfaces, and
`ThroughWalls` draws them over everything. No pipeline writes depth.

The id must be in a namespace your mod owns. There is no per-mod pipeline count cap. A registration
with an unowned namespace or a duplicate id is logged with the mod's id and dropped.

### Required or optional

A pipeline is required unless you register it with `.optional()`, as NeoForge's `registerPipeline`
and `registerOptionalPipeline` are:

```rust,ignore
event.pipelines.push(
    RenderPipeline::new(id, fragment, PipelineBlend::Translucent, WorldDepth::Tested).optional(),
);
```

| Policy | When a resource load cannot build it |
| --- | --- |
| Required (`RenderPipeline::new`) | The load fails with a message naming each required pipeline and why. At startup the game does not start. A resource reload fails and keeps the previous resources, so handles from before it still draw; then, as vanilla does with any failed reload, the selected resource packs are dropped and the reload runs again. |
| Optional (`.optional()`) | The log has one line naming the pipeline and why. It resolves to `RenderPipelineError::Invalid`, and every other pipeline builds and draws. |

A load cannot build a pipeline when its shader file is missing, the file is refused, or the GPU
rejects the shader. The game never panics over a pipeline's shader.

## 3. Resolve and draw

```rust,ignore
fn client_setup(&self, host: &dyn ClientHost, _event: &mut ClientSetupEvent<'_>) {
    let id = Identifier::parse("example:glow").unwrap();
    *self.glow.lock().unwrap() = host.render_assets().render_pipeline(&id).ok();
}

fn submit_custom_geometry(&self, _host: &dyn ClientHost, event: &mut SubmitCustomGeometryEvent<'_>) {
    let Some(glow) = *self.glow.lock().unwrap() else { return };
    let quad = PipelineQuad::spanning(corners, 0xFFFF_C040);
    if let Err(error) = event.draw.pipeline_quads(glow, &[quad]) {
        log::warn!("glow not drawn: {error:?}");
    }
}
```

Resolve again in `client_resources_reloaded`: every resource reload builds the pipelines again
from the new resources, so a resource pack can replace your shader, and a handle from before the
reload answers `Stale`.

| Error | When |
| --- | --- |
| `RenderPipelineError::Unknown` | No mod registered that id. |
| `RenderPipelineError::Invalid` | The pipeline is optional and its shader file is missing, was refused or was rejected by the GPU. The log has a line `render pipeline <id> from mod <owner> refused: <file>: <reason>`, with the file's line number for a syntax or type error, or `render pipeline <id> from mod <owner> refused by the GPU: <reason>`. |
| `RenderPipelineError::Stale` | The handle is from before a resource reload. |
| `PipelineDrawError::NonFinite` | A corner or texture coordinate is not finite, or a camera-relative coordinate overflows `f32`. |
| `PipelineDrawError::CapacityExceeded` | The view's 4,096 quads, shared with `WorldDrawContext::quads`, are used up. |

A refused batch draws nothing. An `Invalid` pipeline never stops the game from starting; a required
pipeline that cannot be built does.

## What a pipeline draws, and when

`pipeline_quads` works wherever a `WorldDrawContext` is given: `submit_custom_geometry` and each
`render_level_stage`. Within a stage, pipeline quads draw after that stage's plain quads and
lines, in the order submitted. Under a shader pack they follow the plain quads: a stage the pack
draws itself is drawn with the next stage the game reaches.

## Sampling a texture

A pipeline can sample one texture (vanilla's `Sampler0`). Name it when you register the pipeline:

```rust,ignore
event.pipelines.push(
    RenderPipeline::new(id.clone(), id, PipelineBlend::Additive, WorldDepth::Tested)
        .textured(PipelineTexture::Sprite(Identifier::parse("example:block/charge").unwrap())),
);
```

`PipelineTexture::Sprite(id)` names a sprite of the block or item atlas by its texture id:
`example:block/charge` is `assets/example/textures/block/charge.png` as the `minecraft:blocks`
atlas holds it, `minecraft:item/diamond` an item sprite. An animated sprite animates. A pipeline
always samples a rectangle of the atlas, never a texture of its own. A texture file of your mod's
own, packed into the runtime atlas's slot pool (arismoko/horizon#4438) and drawn the same way, follows once that
pool exists.

The shader then has one more function:

```wgsl
fn pipeline_fragment(input: PipelineInput) -> vec4<f32> {
    // The texture in linear light, tinted by the quad's colour, as vanilla's position_tex_color does.
    return pipeline_texture(input.uv) * input.color;
}
```

`pipeline_texture(uv)` maps the fraction of `uv` into the sprite's own rectangle (`uv` 0 to 1
spans it once; beyond that, negative values too, it repeats) and loads one texel by integer
coordinates at mip level 0: nearest, unfiltered, so no texel of a neighbouring sprite can enter,
even at the far edge. It returns the texel in linear light with its alpha. `input.uv` is still
what the quad's corners gave, so the quads you already draw keep their coordinates.

A texture is part of the pipeline, so it is checked when the pipeline is built, at startup and on
each reload. A refused texture is named by `PipelineTextureError` and refuses the pipeline as a
missing shader does: a required one fails the load, and an optional one is logged with the named
cause and resolves to `Invalid`:

| Cause | When |
| --- | --- |
| `NoSuchSprite` | Neither the block nor the item atlas has the sprite (`… refused by the GPU: its texture sprite <name>: NoSuchSprite: …`) |
| `NotASpriteKey` | The id names a file (a `textures/` path or a `.png` name), not a sprite key |
| `NoSuchTextureFile`, `OverCap`, `Undecodable`, `NoSlot` | Texture files, once the slot pool exists: missing, over 1024 x 1024, not an image, or no slot within the mod's budget of 16 images and 4 MiB |

A pipeline without `.textured(…)` has no `pipeline_texture`: its file is refused with a note
naming `RenderPipeline::textured`.

A pipeline cannot take values of its own from the mod, sample more than one texture, or replace the
vertex stage.
