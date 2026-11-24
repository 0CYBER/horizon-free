//! The part of `Entity.applyEffectsFromBlocks` that puts a dropped item out —
//! water (`WaterFluid.entityInside`) and rain — with the hiss
//! `playEntityOnFireExtinguishedSound` plays. The burn itself (`Entity.baseTick`)
//! is [`super::ItemEntity::burning_tick`]'s, which runs before the block effects.
use super::ItemEntity;

impl ItemEntity {
    /// `Entity.applyEffectsFromBlocks`: water (`WaterFluid.entityInside`) and
    /// rain clear a burning item's fire, after the block contacts that may have
    /// ignited it. Returns whether the hiss plays:
    /// `playEntityOnFireExtinguishedSound` for an item that burned before the
    /// block effects and does not now.
    pub fn extinguish_if_wet(&mut self, in_water: bool, in_rain: bool, was_on_fire: bool) -> bool {
        if in_water || in_rain {
            // Entity.clearFire keeps only a fire-immune countdown.
            self.fire_ticks = self.fire_ticks.min(0);
        }
        was_on_fire && self.fire_ticks <= 0
    }
}
