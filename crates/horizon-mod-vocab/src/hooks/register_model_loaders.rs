//! The `register_model_loaders` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The model loaders a mod adds (NeoForge's `ModelEvent.RegisterLoaders`): push each loader
    /// id, `<mod id>:<path>`. Asked once at start on the client, before resources load. A model
    /// whose `loader` is a registered id gets the quads `bake_model_loader` answers, baked with
    /// its elements, on every load and reload. An id in vanilla's namespace, not of that form, or
    /// registered already is logged and left out.
    #[notes = r#"## Model loaders

A model JSON may name a `loader`: NeoForge's `neoforge:composite` and `neoforge:obj` are built in, and a mod
adds one of its own (NeoForge's `ModelEvent.RegisterLoaders`) in two steps, as a sprite source:
`Hooks::register_model_loaders` pushes the loader id once at start, before resources load, and
`Hooks::bake_model_loader` answers the quads of each model that names it, when the model is baked on every
load and reload. A loader is for geometry the vanilla `elements` can't express; a model that only needs
cuboids uses `elements`.

```rust,ignore
fn register_model_loaders(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterModelLoadersEvent<'_>,
) {
    event.loaders.push("example:pillar".into());
}

fn bake_model_loader(&self, _context: &RegistrationContext, event: &mut BakeModelLoaderEvent<'_>) {
    if event.loader != "example:pillar" {
        return;
    }
    // The north face of a 4-pixel post in the middle of the block.
    event.quads.push(ModelLoaderQuad {
        positions: [[0.625, 0.0, 0.375], [0.375, 0.0, 0.375], [0.375, 1.0, 0.375], [0.625, 1.0, 0.375]],
        uvs: [[1.0, 1.0], [0.0, 1.0], [0.0, 0.0], [1.0, 0.0]],
        texture: "side".into(),
        cull: None,
        tint_index: -1,
        shade: true,
    });
}
```

The model's whole JSON object reaches the hook as `event.config` (`{"loader": "example:pillar", "textures":
{"side": "example:block/post"}, ...}`), and `event.model` names it. A quad's `texture` is a key of the model's
`textures`, resolved through its parents as an element face's is. Quads are in block units, the way an obj
model's are, and the blockstate variant's rotation moves them whole. Each model is asked once per load.

A model over 65,536 quads is refused whole, with a reason. A quad with a position or UV that isn't finite, or
a texture key the model doesn't define, is skipped by itself with a reason. A `loader` no mod registered is
logged as an error naming it and the model, which then draws only its elements."#]
    #[hook(
        side = "client",
        contract = "collect",
        out = "loaders",
        context = "registration",
        neoforge = "ModelEvent.RegisterLoaders"
    )]
    fn register_model_loaders();
}
