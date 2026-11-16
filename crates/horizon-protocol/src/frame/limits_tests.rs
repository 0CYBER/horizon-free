use super::*;

#[test]
fn compression_cannot_hide_an_uncompressed_packet_above_the_vanilla_limit() {
    let mut bytes = vec![0; MAX_UNCOMPRESSED];
    assert!(contents(&bytes, Some(0)).unwrap().len() < MAX_FRAME);
    bytes.push(0);
    for threshold in [0, MAX_UNCOMPRESSED + 2] {
        let error = contents(&bytes, Some(threshold)).unwrap_err();
        assert_eq!(
            error.frame_limit(),
            Some(&FrameTooLarge {
                stage: FrameSize::Uncompressed,
                size: MAX_UNCOMPRESSED + 1,
                limit: MAX_UNCOMPRESSED,
            })
        );
    }
}

#[test]
fn uncompressed_boundary_and_compressed_frame_expansion_have_named_refusals() {
    let mut bytes = vec![0; MAX_FRAME];
    assert_eq!(contents(&bytes, None).unwrap(), bytes);
    bytes.push(0);
    let error = contents(&bytes, None).unwrap_err();
    assert_eq!(
        error.frame_limit(),
        Some(&FrameTooLarge {
            stage: FrameSize::Encoded,
            size: MAX_FRAME + 1,
            limit: MAX_FRAME,
        })
    );

    bytes.truncate(MAX_FRAME);
    let mut state = 0x71e4_930b_2375_dca9_u64;
    for byte in &mut bytes {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        *byte = state as u8;
    }
    let error = contents(&bytes, Some(0)).unwrap_err();
    let detail = error.frame_limit().unwrap();
    assert_eq!(detail.stage, FrameSize::Encoded);
    assert!(detail.size > MAX_FRAME);
    assert_eq!(detail.limit, MAX_FRAME);
}
