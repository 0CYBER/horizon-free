use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A connection about to log in or be transferred in: `host` is the
    /// server address the client sent (change it to rewrite what the login
    /// goes on with), `port`, `protocol` and `address` (the peer's IP, empty
    /// for the integrated server's own player) are read-only. Set `cancel`
    /// to refuse it with `reason`.
    pub(in crate::game) fn native_player_handshake(
        &mut self,
        host: &mut String,
        port: i32,
        protocol: i32,
        address: &str,
        cancel: &mut bool,
        reason: &mut Option<Text>,
    ) {
        self.fire_player_handshake(host, port, protocol, address, cancel, reason);
    }
}
