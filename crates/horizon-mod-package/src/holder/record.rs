//! Immutable bounded bootstrap data; the separate registry claim, never this file, owns liveness.
use super::*;
use crate::selection::ModSelectionContext;
use std::io::{Read, Write};
use subtle::ConstantTimeEq;
const LIMIT: u64 = 64 * 1024;
mod gc;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Reference {
    #[serde(deserialize_with = "crate::local_control::version")]
    version: u8,
    tree: Tree,
    key: [u8; 16],
    file: [u64; 2],
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    #[serde(deserialize_with = "crate::local_control::version")]
    version: u8,
    key: [u8; 16],
    entry: Entry,
    roots: [[u64; 2]; 2],
}
fn roots(context: &ModSelectionContext) -> io::Result<[[u64; 2]; 2]> {
    let root = |path: &Path| identity(&open_directory(path).map_err(lease_error)?);
    Ok([root(&context.game_dir)?, root(&context.home)?])
}
impl Reference {
    pub(super) fn publish(
        snapshot: &Snapshot,
        entry: &Entry,
        context: &ModSelectionContext,
    ) -> io::Result<(Self, File)> {
        let mut key = [0; 16];
        getrandom::fill(&mut key).map_err(io::Error::other)?;
        let record = Record {
            version: crate::local_control::VERSION,
            key,
            entry: entry.clone(),
            roots: roots(context)?,
        };
        if record
            .roots
            .iter()
            .any(|id| !entry.trees.iter().any(|tree| tree.identity == *id))
        {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let bytes = serde_json::to_vec(&record).map_err(io::Error::other)?;
        if bytes.len() as u64 > LIMIT {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No);
        #[cfg(unix)]
        cap_std::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = File::from_std(crate::runtime_leases::retained(
            snapshot
                .directory
                .open_with("launch.json", &options)?
                .into_std(),
        )?);
        file.write_all(&bytes)?;
        let reference = Self {
            version: crate::local_control::VERSION,
            tree: snapshot.tree.clone(),
            key,
            file: token(&file)?,
        };
        Ok((reference, file))
    }
    pub(super) fn encode(&self) -> io::Result<String> {
        let encoded = serde_json::to_string(self).map_err(io::Error::other)?;
        if encoded.encode_utf16().count() > 4096 {
            return Err(io::ErrorKind::InvalidData.into());
        }
        Ok(encoded)
    }
    pub(super) fn decode(encoded: &str) -> io::Result<Self> {
        if encoded.len() > 4096 {
            return Err(io::ErrorKind::InvalidData.into());
        }
        serde_json::from_str(encoded).map_err(io::Error::other)
    }
    pub(super) fn acquire(&self, context: &ModSelectionContext) -> io::Result<Acquired> {
        let snapshot = self.tree.snapshot()?;
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let file = File::from_std(crate::runtime_leases::retained(
            snapshot
                .directory
                .open_with("launch.json", &options)?
                .into_std(),
        )?);
        if !file.metadata()?.is_file() || token(&file)? != self.file {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let mut bytes = Vec::new();
        (&file).take(LIMIT + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > LIMIT {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let record: Record = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        if !bool::from(self.key.ct_eq(&record.key))
            || !record
                .entry
                .trees
                .iter()
                .any(|tree| tree.identity == self.tree.identity)
        {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        let mut acquired = record.entry.acquire()?;
        if roots(context)? != record.roots {
            return Err(io::ErrorKind::InvalidData.into());
        }
        acquired._record = Some(file);
        acquired._capture = Some(snapshot.directory);
        Ok(acquired)
    }
}
