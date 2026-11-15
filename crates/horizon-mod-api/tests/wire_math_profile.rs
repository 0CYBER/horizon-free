use mod_api::{
    MathProfile,
    wire::{Reader, Wire, WireError, Writer},
};

#[test]
fn math_profile_wire_layout_and_tags_are_pinned() {
    assert_eq!(MathProfile::MIN, 4);
    assert_eq!(MathProfile::layout(&mut vec![]), 0xdbe71919144a332d);
    for (profile, bytes) in [
        (MathProfile::LibmRounding, [0, 0, 0, 0]),
        (MathProfile::CorrectlyRounded, [1, 0, 0, 0]),
        (MathProfile::Aarch64Fma, [2, 0, 0, 0]),
        (MathProfile::StrictMath, [3, 0, 0, 0]),
    ] {
        let mut writer = Writer::new();
        writer.put(&profile);
        assert_eq!(writer.bytes(), bytes);
        assert_eq!(Reader::new(&bytes).take::<MathProfile>().unwrap(), profile);
    }
    assert!(Reader::new(&[0, 0, 0]).take::<MathProfile>().is_err());
    assert!(matches!(
        Reader::new(&[4, 0, 0, 0]).take::<MathProfile>(),
        Err(WireError::Tag(4))
    ));
}
