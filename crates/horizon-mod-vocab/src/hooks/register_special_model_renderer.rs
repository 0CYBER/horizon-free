//! The `register_special_model_renderer` hook: declarations the mod API generator reads (see
//! `hooks/mod.rs`).

pub trait Hooks {
    /// The special item model types a mod draws (NeoForge's `RegisterSpecialModelRendererEvent`):
    /// `register(type, renderer)`, and every `minecraft:special` item model whose `model` object
    /// has the type `type` (`mymod:skillet`) is drawn by `SpecialModelRenderer::extract`. A later
    /// registration of a type replaces the earlier one, vanilla's types included, and the log
    /// names both mods. Asked once at start on the client, before resources load; a special
    /// model of a type no mod registered and vanilla doesn't have is an error naming it, and its
    /// item draws the missing model.
    #[notes = r#"## Special item model renderers

A special item model (vanilla's `minecraft:special`, NeoForge's `RegisterSpecialModelRendererEvent`) draws an
item with code instead of one flat model: a skillet with what it cooks, a tool with moving parts.
`Hooks::register_special_model_renderer` registers one renderer per special model type once at start, before
resources load. The geometry stays in the packs: from the stack, the renderer answers which resource models
draw, each posed and tinted, and the game draws them as it draws any item model.

```rust,ignore
struct Skillet;
impl SpecialModelRenderer for Skillet {
    fn extract(
        &self,
        _host: &dyn ClientHost,
        stack: &ItemStack,
        _config: &Tag,
    ) -> Result<Vec<SpecialModelPart>, String> {
        let mut parts = vec![SpecialModelPart::new("example:item/skillet_pan")];
        if stack.count() > 1 {
            // The food sits a pixel above the pan.
            parts.push(SpecialModelPart::new("example:item/skillet_food").translated([0.0, 0.0625, 0.0]));
        }
        Ok(parts)
    }
}

fn register_special_model_renderer(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterSpecialModelRendererEvent<'_>,
) {
    event.renderers.register("example:skillet".into(), Arc::new(Skillet));
}
```

```json
{"model": {"type": "minecraft:special", "base": "example:item/skillet",
  "model": {"type": "example:skillet"}}}
```

`config` is the `model` object, converted to NBT once at load, so a type reads its own fields from it. Each
part names a model file of the packs, a row-major 3×4 transform in the special model's space (blocks, with the
origin at the model's corner, as an element's coordinates over 16) and its tints per tint index (ARGB, as an
item model's `tints`). The renderer never sees the `base` model: the game draws the parts with its display
transforms, particle and GUI light, as vanilla does, and the node's `transformation` applies to all of them.

The renderer is asked on the client's main thread when an item of such a model is drawn with a stack other
than the last one that model was asked about: an unchanged stack is asked once, however many frames draw it,
and every display context (a hand, a screen, the ground, a frame, a head) draws that answer. The same for
native and wasm mods: a wasm renderer is one call for each new stack, the stack copied in and the parts
copied out.

A later registration of a type replaces the earlier one, whichever mod made it and vanilla's types included,
as in NeoForge; the log names both mods. An id that is not `<namespace>:<path>` is logged and left out. A
special model of a type no mod registered and vanilla doesn't have is an error naming the type and the item:
that item draws the missing model, and the rest of the resources load. A renderer that returns an `Err`,
panics, answers more than `MAX_SPECIAL_MODEL_PARTS` (256) parts or a transform that isn't finite, or (in a
wasm mod) has its call refused or trapped, is logged once with its type and asked no more until resources
reload; its special models draw the missing model until then. A part naming a model no pack has makes that
answer draw the missing model, logged once for the model."#]
    #[hook(
        side = "client",
        contract = "listen",
        context = "registration",
        neoforge = "RegisterSpecialModelRendererEvent"
    )]
    fn register_special_model_renderer();
}
