pub trait DrawContexts {
    #[draw_context(
        surface = "crate::world_geometry::WorldGeometry",
        name = "WorldDrawContext"
    )]
    fn world_draw_context();
}
pub trait Hooks {
    /// Submit unlit geometry after particle submission, before opaque rendering
    /// (NeoForge 26.3 `SubmitCustomGeometryEvent`, vanilla `LevelRenderer.submitFeatures`).
    /// Client-only, noncancellable. `view` holds only camera, partial tick and game time,
    /// not full extracted level state or visible sections. Submit from extracted state.
    #[notes = r#"## Custom world geometry

`Hooks::submit_custom_geometry` runs on the client after particles, before rendering. Its
`WorldRenderView` holds camera, partial tick and game time; `WorldDrawContext::lines` and `quads`
accept unlit world-space ARGB geometry with depth testing or through walls. Batches fail atomically
for invalid inputs or overflow. `usage()` reports shared limits (4,096 of each by default);
output clears every view. Submit extracted state, without world writes or resource loading.

`WorldDrawContext::draw_model_instances` draws one batch of a resolved `ModelRef`. Each
`ModelInstance` carries an f64 world origin, a row-major 3×4 local transform, an `0xAARRGGBB`
tint (the same convention as `WorldQuad::color`) and `LightCoords` block and sky light, each
0 to 15, in vanilla's own convention. The engine converts tint and light to the frame format;
mods never assume that format or supply a packed light word. The view holds 16,384 instances,
counted by `usage().instances_used` against `instances_capacity`. A refused batch draws nothing.
A model with a translucent face returns `InstanceError::Translucent` and is not drawn as solid.
Copies outside the frustum are omitted and do not count. Shader packs and any route that cannot
take GPU instances bake the same prepared geometry on the CPU instead of dropping the model; a
`shaders/` pack shades those copies as block entities, as it shades `BlockEntityDrawContext::model`."#]
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "SubmitCustomGeometryEvent",
        cancellable = false
    )]
    fn submit_custom_geometry();
}
