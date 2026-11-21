//! The `player_move_packet` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A guest's client sent a movement packet and the server's own validation accepted it
    /// (vanilla's `ServerGamePacketListenerImpl.handleMovePlayer`, the flying packet of a packet
    /// library). Heard once for every such packet, also one that changes nothing (a stand-still or
    /// ground-flag packet), where `player_move` hears only a report whose position or rotation
    /// differs. `to` and `to_rotation` (`[yaw, pitch]`) are the packet's values, repeating the
    /// player's own where the packet carries none. `on_ground` is the packet's own on-ground flag as
    /// the client sent it; `has_position` and `has_rotation` say whether the packet carried a
    /// position and a rotation (a status-only packet carries neither). Not heard: the player's first
    /// tick in a level (the tick after a join or a level change), a teleport's confirmation and a
    /// passage crossing. Heard with the player's state swapped in, before `player_move` hears the
    /// same packet. Not cancellable (`player_move` vetoes). With no listener nothing is queued.
    /// Each guest is heard at most 8 packets per tick; the packets over that wait, in order, for
    /// the next ticks (at most 256 wait, the oldest are dropped past that). Only the hook is
    /// budgeted, the movement itself is processed as it arrives, so a packet that waited can be
    /// heard after `player_move` already heard or vetoed the same move. Waiting packets are dropped
    /// when the player changes level, is frozen for a transfer or leaves.
    /// Server-side.
    #[notes = r#"## Player move packet

- **`player_move_packet(host, event)`**: the stream of movement packets an anticheat reads through a packet library (`PacketPlayerFlying`): every validated packet, also one that changes nothing, with the packet's `on_ground` flag and whether it carried a position and a rotation. Use `player_move` to veto a report that moves the player."#]
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; a Bukkit plugin reads the flying packet through a packet library)"
    )]
    fn player_move_packet(
        player: u128,
        to: [f64; 3],
        to_rotation: [f32; 2],
        on_ground: bool,
        has_position: bool,
        has_rotation: bool,
    );
}
