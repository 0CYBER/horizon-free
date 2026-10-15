//! The sign editing screen (`SignEditScreen` / `HangingSignEditScreen`):
//! opened when a sign is placed, four lines limited to the sign's line
//! width, Up/Down/Enter move between lines, Escape or Done closes.
use super::Game;
use horizon_client::{
    containers::BlockEntity,
    hud::{HudBatch, rgba},
    signs::wood_type,
};
use std::time::Instant;
use winit::{
    event::{ElementState, KeyEvent},
    keyboard::{KeyCode, PhysicalKey},
};

pub(super) struct SignEditor {
    pub pos: (i32, i32, i32),
    pub front: bool,
    pub line: usize,
    pub opened: Instant,
}

impl Game<'_> {
    /// `SignBlock.openTextEdit` for the front side after placement.
    pub(super) fn open_sign_editor(&mut self, pos: (i32, i32, i32), front: bool) {
        self.sign_edit = Some(SignEditor {
            pos,
            front,
            line: 0,
            opened: Instant::now(),
        });
        self.keys.clear();
        self.right = false;
        self.capture(false);
    }
    pub(super) fn close_sign_editor(&mut self) {
        if let Some(edit) = self.sign_edit.take() {
            self.world
                .modified
                .insert((edit.pos.0 >> 4, edit.pos.2 >> 4));
        }
        self.capture(true);
    }
    pub(super) fn sign_edit_key(&mut self, event: &KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        if let Some(shortcut) = self.edit_shortcut(event) {
            self.sign_shortcut(shortcut);
            return;
        }
        let key = match event.physical_key {
            PhysicalKey::Code(k) => Some(k),
            _ => None,
        };
        self.sign_input(key, event.text.as_deref());
    }
    /// A key and the text it typed on the sign being edited (the keyboard,
    /// or a controller's on-screen keyboard).
    pub(super) fn sign_input(&mut self, key: Option<KeyCode>, typed: Option<&str>) {
        let Some(edit) = self.sign_edit.as_mut() else {
            return;
        };
        let front = edit.front;
        let id = sign_line_id(front, edit.line);
        match key {
            Some(KeyCode::Escape) => self.close_sign_editor(),
            Some(KeyCode::ArrowUp) => edit.line = (edit.line + 3) & 3,
            Some(KeyCode::ArrowDown | KeyCode::Enter | KeyCode::NumpadEnter) => {
                edit.line = (edit.line + 1) & 3
            }
            Some(KeyCode::Backspace) => {
                let (line, key) = (edit.line, edit.pos);
                if let Some(BlockEntity::Sign(sign)) = self.world.block_entities.get_mut(&key) {
                    let text = if front {
                        &mut sign.front
                    } else {
                        &mut sign.back
                    };
                    let value = &mut text.lines[line];
                    if !self.text_selection.take(id, value) {
                        value.pop();
                    }
                }
            }
            _ => {
                let Some(typed) = typed else {
                    return;
                };
                let line = edit.line;
                let atlas = &self.res.atlas;
                let width = |s: &str| HudBatch::text_width(atlas, s);
                let Some(key) = self.sign_edit.as_ref().map(|e| e.pos) else {
                    return;
                };
                if let Some(BlockEntity::Sign(sign)) = self.world.block_entities.get_mut(&key) {
                    let max = sign.max_line_width();
                    let text = if front {
                        &mut sign.front
                    } else {
                        &mut sign.back
                    };
                    self.text_selection.take(id, &mut text.lines[line]);
                    for ch in typed.chars().filter(|c| !c.is_control()) {
                        let mut next = text.lines[line].clone();
                        next.push(ch);
                        // TextFieldHelper filter: font.width(s) <= maxTextLineWidth.
                        if width(&next) <= max {
                            text.lines[line] = next;
                        }
                    }
                }
            }
        }
    }
    /// `TextFieldHelper.keyPressed`'s shortcuts on the edited line: a paste
    /// (formatting and carriage returns removed) is taken whole or not at
    /// all, as the line must stay within the sign's width.
    fn sign_shortcut(&mut self, shortcut: horizon_client::clipboard::EditShortcut) {
        let Some(edit) = self.sign_edit.as_ref() else {
            return;
        };
        let (key, front, line) = (edit.pos, edit.front, edit.line);
        let Some(BlockEntity::Sign(sign)) = self.world.block_entities.get_mut(&key) else {
            return;
        };
        let max = sign.max_line_width();
        let text = if front {
            &mut sign.front
        } else {
            &mut sign.back
        };
        let atlas = &self.res.atlas;
        self.text_selection.apply(
            sign_line_id(front, line),
            &mut text.lines[line],
            shortcut,
            &mut self.clipboard,
            |value, pasted| {
                let next = format!(
                    "{value}{}",
                    horizon_client::clipboard::helper_clipboard_contents(pasted)
                );
                HudBatch::text_width(atlas, &next) <= max && {
                    *value = next;
                    true
                }
            },
        );
    }
    /// Done button (`this.width / 2 - 100, this.height / 4 + 144`, 200x20).
    pub(super) fn sign_edit_click(&mut self, size: [u32; 2]) {
        let b = HudBatch::new(size);
        let (x, y) = (b.width / 2.0 - 100.0, b.height / 4.0 + 144.0);
        let (mx, my) = self.cursor;
        if mx >= x && mx < x + 200.0 && my >= y && my < y + 20.0 {
            self.audio.play_button_click();
            self.close_sign_editor();
        }
    }
    pub(super) fn draw_sign_editor(&self, batch: &mut HudBatch) {
        let Some(edit) = &self.sign_edit else { return };
        let Some(BlockEntity::Sign(sign)) = self.world.block_entities.get(&edit.pos) else {
            return;
        };
        let atlas = &self.res.atlas;
        let (w, h) = (batch.width, batch.height);
        batch.fill(atlas, 0.0, 0.0, w, h, rgba(16, 16, 16, 160));
        let title = if sign.hanging {
            "Edit Hanging Sign Message"
        } else {
            "Edit Sign Message"
        };
        let tw = HudBatch::text_width(atlas, title);
        batch.text(atlas, title, (w - tw) / 2.0, 40.0, u32::MAX, true);
        let name = self
            .res
            .facts
            .get(self.world.block(edit.pos.0, edit.pos.1, edit.pos.2))
            .name
            .clone();
        let wood = wood_type(&name);
        let cx = w / 2.0;
        let y = if sign.hanging {
            // HangingSignEditScreen: 16x16 texture scaled 4.5 at (0, -13).
            let s = 4.5;
            batch.sprite_scaled(
                atlas,
                &format!("gui/hanging_signs/{wood}"),
                cx - 8.0 * s,
                125.0 - 13.0 - 8.0 * s,
                16.0 * s,
                16.0 * s,
                u32::MAX,
            );
            125.0
        } else {
            // SignEditScreen: 24x26 texture scaled 3.9 at (0, 27); wall signs show 12 rows.
            let s = 3.9;
            let wall = name.contains("_wall_");
            let rows = if wall { 12.0 } else { 26.0 };
            if let Some(mut r) = atlas.sprites.get(&format!("gui/signs/{wood}")).copied() {
                r.v1 = r.v0 + (r.v1 - r.v0) * rows / 26.0;
                batch.rect_uv(
                    cx - 12.0 * s,
                    90.0 + 27.0 - 13.0 * s,
                    24.0 * s,
                    rows * s,
                    r,
                    u32::MAX,
                );
            }
            90.0
        };
        let text = if edit.front { &sign.front } else { &sign.back };
        let color = if text.glowing {
            text.text_color()
        } else {
            text.dark_color()
        };
        let color = rgba((color >> 16) as u8, (color >> 8) as u8, color as u8, 255);
        let line_height = sign.line_height();
        let midpoint = (4.0 * line_height / 2.0).floor();
        let blink = (edit.opened.elapsed().as_millis() / 300) % 2 == 0;
        for (i, line) in text.lines.iter().enumerate() {
            let lw = HudBatch::text_width(atlas, line);
            let x = cx - (lw as i32 / 2) as f32;
            let ly = y + i as f32 * line_height - midpoint;
            batch.text(atlas, line, x, ly, color, false);
            if i == edit.line && blink {
                // TextCursorUtils.extractAppendCursor: an underscore after the text.
                batch.text(atlas, "_", x + lw, ly, color, false);
            }
        }
        let (bx, by) = (w / 2.0 - 100.0, h / 4.0 + 144.0);
        batch.sprite_scaled(
            atlas,
            "gui/sprites/widget/button",
            bx,
            by,
            200.0,
            20.0,
            u32::MAX,
        );
        let tw = HudBatch::text_width(atlas, "Done");
        batch.text(
            atlas,
            "Done",
            bx + (200.0 - tw) / 2.0,
            by + 6.0,
            u32::MAX,
            true,
        );
    }
}

/// The sign line being edited, for Select All.
fn sign_line_id(front: bool, line: usize) -> horizon_client::clipboard::FieldId {
    (if front { "sign.front" } else { "sign.back" }, line)
}
