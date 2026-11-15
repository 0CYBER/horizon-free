//! The `food_level_changed` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0243

pub trait Hooks {
    /// The food level of the server player `player` (`name`) is about to change
    /// from `from` to `to` (Bukkit's `FoodLevelChangeEvent`):
    /// eating, hunger and saturation effects, the exhaustion that costs a
    /// point, starving. Change `to` (kept between 0 and 20; saturation never
    /// exceeds it) or set `cancel` to leave the level at `from`, saturation
    /// included. Heard once for the change a player's turn made as a whole, so
    /// two changes in one tick arrive as one. Not heard: the full food bar a
    /// respawn gives, the food a player joins with and the host of a LAN
    /// world. The player's state is swapped in, so `host.players().acting_player()`,
    /// `PlayerRef::tell` and the item helpers reach them. On a server.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (vanilla: FoodData; Bukkit's FoodLevelChangeEvent)"
    )]
    fn food_level_changed();
}
