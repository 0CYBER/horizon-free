//! Enderman leave-block attempts need the authoritative block simulation.
use super::Game;
use horizon_client::{Result, mobs::Species};
use horizon_core::{
    geometry::{Aabb, Vec3},
    position::BlockPos,
};
use horizon_world::{
    level::{LevelAccess, flags},
    registry::BlockStateId,
};

/// An entity's box (`EntityDimensions.makeBoundingBox`): `width` wide and
/// `height` tall, standing on `feet`.
fn point_box(feet: [f64; 3], width: f64, height: f64) -> Aabb {
    Aabb::new(
        Vec3::new(feet[0] - width / 2.0, feet[1], feet[2] - width / 2.0),
        Vec3::new(
            feet[0] + width / 2.0,
            feet[1] + height,
            feet[2] + width / 2.0,
        ),
    )
}

impl Game<'_> {
    /// `Enderman.hurtServer` teleports inside the hurt (`teleport()`,
    /// `repeatedlyTryToTeleport()`), and so does `Shulker.hurtServer`
    /// (`teleportSomewhere()`, `hitByShulkerBullet()`). `Mob::damage` has no
    /// world, so it leaves them pending (an enderman's tries in `counter2`, a
    /// shulker's flags); a hurt the game layer deals runs them here, right
    /// after it, not on the mob's next AI step.
    pub(super) fn settle_hurt_teleports(&mut self) -> Result<()> {
        let pending: Vec<u32> = self
            .mobs
            .iter()
            .filter(|m| {
                (m.species() == Species::Enderman && m.extra.counter2 > 0)
                    || horizon_client::mobs::shulker::teleport_pending(m)
            })
            .map(|m| m.id)
            .collect();
        for id in pending {
            // What `LivingEntity.hurtServer` raised comes first, where the
            // mob was hurt and from its random's next draws: the hurt
            // sound, `InfestedMobEffect.onMobHurt`'s silverfish.
            let Some(i) = self.mobs.iter().position(|m| m.id == id) else {
                continue;
            };
            let hurt = std::mem::take(&mut self.mobs[i].outbox);
            if !hurt.is_empty() {
                self.handle_mob_events(hurt)?;
            }
            let Some(i) = self.mobs.iter().position(|m| m.id == id) else {
                continue;
            };
            let shulker = self.mobs[i].species() == Species::Shulker;
            // A shulker's teleport reads the mobs around it (`noCollision`,
            // the crowding count of `hitByShulkerBullet`), itself included as
            // in its own tick; an enderman's reads only blocks.
            let uuid = self.level.player_uuid;
            let others: Vec<_> = if shulker {
                self.mobs
                    .iter()
                    .map(|m| super::creatures::seen(m, uuid, self.items()))
                    .collect()
            } else {
                Vec::new()
            };
            // The world is read while the mob moves: take it out, then put it
            // back where it was (the mob order stays).
            let mut mob = self.mobs.swap_remove(i);
            let from = mob.position;
            let mut events = Vec::new();
            // `hitByShulkerBullet` draws its crowding check from the level's random.
            let mut level_random = self.world.random.clone();
            {
                let world = self.mob_world_base(&others);
                if shulker {
                    horizon_client::mobs::shulker::run_pending_teleports(
                        &mut mob,
                        &world,
                        &mut level_random,
                        &mut events,
                    );
                } else {
                    horizon_client::mobs::enderman::run_pending_teleports(
                        &mut mob,
                        &world,
                        &mut events,
                    );
                }
            }
            self.world.random = level_random;
            let to = mob.position;
            self.mobs.push(mob);
            let last = self.mobs.len() - 1;
            self.mobs.swap(i, last);
            // The hurt's teleport (`teleportTo`) is one position update, told at once.
            self.entity_section_moved(id, Some([from.x, from.y, from.z]), [to.x, to.y, to.z]);
            self.handle_mob_events(events)?;
        }
        Ok(())
    }
    /// `EndermanLeaveBlockGoal.tick`: normalize, check, place, post
    /// BLOCK_PLACE with the enderman and the placed state, then clear.
    pub(super) fn enderman_place_block(
        &mut self,
        mob: u32,
        pos: [i32; 3],
        carried: BlockStateId,
        occupants: &[Aabb],
    ) -> Result<()> {
        // Runtime ids identify the actual mob in this level. An obsolete attempt
        // must not clear another mob's hand or a subsequently changed stack.
        if !self.mobs.iter().any(|m| {
            m.id == mob
                && m.alive()
                && !m.removed
                && m.species() == Species::Enderman
                && m.extra.carried == Some(carried)
        }) {
            return Ok(());
        }
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        let area = Aabb::new(
            Vec3::new(f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])),
            Vec3::new(
                f64::from(pos[0] + 1),
                f64::from(pos[1] + 1),
                f64::from(pos[2] + 1),
            ),
        );
        let entity_in_cube = self.non_mob_entity_meets(area);
        struct Candidate {
            state: BlockStateId,
            target: BlockStateId,
            below: BlockStateId,
        }
        let candidate = self.with_level(|b, level, ticks| {
            let target = level.block_state(at);
            let below = level.block_state(at.offset(0, -1, 0));
            // Must run even when eligibility fails: updateShape can schedule
            // ticks and invoke native hooks on the real level.
            let state = b.update_from_neighbour_shapes(level, ticks, at, carried);
            let support = level.facts.get(below);
            if !level.facts.is_air(target)
                || level.facts.is_air(below)
                || &*support.name == "minecraft:bedrock"
                || !support.full_collision
                || !b.can_survive(level, ticks, at, state)
                || occupants.iter().any(|bounds| bounds.intersects(area))
                || entity_in_cube
            {
                return None;
            }
            Some(Candidate {
                state,
                target,
                below,
            })
        });
        let Some(candidate) = candidate else {
            return self.process_level_events();
        };
        // Keep the fork's entity_place veto, with the normalized state. A cancel
        // leaves the original carried block intact, even when it normalized to AIR.
        if self.place_canceled(mob, at, candidate.state, candidate.below, None) {
            return self.process_level_events();
        }
        // A listener can change the candidate's blocks or the mob. Refuse that
        // stale attempt instead of overwriting its writes or normalizing twice.
        let kind = self
            .mobs
            .iter()
            .find(|m| {
                m.id == mob
                    && m.alive()
                    && !m.removed
                    && m.species() == Species::Enderman
                    && m.extra.carried == Some(carried)
            })
            .map(|m| m.kind.id.clone());
        let Some(kind) = kind else {
            return self.process_level_events();
        };
        if self.world.block(pos[0], pos[1], pos[2]) != candidate.target
            || self.world.block(pos[0], pos[1] - 1, pos[2]) != candidate.below
        {
            return self.process_level_events();
        }
        let facts = self.res.facts();
        let from = facts.get(candidate.target).name.clone();
        let to = facts.get(candidate.state).name.clone();
        // AIR is a legitimate normalized state. setBlock may report no
        // change for it; the eligible operation still empties the hand.
        let changed = self.with_level(|b, level, ticks| {
            b.set_block(level, ticks, at, candidate.state, flags::UPDATE_ALL)
        });
        self.post_block_game_event(
            "minecraft:block_place",
            pos,
            Some(horizon_client::mobs::Who::Mob(mob)),
            Some(candidate.state),
        );
        if let Some(m) = self.mobs.iter_mut().find(|m| {
            m.id == mob
                && !m.removed
                && m.species() == Species::Enderman
                && m.extra.carried == Some(carried)
        }) {
            m.extra.carried = None;
        }
        self.process_level_events()?;
        if changed {
            self.native_entity_change_block(mob, &kind, pos, &from, &to);
        }
        Ok(())
    }
    /// The rest of `Level.getEntities(enderman, box)` past the mobs the goal
    /// saw: whether a player who is not a spectator (`NO_SPECTATORS`) or any
    /// other entity has a box meeting `area`.
    fn non_mob_entity_meets(&self, area: Aabb) -> bool {
        use super::entity::{Entity as _, EntityLists, entity_lists_of, for_each_entity};
        let boxed = |(lo, hi): ([f64; 3], [f64; 3])| {
            Aabb::new(
                Vec3::new(lo[0], lo[1], lo[2]),
                Vec3::new(hi[0], hi[1], hi[2]),
            )
        };
        let spectator = self.spectator();
        let player = self.player.as_ref().filter(|_| !spectator).map(|p| {
            point_box(
                [p.position.x, p.position.y, p.position.z],
                f64::from(p.dimensions.width),
                f64::from(p.dimensions.height),
            )
        });
        let lists = EntityLists {
            mobs: &[],
            ..entity_lists_of!(self)
        };
        let mut met = false;
        for_each_entity!(lists, e => {
            met |= !e.is_removed() && e.bounds().intersects(area);
        });
        met || player
            .into_iter()
            .chain(self.lan_block_trigger_boxes().into_iter().map(boxed))
            .chain(self.item_frames.iter().map(|f| boxed(f.bounds())))
            .chain(self.paintings.iter().map(|p| boxed(p.bounds())))
            .chain(self.crystals.iter().map(super::dragon::Crystal::bounds))
            .chain(self.hook.iter().map(super::fishing::Hook::bounds))
            .any(|b| b.intersects(area))
    }
}
#[cfg(test)]
#[path = "enderman/enderman_occupancy_tests.rs"]
mod occupancy_tests;
