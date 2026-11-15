//! The `living_experience_drop` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A dying mob or player `entity` of `kind` is about to drop `original`
    /// points of experience; the vanilla conditions have been checked.
    /// `attacking_player` is the player who last hurt it, if any. Change
    /// `amount` (0 or less drops nothing), or set `cancel` to drop none. A
    /// sculk catalyst that takes the experience does not call this.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "LivingExperienceDropEvent",
        cancellable = true
    )]
    fn living_experience_drop();
}
