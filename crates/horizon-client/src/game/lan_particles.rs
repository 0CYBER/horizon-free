//! Particles a server sends its players and its clients draw:
//! `ClientboundLevelParticlesPacket` (`ServerLevel.sendParticles`, which
//! `/particle` uses) and `ClientboundExplodePacket` (`ServerLevel.explode`).
use super::Game;
use horizon_client::particles::{ParticleOptions, ParticleType};
use horizon_core::{geometry::Vec3, random::RandomSource};
use horizon_protocol::{
    entity_data::{Particle, ParticleData},
    packets::play::{self, Clientbound as C},
};
use horizon_world::registry::BlockStateId;

/// An ARGB colour as the options' RGBA.
fn rgba(argb: i32) -> [f32; 4] {
    let c = |shift: i32| ((argb >> shift) & 255) as f32 / 255.0;
    [c(16), c(8), c(0), c(24)]
}
/// The options' RGB as an RGB colour (`ARGB.colorFromFloat` without alpha).
fn rgb_int(c: [f32; 3]) -> i32 {
    let b = |v: f32| ((v.clamp(0.0, 1.0) * 255.0).round() as i32) & 255;
    (b(c[0]) << 16) | (b(c[1]) << 8) | b(c[2])
}
fn rgb(c: [f32; 4]) -> [f32; 3] {
    [c[0], c[1], c[2]]
}

impl Game<'_> {
    /// `ParticleTypes.STREAM_CODEC`'s value for `options`: the type's
    /// network id and the data it carries (`state` for the block particles).
    /// None while not serving, or for a particle whose data isn't sent from
    /// here (items, vibrations, trails).
    pub(super) fn network_particle(
        &self,
        options: &ParticleOptions,
        state: Option<BlockStateId>,
    ) -> Option<Particle> {
        let name = format!("minecraft:{}", options.ty.name());
        let id = self
            .lan
            .as_ref()?
            .hub
            .registry_id("minecraft:particle_type", &name)?;
        use ParticleType as T;
        let c = options.color;
        let data = match options.ty {
            T::Block | T::BlockMarker | T::FallingDust | T::DustPillar | T::BlockCrumble => {
                Some(ParticleData::State(state?.0 as i32))
            }
            T::Dust => Some(ParticleData::ColorScale(rgb_int(rgb(c)), options.scale)),
            T::DustColorTransition => Some(ParticleData::ColorScale2(
                rgb_int(rgb(c)),
                rgb_int(options.to_color),
                options.scale,
            )),
            T::Effect | T::InstantEffect => {
                Some(ParticleData::ColorPower(rgb_int(rgb(c)), options.scale))
            }
            T::EntityEffect | T::TintedLeaves => Some(ParticleData::Color(
                (((c[3].clamp(0.0, 1.0) * 255.0).round() as i32) << 24) | rgb_int(rgb(c)),
            )),
            T::DragonBreath | T::SculkCharge => Some(ParticleData::Scalar(options.scale)),
            T::Shriek => Some(ParticleData::Delay(options.scale as i32)),
            T::Item | T::Trail | T::Vibration => return None,
            _ => None,
        };
        Some(Particle { id, data })
    }

    /// The options a server's particle stands for (None for a type or data
    /// this client doesn't draw).
    fn received_particle(&mut self, p: &Particle, at: [f64; 3]) -> Option<ParticleOptions> {
        let name = self
            .remote
            .as_ref()?
            .builtin
            .get("minecraft:particle_type")?
            .name(p.id)?
            .to_owned();
        let ty = ParticleType::from_name(&name)?;
        Some(match &p.data {
            None => ParticleOptions::simple(ty),
            Some(ParticleData::State(s)) => {
                let pos = at.map(|v| v.floor() as i32);
                let (sprite, tint) = self
                    .particle_sprite(BlockStateId(*s as u32), pos)
                    .unwrap_or_else(|| ("block/stone".into(), 0xFFFFFF));
                ParticleOptions::block(ty, sprite, tint)
            }
            Some(ParticleData::ColorScale(c, scale)) => {
                ParticleOptions::dust(rgb(rgba(*c)), *scale)
            }
            Some(ParticleData::ColorScale2(from, to, scale)) => {
                ParticleOptions::dust_transition(rgb(rgba(*from)), rgb(rgba(*to)), *scale)
            }
            Some(ParticleData::ColorPower(c, power)) => ParticleOptions {
                scale: *power,
                ..ParticleOptions::colored(ty, [rgba(*c)[0], rgba(*c)[1], rgba(*c)[2], 1.0])
            },
            Some(ParticleData::Color(c)) => ParticleOptions::colored(ty, rgba(*c)),
            Some(ParticleData::Scalar(v)) if ty == ParticleType::SculkCharge => {
                ParticleOptions::sculk_charge(*v)
            }
            Some(ParticleData::Scalar(v)) => ParticleOptions {
                scale: *v,
                ..ParticleOptions::simple(ty)
            },
            Some(ParticleData::Delay(d)) => ParticleOptions::shriek(*d),
            Some(ParticleData::Trail(target, color, duration)) => {
                ParticleOptions::trail(*target, *color as u32 & 0xFF_FFFF, *duration)
            }
            Some(_) => return None,
        })
    }

    /// `ClientPacketListener.handleParticleEvent`: no count is one particle
    /// moving by speed times spread; otherwise each is spread and sped up by
    /// gaussians on each axis.
    pub(super) fn remote_level_particles(&mut self, p: &play::LevelParticles) {
        let Some(options) = self.received_particle(&p.particle, p.pos) else {
            return;
        };
        let (d, s) = (p.dist.map(f64::from), p.max_speed.map(f64::from));
        // ClientLevel.addParticle: the camera's 32 blocks unless the packet
        // (`force`) or the type overrides the limiter.
        let add = |particles: &mut horizon_client::particles::ParticleEngine, at, v| {
            if p.override_limiter {
                particles.create(&options, at, v);
            } else {
                particles.add(&options, at, v);
            }
        };
        if p.count == 0 {
            add(
                &mut self.particles,
                p.pos,
                [s[0] * d[0], s[1] * d[1], s[2] * d[2]],
            );
            return;
        }
        for _ in 0..p.count {
            let r = self.particles.random();
            let at = [
                p.pos[0] + r.next_gaussian() * d[0],
                p.pos[1] + r.next_gaussian() * d[1],
                p.pos[2] + r.next_gaussian() * d[2],
            ];
            let v = [
                r.next_gaussian() * s[0],
                r.next_gaussian() * s[1],
                r.next_gaussian() * s[2],
            ];
            add(&mut self.particles, at, v);
        }
    }

    /// `ClientPacketListener.handleExplosion`: the blast's particles (and its
    /// sound, when the packet carries it), then the player's push.
    pub(super) fn remote_explode(&mut self, p: &play::Explode) {
        if p.play_sound {
            let pitch = (1.0
                + (self.particles.random().next_float() - self.particles.random().next_float())
                    * 0.2)
                * 0.7;
            // The packet's explosionSound: a wind charge's burst, a bed's
            // or anchor's blast, a modded sound.
            let sound = self
                .remote_sound_name(&p.sound)
                .unwrap_or_else(|| "minecraft:entity.generic.explode".to_owned());
            self.audio.play(&sound, Some(p.center), 4.0, pitch);
        }
        if let Some(options) = self.received_particle(&p.particle, p.center) {
            self.particles.add(&options, p.center, [1.0, 0.0, 0.0]);
        }
        // A wind charge's burst throws none.
        if !p.block_particles.is_empty() {
            let (world, facts) = (&self.world, self.res.facts());
            self.particles.explosion_blocks(
                p.center,
                p.radius,
                p.block_count.max(0) as usize,
                &|b| facts.is_air(world.block(b[0], b[1], b[2])),
            );
        }
        // Entity.pushFromExplosion.
        if let (Some(push), Some(player)) = (p.knockback, self.player.as_mut()) {
            player.velocity = Vec3::new(
                player.velocity.x + push[0],
                player.velocity.y + push[1],
                player.velocity.z + push[2],
            );
        }
    }

    /// `ServerLevel.explode`'s packet to each player within 64 blocks. The
    /// sound and the push reach them on their own (the level's sounds, the
    /// motion a hurt player is sent), so the packet carries neither.
    pub(super) fn lan_explode(&self, center: [f64; 3], radius: f32, block_count: usize) {
        let emitter = if radius < 2.0 {
            "minecraft:explosion"
        } else {
            "minecraft:explosion_emitter"
        };
        let sound = "minecraft:entity.generic.explode";
        if let Some(packet) =
            self.explosion_packet(center, radius, block_count, emitter, sound, true)
        {
            self.send_explosion(packet, &[]);
        }
    }

    /// A wind charge's burst (`AbstractWindCharge.explode`): gust emitters,
    /// no block particles, and for each player it pushed the push
    /// (`ServerExplosion.hitPlayers`), which only that client applies, as
    /// nothing hurt them.
    pub(super) fn lan_wind_burst(
        &self,
        center: [f64; 3],
        radius: f32,
        sound: &str,
        block_count: usize,
        pushed: &[(u32, [f64; 3])],
    ) {
        let emitter = if radius < 2.0 {
            "minecraft:gust_emitter_small"
        } else {
            "minecraft:gust_emitter_large"
        };
        if let Some(packet) =
            self.explosion_packet(center, radius, block_count, emitter, sound, false)
        {
            self.send_explosion(packet, pushed);
        }
    }

    /// `ClientboundExplodePacket` without a push, or None while not serving.
    /// The sound is the level's, which the players hear already, so it isn't
    /// played from the packet.
    fn explosion_packet(
        &self,
        center: [f64; 3],
        radius: f32,
        block_count: usize,
        emitter: &str,
        sound: &str,
        block_particles: bool,
    ) -> Option<play::Explode> {
        let hub = &self.lan.as_ref()?.hub;
        let particle = |name: &str| {
            Some(Particle {
                id: hub.registry_id("minecraft:particle_type", name)?,
                data: None,
            })
        };
        // Level.DEFAULT_EXPLOSION_BLOCK_PARTICLES.
        let info = |particle, scaling| play::ExplosionParticle {
            particle,
            scaling,
            speed: 1.0,
            weight: 1,
        };
        let block_particles = if block_particles {
            vec![
                info(particle("minecraft:poof")?, 0.5),
                info(particle("minecraft:smoke")?, 1.0),
            ]
        } else {
            Vec::new()
        };
        Some(play::Explode {
            center,
            radius,
            block_count: block_count as i32,
            knockback: None,
            particle: particle(emitter)?,
            sound: match hub.registry_id("minecraft:sound_event", sound) {
                Some(id) => (id, None),
                None => (-1, Some((sound.into(), None))),
            },
            block_particles,
            play_sound: false,
        })
    }

    /// `packet` to each player within 64 blocks of its centre, with the push
    /// `pushed` holds for them.
    fn send_explosion(&self, mut packet: play::Explode, pushed: &[(u32, [f64; 3])]) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let center = packet.center;
        let plain = C::Explode(packet.clone()).encode();
        for g in host.hub.guests.iter().filter(|g| host.hub.here(g)) {
            let d: f64 = (0..3).map(|k| (g.position[k] - center[k]).powi(2)).sum();
            if d >= 4096.0 {
                continue;
            }
            match pushed.iter().find(|(conn, _)| *conn == g.conn) {
                Some((_, push)) => {
                    packet.knockback = Some(*push);
                    g.send(&C::Explode(packet.clone()));
                }
                None => g.sender.send(plain.clone()),
            }
        }
    }
}
