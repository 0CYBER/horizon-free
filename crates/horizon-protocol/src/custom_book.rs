//! Private custom recipe-book synchronization capability, independent of mod payloads.
/// Configuration advertisement and play Offer/Snapshot channel.
pub const CHANNEL: &str = "horizon:recipe_book_settings";
/// The only supported configuration advertisement body (protocol version 1).
pub const CAPABILITY: &[u8] = &[1];
