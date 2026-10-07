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
    convert_with(
        &mut (),
        stack,
        original_count,
        infinite_materials,
        |()| create(),
        |(), remainder| extra(remainder),
    )
}
/// Variant for creation and extra-item effects that share one mutable world.
/// The world is borrowed only for the current callback.
pub fn convert_with<I: Copy + Eq, K: Clone + Eq, V: Clone + PartialEq, E, W>(
    world: &mut W,
    stack: StackRef<I, K, V>,
    original_count: i32,
    infinite_materials: bool,
    create: impl FnOnce(&mut W) -> StackRef<I, K, V>,
    extra: impl FnOnce(&mut W, StackRef<I, K, V>) -> Result<(), E>,
) -> Result<StackRef<I, K, V>, E> {
    if infinite_materials || stack.read().count() >= original_count {
        return Ok(stack);
    }
    let remainder = create(world);
    if stack.read().is_empty() {
        return Ok(remainder);
    }
    extra(world, remainder)?;
    Ok(stack)
}
