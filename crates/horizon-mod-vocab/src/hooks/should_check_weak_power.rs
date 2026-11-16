//! The `should_check_weak_power` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `IBlockExtension.shouldCheckWeakPower`: set `check` to decide whether
    /// indirect power is read through this block. `check` starts with vanilla's
    /// conductor answer. `direction` is the input side of the block being powered,
    /// opposite the queried block's output side. Authoritative servers only.
    #[hook(
        side = "server",
        contract = "collect",
        out = "check",
        scope = "block",
        key = "state",
        neoforge = "IBlockExtension.shouldCheckWeakPower",
        target = "horizon_client::game::native_hooks::should_check_weak_power::GameLevel<'_>::native_should_check_weak_power"
    )]
    fn should_check_weak_power();
}
