//! Every state's packets, read from bytes that came off a wire: a malformed one is an error, never a
//! panic or a runaway allocation, and one that does decode reads its own body back.

use horizon_protocol::packets::{configuration, handshake, login, play, status};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const GREEDY: &[&[u8]] = &[
    &[0xff, 0xff, 0xff, 0xff, 0x0f],
    &[0x80, 0x80, 0x80, 0x80, 0x08],
    &[0xff, 0xff, 0xff, 0xff, 0x07],
    &[0x01, 0xff, 0xff, 0xff, 0x07],
    &[0x00, 0x00, 0x00, 0x00],
    &[0x01, 0x00],
    &[0x02, 0x00, 0x00],
    &[0x7f, 0x00],
    &[0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
];

fn payload(rng: &mut Rng) -> Vec<u8> {
    match rng.below(3) {
        0 => (0..rng.below(64)).map(|_| rng.next() as u8).collect(),
        1 => {
            let mut v = GREEDY[rng.below(GREEDY.len())].to_vec();
            v.extend((0..rng.below(16)).map(|_| rng.next() as u8));
            v
        }
        _ => Vec::new(),
    }
}

#[test]
fn every_state_survives_random_payloads() {
    let mut rng = Rng(0xdeadbeefcafe1234);
    for id in 0..=0xff {
        for _ in 0..200 {
            let bytes = payload(&mut rng);
            let _ = play::Clientbound::decode(id, &bytes);
            let _ = play::Serverbound::decode(id, &bytes);
            let _ = configuration::Clientbound::decode(id, &bytes);
            let _ = configuration::Serverbound::decode(id, &bytes);
            let _ = login::Clientbound::decode(id, &bytes);
            let _ = login::Serverbound::decode(id, &bytes);
            let _ = status::Clientbound::decode(id, &bytes);
            let _ = status::Serverbound::decode(id, &bytes);
            let _ = handshake::Serverbound::decode(id, &bytes);
        }
    }
}

fn split_id(framed: &[u8]) -> (i32, &[u8]) {
    let mut value = 0u32;
    for (i, &b) in framed.iter().enumerate().take(5) {
        value |= u32::from(b & 0x7f) << (7 * i);
        if b & 0x80 == 0 {
            return (value as i32, &framed[i + 1..]);
        }
    }
    panic!("no id in {framed:02x?}");
}

macro_rules! round_trip {
    ($ty:ty, $id:expr, $bytes:expr) => {{
        let id = $id;
        if let Ok(p) = <$ty>::decode($id, $bytes) {
            let name = p.name();
            if name != "Unknown" && name != "HandshakeUnknown" {
                if let Ok(framed) = p.try_encode() {
                    let (written, body) = split_id(&framed);
                    assert_eq!(written, $id, "{name} wrote another id");
                    let again = <$ty>::decode($id, body);
                    assert!(
                        again.is_ok(),
                        "{} {id} in={:02x?} out={:02x?}: {}",
                        name,
                        $bytes,
                        framed,
                        again.err().map(|e| e.to_string()).unwrap_or_default()
                    );
                    assert_eq!(again.unwrap().name(), name);
                }
            }
        }
    }};
}

#[test]
fn every_state_reads_what_it_wrote() {
    let mut rng = Rng(0x5eed5eed5eed5eed);
    for id in 0..=0xff {
        for _ in 0..200 {
            let bytes = payload(&mut rng);
            round_trip!(play::Clientbound, id, &bytes);
            round_trip!(play::Serverbound, id, &bytes);
            round_trip!(configuration::Clientbound, id, &bytes);
            round_trip!(configuration::Serverbound, id, &bytes);
            round_trip!(login::Clientbound, id, &bytes);
            round_trip!(login::Serverbound, id, &bytes);
            round_trip!(status::Clientbound, id, &bytes);
            round_trip!(status::Serverbound, id, &bytes);
            round_trip!(handshake::Serverbound, id, &bytes);
        }
    }
}
