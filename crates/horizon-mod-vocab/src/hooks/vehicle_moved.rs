//! The `vehicle_moved` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0272

pub trait Hooks {
    /// A boat or minecart (`kind`, `minecraft:oak_boat`) moved since the last
    /// tick (`VehicleMoveEvent`): `from` and `to` are its positions, heard
    /// after the tick moved it. Once a tick for each vehicle that carries a
    /// server player, whose state is then swapped in (as for
    /// `run_mod_command`: `host.players().acting_player()`, `PlayerRef::tell` and
    /// the item helpers reach them; `player` is their UUID), and for one a
    /// server player rode and left, until it has stood still for ten
    /// seconds (`player` is none, nobody is swapped in). It only hears.
    /// Cheap when nothing moves: one position compare a tick for each watched
    /// vehicle. Not heard: a vehicle moved before anyone rode it, a mount
    /// (horse, pig...) or a vehicle carrying only a mob. Server-side, in the
    /// level the vehicle is in.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no vehicle move event; EntityTickEvent covers the tick)"
    )]
    fn vehicle_moved();
}
