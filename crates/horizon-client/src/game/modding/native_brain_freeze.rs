//! A mob whose AI a mod paused while its physics runs on, for
//! `MobRef::set_brain_frozen` (#4478): Apothic Spawners' movable no-AI
//! modifier, which in Java keeps a spawned mob from thinking but not from
//! falling, swimming or being pushed. Goal removal covers goal mobs; a Brain
//! mob (a piglin, a villager, an axolotl, a goat) has no goals to take out.
//!
//! The flag (`Ai::brain_frozen`) is read in one place, the mob's own
//! `LivingEntity.aiStep` (`mobs::ai::ai_step`), as one more `isImmobile`
//! reason: `serverAiStep` is skipped whole and the input is zeroed, so
//! `travel` runs with none. Vanilla never sets it: only this module writes it,
//! a mob starts and loads with it false, and nothing saves it.
//!
//! Server authority: the handle resolves through `native_entity_target`,
//! which answers nothing on a client or for a mob held out for its own tick.
use super::{Game, commands::Target};
use mod_api::{BrainFreezeRefusal, MobRef};

impl Game<'_> {
    /// Whether a mod paused the mob's AI. None for a stale or unavailable
    /// handle, anything but a mob, the dragon, or on a client.
    pub(super) fn mob_brain_frozen(&self, mob: MobRef) -> Option<bool> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        let m = self.mobs.iter().find(|m| m.id == id && !m.removed)?;
        Some(m.ai.brain_frozen)
    }

    /// Pauses or resumes the mob's `serverAiStep` while `aiStep`'s physics
    /// runs on. Refused, with nothing changed, as `BrainFreezeRefusal` names.
    pub(super) fn mob_set_brain_frozen(
        &mut self,
        mob: MobRef,
        frozen: bool,
    ) -> Result<(), BrainFreezeRefusal> {
        let id = match self.native_entity_target(mob.entity()) {
            Some(Target::Mob(id)) => id,
            Some(_) => return Err(BrainFreezeRefusal::NotAMob),
            None => return Err(BrainFreezeRefusal::NoMob),
        };
        let m = self
            .mobs
            .iter_mut()
            .find(|m| m.id == id && !m.removed)
            .ok_or(BrainFreezeRefusal::NoMob)?;
        m.ai.brain_frozen = frozen;
        if frozen {
            // LivingEntity.aiStep's isImmobile branch: no jump or input is left over.
            m.ai.jumping = false;
            m.ai.xxa = 0.0;
            m.ai.zza = 0.0;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::{bind_local_player, with_game};
    use horizon_client::mobs::Mob;
    use horizon_client::motion::PlayerMotion;
    use horizon_core::geometry::Vec3;
    use mod_api::{BrainFreezeRefusal, EntityRef, MobRef, WorldEpoch};

    fn add(game: &mut super::Game<'_>, id: u32, name: &str, x: f64) -> MobRef {
        let kind = game.res.mob_types().get(name).unwrap().clone();
        game.mobs
            .push(Mob::new(id, kind, Vec3::new(x, 100.0, 0.5), 0.0));
        game.mob(game.entity(id).unwrap()).unwrap()
    }

    fn mob<'a>(game: &'a super::Game<'_>, handle: MobRef) -> &'a Mob {
        let id = handle.entity().id();
        game.mobs.iter().find(|m| m.id == id).unwrap()
    }

    /// A frozen villager (a Brain mob) and a frozen cow (a goal mob) run no
    /// `serverAiStep` yet fall and carry a push; unfrozen they think again.
    /// The witness is `noActionTime`: near a player `checkDespawn` sets it to
    /// 0 before each tick and only `serverAiStep` adds to it.
    #[test]
    #[ignore = "daily"]
    fn a_frozen_brain_or_goal_mob_falls_and_is_pushed_but_does_not_think() {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            for (x, z) in (-1..=1).flat_map(|x| (-1..=1).map(move |z| (x, z))) {
                let mut chunk = horizon_client::chunk::Chunk::empty(x, z);
                chunk.lit = true;
                game.world.chunks.insert((x, z), std::sync::Arc::new(chunk));
            }
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            bind_local_player(game, 20, 123);
            let frozen = [
                add(game, 30, "minecraft:villager", 0.5),
                add(game, 31, "minecraft:cow", 4.5),
            ];
            let thinking = [
                add(game, 32, "minecraft:villager", -4.5),
                add(game, 33, "minecraft:cow", 8.5),
            ];
            for handle in frozen {
                assert_eq!(handle.brain_frozen(game), Some(false));
                assert_eq!(handle.set_brain_frozen(game, true), Ok(()));
                assert_eq!(handle.brain_frozen(game), Some(true));
                // NoAI is a separate flag, neither read nor set.
                assert_eq!(handle.no_ai(game), Some(false));
            }
            let pushed = frozen[0];
            let id = pushed.entity().id();
            let m = game.mobs.iter_mut().find(|m| m.id == id).unwrap();
            m.velocity = Vec3::new(0.4, 0.0, 0.0);
            for _ in 0..10 {
                game.tick_mobs().unwrap();
            }
            for handle in frozen {
                let m = mob(game, handle);
                assert_eq!(m.ai.no_action_time, 0, "{}: no AI step", m.kind.id);
                assert!(m.position.y < 99.0, "{}: gravity still applies", m.kind.id);
            }
            assert!(mob(game, pushed).position.x > 0.9, "pushed");
            for handle in thinking {
                let m = mob(game, handle);
                assert!(m.ai.no_action_time >= 1, "{}: thinks", m.kind.id);
            }
            for handle in frozen {
                assert_eq!(handle.set_brain_frozen(game, false), Ok(()));
            }
            game.tick_mobs().unwrap();
            for handle in frozen {
                assert!(mob(game, handle).ai.no_action_time >= 1, "resumed");
            }

            // The named refusals change nothing.
            let player = MobRef::new(game.entity(20).unwrap());
            assert_eq!(
                player.set_brain_frozen(game, true),
                Err(BrainFreezeRefusal::NotAMob)
            );
            assert_eq!(player.brain_frozen(game), None);
            let stale = MobRef::new(EntityRef::new(WorldEpoch::new(999), 30));
            assert_eq!(
                stale.set_brain_frozen(game, true),
                Err(BrainFreezeRefusal::NoMob)
            );
            assert_eq!(stale.brain_frozen(game), None);
            assert_eq!(frozen[0].brain_frozen(game), Some(false));
        });
    }
}
