#[test]
pub(crate) fn scoped_dispatch_looks_up_only_subscribed_hooks_and_parts() {
    game_thread();
    resolve(&[]).unwrap();
    let parts = super::parts(HookId::AnalogOutput, "test");
    let analog = parts.for_hook(HookId::AnalogOutput).unwrap();
    let connect = parts.for_hook(HookId::ShouldRedstoneWireConnectTo).unwrap();
    assert_eq!(analog.block_value(value(1)).len(), 1);
    assert!(connect.block_value(value(1)).is_empty());
    assert_eq!(connect.block_value(value(3)).len(), 1);
    assert!(analog.block_value(value(3)).is_empty());
    // The block entity part's BlockPart override is indexed independently.
    assert_eq!(read(4), 41);
    assert!(!super::has_part_hook(HookId::ShouldCheckWeakPower));
    let mut check = false;
    dispatch_should_check_weak_power(
        &Level,
        LevelDimension::Overworld,
        BlockPos::new(0, 64, 0),
        value(1),
        Direction::East,
        &mut check,
    );
    assert!(!check);
}
