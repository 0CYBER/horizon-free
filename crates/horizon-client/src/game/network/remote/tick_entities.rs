//! Abilities, pushing, and ticking the server's entities on the client.

use super::*;

impl Game<'_> {
    /// The last `ClientboundPlayerAbilities` the server sent, on a client.
    pub(super) fn remote_abilities_body(&self) -> Option<&play::PlayerAbilities> {
        self.remote.as_ref()?.abilities.as_ref()
    }

    /// The local player's share of every touching entity's
    /// `LivingEntity.pushEntities`, and of the boats' pushes: on a client
    /// `EntitySelector.pushableBy` lets entities push only the local player,
    /// so the server never sends these pushes. `doPush(player)` is
    /// `player.push(entity)`. `before_player`: the mobs that tick before the
    /// player (`ticked_before_player`) and the boats, else the mobs that tick
    /// after it.
    pub(super) fn remote_pushes_body(&self, before_player: bool) -> Vec3 {
        let (Some(p), Some(r)) = (self.player.as_ref(), self.remote.as_ref()) else {
            return Vec3::ZERO;
        };
        // LivingEntity.isPushable: alive, no spectator, off ladders.
        let feet = [
            p.position.x.floor() as i32,
            p.position.y.floor() as i32,
            p.position.z.floor() as i32,
        ];
        if self.screen == Screen::Dead
            || p.spectator
            || self.named_block_in_tag("minecraft:climbable", &self.block_name(feet))
        {
            return Vec3::ZERO;
        }
        let bounds = p.bounds();
        let teams = self.mob_teams();
        // `Entity.getTeam` is `Scoreboard.getPlayersTeam(getScoreboardName())`:
        // a player's score holder is its profile name, every other entity's
        // its UUID text. `mob_teams` keys by UUID and names the local player
        // `PLAYER_NAME`, which misses a server's teams for players.
        let board = &self.score.board;
        let player_team = board.players_team(&r.profile_name);
        let mut push = Vec3::ZERO;
        let order = |id: u32| r.ticked_before_player.contains(&id) == before_player;
        for m in self.mobs.iter().filter(|m| order(m.id)) {
            // Bats never push, nor passengers of one vehicle, nor a spectator; the
            // box test comes before the lookups that far mobs never need (#14053).
            if !m.alive()
                || m.species() == horizon_client::mobs::Species::Bat
                || self
                    .riding
                    .is_some_and(|v| v == m.id || m.extra.mount == Some(v))
                || !m.bounds().intersects(bounds)
            {
                continue;
            }
            let listed = r
                .level_players
                .get(&(m.id as i32))
                .and_then(|uuid| r.tab_players.get(uuid));
            let mob_team =
                listed.map_or_else(|| teams.of(Who::Mob(m.id)), |t| board.players_team(&t.name));
            if listed.is_some_and(|t| t.game_mode == SPECTATOR)
                || !horizon_client::scoreboard::pushable_by(mob_team, player_team, true)
            {
                continue;
            }
            // Entity.push(Entity), `this` the player.
            let (mut xa, mut za) = (m.position.x - p.position.x, m.position.z - p.position.z);
            let mut dd = xa.abs().max(za.abs());
            if dd < f64::from(0.01_f32) {
                continue;
            }
            dd = dd.sqrt();
            xa /= dd;
            za /= dd;
            let pow = (1.0 / dd).min(1.0);
            xa *= pow;
            za *= pow;
            xa *= f64::from(0.05_f32);
            za *= f64::from(0.05_f32);
            push = Vec3::new(push.x - xa, push.y, push.z - za);
        }
        // AbstractBoat.push(player) from the boat's own tick (its box grown
        // by 0.2 sideways) and from the player's pushEntities (its box).
        let area = Vec3::new(f64::from(0.2_f32), f64::from(-0.01_f32), f64::from(0.2_f32));
        // The boats' pushes stay where they were measured: with the entities
        // that tick before the player.
        let boats = if before_player {
            &self.boats[..]
        } else {
            &[][..]
        };
        for b in boats.iter().filter(|b| self.riding != Some(b.id)) {
            let bb = b.bounds();
            let times = usize::from(
                bounds.intersects(bb.inflate(area))
                    && horizon_client::scoreboard::pushable_by(None, player_team, true),
            ) + usize::from(
                bounds.intersects(bb)
                    && horizon_client::scoreboard::pushable_by(player_team, None, true),
            );
            if times > 0
                && super::super::boat_passengers::boat_pushes(bb, bounds, false)
                && let Some(v) = super::super::boat_passengers::push_impulse(b.position, p.position)
            {
                push = push.add_vector(v.scale(times as f64));
            }
        }
        push
    }
    /// ClientLevel.tickEntities; a frozen level only ticks the players
    /// (`TickRateManager.isEntityFrozen`).
    /// `GuardianRenderer.getEntityToLookAt`: a guardian's eye follows its
    /// synched attack target, else the camera.
    pub(super) fn tick_remote_guardian_eyes_body(&mut self, running: bool) {
        use horizon_client::mobs::{Species, Who, guardian};
        if !running {
            return;
        }
        let camera = self.player.as_ref().map(|p| {
            [
                p.position.x,
                p.position.y + f64::from(p.dimensions.eye_height),
                p.position.z,
            ]
        });
        let mut looks = std::mem::take(&mut self.guardian_look_scratch);
        looks.clear();
        looks.extend(
            self.mobs
                .iter()
                .enumerate()
                .filter(|(_, m)| {
                    matches!(m.species(), Species::Guardian | Species::ElderGuardian) && m.alive()
                })
                .filter_map(|(i, m)| {
                    let look = match m.ai.target.filter(|_| m.extra.flag) {
                        Some(Who::Mob(id)) => self
                            .mobs
                            .iter()
                            .find(|t| t.id == id)
                            .map(|t| [t.position.x, t.eye_y(), t.position.z]),
                        _ => camera,
                    };
                    look.map(|look| (i, look))
                }),
        );
        for &(i, look) in &looks {
            guardian::look_at(&mut self.mobs[i], look);
        }
        self.guardian_look_scratch = looks;
    }
    /// Humanoid held arm poses, once the tick has set equipment and use state.
    pub(super) fn tick_remote_arm_poses_body(&mut self) {
        for i in 0..self.mobs.len() {
            let m = &self.mobs[i];
            if !horizon_client::mobs::anim::extracts_humanoid(m) {
                continue;
            }
            // Extraction also runs while the world is frozen: derive the use
            // animation from the current equipment rather than last tick's state.
            if &*m.kind.id != "minecraft:player" {
                let slot = if m.extra.using_off_hand {
                    horizon_items::equipment::OFFHAND
                } else {
                    horizon_items::equipment::MAINHAND
                };
                let animation = (m.extra.using_item > 0)
                    .then(|| m.extra.slot_stack(slot))
                    .flatten()
                    .map_or_else(Default::default, |stack| self.item_use_animation(stack));
                self.mobs[i].extra.use_animation = animation;
            }
            let m = &self.mobs[i];
            let poses = self.held_arm_poses(m.id, m);
            let charge_duration = (&*m.kind.id != "minecraft:player").then(|| {
                let using_slot = if m.extra.using_off_hand {
                    horizon_items::equipment::OFFHAND
                } else {
                    horizon_items::equipment::MAINHAND
                };
                m.extra
                    .slot_stack(using_slot)
                    .map_or(25, |stack| Self::charge_ticks(stack).max(1)) as i32
            });
            self.mobs[i].extra.held_arm_pose = poses;
            if let Some(duration) = charge_duration {
                self.mobs[i].extra.ints[5] = duration;
            }
        }
    }
    pub(super) fn tick_remote_entities_body(&mut self, running: bool) {
        if running {
            self.tick_text_displays();
            self.tick_remote_ominous_item_spawners();
        }
        // EntityFluidInteraction.update for the entities this window only displays: the
        // server never sends the fluid contact, and the renderers read it (cod, salmon and
        // tropical fish lie on their side without `in_water`, arismoko/horizon#1591).
        // A frozen level ticks only the players, as below.
        // Entity.isInRain for the wolves, which dry off by their own `Wolf.tick`.
        let (mut fluids, mut rain, mut standings, mut water_fx) = {
            let r = self
                .remote
                .as_mut()
                .expect("remote entity tick without remote state");
            (
                std::mem::take(&mut r.tick_fluids),
                std::mem::take(&mut r.tick_rain),
                std::mem::take(&mut r.tick_standings),
                std::mem::take(&mut r.tick_water_fx),
            )
        };
        let thundering = {
            let w = self.mob_world_base(&[]);
            let players = self.remote.as_ref().map(|r| &r.level_players);
            fluids.clear();
            fluids.extend(self.mobs.iter().map(|m| {
                (running || players.is_some_and(|p| p.contains_key(&(m.id as i32))))
                    .then(|| w.fluids(m.bounds(), m.eye_y()))
            }));
            rain.clear();
            rain.extend(self.mobs.iter().map(|m| {
                m.species() == horizon_client::mobs::Species::Wolf
                    && horizon_client::mobs::tame::in_rain(m, &w)
            }));
            // The block under each mob the packs' random-model rules read (a grazing
            // cow), one hair below the feet like `Entity.getBlockStateOn`; nothing
            // without rules.
            standings.clear();
            standings.extend(self.mobs.iter().map(|m| {
                horizon_client::mobs::cem::tracks_standing(&m.kind.id).then(|| {
                    let (x, y, z) = (
                        m.position.x.floor() as i32,
                        (m.position.y - 0.0001).floor() as i32,
                        m.position.z.floor() as i32,
                    );
                    w.block_name(x, y, z).to_owned()
                })
            }));
            w.thundering
        };
        // The window's own player already splashes from its motion tick.
        let own = self.remote.as_ref().map(|r| r.entity_id);
        for i in 0..self.mobs.len() {
            let m = &mut self.mobs[i];
            let fluids = fluids[i].take();
            let standing = standings[i].take();
            if let Some(fluids) = fluids {
                let was = m.fluids.in_water;
                let first = m.first_tick;
                m.fluids = fluids;
                // Entity.doWaterSplashEffect on the client copy. Particles
                // only: ClientLevel draws addParticle, and playSeededSound
                // drops Entity.playSound (the server broadcasts that).
                if m.fluids.in_water && !was && !first && Some(m.id as i32) != own {
                    horizon_client::mobs::living::client_water_splash(m, &mut water_fx);
                }
                m.first_tick = false;
            }
            if let Some(name) = standing {
                m.standing_on = Some(name);
            }
        }
        let game_time = self.world.game_time;
        // `AbstractPiglin.isConverting`: the dimension is not piglin safe.
        let nether = !self.dimension_type().piglins_zombify;
        // What a wither's side heads can look at, by network entity id: the
        // mobs and the player (position and eye height).
        let wither_targets: Vec<(i32, [f64; 3], f64)> = if self
            .mobs
            .iter()
            .any(|m| m.species() == horizon_client::mobs::Species::Wither)
        {
            let own = self
                .remote
                .as_ref()
                .zip(self.eye_position())
                .map(|(r, eye)| (r.entity_id, eye, eye[1]));
            self.mobs
                .iter()
                .map(|m| {
                    (
                        m.id as i32,
                        [m.position.x, m.position.y, m.position.z],
                        m.eye_y(),
                    )
                })
                .chain(own)
                .collect()
        } else {
            Vec::new()
        };
        // ExperienceOrb.followNearbyPlayer: the players an orb may go for,
        // their feet and the point `eyeHeight / 2` above them. Skip building when no orbs exist.
        let followed: Vec<([f64; 3], [f64; 3])> = if self.orbs.is_empty() {
            Vec::new()
        } else {
            let own = self
                .player
                .as_ref()
                .filter(|p| !p.health.dead_or_dying() && !self.spectator())
                .map(|p| {
                    let feet = [p.position.x, p.position.y, p.position.z];
                    let eye = f64::from(p.dimensions.eye_height);
                    (feet, [feet[0], feet[1] + eye / 2.0, feet[2]])
                });
            let others = self.remote.as_ref().into_iter().flat_map(|r| {
                self.mobs
                    .iter()
                    .filter(move |m| m.alive() && r.level_players.contains_key(&(m.id as i32)))
                    .filter(move |m| {
                        let listed = r.level_players.get(&(m.id as i32));
                        !listed
                            .and_then(|uuid| r.tab_players.get(uuid))
                            .is_some_and(|t| t.game_mode == SPECTATOR)
                    })
                    .map(|m| {
                        let feet = [m.position.x, m.position.y, m.position.z];
                        (
                            feet,
                            [feet[0], feet[1] + (m.eye_y() - feet[1]) / 2.0, feet[2]],
                        )
                    })
            });
            own.into_iter().chain(others).collect()
        };
        // The mount this client moves ticks in `remote_ride` instead, as its
        // authoritative side (LivingEntity.tick with travelRidden).
        let driven = self.driven_mount().map(|m| m.id);
        // Cat.handleLieDown: the sleeping players (another's, drawn lying in
        // a bed, and this client's own) a lying cat can lie on top of.
        let sleepers: Vec<horizon_core::geometry::Aabb> = if self
            .mobs
            .iter()
            .any(|m| m.species() == horizon_client::mobs::Species::Cat)
        {
            self.remote
                .as_ref()
                .map(|r| {
                    self.mobs
                        .iter()
                        .filter(|m| {
                            r.level_players.contains_key(&(m.id as i32)) && m.extra.ints[1] != 0
                        })
                        .map(|m| {
                            let p = m.position;
                            horizon_core::geometry::Aabb::new(
                                horizon_core::geometry::Vec3::new(p.x - 0.1, p.y, p.z - 0.1),
                                horizon_core::geometry::Vec3::new(p.x + 0.1, p.y + 0.2, p.z + 0.1),
                            )
                        })
                        .chain(self.sleeping.map(|bed| {
                            let p = horizon_core::geometry::Vec3::new(
                                f64::from(bed[0]) + 0.5,
                                f64::from(bed[1]) + 0.6875,
                                f64::from(bed[2]) + 0.5,
                            );
                            horizon_core::geometry::Aabb::new(
                                horizon_core::geometry::Vec3::new(p.x - 0.1, p.y, p.z - 0.1),
                                horizon_core::geometry::Vec3::new(p.x + 0.1, p.y + 0.2, p.z + 0.1),
                            )
                        }))
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let mut lerps = std::mem::take(&mut r.lerps);
        let mut cart_lerps = std::mem::take(&mut r.cart_lerps);
        let mut using = Vec::new();
        let mut mob_fx = water_fx;
        let tick_listened = mod_api::__private::listened(mod_api::HookId::PlayerTickPre)
            || mod_api::__private::listened(mod_api::HookId::PlayerTickPost);
        for (i, rain) in rain.iter().copied().enumerate() {
            let id = self.mobs[i].id;
            let player = self
                .remote
                .as_ref()
                .unwrap()
                .level_players
                .contains_key(&(id as i32));
            if (!running && !player) || driven == Some(id) {
                continue;
            }
            let animation = if player && self.mobs[i].extra.using_item > 0 {
                let extra = &self.mobs[i].extra;
                let slot = if extra.using_off_hand {
                    horizon_items::equipment::OFFHAND
                } else {
                    horizon_items::equipment::MAINHAND
                };
                extra
                    .slot_stack(slot)
                    .map_or_else(Default::default, |stack| self.item_use_animation(stack))
            } else {
                Default::default()
            };
            let player_ref = tick_listened
                .then(|| {
                    self.remote.as_ref().and_then(|r| {
                        let uuid = *r.level_players.get(&(id as i32))?;
                        Some(mod_api::__private::game::player_ref(
                            mod_api::EntityRef::new(self.native_lifecycle.epoch()?, id),
                            uuid,
                        ))
                    })
                })
                .flatten();
            let r = self.remote.as_mut().unwrap();
            let stepped = &mut r.stepped;
            let players = &r.level_players;
            let tab_players = &r.tab_players;
            let m = &mut self.mobs[i];
            let section_from = m.position;
            if player {
                // AvatarRenderer.extractRenderState: isSpectator, from the player list.
                m.extra.spectator = players
                    .get(&(m.id as i32))
                    .and_then(|uuid| tab_players.get(uuid))
                    .is_some_and(|t| t.game_mode == SPECTATOR);
            }
            m.previous = m.position;
            // `Entity.deltaMovementOnPreviousTick`: the elytra's flight yaw
            // lerps from it, so another player's model does not snap when a
            // new tick's velocity arrives.
            m.extra.previous_velocity = m.velocity;
            m.previous_yaw = m.yaw;
            m.previous_body_yaw = m.body_yaw;
            m.previous_head_yaw = m.head_yaw;
            m.previous_head_pitch = m.head_pitch;
            m.previous_limb_speed = m.limb_speed;
            m.age += 1;
            if m.species() == horizon_client::mobs::Species::Rabbit {
                horizon_client::mobs::rabbit::base_tick(m);
            }
            if matches!(
                m.species(),
                horizon_client::mobs::Species::Horse
                    | horizon_client::mobs::Species::Donkey
                    | horizon_client::mobs::Species::Mule
                    | horizon_client::mobs::Species::SkeletonHorse
                    | horizon_client::mobs::Species::ZombieHorse
            ) {
                horizon_client::mobs::equine::tail_tick(m);
            }
            if m.species() == horizon_client::mobs::Species::Creeper {
                horizon_client::mobs::hostile::creeper_client_tick(m);
            }
            if matches!(
                m.species(),
                horizon_client::mobs::Species::Guardian
                    | horizon_client::mobs::Species::ElderGuardian
            ) {
                horizon_client::mobs::guardian::client_tick(m);
            }
            // PolarBear.tick: the stand animation follows the synched flag.
            if m.species() == horizon_client::mobs::Species::PolarBear {
                horizon_client::mobs::polar_bear::client_tick(m);
            }
            // The particles these mobs' own ticks draw on the client side.
            {
                use horizon_client::mobs::{
                    Species as S, cube, illager, illusioner, nautilus, tame, wither,
                };
                match m.species() {
                    S::Wolf => tame::client_tick(m, rain, &mut mob_fx),
                    S::Slime | S::MagmaCube | S::SulfurCube => cube::client_tick(m, &mut mob_fx),
                    S::Nautilus | S::ZombieNautilus => nautilus::client_tick(m, &mut mob_fx),
                    S::Illusioner => {
                        illusioner::client_tick(m, &mut mob_fx);
                        illager::cast_particles(m, &mut mob_fx);
                    }
                    S::Evoker => illager::cast_particles(m, &mut mob_fx),
                    S::Wither => {
                        wither::client_smoke(m, &mut mob_fx);
                        // `getInvulnerableTicks() > 0`: the charge's cloud,
                        // after the heads' smoke as in `aiStep`.
                        wither::charging_particles(m, &mut mob_fx);
                    }
                    _ => {}
                }
            }
            if m.hurt_time > 0 {
                m.hurt_time -= 1;
            }
            if !m.alive() {
                m.death_time += 1;
            }
            // LivingEntity.updateSwingTime.
            m.anim.attack_anim_o = m.anim.attack_anim;
            horizon_client::mobs::ai::tick_swing(m);
            // Entity.tick: the SteppedInterpolationHandler's step.
            if let Some(at) = stepped
                .get_mut(&(m.id as i32))
                .and_then(|h| h.interpolate())
            {
                place_interpolated(m, at);
            }
            // `ClientLevel.tickNonPassenger`'s `commonTick` interpolated the
            // entity before `Player.tick`, which `Pre` precedes.
            if let Some(player) = player_ref {
                self.native_player_tick_pre(mod_api::__private::PlayerTickPreCall { player });
            }
            let head_lerps = &mut self.remote.as_mut().unwrap().head_lerps;
            let m = &mut self.mobs[i];
            // LivingEntity.aiStep: the head's own interpolation.
            if let Some((yaw, steps)) = head_lerps.get_mut(&(m.id as i32))
                && *steps > 0
            {
                m.head_yaw += wrap_degrees(*yaw - m.head_yaw) / *steps as f32;
                *steps -= 1;
            }
            // Fox.tick: the head tilt and crouch follow the synched flags.
            if m.species() == horizon_client::mobs::Species::Fox {
                horizon_client::mobs::fox::client_tick(m);
            }
            // Cat.tick: the lying and relaxing amounts follow the synched flags.
            if m.species() == horizon_client::mobs::Species::Cat {
                m.anim.vo = m.anim.v;
                horizon_client::mobs::tame::cat_amounts(m);
                horizon_client::mobs::tame::cat_lying_on_sleeper(m, &sleepers);
            }
            // Chicken.aiStep: the wings follow the server's on-ground flag.
            if m.species() == horizon_client::mobs::Species::Chicken {
                horizon_client::mobs::farm::chicken_flap(m);
            }
            // Parrot.calculateFlapping: the wings follow the server's
            // on-ground flag and the shoulder it sits on (the glide is the
            // server's).
            if m.species() == horizon_client::mobs::Species::Parrot {
                horizon_client::mobs::parrot::flap(m);
            }
            // LivingEntity.updatingUsingItem: the ticks the item has been
            // used before this one. Another player's use particles; a mob's
            // drawn bow only needs the count (its pull follows it).
            if player {
                // AvatarRenderer.getArmPose: the used item's use animation
                // poses the arm.
                m.extra.use_animation = animation;
            }
            if m.extra.using_item > 0 {
                if player {
                    let item = if m.extra.using_off_hand {
                        m.extra.slot_stack(horizon_items::equipment::OFFHAND)
                    } else {
                        m.extra.slot_stack(horizon_items::equipment::MAINHAND)
                    }
                    .map(|s| s.item.clone());
                    if let Some(item) = item {
                        let eye = m.eye_height();
                        let at = [m.position.x, m.position.y, m.position.z];
                        using.push((
                            item,
                            m.extra.using_item as u32 - 1,
                            at,
                            eye,
                            m.head_pitch,
                            m.yaw,
                        ));
                    }
                }
                m.extra.using_item += 1;
            }
            // LivingEntity.tick's tickHeadTurn: a mob's BodyRotationControl,
            // a player's body following its walk and look.
            if player {
                remote_player_head_turn(m);
                // LivingEntity.tick: the glide count, wings and swim amount.
                m.tick_avatar_pose(m.fluids.in_water);
                // LivingEntity.tick: a sleeper's xRot is 0.
                if m.extra.ints[1] != 0 {
                    m.head_pitch = 0.0;
                    m.previous_head_pitch = 0.0;
                }
            } else if m.alive() {
                m.body_rotation_tick();
            }
            // Squid.aiStep; WalkAnimationState.update from the horizontal distance.
            horizon_client::mobs::aquatic::squid_client_step(m, &mut mob_fx);
            let d = (m.position.x - m.previous.x).hypot(m.position.z - m.previous.z) as f32;
            let (target, factor) = if m.species() == horizon_client::mobs::Species::Frog {
                (horizon_client::mobs::frog::walk_target(m, d), 0.4)
            } else if horizon_client::mobs::camel::is_camel(m.species()) {
                horizon_client::mobs::camel::walk_target(m, d)
            } else {
                ((d * 4.0).min(1.0), 0.4)
            };
            m.limb_speed += (target - m.limb_speed) * factor;
            m.limb_position += m.limb_speed;
            if m.species() == horizon_client::mobs::Species::Rabbit {
                horizon_client::mobs::rabbit::post_tick(m);
            }
            if m.species() == horizon_client::mobs::Species::Bat {
                horizon_client::mobs::bat::setup_animation_states(m);
            }
            // The animation states a server computes in its own `ai_step`,
            // and the particles vanilla draws on the client.
            horizon_client::mobs::sync::client_tick(m, game_time, &mut mob_fx);
            match m.species() {
                // WitherBoss.aiStep: the side heads turn toward their targets.
                horizon_client::mobs::Species::Wither => {
                    horizon_client::mobs::wither::client_tick(m, |id| {
                        wither_targets
                            .iter()
                            .find(|t| t.0 == id)
                            .map(|t| (t.1, t.2))
                    });
                }
                // Hoglin and piglin `isConverting` outside the Nether.
                horizon_client::mobs::Species::Hoglin
                | horizon_client::mobs::Species::Piglin
                | horizon_client::mobs::Species::PiglinBrute => {
                    horizon_client::mobs::piglin::client_converting(m, nether);
                }
                horizon_client::mobs::Species::Panda => {
                    horizon_client::mobs::panda::client_tick(m, thundering);
                }
                // Shulker.tick's onPeekAmountChange: the lid moves this
                // client's player, whose movement is its own.
                horizon_client::mobs::Species::Shulker => {
                    horizon_client::mobs::shulker::client_push(m, &mut mob_fx);
                }
                _ => {}
            }
            let section_id = m.id;
            let section_to = m.position;
            if let Some(player) = player_ref {
                self.native_player_tick_post(mod_api::__private::PlayerTickPostCall { player });
            }
            if section_from != section_to {
                self.remote_entity_section_step(section_id, section_from, section_to);
            }
        }
        // ExperienceOrb.tick runs on a client too (all but the merges and the
        // push out of a block, which are the server's): gravity, buoyancy,
        // collisions and the pull of the nearest player between the server's
        // updates; the server removes it on pickup.
        let (world, facts, collision) = (&self.world, self.res.facts(), self.collision);
        let border = Some(self.border.geometry());
        let mut lava_random = None;
        let mut next_float = || {
            lava_random
                .get_or_insert_with(random_source::entity)
                .next_float()
        };
        if running {
            self.orbs.retain_mut(|o| {
                let distance = |feet: &[f64; 3]| -> f64 {
                    (feet[0] - o.position.x).powi(2)
                        + (feet[1] - o.position.y).powi(2)
                        + (feet[2] - o.position.z).powi(2)
                };
                let target = followed
                    .iter()
                    .filter(|(feet, _)| distance(feet) < 64.0)
                    .min_by(|a, b| distance(&a.0).total_cmp(&distance(&b.0)))
                    .map(|(_, target)| *target);
                // Entity.commonTick: setOldPosAndRot, then
                // LinearInterpolationHandler.doInterpolate moves 1/remaining of
                // the way to the server's position, so the frame draws the step
                // from the old position.
                let old = o.position;
                if let Some(l) = lerps.get_mut(&(o.id as i32)).filter(|l| l.steps > 0) {
                    let alpha = 1.0 / f64::from(l.steps);
                    o.position = Vec3::new(
                        o.position.x + (l.target[0] - o.position.x) * alpha,
                        o.position.y + (l.target[1] - o.position.y) * alpha,
                        o.position.z + (l.target[2] - o.position.z) * alpha,
                    );
                    l.steps -= 1;
                }
                let before = o.position;
                let kept = o.tick(
                    world,
                    facts,
                    collision,
                    target,
                    false,
                    &mut next_float,
                    border,
                    &self.res.motion().fluids,
                    &self.loop_context,
                    &mut |_, _| {},
                );
                // `tick` set `previous` to the stepped position: keep the
                // pre-step one, as `xo` is in vanilla.
                o.previous = old;
                // AbstractInterpolationHandler.applyPredictedMovement: the orb's
                // own move carries the target along (vanilla also checks the
                // moved target for collisions; that check is left out here).
                if let Some(l) = lerps.get_mut(&(o.id as i32)).filter(|l| l.steps > 0) {
                    l.target[0] += o.position.x - before.x;
                    l.target[1] += o.position.y - before.y;
                    l.target[2] += o.position.z - before.z;
                }
                kept
            });
        }
        // ItemEntity.tick runs on a client too: gravity, collisions and
        // friction move the item between the server's updates (every 20
        // ticks). Ordinary removal remains server-authoritative. Mods can opt
        // into predicting consumption at block contact.
        self.remote
            .as_mut()
            .unwrap()
            .head_lerps
            .retain(|_, (_, steps)| *steps > 0);
        let mut entered = Vec::new();
        if running {
            entity_section_steps::tick_items(self, &mut entered);
        }
        // Entity.doWaterSplashEffect on this window: Level.addParticle draws
        // the bubbles and splashes. ClientLevel.playSeededSound drops
        // Entity.playSound, so the sound stays the server's.
        for splash in entered {
            self.apply_item_splash(splash);
        }
        let _ = self.handle_mob_events_reusable(&mut mob_fx);
        if let Some(r) = self.remote.as_mut() {
            r.tick_fluids = fluids;
            r.tick_rain = rain;
            r.tick_standings = standings;
            r.tick_water_fx = mob_fx;
        }
        for (item, ticks, at, eye, pitch, yaw) in using {
            self.remote_player_eating(&item, ticks, at, eye, pitch, yaw);
        }
        // ClientLevel.tick: the sky flash fades in a frozen level too.
        self.flash = self.flash.saturating_sub(1);
        if running {
            self.guardian_bubbles();
            // The dragon: LivingEntity's interpolation, inside its aiStep.
            let dragon_lerp = self
                .dragon
                .as_ref()
                .and_then(|d| lerps.get_mut(&(d.id as i32)))
                .filter(|l| l.steps > 0)
                .map(|l| {
                    let steps = l.steps;
                    l.steps -= 1;
                    (l.target, l.rotation.map(|r| r.0), steps)
                });
            self.tick_remote_dragon(dragon_lerp);
            // Armor stands: ClientLevel.tickNonPassenger's setOldPosAndRot,
            // then their SteppedInterpolationHandler's step, as the mobs'.
            if let Some(r) = self.remote.as_mut() {
                for a in &mut self.armor_stands {
                    let step = r
                        .stepped
                        .get_mut(&(a.id as i32))
                        .and_then(|h| h.interpolate());
                    a.interpolate(step.map(|at| {
                        let p = at.position;
                        (Vec3::new(p[0], p[1], p[2]), at.yaw)
                    }));
                }
            }
            // The yaw of a boat eases toward the server's over the same steps
            // as its position, read before `step` counts one off.
            let turns: HashMap<i32, (f32, u32)> = lerps
                .iter()
                .filter(|(_, l)| l.steps > 0)
                .filter_map(|(id, l)| l.rotation.map(|r| (*id, (r.0, l.steps))))
                .collect();
            // Boats, minecarts, projectiles and the rest ease toward the
            // server's position over three ticks (`lerps`).
            let mut step = |id: u32, at: Vec3| match lerps.get_mut(&(id as i32)) {
                Some(l) if l.steps > 0 => {
                    let n = f64::from(l.steps);
                    l.steps -= 1;
                    Vec3::new(
                        at.x + (l.target[0] - at.x) / n,
                        at.y + (l.target[1] - at.y) / n,
                        at.z + (l.target[2] - at.z) / n,
                    )
                }
                _ => at,
            };
            self.tick_remote_objects(
                &mut step,
                |id, yaw| {
                    turns.get(&(id as i32)).map_or(yaw, |&(target, steps)| {
                        super::super::remote_objects::lerp_yaw_step(yaw, target, steps)
                    })
                },
                |c| {
                    cart_lerps
                        .get_mut(&(c.id as i32))
                        .is_some_and(|l| l.tick(c))
                },
            );
            self.tick_remote_projectiles(&mut step);
            // LightningBolt.tick on a client.
            self.tick_bolts();
        }
        lerps.retain(|_, l| l.steps > 0);
        if let Some(r) = self.remote.as_mut() {
            for (id, l) in lerps {
                r.lerps.entry(id).or_insert(l);
            }
            for (id, l) in cart_lerps {
                r.cart_lerps.entry(id).or_insert(l);
            }
        }
    }
}
