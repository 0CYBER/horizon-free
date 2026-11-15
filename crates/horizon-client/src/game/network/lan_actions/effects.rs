use super::*;

impl Game<'_> {
    /// `level.playSound(player, ...)` of a guest's use it predicted (`remote::hand_operated`
    /// blocks only, as the guest decides): the others hear it now, the guest already did.
    pub(super) fn guest_use_sounds(&mut self, conn: u32, logged: usize, heard: usize) {
        if self.lan.is_none() {
            return;
        }
        let changed: Vec<(i32, i32, i32)> = self
            .world
            .block_log
            .as_ref()
            .map_or(&[][..], |log| log.get(logged..).unwrap_or(&[]))
            .to_vec();
        let facts = self.res.facts();
        // BlockItem.place: `playSound(player, ...)`, for the others only. The
        // placer's client plays it as its block appears. The place sound is the
        // one the block's parts answered the placement with (`place_sound_type`).
        let placer = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .map(|g| (g.entity_id as u32, "minecraft:player"));
        let placed: Vec<([f64; 3], String)> = changed
            .iter()
            .map(|&(x, y, z)| {
                let at = [f64::from(x) + 0.5, f64::from(y) + 0.5, f64::from(z) + 0.5];
                let state = self.world.block(x, y, z);
                let place = mod_api::BlockSoundCause::Place;
                let name = self.block_sound_type([x, y, z], state, placer, place).place;
                (at, name.trim_start_matches("minecraft:").to_owned())
            })
            .collect();
        let mut sounds = Vec::new();
        if let Some(capture) = self.audio.capture.as_mut() {
            let mut i = heard.min(capture.len());
            while i < capture.len() {
                let (name, at) = (&capture[i].event, &capture[i].position);
                let place = placed
                    .iter()
                    .any(|(p, n)| p == at && name.trim_start_matches("minecraft:") == n.as_str());
                if place {
                    sounds.push(capture.remove(i));
                } else {
                    i += 1;
                }
            }
        }
        let hand_operated = !changed.is_empty()
            && changed.iter().all(|&(x, y, z)| {
                crate::game::remote::hand_operated(&facts.get(self.world.block(x, y, z)).name)
            });
        if hand_operated && let Some(capture) = self.audio.capture.as_mut() {
            sounds.extend(capture.drain(heard.min(capture.len())..));
        }
        for sound in sounds {
            self.send_captured_level_sound(sound, Some(conn));
        }
    }

    /// `InstrumentItem.play`'s `level.playSound(player, ...)`: the others hear
    /// a guest's horn now, the guest's own client played it as it used it.
    /// The horn is bound to the guest (`playSound(player, player, ...)`).
    pub(super) fn guest_instrument_sound(&mut self, conn: u32, heard: usize, horn: Option<String>) {
        let Some(player) = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .and_then(|g| u32::try_from(g.entity_id).ok())
        else {
            return;
        };
        let (Some(horn), Some(capture)) = (horn, self.audio.capture.as_mut()) else {
            return;
        };
        let mut sounds = Vec::new();
        let mut i = heard.min(capture.len());
        while i < capture.len() {
            if capture[i].event == horn {
                sounds.push(capture.remove(i));
            } else {
                i += 1;
            }
        }
        for mut captured in sounds {
            captured.entity = Some(player);
            self.send_captured_level_sound(captured, Some(conn));
        }
    }

    /// A level sound's old emitter must still be known at playback. Keep
    /// sounds needing a new pairing queued, with their later removals held.
    pub(in crate::game) fn lan_prepare_captured_sounds(
        &mut self,
        entities: &[horizon_client::lan::entity::EntityState],
    ) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let Some(sounds) = self.audio.capture.as_mut() else {
            return;
        };
        let blocks: Vec<_> = host
            .broken
            .iter()
            .copied()
            .chain(
                self.lan_events
                    .iter()
                    .filter(|event| event.kind == 2001)
                    .map(|event| event.pos.map(f64::from)),
            )
            .collect();
        sounds.retain(|sound| !captured_block_break(sound, &blocks));
        host.hub.prepare_captured_sounds(sounds, entities);
    }

    /// Positional sounds and level events of the tick, for the guests.
    pub(in crate::game) fn lan_broadcast_effects(&mut self) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let sounds = self.audio.capture.take().unwrap_or_default();
        self.audio.capture = Some(Vec::new());
        let blocks: Vec<[f64; 3]> = std::mem::take(&mut host.broken);
        for sound in sounds {
            // Block breaks are heard through the level event.
            if captured_block_break(&sound, &blocks) {
                continue;
            }
            self.send_captured_level_sound(sound, None);
        }
        self.send_bound_sounds();
        if let Some(host) = self.lan.as_ref() {
            host.hub.finish_captured_sounds();
        }
    }

    pub(super) fn send_captured_level_sound(
        &mut self,
        mut captured: horizon_client::audio::CapturedSound,
        except: Option<u32>,
    ) {
        let source = mod_api::SoundSource::ALL
            .get(captured.source)
            .copied()
            .unwrap_or(mod_api::SoundSource::Master);
        let sound = LevelSound {
            name: &captured.event,
            at: captured.position,
            volume: captured.volume,
            pitch: captured.pitch,
            source,
            emitter: captured.entity,
        };
        let Some(heard) = self.ask_level_sound(self.dim.id(), sound) else {
            return;
        };
        let name = heard.name.into_owned();
        captured.source = heard.source as usize;
        captured.volume = heard.volume;
        captured.pitch = heard.pitch;
        captured.event = name;
        if let Some(host) = self.lan.as_ref() {
            host.hub.captured_sound(&captured, except);
        }
    }

    /// `LivingEntity.tick`'s arrows and bee stingers falling out of the
    /// players' bodies, and their counts for the players around
    /// (`DATA_ARROW_COUNT_ID`, `DATA_STINGER_COUNT_ID`).
    pub(in crate::game) fn lan_stuck(&mut self, paused: bool) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        if !paused {
            self.combat.stuck.tick();
        }
        host.hub.guests.update_all(|g| {
            g.shoulders = crate::game::shoulders::shoulder_variants(g.saved.as_ref());
            g.stuck = match self.guest_combat.get_mut(&g.conn) {
                Some(combat) => {
                    if !paused {
                        combat.stuck.tick();
                    }
                    (combat.stuck.arrows, combat.stuck.stingers)
                }
                None => (0, 0),
            };
        });
    }

    /// The guests' items in use and Riptide spins, for the players around
    /// (`LivingEntity.startUsingItem`, `stopUsingItem` and `startAutoSpinAttack`
    /// set DATA_LIVING_ENTITY_FLAGS).
    pub(in crate::game) fn lan_living_flags(&mut self) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        host.hub.guests.update_all(|g| {
            let using = host.play.get(&g.conn).and_then(GuestPlay::using);
            let spinning = g
                .motion
                .as_ref()
                .is_some_and(|m| m.auto_spin_attack_ticks > 0);
            g.living_flags = crate::game::lan::living_flags(using, spinning);
        });
    }

    /// A use the server ran swung the guest's hand: their own client is told
    /// too (`swing(hand, true)` for a SwingSource.SERVER result). Horizon's
    /// client does not run uses itself, so it does not predict the swing; a
    /// client that did (a block placed) is still in the first half of its
    /// swing and ignores it (`SwingState.startIfAble`).
    pub(super) fn lan_swing_back(&mut self, conn: u32, swung: Option<bool>) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        if swung != Some(true) {
            return;
        }
        let (Some(g), Some(play)) = (host.hub.guest(conn), host.play.get(&conn)) else {
            return;
        };
        if let Some(c) = play.swing.current {
            host.hub
                .send_to(conn, &swing_packet(g.entity_id, c.hand, c.animation));
        }
    }

    /// The tick's arm swings for the players around (each guest's to everyone
    /// else), and `LivingEntity.updateSwingTime` for the guests' bodies here.
    pub(in crate::game) fn lan_swings(&mut self) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let conns: Vec<u32> = host.play.keys().copied().collect();
        for conn in conns {
            let Some(g) = host.hub.guests.iter().find(|g| g.conn == conn) else {
                continue;
            };
            let (id, at, here) = (g.entity_id, g.position, host.hub.here(g));
            let play = host.play.get_mut(&conn).expect("play");
            let swung = std::mem::take(&mut play.swings) > 0;
            if let (true, true, Some(c)) = (swung, here, play.swing.current) {
                let packet = swing_packet(id, c.hand, c.animation);
                host.hub.broadcast_near(at, 128.0, &packet, Some(conn));
            }
            play.swing.tick();
        }
    }
}
