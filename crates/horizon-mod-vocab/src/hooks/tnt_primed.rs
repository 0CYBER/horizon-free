//! The `tnt_primed` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0268

pub trait Hooks {
    /// TNT at `pos` is about to be lit (primed): a flint and steel or a fire
    /// charge on it, fire burning it, a redstone signal, an arrow, or a
    /// dispenser's flint and steel or TNT item. `player` is the server player
    /// whose click lit it, with their state swapped in (so `PlayerRef::tell`
    /// answers them), None when no player is behind it (fire, redstone, an
    /// arrow, a dispenser). Set `cancel` and it is not primed: a TNT block
    /// stays where it is (one that fire or a signal had already taken out is
    /// put back, without a block update), a dispenser's TNT item stays in it.
    /// Not asked: TNT set off by a blast (`explosion_detonate` spares
    /// blocks), a TNT minecart and a sulfur cube. Server-side and in single
    /// player.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        cancellable = true,
        neoforge = "none (NeoForge has no TNT priming event; the primed TNT entity joins with EntityJoinLevelEvent)"
    )]
    fn tnt_primed();
}
