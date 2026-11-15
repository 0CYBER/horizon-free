//! The `player_fish` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0251

pub trait Hooks {
    /// A server player's fishing rod, with `state`: 0 the cast
    /// (`FishingRodItem.use` throwing the bobber), 1 a fish biting
    /// (`FishingHook.catchingFish`), 2 the reel-in of a bite, which gives the
    /// catch, 3 the reel-in with nothing on the line, 4 the reel-in of a
    /// bobber hooked to an entity or item (like Bukkit's `PlayerFishEvent`;
    /// NeoForge's `ItemFishedEvent` is later, in `FishingHook.retrieve`). Set `cancel` on the cast and nothing
    /// is thrown; on a bite the fish gets away and the wait starts over; on
    /// a reel-in (2 to 4) nothing happens, the bobber stays out and the rod
    /// takes no wear. The bobber ending by itself (too far, the rod put
    /// away, the owner dying), nor the host of a LAN world's rod. The
    /// player's state is swapped in, so `host.players().acting_player()`, `PlayerRef::tell`
    /// and the item helpers reach them. Once for each cast, bite and reel-in,
    /// on a server.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge's ItemFishedEvent fires after the loot roll, in FishingHook.retrieve; Bukkit's PlayerFishEvent)"
    )]
    fn player_fish();
}
