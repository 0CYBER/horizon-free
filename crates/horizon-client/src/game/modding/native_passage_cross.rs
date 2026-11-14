//! Crossing a passage within a level (#3775, docs/modding/src/how-to/passages.md): the client
//! moves its own player through (`ClientHost::cross_passage`) and tells its server the passage
//! and where it crossed from; the server checks that move as any move (the hub,
//! `lan/hub/passage_cross.rs`), finds the passage, asks the mods (`player_passage_cross`),
//! works out where the player comes out itself and follows with a position packet that
//! changes nothing, or puts the player back.
use super::Game;
use horizon_client::lan::hub::passage_cross::{REACH, crossing_source, refusal_named};
use horizon_core::geometry::Vec3 as Motion;
use horizon_protocol::packets::{configuration::CustomPayload, play::Serverbound as S};
use horizon_protocol::passage_sync::{CROSS_CHANNEL, Cross, decode_cross, encode_cross};
use mod_api::{
    Passage, PassageCrossError as Error, PassageCrossRefusal as Refusal, PassageId, PassageTurn,
    Vec3,
};

/// Where `point` comes out of `p`: as far from the destination, turned, as it is from the
/// opening's centre (Immersive Portals' `Portal.transformPoint`).
fn carried(p: &Passage, point: [f64; 3]) -> [f64; 3] {
    let o = p.origin;
    let d = p.turned(Vec3::new(point[0] - o.x, point[1] - o.y, point[2] - o.z));
    [
        p.destination.x + d.x,
        p.destination.y + d.y,
        p.destination.z + d.z,
    ]
}

/// `v` turned with `p`, as a velocity is (`TeleportationUtil.transformEntityVelocity`).
fn turned(p: &Passage, [x, y, z]: [f64; 3]) -> [f64; 3] {
    let v = p.turned(Vec3::new(x, y, z));
    [v.x, v.y, v.z]
}

/// The yaw `p` adds, in degrees: a clockwise quarter turn takes a look south (0) to west (90).
fn yaw_turn(p: &Passage) -> f32 {
    match p.turn {
        PassageTurn::Clockwise90 => 90.0,
        PassageTurn::Clockwise180 => 180.0,
        PassageTurn::Counterclockwise90 => -90.0,
        _ => 0.0,
    }
}

/// Whether `point` is within [`REACH`] of the nearest point of `p`'s opening (Immersive
/// Portals' `getDistanceToNearestPointInPortal`).
fn within_reach(p: &Passage, point: [f64; 3]) -> bool {
    let o = p.origin;
    let d = [point[0] - o.x, point[1] - o.y, point[2] - o.z];
    let dot = |a: Vec3| d[0] * a.x + d[1] * a.y + d[2] * a.z;
    let w = dot(p.axis_w).clamp(-p.width / 2.0, p.width / 2.0);
    let h = dot(p.axis_h).clamp(-p.height / 2.0, p.height / 2.0);
    let nearest = [0, 1, 2].map(|k| {
        let (aw, ah) = (
            [p.axis_w.x, p.axis_w.y, p.axis_w.z],
            [p.axis_h.x, p.axis_h.y, p.axis_h.z],
        );
        d[k] - aw[k] * w - ah[k] * h
    });
    let reach = f64::from(REACH);
    nearest.iter().map(|v| v * v).sum::<f64>() <= reach * reach
}

/// The chunk holding `point`.
fn chunk_of(point: [f64; 3]) -> (i32, i32) {
    (
        (point[0].floor() as i32) >> 4,
        (point[2].floor() as i32) >> 4,
    )
}

impl Game<'_> {
    /// `ClientHost::cross_passage`: the window's player comes out of `passage` with its
    /// feet, its last tick's place, its velocity and its look carried through, and the
    /// server is told the passage and where the feet were; a payload a mod cancels moves
    /// nothing. Until the server's position packet, the next crossing waits.
    pub(super) fn cross_passage(&mut self, passage: PassageId) -> Result<(), Error> {
        if self.remote.is_none() {
            return Err(Error::Unavailable);
        }
        if self.player.is_none() {
            return Err(Error::NoPlayer);
        }
        if self.passages.crossing.is_some() {
            return Err(Error::Pending);
        }
        if self.riding.is_some() {
            return Err(Error::Riding);
        }
        let p = *self
            .passages
            .held
            .get(&passage)
            .ok_or(Error::UnknownPassage)?;
        let Some(player) = self.player.as_ref() else {
            return Err(Error::NoPlayer);
        };
        let from = [player.position.x, player.position.y, player.position.z];
        if !within_reach(&p, from) {
            return Err(Error::TooFar { limit: REACH });
        }
        // Sent first: a mod's `client_packet_send` may cancel it, and then nothing moves.
        let data = encode_cross(&Cross {
            id: passage.raw(),
            from,
        });
        let payload = S::CustomPayload(CustomPayload {
            channel: CROSS_CHANNEL.into(),
            data,
        });
        if !self.send_play(payload) {
            return Err(Error::Cancelled);
        }
        // The send hook's listeners only read the host: the player is still there.
        let player = self.player.as_mut().expect("checked above");
        let [x, y, z] = carried(&p, from);
        player.position = Motion::new(x, y, z);
        let v = player.velocity;
        let [vx, vy, vz] = turned(&p, [v.x, v.y, v.z]);
        player.velocity = Motion::new(vx, vy, vz);
        let o = self.previous_position;
        let [ox, oy, oz] = carried(&p, [o.x, o.y, o.z]);
        self.previous_position = Motion::new(ox, oy, oz);
        let turn = yaw_turn(&p);
        self.yaw += turn;
        self.rope_rot_old.0 += turn;
        self.passages.crossing = Some(passage);
        self.passages.refused = None;
        Ok(())
    }

    /// `ClientHost::last_passage_refusal`.
    pub(super) fn last_passage_refusal(&self) -> Option<(PassageId, Refusal)> {
        self.passages.refused
    }

    /// The server refused the window's crossing of passage `id`: the reason is kept for
    /// the mods, and the position packet that follows puts the player back.
    pub(super) fn remote_passage_refused(&mut self, id: u64, reason: u8) {
        let Some(why) = refusal_named(reason) else {
            log::debug!("Horizon: a passage refusal this build does not know: {reason}");
            return;
        };
        self.passages.refused = Some((PassageId::from_raw(id), why));
    }

    /// A guest's crossing that passed the hub's move check (`Hub::passage_leg`), decided
    /// from the `from` it names (`decide_passage_cross`): the hub then follows to where the
    /// server works out the player comes out. Otherwise refused by name, and the player is
    /// put back where the server has it.
    pub(in crate::game) fn guest_passage_cross(&mut self, conn: u32, data: &[u8]) {
        let Ok(cross) = decode_cross(data) else {
            return;
        };
        let id = PassageId::from_raw(cross.id);
        let Some(gi) = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guests.iter().position(|g| g.conn == conn))
        else {
            return;
        };
        let from = crossing_source(cross.from);
        let decided = self.decide_passage_cross(gi, id, from);
        let host = self.lan.as_mut().expect("the guest was found in the hub");
        match decided {
            Ok((p, to)) => host
                .hub
                .follow_passage(gi, to, yaw_turn(&p), |v| turned(&p, v)),
            Err(why) => host.hub.refuse_passage(gi, cross.id, why),
        }
    }

    /// Guest `gi`'s crossing of `id` from `from`: the whole check, the veto, the whole
    /// check again (a listener that removed the passage, moved the player or made it
    /// ineligible refuses the crossing), and only then where the player comes out, whose
    /// chunk must be loaded and tracked too (ruling 10 on #3775: the server computes `to`
    /// after the veto).
    fn decide_passage_cross(
        &mut self,
        gi: usize,
        id: PassageId,
        from: [f64; 3],
    ) -> Result<(Passage, [f64; 3]), Refusal> {
        self.check_passage_cross(gi, id, from)?;
        let host = self.lan.as_ref().expect("checked");
        let g = &host.hub.guests[gi];
        let epoch = self
            .native_lifecycle
            .epoch()
            .ok_or(Refusal::UnknownPassage)?;
        let player = mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(epoch, g.entity_id as u32),
            g.uuid,
        );
        let mut cancel = false;
        let at = Vec3::new(from[0], from[1], from[2]);
        self.native_player_passage_cross(player, id, at, &mut cancel);
        if cancel {
            return Err(Refusal::Vetoed);
        }
        let p = self.check_passage_cross(gi, id, from)?;
        let to = carried(&p, from);
        if !self.guest_holds_chunk(gi, chunk_of(to)) {
            return Err(Refusal::FarEndNotTracked);
        }
        Ok((p, to))
    }

    /// Whether guest `gi` may cross `id` from `from` before the destination is worked out:
    /// eligible as the hub's leg checks it (`Hub::passage_refusal`), at `from` on the
    /// server, the passage its level's, `from` within reach, and the opening's chunk and
    /// the destination's loaded and tracked for the guest. The passage, or why not.
    fn check_passage_cross(
        &self,
        gi: usize,
        id: PassageId,
        from: [f64; 3],
    ) -> Result<Passage, Refusal> {
        let host = self.lan.as_ref().ok_or(Refusal::UnknownPassage)?;
        let g = &host.hub.guests[gi];
        if let Some(why) = host.hub.passage_refusal(gi) {
            return Err(why);
        }
        if g.position != from {
            return Err(Refusal::MoveRefused);
        }
        let level = self.level(&g.dimension).ok_or(Refusal::UnknownPassage)?;
        let p = self
            .passages
            .find(level.dimension(), id)
            .ok_or(Refusal::UnknownPassage)?;
        if p.destination_level.is_some() {
            return Err(Refusal::FarLevel);
        }
        if !within_reach(&p, from) {
            return Err(Refusal::TooFar { limit: REACH });
        }
        let [cx, cz] = p.chunk();
        if !self.guest_holds_chunk(gi, (cx, cz)) {
            return Err(Refusal::NearEndNotTracked);
        }
        let d = p.destination;
        if !self.guest_holds_chunk(gi, chunk_of([d.x, d.y, d.z])) {
            return Err(Refusal::FarEndNotTracked);
        }
        Ok(p)
    }

    /// Whether the server holds `chunk` in guest `gi`'s level and the guest tracks it.
    fn guest_holds_chunk(&self, gi: usize, chunk: (i32, i32)) -> bool {
        let Some(host) = self.lan.as_ref() else {
            return false;
        };
        let g = &host.hub.guests[gi];
        g.tracks_chunk(&g.dimension, chunk)
            && g.dimension == self.dim.id()
            && self.world.chunks.contains_key(&chunk)
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_guest_turn_tests::with_guests;
    use super::super::native_passages::tests::{doorway, listen, packets, passage_messages};
    use super::{carried, turned, within_reach, yaw_turn};
    use crate::game::Dim;
    use horizon_client::chunk::Chunk;
    use horizon_client::lan::guest::InboxItem;
    use horizon_client::lan::hub::passage_cross::refusal_code;
    use horizon_protocol::packets::configuration::CustomPayload;
    use horizon_protocol::packets::play::{self, Clientbound as C, Serverbound as S, relative};
    use horizon_protocol::passage_sync::{CROSS_CHANNEL, Cross, Message, encode_cross};
    use mod_api::{DimensionKey, Passage, PassageCrossRefusal as Refusal, PassageId, Vec3};
    use std::sync::{Arc, mpsc};

    /// The doorway comes out 24 blocks east, turned a clockwise quarter.
    fn near_doorway() -> Passage {
        Passage {
            destination: Vec3::new(24.5, 65.5, 8.5),
            ..doorway()
        }
    }

    /// A point and a velocity come out turned about the far side's centre; the look turns
    /// a quarter; reach is 20 blocks from the nearest point of the opening, past its edges too.
    #[test]
    fn a_crossing_carries_the_feet_the_motion_and_the_look() {
        let p = near_doorway();
        assert_eq!(carried(&p, [0.5, 65.0, 8.25]), [24.75, 65.0, 8.5]);
        assert_eq!(carried(&p, [1.5, 66.0, 9.0]), [24.0, 66.0, 9.5]);
        assert_eq!(turned(&p, [0.0, -0.0784, 0.25]), [-0.25, -0.0784, 0.0]);
        assert_eq!(yaw_turn(&p), 90.0);
        // The opening runs x -0.5..1.5, y 64..67 in the plane z = 8.5.
        assert!(within_reach(&p, [0.5, 65.0, 28.5]));
        assert!(!within_reach(&p, [0.5, 65.0, 28.51]));
        assert!(within_reach(&p, [21.5, 65.5, 8.5]));
        assert!(!within_reach(&p, [21.6, 65.5, 8.5]));
        assert!(!within_reach(&p, [13.5, 79.0, 20.5]));
    }

    /// Guests 12 and 13 on Horizon clients on a stone floor (y 64) over chunks x 0 to 1,
    /// tracking them; 12 stands loaded at `at`. The doorway is passage 1.
    fn scene(
        game: &mut crate::game::Game<'_>,
        at: [f64; 3],
    ) -> mpsc::Receiver<horizon_client::lan::Out> {
        game.native_lifecycle.world_open_ready().unwrap();
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap()
            .0;
        for cx in 0..=1 {
            let mut chunk = Chunk::empty(cx, 0);
            for x in 0..16 {
                for z in 0..16 {
                    chunk.set(x, 64, z, stone);
                }
            }
            game.world.chunks.insert((cx, 0), Arc::new(chunk));
        }
        let dimension = game.dim.id().to_owned();
        let mut receivers = listen(game, ["horizon", "horizon"]);
        game.lan.as_mut().unwrap().hub.guests.update_all(|g| {
            g.dimension = dimension.clone();
            g.loaded = true;
            g.load_timeout = 0;
            g.center = Some((0, 0));
            g.sent_chunks.extend([(0, 0), (1, 0)]);
            g.position = at;
            g.previous_position = at;
            g.first_good = at;
            g.on_ground = true;
        });
        let level = game.level(game.dim.id()).unwrap();
        let id = game.level_register_passage(level, near_doorway()).unwrap();
        assert_eq!(id, PassageId::from_raw(1));
        game.tick_lan(false);
        receivers.truncate(1);
        let rx = receivers.pop().unwrap();
        packets(&rx);
        rx
    }

    /// Guest 12's next tick: `inbox`, then the rest of the server's tick.
    fn tick(game: &mut crate::game::Game<'_>, inbox: Vec<S>) {
        game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            g.inbox
                .extend(inbox.into_iter().map(|p| InboxItem::Packet(p, None)));
        });
        game.tick_lan(false);
    }

    fn cross(id: u64, from: [f64; 3]) -> S {
        S::CustomPayload(CustomPayload {
            channel: CROSS_CHANNEL.into(),
            data: encode_cross(&Cross { id, from }),
        })
    }

    fn position_packets(sent: &[C]) -> Vec<play::PlayerPosition> {
        sent.iter()
            .filter_map(|p| match p {
                C::PlayerPosition(p) => Some(p.clone()),
                _ => None,
            })
            .collect()
    }

    /// The refusals among `sent` (the passages sent with chunks are left out).
    fn refusals(sent: &[C]) -> Vec<Message> {
        passage_messages(sent)
            .into_iter()
            .filter(|m| matches!(m, Message::Refused { .. }))
            .collect()
    }

    fn guest<'a>(game: &'a crate::game::Game<'_>) -> &'a horizon_client::lan::guest::Guest {
        &game.lan.as_ref().unwrap().hub.guests[0]
    }

    const ALL: i32 = relative::X
        | relative::Y
        | relative::Z
        | relative::Y_ROT
        | relative::X_ROT
        | relative::DELTA_X
        | relative::DELTA_Y
        | relative::DELTA_Z
        | relative::ROTATE_DELTA;

    /// The server follows a guest that crossed: it has the player where the passage leads,
    /// turned, and answers with one position packet that is all relative and zero. The
    /// confirmation, from a step further on, and the next move are accepted as they are:
    /// no correction follows. A passage that leads into the air: the confirmation, checked
    /// off the ground, and the fall after it pass too, and the player is not floating.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn the_server_follows_a_crossing_without_a_snap() {
        for (destination_y, steps) in [(65.5, [65.0, 65.0]), (75.5, [74.9216, 74.6862])] {
            with_guests(|game| {
                let rx = scene(game, [0.5, 65.0, 8.25]);
                if destination_y != 65.5 {
                    let level = game.level(game.dim.id()).unwrap();
                    assert!(game.level_remove_passage(level, PassageId::from_raw(1)));
                    let high = Passage {
                        destination: Vec3::new(24.5, destination_y, 8.5),
                        ..near_doorway()
                    };
                    assert_eq!(
                        game.level_register_passage(level, high),
                        Ok(PassageId::from_raw(2))
                    );
                }
                let id = if destination_y == 65.5 { 1 } else { 2 };
                tick(game, vec![cross(id, [0.5, 65.0, 8.25])]);
                let to = [24.75, destination_y - 0.5, 8.5];
                let g = guest(game);
                assert_eq!(g.position, to);
                assert_eq!(g.yaw, 90.0);
                let (teleport, awaited) = g.awaiting_teleport.expect("the acceptance waits");
                assert_eq!(awaited, to);
                let sent = packets(&rx);
                assert_eq!(
                    position_packets(&sent),
                    [play::PlayerPosition {
                        teleport,
                        position: [0.0; 3],
                        delta: [0.0; 3],
                        yaw: 0.0,
                        pitch: 0.0,
                        relatives: ALL,
                    }]
                );
                assert!(refusals(&sent).is_empty(), "no refusal");

                let on_floor = u8::from(destination_y == 65.5);
                tick(
                    game,
                    vec![
                        S::AcceptTeleportation(play::AcceptTeleportation {
                            id: teleport,
                            x: 24.5,
                            y: steps[0],
                            z: 8.5,
                            yaw: 90.0,
                            pitch: 0.0,
                        }),
                        S::ClientTickEnd(horizon_protocol::packets::Empty),
                    ],
                );
                tick(
                    game,
                    vec![
                        S::MovePlayerPos(play::MovePlayerPos {
                            x: 24.25,
                            y: steps[1],
                            z: 8.5,
                            flags: on_floor,
                        }),
                        S::ClientTickEnd(horizon_protocol::packets::Empty),
                    ],
                );
                let g = guest(game);
                assert_eq!(g.awaiting_teleport, None);
                assert_eq!(g.position, [24.25, steps[1], 8.5]);
                assert!(!g.client_is_floating);
                assert!(position_packets(&packets(&rx)).is_empty(), "no correction");
            });
        }
    }

    /// Two crossings in one inbox from different places, no move behind them: the first is
    /// decided from its own `from` before the hub reads the second, and is followed; the
    /// second, read the next tick while the first's acceptance waits, is refused `Pending`.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn two_crossings_in_a_row_are_decided_each_from_its_own_source() {
        with_guests(|game| {
            let rx = scene(game, [0.5, 65.0, 8.25]);
            tick(
                game,
                vec![cross(1, [0.5, 65.0, 8.25]), cross(1, [0.5, 65.0, 8.0])],
            );
            let g = guest(game);
            assert_eq!(g.position, [24.75, 65.0, 8.5]);
            let (teleport, awaited) = g.awaiting_teleport.expect("the first is followed");
            assert_eq!(awaited, [24.75, 65.0, 8.5]);
            let sent = packets(&rx);
            assert!(refusals(&sent).is_empty());
            let accepted = position_packets(&sent);
            assert_eq!(accepted.len(), 1);
            assert_eq!(
                (accepted[0].teleport, accepted[0].relatives),
                (teleport, ALL)
            );
            tick(game, Vec::new());
            let sent = packets(&rx);
            let reason = refusal_code(Refusal::Pending);
            assert_eq!(refusals(&sent), [Message::Refused { id: 1, reason }]);
            let back = position_packets(&sent);
            assert_eq!(back.len(), 1);
            assert_eq!(
                (back[0].position, back[0].relatives),
                ([24.75, 65.0, 8.5], 0)
            );
        });
    }

    /// Each refusal is told by name, then the player is put back where the server has it:
    /// a passage the level lacks, a crossing while a teleport waits, a far end the guest
    /// does not track, feet more than 20 blocks from the opening, and a move to where it
    /// crossed from that the move checks turn down.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_crossing_the_server_cannot_follow_is_refused_by_name() {
        with_guests(|game| {
            let rx = scene(game, [0.5, 65.0, 8.25]);
            let level = game.level(game.dim.id()).unwrap();
            let away = Passage {
                destination: Vec3::new(100.5, 65.5, 8.5),
                ..doorway()
            };
            assert_eq!(
                game.level_register_passage(level, away),
                Ok(PassageId::from_raw(2))
            );
            let refused =
                |game: &mut crate::game::Game<'_>, id: u64, why: Refusal, at: [f64; 3]| {
                    let sent = packets(&rx);
                    let reason = refusal_code(why);
                    assert_eq!(
                        refusals(&sent),
                        [Message::Refused { id, reason }],
                        "{why:?}"
                    );
                    let back = position_packets(&sent);
                    assert_eq!(back.len(), 1, "{why:?}");
                    assert_eq!((back[0].position, back[0].relatives), (at, 0), "{why:?}");
                    assert_eq!(guest(game).position, at, "{why:?}");
                    back[0].teleport
                };
            let here = [0.5, 65.0, 8.25];
            tick(game, vec![cross(9, here)]);
            refused(game, 9, Refusal::UnknownPassage, here);
            tick(game, vec![cross(1, here)]);
            refused(game, 1, Refusal::Pending, here);
            let accept = |id| {
                S::AcceptTeleportation(play::AcceptTeleportation {
                    id,
                    x: here[0],
                    y: here[1],
                    z: here[2],
                    yaw: 0.0,
                    pitch: 0.0,
                })
            };
            let latest = guest(game).awaiting_teleport.unwrap().0;
            tick(game, vec![accept(latest), cross(2, here)]);
            let teleport = refused(game, 2, Refusal::FarEndNotTracked, here);
            tick(game, vec![accept(teleport), cross(1, [0.5, 65.0, 40.0])]);
            refused(game, 1, Refusal::MoveRefused, here);

            let far = [0.5, 65.0, 29.0];
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.awaiting_teleport = None;
                g.position = far;
                g.previous_position = far;
                g.first_good = far;
            });
            tick(game, vec![cross(1, far)]);
            refused(game, 1, Refusal::TooFar { limit: 20 }, far);
        });
    }

    /// What `player_passage_cross` heard.
    static HEARD: std::sync::Mutex<Vec<(PassageId, Vec3)>> = std::sync::Mutex::new(Vec::new());

    /// Cancels every crossing it hears.
    struct Veto;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Veto {
        fn player_passage_cross(
            &self,
            _: &mut dyn mod_api::ServerHost,
            e: &mut mod_api::PlayerPassageCrossEvent,
        ) -> mod_api::Flow {
            HEARD.lock().unwrap().push((e.passage, e.from));
            mod_api::Flow::Cancel
        }
    }

    /// A passage into another level is refused `FarLevel` however near the player is, and
    /// the player stays: crossing another level waits for clients that can follow.
    #[test]
    #[ignore = "daily"]
    fn a_crossing_into_another_level_is_refused_far_level() {
        with_guests(|game| {
            game.enter_level(Dim::Nether).unwrap();
            game.enter_level(Dim::Overworld).unwrap();
            let rx = scene(game, [0.5, 65.0, 8.25]);
            let level = game.level(game.dim.id()).unwrap();
            let far = Passage {
                destination_level: Some(DimensionKey::NETHER),
                ..near_doorway()
            };
            assert_eq!(
                game.level_register_passage(level, far),
                Ok(PassageId::from_raw(2))
            );
            tick(game, vec![cross(2, [0.5, 65.0, 8.25])]);
            let sent = packets(&rx);
            let reason = refusal_code(Refusal::FarLevel);
            assert_eq!(refusals(&sent), [Message::Refused { id: 2, reason }]);
            assert_eq!(position_packets(&sent)[0].relatives, 0);
            assert_eq!(guest(game).position, [0.5, 65.0, 8.25]);
        });
    }

    /// A mod hears the crossing, its passage and the feet before it, before the server
    /// follows, and cancels it: refused as `Vetoed`, and the player stays.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and an isolated process for native mods"]
    fn a_mod_vetoes_a_crossing_before_the_server_follows() {
        let mods = vec![mod_api::ModEntry::new("veto", Box::new(Veto))];
        mod_platform::testing::with_mods(mods.into(), || {
            with_guests(|game| {
                let rx = scene(game, [0.5, 65.0, 8.25]);
                tick(game, vec![cross(1, [0.5, 65.0, 8.25])]);
                let sent = packets(&rx);
                let reason = refusal_code(Refusal::Vetoed);
                assert_eq!(refusals(&sent), [Message::Refused { id: 1, reason }]);
                assert_eq!(position_packets(&sent)[0].relatives, 0);
                assert_eq!(guest(game).position, [0.5, 65.0, 8.25]);
            });
        });
        assert_eq!(
            *HEARD.lock().unwrap(),
            [(PassageId::from_raw(1), Vec3::new(0.5, 65.0, 8.25))]
        );
    }
}
