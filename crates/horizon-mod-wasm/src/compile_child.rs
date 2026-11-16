//! Internal compiler entry and private input handoff.
use crate::module_cache::{CACHE_DIR, ModuleCache};
use std::{
    ffi::OsString,
    fs::OpenOptions,
    io::{self, Read},
    path::{Path, PathBuf},
};
#[cfg(any(unix, all(test, not(windows))))]
use std::{fs, io::Write};

/// Owns both the spelling and validation of private input names.
struct InputName(u32, u64);
impl InputName {
    const PREFIX: &str = ".input-";
    const SUFFIX: &str = ".tmp";

    fn file_name(&self) -> String {
        format!("{}{}-{}{}", Self::PREFIX, self.0, self.1, Self::SUFFIX)
    }

    fn matches(name: &std::ffi::OsStr) -> bool {
        let Some(name) = name.to_str() else {
            return false;
        };
        let Some(parts) = name
            .strip_prefix(Self::PREFIX)
            .and_then(|s| s.strip_suffix(Self::SUFFIX))
        else {
            return false;
        };
        let Some((pid, sequence)) = parts.split_once('-') else {
            return false;
        };
        let (Ok(pid), Ok(sequence)) = (pid.parse::<u32>(), sequence.parse::<u64>()) else {
            return false;
        };
        Self(pid, sequence).file_name() == name
    }
}

// Production handoff is Unix-only; the non-Windows cache tests also create inputs.
#[cfg(any(unix, all(test, not(windows))))]
pub(crate) struct Input(pub(crate) PathBuf);
#[cfg(any(unix, all(test, not(windows))))]
impl Input {
    pub(crate) fn new(root: &Path, bytes: &[u8]) -> io::Result<Self> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        loop {
            let path = root.join(
                InputName(
                    std::process::id(),
                    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                )
                .file_name(),
            );
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(mut file) => {
                    let input = Self(path);
                    file.write_all(bytes)?;
                    return Ok(input);
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
    }
}
#[cfg(any(unix, all(test, not(windows))))]
impl Drop for Input {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Handles only the hidden compiler invocation, before game initialization.
pub fn compile_child(args: impl IntoIterator<Item = OsString>) -> Option<i32> {
    let mut args = args.into_iter();
    if args.next()?.as_os_str() != "--compile-wasm-module" {
        return None;
    }
    let result = (|| -> wasmtime::Result<()> {
        let input = PathBuf::from(
            args.next()
                .ok_or_else(|| io::Error::other("missing input"))?,
        );
        let key = args
            .next()
            .ok_or_else(|| io::Error::other("missing cache key"))?;
        if args.next().is_some() {
            return Err(io::Error::other("extra arguments").into());
        }
        compile(&input, &key.to_string_lossy())
    })();
    Some(match result {
        Ok(()) => 0,
        Err(why) => {
            eprintln!("wasm compiler: {why}");
            1
        }
    })
}

pub(crate) fn compile(input: &Path, expected_key: &str) -> wasmtime::Result<()> {
    let root = input
        .parent()
        .ok_or_else(|| io::Error::other("missing cache folder"))?;
    if !root.ends_with(Path::new(CACHE_DIR)) || !input.file_name().is_some_and(InputName::matches) {
        return Err(io::Error::other("input is not a private cache input").into());
    }
    let cache = ModuleCache::for_child(root.to_path_buf());
    if cache.disabled.is_some() {
        return Err(io::Error::other("cache folder refused").into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(
            (rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32,
        );
    }
    let mut file = options.open(input)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::other("input is not a regular file").into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != rustix::process::getuid().as_raw()
            || metadata.nlink() != 1
            || metadata.mode() & 0o7777 != 0o600
        {
            return Err(io::Error::other("input is not private").into());
        }
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let engine = crate::runtime::engine()?;
    if expected_key != ModuleCache::key(&engine, &bytes) {
        return Err(io::Error::other("output does not match module cache key").into());
    }
    let module = wasmtime::Module::new(&engine, &bytes)?;
    cache.write(&cache.entry_path(expected_key), &module)
}

/// Waits only on process status; a compiler crash cannot unwind into the game.
#[cfg(unix)]
pub(crate) fn run(
    cache: &ModuleCache,
    bytes: &[u8],
    key: &str,
    command: impl FnOnce(&Path, &str) -> std::process::Command,
    timeout: std::time::Duration,
) -> wasmtime::Result<()> {
    use std::{process::Stdio, time::Instant};
    let input = Input::new(&cache.root, bytes)?;
    let mut command = command(&input.0, key);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command.spawn()?;
    let started = Instant::now();
    let result = loop {
        // Check the deadline before polling: even a completed child must not turn
        // an already-expired budget into success because the parent was descheduled.
        let status = if started.elapsed() >= timeout {
            Ok(None)
        } else {
            child.try_wait()
        };
        match status {
            Ok(Some(status)) => {
                break if status.success() {
                    Ok(())
                } else {
                    Err(io::Error::other(format!("compiler exited {status}")))
                };
            }
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(std::time::Duration::from_millis(10))
            }
            outcome => {
                let _ = child.kill();
                let reaped = child.wait();
                break match (outcome, reaped) {
                    (_, Err(e)) | (Err(e), _) => Err(e),
                    _ => Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "compiler timed out",
                    )),
                };
            }
        }
    };
    // The writer names temporaries by PID. Reap first, then remove only this child's files.
    let prefix = format!(".{}-", child.id());
    for entry in fs::read_dir(&cache.root)?.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(&prefix) && name.ends_with(".tmp") {
            let _ = fs::remove_file(entry.path());
        }
    }
    result?;
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn refuses_linked_nonregular_and_replaced_inputs_without_output() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join(CACHE_DIR);
        fs::create_dir_all(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let bytes = b"\0asm\x01\0\0\0";
        let engine = crate::runtime::engine().unwrap();
        let key = ModuleCache::key(&engine, bytes);
        let input = Input::new(&root, bytes).unwrap();
        let original = root.join("original");
        fs::rename(&input.0, &original).unwrap();
        symlink(&original, &input.0).unwrap();
        assert!(compile(&input.0, &key).is_err());
        fs::remove_file(&input.0).unwrap();
        fs::hard_link(&original, &input.0).unwrap();
        assert!(compile(&input.0, &key).is_err());
        fs::remove_file(&input.0).unwrap();
        // rustix has no FIFO call on Apple targets; mkfifo is in every Unix libc.
        let fifo = std::ffi::CString::new(input.0.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: fifo is a NUL-terminated path that outlives the call.
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        assert!(compile(&input.0, &key).is_err());
        fs::remove_file(&input.0).unwrap();
        let replacement = Input::new(&root, b"different bytes").unwrap();
        fs::rename(&replacement.0, &input.0).unwrap();
        assert!(compile(&input.0, &key).is_err());
        assert!(!root.join(format!("{key}.cwasm")).exists());
    }
}
