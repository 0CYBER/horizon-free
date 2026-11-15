//! The `item_fished` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player `player` (UUID) and `player_entity_id` reels in a catch:
    /// `ItemFishedEvent` in `FishingHook.retrieve`, after fishing loot and its modifiers roll.
    /// `fished` holds the bobber's id and position; listeners may change `drops` and set
    /// absolute non-negative `rod_damage` (`damageRodBy`): vanilla uses 1, or 2 with the
    /// bobber on the ground. After `player_fish`'s reel-in, before rewards. Server-side.
    /// The final list reaches `fishing_rod_hooked`, then each stack spawns its item,
    /// one experience orb, and its fish statistic if it belongs to `#minecraft:fishes`.
    /// Empty stacks are removed. A cancel removes the bobber and skips the trigger,
    /// items, experience and statistic; the rod still takes `rod_damage` last.
    /// An emptied list without cancel gives no items, experience or statistic; the trigger
    /// still fires with an empty list and the rod is still damaged.
    /// An invalid `drops` write (a stack the game refuses, such as a count over the item's
    /// maximum) discards the whole event's write-back, every listener's `cancel` and
    /// `rod_damage` included, and logs a warning naming the hook and the conversion error.
    /// Diverges: Horizon lets a listener change the drop list itself; the reference lets a
    /// listener change the stacks but not the list.
    #[hook(
        side = "server",
        version = 2,
        contract = "collect",
        out = "drops",
        neoforge = "ItemFishedEvent",
        cancellable = true
    )]
    fn item_fished();
}
