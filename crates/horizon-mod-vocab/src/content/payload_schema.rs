//! Typed mod payloads: the schema fingerprint of a payload's value type.
//! The fingerprint is the shape of the bytes a type writes, not its Rust names: a leaf's kind
//! and bound, a struct's field schemas in declaration order, an enum's variants by position and
//! their fields. `#[derive(PayloadCodec)]` writes it from the fields; a hand-written codec names
//! its own leaf. A type that holds itself, directly or through others, is marked by how far up
//! the walk it was entered, so the walk ends and the result does not depend on Rust names.
//! The fingerprint is part of a typed declaration's negotiated version (see
//! [`typed_payload_version`]), so two peers with the same id and another schema are told apart
//! at admission. It is the same on native and wasm targets.
use super::payload_codec::PayloadCodec;

const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const PRIME: u64 = 0x0000_0100_0000_01b3;

/// What a typed declaration's negotiated version holds after the mod's own version: the
/// marker, then the schema fingerprint as 16 lowercase hex digits. A raw declaration's version
/// cannot contain it.
pub const PAYLOAD_SCHEMA_MARKER: &str = "+payload-schema.";

/// The fingerprint of a leaf: a payload form named by `name` (`"varint"`, `"string"`, or a
/// hand-written codec's own name such as `"example:position/1"`), with no children.
pub const fn payload_schema_leaf(name: &str) -> u64 {
    let bytes = name.as_bytes();
    let mut hash = OFFSET;
    let mut i = 0;
    while i < bytes.len() {
        hash = (hash ^ bytes[i] as u64).wrapping_mul(PRIME);
        i += 1;
    }
    hash
}

/// `hash` followed by `child`: how a list, an option or a field joins its parent's fingerprint.
pub const fn payload_schema_mix(mut hash: u64, child: u64) -> u64 {
    let bytes = child.to_le_bytes();
    let mut i = 0;
    while i < bytes.len() {
        hash = (hash ^ bytes[i] as u64).wrapping_mul(PRIME);
        i += 1;
    }
    hash
}

/// Walks one named type: `shape` gives its fingerprint, unless the type is already being walked
/// further up (it holds itself), when the result is a back reference to that level. What a
/// derived codec's `schema` calls; `visiting` is the walk's stack of types, by `TypeId`.
// hidden: the derive's recursion guard; hand-written codecs name a leaf instead.
#[doc(hidden)]
pub fn payload_schema_named<T: ?Sized + 'static>(
    visiting: &mut Vec<std::any::TypeId>,
    shape: impl FnOnce(&mut Vec<std::any::TypeId>) -> u64,
) -> u64 {
    let id = std::any::TypeId::of::<T>();
    if let Some(level) = visiting.iter().rposition(|entered| *entered == id) {
        let up = (visiting.len() - level) as u64;
        return payload_schema_mix(payload_schema_leaf("recursive"), up);
    }
    visiting.push(id);
    let hash = shape(visiting);
    visiting.pop();
    hash
}

/// The schema fingerprint of `T`.
pub fn payload_schema<T: PayloadCodec + 'static>() -> u64 {
    T::schema(&mut Vec::new())
}

/// The negotiated version of a typed declaration: the mod's `version`, the marker, then the
/// schema fingerprint.
pub fn typed_payload_version(version: &str, schema: u64) -> String {
    format!("{version}{PAYLOAD_SCHEMA_MARKER}{schema:016x}")
}

#[cfg(test)]
#[path = "payload_schema_tests.rs"]
mod tests;
