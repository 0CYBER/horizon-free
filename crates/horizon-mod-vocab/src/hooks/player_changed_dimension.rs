//! The `player_changed_dimension` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0247

pub trait Hooks {
    /// `player` (`name`) changed dimension (NeoForge's
    /// `PlayerChangedDimensionEvent`): `from` and `to` are dimension ids
    /// (`minecraft:overworld`, `minecraft:the_nether`, `minecraft:the_end`).
    /// Heard once per move, when it has ended: the portal's exit is found
    /// and the player stands there (after the destination has loaded), a
    /// command's teleport has landed, or a respawn in another level than the
    /// death's is placed for good (a bed or anchor, or the world spawn's
    /// free column, after the fallback when a respawn block was gone). The
    /// level of `to` is the one in place, and the player's state is swapped
    /// in, as for `run_mod_command`: `host.players().acting_player()`,
    /// `PlayerRef::tell`, the item helpers and the rest reach them. Heard for:
    /// Nether and End portals (also riding a vehicle, and leaving the End as
    /// its winner), `/execute in ... run tp` and any other command that
    /// teleports a player into another level, and the death respawn, on a
    /// dedicated server and on the integrated server of single player and
    /// LAN. Not heard by a remote client window; listeners run on the server that simulates the world.
    /// Not heard: a move within a level (`/tp`, an End gateway), a
    /// player who leaves before the move ends, and a move that ends where it
    /// started (a respawn in the dimension of the death).
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "PlayerEvent.PlayerChangedDimensionEvent"
    )]
    fn player_changed_dimension();
}
