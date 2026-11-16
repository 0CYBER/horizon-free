//! The `fluid_rules` hook: a declaration the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// How a mod's world fluid flows at `pos`, starting from what it declared (NeoForge's
    /// `BaseFlowingFluid` and `FluidType` getters: `getTickDelay`, `getDropOff`,
    /// `getSlopeFindDistance`, `canConvertToSource`). `fluid` is the id it was declared with.
    /// Asked for every flow step and every cell a spread looks at: answer fast. An answer
    /// outside what `WorldFluid` accepts is brought back within it. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        scope = "fluid",
        key = "fluid",
        neoforge = "none (BaseFlowingFluid.getTickDelay, getDropOff, getSlopeFindDistance and FluidType.canConvertToSource are overrides; NeoForge adds no event)",
        target = "horizon_client::game::native_hooks::fluid_rules::GameLevel<'_>::native_fluid_rules"
    )]
    fn fluid_rules(
        dimension: LevelDimension,
        pos: BlockPos,
        fluid: &str,
        tick_delay: &mut i32,
        drop_off: &mut i32,
        slope_find_distance: &mut i32,
        converts_to_source: &mut bool,
    );
}
