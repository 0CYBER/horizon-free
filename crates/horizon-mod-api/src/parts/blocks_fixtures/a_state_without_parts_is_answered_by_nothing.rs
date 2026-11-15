#[test]
pub(crate) fn a_state_without_parts_is_answered_by_nothing() {
    game_thread();
    resolve(&[]).unwrap();
    // Out of range, and in range but a block no part is attached to.
    assert!(
        super::parts(HookId::AnalogOutput, "analog_output")
            .for_hook(HookId::AnalogOutput)
            .unwrap()
            .block_value(value(99))
            .is_empty()
    );
    assert!(
        super::parts(HookId::AnalogOutput, "analog_output")
            .for_hook(HookId::AnalogOutput)
            .unwrap()
            .block_value(value(0))
            .is_empty()
    );
    // A scoped hook has no global method, so both keep vanilla's answer.
    assert_eq!(read(0), 1);
    assert_eq!(read(99), 1);
}
