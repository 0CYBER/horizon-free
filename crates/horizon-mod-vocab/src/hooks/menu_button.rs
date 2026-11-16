//! The `menu_button` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0031

pub trait Hooks {
    /// A player pressed a mod container's button (`clickMenuButton`), on
    /// the server. The block entity is out of the world during the call;
    /// mods may change it and the clicking player's carried stack.
    #[hook(
        side = "server",
        contract = "listen",
        scope = "block_entity",
        key = "entity",
        neoforge = "none (NeoForge has no such event; vanilla AbstractContainerMenu.clickMenuButton)"
    )]
    fn menu_button();
}
