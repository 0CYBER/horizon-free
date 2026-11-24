//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The client host's methods of the `draw` area, implemented by the game and reached through [`crate::ClientHost`].
#[horizon_mod_api_macros::host_area]
pub trait DrawClientHost {
    /// Side: client.
    /// Fluid `fluid`'s name as the lang names it (`fluid_type.<mod>.<name>`, vanilla's by its block),
    /// else its path in words.
    #[access(ClientSetup)]
    fn fluid_name(&self, fluid: &str) -> String;
    /// Side: client.
    /// Entity model layer `layer` baked into its `ModelPart` tree, each part
    /// at its pose, as `EntityModelSet.bakeLayer` bakes it: one a native mod
    /// registered (`register_layer_definitions`), or vanilla's, by the name
    /// `entity-models.json` gives it (`minecraft:zombie#main`,
    /// `minecraft:player#main`). The tree is the caller's own copy: bake it
    /// once, as a renderer does, and set its parts' poses each frame. None
    /// for a layer neither has. A vanilla layer no mob has (a player's, a
    /// chest's) is read from the resources each call, so slow. API
    /// @API:model_layers@.
    #[access(ClientSetup)]
    fn bake_layer(&self, layer: &str) -> Option<ModelPart>;
    /// Side: client.
    /// The local player's first-person arm appearance: `(sprite, slim)`.
    /// Pass `sprite` directly to [`BlockEntityDrawContext::model_part`]; use
    /// `minecraft:player_slim#main` when `slim` is true, `minecraft:player#main`
    /// otherwise. Both hands use the same appearance.
    /// The sprite falls back to Steve when the player's skin is not in the atlas;
    /// that fallback does not change the arm width. `None` with no local player
    /// or if neither sprite is available. Read it when drawing, since a skin
    /// download or resource reload can change the answer.
    #[access(ClientSetup)]
    fn local_player_arm_skin(&self) -> Option<(&str, bool)>;
}
/// What the HUD, screen and tooltip hooks draw with, in GUI pixels: one per hook call.
pub trait DrawContext {
    /// The GUI's size in GUI pixels, width then height.
    fn size(&self) -> [f32; 2] {
        unimplemented!("DrawContext::size")
    }
    /// Item `item`'s icon drawn with model `model` (`minecraft:paper` as
    /// `mymod:wheel_icon`, an `item_model` of a content mod's), as `native_item`
    /// draws one; false for an unknown item.
    fn item_model(&mut self, item: ItemKey, model: &str, at: [f32; 2], size: f32) -> bool {
        unimplemented!("DrawContext::item_model")
    }
    /// A quad of corners `points` (in order round it) in GUI pixels, `argb`:
    /// a wedge, a slanted bar, what `native_fill`'s rectangles can't draw.
    fn quad(&mut self, points: [[f32; 2]; 4], argb: u32) {
        unimplemented!("DrawContext::quad")
    }
    /// A rectangle in GUI pixels, `argb` (0x78000000 is the extras' box).
    fn fill(&mut self, rect: [f32; 4], argb: u32) {
        unimplemented!("DrawContext::fill")
    }
    /// `GuiGraphics.fillGradient`: rectangle `rect` (`[x, y, w, h]` in GUI pixels, as
    /// [`fill`](Self::fill) takes it) blending from ARGB `top` at `y` to `bottom` at `y + h`.
    /// It follows the current pose and clip ([`push_clip`](Self::push_clip)) as `fill` does.
    /// Zero area, inversion on exactly one axis, or a non-finite coordinate draws nothing.
    /// With both `w` and `h` negative, bounds are sorted and colours flipped, keeping
    /// `top` at `y`, now the lower edge.
    fn fill_gradient(&mut self, rect: [f32; 4], top: u32, bottom: u32) {
        unimplemented!("DrawContext::fill_gradient")
    }
    /// Part `src` (`[x, y, w, h]` in the sprite's own pixels) of sprite
    /// `name` (vanilla's `gui/...`, or a content mod's GUI texture
    /// `mymod:gui/...`), drawn stretched into `dst` (`[x, y, w, h]` in GUI
    /// pixels), every texel multiplied by `tint` (`0xAARRGGBB`, a fluid's colour)
    /// when there is one. False when there is no such sprite.
    fn sprite(&mut self, name: &str, src: [f32; 4], dst: [f32; 4], tint: Option<u32>) -> bool {
        unimplemented!("DrawContext::sprite")
    }
    /// Item `item`'s icon (`mymod:machine`) with its top left at `at`,
    /// `size` GUI pixels square (16 in a slot); false for an unknown item.
    fn item(&mut self, item: ItemKey, at: [f32; 2], size: f32) -> bool {
        unimplemented!("DrawContext::item")
    }
    /// A line of text at `(x, y)` in GUI pixels, in `argb` as vanilla's `GuiGraphics` takes it
    /// (`0xFFFFFFFF` is opaque white). Fully transparent text draws nothing, as vanilla's.
    fn text(&mut self, text: &str, at: [f32; 2], argb: u32, shadow: bool) {
        unimplemented!("DrawContext::text")
    }
    /// Stack drawing does not enforce resident count or damage maxima. Unknown
    /// item identities or unrepresentable component payloads still refuse drawing.
    /// Draw a stack and its slot decorations, except the count, at `at`,
    /// `size` GUI pixels square. False when nothing was drawn: an unknown item
    /// or an empty stack (air, or a count of 0, as `ItemStack.isEmpty`).
    fn item_stack(&mut self, stack: &ItemStack, at: [f32; 2], size: f32) -> bool {
        unimplemented!("DrawContext::item_stack")
    }
    /// Paint GUI extraction calls through this window's atlas, including player faces.
    /// Refuses text deeper than 512 component nodes before applying any changes.
    fn gui_paint(&mut self, draws: &[GuiDraw]) -> Result<(), TextFormError> {
        unimplemented!("DrawContext::gui_paint")
    }
    /// Vanilla's nine-slice widget rendering. `color` is ARGB, as `fill` uses.
    fn gui_nine_slice(&mut self, sprite: &str, rect: [f32; 4], color: u32) {
        unimplemented!("DrawContext::gui_nine_slice")
    }
    /// A vanilla tooltip at the cursor, in GUI pixels.
    /// Refuses text deeper than 512 component nodes before applying any changes.
    fn gui_tooltip(&mut self, lines: &[Text], at: [f32; 2]) -> Result<(), TextFormError> {
        unimplemented!("DrawContext::gui_tooltip")
    }
    /// The width of `text` in GUI pixels, as `native_text` draws it on `draw` (`Font.width`).
    fn text_width(&self, text: &str) -> f32 {
        unimplemented!("DrawContext::text_width")
    }
    /// Fluid `fluid` (`minecraft:water`, `mymod:oil`) drawn as a square with its top left at `at`, `size`
    /// GUI pixels wide (16 in a slot): the sprite its look names (`data/<mod>/horizon/fluid/<name>.json`)
    /// tinted with its colour, else a flat square of the colour, which vanilla water and lava have. What a
    /// recipe viewer draws for a fluid stack; the slot and the amount are the mod's.
    fn fluid(&mut self, fluid: &str, at: [f32; 2], size: f32) {
        unimplemented!("DrawContext::fluid")
    }
    /// Draws `blocks`, a scene of block states at positions of its own (not
    /// the world's), into the box `rect` (x0, y0, x1, y1, GUI pixels) of a
    /// screen's `batch`, seen as `view` says: Create's Ponder scenes. Each
    /// block is drawn with its model, full bright and shaded by face, a face
    /// against a solid neighbour in the scene left out; a block with an
    /// `entity` (a mod's) is drawn by `block_entity_submit` as in the
    /// world too, its `spin` parts turning with the game time. What lies
    /// outside the box is cut off, and the scene has a depth of its own, so
    /// it never covers the screen's other drawing or is covered by it but in
    /// the order drawn. Draw it from a screen's drawing hook
    /// (`screen_draw`). A client's call; on a server it is refused.
    ///
    /// The scene may live too: entities standing in it (`scene.entities`, the
    /// descriptions [`entity`](Self::entity) takes, prepared and drawn by the same
    /// path, sorted with the blocks) and particles (the blocks' own animate ticks,
    /// `SCENE_ANIMATE_PICKS` picks a tick among its blocks, and the mod's), ticked by
    /// a clock the scene owns, kept per calling mod and clock id. A scene of blocks
    /// alone keeps no state. Refused by name ([`SceneError`]): over a cap (16
    /// entities, 512 live particles, 4 animating scenes a frame), an entity
    /// `entity` would refuse, or input that isn't drawable.
    /// API @API:scenes_in_screens@.
    fn draw_scene(
        &mut self,
        rect: [f32; 4],
        scene: &Scene,
        view: SceneView,
    ) -> Result<(), SceneError> {
        Err(SceneError::Unavailable)
    }
    /// Draw a [`Scene`] with additional textured faces in its own coordinates. They
    /// use the scene's projection, scissor and depth along with its blocks, entities
    /// and particles, and do not need or change its clock. Invalid faces refuse the
    /// whole draw before it changes the scene's state.
    fn draw_scene_with_quads(
        &mut self,
        rect: [f32; 4],
        scene: &Scene,
        view: SceneView,
        quads: &[SceneTexturedQuad],
    ) -> Result<(), SceneWithQuadsError> {
        Err(SceneWithQuadsError::Scene(SceneError::Unavailable))
    }
    /// Draw one scene, with rigid poses on selected ranges of its blocks.
    /// Pass sections in increasing block-index order, with nonempty,
    /// nonoverlapping ranges. Blocks outside the ranges stay still. Model and
    /// block-entity geometry move together. Culling against a solid neighbor
    /// only applies within the same section (or among the still blocks), so
    /// pulling a section away exposes both sides. All geometry shares one
    /// depth pass, including untouched entities and particles.
    ///
    /// The poses change drawing only: the clock, block simulation, and
    /// particles' collision grid still use the original cells. Use an empty
    /// section slice for the same draw as [`draw_scene`](Self::draw_scene).
    fn draw_scene_with_sections(
        &mut self,
        rect: [f32; 4],
        scene: &Scene,
        view: SceneView,
        sections: &[SceneSection],
    ) -> Result<(), SceneSectionError> {
        Err(SceneError::Unavailable.into())
    }
    /// `GuiGraphics.enableScissor`: what this hook call draws from now on, until the matching
    /// [`pop_clip`](Self::pop_clip), is cut to `rect` (`[x, y, w, h]` in GUI pixels, as
    /// [`fill`](Self::fill) takes it), the visible part of a sprite, glyph or item kept.
    /// Clips nest: an inner one is cut to the one around it, so it never shows more than that
    /// does. A negative width or height clips everything. What a scrolling list, a scrolling
    /// label or a viewport into a larger page draws through; a scene from
    /// [`draw_scene`](Self::draw_scene) keeps its own box. A clip still open when the hook call
    /// returns is closed then, so it also cuts what later listeners of the same call draw:
    /// close what you open. Refuses a rectangle that is not finite and a 65th open clip,
    /// leaving the clips as they were. A client's call; nothing is clipped on a server.
    fn push_clip(&mut self, rect: [f32; 4]) -> Result<(), GuiClipRefusal> {
        unimplemented!("DrawContext::push_clip")
    }
    /// `GuiGraphics.disableScissor`: closes the clip [`push_clip`](Self::push_clip) opened
    /// last, so what is drawn next is cut only by the clips around it. Refuses with
    /// [`GuiClipRefusal::NoneOpen`] when this hook call has none open.
    fn pop_clip(&mut self) -> Result<(), GuiClipRefusal> {
        unimplemented!("DrawContext::pop_clip")
    }
    /// Draws living entity `entity` standing in `view.rect`, `view.size` GUI
    /// pixels a block, turned as `view.look` says: vanilla's
    /// `InventoryScreen.renderEntityInInventoryFollowsMouse` (`GuiEntityLook::At`
    /// the cursor) and `renderEntityInInventory` (`GuiEntityLook::Turned`), as
    /// a mob page, an entity icon or a data model's mob shows it. It is drawn by
    /// its type's renderer as in the world (a mod's `register_entity_renderers`
    /// renderer and `register_entity_layers` layers included, else the vanilla
    /// model), full bright, lit by `Lighting.ENTITY_IN_UI`'s two lights, with a
    /// depth of its own and cut off at the box. A client's call from a drawing
    /// hook (`screen_draw`, `gui_render_post`); on a server it is refused.
    fn entity(&mut self, entity: &GuiEntity, view: GuiEntityView) -> Result<(), GuiEntityError> {
        Err(GuiEntityError::Unavailable)
    }
    /// `Screen.extractMenuBackground`: draws the tiled menu background (32x32) across `rect`
    /// (`[x, y, w, h]` in GUI pixels). If `in_world` is true, uses the dark translucent
    /// texture (`gui/inworld_menu_background`); otherwise uses the opaque dirt-like
    /// texture (`gui/menu_background`). API @API:screen_transparent_background@.
    fn draw_menu_background(&mut self, rect: [f32; 4], in_world: bool) {
        unimplemented!("DrawContext::draw_menu_background")
    }
    /// `GuiGraphics.blurBeforeThisStratum` (`Screen.extractBlurredBackground`): what this frame's
    /// GUI drew before the call, with the panorama or world under it, is blurred by the player's
    /// menu background blur (`menuBackgroundBlurriness`; nothing at 0), and what is drawn after stays
    /// sharp: a mod's modal over its own page or screen. One boundary a frame, the last call's.
    /// API @API:screen_transparent_background@.
    fn blur_behind(&mut self) {
        unimplemented!("DrawContext::blur_behind")
    }
    /// Repeats sprite `sprite` across `rect` (`[x, y, w, h]` in GUI pixels) with each tile
    /// sized `size` GUI pixels square, tinted by `argb`. False when the sprite is not in the HUD atlas.
    /// API @API:screen_transparent_background@.
    fn tiled_sprite(&mut self, sprite: &str, rect: [f32; 4], size: f32, argb: u32) -> bool {
        unimplemented!("DrawContext::tiled_sprite")
    }
    /// Draw an item stack with custom count/decoration text (or None) at `at` in GUI units.
    /// Uses the same nonresident stack encoding as `item_stack`, not write validation.
    fn gui_draw_item_with(&mut self, stack: &ItemStack, at: [f32; 2], text: Option<&str>) {
        unimplemented!("DrawContext::gui_draw_item_with")
    }
    /// A button as the game's screens draw one (`AbstractButton`): `label` centred on the
    /// `widget/button` sprite (`button_highlighted` when `highlighted`, `button_disabled` and
    /// grey when not `active`), scrolling when it doesn't fit, and recorded for the narrator and
    /// a controller's focus. `rect` is `[x, y, w, h]` in GUI pixels. What
    /// [`crate::client::widgets::Widgets`] draws its buttons with.
    /// Refuses text deeper than 512 component nodes before applying any changes.
    fn widget_button(
        &mut self,
        rect: [f32; 4],
        label: &Text,
        active: bool,
        highlighted: bool,
    ) -> Result<(), TextFormError> {
        unimplemented!("DrawContext::widget_button")
    }
    /// A text field as the game's screens draw one (`EditBox`): its text from the first
    /// character `field.scroll` shows, scrolled on to keep the cursor in view, the selection
    /// highlighted, the caret, the suggestion, the `widget/text_field` sprite when bordered, and
    /// a record for the narrator and a controller's focus. Answers the scroll it drew with and
    /// the x of each shown character boundary, for the next press. What
    /// [`crate::client::widgets::Widgets`] draws its fields with.
    fn widget_text_field(&mut self, field: &TextFieldView) -> TextFieldLayout {
        unimplemented!("DrawContext::widget_text_field")
    }
}
/// What a block entity, mob or player submit hook draws with, in blocks under a pose stack (`PoseStack`): one per hook call.
pub trait BlockEntityDrawContext {
    /// `PoseStack.pushPose`: `pop` comes back to the pose as it is now.
    fn push(&mut self) {
        unimplemented!("BlockEntityDrawContext::draw_push")
    }
    /// `PoseStack.popPose`; the block's own pose is never popped.
    fn pop(&mut self) {
        unimplemented!("BlockEntityDrawContext::draw_pop")
    }
    /// Moves what is drawn next by `by`, in blocks.
    fn translate(&mut self, by: [f32; 3]) {
        unimplemented!("BlockEntityDrawContext::draw_translate")
    }
    /// Turns what is drawn next by `degrees` about `axis` (counterclockwise
    /// seen from the axis' positive end, as `Axis.rotationDegrees`) through
    /// `pivot`, in blocks from the pose's origin: `[0.5; 3]` is the block's centre.
    fn rotate_about(&mut self, axis: [f32; 3], degrees: f32, pivot: [f32; 3]) {
        unimplemented!("BlockEntityDrawContext::draw_rotate_about")
    }
    /// Scales what is drawn next by `by` about the pose's origin.
    fn scale(&mut self, by: [f32; 3]) {
        unimplemented!("BlockEntityDrawContext::draw_scale")
    }
    /// Keeps what this call drew, with neither `block_entity_submit` nor
    /// `block_entity_render_bounding_box` called until its state, block entity,
    /// light or resource packs change. Parts marked with `spin` keep turning on the
    /// animation clock (`animation_time`); other geometry stays as submitted. Do not bake frame-dependent
    /// values into kept vertices: item glint and time-dependent item models stop
    /// where they were, and neighboring blocks do not invalidate the cache.
    fn keep(&mut self) {
        unimplemented!("BlockEntityDrawContext::draw_keep")
    }
    /// The frame's partial tick, 0 to 1 (`BlockEntityRenderState`).
    fn partial(&self) -> f32 {
        unimplemented!("BlockEntityDrawContext::draw_partial")
    }
    /// The level's game time, for animations.
    fn game_time(&self) -> i64 {
        unimplemented!("BlockEntityDrawContext::draw_game_time")
    }
    /// The frame on this client's animation clock, in ticks with the partial tick: what
    /// turns `spin` parts. It counts the ticks this client's level has run (vanilla's
    /// `TextureManager.tick` count): pausing and `/tick freeze` hold it, and the server's
    /// time updates never set it back, unlike `game_time`. Each client counts from its
    /// own start, so it means nothing on the server or to another client.
    fn animation_time(&self) -> f64 {
        unimplemented!("BlockEntityDrawContext::draw_animation_time")
    }
    /// The light the helpers draw with: the block's own
    /// (`LevelRenderer.getLightCoords`), 16 times sky light in bits 0..8 and
    /// 16 times block light in bits 8..16.
    fn light(&self) -> u32 {
        unimplemented!("BlockEntityDrawContext::draw_light")
    }
    /// Lights what is drawn next with `light` (as `native_draw_light` reads it).
    fn set_light(&mut self, light: u32) {
        unimplemented!("BlockEntityDrawContext::draw_set_light")
    }
    /// What is drawn next shows the enchanted armour glint, as an enchanted piece of armour
    /// does (`RenderTypes.armorCutoutNoCullGlint`: `enchanted_glint_armor` scrolling over the
    /// quads' own texture coordinates on their atlas), until `set_glint(false)`: what a mod's
    /// worn armour drawn in an entity layer shows when its stack `has_foil`. It covers quads,
    /// boxes, blocks, models and model parts; items keep their own foil, and text, terrain
    /// models and fluids show none.
    fn set_glint(&mut self, glint: bool) {
        unimplemented!("BlockEntityDrawContext::draw_set_glint")
    }
    /// Draws a quad with `corners` in blocks under `draw`'s pose, in a model
    /// face's order (top left, bottom left, bottom right, top right as seen
    /// from its front), textured with the `uv` rectangle (u0, v0, u1, v1 in
    /// texels of 16 per side) of `sprite` (`mymod:entity/screen`,
    /// `mymod:block/casing`, `block/stone`) times the 0xRRGGBB `tint`
    /// (0xFFFFFF for none). False for an unknown sprite.
    fn quad(&mut self, sprite: &str, corners: [[f32; 3]; 4], uv: [f32; 4], tint: u32) -> bool {
        unimplemented!("BlockEntityDrawContext::draw_quad")
    }
    /// Draws a box from `from` to `to` (texels of 16 per block, as a block
    /// model's element) under `draw`'s pose, with the faces given, by
    /// `Direction` (down, up, north, south, west, east), baked as a model
    /// element's are (`FaceBakery`), times the 0xRRGGBB `tint`. A turned box
    /// is drawn under a turned pose (`BlockEntityDraw::rotate_about`). False
    /// when a face's sprite is unknown; that face is left out.
    fn cube(
        &mut self,
        from: [f32; 3],
        to: [f32; 3],
        faces: [Option<CubeFace<'_>>; 6],
        tint: u32,
    ) -> bool {
        unimplemented!("BlockEntityDrawContext::draw_cube")
    }
    /// Draws block `block`'s model (`mymod:cog`, any block with a model) in
    /// the state with `properties` (`[("axis", "x")]`, the rest default)
    /// under `draw`'s pose, as vanilla's `submitBlockModel` draws a block in
    /// an entity's place: a block's baked model, with its default tints. A
    /// moving part that is no block is a standalone model
    /// (`native_draw_model`). False for an unknown block or property.
    fn block(&mut self, block: BlockKey, properties: &[(&str, &str)]) -> bool {
        unimplemented!("BlockEntityDrawContext::draw_block")
    }
    /// Draws `stack` under `draw`'s pose as an item is drawn in the world
    /// (`ItemModelResolver.updateForTopItem` and `ItemStackRenderState.submit`), centred on the
    /// pose's origin, in its model's `display` transform: `Fixed` as a
    /// campfire's or an item frame's, `Ground` as a dropped item's. `seed`
    /// picks among random models (a campfire passes its slot plus its
    /// position's hash). `owner_yaw` makes the block entity the item's
    /// owner (`ItemOwner`), at the block's centre and turned that many
    /// degrees (`getVisualRotationYInDegrees`), as a shelf is: a compass on
    /// it points to its target from there. None draws it with no owner, as
    /// a campfire does. False for an empty stack or one with no model. The
    /// client's copy of a block entity has no item slots (`entity.items` is
    /// empty there): send the stacks you draw in
    /// `block_entity_update_tag` and read them from `entity.nbt`.
    fn item(
        &mut self,
        stack: &ItemStack,
        display: ItemDisplayContext,
        owner_yaw: Option<f32>,
        seed: i32,
    ) -> bool {
        unimplemented!("BlockEntityDrawContext::draw_item")
    }
    /// What is drawn next turns by itself: `degrees_per_tick` degrees each tick of the
    /// animation clock (`animation_time`; a shaft at `rpm` turns `rpm * 0.3`), about `axis`
    /// through `pivot`, both in blocks from the block's corner whatever the pose,
    /// counterclockwise seen from the axis' positive end. Its angle is
    /// `animation_time * degrees_per_tick + phase`: parts drawn with the same speed keep
    /// their phases apart (a cog meshing with its neighbour). Draw the part at rest; the
    /// game turns it each frame, and with `keep` without asking again until the block
    /// entity changes. With a new speed, pass `old_angle - animation_time() * new_speed`
    /// as the phase to keep the part where it was. Quads drawn while it is active are lit
    /// where they turn to. Until `still` or the next `spin`. A mob's frame draws it at rest.
    fn spin(&mut self, axis: [f32; 3], pivot: [f32; 3], degrees_per_tick: f32, phase: f32) {
        unimplemented!("BlockEntityDrawContext::draw_spin")
    }
    /// What is drawn next stands still again, after `spin`.
    fn still(&mut self) {
        unimplemented!("BlockEntityDrawContext::draw_still")
    }
    /// Draws the resolved standalone model under `draw`'s pose: unturned, from
    /// the block's lower north-west corner, lit by `draw.light`. Faces with a tint
    /// index are multiplied by `tint` (`0xAARRGGBB`, the same as [`crate::WorldQuad::color`];
    /// `0xFFFFFFFF` for none), alpha included. Faces with no tint index stay opaque.
    /// A stale generation or absent slot returns an error without appending geometry.
    fn model(&mut self, model: ModelRef, tint: u32) -> Result<(), ModelError> {
        unimplemented!("BlockEntityDrawContext::draw_model")
    }
    /// Draws the resolved standalone model under `draw`'s pose, as [`Self::model`] places it,
    /// with the terrain's own materials: the block sprites (animated, mipmapped as the options
    /// set), back faces culled, colour and depth written as opaque terrain, lit by
    /// `draw.light` raised by each quad's own emission. `options` picks the material
    /// (each quad's `solid`, `cutout` or `cutout_mipped`, or one for all: cutout discards
    /// under alpha 0.1 without mipmaps, cutout-mipped under 0.5 with them), the directional
    /// shading (independent of light), one sprite drawn as another with moved UVs, a UV
    /// shrink, and a `0xRRGGBB` tint on quads with a tint index. What a machine's belts,
    /// shafts and casings draw with. A refusal draws nothing.
    fn terrain_model(
        &mut self,
        model: ModelRef,
        options: TerrainModelOptions<'_>,
    ) -> Result<(), TerrainModelError> {
        unimplemented!("BlockEntityDrawContext::draw_terrain_model")
    }
    /// Draws `text` under `draw`'s pose as vanilla's `Font.drawInBatch` draws a component in
    /// the world (a sign's line, a text display's): in the pose's x-y plane, one unit a font
    /// pixel, y down, its left end at `at`, facing +z. Scale the pose first as a sign does
    /// (`draw.scale([1.0 / 64.0, -1.0 / 64.0, 1.0 / 64.0])` for its letters, 8 pixels high).
    /// Styles are drawn as name tags and text displays draw them: each span's colour (the
    /// rest in the 0xRRGGBB `color`), bold, italic, underline, strikethrough, obfuscated, its
    /// font, inline objects and `§` codes, right-to-left text in visual order. `glowing` text
    /// is full bright (a glow ink sac's), the rest lit by `light`. With `outline`, a 0xRRGGBB
    /// colour (`Some(0)` is black), each glyph is ringed with it as glowing sign text is
    /// (`Font.drawInBatch8xOutline`; nine glyphs' worth of quads each). Returns the width
    /// drawn, in font pixels (`Font.width`): centre a line by starting it at minus half its
    /// width. Obfuscated glyphs change only when the block is drawn again: kept text stands
    /// still. Clamp text read from NBT, and widen `block_entity_render_bounding_box`
    /// for a long line. API @API:draw_text@.
    /// Refuses text deeper than 512 component nodes before applying any changes.
    fn text(
        &mut self,
        text: &Text,
        at: [f32; 2],
        color: u32,
        glowing: bool,
        outline: Option<u32>,
    ) -> Result<f32, TextFormError> {
        unimplemented!("BlockEntityDrawContext::draw_text")
    }
    /// The width of `text` in font pixels, as `text` draws it, styles included
    /// (`Font.width`), to lay a line out before drawing it.
    /// Refuses text deeper than 512 component nodes.
    fn text_width(&self, text: &Text) -> Result<f32, TextFormError> {
        unimplemented!("BlockEntityDrawContext::draw_text_width")
    }
    /// Draws model part `part` and the parts under it under `draw`'s pose, as
    /// vanilla's `ModelPart.render` does: each part moved to its place (in
    /// texels, 16 to a block), turned about z, then y, then x, and scaled,
    /// its faces textured from `sprite` (`mymod:entity/golem`, a content
    /// mod's `assets/mymod/textures/entity/golem.png`, or vanilla's
    /// `entity/zombie/zombie`) times the 0xRRGGBB `tint`. A part that isn't
    /// `visible` draws nothing, one that skips drawing (`skip_draw`) only its
    /// children. A mob's model is built in the model's own frame (+y down,
    /// its front towards -z, its feet 24 texels down): in `render_living_pre`,
    /// `draw.scale([1.0, -1.0, -1.0])` then `draw.translate([0.0, -1.501,
    /// 0.0])` first, as `LivingEntityRenderer` does. False for an unknown
    /// sprite. API @API:model_layers@.
    fn model_part(&mut self, part: &ModelPart, sprite: &str, tint: u32) -> bool {
        unimplemented!("BlockEntityDrawContext::draw_model_part")
    }
    /// Draws `stack` as a still fluid filling `bounds` (`[x0, y0, z0, x1, y1, z1]` in blocks
    /// under `draw`'s pose), as a tank's contents: `minecraft:water` or `minecraft:lava` with no
    /// components. Each face tiles the fluid's animated still sprite (`block/water_still`), one
    /// tile per block, cut at whole-block lines and keeping each part's place in its tile, with
    /// no inset of the box. The tint is the stack's, not the biome's water colour, unless
    /// `options.argb` replaces it. Sky light stays as `draw.light`; block light rises to the
    /// fluid's own (15 for lava). No face is shaded by direction. Faces are sorted far to
    /// near, alpha blended, culled from behind and write depth, drawn with the block entities
    /// before the level's translucent terrain. A stack of 0 draws nothing and is `Ok` without
    /// checking `bounds`; a refusal draws nothing.
    fn fluid_box(
        &mut self,
        stack: &FluidStack,
        bounds: [f32; 6],
        options: FluidBoxOptions,
    ) -> Result<(), FluidBoxError> {
        unimplemented!("BlockEntityDrawContext::draw_fluid_box")
    }
}
/// What `block_screen_effect_draw` draws with, one per hook call: quads in camera space, in
/// blocks, with `+x` right, `+y` up and the camera at the origin looking down `-z`. It is the
/// space of `ScreenEffectRenderer`'s own quads, whose whole-view quad has corners at `(±1, ±1, -0.5)`,
/// under the first-person hand's projection. Quads draw in the order given, after the level and
/// the hand and before the GUI, alpha-blended with no depth test; give corners in order round the quad.
pub trait CameraDrawContext {
    /// A quad of corners `corners` in solid `argb`. False, with nothing drawn, for a coordinate
    /// that is not finite.
    fn quad(&mut self, corners: [[f32; 3]; 4], argb: u32) -> bool {
        unimplemented!("CameraDrawContext::quad")
    }
    /// A quad of corners `corners` showing sprite `name` of the game's 2D atlas (`block/fire_1`,
    /// `misc/underwater`), with `uv` the texture coordinates of each corner in sprite units (0 to
    /// 1 is the sprite once; beyond that it repeats), every texel multiplied by `argb`. False,
    /// with nothing drawn, when there is no such sprite or a coordinate is not finite.
    fn sprite(&mut self, name: &str, corners: [[f32; 3]; 4], uv: [[f32; 2]; 4], argb: u32) -> bool {
        unimplemented!("CameraDrawContext::sprite")
    }
}
/// Per-view untextured geometry, cleared even without listeners; `usage` reports fixed setup-time limits shared across listeners and depth modes.
pub trait WorldDrawContext {
    /// Draw a retained group once per view, with per-view culling and the batch route rules.
    /// Repeated submissions in one view are ignored. Only visible copies consume its budget.
    /// The shared limit is 16,384 visible copies and 16,384 groups per view. Refusal is atomic.
    /// Shader packs and overlays use the same CPU path as transient model batches.
    /// Retired or foreign groups return [`GroupError::Stale`], even if drawn earlier this view.
    fn draw_instance_group(&mut self, group: InstanceGroupRef) -> Result<(), GroupError> {
        unimplemented!("WorldDrawContext::draw_instance_group")
    }
    /// Submit unlit ARGB world lines atomically. Invalid coordinates or widths,
    /// or a batch exceeding the view's fixed capacity, leave the batch unchanged.
    fn lines(&mut self, lines: &[WorldLine]) -> Result<(), DrawError> {
        unimplemented!("WorldDrawContext::world_draw_lines")
    }
    /// Submit unlit, alpha-blended world quads atomically, using the white atlas texel.
    fn quads(&mut self, quads: &[WorldQuad]) -> Result<(), DrawError> {
        unimplemented!("WorldDrawContext::world_draw_quads")
    }
    /// Submit a beacon's beam atomically, textured and scrolling as a beacon's at this view's
    /// game time, alpha-blended without writing depth. A taller beam than 2048 blocks is cut
    /// there. Its quads count against the same per-view capacity as [`quads`](Self::quads).
    fn beam(&mut self, beam: &WorldBeam) -> Result<(), DrawError> {
        unimplemented!("WorldDrawContext::world_draw_beam")
    }
    /// Submit a line of text atomically, facing the camera as a name tag does, in the game's
    /// font, alpha-blended without writing depth. Its glyph and background quads count against
    /// the same per-view capacity as [`quads`](Self::quads). A scale that is not finite is
    /// refused with [`DrawError::NonFinite`], one that is zero or negative with
    /// [`DrawError::InvalidWidth`].
    fn text(&mut self, text: &WorldText) -> Result<(), DrawError> {
        unimplemented!("WorldDrawContext::world_draw_text")
    }
    /// Submit world quads drawn by the mod pipeline `pipeline`, atomically: its shader gets each
    /// quad's texture coordinate and colour, and its blend and depth modes apply. The quads
    /// count against the same per-view capacity as [`quads`](Self::quads), and within a stage
    /// they draw after its plain quads and lines, in the order submitted. On `Err`, nothing
    /// from this call is drawn.
    fn pipeline_quads(
        &mut self,
        pipeline: RenderPipelineRef,
        quads: &[PipelineQuad],
    ) -> Result<(), PipelineDrawError> {
        unimplemented!("WorldDrawContext::pipeline_quads")
    }
    /// Queued counts and fixed capacities, shared by both depth modes and all listeners.
    /// Capacity is allocated during setup, never grown during submission.
    fn usage(&self) -> WorldDrawUsage {
        unimplemented!("WorldDrawContext::world_draw_usage")
    }
    /// Draw every copy in `instances` of the resolved `model`.
    ///
    /// One batch per model. The engine uploads only the instance records on the opaque
    /// and cutout route, and bakes the same prepared geometry on the CPU when a shader
    /// pack, a copy with an [`Overlay`], or another route cannot take instances. A
    /// `shaders/` pack shades those copies as block entities, with the program that shades
    /// [`BlockEntityDrawContext::model`]. A model with a translucent face is refused with
    /// [`InstanceError::Translucent`] and is never drawn as solid.
    /// Tint is `0xAARRGGBB` and light is [`LightCoords`]: both stay in
    /// vanilla's space, and the engine converts them to the frame. A tint's alpha
    /// multiplies only faces that have a tint index, the same as vanilla's model
    /// renderer. The view budget is 16,384 copies. Pass at most that many: a longer
    /// list is refused with [`InstanceError::CapacityExceeded`] before any copy is
    /// examined. On wasm the guest pays for the list it passes, because the runtime
    /// lifts the slice before that check. Within the bound, copies outside the frustum
    /// are omitted and do not count. On `Err`, nothing from this call is drawn.
    fn draw_model_instances(
        &mut self,
        model: ModelRef,
        instances: &[ModelInstance],
    ) -> Result<(), InstanceError> {
        let _ = (model, instances);
        unimplemented!("WorldDrawContext::draw_model_instances")
    }
}
