//! Entity model layer registration, baking and drawing.

pub trait Hooks {
    /// Entity model layers built in code (NeoForge's
    /// `EntityRenderersEvent.RegisterLayerDefinitions`): push each one's id,
    /// `<mod id>:<model>#<layer>` as a `ModelLayerLocation` names it
    /// (`mymod:golem#main`), and its `LayerDefinition`, as the mod's
    /// `createBodyLayer()` builds it. `native_bake_layer` bakes one into the
    /// `ModelPart` tree `native_draw_model_part` draws. Asked once at start,
    /// on the client. An id in vanilla's namespace, not of that form, or
    /// registered already is logged and left out. API @API:model_layers@.
    #[notes = r#"## Entity model layers

`Hooks::register_layer_definitions` is the client registration-phase counterpart of NeoForge 26.3's
`EntityRenderersEvent.RegisterLayerDefinitions`. It receives a `RegistrationContext`, once at
client startup, and collects `(id, LayerDefinition)` entries in dependency order. Dedicated
and integrated server roles do not fire this client event. IDs are `<mod id>:<model>#<layer>`;
vanilla-namespace, malformed and duplicate IDs are logged and left out.

A `LayerDefinition` holds a `MeshDefinition` of named parts whose `CubeListBuilder` boxes sit
at a `PartPose`, on a texture of the given size. Texture offsets, mirroring and
`CubeDeformation` match vanilla's model builders. This listener is compiled in the API tests:

```rust,ignore
{{#include ../../../../crates/horizon-client/tests/mod_api_hooks.rs:register_layer_definitions}}
```

`ClientHost::bake_layer` bakes a registered layer or a vanilla layer by its
`entity-models.json` name (`minecraft:zombie#main`) into an owned `ModelPart` tree. Bake once
and keep it. Before each frame, call `reset_all_poses` and set the parts' positions and
rotations as `setupAnim` does. A layer unavailable from the cached mob types is read from
resources for each bake, so do not bake it every frame.

Draw the tree with `BlockEntityDrawContext::model_part`, using the same draw context the mob
or block entity submit hook received. There is no separate Host drawing helper. Positions
are texels, +y down and the front towards -z; convert the mob's frame with scale
`[1.0, -1.0, -1.0]` then translation `[0.0, -1.501, 0.0]`, as `LivingEntityRenderer` does.
The sprite is the entity texture ID and the tint is `0xRRGGBB`."#]
    #[hook(
        side = "client",
        contract = "collect",
        out = "layers",
        context = "registration",
        neoforge = "EntityRenderersEvent.RegisterLayerDefinitions"
    )]
    fn register_layer_definitions();
}
