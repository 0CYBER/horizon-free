//! What `ItemModel.update` reads besides the stack (`item_model_update`): the living entity holding
//! the item and the level it is drawn in, as plain values. Absence stays `None`: an item in a
//! screen has no owner, and one drawn outside a world has no level.
use crate::{GlobalPos, Identifier};

/// The living entity holding the item (`ItemOwner`), as the item model reads it.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct ItemModelOwner {
    /// Its position.
    pub position: [f64; 3],
    /// `getVisualRotationYInDegrees`.
    pub visual_rotation_y_degrees: f32,
    /// Its entity type (`minecraft:player`).
    pub entity_type: Identifier,
    /// Whether its main arm is the left one.
    pub main_arm_left: bool,
    /// Whether it is using this very stack (`isUsingItem() && getUseItem() == stack`).
    pub using_this_item: bool,
    /// `getUseItemRemainingTicks`.
    pub use_item_remaining_ticks: i32,
    /// `stack.getUseDuration(owner)`.
    pub use_duration: i32,
    /// `CrossbowItem.getChargeDuration(stack, owner)`.
    pub crossbow_charge_duration: i32,
    /// Whether its fishing hook was cast with this item.
    pub fishing_cast_with_this_item: bool,
    /// Whether the stack is in its selected hotbar slot.
    pub is_selected: bool,
    /// Whether the stack is carried on the cursor.
    pub is_carried: bool,
    /// Whether it is the entity the camera views from.
    pub is_view_entity: bool,
    /// Its team's colour, `0xRRGGBB`, when it has one.
    pub team_color: Option<i32>,
    /// `ItemCooldowns.getCooldownPercent(stack, 0)`.
    pub cooldown_percent: f32,
    /// Whether it is this client's player.
    pub is_local_player: bool,
    /// Where it last died, when known.
    pub last_death_location: Option<GlobalPos>,
}

/// The level an item is drawn in (`ClientLevel`), as the item model reads it.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct ItemModelLevel {
    /// The dimension's key (`minecraft:overworld`).
    pub dimension: Identifier,
    /// `getGameTime`.
    pub game_time: i64,
    /// `EnvironmentAttributes.SUN_ANGLE` at the owner, in degrees.
    pub sun_angle: f32,
    /// `EnvironmentAttributes.MOON_PHASE` index at the owner.
    pub moon_phase: i32,
    /// The world's respawn point, when set.
    pub respawn: Option<GlobalPos>,
    /// `tickRateManager().runsNormally()`.
    pub runs_normally: bool,
}
