#[test]
pub(crate) fn a_host_without_a_backend_refuses_handle_calls() {
    // NoHost implements nothing, so Host::backend keeps its None default:
    // every handle answers its typed refusal, never a no-op success.
    let level = LevelRef::new(WorldEpoch::new(1), crate::DimensionKey::OVERWORLD);
    assert!(!level.in_place(&NoHost));
    assert_eq!(
        level.entities_in(&NoHost, [0.0, 0.0, 0.0], [1.0, 1.0, 1.0]),
        None
    );
}
