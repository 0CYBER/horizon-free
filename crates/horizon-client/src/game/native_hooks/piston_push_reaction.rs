use super::*;

impl Game<'_> {
    // See arismoko/horizon#3242: fold with resolve_block_paths when it lands
    pub(in crate::game) fn resolve_piston_reactions(&self) {
        let asked = mod_api::__private::has_scoped_hook(mod_api::HookId::PistonPushReaction);
        self.res.behaviours().resolve_piston_reactions(|state| {
            if !asked {
                return None;
            }
            let mut reaction = None;
            Self::native_piston_push_reaction(state, &mut reaction);
            reaction
                .as_ref()
                .map(|value| crate::game::api_values::push_reaction(value).1)
        });
    }
}

impl Game<'_> {
    /// Pure state-only callback, with no game receiver or Host.
    pub(in crate::game) fn native_piston_push_reaction(
        state: BlockStateId,
        reaction: &mut Option<mod_api::PushReaction>,
    ) {
        Self::fire_piston_push_reaction(state, reaction);
    }
}
