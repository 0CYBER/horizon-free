//! Game-owned raw stdout/stderr capture: eight fixed 16 KiB frames per stream, drop newest on overflow.
//! Dropped-byte notices share the rotating file cap. Unix files are created/tightened to 0600.
//! The caller supplies a private launch directory; attached CLI streams must not install capture.
use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, File, OpenOptions};
use std::io::{self, IsTerminal, Read, Write};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
    mpsc::{self, Receiver},
};

mod stdio;
pub(crate) const LIMIT: u64 = 16 * 1024 * 1024;

struct Rotating {
    directory: Dir,
    name: &'static str,
    previous: String,
    file: File,
    written: u64,
}
impl Rotating {
    fn open(directory: Dir, name: &'static str) -> io::Result<Self> {
        let previous = format!("{name}.1");
        match Self::append(&directory, &previous, false) {
            Ok(file) if file.metadata()?.len() > LIMIT => {
                return Err(io::ErrorKind::InvalidData.into());
            }
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
        let file = Self::append(&directory, name, true)?;
        let written = file.metadata()?.len();
        if written > LIMIT {
            return Err(io::ErrorKind::InvalidData.into());
        }
        Ok(Self {
            directory,
            name,
            previous,
            file,
            written,
        })
    }
    fn append(directory: &Dir, name: &str, create: bool) -> io::Result<File> {
        let mut options = OpenOptions::new();
        options
            .append(true)
            .create(create)
            .follow(FollowSymlinks::No);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NONBLOCK);
        }
        let file = directory.open_with(name, &options)?;
        if !file.metadata()?.is_file() {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            if unsafe { libc::fchmod(file.as_raw_fd(), 0o600) } != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(file)
    }
}
impl Write for Rotating {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.written == LIMIT {
            match self.directory.remove_file(&self.previous) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
                _ => {}
            }
            self.directory
                .rename(self.name, &self.directory, &self.previous)?;
            self.file = Self::append(&self.directory, self.name, true)?;
            self.written = 0;
        }
        let count = bytes.len().min((LIMIT - self.written) as usize);
        let written = self.file.write(&bytes[..count])?;
        self.written += written as u64;
        Ok(written)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

fn notice(sink: &mut impl Write, dropped: &AtomicU64) {
    let count = dropped.swap(0, Ordering::Relaxed);
    if count != 0 && writeln!(sink, "\n[horizon.output.dropped_bytes={count}]").is_err() {
        dropped.fetch_add(count, Ordering::Relaxed);
    }
}

fn drain(
    mut reader: impl Read + Send + 'static,
    mut sink: impl Write + Send + 'static,
    name: &str,
) -> io::Result<Receiver<()>> {
    let (frames, pending) = mpsc::sync_channel::<([u8; 16 * 1024], usize)>(8);
    let dropped = Arc::new(AtomicU64::new(0));
    let losses = dropped.clone();
    let (done, completed) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name(format!("{name}-disk"))
        .spawn(move || {
            for (bytes, count) in pending {
                notice(&mut sink, &losses);
                let mut offset = 0;
                while offset < count {
                    match sink.write(&bytes[offset..count]) {
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                        Ok(0) | Err(_) => {
                            losses.fetch_add((count - offset) as u64, Ordering::Relaxed);
                            break;
                        }
                        Ok(n) => offset += n,
                    }
                }
            }
            notice(&mut sink, &losses);
            let _ = sink.flush();
            let _ = done.send(());
        })?;
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let mut buffer = [0; 16 * 1024];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        if frames.try_send((buffer, n)).is_err() {
                            dropped.fetch_add(n as u64, Ordering::Relaxed);
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        })?;
    Ok(completed)
}

struct Stream {
    original: Option<stdio::Restore>,
    done: Receiver<()>,
}
impl Drop for Stream {
    fn drop(&mut self) {
        drop(self.original.take());
        // A descendant can retain stdout: normal exit must not wait for it forever.
        let _ = self.done.recv_timeout(std::time::Duration::from_secs(1));
    }
}

/// Keep alive for the game run. Dropping restores streams and drains pending bytes with a deadline.
/// Install once for detached entry before workers/mods; the caller proves the directory's data-root ownership.
pub struct Capture(Vec<Stream>);
impl Capture {
    /// Leave attached terminal streams inherited; redirect other streams to in-game readers.
    pub fn install(directory: &Dir) -> io::Result<Self> {
        let mut capture = Self(Vec::new());
        for (fd, name, tty) in [
            (1, "stdout.log", io::stdout().is_terminal()),
            (2, "stderr.log", io::stderr().is_terminal()),
        ] {
            if tty {
                continue;
            }
            let sink = Rotating::open(directory.try_clone()?, name)?;
            let (reader, writer) = os_pipe::pipe()?;
            let completed = drain(reader, sink, name)?;
            let original = stdio::redirect(fd, &writer)?;
            capture.0.push(Stream {
                original: Some(original),
                done: completed,
            });
        }
        Ok(capture)
    }
}
#[cfg(test)]
mod tests;

impl Drop for Capture {
    fn drop(&mut self) {
        let _ = io::stdout().flush();
        let _ = io::stderr().flush();
        // SAFETY: flush C streams before restoring their descriptors; readers are still alive.
        unsafe {
            libc::fflush(std::ptr::null_mut());
        }
    }
}
