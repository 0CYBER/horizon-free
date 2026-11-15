//! The `mod_container_viewers` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0128

pub trait Hooks {
    /// The number of players with the mod container at `pos` of `dimension`
    /// open went from `previous` to `viewers` (vanilla's
    /// `ContainerOpenersCounter`, as a barrel keeps it): 0 to more than 0 is
    /// `onOpen`, back to 0 is `onClose`, any change `openerCountChanged`, so a
    /// block can show itself open while `viewers` is not 0. Counted every
    /// tick from the players' open menus, so a player who leaves without
    /// closing (a lost connection, death, another dimension, out of reach, the
    /// block gone) is heard too (`recheckOpeners`). Once for each change, with
    /// no player's state swapped in. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (vanilla ContainerOpenersCounter; NeoForge adds no event or extension for it); PlayerContainerEvent.Open and Close fire per player"
    )]
    fn mod_container_viewers();
}
