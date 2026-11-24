# Drawing resolved models

Register standalone models during `standalone_models`, then resolve their `ModelRef`
through `RenderAssets::model` after resources are published. World render hooks receive
`WorldDrawContext`: submit transient copies with `draw_model_instances`, or retain
copies in a group. GUI scenes, mobs and kept/spinning renderers use their
CPU routes; groups are a world-drawing facility, not another renderer.

## Bake-time standalone forms

`standalone_models` contract version 2 keeps `event.models: &mut Vec<String>` for plain
file ids and adds `event.forms: &mut Vec<StandaloneModelForm>`. Both native and wasm
listeners receive `RegistrationContext`, not a runtime `Host`. Forms are attributed
to the registering mod by dispatch:

```rust,ignore
event.forms.push(StandaloneModelForm::Rotated {
    id: "mymod:shaft_east".into(),
    model: "mymod:block/shaft".into(),
    turns: [0, 1, 0],
    uvlock: true,
});
event.forms.push(StandaloneModelForm::Combined {
    id: "mymod:machine".into(),
    parts: vec!["mymod:block/base".into(), "mymod:block/overlay".into()],
});
```

`turns` are negative quarter-turn counts, modulo four, applied X then Y then Z
about the block centre, exactly as blockstate baking. UV locking uses the inverse
face transformation during baking; rotating a draw pose is not equivalent. For a
Y quarter-turn a north wall becomes east but stays upright either way; a north-pointing
arrow on the top turns east without UV locking and retains its world orientation with it.

Combined parts bake unturned in declaration order, retaining each part's positions,
materials and quad metadata in one `ModelRef`. Their particle is the first available
part particle and model-level ambient occlusion is enabled only if every part enables it.
Dependencies, including parents and loader references, load through the normal model
library; they are not implicitly registered as standalone results. If any dependency
cannot load or bake, no part of that form is published and the warning names the
registering mod, even when the result or source has a different namespace.

Result ids follow the same rule as plain ids: a nonempty namespace and path are
required; they need not equal the registering mod id. Plain ids win duplicate result
ids, then the first successfully baked form wins. Registration runs once, but all
forms rebake from current resources on every successful load. Resolve result ids in
`model_baking_completed`; reload invalidates old handles and failed reloads retain them.
This API bakes quads only, not a mod-defined baked result type.


## Terrain materials in a block entity

`draw.model(model, tint)` draws a resolved model as an entity model. A block entity, mob or
player submit hook that wants the terrain's own materials calls
`draw.terrain_model(model, options)` instead. That covers a machine's belts, shafts and
casings, cut out and shaded as blocks are:

```rust,ignore
let belt = TerrainUvShift::new("mymod:block/belt", "mymod:block/belt_scroll", [1.0, 0.5], [0.0, scroll]);
let options = TerrainModelOptions::default()
    .with_material(TerrainMaterial::Cutout)
    .with_uv_shift(belt);
draw.terrain_model(self.belt, options)?;
```

- Faces sample the animated block sprites, cull back faces and write colour and depth like
  opaque terrain. Light is the draw's own, raised by each quad's emission.
- `material`: `Model` keeps each quad's `render_type` (solid, `cutout`, `cutout_mipped`), and
  a translucent quad refuses the draw. `Solid`, `Cutout` (alpha under 0.1 discarded, no
  mipmaps) and `CutoutMipped` (under 0.5, mipmapped) choose one material for all quads.
- `shade`: `Model` follows each quad's `shade` flag, `Off` turns directional shading off, and
  `Entity` shades every quad by the level's entity light directions. Full-bright light never
  turns shading off.
- `uv_shift` draws faces of exactly one sprite with another, their UVs scaled then moved.
  `uv_shrink_ratio` (`0.0..=1.0`, default `0.0`) pulls UVs towards each face's centre first,
  for a model baked with an older `FaceBakery`'s atlas shrink. The engine never guesses it.
- `tint` (`0xRRGGBB`) multiplies only quads with a tint index.
- A stale handle returns `TerrainModelError::Model(ModelError)`. If any quad can't be drawn, the result is
  `TerrainModelError::Refused` and nothing is drawn. That covers a translucent quad under
  `Model`, a missing sprite, a pose that flattens the model, a ratio out of range, and a shift
  whose numbers aren't finite, whose sprites are unknown or whose source sprite no face uses.

## Persistent groups

- `host.create_group(model, capacity)` reserves up to 16,384 copies of a solid model.
  Initial drawn length is zero. Translucent models are refused.
- `group.write(host, first, copies)` atomically overwrites a range and extends drawn
  length if needed. Unwritten slots start as white, full-bright identity copies at
  world origin. Invalid coordinates or light, or a range beyond capacity, change nothing.
- `group.set_len(host, len)` changes drawn length without changing reserved capacity.
  Shrinking to zero draws nothing; growing restores retained copies, including hidden
  slots written earlier.
- `group.set_group_transform(host, transform)` sets an identity-by-default row-major
  3×4 affine pose. For each local vertex the composition is exactly
  `world = origin + group_transform * (copy_transform * local)`.
  The world-space f64 origin is not rotated or scaled. Non-finite pose values change nothing.
- `draw.draw_instance_group(group)` draws the live length once per view. Repeated
  submissions in that view are ignored. Culling uses the composed pose.
- `group.destroy(host)` retires the handle and releases capacity and CPU caches.
  Copies already accepted for a view remain drawn; subsequent calls return `Stale`.

Each mod may reserve 65,536 copies across all its groups; a zero-capacity group
reserves one slot. Destroying a group releases that reservation immediately. A view
accepts 16,384 visible copies shared across transient batches, groups and listeners,
and at most 16,384 group submissions. Culled copies do not consume the visible-copy
budget. Refusal is atomic: no prefix of a refused submission draws.

Groups cache converted records on the CPU. Unchanged groups avoid pose, normal and
tint conversion, but cull and stage visible copies each frame. Every visible
GPU-routed copy uploads each frame; groups do not retain a private GPU buffer.
Frequent whole-group pose changes rebuild every live record and can cost more than
uncached transient copies. Shader-pack selection, missing prepared GPU records and
overlays use the same CPU baking path as transient batches, including tint and light.
There is no simulation-tick work.

Only the creating mod can access a group. A foreign handle returns `GroupError::Stale`,
as does a destroyed group, a group after resource reload or world close, or a group
whose mod was disabled. Re-enabling starts without groups: recreate them from the
newly published model handles. Slot reuse never revives an old handle. Native and
wasm use the same ownership, budgets, lifetime and rendering implementation.

## Still fluids in a block entity

A block entity, mob or player submit hook draws a tank's contents with
`draw.fluid_box(&stack, bounds, options)`, where `bounds` is
`[x0, y0, z0, x1, y1, z1]` in blocks under the draw's current pose:

```rust,ignore
let options = FluidBoxOptions::new(false, 4.0 / 1024.0);
draw.fluid_box(&tank.stack, [0.125, 0.0625, 0.125, 0.875, level, 0.875], options)?;
```

- The stack is `minecraft:water` or `minecraft:lava` with no components. Any other fluid,
  an unresolved one or a component patch is `FluidBoxError::UnsupportedFluid`.
- Each face tiles the fluid's animated still sprite once per block, cut at whole-block lines.
  Each tile keeps its place in the sprite. The box is not inset. `uv_shrink_ratio` is the
  sprite's `TextureAtlasSprite.uvShrinkRatio` (`0.0..=1.0`), and `render_bottom` adds the bottom face.
- The colour is the stack's tint (plain water `0xFF3F76E4`, lava white), never the biome's
  water colour. `with_argb` replaces it, alpha included. Faces have no directional shading.
  Sky light is the draw's own, and block light rises to the fluid's (lava 15).
- The faces use a material of their own: sorted far to near, alpha blended, culled from behind,
  and depth-writing. They draw with the block entities, before the level's translucent terrain,
  under shader packs too.
- A stack of 0 draws nothing and returns `Ok` without checking `bounds`. Non-finite or reversed
  bounds, bounds beyond 1,048,576 blocks, a box of more than 65,536 tiled quads and a ratio out of range are
  `FluidBoxError::InvalidBox`. Missing fluid sprites return
  `FluidBoxError::MissingSprite`. A refusal draws nothing.

## Before and after a living entity draws

Each mob and player in view gets the render hook pair once a frame, NeoForge's
`RenderLivingEvent` and `RenderPlayerEvent`:

1. `render_player_pre(player, who, draw, hidden, canceled)`, for a player only (`who` is its
   profile name and whether it is this window's own);
2. `render_living_pre(entity, draw, hidden, canceled)`, before the entity's model;
3. the model and its vanilla layers, or in their place the renderer a mod registered for its type
   ([entity renderers](entity-renderers.md)), then the layers mods added to its type
   ([entity layers](entity-layers.md));
4. `render_living_post(entity, parts, draw)`;
5. `render_player_post(player, who, parts, draw)`, for a player only.

The order today: the model and the layers are drawn before the post hooks; the entity's name tag, leash, flames
and shadow are drawn in the frame's later passes, after them. The reference draws the leash and the name tag
before the post (and queues the flames and the shadow after it): tracked in arismoko/horizon#3915. A type with a registered
renderer has the same order, the renderer in the model's place, so it changes with arismoko/horizon#3915 too.

`draw` starts at the entity's feet, turned with its body (+z the way it faces, +x its left, +y up,
in blocks), and keeps what each hook draws. Setting `canceled` in a pre hook skips everything else
of that entity this frame: its model (or its type's registered renderer), its layers, its name tag,
shadow, flames, leash and glowing outline, and the post hooks. A mod that draws one entity's model of
its own draws it in `render_living_pre` and cancels; one that draws every entity of a type registers
a renderer for the type instead; one that wants the name tag or the shadow then draws them itself, or doesn't cancel.
Canceling only `render_living_pre` of a player still asks its `render_player_post`. An entity the
frame culls gets neither hook. The game asks once a frame whether any mod listens to the four, and
with none builds nothing for them.

## A player's or humanoid's parts

`render_living_post` and `render_player_post` get a humanoid's parts as vanilla posed them this frame (`parts`,
`HumanoidModel.setupAnim`), read-only, to attach geometry to an arm or the head without
animating it again. Only `HumanoidMobRenderer`'s families have them (players, zombies and their
kin, skeletons, piglins); every other mob gets `None`, a cow included, though its model has a
head and a body. `PosedParts` is a fixed-size snapshot: `get(part)` or `iter()`. It is taken
from the vanilla body's own draw, so a part `render_player_pre` hid (below) has `visible` false.

`render_player_pre` can hide single parts of the vanilla player model instead of replacing all of
it: `hidden.hide(HumanoidPart::Head)`, as a mod drawing its own outer skin layer does.
`render_living_pre` hides parts of any mob's vanilla model the same way, by their names in the model:
`hidden.hide_named("nose")` leaves a villager's nose out, as a level of detail for a far crowd
does. A frame holds at most `MAX_HIDDEN_PARTS` (8) names of at most `MAX_PART_NAME` (24) bytes;
`hide_named` says `HideError::Full` or `HideError::BadName` past that. An unknown name refuses the draw at first use, reporting the mob type and part name.
Typed humanoid parts have separate storage, so all twelve fit alongside the eight names.

- Hiding happens after Skin Customization, for this frame only.
- A hidden part takes its children with it: the hat goes with the head, a sleeve with its arm.
- The body loses the part, and so do the layers drawn on the body's own model, as vanilla's
  `getParentModel()` layers do: a villager's type, profession and level, a spider's eyes. The
  armour, held items and other layers keep every part, and the cape, a layer of its own, stays.

## Drawing the player's arms with a held item

In `render_hand`, call `host.local_player_arm_skin()` to get `(sprite, slim)` for
the local player. The key is the current first-person arm's atlas sprite: pass it
directly to `event.draw.model_part(part, sprite, 0xFFFFFF)`. If the player's skin
isn't in the atlas, the key uses the same Steve fallback as the ordinary arm
without changing `slim`. Skip drawing if the call returns `None`; read it again
when drawing rather than caching the key across frames.

Bake `minecraft:player_slim#main` and `minecraft:player#main` once with `bake_layer`
and choose the layer by `slim`. `event.render.hand` distinguishes the main and off
hand; the main arm is left when `host.gui_options().main_hand_left` is true,
and the off arm is the opposite. Pose the part in the draw context's hand space
(blocks, +x right, +y up, -z away). Return `Flow::Continue` to keep the default
held item in addition to the arm you drew.

## Text and animation in block entities

`BlockEntityDrawContext::text` draws a component under the current pose in font pixels,
with its styles and the frame's light. Its RGB colour is opaque. Glowing text uses full
light; `Some(0)` is a black outline, not an absent outline. `text` and `text_width`
return the styled advance rounded up, as vanilla `Font.width` does. Use half that width
to centre a line. Kept text retains its glyphs, including obfuscated glyph choices,
until it is drawn again. Resource reload invalidates kept geometry.

Spinning parts use the client's animation clock: level-running texture ticks plus the
level's partial tick, not the camera's partial tick or the server's day time. Pausing
holds that clock; `/tick freeze` holds the level partial at 1 and stops texture ticks.
Camera motion during a freeze does not spin a part backwards. Server time updates do
not reset animation. Held-arm, hand, mob and scene draws use this same level partial.

## Copycat faces

Ask `block_sprites` for the material's side faces, then use each face's `texture` and
`quad_on(side, from, to)` to draw it on your box. The result supplies outward-wound
corners and a UV rectangle in texels, preserving the material's rotations and mirrors
and cropping to your box. It returns `None` for a slanted or sheared texture mapping
that cannot be represented by a rectangle; use another drawing route rather than
substituting an upright texture.

Multiply tinted faces by `block_tint_color(state, face.tint_index, pos)` at their world
position. Untinted faces return `None`. Water faces use tint index 0 and the world's
water colour, including resource-pack water colormaps; other water indices return
`None`. This differs from `fluid_box`, whose colour comes from its fluid stack.

## Texture pixels on the CPU

A mod that colours things by the player's own textures without drawing them on the GPU
(a map averaging each block's top, a software renderer) reads them with
`sprite_pixels(&sprites)`. Find the sprites with `block_sprites` or `texture_sprites`;
each answer is RGBA8, rows top to bottom, a square the arrays' edge on each side, so
`rgba.len()` is edge × edge × 4. Resource packs are included. An animated sprite gives
the frame it starts on. A sprite past the arrays' layers gives `None`, and so does every
sprite on a client that loaded no pixels.

The game keeps the pixels compressed once the GPU has them, and each call uncompresses
the whole store once: ask for every sprite you need in one call, and again only when
`block_sprites_version` changes. One call returns at most 256 MiB, the default texture
budget, and returns `Err` without reading anything past that; ask in parts then.
