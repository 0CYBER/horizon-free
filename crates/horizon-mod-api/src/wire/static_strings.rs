//! Platform-owned bounded storage for static Wire strings.
use super::{MAX_STATIC_STRING_BYTES, MAX_STATIC_STRINGS, WireError};
use std::collections::HashSet;

pub(super) fn intern(value: &str) -> Result<&'static str, WireError> {
    use std::sync::{Mutex, OnceLock};
    static STRINGS: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    if value.len() > MAX_STATIC_STRING_BYTES {
        return Err(WireError::Invalid("static string too long"));
    }
    let mut strings = STRINGS
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(&existing) = strings.get(value) {
        return Ok(existing);
    }
    if strings.len() >= MAX_STATIC_STRINGS {
        return Err(WireError::Invalid("too many static strings"));
    }
    let value = Box::leak(value.to_owned().into_boxed_str());
    strings.insert(value);
    Ok(value)
}
