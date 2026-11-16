//! Shared cauldron client selection. Prediction never invokes authoritative callbacks.
use super::remote::DryUse;
use super::*;
use horizon_client::cauldron::{self, Outcome, interactions::Entry};
use horizon_world::level::InteractionResult;

struct DryHand<'a> {
    creative: bool,
    hand: Option<ItemStack>,
    inventory: &'a [Option<ItemStack>],
}
impl cauldron::Player for DryHand<'_> {
    fn creative(&self) -> bool {
        self.creative
    }
    fn hand(&mut self) -> &mut Option<ItemStack> {
        &mut self.hand
    }
    fn add(&mut self, _: &mut ItemStack) -> bool {
        true
    }
    fn contains(&self, stack: &ItemStack) -> bool {
        self.inventory
            .iter()
            .flatten()
            .any(|have| have.same_item_same_components(stack))
    }
    fn drop_stack(&mut self, _: ItemStack) {}
}
impl Game<'_> {
    /// Both prediction routes use this snapshot and one selected callback. Custom handlers
    /// are not invoked locally; authoritative block updates supply the result.
    pub(super) fn client_cauldron_prediction(
        &self,
        pos: [i32; 3],
        held: Option<&ItemStack>,
    ) -> (Outcome, DryUse) {
        let pass = || (Outcome::Pass, DryUse::default());
        let Some(held) = held else {
            return pass();
        };
        if self.keys.held(Mapping::Sneak) {
            return pass();
        }
        let Some((state, underwater)) = self.cauldron_at(pos) else {
            return pass();
        };
        let table = cauldron_interactions::snapshot();
        let removes_dye = cauldron::removes_dye(self.items(), &held.item);
        let items = self.items();
        let entry = table.select(&state.name, &held.item, removes_dye, |tag| {
            items.item_in_tag(tag, &held.item)
        });
        // Custom handlers have no matching-build/resource-generation admission handshake.
        // Consume locally without running decide or predicting writes; wait for the server.
        if matches!(entry, Some(Entry::Mod(_))) {
            return (Outcome::Consume, DryUse::default());
        }
        let mut hand = DryHand {
            creative: self.creative,
            hand: Some(held.clone()),
            inventory: &self.inventory,
        };
        let result = cauldron::interact_selected(&state, underwater, entry.as_ref(), &mut hand);
        let at = horizon_core::position::BlockPos::new(pos[0], pos[1], pos[2]);
        let changes = result
            .block
            .and_then(|block| self.pack.assets.registry.resolve(&block).ok())
            .map(|block| (at, block))
            .into_iter()
            .collect();
        let sounds = result
            .sound
            .map(|sound| (at, Arc::from(sound), 1.0, 1.0))
            .into_iter()
            .collect();
        (result.outcome, (changes, sounds))
    }
    /// The visual prediction route uses the same decision as the item-use consumption guard.
    pub(super) fn remote_cauldron_reacts(
        &self,
        hit: &Hit,
        held: Option<&ItemStack>,
    ) -> Option<(InteractionResult, DryUse)> {
        let (outcome, changes) = self.client_cauldron_prediction(hit.pos, held);
        let answer = match outcome {
            Outcome::Pass => return None,
            Outcome::Consume => InteractionResult::Consume,
            Outcome::Success => InteractionResult::Success,
        };
        Some((answer, changes))
    }
}
