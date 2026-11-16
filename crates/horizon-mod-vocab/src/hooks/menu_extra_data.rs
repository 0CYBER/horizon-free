//! The `menu_extra_data` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Block `block` at `pos` opens its mod menu type `menu` for server
    /// player `player`: set `data` to hand the menu's screen bytes of your
    /// own (a machine's tier, mode or recipes), up to 32 KiB, which the
    /// client reads with `screen_extra_data` while the screen is open
    /// (NeoForge's `openMenu` with an extra-data writer). The first mod to
    /// set it wins, so answer only for your blocks. The player's state is not
    /// swapped in.
    #[hook(
        side = "server",
        contract = "replace",
        out = "data",
        neoforge = "IPlayerExtension.openMenu",
        cancellable = false
    )]
    fn menu_extra_data();
}
