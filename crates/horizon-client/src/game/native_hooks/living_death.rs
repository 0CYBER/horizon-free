use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `LivingEntity.die`: the complete accepted killing source and its weapon,
    /// before any loot, experience or death message. Cancelling leaves health
    /// exactly where the listener left it; an unhealed body ticks away without loot.
    pub(in crate::game) fn native_living_death(
        &mut self,
        entity: u32,
        kind: &str,
        source: &HurtSource<'_>,
        weapon: Option<&mod_api::ItemStack>,
        cancel: &mut bool,
    ) {
        self.fire_living_death(entity, kind, source, weapon, cancel);
    }
}
/// `native_living_incoming_damage`'s entity, entity type and source for a hit.
pub(in crate::game) fn hurt_server_args<'a>(
    hurting: Hurting<'a>,
    attack: &Attack,
) -> (i32, &'a str, HurtSource<'a>) {
    match hurting {
        Hurting::Player(entity, source) => (entity, "minecraft:player", source),
        Hurting::Mob(mob, damage) => (
            mob.id as i32,
            mob.kind.id.as_str(),
            mob_source(damage, attack),
        ),
        Hurting::Dragon(entity, source) => (entity, "minecraft:ender_dragon", source),
        Hurting::ArmorStand(entity, source) => (entity, "minecraft:armor_stand", source),
        Hurting::Entity(entity, kind, source) => (entity, kind, source),
    }
}

pub(super) fn capture_hit(
    hurting: Hurting<'_>,
    attack: &Attack,
) -> Option<horizon_client::health::AcceptedHit> {
    let death = mod_api::__private::listened(mod_api::HookId::LivingDeath);
    let loot = mod_api::__private::listened(mod_api::HookId::LootDrops);
    if !death && !loot {
        return None;
    }
    let (committed, dying) = match &hurting {
        Hurting::Mob(mob, _) => (mob.last_damage, mob.health <= 0.0),
        _ => (None, true),
    };
    let (_, _, source) = hurt_server_args(hurting, attack);
    if !dying && !(loot && source.direct != 0 && attack.projectile == Some(source.direct)) {
        return None;
    }
    Some(horizon_client::health::AcceptedHit::capture(
        source, attack, committed,
    ))
}

impl Game<'_> {
    /// `LivingEntity.die`: a veto skips its whole tail, without supplying revive health.
    pub(in crate::game) fn mob_death_canceled(&mut self, mob: &horizon_client::mobs::Mob) -> bool {
        if mob.death_canceled {
            return true;
        }
        if !mod_api::__private::listened(mod_api::HookId::LivingDeath) {
            return false;
        }
        let accepted = mob
            .accepted_hit
            .as_ref()
            .filter(|hit| hit.belongs_to(mob.last_damage));
        let (_, causing) = self.mob_damage_source(mob);
        let attack = self.attack_of(causing as i32);
        let fallback = mob.last_damage.map_or(
            HurtSource {
                damage_type: "minecraft:generic",
                causing: 0,
                direct: 0,
                position: None,
            },
            |(damage, _)| mob_source(&damage, &attack),
        );
        let source = accepted.map_or(fallback, |hit| hit.source());
        let weapon = accepted
            .and_then(|hit| hit.weapon.as_ref())
            .or_else(|| attack.weapon_of(source.direct));
        let weapon = weapon.and_then(|stack| stack.try_into_api().ok());
        let kind = super::mob_kind(mob);
        let mut cancel = false;
        self.native_living_death(mob.id, &kind, &source, weapon.as_ref(), &mut cancel);
        if cancel && let Some(body) = self.mobs.iter_mut().find(|m| m.id == mob.id) {
            body.death_canceled = body.health <= 0.0;
            if !body.death_canceled {
                body.death_time = 0;
                self.died.remove(&mob.id);
            }
        }
        cancel
    }
}
