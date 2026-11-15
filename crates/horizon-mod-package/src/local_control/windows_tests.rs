use super::*;
use crate::local_control::Channel;
use std::ptr::{null, null_mut};
use std::{sync::mpsc, time::Duration};
fn marker() -> (tempfile::TempDir, Marker) {
    let root = tempfile::tempdir().unwrap();
    let marker = Marker::new(root.path().canonicalize().unwrap().join("endpoint")).unwrap();
    (root, marker)
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(3)
}
#[test]
fn private_pipe_exchanges_authenticated_messages_once_without_inheriting_handles() {
    let (_root, marker) = marker();
    let mut endpoint = Endpoint::bind(&marker).unwrap();
    let handle = endpoint.0.as_ref().unwrap().0.as_raw_handle();
    let (mut flags, mut maximum) = (0, 0);
    assert_ne!(
        unsafe { GetNamedPipeInfo(handle, null_mut(), null_mut(), null_mut(), &mut maximum,) },
        0
    );
    assert_eq!(maximum, 1);
    assert_ne!(unsafe { GetHandleInformation(handle, &mut flags) }, 0);
    assert_eq!(flags & HANDLE_FLAG_INHERIT, 0);
    assert!(Endpoint::bind(&marker).is_err());
    let client_marker = marker.clone();
    let client = std::thread::spawn(move || {
        let stream = connect(&client_marker, deadline()).unwrap();
        let mut flags = 0;
        assert_ne!(
            unsafe { GetHandleInformation(stream.0.as_raw_handle(), &mut flags) },
            0
        );
        assert_eq!(flags & HANDLE_FLAG_INHERIT, 0);
        let channel = Channel::authenticate(stream, &client_marker, false, deadline())
            .unwrap()
            .send(&17)
            .unwrap();
        let (_, value) = channel.receive::<String>().unwrap();
        assert_eq!(value, "commit");
    });
    let stream = endpoint.accept(deadline()).unwrap();
    assert!(
        matches!(endpoint.accept(deadline()), Err(error) if error.kind() == io::ErrorKind::NotConnected)
    );
    let channel = Channel::authenticate(stream, &marker, true, deadline()).unwrap();
    let (channel, value) = channel.receive::<i32>().unwrap();
    assert_eq!(value, 17);
    let channel = channel.send(&"commit").unwrap();
    client.join().unwrap();
    drop(channel);
    assert!(Endpoint::bind(&marker).is_ok());
}
#[test]
fn silent_and_wrong_key_peers_are_refused_with_bounded_deadlines() {
    for wrong_key in [false, true] {
        let (_root, marker) = marker();
        let mut endpoint = Endpoint::bind(&marker).unwrap();
        let mut other = marker.clone();
        other.key[15] ^= 1;
        let (stop, stopped) = mpsc::channel();
        let client = std::thread::spawn(move || {
            let mut stream = Some(connect(&other, deadline()).unwrap());
            let _channel = wrong_key.then(|| {
                Channel::authenticate(stream.take().unwrap(), &other, false, deadline()).unwrap()
            });
            let _ = stopped.recv_timeout(Duration::from_secs(3));
        });
        let stream = endpoint.accept(deadline()).unwrap();
        let result = Channel::authenticate(
            stream,
            &marker,
            true,
            Instant::now() + Duration::from_millis(40),
        );
        let expected = if wrong_key {
            io::ErrorKind::PermissionDenied
        } else {
            io::ErrorKind::TimedOut
        };
        assert!(matches!(result, Err(error) if error.kind() == expected));
        stop.send(()).unwrap();
        client.join().unwrap();
    }
}
#[test]
fn descriptor_has_exactly_one_allow_ace_for_the_current_token_user() {
    let descriptor = current_user_acl().unwrap();
    unsafe {
        let (mut present, mut defaulted) = (0, 0);
        let mut acl = null_mut();
        assert_ne!(
            GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut acl, &mut defaulted),
            0
        );
        assert_ne!(present, 0);
        assert!(!acl.is_null());
        let mut size = ACL_SIZE_INFORMATION::default();
        assert_ne!(
            GetAclInformation(
                acl,
                (&mut size as *mut ACL_SIZE_INFORMATION).cast(),
                std::mem::size_of_val(&size) as u32,
                AclSizeInformation
            ),
            0
        );
        assert_eq!(size.AceCount, 1);
        let mut ace = null_mut();
        assert_ne!(GetAce(acl, 0, &mut ace), 0);
        let ace = &*ace.cast::<ACCESS_ALLOWED_ACE>();
        assert_eq!(ace.Header.AceType, 0); // ACCESS_ALLOWED_ACE_TYPE from the Windows ABI.
        assert_eq!(ace.Header.AceFlags, 0);
        assert_eq!(ace.Mask, GENERIC_ALL);
        let mut token = null_mut();
        assert_ne!(
            OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token),
            0
        );
        let token = OwnedHandle::from_raw_handle(token);
        let mut bytes = 0;
        // TOKEN_USER plus the Windows ABI maximum 68-byte SID fits this aligned buffer.
        let mut storage = [0usize; 32];
        assert_ne!(
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                storage.as_mut_ptr().cast(),
                std::mem::size_of_val(&storage) as u32,
                &mut bytes
            ),
            0
        );
        let own = &*storage.as_ptr().cast::<TOKEN_USER>();
        let allowed_sid = (&ace.SidStart as *const u32).cast_mut().cast();
        assert_ne!(EqualSid(allowed_sid, own.User.Sid), 0);
    }
}
fn open_remote(marker: &Marker) -> HANDLE {
    let local = String::from_utf16(&name(marker)[..name(marker).len() - 1]).unwrap();
    let path: Vec<u16> = local
        .replacen(r"\\.\", r"\\localhost\", 1)
        .encode_utf16()
        .chain([0])
        .collect();
    unsafe {
        CreateFileW(
            path.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            null(),
            OPEN_EXISTING,
            0,
            null_mut(),
        )
    }
}
#[test]
fn remote_clients_are_rejected_when_the_control_pipe_accepts_the_same_credentials() {
    let (root, marker) = marker();
    let control = Marker::new(root.path().canonicalize().unwrap().join("remote-control")).unwrap();
    let descriptor = current_user_acl().unwrap();
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    let handle = unsafe {
        CreateNamedPipeW(
            name(&control).as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_NOWAIT,
            1,
            65536,
            65536,
            0,
            &attributes,
        )
    };
    assert_ne!(handle, INVALID_HANDLE_VALUE);
    let server = unsafe { OwnedHandle::from_raw_handle(handle) };
    unsafe {
        ConnectNamedPipe(server.as_raw_handle(), null_mut());
    }
    let allowed = open_remote(&control);
    assert_ne!(
        allowed,
        INVALID_HANDLE_VALUE,
        "SMB loopback control must work before remote rejection is proved: {}",
        io::Error::last_os_error()
    );
    let _allowed = unsafe { OwnedHandle::from_raw_handle(allowed) };
    let endpoint = Endpoint::bind(&marker).unwrap();
    unsafe {
        ConnectNamedPipe(endpoint.0.as_ref().unwrap().0.as_raw_handle(), null_mut());
    }
    let denied = open_remote(&marker);
    assert_eq!(denied, INVALID_HANDLE_VALUE);
    assert_eq!(
        io::Error::last_os_error().raw_os_error(),
        Some(ERROR_ACCESS_DENIED as i32)
    );
}
