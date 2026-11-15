//! The `village_siege` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A village siege found a spawn position on the server. Set `canceled`
    /// to stop the siege. There is one siege per dimension.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "VillageSiegeEvent",
        cancellable = true
    )]
    fn village_siege();
}
