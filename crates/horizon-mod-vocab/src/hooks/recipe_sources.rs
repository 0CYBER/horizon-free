//! The `recipe_sources` hook: declarations read by the mod API generator.
pub trait Hooks {
    /// Additional ingredient containers for server player `player` placing or checking a
    /// recipe in the mod crafting menu `menu` (`namespace:path`) at `pos`. The player's
    /// state is not swapped in. Push block positions to `sources`; containers are used
    /// after the player's inventory, in mod order and then push order. The engine silently
    /// skips `pos`, duplicates (first wins), unavailable or non-lendable containers, and
    /// all containers after the first 64 accepted sources, without per-call logging.
    /// Chest-like block entities and mod containers with declared slots are lendable.
    /// No answer preserves vanilla placement. The engine takes ingredients and writes
    /// changed containers back, firing `container_changed`; this hook only names sources.
    /// Asked once per recipe-book placement/apply or server transfer check, not per tick.
    /// Native and wasm listeners receive the same arguments and append to the same output.
    #[hook(
        side = "server",
        contract = "collect",
        out = "sources",
        neoforge = "none (Horizon mod crafting menus lend ingredient containers)",
        cancellable = false
    )]
    fn recipe_sources();
}
