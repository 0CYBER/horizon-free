//! JSON-backed codecs validate nesting before entering serde's recursive code.
use super::{MAX_DEPTH, Reader, Wire, WireError, Writer};
use serde_json::Value;

pub(super) fn value_depth<'a>(roots: impl IntoIterator<Item = &'a Value>, depth: u16) -> bool {
    let mut pending: Vec<_> = roots.into_iter().map(|value| (value, depth)).collect();
    while let Some((value, depth)) = pending.pop() {
        if matches!(value, Value::Array(_) | Value::Object(_)) && depth >= MAX_DEPTH {
            return false;
        }
        match value {
            Value::Array(values) => pending.extend(values.iter().map(|child| (child, depth + 1))),
            Value::Object(values) => {
                pending.extend(values.values().map(|child| (child, depth + 1)))
            }
            _ => {}
        }
    }
    true
}

// Lexical preflight is not a JSON parser: serde still owns syntax validation.
// Quoted brackets and escaped quotes must not count toward container depth.
pub(super) fn text_depth(text: &str) -> Result<(), WireError> {
    let (mut depth, mut quoted, mut escaped) = (0u16, false, false);
    for byte in text.bytes() {
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
        } else {
            match byte {
                b'"' => quoted = true,
                b'[' | b'{' => {
                    if depth >= MAX_DEPTH {
                        return Err(WireError::Depth);
                    }
                    depth += 1;
                }
                b']' | b'}' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
    }
    Ok(())
}

pub(super) fn take<T: serde::de::DeserializeOwned>(input: &mut Reader<'_>) -> Result<T, WireError> {
    let text: String = input.take()?;
    text_depth(&text)?;
    serde_json::from_str(&text).map_err(|_| WireError::Invalid("not valid JSON for its type"))
}

impl Wire for crate::ModelPart {
    const MIN: usize = String::MIN;
    const LAYOUT_SEED: u64 = String::LAYOUT_SEED;
    fn layout(visiting: &mut Vec<&'static str>) -> u64 {
        String::layout(visiting)
    }
    fn put(&self, out: &mut Writer) {
        let mut pending = vec![(self, 0u16)];
        while let Some((part, depth)) = pending.pop() {
            if depth >= MAX_DEPTH {
                out.fail(WireError::Depth);
                return;
            }
            pending.extend(part.children().map(|(_, child)| (child, depth + 1)));
        }
        match serde_json::to_string(self) {
            Ok(text) => match text_depth(&text) {
                Ok(()) => out.put_str(&text),
                Err(error) => out.fail(error),
            },
            Err(_) => out.fail(WireError::Invalid("not valid model JSON")),
        }
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        take(input)
    }
}
