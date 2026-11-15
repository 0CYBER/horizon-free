//! The item model's surroundings (`item_model::Context`) as the values `item_model_update` gives a
//! mod: where the item is drawn, its owner and its level, as `ItemModel.update` reads them.
use super::api_values::TryIntoApi;
use horizon_assets::item_model::{
    DisplayContext, GlobalPos as EngineGlobalPos, LevelState, OwnerState,
};
use mod_api::{
    BlockPos, GlobalPos, Identifier, ItemDisplayContext, ItemModelLevel, ItemModelOwner,
};

impl TryIntoApi for DisplayContext {
    type Api = ItemDisplayContext;
    type Error = std::convert::Infallible;
    fn try_into_api(&self) -> Result<ItemDisplayContext, Self::Error> {
        Ok(match self {
            Self::None => ItemDisplayContext::None,
            Self::ThirdPersonLeftHand => ItemDisplayContext::ThirdPersonLeftHand,
            Self::ThirdPersonRightHand => ItemDisplayContext::ThirdPersonRightHand,
            Self::FirstPersonLeftHand => ItemDisplayContext::FirstPersonLeftHand,
            Self::FirstPersonRightHand => ItemDisplayContext::FirstPersonRightHand,
            Self::Head => ItemDisplayContext::Head,
            Self::Gui => ItemDisplayContext::Gui,
            Self::Ground => ItemDisplayContext::Ground,
            Self::Fixed => ItemDisplayContext::Fixed,
            Self::OnShelf => ItemDisplayContext::OnShelf,
        })
    }
}

/// A position in a dimension; `None` for a dimension key that isn't an identifier.
fn global_pos(pos: &EngineGlobalPos) -> Option<GlobalPos> {
    let [x, y, z] = pos.pos;
    Some(GlobalPos::new(
        Identifier::parse(&pos.dimension).ok()?,
        BlockPos::new(x, y, z),
    ))
}

impl TryIntoApi for OwnerState {
    type Api = ItemModelOwner;
    /// The entity type isn't an identifier.
    type Error = ();
    fn try_into_api(&self) -> Result<ItemModelOwner, ()> {
        Ok(ItemModelOwner {
            position: self.position,
            visual_rotation_y_degrees: self.visual_rotation_y_degrees,
            entity_type: Identifier::parse(&self.entity_type).map_err(|_| ())?,
            main_arm_left: self.main_arm_left,
            using_this_item: self.using_this_item,
            use_item_remaining_ticks: self.use_item_remaining_ticks,
            use_duration: self.use_duration,
            crossbow_charge_duration: self.crossbow_charge_duration,
            fishing_cast_with_this_item: self.fishing_cast_with_this_item,
            is_selected: self.is_selected,
            is_carried: self.is_carried,
            is_view_entity: self.is_view_entity,
            team_color: self.team_color,
            cooldown_percent: self.cooldown_percent,
            is_local_player: self.is_local_player,
            last_death_location: self.last_death_location.as_ref().and_then(global_pos),
        })
    }
}

impl TryIntoApi for LevelState {
    type Api = ItemModelLevel;
    /// The dimension key isn't an identifier.
    type Error = ();
    fn try_into_api(&self) -> Result<ItemModelLevel, ()> {
        Ok(ItemModelLevel {
            dimension: Identifier::parse(&self.dimension).map_err(|_| ())?,
            game_time: self.game_time,
            sun_angle: self.sun_angle,
            moon_phase: self.moon_phase,
            respawn: self.respawn.as_ref().and_then(global_pos),
            runs_normally: self.runs_normally,
        })
    }
}
