//! The `fluid_flow` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The current of the fluid block at `pos` (`FlowingFluid.getFlow`, the vector that pushes an entity standing in it) is about to push a player, mob, item, experience orb, armor stand or arrow. `fluid` is the fluid's id (`minecraft:water`, `minecraft:flowing_lava`...); a mod may change `flow`, so it can rotate, strengthen or still a current. It runs for each fluid block the entity touches, so keep it short. The vector the renderer reads to turn a fluid's texture is not changed. Fires on both sides, on the client's replayed ticks too, with the loop `context` of the side that ticks the entity (its registries, settings and mod list): register the mod on both, and keep the listener a pure function of its inputs so prediction and the server agree. Beyond NeoForge: there is no counterpart. Diverges from upstream: new hook.
    #[notes = r#"## Fluid flow

- **`fluid_flow(context, fluid, pos, flow)`**: edit the current (`FlowingFluid.getFlow`) of the fluid block at `pos` as it pushes an entity: the player, mobs, dropped items, experience orbs, armor stands and arrows all read it through the one shared helper. It runs for each fluid block the entity touches in a tick, before the vanilla scale for shallow fluid, so keep the listener short. The renderer's own flow vector (a fluid texture's rotation) is not changed. `level.fluid_flow(host, pos)` reads the edited current. It fires on both sides and on the client's replayed ticks, with the loop `context` of the side that ticks the entity, so a mod registers on both and keeps the listener a pure function of its inputs. With no listener the cost is one `listened` branch per fluid block. Beyond NeoForge (no counterpart); Diverges from upstream: new hook."#]
    #[hook(
        side = "both",
        contract = "listen",
        context = "loop",
        cancellable = false,
        version = 2,
        target = "horizon_client_lib::motion::native_fluid_flow",
        neoforge = "none (vanilla FlowingFluid.getFlow; NeoForge adds no event or extension for it)"
    )]
    fn fluid_flow();
}
