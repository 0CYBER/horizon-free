//! Window-side mining prediction.

pub trait Hooks {
    /// Veto a predicted client dig after the interaction mapping and vanilla eligibility checks.
    /// Auto-tool and mining overlay mods can use this instead of a mining prediction mixin.
    /// The snapshot is read-only. A veto skips the new mining operation and preserves any
    /// retained dig. Release and correction still perform ordinary cleanup.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "none (client mining prediction)"
    )]
    fn client_mining_predict();
}
