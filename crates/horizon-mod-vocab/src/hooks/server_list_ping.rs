//! The `server_list_ping` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Paper's `PaperServerListPingEvent`: a client asked a server for its
    /// status (the server list's ping, the status request in the status
    /// protocol state), and the answer is about to be sent. `protocol` is the
    /// protocol version the client's handshake stated and `virtual_host` the
    /// host name it connected to (the part before its first NUL, empty when
    /// it stated none). The writable fields start as what vanilla's
    /// `MinecraftServer.buildServerStatus` sends: `motd` the description,
    /// `favicon` the server icon as PNG bytes (`None`: no icon; bytes that
    /// are not a PNG send no icon either), `online` and `max_players` the
    /// counts, `sample` the names and UUIDs the player list shows (vanilla's
    /// shuffled pick of at most 12; empty when the server hides its online
    /// players), and `hide_players` false. Set `hide_players` and the
    /// answer has no players part at all. Set `cancel` and the connection is
    /// closed without an answer. At most 64 sample entries are kept. An
    /// answer that is longer than a client reads (32767 characters) is
    /// replaced by the plain one. Server-side, once for each status request,
    /// on the game thread: the connection waits half a second for the mods,
    /// and sends the plain answer if they have not finished. There is no
    /// player, so no player's state is swapped in.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        paper = "PaperServerListPingEvent",
        neoforge = "none (NeoForge has no such event; Bukkit's ServerListPingEvent)",
        cancellable = true
    )]
    fn server_list_ping(
        protocol: i32,
        virtual_host: &str,
        motd: &mut Text,
        favicon: &mut Option<Vec<u8>>,
        online: &mut i32,
        max_players: &mut i32,
        sample: &mut Vec<(String, u128)>,
        hide_players: &mut bool,
        cancel: &mut bool,
    );
}
