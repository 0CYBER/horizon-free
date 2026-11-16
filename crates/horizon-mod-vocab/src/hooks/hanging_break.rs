//! The `hanging_break` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0244

pub trait Hooks {
    /// An item frame (`minecraft:item_frame`, `minecraft:glow_item_frame`) or
    /// a painting (`minecraft:painting`), `kind` its entity type and `pos` the
    /// block it hangs in, is about to break: a server player's hit
    /// (`player`, their state swapped in, so `PlayerRef::tell` answers them), a
    /// blast, or the wall it hangs on gone (`player` None for both). Where
    /// `BlockAttachedEntity.hurtServer` and its survival check would kill
    /// it, before anything drops or sounds. Set `cancel` and it stays, as it
    /// was. A hit that only takes the item out of a frame does not break the
    /// frame and is not asked. The game's own player's hit is not asked, nor
    /// is `/kill`.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        cancellable = true,
        neoforge = "none (vanilla hanging entity breaking; NeoForge adds no event or extension for it); AttackEntityEvent fires for players only"
    )]
    fn hanging_break();
}
