use super::*;

#[test]
fn network_methods_match_engine_bytes_and_leave_trailing_data() {
    let tag = Tag::compound([
        ("a", Tag::Byte(1)),
        ("b", Tag::String("x".into())),
        ("c", Tag::IntArray(vec![1, 2])),
    ]);
    let mut bytes = tag.to_network_bytes().unwrap();
    assert_eq!(
        bytes,
        [
            10, 1, 0, 1, b'a', 1, 8, 0, 1, b'b', 0, 1, b'x', 11, 0, 1, b'c', 0, 0, 0, 2, 0, 0, 0,
            1, 0, 0, 0, 2, 0
        ]
    );
    assert_eq!(bytes, write_network(&tag, Limits::default()).unwrap());
    // Vanilla 26.3 NbtIo.readAnyTag reports 325: 48 compound +
    // 3*(28 + 2*1 + 36) keys/entries + 9 byte + (36+2) string + (24+8) array.
    assert!(Tag::from_network_bytes(&bytes, Some(325)).is_ok());
    assert!(Tag::from_network_bytes(&bytes, Some(324)).is_err());
    let len = bytes.len();
    assert_eq!(
        Tag::from_network_bytes(&bytes, None).unwrap(),
        (tag.clone(), len)
    );
    for end in 0..len {
        assert!(Tag::from_network_bytes(&bytes[..end], None).is_err());
    }
    bytes.extend([0xab, 0xcd]);
    assert_eq!(Tag::from_network_bytes(&bytes, None).unwrap(), (tag, len));
    assert_eq!(Tag::End.to_network_bytes().unwrap(), [0]);
    assert_eq!(
        Tag::from_network_bytes(&[0, 123], Some(0)).unwrap(),
        (Tag::End, 1)
    );
}

#[test]
fn network_quota_counts_heap_not_wire_bytes() {
    // ByteArrayTag: 24 bytes overhead + n bytes; exactly default quota + 1.
    let tag = Tag::ByteArray(vec![0; NETWORK_NBT_QUOTA as usize - 24 + 1]);
    let bytes = tag.to_network_bytes().unwrap();
    assert_eq!(
        Tag::from_network_bytes(&bytes, None).unwrap_err().reason,
        "heap quota exceeded"
    );
    assert_eq!(
        Tag::from_network_bytes(&bytes, Some(NETWORK_NBT_QUOTA + 1))
            .unwrap()
            .0,
        tag
    );
    let exact = &bytes[..bytes.len() - 1];
    let mut exact = exact.to_vec();
    exact[1..5].copy_from_slice(&((NETWORK_NBT_QUOTA - 24) as i32).to_be_bytes());
    assert!(Tag::from_network_bytes(&exact, None).is_ok());
    // Compound 48 + key (28 + 2 UTF-16 units) + entry 36 + String (36 + 2).
    let bytes = Tag::compound([("a", Tag::String("x".into()))])
        .to_network_bytes()
        .unwrap();
    assert!(Tag::from_network_bytes(&bytes, Some(152)).is_ok());
    assert!(Tag::from_network_bytes(&bytes, Some(151)).is_err());
}

#[test]
fn network_depth_512_and_malformed_inputs() {
    for compound in [false, true] {
        let mut tag = Tag::Int(1);
        for _ in 0..512 {
            tag = if compound {
                Tag::compound([("x", tag)])
            } else {
                Tag::List(vec![tag])
            };
        }
        let bytes = tag.to_network_bytes().unwrap();
        assert_eq!(Tag::from_network_bytes(&bytes, None).unwrap().0, tag);
        let deeper = if compound {
            Tag::compound([("x", tag)])
        } else {
            Tag::List(vec![tag])
        };
        assert!(deeper.to_network_bytes().is_err());
        let bytes = write_network(
            &deeper,
            Limits {
                max_depth: 513,
                ..Limits::default()
            },
        )
        .unwrap();
        assert_eq!(
            Tag::from_network_bytes(&bytes, None).unwrap_err().reason,
            "depth limit exceeded"
        );
    }
    for bytes in [
        &[13][..],
        &[7, 255, 255, 255, 255],
        &[8, 0, 1, 192],
        &[9, 0, 0, 0, 0, 1],
    ] {
        assert!(Tag::from_network_bytes(bytes, None).is_err());
    }
}

#[test]
fn network_all_variants_and_java_strings_reuse_engine_codec() {
    let tags = [
        Tag::End,
        Tag::Byte(-1),
        Tag::Short(-2),
        Tag::Int(-3),
        Tag::Long(-4),
        Tag::Float(0xff80_0001),
        Tag::Double(0x7ff0_0000_0000_0001),
        Tag::ByteArray(vec![1, 2]),
        Tag::IntArray(vec![-1, 2]),
        Tag::LongArray(vec![-1, 2]),
        Tag::String(JavaString(vec![0, 0xd800, 0xdc00, 0xdfff])),
        Tag::List(vec![Tag::Int(1), Tag::Byte(2)]),
        Tag::compound([("x", Tag::Int(7))]),
    ];
    for tag in tags {
        let bytes = tag.to_network_bytes().unwrap();
        assert_eq!(bytes, write_network(&tag, Limits::default()).unwrap());
        assert_eq!(
            Tag::from_network_bytes(&bytes, None).unwrap(),
            (tag, bytes.len())
        );
    }
    let bytes = [10, 1, 0, 1, b'x', 1, 1, 0, 1, b'x', 2, 0];
    // 48 compound + 2 * (28 + 2 key + 9 byte) + 36 for one distinct key.
    assert_eq!(
        Tag::from_network_bytes(&bytes, Some(162)).unwrap().0,
        Tag::compound([("x", Tag::Byte(2))])
    );
    assert!(Tag::from_network_bytes(&bytes, Some(161)).is_err());
    let bytes = Tag::String(JavaString(vec![0, 0xd800]))
        .to_network_bytes()
        .unwrap();
    // UTF-16 units, not the 5 modified UTF-8 bytes: 36 + 2 * 2.
    assert!(Tag::from_network_bytes(&bytes, Some(40)).is_ok());
    assert!(Tag::from_network_bytes(&bytes, Some(39)).is_err());
    assert!(
        Tag::String(JavaString(vec![0; 32768]))
            .to_network_bytes()
            .is_err()
    );
}
