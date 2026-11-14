# Drawing an item with a special model renderer

You can draw an item from several resource models chosen from its stack: a skillet with what it
cooks, a tool with moving parts. This is the counterpart of NeoForge 26.3's
`RegisterSpecialModelRendererEvent`: the mod registers one renderer per special model type at start,
before resources load, and every `minecraft:special` item model of that type is drawn by it. Native
and wasm mods register and draw the same way.

The geometry stays in the packs. The renderer doesn't make quads: it answers which model files to
draw, each with a transform and tints, and the game draws them as it draws any item model.

## Write the renderer

A renderer implements `SpecialModelRenderer`. Its `extract` gets a `ClientHost`, the stack and the
special model's `model` object (`config`, its JSON converted to NBT once at load), and answers the
parts:

```rust,ignore
use mod_api::*;
use std::sync::Arc;

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
            // The food sits one pixel above the pan.
            parts.push(
                SpecialModelPart::new("example:item/skillet_food").translated([0.0, 0.0625, 0.0]),
            );
        }
        Ok(parts)
    }
}

#[mod_api::horizon_mod]
impl Hooks for MyMod {
    fn register_special_model_renderer(
        &self,
        _context: &RegistrationContext,
        event: &mut RegisterSpecialModelRendererEvent<'_>,
    ) {
        event.renderers.register("example:skillet".into(), Arc::new(Skillet));
    }
}
```

The item definition names the type in a `minecraft:special` model, with a `base` model:

```json
{"model": {"type": "minecraft:special", "base": "example:item/skillet",
  "model": {"type": "example:skillet"}}}
```

## What the parts are

- `model` is a model file of the packs (`example:item/skillet_pan`), with `elements` and textures as
  any item model has.
- `transform` is a row-major 3×4 affine transform in the special model's space: blocks, with the
  origin at the model's corner, so an element's coordinates over 16. `translated` moves a part after
  its transform; `with_transform` sets the whole of it.
- `tints` are ARGB colours per tint index, as an item model's `tints`: packed `0xAARRGGBB` in
  vanilla's space, which the game decodes once, as it does vanilla's tints. A face with a tint index
  past them, or none, draws white. A colour brighter than white (a linear value above 1) has no input
  yet.

The renderer never sees the `base` model: the game draws all the parts with its display transforms
(`display` for a hand, the GUI, the ground, a frame, the head), its particle and its GUI light, as
vanilla's special models do. The node's own `transformation` applies to every part.

## When it is asked

The renderer is asked on the client's main thread when an item of such a model is drawn with a
stack that model didn't draw in the last frame. An unchanged stack is asked once, however many
frames draw it in a row and however many stacks share the model, and every display context (a hand,
a screen, the ground, a frame, a head, another player's hand) draws that answer. A stack that leaves
the screen, or changes, is asked again when it is next drawn; a resource reload drops every answer.

## Registrations and refusals

- A later registration of a type replaces the earlier one, whichever mod made it and vanilla's types
  included, as in NeoForge; the log names both mods. An id that is not `<namespace>:<path>` is logged
  and left out.
- A special model of a type no mod registered and vanilla doesn't have is an error naming the type and
  the item: that item draws the missing model, and the rest of the resources load.
- A renderer that returns an `Err`, panics, answers more than `MAX_SPECIAL_MODEL_PARTS` (256) parts
  or a transform that isn't finite, or (in a wasm mod) has its call refused or trapped, is logged once
  with its type and asked no more until resources reload. Its special models draw the missing model
  until then; nothing it answered is drawn in part.
- A part naming a model no pack has makes that answer draw the missing model, logged once for the
  model.

Renderer answers depend only on stack and configuration. Host-state changes do not invalidate a continuously drawn unchanged stack. Answers drop after all views stop drawing the stack or resources reload.
