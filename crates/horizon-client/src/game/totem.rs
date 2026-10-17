//! The totem of undying (`LivingEntity.checkTotemDeathProtection` with the
//! totem's `death_protection` component, entity event 35 on the client).
use super::Game;
use horizon_client::{
    effects::EffectInstance,
    particles::{ParticleOptions, ParticleType},
};

const TOTEM: &str = "minecraft:totem_of_undying";

impl Game<'_> {
    /// Whether either hand holds a totem, for the next hurt.
    pub(super) fn update_totem_ready(&mut self) {
        let ready = self.held().is_some_and(|s| &*s.item == TOTEM)
            || self.offhand.as_ref().is_some_and(|s| &*s.item == TOTEM);
        if let Some(p) = self.player.as_mut() {
            p.health.totem_ready = ready;
        }
    }
    /// A totem fired: use it up (main hand first), set health to 1, clear and
    /// apply its effects, then the sound and particles.
    pub(super) fn apply_totem(&mut self) {
        let Some(p) = self.player.as_mut() else {
            return;
        };
        if !p.health.totem_used {
            return;
        }
        p.health.totem_used = false;
        let _ = p.set_health(1.0, false);
        let at = [p.position.x, p.position.y, p.position.z];
        let height = f64::from(p.dimensions.height);
        let slot = if self.held().is_some_and(|s| &*s.item == TOTEM) {
            Some(self.selected)
        } else {
            None
        };
        let cell = match slot {
            Some(i) => &mut self.inventory[i],
            None => &mut self.offhand,
        };
        if let Some(s) = cell.as_mut() {
            s.count -= 1;
            if s.count == 0 {
                *cell = None;
            }
        }
        // DeathProtection.TOTEM_EFFECTS: clear, then regeneration II, absorption II, fire resistance.
        self.potions.effects.clear();
        for (id, ticks, amp) in [
            ("regeneration", 900, 1),
            ("absorption", 100, 1),
            ("fire_resistance", 800, 0),
        ] {
            if let Some(e) = horizon_client::brewing::effect(id) {
                self.add_player_effect(EffectInstance::new(e.id, ticks, amp));
            }
        }
        self.audio
            .play("minecraft:item.totem.use", Some(at), 1.0, 1.0);
        self.trigger_item(
            "minecraft:used_totem",
            &horizon_client::items::ItemStack::new("minecraft:totem_of_undying", 1),
        );
        self.particles.tracking_emitter_for(
            ParticleOptions::simple(ParticleType::TotemOfUndying),
            at,
            0.6,
            height,
            30,
        );
        self.pad_rumble(horizon_client::gamepad::Rumble::Totem);
    }
}
