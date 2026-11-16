//! Bounded binary framing for the dedicated negotiated settings channel.
use super::{
    BookFlags,
    custom_books::{MAX_BOOKS, MAX_ID_BYTES, valid_book_id},
    settings_sync::{MAX_BYTES, Message},
};

pub const PLACEMENT_TAG: u8 = 8;

pub fn encode(message: &Message) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    fn key(out: &mut Vec<u8>, id: &str) -> Option<()> {
        if id.len() > MAX_ID_BYTES || !valid_book_id(id) {
            return None;
        }
        out.extend_from_slice(&(id.len() as u16).to_be_bytes());
        out.extend_from_slice(id.as_bytes());
        Some(())
    }
    fn pair(out: &mut Vec<u8>, flags: BookFlags) {
        out.extend([u8::from(flags.open), u8::from(flags.filtering)]);
    }
    match message {
        Message::Busy => out.push(7),
        Message::Hello => out.extend([5, 1]),
        Message::Ready => out.extend([6, 1]),
        Message::Offer(keys) => {
            if keys.len() > MAX_BOOKS {
                return None;
            }
            out.push(0);
            out.extend_from_slice(&(keys.len() as u16).to_be_bytes());
            for id in keys {
                key(&mut out, id)?;
            }
        }
        Message::Snapshot {
            epoch,
            revision,
            entries,
        } => {
            if entries.len() > MAX_BOOKS {
                return None;
            }
            out.push(1);
            out.extend(epoch.to_be_bytes());
            out.extend(revision.to_be_bytes());
            out.extend_from_slice(&(entries.len() as u16).to_be_bytes());
            for (id, flags) in entries {
                key(&mut out, id)?;
                pair(&mut out, *flags);
            }
        }
        Message::Request {
            epoch,
            number,
            key: id,
            flags,
        } => {
            out.push(2);
            out.extend(epoch.to_be_bytes());
            out.extend(number.to_be_bytes());
            key(&mut out, id)?;
            pair(&mut out, *flags);
        }
        Message::Update {
            epoch,
            revision,
            key: id,
            flags,
        } => {
            out.push(3);
            out.extend(epoch.to_be_bytes());
            out.extend(revision.to_be_bytes());
            key(&mut out, id)?;
            pair(&mut out, *flags);
        }
        Message::Resnapshot => out.push(4),
        Message::Placement(frame) => {
            out.push(PLACEMENT_TAG);
            out.extend(super::placement_codec::encode(frame)?);
        }
    }
    (out.len() <= MAX_BYTES).then_some(out)
}

pub fn decode(data: &[u8]) -> Option<Message> {
    if data.len() > MAX_BYTES {
        return None;
    }
    struct Reader<'a>(&'a [u8]);
    impl Reader<'_> {
        fn take(&mut self, n: usize) -> Option<&[u8]> {
            let (a, b) = self.0.split_at_checked(n)?;
            self.0 = b;
            Some(a)
        }
        fn byte(&mut self) -> Option<u8> {
            Some(self.take(1)?[0])
        }
        fn number(&mut self) -> Option<u64> {
            Some(u64::from_be_bytes(self.take(8)?.try_into().ok()?))
        }
        fn count(&mut self) -> Option<usize> {
            Some(u16::from_be_bytes(self.take(2)?.try_into().ok()?) as usize)
        }
        fn key(&mut self) -> Option<String> {
            let n = self.count()?;
            if n > MAX_ID_BYTES {
                return None;
            }
            let id = std::str::from_utf8(self.take(n)?).ok()?;
            valid_book_id(id).then(|| id.to_owned())
        }
        fn flags(&mut self) -> Option<BookFlags> {
            let open = self.byte()?;
            let filtering = self.byte()?;
            if open > 1 || filtering > 1 {
                return None;
            }
            Some(BookFlags {
                open: open == 1,
                filtering: filtering == 1,
            })
        }
    }
    let mut r = Reader(data);
    let tag = r.byte()?;
    let result = match tag {
        0 | 1 => {
            let (epoch, revision) = if tag == 1 {
                (r.number()?, r.number()?)
            } else {
                (0, 0)
            };
            let n = r.count()?;
            if n > MAX_BOOKS || n > r.0.len() / 3 {
                return None;
            }
            let mut seen = std::collections::BTreeSet::new();
            let mut entries = Vec::new();
            for _ in 0..n {
                let id = r.key()?;
                if !seen.insert(id.clone()) {
                    return None;
                }
                let flags = if tag == 1 {
                    r.flags()?
                } else {
                    BookFlags::default()
                };
                entries.push((id, flags));
            }
            if tag == 0 {
                Message::Offer(entries.into_iter().map(|(id, _)| id).collect())
            } else {
                Message::Snapshot {
                    epoch,
                    revision,
                    entries,
                }
            }
        }
        2 | 3 => {
            let epoch = r.number()?;
            let number = r.number()?;
            let key = r.key()?;
            let flags = r.flags()?;
            if tag == 2 {
                Message::Request {
                    epoch,
                    number,
                    key,
                    flags,
                }
            } else {
                Message::Update {
                    epoch,
                    revision: number,
                    key,
                    flags,
                }
            }
        }
        4 => Message::Resnapshot,
        7 => Message::Busy,
        5 | 6 => {
            if r.byte()? != 1 {
                return None;
            }
            if tag == 5 {
                Message::Hello
            } else {
                Message::Ready
            }
        }
        PLACEMENT_TAG => {
            let frame = super::placement_codec::decode(r.0)?;
            r.0 = &[];
            Message::Placement(frame)
        }
        _ => return None,
    };
    r.0.is_empty().then_some(result)
}
