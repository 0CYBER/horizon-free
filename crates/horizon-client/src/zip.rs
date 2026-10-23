//! Read-only ZIP archives held in memory (resource packs, data packs and the
//! unifont glyph archive), on the `zip` crate. Like Java's `ZipFile` (what
//! vanilla's `FilePackResources` reads with), entries aren't CRC-checked.
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Cursor, Read},
    path::Path,
    sync::Arc,
};
use zip::{CompressionMethod, ZipArchive};

/// One archive with its entries indexed by name (`\` read as `/`, folders
/// left out).
pub struct Zip {
    archive: ZipArchive<Cursor<Arc<[u8]>>>,
    entries: BTreeMap<String, usize>,
}
fn invalid(e: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e.to_string())
}
impl Zip {
    pub fn open(path: &Path) -> io::Result<Self> {
        Self::from_bytes(fs::read(path)?)
    }
    pub fn from_bytes(data: Vec<u8>) -> io::Result<Self> {
        let archive = ZipArchive::new(Cursor::new(Arc::from(data))).map_err(invalid)?;
        let entries = (0..archive.len())
            .filter_map(|i| Some((archive.name_for_index(i)?.replace('\\', "/"), i)))
            .filter(|(name, _)| !name.ends_with('/'))
            .collect();
        Ok(Self { archive, entries })
    }
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }
    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }
    pub fn read(&self, name: &str) -> io::Result<Vec<u8>> {
        let &index = self
            .entries
            .get(name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, name.to_owned()))?;
        // A clone shares the bytes and the central directory: reading needs `&mut`.
        let mut archive = self.archive.clone();
        let mut entry = archive.by_index_raw(index).map_err(invalid)?;
        let (method, size) = (entry.compression(), entry.size() as usize);
        let mut raw = Vec::with_capacity(entry.compressed_size() as usize);
        entry.read_to_end(&mut raw)?;
        match method {
            CompressionMethod::Stored => Ok(raw),
            CompressionMethod::Deflated => {
                miniz_oxide::inflate::decompress_to_vec_with_limit(&raw, size.max(1) * 2 + 1024)
                    .map_err(|_| invalid("corrupt deflate stream"))
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
        // Its CRCs are zero: Java's ZipFile doesn't check them either.
        let zip = Zip::from_bytes(archive()).unwrap();
        assert_eq!(zip.names().collect::<Vec<_>>(), ["a/b.txt", "pack.mcmeta"]);
        assert_eq!(zip.read("pack.mcmeta").unwrap(), b"{\"pack\":{}}");
        assert_eq!(zip.read("a/b.txt").unwrap(), b"hello hello hello hello");
        assert!(zip.read("missing").is_err());
    }

    /// A pack as zip tools write it: folder entries, a nested pack root,
    /// stored and deflated files, and a Windows-style name.
    fn written() -> Vec<u8> {
        use std::io::Write;
        use zip::write::SimpleFileOptions;
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        w.add_directory("Pack-main/", stored).unwrap();
        w.add_directory("Pack-main/assets/minecraft/", stored)
            .unwrap();
        w.start_file("Pack-main/pack.mcmeta", stored).unwrap();
        w.write_all(b"{\"pack\":{\"pack_format\":64}}").unwrap();
        w.start_file("Pack-main/assets/minecraft/lang/en_us.json", deflated)
            .unwrap();
        w.write_all("{\"a\":\"b\"}".repeat(100).as_bytes()).unwrap();
        w.start_file(
            "Pack-main\\assets\\minecraft\\texts\\splashes.txt",
            deflated,
        )
        .unwrap();
        w.write_all(b"Also try Horizon!").unwrap();
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn reads_nested_folders_and_skips_folder_entries() {
        let zip = Zip::from_bytes(written()).unwrap();
        assert_eq!(
            zip.names().collect::<Vec<_>>(),
            [
                "Pack-main/assets/minecraft/lang/en_us.json",
                "Pack-main/assets/minecraft/texts/splashes.txt",
                "Pack-main/pack.mcmeta",
            ]
        );
        assert!(zip.contains("Pack-main/pack.mcmeta"));
        assert!(!zip.contains("Pack-main/"));
        assert_eq!(
            zip.read("Pack-main/assets/minecraft/lang/en_us.json")
                .unwrap(),
            "{\"a\":\"b\"}".repeat(100).as_bytes()
        );
        assert_eq!(
            zip.read("Pack-main/assets/minecraft/texts/splashes.txt")
                .unwrap(),
            b"Also try Horizon!"
        );
    }

    #[test]
    fn a_bad_entry_fails_alone() {
        let mut bytes = written();
        // The deflated lang file's stream becomes a reserved block type.
        let (start, len) = {
            let mut archive = ZipArchive::new(Cursor::new(bytes.as_slice())).unwrap();
            let index = archive
                .index_for_name("Pack-main/assets/minecraft/lang/en_us.json")
                .unwrap();
            let entry = archive.by_index_raw(index).unwrap();
            (
                entry.data_start() as usize,
                entry.compressed_size() as usize,
            )
        };
        bytes[start..start + len].fill(0xff);
        let zip = Zip::from_bytes(bytes).unwrap();
        assert!(
            zip.read("Pack-main/assets/minecraft/lang/en_us.json")
                .is_err()
        );
        assert_eq!(
            zip.read("Pack-main/pack.mcmeta").unwrap(),
            b"{\"pack\":{\"pack_format\":64}}"
        );
    }

    #[test]
    fn refuses_what_is_not_a_zip() {
        assert!(Zip::from_bytes(b"PK not really".to_vec()).is_err());
        assert!(Zip::from_bytes(Vec::new()).is_err());
        let mut cut = written();
        cut.truncate(cut.len() / 2);
        assert!(Zip::from_bytes(cut).is_err());
    }
}
