//! Landing impacts for falling blocks, before their placement/drop decision,
//! and `Block.fallOn` for every entity that isn't living.
use super::{FallingBlock, Game, Landing};
use horizon_client::falling_block;
use horizon_core::{
    geometry::{Aabb, Vec3, VoxelShape},
    position::BlockPos,
    random::RandomSource,
};
use horizon_world::{
    block_entity_push::BlockPushWorld,
    block_landing::{
        BlockLandingWorld, DamageSourceOwner, LandingDamage, LandingEntityKind, LandingEvent,
        LandingSound,
    },
    entity_obstruction::EntityIdentity,
    level::LevelEvent,
    registry::BlockStateId,
};

/// An entity that isn't living, landing (`Entity.checkFallDamage` -> `Block.fallOn`).
/// Only farmland (the level random's draw), honey (the slide sound and entity
/// event 54) and the fall-damage calls reach one: it writes no block.
struct NonLivingLanding<'g, 'a> {
    game: &'g mut Game<'a>,
    id: u32,
    at: Vec3,
    bounds: Aabb,
    /// What `causeFallDamage` was called with: a falling block hurts by it.
    fall_damage: Option<f64>,
}
impl BlockPushWorld for NonLivingLanding<'_, '_> {
    type Error = String;
    fn collision_shape(&mut self, state: BlockStateId, p: BlockPos) -> Result<VoxelShape, String> {
        let mut scene = horizon_client::collisions::SceneCollisions {
            world: &self.game.world,
            binding: self.game.collision,
            subject: None,
        };
        self.game
            .collision
            .collision_shape(&mut scene, state, [p.x, p.y, p.z])
            .map_err(|e| e.to_string())
    }
    fn entities(&mut self, _: Aabb) -> Result<Vec<EntityIdentity>, String> {
        Ok(Vec::new())
    }
    fn entity_bounds(&self, _: EntityIdentity) -> Aabb {
        self.bounds
    }
    fn teleport_relative(&mut self, _: EntityIdentity, _: Vec3) -> Result<(), String> {
        Ok(())
    }
}
impl BlockLandingWorld for NonLivingLanding<'_, '_> {
    type DamageSource = ();
    type Sound = Option<&'static str>;
    fn is_server_level(&self) -> bool {
        true
    }
    fn is_client_side(&mut self) -> Result<bool, String> {
        Ok(false)
    }
    // `level.getRandom()`.
    fn random_float(&mut self) -> Result<f32, String> {
        Ok(self.game.world.random.next_float())
    }
    fn random_int(&mut self, bound: i32) -> Result<i32, String> {
        self.game
            .world
            .random
            .next_int_bound(bound)
            .map_err(|e| e.to_string())
    }
    fn under_spawn_protection(&mut self, _: BlockPos) -> Result<bool, String> {
        Ok(false)
    }
    fn mob_griefing(&mut self) -> Result<bool, String> {
        Ok(false)
    }
    fn entity_dimensions(&self) -> (f32, f32) {
        let size = self
            .bounds
            .max_position()
            .subtract(self.bounds.min_position());
        (size.x as f32, size.y as f32)
    }
    fn suppressing_bounce(&mut self) -> Result<bool, String> {
        Ok(false)
    }
    fn damage_source(&mut self, _: DamageSourceOwner, _: LandingDamage) -> Result<(), String> {
        Ok(())
    }
    // `Entity.causeFallDamage` is false for all of these; a falling block's
    // override hurts what it lands in (the caller does, by this distance).
    fn cause_fall_damage(&mut self, distance: f64, _: f32, _: ()) -> Result<bool, String> {
        self.fall_damage = Some(distance);
        Ok(false)
    }
    fn known_sound(&self, sound: LandingSound) -> Option<&'static str> {
        match sound {
            LandingSound::HoneySlide => Some("minecraft:block.honey_block.slide"),
            LandingSound::HoneyFall => Some("minecraft:block.honey_block.fall"),
            LandingSound::TurtleEggBreak => Some("minecraft:entity.turtle.egg_break"),
            LandingSound::ShelfMushroomBounce => None,
        }
    }
    fn living_fall_sound(&mut self, _: bool) -> Result<Option<&'static str>, String> {
        Ok(None)
    }
    // `Entity.playSound`: none from a silent entity, on `getSoundSource()` (NEUTRAL).
    fn entity_sound(
        &mut self,
        sound: Option<&'static str>,
        volume: f32,
        pitch: f32,
    ) -> Result<(), String> {
        if let Some(name) = sound
            && !self.game.entity_silent(self.id)
        {
            let at = [self.at.x, self.at.y, self.at.z];
            self.game.audio.play_on(6, name, Some(at), volume, pitch);
        }
        Ok(())
    }
    fn block_sound(
        &mut self,
        _: BlockPos,
        _: Option<&'static str>,
        _: f32,
        _: f32,
    ) -> Result<(), String> {
        Ok(())
    }
    // `Level.broadcastEntityEvent`.
    fn broadcast_entity_event(&mut self, event: u8) -> Result<(), String> {
        let at = [self.at.x, self.at.y, self.at.z];
        self.game.lan_entity_event(self.id as i32, event as i8, at);
        Ok(())
    }
    fn destroy_block(&mut self, _: BlockPos, _: bool) -> Result<bool, String> {
        Ok(false)
    }
    fn set_block(&mut self, _: BlockPos, _: BlockStateId, _: u32) -> Result<bool, String> {
        Ok(false)
    }
    fn block_event(
        &mut self,
        _: LandingEvent,
        _: BlockPos,
        _: BlockStateId,
        _: bool,
    ) -> Result<(), String> {
        Ok(())
    }
    fn level_event(&mut self, _: i32, _: BlockPos, _: i32) -> Result<(), String> {
        Ok(())
    }
}

impl Game<'_> {
    /// `Block.fallOn` for entity `id`, which isn't living, landing at `at`
    /// (its box `bounds`) after falling `fall`, on its supporting block (else
    /// `getOnPosLegacy`). Returns the distance it called `causeFallDamage`
    /// with: none on powder snow, the fall plus 2.5 on a stalagmite's tip,
    /// half on a bed. A client runs none of it.
    pub(super) fn non_living_fall_on(
        &mut self,
        id: u32,
        at: Vec3,
        bounds: Aabb,
        fall: f64,
    ) -> Option<f64> {
        if self.remote.is_some() || self.options.execution_role.is_client() {
            return None;
        }
        let pos = self.landing_pos(at, bounds);
        let state = self.world.block(pos.x, pos.y, pos.z);
        let motion = self.motion;
        let mut landing = NonLivingLanding {
            game: self,
            id,
            at,
            bounds,
            fall_damage: None,
        };
        if let Err(e) =
            motion
                .landing
                .fall_on(&mut landing, state, pos, LandingEntityKind::Other, fall)
        {
            log::warn!("Horizon: fallOn: {e}");
        }
        landing.fall_damage
    }
    /// `FallingBlockEntity.causeFallDamage`: impact damage and anvil wear,
    /// before block effects and portal handling can transfer the entity.
    pub(super) fn falling_block_impact(&mut self, block: &mut FallingBlock, distance: f64) {
        let name = self.res.facts().get(block.state).name.clone();
        let anvil = name.ends_with("anvil");
        let stalactite = matches!(
            &*name,
            "minecraft:pointed_dripstone" | "minecraft:sulfur_spike"
        );
        if let Some(impact) = block.damage.impact(distance) {
            if impact.amount > 0.0 {
                let mob_kind = if stalactite {
                    horizon_client::mobs::DamageKind::FallingStalactite
                } else if anvil {
                    horizon_client::mobs::DamageKind::FallingAnvil
                } else {
                    horizon_client::mobs::DamageKind::FallingBlock
                };
                let attack = horizon_client::health::Attack {
                    attacker: block.id as i32,
                    direct: Some(block.id as i32),
                    position: Some([block.position.x, block.position.y, block.position.z]),
                    ..Default::default()
                };
                falling_block::hurt_mobs_with_kind(
                    crate::game::entity::damage_level_of!(self, &attack),
                    &mut self.mobs,
                    block.position,
                    impact.amount,
                    mob_kind,
                );
                // Enderman.hurtServer: a falling block is no living attacker.
                if let Err(e) = self.settle_hurt_teleports() {
                    log::warn!("Horizon: falling block: enderman teleport: {e}");
                }
                let kind = if stalactite {
                    "minecraft:falling_stalactite"
                } else if anvil {
                    "minecraft:falling_anvil"
                } else {
                    "minecraft:falling_block"
                };
                self.falling_hurt_player(block.id, block.position, impact.amount, kind);
                if let Some(host) = self.lan.as_ref() {
                    let bounds = falling_block::bounds(block.position);
                    let conns = host
                        .hub
                        .guests
                        .iter()
                        .filter(|g| {
                            host.hub.here(g)
                                && g.loaded
                                && !g.dead
                                && !g.creative()
                                && !g.spectator()
                                && bounds.intersects(super::lan_mobs::guest_bounds(g))
                        })
                        .map(|g| g.conn)
                        .collect::<Vec<_>>();
                    for conn in conns {
                        self.guest_hit_by(conn, |g| {
                            g.falling_hurt_player(block.id, block.position, impact.amount, kind)
                        });
                    }
                }
            }
            // `this.random.nextFloat()`: the falling block's own (unseeded)
            // random, not the level's.
            if anvil
                && impact.wear_chance > 0.0
                && horizon_client::random_source::entity().next_float() < impact.wear_chance
                && let Some(mut state) = self.pack.assets.registry.state(block.state).cloned()
            {
                if falling_block::wear_anvil(&mut state) {
                    block.state = self
                        .pack
                        .assets
                        .registry
                        .resolve(&state)
                        .unwrap_or(block.state);
                } else {
                    block.damage.cancel_drop = true;
                }
            }
        }
    }

    /// `FallingBlockEntity.tick`: finish a landing, including the placed block
    /// entity and the carried `TileEntityData` applied through loadWithComponents.
    pub(super) fn finish_falling_block(&mut self, block: FallingBlock, pos: [i32; 3]) {
        let name = self.res.facts().get(block.state).name.clone();
        let anvil = name.ends_with("anvil");
        // Fallable.onBrokenAfterFall: the anvil's, the pointed dripstone's and
        // the sulfur spike's level event.
        let sound_event = match &*name {
            "minecraft:pointed_dripstone" => Some(1045),
            "minecraft:sulfur_spike" => Some(1052),
            _ if anvil => Some(1029),
            _ => None,
        };
        if block.damage.cancel_drop {
            if let Some(id) = sound_event {
                self.level_events.push(LevelEvent::LevelEvent {
                    id,
                    pos: BlockPos::new(pos[0], pos[1], pos[2]),
                    data: 0,
                    source: None,
                });
            } else if self
                .res
                .facts()
                .get(block.state)
                .name
                .starts_with("minecraft:suspicious_")
            {
                // BrushableBlock.onBrokenAfterFall.
                let at = BlockPos::new(pos[0], pos[1], pos[2]);
                self.level_events.push(LevelEvent::LevelEvent {
                    id: 2001,
                    pos: at,
                    data: block.state.0 as i32,
                    source: None,
                });
                self.level_events.push(LevelEvent::GameEvent {
                    pos: at,
                    event: "minecraft:block_destroy",
                    state: None,
                });
            }
            return;
        }
        let landing = self.with_level(|b, level, ticks| {
            b.falling_block_landed(
                level,
                ticks,
                BlockPos::new(pos[0], pos[1], pos[2]),
                block.state,
                false,
            )
        });
        if let Landing::Placed(state) = landing {
            let name = self.res.facts().get(state).name.clone();
            // LevelChunk.setBlockState creates the new block entity even when
            // the falling entity has no supplied data.
            let key = (pos[0], pos[1], pos[2]);
            if !self.world.block_entities.stateful_entity(state.0)
                && let Some(entity) = self.new_block_entity_for_state(state)
            {
                self.world.block_entities.insert(key, entity);
            }
            if let Some(entity) = self.world.block_entities.get(&key) {
                let mut tag = horizon_client::save::block_entity_encode::encode(key, entity, &name);
                // FallingBlockEntity.tick copies each supplied field over the
                // new block entity's saved defaults before loadWithComponents.
                if let Some(horizon_core::nbt::Tag::Compound(carried)) = block
                    .saved
                    .as_ref()
                    .and_then(|saved| saved.1.get("TileEntityData"))
                    && let horizon_core::nbt::Tag::Compound(fields) = &mut tag
                {
                    fields.extend(carried.clone());
                    self.load_block_entity_nbt(pos, &horizon_client::snbt::to_snbt(&tag));
                }
            }
        }
        // FallingBlockEntity.tick: the players tracking the entity are told to
        // show the block for a moment, after the block update.
        if let Landing::Placed(state) = landing
            && block.id != 0
            && let Some(host) = &self.lan
        {
            host.hub.send_to_tracking(
                block.id as i32,
                &horizon_protocol::packets::play::Clientbound::AddTransientBlock(
                    horizon_protocol::packets::play::AddTransientBlock {
                        pos,
                        state: state.0 as i32,
                    },
                ),
            );
        }
        if matches!(landing, Landing::Broken)
            && block.drop_item
            && self
                .ui
                .rules
                .bool_at(horizon_client::gamerules::vanilla::ENTITY_DROPS)
        {
            if let Some(id) = sound_event {
                self.level_events.push(LevelEvent::LevelEvent {
                    id,
                    pos: BlockPos::new(pos[0], pos[1], pos[2]),
                    data: 0,
                    source: None,
                });
            }
            // FallingBlockEntity.tick: spawnAtLocation, from where the entity
            // came to rest (on top of a turtle egg, not inside its cell).
            self.drop_falling_block(block.state, block.position);
        }
    }
    fn falling_hurt_player(&mut self, entity: u32, position: Vec3, amount: f32, kind: &str) {
        if self.creative || self.spectator() {
            return;
        }
        if self.player.as_ref().is_none_or(|p| {
            p.health.dead_or_dying() || !falling_block::bounds(position).intersects(p.bounds())
        }) {
            return;
        }
        let Some(id) = self.motion.health.damage.id(kind) else {
            return;
        };
        let attack = horizon_client::health::Attack {
            attacker: entity as i32,
            direct: Some(entity as i32),
            position: Some([position.x, position.y, position.z]),
            ..Default::default()
        };
        let outcome = self
            .hurt_player_equipment(id, amount, None, false, entity as i32, false, &attack, None)
            .unwrap_or_default();
        if outcome.hurt {
            // LivingEntity.hurtServer: ENTITY_HURT_PLAYER, the falling block
            // (`DamageSources.fallingBlock(this)` and the like) being both the
            // causing and the direct entity.
            let block = horizon_client::advancements::EntityView {
                kind: "minecraft:falling_block".into(),
                position: [position.x, position.y, position.z],
                ..Default::default()
            };
            self.trigger_entity_hurt_player(&horizon_client::advancements::DamageView {
                kind: kind.to_owned(),
                direct: Some(block.clone()),
                is_direct: true,
                source: Some(block),
                original: amount,
                actual: outcome.dealt,
                blocked: false,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::game::{Game, native_entity_refs::tests::with_game};
    use horizon_client::{items::ItemStack, motion::PlayerMotion};
    use horizon_core::{equipment_slot::EquipmentSlot, geometry::Vec3};
    use mod_api::{
        ArmorHurtEvent, DamageItemEvent, Flow, Hooks, ItemPart, LevelRandomHost,
        LivingIncomingDamageEvent, Registrar, RegistrationContext, ServerHost,
    };
    use std::sync::{Arc, Mutex};

    const ANVIL: &str = "minecraft:falling_anvil";

    /// What the probe answers and what it heard of a hit.
    #[derive(Default)]
    struct Log {
        refuse_hit: bool,
        cancel_wear: bool,
        new_damage: Option<f32>,
        incoming: u32,
        /// `armor_hurt`: each call's slots and vanilla damage.
        armor: Vec<Vec<(EquipmentSlot, f32)>>,
        /// `damage_item` of the iron helmet: the points, and whether its host names the acting player.
        worn: Vec<(i32, bool)>,
    }
    #[derive(Clone)]
    struct Probe(Arc<Mutex<Log>>);
    #[mod_api::__subscriptions]
    impl Hooks for Probe {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let helmet = r.item_key("minecraft:iron_helmet");
            r.item_part(helmet, self.clone());
        }
        fn living_incoming_damage(
            &self,
            _: &mut dyn LevelRandomHost,
            _: &mut LivingIncomingDamageEvent<'_>,
        ) -> Flow {
            let mut log = self.0.lock().unwrap();
            log.incoming += 1;
            if log.refuse_hit {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
        fn armor_hurt(&self, _: &mut dyn ServerHost, event: &mut ArmorHurtEvent<'_>) -> Flow {
            let mut log = self.0.lock().unwrap();
            let call = event.entries.iter().map(|e| (e.slot, e.damage)).collect();
            log.armor.push(call);
            if let Some(new_damage) = log.new_damage {
                for entry in event.entries.iter_mut() {
                    entry.new_damage = new_damage;
                }
            }
            if log.cancel_wear {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }
    #[mod_api::horizon_part]
    impl ItemPart for Probe {
        fn damage_item(&self, host: &mut dyn ServerHost, event: &mut DamageItemEvent<'_>) -> Flow {
            let acting = host.players().acting_player().is_some();
            self.0.lock().unwrap().worn.push((event.amount, acting));
            Flow::Continue
        }
    }

    /// Runs `test` as a helmeted server player, with the probe's log.
    fn with_player(test: impl FnOnce(&mut Game<'_>, &Mutex<Log>)) {
        let log = Arc::new(Mutex::new(Log::default()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "probe",
            Box::new(Probe(log.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods.clone(), || {
            crate::game::api_values::install_parts(&mods).unwrap();
            with_game(|game| {
                game.lan_acting_uuid = Some(123);
                game.lan_acting_entity = Some(20);
                game.lan_acting_name = Some("Steve".into());
                game.lan_acting_permission = Some(0);
                test(game, &log);
            })
        });
    }
    /// A new player in `helmet`, and a log with no answers: a hit of 8 wears 2 points.
    fn start(game: &mut Game<'_>, log: &Mutex<Log>, helmet: Option<ItemStack>) {
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        game.armor[0] = helmet;
        game.apply_equipment();
        game.refresh_armor_protection();
        *log.lock().unwrap() = Log::default();
    }
    fn hit(game: &mut Game<'_>) {
        game.falling_hurt_player(241, Vec3::ZERO, 8.0, ANVIL);
    }
    fn health(game: &Game<'_>) -> f32 {
        game.player.as_ref().unwrap().health.health()
    }
    fn iron_helmet() -> ItemStack {
        ItemStack::new("minecraft:iron_helmet", 1)
    }

    #[test]
    #[ignore = "daily"]
    fn a_canceled_wear_leaves_the_helmet_and_its_share_of_the_hit() {
        with_player(|game, log| {
            start(game, log, Some(iron_helmet()));
            log.lock().unwrap().cancel_wear = true;
            hit(game);
            assert_eq!(game.armor[0], Some(iron_helmet()));
            {
                let log = log.lock().unwrap();
                assert_eq!(
                    log.armor,
                    [
                        vec![(EquipmentSlot::Head, 2.0)],
                        vec![(EquipmentSlot::Head, 1.5)]
                    ]
                );
                assert!(log.worn.is_empty());
                assert_eq!(log.incoming, 1);
            }
            // 8 less the helmet's quarter is 6; 2 armor points leave 98.4% of it.
            assert!((health(game) - 14.096).abs() < 0.001, "{}", health(game));

            // Both equipment operations already ran; draining events cannot repeat them.
            let events = std::mem::take(&mut game.player.as_mut().unwrap().events);
            game.wear_armor(&events);
            assert_eq!(log.lock().unwrap().armor.len(), 2);
            assert_eq!(game.armor[0], Some(iron_helmet()));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn the_answered_wear_runs_on_the_real_host_before_the_cooldown_refuses() {
        with_player(|game, log| {
            start(game, log, Some(iron_helmet()));
            log.lock().unwrap().new_damage = Some(7.0);
            hit(game);
            assert_eq!(game.armor[0].as_ref().unwrap().damage, 14);
            assert_eq!(log.lock().unwrap().worn, [(7, true), (7, true)]);
            let after_first = health(game);

            // The cooldown takes the same hit whole, after `hurtHelmet` has worn the piece.
            log.lock().unwrap().new_damage = None;
            hit(game);
            assert_eq!(game.armor[0].as_ref().unwrap().damage, 16);
            assert_eq!(health(game), after_first);
            assert_eq!(log.lock().unwrap().incoming, 2);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_helmet_the_hit_breaks_stops_protecting_in_that_hit() {
        with_player(|game, log| {
            let mut helmet = iron_helmet();
            helmet.damage = game.items().get(&helmet.item).unwrap().max_damage - 1;
            helmet.enchant("minecraft:protection", 4);
            start(game, log, Some(helmet));
            let status = &game.player.as_ref().unwrap().status;
            assert_eq!(status.protection(ANVIL), 4.0);
            hit(game);
            assert_eq!(game.armor[0], None);
            // The quarter stays: `hurtServer` takes it whatever the helmet became.
            assert_eq!(health(game), 14.0);
            assert_eq!(log.lock().unwrap().incoming, 1);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn no_helmet_or_a_refused_hit_fires_no_helmet_wear() {
        with_player(|game, log| {
            start(game, log, None);
            hit(game);
            assert!(log.lock().unwrap().armor.is_empty());
            assert_eq!(health(game), 12.0);

            start(game, log, Some(iron_helmet()));
            log.lock().unwrap().refuse_hit = true;
            hit(game);
            assert!(log.lock().unwrap().armor.is_empty());
            assert_eq!(game.armor[0], Some(iron_helmet()));
            assert_eq!(health(game), 20.0);
        });
    }
}

#[cfg(test)]
#[path = "falling_damage/helmet_tests.rs"]
mod helmet_tests;
