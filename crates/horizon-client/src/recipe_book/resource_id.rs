//! Resource-location syntax shared by book keys and placement identities.
pub(super) fn valid(id: &str) -> bool {
    let Some((namespace, path)) = id.split_once(':') else {
        return false;
    };
    let allowed = |byte: u8, slash| {
        matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-') || slash && byte == b'/'
    };
    !namespace.is_empty()
        && !path.is_empty()
        && namespace.bytes().all(|byte| allowed(byte, false))
        && path.bytes().all(|byte| allowed(byte, true))
}
