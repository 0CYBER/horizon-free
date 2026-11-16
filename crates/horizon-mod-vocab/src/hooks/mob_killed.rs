//! The `mob_killed` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0049

pub trait Hooks {
    /// `player`'s melee blow killed the mob `entity` of `kind` (other deaths
    /// aren't told yet). It only hears, after the blow landed. Server-side.
    #[hook(side = "server", contract = "listen", neoforge = "LivingDeathEvent")]
    fn mob_killed();
}
