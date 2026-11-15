use super::{FluidFile, FluidLook, WorldFluid};
use mod_vocab::wire::{Reader, Wire, WireError, Writer};

fn round<T: Wire + PartialEq + std::fmt::Debug>(value: T) {
    let mut out = Writer::new();
    out.put(&value);
    assert_eq!(out.error(), None);
    assert_eq!(Reader::decode::<T>(out.bytes()).unwrap(), value);
    for end in 0..out.bytes().len() {
        assert!(Reader::decode::<T>(&out.bytes()[..end]).is_err());
    }
}

#[test]
fn fluid_wire_round_trips_and_truncation() {
    round(WorldFluid {
        tick_delay: 12,
        drop_off: 2,
        slope_find_distance: 3,
        source_conversion: true,
        bucket: false,
    });
    round(FluidLook {
        id: "a:oil".into(),
        sprite: Some("a:block/oil".into()),
        tint: Some(0x3F2A10),
        world: Some(WorldFluid {
            tick_delay: 9,
            drop_off: 3,
            slope_find_distance: 2,
            source_conversion: false,
            bucket: true,
        }),
    });
    round(FluidFile {
        sprite: Some("a:block/brine".into()),
        tint: Some(0x123456),
        world: None,
    });
}

#[test]
fn fluid_wire_invalid_option_tag() {
    assert_eq!(Reader::decode::<FluidFile>(&[2]), Err(WireError::Tag(2)));
    // Empty id followed by an invalid sprite Option tag.
    assert_eq!(
        Reader::decode::<FluidLook>(&[0, 0, 0, 0, 2]),
        Err(WireError::Tag(2))
    );
}
