//! The `brewing_complete` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0182

pub trait Hooks {
    /// A brewing stand finished brewing at `pos`, after its bottle contents
    /// were changed. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn brewing_complete();
}
