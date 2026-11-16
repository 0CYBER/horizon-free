//! The `fluid_tick` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The fluid at `pos` is about to run its scheduled tick (`FlowingFluid.tick`). `fluid` is
    /// `water`, `lava` or a mod fluid's id. Set `cancel` and the tick does nothing at all: no
    /// level change, no spreading and no new tick. For a mod that simulates a fluid itself;
    /// `fluid_level_change` and `block_spread` still ask about each change of a tick that runs.
    /// A cancelled tick schedules nothing; a mod taking over must re-schedule itself.
    /// It's asked once per tick, so keep it cheap. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event)",
        cancellable = true,
        target = "horizon_client::game::native_hooks::fluid_tick::GameLevel<'_>::native_fluid_tick"
    )]
    fn fluid_tick();
}
