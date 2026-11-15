//! The `block_render_visibility` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Hides a block state from the mesher: set `event.hidden` and the state draws nothing, as if its blocks were air. Its neighbours' faces toward it show, it no longer hides what is behind it, and its fluid layer and block entity are not drawn; light, collision and the server are untouched. The answer depends on the state alone and is resolved at mesh time, never per frame: the game asks once for every state when the client joins a world, and again for every state each time a mod calls `ClientHost::refresh_render_visibility` (a toggle, an ore scanner). Attach the part to vanilla blocks with `block_part_tag`. With no mod answering, the mesher pays nothing.
    #[hook(
        side = "client",
        contract = "collect",
        out = "hidden",
        scope = "block",
        key = "state",
        neoforge = "none (Horizon-only; NeoForge has no counterpart)"
    )]
    fn block_render_visibility();
}
