//! Attachment sync: the values of a native mod's synced attachments, sent by a
//! Horizon server to the Horizon clients that see their holder
//! (docs/modding/src/how-to/native-mods.md, "Server attachments").
//!
//! A payload is the holder (a varint kind and its fields), whether the
//! client's copy of the holder is replaced, then each change: the attachment's
//! namespace and name and its value as an NBT compound, or no tag for a
//! removal.
//!
//! The channel is an engine declaration in the negotiated payload table
//! (docs/design/mod-api/typed-payloads.md): only a peer that offers it at
//! [`VERSION`] is sent anything, and a message is bounded by the connection's
//! frame limits, not by a body cap of its own.
use {
    horizon_core::{nbt, nbt::Tag},
    horizon_mod_runtime::attachments::{AttachmentSync, Holder},
};

use crate::{
    Error, Result,
    buf::{Reader, Writer},
};

/// The custom payload channel, clientbound only: a client sends nothing back.
pub const CHANNEL: &str = "horizon:attachment_sync";
/// The channel's exact declaration version in the negotiated table.
pub const VERSION: &str = "1";
/// Characters in a namespace or a name at most.
pub const MAX_NAME: usize = 256;

/// The bounds of one value, the same when the server writes it and the client reads it,
/// so a value the server sends is never one the client discards: the engine's NBT defaults
/// (64 MiB, 1,000,000 nodes, depth 512, the depth vanilla's `NbtAccounter` keeps). They are
/// looser than the connection's frame limits, which bound the whole message; the network
/// quota of `Reader::nbt` (2 MiB), meant for vanilla's untrusted packets, does not apply.
pub fn value_limits() -> nbt::Limits {
    nbt::Limits::default()
}

/// A holder's changes as they travel.
#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub holder: Holder,
    /// The changes are the holder's whole synced state: drop what the client
    /// holds first.
    pub replace: bool,
    pub changes: Vec<AttachmentSync>,
}

/// A message's payload; an error for a holder it cannot write, a name past
/// [`MAX_NAME`], or a value that is not an encodable compound. Its size is the
/// sending connection's to bound (its frame limits).
pub fn encode(message: &Message) -> Result<Vec<u8>> {
    let mut w = Writer::new();
    match message.holder {
        Holder::Entity(id) => {
            w.varint(0);
            w.varint(id as i32);
        }
        Holder::Player(uuid) => {
            w.varint(1);
            w.uuid(uuid);
        }
        Holder::BlockEntity(pos) => {
            w.varint(2);
            pos.iter().for_each(|&axis| w.i32(axis));
        }
        Holder::Chunk(pos) => {
            w.varint(3);
            pos.iter().for_each(|&axis| w.i32(axis));
        }
        Holder::Level => w.varint(4),
        _ => return Err(Error::new("attachment sync: unknown holder")),
    }
    w.bool(message.replace);
    w.varint(message.changes.len() as i32);
    for change in &message.changes {
        for name in [&change.namespace, &change.name] {
            let name = name
                .to_string()
                .map_err(|_| Error::new("attachment sync: name is not UTF-16"))?;
            if name.encode_utf16().count() > MAX_NAME {
                return Err(Error::new(format!(
                    "attachment sync: name {name} is too long"
                )));
            }
            w.string(&name);
        }
        match &change.value {
            Some(value) => {
                let tag = Tag::Compound(value.clone());
                let bytes = nbt::write_network(&tag, value_limits())
                    .map_err(|e| Error::new(format!("attachment sync: value: {e}")))?;
                w.bytes(&bytes);
            }
            None => w.bytes(&[0]),
        }
    }
    Ok(w.data)
}

/// A payload's message; an error for one that breaks a cap.
pub fn decode(data: &[u8]) -> Result<Message> {
    let mut r = Reader::new(data);
    let holder = match r.varint()? {
        0 => Holder::Entity(r.varint()? as u32),
        1 => Holder::Player(r.uuid()?),
        2 => Holder::BlockEntity([r.i32()?, r.i32()?, r.i32()?]),
        3 => Holder::Chunk([r.i32()?, r.i32()?]),
        4 => Holder::Level,
        kind => return Err(Error::new(format!("attachment sync: holder kind {kind}"))),
    };
    let replace = r.bool()?;
    let max = r.remaining();
    let changes = (0..r.count(max)?)
        .map(|_| {
            let namespace = r.string(MAX_NAME)?;
            let name = r.string(MAX_NAME)?;
            let value = match r.nbt_within(value_limits())? {
                Tag::Compound(value) => Some(value),
                Tag::End => None,
                _ => return Err(Error::new("attachment sync: value is not a compound")),
            };
            Ok(AttachmentSync {
                namespace: namespace.as_str().into(),
                name: name.as_str().into(),
                value,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    if r.remaining() > 0 {
        return Err(Error::new("attachment sync: trailing bytes"));
    }
    Ok(Message {
        holder,
        replace,
        changes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn changes() -> Vec<AttachmentSync> {
        vec![
            AttachmentSync {
                namespace: "mana".into(),
                name: "pool".into(),
                value: Some([("value".into(), Tag::Int(12))].into()),
            },
            AttachmentSync {
                namespace: "mana".into(),
                name: "gone".into(),
                value: None,
            },
        ]
    }

    #[test]
    fn every_holder_round_trips_with_either_replace_and_a_removal() {
        let holders = [
            Holder::Entity(4_000_000_000),
            Holder::Player(0x0123_4567_89ab_cdef_0011_2233_4455_6677),
            Holder::BlockEntity([-30_000_000, -64, 29_999_999]),
            Holder::Chunk([-5, 7]),
            Holder::Level,
        ];
        for holder in holders {
            for replace in [true, false] {
                let message = Message {
                    holder,
                    replace,
                    changes: changes(),
                };
                assert_eq!(decode(&encode(&message).unwrap()).unwrap(), message);
            }
        }
    }

    #[test]
    fn decode_refuses_trailing_bytes_and_an_unknown_holder_kind() {
        let message = Message {
            holder: Holder::Level,
            replace: false,
            changes: changes(),
        };
        let mut payload = encode(&message).unwrap();
        payload.push(0);
        assert!(decode(&payload).is_err(), "trailing bytes");
        assert!(decode(&[5, 0, 0]).is_err(), "unknown holder kind");
    }

    #[test]
    fn encode_refuses_a_change_the_client_would_not_read_as_sent() {
        let mut message = Message {
            holder: Holder::Level,
            replace: false,
            changes: changes(),
        };
        message.changes[0].name = "n".repeat(MAX_NAME).as_str().into();
        assert!(decode(&encode(&message).unwrap()).is_ok());
        message.changes[0].name = "n".repeat(MAX_NAME + 1).as_str().into();
        assert!(encode(&message).is_err(), "name");
        message.changes[0].name = "pool".into();
        let mut deep = Tag::Int(0);
        for _ in 0..600 {
            deep = Tag::Compound([("v".into(), deep)].into());
        }
        let Tag::Compound(deep) = deep else {
            unreachable!()
        };
        message.changes[0].value = Some(deep);
        assert!(encode(&message).is_err(), "unwritable value");
    }

    #[test]
    fn the_format_is_unchanged_and_has_no_body_cap_of_its_own() {
        // Level holder (kind 4), not replaced, one removal of a:b.
        let removal = Message {
            holder: Holder::Level,
            replace: false,
            changes: vec![AttachmentSync {
                namespace: "a".into(),
                name: "b".into(),
                value: None,
            }],
        };
        assert_eq!(encode(&removal).unwrap(), [4, 0, 1, 1, b'a', 1, b'b', 0]);
        // Network NBT: an unnamed compound holding Int "v" = 7, then its end.
        let mut set = removal.clone();
        set.changes[0].value = Some([("v".into(), Tag::Int(7))].into());
        assert_eq!(
            encode(&set).unwrap(),
            [4, 0, 1, 1, b'a', 1, b'b', 10, 3, 0, 1, b'v', 0, 0, 0, 7, 0]
        );
        // Past the old 1,048,576-byte cap and the 2 MiB network NBT quota: what encodes,
        // decodes; the connection's frame limits bound the message.
        let big = Tag::ByteArray(vec![1; 3 << 20]);
        set.changes[0].value = Some([("big".into(), big)].into());
        let payload = encode(&set).unwrap();
        assert!(payload.len() > 3 << 20);
        assert_eq!(decode(&payload).unwrap(), set);
    }

    #[test]
    fn a_value_past_the_node_limit_is_refused_by_both_ends_alike() {
        // More entries than the 1,000,000 nodes of `value_limits`.
        let list = Tag::Compound([("l".into(), Tag::List(vec![Tag::Int(0); 1_000_001]))].into());
        let Tag::Compound(value) = list.clone() else {
            unreachable!()
        };
        let past = Message {
            holder: Holder::Level,
            replace: false,
            changes: vec![AttachmentSync {
                namespace: "a".into(),
                name: "b".into(),
                value: Some(value),
            }],
        };
        assert!(encode(&past).is_err(), "the writer refuses it");
        // A peer that wrote it anyway is refused by the reader, not truncated: Level
        // holder, not replaced, one change of a:b, then the value.
        let mut forged = vec![4, 0, 1, 1, b'a', 1, b'b'];
        let unbounded = nbt::Limits {
            max_nodes: usize::MAX,
            ..nbt::Limits::default()
        };
        forged.extend(nbt::write_network(&list, unbounded).unwrap());
        assert!(decode(&forged).is_err(), "the reader refuses it");
    }
}
