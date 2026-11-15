//! Bound a ZIP's central directory before the ZIP reader creates its entry table.
use std::io::{Read, Seek, SeekFrom};
use zip::ZipArchive;
use zip::read::{ArchiveOffset, Config};
use zip::result::{ZipError, ZipResult};

mod source;
pub use source::Source;

/// A ZIP whose directory and end records are read from a checked snapshot.
pub type Archive<R> = ZipArchive<Source<R>>;

// GTNH 2.9.0 ships about 18,000 entries. Leave room for larger packs while
// bounding per-entry allocations, including for empty files and directories.
const MAX_ENTRIES: u64 = 100_000;
// Bound names, extra fields and comments independently of the entry count.
const MAX_DIRECTORY_BYTES: u64 = 64 << 20;

const END: [u8; 4] = *b"PK\x05\x06";
const END64: [u8; 4] = *b"PK\x06\x06";
const LOCATOR64: [u8; 4] = *b"PK\x06\x07";
const CENTRAL_HEADER: [u8; 4] = *b"PK\x01\x02";
const DIRECTORY_SIGNATURE: [u8; 4] = *b"PK\x05\x05";
const END_LEN: usize = 22;
const END_WINDOW: usize = END_LEN + (1 << 16);
const END64_LEN: usize = 56;
const LOCATOR64_LEN: usize = 20;
// A fixed ZIP64 footer is 56 bytes. Bound its optional extension and the search
// for the footer when an executable or other prefix shifts the archive offsets.
const END64_WINDOW: usize = 1 << 16;
const CENTRAL_HEADER_LEN: usize = 46;

#[derive(Debug)]
struct Directory {
    entries: u64,
    size: u64,
    offset: u64,
    end: u64,
    // In ZIP64 the locator independently specifies the relative footer offset.
    archive_offset: Option<u64>,
}

/// Opens a ZIP after checking the declared directory and walking its raw records.
/// Every record consumes one entry, including empty files, directories and duplicate names.
pub fn open<R: Read + Seek>(mut reader: R) -> ZipResult<Archive<R>> {
    let len = reader.seek(SeekFrom::End(0))?;
    let tail_start = len.saturating_sub((END_WINDOW + LOCATOR64_LEN + END64_WINDOW) as u64);
    let mut tail = vec![0; (len - tail_start) as usize];
    reader.seek(SeekFrom::Start(tail_start))?;
    reader.read_exact(&mut tail)?;

    let (directory, mark_zip64) = read_directory(&tail, tail_start)?;
    let (start, offset) = directory_offsets(&directory)?;
    if let Some(at) = mark_zip64 {
        // A redundant ZIP64 footer can agree with the classic fields without
        // their sentinels. Signal ZIP64 to the parser so it uses the checked
        // end of the directory, and cannot fall back past it on an error.
        tail[at + 8..at + 12].fill(0xff);
    }
    let snapshot_start = start.min(tail_start);
    if snapshot_start < tail_start {
        // The directory's declared size has already passed its limit. Keep
        // the original footer bytes even if another process writes the file.
        let prefix_len = (tail_start - snapshot_start) as usize;
        let mut snapshot = vec![0; prefix_len + tail.len()];
        reader.seek(SeekFrom::Start(snapshot_start))?;
        reader.read_exact(&mut snapshot[..prefix_len])?;
        snapshot[prefix_len..].copy_from_slice(&tail);
        tail = snapshot;
    }
    let relative_start = (start - snapshot_start) as usize;
    let directory_bytes = &tail[relative_start..relative_start + directory.size as usize];
    check_entries(directory_bytes, directory.entries)?;

    // Both passes see the same metadata. The checked directory abuts the
    // selected footer, so zip stops there if decoding an entry fails.
    ZipArchive::with_config(
        Config {
            archive_offset: ArchiveOffset::Known(offset),
        },
        Source::new(reader, snapshot_start, tail),
    )
}

fn read_directory(tail: &[u8], tail_start: u64) -> ZipResult<(Directory, Option<usize>)> {
    // Like zip, inspect the latest complete end record in the ZIP comment
    // window. Skip disk fields the ZIP reader will certainly reject. Refuse
    // other inconsistent records rather than following an unchecked footer.
    let end_window_start = tail.len().saturating_sub(END_WINDOW);
    let (at, end) = tail[end_window_start..]
        .windows(END_LEN)
        .enumerate()
        .rev()
        .find(|&(i, block)| {
            let at = end_window_start + i;
            block.starts_with(&END)
                && at + END_LEN + usize::from(u16_at(block, 20)) <= tail.len()
                && ((uses_zip64(block) && zip64_locator(tail, at).is_some())
                    || (u16_at(block, 4) == 0
                        && u16_at(block, 6) == 0
                        && u16_at(block, 8) == u16_at(block, 10)))
        })
        .map(|(i, block)| (end_window_start + i, block))
        .ok_or_else(|| invalid("ZIP end record not found"))?;

    let classic = Directory {
        entries: u64::from(u16_at(end, 10)),
        size: u64::from(u32_at(end, 12)),
        offset: u64::from(u32_at(end, 16)),
        end: tail_start + at as u64,
        archive_offset: None,
    };
    if let Some(locator_at) = zip64_locator(tail, at) {
        let zip64 = read_zip64(&tail[..locator_at], tail_start, &tail[locator_at..at]);
        if uses_zip64(end) {
            return zip64.map(|directory| (directory, None));
        }
        if let Ok(directory) = zip64
            && (directory.entries, directory.size, directory.offset)
                == (classic.entries, classic.size, classic.offset)
        {
            return Ok((directory, Some(at)));
        }
    }
    Ok((classic, None))
}

fn uses_zip64(end: &[u8]) -> bool {
    u16_at(end, 10) == u16::MAX || u32_at(end, 12) == u32::MAX || u32_at(end, 16) == u32::MAX
}

fn zip64_locator(tail: &[u8], at: usize) -> Option<usize> {
    let locator = at.checked_sub(LOCATOR64_LEN)?;
    tail[locator..at].starts_with(&LOCATOR64).then_some(locator)
}

fn read_zip64(before_locator: &[u8], tail_start: u64, locator: &[u8]) -> ZipResult<Directory> {
    if u32_at(locator, 4) != 0 || u32_at(locator, 16) != 1 {
        return Err(invalid("multi-disk ZIP archives are not supported"));
    }
    let search_start = before_locator.len().saturating_sub(END64_WINDOW);
    let (at, end) = before_locator[search_start..]
        .windows(END64_LEN)
        .enumerate()
        .rev()
        .find(|&(i, block)| {
            block.starts_with(&END64)
                && u64_at(block, 4).checked_add(12)
                    == Some((before_locator.len() - search_start - i) as u64)
        })
        .ok_or_else(|| invalid("missing or oversized ZIP64 end record"))?;
    let end_at = tail_start + (search_start + at) as u64;
    let archive_offset = end_at
        .checked_sub(u64_at(locator, 8))
        .ok_or_else(|| invalid("invalid ZIP64 end record offset"))?;
    let entries = u64_at(end, 32);
    if u32_at(end, 16) != 0 || u32_at(end, 20) != 0 || u64_at(end, 24) != entries {
        return Err(invalid("multi-disk ZIP archives are not supported"));
    }
    Ok(Directory {
        entries,
        size: u64_at(end, 40),
        offset: u64_at(end, 48),
        end: end_at,
        archive_offset: Some(archive_offset),
    })
}

fn directory_offsets(dir: &Directory) -> ZipResult<(u64, u64)> {
    if dir.entries > MAX_ENTRIES {
        return Err(ZipError::InvalidArchive(
            format!("ZIP entry count exceeds the {MAX_ENTRIES}-entry limit").into(),
        ));
    }
    if dir.size > MAX_DIRECTORY_BYTES {
        return Err(ZipError::InvalidArchive(
            format!(
                "ZIP central directory size exceeds the {} MiB limit",
                MAX_DIRECTORY_BYTES >> 20
            )
            .into(),
        ));
    }
    if dir.size < dir.entries * CENTRAL_HEADER_LEN as u64 {
        return Err(invalid(
            "ZIP central directory size does not match its entry count",
        ));
    }
    let start = dir.end.checked_sub(dir.size).ok_or_else(bad_offset)?;
    let offset = start.checked_sub(dir.offset).ok_or_else(bad_offset)?;
    if dir.archive_offset.is_some_and(|zip64| zip64 != offset) {
        return Err(bad_offset());
    }

    Ok((start, offset))
}

fn check_entries(mut remaining: &[u8], entries: u64) -> ZipResult<()> {
    for _ in 0..entries {
        let (header, rest) = remaining
            .split_first_chunk::<CENTRAL_HEADER_LEN>()
            .ok_or_else(|| invalid("truncated ZIP central directory header"))?;
        if !header.starts_with(&CENTRAL_HEADER) {
            return Err(invalid("invalid ZIP central directory header"));
        }
        // File name, extra field and comment follow the fixed header.
        let extra = usize::from(u16_at(header, 28))
            + usize::from(u16_at(header, 30))
            + usize::from(u16_at(header, 32));
        remaining = rest
            .get(extra..)
            .ok_or_else(|| invalid("ZIP entry exceeds the central directory size"))?;
    }

    if !remaining.is_empty()
        && !(remaining.len() >= 6
            && remaining.starts_with(&DIRECTORY_SIGNATURE)
            && remaining.len() == 6 + usize::from(u16_at(remaining, 4)))
    {
        return Err(invalid(
            "ZIP central directory size does not match its entries",
        ));
    }
    Ok(())
}

fn invalid(message: &'static str) -> ZipError {
    ZipError::InvalidArchive(message.into())
}

fn bad_offset() -> ZipError {
    invalid("invalid ZIP central directory offset")
}

// All callers pass complete fixed-length records and offsets defined by the ZIP format.
fn field<const N: usize>(record: &[u8], offset: usize) -> [u8; N] {
    std::array::from_fn(|i| record[offset + i])
}

fn u16_at(record: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(field(record, offset))
}

fn u32_at(record: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(field(record, offset))
}

fn u64_at(record: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(field(record, offset))
}

#[cfg(test)]
mod tests;
