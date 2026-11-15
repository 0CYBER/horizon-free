//! The `use_item` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0037

pub trait Hooks {
    /// A player uses `stack` (right click) with `self.hand` on the block at
    /// `pos`, `face` the face clicked (`[0, 1, 0]` is the top), once the
    /// block itself let the click through (a chest still opens): where
    /// `ItemStack.useOn` starts. Both logical sides answer independently:
    /// a consuming client result ends the hand loop; only Success swings
    /// there. SuccessServer and Consume consume without a client swing.
    /// Fail ends the block path; Pass and TryWithEmptyHand fall through.
    /// A server-only answer does not stop the client's off-hand prediction.
    /// ClientHost permits reads and local effects, not authoritative writes.
    /// Cooldown items are skipped. `hand` and `hit` describe this click;
    /// `player` and `player_entity_id` identify its player on this side.
    ///
    /// Client prediction cannot invoke server services:
    /// ```compile_fail
    /// use mod_api::{ClientHost, prelude::*};
    /// fn predict(host: &mut dyn ClientHost) {
    ///     host.commands().run_command("say not a client prediction");
    /// }
    /// ```
    #[hook(
        side = "both",
        version = 2,
        contract = "replace",
        out = "result",
        scope = "item",
        key = "stack",
        neoforge = "UseItemOnBlockEvent"
    )]
    fn use_on(
        stack: &ItemStack,
        pos: [i32; 3],
        face: [i32; 3],
        hand: Hand,
        hit: BlockHit,
        player: u128,
        player_entity_id: i32,
        result: &mut Option<InteractionResult>,
    );
    /// A player uses `stack` (right click) with `self.hand`: in the air, or
    /// on a block once `native_use_on` let it through, where `Item.use`
    /// starts. Runs on both logical sides after the global right-click event.
    /// Return an interaction result: Success swings on the client, SuccessServer
    /// swings on the server, Consume handles without swinging. Fail skips this
    /// item's use without swinging, then the off hand may be tried.
    /// None lets later Parts answer; if none answers, the item's own use runs.
    /// Some(Pass) skips later Parts and continues to the item's own use.
    /// TryWithEmptyHand is block-phase-only and is treated as Pass here.
    /// Cooldown items are skipped.
    /// A server-only answer does not stop the client's off-hand prediction.
    /// ClientHost permits reads and local effects, not authoritative writes.
    /// `hand` is the interaction hand of THIS click, including an instantaneous
    /// air use. Unlike this field, `living_used_item_hand` answers only during
    /// active item use. This is `PlayerInteractEvent.RightClickItem.getHand`.
    /// `player` is the clicking player's UUID and `player_entity_id` that
    /// player's entity id on this logical side: the acting player's on a server
    /// (a guest's own, never the host's), this client's own player on a client.
    /// They name who clicked from the event alone, unlike `players().acting_player()`,
    /// which only answers while the host has set the acting player.
    #[hook(
        side = "both",
        version = 2,
        contract = "replace",
        out = "result",
        scope = "item",
        key = "stack",
        neoforge = "PlayerInteractEvent.RightClickItem"
    )]
    fn use_item(
        stack: &ItemStack,
        hand: Hand,
        player: u128,
        player_entity_id: i32,
        result: &mut Option<InteractionResult>,
    );
}
