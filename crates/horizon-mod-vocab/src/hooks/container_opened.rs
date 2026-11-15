//! The `container_opened` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0241

pub trait Hooks {
    /// A player opens one of the game's own menus (`ServerPlayer.openMenu`; the
    /// job of NeoForge's `PlayerContainerEvent.Open`, which comes after the
    /// menu is open and cannot stop it): `kind` is the menu type
    /// (`minecraft:generic_9x3`, `minecraft:furnace`, `minecraft:crafting`,
    /// `minecraft:merchant`, `minecraft:anvil`, `minecraft:lectern`...; a
    /// horse's inventory is `minecraft:horse_inventory`) and `pos` the block
    /// it belongs to, none for a villager, a mount or a vehicle's chest.
    /// Heard after the checks that refuse the menu (a locked container, a
    /// spectator) and before the menu that was open closes. Set `cancel` and
    /// nothing opens: the open menu stays as it was and the click is spent.
    /// The player's state is swapped in for the call, as for
    /// `run_mod_command`, so `host.players().acting_player()`, `PlayerRef::tell` and
    /// the item helpers reach them. Not heard: a menu a mod opens
    /// (`native_open_menu`), the inventory screen, and the editors (sign,
    /// book, command block). Server-side, for every player of the world.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (not NeoForge's PlayerContainerEvent.Open: this hook can cancel the opening)",
        cancellable = true
    )]
    fn container_opened();
}
