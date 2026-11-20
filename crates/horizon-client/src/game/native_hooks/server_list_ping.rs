use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A client asked the server for its status (the server list's ping):
    /// the answer is about to be sent. `protocol` and `virtual_host` are what
    /// the client's handshake stated. Change `motd`, `favicon` (PNG bytes),
    /// `online`, `max_players`, `sample` or `hide_players` (no players part),
    /// or set `cancel` to send no answer. Server-side, with no player's state
    /// swapped in.
    pub(in crate::game) fn native_server_list_ping(
        &mut self,
        call: mod_api::__private::ServerListPingCall<'_>,
    ) {
        self.fire_server_list_ping(call);
    }
}
