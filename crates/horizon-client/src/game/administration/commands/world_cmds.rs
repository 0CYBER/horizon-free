//! `/recipe`, `/damage`, `/rotate`, `/swing`, `/playsound`, `/stopsound`,
//! `/particle`, `/attribute`, `/ride` and `/spreadplayers`.
use super::{Arg, Context, Game, R, Target, err, wrap_degrees};
use crate::game::living_attributes::AttributeWrite;
use horizon_client::{
    mobs::{Damage, DamageKind, MobEvent, NamedDamage, Who, finalize},
    scoreboard_cmd::{Holder, Tracker},
    settings::SOUND_SOURCES,
    text::Text,
};
use horizon_core::nbt::Tag;
use horizon_core::random::RandomSource;
use horizon_protocol::packets::play::{self, Clientbound as C};
use horizon_world::attribute_map::ValueKind;
use horizon_world::damage_source::Catalog as DamageCatalog;

/// Queue bookkeeping for synchronous command damage, preserving prior requests
/// and unrelated events. The existing game policy handles each extracted request.
fn take_new_reinforcements(events: &mut Vec<MobEvent>, queued: usize) -> Vec<(u32, Who)> {
    let mut requests = Vec::new();
    let mut index = 0;
    events.retain(|event| {
        let new = index >= queued;
        index += 1;
        if new && let MobEvent::Reinforcements { mob, target } = event {
            requests.push((*mob, *target));
            false
        } else {
            true
        }
    });
    requests
}

fn num(n: impl std::fmt::Display) -> Text {
    Text::literal(n.to_string())
}
fn holder(text: Text) -> Holder {
    Holder {
        name: String::new(),
        text,
        display: None,
    }
}
/// `ChatFormatting`-free float text as `Component.translatable` prints a `double`.
fn java_double_text(v: f64) -> Text {
    Text::literal(horizon_core::java_number::double_to_string(v))
}

impl Game<'_> {
    // ---- /recipe -----------------------------------------------------------

    /// Non-special recipes, as resolved by `ResourceKeyArgument.getRecipe` and `/recipe *`.
    pub(super) fn learnable_recipes(&self) -> Vec<String> {
        let index = &self.stations().recipe_index;
        index
            .ids()
            .filter(|id| !index.special(id))
            .cloned()
            .collect()
    }
    pub(super) fn cmd_recipe(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let targets = self.entities(ctx, "targets", true)?;
        let give = id == "recipe.give";
        let recipes: Vec<String> = match ctx.get("recipe") {
            Some(Arg::Resource(r)) => {
                if !self.learnable_recipes().contains(r) {
                    return Err(err("recipe.notFound", &[r]));
                }
                vec![r.clone()]
            }
            _ => self.learnable_recipes(),
        };
        let mut tracker = Tracker::default();
        for &t in &targets {
            let h = holder(self.target_name(t));
            // ServerPlayer.awardRecipes / resetRecipes on the target's own
            // ServerRecipeBook: a guest's is swapped in for the call, and the
            // packets go to its client.
            let n = self
                .for_player(t, |g| g.change_recipes(give, &recipes))
                .unwrap_or(0);
            tracker.track(&h, n);
        }
        let verb = if give { "give" } else { "take" };
        let (message, total) = tracker.message(
            false,
            Some(err(&format!("commands.recipe.{verb}.failed"), &[])),
            |h, total| {
                Text::translate(
                    &format!("commands.recipe.{verb}.success.single"),
                    &[num(total), h.text.clone()],
                )
            },
            |count, total| {
                Text::translate(
                    &format!("commands.recipe.{verb}.success.multiple"),
                    &[num(total), num(count)],
                )
            },
        )?;
        self.feedback(message);
        Ok(total)
    }

    /// ServerRecipeBook.addRecipes/removeRecipes for the player in place: how
    /// many recipe book displays changed (a transmute recipe has one per
    /// material count).
    pub(in crate::game) fn change_recipes(&mut self, give: bool, recipes: &[String]) -> i32 {
        if give {
            return self.award_recipes(recipes) as i32;
        }
        let index = &world_stations!(self).recipe_index;
        let removed = self.level.recipe_book.remove_recipes(index, recipes);
        let count = removed.len() as i32;
        if count != 0 {
            self.invalidate_mod_book_placement_knowledge();
        }
        // A network player's client drops them from its book.
        if self.lan_acting.is_some() && !removed.is_empty() {
            self.guest_packets.push(C::RecipeBookRemove(
                horizon_protocol::packets::recipes::RecipeBookRemove { ids: removed },
            ));
        }
        count
    }

    // ---- /damage ----------------------------------------------------------------

    /// The `DamageKind` of a damage type id, for the sources the engine's own
    /// hurt paths take (`DamageCommand`'s, `EntityRef::hurt`'s and a mod
    /// effect's). A type the engine has no kind of its own for keeps its
    /// registry id and tags as a `DamageKind::Named`.
    pub(in crate::game) fn damage_kind(catalog: &DamageCatalog, ty: &str) -> DamageKind {
        match ty.trim_start_matches("minecraft:") {
            "player_attack" => DamageKind::PlayerAttack,
            "mace_smash" => DamageKind::MaceSmash,
            "spear" => DamageKind::Spear,
            "mob_attack" | "mob_attack_no_aggro" => DamageKind::MobAttack,
            "arrow" => DamageKind::Arrow,
            "thrown" | "mob_projectile" => DamageKind::Thrown,
            "fireball" | "unattributed_fireball" => DamageKind::Fireball,
            "wither_skull" => DamageKind::WitherSkull,
            "explosion" | "player_explosion" | "bad_respawn_point" => DamageKind::Explosion,
            "fall" | "ender_pearl" | "fly_into_wall" => DamageKind::Fall,
            "on_fire" => DamageKind::OnFire,
            "in_fire" => DamageKind::InFire,
            "hot_floor" => DamageKind::HotFloor,
            "campfire" => DamageKind::Campfire,
            "lava" => DamageKind::Lava,
            "drown" => DamageKind::Drown,
            "dry_out" => DamageKind::DryOut,
            "magic" | "indirect_magic" | "dragon_breath" => DamageKind::Magic,
            "sting" => DamageKind::Sting,
            "cactus" => DamageKind::Cactus,
            "sweet_berry_bush" => DamageKind::SweetBerryBush,
            "freeze" => DamageKind::Freeze,
            "in_wall" => DamageKind::InWall,
            "thorns" => DamageKind::Thorns,
            "cramming" => DamageKind::Cramming,
            "falling_anvil" => DamageKind::FallingAnvil,
            "falling_block" => DamageKind::FallingBlock,
            "fireworks" => DamageKind::Fireworks,
            "generic_kill" => DamageKind::GenericKill,
            "lightning_bolt" => DamageKind::LightningBolt,
            "out_of_world" => DamageKind::OutOfWorld,
            "outside_border" => DamageKind::OutsideBorder,
            "sonic_boom" => DamageKind::SonicBoom,
            "spit" => DamageKind::Spit,
            "stalagmite" => DamageKind::Stalagmite,
            "starve" => DamageKind::Starve,
            "sulfur_cube_hot" => DamageKind::SulfurCubeHot,
            "wind_charge" => DamageKind::WindCharge,
            "wither" => DamageKind::Wither,
            "generic" => DamageKind::Generic,
            _ => {
                NamedDamage::registered(catalog, ty).map_or(DamageKind::Generic, DamageKind::Named)
            }
        }
    }
    /// `DamageCommand.damage`: apply its source and report an accepted hit.
    pub(super) fn cmd_damage(&mut self, ctx: &Context) -> R<i32> {
        let targets = self.entities(ctx, "target", false)?;
        let Some(target) = targets.first().copied() else {
            return Ok(0);
        };
        let amount = match ctx.get("amount") {
            Some(Arg::Float(a)) => *a,
            _ => 0.0,
        };
        let ty = match ctx.get("damageType") {
            Some(Arg::Resource(t)) => t.clone(),
            _ => "minecraft:generic".into(),
        };
        let by = match ctx.get("entity") {
            Some(Arg::Entity(_)) => self.entities(ctx, "entity", false)?.first().copied(),
            _ => None,
        };
        let cause = match ctx.get("cause") {
            Some(Arg::Entity(_)) => self.entities(ctx, "cause", false)?.first().copied(),
            _ => None,
        };
        let from = match ctx.get("location") {
            Some(Arg::Coords(_)) => self.position_arg(ctx, "location"),
            _ => by.and_then(|t| self.target_position(t)),
        };
        // Destruction can remove the target before its success message is built.
        let discarded_name = matches!(target, Target::Item(_) | Target::Boat(_) | Target::Cart(_))
            .then(|| self.target_name(target));
        let hurt = self.hurt_target(target, amount, &ty, by, cause, from);
        if !hurt {
            return Err(err("commands.damage.invulnerable", &[]));
        }
        let name = discarded_name.unwrap_or_else(|| self.target_name(target));
        self.feedback(Text::translate(
            "commands.damage.success",
            &[
                Text::literal(horizon_core::java_number::float_to_string(amount)),
                name,
            ],
        ));
        Ok(1)
    }

    /// `DamageCommand.damage`'s routing, the engine's one damage entry by
    /// kind: the player and mob hurt paths (the direct and causing entity
    /// and the source position kept, the death message and thorns credited
    /// to the causing one), the item, vehicle, dragon and crystal branches.
    /// False when the kind refuses the damage. `EntityRef::hurt` (#924)
    /// enters here too — there is no second damage pipeline.
    pub(in crate::game) fn hurt_target(
        &mut self,
        target: Target,
        amount: f32,
        ty: &str,
        by: Option<Target>,
        cause: Option<Target>,
        from: Option<[f64; 3]>,
    ) -> bool {
        // Resolve the damage registry at the shared command/API boundary before
        // any target-specific path can mutate state or map it to a DamageKind.
        if self.motion.health.damage.id(ty).is_none() {
            return false;
        }
        let attacker = cause.or(by).and_then(|t| self.network_entity_id(t));
        let attack = self.command_attack(attacker.unwrap_or(0), by);
        self.with_attack(attack, |g| {
            g.hurt_target_from_source(target, amount, ty, by, cause, from)
        })
    }

    /// `DamageCommand.damage`: an explicit direct entity keeps its actual class and weapon.
    fn command_attack(
        &self,
        attacker: i32,
        direct: Option<Target>,
    ) -> horizon_client::health::Attack {
        let mut attack = self.attack_of(attacker);
        attack.direct = Some(
            direct
                .and_then(|target| self.network_entity_id(target))
                .unwrap_or(0),
        );
        match direct {
            Some(Target::Projectile(id)) => {
                if let Some(shot) = self.creatures.projectiles.iter().find(|shot| shot.id == id) {
                    Self::set_projectile_attack(&mut attack, shot);
                }
            }
            Some(Target::Trident(id)) => {
                if let Some(shot) = self.tridents.iter().find(|shot| shot.id == id) {
                    attack.projectile = Some(id as i32);
                    attack.projectile_is_arrow = true;
                    Self::set_projectile_weapon(&mut attack, Some(&shot.stack));
                }
            }
            _ => {}
        }
        attack
    }

    fn hurt_target_from_source(
        &mut self,
        target: Target,
        amount: f32,
        ty: &str,
        by: Option<Target>,
        cause: Option<Target>,
        from: Option<[f64; 3]>,
    ) -> bool {
        match target {
            // The causing mob is the one the hit is credited to and thorns
            // answer; the source position gives the knockback its direction.
            Target::Player | Target::Guest(_) => {
                // `DamageSource(type, by, cause)`: the death message names the
                // causing entity, else the direct one. A mob is noted by
                // `hurt_player_from`, a player here.
                let named = match cause.or(by) {
                    Some(t @ (Target::Player | Target::Guest(_))) => Some(self.target_name(t)),
                    _ => None,
                };
                let attacker = match cause.or(by) {
                    Some(Target::Mob(i)) => Some(Some(i)),
                    _ => Some(None),
                };
                // `DamageSource.isCreativePlayer`: a creative player's hit
                // passes the victim's invulnerability (#916).
                let creative_source = self.creative_source_of(cause.or(by));
                // resolvePlayerResponsibleForDamage: remember the causing player's UUID.
                let by_player = match cause.or(by) {
                    Some(Target::Player) => self.lan_acting,
                    Some(Target::Guest(conn)) => Some(conn),
                    _ => None,
                }
                .and_then(|conn| self.command_guest(conn))
                .map(|g| crate::game::combat::PlayerCredit(g.uuid));
                let now = self.world.game_time;
                let attack = self.current_attack();
                self.for_player(target, |g| {
                    g.with_attack(attack, |g| {
                        if let Some(name) = named {
                            g.note_direct_entity(ty, name);
                        }
                        let hurt = g.hurt_player_from(amount, from, ty, attacker, creative_source);
                        if hurt && let Some(credit) = by_player {
                            g.combat.note_hurt_by_player(credit, now);
                        }
                        hurt
                    })
                })
                .unwrap_or(false)
            }
            Target::Mob(id) => {
                let attacker = cause.or(by).and_then(|t| match t {
                    Target::Player | Target::Guest(_) => Some(Who::Player),
                    Target::Mob(i) => Some(Who::Mob(i)),
                    _ => None,
                });
                let attacker_species = match attacker {
                    Some(Who::Mob(i)) => self.mobs.iter().find(|m| m.id == i).map(|m| m.species()),
                    _ => None,
                };
                let damage = Damage {
                    kind: Self::damage_kind(&self.motion.health.damage, ty),
                    attacker,
                    from,
                    attacker_species,
                };
                // DamageSource.getEntity is the causing player, even when
                // another client issued the command or supplied the direct entity.
                let guest = match cause.or(by) {
                    Some(Target::Guest(conn)) if self.lan_acting != Some(conn) => Some(conn),
                    _ => None,
                };
                if let Some(conn) = guest {
                    let attack = self.current_attack();
                    let hurt = self
                        .as_guest(conn, |g| {
                            g.with_attack(attack, |g| g.command_damage_mob(id, amount, damage))
                        })
                        .unwrap_or(false);
                    if hurt && let Some(host) = self.lan.as_mut() {
                        host.mob_players.insert(id, conn);
                    }
                    hurt
                } else {
                    self.command_damage_mob(id, amount, damage)
                }
            }
            Target::Item(id) => self.hurt_item_entity(id, amount, ty, by, cause.or(by), from),
            Target::Boat(_) | Target::Cart(_) => {
                self.command_damage_vehicle(target, amount, ty, by, cause.or(by), from)
            }
            Target::Dragon(id) => self.command_damage_dragon(
                id,
                amount,
                ty,
                Self::damage_kind(&self.motion.health.damage, ty),
                (
                    by,
                    cause.or(by),
                    from.map(|[x, y, z]| horizon_core::geometry::Vec3::new(x, y, z)),
                ),
            ),
            Target::Crystal(id) => self.command_damage_crystal(
                id,
                ty,
                Self::damage_kind(&self.motion.health.damage, ty),
                by,
                cause.or(by),
                from,
            ),
            Target::Stand(id) => self.command_damage_stand(id, ty, cause.or(by)),
            _ => false,
        }
    }

    /// `VehicleEntity.hurtServer`: report acceptance even when the vehicle survives.
    fn command_damage_vehicle(
        &mut self,
        target: Target,
        amount: f32,
        damage_type: &str,
        direct: Option<Target>,
        causing: Option<Target>,
        position: Option<[f64; 3]>,
    ) -> bool {
        let registry = &self.motion.health.damage;
        let Some(kind) = registry.id(damage_type) else {
            return false;
        };
        let creative = self.creative_source_of(causing);
        let id = match target {
            Target::Boat(id) | Target::Cart(id) => id,
            _ => return false,
        };
        let vanilla = self.saved_refuses_damage(id, damage_type, creative);
        let ignites = registry.in_tag(kind, "minecraft:is_fire")
            || registry.in_tag(kind, "minecraft:is_explosion");
        let entity_kind = match target {
            Target::Boat(_) => self
                .boats
                .iter()
                .find(|b| b.id == id)
                .map(|b| b.entity_type()),
            Target::Cart(_) => self
                .carts
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.kind.id().to_owned()),
            _ => None,
        };
        let Some(entity_kind) = entity_kind else {
            return false;
        };
        let entity_id = |target| match target {
            Some(Target::Player) => self.own_entity_id(),
            Some(Target::Guest(conn)) => self.command_guest(conn).map_or(0, |g| g.entity_id),
            Some(target) => {
                crate::game::native_entities::target_id(target).map_or(0, |id| id as i32)
            }
            None => 0,
        };
        let source = horizon_client::health::HurtSource {
            damage_type,
            causing: entity_id(causing),
            direct: entity_id(direct),
            position,
        };
        if crate::game::entity::damage_level_of!(self, scoped).invulnerability_check(
            horizon_client::health::Hurting::Entity(id as i32, &entity_kind, source),
            vanilla,
        ) {
            return false;
        }
        match target {
            Target::Boat(_) => self
                .boats
                .iter()
                .position(|b| b.id == id)
                .is_some_and(|i| self.hurt_boat_result(i, amount, creative).is_some()),
            Target::Cart(_) => self
                .carts
                .iter()
                .position(|c| c.id == id)
                .is_some_and(|i| self.hurt_cart_result(i, amount, creative, ignites)),
            _ => false,
        }
    }

    /// `DamageCommand.damage`: retain the accepted hit's player context and run
    /// `Zombie.hurtServer`'s reinforcement requests before the command returns.
    /// Health-changing hits also raise their damage vibration before return.
    fn command_damage_mob(&mut self, id: u32, amount: f32, damage: Damage) -> bool {
        let attack = self.combat.attack();
        let mut reinforcements = Vec::new();
        let mut health_changed = false;
        let hurt = self.mobs.iter_mut().find(|m| m.id == id).is_some_and(|m| {
            let queued = m.outbox.len();
            let old_health = m.health;
            let hurt = m.damage(
                crate::game::entity::damage_level_of!(self, attack),
                amount,
                damage,
            );
            health_changed = hurt && m.health != old_health;
            if hurt {
                reinforcements = take_new_reinforcements(&mut m.outbox, queued);
            }
            hurt
        });
        if hurt {
            self.resolve_player_responsible(id, damage);
            if damage.attacker == Some(Who::Player) {
                self.remember_mob_player_credit(id);
                if let Some(host) = self.lan.as_mut() {
                    match self.lan_acting {
                        Some(conn) => {
                            host.mob_players.insert(id, conn);
                        }
                        None => {
                            host.mob_players.remove(&id);
                        }
                    }
                }
            }
            if health_changed {
                self.mob_damage_event(id);
            }
        }
        for (mob, target) in reinforcements {
            self.call_reinforcements(mob, target);
        }
        // Enderman.hurtServer's teleport, refused hit or not, before the command returns.
        if let Err(e) = self.settle_hurt_teleports() {
            log::warn!("Horizon: /damage: enderman teleport: {e}");
        }
        hurt
    }

    // ---- /rotate and /swing -----------------------------------------------------------

    pub(super) fn cmd_rotate(&mut self, ctx: &Context) -> R<i32> {
        let targets = super::without_knots(self.entities(ctx, "target", false)?)?;
        let Some(t) = targets.first().copied() else {
            return Ok(0);
        };
        let Some(pos) = self.target_position(t) else {
            return Ok(0);
        };
        let mut look_at_target = None;
        let rot = if let Some(Arg::Coords(c)) = ctx.get("rotation") {
            // RotateCommand.rotate: relative parts are offsets from the command
            // source's rotation (WorldCoordinates.getRotation); the entity's own
            // rotation cancels out in Entity.forceSetRotation.
            c.rotation(self.source_rotation())
        } else {
            let mut faced_entity = None;
            let to = if ctx.has("entity") {
                let faced = self.entities(ctx, "facingEntity", false)?;
                let Some(f) = faced.first().copied() else {
                    return Ok(0);
                };
                let mut p = self.target_position(f).unwrap_or(pos);
                let eyes = matches!(ctx.get("facingAnchor"), Some(Arg::Anchor(true)));
                if eyes {
                    p[1] += self.target_eye_height(f);
                }
                faced_entity = Some((f, eyes));
                p
            } else {
                self.position_arg(ctx, "facingLocation").unwrap_or(pos)
            };
            // LookAt.perform: Entity.lookAt from the source's anchor (the
            // feet, unless `anchored eyes`).
            let anchored_eyes = self.command_source.is_some_and(|s| s.eyes);
            let eye = if anchored_eyes {
                self.target_eye_height(t)
            } else {
                0.0
            };
            let from = [pos[0], pos[1] + eye, pos[2]];
            look_at_target = Some((anchored_eyes, to, faced_entity));
            look_at(from, to)
        };
        // RotateCommand: Entity.forceSetRotation, not a teleport.
        self.set_rotation(t, rot);
        if let Some((eyes, to, entity)) = look_at_target {
            self.send_player_look_at(t, eyes, to, entity);
        }
        if t == Target::Player {
            self.pitch = rot[1].clamp(-90.0, 90.0);
        }
        let name = self.target_name(t);
        self.feedback(Text::translate("commands.rotate.success", &[name]));
        Ok(1)
    }
    pub(super) fn cmd_swing(&mut self, ctx: &Context) -> R<i32> {
        let targets = if ctx.get("targets").is_some() {
            self.entities(ctx, "targets", false)?
        } else {
            vec![
                self.command_source
                    .and_then(|s| s.entity)
                    .unwrap_or(Target::Player),
            ]
        };
        let mut tracker = Tracker::default();
        for t in targets {
            match t {
                Target::Player => self.swing_arm(),
                Target::Mob(id) => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                        m.ai.swinging = true;
                        m.ai.swing_time = -1;
                    }
                }
                // An armor stand is a LivingEntity: it swings (invisibly).
                Target::Stand(_) => {}
                _ => continue,
            }
            let h = holder(self.target_name(t));
            tracker.mark(&h);
        }
        let (message, total) = tracker.message(
            false,
            Some(err("commands.swing.failed.notliving", &[])),
            |h, _| {
                Text::translate(
                    "commands.swing.success.single",
                    std::slice::from_ref(&h.text),
                )
            },
            |c, _| Text::translate("commands.swing.success.multiple", &[num(c)]),
        )?;
        self.feedback(message);
        Ok(total)
    }

    // ---- sounds -----------------------------------------------------------------------

    pub(super) fn cmd_playsound(&mut self, ctx: &Context) -> R<i32> {
        let Some(Arg::Resource(sound)) = ctx.get("sound").cloned() else {
            return Ok(0);
        };
        let source = ctx
            .literals
            .iter()
            .find_map(|l| SOUND_SOURCES.iter().position(|s| s == l))
            .unwrap_or(0);
        let targets = if ctx.get("targets").is_some() {
            self.entities(ctx, "targets", true)?
        } else {
            vec![Target::Player]
        };
        let pos = self
            .position_arg(ctx, "pos")
            .unwrap_or_else(|| self.source_position());
        let float = |name: &str, default: f32| match ctx.get(name) {
            Some(Arg::Float(v)) => *v,
            _ => default,
        };
        let (volume, pitch, min_volume) = (
            float("volume", 1.0),
            float("pitch", 1.0),
            float("minVolume", 0.0),
        );
        let range = if volume > 1.0 { 16.0 * volume } else { 16.0 };
        // PlaySoundCommand: one seed for every player.
        let seed = self.world.random.next_long();
        let here = self.dim.id();
        let mut tracker = Tracker::default();
        for t in targets {
            if !matches!(t, Target::Player | Target::Guest(_)) {
                continue;
            }
            // Only players in the source's level hear it.
            if let Target::Guest(conn) = t
                && self.command_guest(conn).is_none_or(|g| g.dimension != here)
            {
                continue;
            }
            let Some(p) = self.target_position(t) else {
                continue;
            };
            let Some((at, v)) = playsound_placement(pos, p, range, volume, min_volume) else {
                continue;
            };
            self.play_sound_to(t, &sound, at, v, pitch, source, seed);
            tracker.mark(&holder(self.target_name(t)));
        }
        let name = Text::literal(sound.clone());
        let (message, total) = tracker.message(
            true,
            Some(err("commands.playsound.failed", &[])),
            |h, _| {
                Text::translate(
                    "commands.playsound.success.single",
                    &[name.clone(), h.text.clone()],
                )
            },
            |c, _| {
                Text::translate(
                    "commands.playsound.success.multiple",
                    &[name.clone(), num(c)],
                )
            },
        )?;
        self.feedback(message);
        Ok(total)
    }
    /// `ClientboundSoundPacket` to one player: a guest's connection is sent
    /// it; the window's own player plays it. `/playsound` and a native mod's
    /// `PlayerRef::play_sound` both send through here.
    pub(in crate::game) fn play_sound_to(
        &mut self,
        t: Target,
        sound: &str,
        at: [f64; 3],
        volume: f32,
        pitch: f32,
        source: usize,
        seed: i64,
    ) {
        let guest = match t {
            Target::Guest(conn) => Some(conn),
            _ => self.lan_acting,
        };
        match guest.and_then(|conn| Some((conn, self.lan.as_ref()?))) {
            Some((conn, host)) => {
                let packet = host
                    .hub
                    .sound_packet(sound, at, volume, pitch, source as i32, seed);
                if t == Target::Player {
                    self.guest_packets.push(packet);
                } else {
                    host.hub.send_to(conn, &packet);
                }
            }
            None => self.audio.play_on(source, sound, Some(at), volume, pitch),
        }
    }
    /// `StopSoundCommand`: each target's connection gets the packet; the
    /// window's own player stops its sounds. `/stopsound` and a native mod's
    /// `PlayerRef::stop_sound` both send through here.
    pub(in crate::game) fn stop_sound_for(
        &mut self,
        targets: &[Target],
        sound: Option<&str>,
        source: Option<usize>,
    ) {
        let packet = C::StopSound(play::StopSound {
            source: source.map(|s| s as i32),
            sound: sound.map(str::to_owned),
        });
        for &t in targets {
            match t {
                Target::Player if self.lan_acting.is_some() => {
                    self.guest_packets.push(packet.clone());
                }
                Target::Player => self.audio.stop_sounds(sound, source),
                Target::Guest(conn) => {
                    if let Some(host) = self.lan.as_ref() {
                        host.hub.send_to(conn, &packet);
                    }
                }
                _ => {}
            }
        }
    }
    pub(super) fn cmd_stopsound(&mut self, ctx: &Context) -> R<i32> {
        let targets = self.entities(ctx, "targets", true)?;
        let source = ctx
            .literals
            .iter()
            .find_map(|l| SOUND_SOURCES.iter().position(|s| s == l));
        let sound = match ctx.get("sound") {
            Some(Arg::Resource(s)) => Some(s.clone()),
            _ => None,
        };
        self.stop_sound_for(&targets, sound.as_deref(), source);
        let key = match (source, &sound) {
            (Some(_), Some(_)) => "commands.stopsound.success.source.sound",
            (Some(_), None) => "commands.stopsound.success.source.any",
            (None, Some(_)) => "commands.stopsound.success.sourceless.sound",
            (None, None) => "commands.stopsound.success.sourceless.any",
        };
        let mut args = Vec::new();
        if let Some(s) = &sound {
            args.push(Text::literal(s.clone()));
        }
        if let Some(i) = source {
            args.push(Text::literal(SOUND_SOURCES[i]));
        }
        self.feedback(Text::translate(key, &args));
        Ok(targets.len() as i32)
    }

    // ---- /particle ----------------------------------------------------------------------

    pub(super) fn cmd_particle(&mut self, ctx: &Context) -> R<i32> {
        use horizon_client::particles::{ParticleOptions, ParticleType};
        let Some(Arg::Str(spec)) = ctx.get("name").cloned() else {
            return Ok(0);
        };
        // ParticleArgument: `id` then optional SNBT options (`dust{color:[1,0,0],scale:2}`).
        let (name, options_text) = match spec.find('{') {
            Some(i) => (&spec[..i], &spec[i..]),
            None => (spec.as_str(), ""),
        };
        let ty = ParticleType::from_name(name).ok_or_else(|| err("particle.notFound", &[&name]))?;
        let opts = match horizon_client::command::snbt::parse_fully(options_text) {
            Ok(Tag::Compound(c)) if !options_text.is_empty() => Some(c),
            _ => None,
        };
        let field = |key: &str| opts.as_ref().and_then(|c| c.get(&key.into()).cloned());
        let number = |t: &Tag| -> Option<f32> {
            Some(match t {
                Tag::Byte(v) => f32::from(*v),
                Tag::Short(v) => f32::from(*v),
                Tag::Int(v) => *v as f32,
                Tag::Long(v) => *v as f32,
                Tag::Float(v) => f32::from_bits(*v),
                Tag::Double(v) => f64::from_bits(*v) as f32,
                _ => return None,
            })
        };
        let string = |t: &Tag| match t {
            Tag::String(s) => Some(s.to_string_lossy()),
            _ => None,
        };
        // ExtraCodecs.RGB_COLOR_CODEC: an int or a list of three floats.
        let rgb = match field("color") {
            Some(Tag::List(l)) if l.len() >= 3 => [
                number(&l[0]).unwrap_or(1.0),
                number(&l[1]).unwrap_or(0.0),
                number(&l[2]).unwrap_or(0.0),
            ],
            Some(Tag::Int(v)) => [
                ((v >> 16) & 255) as f32 / 255.0,
                ((v >> 8) & 255) as f32 / 255.0,
                (v & 255) as f32 / 255.0,
            ],
            _ => [1.0, 0.0, 0.0],
        };
        // The block state of a block particle, for its packet.
        let mut block_state = None;
        let options = match ty {
            ParticleType::Dust => ParticleOptions::dust(
                rgb,
                field("scale")
                    .as_ref()
                    .and_then(number)
                    .unwrap_or(1.0)
                    .clamp(0.01, 4.0),
            ),
            // ItemParticleOption: an item id, or a stack {id, ...}.
            ParticleType::Item => {
                let id = match field("item") {
                    Some(Tag::String(s)) => s.to_string_lossy(),
                    Some(Tag::Compound(c)) => {
                        c.get(&"id".into()).and_then(string).unwrap_or_default()
                    }
                    _ => String::new(),
                };
                let id = if id.contains(':') {
                    id
                } else {
                    format!("minecraft:{id}")
                };
                self.item_particle(&horizon_client::items::ItemStack::new(&id, 1))
            }
            ParticleType::ItemSlime | ParticleType::ItemSnowball | ParticleType::ItemCobweb => {
                self.ensure_item_particle_sprites();
                ParticleOptions::simple(ty)
            }
            ParticleType::Block
            | ParticleType::FallingDust
            | ParticleType::DustPillar
            | ParticleType::BlockMarker => {
                // BlockParticleOption: a block id, or {Name, Properties}.
                let text = match field("block_state") {
                    Some(Tag::String(s)) => s.to_string_lossy(),
                    Some(Tag::Compound(c)) => {
                        let name = c.get(&"Name".into()).and_then(string).unwrap_or_default();
                        let props: Vec<String> = match c.get(&"Properties".into()) {
                            Some(Tag::Compound(p)) => p
                                .iter()
                                .filter_map(|(k, v)| {
                                    string(v).map(|v| format!("{}={v}", k.to_string_lossy()))
                                })
                                .collect(),
                            _ => Vec::new(),
                        };
                        if props.is_empty() {
                            name
                        } else {
                            format!("{name}[{}]", props.join(","))
                        }
                    }
                    _ => String::new(),
                };
                let mut reader = horizon_client::command::Reader::new(&text);
                let block = horizon_client::command::args::ArgType::BlockState
                    .parse(&mut reader, &self.ui.registries);
                match block {
                    Ok(Arg::Block(b)) => {
                        let state = self.block_state(&b)?;
                        block_state = Some(state);
                        let at = self
                            .position_arg(ctx, "pos")
                            .unwrap_or_else(|| self.source_position())
                            .map(|c| c.floor() as i32);
                        let (sprite, tint) = self
                            .particle_sprite(state, at)
                            .unwrap_or_else(|| ("block/stone".into(), 0xFFFFFF));
                        ParticleOptions::block(ty, sprite, tint)
                    }
                    _ => ParticleOptions::simple(ty),
                }
            }
            _ => ParticleOptions::simple(ty),
        };
        let pos = self
            .position_arg(ctx, "pos")
            .unwrap_or_else(|| self.source_position());
        // Vec3Argument.getVec3: the spread as given (relative parts from the
        // source), not a position to measure from the source.
        let delta = self.position_arg(ctx, "delta").unwrap_or([0.0; 3]);
        let speed = match ctx.get("speed") {
            Some(Arg::Float(v)) => *v,
            _ => 0.0,
        };
        let count = match ctx.get("count") {
            Some(Arg::Int(c)) => *c,
            _ => 0,
        };
        // ParticleCommand.sendParticles: the viewers it reached, each within
        // 32 blocks (512 with `force`) of the particle's block centre.
        // Without viewers, every player (PlayerList.getPlayers).
        let viewers = if ctx.get("viewers").is_some() {
            self.entities(ctx, "viewers", true)?
        } else {
            let acting = self.lan_acting;
            let guests = self.lan.iter().flat_map(|h| h.hub.online_players());
            self.player
                .as_ref()
                .filter(|_| !self.lan_acting_fake)
                .map(|_| Target::Player)
                .into_iter()
                .chain(
                    guests
                        .filter(|g| Some(g.conn) != acting)
                        .map(|g| Target::Guest(g.conn)),
                )
                .collect()
        };
        let force = ctx.has("force");
        let limit = if force { 512.0 } else { 32.0 };
        let reached: Vec<Target> = viewers
            .into_iter()
            .filter(|&t| {
                self.target_position(t).is_some_and(|p| {
                    let c = p.map(|v| v.floor() + 0.5);
                    (c[0] - pos[0]).powi(2) + (c[1] - pos[1]).powi(2) + (c[2] - pos[2]).powi(2)
                        < limit * limit
                })
            })
            .collect();
        if reached.is_empty() {
            return Err(err("commands.particle.failed", &[]));
        }
        // ServerLevel.sendParticles: each viewer a server serves is sent the
        // particles (the player in place too when it is a guest); the
        // window's own player draws them.
        let packet = self
            .network_particle(&options, block_state)
            .map(|particle| {
                C::LevelParticles(play::LevelParticles {
                    particle,
                    override_limiter: force,
                    always_show: false,
                    pos,
                    dist: delta.map(|d| d as f32),
                    max_speed: [speed; 3],
                    count: count.max(0),
                    randomization: 0,
                })
            });
        let mut local = false;
        for &t in &reached {
            match (t, &packet) {
                (Target::Player, Some(p)) if self.lan_acting.is_some() => {
                    self.guest_packets.push(p.clone());
                }
                (Target::Player, _) => local = true,
                (Target::Guest(conn), Some(p)) => {
                    if let Some(host) = self.lan.as_ref() {
                        host.hub.send_to(conn, p);
                    }
                }
                _ => {}
            }
        }
        let reached = reached.len() as i32;
        if !local {
        } else if count <= 0 {
            // handleParticleEvent, count 0: one particle, the speed times
            // the delta as velocity (both sent as floats, multiplied as floats).
            self.particles
                .create(&options, pos, delta.map(|d| f64::from(speed * d as f32)));
        } else {
            for _ in 0..count {
                let r = self.particles.random();
                let p = [
                    pos[0] + r.next_gaussian() * delta[0],
                    pos[1] + r.next_gaussian() * delta[1],
                    pos[2] + r.next_gaussian() * delta[2],
                ];
                let v = [
                    r.next_gaussian() * f64::from(speed),
                    r.next_gaussian() * f64::from(speed),
                    r.next_gaussian() * f64::from(speed),
                ];
                self.particles.create(&options, p, v);
            }
        }
        self.feedback(Text::translate(
            "commands.particle.success",
            &[Text::literal(name.to_owned())],
        ));
        Ok(reached)
    }

    // ---- /attribute ---------------------------------------------------------------------

    /// The attribute id without `minecraft:`, as the attribute map names it.
    fn attribute_description(attribute: &str) -> Text {
        let short = attribute.trim_start_matches("minecraft:");
        Text::translate(&format!("attribute.name.{short}"), &[])
    }
    /// Read arms follow `AttributeCommand.getAttributeValue` and
    /// `AttributeCommand.getAttributeModifier`: effective values include modeled
    /// transient modifiers; base reads still use the unmodified base.
    pub(super) fn cmd_attribute(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let targets = self.entities(ctx, "target", false)?;
        let Some(target) = targets.first().copied() else {
            return Ok(0);
        };
        let Some(Arg::Resource(attribute)) = ctx.get("attribute").cloned() else {
            return Ok(0);
        };
        // An unbounded selector reaches every loaded level, so the target may
        // be in another one than the command's (EntitySelector.findEntities,
        // AttributeCommand acting on the entity itself).
        let reply = if matches!(target, Target::Guest(_)) {
            self.for_player(target, |g| {
                g.attribute_in_level(id, ctx, Target::Player, &attribute)
            })
        } else {
            self.in_target_level(target, |g| {
                g.attribute_in_level(id, ctx, target, &attribute)
            })
        }
        .unwrap_or(Ok(None))?;
        // Command success belongs to the issuer, after the actual target's
        // body/level is restored. Internal system messages still use for_player.
        let Some((value, message)) = reply else {
            return Ok(0);
        };
        self.feedback(message);
        Ok(value)
    }
    /// `/attribute` on a target, with the target's own level in place.
    fn attribute_in_level(
        &mut self,
        id: &str,
        ctx: &Context,
        target: Target,
        attribute: &str,
    ) -> R<Option<(i32, Text)>> {
        let name = self.target_name(target);
        let description = Self::attribute_description(attribute);
        let short = attribute.trim_start_matches("minecraft:").to_owned();
        let no_attribute = |name: &Text, d: &Text| {
            text_error(
                "commands.attribute.failed.no_attribute",
                &[name.clone(), d.clone()],
            )
        };
        let scale = match ctx.get("scale") {
            Some(Arg::Double(s)) => *s,
            _ => 1.0,
        };
        let value_arg = match ctx.get("value") {
            Some(Arg::Double(v)) => *v,
            _ => 0.0,
        };
        let modifier_id = match ctx.get("id") {
            Some(Arg::Resource(m)) => m.clone(),
            _ => String::new(),
        };
        match target {
            Target::Player => {
                let Some(player) = self.player.as_mut() else {
                    return Ok(None);
                };
                let Some(key) = player.attributes.key(attribute) else {
                    return Err(no_attribute(&name, &description));
                };
                let unavailable = |_| no_attribute(&name, &description);
                match id {
                    "attribute.get" | "attribute.base.get" => {
                        let kind = if id == "attribute.get" {
                            ValueKind::Computed
                        } else {
                            ValueKind::Base
                        };
                        let v = player
                            .attributes
                            .value(Some(key), kind)
                            .map_err(unavailable)?;
                        let k = if id == "attribute.get" {
                            "commands.attribute.value.get.success"
                        } else {
                            "commands.attribute.base_value.get.success"
                        };
                        Ok(Some((
                            (v * scale) as i32,
                            Text::translate(k, &[description, name, java_double_text(v)]),
                        )))
                    }
                    "attribute.base.set" => {
                        self.write_attribute(target, attribute, AttributeWrite::Base(value_arg))
                            .ok_or_else(|| no_attribute(&name, &description))?;
                        // onAttributeUpdated: a lowered max_health clamps health.
                        if attribute == "minecraft:max_health"
                            && let Some(player) = self.player.as_mut()
                        {
                            let _ = player.clamp_health_to_max();
                        }
                        Ok(Some((
                            1,
                            Text::translate(
                                "commands.attribute.base_value.set.success",
                                &[description, name, java_double_text(value_arg)],
                            ),
                        )))
                    }
                    "attribute.base.reset" => {
                        self.write_attribute(target, attribute, AttributeWrite::ResetBase)
                            .ok_or_else(|| no_attribute(&name, &description))?;
                        let v = self
                            .player
                            .as_mut()
                            .and_then(|p| {
                                let v = p.attributes.value(Some(key), ValueKind::Base).ok();
                                if attribute == "minecraft:max_health" {
                                    let _ = p.clamp_health_to_max();
                                }
                                v
                            })
                            .unwrap_or(0.0);
                        Ok(Some((
                            1,
                            Text::translate(
                                "commands.attribute.base_value.reset.success",
                                &[description, name, java_double_text(v)],
                            ),
                        )))
                    }
                    "attribute.modifier.add" => {
                        let added = AttributeWrite::add(
                            &modifier_id,
                            value_arg,
                            command_operation(ctx),
                            true,
                            false,
                        )
                        .and_then(|write| self.write_attribute(target, attribute, write))
                        .ok_or_else(|| no_attribute(&name, &description))?;
                        if !added {
                            return Err(text_error(
                                "commands.attribute.failed.modifier_already_present",
                                &[
                                    Text::literal(modifier_id.clone()),
                                    description.clone(),
                                    name.clone(),
                                ],
                            ));
                        }
                        Ok(Some((
                            1,
                            Text::translate(
                                "commands.attribute.modifier.add.success",
                                &[Text::literal(modifier_id), description, name],
                            ),
                        )))
                    }
                    "attribute.modifier.remove" => {
                        let removed = self.write_attribute(
                            target,
                            attribute,
                            AttributeWrite::Remove(modifier_id.clone()),
                        );
                        if removed != Some(true) {
                            return Err(text_error(
                                "commands.attribute.failed.no_modifier",
                                &[
                                    description.clone(),
                                    name.clone(),
                                    Text::literal(modifier_id.clone()),
                                ],
                            ));
                        }
                        Ok(Some((
                            1,
                            Text::translate(
                                "commands.attribute.modifier.remove.success",
                                &[Text::literal(modifier_id), description, name],
                            ),
                        )))
                    }
                    _ => {
                        let Some(handle) = player.attributes.get_instance(Some(key)).ok().flatten()
                        else {
                            return Err(no_attribute(&name, &description));
                        };
                        let found = player
                            .attributes
                            .edit(&handle, |value, _| {
                                Ok(value
                                    .modifiers()
                                    .iter()
                                    .find(|m| m.id() == modifier_id)
                                    .map(|m| m.amount()))
                            })
                            .ok()
                            .flatten();
                        let Some(v) = found else {
                            return Err(text_error(
                                "commands.attribute.failed.no_modifier",
                                &[
                                    description.clone(),
                                    name.clone(),
                                    Text::literal(modifier_id.clone()),
                                ],
                            ));
                        };
                        Ok(Some((
                            (v * scale) as i32,
                            Text::translate(
                                "commands.attribute.modifier.value.get.success",
                                &[
                                    Text::literal(modifier_id),
                                    description,
                                    name,
                                    java_double_text(v),
                                ],
                            ),
                        )))
                    }
                }
            }
            Target::Mob(_) | Target::Stand(_)
                if self.sulfur_archetype_attribute(target, attribute)
                    || !(matches!(target, Target::Mob(_))
                        && finalize::command_attr(short.as_str()).is_some()) =>
            {
                let op = ctx
                    .literals
                    .last()
                    .cloned()
                    .unwrap_or_else(|| "add_value".into());
                self.generic_attribute(
                    target,
                    id,
                    attribute,
                    (&name, &description),
                    (value_arg, scale, &modifier_id, &op),
                )
                .map(Some)
            }
            Target::Mob(mob_id) => {
                let Some(m) = self.mobs.iter().find(|m| m.id == mob_id) else {
                    return Ok(None);
                };
                // The attributes the mob's gameplay reads live in `finalize`;
                // the command edits that store, never a copy of it.
                let Some(attr) = finalize::command_attr(short.as_str())
                    .filter(|a| finalize::default_value(m, *a).is_some())
                else {
                    return Err(no_attribute(&name, &description));
                };
                let current = if id == "attribute.base.get" {
                    finalize::base_value(m, attr)
                } else {
                    finalize::value(m, attr)
                };
                let no_modifier = text_error(
                    "commands.attribute.failed.no_modifier",
                    &[
                        description.clone(),
                        name.clone(),
                        Text::literal(modifier_id.clone()),
                    ],
                );
                match id {
                    "attribute.get" | "attribute.base.get" => {
                        let k = if id == "attribute.get" {
                            "commands.attribute.value.get.success"
                        } else {
                            "commands.attribute.base_value.get.success"
                        };
                        Ok(Some((
                            (current * scale) as i32,
                            Text::translate(k, &[description, name, java_double_text(current)]),
                        )))
                    }
                    "attribute.base.set" => {
                        // AttributeInstance keeps the raw base and clamps only the
                        // effective value.
                        self.write_attribute(target, attribute, AttributeWrite::Base(value_arg));
                        Ok(Some((
                            1,
                            Text::translate(
                                "commands.attribute.base_value.set.success",
                                &[description, name, java_double_text(value_arg)],
                            ),
                        )))
                    }
                    "attribute.base.reset" => {
                        self.write_attribute(target, attribute, AttributeWrite::ResetBase);
                        let v = self
                            .mobs
                            .iter()
                            .find(|m| m.id == mob_id)
                            .map_or(0.0, |m| finalize::base_value(m, attr));
                        Ok(Some((
                            1,
                            Text::translate(
                                "commands.attribute.base_value.reset.success",
                                &[description, name, java_double_text(v)],
                            ),
                        )))
                    }
                    "attribute.modifier.add" => {
                        let added = AttributeWrite::add(
                            &modifier_id,
                            value_arg,
                            command_operation(ctx),
                            true,
                            false,
                        )
                        .and_then(|write| self.write_attribute(target, attribute, write));
                        if added != Some(true) {
                            return Err(text_error(
                                "commands.attribute.failed.modifier_already_present",
                                &[Text::literal(modifier_id.clone()), description, name],
                            ));
                        }
                        Ok(Some((
                            1,
                            Text::translate(
                                "commands.attribute.modifier.add.success",
                                &[Text::literal(modifier_id.clone()), description, name],
                            ),
                        )))
                    }
                    "attribute.modifier.remove" => {
                        let removed = self.write_attribute(
                            target,
                            attribute,
                            AttributeWrite::Remove(modifier_id.clone()),
                        );
                        if removed != Some(true) {
                            return Err(no_modifier);
                        }
                        Ok(Some((
                            1,
                            Text::translate(
                                "commands.attribute.modifier.remove.success",
                                &[Text::literal(modifier_id.clone()), description, name],
                            ),
                        )))
                    }
                    _ => {
                        // Permanent entries stay authoritative. The modeled baby
                        // modifier is transient and absent from that saved store.
                        let amount =
                            finalize::modifier_amount(m, attr, &modifier_id).or_else(|| {
                                finalize::transient(m, attr)
                                    .filter(|_| modifier_id == "minecraft:baby")
                                    .map(|(amount, _)| amount)
                            });
                        let Some(v) = amount else {
                            return Err(no_modifier);
                        };
                        Ok(Some((
                            (v * scale) as i32,
                            Text::translate(
                                "commands.attribute.modifier.value.get.success",
                                &[
                                    Text::literal(modifier_id.clone()),
                                    description,
                                    name,
                                    java_double_text(v),
                                ],
                            ),
                        )))
                    }
                }
            }
            _ => Err(text_error(
                "commands.attribute.failed.entity",
                std::slice::from_ref(&name),
            )),
        }
    }

    /// The attribute map of a living entity whose attributes Horizon does
    /// not model, created from its type's defaults on first use.
    pub(in crate::game) fn generic_attributes(
        &mut self,
        t: Target,
        attribute: &str,
    ) -> Option<&mut horizon_world::attributes::entity::EntityAttributes> {
        if let Target::Mob(id) = t
            && (matches!(
                attribute,
                "minecraft:armor" | "minecraft:armor_toughness" | "minecraft:max_absorption"
            ) || self.sulfur_archetype_attribute(t, attribute))
        {
            let mob = self.mobs.iter_mut().find(|m| m.id == id)?;
            if mob.extra.armor_attributes.is_none() {
                mob.extra.armor_attributes =
                    self.motion.attributes.create(&mob.kind.id).map(Box::new);
            }
            return mob.extra.armor_attributes.as_deref_mut();
        }
        if !self.cmds.attributes.contains_key(&t) {
            let ty = self.target_type(t);
            let map = self.motion.attributes.create(&ty)?;
            self.cmds.attributes.insert(t, map);
        }
        self.cmds.attributes.get_mut(&t)
    }
    /// A sulfur cube's body archetype attributes live on its own map, beside
    /// the archetype's transient modifiers.
    pub(in crate::game) fn sulfur_archetype_attribute(&self, t: Target, attribute: &str) -> bool {
        let Target::Mob(id) = t else {
            return false;
        };
        horizon_client::mobs::sulfur_cube::ARCHETYPE_ATTRIBUTES.contains(&attribute)
            && self
                .mobs
                .iter()
                .any(|m| m.id == id && m.species() == horizon_client::mobs::Species::SulfurCube)
    }
    /// AttributeCommand on a living entity through its generic attribute map.
    fn generic_attribute(
        &mut self,
        target: Target,
        id: &str,
        attribute: &str,
        (name, description): (&Text, &Text),
        (value_arg, scale, modifier_id, op): (f64, f64, &str, &str),
    ) -> R<(i32, Text)> {
        let no_attribute = || {
            text_error(
                "commands.attribute.failed.no_attribute",
                &[name.clone(), description.clone()],
            )
        };
        let no_modifier = || {
            text_error(
                "commands.attribute.failed.no_modifier",
                &[
                    description.clone(),
                    name.clone(),
                    Text::literal(modifier_id),
                ],
            )
        };
        let catalog = self.motion.attributes.clone();
        let key = catalog.id(attribute).ok_or_else(no_attribute)?;
        // A mob's max_absorption counts its Absorption effect's modifier.
        let absorption_bonus = match target {
            Target::Mob(mob) if attribute == "minecraft:max_absorption" => self
                .mobs
                .iter()
                .find(|m| m.id == mob)
                .map(|m| horizon_client::mobs::armor::absorption_bonus(&m.extra.effects)),
            _ => None,
        };
        let map = self
            .generic_attributes(target, attribute)
            .ok_or_else(no_attribute)?;
        if !map.has(key) {
            return Err(no_attribute());
        }
        let (message, args, result) = match id {
            "attribute.get" | "attribute.base.get" => {
                let v = if id == "attribute.get"
                    && let Some(bonus) = absorption_bonus
                {
                    horizon_client::mobs::armor::max_absorption_of(map, 0.0, bonus)
                        .ok_or_else(no_attribute)?
                } else if id == "attribute.get" {
                    map.value(key).map_err(|_| no_attribute())?
                } else {
                    map.base_value(key).map_err(|_| no_attribute())?
                };
                let k = if id == "attribute.get" {
                    "commands.attribute.value.get.success"
                } else {
                    "commands.attribute.base_value.get.success"
                };
                (
                    k,
                    vec![description.clone(), name.clone(), java_double_text(v)],
                    (v * scale) as i32,
                )
            }
            "attribute.base.set" => {
                self.write_attribute(target, attribute, AttributeWrite::Base(value_arg))
                    .ok_or_else(no_attribute)?;
                (
                    "commands.attribute.base_value.set.success",
                    vec![
                        description.clone(),
                        name.clone(),
                        java_double_text(value_arg),
                    ],
                    1,
                )
            }
            "attribute.base.reset" => {
                self.write_attribute(target, attribute, AttributeWrite::ResetBase)
                    .ok_or_else(no_attribute)?;
                let v = self
                    .generic_attributes(target, attribute)
                    .and_then(|map| map.base_value(key).ok())
                    .unwrap_or(0.0);
                (
                    "commands.attribute.base_value.reset.success",
                    vec![description.clone(), name.clone(), java_double_text(v)],
                    1,
                )
            }
            "attribute.modifier.add" => {
                let write =
                    AttributeWrite::add(modifier_id, value_arg, operation_named(op), true, false)
                        .ok_or_else(no_attribute)?;
                if !self
                    .write_attribute(target, attribute, write)
                    .ok_or_else(no_attribute)?
                {
                    return Err(text_error(
                        "commands.attribute.failed.modifier_already_present",
                        &[
                            Text::literal(modifier_id),
                            description.clone(),
                            name.clone(),
                        ],
                    ));
                }
                (
                    "commands.attribute.modifier.add.success",
                    vec![
                        Text::literal(modifier_id),
                        description.clone(),
                        name.clone(),
                    ],
                    1,
                )
            }
            "attribute.modifier.remove" => {
                let write = AttributeWrite::Remove(modifier_id.to_owned());
                if self.write_attribute(target, attribute, write) != Some(true) {
                    return Err(no_modifier());
                }
                (
                    "commands.attribute.modifier.remove.success",
                    vec![
                        Text::literal(modifier_id),
                        description.clone(),
                        name.clone(),
                    ],
                    1,
                )
            }
            _ => {
                let v = map
                    .modifier_value(key, modifier_id)
                    .map_err(|_| no_modifier())?;
                (
                    "commands.attribute.modifier.value.get.success",
                    vec![
                        Text::literal(modifier_id),
                        description.clone(),
                        name.clone(),
                        java_double_text(v),
                    ],
                    (v * scale) as i32,
                )
            }
        };
        Ok((result, Text::translate(message, &args)))
    }

    // ---- /ride ----------------------------------------------------------------------------

    pub(super) fn cmd_ride(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let targets = self.entities(ctx, "target", false)?;
        let Some(target) = targets.first().copied() else {
            return Ok(0);
        };
        let name = self.target_name(target);
        // A server player other than the one in place (the console's, a function's or another
        // player's /ride): their riding state is their own, so the command runs as them, as
        // `native_mount` does. In vanilla every ServerPlayer rides alike.
        if let Target::Guest(_) = target {
            return self.ride_guest(id, ctx, target, name);
        }
        if id == "ride.dismount" {
            let Some(v) = self.ride_vehicle(target) else {
                return Err(err("commands.ride.not_riding", &[&name.plain()]));
            };
            let vehicle_name = self.target_name(v);
            match target {
                Target::Player => self.stop_riding(),
                Target::Mob(rider) => self.dismount_mob(rider),
                _ => {}
            }
            self.feedback(Text::translate(
                "commands.ride.dismount.success",
                &[name, vehicle_name],
            ));
            return Ok(1);
        }
        let vehicles = self.entities(ctx, "vehicle", false)?;
        let Some(vehicle) = vehicles.first().copied() else {
            return Ok(0);
        };
        let vehicle_name = self.target_name(vehicle);
        self.ride_mount(target, vehicle)?;
        self.feedback(Text::translate(
            "commands.ride.mount.success",
            &[name, vehicle_name],
        ));
        Ok(1)
    }

    /// `cmd_ride` for a server player not in place (`Target::Guest`): `RideCommand.dismount` and
    /// `RideCommand.mount` run as that player, through `for_player`.
    fn ride_guest(&mut self, id: &str, ctx: &Context, target: Target, name: Text) -> R<i32> {
        if id == "ride.dismount" {
            let not_riding = || err("commands.ride.not_riding", &[&name.plain()]);
            let vehicle_name = self
                .for_player(target, |g| {
                    let vehicle = g.ride_vehicle(Target::Player)?;
                    let vehicle_name = g.target_name(vehicle);
                    g.stop_riding();
                    Some(vehicle_name)
                })
                .flatten()
                .ok_or_else(not_riding)?;
            self.feedback(Text::translate(
                "commands.ride.dismount.success",
                &[name, vehicle_name],
            ));
            return Ok(1);
        }
        let vehicles = self.entities(ctx, "vehicle", false)?;
        let Some(vehicle) = vehicles.first().copied() else {
            return Ok(0);
        };
        let vehicle_name = self.target_name(vehicle);
        self.for_player(target, |g| g.ride_mount(Target::Player, vehicle))
            .unwrap_or_else(|| {
                Err(err(
                    "commands.ride.mount.failure.generic",
                    &[&name.plain(), &vehicle_name.plain()],
                ))
            })?;
        self.feedback(Text::translate(
            "commands.ride.mount.success",
            &[name, vehicle_name],
        ));
        Ok(1)
    }

    /// `RideCommand.mount` once the targets are known: `target` (the player
    /// in place or a mob) rides `vehicle`, as `startRiding(vehicle, true,
    /// true)` forces it (also for `native_mount`).
    pub(in crate::game) fn ride_mount(&mut self, target: Target, vehicle: Target) -> R<()> {
        let name = self.target_name(target);
        let vehicle_name = self.target_name(vehicle);
        if let Some(current) = self.ride_vehicle(target) {
            let current = self.target_name(current);
            return Err(err(
                "commands.ride.already_riding",
                &[&name.plain(), &current.plain()],
            ));
        }
        // RideCommand.mount: `vehicle.getType() == EntityType.PLAYER`, any server player.
        if matches!(vehicle, Target::Player | Target::Guest(_)) {
            return Err(err("commands.ride.mount.failure.cant_ride_players", &[]));
        }
        // getSelfAndPassengers: the vehicle is the target or rides it.
        let mut up = Some(vehicle);
        while let Some(v) = up {
            if v == target {
                return Err(err("commands.ride.mount.failure.loop", &[]));
            }
            up = self.ride_vehicle(v);
        }
        if !self.start_riding_target(target, vehicle, true) {
            return Err(err(
                "commands.ride.mount.failure.generic",
                &[&name.plain(), &vehicle_name.plain()],
            ));
        }
        Ok(())
    }

    /// Entity.getVehicle: the player's mount, or a mob's.
    pub(super) fn ride_vehicle(&self, t: Target) -> Option<Target> {
        let id = match t {
            Target::Player => self.riding?,
            Target::Guest(conn) => self.command_guest(conn)?.vehicle?,
            Target::Mob(id) => self.mobs.iter().find(|m| m.id == id)?.extra.mount?,
            _ => return None,
        };
        Some(if self.boats.iter().any(|b| b.id == id) {
            Target::Boat(id)
        } else if self.carts.iter().any(|c| c.id == id) {
            Target::Cart(id)
        } else if self.armor_stands.iter().any(|a| a.id == id) {
            Target::Stand(id)
        } else if self.others.iter().any(|o| o.id == id) {
            Target::Other(id)
        } else {
            Target::Mob(id)
        })
    }
    /// LivingEntity.stopRiding for a mob: off its vehicle
    /// (Entity.removePassenger), then to where the vehicle lets it out
    /// (LivingEntity.dismountVehicle).
    pub(in crate::game) fn dismount_mob(&mut self, rider: u32) {
        self.dismount_mob_with_cart(rider, None, None, false);
    }

    pub(in crate::game) fn dismount_mob_from_cart(
        &mut self,
        rider: u32,
        cart: &super::super::minecarts::Minecart,
    ) {
        self.dismount_mob_with_cart(rider, Some(cart), None, false);
    }

    pub(in crate::game) fn dismount_mob_reentrant(&mut self, rider: u32) {
        self.dismount_mob_with_cart(rider, None, None, true);
    }

    pub(in crate::game) fn dismount_mob_from_removed_vehicle(&mut self, rider: u32, y: f64) {
        self.dismount_mob_with_cart(rider, None, Some(y), false);
    }

    fn dismount_mob_with_cart(
        &mut self,
        rider: u32,
        cart: Option<&super::super::minecarts::Minecart>,
        removed_y: Option<f64>,
        notified: bool,
    ) {
        let Some(v) = self
            .mob_object(rider)
            .filter(|mob| !mob.removed)
            .and_then(|mob| mob.extra.mount)
        else {
            return;
        };
        if cart.is_some_and(|cart| cart.id != v) {
            return;
        }
        if !notified && !self.native_mob_may_ride(rider, v, false) {
            return;
        }
        // A callback can remove or remount the rider. Do not overwrite its decision.
        if self
            .mob_object(rider)
            .is_none_or(|mob| mob.removed || mob.extra.mount != Some(v))
        {
            return;
        }
        if let Some(mob) = self.mob_object_mut(rider) {
            mob.extra.mount = None;
        }
        let remaining = self
            .mob_objects()
            .filter(|m| m.extra.mount == Some(v))
            .min_by_key(|m| (m.extra.vehicle_order, m.id))
            .map(|m| m.id);
        if let Some(vehicle) = self.mob_object_mut(v) {
            vehicle.extra.rider = remaining;
        }
        let at = if let Some(y) = removed_y {
            self.mob_object(rider).map(|mob| {
                self.removed_vehicle_dismount_point(
                    mob.position,
                    f64::from(mob.width()),
                    f64::from(mob.height()),
                    y,
                )
            })
        } else {
            match cart.filter(|cart| cart.id == v) {
                Some(cart) => self
                    .mob_object(rider)
                    .and_then(|rider| self.cart_dismount_point_from(cart, rider)),
                None => self.mob_dismount_point(v, rider),
            }
        };
        let Some(m) = self.mob_object_mut(rider) else {
            return;
        };
        let at = at.unwrap_or(m.position);
        m.extra.passenger = false;
        m.extra.boarding_cooldown = super::super::boat_passengers::BOARDING_COOLDOWN;
        m.position = at;
        m.previous = at;
        self.snap_mob_passengers(rider);
        self.release_removed_vehicle(v);
    }
    /// Where `dismount_mob` sets object `rider` down off the vehicle `v`: None when
    /// `v` is no boat, cart or mob.
    pub(in crate::game) fn mob_dismount_point(
        &self,
        v: u32,
        rider: u32,
    ) -> Option<horizon_core::geometry::Vec3> {
        let mob = self.mob_object(rider)?;
        if let Some(vehicle) = self.mob_object(v).filter(|vehicle| vehicle.removed) {
            return Some(self.removed_vehicle_dismount_point(
                mob.position,
                f64::from(mob.width()),
                f64::from(mob.height()),
                vehicle.position.y,
            ));
        }
        let removed_y = self
            .persist
            .removed_vehicles
            .get(&v)
            .map(|vehicle| vehicle.position().y)
            .or_else(|| {
                self.persist
                    .removals
                    .iter()
                    .chain(self.persist.destroying_vehicles.iter())
                    .find_map(|target| match *target {
                        Target::Boat(id) if id == v => {
                            self.boats.iter().find(|b| b.id == v).map(|b| b.position.y)
                        }
                        Target::Cart(id) if id == v => {
                            self.carts.iter().find(|c| c.id == v).map(|c| c.position.y)
                        }
                        _ => None,
                    })
            });
        if let Some(y) = removed_y {
            return Some(self.removed_vehicle_dismount_point(
                mob.position,
                f64::from(mob.width()),
                f64::from(mob.height()),
                y,
            ));
        }
        let (width, yaw) = (f64::from(mob.width()), mob.yaw);
        if let Some(b) = self.boat_object(v) {
            return Some(self.boat_dismount_target(b, width, yaw));
        }
        self.horse_dismount_point(v, rider)
            .or_else(|| self.cart_dismount_point(v, rider))
            .or_else(|| {
                // Entity.getDismountLocationForPassenger: on top of the vehicle.
                let vehicle = self.mob_object(v)?;
                Some(super::super::mounts::on_top(vehicle))
            })
    }

    // ---- /spreadplayers -----------------------------------------------------------------------

    pub(super) fn cmd_spreadplayers(&mut self, ctx: &Context) -> R<i32> {
        let centre = self
            .position_arg(ctx, "center")
            .unwrap_or_else(|| self.source_position());
        let float = |name: &str| match ctx.get(name) {
            Some(Arg::Float(v)) => f64::from(*v),
            _ => 0.0,
        };
        let (spread, range) = (float("spreadDistance"), float("maxRange"));
        let respect = matches!(ctx.get("respectTeams"), Some(Arg::Bool(true)));
        let max_y = match ctx.get("maxHeight") {
            Some(Arg::Int(h)) => *h,
            _ => self.world.top_y(),
        };
        if max_y < self.world.bottom_y() {
            return Err(err(
                "commands.spreadplayers.failed.invalid.height",
                &[&max_y, &self.world.bottom_y()],
            ));
        }
        let targets = super::without_knots(self.entities(ctx, "targets", false)?)?;
        if targets.is_empty() {
            return Err(err("argument.entity.notfound.entity", &[]));
        }
        // SpreadPlayersCommand: one position per entity, or per team (every
        // non-player and team-less player sharing one) with respectTeams.
        let team_of = |g: &mut Self, t: Target| -> Option<String> {
            if t != Target::Player {
                return None;
            }
            g.holder_name(t)
                .and_then(|h| g.score.board.players_team(&h).map(|x| x.name.clone()))
        };
        let count = if respect {
            let mut seen: Vec<Option<String>> = Vec::new();
            for &t in &targets {
                let team = team_of(self, t);
                if !seen.contains(&team) {
                    seen.push(team);
                }
            }
            seen.len()
        } else {
            targets.len()
        };
        let centre_f = [centre[0] as f32, centre[2] as f32];
        let (cx, cz) = (f64::from(centre_f[0]), f64::from(centre_f[1]));
        let (min_x, max_x) = (cx - range, cx + range);
        let (min_z, max_z) = (cz - range, cz + range);
        // Mth.nextDouble.
        let next = |g: &mut Self, lo: f64, hi: f64| {
            if lo >= hi {
                lo
            } else {
                g.world.random.next_double() * (hi - lo) + lo
            }
        };
        let mut points: Vec<[f64; 2]> = Vec::with_capacity(count);
        for _ in 0..count {
            let x = next(self, min_x, max_x);
            let z = next(self, min_z, max_z);
            points.push([x, z]);
        }
        let mut collisions = true;
        let mut min_distance = f64::from(f32::MAX);
        let mut iteration = 0;
        while iteration < 10_000 && collisions {
            collisions = false;
            min_distance = f64::from(f32::MAX);
            for i in 0..points.len() {
                let mut neighbours = 0;
                let mut avg = [0.0f64, 0.0f64];
                for j in 0..points.len() {
                    if i == j {
                        continue;
                    }
                    let (dx, dz) = (points[i][0] - points[j][0], points[i][1] - points[j][1]);
                    let dist = (dx * dx + dz * dz).sqrt();
                    min_distance = min_distance.min(dist);
                    if dist < spread {
                        neighbours += 1;
                        avg[0] += points[j][0] - points[i][0];
                        avg[1] += points[j][1] - points[i][1];
                    }
                }
                if neighbours > 0 {
                    avg[0] /= f64::from(neighbours);
                    avg[1] /= f64::from(neighbours);
                    let length = (avg[0] * avg[0] + avg[1] * avg[1]).sqrt();
                    if length > 0.0 {
                        points[i][0] -= avg[0] / length;
                        points[i][1] -= avg[1] / length;
                    } else {
                        points[i][0] = next(self, min_x, max_x);
                        points[i][1] = next(self, min_z, max_z);
                    }
                    collisions = true;
                }
                let p = &mut points[i];
                let before = *p;
                p[0] = if p[0] < min_x {
                    min_x
                } else if p[0] > max_x {
                    max_x
                } else {
                    p[0]
                };
                p[1] = if p[1] < min_z {
                    min_z
                } else if p[1] > max_z {
                    max_z
                } else {
                    p[1]
                };
                if before != *p {
                    collisions = true;
                }
            }
            if !collisions {
                for i in 0..points.len() {
                    if !self.spread_safe(points[i], max_y) {
                        points[i][0] = next(self, min_x, max_x);
                        points[i][1] = next(self, min_z, max_z);
                        collisions = true;
                    }
                }
            }
            iteration += 1;
        }
        if min_distance == f64::from(f32::MAX) {
            min_distance = 0.0;
        }
        let float = |v: f32| Text::literal(horizon_core::java_number::float_to_string(v));
        if iteration >= 10_000 {
            let key = if respect {
                "commands.spreadplayers.failed.teams"
            } else {
                "commands.spreadplayers.failed.entities"
            };
            return Err(text_error(
                key,
                &[
                    num(points.len()),
                    float(centre_f[0]),
                    float(centre_f[1]),
                    Text::literal(super::percent_f(min_distance, 2)),
                ],
            ));
        }
        // setPlayerPositions.
        let mut total = 0.0;
        let mut index = 0;
        let mut team_points: Vec<(Option<String>, usize)> = Vec::new();
        for &t in &targets {
            let i = if respect {
                let team = team_of(self, t);
                match team_points.iter().find(|(k, _)| *k == team) {
                    Some((_, i)) => *i,
                    None => {
                        team_points.push((team, index));
                        index += 1;
                        index - 1
                    }
                }
            } else {
                index += 1;
                index - 1
            };
            let p = points[i];
            let y = self.spread_spawn_y(p, max_y);
            let mut to = [p[0].floor() + 0.5, f64::from(y), p[1].floor() + 0.5];
            let mut skip = false;
            if mod_api::__private::listened(mod_api::HookId::SpreadPlayersTeleport)
                && let (Some(entity), Some(from)) =
                    (self.network_entity_id(t), self.target_position(t))
            {
                let mut moved = to;
                let uuid = match t {
                    Target::Player => self.lan_acting_uuid,
                    Target::Guest(conn) => self.command_guest(conn).map(|g| g.uuid),
                    _ => None,
                };
                self.native_spread_players_teleport(
                    self.dim.id(),
                    entity as u32,
                    uuid,
                    from,
                    &mut moved,
                    &mut skip,
                );
                if moved.iter().all(|v| v.is_finite()) {
                    to = moved;
                }
            }
            if !skip {
                self.teleport_to(t, to, None, false);
                self.teleport_stops(t);
            }
            let closest = points
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(_, q)| ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt())
                .fold(f64::MAX, f64::min);
            total += closest;
        }
        let average = if targets.len() < 2 {
            0.0
        } else {
            total / targets.len() as f64
        };
        let key = if respect {
            "commands.spreadplayers.success.teams"
        } else {
            "commands.spreadplayers.success.entities"
        };
        self.feedback(Text::translate(
            key,
            &[
                num(points.len()),
                float(centre_f[0]),
                float(centre_f[1]),
                Text::literal(super::percent_f(average, 2)),
            ],
        ));
        Ok(points.len() as i32)
    }
    /// `SpreadPlayersCommand.Position.getSpawnY`.
    fn spread_spawn_y(&self, p: [f64; 2], max_y: i32) -> i32 {
        let (x, z) = (p[0].floor() as i32, p[1].floor() as i32);
        let air = |y: i32| {
            let name: &str = &self.res.facts().get(self.world.block(x, y, z)).name;
            matches!(
                name,
                "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
            )
        };
        let mut y = max_y + 1;
        let mut air2 = air(y);
        y -= 1;
        let mut air1 = air(y);
        while y > self.world.bottom_y() {
            y -= 1;
            let current = air(y);
            if !current && air1 && air2 {
                return y + 1;
            }
            air2 = air1;
            air1 = current;
        }
        max_y + 1
    }
    /// `SpreadPlayersCommand.Position.isSafe`.
    fn spread_safe(&self, p: [f64; 2], max_y: i32) -> bool {
        let y = self.spread_spawn_y(p, max_y) - 1;
        let (x, z) = (p[0].floor() as i32, p[1].floor() as i32);
        let name = self.res.facts().get(self.world.block(x, y, z)).name.clone();
        let liquid = matches!(&*name, "minecraft:water" | "minecraft:lava");
        let teleportable = self
            .res
            .items()
            .block_tags
            .get("entities_can_teleport_to")
            .or_else(|| {
                self.res
                    .items()
                    .block_tags
                    .get("minecraft:entities_can_teleport_to")
            })
            .is_some_and(|set| {
                set.contains(&*name) || set.contains(name.trim_start_matches("minecraft:"))
            });
        y < max_y && !liquid && teleportable
    }
}

/// A `CommandError` whose message is a translation with component arguments.
fn text_error(key: &str, args: &[Text]) -> horizon_client::command::CommandError {
    horizon_client::command::CommandError::new(Text::translate(key, args))
}

/// `PlaySoundCommand.playSound`: the point and the volume one target hears the
/// sound at. A target past `range` hears nothing unless `minVolume` is above
/// zero, and then the point moves two blocks from the target towards the sound
/// and the volume is `minVolume`; a target exactly at `range` still hears it
/// (the command's test is `>`, unlike `PlayerList.broadcast`'s `<`).
fn playsound_placement(
    pos: [f64; 3],
    target: [f64; 3],
    range: f32,
    volume: f32,
    min_volume: f32,
) -> Option<([f64; 3], f32)> {
    let d = [pos[0] - target[0], pos[1] - target[1], pos[2] - target[2]];
    let dist2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
    // PlaySoundCommand compares against Mth.square(float range): the square
    // rounds in float and only then widens, so a point between that square and
    // the double square is out of range in vanilla.
    if dist2 <= f64::from(range * range) {
        return Some((pos, volume));
    }
    if min_volume <= 0.0 {
        return None;
    }
    let dist = dist2.sqrt();
    Some((
        [
            target[0] + d[0] / dist * 2.0,
            target[1] + d[1] / dist * 2.0,
            target[2] + d[2] / dist * 2.0,
        ],
        min_volume,
    ))
}

/// `Entity.lookAt`: yaw and pitch from `from` towards `to`, the yaw wrapped
/// to -180..180 as `Mth.wrapDegrees` does.
pub(super) fn look_at(from: [f64; 3], to: [f64; 3]) -> [f32; 2] {
    let (dx, dy, dz) = (to[0] - from[0], to[1] - from[1], to[2] - from[2]);
    let horizontal = (dx * dx + dz * dz).sqrt();
    // `Entity.lookAt` turns radians to degrees with the float constant
    // `57.2957763671875`, not `Math.toDegrees`.
    let pitch = wrap_degrees((-(dy.atan2(horizontal) * 57.295_776_367_187_5)) as f32);
    let yaw = (dz.atan2(dx) * 57.295_776_367_187_5) as f32 - 90.0;
    [wrap_degrees(yaw), pitch]
}

/// `AttributeModifier.Operation` by its name in `/attribute … modifier add`.
fn operation_named(op: &str) -> horizon_world::attributes::Operation {
    use horizon_world::attributes::Operation;
    match op {
        "add_multiplied_base" => Operation::AddMultipliedBase,
        "add_multiplied_total" => Operation::AddMultipliedTotal,
        _ => Operation::AddValue,
    }
}

/// The operation `/attribute … modifier add` names last.
fn command_operation(ctx: &Context) -> horizon_world::attributes::Operation {
    operation_named(ctx.literals.last().map_or("add_value", String::as_str))
}

#[cfg(test)]
mod allay_owner_damage_tests {
    use crate::game::native_entity_refs::tests::with_game_at;
    use horizon_client::mobs::{Damage, Mob};
    use horizon_core::geometry::Vec3;

    #[test]
    #[ignore = "daily"]
    fn allay_owner_adapters_compare_the_actual_player_uuid() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            eprintln!("Prepared resources unavailable; Allay Game damage coverage did not run");
            return;
        }
        with_game_at(root, |game| {
            game.build_registries();
            let actual = [101, 102, 103, 104];
            let other = [10, 20, 30, 40];
            game.level.player_uuid = Some(actual);
            game.lan_acting_uuid = Some(horizon_client::lan::playerdata::ints_uuid(actual));
            assert_eq!(game.owner_damage_uuid(), Some(actual));
            game.lan_acting_uuid = Some(horizon_client::lan::playerdata::ints_uuid(other));
            assert_eq!(
                game.owner_damage_uuid(),
                None,
                "a guest is not the owner route"
            );
            game.lan_acting_uuid = None;
            game.level.player_uuid = None;
            assert_eq!(
                game.owner_damage_uuid(),
                None,
                "no invented UUID for an unknown source"
            );
            game.level.player_uuid = Some(actual);
            game.lan_acting_uuid = Some(horizon_client::lan::playerdata::ints_uuid(actual));
            let kind = game.res.mob_types().get("minecraft:allay").unwrap().clone();
            let before = game.world.game_time;
            for melee in [true, false] {
                for liked in [Some(actual), None, Some(other)] {
                    game.mobs.clear();
                    let mut mob = Mob::new(1, kind.clone(), Vec3::new(22.5, 101.0, 22.5), 0.0);
                    mob.no_ai = true;
                    mob.extra.tame = liked.is_some();
                    mob.extra.owner = liked;
                    assert_eq!((mob.health, mob.hurt_time), (20.0, 0));
                    game.mobs.push(mob);
                    let source = Damage::player([20.5, 102.0, 20.5]);
                    let accepted = if melee {
                        game.hurt_by_player(0, 2.0, source)
                    } else {
                        // The command entry point lends its resolved attacker to the damage path.
                        let attack = game.attack_of_who(source.attacker);
                        game.with_attack(attack, |g| g.command_damage_mob(1, 2.0, source))
                    };
                    let expected = liked != Some(actual);
                    assert_eq!(accepted, expected, "melee={melee}, liked={liked:?}");
                    assert_eq!(game.mobs[0].health, if expected { 18.0 } else { 20.0 });
                    assert_eq!(game.mobs[0].extra.owner, liked);
                    assert_eq!(game.world.game_time, before);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{look_at, playsound_placement, take_new_reinforcements};
    use horizon_client::mobs::{MobEvent, Who};

    #[test]
    fn command_reinforcements_leave_prior_requests_and_other_events_queued() {
        let request = |mob| MobEvent::Reinforcements {
            mob,
            target: Who::Player,
        };
        let mut events = vec![request(1), MobEvent::Sound("before", [0.; 3], 1., 1.)];
        let queued = events.len();
        events.extend([
            request(2),
            MobEvent::Sound("after", [0.; 3], 1., 1.),
            request(3),
        ]);
        assert_eq!(
            take_new_reinforcements(&mut events, queued),
            [(2, Who::Player), (3, Who::Player)]
        );
        assert!(matches!(
            events.as_slice(),
            [
                MobEvent::Reinforcements { mob: 1, .. },
                MobEvent::Sound("before", ..),
                MobEvent::Sound("after", ..)
            ]
        ));
        assert!(take_new_reinforcements(&mut events, queued).is_empty());
        assert_eq!(events.len(), 3);
    }

    #[test]
    fn command_reinforcements_tolerate_a_damage_handler_clearing_its_outbox() {
        let mut events = vec![MobEvent::Sound("after", [0.; 3], 1., 1.)];
        assert!(take_new_reinforcements(&mut events, 4).is_empty());
        assert_eq!(events.len(), 1);
    }

    fn assert_angle(got: f32, want: f32) {
        assert!((got - want).abs() < 1.0e-4, "{got} is not {want}");
    }

    /// `Entity.lookAt` wraps the yaw with `Mth.wrapDegrees`, so a target to
    /// the north-west turns the entity to 135 degrees, not to -225, and a
    /// target due north leaves -180 rather than 180.
    #[test]
    fn a_facing_target_wraps_the_yaw_to_180() {
        let at = [0.0, 64.0, 0.0];
        assert_angle(look_at(at, [-1.0, 64.0, -1.0])[0], 135.0);
        assert_angle(look_at(at, [0.0, 64.0, -1.0])[0], -180.0);
        assert_angle(look_at(at, [1.0, 64.0, 0.0])[0], -90.0);
        assert_angle(look_at(at, [0.0, 64.0, 1.0])[0], 0.0);
        assert_angle(look_at(at, [-1.0, 64.0, 0.0])[0], 90.0);
        // The pitch is the elevation, unchanged; its own wrap is a no-op.
        assert_angle(look_at(at, [0.0, 65.0, 1.0])[1], -45.0);
    }

    /// `PlaySoundCommand.playSound`: inside the range the point and volume are
    /// the command's own, exactly on it still is, past it there is nothing
    /// without a minimum volume, and with one the point is two blocks from the
    /// target towards the sound and the volume is that minimum.
    #[test]
    fn a_target_past_the_range_hears_it_two_blocks_away() {
        let pos = [0.0, 64.0, 0.0];
        let near = [0.0, 64.0, 15.0];
        assert_eq!(
            playsound_placement(pos, near, 16.0, 3.0, 0.0),
            Some((pos, 3.0))
        );
        let on_the_edge = [16.0, 64.0, 0.0];
        assert_eq!(
            playsound_placement(pos, on_the_edge, 16.0, 3.0, 0.0),
            Some((pos, 3.0))
        );
        let far = [0.0, 64.0, 32.0];
        assert_eq!(playsound_placement(pos, far, 16.0, 3.0, 0.0), None);
        assert_eq!(
            playsound_placement(pos, far, 16.0, 3.0, 0.5),
            Some(([0.0, 64.0, 30.0], 0.5))
        );
    }

    /// The range is squared as a float, as `Mth.square(getRange(volume))` is:
    /// a point between that square and the double one is past the range in
    /// vanilla, so it gets the `minVolume` placement rather than full volume.
    #[test]
    fn the_range_square_rounds_as_a_float() {
        let volume = 1.1f32;
        let range = 16.0 * volume;
        let float_square = f64::from(range * range);
        let double_square = f64::from(range) * f64::from(range);
        assert!(float_square < double_square);
        let dist2 = (float_square + double_square) / 2.0;
        let pos = [0.0, 64.0, 0.0];
        let target = [dist2.sqrt(), 64.0, 0.0];
        assert_eq!(
            playsound_placement(pos, target, range, volume, 0.0),
            None,
            "past the float range"
        );
        let moved = playsound_placement(pos, target, range, volume, 0.7);
        let Some((point, heard)) = moved else {
            panic!("minVolume should place it");
        };
        assert_eq!(heard, 0.7);
        assert!((point[0] - (target[0] - 2.0)).abs() < 1e-9, "{point:?}");
        assert_eq!(point[1..], [64.0, 0.0]);
    }
}

#[cfg(test)]
mod vehicle_damage_command_tests {
    use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};
    use horizon_client::chunk::Chunk;
    use horizon_core::position::ChunkPos;
    use std::sync::Arc;
    fn with_game(test: impl FnOnce(&mut Game<'_>)) {
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.game_dir = Some(
            std::path::PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap())
                .join(format!("h050-{:?}", std::thread::current().id())),
        );
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);

        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("difficulty normal"));
        assert!(game.run_command("forceload add 0 0"));
        test(&mut game);
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn generic_commands_reach_actual_vehicle_damage_and_drop() {
        with_game(|game| {
            game.creative = true; // issuer creative is not an attacker
            let clock = game.world.game_time;
            for (kind, n) in [("oak_boat", 1), ("minecart", 2)] {
                let uuid = format!("000001f4-0000-0000-0000-{n:012x}");
                assert!(game.run_command(&format!(
                    "summon {kind} 4.5 200 5.5 {{UUID:[I;500,0,0,{n}],NoGravity:1b}}"
                )));
                let id = game
                    .boats
                    .iter()
                    .map(|b| b.id)
                    .chain(game.carts.iter().map(|c| c.id))
                    .find(|id| game.persist.uuid_of(*id) == Some([500, 0, 0, n]))
                    .unwrap();
                assert!(
                    game.run_command(&format!("damage {uuid} 1 minecraft:generic")),
                    "accepted small hit must reach vehicle"
                );
                let damage = if kind == "oak_boat" {
                    game.boats.iter().find(|b| b.id == id).unwrap().damage
                } else {
                    game.carts.iter().find(|c| c.id == id).unwrap().damage
                };
                assert_eq!(damage, 10.0);
                assert!(game.run_command(&format!("damage {uuid} 4 minecraft:generic")));
                assert!(!game.boats.iter().any(|b| b.id == id));
                assert!(!game.carts.iter().any(|c| c.id == id));
                assert_eq!(
                    game.dropped
                        .iter()
                        .filter(|d| d.stack.item.as_ref() == format!("minecraft:{kind}")
                            && d.stack.count == 1)
                        .count(),
                    1
                );
                let n = n + 2;
                let uuid = format!("000001f4-0000-0000-0000-{n:012x}");
                assert!(game.run_command(&format!(
                    "summon {kind} 8.5 200 5.5 {{UUID:[I;500,0,0,{n}],NoGravity:1b}}"
                )));
                let id = game
                    .boats
                    .iter()
                    .map(|b| b.id)
                    .chain(game.carts.iter().map(|c| c.id))
                    .find(|id| game.persist.uuid_of(*id) == Some([500, 0, 0, n]))
                    .unwrap();
                assert!(game.run_command(&format!("damage {uuid} 4 minecraft:generic")));
                let damage = if kind == "oak_boat" {
                    game.boats.iter().find(|b| b.id == id).unwrap().damage
                } else {
                    game.carts.iter().find(|c| c.id == id).unwrap().damage
                };
                assert_eq!(damage, 40.0, "exact threshold survives");
            }
            // Fresh rejected subjects avoid relying on vehicle data adoption.
            for (kind, n) in [("oak_boat", 5), ("minecart", 6)] {
                let uuid = format!("000001f4-0000-0000-0000-{n:012x}");
                assert!(game.run_command(&format!("summon {kind} 11.5 200 5.5 {{UUID:[I;500,0,0,{n}],NoGravity:1b,Invulnerable:1b}}")));
                let id = game
                    .boats
                    .iter()
                    .map(|b| b.id)
                    .chain(game.carts.iter().map(|c| c.id))
                    .find(|id| game.persist.uuid_of(*id) == Some([500, 0, 0, n]))
                    .unwrap();
                assert!(!game.run_command(&format!("damage {uuid} 4 minecraft:generic")));
                let damage = if kind == "oak_boat" {
                    game.boats.iter().find(|b| b.id == id).unwrap().damage
                } else {
                    game.carts.iter().find(|c| c.id == id).unwrap().damage
                };
                assert_eq!(damage, 0.0);
            }
            assert_eq!(game.world.game_time, clock);
        });
    }
    fn attribute_guests(
        game: &mut Game<'_>,
    ) -> Vec<std::sync::mpsc::Receiver<horizon_client::lan::Out>> {
        use horizon_client::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
        use horizon_protocol::packets::configuration::ClientInformation;
        use std::sync::{atomic::AtomicI32, mpsc};
        game.world_root = game.options.game_dir.as_ref().unwrap().join("h082-world");
        std::fs::create_dir_all(&game.world_root).unwrap();
        game.lan_host(
            crate::game::lan::LanSettings {
                port: 0,
                guest_commands: true,
                force_game_mode: false,
            },
            NetConfig {
                announce: false,
                ..NetConfig::loopback(0, "attribute-test".into(), 2)
            },
            HubConfig {
                default_game_mode: 1,
                guest_commands: true,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: "minecraft:overworld".into(),
                allow_flight: false,
                seed: 0,
                max_players: 3,
                integrated: false,
                levels: Default::default(),
            },
        )
        .unwrap();
        let mut receivers = Vec::new();
        for (conn, name, position) in [
            (2, "Bob", [7.5, 100.0, 4.5]),
            (3, "Carol", [10.5, 100.0, 4.5]),
        ] {
            let (tx, rx) = mpsc::channel();
            let mut guest = Guest::new(
                Joined {
                    conn,
                    uuid: horizon_protocol::crypto::offline_uuid(name),
                    name: name.into(),
                    properties: Vec::new(),
                    info: ClientInformation::new(2),
                    brand: "test".into(),
                    sender: PlayerSender::new(tx, Arc::default()),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:25565".parse().unwrap(),
                    expected: false,
                },
                conn as i32,
                position,
                2,
            );
            guest.dimension = "minecraft:overworld".into();
            guest.placed = true;
            guest.loaded = true;
            guest.game_mode = 1;
            game.lan.as_mut().unwrap().hub.guests.push(guest);
            receivers.push(rx);
        }
        let host = game.lan.as_mut().unwrap();
        for name in ["Bob", "Carol"] {
            host.hub
                .ops
                .insert(horizon_protocol::crypto::offline_uuid(name), 2);
        }
        host.hub.refresh_permissions();
        assert!(host.hub.guests.iter().all(|guest| guest.permission == 2));
        receivers
    }
    fn attribute_chat_count(rx: &std::sync::mpsc::Receiver<horizon_client::lan::Out>) -> usize {
        use horizon_client::lan::Out;
        use horizon_protocol::packets::play::Clientbound;
        let mut bodies = Vec::new();
        for out in rx.try_iter() {
            match out {
                Out::Packet(body) => bodies.push(body),
                Out::Packets(more) => bodies.extend(more),
                Out::Disconnect(_) => panic!("actual registered guest disconnected"),
            }
        }
        bodies
            .iter()
            .filter(|body| {
                let mut r = horizon_protocol::buf::Reader::new(body);
                let id = r.varint().unwrap();
                matches!(
                    Clientbound::decode(id, r.rest()).unwrap(),
                    Clientbound::SystemChat(_)
                )
            })
            .count()
    }
    #[test]
    #[ignore = "needs prepared resources; real registered Guest context and issuer feedback"]
    fn direct_guest_attributes_adopt_the_target_and_restore_the_issuer() {
        use crate::game::commands::Target;
        use horizon_client::motion::PlayerMotion;
        use horizon_core::geometry::Vec3;
        with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 100.0, 4.5)).unwrap());
            crate::game::native_entity_refs::tests::bind_local_player(game, 1, 99);
            game.creative = true;
            assert!(game.run_command("fill 0 99 0 15 99 15 stone"));
            assert!(game.run_command("attribute @s minecraft:armor base set 2"));
            let host_uuid = game.acting_uuid();
            let clock = game.world.game_time;
            let dimension = game.dim;
            let receivers = attribute_guests(game);
            for (conn, name) in [(2, "Bob"), (3, "Carol")] {
                let g = game.command_guest(conn).unwrap();
                assert_eq!(g.uuid, horizon_protocol::crypto::offline_uuid(name));
                assert_eq!(g.health, 20.0);
                assert!(g.loaded && g.placed);
                assert_eq!(g.game_mode, 1);
                assert_eq!(
                    game.for_player(Target::Guest(conn), |g| g
                        .player
                        .as_mut()
                        .unwrap()
                        .attribute("minecraft:armor")
                        .unwrap()),
                    Some(0.0)
                );
            }
            // Existing guest-issued Player path is positive before direct Guest dispatch.
            assert_eq!(
                game.for_player(Target::Guest(2), |g| g
                    .run_command("attribute @s minecraft:armor base set 3")),
                Some(true)
            );
            assert_eq!(
                game.for_player(Target::Guest(2), |g| g
                    .player
                    .as_mut()
                    .unwrap()
                    .attribute("minecraft:armor")
                    .unwrap()),
                Some(3.0)
            );
            for rx in &receivers {
                attribute_chat_count(rx);
            }
            game.chat_capture = Some(Vec::new());
            let accepted = game.run_command("attribute Bob minecraft:armor base set 4");
            let issuer = game.chat_capture.take().unwrap();
            // This assertion fails old on the actual guest model, not feedback alone.
            assert_eq!(
                game.for_player(Target::Guest(2), |g| g
                    .player
                    .as_mut()
                    .unwrap()
                    .attribute("minecraft:armor")
                    .unwrap()),
                Some(4.0)
            );
            assert!(accepted);
            assert_eq!(issuer.len(), 1);
            assert!(issuer[0].plain().contains("Bob"));
            assert_eq!(
                attribute_chat_count(&receivers[0]),
                0,
                "success belongs to issuer, not Bob"
            );
            assert_eq!(
                game.player
                    .as_mut()
                    .unwrap()
                    .attribute("minecraft:armor")
                    .unwrap(),
                2.0
            );
            assert_eq!(
                game.for_player(Target::Guest(3), |g| g
                    .player
                    .as_mut()
                    .unwrap()
                    .attribute("minecraft:armor")
                    .unwrap()),
                Some(0.0)
            );
            assert_eq!(
                game.for_player(Target::Guest(2), |g| g
                    .player
                    .as_mut()
                    .unwrap()
                    .attribute("minecraft:max_health")
                    .unwrap()),
                Some(20.0)
            );
            game.chat_capture = Some(Vec::new());
            assert!(
                !game.run_command("attribute Bob minecraft:armor modifier remove h082:missing")
            );
            let errors = game.chat_capture.take().unwrap();
            assert_eq!(errors.len(), 1);
            assert!(errors[0].plain().contains("h082:missing"));
            assert_eq!(
                attribute_chat_count(&receivers[0]),
                0,
                "error returns to issuer"
            );
            game.chat_capture = Some(Vec::new());
            assert!(game.run_command("attribute Bob minecraft:armor base reset"));
            assert_eq!(game.chat_capture.take().unwrap().len(), 1);
            assert_eq!(
                game.for_player(Target::Guest(2), |g| g
                    .player
                    .as_mut()
                    .unwrap()
                    .attribute("minecraft:armor")
                    .unwrap()),
                Some(0.0)
            );
            assert_eq!(game.acting_uuid(), host_uuid);
            assert_eq!(game.lan_acting, None);
            assert_eq!(game.dim, dimension);
            assert_eq!(game.world.game_time, clock);
            assert_eq!(game.player.as_ref().unwrap().health.health(), 20.0);
        });
    }
    #[test]
    #[ignore = "needs prepared resources; existing target/operation reply controls"]
    fn existing_attribute_operations_keep_their_exact_success_messages() {
        use horizon_client::motion::PlayerMotion;
        use horizon_core::geometry::Vec3;
        with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 200.0, 4.5)).unwrap());
            crate::game::native_entity_refs::tests::bind_local_player(game, 1, 99);
            assert!(
                game.run_command("summon pig 6.5 200 4.5 {UUID:[I;82,0,0,1],NoAI:1b,NoGravity:1b}")
            );
            assert!(
                game.run_command("summon armor_stand 8.5 200 4.5 {UUID:[I;82,0,0,2],NoGravity:1b}")
            );
            for target in [
                "@s",
                "00000052-0000-0000-0000-000000000001",
                "00000052-0000-0000-0000-000000000002",
            ] {
                for operation in [
                    "get",
                    "base get",
                    "base set 2",
                    "base get",
                    "base reset",
                    "modifier add h082:reply 1 add_value",
                    "modifier value get h082:reply",
                    "get",
                    "modifier remove h082:reply",
                ] {
                    let command = format!("attribute {target} minecraft:armor {operation}");
                    game.chat_capture = Some(Vec::new());
                    game.cmds.results = Some(Vec::new());
                    assert!(game.run_command(&command), "{command}");
                    let messages = game.chat_capture.take().unwrap();
                    assert_eq!(messages.len(), 1, "{command}");
                    let components: Vec<_> = messages.iter().map(|m| m.component()).collect();
                    println!(
                        "H082_EXISTING {}",
                        serde_json::json!({"command":command,"messages":components,"results":game.cmds.results.take().unwrap()})
                    );
                }
            }
        });
    }
}

impl Game<'_> {
    /// `PoiTypes`: the point of interest type of a block state.
    pub(super) fn poi_type(&self, state: horizon_world::registry::BlockStateId) -> Option<&str> {
        let table = &self.stations().professions;
        let state = self.pack.assets.registry.state(state)?;
        table.pois.name(table.pois.classify(state)?)
    }
}
