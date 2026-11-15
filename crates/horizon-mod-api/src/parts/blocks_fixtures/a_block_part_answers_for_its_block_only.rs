#[test]
pub(crate) fn a_block_part_answers_for_its_block_only() {
    game_thread();
    // No world is open, so no state has its parts yet: nothing answers.
    assert_eq!(read(1), 1);
    resolve(&[]).unwrap();
    // 5 more from the part: 1 + 5.
    assert_eq!(read(1), 6);
    assert_eq!(read(2), 1);
}
