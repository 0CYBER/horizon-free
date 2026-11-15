use mod_api::wire::{Reader, Wire, WireError, Writer};

fn check<T: Wire>(values: Vec<T>, expected: &[u8]) {
    let mut writer = Writer::new();
    writer.put(&values);
    assert_eq!(writer.bytes(), expected);
    let decoded = Reader::decode::<Vec<T>>(expected).unwrap();
    let mut again = Writer::new();
    again.put(&decoded);
    assert_eq!(again.bytes(), expected);
    let mut owned = Writer::new();
    owned.put_owned(decoded);
    assert_eq!(owned.bytes(), expected);
    for end in 0..expected.len() {
        assert!(Reader::decode::<Vec<T>>(&expected[..end]).is_err());
    }
    let mut trailing = expected.to_vec();
    trailing.push(0);
    assert!(matches!(
        Reader::decode::<Vec<T>>(&trailing),
        Err(WireError::Trailing)
    ));
    let mut limited = Writer::new();
    limited.limit(expected.len() - 1);
    limited.put(&values);
    assert_eq!(limited.error(), Some(WireError::Length));
    assert!(limited.bytes().is_empty());
}

#[test]
fn numeric_lists_preserve_little_endian_bytes() {
    check(
        vec![-1i32, 0x12345678],
        &[2, 0, 0, 0, 255, 255, 255, 255, 120, 86, 52, 18],
    );
    check(
        vec![i64::MIN, 1],
        &[2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 128, 1, 0, 0, 0, 0, 0, 0, 0],
    );
    check(
        vec![1.0f32, -0.0, f32::from_bits(0x7fc00001)],
        &[3, 0, 0, 0, 0, 0, 128, 63, 0, 0, 0, 128, 1, 0, 192, 127],
    );
    check(vec![0x1234u16], &[1, 0, 0, 0, 52, 18]);
    check(vec![1.0f64], &[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 240, 63]);
    check(Vec::<i32>::new(), &[0, 0, 0, 0]);
    check(vec![255u8], &[1, 0, 0, 0, 255]);
    check(vec![-2i8], &[1, 0, 0, 0, 254]);
    check(vec![-2i16], &[1, 0, 0, 0, 254, 255]);
    check(vec![0x12345678u32], &[1, 0, 0, 0, 120, 86, 52, 18]);
    check(vec![1u64], &[1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]);
    check(
        vec![1u128],
        &[1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    );
    check(
        vec![i128::MIN],
        &[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 128],
    );
    let nested = vec![vec![1i32], vec![-1]];
    let expected = [
        2, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 255, 255, 255, 255,
    ];
    let mut writer = Writer::new();
    writer.put_owned(nested.clone());
    assert_eq!(writer.bytes(), expected);
    assert_eq!(Reader::decode::<Vec<Vec<i32>>>(&expected).unwrap(), nested);
}

#[test]
fn oversized_numeric_counts_refuse_before_allocating() {
    for bytes in [&[255, 255, 255, 255][..], &[2, 0, 0, 0, 1, 0, 0, 0][..]] {
        assert!(matches!(
            Reader::decode::<Vec<i32>>(bytes),
            Err(WireError::Length)
        ));
        assert!(Reader::decode::<Vec<i64>>(bytes).is_err());
        assert!(Reader::decode::<Vec<f32>>(bytes).is_err());
    }
    assert!(Reader::decode::<Vec<bool>>(&[1, 0, 0, 0, 2]).is_err());
}
