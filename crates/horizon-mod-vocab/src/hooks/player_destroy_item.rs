//! The `player_destroy_item` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0250

pub trait Hooks {
    /// The item `item` of the server player `player` (`name`) broke in `slot`
    /// (`mainhand`, `offhand`, `head`, `chest`, `legs`, `feet`, `body` or
    /// `saddle`): `LivingEntity.onEquippedItemBroken`, Bukkit's
    /// `PlayerItemBreakEvent`. Listen only; armour has a slot, not a hand.
    /// The player's state is swapped in. Once for each break, on a server; a
    /// creative player and the host of a LAN world are not heard.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge's PlayerDestroyItemEvent carries the original stack and a hand, which armour has not; this hook only listens and names the slot instead. Bukkit's PlayerItemBreakEvent)"
    )]
    fn player_destroy_item();
}
