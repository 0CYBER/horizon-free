//! The `validate_tickets` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `dimension`'s level has loaded the saved chunk tickets of `controller`
    /// (NeoForge's `LoadingValidationCallback.validateTickets`), before its
    /// first tick and after `level_load`, once for each open and each
    /// controller registered with a validator that has tickets in the level.
    /// `validation` holds that controller's tickets, block owners and entity
    /// owners apart; the mod that owns the controller removes the ones that no
    /// longer hold (a machine that is gone), and they stop forcing their
    /// chunks. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "LoadingValidationCallback",
        cancellable = false
    )]
    fn validate_tickets();
}
