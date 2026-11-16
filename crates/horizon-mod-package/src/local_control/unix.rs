use super::{Marker, check_deadline, pause};
use crate::runtime_leases::open_directory;
use cap_fs_ext::DirExt;
use cap_std::fs::Dir;
use std::{
    io,
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    time::Instant,
};
pub type Stream = UnixStream;

pub struct Endpoint {
    parent: Dir,
    path: PathBuf,
    listener: Option<UnixListener>,
}
impl Marker {
    /// Fresh short-lived control address, independent of installed data paths.
    /// The endpoint creates its own private directory; no secret is encoded in its name.
    pub fn runtime() -> io::Result<Self> {
        Self::runtime_at(
            std::env::var_os("XDG_RUNTIME_DIR")
                .as_deref()
                .map(std::path::Path::new),
        )
    }

    fn runtime_at(runtime: Option<&std::path::Path>) -> io::Result<Self> {
        let mut nonce = [0u8; 16];
        getrandom::fill(&mut nonce).map_err(|_| io::ErrorKind::Other)?;
        let name = format!("hz-{:032x}", u128::from_ne_bytes(nonce));
        let suitable = |path: &std::path::Path| -> bool {
            socket2::SockAddr::unix(path.join(&name).join("control")).is_ok()
                && open_directory(path).is_ok_and(|dir| check_directory(&dir, true).is_ok())
                && path.ancestors().skip(1).all(|ancestor| {
                    open_directory(ancestor).is_ok_and(|dir| check_directory(&dir, false).is_ok())
                })
        };
        if let Some(runtime) = runtime.filter(|path| suitable(path)) {
            return Self::new(runtime.join(name));
        }
        // Deliberately not temp_dir(): TMPDIR may be arbitrarily long or unsafe.
        // Endpoint::bind checks /tmp and every ancestor and creates a 0700 leaf
        // with exclusive mkdir, rejecting collisions rather than reusing anything.
        #[cfg(target_os = "macos")]
        let fallback = "/private/tmp"; // /tmp is a symlink on macOS.
        #[cfg(not(target_os = "macos"))]
        let fallback = "/tmp";
        Self::new(std::path::Path::new(fallback).join(name))
    }
}

impl Endpoint {
    pub fn bind(marker: &Marker) -> io::Result<Self> {
        let parent = marker
            .directory
            .parent()
            .ok_or(io::ErrorKind::InvalidInput)?;
        let parent = open_directory(parent).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("open control socket parent {}: {error:?}", parent.display()),
            )
        })?;
        check_directory(&parent, false)?;
        for ancestor in marker.directory.ancestors().skip(2) {
            let directory = open_directory(ancestor).map_err(|_| io::ErrorKind::InvalidInput)?;
            check_directory(&directory, false)?;
        }
        let name = marker
            .directory
            .file_name()
            .ok_or(io::ErrorKind::InvalidInput)?;
        rustix::fs::mkdirat(&parent, name, rustix::fs::Mode::RWXU).map_err(|error| {
            let error = io::Error::from(error);
            io::Error::new(
                error.kind(),
                format!(
                    "create control socket directory {}: {error}",
                    marker.directory.display()
                ),
            )
        })?;
        let mut endpoint = Self {
            parent,
            path: marker.directory.clone(),
            listener: None,
        };
        let directory = endpoint.parent.open_dir_nofollow(name)?;
        check_directory(&directory, true)?;
        let socket_path = endpoint.path.join("control");
        let listener = UnixListener::bind(&socket_path).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("bind control socket {}: {error}", socket_path.display()),
            )
        })?;
        listener.set_nonblocking(true)?;
        endpoint.listener = Some(listener);
        Ok(endpoint)
    }
    /// Consumes the sole listener: no second client can ever be accepted.
    pub fn accept(&mut self, deadline: Instant) -> io::Result<Stream> {
        let listener = self.listener.take().ok_or(io::ErrorKind::NotConnected)?;
        loop {
            check_deadline(deadline)?;
            match listener.accept() {
                Ok((stream, _)) => {
                    peer(&stream)?;
                    // A concurrent fork can keep the listening descriptor open
                    // until exec. Remove its address before returning so even
                    // that inherited listener cannot admit another connection.
                    let directory = self.parent.open_dir_nofollow(
                        self.path
                            .file_name()
                            .expect("bound endpoint has a directory name"),
                    )?;
                    directory.remove_file("control")?;
                    return Ok(stream);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => pause(deadline)?,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
    }
}
pub fn connect(marker: &Marker, deadline: Instant) -> io::Result<Stream> {
    loop {
        check_deadline(deadline)?;
        let directory = match open_directory(&marker.directory) {
            Ok(directory) => directory,
            Err(crate::runtime_leases::Error::Io(error))
                if error.kind() == io::ErrorKind::NotFound =>
            {
                pause(deadline)?;
                continue;
            }
            Err(_) => return Err(io::ErrorKind::PermissionDenied.into()),
        };
        check_directory(&directory, true)?;
        let socket = socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None)?;
        let address = socket2::SockAddr::unix(marker.directory.join("control"))?;
        // AF_UNIX backlog exhaustion can report EAGAIN without a pending connection.
        let connected = socket
            .connect_timeout(&address, deadline.saturating_duration_since(Instant::now()))
            .and_then(|()| socket.peer_addr());
        match connected {
            Ok(_) => {
                let stream = Stream::from(std::os::fd::OwnedFd::from(socket));
                peer(&stream)?;
                return Ok(stream);
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound
                        | io::ErrorKind::ConnectionRefused
                        | io::ErrorKind::NotConnected
                ) =>
            {
                pause(deadline)?
            }
            Err(error) => return Err(error),
        }
    }
}
fn check_directory(directory: &Dir, private: bool) -> io::Result<()> {
    let metadata = rustix::fs::fstat(directory)?;
    if (metadata.st_uid != unsafe { libc::geteuid() } && (private || metadata.st_uid != 0))
        || (metadata.st_mode & 0o022 != 0 && (private || metadata.st_mode & 0o1000 == 0))
        || (private && metadata.st_mode & 0o777 != 0o700)
    {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    Ok(())
}
fn peer(stream: &Stream) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    let uid = rustix::net::sockopt::socket_peercred(stream)?.uid.as_raw();
    #[cfg(target_os = "macos")]
    let uid = {
        use std::os::fd::AsRawFd;
        let (mut uid, mut gid) = (0, 0);
        if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0 {
            return Err(io::Error::last_os_error());
        }
        uid
    };
    #[cfg(test)]
    let uid = TEST_PEER_UID.with(|source| source.get().unwrap_or(uid));
    check_peer_uid(uid)
}
fn check_peer_uid(uid: u32) -> io::Result<()> {
    if uid != unsafe { libc::geteuid() } {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    Ok(())
}
#[cfg(test)]
std::thread_local! {
    // Per-thread override keeps parallel real-credential tests isolated.
    static TEST_PEER_UID: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
}
#[test]
fn foreign_peer_uid_is_refused_on_connect_and_accept() {
    for server in [false, true] {
        let (_root, marker) = super::tests::marker();
        let mut endpoint = Endpoint::bind(&marker).unwrap();
        let deadline = Instant::now() + std::time::Duration::from_secs(3);
        let _client = server.then(|| connect(&marker, deadline).unwrap());
        TEST_PEER_UID.set(Some(unsafe { libc::geteuid() } ^ 1));
        let result = if server {
            endpoint.accept(deadline)
        } else {
            connect(&marker, deadline)
        };
        TEST_PEER_UID.set(None);
        assert!(matches!(result, Err(error) if error.kind() == io::ErrorKind::PermissionDenied));
    }
}
#[test]
fn consumed_listener_refuses_connections_even_while_an_inherited_copy_is_open() {
    let (_root, marker) = super::tests::marker();
    let mut endpoint = Endpoint::bind(&marker).unwrap();
    // A concurrent fork retains this reference until exec, even with CLOEXEC.
    // Duplicate it explicitly so the regression never depends on scheduling.
    let inherited = endpoint.listener.as_ref().unwrap().try_clone().unwrap();
    let deadline = Instant::now() + std::time::Duration::from_secs(3);
    let _client = connect(&marker, deadline).unwrap();
    let _accepted = endpoint.accept(deadline).unwrap();
    assert!(
        UnixStream::connect(marker.directory.join("control")).is_err(),
        "a consumed endpoint still accepts connections through an inherited listener"
    );
    drop(inherited);
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        self.listener.take();
        if let Some(name) = self.path.file_name() {
            if let Ok(directory) = self.parent.open_dir_nofollow(name) {
                let _ = directory.remove_file("control");
            }
            let _ = self.parent.remove_dir(name);
        }
    }
}

#[cfg(test)]
#[test]
fn runtime_addresses_validate_candidates_and_exchange() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    #[cfg(target_os = "macos")]
    let fallback = "/private/tmp";
    #[cfg(not(target_os = "macos"))]
    let fallback = "/tmp";
    let root = tempfile::tempdir_in(fallback).unwrap();
    let safe = root.path().canonicalize().unwrap().join("runtime");
    std::fs::create_dir(&safe).unwrap();
    std::fs::set_permissions(&safe, std::fs::Permissions::from_mode(0o700)).unwrap();
    let first = Marker::runtime_at(Some(&safe)).unwrap();
    let second = Marker::runtime_at(Some(&safe)).unwrap();
    assert_ne!(first.directory, second.directory);
    assert_eq!(first.directory.parent(), Some(safe.as_path()));
    let second_endpoint = Endpoint::bind(&second).unwrap();
    let mut endpoint = Endpoint::bind(&first).unwrap();
    let deadline = Instant::now() + std::time::Duration::from_secs(3);
    let client = connect(&first, deadline).unwrap();
    let server = endpoint.accept(deadline).unwrap();
    let client = super::Channel::authenticate(client, &first, false, deadline).unwrap();
    let server = super::Channel::authenticate(server, &first, true, deadline).unwrap();
    client.send(&42u32).unwrap();
    assert_eq!(server.receive::<u32>().unwrap().1, 42);
    let link = root.path().join("link");
    symlink(&safe, &link).unwrap();
    let long = safe.join("long-home-".repeat(18));
    std::fs::create_dir(&long).unwrap();
    std::fs::set_permissions(&long, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(socket2::SockAddr::unix(long.join("control")).is_err());
    let old_marker = Marker::new(long.join("holder-control-123-456")).unwrap();
    assert!(Endpoint::bind(&old_marker).is_err());
    assert!(!old_marker.directory.exists());
    drop(second_endpoint);
    std::fs::set_permissions(&safe, std::fs::Permissions::from_mode(0o777)).unwrap();
    for candidate in [
        Some(safe.as_path()),
        Some(link.as_path()),
        Some(long.as_path()),
        None,
    ] {
        let marker = Marker::runtime_at(candidate).unwrap();
        assert_ne!(marker.directory.parent(), candidate);
        let endpoint = Endpoint::bind(&marker).unwrap();
        drop(endpoint);
        assert!(!marker.directory.exists());
    }
}
