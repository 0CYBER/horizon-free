//! Chat: the message list (ChatComponent: wrapping, 100-line history,
//! 200-tick fade, scrolling, system message indicator) and the chat screen
//! (ChatScreen + EditBox + CommandSuggestions: history navigation, argument
//! colouring, usage hints, parse errors and tab completion).
use crate::{
    command::{Dispatcher, ParseResults, Registries, Suggestion},
    hud::{HudAtlas, HudBatch, rgba},
    text::{self, Style, Text},
    ui::{EditBox, char_len},
};

/// GuiMessageTag.systemSinglePlayer's indicator colour.
pub const SYSTEM_INDICATOR: u32 = 0xD0D0D0;

#[derive(Clone, Copy, Debug)]
pub struct ChatOptions {
    pub opacity: f32,
    pub background_opacity: f32,
    pub scale: f32,
    pub width: f32,
    pub height_focused: f32,
    pub height_unfocused: f32,
    pub line_spacing: f32,
}
impl Default for ChatOptions {
    fn default() -> Self {
        Self {
            opacity: 1.0,
            background_opacity: 0.5,
            scale: 1.0,
            width: 1.0,
            height_focused: 1.0,
            height_unfocused: 70.0 / 160.0,
            line_spacing: 0.0,
        }
    }
}
impl ChatOptions {
    /// ChatComponent.getWidth/getHeight.
    pub fn width_px(&self) -> f32 {
        (self.width * 280.0 + 40.0).floor()
    }
    pub fn height_px(&self, focused: bool) -> f32 {
        ((if focused {
            self.height_focused
        } else {
            self.height_unfocused
        }) * 160.0
            + 20.0)
            .floor()
    }
    pub fn line_height(&self) -> f32 {
        (9.0 * (self.line_spacing + 1.0)).floor()
    }
}
#[derive(Clone, Debug)]
struct Message {
    added: u64,
    content: Text,
    indicator: Option<u32>,
}
#[derive(Clone, Debug)]
struct Line {
    added: u64,
    content: Text,
    indicator: Option<u32>,
}
#[derive(Default)]
pub struct Chat {
    messages: Vec<Message>,
    /// Wrapped lines, newest first.
    lines: Vec<Line>,
    /// Sent messages and commands (ChatComponent.recentChat).
    pub recent: Vec<String>,
    scroll: usize,
    new_since_scroll: bool,
    pub options: ChatOptions,
    /// Messages printed to stdout as `[CHAT]` (vanilla logs them).
    pub log: bool,
}
impl Chat {
    pub fn lines_per_page(&self, focused: bool) -> usize {
        (self.options.height_px(focused) / self.options.line_height().max(1.0)) as usize
    }
    fn max_width(&self) -> f32 {
        (self.options.width_px() / self.options.scale.max(0.01)).floor()
    }
    /// ChatComponent.addMessage: wrap, prepend and trim to 100 lines.
    pub fn add(
        &mut self,
        atlas: &HudAtlas,
        content: Text,
        indicator: Option<u32>,
        ticks: u64,
        focused: bool,
    ) {
        if self.log {
            println!("[CHAT] {}", content.plain());
        }
        let message = Message {
            added: ticks,
            content,
            indicator,
        };
        self.add_lines(atlas, &message, focused);
        self.messages.insert(0, message);
        self.messages.truncate(100);
    }
    fn add_lines(&mut self, atlas: &HudAtlas, message: &Message, focused: bool) {
        let width = self.max_width();
        for line in atlas.font.split(&message.content, width) {
            if focused && self.scroll > 0 {
                self.new_since_scroll = true;
                self.scroll_by(1, focused);
            }
            self.lines.insert(
                0,
                Line {
                    added: message.added,
                    content: line,
                    indicator: message.indicator,
                },
            );
        }
        self.lines.truncate(100);
    }
    /// Re-wrap after a width or scale change (ChatComponent.rescaleChat).
    pub fn rescale(&mut self, atlas: &HudAtlas) {
        self.scroll = 0;
        self.new_since_scroll = false;
        self.lines.clear();
        let messages: Vec<Message> = self.messages.iter().rev().cloned().collect();
        for m in &messages {
            self.add_lines(atlas, m, false);
        }
    }
    pub fn clear(&mut self) {
        self.messages.clear();
        self.lines.clear();
        self.scroll = 0;
    }
    pub fn add_recent(&mut self, message: &str) {
        if self.recent.last().map(String::as_str) != Some(message) {
            if self.recent.len() >= 100 {
                self.recent.remove(0);
            }
            self.recent.push(message.to_owned());
        }
    }
    pub fn reset_scroll(&mut self) {
        self.scroll = 0;
        self.new_since_scroll = false;
    }
    pub fn scroll_by(&mut self, dir: i32, focused: bool) {
        let max = self
            .lines
            .len()
            .saturating_sub(self.lines_per_page(focused)) as i32;
        let pos = (self.scroll as i32 + dir).min(max);
        if pos <= 0 {
            self.scroll = 0;
            self.new_since_scroll = false;
        } else {
            self.scroll = pos as usize;
        }
    }
    /// ChatComponent.extractRenderState.
    pub fn draw(&self, batch: &mut HudBatch, atlas: &HudAtlas, ticks: u64, focused: bool) {
        if self.lines.is_empty() {
            return;
        }
        let o = &self.options;
        let scale = o.scale;
        if scale <= 0.0 {
            return;
        }
        let max_width = (o.width_px() / scale).ceil();
        let chat_bottom = ((batch.height - 40.0) / scale).floor();
        let text_opacity = o.opacity * 0.9 + 0.1;
        let entry_height = o.line_height();
        let bottom_to_text = (8.0 * (o.line_spacing + 1.0) - 4.0 * o.line_spacing).round();
        let per_page = self.lines_per_page(focused);
        let alpha_of = |line: &Line| -> f32 {
            if focused {
                return 1.0;
            }
            let t =
                ((1.0 - (ticks.saturating_sub(line.added)) as f64 / 200.0) * 10.0).clamp(0.0, 1.0);
            (t * t) as f32
        };
        let visible = self.lines.len().saturating_sub(self.scroll).min(per_page);
        let mut count = 0;
        // Pass 1: backgrounds; pass 2: text (same order as vanilla).
        let s = |v: f32| v * scale;
        for i in (0..visible).rev() {
            let line = &self.lines[i + self.scroll];
            let alpha = alpha_of(line);
            if alpha <= 1.0e-5 {
                continue;
            }
            count += 1;
            let bottom = chat_bottom - i as f32 * entry_height;
            let top = bottom - entry_height;
            batch.fill(
                atlas,
                0.0,
                s(top),
                s(max_width + 8.0),
                s(entry_height),
                rgba(0, 0, 0, (alpha * o.background_opacity * 255.0) as u8),
            );
        }
        for i in (0..visible).rev() {
            let line = &self.lines[i + self.scroll];
            let alpha = alpha_of(line);
            if alpha <= 1.0e-5 {
                continue;
            }
            let bottom = chat_bottom - i as f32 * entry_height;
            let top = bottom - entry_height;
            let text_top = bottom - bottom_to_text;
            let a = (alpha * text_opacity * 255.0) as u8;
            if let Some(c) = line.indicator {
                batch.fill(
                    atlas,
                    0.0,
                    s(top),
                    s(2.0),
                    s(entry_height),
                    rgba((c >> 16) as u8, (c >> 8) as u8, c as u8, a),
                );
            }
            if a > 3 {
                batch.text_styled(
                    atlas,
                    &line.content,
                    s(4.0),
                    s(text_top),
                    rgba(255, 255, 255, a),
                    true,
                );
            }
        }
        // Scroll bar while focused and scrolled content exists.
        if focused && count > 0 {
            let total = self.lines.len() as f32;
            let chat_height = count as f32 * entry_height;
            let virtual_height = total * entry_height;
            if virtual_height != chat_height {
                let y = self.scroll as f32 * chat_height / total - chat_bottom;
                let height = chat_height * chat_height / virtual_height;
                let alpha = if y > 0.0 { 170 } else { 96 };
                let c = if self.new_since_scroll {
                    0xCC3333
                } else {
                    0x3333AA
                };
                let x = max_width + 4.0 + 4.0;
                batch.fill(
                    atlas,
                    s(x),
                    s(-y - height),
                    s(2.0),
                    s(height),
                    rgba((c >> 16) as u8, (c >> 8) as u8, c as u8, alpha),
                );
                batch.fill(
                    atlas,
                    s(x + 1.0),
                    s(-y - height),
                    s(1.0),
                    s(height),
                    rgba(0xCC, 0xCC, 0xCC, alpha),
                );
            }
        }
    }
    /// Plain text of the newest messages (for tests and the control socket).
    pub fn recent_lines(&self, n: usize) -> Vec<String> {
        self.messages
            .iter()
            .take(n)
            .map(|m| m.content.plain())
            .collect()
    }
}

/// CommandSuggestions.SuggestionsList.
#[derive(Clone, Debug)]
struct SuggestionList {
    x: f32,
    width: f32,
    list: Vec<Suggestion>,
    offset: usize,
    current: usize,
    original: String,
    tab_cycles: bool,
    last_mouse: (f32, f32),
}

/// The open chat screen.
pub struct ChatScreen {
    pub input: EditBox,
    history_pos: usize,
    history_buffer: String,
    parse: Option<ParseResults>,
    usage: Vec<Text>,
    usage_x: f32,
    suggestions: Option<SuggestionList>,
    pending: Vec<Suggestion>,
    pending_start: usize,
    keep_suggestions: bool,
    allow_suggestions: bool,
    pub auto_suggestions: bool,
    dirty: bool,
}
const FILL: u32 = 0xD000_0000;
fn fill_color() -> u32 {
    rgba(0, 0, 0, (FILL >> 24) as u8)
}
impl ChatScreen {
    pub fn new(initial: &str, history_len: usize, width: f32, height: f32) -> Self {
        let mut input = EditBox::new(4.0, height - 12.0, width - 4.0, 12.0);
        input.bordered = false;
        input.max_length = 256;
        input.set_value(initial);
        Self {
            input,
            history_pos: history_len,
            history_buffer: String::new(),
            parse: None,
            usage: Vec::new(),
            usage_x: 0.0,
            suggestions: None,
            pending: Vec::new(),
            pending_start: 0,
            keep_suggestions: false,
            allow_suggestions: false,
            auto_suggestions: true,
            dirty: true,
        }
    }
    /// Keep the input box at the bottom when the window resizes.
    pub fn resize(&mut self, width: f32, height: f32) {
        self.input.x = 4.0;
        self.input.y = height - 12.0;
        self.input.width = width - 4.0;
    }
    pub fn value(&self) -> &str {
        &self.input.value
    }
    /// ChatScreen.onEdited.
    pub fn edited(&mut self) {
        self.allow_suggestions = true;
        self.dirty = true;
    }
    /// CommandSuggestions.updateCommandInfo (run lazily before drawing/keys).
    pub fn update(&mut self, atlas: &HudAtlas, d: &Dispatcher, reg: &Registries) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let command = self.input.value.clone();
        if self.parse.as_ref().is_some_and(|p| p.input != command) {
            self.parse = None;
        }
        if !self.keep_suggestions {
            self.input.suggestion = None;
            self.suggestions = None;
        }
        self.usage.clear();
        self.pending.clear();
        let cursor_chars = self.input.cursor;
        let cursor = command
            .char_indices()
            .nth(cursor_chars)
            .map_or(command.len(), |(i, _)| i);
        if !command.starts_with('/') {
            return;
        }
        if self.parse.is_none() {
            self.parse = Some(d.parse_at(&command, 1, reg));
        }
        let Some(parse) = &self.parse else { return };
        if cursor >= 1 && (self.suggestions.is_none() || !self.keep_suggestions) {
            let (range, list) = d.suggestions(parse, cursor, reg);
            self.pending = list;
            self.pending_start = range.0;
            // CommandSuggestions.updateUsageInfo.
            let mut trailing = false;
            if cursor == command.len() {
                if self.pending.is_empty() && !parse.errors.is_empty() {
                    let mut literals = 0;
                    for (_, e) in &parse.errors {
                        if e.literal_incorrect {
                            literals += 1;
                        } else {
                            self.usage.push(e.with_context().with_color(text::RED));
                        }
                    }
                    if literals > 0 {
                        let reader = crate::command::Reader {
                            string: &parse.input,
                            cursor: parse.cursor,
                        };
                        self.usage.push(
                            crate::command::CommandError::tr_at(
                                "command.unknown.argument",
                                &[],
                                &reader,
                            )
                            .with_context()
                            .with_color(text::RED),
                        );
                    }
                } else if parse.can_read() {
                    trailing = true;
                }
            }
            let (node, start, _) = parse.context.suggestion_context(cursor);
            if self.usage.is_empty() {
                let mut lines: Vec<Text> = d
                    .smart_usage(node, reg.permission)
                    .into_iter()
                    .filter(|(n, _)| {
                        !matches!(d.nodes[*n].kind, crate::command::NodeKind::Literal(_))
                    })
                    .map(|(_, u)| Text::colored(u, text::GRAY))
                    .collect();
                if lines.is_empty()
                    && trailing
                    && let Some(e) = parse.error()
                {
                    lines.push(e.with_context().with_color(text::RED));
                }
                self.usage = lines;
            }
            let usage_width = self
                .usage
                .iter()
                .map(|u| atlas.font.width_text(u))
                .fold(0.0, f32::max);
            let start_chars = char_len(&command[..start.min(command.len())]);
            self.usage_x = if self.usage.is_empty() {
                0.0
            } else {
                self.input.screen_x(atlas, start_chars).clamp(
                    0.0,
                    (self.input.screen_x(atlas, 0) + self.input.inner_width() - usage_width)
                        .max(0.0),
                )
            };
            self.suggestions = None;
            if self.allow_suggestions && self.auto_suggestions {
                self.show_suggestions(atlas);
            }
        }
    }
    /// CommandSuggestions.showSuggestions.
    fn show_suggestions(&mut self, atlas: &HudAtlas) {
        if self.pending.is_empty() {
            return;
        }
        let value = self.input.value.clone();
        let width = self
            .pending
            .iter()
            .map(|s| atlas.font.width(&s.text))
            .fold(0.0, f32::max);
        let start_chars = char_len(&value[..self.pending_start.min(value.len())]);
        let x = self.input.screen_x(atlas, start_chars).clamp(
            0.0,
            (self.input.screen_x(atlas, 0) + self.input.inner_width() - width).max(0.0),
        );
        // Sort: suggestions starting with the last word first.
        let cursor = value
            .char_indices()
            .nth(self.input.cursor)
            .map_or(value.len(), |(i, _)| i);
        let partial = &value[..cursor];
        let last_word = partial.rfind(char::is_whitespace).map_or(0, |i| i + 1);
        let word = partial[last_word..].to_lowercase();
        let (mut first, partial_matches): (Vec<Suggestion>, Vec<Suggestion>) =
            self.pending.iter().cloned().partition(|s| {
                s.text.starts_with(&word) || s.text.starts_with(&format!("minecraft:{word}"))
            });
        first.extend(partial_matches);
        let mut list = SuggestionList {
            x: x - 1.0,
            width: width + 1.0,
            list: first,
            offset: 0,
            current: 0,
            original: value,
            tab_cycles: false,
            last_mouse: (-1.0, -1.0),
        };
        self.select(&mut list, 0);
        self.suggestions = Some(list);
    }
    fn select(&mut self, list: &mut SuggestionList, index: isize) {
        let n = list.list.len() as isize;
        list.current = index.rem_euclid(n.max(1)) as usize;
        let applied = list.list[list.current].apply(&list.original);
        self.input.suggestion = applied
            .strip_prefix(self.input.value.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
    }
    fn cycle(&mut self, list: &mut SuggestionList, dir: isize) {
        self.select(list, list.current as isize + dir);
        let limit = 10;
        let max_offset = list.list.len().saturating_sub(limit);
        if list.current < list.offset {
            list.offset = list.current.min(max_offset);
        } else if list.current > list.offset + limit - 1 {
            list.offset = (list.current + 1).saturating_sub(limit).min(max_offset);
        }
    }
    fn use_suggestion(&mut self, list: &mut SuggestionList) {
        let s = list.list[list.current].clone();
        self.keep_suggestions = true;
        let applied = s.apply(&list.original);
        self.input.set_value(&applied);
        let end =
            char_len(&list.original[..s.range.0.min(list.original.len())]) + char_len(&s.text);
        self.input.set_cursor(end, false);
        self.dirty = true;
        self.select(list, list.current as isize);
        self.keep_suggestions = false;
        list.tab_cycles = true;
    }
    /// Suggestion-list keys (CommandSuggestions.keyPressed). Returns true if handled.
    pub fn suggestion_key(
        &mut self,
        atlas: &HudAtlas,
        d: &Dispatcher,
        reg: &Registries,
        key: winit::keyboard::KeyCode,
        shift: bool,
    ) -> bool {
        use winit::keyboard::KeyCode as K;
        self.update(atlas, d, reg);
        if let Some(mut list) = self.suggestions.take() {
            let handled = match key {
                K::ArrowUp => {
                    self.cycle(&mut list, -1);
                    list.tab_cycles = false;
                    true
                }
                K::ArrowDown => {
                    self.cycle(&mut list, 1);
                    list.tab_cycles = false;
                    true
                }
                K::Tab => {
                    if list.tab_cycles {
                        self.cycle(&mut list, if shift { -1 } else { 1 });
                    }
                    self.use_suggestion(&mut list);
                    // Keep the list across the re-parse triggered by the edit.
                    let keep = list.clone();
                    self.keep_suggestions = true;
                    self.update(atlas, d, reg);
                    self.keep_suggestions = false;
                    self.suggestions = Some(keep);
                    return true;
                }
                K::Escape => {
                    self.input.suggestion = None;
                    return true;
                }
                _ => false,
            };
            self.suggestions = Some(list);
            return handled;
        }
        if key == K::Tab {
            self.show_suggestions(atlas);
            return true;
        }
        false
    }
    /// ChatScreen.moveInHistory.
    pub fn move_in_history(&mut self, recent: &[String], dir: isize) {
        let max = recent.len();
        let new = (self.history_pos as isize + dir).clamp(0, max as isize) as usize;
        if new == self.history_pos {
            return;
        }
        if new == max {
            self.history_pos = max;
            let b = self.history_buffer.clone();
            self.input.set_value(&b);
        } else {
            if self.history_pos == max {
                self.history_buffer = self.input.value.clone();
            }
            self.input.set_value(&recent[new]);
            self.suggestions = None;
            self.allow_suggestions = false;
            self.history_pos = new;
        }
        self.dirty = true;
    }
    /// Mouse wheel over the suggestion list.
    pub fn scroll_suggestions(&mut self, cursor: (f32, f32), height: f32, amount: f32) -> bool {
        let Some(list) = &mut self.suggestions else {
            return false;
        };
        let n = list.list.len().min(10);
        let top = height - 12.0 - 3.0 - n as f32 * 12.0;
        if crate::ui::hovered(cursor, list.x, top, list.width, n as f32 * 12.0) {
            let max = list.list.len().saturating_sub(10) as isize;
            list.offset = (list.offset as isize - amount.signum() as isize).clamp(0, max) as usize;
            return true;
        }
        false
    }
    /// Click on a suggestion row.
    pub fn click(&mut self, cursor: (f32, f32), height: f32) -> bool {
        let Some(mut list) = self.suggestions.take() else {
            return false;
        };
        let n = list.list.len().min(10);
        let top = height - 12.0 - 3.0 - n as f32 * 12.0;
        if crate::ui::hovered(cursor, list.x, top, list.width, n as f32 * 12.0) {
            let row = ((cursor.1 - top) / 12.0) as usize + list.offset;
            if row < list.list.len() {
                self.select(&mut list, row as isize);
                self.use_suggestion(&mut list);
            }
            self.suggestions = Some(list);
            return true;
        }
        self.suggestions = Some(list);
        false
    }
    /// CommandSuggestions.formatText: argument colours over the input.
    fn formatted(&self) -> Option<Text> {
        let parse = self.parse.as_ref()?;
        let value = &self.input.value;
        let styles = [
            text::AQUA,
            text::YELLOW,
            text::GREEN,
            text::LIGHT_PURPLE,
            text::GOLD,
        ];
        let mut colors = vec![text::GRAY; value.len()];
        let ctx = parse.context.last_child();
        for (i, arg) in ctx.args.iter().enumerate() {
            let c = styles[i % styles.len()];
            for b in arg.range.0.min(value.len())..arg.range.1.min(value.len()) {
                colors[b] = c;
            }
        }
        if parse.can_read() {
            for c in colors.iter_mut().skip(parse.cursor) {
                *c = text::RED;
            }
        }
        let mut out = Text::empty();
        for (i, ch) in value.char_indices() {
            out.push(Text::styled(ch.to_string(), Style::color(colors[i])));
        }
        Some(out)
    }
    /// ChatScreen.extractRenderState minus the chat lines.
    pub fn draw(
        &mut self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        d: &Dispatcher,
        reg: &Registries,
        cursor: (f32, f32),
        background_opacity: f32,
    ) {
        self.update(atlas, d, reg);
        let (w, h) = (batch.width, batch.height);
        batch.fill(
            atlas,
            2.0,
            h - 14.0,
            w - 4.0,
            12.0,
            rgba(0, 0, 0, (background_opacity * 255.0) as u8),
        );
        let formatted = self.formatted();
        self.input.draw(batch, atlas, formatted.as_ref());
        let fill = fill_color();
        if let Some(mut list) = self.suggestions.take() {
            let limit = list.list.len().min(10);
            let top = h - 12.0 - 3.0 - limit as f32 * 12.0;
            let has_prev = list.offset > 0;
            let has_next = list.list.len() > list.offset + limit;
            let moved = list.last_mouse != cursor;
            list.last_mouse = cursor;
            if has_prev || has_next {
                batch.fill(atlas, list.x, top - 1.0, list.width, 1.0, fill);
                batch.fill(
                    atlas,
                    list.x,
                    top + limit as f32 * 12.0,
                    list.width,
                    1.0,
                    fill,
                );
                for x in (0..list.width as i32).step_by(2) {
                    if has_prev {
                        batch.fill(atlas, list.x + x as f32, top - 1.0, 1.0, 1.0, u32::MAX);
                    }
                    if has_next {
                        batch.fill(
                            atlas,
                            list.x + x as f32,
                            top + limit as f32 * 12.0,
                            1.0,
                            1.0,
                            u32::MAX,
                        );
                    }
                }
            }
            let mut hovered_row = None;
            for i in 0..limit {
                let y = top + 12.0 * i as f32;
                batch.fill(atlas, list.x, y, list.width, 12.0, fill);
                if cursor.0 > list.x
                    && cursor.0 < list.x + list.width
                    && cursor.1 > y
                    && cursor.1 < y + 12.0
                {
                    hovered_row = Some(i + list.offset);
                }
            }
            if moved && let Some(r) = hovered_row {
                self.select(&mut list, r as isize);
            }
            for i in 0..limit {
                let s = &list.list[i + list.offset];
                let color = if i + list.offset == list.current {
                    rgba(255, 255, 0, 255)
                } else {
                    rgba(0xAA, 0xAA, 0xAA, 255)
                };
                batch.text(
                    atlas,
                    &s.text,
                    list.x + 1.0,
                    top + 2.0 + 12.0 * i as f32,
                    color,
                    true,
                );
            }
            if hovered_row.is_some()
                && let Some(tip) = &list.list[list.current].tooltip
            {
                crate::ui::tooltip(batch, atlas, &[Text::literal(tip.clone())], cursor);
            }
            self.suggestions = Some(list);
        } else {
            for (i, line) in self.usage.iter().enumerate() {
                let y = h - 27.0 - 12.0 * i as f32;
                let width = atlas.font.width_text(line);
                batch.fill(atlas, self.usage_x - 1.0, y, width + 2.0, 12.0, fill);
                batch.text_styled(atlas, line, self.usage_x, y + 2.0, u32::MAX, true);
            }
        }
    }
    /// ChatScreen.normalizeChatMessage.
    pub fn normalized(&self) -> String {
        let collapsed = self
            .input
            .value
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        collapsed.chars().take(256).collect()
    }
    pub fn suggestions_visible(&self) -> bool {
        self.suggestions.is_some()
    }
}
