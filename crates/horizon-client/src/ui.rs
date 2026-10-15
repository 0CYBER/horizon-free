//! Shared GUI widgets drawn as HUD quads: nine-slice sprites (gui.scaling
//! metadata), buttons, sliders, tiled menu backgrounds, separators and the
//! EditBox text field with vanilla cursor, selection and scrolling.
use crate::{
    hud::{HudAtlas, HudBatch, Rect, rgba},
    text::Text,
};
use std::{collections::HashMap, path::Path, sync::RwLock, time::Instant};

/// Nine-slice borders (left, top, right, bottom) from `.png.mcmeta` files.
static NINE_SLICE: RwLock<Option<HashMap<String, [u32; 4]>>> = RwLock::new(None);

/// Read `gui.scaling` metadata for every GUI sprite under `root`.
pub fn load_metadata(root: &Path) {
    let mut map = HashMap::new();
    let base = root.join("assets/minecraft/textures");
    let mut stack = vec![base.join("gui/sprites")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let name = p.to_string_lossy().into_owned();
            let Some(stripped) = name.strip_suffix(".png.mcmeta") else {
                continue;
            };
            let Ok(text) = std::fs::read_to_string(&p) else {
                continue;
            };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
                continue;
            };
            let s = &v["gui"]["scaling"];
            if s["type"] != "nine_slice" {
                continue;
            }
            let border = &s["border"];
            let b = if let Some(n) = border.as_u64() {
                [n as u32; 4]
            } else {
                let g = |k: &str| border[k].as_u64().unwrap_or(0) as u32;
                [g("left"), g("top"), g("right"), g("bottom")]
            };
            let key = Path::new(stripped)
                .strip_prefix(&base)
                .map(|r| r.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            map.insert(key, b);
        }
    }
    *NINE_SLICE.write().unwrap_or_else(|e| e.into_inner()) = Some(map);
}
fn border(sprite: &str) -> [u32; 4] {
    NINE_SLICE
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(sprite).copied())
        .unwrap_or(match sprite {
            "gui/sprites/widget/button"
            | "gui/sprites/widget/button_highlighted"
            | "gui/sprites/widget/button_disabled" => [3; 4],
            "gui/sprites/widget/slider_handle" | "gui/sprites/widget/slider_handle_highlighted" => {
                [2, 2, 2, 3]
            }
            _ => [1; 4],
        })
}
fn sub(r: Rect, x: f32, y: f32, w: f32, h: f32) -> Rect {
    let (du, dv) = ((r.u1 - r.u0) / r.w as f32, (r.v1 - r.v0) / r.h as f32);
    Rect {
        u0: r.u0 + x * du,
        v0: r.v0 + y * dv,
        u1: r.u0 + (x + w) * du,
        v1: r.v0 + (y + h) * dv,
        w: w as u32,
        h: h as u32,
    }
}
/// Tile `src` (a sub-rectangle of a sprite, in its pixels) over a target area.
fn tile(batch: &mut HudBatch, r: Rect, src: [f32; 4], dst: [f32; 4], color: u32) {
    let [sx, sy, sw, sh] = src;
    let [dx, dy, dw, dh] = dst;
    if sw <= 0.0 || sh <= 0.0 || dw <= 0.0 || dh <= 0.0 {
        return;
    }
    let mut y = 0.0;
    while y < dh {
        let h = sh.min(dh - y);
        let mut x = 0.0;
        while x < dw {
            let w = sw.min(dw - x);
            batch.rect_uv(dx + x, dy + y, w, h, sub(r, sx, sy, w, h), color);
            x += sw;
        }
        y += sh;
    }
}
/// GuiGraphics.blitSprite with nine-slice scaling (inner parts tiled).
pub fn nine_slice(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    sprite: &str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: u32,
) {
    let Some(r) = atlas.sprites.get(sprite).copied() else {
        return;
    };
    let [l, t, rr, b] = border(sprite).map(|v| v as f32);
    let (sw, sh) = (r.w as f32, r.h as f32);
    let l = l.min(w / 2.0);
    let rr = rr.min(w / 2.0);
    let t = t.min(h / 2.0);
    let b = b.min(h / 2.0);
    let (cw, ch) = (w - l - rr, h - t - b);
    let (scw, sch) = (sw - l - rr, sh - t - b);
    // Corners.
    batch.rect_uv(x, y, l, t, sub(r, 0.0, 0.0, l, t), color);
    batch.rect_uv(x + w - rr, y, rr, t, sub(r, sw - rr, 0.0, rr, t), color);
    batch.rect_uv(x, y + h - b, l, b, sub(r, 0.0, sh - b, l, b), color);
    batch.rect_uv(
        x + w - rr,
        y + h - b,
        rr,
        b,
        sub(r, sw - rr, sh - b, rr, b),
        color,
    );
    // Edges and centre.
    tile(batch, r, [l, 0.0, scw, t], [x + l, y, cw, t], color);
    tile(
        batch,
        r,
        [l, sh - b, scw, b],
        [x + l, y + h - b, cw, b],
        color,
    );
    tile(batch, r, [0.0, t, l, sch], [x, y + t, l, ch], color);
    tile(
        batch,
        r,
        [sw - rr, t, rr, sch],
        [x + w - rr, y + t, rr, ch],
        color,
    );
    tile(batch, r, [l, t, scw, sch], [x + l, y + t, cw, ch], color);
}
/// Repeat a whole texture with each copy drawn `size` GUI pixels square.
pub fn tiled(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    sprite: &str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    size: f32,
    color: u32,
) {
    let Some(r) = atlas.sprites.get(sprite).copied() else {
        return;
    };
    let scale = r.w as f32 / size;
    let mut yy = 0.0;
    while yy < h {
        let th = size.min(h - yy);
        let mut xx = 0.0;
        while xx < w {
            let tw = size.min(w - xx);
            batch.rect_uv(
                x + xx,
                y + yy,
                tw,
                th,
                sub(r, 0.0, 0.0, tw * scale, th * scale),
                color,
            );
            xx += size;
        }
        yy += size;
    }
}
/// Text drawn at `scale` times its size (a scaled pose).
pub fn text_scaled(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    text: &Text,
    x: f32,
    y: f32,
    scale: f32,
    color: u32,
) {
    let start = batch.vertices.len();
    batch.text_styled(atlas, text, 0.0, 0.0, color, true);
    for v in &mut batch.vertices[start..] {
        v.pos = [x + v.pos[0] * scale, y + v.pos[1] * scale];
    }
}
pub fn hovered(cursor: (f32, f32), x: f32, y: f32, w: f32, h: f32) -> bool {
    cursor.0 >= x && cursor.0 < x + w && cursor.1 >= y && cursor.1 < y + h
}
/// Centered text with a shadow, scrolled when wider than `w` (AbstractWidget.renderScrollingString).
pub fn centered_text(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    text: &Text,
    x: f32,
    y: f32,
    w: f32,
    color: u32,
) {
    let width = atlas.font.width_text(text);
    if width <= w {
        batch.text_styled(atlas, text, x + ((w - width) / 2.0).floor(), y, color, true);
    } else {
        // Clip by characters (vanilla scrolls; we show the part that fits).
        let mut shown = Text::empty();
        let mut used = 0.0;
        for (i, (c, s)) in text.chars().enumerate() {
            used += atlas.font.advance(c, s);
            if used > w - 2.0 {
                break;
            }
            shown.push(text.slice(i, i + 1));
        }
        batch.text_styled(atlas, &shown, x + 1.0, y, color, true);
    }
}
/// Button (AbstractButton): nine-slice sprite plus the centred label.
pub fn button(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    label: &Text,
    active: bool,
    hover: bool,
) {
    crate::widgets::record(&label.plain(), [x, y, w, h], active);
    let sprite = if !active {
        "gui/sprites/widget/button_disabled"
    } else if hover {
        "gui/sprites/widget/button_highlighted"
    } else {
        "gui/sprites/widget/button"
    };
    nine_slice(batch, atlas, sprite, x, y, w, h, u32::MAX);
    let color = if active {
        rgba(255, 255, 255, 255)
    } else {
        rgba(160, 160, 160, 255)
    };
    centered_text(
        batch,
        atlas,
        label,
        x + 2.0,
        y + ((h - 8.0) / 2.0).floor(),
        w - 4.0,
        color,
    );
}
/// AbstractSliderButton: track, 8px handle at `value` (0..1) and the label.
pub fn slider(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    x: f32,
    y: f32,
    w: f32,
    label: &Text,
    value: f32,
    hover: bool,
    active: bool,
) {
    if active {
        // A controller's D-pad can focus it (Horizon's controller support).
        crate::widgets::target_as(
            [x, y, w, 20.0],
            crate::widgets::Kind::Slider {
                value,
                track: [x + 4.0, w - 8.0],
            },
        );
    }
    let track = if hover && active {
        "gui/sprites/widget/slider_highlighted"
    } else {
        "gui/sprites/widget/slider"
    };
    nine_slice(batch, atlas, track, x, y, w, 20.0, u32::MAX);
    let handle = if hover && active {
        "gui/sprites/widget/slider_handle_highlighted"
    } else {
        "gui/sprites/widget/slider_handle"
    };
    nine_slice(
        batch,
        atlas,
        handle,
        x + (value.clamp(0.0, 1.0) * (w - 8.0)).floor(),
        y,
        8.0,
        20.0,
        u32::MAX,
    );
    let color = if active {
        u32::MAX
    } else {
        rgba(160, 160, 160, 255)
    };
    centered_text(batch, atlas, label, x + 2.0, y + 6.0, w - 4.0, color);
}
/// Slider value for a click at `cx` (AbstractSliderButton.setValueFromMouse).
pub fn slider_value(x: f32, w: f32, cx: f32) -> f32 {
    ((cx - (x + 4.0)) / (w - 8.0)).clamp(0.0, 1.0)
}
/// Screen.renderMenuBackground: in a world the translucent dark texture,
/// otherwise the dirt-like menu texture, tiled at 32 pixels.
pub fn menu_background(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    in_world: bool,
) {
    let sprite = if in_world {
        "gui/inworld_menu_background"
    } else {
        "gui/menu_background"
    };
    if atlas.sprites.contains_key(sprite) {
        tiled(batch, atlas, sprite, x, y, w, h, 32.0, u32::MAX);
    } else {
        batch.fill(atlas, x, y, w, h, rgba(0, 0, 0, 160));
    }
}
/// Header and footer separators of list screens.
pub fn separators(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    header_bottom: f32,
    footer_top: f32,
    in_world: bool,
) {
    let (header, footer) = if in_world {
        (
            "gui/inworld_header_separator",
            "gui/inworld_footer_separator",
        )
    } else {
        ("gui/header_separator", "gui/footer_separator")
    };
    let w = batch.width;
    tiled(
        batch,
        atlas,
        header,
        0.0,
        header_bottom - 2.0,
        w,
        2.0,
        32.0,
        u32::MAX,
    );
    tiled(
        batch,
        atlas,
        footer,
        0.0,
        footer_top,
        w,
        2.0,
        32.0,
        u32::MAX,
    );
}
/// List background between the separators.
pub fn list_background(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    top: f32,
    bottom: f32,
    in_world: bool,
) {
    let sprite = if in_world {
        "gui/inworld_menu_list_background"
    } else {
        "gui/menu_list_background"
    };
    tiled(
        batch,
        atlas,
        sprite,
        0.0,
        top,
        batch.width,
        bottom - top,
        32.0,
        u32::MAX,
    );
}
/// The visible scrollbar thumb, shared by painting and pointer input.
pub(crate) fn scrollbar_thumb(
    top: f32,
    bottom: f32,
    content: f32,
    scroll: f32,
) -> Option<(f32, f32)> {
    let height = bottom - top;
    if content <= height {
        return None;
    }
    let bar = (height * height / content)
        .clamp(32.0, height - 8.0)
        .floor();
    let y = top + ((scroll / (content - height)) * (height - bar)).floor();
    Some((y, bar))
}

/// `AbstractScrollArea`'s scrolling of a list `top..bottom` holding `content`
/// pixels: the amount, the mouse wheel and the held scrollbar.
#[derive(Clone, Copy, Debug, Default)]
pub struct Scroll {
    pub amount: f32,
    /// The pointer's last y while the scrollbar is held (`scrolling`).
    held: Option<f32>,
}
impl Scroll {
    fn max(top: f32, bottom: f32, content: f32) -> f32 {
        (content - (bottom - top)).max(0.0)
    }
    /// `setScrollAmount`: kept within the content.
    pub fn clamp(&mut self, top: f32, bottom: f32, content: f32) {
        self.amount = self.amount.clamp(0.0, Self::max(top, bottom, content));
    }
    /// `mouseScrolled`: `rate` pixels per wheel step.
    pub fn wheel(&mut self, delta: f32, rate: f32, top: f32, bottom: f32, content: f32) {
        self.amount -= delta * rate;
        self.clamp(top, bottom, content);
    }
    /// `updateScrolling`: a press on the scrollbar column at `bar_x` (when
    /// there is something to scroll) holds it.
    pub fn press(
        &mut self,
        (x, y): (f32, f32),
        bar_x: f32,
        top: f32,
        bottom: f32,
        content: f32,
    ) -> bool {
        let over = x >= bar_x && x <= bar_x + 6.0 && y >= top && y < bottom;
        self.held = (over && Self::max(top, bottom, content) > 0.0).then_some(y);
        self.held.is_some()
    }
    pub fn held(&self) -> bool {
        self.held.is_some()
    }
    /// `mouseDragged` while held: above or below the list jumps to an end,
    /// otherwise the thumb follows the pointer.
    pub fn drag(&mut self, y: f32, top: f32, bottom: f32, content: f32) {
        let Some(last) = self.held else { return };
        let max = Self::max(top, bottom, content);
        if y < top {
            self.amount = 0.0;
        } else if y > bottom {
            self.amount = max;
        } else {
            let height = bottom - top;
            let bar = ((height * height / content).trunc()).clamp(32.0, height - 8.0);
            self.amount += (y - last) * (max.max(1.0) / (height - bar)).max(1.0);
        }
        self.clamp(top, bottom, content);
        self.held = Some(y);
    }
    /// `onRelease`.
    pub fn release(&mut self) {
        self.held = None;
    }
}

/// Vertical scroll bar (AbstractScrollArea): background and scroller sprites.
pub fn scrollbar(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    x: f32,
    top: f32,
    bottom: f32,
    content: f32,
    scroll: f32,
) {
    let Some((y, bar)) = scrollbar_thumb(top, bottom, content, scroll) else {
        return;
    };
    let height = bottom - top;
    nine_slice(
        batch,
        atlas,
        "gui/sprites/widget/scroller_background",
        x,
        top,
        6.0,
        height,
        u32::MAX,
    );
    nine_slice(
        batch,
        atlas,
        "gui/sprites/widget/scroller",
        x,
        y,
        6.0,
        bar,
        u32::MAX,
    );
}

/// Screen tooltip (DefaultTooltipPositioner + TooltipRenderUtil): the
/// background and frame sprites around the lines, beside the cursor.
pub fn tooltip(batch: &mut HudBatch, atlas: &HudAtlas, lines: &[Text], cursor: (f32, f32)) {
    if lines.is_empty() {
        return;
    }
    let w = lines
        .iter()
        .map(|l| atlas.font.width_text(l))
        .fold(0.0, f32::max);
    let h = if lines.len() == 1 {
        8.0
    } else {
        10.0 * lines.len() as f32
    };
    let mut x = cursor.0 + 12.0;
    let mut y = cursor.1 - 12.0;
    if x + w > batch.width {
        x = (cursor.0 - 24.0 - w).max(4.0);
    }
    if y + h + 3.0 > batch.height {
        y = batch.height - h - 3.0;
    }
    y = y.max(4.0);
    nine_slice(
        batch,
        atlas,
        "gui/sprites/tooltip/background",
        x - 12.0,
        y - 12.0,
        w + 24.0,
        h + 24.0,
        u32::MAX,
    );
    nine_slice(
        batch,
        atlas,
        "gui/sprites/tooltip/frame",
        x - 12.0,
        y - 12.0,
        w + 24.0,
        h + 24.0,
        u32::MAX,
    );
    let mut ly = y;
    for (i, line) in lines.iter().enumerate() {
        batch.text_styled(atlas, line, x, ly, u32::MAX, true);
        ly += if i == 0 { 12.0 } else { 10.0 };
    }
}

/// The EditBox text field.
#[derive(Clone, Debug)]
pub struct EditBox {
    pub value: String,
    /// Cursor and selection anchor, in characters.
    pub cursor: usize,
    pub highlight: usize,
    display: usize,
    pub max_length: usize,
    pub bordered: bool,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub focused: bool,
    focused_at: Instant,
    /// Grey completion shown after the text (EditBox.setSuggestion).
    pub suggestion: Option<String>,
    pub text_color: u32,
}
fn byte_index(s: &str, chars: usize) -> usize {
    s.char_indices().nth(chars).map_or(s.len(), |(i, _)| i)
}
pub fn char_len(s: &str) -> usize {
    s.chars().count()
}
impl crate::clipboard::Editable for EditBox {
    fn select_all(&mut self) {
        self.move_cursor_to_end(false);
        self.highlight = 0;
    }
    fn selected_text(&self) -> String {
        EditBox::selected_text(self)
    }
    fn insert(&mut self, text: &str) -> bool {
        let before = self.value.clone();
        self.insert_text(text);
        self.value != before
    }
}
impl EditBox {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            value: String::new(),
            cursor: 0,
            highlight: 0,
            display: 0,
            max_length: 32,
            bordered: true,
            x,
            y,
            width,
            height,
            focused: true,
            focused_at: Instant::now(),
            suggestion: None,
            text_color: rgba(224, 224, 224, 255),
        }
    }
    /// When the field gained focus, for the cursor's blink.
    pub fn focused_since(&mut self, at: Instant) {
        self.focused_at = at;
    }
    pub fn inner_width(&self) -> f32 {
        if self.bordered {
            self.width - 8.0
        } else {
            self.width
        }
    }
    pub fn text_x(&self) -> f32 {
        if self.bordered { self.x + 4.0 } else { self.x }
    }
    pub fn text_y(&self) -> f32 {
        if self.bordered {
            self.y + ((self.height - 8.0) / 2.0).floor()
        } else {
            self.y
        }
    }
    pub fn reset_blink(&mut self) {
        self.focused_at = Instant::now();
    }
    pub fn set_value(&mut self, value: &str) {
        self.value = value.chars().take(self.max_length).collect();
        self.move_cursor_to_end(false);
    }
    fn selected(&self) -> (usize, usize) {
        (
            self.cursor.min(self.highlight),
            self.cursor.max(self.highlight),
        )
    }
    pub fn selected_text(&self) -> String {
        let (a, b) = self.selected();
        self.value.chars().skip(a).take(b - a).collect()
    }
    /// EditBox.insertText: replace the selection, respecting the maximum length.
    pub fn insert_text(&mut self, text: &str) {
        let (a, b) = self.selected();
        let filtered = crate::clipboard::filter_text(text, false);
        let room = self
            .max_length
            .saturating_sub(char_len(&self.value) - (b - a));
        let insert: String = filtered.chars().take(room).collect();
        let (ba, bb) = (byte_index(&self.value, a), byte_index(&self.value, b));
        self.value.replace_range(ba..bb, &insert);
        let pos = a + char_len(&insert);
        self.set_cursor(pos, false);
    }
    /// Delete `n` characters (negative: backwards) or the selection.
    pub fn delete_chars(&mut self, n: isize) {
        if self.cursor != self.highlight {
            self.insert_text("");
            return;
        }
        let target = (self.cursor as isize + n).clamp(0, char_len(&self.value) as isize) as usize;
        let (a, b) = (self.cursor.min(target), self.cursor.max(target));
        let (ba, bb) = (byte_index(&self.value, a), byte_index(&self.value, b));
        self.value.replace_range(ba..bb, "");
        self.set_cursor(a, false);
    }
    /// EditBox.getWordPosition.
    pub fn word_position(&self, dir: isize) -> usize {
        let chars: Vec<char> = self.value.chars().collect();
        let mut pos = self.cursor;
        if dir < 0 {
            while pos > 0 && chars[pos - 1] == ' ' {
                pos -= 1;
            }
            while pos > 0 && chars[pos - 1] != ' ' {
                pos -= 1;
            }
        } else {
            let len = chars.len();
            while pos < len && chars[pos] != ' ' {
                pos += 1;
            }
            while pos < len && chars[pos] == ' ' {
                pos += 1;
            }
        }
        pos
    }
    pub fn set_cursor(&mut self, pos: usize, select: bool) {
        self.cursor = pos.min(char_len(&self.value));
        if !select {
            self.highlight = self.cursor;
        }
        self.scroll_to(self.cursor);
        self.reset_blink();
    }
    pub fn move_cursor_to_end(&mut self, select: bool) {
        self.set_cursor(char_len(&self.value), select);
    }
    fn scroll_to(&mut self, pos: usize) {
        // EditBox.scrollTo without font access is completed in `layout`.
        self.display = self.display.min(char_len(&self.value));
        if pos < self.display {
            self.display = pos;
        }
    }
    fn layout(&mut self, atlas: &HudAtlas) {
        let len = char_len(&self.value);
        self.display = self.display.min(len);
        let (value, inner) = (self.value.clone(), self.inner_width());
        let visible = |d: usize| -> usize {
            let rest: String = value.chars().skip(d).collect();
            atlas.font.fit(&rest, inner)
        };
        while self.cursor > self.display + visible(self.display) && self.display < len {
            self.display += 1;
        }
        if self.cursor < self.display {
            self.display = self.cursor;
        }
    }
    /// Keyboard editing shared by every text field. Returns true if handled.
    pub fn key(&mut self, key: winit::keyboard::KeyCode, ctrl: bool, shift: bool) -> bool {
        use winit::keyboard::KeyCode as K;
        match key {
            K::Backspace => {
                let n = if ctrl {
                    self.word_position(-1) as isize - self.cursor as isize
                } else {
                    -1
                };
                self.delete_chars(n);
            }
            K::Delete => {
                let n = if ctrl {
                    self.word_position(1) as isize - self.cursor as isize
                } else {
                    1
                };
                self.delete_chars(n);
            }
            K::ArrowLeft => {
                let p = if ctrl {
                    self.word_position(-1)
                } else {
                    self.cursor.saturating_sub(1)
                };
                self.set_cursor(p, shift);
            }
            K::ArrowRight => {
                let p = if ctrl {
                    self.word_position(1)
                } else {
                    self.cursor + 1
                };
                self.set_cursor(p, shift);
            }
            K::Home => self.set_cursor(0, shift),
            K::End => self.move_cursor_to_end(shift),
            K::KeyA if ctrl => {
                self.move_cursor_to_end(false);
                self.highlight = 0;
            }
            _ => return false,
        }
        true
    }
    /// Ctrl+A/C/V/X (`EditBox.keyPressed`). True when the value changed.
    pub fn shortcut(
        &mut self,
        shortcut: crate::clipboard::EditShortcut,
        clipboard: &mut dyn crate::clipboard::Clipboard,
    ) -> bool {
        let before = self.value.clone();
        crate::clipboard::apply(self, shortcut, clipboard);
        self.reset_blink();
        self.value != before
    }
    /// The first shown character after scrolling to the cursor, and the
    /// milliseconds since the field was focused (the cursor's blink).
    pub fn view_state(&mut self, atlas: &HudAtlas) -> (usize, u64) {
        self.layout(atlas);
        (self.display, self.focused_at.elapsed().as_millis() as u64)
    }
    /// Draw the field; `format` styles the visible text (command colouring).
    pub fn draw(&mut self, batch: &mut HudBatch, atlas: &HudAtlas, format: Option<&Text>) {
        self.layout(atlas);
        crate::widgets::target_as(
            [self.x, self.y, self.width, self.height],
            crate::widgets::Kind::Field,
        );
        if self.bordered {
            let sprite = if self.focused {
                "gui/sprites/widget/text_field_highlighted"
            } else {
                "gui/sprites/widget/text_field"
            };
            nine_slice(
                batch,
                atlas,
                sprite,
                self.x,
                self.y,
                self.width,
                self.height,
                u32::MAX,
            );
        }
        let color = self.text_color;
        let rest: String = self.value.chars().skip(self.display).collect();
        let shown_len = atlas.font.fit(&rest, self.inner_width());
        let displayed: String = rest.chars().take(shown_len).collect();
        let rel_cursor = self.cursor as isize - self.display as isize;
        let on_screen = rel_cursor >= 0 && rel_cursor as usize <= shown_len;
        let show_cursor = self.focused
            && (self.focused_at.elapsed().as_millis() / 300).is_multiple_of(2)
            && on_screen;
        let styled = |from: usize, to: usize| -> Text {
            match format {
                Some(t) => t.slice(self.display + from, self.display + to),
                None => Text::literal(
                    displayed
                        .chars()
                        .skip(from)
                        .take(to - from)
                        .collect::<String>(),
                ),
            }
        };
        let (tx, ty) = (self.text_x(), self.text_y());
        let mut draw_x = tx;
        if !displayed.is_empty() {
            let half = if on_screen {
                rel_cursor as usize
            } else {
                shown_len
            };
            let t = styled(0, half);
            batch.text_styled(atlas, &t, draw_x, ty, color, true);
            draw_x += atlas.font.width_text(&t) + 1.0;
        }
        let insert =
            self.cursor < char_len(&self.value) || char_len(&self.value) >= self.max_length;
        let mut cursor_x = draw_x;
        if !on_screen {
            cursor_x = if rel_cursor > 0 { tx + self.width } else { tx };
        } else if insert {
            cursor_x -= 1.0;
            draw_x -= 1.0;
        }
        if !displayed.is_empty() && on_screen && (rel_cursor as usize) < shown_len {
            batch.text_styled(
                atlas,
                &styled(rel_cursor as usize, shown_len),
                draw_x,
                ty,
                color,
                true,
            );
        }
        if !insert && let Some(s) = &self.suggestion {
            batch.text(atlas, s, cursor_x - 1.0, ty, rgba(128, 128, 128, 255), true);
        }
        let rel_highlight =
            (self.highlight as isize - self.display as isize).clamp(0, shown_len as isize) as usize;
        if rel_highlight as isize != rel_cursor {
            let hx = tx
                + atlas
                    .font
                    .width(&displayed.chars().take(rel_highlight).collect::<String>());
            let (a, b) = (cursor_x.min(hx - 1.0), cursor_x.max(hx - 1.0));
            batch.fill(atlas, a, ty - 1.0, b - a, 11.0, rgba(0, 0, 255, 128));
        }
        if show_cursor {
            if insert {
                batch.fill(atlas, cursor_x, ty - 1.0, 1.0, 11.0, color);
            } else {
                batch.text(atlas, "_", cursor_x, ty, color, true);
            }
        }
    }
    /// Character index under GUI x (mouse click).
    pub fn index_at(&self, atlas: &HudAtlas, x: f32) -> usize {
        let rest: String = self.value.chars().skip(self.display).collect();
        self.display + atlas.font.fit(&rest, (x - self.text_x()).max(0.0))
    }
    /// GUI x of a character index (EditBox.getScreenX).
    pub fn screen_x(&self, atlas: &HudAtlas, index: usize) -> f32 {
        if index > char_len(&self.value) {
            return self.x;
        }
        let shown: String = self
            .value
            .chars()
            .skip(self.display)
            .take(index.saturating_sub(self.display))
            .collect();
        self.text_x() + atlas.font.width(&shown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn held_scrollbars_follow_the_pointer_and_stop_at_the_ends() {
        // A 100-pixel list holding 400: the thumb is 25 -> 32 high, so a
        // pixel of drag scrolls 300 / 68 pixels (AbstractScrollArea.mouseDragged).
        let (top, bottom, content) = (0.0, 100.0, 400.0);
        let mut s = Scroll::default();
        assert!(!s.press((50.0, 10.0), 100.0, top, bottom, content));
        assert!(s.press((103.0, 10.0), 100.0, top, bottom, content));
        s.drag(20.0, top, bottom, content);
        assert!((s.amount - 10.0 * 300.0 / 68.0).abs() < 1e-3);
        s.drag(150.0, top, bottom, content);
        assert_eq!(s.amount, 300.0);
        s.drag(-5.0, top, bottom, content);
        assert_eq!(s.amount, 0.0);
        s.release();
        s.drag(50.0, top, bottom, content);
        assert_eq!(s.amount, 0.0);
        // Nothing to scroll: the column doesn't take the press.
        assert!(!s.press((103.0, 10.0), 100.0, top, bottom, 90.0));
        s.wheel(-2.0, 18.0, top, bottom, content);
        assert_eq!(s.amount, 36.0);
    }
    #[test]
    fn edit_box_editing() {
        let mut e = EditBox::new(0.0, 0.0, 100.0, 20.0);
        e.max_length = 10;
        e.insert_text("hello world");
        assert_eq!(e.value, "hello worl");
        e.set_cursor(5, false);
        assert_eq!(e.word_position(-1), 0);
        assert_eq!(e.word_position(1), 6);
        e.delete_chars(-2);
        assert_eq!(e.value, "hel worl");
        e.set_cursor(0, false);
        e.set_cursor(3, true);
        assert_eq!(e.selected_text(), "hel");
        e.insert_text("é§x");
        assert_eq!(e.value, "éx worl");
        assert_eq!(e.cursor, 2);
    }
}
