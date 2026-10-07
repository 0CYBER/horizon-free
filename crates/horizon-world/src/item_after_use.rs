//! ItemStack finish-use component effects over shared stack storage. The copy
//! captured before the item handler determines both remainder and cooldown.
use crate::{
    component_patch::Prototype,
    item_cooldowns::UseCooldown,
    item_stack_refs::{ItemStacks, StackRef},
    item_stack_storage::ItemStackData,
    item_template::{ItemTemplate, ValidationRules},
    use_remainder,
};
pub trait AfterUseWorld<I, K, V>: ValidationRules<I, K, V> {
    type Error;
    type Level;
    type Group;
    fn remainder(&self, stack: &ItemStackData<I, K, V>) -> Option<ItemTemplate<I, K, V>>;
    fn cooldown(&self, stack: &ItemStackData<I, K, V>) -> Option<UseCooldown<Self::Group>>;
    fn prototype(&self, item: I) -> Prototype<K, V>;
    fn finish_item(
        &mut self,
        stack: StackRef<I, K, V>,
        level: Self::Level,
    ) -> Result<StackRef<I, K, V>, Self::Error>;
    fn infinite_materials(&mut self) -> Result<bool, Self::Error>;
    fn extra_item(&mut self, stack: StackRef<I, K, V>) -> Result<(), Self::Error>;
    fn apply_cooldown(
        &mut self,
        component: UseCooldown<Self::Group>,
        original: StackRef<I, K, V>,
    ) -> Result<(), Self::Error>;
}
pub fn finish<I: Copy + Eq, K: Clone + Eq, V: Clone + PartialEq, W: AfterUseWorld<I, K, V>>(
    stacks: &ItemStacks<I, K, V>,
    world: &mut W,
    stack: StackRef<I, K, V>,
    level: W::Level,
) -> Result<StackRef<I, K, V>, W::Error> {
    let original = stacks.copy(&stack);
    let result = world.finish_item(stack, level)?;
    after_use(stacks, world, result, original)
}
pub fn after_use<I: Copy + Eq, K: Clone + Eq, V: Clone + PartialEq, W: AfterUseWorld<I, K, V>>(
    stacks: &ItemStacks<I, K, V>,
    world: &mut W,
    mut result: StackRef<I, K, V>,
    original: StackRef<I, K, V>,
) -> Result<StackRef<I, K, V>, W::Error> {
    let remainder = world.remainder(&original.read());
    let cooldown = world.cooldown(&original.read());
    let count = original.read().count();
    if let Some(template) = remainder {
        let infinite = world.infinite_materials()?;
        result = use_remainder::convert_with(
            world,
            result,
            count,
            infinite,
            |world| {
                template
                    .create(stacks, world.prototype(template.item()), world)
                    .0
            },
            |world, extra| world.extra_item(extra),
        )?;
    }
    if let Some(cooldown) = cooldown {
        world.apply_cooldown(cooldown, original)?;
    }
    Ok(result)
}
