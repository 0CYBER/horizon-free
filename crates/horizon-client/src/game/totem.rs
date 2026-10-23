//! The totem of undying (`LivingEntity.checkTotemDeathProtection` with the
//! totem's `death_protection` component, entity event 35 on the client).
use super::Game;
use horizon_client::{
    effects::EffectInstance,
    particles::{ParticleOptions, ParticleType},
};

const TOTEM: &str = "minecraft:totem_of_undying";

/// `EntityEvent.TOTEM_OF_UNDYING`.
pub(super) const TOTEM_EVENT: i8 = 35;

impl Game<'_> {
    /// Whether either hand holds a totem, for the next hurt: the window's
    /// player once a tick, a server's player for each of its turns.
    pub(super) fn update_totem_ready(&mut self) {
        let ready = self.held().is_some_and(|s| &*s.item == TOTEM)
            || self.offhand.as_ref().is_some_and(|s| &*s.item == TOTEM);
        if let Some(p) = self.player.as_mut() {
            p.health.totem_ready = ready;
        }
    }
    /// The server's half once a totem fired: use it up (main hand first),
    /// count it, set health to 1, then clear and apply its effects. Returns
    /// the player's body, for entity event 35, which the caller broadcasts.
    pub(super) fn apply_totem(&mut self) -> Option<Body> {
        let p = self.player.as_mut()?;
        if !p.health.totem_used {
            return None;
        }
        p.health.totem_used = false;
        let _ = p.set_health(1.0, false);
        let body = Body {
            at: [p.position.x, p.position.y, p.position.z],
            width: f64::from(p.dimensions.width),
            height: f64::from(p.dimensions.height),
        };
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
        self.stats.add("used", TOTEM, 1);
        self.trigger_item(
            "minecraft:used_totem",
            &horizon_client::items::ItemStack::new(TOTEM, 1),
        );
        // ItemStack.causeUseVibration: the totem's use effects make one.
        let ctx = self.player_context(None);
        self.post_game_event("minecraft:item_interact_finish", body.at, ctx);
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
        Some(body)
    }
    /// The window's player's totem: the server's half, then the event to
    /// the guests and to this window's own client.
    pub(super) fn tick_totem(&mut self) {
        let Some(body) = self.apply_totem() else {
            return;
        };
        if let Some(host) = &self.lan {
            host.hub
                .entity_event(host.host_entity, TOTEM_EVENT, body.at);
        }
        self.totem_event(body, true);
    }
    /// Entity event 35 from a server, for this client's own player or an
    /// entity it draws.
    pub(super) fn remote_totem(&mut self, id: i32) {
        let own = self.remote.as_ref().is_some_and(|r| r.entity_id == id);
        let body = if own {
            self.player.as_ref().map(|p| Body {
                at: [p.position.x, p.position.y, p.position.z],
                width: f64::from(p.dimensions.width),
                height: f64::from(p.dimensions.height),
            })
        } else {
            self.mobs.iter().find(|m| m.id == id as u32).map(|m| Body {
                at: [m.position.x, m.position.y, m.position.z],
                width: f64::from(m.width()),
                height: f64::from(m.height()),
            })
        };
        if let Some(body) = body {
            self.totem_event(body, own);
        }
    }
    /// `ClientPacketListener.handleEntityEvent` 35: the totem particles (a
    /// 30-tick emitter) and its sound, heard here only (`playLocalSound`),
    /// and the player's own rumble.
    pub(super) fn totem_event(&mut self, body: Body, own: bool) {
        self.particles.tracking_emitter_for(
            ParticleOptions::simple(ParticleType::TotemOfUndying),
            body.at,
            body.width,
            body.height,
            30,
        );
        self.audio
            .play_unshared("minecraft:item.totem.use", Some(body.at), 1.0, 1.0);
        if own {
            self.pad_rumble(horizon_client::gamepad::Rumble::Totem);
        }
    }
}

/// Where an entity event's entity stands and its size.
#[derive(Clone, Copy)]
pub(super) struct Body {
    pub at: [f64; 3],
    width: f64,
    height: f64,
}
