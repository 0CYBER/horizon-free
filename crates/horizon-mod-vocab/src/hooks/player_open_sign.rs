//! The `player_open_sign` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0300

pub trait Hooks {
    /// A server player opened a sign for editing, after the editor state was made. `front` selects the
    /// front or back side. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_open_sign();
}
