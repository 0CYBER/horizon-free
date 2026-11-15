use super::widened;
use crate::content::payload_codec::{
    PayloadDecodeError, PayloadEncodeError, decode_payload, encode_payload,
};

/// `lists` lists of 4096 empty strings, written out by hand: VarInt counts, then one zero
/// length byte per string.
fn body(lists: u8) -> Vec<u8> {
    let mut body = match lists {
        0..0x80 => vec![lists],
        _ => vec![lists & 0x7f | 0x80, lists >> 7],
    };
    for _ in 0..lists {
        body.extend([0x80, 0x20]);
        body.extend(std::iter::repeat_n(0, 4096));
    }
    body
}

fn value(lists: usize) -> Vec<Vec<String>> {
    vec![vec![String::new(); 4096]; lists]
}

#[test]
fn a_writer_refuses_what_the_readers_allocation_budget_refuses() {
    // 128 lists of 4096 empty strings are about 512 KiB on the wire, but a 64-bit reader
    // reserves 128 x 4096 x 24 bytes of String headers: past the 8 MiB budget.
    let refused = body(128);
    assert!(refused.len() < 1 << 20);
    assert_eq!(
        decode_payload::<Vec<Vec<String>>>(&refused),
        Err(PayloadDecodeError::BudgetExhausted)
    );
    assert_eq!(
        encode_payload(&value(128), 1 << 20),
        Err(PayloadEncodeError::BudgetExhausted)
    );
    // 64 of them reserve about 6 MiB: written and read.
    let accepted = body(64);
    assert_eq!(encode_payload(&value(64), 1 << 20), Ok(accepted.clone()));
    assert_eq!(decode_payload::<Vec<Vec<String>>>(&accepted), Ok(value(64)));
}

#[test]
fn every_target_charges_a_64_bit_peers_sizes() {
    // A String is 12 bytes on wasm32 (three 4-byte words) and 24 on a 64-bit target: a wasm32
    // writer charges the 24 a native reader will.
    assert_eq!(widened(12, 4), 24);
    assert_eq!(widened(24, 8), 24);
    // A type without pointers is charged its 64-bit size on 64-bit, and at most twice its
    // size on 32-bit, never less.
    assert_eq!(widened(4, 8), 4);
    assert_eq!(widened(4, 4), 8);
}
