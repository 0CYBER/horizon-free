//! Projectile impacts for native mods (`native_projectile_impact`, NeoForge's
//! `ProjectileImpactEvent`). `tick_projectiles` flies every projectile first and
//! collects the impacts; once its worlds are let go, the mods hear each one, and a
//! projectile whose impact a mod canceled flies the tick again from where it was,
//! through whatever it hit.
use super::{
    Game,
    creatures::{ProjWorld, pickable_projectile_hit},
    dragon::{CRYSTAL_IDS, SPECIAL},
    lan_pvp::meets_players,
    native_hooks::mob_kind,
};
use horizon_client::{
    arrows::ArrowEvent,
    join_seq::JoinList,
    mobs::Who,
    projectiles::{Hit, Projectile},
};
use horizon_core::geometry::Vec3;
use std::ops::Range;

#[cfg(test)]
#[path = "native_projectiles/impact_probe.rs"]
pub(in crate::game) mod impact_probe;

/// The hit target state an independent projectile decided against before a hook.
#[derive(PartialEq)]
pub(super) enum ImpactTarget {
    Block(horizon_world::registry::BlockStateId),
    Entity(mod_api::EntityRef, [f64; 3]),
}

/// A projectile's impact in one tick of `tick_projectiles`.
pub(super) struct Impact {
    /// Where the projectile was in the tick's list.
    pub index: usize,
    /// The projectile before the tick.
    pub before: Projectile,
    pub hit: Hit,
    /// Where its hit is in the tick's `hits`.
    pub hit_slot: usize,
    /// Where its arrow events are in the tick's `arrow_events`.
    pub events: Range<usize>,
}

/// The type the mods know a projectile by: a spectral arrow is its own.
fn projectile_type(p: &Projectile) -> &'static str {
    p.entity_type()
}

/// An owned flight pass restored only if its dispatch unwinds.
struct ProjectileDispatch<'game, 'resources> {
    game: &'game mut Game<'resources>,
    pass: Option<JoinList<Projectile, false>>,
}
impl Drop for ProjectileDispatch<'_, '_> {
    fn drop(&mut self) {
        if let Some(pass) = self.pass.take() {
            self.game
                .creatures
                .projectiles
                .restore(pass, horizon_client::join_seq::RestoreOrder::SpawnsFirst);
        }
    }
}

impl Game<'_> {
    /// Snapshot the hit precondition so listener writes can invalidate its
    /// application without rerunning the collision decision.
    pub(super) fn native_impact_target(&self, hit: Hit) -> Option<ImpactTarget> {
        let (entity, block, _, _) = self.pearl_hit(hit);
        if let Some(id) = entity {
            let entity = self.entity(id)?;
            Some(ImpactTarget::Entity(entity, self.entity_position(entity)?))
        } else if matches!(hit, Hit::Block { .. }) {
            Some(ImpactTarget::Block(
                self.world.block(block[0], block[1], block[2]),
            ))
        } else {
            None
        }
    }

    /// Dispatch a hit for a projectile ticked outside the shared projectile list.
    /// The caller checks the server opt-in gate and exposes the live projectile
    /// before calling; misses never dispatch.
    pub(super) fn native_separate_projectile_impact(
        &mut self,
        projectile: u32,
        kind: &str,
        owner: Option<Who>,
        hit: Hit,
    ) -> bool {
        if matches!(hit, Hit::None) {
            return false;
        }
        let owner = match owner {
            Some(Who::Mob(id)) => self.native_impact_entity_id(id),
            Some(Who::Player) => self.own_entity_id() as u32,
            None => 0,
        };
        let (entity, block, face, at) = self.pearl_hit(hit);
        let entity_kind = entity
            .and_then(|id| self.entity(id))
            .and_then(|entity| self.entity_kind(entity))
            .unwrap_or_default();
        let mut canceled = false;
        self.native_projectile_impact(mod_api::__private::ProjectileImpactCall {
            dimension: self.dim.id(),
            projectile,
            kind,
            owner,
            entity,
            entity_kind: &entity_kind,
            block,
            face,
            at,
            canceled: &mut canceled,
        });
        canceled
    }

    /// Guard just the dispatch: normal completion returns the still-owned pass
    /// to its caller, which restores explicitly at the established tick boundary.
    pub(super) fn with_projectile_dispatch(
        &mut self,
        pass: &mut JoinList<Projectile, false>,
        fire: impl FnOnce(&mut Self),
    ) {
        let mut guard = ProjectileDispatch {
            game: self,
            pass: Some(pass.take_owned_pass()),
        };
        fire(&mut *guard.game);
        *pass = guard.pass.take().unwrap();
    }

    /// Whether this game is a server that flies projectiles for the mods (the
    /// server thread or a dedicated server, never a client) and a mod asked to hear their impacts
    /// (`native_hears_projectile_impacts`). Without one, the tick keeps no copy
    /// of its projectiles: it costs nothing.
    pub(super) fn native_hears_impacts(&self) -> bool {
        if self.serving.is_none() && self.options.dedicated.is_none() {
            return false;
        }
        let mut hears = false;
        self.native_hears_projectile_impacts(&mut hears);
        hears
    }

    /// The entity id of the one who shot `p`: a mob's, a player's, else 0.
    fn native_impact_owner(&self, p: &Projectile) -> u32 {
        match p.owner {
            Some(Who::Mob(id)) => id,
            Some(Who::Player) => self.native_thrower(p.guest),
            None => 0,
        }
    }

    /// The entity id of the player who threw something: the guest `guest`, or
    /// the host.
    fn native_thrower(&self, guest: Option<u32>) -> u32 {
        match guest {
            Some(conn) => self
                .lan_guest_entities()
                .into_iter()
                .find(|&(c, _)| c == conn)
                .map_or(0, |(_, id)| id),
            None => 0,
        }
    }

    /// `native_projectile_impact` for a player's thrown trident, which
    /// `tick_tridents` flies rather than `tick_projectiles`: it hit the mob
    /// `mob`, or with `mob` None the block at `block` on its `face`, at `at`.
    /// Whether a mod canceled it.
    pub(super) fn native_trident_impact(
        &mut self,
        trident: u32,
        guest: Option<u32>,
        mob: Option<u32>,
        block: [i32; 3],
        face: [i32; 3],
        at: Vec3,
    ) -> bool {
        if !self.native_hears_impacts() {
            return false;
        }
        let owner = self.native_thrower(guest);
        let (entity, entity_kind) = match mob {
            Some(id) => {
                let (entity, kind) = self.native_impact_entity(id);
                (Some(entity), kind)
            }
            None => (None, String::new()),
        };
        let dimension = self.dim.id();
        let mut canceled = false;
        self.native_projectile_impact(mod_api::__private::ProjectileImpactCall {
            dimension,
            projectile: trident,
            kind: "minecraft:trident",
            owner,
            entity,
            entity_kind: &entity_kind,
            block,
            face,
            at: [at.x, at.y, at.z],
            canceled: &mut canceled,
        });
        canceled
    }

    /// The entity id a projectile's hit on the reserved or ordinary `id` stands for: a dragon
    /// part is the dragon, an End crystal its own id. Allocates nothing.
    fn native_impact_entity_id(&self, id: u32) -> u32 {
        if id < SPECIAL {
            id
        } else if id < CRYSTAL_IDS {
            self.dragon.as_ref().map_or(0, |d| d.id)
        } else {
            self.crystals
                .get((id - CRYSTAL_IDS) as usize)
                .map_or(0, |c| c.id)
        }
    }

    /// The entity id and type a projectile's hit on the reserved or ordinary
    /// `id` stands for: a dragon part is the dragon, an End crystal its own id.
    fn native_impact_entity(&self, id: u32) -> (u32, String) {
        let entity = self.native_impact_entity_id(id);
        if id >= SPECIAL {
            let kind = if id < CRYSTAL_IDS {
                "minecraft:ender_dragon"
            } else {
                "minecraft:end_crystal"
            };
            return (entity, kind.into());
        }
        if let Some(mob) = self.mobs.iter().find(|m| m.id == id) {
            return (id, mob_kind(mob));
        }
        if let Some(cart) = self.carts.iter().find(|c| c.id == id) {
            return (id, cart.kind.id().into());
        }
        if self.lan_guest_entities().iter().any(|&(_, e)| e == id) {
            return (id, "minecraft:player".into());
        }
        (id, String::new())
    }

    /// `EntityTeleportEvent.EnderPearl` for `owner`'s pearl that hit `hit`:
    /// the listeners' `(target, damage, cancel)`, starting from `to` and `damage`.
    pub(in crate::game) fn native_pearl_teleport(
        &mut self,
        owner: mod_api::EntityRef,
        pearl: u32,
        from: Vec3,
        to: Vec3,
        mut damage: f32,
        hit: Hit,
    ) -> ([f64; 3], f32, bool) {
        let mut target = [to.x, to.y, to.z];
        let mut cancel = false;
        if !mod_api::__private::listened(mod_api::HookId::EnderPearlTeleport) {
            return (target, damage, cancel);
        }
        let (hit_entity, hit_block, hit_face, hit_at) = self.pearl_hit(hit);
        self.native_ender_pearl_teleport(mod_api::__private::EnderPearlTeleportCall {
            owner,
            pearl,
            from: [from.x, from.y, from.z],
            target: &mut target,
            damage: &mut damage,
            cancel: &mut cancel,
            hit_entity,
            hit_block,
            hit_face,
            hit_at,
        });
        (target, damage, cancel)
    }

    /// What a pearl hit (`getHitResult`), as `ender_pearl_teleport` tells it:
    /// `(hit_entity, hit_block, hit_face, hit_at)`. An entity's block is the
    /// cell of the hit point and its face `[0, 0, 0]`; a dragon part is the dragon.
    fn pearl_hit(&self, hit: Hit) -> (Option<u32>, [i32; 3], [i32; 3], [f64; 3]) {
        match hit {
            Hit::Block { pos, at, face } => (None, pos, face, [at.x, at.y, at.z]),
            Hit::Entity { who, at } => {
                let entity = match who {
                    Who::Mob(id) => self.native_impact_entity_id(id),
                    Who::Player => self.own_entity_id() as u32,
                };
                let cell = [
                    at.x.floor() as i32,
                    at.y.floor() as i32,
                    at.z.floor() as i32,
                ];
                (Some(entity), cell, [0; 3], [at.x, at.y, at.z])
            }
            Hit::None => (None, [0; 3], [0; 3], [0.0; 3]),
        }
    }

    /// `native_projectile_impact` for each of `impacts`: whether the mods
    /// canceled it. A hit on a wind charge is not told.
    fn native_projectile_impacts(&mut self, impacts: &[Impact]) -> Vec<bool> {
        let dimension = self.dim.id();
        let mut canceled = Vec::with_capacity(impacts.len());
        for impact in impacts {
            let p = &impact.before;
            let (entity, entity_kind, block, face, at) = match impact.hit {
                Hit::Block { pos, at, face } => (None, String::new(), pos, face, at),
                Hit::Entity {
                    who: Who::Mob(id),
                    at,
                } => {
                    if pickable_projectile_hit(id).is_some() {
                        canceled.push(false);
                        continue;
                    }
                    let (entity, kind) = self.native_impact_entity(id);
                    (Some(entity), kind, [0; 3], [0; 3], at)
                }
                // A server's players are hit as their entities, never as `Player`.
                Hit::Entity {
                    who: Who::Player, ..
                } => {
                    canceled.push(false);
                    continue;
                }
                Hit::None => {
                    canceled.push(false);
                    continue;
                }
            };
            let owner = self.native_impact_owner(p);
            let mut stop = false;
            self.native_projectile_impact(mod_api::__private::ProjectileImpactCall {
                dimension,
                projectile: p.id,
                kind: projectile_type(p),
                owner,
                entity,
                entity_kind: &entity_kind,
                block,
                face,
                at: [at.x, at.y, at.z],
                canceled: &mut stop,
            });
            canceled.push(stop);
        }
        canceled
    }

    /// The end of a tick of `tick_projectiles` that met `impacts`, with
    /// `projectiles` in place (those it keeps, as `listed` says of the tick's
    /// list): the mods hear each, and a projectile one canceled flies the tick
    /// again from before it, through what it hit. Its hit and its arrow events are
    /// taken out of `hits` and `arrow_events`, and it goes back among the
    /// projectiles. One a mod removed in its hook stays removed.
    pub(super) fn native_settle_impacts(
        &mut self,
        projectiles: &mut JoinList<Projectile, false>,
        removed: &mut JoinList<Projectile, false>,
        listed: &[bool],
        impacts: Vec<Impact>,
        hits: &mut Vec<(Projectile, Hit)>,
        arrow_events: &mut Vec<(Vec3, ArrowEvent)>,
    ) {
        // The mods see the projectiles where they will be, as `projectile_hit` does.
        // Move the already-lifted pass: lifting it again would leave its outer
        // pass unpaired when the empty original batch is replaced below.
        self.creatures.projectiles.restore(
            projectiles.take_owned_pass(),
            horizon_client::join_seq::RestoreOrder::SurvivorsFirst,
        );
        let canceled = self.native_projectile_impacts(&impacts);
        *projectiles = self.creatures.projectiles.lift();
        if !canceled.contains(&true) {
            return;
        }
        let mut drop_hits = vec![false; hits.len()];
        let mut drop_events = vec![false; arrow_events.len()];
        let mut flown = Vec::new();
        for (impact, _) in impacts.into_iter().zip(canceled).filter(|(_, c)| *c) {
            drop_hits[impact.hit_slot] = true;
            drop_events[impact.events.clone()].fill(true);
            let present = projectiles.iter().position(|q| q.id == impact.before.id);
            if self
                .persist
                .removals
                .contains(&super::commands::Target::Projectile(impact.before.id))
                || present.is_none() && listed[impact.index]
            {
                continue;
            }
            let mut before = impact.before;
            // Replaying a canceled impact must not repeat the first-tick event
            // already queued by the original tick.
            before.has_been_shot = hits[impact.hit_slot].0.has_been_shot;
            let (p, keep, events) = self.native_projectile_through(before);
            flown.extend(events);
            match (present, keep) {
                (Some(i), true) => {
                    projectiles.store_back(i, p);
                }
                (Some(i), false) => {
                    projectiles.remove(i);
                }
                (None, true) => {
                    let i = removed
                        .iter()
                        .position(|q| q.id == p.id)
                        .expect("unlisted impact owner");
                    let mut owner = removed.remove(i);
                    horizon_client::join_seq::store_back(&mut owner, p);
                    projectiles.push(owner);
                }
                (None, false) => {}
            }
        }
        let mut n = 0;
        hits.retain(|_| {
            n += 1;
            !drop_hits[n - 1]
        });
        let mut n = 0;
        arrow_events.retain(|_| {
            n += 1;
            !drop_events[n - 1]
        });
        arrow_events.extend(flown);
    }

    /// `p`, as it was before its tick, flown that tick through what it hit
    /// (`Projectile::tick_through`, `arrows::tick_through`): the projectile after
    /// it, whether it stays, and the arrow events it made.
    fn native_projectile_through(
        &self,
        mut p: Projectile,
    ) -> (Projectile, bool, Vec<(Vec3, ArrowEvent)>) {
        let mob_shot = !matches!(p.owner, Some(Who::Player));
        let pvp = !mob_shot && meets_players(&p.kind);
        // A guest's shot flies from that guest, as in `tick_projectiles`.
        let shooter = p
            .guest
            .filter(|_| pvp)
            .and_then(|c| self.lan_guest_entities().into_iter().find(|&(g, _)| g == c))
            .map(|(_, id)| Who::Mob(id));
        if shooter.is_some() {
            p.owner = shooter;
        }
        let mut targets = self.projectile_targets();
        if mob_shot || pvp {
            targets.extend(self.lan_projectile_targets());
        }
        let rain = self.arrow_rain();
        let scene = self.arrow_scene(&rain);
        let (keep, _) = if p.arrow.is_some() {
            horizon_client::arrows::tick_through(&mut p, &scene, &targets)
        } else {
            let w = self.mob_world_base(&[]);
            p.tick_through(&ProjWorld {
                w: &w,
                targets: &targets,
                border: Some(self.border.bounds()),
            })
        };
        if shooter.is_some() {
            p.owner = Some(Who::Player);
        }
        let at = p.position;
        let events = p
            .arrow
            .as_mut()
            .map(|a| a.events.drain(..).map(|e| (at, e)).collect())
            .unwrap_or_default();
        (p, keep, events)
    }
}

#[cfg(test)]
#[path = "native_projectiles/impact_panic_tests.rs"]
mod impact_panic_tests;

#[cfg(test)]
mod nested_spawn_tests {
    use super::*;

    fn impact_handoff_balances_the_pass(park: bool) {
        use horizon_client::join_seq::residency::{Change, Journal, Kind, Owner};
        crate::game::native_entity_refs::tests::with_game(|game| {
            if park {
                game.enter_level(crate::game::Dim::Nether).unwrap();
            }
            let journal = Journal::default();
            let owner = Owner {
                dimension: game.dim.key(),
                kind: Kind::Projectile,
            };
            game.creatures
                .projectiles
                .bind_residency(&journal, owner, |p| (p.id, p.join));
            let mut shot = Projectile::new(
                horizon_client::projectiles::ProjectileKind::Egg,
                Vec3::new(3.5, 80.0, 3.5),
                None,
            );
            shot.id = 71;
            game.creatures.projectiles.push(shot);
            let join = game.creatures.projectiles[0].join;
            let mut pass = game.creatures.projectiles.lift();
            let mut child = pass[0].clone();
            child.id = 72;
            child.join = horizon_client::join_seq::JoinSeq::UNJOINED;
            game.creatures.projectiles.push(child);
            let child_join = game.creatures.projectiles[0].join;
            game.native_settle_impacts(
                &mut pass,
                &mut JoinList::default(),
                &[],
                Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
            );
            assert_eq!(pass.iter().map(|p| p.id).collect::<Vec<_>>(), [71, 72]);
            game.creatures
                .projectiles
                .restore(pass, horizon_client::join_seq::RestoreOrder::SpawnsFirst);
            if park {
                game.enter_level(crate::game::Dim::Overworld).unwrap();
            }
            let mut events = Vec::new();
            journal.drain_into(&mut events);
            assert!(events.iter().all(|event| event.owner == owner));
            let balance: i32 = events
                .iter()
                .map(|event| match event.change {
                    Change::Lifted => 1,
                    Change::Restored => -1,
                    _ => 0,
                })
                .sum();
            assert_eq!(balance, 0, "impact exposure left an unmatched lifted pass");
            if park {
                game.enter_level(crate::game::Dim::Nether).unwrap();
            }
            assert_eq!(
                game.creatures
                    .projectiles
                    .iter()
                    .map(|p| (p.id, p.join))
                    .collect::<Vec<_>>(),
                [(71, join), (72, child_join)]
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn impact_handoff_balances_the_active_owner() {
        impact_handoff_balances_the_pass(false);
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn impact_handoff_balances_the_parked_owner() {
        impact_handoff_balances_the_pass(true);
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn impact_exposure_retains_the_existing_spawn_buffer_for_the_next_pass() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            fn shot(id: u32) -> Projectile {
                let mut p = Projectile::new(
                    horizon_client::projectiles::ProjectileKind::Egg,
                    horizon_core::geometry::Vec3::new(3.5, 80.0, 3.5),
                    None,
                );
                p.id = id;
                p
            }
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            game.block_ticks
                .add_empty_chunk(horizon_core::position::ChunkPos::new(0, 0))
                .unwrap();
            let mut taken = JoinList::default();
            for id in [71, 73, 74] {
                taken.push(shot(id));
            }
            let _ = taken.remove(1);
            let mut expected: Vec<_> = taken.iter().map(|p| (p.id, p.join)).collect();
            game.creatures.projectiles.push(shot(72));
            let join = game.creatures.projectiles[0].join;
            expected.push((72, join));
            game.native_settle_impacts(
                &mut taken,
                &mut JoinList::default(),
                &[],
                Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
            );
            assert_eq!(
                taken.iter().map(|p| (p.id, p.join)).collect::<Vec<_>>(),
                expected
            );
            assert_eq!(taken[2].age, 0);
            assert_eq!(taken[2].join, join);
            game.creatures
                .projectiles
                .restore(taken, horizon_client::join_seq::RestoreOrder::SpawnsFirst);
            game.tick_projectiles().unwrap();
            let spawned = game
                .creatures
                .projectiles
                .iter()
                .find(|p| p.id == 72)
                .unwrap();
            assert_eq!(spawned.age, 1);
            assert_eq!(spawned.join, join);
        });
    }
}

#[cfg(test)]
mod pearl_hit_tests {
    use super::*;

    /// `ender_pearl_teleport`'s hit fields: a block and its face normal, or an
    /// entity with the cell of its hit point and no face.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_pearl_tells_the_block_face_or_the_entity_it_hit() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            let wall = Hit::Block {
                pos: [4, 70, -2],
                at: Vec3::new(4.0, 70.5, -1.5),
                face: [-1, 0, 0],
            };
            assert_eq!(
                game.pearl_hit(wall),
                (None, [4, 70, -2], [-1, 0, 0], [4.0, 70.5, -1.5])
            );
            let cow = Hit::Entity {
                who: Who::Mob(42),
                at: Vec3::new(1.25, 64.5, -0.5),
            };
            assert_eq!(
                game.pearl_hit(cow),
                (Some(42), [1, 64, -1], [0; 3], [1.25, 64.5, -0.5])
            );
        });
    }

    /// A dragon part is the dragon and the local player is its own entity.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_pearl_names_the_dragon_for_a_part_and_the_local_player_by_its_own_id() {
        use crate::game::dragon::Dragon;
        crate::game::native_entity_refs::tests::with_game(|game| {
            let mut dragon = Dragon::new(Vec3::ZERO, 0.0, 0);
            dragon.id = 225009;
            game.dragon.replace(dragon);
            let at = Vec3::new(1.5, 64.5, -0.5);
            let part = Hit::Entity {
                who: Who::Mob(SPECIAL + 2),
                at,
            };
            assert_eq!(
                game.pearl_hit(part),
                (Some(225009), [1, 64, -1], [0; 3], [1.5, 64.5, -0.5])
            );
            let player = Hit::Entity {
                who: Who::Player,
                at,
            };
            // The local player is the entity the game gives it, here as the game reports it.
            let own = game.own_entity_id() as u32;
            assert_eq!(game.pearl_hit(player).0, Some(own));
        });
    }

    /// A listener hears the hit through `native_pearl_teleport` and its write comes back.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_listening_mod_hears_the_pearls_hit_and_its_target_comes_back() {
        use crate::game::dragon::Dragon;
        use mod_api::{EnderPearlTeleportEvent, Flow, Hooks, ModEntry, ServerHost};
        use std::sync::{Arc, Mutex};
        type Heard = Arc<Mutex<Vec<(Option<u32>, [i32; 3])>>>;
        struct Hear(Heard);
        #[mod_api::__subscriptions]
        impl Hooks for Hear {
            fn ender_pearl_teleport(
                &self,
                _: &mut dyn ServerHost,
                event: &mut EnderPearlTeleportEvent<'_>,
            ) -> Flow {
                self.0
                    .lock()
                    .unwrap()
                    .push((event.hit_entity, event.hit_block));
                *event.target = [1.0, 2.0, 3.0];
                Flow::Continue
            }
        }
        let heard = Heard::default();
        let mods: mod_api::Mods = vec![ModEntry::new("hear", Box::new(Hear(heard.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                let _phase =
                    mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
                let mut dragon = Dragon::new(Vec3::ZERO, 0.0, 0);
                dragon.id = 225009;
                game.dragon.replace(dragon);
                let owner = mod_api::EntityRef::new(mod_api::WorldEpoch::new(1), 7);
                let hit = Hit::Entity {
                    who: Who::Mob(SPECIAL + 2),
                    at: Vec3::new(1.5, 64.5, -0.5),
                };
                let (target, damage, cancel) = game.native_pearl_teleport(
                    owner,
                    9,
                    Vec3::ZERO,
                    Vec3::new(8.0, 70.0, 8.0),
                    0.0,
                    hit,
                );
                assert_eq!((target, damage, cancel), ([1.0, 2.0, 3.0], 0.0, false));
                assert_eq!(*heard.lock().unwrap(), [(Some(225009), [1, 64, -1])]);
            });
        });
    }
}
