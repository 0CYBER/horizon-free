use super::*;
use std::os::{fd::AsRawFd, unix::ffi::OsStrExt};

pub(super) fn marker() -> (tempfile::TempDir, Marker) {
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().canonicalize().unwrap();
    let directory = parent.join(std::ffi::OsStr::from_bytes(b"fresh-\xff"));
    let marker = Marker::new(directory).unwrap();
    (root, marker)
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(3)
}
#[derive(Debug, Serialize, serde::Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
enum Message {
    Acquired,
    Commit,
    Status(i32),
}

#[test]
fn authenticated_typed_exchange_consumes_the_single_listener() {
    let (_root, marker) = marker();
    let mut endpoint = Endpoint::bind(&marker).unwrap();
    let other = marker.clone();
    let client = std::thread::spawn(move || {
        let stream = connect(&other, deadline()).unwrap();
        let channel = Channel::authenticate(stream, &other, false, deadline()).unwrap();
        let (channel, message) = channel.receive::<Message>().unwrap();
        assert_eq!(message, Message::Acquired);
        let channel = channel.send(&Message::Commit).unwrap();
        let (_channel, message) = channel.receive::<Message>().unwrap();
        assert_eq!(message, Message::Status(37));
    });
    let stream = endpoint.accept(deadline()).unwrap();
    assert_eq!(
        endpoint.accept(deadline()).err().unwrap().kind(),
        io::ErrorKind::NotConnected
    );
    let channel = Channel::authenticate(stream, &marker, true, deadline()).unwrap();
    let channel = channel.send(&Message::Acquired).unwrap();
    let (channel, message) = channel.receive::<Message>().unwrap();
    assert_eq!(message, Message::Commit);
    let _channel = channel.send(&Message::Status(37)).unwrap();
    client.join().unwrap();
}

#[test]
fn wrong_key_version_oversize_and_unknown_messages_close_the_connection() {
    for case in ["key", "key_tail", "version", "size", "unknown", "eof"] {
        let (_root, marker) = marker();
        let mut endpoint = Endpoint::bind(&marker).unwrap();
        let mut stream = connect(&marker, deadline()).unwrap();
        let mut hello = vec![if case == "version" { 9 } else { VERSION }];
        hello.extend_from_slice(&marker.key);
        if matches!(case, "key" | "key_tail") {
            hello[if case == "key" { 1 } else { 16 }] ^= 1;
        }
        stream.write_all(&hello).unwrap();
        if case == "size" {
            stream
                .write_all(&((LIMIT + 1) as u32).to_le_bytes())
                .unwrap();
        }
        if case == "unknown" {
            let bytes = b"\"Unknown\"";
            stream
                .write_all(&(bytes.len() as u32).to_le_bytes())
                .unwrap();
            stream.write_all(bytes).unwrap();
        }
        stream.shutdown(std::net::Shutdown::Write).unwrap();
        let stream = endpoint.accept(deadline()).unwrap();
        let result = Channel::authenticate(stream, &marker, true, deadline());
        if matches!(case, "key" | "key_tail" | "version") {
            assert!(
                matches!(result, Err(error) if error.kind() == io::ErrorKind::PermissionDenied)
            );
        } else {
            let error = result.unwrap().receive::<Message>().err().unwrap();
            let expected = if case == "eof" {
                io::ErrorKind::UnexpectedEof
            } else {
                io::ErrorKind::InvalidData
            };
            assert_eq!(error.kind(), expected, "{case}");
        }
    }
}

#[test]
fn first_instance_is_exclusive_and_drop_removes_its_endpoint() {
    let (_root, marker) = marker();
    let endpoint = Endpoint::bind(&marker).unwrap();
    assert!(
        Endpoint::bind(&marker).is_err(),
        "reused a preexisting endpoint"
    );
    drop(endpoint);
    assert!(!marker.directory.exists());
    assert!(Endpoint::bind(&marker).is_ok());
}

#[test]
fn quiet_peer_times_out_and_a_second_connection_is_refused() {
    let (_root, marker) = marker();
    let mut endpoint = Endpoint::bind(&marker).unwrap();
    let client = connect(&marker, deadline()).unwrap();
    let (finished, wait) = std::sync::mpsc::channel();
    let closer = std::thread::spawn(move || {
        // Remain quiet until authentication returns. This watchdog only bounds
        // a broken timeout implementation; it is not the normal peer lifetime.
        let completed = wait.recv_timeout(Duration::from_secs(10)).is_ok();
        drop(client);
        completed
    });
    let stream = endpoint.accept(deadline()).unwrap();
    assert_ne!(
        unsafe { libc::fcntl(stream.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC,
        0
    );
    assert!(std::os::unix::net::UnixStream::connect(marker.directory.join("control")).is_err());
    // Read the quiet socket once, then advance the injected clock to the
    // deadline on WouldBlock. No real sleep or scheduler progress is needed.
    let now = Instant::now();
    let expiry = now + Duration::from_secs(1);
    TEST_NOW.set(Some(now));
    let result = Channel::authenticate(stream, &marker, true, expiry);
    let observed_now = TEST_NOW.replace(None);
    let _ = finished.send(());
    assert!(closer.join().unwrap(), "authentication exceeded watchdog");
    assert_eq!(
        observed_now,
        Some(expiry),
        "quiet read never reached WouldBlock"
    );
    assert_eq!(result.err().unwrap().kind(), io::ErrorKind::TimedOut);
}

#[test]
fn unsafe_parents_are_refused() {
    use std::{
        fs,
        os::unix::fs::{PermissionsExt, symlink},
    };
    let (_root, marker) = marker();
    let parent = marker.directory.parent().unwrap();
    symlink(parent, parent.join("link")).unwrap();
    let redirected = Marker::new(parent.join("link/fresh")).unwrap();
    assert!(Endpoint::bind(&redirected).is_err());
    fs::create_dir(parent.join("safe")).unwrap();
    let nested = Marker::new(parent.join("safe/fresh")).unwrap();
    fs::set_permissions(parent, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(Endpoint::bind(&marker).is_err());
    assert!(Endpoint::bind(&nested).is_err());
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn full_backlog_cannot_block_connect_past_its_deadline() {
    use std::{os::unix::fs::DirBuilderExt, sync::mpsc};
    let (_root, marker) = marker();
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&marker.directory)
        .unwrap();
    let listener =
        std::os::unix::net::UnixListener::bind(marker.directory.join("control")).unwrap();
    // SAFETY: live listener, lowering the backlog exposes exhaustion with one queued client.
    assert_eq!(unsafe { libc::listen(listener.as_raw_fd(), 0) }, 0);
    let _queued = connect(&marker, deadline()).unwrap();
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        let result = connect(&marker, Instant::now() + Duration::from_millis(20)).map(drop);
        let _ = send.send(result);
    });
    let result = receive.recv_timeout(Duration::from_secs(1));
    assert!(
        matches!(result, Ok(Err(_))),
        "connect blocked behind the full backlog: {result:?}"
    );
}

#[test]
fn marker_rejects_unknown_versions() {
    let (_root, marker) = marker();
    let mut invalid = serde_json::to_value(&marker).unwrap();
    invalid["version"] = 17.into();
    assert!(serde_json::from_value::<Marker>(invalid).is_err());
}

#[test]
fn unsupported_version_is_refused_without_waiting_for_key_bytes() {
    let (_root, marker) = marker();
    let (stream, mut silent) = std::os::unix::net::UnixStream::pair().unwrap();
    silent.write_all(&[9]).unwrap();
    let result = Channel::authenticate(stream, &marker, true, deadline());
    assert!(matches!(result, Err(error) if error.kind() == io::ErrorKind::PermissionDenied));
}

#[test]
fn public_endpoint_directory_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let (_root, marker) = marker();
    std::fs::create_dir(&marker.directory).unwrap();
    std::fs::set_permissions(&marker.directory, std::fs::Permissions::from_mode(0o755)).unwrap();
    let _listener =
        std::os::unix::net::UnixListener::bind(marker.directory.join("control")).unwrap();
    assert!(
        matches!(connect(&marker, deadline()), Err(error) if error.kind() == io::ErrorKind::PermissionDenied)
    );
}

#[test]
fn stale_non_socket_and_long_path_are_safe_errors() {
    let (_root, marker) = marker();
    std::fs::create_dir(&marker.directory).unwrap();
    let sentinel = marker.directory.join("control");
    std::fs::write(&sentinel, b"retain").unwrap();
    assert!(Endpoint::bind(&marker).is_err());
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"retain");
    let long = Marker::new(marker.directory.parent().unwrap().join("x".repeat(180))).unwrap();
    assert!(Endpoint::bind(&long).is_err());
    assert!(!long.directory.exists());
}
