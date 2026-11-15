use mod_api::{
    SpawnFlags, SpawnLimits,
    wire::{Reader, Writer},
};

#[test]
fn spawn_policy_crosses_wire_without_narrowing_limits_or_flags() {
    let limits = SpawnLimits {
        monster: u32::MAX,
        creature: 0,
        ambient: 1,
        axolotls: 2,
        underground_water_creature: 3,
        water_creature: 4,
        water_ambient: 5,
    };
    let mut writer = Writer::new();
    writer.put(&limits);
    let expected: Vec<u8> = [u32::MAX, 0, 1, 2, 3, 4, 5]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    assert_eq!(writer.bytes(), expected);
    assert_eq!(
        Reader::new(&expected).take::<SpawnLimits>().unwrap(),
        limits
    );
    for monsters in [false, true] {
        for friendlies in [false, true] {
            let flags = SpawnFlags {
                monsters,
                friendlies,
            };
            writer.clear();
            writer.put(&flags);
            assert_eq!(writer.bytes(), [u8::from(monsters), u8::from(friendlies)]);
            assert_eq!(
                Reader::new(writer.bytes()).take::<SpawnFlags>().unwrap(),
                flags
            );
        }
    }
}
