//! `SnbtGrammar`/`TagParser` over NbtOps, shared by commands and persistent codecs.
use super::{Compound, JavaString, Tag};
use std::borrow::Cow;

/// A grammar failure, with an input-relative cursor and its completion suppliers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnbtError {
    pub key: &'static str,
    pub args: Vec<String>,
    pub cursor: usize,
    pub suggestions: Vec<String>,
}
impl std::fmt::Display for SnbtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at {}", self.key, self.cursor)?;
        for arg in &self.args {
            write!(f, ": {arg}")?;
        }
        Ok(())
    }
}
impl std::error::Error for SnbtError {}

#[derive(Clone, Copy)]
enum Input<'a> {
    Utf8(&'a str),
    Utf16(&'a [u16]),
}
#[inline]
fn is_ascii_snbt_whitespace(b: u8) -> bool {
    matches!(b, 0x09..=0x0d | 0x1c..=0x20)
}

#[inline]
fn is_ascii_digit_radix(b: u8, radix: u32) -> bool {
    match radix {
        2 => matches!(b, b'0' | b'1'),
        10 => b.is_ascii_digit(),
        16 => b.is_ascii_hexdigit(),
        _ => char::from(b).is_digit(radix),
    }
}

impl Input<'_> {
    #[inline]
    fn at(self, cursor: usize) -> Option<u32> {
        match self {
            Self::Utf8(text) => {
                let b = *text.as_bytes().get(cursor)?;
                if b < 0x80 {
                    Some(u32::from(b))
                } else {
                    text.get(cursor..)?.chars().next().map(u32::from)
                }
            }
            Self::Utf16(text) => text.get(cursor).copied().map(u32::from),
        }
    }
    #[inline]
    fn width(self, value: u32) -> usize {
        match self {
            Self::Utf8(_) => {
                if value < 0x80 {
                    1
                } else {
                    char::from_u32(value).expect("UTF-8 scalar").len_utf8()
                }
            }
            Self::Utf16(_) => 1,
        }
    }
}

/// `TagParser.parseAsArgument`: one value, preserving a byte cursor for UTF-8.
pub fn parse_snbt_argument(text: &str) -> Result<(Tag, usize), SnbtError> {
    Parser::new(Input::Utf8(text)).parse()
}

/// One value in a Java string; its cursor counts UTF-16 code units.
pub fn parse_snbt_utf16(text: &[u16]) -> Result<(Tag, usize), SnbtError> {
    Parser::new(Input::Utf16(text)).parse()
}

/// One value and its byte count, with a printable error for non-command callers.
pub fn parse_snbt(text: &str) -> Result<(Tag, usize), String> {
    parse_snbt_argument(text).map_err(|error| error.to_string())
}

/// `ParserBasedArgument.listSuggestions`: the grammar's longest-position suppliers.
pub fn snbt_suggestions(text: &str) -> (usize, Vec<String>) {
    let mut parser = Parser::new(Input::Utf8(text));
    parser.collect_suggestions = true;
    match parser.value() {
        Ok(_) if parser.cursor > parser.suggestion_cursor => {
            parser.suggestions.clear();
            parser.suggestion_cursor = parser.cursor;
        }
        Ok(_) => {}
        Err(error) => {
            let values: Vec<&str> = error.suggestions.iter().map(String::as_str).collect();
            parser.record(error.cursor, &values);
        }
    }
    (parser.suggestion_cursor, parser.suggestions)
}

/// `StringReader.skipWhitespace`, not Unicode's broader White_Space property.
pub fn snbt_whitespace(value: u32) -> bool {
    matches!(value, 0x09..=0x0d | 0x1c..=0x20 | 0x1680 | 0x2000..=0x2006
        | 0x2008..=0x200a | 0x2028..=0x2029 | 0x205f | 0x3000)
}

struct Parser<'a> {
    input: Input<'a>,
    cursor: usize,
    depth: usize,
    collect_suggestions: bool,
    suggestion_cursor: usize,
    suggestions: Vec<String>,
}
impl<'a> Parser<'a> {
    fn new(input: Input<'a>) -> Self {
        Self {
            input,
            cursor: 0,
            depth: 0,
            collect_suggestions: false,
            suggestion_cursor: 0,
            suggestions: Vec::new(),
        }
    }
    fn parse(mut self) -> Result<(Tag, usize), SnbtError> {
        let value = self.value()?;
        Ok((value, self.cursor))
    }
    fn error(&self, key: &'static str) -> SnbtError {
        SnbtError {
            key,
            args: Vec::new(),
            cursor: self.cursor,
            suggestions: Vec::new(),
        }
    }
    fn integer_error(
        &self,
        digits: &str,
        minus: bool,
        kind: u8,
        signed: bool,
        radix: u32,
        magnitude: Option<u64>,
    ) -> SnbtError {
        // The IntegerLiteral factories expose the Java numeric parser's detail.
        let text = if minus {
            format!("-{digits}")
        } else {
            digits.to_owned()
        };
        let for_input = || {
            if radix == 10 {
                format!("For input string: \"{text}\"")
            } else {
                format!("For input string: \"{text}\" under radix {radix}")
            }
        };
        let message = match magnitude {
            Some(value) if kind < 2 && value <= i32::MAX as u64 + u64::from(minus) => {
                if signed {
                    format!("Value out of range. Value:\"{text}\" Radix:{radix}")
                } else {
                    format!("out of range: {value}")
                }
            }
            Some(value) if !signed && kind == 2 && value <= i64::MAX as u64 => {
                format!("String value {text} exceeds range of unsigned int.")
            }
            None if !signed && kind == 3 => {
                format!("String value {text} exceeds range of unsigned long.")
            }
            _ => for_input(),
        };
        let mut error = self.error("snbt.parser.number_parse_failure");
        error.args.push(message);
        error
    }
    fn record(&mut self, cursor: usize, values: &[&str]) {
        if !self.collect_suggestions || cursor < self.suggestion_cursor {
            return;
        }
        if cursor > self.suggestion_cursor {
            self.suggestion_cursor = cursor;
            self.suggestions.clear();
        }
        for value in values {
            if !self.suggestions.iter().any(|present| present == value) {
                self.suggestions.push((*value).to_owned());
            }
        }
    }
    #[inline]
    fn skip(&mut self) {
        if let Input::Utf8(text) = self.input {
            let bytes = text.as_bytes();
            while let Some(&b) = bytes.get(self.cursor) {
                if b < 0x80 {
                    if is_ascii_snbt_whitespace(b) {
                        self.cursor += 1;
                    } else {
                        return;
                    }
                } else {
                    let Some(c) = text[self.cursor..].chars().next() else {
                        return;
                    };
                    if snbt_whitespace(u32::from(c)) {
                        self.cursor += c.len_utf8();
                    } else {
                        return;
                    }
                }
            }
            return;
        }
        while self.peek().is_some_and(snbt_whitespace) {
            self.bump();
        }
    }
    fn peek(&self) -> Option<u32> {
        self.input.at(self.cursor)
    }
    fn bump(&mut self) -> Option<u32> {
        let value = self.peek()?;
        self.cursor += self.input.width(value);
        Some(value)
    }
    fn eat(&mut self, value: u8) -> bool {
        let start = self.cursor;
        self.skip();
        if self.peek() == Some(u32::from(value)) {
            self.bump();
            true
        } else {
            if self.collect_suggestions {
                self.record(self.cursor, &[&char::from(value).to_string()]);
            }
            self.cursor = start;
            false
        }
    }
    fn expect(&mut self, value: u8) -> Result<(), SnbtError> {
        if self.eat(value) {
            Ok(())
        } else {
            self.skip();
            let text = char::from(value).to_string();
            let mut error = self.error("argument.literal.incorrect");
            error.args.push(text.clone());
            error.suggestions.push(text);
            Err(error)
        }
    }
    fn lower(&mut self) -> Option<u8> {
        self.skip();
        u8::try_from(self.peek()?)
            .ok()
            .map(|value| value.to_ascii_lowercase())
    }
    fn enter(&mut self) -> Result<(), SnbtError> {
        // The shared NBT boundary already limits containers to 512. Count builtin
        // calls too: deeply nested bool(...) must not overflow the native stack.
        if self.depth >= 512 {
            return Err(self.error("arguments.nbtpath.too_deep"));
        }
        self.depth += 1;
        Ok(())
    }
    fn value(&mut self) -> Result<Tag, SnbtError> {
        self.skip();
        match self.peek() {
            Some(0x7b) => {
                self.enter()?;
                let value = self.compound();
                self.depth -= 1;
                value
            }
            Some(0x5b) => {
                self.enter()?;
                let value = self.list();
                self.depth -= 1;
                value
            }
            Some(0x22 | 0x27) => self.string().map(Tag::String),
            Some(0x2b | 0x2d | 0x2e | 0x30..=0x39) => {
                if let Some(value) = self.float()? {
                    Ok(value)
                } else {
                    self.integer(None)
                }
            }
            _ => self.word(),
        }
    }
    fn unquoted(&mut self) -> Result<Cow<'a, str>, SnbtError> {
        self.skip();
        if let Input::Utf8(text) = self.input {
            let bytes = text.as_bytes();
            let start = self.cursor;
            let mut end = start;
            while let Some(&b) = bytes.get(end) {
                if !b.is_ascii_alphanumeric() && !matches!(b, b'_' | b'-' | b'.' | b'+') {
                    break;
                }
                end += 1;
            }
            self.cursor = end;
            return if start == end {
                Err(self.error("snbt.parser.expected_unquoted_string"))
            } else {
                Ok(Cow::Borrowed(&text[start..end]))
            };
        }
        let mut value = String::new();
        while let Some(c) = self.peek().and_then(|c| u8::try_from(c).ok()) {
            if !c.is_ascii_alphanumeric() && !matches!(c, b'_' | b'-' | b'.' | b'+') {
                break;
            }
            self.bump();
            value.push(char::from(c));
        }
        if value.is_empty() {
            Err(self.error("snbt.parser.expected_unquoted_string"))
        } else {
            Ok(Cow::Owned(value))
        }
    }
    fn string(&mut self) -> Result<JavaString, SnbtError> {
        self.skip();
        let Some(quote @ (0x22 | 0x27)) = self.peek() else {
            self.record(self.cursor, &["\"", "'"]);
            return self.unquoted().map(|value| {
                // Unquoted strings are strictly ASCII.
                JavaString(value.as_bytes().iter().map(|&b| u16::from(b)).collect())
            });
        };
        self.cursor += 1;
        let mut units = Vec::new();
        if let Input::Utf8(text) = self.input {
            let bytes = text.as_bytes();
            let quote_byte = quote as u8;
            loop {
                let start = self.cursor;
                let mut i = start;
                while i < bytes.len() {
                    let b = bytes[i];
                    if b == quote_byte || b == b'\\' || b >= 0x80 {
                        break;
                    }
                    i += 1;
                }
                if i > start {
                    units.extend(bytes[start..i].iter().map(|&b| u16::from(b)));
                    self.cursor = i;
                }
                if i >= bytes.len() {
                    let mut error = self.error("snbt.parser.invalid_string_contents");
                    error.suggestions = ["\"", "'", "\\"].map(str::to_owned).to_vec();
                    return Err(error);
                }
                let b = bytes[i];
                if b == quote_byte {
                    self.cursor = i + 1;
                    return Ok(JavaString(units));
                }
                if b == b'\\' {
                    self.cursor = i + 1;
                    self.escape(&mut units)?;
                } else {
                    let c = text[i..].chars().next().expect("valid UTF-8");
                    self.cursor = i + c.len_utf8();
                    push_code_point(&mut units, u32::from(c));
                }
            }
        }
        loop {
            match self.bump() {
                Some(c) if c == quote => return Ok(JavaString(units)),
                Some(0x5c) => self.escape(&mut units)?,
                Some(c) => push_code_point(&mut units, c),
                None => {
                    // The contents rule fails first at the end; the closing quote, the other
                    // quote and a backslash are what could continue the string.
                    let mut error = self.error("snbt.parser.invalid_string_contents");
                    error.suggestions = ["\"", "'", "\\"].map(str::to_owned).to_vec();
                    return Err(error);
                }
            }
        }
    }
    fn escape(&mut self, units: &mut Vec<u16>) -> Result<(), SnbtError> {
        // StringReaderTerms terminals skip whitespace even after a backslash;
        // fixed-width hex runs and ordinary quoted contents do not.
        self.skip();
        self.record(
            self.cursor,
            &[
                "b", "s", "t", "n", "f", "r", "\\", "'", "\"", "x", "u", "U", "N",
            ],
        );
        let escape_cursor = self.cursor;
        let value = match self.bump() {
            Some(0x62) => 8,
            Some(0x73) => 32,
            Some(0x74) => 9,
            Some(0x6e) => 10,
            Some(0x66) => 12,
            Some(0x72) => 13,
            Some(c @ (0x5c | 0x27 | 0x22)) => c,
            Some(c @ (0x78 | 0x75 | 0x55)) => {
                let count = match c {
                    0x78 => 2,
                    0x75 => 4,
                    _ => 8,
                };
                let digits_cursor = self.cursor;
                let mut value = 0u32;
                for _ in 0..count {
                    let Some(digit) = self
                        .peek()
                        .and_then(char::from_u32)
                        .and_then(|c| c.to_digit(16))
                    else {
                        let mut error = self.error("snbt.parser.expected_hex_escape");
                        error.cursor = digits_cursor;
                        error.args.push(count.to_string());
                        return Err(error);
                    };
                    self.bump();
                    value = (value << 4) | digit;
                }
                if value > 0x10ffff {
                    let mut error = self.error("snbt.parser.invalid_codepoint");
                    error.args.push(format!("U+{value:08X}"));
                    return Err(error);
                }
                value
            }
            Some(0x4e) => {
                self.expect(b'{')?;
                let mut name = String::new();
                while let Some(c) = self.peek().and_then(|c| u8::try_from(c).ok()) {
                    if !c.is_ascii_alphanumeric() && !matches!(c, b'-' | b' ') {
                        break;
                    }
                    self.bump();
                    name.push(char::from(c));
                }
                if name.is_empty() && self.peek() == Some(0x7d) {
                    return Err(self.error("snbt.parser.invalid_character_name"));
                }
                self.expect(b'}')?;
                crate::character_names::code_point_of(&name)
                    .ok_or_else(|| self.error("snbt.parser.invalid_character_name"))?
            }
            _ => {
                // The first escape alternative is what a failed literal reports.
                let mut error = self.error("argument.literal.incorrect");
                error.cursor = escape_cursor;
                error.args.push("b".to_owned());
                return Err(error);
            }
        };
        push_code_point(units, value);
        Ok(())
    }
    fn compound(&mut self) -> Result<Tag, SnbtError> {
        self.expect(b'{')?;
        let mut values = Compound::new();
        if self.eat(b'}') {
            return Ok(Tag::Compound(values));
        }
        loop {
            let key = self.string()?;
            if key.0.is_empty() {
                return Err(self.error("snbt.parser.empty_key"));
            }
            self.expect(b':')?;
            values.insert(key, self.value()?);
            if self.eat(b'}') {
                return Ok(Tag::Compound(values));
            }
            self.expect(b',')?;
            if self.eat(b'}') {
                return Ok(Tag::Compound(values));
            }
        }
    }
    fn list(&mut self) -> Result<Tag, SnbtError> {
        self.expect(b'[')?;
        let start = self.cursor;
        self.skip();
        self.record(self.cursor, &["B", "L", "I"]);
        let typed = match self.peek() {
            Some(0x42) => Some(0),
            Some(0x49) => Some(2),
            Some(0x4c) => Some(3),
            _ => None,
        };
        let typed = typed.filter(|_| {
            self.bump();
            self.eat(b';')
        });
        if typed.is_none() {
            self.cursor = start;
        }
        if let Some(kind) = typed {
            let mut bytes = Vec::new();
            let mut ints = Vec::new();
            let mut longs = Vec::new();
            if !self.eat(b']') {
                loop {
                    let value = integer_value(self.integer(Some(kind))?);
                    match kind {
                        0 => bytes.push(value as u8),
                        2 => ints.push(value as i32),
                        _ => longs.push(value),
                    }
                    if self.eat(b']') {
                        break;
                    }
                    self.expect(b',')?;
                    if self.eat(b']') {
                        break;
                    }
                }
            }
            return Ok(match kind {
                0 => Tag::ByteArray(bytes),
                2 => Tag::IntArray(ints),
                _ => Tag::LongArray(longs),
            });
        }
        let mut values = Vec::new();
        if !self.eat(b']') {
            loop {
                values.push(self.value()?);
                if self.eat(b']') {
                    break;
                }
                self.expect(b',')?;
                if self.eat(b']') {
                    break;
                }
            }
        }
        Ok(Tag::List(values))
    }
    fn sign(&mut self) -> bool {
        self.eat(b'-') || {
            self.eat(b'+');
            false
        }
    }
    fn digits(&mut self, radix: u32) -> Result<Option<Cow<'a, str>>, SnbtError> {
        self.skip();
        if let Input::Utf8(text) = self.input {
            let bytes = text.as_bytes();
            let start = self.cursor;
            let mut end = start;
            while let Some(&b) = bytes.get(end) {
                if b != b'_' && !is_ascii_digit_radix(b, radix) {
                    break;
                }
                end += 1;
            }
            self.cursor = end;
            if start == end {
                return Ok(None);
            }
            let slice = &text[start..end];
            if slice.starts_with('_') || slice.ends_with('_') {
                return Err(self.error("snbt.parser.underscore_not_allowed"));
            }
            return Ok(Some(Cow::Borrowed(slice)));
        }
        let mut text = String::new();
        while let Some(c) = self.peek().and_then(char::from_u32) {
            if c != '_' && !c.is_digit(radix) {
                break;
            }
            self.bump();
            text.push(c);
        }
        if text.starts_with('_') || text.ends_with('_') {
            return Err(self.error("snbt.parser.underscore_not_allowed"));
        }
        if text.is_empty() {
            Ok(None)
        } else {
            Ok(Some(Cow::Owned(text)))
        }
    }
    fn required_digits(&mut self, radix: u32) -> Result<Cow<'a, str>, SnbtError> {
        self.digits(radix)?.ok_or_else(|| {
            self.error(match radix {
                2 => "snbt.parser.expected_binary_numeral",
                16 => "snbt.parser.expected_hex_numeral",
                _ => "snbt.parser.expected_decimal_numeral",
            })
        })
    }
    fn parse_exponent(&mut self) -> Result<Option<(bool, Cow<'a, str>)>, SnbtError> {
        if !self.eat(b'e') && !self.eat(b'E') {
            return Ok(None);
        }
        let exp_minus = self.sign();
        let exp_digits = self.required_digits(10)?;
        Ok(Some((exp_minus, exp_digits)))
    }
    fn float(&mut self) -> Result<Option<Tag>, SnbtError> {
        let start = self.cursor;
        let minus = self.sign();
        let whole = self.digits(10)?;
        let dot = self.eat(b'.');
        let fraction = if dot {
            match self.digits(10)? {
                Some(fraction) => Some(fraction),
                None if whole.is_none() => {
                    return Err(self.error("snbt.parser.expected_decimal_numeral"));
                }
                None => None,
            }
        } else if whole.is_none() {
            self.cursor = start;
            return Ok(None);
        } else {
            None
        };
        let exponent = self.parse_exponent()?;
        let suffix_start = self.cursor;
        let suffix = self.lower().filter(|suffix| matches!(suffix, b'f' | b'd'));
        if suffix.is_none() {
            self.record(self.cursor, &["f", "F", "d", "D"]);
        }
        if suffix.is_some() {
            self.bump();
        } else {
            self.cursor = suffix_start;
        }
        if !dot && exponent.is_none() && suffix.is_none() {
            self.cursor = start;
            return Ok(None);
        }
        let mut stack = [0u8; 64];
        let mut len = 0usize;
        let mut heap = String::new();
        let mut push_bytes = |slice: &[u8]| {
            for &b in slice {
                if b == b'_' {
                    continue;
                }
                if !heap.is_empty() {
                    heap.push(char::from(b));
                } else if len < stack.len() {
                    stack[len] = b;
                    len += 1;
                } else {
                    heap.reserve(96);
                    heap.push_str(std::str::from_utf8(&stack[..len]).expect("ASCII float"));
                    heap.push(char::from(b));
                }
            }
        };
        if minus {
            push_bytes(b"-");
        }
        if let Some(whole) = &whole {
            push_bytes(whole.as_bytes());
        }
        if dot {
            push_bytes(b".");
            if let Some(fraction) = &fraction {
                push_bytes(fraction.as_bytes());
            }
        }
        if let Some((exp_minus, exp_digits)) = &exponent {
            push_bytes(if *exp_minus { b"e-" } else { b"e" });
            push_bytes(exp_digits.as_bytes());
        }
        let text = if !heap.is_empty() {
            heap.as_str()
        } else {
            std::str::from_utf8(&stack[..len]).expect("ASCII float")
        };
        if suffix == Some(b'f') {
            let value = text
                .parse::<f32>()
                .map_err(|_| self.error("snbt.parser.number_parse_failure"))?;
            if !value.is_finite() {
                return Err(self.error("snbt.parser.infinity_not_allowed"));
            }
            Ok(Some(Tag::Float(if value == 0.0 {
                0
            } else {
                value.to_bits()
            })))
        } else {
            let value = text
                .parse::<f64>()
                .map_err(|_| self.error("snbt.parser.number_parse_failure"))?;
            if !value.is_finite() {
                return Err(self.error("snbt.parser.infinity_not_allowed"));
            }
            Ok(Some(Tag::Double(if value == 0.0 {
                0
            } else {
                value.to_bits()
            })))
        }
    }
    fn integer(&mut self, array: Option<u8>) -> Result<Tag, SnbtError> {
        let minus = self.sign();
        let (digits, radix) = if self.eat(b'0') {
            if self.eat(b'x') || self.eat(b'X') {
                (self.required_digits(16)?, 16)
            } else {
                let after_zero = self.cursor;
                let binary = if self.eat(b'b') || self.eat(b'B') {
                    self.digits(2)?
                } else {
                    None
                };
                if let Some(binary) = binary {
                    (binary, 2)
                } else {
                    self.cursor = after_zero;
                    if self.digits(10)?.is_some() {
                        return Err(self.error("snbt.parser.leading_zero_not_allowed"));
                    }
                    self.cursor = after_zero;
                    (Cow::Borrowed("0"), 10)
                }
            }
        } else {
            (self.required_digits(10)?, 10)
        };
        let suffix_start = self.cursor;
        let prefix = self.lower().filter(|c| matches!(c, b'u' | b's'));
        if prefix.is_none() {
            self.record(self.cursor, &["u", "U", "s", "S"]);
            self.cursor = suffix_start;
        }
        let mut signed = None;
        if let Some(prefix) = prefix {
            self.bump();
            let ty = self.lower();
            if ty.is_some_and(|ty| matches!(ty, b'b' | b's' | b'i' | b'l')) {
                signed = Some(prefix == b's');
            } else {
                self.cursor = suffix_start;
            }
        }
        let suffix_cursor = self.cursor;
        let kind = match self.lower() {
            Some(b'b') => Some(0),
            Some(b's') => Some(1),
            Some(b'i') => Some(2),
            Some(b'l') => Some(3),
            _ => None,
        };
        if kind.is_none() {
            self.record(self.cursor, &["b", "B", "s", "S", "i", "I", "l", "L"]);
        }
        if kind.is_some() {
            self.bump();
        } else {
            self.cursor = suffix_cursor;
        }
        let kind = kind.unwrap_or(array.unwrap_or(2));
        if array.is_some_and(|array| kind > array) {
            return Err(self.error("snbt.parser.invalid_array_element_type"));
        }
        let signed = signed.unwrap_or(radix == 10);
        if minus && !signed {
            return Err(self.error("snbt.parser.expected_non_negative_number"));
        }
        let cleaned: Cow<'_, str> = if digits.as_bytes().contains(&b'_') {
            Cow::Owned(digits.chars().filter(|&c| c != '_').collect())
        } else {
            digits
        };
        let bits = [8, 16, 32, 64][usize::from(kind)];
        let magnitude = u64::from_str_radix(&cleaned, radix)
            .map_err(|_| self.integer_error(&cleaned, minus, kind, signed, radix, None))?;
        let max = if signed {
            (1u64 << (bits - 1)) - 1 + u64::from(minus)
        } else if bits == 64 {
            u64::MAX
        } else {
            (1u64 << bits) - 1
        };
        if magnitude > max {
            return Err(self.integer_error(&cleaned, minus, kind, signed, radix, Some(magnitude)));
        }
        let value = if minus {
            magnitude.wrapping_neg() as i64
        } else {
            magnitude as i64
        };
        Ok(match kind {
            0 => Tag::Byte(value as i8),
            1 => Tag::Short(value as i16),
            2 => Tag::Int(value as i32),
            _ => Tag::Long(value),
        })
    }
    fn word(&mut self) -> Result<Tag, SnbtError> {
        let start = self.cursor;
        let word = self.unquoted()?;
        if self.eat(b'(') {
            self.enter()?;
            let result = self.arguments(&word);
            self.depth -= 1;
            return result;
        }
        if word.eq_ignore_ascii_case("true") {
            Ok(Tag::Byte(1))
        } else if word.eq_ignore_ascii_case("false") {
            Ok(Tag::Byte(0))
        } else {
            if matches!(word.as_bytes()[0], b'+' | b'-' | b'.' | b'0'..=b'9') {
                self.cursor = start;
                return Err(self.error("snbt.parser.invalid_unquoted_start"));
            }
            Ok(Tag::String(word.as_ref().into()))
        }
    }
    fn arguments(&mut self, word: &str) -> Result<Tag, SnbtError> {
        let mut arguments = Vec::new();
        if !self.eat(b')') {
            loop {
                arguments.push(self.value()?);
                if self.eat(b')') {
                    break;
                }
                self.expect(b',')?;
                if self.eat(b')') {
                    break;
                }
            }
        }
        match (word, arguments.as_slice()) {
            ("bool", [value]) => {
                let value = match value {
                    Tag::Byte(v) => f64::from(*v),
                    Tag::Short(v) => f64::from(*v),
                    Tag::Int(v) => f64::from(*v),
                    Tag::Long(v) => *v as f64,
                    Tag::Float(v) => f64::from(f32::from_bits(*v)),
                    Tag::Double(v) => f64::from_bits(*v),
                    _ => return Err(self.error("snbt.parser.expected_number_or_boolean")),
                };
                Ok(Tag::Byte(i8::from(value != 0.0)))
            }
            ("uuid", [Tag::String(value)]) => {
                let value = value
                    .to_string()
                    .ok()
                    .and_then(|value| crate::text::parse_java_uuid(&value))
                    .ok_or_else(|| self.error("snbt.parser.expected_string_uuid"))?;
                Ok(Tag::IntArray(vec![
                    (value >> 96) as u32 as i32,
                    (value >> 64) as u32 as i32,
                    (value >> 32) as u32 as i32,
                    value as u32 as i32,
                ]))
            }
            ("uuid", [_]) => Err(self.error("snbt.parser.expected_string_uuid")),
            _ => {
                let mut error = self.error("snbt.parser.no_such_operation");
                error.args.push(format!("{word}/{}", arguments.len()));
                Err(error)
            }
        }
    }
}
fn integer_value(value: Tag) -> i64 {
    match value {
        Tag::Byte(value) => i64::from(value),
        Tag::Short(value) => i64::from(value),
        Tag::Int(value) => i64::from(value),
        Tag::Long(value) => value,
        _ => unreachable!("integer array elements are validated before construction"),
    }
}
fn push_code_point(units: &mut Vec<u16>, value: u32) {
    if value <= 0xffff {
        units.push(value as u16);
    } else {
        let value = value - 0x10000;
        units.push(0xd800 | (value >> 10) as u16);
        units.push(0xdc00 | (value & 0x3ff) as u16);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_persistent_grammar_preserves_utf16_and_numeric_types() {
        let text = r#"{byte:0xffub,short:65535us,int:0xffffffff,long:0xffffffffffffffffL,signed:-0x80sb,binary:0b1111,float:1 . 25 f,array:[I;1b,2s,3],flags:[TRUE,FALSE,bool(256)],uuid:uuid('1-1-1-1-1'),text:'\x41\uD800\U0001F600\N{snowman}'}"#;
        let (tag, used) = parse_snbt_argument(text).unwrap();
        assert_eq!(used, text.len());
        assert_eq!(tag.get("byte"), Some(&Tag::Byte(-1)));
        assert_eq!(tag.get("short"), Some(&Tag::Short(-1)));
        assert_eq!(tag.get("int"), Some(&Tag::Int(-1)));
        assert_eq!(tag.get("long"), Some(&Tag::Long(-1)));
        assert_eq!(tag.get("signed"), Some(&Tag::Byte(-128)));
        assert_eq!(tag.get("binary"), Some(&Tag::Int(15)));
        assert_eq!(tag.get("float"), Some(&Tag::Float(1.25f32.to_bits())));
        assert_eq!(tag.get("array"), Some(&Tag::IntArray(vec![1, 2, 3])));
        assert_eq!(
            tag.get("flags"),
            Some(&Tag::List(vec![Tag::Byte(1), Tag::Byte(0), Tag::Byte(1)]))
        );
        assert_eq!(
            tag.get("uuid"),
            Some(&Tag::IntArray(vec![1, 65537, 65536, 1]))
        );
        assert_eq!(
            tag.get("text"),
            Some(&Tag::String(JavaString(vec![
                0x41, 0xd800, 0xd83d, 0xde00, 0x2603
            ]))),
        );
        let units: Vec<u16> = "{'"
            .encode_utf16()
            .chain([0xd800])
            .chain("':'".encode_utf16())
            .chain([0xdc00])
            .chain("'}".encode_utf16())
            .collect();
        assert_eq!(
            parse_snbt_utf16(&units).unwrap(),
            (
                Tag::compound([(
                    JavaString(vec![0xd800]),
                    Tag::String(JavaString(vec![0xdc00]))
                )]),
                units.len()
            ),
        );
        for source in [
            "{v:128b}",
            "{v:65536us}",
            "{v:0xffsb}",
            "{v:-0xff}",
            "{v:1e400}",
            "{v:01}",
            "{v:[B;1s]}",
            "{v:[I;1L]}",
            "{v:'\\U00110000'}",
            "{v:'\\u123'}",
            "{v:'\\q'}",
            "{v:'\\N{LOW-LINE}'}",
            "{v:bool('true')}",
            "{v:uuid('invalid')}",
            "{v:unknown(1)}",
            "{'':1}",
        ] {
            assert!(parse_snbt_argument(source).is_err(), "{source}");
        }
        assert_eq!(
            crate::text::parse_java_uuid("١-１-1-1-1"),
            Some(0x00000001_00010001_00010000_00000001)
        );
    }
}
