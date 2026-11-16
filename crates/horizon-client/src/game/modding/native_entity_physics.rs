//! An entity's `Entity.noPhysics` for native server mods, through an
//! `EntityRef` (docs/modding/src/how-to/native-mods.md, "Entity and level
//! handles"): whether its moves pass through blocks. Offered only for the
//! kinds whose flag has one value between ticks; an item recomputes its own
//! inside its tick, so it reads None, as do the other kinds, a stale handle
//! and a client. The write is refused for the three kinds whose own tick
//! rewrites the flag; the ender dragon, an area-effect cloud, an ominous item
//! spawner and a shulker bullet take it, starting from the true value their
//! constructors set.
use super::{Game, commands::Target};
use horizon_client::mobs::Species;
use horizon_client::projectiles::{Projectile, ProjectileKind};
use mod_api::EntityRef;

impl Game<'_> {
    /// `Entity.noPhysics`: a player's is whether it spectates (`Player.tick`
    /// sets it from `isSpectator`); a mob's is the flag a mod wrote, false
    /// until then (a vex sets it only for the length of its own tick); an
    /// arrow's or a trident's is its `AbstractArrow.noPhysics`, set once a
    /// loyal trident turns back. The ender dragon, an area-effect cloud, an
    /// ominous item spawner and a shulker bullet set it in their constructors
    /// and never clear it, so theirs is the value a mod wrote, or that
    /// constructor default (`true`) until then. An armor stand's is recomputed
    /// on load, so it keeps a written value only until then; a kind Horizon
    /// does not simulate keeps it in its saved compound.
    pub(super) fn entity_no_physics(&self, entity: EntityRef) -> Option<bool> {
        match self.native_entity_target(entity)? {
            Target::Player => Some(self.player.as_ref()?.spectator),
            Target::Guest(conn) => Some(self.command_guest(conn)?.motion.as_deref()?.spectator),
            Target::Mob(id) => Some(self.mobs.iter().find(|m| m.id == id)?.no_physics),
            Target::Dragon(id) | Target::Cloud(id) | Target::Spawner(id) => {
                Some(self.stored_no_physics(id))
            }
            Target::Projectile(id) => {
                let p = self.creatures.projectiles.iter().find(|p| p.id == id)?;
                match (&p.kind, &p.arrow) {
                    (ProjectileKind::ShulkerBullet { .. }, _) => Some(self.stored_no_physics(id)),
                    (_, Some(arrow)) => Some(arrow.no_physics),
                    _ => None,
                }
            }
            Target::Trident(id) => Some(self.tridents.iter().find(|t| t.id == id)?.no_physics),
            Target::Stand(id) => {
                let stand = self.armor_stands.iter().find(|s| s.id == id)?;
                Some(stand.no_physics.unwrap_or(stand.marker || stand.no_gravity))
            }
            Target::Other(id) => Some(
                self.persist
                    .raw
                    .get(&id)
                    .is_some_and(|tag| horizon_client::entity_nbt::flag(tag, "NoPhysics")),
            ),
            _ => None,
        }
    }

    /// `Entity.noPhysics` for a kind that keeps no field of its own: the value
    /// a mod wrote, or the kind's constructor default (`true` for the dragon,
    /// an area-effect cloud, an ominous item spawner and a shulker bullet).
    fn stored_no_physics(&self, id: u32) -> bool {
        self.persist.no_physics.get(&id).copied().unwrap_or(true)
    }

    /// `Entity.noPhysics` for a native server mod: the value
    /// `entity_no_physics` reads. Server-only; a player, an item entity and a
    /// vex rewrite the flag every tick (`Player.tick`, `ItemEntity.tick`,
    /// `Vex.tick`), so they are refused by kind, and so is a client's write.
    /// An arrow's or a trident's write carries the synced flag clients read.
    /// An armor stand recomputes the flag on load, so its write is not
    /// persisted; an unsimulated kind keeps it in its saved compound. The
    /// dragon, an area-effect cloud, an ominous item spawner and a shulker
    /// bullet take the write, which replaces their constructor's `true`.
    pub(super) fn entity_set_no_physics(&mut self, entity: EntityRef, no_physics: bool) -> bool {
        if !self.is_logical_server() {
            return false;
        }
        match self.native_entity_target(entity) {
            Some(Target::Mob(id)) => {
                let Some(mob) = self.mobs.iter_mut().find(|m| m.id == id) else {
                    return false;
                };
                if mob.species() == Species::Vex {
                    return false;
                }
                mob.no_physics = no_physics;
                true
            }
            Some(Target::Dragon(id) | Target::Cloud(id) | Target::Spawner(id)) => {
                self.persist.no_physics.insert(id, no_physics);
                true
            }
            Some(Target::Projectile(id)) => {
                let Some(index) = self.creatures.projectiles.iter().position(|p| p.id == id) else {
                    return false;
                };
                if matches!(
                    self.creatures.projectiles[index].kind,
                    ProjectileKind::ShulkerBullet { .. }
                ) {
                    self.persist.no_physics.insert(id, no_physics);
                    return true;
                }
                self.creatures
                    .projectiles
                    .update(index, |p| p.set_no_physics(no_physics))
            }
            Some(Target::Trident(id)) => {
                self.tridents
                    .iter_mut()
                    .find(|t| t.id == id)
                    .is_some_and(|t| {
                        t.set_no_physics(no_physics);
                        true
                    })
            }
            Some(Target::Stand(id)) => self
                .armor_stands
                .iter_mut()
                .find(|s| s.id == id)
                .is_some_and(|s| {
                    s.no_physics = Some(no_physics);
                    true
                }),
            Some(Target::Other(id)) => {
                if !self.others.iter().any(|o| o.id == id) {
                    return false;
                }
                use horizon_core::nbt::Tag;
                let mut compound = match self.persist.raw.get(&id) {
                    Some(Tag::Compound(map)) => map.clone(),
                    _ => std::collections::BTreeMap::new(),
                };
                if no_physics {
                    compound.insert("NoPhysics".into(), Tag::Byte(1));
                } else {
                    compound.remove(&horizon_core::nbt::JavaString::from("NoPhysics"));
                }
                self.persist.raw.insert(id, Tag::Compound(compound));
                true
            }
            _ => false,
        }
    }

    /// `/kill` and the native discard drop a projectile from its list: the
    /// `noPhysics` write stored for a shulker bullet leaves with it.
    pub(super) fn remove_projectile(&mut self, id: u32) {
        self.creatures.projectiles.retain(|p| p.id != id);
        self.forget_entity(id);
    }

    /// A player's blow takes a projectile out at `i` (`hit_shulker_bullet`):
    /// the `noPhysics` write stored for a shulker bullet leaves with it.
    pub(super) fn remove_projectile_at(&mut self, i: usize) -> Projectile {
        let p = self.creatures.projectiles.remove(i);
        self.forget_entity(p.id);
        p
    }

    /// A command drops an ominous item spawner: its stored `noPhysics` write
    /// leaves with it.
    pub(super) fn remove_ominous_spawner(&mut self, id: u32) {
        self.ominous_spawners.retain(|s| s.id != id);
        self.forget_entity(id);
    }

    /// A command drops one breath cloud: its stored `noPhysics` write leaves
    /// with it.
    pub(super) fn remove_breath_cloud(&mut self, id: u32) {
        self.breath_clouds.retain(|c| c.id != id);
        self.forget_entity(id);
    }

    /// Drop every breath cloud the predicate rejects and forget each one's
    /// stored `noPhysics` write (the dragon's flame going).
    pub(super) fn retain_breath_clouds(
        &mut self,
        mut keep: impl FnMut(&super::dragon::Breath) -> bool,
    ) {
        let mut gone = Vec::new();
        self.breath_clouds.retain(|c| {
            let kept = keep(c);
            if !kept {
                gone.push(c.id);
            }
            kept
        });
        for id in gone {
            self.forget_entity(id);
        }
    }

    /// A breath cloud expiring inside a `retain_mut`: forget its stored
    /// `noPhysics` write and refuse it, in one call.
    pub(super) fn forget_expired_cloud(&mut self, id: u32) -> bool {
        self.forget_entity(id);
        false
    }

    /// An ominous item spawner finishing inside a `retain_mut`: forget its
    /// stored `noPhysics` write and refuse it, in one call.
    pub(super) fn forget_finished_spawner(&mut self, id: u32) -> bool {
        self.forget_entity(id);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "daily"]
    fn no_physics_reads_supported_entities_and_refuses_others() {
        use super::super::native_entity_refs::tests::with_game;
        use super::super::trident::ThrownTrident;
        use horizon_client::mobs::Mob;
        use horizon_client::motion::PlayerMotion;
        use horizon_client::projectiles::{Projectile, ProjectileKind};
        use horizon_core::geometry::Vec3;
        use mod_api::WorldEpoch;

        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            crate::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let player = game.entity(20).unwrap();
            game.player.as_mut().unwrap().spectator = false;
            assert_eq!(player.no_physics(game), Some(false));
            game.player.as_mut().unwrap().spectator = true;
            assert_eq!(player.no_physics(game), Some(true));

            let kind = game.res.mob_types().get("minecraft:vex").unwrap().clone();
            game.mobs.push(Mob::new(42, kind, Vec3::ZERO, 0.0));
            let mob = game.entity(42).unwrap();
            assert_eq!(mob.no_physics(game), Some(false));

            let mut arrow = Projectile::new(
                ProjectileKind::Arrow {
                    base_damage: 2.0,
                    critical: false,
                    effect: None,
                },
                Vec3::ZERO,
                None,
            );
            arrow.id = game.take_entity_id();
            let arrow_id = arrow.id;
            game.creatures.projectiles.push(arrow);
            let arrow = game.entity(arrow_id).unwrap();
            assert_eq!(arrow.no_physics(game), Some(false));
            game.creatures
                .projectiles
                .update(0, |p| p.arrow.as_mut().unwrap().no_physics = true);
            assert_eq!(arrow.no_physics(game), Some(true));

            let trident_id = game.take_entity_id();
            game.tridents
                .push(ThrownTrident::remote(trident_id, Vec3::ZERO, 0.0, 0.0));
            let trident = game.entity(trident_id).unwrap();
            assert_eq!(trident.no_physics(game), Some(false));
            game.tridents[0].no_physics = true;
            assert_eq!(trident.no_physics(game), Some(true));

            let mut snowball = Projectile::new(ProjectileKind::Snowball, Vec3::ZERO, None);
            snowball.id = game.take_entity_id();
            let snowball_id = snowball.id;
            game.creatures.projectiles.push(snowball);
            let snowball = game.entity(snowball_id).unwrap();
            assert_eq!(snowball.no_physics(game), None);

            let mut bullet = Projectile::new(
                ProjectileKind::ShulkerBullet {
                    target: None,
                    dir: None,
                    steps: 0,
                    delta: [0.0; 3],
                    seed: 0,
                },
                Vec3::ZERO,
                None,
            );
            bullet.id = game.take_entity_id();
            let bullet_id = bullet.id;
            game.creatures.projectiles.push(bullet);
            let bullet = game.entity(bullet_id).unwrap();
            assert_eq!(bullet.no_physics(game), Some(true));

            let mut dragon = super::super::dragon::Dragon::new(Vec3::ZERO, 0.0, 0);
            dragon.id = game.take_entity_id();
            let dragon_id = dragon.id;
            game.dragon.replace(dragon);
            let dragon = game.entity(dragon_id).unwrap();
            assert_eq!(dragon.no_physics(game), Some(true));

            game.boats.push(super::super::boats::Boat::new(
                44,
                "oak".into(),
                Vec3::ZERO,
                0.0,
            ));
            let boat = game.entity(44).unwrap();
            assert_eq!(boat.no_physics(game), None);
            let stale = EntityRef::new(WorldEpoch::new(mob.epoch().get() + 1), mob.id());
            assert_eq!(stale.no_physics(game), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn no_physics_write_holds_and_covers_the_four_constant_kinds() {
        use super::super::native_entity_refs::tests::with_game;
        use super::super::trident::ThrownTrident;
        use crate::game::commands::OtherEntity;
        use crate::game::entity::Entity as _;
        use horizon_client::entities::ArmorStand;
        use horizon_client::items::ItemStack;
        use horizon_client::mobs::Mob;
        use horizon_client::motion::PlayerMotion;
        use horizon_client::projectiles::Projectile;
        use horizon_core::geometry::Vec3;

        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            for (x, z) in (-1..=1).flat_map(|x| (-1..=1).map(move |z| (x, z))) {
                let mut chunk = horizon_client::chunk::Chunk::empty(x, z);
                chunk.lit = true;
                game.world.chunks.insert((x, z), std::sync::Arc::new(chunk));
            }
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            crate::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            game.others.push(OtherEntity {
                attachment_lifetime: Default::default(),
                join: Default::default(),
                id: 14,
                kind: "minecraft:marker".into(),
                position: Vec3::new(0.5, 64.0, 0.5),
                yaw: 0.0,
                pitch: 0.0,
                item_transformation: None,
                interaction: None,
                knot_check: None,
                eye: None,
            });
            let marker = game.entity(14).unwrap();
            assert_eq!(marker.no_physics(game), Some(false));
            // The write lands, and holds: no tick of the marker's own rewrites it.
            assert!(marker.set_no_physics(game, true));
            assert_eq!(marker.no_physics(game), Some(true));
            for _ in 0..20 {
                game.tick_mobs().unwrap();
            }
            assert_eq!(
                marker.no_physics(game),
                Some(true),
                "the write did not hold across ticks"
            );
            assert!(marker.set_no_physics(game, false));
            assert_eq!(marker.no_physics(game), Some(false));

            // A player rewrites the flag from its spectator state every tick.
            let player = game.entity(20).unwrap();
            assert!(!player.set_no_physics(game, true));
            assert_eq!(player.no_physics(game), Some(false));

            // An item entity recomputes it from the blocks it is in.
            game.spawn_item(
                ItemStack {
                    item: "minecraft:stone".into(),
                    count: 1,
                    damage: 0,
                    extra: None,
                },
                [0.0; 3],
                [0.0; 3],
                0,
            );
            let item = game.entity(game.dropped[0].id).unwrap();
            assert!(!item.set_no_physics(game, true));
            assert_eq!(item.no_physics(game), None);

            // A vex sets it inside its own tick.
            let vex = game.res.mob_types().get("minecraft:vex").unwrap().clone();
            game.mobs
                .push(Mob::new(42, vex, Vec3::new(0.5, 100.0, 0.5), 0.0));
            let vex = game.entity(42).unwrap();
            assert!(!vex.set_no_physics(game, true));
            assert_eq!(vex.no_physics(game), Some(false));

            // A mob that keeps its flag takes the write and holds it.
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(43, zombie, Vec3::new(1.5, 100.0, 0.5), 0.0));
            let zombie = game.entity(43).unwrap();
            assert!(zombie.set_no_physics(game, true));
            game.tick_mobs().unwrap();
            assert_eq!(zombie.no_physics(game), Some(true));

            // An arrow and a trident write their synced flag with it.
            let mut arrow = Projectile::new(
                ProjectileKind::Arrow {
                    base_damage: 2.0,
                    critical: false,
                    effect: None,
                },
                Vec3::ZERO,
                None,
            );
            arrow.id = game.take_entity_id();
            let arrow_id = arrow.id;
            game.creatures.projectiles.push(arrow);
            let arrow = game.entity(arrow_id).unwrap();
            assert!(arrow.set_no_physics(game, true));
            assert_eq!(arrow.no_physics(game), Some(true));
            assert!(game.creatures.projectiles[0].needs_sync);

            let trident_id = game.take_entity_id();
            game.tridents
                .push(ThrownTrident::remote(trident_id, Vec3::ZERO, 0.0, 0.0));
            let trident_id = game.tridents[0].id;
            let trident = game.entity(trident_id).unwrap();
            assert!(trident.set_no_physics(game, true));
            assert_eq!(trident.no_physics(game), Some(true));
            assert!(game.tridents[0].needs_sync);

            // An armor stand takes it, and recomputes it on a load.
            game.armor_stands.push(ArmorStand::new(44, Vec3::ZERO, 0.0));
            let stand = game.entity(44).unwrap();
            assert!(stand.set_no_physics(game, true));
            assert_eq!(stand.no_physics(game), Some(true));
            let saved = horizon_client::entity_nbt::armor_stand_to_nbt(
                &game.armor_stands[0],
                [1, 2, 3, 4],
                None,
            );
            let loaded = horizon_client::entity_nbt::armor_stand_from_nbt(&saved, 44).unwrap();
            assert!(
                !loaded.no_physics(None),
                "a stand recomputes the flag on load"
            );

            // A shulker bullet and the dragon read a constant `true` before;
            // now their write lands and the read answers it.
            let mut bullet = Projectile::new(
                ProjectileKind::ShulkerBullet {
                    target: None,
                    dir: None,
                    steps: 0,
                    delta: [0.0; 3],
                    seed: 0,
                },
                Vec3::ZERO,
                None,
            );
            bullet.id = game.take_entity_id();
            let bullet_id = bullet.id;
            game.creatures.projectiles.push(bullet);
            let bullet = game.entity(bullet_id).unwrap();
            assert_eq!(bullet.no_physics(game), Some(true));
            assert!(bullet.set_no_physics(game, false));
            assert_eq!(bullet.no_physics(game), Some(false));
            assert!(bullet.set_no_physics(game, true));
            assert_eq!(bullet.no_physics(game), Some(true));

            let mut dragon = super::super::dragon::Dragon::new(Vec3::ZERO, 0.0, 0);
            dragon.id = game.take_entity_id();
            let dragon_id = dragon.id;
            game.dragon.replace(dragon);
            let dragon = game.entity(dragon_id).unwrap();
            assert_eq!(dragon.no_physics(game), Some(true));
            assert!(dragon.set_no_physics(game, false));
            assert_eq!(dragon.no_physics(game), Some(false));
        });
    }

    /// The write is what the movement code reads, not only the read-back: a
    /// shulker's lid (`piston_push::lid_push`, the gate that asks
    /// `Entity::no_physics`) pushes a bullet whose flag a mod cleared and
    /// spares one left at the kind's default `true`.
    #[test]
    #[ignore = "daily"]
    fn a_mod_cleared_no_physics_shulker_bullet_is_pushed_by_a_shulker_lid() {
        use super::super::native_entity_refs::tests::with_game;
        use crate::game::piston_push::Lid;
        use horizon_client::mobs::Mob;
        use horizon_client::projectiles::Projectile;
        use horizon_core::geometry::{Aabb, Vec3};

        let bullet = |game: &mut Game<'_>, at: Vec3| {
            let mut bullet = Projectile::new(
                ProjectileKind::ShulkerBullet {
                    target: None,
                    dir: None,
                    steps: 0,
                    delta: [0.0; 3],
                    seed: 0,
                },
                at,
                None,
            );
            bullet.id = game.take_entity_id();
            let id = bullet.id;
            game.creatures.projectiles.push(bullet);
            id
        };

        with_game(|game| {
            let kind = game
                .res
                .mob_types()
                .get("minecraft:shulker")
                .expect("the shulker type")
                .clone();
            game.mobs
                .push(Mob::new(24, kind, Vec3::new(24.0, 200.0, 24.0), 0.0));
            let cleared = bullet(game, Vec3::new(10.5, 200.0, 10.5));
            let left_alone = bullet(game, Vec3::new(13.5, 200.0, 10.5));
            let handle = game.entity(cleared).expect("the bullet handle");
            assert!(handle.set_no_physics(game, false));
            assert_eq!(handle.no_physics(game), Some(false));

            let area = Aabb::new(Vec3::new(9.0, 200.0, 9.0), Vec3::new(15.0, 201.0, 12.0));
            game.lid_push(Lid::Shulker(24), area, Vec3::new(0.0, 0.25, 0.0));
            let y = |id: u32| {
                game.creatures
                    .projectiles
                    .iter()
                    .find(|p| p.id == id)
                    .expect("the bullet")
                    .position
                    .y
            };
            assert_eq!(
                y(cleared),
                200.25,
                "a cleared flag lets a shulker's lid push the bullet"
            );
            assert_eq!(
                y(left_alone),
                200.0,
                "a bullet left at its default noPhysics is spared"
            );
        });
    }

    /// The stored value leaves the world with its entity: a projectile unloaded
    /// with its chunk is removed without `forget_entity` on the removal path,
    /// so the sweep has to drop the entry itself.
    #[test]
    #[ignore = "daily"]
    fn no_physics_entry_is_cleared_when_the_bullet_leaves_with_its_chunk() {
        use super::super::native_entity_refs::tests::with_game;
        use horizon_client::projectiles::Projectile;
        use horizon_core::geometry::Vec3;
        use std::collections::HashSet;

        with_game(|game| {
            let mut bullet = Projectile::new(
                ProjectileKind::ShulkerBullet {
                    target: None,
                    dir: None,
                    steps: 0,
                    delta: [0.0; 3],
                    seed: 0,
                },
                Vec3::new(4.5, 200.0, 4.5),
                None,
            );
            bullet.id = game.take_entity_id();
            let id = bullet.id;
            game.creatures.projectiles.push(bullet);
            let bullet = game.entity(id).expect("the bullet handle");
            assert!(bullet.set_no_physics(game, false));
            assert!(game.persist.no_physics.contains_key(&id));

            let _ = game.entity_chunks(&HashSet::from([(0, 0)]), true);
            assert!(
                !game.persist.no_physics.contains_key(&id),
                "the entry left with its entity"
            );
        });
    }

    /// `/kill` (and the native discard that shares `kill_target_in_level`)
    /// takes a shulker bullet out of its list: the `noPhysics` write stored
    /// for it has to leave with it.
    #[test]
    #[ignore = "daily"]
    fn no_physics_entry_is_cleared_when_a_command_kills_a_shulker_bullet() {
        use super::super::native_entity_refs::tests::with_game;
        use horizon_client::projectiles::Projectile;
        use horizon_core::geometry::Vec3;

        with_game(|game| {
            let mut bullet = Projectile::new(
                ProjectileKind::ShulkerBullet {
                    target: None,
                    dir: None,
                    steps: 0,
                    delta: [0.0; 3],
                    seed: 0,
                },
                Vec3::new(4.5, 200.0, 4.5),
                None,
            );
            bullet.id = game.take_entity_id();
            let id = bullet.id;
            game.creatures.projectiles.push(bullet);
            let bullet = game.entity(id).expect("the bullet handle");
            assert!(bullet.set_no_physics(game, false));
            assert!(game.persist.no_physics.contains_key(&id));

            game.set_allow_commands(true);
            assert!(
                game.run_command("kill @e[type=minecraft:shulker_bullet]"),
                "the command found and removed the bullet"
            );
            assert!(!game.creatures.projectiles.iter().any(|p| p.id == id));
            assert!(
                !game.persist.no_physics.contains_key(&id),
                "the entry left with the entity the command killed"
            );
        });
    }

    /// A player's blow removes a shulker bullet (`hit_shulker_bullet`): the
    /// `noPhysics` write stored for it has to leave with it.
    #[test]
    #[ignore = "daily"]
    fn no_physics_entry_is_cleared_when_a_players_blow_removes_a_shulker_bullet() {
        use super::super::native_entity_refs::tests::with_game;
        use horizon_client::projectiles::Projectile;
        use horizon_core::geometry::Vec3;

        with_game(|game| {
            let mut bullet = Projectile::new(
                ProjectileKind::ShulkerBullet {
                    target: None,
                    dir: None,
                    steps: 0,
                    delta: [0.0; 3],
                    seed: 0,
                },
                Vec3::new(4.5, 200.0, 4.5),
                None,
            );
            bullet.id = game.take_entity_id();
            let id = bullet.id;
            game.creatures.projectiles.push(bullet);
            let bullet = game.entity(id).expect("the bullet handle");
            assert!(bullet.set_no_physics(game, false));
            assert!(game.persist.no_physics.contains_key(&id));

            game.hit_shulker_bullet(0);
            assert!(!game.creatures.projectiles.iter().any(|p| p.id == id));
            assert!(
                !game.persist.no_physics.contains_key(&id),
                "the entry left with the bullet the player's blow removed"
            );
        });
    }

    /// An ominous item spawner that finishes its countdown is taken away by
    /// `tick_ominous_item_spawners`: its entry goes with it.
    #[test]
    #[ignore = "daily"]
    fn no_physics_entry_is_cleared_when_an_ominous_item_spawner_finishes() {
        use super::super::native_entity_refs::tests::with_game;
        use horizon_client::items::ItemStack;

        with_game(|game| {
            game.add_ominous_item_spawner(ItemStack::new("minecraft:air", 1), [4.5, 200.0, 4.5], 1);
            let id = game.ominous_spawners[0].id;
            game.persist.no_physics.insert(id, false);
            assert!(game.persist.no_physics.contains_key(&id));

            game.tick_ominous_item_spawners();
            assert!(
                !game.ominous_spawners.iter().any(|s| s.id == id),
                "the spawner finished"
            );
            assert!(
                !game.persist.no_physics.contains_key(&id),
                "the entry left with the spawner that finished"
            );
        });
    }

    /// A breath cloud that runs out of time is taken away by `tick_breath`,
    /// not by a command: its entry goes with it.
    #[test]
    #[ignore = "daily"]
    fn no_physics_entry_is_cleared_when_a_breath_cloud_expires() {
        use super::super::dragon::Breath;
        use super::super::native_entity_refs::tests::with_game;

        with_game(|game| {
            let id = game.take_entity_id();
            game.breath_clouds.push(Breath {
                attachment_lifetime: Default::default(),
                join: Default::default(),
                id,
                at: [0.0, 80.0, 0.0],
                motion: [0.0; 3],
                radius: 3.0,
                age: 20,
                duration: 1,
                from_dragon: false,
                victims: Vec::new(),
                potion: None,
            });
            game.persist.no_physics.insert(id, false);
            assert!(game.persist.no_physics.contains_key(&id));

            game.tick_breath();
            assert!(
                !game.breath_clouds.iter().any(|c| c.id == id),
                "the cloud expired"
            );
            assert!(
                !game.persist.no_physics.contains_key(&id),
                "the entry left with the cloud that expired"
            );
        });
    }

    /// Leaving the flaming phase removes the dragon's flame cloud
    /// (`end_dragon_flame`): its entry leaves with it too.
    #[test]
    #[ignore = "daily"]
    fn no_physics_entry_is_cleared_when_the_dragons_flame_is_removed() {
        use super::super::dragon::{Breath, Dragon};
        use super::super::native_entity_refs::tests::with_game;
        use horizon_core::geometry::Vec3;

        with_game(|game| {
            let mut dragon = Dragon::new(Vec3::ZERO, 0.0, 0);
            dragon.id = game.take_entity_id();
            game.dragon.replace(dragon);
            let id = game.take_entity_id();
            game.breath_clouds.push(Breath {
                attachment_lifetime: Default::default(),
                join: Default::default(),
                id,
                at: [0.0, 80.0, 0.0],
                motion: [0.0; 3],
                radius: 3.0,
                age: 0,
                duration: 600,
                from_dragon: true,
                victims: Vec::new(),
                potion: None,
            });
            game.persist.no_physics.insert(id, false);
            assert!(game.persist.no_physics.contains_key(&id));

            game.dragon.as_mut().unwrap().flame_ended = true;
            game.tick_breath();
            assert!(
                !game.breath_clouds.iter().any(|c| c.id == id),
                "the flame cloud is gone"
            );
            assert!(
                !game.persist.no_physics.contains_key(&id),
                "the entry left with the flame cloud"
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn no_physics_write_is_refused_on_a_client() {
        use super::super::native_entity_refs::tests::with_client_game;
        use mod_api::WorldEpoch;

        with_client_game(|game| {
            let entity = EntityRef::new(WorldEpoch::new(0), 1);
            assert!(!entity.set_no_physics(game, true));
            assert!(!entity.set_no_physics(game, false));
        });
    }
}
