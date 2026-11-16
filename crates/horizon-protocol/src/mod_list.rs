//! Configuration-phase mod compatibility. Only both-side ids and versions cross
//! [`CHANNEL`]; client-only and server-only mods stay private. Content identity
//! is checked separately. A vanilla client never hears the channel.
use crate::{
    Error, Result,
    buf::{Reader, Writer},
};
use std::sync::OnceLock;

/// The custom payload channel, both ways.
pub const CHANNEL: &str = "horizon:mod_list";
/// Mods in a list at most; a list with more is refused whole.
pub const MAX_ENTRIES: usize = 128;
/// Characters in a mod's id or version at most.
pub const MAX_TEXT: usize = 64;

/// Where a mod runs (a manifest's `side`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// Both halves must have it: it changes what the world holds.
    Both,
    /// Only the server needs it.
    Server,
    /// Only the player's game: a HUD, keys, a minimap. Any server accepts it.
    Client,
}

/// A mod that is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub version: String,
    pub side: Side,
}

/// The mods this game has on, set once when they load (`game/modding/mods/mod.rs`), like
/// the registries they fill, and read when a connection is set up, whatever
/// side of it this game is.
static LOCAL: OnceLock<Vec<Entry>> = OnceLock::new();

/// Sets the mods this game has on; false when they were set already (a pack
/// reload builds another root, the registries keep the first).
pub fn set_local(mut entries: Vec<Entry>) -> bool {
    entries.sort_by(|a, b| a.id.cmp(&b.id));
    LOCAL.set(entries).is_ok()
}

/// Whether `set_local` has run: a pack reload keeps the first list.
pub fn is_set() -> bool {
    LOCAL.get().is_some()
}

/// The mods this game has on, by id.
pub fn local() -> Vec<Entry> {
    LOCAL.get().cloned().unwrap_or_default()
}

/// Both-side ids and versions as a payload. Never truncate an identity or list:
/// the receiver refuses an oversized list rather than comparing a partial one.
pub fn encode(entries: &[Entry]) -> Vec<u8> {
    let shared: Vec<_> = entries.iter().filter(|e| e.side == Side::Both).collect();
    let mut w = Writer::new();
    w.varint(shared.len() as i32);
    for e in shared {
        w.string(&e.id);
        w.string(&e.version);
    }
    w.data
}

/// A payload's list; an error for one that breaks a cap.
pub fn decode(data: &[u8]) -> Result<Vec<Entry>> {
    let mut r = Reader::new(data);
    let count = r.varint()?;
    if !(0..=MAX_ENTRIES as i32).contains(&count) {
        return Err(Error::new(format!("mod list: {count} mods")));
    }
    let mut ids = std::collections::BTreeSet::new();
    let entries = (0..count)
        .map(|_| {
            let id = r.string(MAX_TEXT)?;
            let version = r.string(MAX_TEXT)?;
            if id.is_empty() || version.is_empty() || !ids.insert(id.clone()) {
                return Err(Error::new("mod list: empty identity or duplicate id"));
            }
            Ok(Entry {
                id,
                version,
                side: Side::Both,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    if r.remaining() > 0 {
        return Err(Error::new("mod list: trailing bytes"));
    }
    Ok(entries)
}

/// How a client's mods differ from a server's.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Difference {
    /// The server has it on and the client does not: `id version`.
    pub missing: Vec<String>,
    /// The client has it on, the server does not, and it changes the world.
    pub extra: Vec<String>,
    /// Both have it, at different versions.
    pub changed: Vec<VersionDifference>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionDifference {
    pub id: String,
    pub server: String,
    pub client: String,
}

impl Difference {
    pub fn is_empty(&self) -> bool {
        self.missing.is_empty() && self.extra.is_empty() && self.changed.is_empty()
    }
}

/// What a client must change to join a server. Only `Both` mods count, on
/// either side: a `Server` mod is the server's business, a `Client` one the
/// player's.
pub fn compare(server: &[Entry], client: &[Entry]) -> Difference {
    let mut difference = Difference::default();
    for s in server.iter().filter(|s| s.side == Side::Both) {
        match client.iter().find(|c| c.side == Side::Both && c.id == s.id) {
            None => difference.missing.push(format!("{} {}", s.id, s.version)),
            Some(c) if s.version != c.version => difference.changed.push(VersionDifference {
                id: s.id.clone(),
                server: s.version.clone(),
                client: c.version.clone(),
            }),
            Some(_) => {}
        }
    }
    for c in client.iter().filter(|c| c.side == Side::Both) {
        if !server.iter().any(|s| s.side == Side::Both && s.id == c.id) {
            difference.extra.push(format!("{} {}", c.id, c.version));
        }
    }
    difference
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, version: &str, side: Side) -> Entry {
        Entry {
            id: id.into(),
            version: version.into(),
            side,
        }
    }

    #[test]
    fn only_shared_mods_are_sent() {
        let list = [
            entry("ruby", "1.0", Side::Both),
            entry("minimap", "2.0", Side::Client),
            entry("admin", "3.0", Side::Server),
        ];
        let sent = decode(&encode(&list)).unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].id, "ruby");
        assert!(compare(&list, &sent).is_empty());
    }

    #[test]
    fn a_list_round_trips() {
        let list = vec![entry("ruby", "1.2.0", Side::Both)];
        assert_eq!(decode(&encode(&list)).unwrap(), list);
        assert_eq!(decode(&encode(&[])).unwrap(), []);
    }

    #[test]
    fn lists_over_the_caps_are_refused() {
        let mut w = Writer::new();
        w.varint(MAX_ENTRIES as i32 + 1);
        assert!(decode(&w.data).is_err());
        let mut w = Writer::new();
        w.varint(1);
        w.string(&"x".repeat(MAX_TEXT + 1));
        assert!(decode(&w.data).is_err());
        let mut w = Writer::new();
        w.varint(1);
        w.string("a");
        w.string("");
        assert!(decode(&w.data).is_err());
        let duplicate = entry("a", "1", Side::Both);
        assert!(decode(&encode(&[duplicate.clone(), duplicate])).is_err());
        let mut trailing = encode(&[]);
        trailing.push(0);
        assert!(decode(&trailing).is_err());
        assert!(decode(&[0xFF]).is_err());
    }

    #[test]
    fn the_same_mods_match_whatever_the_order() {
        let a = entry("a", "1", Side::Both);
        let b = entry("b", "2", Side::Both);
        assert!(compare(&[a.clone(), b.clone()], &[b, a]).is_empty());
    }

    #[test]
    fn each_kind_of_difference_is_named() {
        let server = [
            entry("ruby", "1.0", Side::Both),
            entry("pipes", "2.0", Side::Both),
            entry("glass", "1.0", Side::Both),
        ];
        let client = [
            entry("ruby", "1.0", Side::Both),
            entry("pipes", "2.0", Side::Both),
            entry("glass", "0.9", Side::Both),
            entry("xray_blocks", "1.0", Side::Both),
        ];
        let d = compare(&server, &client);
        assert_eq!(d.missing, Vec::<String>::new());
        assert_eq!(d.extra, ["xray_blocks 1.0"]);
        assert_eq!(d.changed.len(), 1);
        assert_eq!(d.changed[0].id, "glass");
        assert_eq!(d.changed[0].server, "1.0");
        assert_eq!(d.changed[0].client, "0.9");
        let d = compare(&server, &client[..1]);
        assert_eq!(d.missing, ["pipes 2.0", "glass 1.0"]);
    }

    #[test]
    fn client_only_and_server_only_mods_never_block_a_join() {
        let server = [
            entry("anti_grief", "1", Side::Server),
            entry("ruby", "1", Side::Both),
        ];
        let client = [
            entry("minimap", "3", Side::Client),
            entry("ruby", "1", Side::Both),
        ];
        assert!(compare(&server, &client).is_empty());
        assert!(compare(&[], &[entry("minimap", "3", Side::Client)]).is_empty());
    }

    #[test]
    fn a_client_only_identity_cannot_satisfy_a_both_side_requirement() {
        assert_eq!(
            compare(
                &[entry("ruby", "1", Side::Both)],
                &[entry("ruby", "1", Side::Client)]
            )
            .missing,
            ["ruby 1"]
        );
        assert_eq!(
            compare(&[], &[entry("ruby", "1", Side::Both)]).extra,
            ["ruby 1"]
        );
    }

    #[test]
    fn every_missing_mod_is_reported() {
        let server: Vec<_> = (0..9)
            .map(|i| entry(&format!("m{i}"), "1", Side::Both))
            .collect();
        assert_eq!(
            compare(&server, &[]).missing,
            [
                "m0 1", "m1 1", "m2 1", "m3 1", "m4 1", "m5 1", "m6 1", "m7 1", "m8 1"
            ]
        );
    }
}
