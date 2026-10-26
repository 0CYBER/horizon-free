//! Independent copies, using filesystem cloning when available. Never hard links: exporters
//! rewrite prepared resources in place. Checked imports publish only after verifying the copy.

use crate::{digest, hex};
use sha2::Digest;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

fn clone_file(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        let source = fs::File::open(from)?;
        let target = fs::File::create(to)?;
        // SAFETY: FICLONE takes a source fd, both descriptors stay open for the call.
        if unsafe { libc::ioctl(target.as_raw_fd(), libc::FICLONE, source.as_raw_fd()) } == 0 {
            return Ok(());
        }
        Err(io::Error::last_os_error())
    }
    #[cfg(target_os = "macos")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let path = |p: &Path| {
            CString::new(p.as_os_str().as_bytes())
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))
        };
        let (source, target) = (path(from)?, path(to)?);
        // SAFETY: both paths are valid NUL-terminated strings for the duration of the call.
        if unsafe { libc::clonefile(source.as_ptr(), target.as_ptr(), 0) } == 0 {
            return Ok(());
        }
        Err(io::Error::last_os_error())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (from, to);
        Err(io::ErrorKind::Unsupported.into())
    }
}

/// Copies an independent file, attempting a reflink first. The destination may be replaced.
pub fn clone_or_copy(from: &Path, to: &Path) -> io::Result<()> {
    clone_file(from, to).or_else(|_| fs::copy(from, to).map(|_| ()))
}

fn copy_hash<D: Digest>(from: &Path, to: &Path) -> io::Result<String> {
    let mut source = fs::File::open(from)?;
    let mut target = fs::File::create(to)?;
    let mut hash = D::new();
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let n = source.read(&mut buffer)?;
        if n == 0 {
            return Ok(hex(&hash.finalize()));
        }
        target.write_all(&buffer[..n])?;
        hash.update(&buffer[..n]);
    }
}

/// A verified replacement staged beside its destination. Dropping an unfinished set cleans
/// every temporary file, so a corrupt late panorama face cannot replace earlier placeholders.
pub(crate) struct Import {
    temporary: Option<PathBuf>,
    destination: PathBuf,
}

impl Import {
    pub(crate) fn stage(
        source: &Path,
        destination: &Path,
        checksum: &str,
        algorithm: &str,
        verified: bool,
    ) -> Result<Self, String> {
        let mut import = Self {
            temporary: None,
            destination: destination.into(),
        };
        if verified || (destination.is_file() && digest(destination, algorithm)? == checksum) {
            return Ok(import);
        }
        fs::create_dir_all(destination.parent().unwrap()).map_err(|e| e.to_string())?;
        static COPIES: AtomicUsize = AtomicUsize::new(0);
        let temporary = destination.with_file_name(format!(
            ".{}.partial-{}-{}",
            destination.file_name().unwrap().to_string_lossy(),
            std::process::id(),
            COPIES.fetch_add(1, Ordering::Relaxed)
        ));
        import.temporary = Some(temporary.clone());
        let actual = if clone_file(source, &temporary).is_ok() {
            digest(&temporary, algorithm)?
        } else {
            match algorithm {
                "sha1" => copy_hash::<sha1::Sha1>(source, &temporary),
                _ => copy_hash::<sha2::Sha256>(source, &temporary),
            }
            .map_err(|e| format!("{}: {e}", source.display()))?
        };
        if actual != checksum {
            return Err(format!(
                "{} is not the pinned file: expected {algorithm} {checksum}, got {actual}",
                source.display()
            ));
        }
        Ok(import)
    }

    pub(crate) fn commit(mut self) -> Result<PathBuf, String> {
        if let Some(temporary) = &self.temporary {
            fs::rename(temporary, &self.destination)
                .map_err(|e| format!("{}: {e}", self.destination.display()))?;
        }
        self.temporary = None;
        Ok(self.destination.clone())
    }
}

impl Drop for Import {
    fn drop(&mut self) {
        if let Some(temporary) = &self.temporary {
            let _ = fs::remove_file(temporary);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_are_independent_and_checked_before_publication() {
        let dir = std::env::temp_dir().join(format!("horizon-file-copy-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source");
        let target = dir.join("target");
        let data = vec![42; 200_000]; // More than one streaming buffer.
        fs::write(&source, &data).unwrap();
        for algorithm in ["sha1", "sha256"] {
            let checksum = digest(&source, algorithm).unwrap();
            Import::stage(&source, &target, &checksum, algorithm, false)
                .unwrap()
                .commit()
                .unwrap();
            assert_eq!(digest(&target, algorithm).unwrap(), checksum);
            fs::write(&target, b"old destination").unwrap();
            assert_eq!(fs::read(&source).unwrap(), data);
            assert!(Import::stage(&source, &target, "bad hash", algorithm, false).is_err());
            assert_eq!(fs::read(&target).unwrap(), b"old destination");
            // Explicitly exercise ordinary streaming copies even on a reflink filesystem.
            let streamed = if algorithm == "sha1" {
                copy_hash::<sha1::Sha1>(&source, &target).unwrap()
            } else {
                copy_hash::<sha2::Sha256>(&source, &target).unwrap()
            };
            assert_eq!(streamed, checksum);
            assert_eq!(fs::read(&target).unwrap(), data);
        }
        clone_or_copy(&source, &target).unwrap();
        fs::write(&target, b"rewrite").unwrap();
        assert_eq!(fs::read(&source).unwrap(), data);
        let staged = Import::stage(
            &source,
            &target,
            &digest(&source, "sha1").unwrap(),
            "sha1",
            false,
        )
        .unwrap();
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 3);
        drop(staged);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
        fs::remove_dir_all(dir).unwrap();
    }
}
