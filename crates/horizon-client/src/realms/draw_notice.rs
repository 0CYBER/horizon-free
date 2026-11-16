use super::*;

impl Screen {
    /// `RealmsMainScreen.NotificationMessageEntry`: the info sprite, the message centred in
    /// the 220 the 40-wide margins leave, the dismiss cross and the open-link button.
    /// Returns the entry's height, which moves the servers below it.
    pub(super) fn draw_notice(
        &self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        cursor: (f32, f32),
        rx: f32,
        top: f32,
        bottom: f32,
    ) -> f32 {
        let Some(notification) = self.notice() else {
            self.notice_height.set(0.0);
            return 0.0;
        };
        let lines = self.notice_lines(atlas);
        // `MultiLineTextWidget`'s line height is 9 (`wordWrapHeight`).
        let message_height = lines.len() as f32 * 9.0;
        // `addEntriesForNotification`: HEIGHT_WITHOUT_TEXT 38 plus the wrapped message.
        let height = 38.0 + message_height;
        let y = top + 2.0;
        self.notice_height.set(height);
        // The list scrolled the entry out of view: nothing draws, nothing clicks.
        if y + height <= top || y >= bottom {
            self.notice_rects.set([[0.0; 4]; 2]);
            return height;
        }
        // The grid starts at the entry's content origin: the entry's own +2.
        let content_x = rx + 2.0;
        // `ImageWidget.sprite(20, 20, icon/info)`, padded 7.
        crate::ui::blit_sprite(
            batch,
            atlas,
            "gui/sprites/icon/info",
            content_x + 7.0,
            y + 7.0,
            20.0,
            20.0,
            u32::MAX,
        );
        // Row 0's height is the icon's 27 or the text frame's 7 + message; the text is
        // centred in `textWidth` (300 - 80) from the content's 40.
        let row_height = 27.0_f32.max(7.0 + message_height);
        for (i, line) in lines.iter().enumerate() {
            let width = HudBatch::text_width(atlas, line);
            batch.text(
                atlas,
                line,
                content_x + 40.0 + ((ROW_WIDTH - 80.0 - width) / 2.0).floor(),
                y + 7.0 + i as f32 * 9.0,
                u32::MAX,
                true,
            );
        }
        // `VisitUrl.buildOpenLinkButton`: 150 wide, 4 below row 0, centred in the text
        // frame's 220.
        let button = [
            content_x + 40.0 + ((ROW_WIDTH - 80.0 - 150.0) / 2.0).floor(),
            y + row_height + 4.0,
            150.0,
            20.0,
        ];
        let label = Text::literal(notification.visit_button_text());
        let hovered = crate::ui::hovered(cursor, button[0], button[1], button[2], button[3]);
        crate::ui::button(
            batch, atlas, button[0], button[1], button[2], button[3], &label, true, hovered,
        );
        // `CrossButton`: 14x14, padded 7 from the content's right and top.
        let dismiss = [content_x + ROW_WIDTH - 7.0 - 14.0, y + 7.0, 14.0, 14.0];
        if notification.dismissable {
            let hovered =
                crate::ui::hovered(cursor, dismiss[0], dismiss[1], dismiss[2], dismiss[3]);
            let sprite = if hovered {
                "gui/sprites/widget/cross_button_highlighted"
            } else {
                "gui/sprites/widget/cross_button"
            };
            crate::ui::blit_sprite(
                batch,
                atlas,
                sprite,
                dismiss[0],
                dismiss[1],
                dismiss[2],
                dismiss[3],
                u32::MAX,
            );
        }
        self.notice_rects.set([button, dismiss]);
        self.notice_height.set(height);
        height
    }
}
