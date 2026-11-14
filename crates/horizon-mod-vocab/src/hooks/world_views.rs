//! The `world_views` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0349

pub trait Hooks {
    /// Asked once as each frame is built (not on the title screen's panorama): push the world
    /// views (see [`WorldView`]) your mod wants drawn this frame, each parent before the views
    /// seen in it. The world is drawn again from each one's camera into a target of its own.
    #[notes = r#"## World views

A client mod can have the world drawn again from cameras of its own, into targets of its own: a camera monitor, a
second camera's preview beside the player's view, stills for a film strip. Once as each frame is built (not on the
title screen's panorama), before its entities are, `world_views` asks for them, so keep it cheap: push a
`WorldView::new(id, camera, yaw, pitch, fov_degrees, size)` (`with_roll(degrees)` turns it about the view
direction) to the `WorldViews` it hands you for each one wanted this frame. `push` returns `Ok(())` for a view
taken, or a `WorldViewRefusal` naming the view (`id`) and why (`reason`). For a view through a portal or a
mirror:
- `with_clip_plane([x, y, z, d])` draws only what lies in front of that world-space plane (`x * p.x + y * p.y +
  z * p.z + d >= 0`): terrain, entities and block entities behind it are left out, as a wall behind a portal's far
  side is. It becomes the view's near plane, so it needs no device feature, and the camera must be behind it.
  Entities and terrain are cut at the same plane; past it, an entity wins the depth test against terrain up to
  1/1024 of its distance in front of it (about 1 block at 1024 blocks).
- `with_mirror()` mirrors the picture left to right, as a mirror shows it: give the camera reflected through the
  mirror. A mirrored picture turns every triangle's winding round, so the view draws with face culling flipped to
  make up for it: the faces culled are the ones the unmirrored view culls, and back faces stay hidden.
- `with_parent(id)` says the view is seen in the view `id`, a portal's far side seen through another portal.
  Push the parent first, so a cycle cannot be asked for. A view with no parent is at depth 1, one seen in it at
  2, up to `MAX_WORLD_VIEW_DEPTH` (5, Immersive Portals' default `maxPortalLayer`). Views draw deepest first and
  in the order pushed within a depth, so a child's `world_view_drawn` runs before its parent is drawn: in the
  parent's `world_view_drawn`, the child's `color` and `depth` hold this frame's picture, to draw into the
  parent's target, as a view with no parent is drawn into the frame in `world_drawn`. Nothing more is bound: the
  frame encoder's order is the guarantee. Otherwise a child is drawn from its own camera like any view.

Up to `MAX_WORLD_VIEWS` (8) are taken, each at most `MAX_WORLD_VIEW_SIDE` (4096) pixels a side and the device's
largest texture side, and all together at most `WORLD_VIEW_PIXEL_BUDGET` pixels (4 x 4096 x 4096, as much as four
views took before views had parents). Nothing is left out without a word: `push` refuses, by its
`WorldViewRefusalReason`, a camera, yaw, pitch or roll not finite (`NotFinite`), a field of view outside 1 to 170
(`FieldOfView`), a side of 0 or over 4096 (`Side`), a side over the device's (`DeviceLimit`), a clip plane out of
range (`ClipPlane`: a normal `[x, y, z]` of zero, a value or its scaled or camera-relative form not finite, as f32
too, or the camera not behind it), an `id` already pushed this frame (`DuplicateId`), a parent not pushed before
it, refused, or the view itself (`MissingParent`, so a refused view's children are refused too), depth 6
(`TooDeep`; never clamped), a ninth view (`TooMany`) and a view past the pixel budget (`PixelBudget`, with its
pixels and those left) and a view of a level other than the player's own (`Level`: `with_level` names a level
by a handle; `None` and the player's own level draw, an open viewed level, a stale handle, a closed or never open
level are refused, as viewed levels are not drawn yet). Views taken for a frame that turns out to be a title or loading screen's are refused
afterwards (`TitleScreen`). Each refusal is logged once: the first time a view of that `id` is refused for that
reason.

There is no Wire form: `world_views`, `world_view_drawn` and their types are native-only, since the renderer they
come from refuses to build for wasm.

Each view is drawn after the frame's world with the terrain it sees (solid and translucent), the frame's sky colour,
fog and entities (block entities too), from its own camera and with its own frustum; then `world_view_drawn` hands
it back as a `WorldViewDrawn`: its `id`, `parent` and `nesting_depth` (1 for a view with no parent), `color` (`color_format`, the world's light as `WorldDrawn::color` holds it,
before the tone curve, without the hand, the HUD or the frame's post passes), `depth` (Depth32Float, reversed),
`view_proj` (the world relative to the view's `camera`, to its clip space, as the picture and `depth` were drawn:
the clip plane as its near plane, x flipped for a mirror), and the device, queue and the frame's encoder. Draw your
own geometry into it there, sample it (both textures can be bound) or copy it out: `copy_texture_to_buffer` into a
buffer of your own and map that after the frame, for a still. The target stays the view's while each frame asks for
it, so a texture view of it can be kept and shown, and is drawn over every frame; one no frame asks for any more is
freed. Views draw before `world_drawn`, which can put them into the frame's picture.

A view sees what lies in its own frustum within the render distance of its camera: it lists the loaded sections its
frustum takes in itself (the clip plane is its near plane), without the caves, occlusion or faces turned away, and
its translucent sections go back to front from its camera (within one, the quads keep the player camera's order).
The player's camera keeps all of its culling, GPU culling too, so the frame's own picture and draws stay as they
were; each view costs one more pass over the loaded columns and its own draws. The entities and block entities the
frame builds are those the player's camera or a view sees. A view draws no sun, moon, stars, clouds, particles,
weather, block outline or hand, and none of a shader pack's passes. A frame without views costs nothing more: while
no mod listens to `world_views` it is not called.

`python3 tools/run_world_views.py <dir>` (throwaway homes) draws views through the renderer offscreen and checks
that the frame's picture is unchanged, with eight views too, that mirrored views cull the faces unmirrored ones do,
that a clip plane leaves out what is behind it and that a child's picture is this frame's in its parent's
`world_view_drawn`; `--bench` times a frame with no view, one and four."#]
    #[hook(
        side = "client",
        contract = "collect",
        out = "views",
        target = "mod_render::render::world_views",
        neoforge = "none (Horizon-only; NeoForge has no counterpart)"
    )]
    fn world_views();
}
