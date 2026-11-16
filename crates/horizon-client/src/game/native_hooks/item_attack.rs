use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `IItemExtension.onLeftClickEntity`: `player`, holding `stack`, is
    /// about to attack the entity `entity` of type `kind`, where
    /// `Player.attack` starts, once `attack_entity` let the blow through and
    /// before any damage, `hurt_enemy` or wear. Set `canceled` and the attack
    /// ends there.
    pub(in crate::game) fn native_on_left_click_entity(
        &mut self,
        player: u128,
        stack: &ItemStack,
        entity: i32,
        kind: &str,
        canceled: &mut bool,
    ) {
        self.fire_on_left_click_entity(player, stack, entity, kind, canceled);
    }
    /// `IItemExtension.onEntitySwing`: the player in place (`entity`) is
    /// about to swing `stack` in `hand`, where `LivingEntity.swing` starts.
    /// Set `canceled` and no swing starts: no animation, no packet.
    pub(in crate::game) fn native_on_entity_swing(
        &mut self,
        stack: &ItemStack,
        entity: Option<i32>,
        hand: Hand,
        canceled: &mut bool,
    ) {
        self.fire_on_entity_swing(stack, entity, hand, canceled);
    }
}

impl Game<'_> {
    /// `ItemStack.onLeftClickEntity` for the acting player's blow at `entity`
    /// of `kind`, once `attack_entity` let it through: true when the held
    /// item's Part canceled the attack. Nothing runs for a hand holding an
    /// item without the Part.
    pub(in crate::game) fn native_left_click_entity_event(
        &mut self,
        entity: i32,
        kind: &str,
    ) -> bool {
        if !mod_api::__private::has_part_hook(mod_api::HookId::OnLeftClickEntity) {
            return false;
        }
        let Some(stack) = self.held() else {
            return false;
        };
        if !mod_api::__private::item_listens(mod_api::HookId::OnLeftClickEntity, &stack.item) {
            return false;
        }
        let stack = stack.clone();
        let player = self.native_acting_id();
        let mut canceled = false;
        self.native_on_left_click_entity(player, &stack, entity, kind, &mut canceled);
        canceled
    }
    /// `ItemStack.onEntitySwing` for the player in place swinging `hand`:
    /// true when the item's Part canceled the swing. Nothing runs for a hand
    /// holding nothing or an item without the Part.
    pub(in crate::game) fn native_entity_swing_event(&mut self, hand: Hand) -> bool {
        if !mod_api::__private::has_part_hook(mod_api::HookId::OnEntitySwing) {
            return false;
        }
        let stack = match hand {
            Hand::Main => self.held(),
            Hand::Off => self.offhand.as_ref(),
        };
        let Some(stack) = stack else {
            return false;
        };
        if !mod_api::__private::item_listens(mod_api::HookId::OnEntitySwing, &stack.item) {
            return false;
        }
        let stack = stack.clone();
        let entity = self.player.as_ref().map(|p| p.entity_id);
        let mut canceled = false;
        self.native_on_entity_swing(&stack, entity, hand, &mut canceled);
        canceled
    }
}
