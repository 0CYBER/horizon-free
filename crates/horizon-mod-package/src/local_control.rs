//! Private local process control. Keys are never arguments or diagnostics; no network telemetry.
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    io::{self, Read, Write},
    path::PathBuf,
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;

pub(crate) const VERSION: u8 = 1;
const LIMIT: usize = 64 * 1024 * 1024;
/// A private bootstrap marker. Deliberately not Debug: it contains a secret.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Marker {
    #[serde(deserialize_with = "version")]
    version: u8,
    key: [u8; 16],
    #[serde(serialize_with = "path_out", deserialize_with = "path_in")]
    directory: PathBuf,
}
pub(crate) fn version<'de, D: serde::Deserializer<'de>>(input: D) -> Result<u8, D::Error> {
    let version = u8::deserialize(input)?;
    if version != VERSION {
        return Err(serde::de::Error::custom("unknown control version"));
    }
    Ok(version)
}
pub(crate) fn path_out<S: serde::Serializer>(
    path: &std::path::Path,
    output: S,
) -> Result<S::Ok, S::Error> {
    path.as_os_str().serialize(output)
}
pub(crate) fn path_in<'de, D: serde::Deserializer<'de>>(input: D) -> Result<PathBuf, D::Error> {
    let path = PathBuf::from(std::ffi::OsString::deserialize(input)?);
    if !path.is_absolute() {
        return Err(serde::de::Error::custom("control path must be absolute"));
    }
    Ok(path)
}
impl Marker {
    pub fn new(directory: PathBuf) -> io::Result<Self> {
        let mut key = [0; 16];
        getrandom::fill(&mut key).map_err(|_| io::ErrorKind::Other)?;
        if !directory.is_absolute() {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        Ok(Self {
            version: VERSION,
            key,
            directory,
        })
    }
}

#[cfg_attr(unix, path = "local_control/unix.rs")]
#[cfg_attr(windows, path = "local_control/windows.rs")]
mod platform;
pub use platform::{Endpoint, Stream, connect};

pub struct Channel {
    stream: Stream,
    deadline: Instant,
}
impl Channel {
    pub fn authenticate(
        stream: Stream,
        marker: &Marker,
        server: bool,
        deadline: Instant,
    ) -> io::Result<Self> {
        stream.set_nonblocking(true)?;
        let mut channel = Self { stream, deadline };
        if server {
            let mut version = [0];
            channel.read_all(&mut version)?;
            if version[0] != VERSION {
                return Err(io::ErrorKind::PermissionDenied.into());
            }
            let mut key = [0; 16];
            channel.read_all(&mut key)?;
            if !bool::from(key.ct_eq(&marker.key)) {
                return Err(io::ErrorKind::PermissionDenied.into());
            }
        } else {
            channel.write_all(&[VERSION])?;
            channel.write_all(&marker.key)?;
        }
        Ok(channel)
    }
    pub fn send<T: Serialize>(mut self, message: &T) -> io::Result<Self> {
        let bytes = serde_json::to_vec(message).map_err(io::Error::other)?;
        if bytes.len() > LIMIT {
            return Err(io::ErrorKind::InvalidData.into());
        }
        self.write_all(&(bytes.len() as u32).to_le_bytes())?;
        self.write_all(&bytes)?;
        Ok(self)
    }
    pub fn receive<T: DeserializeOwned>(mut self) -> io::Result<(Self, T)> {
        let mut header = [0; 4];
        self.read_all(&mut header)?;
        let length = u32::from_le_bytes(header) as usize;
        if length > LIMIT {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let mut bytes = vec![0; length];
        self.read_all(&mut bytes)?;
        let message = serde_json::from_slice(&bytes)
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
        Ok((self, message))
    }
    fn retry_io(
        &mut self,
        mut operation: impl FnMut(&mut Stream) -> io::Result<usize>,
    ) -> io::Result<usize> {
        loop {
            check_deadline(self.deadline)?;
            match operation(&mut self.stream) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => pause(self.deadline)?,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                result => return result,
            }
        }
    }
    fn read_all(&mut self, mut bytes: &mut [u8]) -> io::Result<()> {
        while !bytes.is_empty() {
            let count = self.retry_io(|stream| stream.read(bytes))?;
            if count == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            bytes = &mut bytes[count..];
        }
        Ok(())
    }
    fn write_all(&mut self, mut bytes: &[u8]) -> io::Result<()> {
        while !bytes.is_empty() {
            let count = self.retry_io(|stream| stream.write(bytes))?;
            if count == 0 {
                return Err(io::ErrorKind::WriteZero.into());
            }
            bytes = &bytes[count..];
        }
        Ok(())
    }
}
#[cfg(test)]
std::thread_local! {
    // Only the calling test's I/O loop sees this clock. The first WouldBlock
    // pause advances it to the deadline instead of waiting on the scheduler.
    static TEST_NOW: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
}
fn check_deadline(deadline: Instant) -> io::Result<()> {
    let now = Instant::now();
    #[cfg(test)]
    let now = TEST_NOW.get().unwrap_or(now);
    if now >= deadline {
        return Err(io::ErrorKind::TimedOut.into());
    }
    Ok(())
}
fn pause(deadline: Instant) -> io::Result<()> {
    check_deadline(deadline)?;
    #[cfg(test)]
    if TEST_NOW.get().is_some() {
        TEST_NOW.set(Some(deadline));
        return Ok(());
    }
    std::thread::sleep(Duration::from_millis(2));
    Ok(())
}
#[cfg(all(test, unix))]
mod tests;
