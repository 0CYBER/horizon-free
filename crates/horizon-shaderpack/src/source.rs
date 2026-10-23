//! A Java shader pack's files: a folder or a zip holding a `shaders/` folder,
//! found the way Iris finds it (`Iris.loadExternalZipShaderpack`).

use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

/// Program source extensions Iris loads (vertex, fragment, geometry,
/// compute, tessellation control and evaluation).
pub const PROGRAM_EXTENSIONS: &[&str] = &["vsh", "fsh", "gsh", "csh", "tcs", "tes"];

/// The files under a pack's `shaders/` folder, addressed by `/`-separated
/// paths relative to it (`lib/common.glsl`).
pub enum PackFiles {
    Folder(PathBuf),
    /// A zip is small (a few MB of text), so it is read once.
    Zip(BTreeMap<String, Vec<u8>>),
}

impl PackFiles {
    /// Open `path`, a pack folder (with `shaders/` in it) or a `.zip`.
    pub fn open(path: &Path) -> Result<Self, String> {
        if path.is_dir() {
            let shaders = path.join("shaders");
            return if shaders.is_dir() {
                Ok(Self::Folder(shaders))
            } else {
                Err("no shaders folder".into())
            };
        }
        let file = fs::File::open(path).map_err(|e| e.to_string())?;
        let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("not a zip: {e}"))?;
        let names: Vec<String> = zip.file_names().map(str::to_owned).collect();
        // `shaders/` at the root, else the first folder ending in `shaders`
        // (`Pack-main/shaders/`), as Iris walks the zip.
        let mut prefixes: Vec<&str> = names
            .iter()
            .filter_map(|n| {
                let at = n.find("shaders/")?;
                (at == 0 || n.as_bytes()[at - 1] == b'/').then(|| &n[..at + "shaders/".len()])
            })
            .collect();
        prefixes.sort_by_key(|p| (p.len(), *p));
        let prefix = prefixes.first().ok_or("no shaders folder")?.to_string();
        let mut files = BTreeMap::new();
        for name in &names {
            let Some(rel) = name.strip_prefix(&prefix) else {
                continue;
            };
            if rel.is_empty() || rel.ends_with('/') {
                continue;
            }
            let mut entry = zip.by_name(name).map_err(|e| e.to_string())?;
            let mut bytes = Vec::new();
            entry
                .read_to_end(&mut bytes)
                .map_err(|e| format!("{name}: {e}"))?;
            files.insert(rel.to_owned(), bytes);
        }
        Ok(Self::Zip(files))
    }

    /// Raw bytes of a file relative to `shaders/` (leading `/` or `./` stripped),
    /// or `None` when it doesn't exist.
    pub fn read_bytes(&self, rel: &str) -> Option<Vec<u8>> {
        let clean = rel
            .trim()
            .trim_start_matches("./")
            .trim_start_matches('/')
            .trim_start_matches("shaders/");
        if clean.is_empty() || clean.contains("..") {
            return None;
        }
        match self {
            Self::Folder(root) => fs::read(root.join(clean)).ok(),
            Self::Zip(files) => files.get(clean).cloned(),
        }
    }

    /// A text file, or `None` when it doesn't exist. Invalid UTF-8 is
    /// replaced, as packs are often saved in Latin-1.
    pub fn read(&self, rel: &str) -> Option<String> {
        let bytes = self.read_bytes(rel)?;
        Some(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// The files directly in folder `dir` (`""` for `shaders/` itself), sorted.
    pub fn list(&self, dir: &str) -> Vec<String> {
        let prefix = if dir.is_empty() {
            String::new()
        } else {
            format!("{dir}/")
        };
        let mut out: Vec<String> = match self {
            Self::Folder(root) => fs::read_dir(root.join(dir))
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                .filter_map(|e| Some(format!("{prefix}{}", e.file_name().to_str()?)))
                .collect(),
            Self::Zip(files) => files
                .keys()
                .filter(|k| {
                    k.strip_prefix(&prefix)
                        .is_some_and(|rest| !rest.contains('/'))
                })
                .cloned()
                .collect(),
        };
        out.sort();
        out
    }

    /// The program folders: `shaders/` itself and the per-dimension folders,
    /// from `dimension.properties` (`dimension.world-1=minecraft:the_nether`)
    /// or, without one, Iris's defaults `world0`, `world-1` and `world1`.
    pub fn program_dirs(&self) -> Vec<String> {
        let mut dirs = vec![String::new()];
        match self.read("dimension.properties") {
            Some(text) => {
                for line in text.lines() {
                    if let Some((key, _)) = line.trim().split_once('=') {
                        if let Some(dir) = key.trim().strip_prefix("dimension.") {
                            dirs.push(dir.to_owned());
                        }
                    }
                }
            }
            None => dirs.extend(["world0", "world-1", "world1"].map(String::from)),
        }
        dirs.dedup();
        dirs
    }

    /// Every program source file of the pack (`composite1.fsh`,
    /// `world-1/gbuffers_water.vsh`), sorted.
    pub fn programs(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .program_dirs()
            .iter()
            .flat_map(|d| self.list(d))
            .filter(|f| {
                f.rsplit_once('.')
                    .is_some_and(|(_, ext)| PROGRAM_EXTENSIONS.contains(&ext))
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }
}
