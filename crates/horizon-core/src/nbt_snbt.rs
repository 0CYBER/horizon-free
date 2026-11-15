//! `SnbtGrammar`/`TagParser` over NbtOps, shared by commands and persistent codecs.
use super::{Compound, JavaString, Tag};

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
impl Input<'_> {
    fn at(self, cursor: usize) -> Option<u32> {
        match self {
            Self::Utf8(text) => text.get(cursor..)?.chars().next().map(u32::from),
            Self::Utf16(text) => text.get(cursor).copied().map(u32::from),
        }
    }
    fn width(self, value: u32) -> usize {
        match self {
            Self::Utf8(_) => char::from_u32(value).expect("UTF-8 scalar").len_utf8(),
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
    if parser.value().is_ok() && parser.cursor > parser.suggestion_cursor {
        parser.suggestions.clear();
        parser.suggestion_cursor = parser.cursor;
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
    fn skip(&mut self) {
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
    fn unquoted(&mut self) -> Result<String, SnbtError> {
        self.skip();
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
            Ok(value)
        }
    }
    fn string(&mut self) -> Result<JavaString, SnbtError> {
        self.skip();
        let Some(quote @ (0x22 | 0x27)) = self.peek() else {
            self.record(self.cursor, &["\"", "'"]);
            return self.unquoted().map(|value| value.as_str().into());
        };
        self.bump();
        let mut units = Vec::new();
        loop {
            match self.bump() {
                Some(c) if c == quote => return Ok(JavaString(units)),
                Some(0x5c) => self.escape(&mut units)?,
                Some(c) => push_code_point(&mut units, c),
                None => {
                    let mut error = self.error("argument.literal.incorrect");
                    let quote = char::from_u32(quote).expect("quote").to_string();
                    error.args.push(quote.clone());
                    error.suggestions.push(quote);
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
        let escape = self.bump();
        let value = match escape {
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
                let mut value = 0u32;
                for _ in 0..count {
                    let Some(digit) = self
                        .peek()
                        .and_then(char::from_u32)
                        .and_then(|c| c.to_digit(16))
                    else {
                        let mut error = self.error("snbt.parser.expected_hex_escape");
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
                self.expect(b'}')?;
                crate::character_names::code_point_of(&name)
                    .ok_or_else(|| self.error("snbt.parser.invalid_character_name"))?
            }
            _ => return Err(self.error("snbt.parser.invalid_string_contents")),
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
        let mut values = Vec::new();
        if !self.eat(b']') {
            loop {
                values.push(if let Some(kind) = typed {
                    self.integer(Some(kind))?
                } else {
                    self.value()?
                });
                if self.eat(b']') {
                    break;
                }
                self.expect(b',')?;
                if self.eat(b']') {
                    break;
                }
            }
        }
        Ok(match typed {
            Some(0) => Tag::ByteArray(
                values
                    .into_iter()
                    .map(|value| integer_value(value) as u8)
                    .collect(),
            ),
            Some(2) => Tag::IntArray(
                values
                    .into_iter()
                    .map(|value| integer_value(value) as i32)
                    .collect(),
            ),
            Some(3) => Tag::LongArray(values.into_iter().map(integer_value).collect()),
            _ => Tag::List(values),
        })
    }
    fn sign(&mut self) -> bool {
        self.eat(b'-') || {
            self.eat(b'+');
            false
        }
    }
    fn digits(&mut self, radix: u32) -> Result<Option<String>, SnbtError> {
        self.skip();
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
            Ok(Some(text))
        }
    }
    fn required_digits(&mut self, radix: u32) -> Result<String, SnbtError> {
        self.digits(radix)?.ok_or_else(|| {
            self.error(match radix {
                2 => "snbt.parser.expected_binary_numeral",
                16 => "snbt.parser.expected_hex_numeral",
                _ => "snbt.parser.expected_decimal_numeral",
            })
        })
    }
    fn exponent(&mut self, text: &mut String) -> Result<bool, SnbtError> {
        if !self.eat(b'e') && !self.eat(b'E') {
            return Ok(false);
        }
        text.push('e');
        if self.sign() {
            text.push('-');
        }
        text.push_str(&self.required_digits(10)?);
        Ok(true)
    }
    fn float(&mut self) -> Result<Option<Tag>, SnbtError> {
        let start = self.cursor;
        let minus = self.sign();
        let whole = self.digits(10)?;
        let mut text = String::new();
        if minus {
            text.push('-');
        }
        if let Some(whole) = &whole {
            text.push_str(whole);
        }
        let dot = self.eat(b'.');
        if dot {
            text.push('.');
            match self.digits(10)? {
                Some(fraction) => text.push_str(&fraction),
                None if whole.is_none() => {
                    return Err(self.error("snbt.parser.expected_decimal_numeral"));
                }
                None => {}
            }
        } else if whole.is_none() {
            self.cursor = start;
            return Ok(None);
        }
        let exponent = self.exponent(&mut text)?;
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
        if !dot && !exponent && suffix.is_none() {
            self.cursor = start;
            return Ok(None);
        }
        text.retain(|c| c != '_');
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
        let (mut digits, radix) = if self.eat(b'0') {
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
                    ("0".to_owned(), 10)
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
        digits.retain(|c| c != '_');
        let bits = [8, 16, 32, 64][usize::from(kind)];
        let magnitude = u64::from_str_radix(&digits, radix)
            .map_err(|_| self.integer_error(&digits, minus, kind, signed, radix, None))?;
        let max = if signed {
            (1u64 << (bits - 1)) - 1 + u64::from(minus)
        } else if bits == 64 {
            u64::MAX
        } else {
            (1u64 << bits) - 1
        };
        if magnitude > max {
            return Err(self.integer_error(&digits, minus, kind, signed, radix, Some(magnitude)));
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
        let word = match self.unquoted() {
            Ok(word) => word,
            Err(mut error) => {
                let values = [
                    "false", "true", "bool", "uuid", "{", "[", "\"", "'", "+", "-", "0", ".",
                ];
                self.record(error.cursor, &values);
                error.suggestions = values.map(str::to_owned).to_vec();
                return Err(error);
            }
        };
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
            Ok(Tag::String(word.as_str().into()))
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
