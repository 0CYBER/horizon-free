//! The `permissions_changed` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `PermissionsChangedEvent`: the operator list is about to give player
    /// `player` (`name`) the permission level `new` in place of `old`, through `PlayerList.op`
    /// (`/op`, the console's `op`, the management API's `operators/add` and `operators/set`)
    /// or `PlayerList.deop` (`/deop` and the same on the console and the management API;
    /// `new` is then `All`). Set `cancel` and the list keeps the old level. The player need
    /// not be online: `player_entity_id` is their server entity id when they are. Clearing
    /// the list (`operators/clear`) is not a call to either, as in vanilla. Dedicated
    /// servers only; a single-player world has no operator list. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "PermissionsChangedEvent",
        cancellable = true
    )]
    fn permissions_changed();
}
