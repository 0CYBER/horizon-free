//! Projectile handles for native server mods: owner, deflection, and an
//! arrow's pickup, damage and critical flag. They cover the projectile list
//! and the player-thrown tridents and firework rockets, which Horizon keeps
//! in lists of their own.
use super::{Game, commands::Target};
use horizon_client::{
    arrows::Pickup,
    lan::playerdata::{ints_uuid, uuid_ints},
    mobs::Who,
    projectiles::{OwnerFields, Projectile, ProjectileKind, Turn},
};
use horizon_core::geometry::Vec3;
use mod_api::{ArrowPickup, Deflection, EntityRef, PierceLevelError, ProjectileRef};

/// Where a projectile handle's entity lives.
#[derive(Clone, Copy)]
enum Backing {
    /// `creatures.projectiles`.
    List(usize),
    /// `tridents`: a trident a player threw.
    Trident(usize),
    /// `rockets`: a firework rocket.
    Rocket(usize),
}

impl Game<'_> {
    fn native_projectile_backing(&self, projectile: ProjectileRef) -> Option<Backing> {
        match self.native_entity_target(projectile.entity())? {
            Target::Projectile(id) => (self.creatures.projectiles.iter())
                .position(|p| p.id == id)
                .map(Backing::List),
            Target::Trident(id) => (self.tridents.iter())
                .position(|t| t.id == id)
                .map(Backing::Trident),
            Target::Rocket(id) => (self.rockets.iter())
                .position(|r| r.id == id)
                .map(Backing::Rocket),
            _ => None,
        }
    }

    /// The index in the projectile list: None for a trident a player threw,
    /// a rocket or a stale handle, which have no arrow state, kind or acceleration.
    fn native_projectile_index(&self, projectile: ProjectileRef) -> Option<usize> {
        match self.native_projectile_backing(projectile)? {
            Backing::List(i) => Some(i),
            _ => None,
        }
    }

    fn native_projectile(&self, projectile: ProjectileRef) -> Option<&Projectile> {
        self.creatures
            .projectiles
            .get(self.native_projectile_index(projectile)?)
    }

    /// Narrows an entity to a projectile: one in the projectile list, a trident
    /// a player threw or a firework rocket. None for another kind, a stale
    /// handle or a client.
    pub(super) fn projectile(&self, entity: EntityRef) -> Option<ProjectileRef> {
        let projectile = ProjectileRef::new(entity);
        self.native_projectile_backing(projectile)
            .map(|_| projectile)
    }

    /// The owner a target names, as a resolved entity in the level in place.
    fn native_owner_entity(&self, target: Target) -> Option<EntityRef> {
        if self.target_level(target) != self.dim {
            return None;
        }
        let id = u32::try_from(self.network_entity_id(target)?).ok()?;
        self.entity(id)
    }

    /// `Projectile.getOwner`: the owner as a resolved entity. None for no
    /// owner, one that is gone or in another level, or a stale handle.
    pub(super) fn projectile_owner(&self, projectile: ProjectileRef) -> Option<EntityRef> {
        let target = match self.native_projectile_backing(projectile)? {
            Backing::List(i) => {
                let uuid = self.creatures.projectiles[i].owner_uuid?;
                self.target_by_uuid(ints_uuid(uuid))?
            }
            // A trident belongs to the guest who threw it, else the host.
            Backing::Trident(i) => self.tridents[i].guest.map_or(Target::Player, Target::Guest),
            Backing::Rocket(i) => {
                let r = &self.rockets[i];
                match r.owner {
                    Some(Who::Player) => r.guest.map_or(Target::Player, Target::Guest),
                    Some(Who::Mob(id)) => Target::Mob(id),
                    None => self.target_by_uuid(ints_uuid(r.owner_uuid?))?,
                }
            }
        };
        self.native_owner_entity(target)
    }

    /// Resolves hit credit, persistent identity, and a separate non-living
    /// vehicle identity for collision exclusion. None for a stale owner or hook.
    pub(super) fn native_projectile_owner_fields(
        &mut self,
        owner: Option<EntityRef>,
    ) -> Option<(OwnerFields, Option<u32>)> {
        let Some(owner) = owner else {
            return Some(((None, None, None), None));
        };
        let target = self.native_entity_target(owner)?;
        let vehicle = match target {
            Target::Boat(id) | Target::Cart(id) => Some(id),
            _ => None,
        };
        let (who, guest) = match target {
            Target::Player => (Some(Who::Player), self.lan_acting),
            Target::Guest(conn) => (Some(Who::Player), Some(conn)),
            Target::Mob(id) => (Some(Who::Mob(id)), None),
            Target::Hook(_) => return None,
            _ => (None, None),
        };
        let uuid = uuid_ints(self.entity_uuid(owner)?);
        Some(((who, Some(uuid), guest), vehicle))
    }

    /// `Projectile.setOwner` on the thing a handle names. False when the
    /// owner does not fit: a trident a player threw always has a player owner.
    fn native_projectile_apply_owner(
        &mut self,
        backing: Backing,
        owner: (OwnerFields, Option<u32>),
    ) -> bool {
        let (fields, vehicle) = owner;
        let (who, uuid, guest) = fields;
        match backing {
            Backing::List(i) => {
                self.creatures.projectiles.update(i, |p| {
                    (p.owner, p.owner_uuid, p.guest) = fields;
                    p.owner_vehicle = vehicle;
                    // `AbstractArrow.setOwner(Entity)`: a player's arrow may be picked up.
                    if let Some(a) = &mut p.arrow {
                        a.set_owner(who);
                    }
                });
            }
            Backing::Trident(i) => {
                if who != Some(Who::Player) {
                    return false;
                }
                self.tridents[i].guest = guest;
            }
            Backing::Rocket(i) => {
                self.rockets[i].set_owner(who, uuid, guest);
            }
        }
        true
    }

    /// `Projectile.setOwner`. False for a stale projectile or owner, or an
    /// owner that is a fishing hook (a trident a player threw takes a player
    /// only). Does not touch `leftOwner`.
    pub(super) fn projectile_set_owner(
        &mut self,
        projectile: ProjectileRef,
        owner: Option<EntityRef>,
    ) -> bool {
        let Some(backing) = self.native_projectile_backing(projectile) else {
            return false;
        };
        let Some(fields) = self.native_projectile_owner_fields(owner) else {
            return false;
        };
        self.native_projectile_apply_owner(backing, fields)
    }

    /// `Projectile.deflect(deflection, by, newOwner, byAttack, power)`, with
    /// `power` per axis, through `Projectile::deflect` for the projectile list
    /// (the engine path gameplay uses). `REVERSE` draws one float from the
    /// projectile's own random; `AIM_DEFLECT` uses `by`'s look and
    /// `MOMENTUM_DEFLECT` its normalized known speed, and both change nothing
    /// without `by`. The new owner is set, then `onDeflection`: a hurting
    /// projectile's acceleration power is 0.1 by an attack, else halved. A
    /// wind charge refuses for its first five ticks, with no change and no
    /// draw. A trident a player threw and a rocket turn the same way (no
    /// acceleration power). False, and nothing changed, for a stale projectile,
    /// `by` or owner, a non-finite power or a refusal.
    pub(super) fn projectile_deflect(
        &mut self,
        projectile: ProjectileRef,
        deflection: Deflection,
        by: Option<EntityRef>,
        new_owner: Option<EntityRef>,
        by_attack: bool,
        power: [f64; 3],
    ) -> bool {
        let Some(backing) = self.native_projectile_backing(projectile) else {
            return false;
        };
        let Some((owner, vehicle)) = self.native_projectile_owner_fields(new_owner) else {
            return false;
        };
        if !power.iter().all(|v| v.is_finite()) {
            return false;
        }
        let turn = match (deflection, by) {
            (Deflection::Reverse, _) => Turn::Reverse,
            (_, None) => Turn::Direction(None),
            (Deflection::AimDeflect, Some(by)) => {
                let Some([x, y, z]) = self.entity_view_vector(by) else {
                    return false;
                };
                Turn::Direction(Some(Vec3::new(x, y, z)))
            }
            (Deflection::MomentumDeflect, Some(by)) => {
                let Some(speed) = self.native_entity_known_speed(by) else {
                    return false;
                };
                Turn::Direction(Some(speed.normalize()))
            }
            _ => unreachable!("(Deflection, Option<EntityRef>): unhandled variant"),
        };
        match backing {
            // `deflect` takes the `EntityReference` overload of `setOwner`: an
            // arrow's pickup does not change, unlike `set_owner`.
            Backing::List(i) => self.creatures.projectiles.update(i, |p| {
                let applied = p.deflect(turn, power, owner, by_attack);
                if applied {
                    p.owner_vehicle = vehicle;
                }
                applied
            }),
            Backing::Trident(i) => {
                if owner.0 != Some(Who::Player) {
                    return false;
                }
                let t = &mut self.tridents[i];
                if let Some((velocity, rotation)) = turn.apply(t.velocity, power, &mut t.random) {
                    t.velocity = velocity;
                    t.yaw += rotation;
                }
                t.guest = owner.2;
                true
            }
            Backing::Rocket(i) => {
                let r = &mut self.rockets[i];
                if let Some((velocity, _)) = turn.apply(r.velocity, power, &mut r.random) {
                    r.velocity = velocity;
                }
                r.set_owner(owner.0, owner.1, owner.2);
                true
            }
        }
    }

    /// `AbstractArrow.pickup`, for an arrow or a mob's trident.
    pub(super) fn projectile_pickup(&self, projectile: ProjectileRef) -> Option<ArrowPickup> {
        Some(
            match self.native_projectile(projectile)?.arrow.as_ref()?.pickup {
                Pickup::Disallowed => ArrowPickup::Disallowed,
                Pickup::Allowed => ArrowPickup::Allowed,
                Pickup::CreativeOnly => ArrowPickup::CreativeOnly,
            },
        )
    }

    /// Sets `AbstractArrow.pickup`. False for a stale handle or a projectile
    /// that is not an arrow or a mob's trident.
    pub(super) fn projectile_set_pickup(
        &mut self,
        projectile: ProjectileRef,
        pickup: ArrowPickup,
    ) -> bool {
        let Some(index) = self.native_projectile_index(projectile) else {
            return false;
        };
        self.creatures.projectiles.update(index, |p| {
            let Some(arrow) = p.arrow.as_mut() else {
                return false;
            };
            arrow.pickup = match pickup {
                ArrowPickup::Disallowed => Pickup::Disallowed,
                ArrowPickup::Allowed => Pickup::Allowed,
                ArrowPickup::CreativeOnly => Pickup::CreativeOnly,
                _ => unreachable!("ArrowPickup: unhandled variant"),
            };
            true
        })
    }

    fn native_arrow_index(&self, projectile: ProjectileRef) -> Result<usize, PierceLevelError> {
        if !self.is_logical_server() {
            return Err(PierceLevelError::WrongSide);
        }
        let backing = self
            .native_projectile_backing(projectile)
            .ok_or(PierceLevelError::Stale)?;
        match backing {
            Backing::List(index) if self.creatures.projectiles[index].arrow.is_some() => Ok(index),
            _ => Err(PierceLevelError::NotArrow),
        }
    }

    /// `AbstractArrow.getPierceLevel`: the signed byte stored by vanilla.
    pub(super) fn projectile_pierce_level(
        &self,
        projectile: ProjectileRef,
    ) -> Result<i8, PierceLevelError> {
        let index = self.native_arrow_index(projectile)?;
        Ok(self.creatures.projectiles[index]
            .arrow
            .as_ref()
            .unwrap()
            .pierce as i8)
    }

    /// `AbstractArrow.setPierceLevel`: stores the signed byte without clamping.
    pub(super) fn projectile_set_pierce_level(
        &mut self,
        projectile: ProjectileRef,
        level: i8,
    ) -> Result<(), PierceLevelError> {
        let index = self.native_arrow_index(projectile)?;
        self.creatures
            .projectiles
            .update(index, |p| p.arrow.as_mut().unwrap().pierce = level as u8);
        Ok(())
    }

    /// `AbstractArrow.getBaseDamage`, for an arrow (a trident's is fixed).
    pub(super) fn projectile_base_damage(&self, projectile: ProjectileRef) -> Option<f64> {
        match self.native_projectile(projectile)?.kind {
            ProjectileKind::Arrow { base_damage, .. } => Some(base_damage),
            _ => None,
        }
    }

    /// `AbstractArrow.setBaseDamage`. False for a stale handle, a projectile
    /// that is not an arrow, or a value that is not finite.
    pub(super) fn projectile_set_base_damage(
        &mut self,
        projectile: ProjectileRef,
        damage: f64,
    ) -> bool {
        let Some(index) = self.native_projectile_index(projectile) else {
            return false;
        };
        self.creatures.projectiles.update(index, |p| {
            let ProjectileKind::Arrow { base_damage, .. } = &mut p.kind else {
                return false;
            };
            if !damage.is_finite() {
                return false;
            }
            *base_damage = damage;
            true
        })
    }

    /// `AbstractArrow.isCritArrow`, for an arrow.
    pub(super) fn projectile_critical(&self, projectile: ProjectileRef) -> Option<bool> {
        let p = self.native_projectile(projectile)?;
        matches!(p.kind, ProjectileKind::Arrow { .. }).then(|| p.critical())
    }

    /// `AbstractArrow.setCritArrow`; clients see the flag at once through the
    /// entity data. False for a stale handle or a projectile that is not an arrow.
    pub(super) fn projectile_set_critical(
        &mut self,
        projectile: ProjectileRef,
        critical: bool,
    ) -> bool {
        let Some(index) = self.native_projectile_index(projectile) else {
            return false;
        };
        self.creatures.projectiles.update(index, |p| {
            if !matches!(p.kind, ProjectileKind::Arrow { .. }) {
                return false;
            }
            p.set_critical(critical);
            true
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        fireworks::Rocket, native_entity_refs::tests::with_game, trident::ThrownTrident,
    };
    use super::*;
    use horizon_client::{mobs::Mob, motion::PlayerMotion};
    use horizon_core::random::RandomSource;
    use mod_api::WorldEpoch;

    #[test]
    #[ignore = "daily"]
    fn a_boat_owned_snowball_excludes_its_owner_until_it_leaves() {
        use horizon_client::projectiles::{Hit, ProjectileWorld};
        use horizon_core::geometry::Aabb;
        struct Air(Vec<(Who, Aabb)>);
        impl ProjectileWorld for Air {
            fn clip(&self, _: Vec3, _: Vec3) -> Option<(Vec3, [i32; 3], [i32; 3])> {
                None
            }
            fn in_water(&self, _: Aabb) -> bool {
                false
            }
            fn targets(&self) -> &[(Who, Aabb)] {
                &self.0
            }
        }
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let boat_id = game.take_entity_id();
            game.boats.push(super::super::boats::Boat::new(
                boat_id,
                "oak".into(),
                Vec3::ZERO,
                0.0,
            ));
            let boat = game.entity(boat_id).unwrap();
            let id = game.take_entity_id();
            let mut p = Projectile::new(ProjectileKind::Snowball, Vec3::new(-1.1, 0.2, 0.0), None);
            p.id = id;
            p.velocity = Vec3::new(1.0, 0.0, 0.0);
            game.creatures.projectiles.push(p);
            let entity = game.entity(id).unwrap();
            let projectile = game.projectile(entity).unwrap();
            assert!(game.projectile_set_owner(projectile, Some(boat)));
            assert_eq!(game.projectile_owner(projectile), Some(boat));
            let scene = Air(game.vehicle_targets());
            let mut p = game.creatures.projectiles[0].clone();
            assert!(!p.left_owner);
            assert_eq!(p.owner, None, "a vehicle must not credit a living attacker");
            assert_eq!(p.owner_uuid, Some(game.uuid_for(boat_id)));
            let mut control = p.clone();
            control.owner = Some(Who::Mob(boat_id));
            assert_eq!(control.tick(&scene).1, Hit::None);
            let mut returned = p.clone();
            returned.left_owner = true;
            assert!(
                matches!(returned.tick(&scene).1, Hit::Entity { who: Who::Mob(id), .. } if id == boat_id)
            );
            assert_eq!(p.tick(&scene).1, Hit::None);
            // Reload mid-flight, before leftOwner: the UUID restores exclusion too.
            let keys = std::collections::HashSet::from([(-1, 0)]);
            let (_, tag) = game.save_projectile(0, &keys).unwrap();
            game.creatures.projectiles.clear();
            game.load_listed("minecraft:snowball", Some([1, 2, 3, 4]), None, tag)
                .unwrap();
            let mut loaded = game.creatures.projectiles[0].clone();
            assert!(!loaded.left_owner);
            assert_eq!(loaded.owner, None);
            assert_eq!(loaded.tick(&scene).1, Hit::None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_vehicle_owned_trident_does_not_scale_damage_as_a_living_attacker() {
        use horizon_client::projectiles::Hit;
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            game.world.health_rules.difficulty = horizon_world::food::Difficulty::Easy;
            let boat_id = game.take_entity_id();
            game.boats.push(super::super::boats::Boat::new(
                boat_id,
                "oak".into(),
                Vec3::ZERO,
                0.0,
            ));
            let boat = game.entity(boat_id).unwrap();
            game.add_projectile(Projectile::new(
                ProjectileKind::Trident,
                Vec3::new(3.0, 0.0, 0.0),
                None,
            ));
            let id = game.creatures.projectiles[0].id;
            let projectile = game.projectile(game.entity(id).unwrap()).unwrap();
            assert!(game.projectile_set_owner(projectile, Some(boat)));
            let p = game.creatures.projectiles[0].clone();
            game.projectile_hit(
                &p,
                Hit::Entity {
                    who: Who::Player,
                    at: Vec3::ZERO,
                },
            )
            .unwrap();
            // Vanilla's 8 damage is not Easy's living-attacker reduction to 5.
            assert_eq!(game.player.as_ref().unwrap().health.health(), 12.0);
        });
    }

    fn arrow(owner: Who) -> Projectile {
        let kind = ProjectileKind::Arrow {
            base_damage: 2.0,
            critical: false,
            effect: None,
        };
        let mut p = Projectile::new(kind, Vec3::new(0.0, 64.0, 0.0), Some(owner));
        p.velocity = Vec3::new(1.0, 2.0, 3.0);
        p
    }

    #[test]
    #[ignore = "daily"]
    fn projectile_handles_read_and_write_owner_deflection_and_arrow_state() {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(99, kind, Vec3::ZERO, 0.0));
            let mob = game.entity(99).unwrap();
            game.add_projectile(arrow(Who::Mob(99)));
            let id = game.creatures.projectiles[0].id;
            let entity = game.entity(id).unwrap();
            assert!(game.projectile(mob).is_none());
            let handle = game.projectile(entity).unwrap();
            assert_eq!(handle.entity(), entity);

            assert_eq!(handle.owner(game), Some(mob));
            assert!(handle.set_owner(game, None));
            assert_eq!(handle.owner(game), None);
            assert_eq!(game.creatures.projectiles[0].owner, None);
            assert!(handle.set_owner(game, Some(mob)));
            assert_eq!(handle.owner(game), Some(mob));
            assert_eq!(game.creatures.projectiles[0].owner, Some(Who::Mob(99)));
            // A non-living owner has collision identity but no living hit credit.
            let boat_id = game.take_entity_id();
            game.boats.push(super::super::boats::Boat::new(
                boat_id,
                "oak".into(),
                Vec3::ZERO,
                0.0,
            ));
            let boat = game.entity(boat_id).unwrap();
            assert!(handle.set_owner(game, Some(boat)));
            assert_eq!(handle.owner(game), Some(boat));
            assert_eq!(game.creatures.projectiles[0].owner, None);
            assert_eq!(game.creatures.projectiles[0].owner_vehicle, Some(boat_id));
            let saved = game.uuid_for(boat_id);
            assert_eq!(game.creatures.projectiles[0].owner_uuid, Some(saved));
            assert!(handle.set_owner(game, Some(mob)));

            assert_eq!(handle.pierce_level(game), Ok(0));
            let role = game.options.execution_role;
            game.options.execution_role = super::super::execution_role::ExecutionRole::Client;
            assert_eq!(handle.pierce_level(game), Err(PierceLevelError::WrongSide));
            assert_eq!(
                handle.set_pierce_level(game, 1),
                Err(PierceLevelError::WrongSide)
            );
            game.options.execution_role = role;
            for level in [i8::MIN, -1, 0, 1, i8::MAX] {
                assert_eq!(handle.set_pierce_level(game, level), Ok(()));
                assert_eq!(handle.pierce_level(game), Ok(level));
            }
            assert_eq!(handle.pickup(game), Some(ArrowPickup::Disallowed));
            assert!(handle.set_pickup(game, ArrowPickup::CreativeOnly));
            assert_eq!(
                game.creatures.projectiles[0].arrow.as_ref().unwrap().pickup,
                Pickup::CreativeOnly
            );
            assert_eq!(handle.base_damage(game), Some(2.0));
            assert!(handle.set_base_damage(game, 7.5));
            assert!(!handle.set_base_damage(game, f64::NAN));
            assert_eq!(handle.base_damage(game), Some(7.5));
            assert_eq!(handle.critical(game), Some(false));
            assert!(handle.set_critical(game, true));
            assert!(game.creatures.projectiles[0].critical());

            // REVERSE: one float from the projectile's `Entity.random`, then
            // the scaled velocity.
            let draw = game.creatures.projectiles[0].random.0.clone().next_float();
            let yaw = game.creatures.projectiles[0].yaw;
            assert!(handle.deflect(game, Deflection::Reverse, None, None, false, [0.2; 3]));
            let p = &game.creatures.projectiles[0];
            assert_eq!(
                p.velocity,
                Vec3::new(1.0 * (-0.2 * 0.5), 2.0 * (-0.2 * 0.5), 3.0 * (-0.2 * 0.5))
            );
            assert_eq!(p.yaw, yaw + (170.0 + draw * 20.0));
            assert_eq!(p.owner, None);
            assert!(p.needs_sync);

            // AIM_DEFLECT along the deflector's look, scaled by the power.
            let look = game.entity_view_vector(mob).unwrap();
            assert!(handle.deflect(
                game,
                Deflection::AimDeflect,
                Some(mob),
                Some(mob),
                true,
                [2.0; 3]
            ));
            let p = &game.creatures.projectiles[0];
            assert_eq!(
                p.velocity,
                Vec3::new(look[0] * 2.0, look[1] * 2.0, look[2] * 2.0)
            );
            assert_eq!(p.owner, Some(Who::Mob(99)));

            // MOMENTUM_DEFLECT of a mob that has not moved has no direction.
            assert!(handle.deflect(
                game,
                Deflection::MomentumDeflect,
                Some(mob),
                None,
                false,
                [1.0; 3]
            ));
            assert_eq!(game.creatures.projectiles[0].velocity, Vec3::ZERO);

            // A hurting projectile's acceleration: 0.1 by an attack, else halved.
            let mut ball = Projectile::fireball(
                ProjectileKind::LargeFireball { power: 1 },
                Vec3::ZERO,
                None,
                [1.0, 0.0, 0.0],
            );
            ball.acceleration = 0.1;
            game.add_projectile(ball);
            let ball_id = game.creatures.projectiles[1].id;
            let ball = game.projectile(game.entity(ball_id).unwrap()).unwrap();
            assert_eq!(ball.pierce_level(game), Err(PierceLevelError::NotArrow));
            assert_eq!(
                ball.set_pierce_level(game, 1),
                Err(PierceLevelError::NotArrow)
            );
            assert_eq!(ball.pickup(game), None);
            assert_eq!(ball.base_damage(game), None);
            assert!(!ball.set_critical(game, true));
            assert!(ball.deflect(game, Deflection::Reverse, None, None, false, [1.0; 3]));
            assert_eq!(game.creatures.projectiles[1].acceleration, 0.05);
            assert!(ball.deflect(game, Deflection::Reverse, None, None, true, [1.0; 3]));
            assert_eq!(game.creatures.projectiles[1].acceleration, 0.1);

            let stale = ProjectileRef::new(EntityRef::new(
                WorldEpoch::new(entity.epoch().get() + 1),
                entity.id(),
            ));
            assert_eq!(stale.pierce_level(game), Err(PierceLevelError::Stale));
            assert_eq!(
                stale.set_pierce_level(game, 1),
                Err(PierceLevelError::Stale)
            );
            assert_eq!(stale.owner(game), None);
            assert!(!stale.set_pickup(game, ArrowPickup::Allowed));
            assert!(!stale.deflect(game, Deflection::Reverse, None, None, false, [1.0; 3]));
            let taken = game.creatures.projectiles.lift();
            assert!(handle.base_damage(game).is_none());
            game.creatures.projectiles = taken;
        });
    }

    /// A player entity for the tests: the host's player standing in for a guest.
    fn player(game: &mut Game<'_>) -> EntityRef {
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        crate::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
        game.entity(20).unwrap()
    }

    #[test]
    #[ignore = "daily"]
    fn trident_and_rocket_handles_narrow_and_turn_with_their_own_random() {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let player = player(game);
            let id = game.take_entity_id();
            let mut trident = ThrownTrident::remote(id, Vec3::new(0.0, 64.0, 0.0), 10.0, 0.0);
            trident.velocity = Vec3::new(1.0, 2.0, 3.0);
            let mut expect = trident.random.clone();
            game.tridents.push(trident);
            let id = game.take_entity_id();
            let mut rocket =
                Rocket::remote(id, Vec3::new(0.0, 70.0, 0.0), Vec3::new(0.0, 1.0, 0.0));
            let mut rocket_random = rocket.random.clone();
            rocket.velocity = Vec3::new(2.0, 4.0, 6.0);
            game.rockets.push(rocket);

            let trident = game
                .projectile(game.entity(game.tridents[0].id).unwrap())
                .unwrap();
            let rocket = game
                .projectile(game.entity(game.rockets[0].id).unwrap())
                .unwrap();
            // A trident or rocket has no arrow state: those reads are None.
            assert_eq!(trident.pickup(game), None);
            assert_eq!(rocket.base_damage(game), None);
            assert!(!rocket.set_critical(game, true));

            // A trident a player threw keeps a player owner.
            assert!(!trident.set_owner(game, None));
            assert!(trident.set_owner(game, Some(player)));
            assert_eq!(game.tridents[0].guest, None);
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(99, zombie, Vec3::ZERO, 0.0));
            let mob = game.entity(99).unwrap();
            assert!(!trident.set_owner(game, Some(mob)));

            // REVERSE: one float from the trident's own random, the level's untouched.
            let level = game.world.random.clone().next_float();
            let draw = expect.next_float();
            assert!(trident.deflect(
                game,
                Deflection::Reverse,
                None,
                Some(player),
                false,
                [0.02, 0.2, 0.02]
            ));
            assert_eq!(
                game.tridents[0].velocity,
                Vec3::new(1.0 * (-0.02 * 0.5), 2.0 * (-0.2 * 0.5), 3.0 * (-0.02 * 0.5))
            );
            assert_eq!(game.tridents[0].yaw, 10.0 + (170.0 + draw * 20.0));
            assert_eq!(
                game.tridents[0].random.clone().next_float(),
                expect.next_float()
            );
            assert_eq!(game.world.random.clone().next_float(), level);
            // No owner for a trident a player threw: refused, nothing changed.
            let velocity = game.tridents[0].velocity;
            assert!(!trident.deflect(game, Deflection::Reverse, None, None, false, [1.0; 3]));
            assert_eq!(game.tridents[0].velocity, velocity);

            // A rocket: owner, then REVERSE, then an owner of none.
            assert_eq!(rocket.owner(game), None);
            assert!(rocket.set_owner(game, Some(mob)));
            assert_eq!(rocket.owner(game), Some(mob));
            assert_eq!(game.rockets[0].owner, Some(Who::Mob(99)));
            let _ = rocket_random.next_float();
            assert!(rocket.deflect(game, Deflection::Reverse, None, None, false, [0.5; 3]));
            assert_eq!(game.rockets[0].velocity, Vec3::new(-0.5, -1.0, -1.5));
            assert_eq!(
                game.rockets[0].random.clone().next_float(),
                rocket_random.next_float()
            );
            assert_eq!(rocket.owner(game), None);

            // A rocket keeps an owner that is neither player nor mob, across a
            // save too, but a player's trident still refuses one.
            let boat_id = game.take_entity_id();
            game.boats.push(super::super::boats::Boat::new(
                boat_id,
                "oak".into(),
                Vec3::ZERO,
                0.0,
            ));
            let boat = game.entity(boat_id).unwrap();
            assert!(!trident.set_owner(game, Some(boat)));
            assert!(rocket.set_owner(game, Some(boat)));
            assert_eq!(rocket.owner(game), Some(boat));
            assert_eq!(game.rockets[0].owner, None);
            let saved = game.uuid_for(boat_id);
            let tag = game.rockets[0].to_nbt([9; 4], None, None);
            let back = Rocket::from_nbt(
                horizon_client::entity_nbt::firework_rocket_from_nbt(&tag, None).unwrap(),
            );
            assert_eq!(back.owner_uuid, Some(saved));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn deflection_follows_vanilla_for_wind_charges_owners_random_and_known_speed() {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let player = player(game);
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(99, kind, Vec3::ZERO, 0.0));
            let mob = game.entity(99).unwrap();

            // A wind charge refuses for its first five ticks, with no change and no draw.
            let mut charge =
                Projectile::new(ProjectileKind::WindCharge, Vec3::ZERO, Some(Who::Mob(99)));
            charge.velocity = Vec3::new(0.0, 1.0, 0.0);
            game.add_projectile(charge);
            let charge = game
                .projectile(game.entity(game.creatures.projectiles[0].id).unwrap())
                .unwrap();
            let random = game.creatures.projectiles[0].random.0.clone().next_float();
            for left in 1..=5 {
                game.creatures
                    .projectiles
                    .update(0, |p| p.no_deflect_ticks = left);
                assert!(!charge.deflect(game, Deflection::Reverse, None, None, true, [1.0; 3]));
                let p = &game.creatures.projectiles[0];
                assert_eq!(p.velocity, Vec3::new(0.0, 1.0, 0.0));
                assert_eq!(p.owner, Some(Who::Mob(99)));
                assert_eq!(p.random.0.clone().next_float(), random);
            }
            game.creatures
                .projectiles
                .update(0, |p| p.no_deflect_ticks = 0);

            // REVERSE of a non-arrow takes one float from its own random, not the level's.
            let level = game.world.random.clone().next_float();
            let mut expect = game.creatures.projectiles[0].random.0.clone();
            let _ = expect.next_float();
            assert!(charge.deflect(game, Deflection::Reverse, None, None, true, [1.0; 3]));
            assert_eq!(game.creatures.projectiles[0].owner, None);
            assert_eq!(game.world.random.clone().next_float(), level);
            assert_eq!(
                game.creatures.projectiles[0].random.0.clone().next_float(),
                expect.next_float()
            );

            // Setting a disallowed arrow's owner to a player allows pickup; `deflect`
            // takes the other overload and does not.
            let mut shot = arrow(Who::Mob(99));
            shot.velocity = Vec3::new(0.0, 0.0, 1.0);
            game.add_projectile(shot);
            let arrow_ref = game
                .projectile(game.entity(game.creatures.projectiles[1].id).unwrap())
                .unwrap();
            assert_eq!(arrow_ref.pickup(game), Some(ArrowPickup::Disallowed));
            assert!(arrow_ref.deflect(
                game,
                Deflection::Reverse,
                None,
                Some(player),
                false,
                [1.0; 3]
            ));
            assert_eq!(arrow_ref.pickup(game), Some(ArrowPickup::Disallowed));
            assert!(arrow_ref.set_owner(game, Some(player)));
            assert_eq!(arrow_ref.pickup(game), Some(ArrowPickup::Allowed));

            // MOMENTUM_DEFLECT normalizes the known speed (the last move), not the velocity.
            game.mobs[0].previous = Vec3::new(0.0, 0.0, -3.0);
            game.mobs[0].position = Vec3::ZERO;
            game.mobs[0].velocity = Vec3::new(5.0, 0.0, 0.0);
            assert!(arrow_ref.deflect(
                game,
                Deflection::MomentumDeflect,
                Some(mob),
                None,
                true,
                [2.0; 3]
            ));
            assert_eq!(
                game.creatures.projectiles[1].velocity,
                Vec3::new(0.0, 0.0, 2.0)
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn momentum_follows_a_boats_displacement_and_a_refused_blow_still_attacks() {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            // A boat that moved along +Z while its velocity points along +X.
            let id = game.take_entity_id();
            let mut boat = super::super::boats::Boat::new(id, "oak".into(), Vec3::ZERO, 0.0);
            boat.previous = Vec3::new(0.0, 0.0, -2.0);
            boat.position = Vec3::ZERO;
            boat.velocity = Vec3::new(5.0, 0.0, 0.0);
            game.boats.push(boat);
            let boat = game.entity(id).unwrap();
            assert_eq!(
                game.native_entity_known_speed(boat),
                Some(Vec3::new(0.0, 0.0, 2.0))
            );
            let mut shot = arrow(Who::Mob(99));
            shot.velocity = Vec3::new(1.0, 0.0, 0.0);
            game.add_projectile(shot);
            let arrow_ref = game
                .projectile(game.entity(game.creatures.projectiles[0].id).unwrap())
                .unwrap();
            assert!(arrow_ref.deflect(
                game,
                Deflection::MomentumDeflect,
                Some(boat),
                None,
                true,
                [3.0; 3]
            ));
            assert_eq!(
                game.creatures.projectiles[0].velocity,
                Vec3::new(0.0, 0.0, 3.0)
            );

            // A player's blow at a new wind charge: refused, yet the attack
            // charge resets and nothing about the charge changes.
            let mut charge =
                Projectile::new(ProjectileKind::WindCharge, Vec3::ZERO, Some(Who::Mob(99)));
            charge.velocity = Vec3::new(0.0, 1.0, 0.0);
            game.add_projectile(charge);
            let i = game.creatures.projectiles.len() - 1;
            game.attack_ticks = 50;
            assert!(!game.deflect(i));
            assert_eq!(game.attack_ticks, 0);
            assert_eq!(
                game.creatures.projectiles[i].velocity,
                Vec3::new(0.0, 1.0, 0.0)
            );
            assert_eq!(game.creatures.projectiles[i].owner, Some(Who::Mob(99)));
            game.creatures
                .projectiles
                .update(i, |p| p.no_deflect_ticks = 0);
            game.attack_ticks = 50;
            assert!(game.deflect(i));
            assert_eq!(game.attack_ticks, 0);
            assert_eq!(game.creatures.projectiles[i].owner, Some(Who::Player));
        });
    }
}
