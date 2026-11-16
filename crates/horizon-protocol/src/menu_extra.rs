//! Menu extra data: what a Horizon server tells a Horizon client about a
//! native mod's menu type its block opened, just before the
//! `ClientboundOpenScreenPacket` of the same window (docs/native-mods.md,
//! "Menu extra data"). NeoForge's counterpart is `openMenu` with an extra-data
//! writer, read by the menu type's `IContainerFactory`.
//!
//! A payload is the window id, the block's position if there is one (a
//! boolean, then the three axes), then the bytes the mod wrote (empty when it
//! wrote none). Clientbound only.
//!
//! **Revision** (configuration, both directions): before play, the server
//! sends a client whose `minecraft:brand` is Horizon's [`query`]
//! (`[QUERY, REVISION]`). A client that knows this revision answers with
//! [`answer`] (`[ANSWER, REVISION]`); the server sends it the layout above.
//! A client that answers nothing (an older Horizon build) is sent
//! [`encode_legacy`]'s layout instead (the window, then the three axes —
//! never `None` — then the bytes), and no menu is opened without a block
//! for it. A vanilla client is never asked.
use crate::{
    Error, Result,
    buf::{Reader, Writer},
};

/// The custom payload channel.
pub const CHANNEL: &str = "horizon:menu_extra";
/// The payload layout's revision: a boolean, then optionally the axes.
pub const REVISION: u8 = 1;
/// Bytes a mod may write at most.
pub const MAX_DATA: usize = 32_768;

const QUERY: u8 = 0;
const ANSWER: u8 = 1;

/// The server's query, configuration clientbound: the custom payload's data.
pub fn query() -> Vec<u8> {
    vec![QUERY, REVISION]
}

/// Reads the server's query. A query of another revision is still answered
/// (with this one's), so its bytes after the revision are not read.
pub fn read_query(data: &[u8]) -> Result<()> {
    let mut r = Reader::new(data);
    if r.u8()? != QUERY {
        return Err(Error::new("menu extra revision: not a query"));
    }
    if r.u8()? == REVISION && r.remaining() > 0 {
        return Err(Error::new("menu extra revision: trailing bytes"));
    }
    Ok(())
}

/// The client's answer, configuration serverbound: it reads this revision's layout.
pub fn answer() -> Vec<u8> {
    vec![ANSWER, REVISION]
}

/// Reads a client's answer: the revision whose layout it reads, 0 for one of
/// another revision (an older build: the legacy layout). Its bytes after the
/// revision are not read.
pub fn read_answer(data: &[u8]) -> Result<u8> {
    let mut r = Reader::new(data);
    if r.u8()? != ANSWER {
        return Err(Error::new("menu extra revision: not an answer"));
    }
    if r.u8()? != REVISION {
        return Ok(0);
    }
    if r.remaining() > 0 {
        return Err(Error::new("menu extra revision: trailing bytes"));
    }
    Ok(REVISION)
}

/// A menu's extra data as it travels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    /// The window id the `OpenScreen` that follows carries.
    pub window: i32,
    /// The block that opened the menu; None for a menu opened without one.
    pub pos: Option<[i32; 3]>,
    pub data: Vec<u8>,
}

/// A message's payload; an error for data past [`MAX_DATA`].
pub fn encode(message: &Message) -> Result<Vec<u8>> {
    if message.data.len() > MAX_DATA {
        return Err(Error::new("menu extra: data too big"));
    }
    let mut w = Writer::new();
    w.varint(message.window);
    w.bool(message.pos.is_some());
    message.pos.iter().flatten().for_each(|&axis| w.i32(axis));
    w.byte_array(&message.data);
    Ok(w.data)
}

/// The legacy layout, for a Horizon client that answered no revision query:
/// the window, then the block's position (always there), then the bytes.
/// An error for data past [`MAX_DATA`] or a menu with no block, which such
/// a client is never sent.
pub fn encode_legacy(message: &Message) -> Result<Vec<u8>> {
    let Some([x, y, z]) = message.pos else {
        return Err(Error::new("menu extra: legacy payload needs a position"));
    };
    if message.data.len() > MAX_DATA {
        return Err(Error::new("menu extra: data too big"));
    }
    let mut w = Writer::new();
    w.varint(message.window);
    w.i32(x);
    w.i32(y);
    w.i32(z);
    w.byte_array(&message.data);
    Ok(w.data)
}

/// A payload's message; an error for one that breaks a cap. The layout a
/// client that answered the revision query reads.
pub fn decode(data: &[u8]) -> Result<Message> {
    let mut r = Reader::new(data);
    let window = r.varint()?;
    let pos = if r.bool()? {
        Some([r.i32()?, r.i32()?, r.i32()?])
    } else {
        None
    };
    let data = r.byte_array(MAX_DATA)?;
    if r.remaining() > 0 {
        return Err(Error::new("menu extra: trailing bytes"));
    }
    Ok(Message { window, pos, data })
}

/// A payload's message in the legacy layout, as an older Horizon client sends
/// or expects it: the window, the block's position, then the bytes.
pub fn decode_legacy(data: &[u8]) -> Result<Message> {
    let mut r = Reader::new(data);
    let window = r.varint()?;
    let pos = Some([r.i32()?, r.i32()?, r.i32()?]);
    let data = r.byte_array(MAX_DATA)?;
    if r.remaining() > 0 {
        return Err(Error::new("menu extra: trailing bytes"));
    }
    Ok(Message { window, pos, data })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_round_trips() {
        for (pos, data) in [
            (Some([-30_000_000, -64, 29_999_999]), vec![]),
            (Some([0, 0, 0]), vec![0, 255, 7]),
            (None, vec![9; MAX_DATA]),
            (None, vec![1, 2, 3]),
        ] {
            let m = Message {
                window: 100,
                pos,
                data,
            };
            assert_eq!(decode(&encode(&m).unwrap()).unwrap(), m);
        }
    }

    #[test]
    fn caps_are_enforced_both_ways() {
        let big = Message {
            window: 1,
            pos: Some([0; 3]),
            data: vec![0; MAX_DATA + 1],
        };
        assert!(encode(&big).is_err());
        // Built by hand, as a server that skipped the check would.
        let mut w = Writer::new();
        w.varint(1);
        w.bool(false);
        w.byte_array(&vec![0; MAX_DATA + 1]);
        assert!(decode(&w.data).is_err());
        let mut ok = encode(&Message {
            window: 1,
            pos: Some([0; 3]),
            data: vec![1],
        })
        .unwrap();
        ok.push(0);
        assert!(decode(&ok).is_err(), "trailing bytes");
        assert!(decode(&[]).is_err());
        assert!(decode(&[1, 1, 0]).is_err(), "a cut position");
    }

    #[test]
    fn the_revision_exchange_round_trips() {
        assert_eq!(read_query(&query()).unwrap(), ());
        assert_eq!(read_answer(&answer()).unwrap(), REVISION);
        // Another revision's exchange is still answered, and reads as none.
        let other = [QUERY, REVISION + 1, 9];
        assert_eq!(read_query(&other).unwrap(), ());
        let answer_of_another = [ANSWER, REVISION + 1, 9];
        assert_eq!(read_answer(&answer_of_another).unwrap(), 0);
        assert!(read_query(&[ANSWER, REVISION]).is_err());
        assert!(read_answer(&[QUERY, REVISION]).is_err());
        assert!(read_query(&[QUERY, REVISION, 0]).is_err(), "trailing bytes");
        assert!(read_query(&[]).is_err());
    }

    #[test]
    fn the_legacy_layout_round_trips_and_refuses_none() {
        let m = Message {
            window: 4,
            pos: Some([3, 70, -5]),
            data: vec![1, 2, 3],
        };
        assert_eq!(decode_legacy(&encode_legacy(&m).unwrap()).unwrap(), m);
        // A legacy payload is not this revision's layout.
        assert!(decode(&encode_legacy(&m).unwrap()).is_err());
        assert!(decode_legacy(&encode(&m).unwrap()).is_err());
        // A client that reads the legacy layout is never sent a blockless menu.
        let blockless = Message {
            pos: None,
            ..m.clone()
        };
        assert!(encode_legacy(&blockless).is_err());
        let big = Message {
            data: vec![0; MAX_DATA + 1],
            ..m
        };
        assert!(encode_legacy(&big).is_err());
    }
}
