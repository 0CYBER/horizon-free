//! `/recipe`, `/damage`, `/rotate`, `/swing`, `/playsound`, `/stopsound`,
//! `/particle`, `/attribute`, `/ride` and `/spreadplayers`.
use super::{Arg, Context, Game, R, Target, err};
use crate::game::chat::PLAYER_NAME;
use horizon_client::{
    mobs::{Damage, DamageKind, Who},
    scoreboard_cmd::{Holder, Tracker},
    settings::SOUND_SOURCES,
    text::Text,
};
use horizon_core::random::RandomSource;
use horizon_world::attribute_map::ValueKind;

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
    Text::literal(horizon_client::snbt::java_double(v))
}

impl Game<'_> {
    // ---- /recipe -----------------------------------------------------------

    /// Every `minecraft:` recipe that `/recipe` can give (non-special ones).
    pub(super) fn learnable_recipes(&self) -> Vec<String> {
        let dir = self.options.resources.join("data/minecraft/recipe");
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return out;
        };
        for e in entries.flatten() {
            let path = e.path();
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if path.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            let special = std::fs::read(&path)
                .ok()
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                .and_then(|v| v["type"].as_str().map(|t| t.contains("crafting_special_")))
                .unwrap_or(false);
            if !special {
                out.push(format!("minecraft:{stem}"));
            }
        }
        out.sort();
        out
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
            let mut n = 0;
            for r in &recipes {
                let changed = if give {
                    self.adv.recipes.insert(r.clone())
                } else {
                    self.adv.recipes.remove(r)
                };
                n += i32::from(changed);
            }
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

    // ---- /damage ----------------------------------------------------------------

    fn damage_kind(ty: &str) -> DamageKind {
        match ty.trim_start_matches("minecraft:") {
            "player_attack" => DamageKind::PlayerAttack,
            "mob_attack" | "mob_attack_no_aggro" => DamageKind::MobAttack,
            "arrow" => DamageKind::Arrow,
            "thrown" | "trident" | "mob_projectile" => DamageKind::Thrown,
            "fireball" | "unattributed_fireball" | "wither_skull" => DamageKind::Fireball,
            "explosion" | "player_explosion" | "bad_respawn_point" => DamageKind::Explosion,
            "fall" | "ender_pearl" | "fly_into_wall" => DamageKind::Fall,
            "on_fire" => DamageKind::OnFire,
            "in_fire" | "campfire" | "hot_floor" => DamageKind::InFire,
            "lava" => DamageKind::Lava,
            "drown" => DamageKind::Drown,
            "magic" | "indirect_magic" | "dragon_breath" => DamageKind::Magic,
            "sting" => DamageKind::Sting,
            "cactus" | "sweet_berry_bush" => DamageKind::Cactus,
            "in_wall" => DamageKind::InWall,
            "thorns" => DamageKind::Thorns,
            _ => DamageKind::Generic,
        }
    }
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
        let from = match ctx.get("location") {
            Some(Arg::Coords(_)) => self.position_arg(ctx, "location"),
            _ => by.and_then(|t| self.target_position(t)),
        };
        let hurt = match target {
            Target::Player => {
                if self.creative || self.spectator() {
                    false
                } else if let Some(player) = self.player.as_mut() {
                    match self.motion.health.damage.id(&ty) {
                        Some(id) => player
                            .hurt(&self.motion.health, self.world.health_rules, id, amount)
                            .unwrap_or(false),
                        None => false,
                    }
                } else {
                    false
                }
            }
            Target::Mob(id) => {
                let attacker = by.and_then(|t| match t {
                    Target::Player => Some(Who::Player),
                    Target::Mob(i) => Some(Who::Mob(i)),
                    _ => None,
                });
                let damage = Damage {
                    kind: Self::damage_kind(&ty),
                    attacker,
                    from,
                };
                self.mobs
                    .iter_mut()
                    .find(|m| m.id == id)
                    .is_some_and(|m| m.damage(amount, damage))
            }
            _ => false,
        };
        if !hurt {
            return Err(err("commands.damage.invulnerable", &[]));
        }
        let name = self.target_name(target);
        self.feedback(Text::translate(
            "commands.damage.success",
            &[
                Text::literal(horizon_client::snbt::java_float(amount)),
                name,
            ],
        ));
        Ok(1)
    }

    // ---- /rotate and /swing -----------------------------------------------------------

    pub(super) fn cmd_rotate(&mut self, ctx: &Context) -> R<i32> {
        let targets = self.entities(ctx, "target", false)?;
        let Some(t) = targets.first().copied() else {
            return Ok(0);
        };
        let Some(pos) = self.target_position(t) else {
            return Ok(0);
        };
        let current = self.target_rotation(t);
        let rot = if let Some(Arg::Coords(c)) = ctx.get("rotation") {
            // Relative parts are offsets from the entity's own rotation.
            c.rotation(current)
        } else {
            let to = if ctx.has("entity") {
                let faced = self.entities(ctx, "facingEntity", false)?;
                let Some(f) = faced.first().copied() else {
                    return Ok(0);
                };
                let mut p = self.target_position(f).unwrap_or(pos);
                if matches!(ctx.get("facingAnchor"), Some(Arg::Anchor(true))) {
                    p[1] += self.target_eye_height(f);
                }
                p
            } else {
                self.position_arg(ctx, "facingLocation").unwrap_or(pos)
            };
            let from = [pos[0], pos[1] + self.target_eye_height(t), pos[2]];
            look_at(from, to)
        };
        self.teleport_to(t, pos, Some(rot));
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
        let mut tracker = Tracker::default();
        for t in targets {
            if t != Target::Player {
                continue;
            }
            let Some(p) = self.target_position(t) else {
                continue;
            };
            let d = [pos[0] - p[0], pos[1] - p[1], pos[2] - p[2]];
            let dist2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
            let (at, v) = if dist2 > f64::from(range) * f64::from(range) {
                if min_volume <= 0.0 {
                    continue;
                }
                let dist = dist2.sqrt();
                (
                    [
                        p[0] + d[0] / dist * 2.0,
                        p[1] + d[1] / dist * 2.0,
                        p[2] + d[2] / dist * 2.0,
                    ],
                    min_volume,
                )
            } else {
                (pos, volume)
            };
            self.audio.play_on(source, &sound, Some(at), v, pitch);
            tracker.mark(&holder(Text::literal(PLAYER_NAME)));
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
        self.audio.stop_sounds(sound.as_deref(), source);
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
        let mut words = spec.split(' ');
        let name = words.next().unwrap_or("");
        let ty = ParticleType::from_name(name).ok_or_else(|| err("particle.notFound", &[&name]))?;
        let rest: Vec<&str> = words.collect();
        let float = |i: usize, d: f32| rest.get(i).and_then(|w| w.parse::<f32>().ok()).unwrap_or(d);
        let options = match ty {
            ParticleType::Dust => ParticleOptions::dust(
                [float(0, 1.0), float(1, 0.0), float(2, 0.0)],
                float(3, 1.0).clamp(0.01, 4.0),
            ),
            ParticleType::Block | ParticleType::FallingDust | ParticleType::DustPillar => {
                let mut reader = horizon_client::command::Reader::new(rest.join(" ").leak());
                let block = horizon_client::command::args::ArgType::BlockState
                    .parse(&mut reader, &self.ui.registries);
                match block {
                    Ok(Arg::Block(b)) => {
                        let state = self.block_state(&b)?;
                        let (sprite, tint) = self
                            .particle_sprite(state)
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
        let delta = self.position_arg(ctx, "delta").map_or([0.0; 3], |d| {
            let s = self.source_position();
            [d[0] - s[0], d[1] - s[1], d[2] - s[2]]
        });
        let speed = match ctx.get("speed") {
            Some(Arg::Float(v)) => *v,
            _ => 0.0,
        };
        let count = match ctx.get("count") {
            Some(Arg::Int(c)) => *c,
            _ => 0,
        };
        // `ServerLevel.sendParticles`: no count means one particle with the delta as velocity.
        if count <= 0 {
            self.particles.create(&options, pos, delta);
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
        Ok(count.max(1))
    }

    // ---- /attribute ---------------------------------------------------------------------

    /// The attribute id without `minecraft:`, as the attribute map names it.
    fn attribute_description(attribute: &str) -> Text {
        let short = attribute.trim_start_matches("minecraft:");
        Text::translate(&format!("attribute.name.{short}"), &[])
    }
    pub(super) fn cmd_attribute(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let targets = self.entities(ctx, "target", false)?;
        let Some(target) = targets.first().copied() else {
            return Ok(0);
        };
        let Some(Arg::Resource(attribute)) = ctx.get("attribute").cloned() else {
            return Ok(0);
        };
        let name = self.target_name(target);
        let description = Self::attribute_description(&attribute);
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
                    return Ok(0);
                };
                let Some(key) = player.attributes.key(&attribute) else {
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
                        self.feedback(Text::translate(
                            k,
                            &[description, name, java_double_text(v)],
                        ));
                        Ok((v * scale) as i32)
                    }
                    "attribute.base.set" => {
                        let handle = player
                            .attributes
                            .get_instance(Some(key))
                            .ok()
                            .flatten()
                            .ok_or_else(|| no_attribute(&name, &description))?;
                        let _ = player.attributes.edit(&handle, |value, observer| {
                            value.set_base_value(value_arg, observer)
                        });
                        self.feedback(Text::translate(
                            "commands.attribute.base_value.set.success",
                            &[description, name, java_double_text(value_arg)],
                        ));
                        Ok(1)
                    }
                    "attribute.base.reset" => {
                        if !player
                            .attributes
                            .reset_base(Some(key.clone()))
                            .unwrap_or(false)
                        {
                            return Err(no_attribute(&name, &description));
                        }
                        let v = player
                            .attributes
                            .value(Some(key.clone()), ValueKind::Base)
                            .unwrap_or(0.0);
                        self.feedback(Text::translate(
                            "commands.attribute.base_value.reset.success",
                            &[description, name, java_double_text(v)],
                        ));
                        Ok(1)
                    }
                    "attribute.modifier.add" => {
                        if player
                            .attributes
                            .has_modifier(Some(key), &modifier_id)
                            .unwrap_or(false)
                        {
                            return Err(err(
                                "commands.attribute.failed.modifier_already_present",
                                &[&modifier_id, &description.plain(), &name.plain()],
                            ));
                        }
                        let op = ctx
                            .literals
                            .last()
                            .cloned()
                            .unwrap_or_else(|| "add_value".into());
                        player
                            .set_modifier(&attribute, &modifier_id, value_arg, &op, true)
                            .map_err(|_| no_attribute(&name, &description))?;
                        self.feedback(Text::translate(
                            "commands.attribute.modifier.add.success",
                            &[Text::literal(modifier_id), description, name],
                        ));
                        Ok(1)
                    }
                    "attribute.modifier.remove" => {
                        if !player
                            .attributes
                            .has_modifier(Some(key), &modifier_id)
                            .unwrap_or(false)
                        {
                            return Err(err(
                                "commands.attribute.failed.no_modifier",
                                &[&description.plain(), &name.plain(), &modifier_id],
                            ));
                        }
                        let _ =
                            player.set_modifier(&attribute, &modifier_id, 0.0, "add_value", false);
                        self.feedback(Text::translate(
                            "commands.attribute.modifier.remove.success",
                            &[Text::literal(modifier_id), description, name],
                        ));
                        Ok(1)
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
                            return Err(err(
                                "commands.attribute.failed.no_modifier",
                                &[&description.plain(), &name.plain(), &modifier_id],
                            ));
                        };
                        self.feedback(Text::translate(
                            "commands.attribute.modifier.value.get.success",
                            &[
                                Text::literal(modifier_id),
                                description,
                                name,
                                java_double_text(v),
                            ],
                        ));
                        Ok((v * scale) as i32)
                    }
                }
            }
            Target::Mob(mob_id) => {
                let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob_id) else {
                    return Ok(0);
                };
                // Mobs keep three attributes as plain values.
                let slot: Option<f64> = match short.as_str() {
                    "max_health" => Some(f64::from(m.attrs.max_health)),
                    "movement_speed" => Some(m.attrs.speed),
                    "attack_damage" => Some(f64::from(m.attrs.attack_damage)),
                    _ => None,
                };
                let Some(current) = slot else {
                    return Err(no_attribute(&name, &description));
                };
                match id {
                    "attribute.get" | "attribute.base.get" => {
                        let k = if id == "attribute.get" {
                            "commands.attribute.value.get.success"
                        } else {
                            "commands.attribute.base_value.get.success"
                        };
                        self.feedback(Text::translate(
                            k,
                            &[description, name, java_double_text(current)],
                        ));
                        Ok((current * scale) as i32)
                    }
                    "attribute.base.set" => {
                        match short.as_str() {
                            "max_health" => {
                                m.attrs.max_health = value_arg as f32;
                                m.health = m.health.min(m.attrs.max_health);
                            }
                            "movement_speed" => m.attrs.speed = value_arg,
                            _ => m.attrs.attack_damage = value_arg as f32,
                        }
                        self.feedback(Text::translate(
                            "commands.attribute.base_value.set.success",
                            &[description, name, java_double_text(value_arg)],
                        ));
                        Ok(1)
                    }
                    "attribute.base.reset" => {
                        match short.as_str() {
                            "max_health" => m.attrs.max_health = m.kind.max_health,
                            "movement_speed" => m.attrs.speed = m.kind.speed,
                            _ => m.attrs.attack_damage = m.kind.attack_damage,
                        }
                        let v = match short.as_str() {
                            "max_health" => f64::from(m.attrs.max_health),
                            "movement_speed" => m.attrs.speed,
                            _ => f64::from(m.attrs.attack_damage),
                        };
                        self.feedback(Text::translate(
                            "commands.attribute.base_value.reset.success",
                            &[description, name, java_double_text(v)],
                        ));
                        Ok(1)
                    }
                    _ => Err(err(
                        "commands.attribute.failed.no_modifier",
                        &[&description.plain(), &name.plain(), &modifier_id],
                    )),
                }
            }
            _ => Err(err("commands.attribute.failed.entity", &[&name.plain()])),
        }
    }

    // ---- /ride ----------------------------------------------------------------------------

    pub(super) fn cmd_ride(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let targets = self.entities(ctx, "target", false)?;
        let Some(target) = targets.first().copied() else {
            return Ok(0);
        };
        let name = self.target_name(target);
        if id == "ride.dismount" {
            let vehicle = match target {
                Target::Player => self.riding.map(Target::Mob),
                _ => None,
            };
            let Some(v) = vehicle else {
                return Err(err("commands.ride.not_riding", &[&name.plain()]));
            };
            let vehicle_name = self.target_name(v);
            self.dismount_mount();
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
        if target == Target::Player && self.riding.is_some() {
            let current = self
                .riding
                .map(Target::Mob)
                .map_or_else(Text::empty, |v| self.target_name(v));
            return Err(err(
                "commands.ride.already_riding",
                &[&name.plain(), &current.plain()],
            ));
        }
        if vehicle == Target::Player {
            return Err(err("commands.ride.mount.failure.cant_ride_players", &[]));
        }
        if target == vehicle {
            return Err(err("commands.ride.mount.failure.loop", &[]));
        }
        // The player can ride mobs that carry a rider.
        let Target::Mob(mob_id) = vehicle else {
            return Err(err(
                "commands.ride.mount.failure.generic",
                &[&name.plain(), &vehicle_name.plain()],
            ));
        };
        if target != Target::Player || !self.mobs.iter().any(|m| m.id == mob_id && m.alive()) {
            return Err(err(
                "commands.ride.mount.failure.generic",
                &[&name.plain(), &vehicle_name.plain()],
            ));
        }
        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob_id) {
            m.extra.flag = false;
            m.extra.counter = 0;
            self.yaw = m.yaw;
        }
        self.riding = Some(mob_id);
        self.mount.jump_ticks = 0;
        self.mount.jump_scale = 0.0;
        self.feedback(Text::translate(
            "commands.ride.mount.success",
            &[name, vehicle_name],
        ));
        Ok(1)
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
            _ => horizon_client::chunk::MIN_Y + horizon_client::chunk::HEIGHT,
        };
        if max_y < horizon_client::chunk::MIN_Y {
            return Err(err(
                "commands.spreadplayers.failed.invalid.height",
                &[&max_y, &horizon_client::chunk::MIN_Y],
            ));
        }
        let targets = self.entities(ctx, "targets", false)?;
        if targets.is_empty() {
            return Err(err("argument.entity.notfound.entity", &[]));
        }
        // Units: each entity, or each team together with `respectTeams`.
        let mut units: Vec<Vec<Target>> = Vec::new();
        let mut teams: Vec<(String, usize)> = Vec::new();
        for &t in &targets {
            let team = if respect {
                self.holder_name(t)
                    .and_then(|h| self.score.board.players_team(&h).map(|x| x.name.clone()))
            } else {
                None
            };
            match team {
                Some(name) => match teams.iter().find(|(n, _)| *n == name) {
                    Some((_, i)) => units[*i].push(t),
                    None => {
                        teams.push((name, units.len()));
                        units.push(vec![t]);
                    }
                },
                None => units.push(vec![t]),
            }
        }
        let (min_x, max_x) = (centre[0] - range, centre[0] + range);
        let (min_z, max_z) = (centre[2] - range, centre[2] + range);
        let mut points: Vec<[f64; 2]> = units
            .iter()
            .map(|_| {
                let r = &mut self.world.random;
                [
                    min_x + r.next_double() * (max_x - min_x),
                    min_z + r.next_double() * (max_z - min_z),
                ]
            })
            .collect();
        let n = points.len();
        let mut ok = false;
        for _ in 0..10_000 {
            let mut moved = false;
            let mut crowded = 0;
            for i in 0..n {
                let mut push = [0.0, 0.0];
                let mut near = 0;
                for j in 0..n {
                    if i == j {
                        continue;
                    }
                    let (dx, dz) = (points[j][0] - points[i][0], points[j][1] - points[i][1]);
                    let d = (dx * dx + dz * dz).sqrt();
                    if d < spread {
                        near += 1;
                        if d > 1.0e-6 {
                            push[0] -= dx / d;
                            push[1] -= dz / d;
                        } else {
                            let r = &mut self.world.random;
                            push[0] += r.next_double() - 0.5;
                            push[1] += r.next_double() - 0.5;
                        }
                    }
                }
                if near > 0 {
                    crowded += 1;
                    let len = (push[0] * push[0] + push[1] * push[1]).sqrt().max(1.0e-6);
                    points[i][0] += push[0] / len;
                    points[i][1] += push[1] / len;
                    moved = true;
                }
                let before = points[i];
                points[i][0] = points[i][0].clamp(min_x, max_x);
                points[i][1] = points[i][1].clamp(min_z, max_z);
                moved |= before != points[i];
            }
            if !moved || crowded == 0 {
                ok = true;
                break;
            }
        }
        if !ok {
            let key = if teams.is_empty() {
                "commands.spreadplayers.failed.entities"
            } else {
                "commands.spreadplayers.failed.teams"
            };
            return Err(err(
                key,
                &[
                    &n,
                    &format!("{:.2}", centre[0]),
                    &format!("{:.2}", centre[2]),
                    &format!("{:.2}", spread),
                ],
            ));
        }
        // Stand each unit on the ground under `max_y`.
        let mut total = 0.0;
        for (unit, p) in units.iter().zip(&points) {
            let (bx, bz) = (p[0].floor() as i32, p[1].floor() as i32);
            let mut y = None;
            for yy in (horizon_client::chunk::MIN_Y
                ..max_y.min(horizon_client::chunk::MIN_Y + horizon_client::chunk::HEIGHT))
                .rev()
            {
                let solid = |dy: i32| {
                    self.res
                        .facts
                        .get(self.world.block(bx, yy + dy, bz))
                        .full_collision
                };
                if solid(-1) && !solid(0) && !solid(1) {
                    y = Some(yy);
                    break;
                }
            }
            let y = y.unwrap_or(max_y);
            for &t in unit {
                self.teleport_to(
                    t,
                    [f64::from(bx) + 0.5, f64::from(y), f64::from(bz) + 0.5],
                    None,
                );
            }
            let (dx, dz) = (p[0] - centre[0], p[1] - centre[2]);
            total += (dx * dx + dz * dz).sqrt();
        }
        let average = if n == 0 { 0.0 } else { total / n as f64 };
        let key = if teams.is_empty() {
            "commands.spreadplayers.success.entities"
        } else {
            "commands.spreadplayers.success.teams"
        };
        self.feedback(Text::translate(
            key,
            &[
                num(n),
                Text::literal(format!("{:.2}", centre[0])),
                Text::literal(format!("{:.2}", centre[2])),
                Text::literal(format!("{average:.2}")),
            ],
        ));
        Ok(n as i32)
    }
}

/// A `CommandError` whose message is a translation with component arguments.
fn text_error(key: &str, args: &[Text]) -> horizon_client::command::CommandError {
    horizon_client::command::CommandError::new(Text::translate(key, args))
}

/// `Entity.lookAt`: yaw and pitch from `from` towards `to`.
fn look_at(from: [f64; 3], to: [f64; 3]) -> [f32; 2] {
    let (dx, dy, dz) = (to[0] - from[0], to[1] - from[1], to[2] - from[2]);
    let horizontal = (dx * dx + dz * dz).sqrt();
    let pitch = (-(dy.atan2(horizontal).to_degrees())) as f32;
    let yaw = (dz.atan2(dx).to_degrees() - 90.0) as f32;
    [yaw.rem_euclid(360.0), pitch]
}
