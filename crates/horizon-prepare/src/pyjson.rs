//! JSON read and written exactly as Python's `json` module does, so the files this crate writes
//! are byte for byte the ones `tools/prepare_native_client.py` wrote: objects keep their key
//! order (a repeated key keeps its first place and its last value), integers keep every digit,
//! floats print as Python's `repr`, and strings are escaped as `ensure_ascii=True` escapes them.

use std::fmt::Write;

#[derive(Clone, Debug)]
pub enum Value {
    Null,
    Bool(bool),
    /// A JSON integer, kept as its decimal digits (Python's integers have no size limit).
    Int(String),
    Float(f64),
    Str(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

impl Value {
    pub fn str(s: impl Into<String>) -> Value {
        Value::Str(s.into())
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn index(&self, i: usize) -> Option<&Value> {
        match self {
            Value::Array(items) => items.get(i),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&[(String, Value)]> {
        match self {
            Value::Object(fields) => Some(fields),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(digits) => digits.parse().ok(),
            _ => None,
        }
    }

    /// `dict[key] = value`: a new key goes last, an existing one keeps its place.
    pub fn insert(&mut self, key: &str, value: Value) {
        if let Value::Object(fields) = self {
            match fields.iter_mut().find(|(k, _)| k == key) {
                Some(field) => field.1 = value,
                None => fields.push((key.to_string(), value)),
            }
        }
    }
}

/// Python's `==`: objects compare without regard to key order, and 1 == 1.0.
impl PartialEq for Value {
    fn eq(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Null, Value::Null) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Int(a), Value::Float(b)) | (Value::Float(b), Value::Int(a)) => {
                a.parse::<f64>().is_ok_and(|a| a == *b && b.fract() == 0.0)
            }
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Array(a), Value::Array(b)) => a == b,
            (Value::Object(a), Value::Object(b)) => {
                a.len() == b.len() && a.iter().all(|(k, v)| other.get(k) == Some(v))
            }
            _ => false,
        }
    }
}

pub fn loads(text: &str) -> Result<Value, String> {
    let mut parser = Parser {
        text: text.as_bytes(),
        at: 0,
    };
    parser.space();
    let value = parser.value()?;
    parser.space();
    if parser.at != parser.text.len() {
        return Err(parser.error("Extra data"));
    }
    Ok(value)
}

struct Parser<'a> {
    text: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn error(&self, what: &str) -> String {
        format!("{what} at byte {}", self.at)
    }

    fn space(&mut self) {
        while matches!(self.text.get(self.at), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn literal(&mut self, word: &str) -> bool {
        if self.text[self.at..].starts_with(word.as_bytes()) {
            self.at += word.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self) -> Result<Value, String> {
        match self.text.get(self.at) {
            Some(b'{') => {
                self.at += 1;
                let mut object = Value::Object(Vec::new());
                self.space();
                if self.text.get(self.at) == Some(&b'}') {
                    self.at += 1;
                    return Ok(object);
                }
                loop {
                    self.space();
                    if self.text.get(self.at) != Some(&b'"') {
                        return Err(self.error("Expecting property name enclosed in double quotes"));
                    }
                    let key = self.string()?;
                    self.space();
                    if self.text.get(self.at) != Some(&b':') {
                        return Err(self.error("Expecting ':' delimiter"));
                    }
                    self.at += 1;
                    self.space();
                    let value = self.value()?;
                    object.insert(&key, value);
                    self.space();
                    match self.text.get(self.at) {
                        Some(b',') => self.at += 1,
                        Some(b'}') => {
                            self.at += 1;
                            return Ok(object);
                        }
                        _ => return Err(self.error("Expecting ',' delimiter")),
                    }
                }
            }
            Some(b'[') => {
                self.at += 1;
                let mut items = Vec::new();
                self.space();
                if self.text.get(self.at) == Some(&b']') {
                    self.at += 1;
                    return Ok(Value::Array(items));
                }
                loop {
                    self.space();
                    items.push(self.value()?);
                    self.space();
                    match self.text.get(self.at) {
                        Some(b',') => self.at += 1,
                        Some(b']') => {
                            self.at += 1;
                            return Ok(Value::Array(items));
                        }
                        _ => return Err(self.error("Expecting ',' delimiter")),
                    }
                }
            }
            Some(b'"') => Ok(Value::Str(self.string()?)),
            Some(b'n') if self.literal("null") => Ok(Value::Null),
            Some(b't') if self.literal("true") => Ok(Value::Bool(true)),
            Some(b'f') if self.literal("false") => Ok(Value::Bool(false)),
            Some(b'N') if self.literal("NaN") => Ok(Value::Float(f64::NAN)),
            Some(b'I') if self.literal("Infinity") => Ok(Value::Float(f64::INFINITY)),
            Some(b'-') if self.literal("-Infinity") => Ok(Value::Float(f64::NEG_INFINITY)),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(self.error("Expecting value")),
        }
    }

    /// Python's NUMBER_RE: `(-?(?:0|[1-9]\d*))(\.\d+)?([eE][-+]?\d+)?`.
    fn number(&mut self) -> Result<Value, String> {
        let start = self.at;
        let digits = |p: &mut Self| {
            let from = p.at;
            while p.text.get(p.at).is_some_and(u8::is_ascii_digit) {
                p.at += 1;
            }
            p.at - from
        };
        if self.text.get(self.at) == Some(&b'-') {
            self.at += 1;
        }
        if self.text.get(self.at) == Some(&b'0') {
            self.at += 1;
        } else if digits(self) == 0 {
            self.at = start;
            return Err(self.error("Expecting value"));
        }
        let integer_end = self.at;
        let mut float = false;
        if self.text.get(self.at) == Some(&b'.') {
            let dot = self.at;
            self.at += 1;
            if digits(self) == 0 {
                self.at = dot;
            } else {
                float = true;
            }
        }
        if matches!(self.text.get(self.at), Some(b'e' | b'E')) {
            let mark = self.at;
            self.at += 1;
            if matches!(self.text.get(self.at), Some(b'-' | b'+')) {
                self.at += 1;
            }
            if digits(self) == 0 {
                self.at = mark;
            } else {
                float = true;
            }
        }
        let text = std::str::from_utf8(&self.text[start..self.at]).unwrap();
        if float {
            text.parse::<f64>()
                .map(Value::Float)
                .map_err(|e| e.to_string())
        } else {
            debug_assert_eq!(self.at, integer_end);
            Ok(Value::Int(if text == "-0" {
                "0".into()
            } else {
                text.into()
            }))
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self
            .text
            .get(self.at..self.at + 4)
            .and_then(|d| std::str::from_utf8(d).ok())
            .filter(|d| d.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or_else(|| self.error("Invalid \\uXXXX escape"))?;
        self.at += 4;
        Ok(u32::from_str_radix(digits, 16).unwrap())
    }

    fn string(&mut self) -> Result<String, String> {
        self.at += 1;
        let mut out = String::new();
        loop {
            let run = self.at;
            while self
                .text
                .get(self.at)
                .is_some_and(|&b| b != b'"' && b != b'\\' && b >= 0x20)
            {
                self.at += 1;
            }
            out.push_str(std::str::from_utf8(&self.text[run..self.at]).map_err(|e| e.to_string())?);
            match self.text.get(self.at) {
                Some(b'"') => {
                    self.at += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.at += 1;
                    let escape = *self
                        .text
                        .get(self.at)
                        .ok_or_else(|| self.error("Unterminated string"))?;
                    self.at += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let mut code = self.hex4()?;
                            if (0xd800..0xdc00).contains(&code)
                                && self.text[self.at..].starts_with(b"\\u")
                            {
                                let back = self.at;
                                self.at += 2;
                                let low = self.hex4()?;
                                if (0xdc00..0xe000).contains(&low) {
                                    code = 0x10000 + ((code - 0xd800) << 10) + (low - 0xdc00);
                                } else {
                                    self.at = back;
                                }
                            }
                            out.push(char::from_u32(code).ok_or_else(|| {
                                self.error(
                                    "Lone surrogate (Python keeps it; Horizon's files have none)",
                                )
                            })?);
                        }
                        _ => return Err(self.error("Invalid \\escape")),
                    }
                }
                _ => return Err(self.error("Unterminated string or control character")),
            }
        }
    }
}

/// `json.dumps(value, indent=indent, separators=(item, key))` with `ensure_ascii=True`.
/// Python's defaults are `(", ", ": ")` without an indent and `(",", ": ")` with one.
pub struct Format {
    pub indent: Option<usize>,
    pub item: &'static str,
    pub key: &'static str,
}

/// `json.dumps(value)`.
pub const DEFAULT: Format = Format {
    indent: None,
    item: ", ",
    key: ": ",
};
/// `json.dumps(value, separators=(',', ':'))`.
pub const COMPACT: Format = Format {
    indent: None,
    item: ",",
    key: ":",
};
/// `json.dumps(value, indent=2)`.
pub const INDENT2: Format = Format {
    indent: Some(2),
    item: ",",
    key: ": ",
};

pub fn dumps(value: &Value, format: &Format) -> String {
    let mut out = String::new();
    write_value(&mut out, value, format, 0);
    out
}

fn newline(out: &mut String, format: &Format, level: usize) {
    if let Some(indent) = format.indent {
        out.push('\n');
        out.extend(std::iter::repeat_n(' ', indent * level));
    }
}

fn write_value(out: &mut String, value: &Value, format: &Format, level: usize) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(digits) => out.push_str(digits),
        Value::Float(f) => out.push_str(&float_repr(*f)),
        Value::Str(s) => write_string(out, s),
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(format.item);
                }
                newline(out, format, level + 1);
                write_value(out, item, format, level + 1);
            }
            newline(out, format, level);
            out.push(']');
        }
        Value::Object(fields) => {
            if fields.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push('{');
            for (i, (key, item)) in fields.iter().enumerate() {
                if i > 0 {
                    out.push_str(format.item);
                }
                newline(out, format, level + 1);
                write_string(out, key);
                out.push_str(format.key);
                write_value(out, item, format, level + 1);
            }
            newline(out, format, level);
            out.push('}');
        }
    }
}

/// Python's `py_encode_basestring_ascii`: every character outside space..`~` is escaped,
/// with lowercase hex and surrogate pairs above the BMP.
pub fn write_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ' '..='~' => out.push(c),
            _ => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    let _ = write!(out, "\\u{unit:04x}");
                }
            }
        }
    }
    out.push('"');
}

/// Python's `repr(float)`: the shortest digits that read back the same, in fixed notation when
/// the decimal point falls between 10^-4 and 10^16, with `.0` on whole numbers.
pub fn float_repr(f: f64) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    // Rust's `{:e}` gives the same shortest round-trip digits: "d.ddde-x".
    let scientific = format!("{:e}", f.abs());
    let (mantissa, exponent) = scientific.split_once('e').unwrap();
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let exponent: i32 = exponent.parse().unwrap();
    let point = exponent + 1; // The value is 0.digits * 10^point.
    let sign = if f.is_sign_negative() { "-" } else { "" };
    let n = digits.len() as i32;
    if f == 0.0 {
        return format!("{sign}0.0");
    }
    if -4 < point && point <= 16 {
        if point <= 0 {
            format!("{sign}0.{}{digits}", "0".repeat((-point) as usize))
        } else if point >= n {
            format!("{sign}{digits}{}.0", "0".repeat((point - n) as usize))
        } else {
            let (whole, fraction) = digits.split_at(point as usize);
            format!("{sign}{whole}.{fraction}")
        }
    } else {
        let (first, rest) = digits.split_at(1);
        let mantissa = if rest.is_empty() {
            first.to_string()
        } else {
            format!("{first}.{rest}")
        };
        let e = point - 1;
        format!(
            "{sign}{mantissa}e{}{:02}",
            if e < 0 { '-' } else { '+' },
            e.abs()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats_print_as_python_repr() {
        for (f, repr) in [
            (0.0, "0.0"),
            (-0.0, "-0.0"),
            (1.0, "1.0"),
            (0.1, "0.1"),
            (1.5e-5, "1.5e-05"),
            (0.0001, "0.0001"),
            (1e16, "1e+16"),
            (1234567890123456.0, "1234567890123456.0"),
            (12345678901234567.0, "1.2345678901234568e+16"),
            (-2.5, "-2.5"),
            (1e-310, "1e-310"),
            (f64::MAX, "1.7976931348623157e+308"),
            (100.0, "100.0"),
            (0.30000000000000004, "0.30000000000000004"),
        ] {
            assert_eq!(float_repr(f), repr, "{f}");
        }
    }

    #[test]
    fn strings_escape_as_ensure_ascii() {
        let mut out = String::new();
        write_string(&mut out, "a\"\\\n\u{7f}é\u{1F600}\u{1}");
        assert_eq!(out, "\"a\\\"\\\\\\n\\u007f\\u00e9\\ud83d\\ude00\\u0001\"");
    }

    #[test]
    fn objects_keep_first_place_and_last_value() {
        let v = loads(r#"{"b": 1, "a": [1.0, -0, 1e2, true, null], "b": 2}"#).unwrap();
        assert_eq!(
            dumps(&v, &DEFAULT),
            r#"{"b": 2, "a": [1.0, 0, 100.0, true, null]}"#
        );
        assert_eq!(
            dumps(&v, &INDENT2),
            "{\n  \"b\": 2,\n  \"a\": [\n    1.0,\n    0,\n    100.0,\n    true,\n    null\n  ]\n}"
        );
        assert_eq!(
            dumps(&loads("{\"x\":{},\"y\":[]}").unwrap(), &INDENT2),
            "{\n  \"x\": {},\n  \"y\": []\n}"
        );
    }
}
