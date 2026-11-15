//! The `potion_brew_pre` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A brewing stand at `pos` is about to brew (`PotionBrewEvent.Pre`,
    /// in `BrewingStandBlockEntity.doBrew`): `stacks` is a copy of its five
    /// slots (three bottles, the ingredient, the fuel). Set `cancel` to skip
    /// vanilla's brew: the stand then takes `stacks` as the mods left them,
    /// uses no ingredient and plays no brew sound, and `potion_brew_post`
    /// fires with those slots; `brewing_complete` doesn't, as nothing was
    /// brewed. Uncancelled, changes to `stacks` are dropped. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "PotionBrewEvent.Pre",
        cancellable = true
    )]
    fn potion_brew_pre();
}
