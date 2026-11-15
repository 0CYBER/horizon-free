//! The `player_totem` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0265

pub trait Hooks {
    /// A totem of undying saved the server player `player` (`name`): their
    /// hurt would have killed them and the totem held in either hand fired
    /// (`LivingEntity.checkTotemDeathProtection`; Bukkit's
    /// `EntityResurrectEvent`), heard just before it is used up and its
    /// effects apply. Listen only: whether the totem saves is decided inside
    /// the health state machine, before a mod is asked, so a mod cannot veto
    /// it from here. The player's state is swapped in, so
    /// `host.players().acting_player()`, `PlayerRef::tell` and the item helpers reach them.
    /// Once for each totem, on a server; the host of a LAN world's totem is
    /// not heard.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge's LivingUseTotemEvent can cancel the save; this hook only listens. Bukkit's EntityResurrectEvent)"
    )]
    fn player_totem();
}
