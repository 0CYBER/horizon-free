use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
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
    pub(in crate::game) fn native_client_packet(&mut self, packet: &ClientPacket) {
        self.fire_client_packet(packet);
    }
}
