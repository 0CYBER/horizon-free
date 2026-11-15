//! Opt-in client targeting. Vanilla reach attributes and server attack validation stay intact.
use super::{Game, Hit, commands::Target};
use mod_api::{ClientTargeting, CrosshairTarget, EntityRef};

fn target_point(eye: [f64; 3], bounds: [[f64; 3]; 2], reach: f64) -> Option<([f64; 3], f64)> {
    if !reach.is_finite()
        || reach < 0.0
        || (0..3).any(|i| {
            !eye[i].is_finite()
                || !bounds[0][i].is_finite()
                || !bounds[1][i].is_finite()
                || bounds[0][i] > bounds[1][i]
        })
    {
        return None;
    }
    let point = std::array::from_fn(|i| eye[i].clamp(bounds[0][i], bounds[1][i]));
    let distance = (0..3)
        .map(|i| (point[i] - eye[i]).powi(2))
        .sum::<f64>()
        .sqrt();
    (distance <= reach).then_some((point, distance))
}

impl Game<'_> {
    /// The same traversal without `filterHitResult`: the block hit that
    /// bounds the entity ray in `GameRenderer.pick`, out to the larger
    /// interaction range — a wall between the two still hides the entity.
    pub(super) fn pick_unfiltered(&self) -> Option<Hit> {
        if let Some(hit) = self.forced_hit {
            return Some(hit);
        }
        let (origin, dir) = self.aim()?;
        self.clip_ray(origin, dir, self.block_reach.max(self.entity_reach))
    }

    pub(super) fn targeting(&self) -> ClientTargeting {
        let mut options = self.client_targeting.get();
        options.entity = options.entity.filter(|e| self.entity(e.id()) == Some(*e));
        options
    }
    pub(super) fn set_targeting(&self, targeting: ClientTargeting) -> bool {
        if !self.native_window()
            || self.title.is_some()
            || self.player.is_none()
            || self.native_lifecycle.epoch().is_none()
            || !self.remote.as_ref().is_some_and(|r| r.logged_in)
            || targeting.entity.is_some_and(|e| {
                self.entity(e.id()) != Some(e)
                    || self
                        .remote
                        .as_ref()
                        .is_some_and(|r| r.entity_id as u32 == e.id())
            })
        {
            return false;
        }
        if targeting.pick_liquids != self.client_targeting.get().pick_liquids {
            self.tick_hit.set(None);
        }
        self.client_targeting.set(targeting);
        true
    }
    pub(super) fn clip_targeting_ray(
        &self,
        origin: [f64; 3],
        dir: [f64; 3],
        reach: f64,
    ) -> Option<Hit> {
        if !self.native_window() || !self.targeting().pick_liquids {
            return self.clip_ray(origin, dir, reach);
        }
        use horizon_core::geometry::Vec3;
        use horizon_world::world_ray::{RayBlockMode, RayFluidMode, clip_world};
        // Boat placement already supplies the block outlines and real fluid shapes.
        let mut rays = super::boats::OutlineFluidRay(self);
        let start = Vec3::new(origin[0], origin[1], origin[2]);
        let end = Vec3::new(
            origin[0] + dir[0] * reach,
            origin[1] + dir[1] * reach,
            origin[2] + dir[2] * reach,
        );
        let result = clip_world(
            &mut rays,
            start,
            end,
            RayBlockMode::Outline,
            RayFluidMode::Any,
            4096,
        )
        .ok()?;
        if result.miss {
            return None;
        }
        let hit = result.hit;
        Some(Hit {
            pos: [hit.block.x, hit.block.y, hit.block.z],
            face: hit.direction.normal(),
            point: [hit.location.x, hit.location.y, hit.location.z],
            inside: hit.inside,
        })
    }
    pub(super) fn targeting_entity_hit(&self, entity: EntityRef) -> Option<([f64; 3], f64)> {
        let remote = self.remote.as_ref().filter(|r| r.logged_in)?;
        if !self.native_window() || remote.entity_id as u32 == entity.id() {
            return None;
        }
        let target = self.native_client_entity_target(entity)?;
        let attackable = match target {
            Target::Mob(_)
            | Target::Stand(_)
            | Target::Frame(_)
            | Target::Painting(_)
            | Target::Boat(_)
            | Target::Cart(_)
            | Target::Crystal(_)
            | Target::DragonPart(_)
            | Target::Cushion(_) => true,
            Target::Other(id) => self.interactions.contains_key(&(id as i32)),
            Target::Projectile(id) => self.creatures.projectiles.iter().any(|p| {
                p.id == id
                    && (super::deflect::redirectable(p)
                        || matches!(
                            p.kind,
                            horizon_client::projectiles::ProjectileKind::ShulkerBullet { .. }
                        ))
            }),
            _ => false,
        };
        if !attackable {
            return None;
        }
        let bounds = self.entity_bounding_box(entity).or_else(|| {
            self.tracked_special_box(entity.id() as i32).map(|b| {
                let (a, b) = (b.min_position(), b.max_position());
                [[a.x, a.y, a.z], [b.x, b.y, b.z]]
            })
        })?;
        let eye = self.eye_position()?;
        let (point, distance) = target_point(eye, bounds, self.entity_reach)?;
        if !self.held_range_allows(distance) {
            return None;
        }
        if distance > 0.0 && !self.targeting().hit_through_walls {
            let dir = std::array::from_fn(|i| (point[i] - eye[i]) / distance);
            if self.clip_ray(eye, dir, distance).is_some() {
                return None;
            }
        }
        Some((point, distance))
    }
    /// The optional batch is resolved once for this click, then each hit uses vanilla's send path.
    pub(super) fn mod_attack_targets(&mut self) -> bool {
        let overridden = self.targeting().entity.is_some();
        if !overridden && !mod_api::__private::listened(mod_api::HookId::ClientAttackTarget) {
            return false;
        }
        let selected = match self.crosshair_target() {
            Some(CrosshairTarget::Entity { entity, .. }) => Some(entity),
            _ => None,
        };
        let mut targets: Vec<_> = selected.into_iter().collect();
        let mut canceled = false;
        self.native_client_attack_target(&mut targets, &mut canceled);
        if canceled {
            return true;
        }
        let mut seen = std::collections::HashSet::new();
        for &entity in &targets {
            if !seen.insert(entity) {
                continue;
            }
            if let Some((_, distance)) = self.targeting_entity_hit(entity) {
                self.remote_attack_entity(entity.id() as i32, distance);
            }
        }
        overridden || selected.is_some() || !targets.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_range_uses_nearest_box_point_and_rejects_invalid_geometry() {
        let bounds = [[2.0, -1.0, -1.0], [4.0, 1.0, 1.0]];
        assert_eq!(
            target_point([0.0; 3], bounds, 2.0),
            Some(([2.0, 0.0, 0.0], 2.0))
        );
        assert_eq!(target_point([0.0; 3], bounds, 1.999), None);
        assert_eq!(
            target_point([3.0, 0.0, 0.0], bounds, 0.0),
            Some(([3.0, 0.0, 0.0], 0.0))
        );
        for reach in [-1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(target_point([0.0; 3], bounds, reach), None);
        }
        assert_eq!(target_point([f64::NAN; 3], bounds, 3.0), None);
        assert_eq!(target_point([0.0; 3], [bounds[1], bounds[0]], 3.0), None);
    }

    #[derive(Default)]
    struct AttackBatch {
        targets: std::sync::Mutex<Vec<EntityRef>>,
        cancel: std::sync::atomic::AtomicBool,
    }
    struct Observer(std::sync::Arc<AttackBatch>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Observer {
        fn client_attack_target(
            &self,
            _: &mut dyn mod_api::ClientHost,
            event: &mut mod_api::ClientAttackTargetEvent<'_>,
        ) -> mod_api::Flow {
            *event.targets = self.0.targets.lock().unwrap().clone();
            if self.0.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                mod_api::Flow::Cancel
            } else {
                mod_api::Flow::Continue
            }
        }
    }

    fn finish_login(server: &mut horizon_protocol::connection::Connection) {
        use horizon_protocol::packets::{Empty, configuration as cfg, login};
        server.recv().unwrap(); // Intention.
        server.recv().unwrap(); // Hello.
        server
            .send(
                &login::Clientbound::LoginFinished(login::LoginFinished {
                    profile: login::GameProfile {
                        uuid: 42,
                        name: "SessionReadTest".into(),
                        properties: vec![],
                    },
                    session: 1,
                })
                .encode(),
            )
            .unwrap();
        let (id, payload) = server.recv().unwrap();
        assert!(matches!(
            login::Serverbound::decode(id, &payload).unwrap(),
            login::Serverbound::LoginAcknowledged(_)
        ));
        server.recv().unwrap(); // Brand.
        server.recv().unwrap(); // Client information.
        server
            .send(&cfg::Clientbound::FinishConfiguration(Empty).encode())
            .unwrap();
        let (id, payload) = server.recv().unwrap();
        assert!(matches!(
            cfg::Serverbound::decode(id, &payload).unwrap(),
            cfg::Serverbound::FinishConfiguration(_)
        ));
    }

    /// A FIFO swing packet fences the real connection's outgoing attack packets.
    fn sent_attacks(
        game: &Game<'_>,
        server: &mut horizon_protocol::connection::Connection,
    ) -> Vec<i32> {
        use horizon_protocol::packets::{Empty, play::Serverbound as S};
        game.send(S::Punch(Empty));
        receive_attacks(server)
    }

    fn receive_attacks(server: &mut horizon_protocol::connection::Connection) -> Vec<i32> {
        use horizon_protocol::packets::play::Serverbound as S;
        let mut ids = Vec::new();
        loop {
            let (id, payload) = server.recv().unwrap();
            match S::decode(id, &payload).unwrap() {
                S::Attack(attack) => ids.push(attack.id),
                S::Punch(_) => return ids,
                _ => {}
            }
        }
    }

    #[test]
    #[ignore = "daily"]
    fn client_targeting_respects_range_walls_liquids_and_handle_lifetime() {
        use crate::game::{
            native_client_session::tests::logged_client,
            native_entity_refs::tests::with_client_game,
        };
        use horizon_client::{chunk::Chunk, motion::PlayerMotion};
        use horizon_core::geometry::Vec3;
        use horizon_protocol::packets::play::{self, Clientbound as C};
        let state = std::sync::Arc::new(AttackBatch::default());
        let mods = vec![mod_api::ModEntry::new(
            "targeting_test",
            Box::new(Observer(state.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_client_game(|game| {
                assert!(!game.set_targeting(ClientTargeting::default()));
                let mut server =
                    horizon_protocol::connection::Connection::local(logged_client(game));
                server
                    .reader
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                finish_login(&mut server);
                game.native_lifecycle
                    .begin_open(mod_api::Authority::RemoteClient);
                game.title = None;
                game.player =
                    Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 1.0, 0.5)).unwrap());
                game.entity_reach = 3.0;
                game.world
                    .chunks
                    .insert((0, 0), std::sync::Arc::new(Chunk::empty(0, 0)));
                let kind = game
                    .remote
                    .as_ref()
                    .unwrap()
                    .builtin
                    .get("minecraft:entity_type")
                    .unwrap()
                    .id("minecraft:pig")
                    .unwrap();
                for (id, position) in [(31, [2.5, 2.0, 0.5]), (32, [0.5, 2.0, 2.5])] {
                    game.handle_packet(C::AddEntity(play::AddEntity {
                        id,
                        uuid: id as u128,
                        kind,
                        position,
                        velocity: [0.0; 3],
                        pitch: 0,
                        yaw: 0,
                        head_yaw: 0,
                        data: 0,
                    }))
                    .unwrap();
                }
                let entity = game.entity(31).unwrap();
                let mut options = ClientTargeting::default();
                options.entity = Some(entity);
                assert!(game.set_targeting(options));
                assert!(
                    matches!(game.crosshair_target(), Some(CrosshairTarget::Entity { entity: hit, .. }) if hit == entity)
                );
                let frame_kind = game
                    .remote
                    .as_ref()
                    .unwrap()
                    .builtin
                    .get("minecraft:entity_type")
                    .unwrap()
                    .id("minecraft:item_frame")
                    .unwrap();
                game.handle_packet(C::AddEntity(play::AddEntity {
                    id: 33,
                    uuid: 33,
                    kind: frame_kind,
                    position: [2.5, 2.0, 0.5],
                    velocity: [0.0; 3],
                    pitch: 0,
                    yaw: 0,
                    head_yaw: 0,
                    data: 0,
                }))
                .unwrap();
                let frame = game.entity(33).unwrap();
                assert!(game.targeting_entity_hit(frame).is_some());
                let mut held = horizon_items::items::ItemStack::new("minecraft:stick", 1);
                held.data_mut().set_other(
                    "minecraft:attack_range",
                    Some(horizon_core::nbt::Tag::Compound(
                        [
                            (
                                "min_reach".into(),
                                horizon_core::nbt::Tag::Float(0.0f32.to_bits()),
                            ),
                            (
                                "max_reach".into(),
                                horizon_core::nbt::Tag::Float(0.25f32.to_bits()),
                            ),
                            (
                                "hitbox_margin".into(),
                                horizon_core::nbt::Tag::Float(0.0f32.to_bits()),
                            ),
                        ]
                        .into_iter()
                        .collect(),
                    )),
                );
                game.inventory[0] = Some(held);
                assert!(game.targeting_entity_hit(entity).is_none());
                assert!(game.targeting_entity_hit(frame).is_none());
                let mut frame_override = options;
                frame_override.entity = Some(frame);
                assert!(game.set_targeting(frame_override));
                assert!(game.crosshair_target().is_none());
                *state.targets.lock().unwrap() = vec![entity, frame];
                assert!(game.mod_attack_targets());
                assert!(sent_attacks(game, &mut server).is_empty());
                game.inventory[0] = None;
                assert!(game.set_targeting(options));
                let second = game.entity(32).unwrap();
                *state.targets.lock().unwrap() = vec![entity, second, entity];
                game.remote_start_attack();
                assert_eq!(receive_attacks(&mut server), [31, 32]);
                state
                    .cancel
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                assert!(game.mod_attack_targets());
                assert!(sent_attacks(game, &mut server).is_empty());
                state
                    .cancel
                    .store(false, std::sync::atomic::Ordering::Relaxed);
                *state.targets.lock().unwrap() = vec![entity; 65];
                assert!(game.mod_attack_targets());
                assert_eq!(sent_attacks(game, &mut server), [31]);
                let many = (0..65).flat_map(|_| [second, entity]).collect();
                *state.targets.lock().unwrap() = many;
                assert!(game.mod_attack_targets());
                assert_eq!(sent_attacks(game, &mut server), [32, 31]);
                for id in 100..165 {
                    game.handle_packet(C::AddEntity(play::AddEntity {
                        id,
                        uuid: id as u128,
                        kind,
                        position: [2.5, 2.0, 0.5],
                        velocity: [0.0; 3],
                        pitch: 0,
                        yaw: 0,
                        head_yaw: 0,
                        data: 0,
                    }))
                    .unwrap();
                }
                *state.targets.lock().unwrap() =
                    (100..165).map(|id| game.entity(id).unwrap()).collect();
                assert!(game.mod_attack_targets());
                assert_eq!(
                    sent_attacks(game, &mut server),
                    (100..165).collect::<Vec<_>>()
                );
                *state.targets.lock().unwrap() = vec![entity];
                game.yaw = 90.0; // The override is off the aiming ray.
                assert!(game.entity_targeted());
                game.entity_reach = 1.0;
                assert!(!game.entity_targeted());
                assert!(game.targeting_entity_hit(entity).is_none());
                assert!(game.mod_attack_targets());
                assert!(sent_attacks(game, &mut server).is_empty());
                game.entity_reach = 3.0;
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(1, 2, 0, stone).unwrap();
                assert!(game.targeting_entity_hit(entity).is_none());
                let mut through_walls = options;
                through_walls.hit_through_walls = true;
                assert!(game.set_targeting(through_walls));
                assert!(game.targeting_entity_hit(entity).is_some());
                let water = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:water")
                    .unwrap();
                game.world.set(1, 2, 0, water).unwrap();
                assert!(game.set_targeting(ClientTargeting::default()));
                game.yaw = -90.0;
                game.pitch = 0.0;
                game.pick_tick_targets(); // Cache the ordinary miss before enabling liquids.
                assert!(game.pick().is_none());
                let mut liquids = ClientTargeting::default();
                liquids.pick_liquids = true;
                assert!(game.set_targeting(liquids));
                assert_eq!(game.pick().unwrap().pos, [1, 2, 0]);
                assert!(game.targeting_entity_hit(entity).is_some());
                assert!(game.block_distance().is_none());
                game.world.set(0, 2, 0, water).unwrap(); // Eyes inside water.
                assert!(game.targeting_entity_hit(entity).is_some());
                assert!(game.block_distance().is_none());
                game.pick_tick_targets(); // Cache the fluid hit before disabling liquids.
                assert!(game.set_targeting(ClientTargeting::default()));
                assert!(game.pick().is_none());
                game.release_tick_targets();
                assert!(game.set_targeting(options));
                let mut own = options;
                own.entity = game.entity(7);
                assert!(!game.set_targeting(own));
                assert_eq!(game.targeting(), options);
                game.native_lifecycle
                    .begin_open(mod_api::Authority::RemoteClient);
                assert!(!game.set_targeting(options));
                assert_eq!(game.targeting().entity, None);
                assert!(game.mod_attack_targets());
                assert!(sent_attacks(game, &mut server).is_empty());
            })
        });
    }
}
