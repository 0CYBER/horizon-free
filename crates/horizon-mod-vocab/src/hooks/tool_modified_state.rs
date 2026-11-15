//! The `tool_modified_state` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The state the block `state` becomes under a tool action (`IBlockExtension.getToolModifiedState`:
    /// an axe's strip, scrape or wax-off, a shovel's flatten or douse, a hoe's till), asked of the
    /// block a tool is used on (`tool.pos`). `final_state` starts with vanilla's answer, `None` for a
    /// block the tool does nothing to: set it to `Some(new)` to allow the action or replace its
    /// result (a mod log an axe strips, a path a shovel makes), and `Cancel` it to deny the action,
    /// so the tool does nothing. `tool.simulated` says the game is only asking what the tool would
    /// do (`LevelRef::block_transform`, NeoForge's `simulate`): answer the same and change nothing in
    /// the world. A part answers before the global `block_tool_modification`, which still hears the
    /// result and may change or cancel it; a cancelled part skips it. Asked for the block's own parts only, so
    /// a block with none costs nothing.
    #[hook(
        side = "both",
        contract = "cancel",
        out = "cancel",
        scope = "block",
        key = "state",
        host = "read",
        neoforge = "IBlockExtension.getToolModifiedState"
    )]
    fn tool_modified_state();
}
