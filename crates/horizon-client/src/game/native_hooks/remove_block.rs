use super::*;

impl GameLevel<'_> {
    pub(in crate::game) fn native_remove_block(&self, call: super::fire::RemoveBlockCall<'_>) {
        self.fire_remove_block(call);
    }

    pub(in crate::game) fn removal_replacement(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        moved_by_piston: bool,
        vanilla: BlockStateId,
    ) -> BlockStateId {
        if !self.native_authoritative {
            return vanilla;
        }
        let mut answer = None;
        self.native_remove_block(super::fire::RemoveBlockCall {
            dimension: self.dimension,
            pos,
            state,
            moved_by_piston,
            answer: &mut answer,
        });
        match answer {
            Some(replacement) if self.registry.state(replacement).is_some() => replacement,
            Some(replacement) => {
                log::warn!(
                    "remove_block refused invalid replacement state {}; using vanilla",
                    replacement.0
                );
                vanilla
            }
            None => vanilla,
        }
    }
}
