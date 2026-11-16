# Drawing an entity type with your own renderer

You can draw content-defined thrown entities and living entity types (mobs, vanilla's or a content
mod's) with your own renderer. This is the counterpart of
NeoForge 26.3's `EntityRenderersEvent.RegisterRenderers` (`registerEntityRenderer`): the mod
registers one renderer per entity type at start, and the game hands each entity of that type in
view to it, once a frame. Native and wasm mods register and draw the same way.

## Write the renderer

A renderer implements `EntityRenderer`. Its `submit` gets the entity's frame and a draw context, the
same one a block entity or mob submit hook draws with (`quad`, `cube`, `block`, `item`,
`model_part`, `push`, `pop`, `translate`, `rotate_about`, `scale`):

```rust,ignore
use mod_api::*;
use std::sync::Arc;

struct Bolt;
impl EntityRenderer for Bolt {
    fn submit(
        &self,
        entity: &EntityRenderState,
        draw: &mut dyn BlockEntityDrawContext,
    ) -> Result<(), String> {
        // The pose starts at the entity, not turned: face the way it flies.
        draw.rotate_about([0.0, 1.0, 0.0], -entity.yaw, [0.0; 3]);
        draw.rotate_about([1.0, 0.0, 0.0], entity.pitch, [0.0; 3]);
        let face = Some(CubeFace::new("example:entity/bolt"));
        draw.cube([-1.0, -1.0, -4.0], [1.0, 1.0, 4.0], [face; 6], 0xFFFFFF);
        Ok(())
    }
}
```

| `EntityRenderState` | Meaning |
| --- | --- |
| `id` | The entity's network id, as mod messages name it. |
| `partial` | The frame's partial tick, 0 to 1. |
| `age` | Ticks lived plus the partial tick (`ageInTicks`). |
| `yaw`, `pitch` | The way it faces, in degrees, interpolated to the frame (0 is south, positive pitch looks down). |
| `living` | A living entity's own frame (`LivingRenderState`, below); none for any other entity. |

The pose stack starts at the entity's interpolated position, in blocks, and is not turned. `draw`
also has the partial tick, the game time and the entity's light. What `submit` draws replaces the
type's own drawing; the glowing outline draws around it. Every entity of the type shares the
one renderer, so keep per-entity values out of it: read them from the state. A renderer has no
`Host`; one that needs the world reads it in a hook and keeps what it read. A frame is drawn afresh,
so `draw.keep()` does nothing here.

### A living entity's frame

A living entity's state has `living: Some(LivingRenderState)`, NeoForge's `LivingEntityRenderState`
under the same names. Angles are in degrees, interpolated to the frame.

| `LivingRenderState` | Meaning |
| --- | --- |
| `body_rot` | The way the body faces (0 is south, 90 west). |
| `y_rot`, `x_rot` | The head's turn from the body and its pitch (both flipped when `is_upside_down`). |
| `death_time` | Ticks since it died plus the partial tick; 0 while alive. |
| `walk_animation_pos`, `walk_animation_speed` | The walk cycle: how far the legs have swung, and how fast (0 to 1); 0 riding or dead. |
| `scale`, `age_scale` | Its `minecraft:scale`, and 0.5 for a baby (else 1). |
| `is_upside_down`, `is_baby`, `is_in_water`, `has_red_overlay` | Named Dinnerbone or Grumm; a baby; in water; hurt or dying. |
| `pose` | Its stance (`Pose::Standing`, `Pose::Sleeping`, `Pose::Crouching`...). |
| `bed_orientation` | The way the bed it sleeps in faces, while it sleeps in one. |

A living renderer replaces the type's whole renderer: its model and its vanilla layers (armour, held
items, a worn head), as a NeoForge `LivingEntityRenderer` does. Vanilla's turn, scale and death
tip-over aren't applied for it: the pose stack starts at the feet, not turned, so the renderer turns by
`body_rot` and scales by `scale` itself. Around it the entity keeps everything else:
`render_living_pre` (a cancel skips the renderer), the layers mods added to the type, with no posed
parts ([entity layers](entity-layers.md)), `render_living_post`, and the frame's later passes (name
tag, leash, flames, shadow), in the order the type's own model has ([model drawing](model-drawing.md)).

## Register it

```rust,ignore
fn register_entity_renderers(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterEntityRenderersEvent<'_>,
) {
    event.renderers.register("example:bolt".into(), Arc::new(Bolt));
}
```

The hook is asked once at start on the client, before resources load. The game keeps the
registrations and resolves each type's renderer on every resource load, the first one and every
reload (F3+T, a resource pack change), so a frame dispatches by slot, with no lookup by name and no
scan over the mods.

- A later registration for a type replaces the earlier one silently, whichever mod made it, as in
  NeoForge, also for a vanilla type.
- A type the game does not know is logged with its id and left out; nothing panics.
- `submit` returns `Ok(())` to keep what it drew. A renderer that returns an `Err`, panics, or (in a
  wasm mod) has its call refused or trapped is logged once and asked no more until the next resource
  load (a reload resolves it again); what it drew that frame is dropped, and until then its type draws
  as it would without one.

## What draws through registered renderers

Registered renderers draw two routes:

- content-defined thrown types: a `data/<id>/horizon/entity_type/<name>.json` using the
  `minecraft:snowball` template ([content mods](../content-mods.md)) draws the renderer's geometry
  instead of its flying item. Without a renderer, it draws its item;
- living types drawn as mobs, vanilla's (`minecraft:zombie`) and the content mods' living types: the
  renderer draws instead of the model and its vanilla layers.

Registrations for other known entity families, including boats, displays, vanilla projectiles, the
player (whose model goes by its skin), the armour stand and the ender dragon (which draw through
routes of their own), are retained without changing their drawing.

To draw over a living renderer instead of replacing it, add a layer
([entity layers](entity-layers.md)).

To draw an entity of any type in a screen, through its renderer, see [entities in screens](entities-in-screens.md).
