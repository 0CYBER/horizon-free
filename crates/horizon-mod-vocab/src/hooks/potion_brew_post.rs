//! The `potion_brew_post` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A brewing stand at `pos` brewed (`PotionBrewEvent.Post`, at the end of
    /// `BrewingStandBlockEntity.doBrew`), or `potion_brew_pre` was cancelled:
    /// `stacks` holds its five slots after the brew (or as the cancelling
    /// mods left them), and what the mods leave there is the stand's. Fires
    /// last, as `doBrew` ends: after `brewing_complete`, the brew sound and
    /// the ingredient's remainder dropping. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "PotionBrewEvent.Post",
        cancellable = false
    )]
    fn potion_brew_post();
}
