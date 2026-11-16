//! The `fill_bucket` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0043

pub trait Hooks {
    /// A player fills (`stack` an empty bucket) or empties a bucket at `pos`:
    /// `BucketItem.use`, where it asks `Level.mayInteract`. Set `canceled`
    /// and the bucket does nothing. Server-side, for every player of the
    /// world (`host.players().acting_player()` says who).
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "none (NeoForge 26.3 has no bucket-fill event)"
    )]
    fn fill_bucket();
}
