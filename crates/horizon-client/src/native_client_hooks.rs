//! Callbacks into the executable for native trait mods' client-library hooks.
use crate::items::ItemStack;
use horizon_core::position::BlockPos;
use horizon_world::native_load_context::LoopContext;
use horizon_world::registry::BlockStateId;
use mod_api::{BlockSoundCause, BlockSoundType};
use std::sync::OnceLock;

type CanSwimInFluid = fn(&LoopContext, u32, &str, &str, bool, &mut bool);
type LivingFluidTravel = fn(crate::motion::LivingFluidTravelCall<'_>);

/// Client-library hooks, installed before any menu, entity or recipe is used.
pub struct NativeClientHooks {
    pub menu_still_valid: fn(&LoopContext, u128, &str, [i32; 3], [f64; 3], &mut bool),
    pub menu_still_valid_listens: fn(&str) -> bool,
    pub item_stacked_on_other: fn(&LoopContext, &mut crate::menu::StackedOnOther<'_>, &mut bool),
    pub living_heal: fn(&LoopContext, u32, &mut f32, &mut bool),
    /// `living_heal_cause`: the entity, the cause name and the amount applied.
    pub living_heal_cause: fn(&LoopContext, u32, &str, f32),
    pub living_breathe: fn(&LoopContext, u32, &str, &mut bool, &mut i32, &mut i32),
    /// `player_login_profile`: a login's UUID, name and profile properties, before the login
    /// success sends them. The login thread calls it, so the callback has no host.
    pub player_login_profile: fn(u128, &str, &mut Vec<mod_api::ProfileProperty>),
    pub farmland_trample: fn(&LoopContext, u32, [i32; 3], u32, f64, &mut bool),
    pub has_mods: fn() -> bool,
    pub player_name_format: fn(&LoopContext, &crate::text::Text, &mut crate::text::Text),
    pub player_tab_list_name_format: fn(&LoopContext, u128, &str, &mut Option<crate::text::Text>),
    pub living_drown: fn(&LoopContext, u32, &str, &mut bool, &mut f32, &mut i32, &mut bool),
    pub living_fall: fn(&LoopContext, u32, &str, &mut f64, &mut f32, &mut bool),
    pub living_jump: fn(&LoopContext, u32, &str),
    pub living_jump_power: fn(&LoopContext, u32, &str, &mut f32),
    pub living_speed: fn(&LoopContext, u32, &str, &mut f32),
    pub living_flying_speed: fn(&LoopContext, u32, &str, &mut f32),
    pub can_swim_in_fluid_listened: fn() -> bool,
    pub can_swim_in_fluid: CanSwimInFluid,
    pub living_fluid_travel_listened: fn() -> bool,
    pub living_fluid_travel: LivingFluidTravel,
    pub block_jump_factor: fn(&LoopContext, BlockStateId, &mut f32),
    pub block_speed_factor: fn(&LoopContext, BlockStateId, &mut f32),
    pub is_ladder: fn(&LoopContext, u32, BlockPos, BlockStateId, &mut bool),
    pub makes_open_trapdoor_above_climbable:
        fn(&LoopContext, BlockStateId, BlockPos, BlockStateId, &mut bool),
    pub friction: fn(&LoopContext, u32, &str, BlockPos, BlockStateId, &mut f32),
    pub player_stays_on_ground_surface: fn(&LoopContext, u32, &mut bool),
    pub ender_entity_teleport: fn(&LoopContext, u32, &str, [f64; 3], &mut [f64; 3], &mut bool),
    pub fluid_flow: fn(&LoopContext, &str, BlockPos, &mut [f64; 3]),
    pub sound_type: fn(
        &LoopContext,
        BlockPos,
        BlockStateId,
        Option<(u32, &str)>,
        BlockSoundCause,
        &mut BlockSoundType,
    ),
    /// `ItemPart::can_grindstone_repair` of the stack's item; `can_repair` arrives with
    /// vanilla's.
    pub can_grindstone_repair: fn(&LoopContext, &ItemStack, &mut bool),
    /// `ItemPart::is_combine_repairable` of the stack's item; `repairable` arrives with vanilla's.
    pub is_combine_repairable: fn(&LoopContext, &ItemStack, &mut bool),
    /// `ItemPart::can_fit_inside_container_items` of the stack's item; `fits` arrives with
    /// vanilla's.
    pub can_fit_inside_container_items: fn(&LoopContext, &ItemStack, &mut bool),
}

static HOOKS: OnceLock<NativeClientHooks> = OnceLock::new();

#[cfg(test)]
#[path = "native_client_hooks_tests.rs"]
pub(crate) mod test_hooks;

/// Install the process-wide callbacks once; a second installation keeps the first and answers false.
pub fn install(hooks: NativeClientHooks) -> bool {
    HOOKS.set(hooks).is_ok()
}

/// Ask the installed adapter for a mod menu type's `stillValid` answer; with
/// none installed, `valid` keeps the game's answer.
pub(crate) fn menu_still_valid(
    context: &LoopContext,
    player: u128,
    menu: &str,
    pos: [i32; 3],
    eye: [f64; 3],
    valid: &mut bool,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.menu_still_valid)(context, player, menu, pos, eye, valid);
    }
}

/// Whether a part on the menu type `menu` answers `menu_still_valid`.
pub(crate) fn menu_still_valid_listens(menu: &str) -> bool {
    HOOKS
        .get()
        .is_some_and(|hooks| (hooks.menu_still_valid_listens)(menu))
}

pub(crate) fn item_stacked_on_other(
    context: &LoopContext,
    event: &mut crate::menu::StackedOnOther<'_>,
    canceled: &mut bool,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.item_stacked_on_other)(context, event, canceled);
    }
}

pub(crate) fn living_heal(context: &LoopContext, entity: u32, amount: &mut f32, cancel: &mut bool) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.living_heal)(context, entity, amount, cancel);
    }
}

pub(crate) fn living_heal_cause(context: &LoopContext, entity: u32, cause: &str, amount: f32) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.living_heal_cause)(context, entity, cause, amount);
    }
}

pub(crate) fn living_breathe(
    context: &LoopContext,
    entity: u32,
    kind: &str,
    can_breathe: &mut bool,
    consume: &mut i32,
    refill: &mut i32,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.living_breathe)(context, entity, kind, can_breathe, consume, refill);
    }
}

pub(crate) fn player_login_profile(
    player: u128,
    name: &str,
    properties: &mut Vec<mod_api::ProfileProperty>,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.player_login_profile)(player, name, properties);
    }
}

pub(crate) fn farmland_trample(
    context: &LoopContext,
    entity: u32,
    pos: [i32; 3],
    state: u32,
    distance: f64,
    cancel: &mut bool,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.farmland_trample)(context, entity, pos, state, distance, cancel);
    }
}

pub(crate) fn has_mods() -> bool {
    HOOKS.get().is_some_and(|hooks| (hooks.has_mods)())
}

pub(crate) fn player_name_format(
    context: &LoopContext,
    username: &crate::text::Text,
    displayname: &mut crate::text::Text,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.player_name_format)(context, username, displayname);
    }
}

pub(crate) fn player_tab_list_name_format(
    context: &LoopContext,
    player: u128,
    name: &str,
    display_name: &mut Option<crate::text::Text>,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.player_tab_list_name_format)(context, player, name, display_name);
    }
}

pub(crate) fn living_drown(
    context: &LoopContext,
    entity: u32,
    kind: &str,
    is_drowning: &mut bool,
    damage_amount: &mut f32,
    bubble_count: &mut i32,
    cancel: &mut bool,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.living_drown)(
            context,
            entity,
            kind,
            is_drowning,
            damage_amount,
            bubble_count,
            cancel,
        );
    }
}

pub(crate) fn living_fall(
    context: &LoopContext,
    entity: u32,
    kind: &str,
    distance: &mut f64,
    multiplier: &mut f32,
    cancel: &mut bool,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.living_fall)(context, entity, kind, distance, multiplier, cancel);
    }
}

pub(crate) fn living_jump(context: &LoopContext, entity: u32, kind: &str) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.living_jump)(context, entity, kind);
    }
}

pub(crate) fn living_jump_power(context: &LoopContext, entity: u32, kind: &str, power: &mut f32) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.living_jump_power)(context, entity, kind, power);
    }
}

pub(crate) fn living_speed(context: &LoopContext, entity: u32, kind: &str, speed: &mut f32) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.living_speed)(context, entity, kind, speed);
    }
}

pub(crate) fn living_flying_speed(context: &LoopContext, entity: u32, kind: &str, speed: &mut f32) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.living_flying_speed)(context, entity, kind, speed);
    }
}

pub(crate) fn can_swim_in_fluid_listened() -> bool {
    HOOKS
        .get()
        .is_some_and(|hooks| (hooks.can_swim_in_fluid_listened)())
}

pub(crate) fn can_swim_in_fluid() -> Option<CanSwimInFluid> {
    HOOKS.get().map(|hooks| hooks.can_swim_in_fluid)
}

pub(crate) fn living_fluid_travel_listened() -> bool {
    HOOKS
        .get()
        .is_some_and(|hooks| (hooks.living_fluid_travel_listened)())
}

pub(crate) fn living_fluid_travel() -> Option<LivingFluidTravel> {
    HOOKS.get().map(|hooks| hooks.living_fluid_travel)
}

pub(crate) fn block_jump_factor(context: &LoopContext, state: BlockStateId, factor: &mut f32) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.block_jump_factor)(context, state, factor);
    }
}

pub(crate) fn block_speed_factor(context: &LoopContext, state: BlockStateId, factor: &mut f32) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.block_speed_factor)(context, state, factor);
    }
}

pub(crate) fn is_ladder(
    context: &LoopContext,
    entity: u32,
    pos: BlockPos,
    state: BlockStateId,
    ladder: &mut bool,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.is_ladder)(context, entity, pos, state, ladder);
    }
}

pub(crate) fn makes_open_trapdoor_above_climbable(
    context: &LoopContext,
    state: BlockStateId,
    pos: BlockPos,
    trapdoor: BlockStateId,
    climbable: &mut bool,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.makes_open_trapdoor_above_climbable)(context, state, pos, trapdoor, climbable);
    }
}

pub(crate) fn friction(
    context: &LoopContext,
    entity: u32,
    kind: &str,
    pos: BlockPos,
    state: BlockStateId,
    friction: &mut f32,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.friction)(context, entity, kind, pos, state, friction);
    }
}

pub(crate) fn player_stays_on_ground_surface(
    context: &LoopContext,
    entity: u32,
    staying: &mut bool,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.player_stays_on_ground_surface)(context, entity, staying);
    }
}

pub(crate) fn ender_entity_teleport(
    context: &LoopContext,
    entity: u32,
    kind: &str,
    from: [f64; 3],
    target: &mut [f64; 3],
    cancel: &mut bool,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.ender_entity_teleport)(context, entity, kind, from, target, cancel);
    }
}

pub(crate) fn fluid_flow(context: &LoopContext, fluid: &str, pos: BlockPos, flow: &mut [f64; 3]) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.fluid_flow)(context, fluid, pos, flow);
    }
}

pub(crate) fn sound_type(
    context: &LoopContext,
    pos: BlockPos,
    state: BlockStateId,
    entity: Option<(u32, &str)>,
    cause: BlockSoundCause,
    sound: &mut BlockSoundType,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.sound_type)(context, pos, state, entity, cause, sound);
    }
}

/// Ask the installed adapter for the parts of `stack`'s item (`can_grindstone_repair`); with none
/// installed, `can_repair` keeps vanilla's answer.
pub(crate) fn can_grindstone_repair(
    context: &LoopContext,
    stack: &ItemStack,
    can_repair: &mut bool,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.can_grindstone_repair)(context, stack, can_repair);
    }
}

/// Ask the installed adapter for the parts of `stack`'s item (`is_combine_repairable`); with none
/// installed, `repairable` keeps vanilla's answer.
pub(crate) fn is_combine_repairable(
    context: &LoopContext,
    stack: &ItemStack,
    repairable: &mut bool,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.is_combine_repairable)(context, stack, repairable);
    }
}

/// Ask the installed adapter for the parts of `stack`'s item (`can_fit_inside_container_items`);
/// with none installed, `fits` keeps vanilla's answer.
pub(crate) fn can_fit_inside_container_items(
    context: &LoopContext,
    stack: &ItemStack,
    fits: &mut bool,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.can_fit_inside_container_items)(context, stack, fits);
    }
}
