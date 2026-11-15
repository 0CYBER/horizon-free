//! The `player_name_entity` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0299

pub trait Hooks {
    /// A server player named a mob with a name tag, after its name and persistence changed and the tag
    /// was consumed. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_name_entity();
}
