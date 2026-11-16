use super::*;

impl Game<'_> {
    /// Apply a consuming result's swing on its owning logical side.
    pub(in crate::game) fn interaction_consumed(
        &mut self,
        answer: mod_api::InteractionResult,
    ) -> bool {
        use mod_api::InteractionResult as Answer;
        if (answer == Answer::Success && self.remote.is_some())
            || (answer == Answer::SuccessServer && self.remote.is_none())
        {
            self.swing_arm();
        }
        answer.consumes_action()
    }

    /// `ItemStack.useOn` after the block and before the item's implementation.
    pub(in crate::game) fn item_on_interaction(
        &mut self,
        hit: &crate::game::Hit,
        hand: Hand,
        stack: &ItemStack,
    ) -> Option<mod_api::InteractionResult> {
        if !mod_api::__private::has_part_hook(mod_api::HookId::UseOn) {
            return None;
        }
        let snapshot = stack.try_into_api().ok()?;
        let (player, player_entity_id) = self.native_clicker();
        let mut result = None;
        self.native_use_on(mod_api::__private::UseOnCall {
            stack: &snapshot,
            pos: hit.pos,
            face: hit.face,
            hand,
            hit: hit.native(),
            player,
            player_entity_id,
            result: &mut result,
        });
        result.filter(|answer| {
            !matches!(
                answer,
                mod_api::InteractionResult::Pass | mod_api::InteractionResult::TryWithEmptyHand
            )
        })
    }

    /// `Player.interactOn`: Fail skips this interaction, not the hand loop.
    pub(in crate::game) fn entity_interaction(&mut self, hand: Hand, id: i32) -> Option<bool> {
        if self.spectator() || !mod_api::__private::listened(mod_api::HookId::EntityInteract) {
            return None;
        }
        let entity = self.level_entity(id)?;
        let kind = self.target_type(entity);
        let stack = self.with_hand(hand, |g| g.held().cloned());
        let mut result = None;
        self.native_entity_interact(hand, id, &kind, stack.as_ref(), &mut result);
        match result? {
            mod_api::InteractionResult::Pass | mod_api::InteractionResult::TryWithEmptyHand => None,
            answer => Some(self.with_hand(hand, |g| g.interaction_consumed(answer))),
        }
    }

    pub(in crate::game) fn client_item_interaction(
        &mut self,
        hand: Hand,
        stack: &ItemStack,
    ) -> Option<bool> {
        if let Some(answer) = self.right_click_item(hand, stack) {
            if answer == mod_api::InteractionResult::Success {
                self.with_hand(hand, Game::swing_arm);
            }
            return Some(answer.consumes_action());
        }
        self.with_hand(hand, |g| g.use_item_in_hand(stack))
    }

    /// Dispatch the item Part and apply the result on this logical side.
    pub(in crate::game) fn use_item_in_hand(&mut self, stack: &ItemStack) -> Option<bool> {
        use mod_api::InteractionResult as Result;
        let mut result = None;
        if mod_api::__private::has_part_hook(mod_api::HookId::UseItem)
            && let Ok(snapshot) = stack.try_into_api()
        {
            let (player, player_entity_id) = self.native_clicker();
            self.native_use_item(mod_api::__private::UseItemCall {
                stack: &snapshot,
                hand: self.hand,
                player,
                player_entity_id,
                result: &mut result,
            });
        }
        match result? {
            Result::Pass | Result::TryWithEmptyHand => None,
            Result::Success => {
                if self.remote.is_some() {
                    self.swing_arm();
                }
                Some(true)
            }
            Result::SuccessServer => {
                if self.remote.is_none() {
                    self.swing_arm();
                }
                Some(true)
            }
            Result::Consume => Some(true),
            Result::Fail => Some(false),
            _ => None,
        }
    }
}
