//! Placement framing delegates values to the shared Wire codec.
use super::{
    placement_sync::{Frame, RESOURCE_ID_BYTES},
    settings_sync::MAX_BYTES,
};
use mod_api::wire::{Reader, Writer};
// Three resource locations plus all scalar fields fit. Reject oversized requests
// before String decoding, rather than allocating a full catalogue-sized request.
const MAX_REQUEST_BYTES: usize = 3 * RESOURCE_ID_BYTES + 128;

pub(super) fn encode(frame: &Frame) -> Option<Vec<u8>> {
    let mut out = Writer::new();
    out.limit(MAX_BYTES - 1);
    out.put(frame);
    out.error().is_none().then(|| out.into_bytes())
}
// The owning settings-channel decoder has already checked MAX_BYTES.
pub(super) fn decode(bytes: &[u8]) -> Option<Frame> {
    let tag = u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?);
    if matches!(tag, 0 | 2 | 5) && bytes.len() > MAX_REQUEST_BYTES {
        return None;
    }
    Reader::decode(bytes).ok()
}
