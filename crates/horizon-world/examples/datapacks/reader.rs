//! Data packs (`pack.mcmeta` + `data/`), folders and zips, into the worldgen
//! entries their files give (`WorldgenOverrides`, lowest pack first). Shared by
//! `chunkgen_bench --datapack` and the chunk_dump probe's `HORIZON_DATAPACKS`.
use horizon_world::worldgen::overrides::{PackFile, WorldgenOverrides, template_id};
use std::{
    collections::BTreeMap,
    fs,
    io::Read as _,
    path::{Path, PathBuf},
};

type Error = Box<dyn std::error::Error + Send + Sync>;

/// `packs` (lowest first, each a folder or a `.zip` with `pack.mcmeta`): the
/// entries their `data/` files give, the structure templates among them;
/// every biome is counted known (the generator needs no more of a biome than
/// its worldgen files).
pub fn overrides(packs: &[&Path]) -> Result<WorldgenOverrides, Error> {
    let mut overrides = WorldgenOverrides::default();
    for &path in packs {
        let pack = Pack::open(path)?;
        for file in pack.names() {
            if let Some(id) = template_id(&file) {
                let nbt = pack
                    .bytes(&file)
                    .ok_or_else(|| format!("{}: {file}: unreadable", pack.id))?;
                overrides.set_template(&id, nbt);
            } else if let PackFile::Unreadable =
                overrides.add_file(&pack.id, &file, || pack.read(&file))
            {
                return Err(format!("{}: {file}: unreadable", pack.id).into());
            }
        }
    }
    for warning in overrides.finish(|_| true) {
        eprintln!("data pack worldgen: {warning}");
    }
    Ok(overrides)
}

/// A pack on disk.
struct Pack {
    /// `file/<name>` in `/datapack` and level.dat.
    id: String,
    source: Source,
}
enum Source {
    Dir(PathBuf),
    Zip(Zip),
}
impl Pack {
    /// The folder or `.zip` at `path`, both with `pack.mcmeta` as the game's
    /// pack repository needs (`datapack::discover_folder`).
    fn open(path: &Path) -> Result<Self, Error> {
        if !path.exists() {
            return Err(format!("{}: no such pack", path.display()).into());
        }
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let source = if path.is_dir() {
            if !path.join("pack.mcmeta").exists() {
                return Err(not_a_pack(path));
            }
            Source::Dir(path.to_owned())
        } else {
            let zip = Zip::open(path)?;
            if !zip.entries.contains_key("pack.mcmeta") {
                return Err(not_a_pack(path));
            }
            Source::Zip(zip)
        };
        Ok(Self {
            id: format!("file/{name}"),
            source,
        })
    }
    /// Every file under `data/`, in the order the game lists a pack's
    /// (`DataPack::list`: a folder walked, a zip's names as its directory
    /// holds them, both sorted).
    fn names(&self) -> Vec<String> {
        match &self.source {
            Source::Dir(root) => {
                let mut out = Vec::new();
                walk(root, &root.join("data"), &mut out);
                out
            }
            Source::Zip(zip) => zip
                .entries
                .keys()
                .filter(|n| n.starts_with("data/"))
                .cloned()
                .collect(),
        }
    }
    fn read(&self, file: &str) -> Option<String> {
        String::from_utf8(self.bytes(file)?).ok()
    }
    fn bytes(&self, file: &str) -> Option<Vec<u8>> {
        match &self.source {
            Source::Dir(root) => fs::read(root.join(file)).ok(),
            Source::Zip(zip) => zip.read(file),
        }
    }
}
fn not_a_pack(path: &Path) -> Error {
    format!("{}: not a data pack (missing pack.mcmeta)", path.display()).into()
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            walk(root, &path, out);
        } else if let Ok(rel) = path.strip_prefix(root) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

/// A zip's entries, from its central directory (what Java's `ZipFile` reads):
/// where each one's bytes are and how they are packed. Stored and deflated
/// entries, as the game's `zip` reader takes. The archive is read whole: a
/// pack is small next to the chunks generated with it.
struct Zip {
    bytes: Vec<u8>,
    /// `name` (with `\` read as `/`) -> entry.
    entries: BTreeMap<String, Entry>,
}
struct Entry {
    method: u16,
    /// Where the entry's bytes start: after its local header.
    at: usize,
    compressed: usize,
}
impl Zip {
    fn open(path: &Path) -> Result<Self, Error> {
        let bytes = fs::read(path)?;
        let entries =
            entries(&bytes).map_err(|e| -> Error { format!("{}: {e}", path.display()).into() })?;
        Ok(Self { bytes, entries })
    }
    fn read(&self, name: &str) -> Option<Vec<u8>> {
        let entry = self.entries.get(name)?;
        let end = entry.at.checked_add(entry.compressed)?;
        let raw = self.bytes.get(entry.at..end)?;
        match entry.method {
            0 => Some(raw.to_vec()),
            8 => {
                let mut out = Vec::new();
                flate2::read::DeflateDecoder::new(raw)
                    .read_to_end(&mut out)
                    .ok()?;
                Some(out)
            }
            _ => None,
        }
    }
}
/// The central directory's entries, from the end record at the archive's tail.
fn entries(bytes: &[u8]) -> Result<BTreeMap<String, Entry>, &'static str> {
    let slice = |at: usize, len: usize| bytes.get(at..at.checked_add(len)?);
    let u16_at = |at: usize| Some(u16::from_le_bytes(slice(at, 2)?.try_into().ok()?));
    let u32_at = |at: usize| Some(u32::from_le_bytes(slice(at, 4)?.try_into().ok()?));
    // The end record, at most 22 bytes plus a 64 KiB comment from the end.
    let end = (0..=bytes.len().saturating_sub(22))
        .rev()
        .take(22 + 0xffff)
        .find(|&at| slice(at, 4) == Some(b"PK\x05\x06"))
        .ok_or("not a zip archive")?;
    let (count, directory) = (
        u16_at(end + 10).ok_or("truncated end record")?,
        u32_at(end + 16).ok_or("truncated end record")?,
    );
    // All ones: the real values are in the Zip64 record instead.
    if count == u16::MAX || directory == u32::MAX {
        return Err("zip64 archives are not read");
    }
    let mut out = BTreeMap::new();
    let mut entry = directory as usize;
    while slice(entry, 4) == Some(b"PK\x01\x02") {
        let head = |o: usize| u16_at(entry + o).ok_or("truncated directory");
        let (method, compressed) = (
            head(10)?,
            u32_at(entry + 20).ok_or("truncated directory")? as usize,
        );
        let name_len = head(28)? as usize;
        let (extra, comment) = (head(30)? as usize, head(32)? as usize);
        let local = u32_at(entry + 42).ok_or("truncated directory")? as usize;
        let name = slice(entry + 46, name_len).ok_or("truncated directory")?;
        // The local header's own name and extra lengths, then the bytes.
        let (local_name, local_extra) = (
            u16_at(local + 26).ok_or("truncated local header")? as usize,
            u16_at(local + 28).ok_or("truncated local header")? as usize,
        );
        let name = std::str::from_utf8(name).map_err(|_| "a file name is not UTF-8")?;
        if !name.ends_with('/') {
            let start = local + 30 + local_name + local_extra;
            out.insert(
                name.replace('\\', "/"),
                Entry {
                    method,
                    at: start,
                    compressed,
                },
            );
        }
        entry += 46 + name_len + extra + comment;
    }
    Ok(out)
}
