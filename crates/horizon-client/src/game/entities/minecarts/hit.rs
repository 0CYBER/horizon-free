//! `VehicleEntity.hurtServer`'s `DamageSource` as far as a minecart reads it,
//! and the tail of an accepted hit: the `ENTITY_DAMAGE` game event, the drop's
//! `CustomName` (`destroy`) and `chestVehicleDestroyed`'s angry piglins.
use super::*;
use crate::game::commands::Target;

/// The `DamageSource` a cart took. `VehicleEntity.hurtServer` attributes its
/// `ENTITY_DAMAGE` to `source.getEntity()`, and `chestVehicleDestroyed` angers
/// piglins only when `source.getDirectEntity()` is the player.
#[derive(Clone, Copy, Default)]
pub(in crate::game) struct CartHit {
    pub(in crate::game) by: Option<Target>,
    pub(in crate::game) by_player: bool,
}

impl CartHit {
    /// A projectile's hit: `getEntity()` is its owner, `getDirectEntity()` the
    /// projectile itself, so it angers no piglin.
    pub(in crate::game) fn projectile(owner: Option<Who>, guest: Option<u32>) -> Self {
        let by = match owner {
            Some(Who::Player) => Some(guest.map_or(Target::Player, Target::Guest)),
            Some(Who::Mob(id)) => Some(Target::Mob(id)),
            None => None,
        };
        Self {
            by,
            by_player: false,
        }
    }
    /// The acting player's own hit (`Player.attack`'s `playerAttack`): its
    /// direct entity is the player in place (the host, or the guest whose turn
    /// it is), whose state `anger_piglins` reads.
    pub(in crate::game) fn player() -> Self {
        Self {
            by: Some(Target::Player),
            by_player: true,
        }
    }
    /// `/damage`'s `DamageSource`: `by` is its direct entity, `causing` the
    /// causing entity `getEntity()` names.
    pub(in crate::game) fn command(by: Option<Target>, causing: Option<Target>) -> Self {
        Self {
            by: causing,
            by_player: matches!(by, Some(Target::Player)),
        }
    }
    /// A hit whose causing entity is `by` and whose direct entity is no player:
    /// a thrown trident, or a blast's indirect source.
    pub(in crate::game) fn attributed(by: Option<Target>) -> Self {
        Self {
            by,
            by_player: false,
        }
    }
}

/// The tail of a cart's `hurtServer`.
impl Game<'_> {
    /// `VehicleEntity.hurtServer`: shake, add the damage, break past 40 (at
    /// once for creative players); TNT minecarts prime instead when moving
    /// or hurt by fire or an explosion (`damageSourceIgnitesTnt`).
    pub(in crate::game) fn hurt_cart(
        &mut self,
        i: usize,
        damage: f32,
        creative: bool,
        fire: bool,
        hit: CartHit,
    ) {
        self.hurt_cart_result(i, damage, creative, fire, hit);
    }
    /// `VehicleEntity.destroy(level, dropItem)`: the cart's item, carrying the
    /// vehicle's `CustomName` as `DataComponents.CUSTOM_NAME`.
    pub(super) fn cart_drop(&self, kind: CartKind, id: u32) -> ItemStack {
        let mut stack = ItemStack::new(kind.item(), 1);
        if let Some(name) = self.persist_raw(id).and_then(|raw| raw.get("CustomName")) {
            let text = horizon_client::entity_nbt::name_text(name);
            stack.data_mut().custom_name =
                Some(horizon_client::scoreboard::component_tag(&text.component()));
        }
        stack
    }
    /// `chestVehicleDestroyed`'s `angerNearbyPiglins`, for a container cart
    /// (`AbstractMinecartContainer`) that the hit's direct entity, a player,
    /// broke.
    pub(super) fn cart_destroy_angers(&mut self, hit: CartHit, kind: CartKind) {
        if hit.by_player && kind.slots() > 0 {
            self.anger_piglins(true);
        }
    }
}

#[cfg(test)]
#[path = "minecart_hit_tests.rs"]
mod hit_tests;
