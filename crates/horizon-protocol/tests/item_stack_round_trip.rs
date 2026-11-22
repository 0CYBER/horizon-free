//! Item stacks arrive from the server, a proxy or a mod. A stack an item codec reads must write back to
//! a body the same codec reads to the same stack: a patch the writer drops desynchronises the
//! connection, and the next packet's bytes are read as this one's.

use horizon_protocol::{
    buf::{Codec, Reader, Writer},
    item::{DelimitedSlot, Slot, Stack},
};

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

/// The openings of a stack's body: count, item id, and the patch lengths behind them.
const SEEDS: &[&[u8]] = &[
    &[0x01, 0x01, 0x00, 0x00],
    &[0x01, 0x01, 0x00, 0x00, 0x00],
    &[0x40, 0x01, 0x00, 0x00],
    &[0x01, 0x40, 0x01, 0x00, 0xff, 0xff, 0xff, 0x0f],
    &[0x00],
    &[0xff, 0xff, 0xff, 0xff, 0x0f],
    &[0x05, 0x00, 0x00, 0x00, 0x00],
];

fn payload(rng: &mut Rng) -> Vec<u8> {
    match rng.below(3) {
        0 => (0..rng.below(40)).map(|_| rng.next() as u8).collect(),
        1 => {
            let mut bytes = SEEDS[rng.below(SEEDS.len())].to_vec();
            bytes.extend((0..rng.below(12)).map(|_| rng.next() as u8));
            bytes
        }
        _ => Vec::new(),
    }
}

fn round_trip<C: Codec<Option<Stack>>>(bytes: &[u8], codec: &str) {
    let Ok(stack) = C::read(&mut Reader::new(bytes)) else {
        return;
    };
    let mut w = Writer::new();
    C::write(&stack, &mut w);
    let again = C::read(&mut Reader::new(&w.data));
    assert!(
        again.is_ok(),
        "{codec}: {bytes:02x?} decoded but its own body {:02x?} did not: {}",
        w.data,
        again.err().map(|e| e.to_string()).unwrap_or_default()
    );
    assert_eq!(
        again.unwrap(),
        stack,
        "{codec}: {bytes:02x?} changed through its own body {:02x?}",
        w.data
    );
}

#[test]
fn a_stack_survives_its_own_body() {
    let mut rng = Rng(0xabcd_ef01_2345_6789);
    for _ in 0..20000 {
        let bytes = payload(&mut rng);
        round_trip::<Slot>(&bytes, "Slot");
        round_trip::<DelimitedSlot>(&bytes, "DelimitedSlot");
    }
}
