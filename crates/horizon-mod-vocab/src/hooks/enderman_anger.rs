//! The `enderman_anger` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `player` stares at `enderman` (NeoForge's `EndermanAngerEvent`, at
    /// the end of `Enderman.isBeingStaredBy`): the look is within the
    /// enderman's cone, in line of sight, and no carved pumpkin hides the
    /// face. Asked each time the enderman's goals check the stare: to get
    /// angry, to freeze while looked at and to teleport away up close. Set
    /// `cancel` and that stare doesn't count. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        host = "read",
        neoforge = "EndermanAngerEvent",
        cancellable = true
    )]
    fn enderman_anger();
}
