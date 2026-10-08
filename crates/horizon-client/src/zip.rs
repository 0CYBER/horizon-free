//! Minimal read-only ZIP archive support (stored and deflated entries), used
//! for resource packs and the unifont glyph archive.
use std::{collections::BTreeMap, fs, io, path::Path};

/// One archive held in memory with its central directory indexed by name.
pub struct Zip {
    data: Vec<u8>,
    entries: BTreeMap<String, Entry>,
}
#[derive(Clone, Copy)]
struct Entry {
    method: u16,
    compressed: usize,
    size: usize,
    header: usize,
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn u16_at(d: &[u8], at: usize) -> io::Result<u16> {
    d.get(at..at + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| invalid("truncated zip"))
}
fn u32_at(d: &[u8], at: usize) -> io::Result<u32> {
    d.get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| invalid("truncated zip"))
}
impl Zip {
    pub fn open(path: &Path) -> io::Result<Self> {
        Self::from_bytes(fs::read(path)?)
    }
    pub fn from_bytes(data: Vec<u8>) -> io::Result<Self> {
        // End of central directory: the last "PK\x05\x06" within 64 KiB of the end.
        let min = data.len().saturating_sub(22 + 65_535);
        let eocd = (min..=data.len().saturating_sub(22))
            .rev()
            .find(|&i| data[i..i + 4] == [0x50, 0x4b, 0x05, 0x06])
            .ok_or_else(|| invalid("not a zip archive"))?;
        let count = usize::from(u16_at(&data, eocd + 10)?);
        let mut at = u32_at(&data, eocd + 16)? as usize;
        let mut entries = BTreeMap::new();
        for _ in 0..count {
            if u32_at(&data, at)? != 0x0201_4b50 {
                return Err(invalid("bad central directory entry"));
            }
            let method = u16_at(&data, at + 10)?;
            let compressed = u32_at(&data, at + 20)? as usize;
            let size = u32_at(&data, at + 24)? as usize;
            let name_len = usize::from(u16_at(&data, at + 28)?);
            let extra_len = usize::from(u16_at(&data, at + 30)?);
            let comment_len = usize::from(u16_at(&data, at + 32)?);
            let header = u32_at(&data, at + 42)? as usize;
            let name = data
                .get(at + 46..at + 46 + name_len)
                .ok_or_else(|| invalid("truncated zip name"))?;
            let name = String::from_utf8_lossy(name).replace('\\', "/");
            if !name.ends_with('/') {
                entries.insert(
                    name,
                    Entry {
                        method,
                        compressed,
                        size,
                        header,
                    },
                );
            }
            at += 46 + name_len + extra_len + comment_len;
        }
        Ok(Self { data, entries })
    }
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }
    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }
    pub fn read(&self, name: &str) -> io::Result<Vec<u8>> {
        let e = *self
            .entries
            .get(name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, name.to_owned()))?;
        if u32_at(&self.data, e.header)? != 0x0403_4b50 {
            return Err(invalid("bad local file header"));
        }
        let start = e.header
            + 30
            + usize::from(u16_at(&self.data, e.header + 26)?)
            + usize::from(u16_at(&self.data, e.header + 28)?);
        let raw = self
            .data
            .get(start..start + e.compressed)
            .ok_or_else(|| invalid("truncated zip entry"))?;
        match e.method {
            0 => Ok(raw.to_vec()),
            8 => {
                let out = miniz_oxide::inflate::decompress_to_vec_with_limit(
                    raw,
                    e.size.max(1) * 2 + 1024,
                )
                .map_err(|_| invalid("corrupt deflate stream"))?;
                Ok(out)
            }
            _ => Err(invalid("unsupported zip compression")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// A hand-built archive with one stored and one deflated entry.
    fn archive() -> Vec<u8> {
        let files: [(&str, &[u8], bool); 2] = [
            ("pack.mcmeta", b"{\"pack\":{}}", false),
            ("a/b.txt", b"hello hello hello hello", true),
        ];
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, body, deflate) in files {
            let data = if deflate {
                miniz_oxide::deflate::compress_to_vec(body, 6)
            } else {
                body.to_vec()
            };
            let offset = out.len() as u32;
            let local = |v: &mut Vec<u8>, sig: u32| {
                v.extend(sig.to_le_bytes());
            };
            local(&mut out, 0x0403_4b50);
            out.extend([20, 0, 0, 0]);
            out.extend((if deflate { 8u16 } else { 0 }).to_le_bytes());
            out.extend([0; 8]);
            out.extend((data.len() as u32).to_le_bytes());
            out.extend((body.len() as u32).to_le_bytes());
            out.extend((name.len() as u16).to_le_bytes());
            out.extend(0u16.to_le_bytes());
            out.extend(name.as_bytes());
            out.extend(&data);
            local(&mut central, 0x0201_4b50);
            central.extend([20, 0, 20, 0, 0, 0]);
            central.extend((if deflate { 8u16 } else { 0 }).to_le_bytes());
            central.extend([0; 8]);
            central.extend((data.len() as u32).to_le_bytes());
            central.extend((body.len() as u32).to_le_bytes());
            central.extend((name.len() as u16).to_le_bytes());
            central.extend([0; 12]);
            central.extend(offset.to_le_bytes());
            central.extend(name.as_bytes());
        }
        let start = out.len() as u32;
        out.extend(&central);
        out.extend(0x0605_4b50u32.to_le_bytes());
        out.extend([0; 4]);
        out.extend(2u16.to_le_bytes());
        out.extend(2u16.to_le_bytes());
        out.extend((central.len() as u32).to_le_bytes());
        out.extend(start.to_le_bytes());
        out.extend([0; 2]);
        out
    }
    #[test]
    fn reads_stored_and_deflated_entries() {
        let zip = Zip::from_bytes(archive()).unwrap();
        assert_eq!(zip.names().collect::<Vec<_>>(), ["a/b.txt", "pack.mcmeta"]);
        assert_eq!(zip.read("pack.mcmeta").unwrap(), b"{\"pack\":{}}");
        assert_eq!(zip.read("a/b.txt").unwrap(), b"hello hello hello hello");
        assert!(zip.read("missing").is_err());
    }
}
