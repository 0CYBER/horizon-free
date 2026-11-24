//! The game's side of the `draw` area (`horizon-mod-api/src/host/draw.rs`).
use crate::game::{
    Game,
    api_values::TryIntoEngine,
    native_scene::{SceneBlocks, SceneFrame},
};
use mod_api::{prelude::*, *};

/// [`DrawContext`] on the game's `crate::hud::HudBatch`: one per hook call.
pub(crate) struct GameDrawContext<'a, 'g> {
    pub(crate) game: &'a Game<'g>,
    pub(crate) surface: &'a mut crate::hud::HudBatch,
    /// The call's open clips (`push_clip`), closed when the call returns.
    clips: crate::game::gui_clips::GuiClips,
    translations: [[f32; 2]; 9],
    translation_depth: usize,
}
impl<'a, 'g> GameDrawContext<'a, 'g> {
    pub(crate) fn new(game: &'a Game<'g>, surface: &'a mut crate::hud::HudBatch) -> Self {
        Self {
            game,
            surface,
            clips: Default::default(),
            translations: [[0.0; 2]; 9],
            translation_depth: 0,
        }
    }

    fn translated_at(&self, [x, y]: [f32; 2]) -> [f32; 2] {
        let [dx, dy] = self.translation();
        [x + dx, y + dy]
    }

    fn translated_rect(&self, [x, y, w, h]: [f32; 4]) -> [f32; 4] {
        let [x, y] = self.translated_at([x, y]);
        [x, y, w, h]
    }

    fn translated_corners(&self, [x0, y0, x1, y1]: [f32; 4]) -> [f32; 4] {
        let [dx, dy] = self.translation();
        [x0 + dx, y0 + dy, x1 + dx, y1 + dy]
    }
}
impl Drop for GameDrawContext<'_, '_> {
    fn drop(&mut self) {
        self.clips.close_all(self.surface);
    }
}
impl DrawContext for GameDrawContext<'_, '_> {
    fn size(&self) -> [f32; 2] {
        self.game.native_size(&*self.surface)
    }
    fn item_model(&mut self, item: ItemKey, model: &str, at: [f32; 2], size: f32) -> bool {
        let Some(item) = mod_api::__private::item_key_id(item) else {
            return false;
        };
        let at = self.translated_at(at);
        self.game
            .native_item_model(&mut *self.surface, item, model, at, size)
    }
    fn quad(&mut self, points: [[f32; 2]; 4], argb: u32) {
        let points = points.map(|point| self.translated_at(point));
        self.game.native_quad(&mut *self.surface, points, argb)
    }
    fn fill(&mut self, rect: [f32; 4], argb: u32) {
        let rect = self.translated_rect(rect);
        self.game.native_fill(&mut *self.surface, rect, argb)
    }
    fn fill_gradient(&mut self, rect: [f32; 4], top: u32, bottom: u32) {
        let rect = self.translated_rect(rect);
        self.game
            .native_fill_gradient(&mut *self.surface, rect, top, bottom)
    }
    fn sprite(&mut self, name: &str, src: [f32; 4], dst: [f32; 4], tint: Option<u32>) -> bool {
        let dst = self.translated_rect(dst);
        self.game
            .native_sprite(&mut *self.surface, name, src, dst, tint)
    }
    fn item(&mut self, item: ItemKey, at: [f32; 2], size: f32) -> bool {
        let Some(item) = mod_api::__private::item_key_id(item) else {
            return false;
        };
        let at = self.translated_at(at);
        self.game.native_item(&mut *self.surface, item, at, size)
    }
    fn text(&mut self, text: &str, at: [f32; 2], argb: u32, shadow: bool) {
        let at = self.translated_at(at);
        self.game
            .native_text(&mut *self.surface, text, at, argb, shadow)
    }
    fn item_stack(&mut self, stack: &ItemStack, at: [f32; 2], size: f32) -> bool {
        let Ok(stack) = crate::game::api_values::stack_value(stack) else {
            return false;
        };
        let at = self.translated_at(at);
        self.game
            .native_item_stack(&mut *self.surface, &stack, at, size)
    }
    fn gui_paint(&mut self, draws: &[GuiDraw]) -> Result<(), mod_api::text::TextFormError> {
        let draws = draws.try_into_engine(&())?;
        let translation = self.translation();
        self.game
            .native_gui_paint(&mut *self.surface, &draws, translation);
        Ok(())
    }
    fn gui_nine_slice(&mut self, sprite: &str, rect: [f32; 4], color: u32) {
        let rect = self.translated_rect(rect);
        self.game
            .native_gui_nine_slice(&mut *self.surface, sprite, rect, color)
    }
    fn gui_tooltip(
        &mut self,
        lines: &[Text],
        at: [f32; 2],
    ) -> Result<(), mod_api::text::TextFormError> {
        let lines = lines.try_into_engine(&())?;
        let at = self.translated_at(at);
        self.game.native_gui_tooltip(&mut *self.surface, &lines, at);
        Ok(())
    }
    fn text_width(&self, text: &str) -> f32 {
        self.game.native_text_width(&*self.surface, text)
    }
    fn fluid(&mut self, fluid: &str, at: [f32; 2], size: f32) {
        let at = self.translated_at(at);
        self.game.native_fluid(&mut *self.surface, fluid, at, size)
    }
    fn draw_scene(
        &mut self,
        rect: [f32; 4],
        scene: &mod_api::Scene,
        view: SceneView,
    ) -> Result<(), mod_api::SceneError> {
        let blocks = crate::game::api_values::aggregates::scene_blocks(&scene.blocks)?;
        let rect = self.translated_corners(rect);
        self.game.native_draw_scene(
            &mut *self.surface,
            &SceneBlocks::stationary(&blocks),
            scene,
            SceneFrame::new(rect, view),
        )
    }
    fn draw_scene_with_quads(
        &mut self,
        rect: [f32; 4],
        scene: &Scene,
        view: SceneView,
        quads: &[SceneTexturedQuad],
    ) -> Result<(), SceneWithQuadsError> {
        if !self.game.native_window() {
            return Err(SceneWithQuadsError::Scene(SceneError::Unavailable));
        }
        let rect = self.translated_corners(rect);
        let frame = SceneFrame::new(rect, view).with_textured_quads(&self.game.gui.atlas, quads)?;
        let blocks = crate::game::api_values::aggregates::scene_blocks(&scene.blocks)
            .map_err(SceneWithQuadsError::Scene)?;
        self.game
            .native_draw_scene(
                &mut *self.surface,
                &SceneBlocks::stationary(&blocks),
                scene,
                frame,
            )
            .map_err(SceneWithQuadsError::Scene)
    }
    fn draw_scene_with_sections(
        &mut self,
        rect: [f32; 4],
        scene: &Scene,
        view: SceneView,
        sections: &[SceneSection],
    ) -> Result<(), SceneSectionError> {
        if sections.is_empty() {
            return self.draw_scene(rect, scene, view).map_err(Into::into);
        }
        let cells = crate::game::api_values::aggregates::scene_blocks(&scene.blocks)?;
        let blocks = SceneBlocks::posed(&cells, sections)?;
        self.game.native_draw_scene(
            &mut *self.surface,
            &blocks,
            scene,
            SceneFrame::new(rect, view),
        )?;
        Ok(())
    }
    fn push_clip(&mut self, rect: [f32; 4]) -> Result<(), GuiClipRefusal> {
        let rect = self.translated_rect(rect);
        self.clips.push(self.surface, rect)
    }
    fn pop_clip(&mut self) -> Result<(), GuiClipRefusal> {
        self.clips.pop(self.surface)
    }
    fn entity(
        &mut self,
        entity: &GuiEntity,
        mut view: GuiEntityView,
    ) -> Result<(), GuiEntityError> {
        view.rect = self.translated_corners(view.rect);
        if let GuiEntityLook::At(at) = view.look {
            view.look = GuiEntityLook::At(self.translated_at(at));
        }
        self.game
            .native_draw_entity(&mut *self.surface, entity, view)
    }
    fn draw_menu_background(&mut self, rect: [f32; 4], in_world: bool) {
        let rect = self.translated_rect(rect);
        self.game
            .native_draw_menu_background(&mut *self.surface, rect, in_world)
    }
    fn blur_behind(&mut self) {
        self.game.native_blur_behind(&mut *self.surface)
    }
    fn tiled_sprite(&mut self, sprite: &str, rect: [f32; 4], size: f32, argb: u32) -> bool {
        let rect = self.translated_rect(rect);
        self.game
            .native_tiled_sprite(&mut *self.surface, sprite, rect, size, argb)
    }
    fn gui_draw_item_with(&mut self, stack: &ItemStack, at: [f32; 2], text: Option<&str>) {
        let Ok(stack) = crate::game::api_values::stack_value(stack) else {
            return;
        };
        let at = self.translated_at(at);
        self.game
            .native_gui_draw_item_with(&mut *self.surface, &stack, at, text)
    }
    fn widget_button(
        &mut self,
        rect: [f32; 4],
        label: &Text,
        active: bool,
        highlighted: bool,
    ) -> Result<(), mod_api::text::TextFormError> {
        let label = label.try_into_engine(&())?;
        let rect = self.translated_rect(rect);
        self.game
            .native_widget_button(&mut *self.surface, rect, &label, active, highlighted);
        Ok(())
    }
    fn widget_text_field(&mut self, field: &TextFieldView) -> TextFieldLayout {
        let rect = self.translated_rect(field.rect);
        self.game
            .native_widget_text_field(&mut *self.surface, field, rect)
    }

    fn push_translation(&mut self) -> bool {
        let translation = self.translation();
        let Some(next) = self.translations.get_mut(self.translation_depth + 1) else {
            return false;
        };
        *next = translation;
        self.translation_depth += 1;
        true
    }
    fn translate(&mut self, by: [f32; 2]) -> bool {
        let translated = self.translated_at(by);
        if !translated.iter().all(|v| v.is_finite()) {
            return false;
        }
        self.translations[self.translation_depth] = translated;
        true
    }
    fn translation(&self) -> [f32; 2] {
        self.translations[self.translation_depth]
    }
    fn pop_translation(&mut self) -> bool {
        let Some(previous) = self.translation_depth.checked_sub(1) else {
            return false;
        };
        self.translation_depth = previous;
        true
    }
    fn game_time_delta_ticks(&self) -> f32 {
        self.game.clock.game_time_delta_ticks()
    }
    fn hotbar_origin(&self) -> Option<[f32; 2]> {
        self.surface.stock_hotbar_origin
    }
    fn gui_draw_item_decorations_with(
        &mut self,
        stack: &ItemStack,
        at: [f32; 2],
        text: Option<&str>,
    ) {
        let Ok(stack) = crate::game::api_values::stack_value(stack) else {
            return;
        };
        let at = self.translated_at(at);
        self.game
            .native_gui_draw_item_decorations_with(&mut *self.surface, &stack, at, text)
    }
}
/// [`BlockEntityDrawContext`] on the game's `crate::game::native_block_entity_draw::BlockEntityDraw`.
pub(crate) struct GameBlockEntityDrawContext<'a, 'g> {
    pub(crate) game: &'a Game<'g>,
    pub(crate) surface: &'a mut crate::game::native_block_entity_draw::BlockEntityDraw,
}
impl<'a, 'g> GameBlockEntityDrawContext<'a, 'g> {
    pub(crate) fn new(
        game: &'a Game<'g>,
        surface: &'a mut crate::game::native_block_entity_draw::BlockEntityDraw,
    ) -> Self {
        Self { game, surface }
    }
}
impl BlockEntityDrawContext for GameBlockEntityDrawContext<'_, '_> {
    fn push(&mut self) {
        self.game.native_draw_push(&mut *self.surface)
    }
    fn pop(&mut self) {
        self.game.native_draw_pop(&mut *self.surface)
    }
    fn translate(&mut self, by: [f32; 3]) {
        self.game.native_draw_translate(&mut *self.surface, by)
    }
    fn rotate_about(&mut self, axis: [f32; 3], degrees: f32, pivot: [f32; 3]) {
        self.game
            .native_draw_rotate_about(&mut *self.surface, axis, degrees, pivot)
    }
    fn scale(&mut self, by: [f32; 3]) {
        self.game.native_draw_scale(&mut *self.surface, by)
    }
    fn keep(&mut self) {
        self.game.native_draw_keep(&mut *self.surface)
    }
    fn partial(&self) -> f32 {
        self.game.native_draw_partial(&*self.surface)
    }
    fn game_time(&self) -> i64 {
        self.game.native_draw_game_time(&*self.surface)
    }
    fn animation_time(&self) -> f64 {
        self.game.native_draw_animation_time(&*self.surface)
    }
    fn light(&self) -> u32 {
        self.game.native_draw_light(&*self.surface)
    }
    fn set_light(&mut self, light: u32) {
        self.game.native_draw_set_light(&mut *self.surface, light)
    }
    fn set_glint(&mut self, glint: bool) {
        self.game.native_draw_set_glint(&mut *self.surface, glint)
    }
    fn quad(&mut self, sprite: &str, corners: [[f32; 3]; 4], uv: [f32; 4], tint: u32) -> bool {
        self.game
            .native_draw_quad(&mut *self.surface, sprite, corners, uv, tint)
    }
    fn cube(
        &mut self,
        from: [f32; 3],
        to: [f32; 3],
        faces: [Option<CubeFace<'_>>; 6],
        tint: u32,
    ) -> bool {
        self.game
            .native_draw_cube(&mut *self.surface, from, to, faces, tint)
    }
    fn block(&mut self, block: BlockKey, properties: &[(&str, &str)]) -> bool {
        let Some(block) = mod_api::__private::block_key_id(block) else {
            return false;
        };
        self.game
            .native_draw_block(&mut *self.surface, block, properties)
    }
    fn item(
        &mut self,
        stack: &ItemStack,
        display: ItemDisplayContext,
        owner_yaw: Option<f32>,
        seed: i32,
    ) -> bool {
        let Ok(stack) = crate::game::api_values::stack_value(stack) else {
            return false;
        };
        let Ok(display) = display.try_into_engine(self.game.items()) else {
            return false;
        };
        self.game
            .native_draw_item(&mut *self.surface, &stack, display, owner_yaw, seed)
    }
    fn spin(&mut self, axis: [f32; 3], pivot: [f32; 3], degrees_per_tick: f32, phase: f32) {
        self.game
            .native_draw_spin(&mut *self.surface, axis, pivot, degrees_per_tick, phase)
    }
    fn still(&mut self) {
        self.game.native_draw_still(&mut *self.surface)
    }
    fn model(&mut self, model: ModelRef, tint: u32) -> Result<(), ModelError> {
        self.game.native_draw_model(&mut *self.surface, model, tint)
    }
    fn text(
        &mut self,
        text: &Text,
        at: [f32; 2],
        color: u32,
        glowing: bool,
        outline: Option<u32>,
    ) -> Result<f32, mod_api::text::TextFormError> {
        let text = text.try_into_engine(&())?;
        Ok(self
            .game
            .native_draw_text(&mut *self.surface, &text, at, color, glowing, outline))
    }
    fn terrain_model(
        &mut self,
        model: ModelRef,
        options: TerrainModelOptions<'_>,
    ) -> Result<(), TerrainModelError> {
        self.game
            .native_draw_terrain_model(&mut *self.surface, model, options)
    }
    fn text_width(&self, text: &Text) -> Result<f32, mod_api::text::TextFormError> {
        let text = text.try_into_engine(&())?;
        Ok(self.game.native_draw_text_width(&*self.surface, &text))
    }
    fn model_part(&mut self, part: &ModelPart, sprite: &str, tint: u32) -> bool {
        self.game
            .native_draw_model_part(&mut *self.surface, part, sprite, tint)
    }
    fn fluid_box(
        &mut self,
        stack: &FluidStack,
        bounds: [f32; 6],
        options: FluidBoxOptions,
    ) -> Result<(), FluidBoxError> {
        self.game
            .native_draw_fluid_box(&mut *self.surface, stack, bounds, options)
    }
}
/// [`CameraDrawContext`] on the game's `crate::screen_effects::CameraQuads`.
pub(crate) struct GameCameraDrawContext<'a, 'g> {
    pub(crate) game: &'a Game<'g>,
    pub(crate) surface: &'a mut crate::screen_effects::CameraQuads,
}
impl<'a, 'g> GameCameraDrawContext<'a, 'g> {
    pub(crate) fn new(
        game: &'a Game<'g>,
        surface: &'a mut crate::screen_effects::CameraQuads,
    ) -> Self {
        Self { game, surface }
    }
}
impl CameraDrawContext for GameCameraDrawContext<'_, '_> {
    fn quad(&mut self, corners: [[f32; 3]; 4], argb: u32) -> bool {
        self.game
            .native_camera_quad(&mut *self.surface, corners, argb)
    }
    fn sprite(&mut self, name: &str, corners: [[f32; 3]; 4], uv: [[f32; 2]; 4], argb: u32) -> bool {
        self.game
            .native_camera_sprite(&mut *self.surface, name, corners, uv, argb)
    }
}
/// [`WorldDrawContext`] on the game's `crate::world_geometry::WorldGeometry`.
pub(crate) struct GameWorldDrawContext<'a, 'g> {
    pub(crate) game: &'a Game<'g>,
    pub(crate) surface: &'a mut crate::world_geometry::WorldGeometry,
}
impl<'a, 'g> GameWorldDrawContext<'a, 'g> {
    pub(crate) fn new(
        game: &'a Game<'g>,
        surface: &'a mut crate::world_geometry::WorldGeometry,
    ) -> Self {
        Self { game, surface }
    }
}
impl WorldDrawContext for GameWorldDrawContext<'_, '_> {
    fn draw_instance_group(&mut self, group: InstanceGroupRef) -> Result<(), GroupError> {
        self.game.draw_instance_group(self.surface, group)
    }
    fn lines(&mut self, lines: &[WorldLine]) -> Result<(), DrawError> {
        self.game.native_world_draw_lines(&mut *self.surface, lines)
    }
    fn quads(&mut self, quads: &[WorldQuad]) -> Result<(), DrawError> {
        self.game.native_world_draw_quads(&mut *self.surface, quads)
    }
    fn beam(&mut self, beam: &WorldBeam) -> Result<(), DrawError> {
        self.game.native_world_draw_beam(&mut *self.surface, beam)
    }
    fn text(&mut self, text: &WorldText) -> Result<(), DrawError> {
        self.game.native_world_draw_text(&mut *self.surface, text)
    }
    fn pipeline_quads(
        &mut self,
        pipeline: RenderPipelineRef,
        quads: &[PipelineQuad],
    ) -> Result<(), PipelineDrawError> {
        self.game
            .native_world_draw_pipeline_quads(&mut *self.surface, pipeline, quads)
    }
    fn usage(&self) -> WorldDrawUsage {
        self.game.native_world_draw_usage(&*self.surface)
    }
    fn draw_model_instances(
        &mut self,
        model: ModelRef,
        instances: &[ModelInstance],
    ) -> Result<(), InstanceError> {
        self.game
            .native_world_draw_model_instances(&mut *self.surface, model, instances)
    }
}
