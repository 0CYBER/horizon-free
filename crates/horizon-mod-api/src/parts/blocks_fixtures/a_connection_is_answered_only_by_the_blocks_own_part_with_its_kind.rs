#[test]
pub(crate) fn a_connection_is_answered_only_by_the_blocks_own_part_with_its_kind() {
    game_thread();
    resolve(&[]).unwrap();
    for kind in [
        ConnectionKind::Fence,
        ConnectionKind::Wall,
        ConnectionKind::Bars,
    ] {
        // `test:connector` answers whether the kind is a wall, from the
        // opposite answer; `test:other` has no part and keeps it.
        let answer = kind == ConnectionKind::Wall;
        for (state, expected) in [(3, answer), (2, !answer)] {
            let mut connected = !answer;
            dispatch_connects_to(
                &Level,
                crate::__private::ConnectsToCall {
                    dimension: LevelDimension::Overworld,
                    pos: BlockPos::new(0, 64, 0),
                    state: value(state),
                    kind,
                    neighbour: value(1),
                    direction: Direction::West,
                    face_solid: true,
                    connected: &mut connected,
                },
            );
            assert_eq!(connected, expected, "{kind:?} on state {state}");
        }
    }
}
