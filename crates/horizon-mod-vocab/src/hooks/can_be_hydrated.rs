//! The `can_be_hydrated` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Whether `fluid` at `fluid_pos` waters the farmland this part is attached to, the block
    /// `state` at `pos` (`IBlockExtension.canBeHydrated`): set `hydrates` to `Some(true)` or
    /// `Some(false)`, or leave it `None` for vanilla's rule, a fluid in `#minecraft:water`.
    /// `FarmlandBlock.randomTick` scans the 9×2×9 box from `pos` (x and z from −4 to 4, y 0 and 1)
    /// and asks only where the fluid is not empty, in x, then y, then z order; the first position
    /// that hydrates ends the scan, and the farmland goes to moisture 7. Rain on the block above,
    /// moisture changes and drying out are unchanged. The farmland decides which fluids water it.
    /// Asked on an authoritative server; the level is read-only. With no part attached to the
    /// farmland, the scan is vanilla's. Diverges from upstream: new hook.
    #[hook(
        side = "server",
        contract = "replace",
        out = "hydrates",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::can_be_hydrated::GameLevel<'_>::native_can_be_hydrated",
        neoforge = "IBlockExtension.canBeHydrated"
    )]
    fn can_be_hydrated();
}
