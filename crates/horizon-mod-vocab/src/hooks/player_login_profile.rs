//! The `player_login_profile` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A player's game profile at login, before the server sends the login
    /// success: the UUID and name the login authenticated and the properties
    /// it would send. Change `properties` to send a different profile: the
    /// login success and the player list use the changed list. The UUID and
    /// name are the login's and are not changed here.
    ///
    /// In online mode `properties` holds the account's signed `textures`
    /// property, the skin; a proxy's forwarded properties are here too. An
    /// offline-mode player has none. A skin mod replaces the `textures`
    /// property with its own signed one; keep the other properties.
    ///
    /// Once for each player a server accepts, after the name, whitelist, ban
    /// and server-full checks have passed and before the login success is
    /// sent, so a changed profile reaches the player's client and the player
    /// list. It runs on the connection thread and has no host: answer from
    /// the arguments and the mod's own state alone, and do not block the
    /// login. The player is not placed and `player_login` follows later; this
    /// hook cannot refuse the login.
    ///
    /// The properties are the profile's wire limits: at most 16, non-empty
    /// names of 64 UTF-16 units, values of 32767 and signatures of 1024. A
    /// list past a limit is refused and the login's own properties are sent
    /// instead.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (Bukkit's AsyncPlayerPreLoginEvent carries the profile; NeoForge's PlayerNegotiationEvent is not cancellable)",
        cancellable = false,
        target = "horizon_client_lib::lan::conn::native_player_login_profile"
    )]
    fn player_login_profile();
}
