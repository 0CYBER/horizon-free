//! The `player_pong` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A guest's client answered a ping (`ServerboundPongPacket`, vanilla's
    /// `ServerGamePacketListenerImpl.handlePong`): `id` is the number the ping carried. The mod
    /// sends the ping itself with `player_send_packet` (`ping`) and measures the round trip, or
    /// counts the client ticks between two answers, as a lag-compensating anticheat does. The
    /// engine's own pings (a recording's end-of-tick ping, whose id is the server tick) are answered
    /// here too, so a mod picks ids it can tell apart from small tick numbers. Heard only for a
    /// pong that answers a ping the server sent on that connection (a mod's or the engine's), once
    /// per ping: an unsolicited or repeated pong, one for a ping sent more than 30 seconds ago, or
    /// one for a ping pushed out by 64 newer unanswered ones, is dropped silently, as vanilla
    /// ignores it. Heard after the packet's turn in the guest's inbox, in order with its other
    /// packets, with the guest's state swapped in. Not cancellable. With no listener nothing is
    /// queued. Server-side.
    #[notes = r#"## Player pong

- **`player_pong(host, event)`**: Paper's `ServerboundPongPacket` as an anticheat reads it through a packet library. Send `ping` to a player with `player_send_packet`, hear the client's `pong` with the same `id`; the time between them is a transaction round trip, finer than the keep-alive average `player_latency` reads."#]
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; a Bukkit plugin reads the pong packet through a packet library)"
    )]
    fn player_pong();
}
