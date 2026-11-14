// Included in creatures/mod.rs to preserve its existing scope.
impl horizon_client::mobs::MobHooks for Game<'_> {
    fn block_path_type(
        &self,
        state: horizon_world::registry::BlockStateId,
        pos: [i32; 3],
    ) -> Option<mod_api::PathType> {
        if !self.is_logical_server()
            || !crate::game::api_values::block_state::block_listens(
                mod_api::HookId::BlockPathType,
                state,
            )
        {
            return None;
        }
        let mut answer = None;
        if let Err(error) = horizon_client::crash::contain(|| {
            self.native_block_path_type(
                horizon_core::position::BlockPos::new(pos[0], pos[1], pos[2]),
                state,
                &mut answer,
            );
        }) {
            log::warn!("block_path_type callback failed: {error}");
            return None;
        }
        answer
    }
    fn adjacent_block_path_type(
        &self,
        state: horizon_world::registry::BlockStateId,
        pos: [i32; 3],
        path_type: mod_api::PathType,
        default: Option<mod_api::PathType>,
    ) -> Option<mod_api::PathType> {
        if !self.is_logical_server()
            || !crate::game::api_values::block_state::block_listens(
                mod_api::HookId::AdjacentBlockPathType,
                state,
            )
        {
            return default;
        }
        let mut answer = None;
        if let Err(error) = horizon_client::crash::contain(|| {
            self.native_adjacent_block_path_type(
                horizon_core::position::BlockPos::new(pos[0], pos[1], pos[2]),
                state,
                path_type,
                &mut answer,
            );
        }) {
            log::warn!("adjacent_block_path_type callback failed: {error}");
            return default;
        }
        answer
    }
    fn living_change_target(
        &self,
        mob: &mut Mob,
        world: &MobWorld<'_>,
        is_brain: bool,
        target: Option<Who>,
    ) -> horizon_client::mobs::TargetChange {
        self.native_mob_change_target_in_tick(mob, world, is_brain, target)
    }

    fn enderman_anger_suppressed(&self, enderman: u32, player: &PlayerView) -> bool {
        if !mod_api::__private::listened(mod_api::HookId::EndermanAnger) {
            return false;
        }
        let Some(epoch) = self.native_lifecycle.epoch() else {
            return false;
        };
        let uuid = horizon_client::lan::playerdata::ints_uuid(player.uuid);
        let Some(player) = self.player(uuid) else {
            return false;
        };
        let mut cancel = false;
        self.native_enderman_anger(
            mod_api::EntityRef::new(epoch, enderman),
            player,
            &mut cancel,
        );
        cancel
    }
    fn can_grief(&self, mob: u32, rule: bool) -> bool {
        self.entity_grief(mob, rule)
    }
    fn grief_hooked(&self) -> bool {
        Game::grief_hooked(self)
    }
    fn can_entity_destroy(&self, mob: u32, pos: [i32; 3], vanilla: bool) -> bool {
        self.entity_may_destroy(mob, pos, vanilla)
    }
    fn living_visibility(&self, player: &PlayerView, looking: u32, visibility: f64) -> f64 {
        let uuid = horizon_client::lan::playerdata::ints_uuid(player.uuid);
        match self.player(uuid) {
            Some(player) => self.scaled_visibility(|_| player.entity(), looking, visibility),
            None => visibility,
        }
    }
    fn living_visibility_mob(&self, target: u32, looking: u32, visibility: f64) -> f64 {
        self.scaled_visibility(
            |epoch| mod_api::EntityRef::new(epoch, target),
            looking,
            visibility,
        )
    }
}
