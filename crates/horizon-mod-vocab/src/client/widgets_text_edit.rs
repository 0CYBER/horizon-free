//! `EditBox`'s editing core without a font: the text, the cursor and the selection anchor, the
//! maximum length, and what typing, deleting and moving do to them. The game's `EditBox` keeps
//! its layout, scroll and drawing on top of this, and a mod's [`super::TextField`] does too, so
//! both edit text the same way.

/// The text of a field and where its cursor and selection are, in characters
/// (`EditBox.value`, `cursorPos`, `highlightPos`, `maxLength`).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct TextEdit {
    /// The text.
    pub value: String,
    /// The cursor, in characters from the start.
    pub cursor: usize,
    /// The other end of the selection; the same as `cursor` when nothing is selected.
    pub highlight: usize,
    /// The most UTF-16 code units the text may have (`EditBox.setMaxLength`, 32 by default).
    pub max_length: usize,
    /// `MultiLineEditBox`: line breaks are kept and words split at any whitespace.
    pub multiline: bool,
}

impl Default for TextEdit {
    fn default() -> Self {
        Self {
            value: String::new(),
            cursor: 0,
            highlight: 0,
            max_length: 32,
            multiline: false,
        }
    }
}

/// An editing key, as `EditBox.keyPressed` handles it. `ctrl` is the platform's word modifier
/// and `shift` extends the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the keys EditBox.keyPressed edits with; a field matches them all
#[derive(horizon_mod_api_macros::Wire)]
pub enum EditKey {
    /// Deletes the character (with Ctrl, the word) before the cursor, or the selection.
    Backspace,
    /// Deletes the character (with Ctrl, the word) after the cursor, or the selection.
    Delete,
    /// Moves a character (with Ctrl, a word) left.
    Left,
    /// Moves a character (with Ctrl, a word) right.
    Right,
    /// Moves to the start.
    Home,
    /// Moves to the end.
    End,
    /// Selects everything (Ctrl+A).
    SelectAll,
}

/// Java's `String.length()`: UTF-16 code units, the unit `EditBox.maxLength`
/// counts. A character outside the BMP — an emoji, say — is two of them, so
/// a box with room for one more character takes an `a` and rejects a `😀`.
pub fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}
/// The longest prefix of `s` that fits `max` UTF-16 code units without
/// splitting a character. `EditBox.insertText` drops the half surrogate it
/// would otherwise keep, so this is what vanilla inserts; Rust strings cannot
/// hold the lone surrogate that `setValue`'s unguarded `substring` may leave,
/// so a value too long by half a character keeps the whole characters before
/// the cut.
pub fn utf16_prefix(s: &str, max: usize) -> String {
    let mut units = 0;
    for (i, c) in s.char_indices() {
        let n = c.len_utf16();
        if units + n > max {
            return s[..i].to_owned();
        }
        units += n;
    }
    s.to_owned()
}

impl TextEdit {
    /// An empty field that takes up to `max_length` UTF-16 code units.
    pub fn new(max_length: usize) -> Self {
        Self {
            max_length,
            ..Self::default()
        }
    }

    /// `EditBox.setValue`: the text, cut to the maximum length, with the cursor at its end.
    pub fn set_value(&mut self, value: &str) {
        self.value = utf16_prefix(value, self.max_length);
        self.move_cursor_to_end(false);
    }

    /// The selection's start and end, in characters (kept within the text).
    pub fn selected(&self) -> (usize, usize) {
        let len = char_len(&self.value);
        (
            self.cursor.min(self.highlight).min(len),
            self.cursor.max(self.highlight).min(len),
        )
    }

    /// Brings a cursor or anchor left past the end, by setting `value` directly, back within
    /// the text, so every edit starts from positions inside it.
    fn clamp(&mut self) {
        let len = char_len(&self.value);
        self.cursor = self.cursor.min(len);
        self.highlight = self.highlight.min(len);
    }

    /// `EditBox.getHighlighted`: the selected text.
    pub fn selected_text(&self) -> String {
        let (a, b) = self.selected();
        self.value.chars().skip(a).take(b - a).collect()
    }

    /// `EditBox.insertText`: `text`, filtered as `StringUtil.filterText` does, replaces the
    /// selection, cut to what the maximum length leaves room for.
    pub fn insert_text(&mut self, text: &str) {
        self.clamp();
        let (a, b) = self.selected();
        let filtered = filter_text(text, self.multiline);
        let selected_units: usize = self
            .value
            .chars()
            .skip(a)
            .take(b - a)
            .map(char::len_utf16)
            .sum();
        let room = self
            .max_length
            .saturating_sub(utf16_len(&self.value) - selected_units);
        let insert = utf16_prefix(&filtered, room);
        let (ba, bb) = (byte_index(&self.value, a), byte_index(&self.value, b));
        self.value.replace_range(ba..bb, &insert);
        let pos = a + char_len(&insert);
        self.set_cursor(pos, false);
    }

    /// `EditBox.deleteChars`: `n` characters after the cursor (before it when negative), or the
    /// selection when there is one.
    pub fn delete_chars(&mut self, n: isize) {
        self.clamp();
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

    /// `EditBox.getWordPosition(dir)` from the cursor.
    pub fn word_position(&self, dir: isize) -> usize {
        self.word_position_from(dir, self.cursor)
    }

    /// `EditBox.getWordPosition(dir, from)`, or `MultilineTextField`'s word for a multi-line
    /// field.
    pub fn word_position_from(&self, dir: isize, from: usize) -> usize {
        if self.multiline {
            multiline_word_position(&self.value, dir, from)
        } else {
            word_position(&self.value, dir, from)
        }
    }

    /// `EditBox.moveCursorTo`: the cursor to `pos` (kept within the text), the selection's
    /// anchor with it unless `select`.
    pub fn set_cursor(&mut self, pos: usize, select: bool) {
        self.cursor = pos.min(char_len(&self.value));
        if !select {
            self.highlight = self.cursor;
        }
    }

    /// `EditBox.moveCursorToEnd`.
    pub fn move_cursor_to_end(&mut self, select: bool) {
        self.set_cursor(char_len(&self.value), select);
    }

    /// Ctrl+A: the cursor at the end and everything selected.
    pub fn select_all(&mut self) {
        self.move_cursor_to_end(false);
        self.highlight = 0;
    }

    /// The word around character `pos` (`EditBox.selectWord`, a double click): its start and
    /// end.
    pub fn word_at(&self, pos: usize) -> (usize, usize) {
        (
            self.word_position_from(-1, pos),
            self.word_position_from(1, pos),
        )
    }

    /// `EditBox.keyPressed`'s editing keys. True when the key edits or moves (the caller then
    /// scrolls to the cursor and restarts its blink).
    pub fn key(&mut self, key: EditKey, ctrl: bool, shift: bool) -> bool {
        self.clamp();
        match key {
            EditKey::Backspace => {
                let n = if ctrl {
                    self.word_position(-1) as isize - self.cursor as isize
                } else {
                    -1
                };
                self.delete_chars(n);
            }
            EditKey::Delete => {
                let n = if ctrl {
                    self.word_position(1) as isize - self.cursor as isize
                } else {
                    1
                };
                self.delete_chars(n);
            }
            EditKey::Left => {
                let p = if ctrl {
                    self.word_position(-1)
                } else {
                    self.cursor.saturating_sub(1)
                };
                self.set_cursor(p, shift);
            }
            EditKey::Right => {
                let p = if ctrl {
                    self.word_position(1)
                } else {
                    self.cursor + 1
                };
                self.set_cursor(p, shift);
            }
            EditKey::Home => self.set_cursor(0, shift),
            EditKey::End => self.move_cursor_to_end(shift),
            EditKey::SelectAll => {
                if !ctrl {
                    return false;
                }
                self.select_all();
            }
        }
        true
    }
}

/// The byte index of character `chars` in `s` (its length past the end).
pub fn byte_index(s: &str, chars: usize) -> usize {
    s.char_indices().nth(chars).map_or(s.len(), |(i, _)| i)
}

/// The length of `s` in characters.
pub fn char_len(s: &str) -> usize {
    s.chars().count()
}

/// `EditBox.getWordPosition(dir, from)`: the character index `dir` words away
/// from `from`. Forward, past the rest of the word and its spaces; backward,
/// over any spaces to the start of the word.
pub fn word_position(value: &str, dir: isize, from: usize) -> usize {
    let chars: Vec<char> = value.chars().collect();
    let mut pos = from.min(chars.len());
    if dir < 0 {
        while pos > 0 && chars[pos - 1] == ' ' {
            pos -= 1;
        }
        while pos > 0 && chars[pos - 1] != ' ' {
            pos -= 1;
        }
    } else {
        while pos < chars.len() && chars[pos] != ' ' {
            pos += 1;
        }
        while pos < chars.len() && chars[pos] == ' ' {
            pos += 1;
        }
    }
    pos
}

/// `MultilineTextField.getPreviousWord` / `getNextWord`'s `beginIndex` (a book page): any
/// whitespace, line breaks included, separates words, and the search starts from the cursor
/// clamped to the last character, as vanilla's does.
pub fn multiline_word_position(value: &str, dir: isize, from: usize) -> usize {
    let chars: Vec<char> = value.chars().collect();
    if chars.is_empty() {
        return 0;
    }
    let mut pos = from.min(chars.len() - 1);
    if dir < 0 {
        while pos > 0 && java_whitespace(chars[pos - 1]) {
            pos -= 1;
        }
        while pos > 0 && !java_whitespace(chars[pos - 1]) {
            pos -= 1;
        }
    } else {
        while pos < chars.len() && !java_whitespace(chars[pos]) {
            pos += 1;
        }
        while pos < chars.len() && java_whitespace(chars[pos]) {
            pos += 1;
        }
    }
    pos
}

/// `Character.isWhitespace`: Unicode spaces and line breaks but not the no-break spaces, plus
/// the ASCII separators U+001C to U+001F.
pub fn java_whitespace(c: char) -> bool {
    (c.is_whitespace() && !matches!(c, '\u{85}' | '\u{a0}' | '\u{2007}' | '\u{202f}'))
        || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// `StringUtil.isAllowedChatCharacter`.
pub fn is_allowed_chat_character(c: char) -> bool {
    c != '§' && c >= ' ' && c != '\u{7f}'
}

/// `StringUtil.filterText`: the allowed characters, and line breaks where
/// the field takes several lines.
pub fn filter_text(input: &str, multiline: bool) -> String {
    input
        .chars()
        .filter(|&c| is_allowed_chat_character(c) || (multiline && c == '\n'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(value: &str) -> TextEdit {
        let mut edit = TextEdit::new(32);
        edit.set_value(value);
        edit
    }

    /// A cursor or selection left past the end by setting `value` directly is brought back
    /// within the text before an edit, instead of overflowing.
    #[test]
    fn edits_after_the_value_is_set_directly_stay_within_it() {
        let mut edit = field("hello");
        edit.select_all();
        edit.value.clear();
        assert_eq!(edit.selected_text(), "");
        edit.insert_text("x");
        assert_eq!(
            (edit.value.as_str(), edit.cursor, edit.highlight),
            ("x", 1, 1)
        );
        let mut edit = field("hello world");
        edit.value = "hi".into();
        assert!(edit.key(EditKey::Backspace, true, false));
        assert_eq!((edit.value.as_str(), edit.cursor), ("", 0));
        let mut edit = field("hello");
        edit.value.truncate(2);
        edit.delete_chars(-1);
        assert_eq!((edit.value.as_str(), edit.cursor), ("h", 1));
    }

    /// Typing replaces the selection, filters control characters and `§`, and stops at the
    /// maximum length.
    #[test]
    fn typing_replaces_the_selection_within_the_maximum_length() {
        let mut edit = field("hello world");
        edit.set_cursor(0, false);
        edit.set_cursor(5, true);
        assert_eq!(edit.selected_text(), "hello");
        edit.insert_text("bye\u{7}§");
        assert_eq!((edit.value.as_str(), edit.cursor), ("bye world", 3));
        let mut short = TextEdit::new(5);
        short.insert_text("abcdefgh");
        assert_eq!((short.value.as_str(), short.cursor), ("abcde", 5));
        let mut lines = TextEdit::new(32);
        lines.insert_text("a\nb");
        assert_eq!(lines.value, "ab");
        lines.multiline = true;
        lines.insert_text("\nc");
        assert_eq!(lines.value, "ab\nc");
    }

    /// Backspace and Delete take a character, a word with Ctrl, or the selection; the arrows
    /// move a character or a word and extend the selection with Shift.
    #[test]
    fn editing_keys_move_and_delete_as_edit_box_does() {
        let mut edit = field("one two  three");
        assert!(edit.key(EditKey::Backspace, true, false));
        assert_eq!(edit.value, "one two  ");
        assert!(edit.key(EditKey::Backspace, false, false));
        assert_eq!(edit.value, "one two ");
        assert!(edit.key(EditKey::Left, true, false));
        assert_eq!(edit.cursor, 4);
        assert!(edit.key(EditKey::Right, true, true));
        assert_eq!((edit.cursor, edit.highlight), (8, 4));
        assert_eq!(edit.selected_text(), "two ");
        assert!(edit.key(EditKey::Delete, false, false));
        assert_eq!((edit.value.as_str(), edit.cursor), ("one ", 4));
        assert!(edit.key(EditKey::Home, false, false));
        assert!(edit.key(EditKey::Delete, true, false));
        assert_eq!(edit.value, "");
        let mut all = field("abc");
        assert!(!all.key(EditKey::SelectAll, false, false));
        assert!(all.key(EditKey::SelectAll, true, false));
        assert_eq!((all.cursor, all.highlight), (3, 0));
        assert!(all.key(EditKey::Left, false, false));
        assert_eq!((all.cursor, all.highlight), (2, 2));
    }

    /// A double click's word, and the multi-line word rule that any whitespace splits.
    #[test]
    fn words_split_at_spaces_or_at_any_whitespace_when_multiline() {
        let edit = field("alpha beta gamma");
        assert_eq!(edit.word_at(7), (6, 11));
        let mut page = field("a\nbc d");
        // A single-line field splits at spaces only, so the line break is part of the word.
        assert_eq!(page.word_position_from(1, 0), 5);
        page.multiline = true;
        assert_eq!(page.word_position_from(1, 0), 2);
        assert_eq!(multiline_word_position("", 1, 0), 0);
        assert!(java_whitespace('\u{1c}') && !java_whitespace('\u{a0}'));
    }
}
