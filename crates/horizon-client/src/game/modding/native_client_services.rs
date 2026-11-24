// game-module-visibility: pub
//! Client draw operations through the existing GUI drawing context.

use super::Game;
use horizon_client::{
    hud::{HudBatch, rgba},
    hud_layout::GuiDraw,
    text::Text,
};
use horizon_items::items::ItemStack as EngineStack;
use mod_api::{TextFieldLayout, TextFieldView};

impl Game<'_> {
    /// Paint GUI extraction calls through this window's atlas, including player faces.
    pub(super) fn native_gui_paint(
        &self,
        batch: &mut HudBatch,
        draws: &[GuiDraw],
        translation: [f32; 2],
    ) {
        horizon_client::hud_layout::paint_translated(batch, &self.gui.atlas, draws, translation);
    }
    /// `GuiGraphics.fillGradient` for [`mod_api::DrawContext::fill_gradient`]: `rect` is
    /// `[x, y, w, h]` and `top` / `bottom` are ARGB at `y` / `y + h`. Zero area,
    /// one-axis inversion or non-finite input draws nothing; both axes inverted flips the colours.
    pub(super) fn native_fill_gradient(
        &self,
        batch: &mut HudBatch,
        rect: [f32; 4],
        top: u32,
        bottom: u32,
    ) {
        let [x, y, w, h] = rect;
        if !rect.iter().all(|v| v.is_finite()) || w == 0.0 || h == 0.0 || (w > 0.0) != (h > 0.0) {
            return;
        }
        let ((x, y, w, h), top, bottom) = if w < 0.0 {
            ((x + w, y + h, -w, -h), bottom, top)
        } else {
            ((x, y, w, h), top, bottom)
        };
        let abgr = horizon_client::hud_layout::abgr;
        batch.fill_gradient(&self.gui.atlas, (x, y, w, h), abgr(top), abgr(bottom));
    }
    /// Vanilla's nine-slice widget rendering. `color` is ARGB, as `fill` uses.
    pub(super) fn native_gui_nine_slice(
        &self,
        batch: &mut HudBatch,
        sprite: &str,
        rect: [f32; 4],
        color: u32,
    ) {
        let [a, r, g, b] = color.to_be_bytes();
        horizon_client::ui::nine_slice(
            batch,
            &self.gui.atlas,
            sprite,
            rect[0],
            rect[1],
            rect[2],
            rect[3],
            rgba(r, g, b, a),
        );
    }
    /// A mod's button, drawn as the game's screens draw theirs (`ui::button`).
    pub(super) fn native_widget_button(
        &self,
        batch: &mut HudBatch,
        [x, y, w, h]: [f32; 4],
        label: &Text,
        active: bool,
        highlighted: bool,
    ) {
        horizon_client::ui::button(
            batch,
            &self.gui.atlas,
            x,
            y,
            w,
            h,
            label,
            active,
            highlighted,
        );
    }
    /// A mod's text field, drawn as the game's screens draw theirs (`ui::EditBox`), from the
    /// state the mod keeps; answers the scroll it drew with and its character boundaries.
    pub(super) fn native_widget_text_field(
        &self,
        batch: &mut HudBatch,
        field: &TextFieldView,
        [x, y, w, h]: [f32; 4],
    ) -> TextFieldLayout {
        let mut edit = horizon_client::ui::EditBox::new(x, y, w, h);
        edit.max_length = field.max_length;
        edit.value = field.value.clone();
        let len = horizon_client::ui::char_len(&edit.value);
        edit.cursor = field.cursor.min(len);
        edit.highlight = field.highlight.min(len);
        edit.set_scroll(field.scroll);
        edit.bordered = field.bordered;
        edit.focused = field.focused;
        let [a, r, g, b] = field.text_color.to_be_bytes();
        edit.text_color = rgba(r, g, b, a);
        edit.suggestion = field.suggestion.clone();
        edit.message = field.message.clone();
        edit.draw_with_caret(batch, &self.gui.atlas, None, field.caret_shown);
        let (scroll, edges) = edit.shown_edges(&self.gui.atlas);
        TextFieldLayout::new(scroll, edges)
    }
    /// A vanilla tooltip at the cursor, in GUI pixels.
    pub(super) fn native_gui_tooltip(&self, batch: &mut HudBatch, lines: &[Text], at: [f32; 2]) {
        horizon_client::ui::tooltip(batch, &self.gui.atlas, lines, (at[0], at[1]));
    }

    /// Draw an item stack with custom count/decoration text (or None) at `at` in GUI units.
    pub(super) fn native_gui_draw_item_with(
        &self,
        batch: &mut HudBatch,
        stack: &EngineStack,
        at: [f32; 2],
        text: Option<&str>,
    ) {
        self.draw_item_model(batch, stack, at[0], at[1]);
        self.draw_item_decorations(batch, stack, at[0], at[1], text.map(str::to_owned));
    }

    /// Vanilla's decoration pass without the icon or another item decoration callback.
    pub(super) fn native_gui_draw_item_decorations_with(
        &self,
        batch: &mut HudBatch,
        stack: &EngineStack,
        at: [f32; 2],
        text: Option<&str>,
    ) {
        if stack.count != 0
            && &*stack.item != "minecraft:air"
            && self.items().get(&stack.item).is_some()
        {
            self.paint_item_decorations(batch, stack, at[0], at[1], text.map(str::to_owned));
        }
    }
    /// Whether a specific key is currently held down.
    /// `key` is the vanilla physical key name (`key.keyboard.left.shift`, for example).
    /// Unknown names are not down. Modifier combinations use `ClientHost::modifiers`.
    pub(super) fn is_key_down(&self, key: &str) -> bool {
        horizon_client::keybinds::key_code(key).is_some_and(|key| self.physical_keys.contains(&key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "daily"]
    fn tooltip_helpers_read_live_keys_and_draw_count_overrides() {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            use winit::keyboard::KeyCode;

            game.physical_keys.insert(KeyCode::ShiftLeft);
            assert!(game.is_key_down("key.keyboard.left.shift"));
            assert!(!game.is_key_down("key.keyboard.right.shift"));
            assert!(!game.is_key_down("not-a-key"));
            game.physical_keys.clear();
            assert!(!game.is_key_down("key.keyboard.left.shift"));

            let stack = EngineStack::new("minecraft:diamond", 5);
            let mut automatic = HudBatch::scratch([320, 240]);
            game.native_gui_draw_item_with(&mut automatic, &stack, [16.0, 32.0], None);
            let mut custom = HudBatch::scratch([320, 240]);
            game.native_gui_draw_item_with(&mut custom, &stack, [16.0, 32.0], Some("7"));
            assert!(!automatic.vertices.is_empty());
            assert_ne!(
                bytemuck::cast_slice::<_, u8>(&automatic.vertices),
                bytemuck::cast_slice::<_, u8>(&custom.vertices),
                "custom count text must reach the actual GUI geometry"
            );
        });
    }

    /// A mod's widgets draw with the game's own: a button as `ui::button` draws it, and a text
    /// field as `ui::EditBox` draws it, answering where each shown character ends so a press
    /// lands on it, and scrolling on to keep the cursor in view.
    #[test]
    #[ignore = "daily"]
    fn mod_widgets_draw_as_the_games_own() {
        use mod_api::client::widgets::{TextField, Widgets};
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let label = Text::literal("Done");
            let mut ours = HudBatch::scratch([320, 240]);
            game.native_widget_button(&mut ours, [10.0, 10.0, 100.0, 20.0], &label, true, true);
            let mut games = HudBatch::scratch([320, 240]);
            horizon_client::ui::button(
                &mut games,
                &game.gui.atlas,
                10.0,
                10.0,
                100.0,
                20.0,
                &label,
                true,
                true,
            );
            assert!(!ours.vertices.is_empty());
            assert_eq!(
                bytemuck::cast_slice::<_, u8>(&ours.vertices),
                bytemuck::cast_slice::<_, u8>(&games.vertices)
            );

            // The view a mod's group sends for a focused field, as `Widgets::draw` builds it.
            struct Draws<'a, 'g>(&'a super::Game<'g>, HudBatch, Vec<TextFieldView>);
            impl mod_api::DrawContext for Draws<'_, '_> {
                fn widget_text_field(&mut self, field: &TextFieldView) -> TextFieldLayout {
                    self.2.push(field.clone());
                    self.0
                        .native_widget_text_field(&mut self.1, field, field.rect)
                }
            }
            let mut widgets = Widgets::new();
            let id = widgets.add_text_field(TextField::new([10.0, 40.0, 60.0, 20.0], 64));
            let long = "a long name that does not fit in sixty pixels";
            widgets.text_field_mut(id).unwrap().set_value(long);
            widgets.set_focus(&*game, Some(id));
            let mut draws = Draws(game, HudBatch::scratch([320, 240]), Vec::new());
            widgets.draw(&mut draws, &*game, [0.0, 0.0]).unwrap();
            let view = draws.2[0].clone();
            assert!(!draws.1.vertices.is_empty());
            let layout =
                game.native_widget_text_field(&mut HudBatch::scratch([320, 240]), &view, view.rect);
            // The cursor is at the end, so the field scrolled on to show it.
            assert!(layout.scroll > 0);
            let shown = layout.edges.len() - 1;
            assert_eq!(layout.scroll + shown, long.chars().count());
            assert_eq!(
                layout.edges[0], 14.0,
                "4 pixels in from a bordered field's left"
            );
            assert!(layout.edges.windows(2).all(|w| w[0] < w[1]));
            assert!(*layout.edges.last().unwrap() <= 10.0 + 60.0 - 4.0);
        });
    }
}
