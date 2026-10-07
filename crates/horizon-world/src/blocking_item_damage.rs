//! Blocking-component durability dispatch bound to stored item durability.
use crate::{
    blocking_rules::ItemDamageFunction,
    item_durability::{self, DurabilityWorld},
};

pub trait BlockingItemDamageWorld: DurabilityWorld {
    type Level: Copy;
    fn is_player(&self) -> bool;
    fn is_server_player(&self) -> bool;
    fn level_is_client(
        &mut self,
        stack: &mut Self::Stack,
        level: Self::Level,
    ) -> Result<bool, Self::Error>;
    /// Implementations capture the stack's current item type before awarding it.
    fn award_item_used(&mut self, stack: &mut Self::Stack) -> Result<(), Self::Error>;
    fn entity_level_is_server(&mut self, stack: &mut Self::Stack) -> Result<bool, Self::Error>;
}

pub fn hurt<W: BlockingItemDamageWorld>(
    stack: &mut W::Stack,
    world: &mut W,
    level: W::Level,
    damage: f32,
    rule: ItemDamageFunction,
) -> Result<(), W::Error> {
    if !world.is_player() {
        return Ok(());
    }
    if !world.level_is_client(stack, level)? {
        world.award_item_used(stack)?;
    }
    let amount = rule.apply(damage);
    if amount > 0 && world.entity_level_is_server(stack)? {
        let player = world.is_server_player();
        item_durability::hurt_and_break(stack, world, amount, player)?;
    }
    Ok(())
}
