//! A server's dialogs bound to the game (`ClientCommonPacketListenerImpl.
//! showDialog` and `clearDialog`, `DialogScreen`, its `WarningScreen` and the
//! `WaitingForResponseScreen`): the screen over whatever is open (the
//! connecting screen in configuration), its inputs, and the actions its
//! buttons send back.
use super::Game;
use horizon_client::{
    Result,
    confirm_screen::Confirm,
    dialog::{AfterAction, Click, Dialog, InputValue},
    dialog_screen::{Layout, Part, WARNING_SPRITES, waiting_button},
    hud::{HudBatch, rgba},
    text::Text,
    ui,
};
use horizon_world::nbt::Tag;
use std::time::Instant;
use winit::keyboard::KeyCode;

/// An open `DialogScreen`.
struct Open {
    dialog: Dialog,
    values: Vec<InputValue>,
    scroll: ui::Scroll,
}

/// The dialog screens a server can open.
#[derive(Default)]
pub(super) struct DialogState {
    open: Option<Open>,
    /// `WaitingForResponseScreen`, since when.
    waiting: Option<Instant>,
    /// `DialogScreen.WarningScreen`, over the dialog.
    warning: bool,
}

fn warning_texts() -> (Text, Text, [Text; 2]) {
    let t = |key: &str| Text::translate(key, &[]);
    (
        t("menu.custom_screen_info.title"),
        t("menu.custom_screen_info.contents"),
        [t("menu.disconnect"), t("gui.back")],
    )
}

impl Game<'_> {
    /// Whether a dialog screen takes the input.
    pub(super) fn dialog_open(&self) -> bool {
        let d = &self.dialogs;
        d.open.is_some() || d.waiting.is_some() || d.warning
    }

    /// `showDialog`: the dialog replaces an open one (and a waiting screen);
    /// under the warning screen, it is what Back returns to.
    pub(super) fn show_dialog(&mut self, tag: &Tag) {
        let Some(dialog) = Dialog::parse(tag) else {
            log::warn!("Horizon: failed to show dialog for data {tag:?}");
            return;
        };
        let values = dialog.inputs.iter().map(|i| i.control.initial()).collect();
        self.dialogs.open = Some(Open {
            dialog,
            values,
            scroll: ui::Scroll::default(),
        });
        self.dialogs.waiting = None;
        self.capture(false);
    }

    /// `clearDialog`: the dialog closes (under the warning screen, Back then
    /// returns to what was open before it).
    pub(super) fn clear_dialog(&mut self) {
        if self.dialogs.open.take().is_some() && !self.dialog_open() {
            self.dialog_closed();
        }
    }

    /// Every dialog screen goes (a disconnect, or the level loading screen).
    pub(super) fn close_dialogs(&mut self) {
        self.dialogs = DialogState::default();
    }

    fn dialog_closed(&mut self) {
        if self.player.is_some() && self.screen == super::Screen::Playing && self.title.is_none() {
            self.capture(true);
        }
    }

    /// `DialogScreen.runAction`: the click event, then the screen the dialog's
    /// `after_action` leaves.
    fn run_dialog_action(&mut self, click: Option<Click>, after: AfterAction) -> Result<()> {
        match after {
            AfterAction::None => {}
            AfterAction::Close => self.dialogs.open = None,
            AfterAction::WaitForResponse => {
                self.dialogs.open = None;
                self.dialogs.waiting = Some(Instant::now());
            }
        }
        match click {
            Some(Click::Custom { id, payload }) => {
                if let Some(r) = &self.remote {
                    r.session.send_custom_click(&id, payload);
                }
            }
            Some(Click::RunCommand(command)) => {
                // In configuration there is nothing to run it (vanilla warns).
                if self.player.is_some() {
                    let command = command.strip_prefix('/').unwrap_or(&command);
                    self.send_chat(&format!("/{command}"));
                } else {
                    log::warn!(
                        "Horizon: commands are not supported in configuration, trying to run '{command}'"
                    );
                }
            }
            Some(Click::ShowDialog(dialog)) => self.show_dialog_ref(&dialog),
            Some(Click::CopyToClipboard(value)) => {
                use horizon_client::clipboard::Clipboard;
                self.clipboard.set(&value);
            }
            Some(other) => log::info!("Horizon: dialog action {other:?} is not supported yet"),
            None => {}
        }
        if !self.dialog_open() {
            self.dialog_closed();
        }
        Ok(())
    }

    /// A `show_dialog` click event's dialog: a registry id or the dialog itself.
    fn show_dialog_ref(&mut self, dialog: &Tag) {
        match dialog {
            Tag::String(id) => {
                let id = id.to_string_lossy();
                let name = if id.contains(':') {
                    id.clone()
                } else {
                    format!("minecraft:{id}")
                };
                let found = self.remote.as_ref().and_then(|r| {
                    let entries = r.registries.registries.get("minecraft:dialog")?;
                    entries.iter().find(|(n, _)| *n == name)?.1.clone()
                });
                match found {
                    Some(tag) => self.show_dialog(&tag),
                    None => log::warn!("Horizon: unknown dialog {id}"),
                }
            }
            tag => self.show_dialog(tag),
        }
    }

    /// `ClientboundShowDialogPacket` in play: a registry entry or the dialog.
    pub(super) fn show_play_dialog(
        &mut self,
        dialog: &horizon_protocol::packets::play::DialogHolder,
    ) {
        use horizon_protocol::packets::play::DialogHolder;
        match dialog {
            DialogHolder::Direct(tag) => self.show_dialog(tag),
            DialogHolder::Registry(id) => {
                let tag = self
                    .remote
                    .as_ref()
                    .and_then(|r| r.registries.data("minecraft:dialog", *id).cloned());
                match tag {
                    Some(tag) => self.show_dialog(&tag),
                    None => log::warn!("Horizon: unknown dialog {id}"),
                }
            }
        }
    }

    fn dialog_layout(&self) -> Option<Layout> {
        let open = self.dialogs.open.as_ref()?;
        Some(Layout::new(
            &open.dialog,
            &self.gui.atlas.font,
            self.gui_size(),
        ))
    }

    pub(super) fn draw_dialog(&mut self, batch: &mut HudBatch) {
        if !self.dialog_open() {
            return;
        }
        let (w, h) = (batch.width, batch.height);
        let cursor = self.cursor;
        let atlas = &self.gui.atlas;
        if self.player.is_none() {
            // In place of the connecting screen, on its background.
            batch.fill(atlas, 0.0, 0.0, w, h, rgba(20, 20, 20, 255));
        }
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, self.player.is_some());
        if self.dialogs.warning {
            let (title, message, [disconnect, back]) = warning_texts();
            Confirm::new(&atlas.font, (w, h), &title, &message).draw(
                batch,
                atlas,
                cursor,
                [&disconnect, &back],
            );
            return;
        }
        if let Some(since) = self.dialogs.waiting {
            // HeaderAndFooterLayout(33, 0) centred: the title, the button 33 below.
            let ticks = (since.elapsed().as_millis() / 50) as u32;
            let (label, visible, active) = waiting_button(ticks);
            let title = Text::translate("gui.waitingForResponse.title", &[]);
            let top = ((h - 53.0) / 2.0).trunc();
            let tw = atlas.font.width_text(&title);
            batch.text_styled(
                atlas,
                &title,
                ((w - tw) / 2.0).trunc(),
                top + 12.0,
                u32::MAX,
                true,
            );
            if visible {
                let (bx, by) = (((w - 200.0) / 2.0).trunc(), top + 33.0);
                let hover = ui::hovered(cursor, bx, by, 200.0, 20.0);
                ui::button(batch, atlas, bx, by, 200.0, 20.0, &label, active, hover);
            }
            return;
        }
        let Some(layout) = self.dialog_layout() else {
            return;
        };
        let Some(open) = self.dialogs.open.as_mut() else {
            return;
        };
        let atlas = &self.gui.atlas;
        let [ax, ay, aw, ah] = layout.area;
        open.scroll.clamp(ay, ay + ah, layout.content);
        batch.text_styled(
            atlas,
            &open.dialog.title,
            layout.title.0,
            layout.title.1,
            u32::MAX,
            true,
        );
        let [wx, wy, ww, wh] = layout.warning;
        let warn_hover = ui::hovered(cursor, wx, wy, ww, wh);
        horizon_client::widgets::record("warning", layout.warning, true);
        batch.sprite_scaled(
            atlas,
            WARNING_SPRITES[if warn_hover { 2 } else { 0 }],
            wx,
            wy,
            ww,
            wh,
            u32::MAX,
        );
        let buttons = open.dialog.buttons();
        let in_area = |y: f32| y >= ay && y < ay + ah;
        let mut tooltip = None;
        let mut start = None;
        for (part, clipped) in layout.placed(open.scroll.amount) {
            if clipped && start.is_none() {
                start = Some(batch.vertices.len());
            } else if !clipped && let Some(s) = start.take() {
                batch.clip_since(s, ax, ay, ax + aw, ay + ah);
            }
            match part {
                Part::Line(text, x, y) => {
                    batch.text_styled(atlas, &text, x, y, u32::MAX, true);
                }
                Part::Button { index, rect } => {
                    let [x, y, bw, bh] = rect;
                    let hover =
                        (!clipped || in_area(cursor.1)) && ui::hovered(cursor, x, y, bw, bh);
                    let b = buttons[index];
                    ui::button(batch, atlas, x, y, bw, bh, &b.label, true, hover);
                    if hover && let Some(t) = &b.tooltip {
                        tooltip = Some(t.clone());
                    }
                }
            }
        }
        if let Some(s) = start {
            batch.clip_since(s, ax, ay, ax + aw, ay + ah);
        }
        if layout.content > ah {
            ui::scrollbar(
                batch,
                atlas,
                ax + aw - 6.0,
                ay,
                ay + ah,
                layout.content,
                open.scroll.amount,
            );
        }
        if warn_hover {
            tooltip = Some(Text::translate("menu.custom_screen_info.tooltip", &[]));
        }
        if let Some(t) = tooltip {
            let lines: Vec<Text> = atlas
                .font
                .split_lines(&t, 170.0)
                .into_iter()
                .map(|l| l.0)
                .collect();
            ui::tooltip(batch, atlas, &lines, cursor);
        }
    }

    /// A left click on a dialog screen.
    pub(super) fn dialog_click(&mut self) -> Result<()> {
        let cursor = self.cursor;
        if self.dialogs.warning {
            let (title, message, _) = warning_texts();
            let answer = Confirm::new(&self.gui.atlas.font, self.gui_size(), &title, &message)
                .answer_at(cursor);
            match answer {
                Some(true) => {
                    self.audio.play_button_click();
                    let reason = Text::translate("menu.custom_screen_info.disconnect", &[]);
                    return self.leave_server(Some(reason.plain()));
                }
                Some(false) => {
                    self.audio.play_button_click();
                    self.dialogs.warning = false;
                    if !self.dialog_open() {
                        self.dialog_closed();
                    }
                }
                None => {}
            }
            return Ok(());
        }
        if let Some(since) = self.dialogs.waiting {
            let (w, h) = self.gui_size();
            let active = waiting_button((since.elapsed().as_millis() / 50) as u32).2;
            let (bx, by) = (
                ((w - 200.0) / 2.0).trunc(),
                ((h - 53.0) / 2.0).trunc() + 33.0,
            );
            if active && ui::hovered(cursor, bx, by, 200.0, 20.0) {
                self.audio.play_button_click();
                self.dialogs.waiting = None;
                self.dialog_closed();
            }
            return Ok(());
        }
        let Some(layout) = self.dialog_layout() else {
            return Ok(());
        };
        let [wx, wy, ww, wh] = layout.warning;
        if ui::hovered(cursor, wx, wy, ww, wh) {
            self.audio.play_button_click();
            self.dialogs.warning = true;
            return Ok(());
        }
        let Some(open) = self.dialogs.open.as_mut() else {
            return Ok(());
        };
        let [_, ay, _, ah] = layout.area;
        let mut action = None;
        for (part, clipped) in layout.placed(open.scroll.amount) {
            if clipped && !(cursor.1 >= ay && cursor.1 < ay + ah) {
                continue;
            }
            match part {
                Part::Button {
                    index,
                    rect: [x, y, w, h],
                } if ui::hovered(cursor, x, y, w, h) => {
                    action = Some(index);
                }
                _ => {}
            }
        }
        let Some(index) = action else {
            return Ok(());
        };
        self.audio.play_button_click();
        let d = &open.dialog;
        let click = d.buttons()[index]
            .action
            .as_ref()
            .and_then(|a| a.click(&d.inputs, &open.values));
        let after = d.after_action;
        self.run_dialog_action(click, after)
    }

    /// The mouse wheel over a dialog: its body scrolls 10 a step.
    pub(super) fn dialog_scroll(&mut self, delta: f32) {
        let Some(layout) = self.dialog_layout() else {
            return;
        };
        if let Some(open) = self.dialogs.open.as_mut() {
            let [_, ay, _, ah] = layout.area;
            open.scroll.wheel(delta, 10.0, ay, ay + ah, layout.content);
        }
    }

    /// A key pressed on a dialog screen: Escape closes what allows it.
    pub(super) fn dialog_key(&mut self, key: KeyCode) -> Result<()> {
        let escape = key == KeyCode::Escape;
        if self.dialogs.warning {
            if escape {
                self.dialogs.warning = false;
                if !self.dialog_open() {
                    self.dialog_closed();
                }
            }
            return Ok(());
        }
        if let Some(since) = self.dialogs.waiting {
            // shouldCloseOnEsc: once Back works.
            if escape && waiting_button((since.elapsed().as_millis() / 50) as u32).2 {
                self.dialogs.waiting = None;
                self.dialog_closed();
            }
            return Ok(());
        }
        let Some(open) = self.dialogs.open.as_ref() else {
            return Ok(());
        };
        if escape {
            if !open.dialog.can_close_with_escape {
                return Ok(());
            }
            // onClose: the cancel action, and the screen closes.
            let d = &open.dialog;
            let click = d.on_cancel().and_then(|a| a.click(&d.inputs, &open.values));
            return self.run_dialog_action(click, AfterAction::Close);
        }
        Ok(())
    }

    /// What `state` reports: the open dialog and its values, or the waiting
    /// and warning screens.
    pub(super) fn dialog_status(&self) -> serde_json::Value {
        let d = &self.dialogs;
        let screen = if d.warning {
            Some("warning")
        } else if d.waiting.is_some() {
            Some("waiting_for_response")
        } else {
            None
        };
        let open = d.open.as_ref().map(|open| open.dialog.json(&open.values));
        serde_json::json!({ "open": open, "screen": screen })
    }
}
