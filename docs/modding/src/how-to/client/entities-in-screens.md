# Drawing an entity in a screen

A screen can show a living entity standing in a box, turned toward the cursor or by a turn of its own: a mob
page in a recipe viewer, an entity icon, the mob of a data model. This is vanilla's
`InventoryScreen.renderEntityInInventoryFollowsMouse` and `renderEntityInInventory`. Draw it with
`DrawContext::entity` from a drawing hook (`screen_draw`, `gui_render_post`). Native and wasm mods draw the same way.

```rust,ignore
use mod_api::*;

fn draw_model_mob(draw: &mut dyn DrawContext, origin: [f32; 2], cursor: [f32; 2]) {
    let [x, y] = origin;
    // The inventory's player box: 30 pixels a block, moved down 0.0625 of a block.
    let view = GuiEntityView::new([x + 26.0, y + 8.0, x + 75.0, y + 78.0], 30.0, 0.0625, GuiEntityLook::At(cursor));
    if let Err(refused) = draw.entity(&GuiEntity::kind("minecraft:zombie"), view) {
        log::debug!("no mob drawn: {refused}");
    }
}
```

## Which entity

| `GuiEntity` | What is drawn |
|---|---|
| `GuiEntity::kind(kind)` | a new entity of living type `kind` with its defaults, never added to a level |
| `GuiEntity::with_data(kind, data)` | the same, loaded from `data` as `/summon <kind> ~ ~ ~ <data>` reads it (`{IsBaby:1b}`) |
| `GuiEntity::existing(entity)` | an entity of the client's level as it is this frame (`ClientHost::crosshair_target`'s handle) |

An entity made for a screen is built for drawing only, as a client's copy of a server's entity is: no mod part's
`register_goals` or behaviour registration runs for it, with or without `data`.

The game keeps the last few entities it made from a type and its data, so a screen that draws the same one every
frame makes it once, as a Java screen that keeps its `LivingEntity` does. Drawing allocates nothing once its buffers
have grown, except one copy of an existing entity a frame (vanilla's `createRenderState` makes one too).

## How it is drawn

- **Turned.** `GuiEntityLook::At(point)` turns the body and head toward `point` with vanilla's
  `atan((centre - point) / 40)` angles and tilts the entity with the vertical one. `GuiEntityLook::Turned { yaw, pitch }`
  turns the body `yaw` degrees from facing the viewer and pitches the head; add to `yaw` each frame to spin it.
- **Placed.** The entity's middle is at the box's centre, `size` GUI pixels a block, moved down by `offset_y`
  blocks, and what lies outside the box is cut off.
- **Drawn as in the world.** A mod's renderer for the type ([entity renderers](entity-renderers.md)) draws in place of
  the model, else the type's model; then, either way, the layers mods added ([entity layers](entity-layers.md)). Custom entity
  model animations see `is_in_gui`.
- **Lit as vanilla's.** Full bright, shaded by `Lighting.ENTITY_IN_UI`'s two lights, back faces culled, with a
  depth of its own.

## Refusals

`entity` returns `Err(GuiEntityError)` and draws nothing for:

| Refusal | When |
|---|---|
| `UnknownType` | no living entity type has that id |
| `BadData` | `data` names another type in its `id`, or does not load as the type |
| `NoEntity` | the handle is stale, a server handle, gone, or not a living entity |
| `BadView` | an empty box, a size that is not positive, or a value that is not finite, alone or once combined (a box whose centre or a placement that overflows) |
| `Unavailable` | the drawing context has no game, or its game is a logical server (a server, a test surface) |
