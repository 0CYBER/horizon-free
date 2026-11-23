//! `Mob.doHurtTarget` for native mod handles: a mob's melee blow, run through the same combat path
//! a melee goal's blow takes.
use super::{Game, commands::Target};
use horizon_client::mobs::{self, Who, ai::do_hurt_target, goals::Cx};
use mod_api::{LivingRef, MobRef};

impl Game<'_> {
    /// `Mob.doHurtTarget(target)`, as `MeleeAttack` calls it after its own swing: the blow with
    /// the held weapon, its knockback and the species' effects, dealt through the mob's combat
    /// tick context (`mob_reset_goals` builds the same one). It runs no swing, cooldown or reach
    /// check. True means the blow was dealt; a shield or armour may still take it. False for a
    /// stale or unavailable mob or target, a client, the dragon, a target `native_mob_valid_target`
    /// refuses, a mob that names itself, and a guest target.
    pub(super) fn mob_attack(&mut self, mob: MobRef, target: LivingRef) -> bool {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        let Some(kind) = self.native_entity_target(target.entity()) else {
            return false;
        };
        let who = match kind {
            Target::Player => Who::Player,
            Target::Mob(other) if other != id => Who::Mob(other),
            _ => return false,
        };
        let Some(index) = self.mobs.iter().position(|m| m.id == id && !m.removed) else {
            return false;
        };
        if !self.native_mob_valid_target(&self.mobs[index], kind) {
            return false;
        }
        let snapshot: Vec<_> = self
            .mobs
            .iter()
            .map(|m| super::creatures::seen(m, self.level.player_uuid, self.items()))
            .collect();
        let mut mobs = self.mobs.lift();
        let mut lent = horizon_client::health::LentLevel::take(&mut self.world);
        let mut w = self.mob_world_base(&snapshot);
        let pos = mobs[index].position;
        w.difficulty = self.local_difficulty([pos.x, pos.y, pos.z]);
        let mut events = Vec::new();
        {
            let mut level = horizon_client::health::DamageLevel::lent(
                &self.world,
                self.pack.assets.registry,
                &mut lent,
                Some(world_items!(self)),
                Some(&self.pack.block_registries),
                &self.damage_hooks,
            );
            let m = &mut mobs[index];
            let mut cx = Cx {
                w: &w,
                level: &mut level,
                events: &mut events,
            };
            do_hurt_target(m, who, &mut cx);
            mobs::apply_brain_events(&mut mobs, &mut level, &mut events, 0);
        }
        drop(w);
        lent.give_back(&mut self.world);
        self.mobs
            .restore(mobs, horizon_client::join_seq::RestoreOrder::SurvivorsFirst);
        self.handle_mob_events(events).ok();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use super::*;
    use horizon_client::mobs::Mob;
    use horizon_core::geometry::Vec3;
    use mod_api::{EntityRef, WorldEpoch};

    /// A zombie (id 12) hits a zombie (id 13) for its 3.0 attack damage, and the struck one loses
    /// exactly that; a mob that names itself, a dragon and a stale handle are refused.
    #[test]
    #[ignore = "daily"]
    fn a_mob_blow_hits_the_target_with_its_own_attack_damage() {
        with_game(|game| {
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(12, zombie.clone(), Vec3::new(3.5, 65.0, 3.5), 0.0));
            game.mobs
                .push(Mob::new(13, zombie, Vec3::new(4.5, 65.0, 3.5), 0.0));
            let attacker = game.mob(game.entity(12).unwrap()).unwrap();
            let victim = game.living(game.entity(13).unwrap()).unwrap();
            let before = game.mobs[1].health;
            assert!(game.mob_attack(attacker, victim));
            assert_eq!(game.mobs[1].health, before - 3.0);
            let itself = game.living(game.entity(12).unwrap()).unwrap();
            assert!(!game.mob_attack(attacker, itself));
            let stale = MobRef::new(EntityRef::new(
                WorldEpoch::new(attacker.entity().epoch().get() + 1),
                12,
            ));
            assert!(!game.mob_attack(stale, victim));
            assert_eq!(game.mobs[1].health, before - 3.0);
        });
    }
}
