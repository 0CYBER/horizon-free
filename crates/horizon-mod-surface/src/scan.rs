//! The tokens of Rust source text, each with its line, for [`crate::uses`].
//!
//! A token scan, not a parser: it skips comments (nested block comments included) and
//! whitespace, reads string, byte, C and raw string literals, char and byte literals and number
//! literals with their suffixes (a float's fraction included) as one [`Token::Literal`] each,
//! lifetimes and labels as [`Token::Lifetime`], `r#` raw identifiers by their name, a tuple
//! field's position after a single `.` token (`pair.0`, `pair. 0`) as the identifier it names, and every other
//! ASCII character as [`Token::Punct`]. Macro bodies are tokens like any other, so a name used only inside
//! `vec![…]` or `format!(…)` is seen. Any text scans: an unterminated literal or comment ends the
//! scan, never a panic.

/// One token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token<'a> {
    Ident(&'a str),
    /// One ASCII punctuation character: `::` is two.
    Punct(u8),
    /// A literal, as written (quotes and suffix included).
    Literal(&'a str),
    Lifetime,
}

/// Every token of `text` with its 1-based line, in order of appearance.
pub fn tokens(text: &str) -> Tokens<'_> {
    Tokens {
        text,
        bytes: text.as_bytes(),
        at: 0,
        line: 1,
        dots: (false, false),
    }
}

/// The iterator [`tokens`] returns.
pub struct Tokens<'a> {
    text: &'a str,
    bytes: &'a [u8],
    at: usize,
    line: usize,
    /// Whether the last token was a `.`, and whether the one before it was.
    dots: (bool, bool),
}

/// An identifier byte: ASCII letters, digits and `_`, and every byte of a non-ASCII character
/// (Rust identifiers may hold Unicode letters; the scan takes them whole).
fn is_ident(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

impl<'a> Tokens<'a> {
    fn peek(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.at + offset).copied()
    }

    /// Moves past one byte, counting lines.
    fn bump(&mut self) {
        if self.bytes[self.at] == b'\n' {
            self.line += 1;
        }
        self.at += 1;
    }

    fn word(&mut self) -> &'a str {
        let start = self.at;
        while self.peek(0).is_some_and(is_ident) {
            self.at += 1;
        }
        &self.text[start..self.at]
    }

    /// After `//`: to the end of the line.
    fn line_comment(&mut self) {
        while self.peek(0).is_some_and(|b| b != b'\n') {
            self.at += 1;
        }
    }

    /// After `/*`: to its matching `*/`, nested comments included.
    fn block_comment(&mut self) {
        let mut depth = 1usize;
        while depth > 0 && self.at < self.bytes.len() {
            match (self.peek(0), self.peek(1)) {
                (Some(b'/'), Some(b'*')) => {
                    depth += 1;
                    self.at += 2;
                }
                (Some(b'*'), Some(b'/')) => {
                    depth -= 1;
                    self.at += 2;
                }
                _ => self.bump(),
            }
        }
    }

    /// At the opening `"`: to the closing one, past escapes.
    fn string(&mut self) {
        self.at += 1;
        while let Some(byte) = self.peek(0) {
            match byte {
                b'\\' => {
                    self.at += 1;
                    if self.at < self.bytes.len() {
                        self.bump();
                    }
                }
                b'"' => {
                    self.at += 1;
                    return;
                }
                _ => self.bump(),
            }
        }
    }

    /// At the `#`s or `"` after `r`: to the `"` followed by as many `#`s.
    fn raw_string(&mut self) {
        let mut hashes = 0;
        while self.peek(0) == Some(b'#') {
            hashes += 1;
            self.at += 1;
        }
        self.at += 1; // the opening quote
        while self.at < self.bytes.len() {
            if self.bytes[self.at] == b'"'
                && self.bytes[self.at + 1..]
                    .iter()
                    .take(hashes)
                    .filter(|b| **b == b'#')
                    .count()
                    == hashes
            {
                self.at += 1 + hashes;
                return;
            }
            self.bump();
        }
    }

    /// At `'`: a char or byte literal (`'a'`, `'\n'`, `'é'`) to its closing quote (true), or a
    /// lifetime or label (`'a`), skipped with its name (false).
    fn quote(&mut self) -> bool {
        self.at += 1;
        if self.peek(0) == Some(b'\\') {
            // An escape: only a char literal starts so.
            self.at = (self.at + 2).min(self.bytes.len());
            while self.peek(0).is_some_and(|b| b != b'\'' && b != b'\n') {
                self.at += 1;
            }
            if self.peek(0) == Some(b'\'') {
                self.at += 1;
            }
            return true;
        }
        // One character, then a quote: a char literal. Otherwise a lifetime or label.
        let width = self.text[self.at..]
            .chars()
            .next()
            .map_or(0, char::len_utf8);
        if width > 0 && self.peek(width) == Some(b'\'') {
            if self.peek(0) == Some(b'\n') {
                self.line += 1;
            }
            self.at += width + 1;
            true
        } else {
            self.word();
            false
        }
    }
}

impl<'a> Iterator for Tokens<'a> {
    type Item = (Token<'a>, usize);

    fn next(&mut self) -> Option<Self::Item> {
        self.spanned().map(|(token, line, _)| (token, line))
    }
}

impl<'a> Tokens<'a> {
    /// The next token with its line and the byte it starts at.
    pub fn spanned(&mut self) -> Option<(Token<'a>, usize, usize)> {
        let item = self.token()?;
        self.dots = (item.0 == Token::Punct(b'.'), self.dots.0);
        Some(item)
    }

    /// The next token, comments and whitespace skipped.
    fn token(&mut self) -> Option<(Token<'a>, usize, usize)> {
        while let Some(byte) = self.peek(0) {
            let line = self.line;
            let start = self.at;
            // What was read since `start`: always whole characters, so the slice holds.
            let literal = |this: &Self| Token::Literal(this.text.get(start..this.at).unwrap_or(""));
            let quoted = |this: &Self, is_literal: bool| {
                if is_literal {
                    literal(this)
                } else {
                    Token::Lifetime
                }
            };
            match byte {
                b'/' if self.peek(1) == Some(b'/') => self.line_comment(),
                b'/' if self.peek(1) == Some(b'*') => {
                    self.at += 2;
                    self.block_comment();
                }
                b'"' => {
                    self.string();
                    return Some((literal(self), line, start));
                }
                b'\'' => {
                    let is_literal = self.quote();
                    return Some((quoted(self, is_literal), line, start));
                }
                b'0'..=b'9' => {
                    // After a single `.` token (`pair.0`, `nested.0.1`, `pair. 0`, `pair./**/0`):
                    // a tuple field's position.
                    let field = self.dots == (true, false);
                    if field {
                        while self.peek(0).is_some_and(|b| b.is_ascii_digit()) {
                            self.at += 1;
                        }
                        return Some((Token::Ident(&self.text[start..self.at]), line, start));
                    }
                    // A number with its suffix (`1u32`, `0x_ff`), and a float's fraction.
                    self.word();
                    if self.peek(0) == Some(b'.')
                        && self.peek(1).is_some_and(|b| b.is_ascii_digit())
                    {
                        self.at += 1;
                        self.word();
                    }
                    return Some((literal(self), line, start));
                }
                _ if is_ident(byte) => {
                    let word = self.word();
                    let next = self.peek(0);
                    match (word, next) {
                        ("r" | "br" | "cr", Some(b'"' | b'#'))
                            if next == Some(b'"') || self.raw_string_ahead() =>
                        {
                            self.raw_string();
                            return Some((literal(self), line, start));
                        }
                        ("r", Some(b'#')) => {
                            // A raw identifier: `r#type` is `type`.
                            self.at += 1;
                            let name = self.word();
                            if !name.is_empty() {
                                return Some((Token::Ident(name), line, start));
                            }
                        }
                        ("b" | "c", Some(b'"')) => {
                            self.string();
                            return Some((literal(self), line, start));
                        }
                        ("b", Some(b'\'')) => {
                            let is_literal = self.quote();
                            return Some((quoted(self, is_literal), line, start));
                        }
                        _ => return Some((Token::Ident(word), line, start)),
                    }
                }
                _ if byte.is_ascii_whitespace() => self.bump(),
                _ => {
                    self.at += 1;
                    return Some((Token::Punct(byte), line, start));
                }
            }
        }
        None
    }
}

impl Tokens<'_> {
    /// At `#` after `r`, `br` or `cr`: `#`s then `"`, a raw string.
    fn raw_string_ahead(&self) -> bool {
        let rest = &self.bytes[self.at..];
        let hashes = rest.iter().take_while(|b| **b == b'#').count();
        rest.get(hashes) == Some(&b'"')
    }
}
