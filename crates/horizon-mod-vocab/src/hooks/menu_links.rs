//! The `menu_links` hook: declarations read by the mod API generator.
pub trait Hooks {
    /// Containers exposed by linked slots of server player `player`'s open mod menu
    /// `menu` (`namespace:path`) at `pos`. The player's state is not swapped in.
    /// Push container positions to `links`, in order. The engine resolves containers and
    /// refreshes their linked slots in the same tick. Asked each server tick only for open
    /// mod menu types declaring links, and dispatched only to subscribers. With no
    /// subscribers the linked-container tick does no work and allocates nothing.
    /// Silently skips `pos`, duplicate positions, missing or non-lendable containers,
    /// and positions after 64 accepted containers (`MAX_MENU_LINKS`). First accepted
    /// occurrence wins, preserving answer order.
    /// Native and wasm listeners append positions with identical copy-out semantics.
    #[hook(
        side = "server",
        contract = "collect",
        out = "links",
        neoforge = "none (Horizon mod menus expose linked container slots)",
        cancellable = false
    )]
    fn menu_links();
}
