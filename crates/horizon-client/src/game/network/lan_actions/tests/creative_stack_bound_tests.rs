//! The count a creative stack is held to: vanilla's codec range (`ItemStack.CODEC`'s
//! `ExtraCodecs.intRange(1, 99)`) refuses 100, and a native mod's larger stack size raises it.

#[test]
fn the_creative_count_bound_is_99_unless_a_modded_stack_size_is_larger() {
    assert_eq!(super::creative_stack_limit(64), 99);
    assert_eq!(super::creative_stack_limit(150), 150);
}
