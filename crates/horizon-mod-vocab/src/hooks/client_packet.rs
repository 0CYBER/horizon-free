//! The `client_packet` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A play packet from the server reached the window's game, decoded,
    /// just before the game handles it (`Connection.genericsFtw` handing it
    /// to `ClientPacketListener`): every packet of the play phase, custom
    /// payloads included, in the order the server sent them. Packets the
    /// network thread handles itself are not heard: keep-alives, bundle
    /// delimiters, resource pack pushes and pops, cookies, server links,
    /// transfers, disconnects, the configuration switch, `minecraft:brand`
    /// and far terrain tiles; neither is the configuration phase.
    /// Read-only: the game handles the packet as it would without the mod.
    /// Client-side, on any server, single player's own included.
    /// Every packet exposes its protocol type id. Custom payloads also expose
    /// their channel and raw bytes; no other decoded fields cross the API.
    #[notes = r#"## Decoded client packets

`Hooks::client_packet` receives `&mut dyn ClientHost` and a read-only
`ClientPacketEvent` before the window handles a decoded server play packet.
The read-only `event.packet` is a `mod_api::ClientPacket`: `id` identifies the
packet type (for example, `minecraft:custom_payload`); `custom_payload` contains
the channel and raw bytes for a custom payload, or `None` otherwise. An id
outside the protocol's packet table is reported as `horizon:unknown`.
Other decoded fields are not exposed. This one plain-data contract works
for native and wasm mods, with a generated codec.

The hook cannot cancel or replace a packet. It also observes chunk packets
handled by the batching path, once each. Configuration packets and the network
thread's own handling (including keep-alives and resource pack messages) do not fire it.

`Hooks::client_packet_send` is the other direction: a `ClientPacketSendEvent` with the same
`ClientPacket` view for each play packet the window's game is about to send (`Connection.send`),
in order, including the selected slot sent before an attack or a use. Returning `Flow::Cancel` drops
the packet, and the game goes on as if it had been sent. Play-phase pongs pass through this hook.
Packets the network thread sends itself (keep-alives, configuration pongs, cookies, configuration), the phase-dependent answers (resource pack status,
a dialog's custom click action) and far terrain requests do not fire it. With no listener the
send path checks one flag and builds nothing."#]
    #[hook(
        side = "client",
        contract = "listen",
        cancellable = false,
        neoforge = "none (NeoForge has no raw play-packet receive event)"
    )]
    fn client_packet();
}
