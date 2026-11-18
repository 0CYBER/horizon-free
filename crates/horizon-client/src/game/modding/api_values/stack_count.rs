//! Tests for stack count admission, which lives in horizon-gameplay.
use super::*;
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_counts_but_keeps_other_write_validation() {
        let items = super::super::stack_limit_tests::catalog();
        let key = |name| mod_api::__private::item_key(name).unwrap();
        let oversized = ApiStack::new(key("minecraft:stick"), u32::MAX).unwrap();
        assert_eq!(
            oversized
                .try_into_engine_preserving_count(&items)
                .unwrap()
                .count,
            u32::MAX
        );
        assert_eq!(
            oversized.try_into_engine(&items),
            Err(StackWriteError::CountExceeded { maximum: 64 })
        );
        let mut sword = ApiStack::new(key("minecraft:iron_sword"), 100).unwrap();
        sword.set_damage(11);
        assert_eq!(
            sword.try_into_engine_preserving_count(&items),
            Err(StackWriteError::DamageExceeded { maximum: 10 })
        );
        let mut malformed = oversized;
        malformed
            .set_components([(JavaString::from("minecraft:max_stack_size"), Tag::Int(-1))].into())
            .unwrap();
        assert_eq!(
            malformed.try_into_engine_preserving_count(&items),
            Err(StackWriteError::InvalidMaximum)
        );
    }
}
