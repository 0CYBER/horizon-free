//! The plugin channels a client announces (`minecraft:register`, `minecraft:unregister`):
//! NUL-separated identifiers, as Bukkit's `StandardMessenger` and Velocity read them.
//! The server keeps them per connection, in configuration and play, for mods to read.
use std::sync::{Mutex, PoisonError};

/// The channel lists' payload channels.
pub const REGISTER: &str = "minecraft:register";
pub const UNREGISTER: &str = "minecraft:unregister";
/// Channels kept for one connection at most (Bukkit's `CraftPlayer.addChannel` refuses past 128).
pub const MAX_CHANNELS: usize = 128;
/// One channel's length at most, in bytes (Bukkit's `Messenger.MAX_CHANNEL_SIZE`).
pub const MAX_CHANNEL_LEN: usize = 64;

/// One connection's announced channels, in announcement order.
#[derive(Debug, Default)]
pub struct AnnouncedChannels(Mutex<Vec<String>>);

/// Whether `channel` is one of the two list channels the engine consumes.
pub fn is_list_channel(channel: &str) -> bool {
    channel == REGISTER || channel == UNREGISTER
}

impl AnnouncedChannels {
    /// Applies a `minecraft:register` / `minecraft:unregister` body; another channel
    /// is not this list's (false). Malformed entries (not UTF-8, too long, not a
    /// namespaced identifier) are skipped, and a register past [`MAX_CHANNELS`]
    /// keeps the first ones: a client's bytes never panic or grow it unbounded.
    pub fn apply(&self, channel: &str, data: &[u8]) -> bool {
        let register = match channel {
            REGISTER => true,
            UNREGISTER => false,
            _ => return false,
        };
        let mut list = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        for entry in data.split(|&b| b == 0) {
            let Some(id) = valid(entry) else {
                continue;
            };
            let known = list.iter().position(|c| c == id);
            match (register, known) {
                (true, None) if list.len() < MAX_CHANNELS => list.push(id.to_owned()),
                (false, Some(at)) => {
                    list.remove(at);
                }
                _ => {}
            }
        }
        true
    }
    /// The channels, in announcement order.
    pub fn list(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
    /// Moves the list collected during configuration into the play connection's.
    pub fn take_from(&self, other: &AnnouncedChannels) {
        let taken = std::mem::take(&mut *other.0.lock().unwrap_or_else(PoisonError::into_inner));
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = taken;
    }
}

fn valid(entry: &[u8]) -> Option<&str> {
    if entry.is_empty() || entry.len() > MAX_CHANNEL_LEN {
        return None;
    }
    let id = std::str::from_utf8(entry).ok()?;
    horizon_core::identifier::Identifier::parse_namespaced(id).map(|_| id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_two_list_channels_are_consumed() {
        assert!(is_list_channel("minecraft:register"));
        assert!(is_list_channel("minecraft:unregister"));
        for other in ["minecraft:brand", "voicechat:secret", "register"] {
            assert!(!is_list_channel(other), "{other}");
        }
    }

    #[test]
    fn register_and_unregister_keep_an_ordered_set() {
        let channels = AnnouncedChannels::default();
        assert!(channels.apply(
            REGISTER,
            b"voicechat:secret\0bungeecord:main\0voicechat:secret"
        ));
        assert_eq!(channels.list(), ["voicechat:secret", "bungeecord:main"]);
        assert!(channels.apply(UNREGISTER, b"voicechat:secret\0not:there"));
        assert_eq!(channels.list(), ["bungeecord:main"]);
        assert!(!channels.apply("minecraft:brand", b"vanilla"));
        assert_eq!(channels.list(), ["bungeecord:main"]);
    }

    #[test]
    fn a_hostile_list_is_bounded_and_skips_malformed_entries() {
        let channels = AnnouncedChannels::default();
        let mut body = Vec::new();
        for entry in [
            &b"\0\0"[..],
            b"\xff\xfe:bad",
            b"no_namespace",
            b"UPPER:case",
            &[b'a'; MAX_CHANNEL_LEN + 1][..],
            b"ok:first",
        ] {
            body.extend_from_slice(entry);
            body.push(0);
        }
        for n in 0..MAX_CHANNELS * 4 {
            body.extend_from_slice(format!("flood:c{n}\0").as_bytes());
        }
        assert!(channels.apply(REGISTER, &body));
        let list = channels.list();
        assert_eq!(list.len(), MAX_CHANNELS);
        assert_eq!(list[0], "ok:first");
        assert!(channels.apply(REGISTER, &vec![0xff; 1 << 16]));
        assert_eq!(channels.list(), list);
        let configured = AnnouncedChannels::default();
        configured.apply(REGISTER, b"cfg:channel");
        channels.take_from(&configured);
        assert_eq!(channels.list(), ["cfg:channel"]);
        assert!(configured.list().is_empty());
    }
}
