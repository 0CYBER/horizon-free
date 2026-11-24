# Entities, particles and textured faces in screen scenes

`DrawContext::draw_scene` draws a `Scene` in a screen, as a guide page or a Ponder scene
shows a structure. A scene of blocks alone is drawn as it always was and keeps nothing. A
scene can also live: mobs standing among its blocks, and particles that live in it. This is
GuideME's living scene: a lit furnace shows its flames and smoke, a campfire smokes, fire
flickers, and dripstone and leaves drip, the way the client's animate tick makes these
particles for blocks near the player.

```rust
use mod_api::{GuiEntity, Scene, SceneClock, SceneEntity, SceneView};

// Blocks alone: no clock, no state.
event.batch.draw_scene(rect, &Scene::new(blocks.clone()), view)?;

// Build a living scene once and keep it. Change only its clock from frame to frame.
let mut scene = Scene::new(blocks)
    .with_clock(SceneClock::new(1, 0, 0.0))
    .with_entities(vec![
        SceneEntity::new(GuiEntity::kind("minecraft:cow"), [1.5, 0.0, 0.5]).turned(135.0, 150.0, 0.0),
    ]);

// In `screen_draw`: the page's own tick count, 20 a second, and the frame's partial tick.
if let Some(clock) = &mut scene.clock {
    clock.tick = page_ticks;
    clock.partial = partial;
}
let view = SceneView::new([1.0, 0.5, 0.5], 225.0, 30.0, 18.0);
if let Err(why) = event.batch.draw_scene([12.0, 20.0, 140.0, 110.0], &scene, view) {
    log::warn!("scene not drawn whole: {why:?}");
}
```

## Entities

A `SceneEntity` holds the same `GuiEntity` that `DrawContext::entity` draws: a new entity
of a living type, with optional saved data (a baby, a sheep's colour), or an entity of the
client's level as it is this frame. It is prepared and drawn by that same path: its type's
renderer as in the world (a mod's `register_entity_renderers` renderer in place of the
vanilla model, then the `register_entity_layers` layers), full bright, shaded by face like
the scene's blocks and sorted with them by depth. Its feet stand at `pos` in the scene's
coordinates, turned by `yaw`, `head_yaw` and `head_pitch` in entity degrees (yaw 0 faces
south).

The entity is made for drawing only. It never joins a level and never ticks, and no mod
part's `register_goals` runs for it. Its age is the clock's tick, so the model's idle
animations follow the scene's clock. A scene takes at most 16 entities.

## The scene's clock and its particles

Entities and particles need a clock. The scene keeps its live particles between draws under
the calling mod and `clock.id`: give each scene on your screens its own id; another mod's
id 1 is another scene.

- **A tick ahead** of the last tick the scene ran: the scene runs the ticks in between. On
  each tick the blocks run their animate ticks (if `animate_blocks` is set: 64 picks among
  the scene's own blocks), then the particles move, collide with the scene's blocks and age.
  The particles in `Scene::particles` are added on the first tick run.
- **More than 20 ticks ahead** (a page resumed): the scene starts over from no particles
  and runs only the last 20 ticks, so a frame never stalls.
- **The same tick** again: the scene is paused. Nothing is added and nothing moves.
- **A tick behind**, or changed blocks or entities: the scene starts over with no
  particles.

Each tick's randomness is seeded from the id and the tick, and the picks cover the scene's
blocks, not the view, so the same ticks always give the same picture from any angle. A
scene holds at most 512 live particles, the blocks' own and the mod's together; past that
no new one is added. The particles are drawn facing the viewer and cut off at the box. The
game's Particles option applies, as it does in the world. Scenes make no sound.

At most four scenes with entities or particles animate in one frame. A fifth still draws
its blocks and its last state, but its clock does not advance, and the call returns
`TooManyScenes`.

## Textured faces

Use `DrawContext::draw_scene_with_quads` to put a sprite on a scene-local face, for
example a fluid surface or a panel. It takes the same `Scene` and `SceneView` as
`draw_scene`, plus a slice of faces. Quads do not need or change the clock, and can
be used in an empty `Scene`. Both sides draw. The same scene box and depth buffer
apply to blocks, entities, particles and faces, so a nearer block can hide a quad.

```rust
use mod_api::{Scene, SceneTexturedQuad};

// A flat surface one block across, cropped to part of the flowing water sprite.
let surface = SceneTexturedQuad::new(
    "minecraft:block/water_flow",
    [[0.0, 0.8, 0.0], [1.0, 0.8, 0.0], [1.0, 0.8, 1.0], [0.0, 0.8, 1.0]],
    [[0.25, 0.25], [0.75, 0.25], [0.75, 0.75], [0.25, 0.75]],
    0xA03F_76E4,
);
draw.draw_scene_with_quads(rect, &Scene::new(blocks), view, &[surface])?;
```

List the four corners in order around the face, in blocks. Give each corner its own
normalized UV in the sprite (`[0, 0]` at its top left, `[1, 1]` at its bottom right).
The tint multiplies the sampled RGBA, including alpha; use `0xFFFF_FFFF` for no tint.
Use a sprite from the scene's existing GUI atlas, such as `block/water_still`,
`minecraft:block/water_overlay` or `mymod:gui/panel`. The atlas uses the player's
resource packs and updates animated sprites; no separate texture upload is needed.

`SceneWithQuadsError::BadTexturedQuad { index }` means a non-finite or out-of-range
corner (coordinates must be within 30 million blocks on each axis) or a UV outside
`0..=1`. `UnknownSprite { index }` means no sprite by that name is packed.
`TooManyTexturedQuads` means more than `SCENE_MAX_TEXTURED_QUADS` (4096) faces. These
errors draw nothing and leave the scene's state unchanged. Other scene refusals come
back wrapped in `SceneWithQuadsError::Scene`; for `TooManyScenes` the textured faces
draw along with the blocks and whatever last state is available, without advancing
the clock. The existing `draw_scene` call is unchanged.

## Refusals

When `draw_scene` refuses a scene, it draws none of it (but for `TooManyScenes`) and
returns a `SceneError`:

| Error | Cause |
|---|---|
| `Unavailable` | Not a client's drawing |
| `BadView` | A view or box that isn't finite, or a scene too large to stay in finite GUI coordinates |
| `TooManyBlocks` | More than `SCENE_MAX_BLOCKS` (4096) blocks |
| `TooManyEntities` | More than `SCENE_MAX_ENTITIES` (16) entities |
| `TooManyParticles` | More than `SCENE_MAX_PARTICLES` (512) particles to add at once |
| `TooManyScenes` | Four other scenes animated this frame; the blocks and last state were drawn |
| `BadBlock { index }` | A block state the game doesn't have |
| `UnknownType { index }` | No living entity type by that id, as `DrawContext::entity` refuses it |
| `BadData { index }` | Entity data naming another type, or that doesn't load |
| `NoEntity { index }` | A handle naming no living entity of the client's level |
| `BadEntity { index }` | A position or turn that isn't finite, or a position more than 30 million blocks out |
| `BadParticle { index }` | Particle options, a position or a velocity that isn't finite, or options that name a stack |
| `BadClock` | A partial tick that isn't finite |
| `NoClock` | Entities or particles without a clock |

## Not yet

- Additive (glowing) particle quads and pack (Polytone) particles are not drawn in scenes.
- A scene's blocks have no light, so a block whose animate tick reads light behaves as
  it would in the dark.
