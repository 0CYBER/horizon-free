//! Use-remainder conversion preserves stack identity and only creates a
//! remainder after an actual count decrease for a finite-material actor.
use crate::item_stack_refs::StackRef;

pub fn convert<I: Copy + Eq, K: Clone + Eq, V: Clone + PartialEq, E>(
    stack: StackRef<I, K, V>,
    original_count: i32,
    infinite_materials: bool,
    create: impl FnOnce() -> StackRef<I, K, V>,
    extra: impl FnOnce(StackRef<I, K, V>) -> Result<(), E>,
) -> Result<StackRef<I, K, V>, E> {
    if infinite_materials || stack.read().count() >= original_count {
        return Ok(stack);
    }
    let remainder = create();
    if stack.read().is_empty() {
        return Ok(remainder);
    }
    extra(remainder)?;
    Ok(stack)
}
