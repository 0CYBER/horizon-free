//! The `player_handshake` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The handshake of a connection that is about to log in or be
    /// transferred in (Paper's `PlayerHandshakeEvent`), before the login
    /// starts and before the protocol version, the proxy's forwarding, the
    /// name, the whitelist or the bans are looked at. `host` is the server
    /// address string the client sent, with whatever a proxy put after the
    /// first NUL (a Bedrock bridge's encrypted player data, Forge's marker);
    /// change it and the rest of the login and `PlayerRef::connect_address`
    /// see the new text, so a mod may cut it back to the plain name. A `host`
    /// longer than 32767 characters is cut at that length. `port` and
    /// `protocol` are the client's own and cannot be changed; `address` is
    /// the IP the connection comes from, empty for an integrated server's own
    /// player (whose `host` is empty too). Set `cancel` to refuse the
    /// connection: the client is disconnected before it logs in, with
    /// `reason` (a plain "Disconnected" if it is left unset). Once for each
    /// login or transfer connection, on a server, never for a status ping.
    /// The hook runs on the server thread at the next tick: the connection
    /// waits for it for 5 seconds and is refused when no answer comes. No
    /// player exists yet, so the host's player helpers reach nobody.
    #[notes = r#"## Player handshake

- **`player_handshake(host, port, protocol, address, cancel, reason)`**: a connection that is about to log in, heard before the login starts. For a mod that reads data a proxy hides in the host string (Geyser with Floodgate encodes a Bedrock player's identity there), rewrites the host, or refuses a connection with a message. Paper's `PlayerHandshakeEvent` is the counterpart. A connection thread asks and the server thread answers at the next tick, so a server with no mod that hears this hook pays one atomic load per connection. A mod that keeps what it read for the later `player_login` keys it by something the proxy forwards (the profile id), as no player exists yet."#]
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (Bukkit's PlayerHandshakeEvent; NeoForge has no handshake event)",
        paper = "PlayerHandshakeEvent"
    )]
    fn player_handshake();
}
