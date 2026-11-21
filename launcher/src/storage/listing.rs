//! The records of a Windows `FileIdBothDirectoryInfo` folder listing, read from the bytes the
//! call filled. Plain byte reads at the documented offsets, so the checks run on every platform.

/// Where each field of `FILE_ID_BOTH_DIR_INFO` starts (its x86 and x64 layouts are the same).
const NEXT: usize = 0;
const ALLOCATION: usize = 48;
const ATTRIBUTES: usize = 56;
const NAME_LENGTH: usize = 60;
const FILE_ID: usize = 96;
pub(super) const NAME: usize = 104;

#[cfg(windows)]
const _: () = {
    use std::mem::offset_of;
    use windows_sys::Win32::Storage::FileSystem::FILE_ID_BOTH_DIR_INFO as Info;
    assert!(offset_of!(Info, NextEntryOffset) == NEXT);
    assert!(offset_of!(Info, AllocationSize) == ALLOCATION);
    assert!(offset_of!(Info, FileAttributes) == ATTRIBUTES);
    assert!(offset_of!(Info, FileNameLength) == NAME_LENGTH);
    assert!(offset_of!(Info, FileId) == FILE_ID);
    assert!(offset_of!(Info, FileName) == NAME);
};

/// One child of the listed folder.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Listed {
    /// One path component in UTF-16: never empty, `.`, `..`, or holding a separator.
    pub name: Vec<u16>,
    pub attributes: u32,
    /// 0 where the filesystem gives no file ID.
    pub id: u64,
    pub bytes: u64,
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}

fn i64_at(bytes: &[u8], at: usize) -> i64 {
    i64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}

fn malformed() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "a folder listing record is malformed",
    )
}

/// The children in one filled listing buffer, `.` and `..` left out. A record that reaches past
/// the buffer, a name of odd length or one that is not a single plain component, and a next
/// record that overlaps this one, is misaligned or lies past the buffer are refused: nothing is
/// read twice, outside the buffer or as a path other than a child of the folder. The caller
/// zeroes the buffer before each call, so a chain never runs into an earlier call's records.
pub(super) fn parse(buffer: &[u8]) -> std::io::Result<Vec<Listed>> {
    let mut children = Vec::new();
    let mut offset = 0usize;
    loop {
        let record = buffer.get(offset..).ok_or_else(malformed)?;
        if record.len() < NAME {
            return Err(malformed());
        }
        let length = u32_at(record, NAME_LENGTH) as usize;
        if length == 0 || !length.is_multiple_of(2) || length > record.len() - NAME {
            return Err(malformed());
        }
        let name: Vec<u16> = record[NAME..NAME + length]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&unit| u16::from_le_bytes(unit))
            .collect();
        let dot = u16::from(b'.');
        if name.iter().any(|&unit| {
            unit == 0
                || unit == u16::from(b'/')
                || unit == u16::from(b'\\')
                || unit == u16::from(b':')
        }) {
            return Err(malformed());
        }
        if name != [dot] && name != [dot, dot] {
            children.push(Listed {
                name,
                attributes: u32_at(record, ATTRIBUTES),
                id: u64_at(record, FILE_ID),
                // A signed size: a negative one counts nothing, never 8 EiB.
                bytes: u64::try_from(i64_at(record, ALLOCATION)).unwrap_or(0),
            });
        }
        let next = u32_at(record, NEXT) as usize;
        if next == 0 {
            return Ok(children);
        }
        if next < NAME + length || !next.is_multiple_of(8) || next >= record.len() {
            return Err(malformed());
        }
        offset += next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A record as Windows lays it out, `next` bytes long (0: the last), with a name of
    /// `length` bytes (the name's own length unless given).
    fn record(name: &str, next: u32, length: Option<u32>) -> Vec<u8> {
        let units: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let mut bytes = vec![0u8; NAME + units.len()];
        bytes[NEXT..NEXT + 4].copy_from_slice(&next.to_le_bytes());
        bytes[ALLOCATION..ALLOCATION + 8].copy_from_slice(&4096u64.to_le_bytes());
        bytes[ATTRIBUTES..ATTRIBUTES + 4].copy_from_slice(&0x20u32.to_le_bytes());
        let length = length.unwrap_or(units.len() as u32);
        bytes[NAME_LENGTH..NAME_LENGTH + 4].copy_from_slice(&length.to_le_bytes());
        bytes[FILE_ID..FILE_ID + 8].copy_from_slice(&7u64.to_le_bytes());
        bytes[NAME..].copy_from_slice(&units);
        if next != 0 {
            bytes.resize(next as usize, 0);
        }
        bytes
    }

    fn listing(records: &[Vec<u8>]) -> Vec<u8> {
        let mut buffer = records.concat();
        buffer.resize(buffer.len() + 64, 0);
        buffer
    }

    #[test]
    fn records_chain_and_dots_are_left_out() {
        let buffer = listing(&[
            record(".", 112, None),
            record("..", 112, None),
            record("lib.rs", 120, None),
            record("target", 0, None),
        ]);
        let children = parse(&buffer).unwrap();
        let names: Vec<String> = children
            .iter()
            .map(|child| String::from_utf16(&child.name).unwrap())
            .collect();
        assert_eq!(names, ["lib.rs", "target"]);
        assert_eq!((children[0].id, children[0].bytes), (7, 4096));
        assert_eq!(children[0].attributes, 0x20);
    }

    #[test]
    fn a_negative_allocation_counts_nothing() {
        let mut negative = record("a", 0, None);
        negative[ALLOCATION..ALLOCATION + 8].copy_from_slice(&(-1i64).to_le_bytes());
        assert_eq!(parse(&listing(&[negative])).unwrap()[0].bytes, 0);
    }

    #[test]
    fn malformed_records_are_refused() {
        let refused = |records: &[Vec<u8>]| parse(&listing(records)).is_err();
        assert!(refused(&[record("a", 0, Some(3))]), "odd name length");
        assert!(refused(&[record("a", 0, Some(0))]), "empty name");
        assert!(
            refused(&[record("a", 0, Some(4096))]),
            "name past the buffer"
        );
        for name in ["a/b", "a\\b", "c:", "a\0"] {
            assert!(
                refused(&[record(name, 0, None)]),
                "{name:?} is no single component"
            );
        }
        // A whole name, 116 bytes, then a next record said to start at 112, inside it.
        let mut overlapping = record("abcdef", 0, None);
        overlapping[NEXT..NEXT + 4].copy_from_slice(&112u32.to_le_bytes());
        overlapping.resize(120, 0);
        assert!(
            refused(&[overlapping, record("b", 0, None)]),
            "the next record overlaps this one's name"
        );
        assert!(
            refused(&[record("a", 116, None), record("b", 0, None)]),
            "the next record is misaligned"
        );
        let mut far = record("a", 0, None);
        far[NEXT..NEXT + 4].copy_from_slice(&1_000_000u32.to_le_bytes());
        assert!(refused(&[far]), "the next record is past the buffer");
        assert!(
            parse(&[0u8; NAME - 1]).is_err(),
            "a buffer shorter than one record"
        );
        // A zeroed buffer, as an empty call leaves it, holds no name: refused, not a child.
        assert!(parse(&[0u8; 256]).is_err());
    }
}
