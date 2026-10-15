//! End-to-end protection between a host and a guest, whatever carries the
//! bytes. SPAKE2 turns the code's short secret into a session key without
//! giving anyone on the path (the relay included) a way to test guesses
//! offline; every stream of the session then derives its own
//! ChaCha20-Poly1305 keys from it and two fresh randoms, and starts with a
//! key confirmation so a wrong code or a changed byte ends it.
use chacha20poly1305::{
    ChaCha20Poly1305, KeyInit,
    aead::{Aead, Payload},
};
use hkdf::Hkdf;
use sha2::Sha256;
use spake2::{Ed25519Group, Identity, Password, Spake2};
use std::io::{self, Read, Write};

/// The most game bytes in one frame.
pub const MAX_PAYLOAD: usize = 16 * 1024;
const TAG: usize = 16;
const CONFIRM: &[u8] = b"horizon-friends v1 confirm";

const DATA: u8 = 0;
const CONTROL: u8 = 1;
const CONFIRMATION: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Host,
    Guest,
}

/// The session key both ends derive from the code.
#[derive(Clone)]
pub struct Key([u8; 32]);

/// The error a mismatched code produces: the peer's confirmation does not
/// open with our key.
pub fn wrong_code() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "the code doesn't match")
}
pub fn is_wrong_code(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::PermissionDenied
}

/// SPAKE2 over a stream: both ends send their message, then read the
/// other's. The identity binds the key to this nameplate and session.
pub fn exchange<S: Read + Write>(
    stream: &mut S,
    secret: &str,
    nameplate: &str,
    session: u64,
) -> io::Result<Key> {
    let identity = format!("horizon-friends v1/{nameplate}/{session}");
    let (state, outbound) = Spake2::<Ed25519Group>::start_symmetric(
        &Password::new(secret.as_bytes()),
        &Identity::new(identity.as_bytes()),
    );
    let len = u8::try_from(outbound.len()).map_err(io::Error::other)?;
    let mut message = vec![len];
    message.extend_from_slice(&outbound);
    stream.write_all(&message)?;
    stream.flush()?;
    let mut len = [0u8; 1];
    stream.read_exact(&mut len)?;
    let mut inbound = vec![0u8; usize::from(len[0])];
    stream.read_exact(&mut inbound)?;
    let key = state
        .finish(&inbound)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{e:?}")))?;
    let mut out = [0u8; 32];
    let hk = Hkdf::<Sha256>::new(None, &key);
    hk.expand(b"horizon-friends v1 session", &mut out)
        .expect("32 bytes is a valid HKDF output");
    Ok(Key(out))
}

fn nonce(counter: u64) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[4..].copy_from_slice(&counter.to_be_bytes());
    n
}

fn cipher(key: &Key, salt: &[u8], info: &[u8]) -> ChaCha20Poly1305 {
    let mut okm = [0u8; 32];
    Hkdf::<Sha256>::new(Some(salt), &key.0)
        .expand(info, &mut okm)
        .expect("32 bytes is a valid HKDF output");
    ChaCha20Poly1305::new(&okm.into())
}

pub enum Frame {
    Data(Vec<u8>),
    Control(Vec<u8>),
}

pub struct SecureWriter<W> {
    inner: W,
    cipher: ChaCha20Poly1305,
    counter: u64,
}
impl<W: Write> SecureWriter<W> {
    fn frame(&mut self, kind: u8, payload: &[u8]) -> io::Result<()> {
        debug_assert!(payload.len() <= MAX_PAYLOAD);
        let mut plain = Vec::with_capacity(1 + payload.len());
        plain.push(kind);
        plain.extend_from_slice(payload);
        let len = (plain.len() + TAG) as u16;
        let header = len.to_be_bytes();
        let sealed = self
            .cipher
            .encrypt(
                &nonce(self.counter).into(),
                Payload {
                    msg: &plain,
                    aad: &header,
                },
            )
            .map_err(|_| io::Error::other("encryption failed"))?;
        self.counter = self
            .counter
            .checked_add(1)
            .ok_or_else(|| io::Error::other("frame counter exhausted"))?;
        let mut out = Vec::with_capacity(2 + sealed.len());
        out.extend_from_slice(&header);
        out.extend_from_slice(&sealed);
        self.inner.write_all(&out)
    }
    /// Game bytes, split into frames.
    pub fn write_data(&mut self, bytes: &[u8]) -> io::Result<()> {
        for chunk in bytes.chunks(MAX_PAYLOAD) {
            self.frame(DATA, chunk)?;
        }
        self.inner.flush()
    }
    /// A message between the two tunnel ends.
    pub fn write_control(&mut self, message: &[u8]) -> io::Result<()> {
        if message.len() > MAX_PAYLOAD {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "control message too long",
            ));
        }
        self.frame(CONTROL, message)?;
        self.inner.flush()
    }
    pub fn get_ref(&self) -> &W {
        &self.inner
    }
}

pub struct SecureReader<R> {
    inner: R,
    cipher: ChaCha20Poly1305,
    counter: u64,
}
impl<R: Read> SecureReader<R> {
    /// The next frame's kind and payload; None at a clean end of stream.
    fn open(&mut self) -> io::Result<Option<(u8, Vec<u8>)>> {
        let mut header = [0u8; 2];
        let mut got = 0;
        while got < 2 {
            match self.inner.read(&mut header[got..]) {
                Ok(0) if got == 0 => return Ok(None),
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(n) => got += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        let len = usize::from(u16::from_be_bytes(header));
        if !(TAG + 1..=TAG + 1 + MAX_PAYLOAD).contains(&len) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "bad frame length",
            ));
        }
        let mut sealed = vec![0u8; len];
        self.inner.read_exact(&mut sealed)?;
        let plain = self
            .cipher
            .decrypt(
                &nonce(self.counter).into(),
                Payload {
                    msg: &sealed,
                    aad: &header,
                },
            )
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "frame failed to open"))?;
        self.counter += 1;
        Ok(Some((plain[0], plain[1..].to_vec())))
    }
    pub fn read_frame(&mut self) -> io::Result<Option<Frame>> {
        match self.open()? {
            None => Ok(None),
            Some((DATA, p)) => Ok(Some(Frame::Data(p))),
            Some((CONTROL, p)) => Ok(Some(Frame::Control(p))),
            Some(_) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unexpected frame",
            )),
        }
    }
    pub fn get_ref(&self) -> &R {
        &self.inner
    }
}

/// Protect one stream of the session: swap randoms, derive the keys for
/// each direction, confirm. Fails with [`wrong_code`] when the peer used
/// another key.
pub fn secure<R: Read, W: Write>(
    mut read: R,
    mut write: W,
    key: &Key,
    role: Role,
) -> io::Result<(SecureReader<R>, SecureWriter<W>)> {
    let mut mine = [0u8; 16];
    crate::fill_random(&mut mine)?;
    write.write_all(&mine)?;
    write.flush()?;
    let mut theirs = [0u8; 16];
    read.read_exact(&mut theirs)?;
    let (guest, host) = match role {
        Role::Guest => (mine, theirs),
        Role::Host => (theirs, mine),
    };
    let mut salt = [0u8; 32];
    salt[..16].copy_from_slice(&guest);
    salt[16..].copy_from_slice(&host);
    let to_guest = cipher(key, &salt, b"horizon-friends v1 host->guest");
    let to_host = cipher(key, &salt, b"horizon-friends v1 guest->host");
    let (send, recv) = match role {
        Role::Host => (to_guest, to_host),
        Role::Guest => (to_host, to_guest),
    };
    let mut writer = SecureWriter {
        inner: write,
        cipher: send,
        counter: 0,
    };
    let mut reader = SecureReader {
        inner: read,
        cipher: recv,
        counter: 0,
    };
    writer.frame(CONFIRMATION, CONFIRM)?;
    writer.inner.flush()?;
    match reader.open() {
        Ok(Some((CONFIRMATION, p))) if p == CONFIRM => Ok((reader, writer)),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "no confirmation",
        )),
        Err(e) if e.kind() == io::ErrorKind::InvalidData => Err(wrong_code()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{TcpListener, TcpStream};

    fn pair() -> (TcpStream, TcpStream) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let a = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        let (b, _) = l.accept().unwrap();
        (a, b)
    }

    fn both<A: Send + 'static, B: Send + 'static>(
        a: impl FnOnce() -> A + Send + 'static,
        b: impl FnOnce() -> B + Send + 'static,
    ) -> (A, B) {
        let t = std::thread::spawn(a);
        let rb = b();
        (t.join().unwrap(), rb)
    }

    fn keys(
        host_secret: &'static str,
        guest_secret: &'static str,
    ) -> (Key, Key, TcpStream, TcpStream) {
        let (mut h, mut g) = pair();
        let (hk, gk) = both(
            {
                let mut h2 = h.try_clone().unwrap();
                move || exchange(&mut h2, host_secret, "K7QM", 9).unwrap()
            },
            {
                let mut g2 = g.try_clone().unwrap();
                move || exchange(&mut g2, guest_secret, "K7QM", 9).unwrap()
            },
        );
        h.flush().unwrap();
        g.flush().unwrap();
        (hk, gk, h, g)
    }

    #[test]
    fn same_code_same_key_and_frames_flow() {
        let (hk, gk, h, g) = keys("3XPD", "3XPD");
        assert_eq!(hk.0, gk.0);
        let ((mut hr, mut hw), (mut gr, mut gw)) = both(
            move || secure(h.try_clone().unwrap(), h, &hk, Role::Host).unwrap(),
            move || secure(g.try_clone().unwrap(), g, &gk, Role::Guest).unwrap(),
        );
        let big: Vec<u8> = (0..MAX_PAYLOAD * 2 + 5).map(|i| i as u8).collect();
        gw.write_data(&big).unwrap();
        gw.write_control(b"{\"use\":true}").unwrap();
        hw.write_data(b"pong").unwrap();
        let mut got = Vec::new();
        while got.len() < big.len() {
            match hr.read_frame().unwrap().unwrap() {
                Frame::Data(d) => got.extend(d),
                Frame::Control(_) => panic!("control before data"),
            }
        }
        assert_eq!(got, big);
        assert!(
            matches!(hr.read_frame().unwrap(), Some(Frame::Control(c)) if c == b"{\"use\":true}")
        );
        assert!(matches!(gr.read_frame().unwrap(), Some(Frame::Data(d)) if d == b"pong"));
        drop(gw);
        drop(gr);
        assert!(hr.read_frame().unwrap().is_none());
    }

    #[test]
    fn a_wrong_secret_is_refused_on_both_ends() {
        let (hk, gk, h, g) = keys("3XPD", "3XPE");
        assert_ne!(hk.0, gk.0);
        let (a, b) = both(
            move || secure(h.try_clone().unwrap(), h, &hk, Role::Host).err(),
            move || secure(g.try_clone().unwrap(), g, &gk, Role::Guest).err(),
        );
        assert!(is_wrong_code(&a.unwrap()));
        assert!(is_wrong_code(&b.unwrap()));
    }

    #[test]
    fn the_session_is_part_of_the_key() {
        let (mut h, mut g) = pair();
        let (hk, gk) = both(
            move || exchange(&mut h, "3XPD", "K7QM", 1).unwrap(),
            move || exchange(&mut g, "3XPD", "K7QM", 2).unwrap(),
        );
        assert_ne!(hk.0, gk.0);
    }

    /// A byte changed in flight, frames replayed or reordered: the reader
    /// stops. Built on in-memory buffers to edit the bytes.
    #[test]
    fn tampering_is_detected() {
        let key = Key([7; 32]);
        let seal = |frames: &[&[u8]]| {
            let mut w = SecureWriter {
                inner: Vec::new(),
                cipher: cipher(&key, &[1; 32], b"horizon-friends v1 guest->host"),
                counter: 0,
            };
            for f in frames {
                w.write_data(f).unwrap();
            }
            w.inner
        };
        let reader = |bytes: Vec<u8>| SecureReader {
            inner: io::Cursor::new(bytes),
            cipher: cipher(&key, &[1; 32], b"horizon-friends v1 guest->host"),
            counter: 0,
        };
        let good = seal(&[b"hello", b"world"]);
        let mut r = reader(good.clone());
        assert!(matches!(r.read_frame().unwrap(), Some(Frame::Data(d)) if d == b"hello"));
        assert!(matches!(r.read_frame().unwrap(), Some(Frame::Data(d)) if d == b"world"));
        assert!(r.read_frame().unwrap().is_none());
        // One flipped bit.
        let mut bad = good.clone();
        bad[5] ^= 1;
        assert!(reader(bad).read_frame().is_err());
        // Frames swapped: the counter no longer matches.
        let first = 2 + usize::from(u16::from_be_bytes([good[0], good[1]]));
        let mut swapped = good[first..].to_vec();
        swapped.extend_from_slice(&good[..first]);
        assert!(reader(swapped).read_frame().is_err());
        // The first frame twice.
        let mut r = reader([&good[..first], &good[..first]].concat());
        r.read_frame().unwrap();
        assert!(r.read_frame().is_err());
        // A header claiming more than a frame can hold.
        assert!(reader(vec![0xFF, 0xFF, 0]).read_frame().is_err());
    }
}
