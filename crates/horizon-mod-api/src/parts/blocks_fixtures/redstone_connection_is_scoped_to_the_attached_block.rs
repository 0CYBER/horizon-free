#[test]
pub(crate) fn redstone_connection_is_scoped_to_the_attached_block() {
    game_thread();
    resolve(&[]).unwrap();
    for (state, expected) in [(3, true), (2, false)] {
        let mut connected = false;
        dispatch_should_redstone_wire_connect_to(
            &Level,
            LevelDimension::Overworld,
            BlockPos::new(0, 64, 0),
            value(state),
            Some(Direction::East),
            &mut connected,
        );
        assert_eq!(connected, expected);
    }
}
