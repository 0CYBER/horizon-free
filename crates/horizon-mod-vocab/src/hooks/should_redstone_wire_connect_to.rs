//! The `should_redstone_wire_connect_to` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `BlockBehaviour.shouldRedstoneWireConnectTo`: set `connected` to say
    /// whether redstone wire connects to the block `state` at `pos`.
    /// `direction` points from the wire toward the neighbouring block, or is
    /// None when the wire asks about the block above or below. `connected` holds vanilla's answer: change
    /// it when the block is yours. Asked whenever wire settles its shape:
    /// answer from `state` and `pos` without touching the world. When the
    /// answer changes, and once the wire is placed, update its neighbours
    /// (`level.update_neighbors`). Authoritative servers only.
    #[hook(
        side = "server",
        contract = "collect",
        out = "connected",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::should_redstone_wire_connect_to::GameLevel<'_>::native_should_redstone_wire_connect_to",
        neoforge = "none (vanilla BlockBehaviour.shouldRedstoneWireConnectTo; NeoForge adds no event or extension for it)"
    )]
    fn should_redstone_wire_connect_to();
}
