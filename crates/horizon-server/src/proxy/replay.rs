//! What a game that is logged into again must be told, because the client said it once and will not say it
//! again: its settings (ClientInformation), its brand, the plugin channels it registered and, in play, its
//! chat session (the key its signed chat is checked with). The proxy hears each as it passes and keeps the last.

use super::wire::{Reader, Writer, ids, write_varint};
use std::collections::BTreeSet;

/// Which state a client's packets are in: ids mean different packets in each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Config,
    Play,
}

const BRAND: &str = "minecraft:brand";
const REGISTER: &str = "minecraft:register";
const UNREGISTER: &str = "minecraft:unregister";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Replay {
    info: Option<Vec<u8>>,
    brand: Option<Vec<u8>>,
    channels: BTreeSet<String>,
    chat_session: Option<Vec<u8>>,
}

/// A packet body: its id, then the payload.
pub fn body(id: i32, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 5);
    write_varint(&mut out, id);
    out.extend_from_slice(payload);
    out
}

impl Replay {
    /// Notes a packet the client sent in `phase`; the packet itself is relayed as it is.
    pub fn note(&mut self, phase: Phase, id: i32, payload: &[u8]) {
        use ids::{configuration::serverbound as cfg, play::serverbound as play};
        let (info, custom) = match phase {
            Phase::Config => (cfg::CLIENT_INFORMATION, cfg::CUSTOM_PAYLOAD),
            Phase::Play => (play::CLIENT_INFORMATION, play::CUSTOM_PAYLOAD),
        };
        if id == info {
            self.info = Some(payload.to_vec());
        } else if id == custom {
            self.note_payload(payload);
        } else if phase == Phase::Play && id == play::CHAT_SESSION_UPDATE {
            self.chat_session = Some(payload.to_vec());
        }
    }

    fn note_payload(&mut self, payload: &[u8]) {
        let mut r = Reader::new(payload);
        let Ok(channel) = r.string(32767) else { return };
        let data = r.rest();
        match channel.as_str() {
            BRAND => self.brand = Some(data.to_vec()),
            REGISTER => {
                for c in split_channels(data) {
                    if self.channels.len() < MAX_CHANNELS || self.channels.contains(&c) {
                        self.channels.insert(c);
                    }
                }
            }
            UNREGISTER => {
                for c in split_channels(data) {
                    self.channels.remove(&c);
                }
            }
            _ => {}
        }
    }

    // Used by the hold through a restart (the next change of #16251); tested here.
    #[allow(dead_code)]
    /// The configuration packets (bodies) that tell a game what the client told the last one.
    pub fn config_packets(&self) -> Vec<Vec<u8>> {
        use ids::configuration::serverbound as cfg;
        let mut out = Vec::new();
        if let Some(info) = &self.info {
            out.push(body(cfg::CLIENT_INFORMATION, info));
        }
        if let Some(brand) = &self.brand {
            out.push(body(cfg::CUSTOM_PAYLOAD, &payload(BRAND, brand)));
        }
        if !self.channels.is_empty() {
            let list: Vec<&str> = self.channels.iter().map(String::as_str).collect();
            out.push(body(
                cfg::CUSTOM_PAYLOAD,
                &payload(REGISTER, list.join("\0").as_bytes()),
            ));
        }
        out
    }

    #[allow(dead_code)]
    /// The play packets (bodies) for a game that has just sent its Login: the chat session.
    pub fn play_packets(&self) -> Vec<Vec<u8>> {
        self.chat_session
            .iter()
            .map(|s| body(ids::play::serverbound::CHAT_SESSION_UPDATE, s))
            .collect()
    }
}

/// The most plugin channels kept for a client: a client may register as many as it likes.
const MAX_CHANNELS: usize = 128;

fn split_channels(data: &[u8]) -> impl Iterator<Item = String> + '_ {
    data.split(|b| *b == 0)
        .filter(|c| !c.is_empty())
        .map(|c| String::from_utf8_lossy(c).into_owned())
}

#[allow(dead_code)]
fn payload(channel: &str, data: &[u8]) -> Vec<u8> {
    let mut w = Writer::new();
    w.string(channel);
    w.bytes(data);
    w.data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_the_client_said_is_said_again_to_the_next_game() {
        let mut replay = Replay::default();
        // A configuration client: settings, brand, two channels, then one more and one taken back.
        replay.note(Phase::Config, 0, &[1, 2, 3]);
        replay.note(
            Phase::Config,
            2,
            &payload(BRAND, &[7, b'h', b'o', b'r', b'i', b'z', b'o', b'n']),
        );
        replay.note(Phase::Config, 2, &payload(REGISTER, b"a:x\0b:y"));
        replay.note(Phase::Play, 22, &payload(REGISTER, b"c:z"));
        replay.note(Phase::Play, 22, &payload(UNREGISTER, b"a:x"));
        // Play's own settings and chat session; a packet nobody keeps changes nothing.
        replay.note(Phase::Play, 14, &[9, 9]);
        replay.note(Phase::Play, 10, &[5; 40]);
        replay.note(Phase::Play, 28, &[0; 8]);

        let config = replay.config_packets();
        assert_eq!(config.len(), 3);
        assert_eq!(config[0], [&[0u8][..], &[9, 9]].concat());
        let channels = &config[2];
        assert_eq!(channels[0], 2);
        assert!(channels.ends_with(b"b:y\0c:z"), "{channels:?}");
        assert_eq!(
            replay.play_packets(),
            vec![[&[10u8][..], &[5; 40]].concat()]
        );
    }

    #[test]
    fn a_client_that_said_nothing_has_nothing_to_replay() {
        assert!(Replay::default().config_packets().is_empty());
        assert!(Replay::default().play_packets().is_empty());
    }

    #[test]
    fn the_same_id_means_another_packet_in_the_other_state() {
        let mut replay = Replay::default();
        // 14 is ClientInformation in play but nothing kept in configuration; 0 is the reverse.
        replay.note(Phase::Config, 14, &[1]);
        replay.note(Phase::Play, 0, &[1]);
        assert_eq!(replay, Replay::default());
    }

    #[test]
    fn a_client_cannot_make_the_kept_channels_grow_without_end() {
        let mut replay = Replay::default();
        let names: Vec<String> = (0..MAX_CHANNELS + 50).map(|i| format!("a:c{i}")).collect();
        replay.note(
            Phase::Config,
            2,
            &payload(REGISTER, names.join("\0").as_bytes()),
        );
        assert_eq!(replay.channels.len(), MAX_CHANNELS);
    }
}
