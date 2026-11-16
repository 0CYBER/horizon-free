//! Strict region reads for carrier recovery (issue arismoko/horizon#2413, slice 2 part B).
//!
//! Recovery does not go through the ordinary chunk loader, which treats
//! damage as absence and would happily generate terrain. This reader
//! addresses a region file directly and refuses everything it cannot
//! prove: a full 8 KiB header, an offset and sector count inside the
//! file, a non-zero entry, and a payload that decompresses and decodes,
//! external `.mcc` streams included. An affected chunk that is not on
//! disk is [`Strict::Missing`] — corruption, not absence — and nothing
//! here ever invents content.

use crate::compression::Compression;
use horizon_core::nbt::{self, Limits, NamedTag};
use horizon_core::position::ChunkPos;
use std::{
    fs,
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

/// A region sector, in bytes.
const SECTOR: u64 = 4096;
/// The region header: the offset and timestamp tables.
const HEADER: usize = 8192;
/// The most an external `.mcc` stream may hold, as the region writer
/// caps it.
const MAX_COMPRESSED: usize = 64 * 1024 * 1024;

/// Why a strictly read chunk did not read. `Missing` is a well-formed
/// region without the chunk; every other variant is damage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Strict {
    /// The region file does not exist at all.
    NoRegion,
    /// The chunk has no entry in this region.
    Missing,
    /// The header is short, so the offset table cannot be trusted.
    ShortHeader,
    /// The entry's sector range does not lie inside the file.
    EntryPastEof,
    /// The payload's own length does not fit the entry's sectors.
    TruncatedPayload,
    /// The payload's compression byte is not one the writer writes.
    UnknownCompression(u8),
    /// The external `.mcc` stream the entry points at is gone.
    ExternalMissing,
    /// The payload does not decompress, or the NBT does not decode.
    Undecodable(String),
    /// Plain I/O.
    Io(String),
}
impl Strict {
    /// Whether this refusal means damage rather than a well-formed
    /// absence. Damage refuses the world's load.
    pub fn is_damage(&self) -> bool {
        !matches!(self, Self::NoRegion | Self::Missing)
    }
}
impl fmt::Display for Strict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoRegion => write!(f, "region file does not exist"),
            Self::Missing => write!(f, "chunk has no region entry"),
            Self::ShortHeader => write!(f, "region header is short"),
            Self::EntryPastEof => write!(f, "region entry lies past the file"),
            Self::TruncatedPayload => write!(f, "chunk payload does not fit its sectors"),
            Self::UnknownCompression(byte) => write!(f, "unknown compression byte {byte}"),
            Self::ExternalMissing => write!(f, "the external .mcc stream is missing"),
            Self::Undecodable(why) => write!(f, "payload does not decode: {why}"),
            Self::Io(error) => write!(f, "region I/O: {error}"),
        }
    }
}
impl std::error::Error for Strict {}
use std::fmt;

/// The region file a chunk belongs to.
fn region_path(directory: &Path, chunk: ChunkPos) -> PathBuf {
    directory.join(format!("r.{}.{}.mca", chunk.x >> 5, chunk.z >> 5))
}

/// Reads `chunk` from `directory`'s region strictly, returning its root
/// tag. `Ok(None)` is only a well-formed region without the chunk;
/// every damage is a typed [`Strict`] refusal.
pub fn read_chunk(directory: &Path, chunk: ChunkPos) -> Result<Option<NamedTag>, Strict> {
    let path = region_path(directory, chunk);
    let mut file = fs::File::open(&path).map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => Strict::NoRegion,
        _ => Strict::Io(error.to_string()),
    })?;
    let file_len = file
        .metadata()
        .map_err(|e| Strict::Io(e.to_string()))?
        .len();
    // A full 8 KiB header, or nothing: a short header leaves the offset
    // table half-invented.
    let mut header = vec![0_u8; HEADER];
    file.read_exact(&mut header)
        .map_err(|_| Strict::ShortHeader)?;
    let index = (chunk.x.rem_euclid(32) + chunk.z.rem_euclid(32) * 32) as usize;
    let entry = u32::from_be_bytes(header[index * 4..index * 4 + 4].try_into().unwrap());
    if entry == 0 {
        return Ok(None);
    }
    let start = u64::from(entry >> 8);
    let sectors = u64::from(entry & 255);
    // The entry must lie inside the file, payload header included.
    if start < 2 || sectors == 0 || (start + sectors) * SECTOR > file_len {
        return Err(Strict::EntryPastEof);
    }
    file.seek(SeekFrom::Start(start * SECTOR))
        .map_err(|e| Strict::Io(e.to_string()))?;
    let mut head = [0_u8; 5];
    file.read_exact(&mut head)
        .map_err(|_| Strict::TruncatedPayload)?;
    let len = u32::from_be_bytes(head[..4].try_into().unwrap()) as u64;
    if len < 1 || len + 4 > sectors * SECTOR {
        return Err(Strict::TruncatedPayload);
    }
    let compression =
        Compression::try_from(head[4] & 127).map_err(|_| Strict::UnknownCompression(head[4]))?;
    let compressed = if head[4] & 128 != 0 {
        // The stub says nothing about the stream's size; the external file
        // must exist and hold at most what the writer caps.
        let external = directory.join(format!("c.{}.{}.mcc", chunk.x, chunk.z));
        let mut data = Vec::new();
        fs::File::open(&external)
            .map_err(|error| match error.kind() {
                io::ErrorKind::NotFound => Strict::ExternalMissing,
                _ => Strict::Io(error.to_string()),
            })?
            .take((MAX_COMPRESSED + 1) as u64)
            .read_to_end(&mut data)
            .map_err(|e| Strict::Io(e.to_string()))?;
        if data.len() > MAX_COMPRESSED {
            return Err(Strict::TruncatedPayload);
        }
        data
    } else {
        let mut data = vec![0_u8; (len - 1) as usize];
        file.read_exact(&mut data)
            .map_err(|_| Strict::TruncatedPayload)?;
        data
    };
    let raw = compression
        .decode(&compressed, MAX_COMPRESSED)
        .map_err(|e| Strict::Undecodable(e.to_string()))?;
    let (root, used) =
        nbt::read_named(&raw, Limits::saved()).map_err(|e| Strict::Undecodable(e.to_string()))?;
    if used != raw.len() {
        return Err(Strict::Undecodable("bytes follow the chunk root".into()));
    }
    Ok(Some(root))
}
