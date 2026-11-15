//! Declarations carried from upstream `entity_change_block_v2`; the generator owns the typed bindings.

pub trait Hooks {
    /// A mob or a player is about to change the block at `pos`: a landing's
    /// trampling (farmland to dirt, a turtle egg breaking), before its
    /// neighbour updates and events. `player` is the player whose landing it
    /// was, `entity` and `kind` the mob for a mob's landing (`None` and the
    /// player's type here). `from` and `to` are resolved block states. Set `cancel`
    /// and the block stays as it was and nothing the change would have caused
    /// happens, as Bukkit's `EntityChangeBlockEvent` (@API:entity_change_block_v2@).
    /// The fork names this pre-change veto `entity_change_block_pre`; `entity_change_block` is the separate post-change notice.
    #[hook(
        side = "server",
        contract = "cancel",
        neoforge = "none (NeoForge has no such event; Bukkit's EntityChangeBlockEvent is cancellable)",
        cancellable = true,
        out = "cancel",
        target = "horizon_client::game::native_hooks::entity_change_block_pre::Game<'_>::native_entity_change_block_pre"
    )]
    fn entity_change_block_pre(
        player: Option<u128>,
        entity: Option<u32>,
        kind: &str,
        pos: [i32; 3],
        from: BlockState,
        to: BlockState,
        cancel: &mut bool,
    );
}
