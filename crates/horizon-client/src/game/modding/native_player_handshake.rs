//! `player_handshake`: the game's half of a login connection's handshake, which a connection
//! thread queued for the mods (`horizon_client::lan::handshake_gate`).
use super::Game;
use horizon_client::lan::handshake_gate::Answer;
use horizon_client::text::Text;

impl Game<'_> {
    /// Tell the mods about the handshakes the connection threads are waiting on, and hand each
    /// connection the host the mods left or their refusal.
    pub(in crate::game) fn answer_handshakes(&mut self) {
        let Some(gate) = self.lan.as_ref().map(|host| &host.hub.lan.shared.handshake) else {
            return;
        };
        let asks = gate.take();
        for ask in asks {
            let mut host = ask.host.clone();
            let (mut cancel, mut reason): (bool, Option<Text>) = (false, None);
            let address = ask.address.map(|ip| ip.to_string()).unwrap_or_default();
            self.native_player_handshake(
                &mut host,
                i32::from(ask.port),
                ask.protocol,
                &address,
                &mut cancel,
                &mut reason,
            );
            let refusal = cancel.then(|| match reason {
                Some(text) => horizon_client::lan::data::component_nbt(&text.component()),
                None => horizon_protocol::text::translatable("multiplayer.disconnect.generic", &[]),
            });
            ask.answer(Answer { host, refusal });
        }
    }
}
