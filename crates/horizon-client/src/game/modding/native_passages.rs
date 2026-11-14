//! Passages within one level (#3775, docs/modding/src/how-to/passages.md): the server's
//! registry a mod fills through `LevelRef::register_passage`, its sync to the Horizon
//! clients that hold each passage's chunk (`horizon:passage_sync`, sent with the chunk
//! through the hub's chunk watches), and the copy a client keeps until it forgets the chunk.
use super::Game;
use horizon_client::fast_hash::FxHashMap;
use horizon_protocol::passage_sync::{Message, PassageForm};
use mod_api::{
    DimensionKey, LevelRef, Passage, PassageId, PassageRegistrationError as Refused, PassageTurn,
    Vec3,
};

/// Passages a level holds at most. A client holds every passage of the chunks it has, so
/// this bounds its copy and what the server sends it whatever the view distance.
const MAX_PER_LEVEL: u32 = 256;
/// Passages one chunk holds at most: what the server sends with the chunk, and the
/// passages a client looks through around a moving player (#3775).
const MAX_PER_CHUNK: u32 = 8;
/// The longest side of an opening, in blocks: an opening stays within 23 blocks of its
/// centre, so the chunk holding the centre finds it.
const MAX_SIDE: u32 = 32;
/// How far from 1 a unit axis's squared length, and from 0 the axes' dot product, may be.
const AXIS_TOLERANCE: f64 = 1e-6;
/// `ServerGamePacketListenerImpl.clampHorizontal` and `clampVertical`.
const HORIZONTAL_LIMIT: f64 = 3.0e7;
const VERTICAL_LIMIT: f64 = 2.0e7;

/// The passages of the server's levels and their changes still to tell, and on a client
/// the passages its server told it of. Everything goes when a level or the world retires
/// (`retire_level`, `retire_world`); the numbers go on, so an old one is never reused while
/// the game runs.
#[derive(Default)]
pub(crate) struct Passages {
    /// The server's passages, by level, then by chunk ([`Passage::chunk`]).
    levels: FxHashMap<DimensionKey, LevelPassages>,
    /// Where each of the server's passages is.
    index: FxHashMap<PassageId, (DimensionKey, [i32; 2])>,
    /// The last number given; numbers start at 1 and are never reused.
    last: u64,
    /// The passages whose change the clients have not all been told, one entry each: a
    /// live passage not told to every client yet, or a removed one some were told of. So it
    /// holds at most the live passages and those told before their removal.
    dirty: FxHashMap<PassageId, Dirty>,
    /// A client's copy: what its server told it of, kept with the chunk.
    pub(super) held: FxHashMap<PassageId, Passage>,
    /// A client's crossing that waits for the server's position packet
    /// (`native_passage_cross.rs`).
    pub(super) crossing: Option<PassageId>,
    /// The server's refusal of the client's last crossing, until the next.
    pub(super) refused: Option<(PassageId, mod_api::PassageCrossRefusal)>,
}

#[derive(Default)]
struct LevelPassages {
    count: u32,
    chunks: FxHashMap<[i32; 2], Vec<(PassageId, Passage)>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dirty {
    /// Registered and not told to every client tracking its chunk; `told` once a chunk's
    /// sync gave it to one, so a removal must be told too.
    Added { told: bool },
    /// Removed from this level and chunk after the clients were told of it.
    Removed(DimensionKey, [i32; 2]),
}

/// Why a client did not keep a passage its server sent (`remote_passage_sync`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HeldRefusal {
    /// Not a passage the server could have registered.
    Bad(Refused),
    /// Its chunk is not one the client holds.
    ChunkNotHeld,
    /// The client already holds a level's worth (256).
    Full,
}

impl Passages {
    /// Level `key` closed (`LevelEvent.Unload`): its passages and their untold changes go.
    pub(in crate::game) fn retire_level(&mut self, key: DimensionKey) {
        if self.levels.remove(&key).is_none() && self.dirty.is_empty() {
            return;
        }
        self.index.retain(|_, (level, _)| *level != key);
        let index = &self.index;
        self.dirty.retain(|id, dirty| match dirty {
            Dirty::Added { .. } => index.contains_key(id),
            Dirty::Removed(level, _) => *level != key,
        });
    }

    /// The world opened, closed or failed: every passage and change goes, the server's and
    /// a client's copy, but not the numbering.
    pub(in crate::game) fn retire_world(&mut self) {
        self.levels.clear();
        self.index.clear();
        self.dirty.clear();
        self.held.clear();
        self.crossing = None;
        self.refused = None;
    }

    /// A new hub (the LAN opened again) records the chunks it sends from its start while a
    /// passage lives, so a chunk it sends before the next `flush_passages` carries them.
    pub(in crate::game) fn record_on(&self, hub: &mut horizon_client::lan::hub::Hub) {
        if !self.index.is_empty() {
            hub.chunk_watches.record_for_passages();
        }
    }

    /// The server's passage `id`, if level `key` holds it.
    pub(super) fn find(&self, key: DimensionKey, id: PassageId) -> Option<Passage> {
        let &(at, chunk) = self.index.get(&id).filter(|(at, _)| *at == key)?;
        let list = self.levels.get(&at)?.chunks.get(&chunk)?;
        list.iter().find(|(held, _)| *held == id).map(|(_, p)| *p)
    }
}

impl Game<'_> {
    /// `LevelRef::register_passage`: checks `passage`, numbers it and keeps it under its
    /// chunk; the clients holding that chunk hear of it in `flush_passages`.
    pub(super) fn level_register_passage(
        &mut self,
        level: LevelRef,
        passage: Passage,
    ) -> Result<PassageId, Refused> {
        self.native_level_world_result(level)?;
        check(&passage)?;
        if let Some(to) = passage.destination_level {
            let open =
                mod_api::__private::dimension_id(to).is_some_and(|id| self.level(id).is_some());
            if to == level.dimension() || !open {
                return Err(Refused::UnknownLevel);
            }
        }
        let chunk = passage.chunk();
        let passages = &mut self.passages;
        let held = passages.levels.entry(level.dimension()).or_default();
        if held.count >= MAX_PER_LEVEL {
            return Err(Refused::LevelFull {
                limit: MAX_PER_LEVEL,
            });
        }
        if held.chunks.get(&chunk).map_or(0, Vec::len) >= MAX_PER_CHUNK as usize {
            return Err(Refused::ChunkFull {
                limit: MAX_PER_CHUNK,
            });
        }
        let Some(next) = passages.last.checked_add(1) else {
            return Err(Refused::NoIdsLeft);
        };
        passages.last = next;
        let id = PassageId::from_raw(next);
        held.chunks.entry(chunk).or_default().push((id, passage));
        held.count += 1;
        passages.index.insert(id, (level.dimension(), chunk));
        passages.dirty.insert(id, Dirty::Added { told: false });
        Ok(id)
    }

    /// `LevelRef::remove_passage`: true when `id` was one of the level's passages. The
    /// clients holding its chunk hear it is gone in `flush_passages`, unless they were never
    /// told of it.
    pub(super) fn level_remove_passage(&mut self, level: LevelRef, id: PassageId) -> bool {
        if self.native_level_world_result(level).is_err() {
            return false;
        }
        let passages = &mut self.passages;
        let key = level.dimension();
        let Some(&(at, chunk)) = passages.index.get(&id).filter(|(at, _)| *at == key) else {
            return false;
        };
        passages.index.remove(&id);
        if let Some(held) = passages.levels.get_mut(&at) {
            if let Some(list) = held.chunks.get_mut(&chunk) {
                list.retain(|(held, _)| *held != id);
                if list.is_empty() {
                    held.chunks.remove(&chunk);
                }
            }
            held.count -= 1;
        }
        if passages.dirty.remove(&id) != Some(Dirty::Added { told: false }) {
            passages.dirty.insert(id, Dirty::Removed(at, chunk));
        }
        true
    }

    /// The changes since the last call, by number, told to the Horizon clients that track
    /// the passage's chunk (`ChunkMap.isChunkTracked`). While a passage lives, the hub
    /// records the chunks it sends (`ChunkWatchLog::record_for_passages`), a new hub after
    /// the LAN reopens too, so a client that gets the chunk later hears of it with the
    /// chunk. Kept for the next call while the hub is lent out.
    pub(in crate::game) fn flush_passages(&mut self) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let hub = &mut host.hub;
        if !self.passages.index.is_empty() {
            hub.chunk_watches.record_for_passages();
        }
        if self.passages.dirty.is_empty() {
            return;
        }
        let passages = &mut self.passages;
        let mut changes: Vec<_> = passages.dirty.drain().collect();
        changes.sort_unstable_by_key(|(id, _)| *id);
        for (id, dirty) in changes {
            let (key, chunk, message) = match dirty {
                Dirty::Added { .. } => {
                    let Some(&(key, chunk)) = passages.index.get(&id) else {
                        continue;
                    };
                    let Some(p) = passages.find(key, id) else {
                        continue;
                    };
                    if p.destination_level.is_some() {
                        continue;
                    }
                    (key, chunk, Message::Add(vec![form(id, &p)]))
                }
                Dirty::Removed(key, chunk) => (key, chunk, Message::Remove(vec![id.raw()])),
            };
            let Some(dimension) = mod_api::__private::dimension_id(key) else {
                continue;
            };
            for g in hub.guests.iter() {
                if g.tracks_chunk(dimension, (chunk[0], chunk[1])) {
                    g.send_passage_sync(&message);
                }
            }
        }
    }

    /// `chunk` of `level` went to player `uuid` (the hub's `chunk_sent`): its passages go
    /// after it, if it has any and the player still tracks it there.
    pub(super) fn send_chunk_passages(&mut self, level: LevelRef, uuid: u128, chunk: [i32; 2]) {
        let Some(list) = self
            .passages
            .levels
            .get(&level.dimension())
            .and_then(|l| l.chunks.get(&chunk))
        else {
            return;
        };
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let Some(g) = host.hub.guests.iter().find(|g| g.uuid == uuid) else {
            return;
        };
        if !g.tracks_chunk(level.dimension_id(), (chunk[0], chunk[1])) {
            return;
        }
        let told: Vec<_> = list
            .iter()
            .filter(|(_, p)| p.destination_level.is_none())
            .collect();
        if told.is_empty() {
            return;
        }
        let forms = told.iter().map(|(id, p)| form(*id, p)).collect();
        g.send_passage_sync(&Message::Add(forms));
        for (id, _) in told {
            if let Some(Dirty::Added { told }) = self.passages.dirty.get_mut(id) {
                *told = true;
            }
        }
    }

    /// A `horizon:passage_sync` payload the server sent the window. Each passage is checked
    /// as the server checks a registration, and kept only for a chunk the window holds and
    /// within a level's 256; the refused ones are logged and returned with their number.
    pub(super) fn remote_passage_sync(&mut self, data: &[u8]) -> Vec<(u64, HeldRefusal)> {
        let message = match horizon_protocol::passage_sync::decode(data) {
            Ok(message) => message,
            Err(e) => {
                log::debug!("Horizon: a bad passage sync: {e}");
                return Vec::new();
            }
        };
        let mut refused = Vec::new();
        match message {
            Message::Add(forms) => {
                for f in forms {
                    if let Err(why) = self.hold_passage(&f) {
                        log::debug!("Horizon: passage {} not kept: {why:?}", f.id);
                        refused.push((f.id, why));
                    }
                }
            }
            Message::Remove(ids) => {
                for id in ids {
                    self.passages.held.remove(&PassageId::from_raw(id));
                }
            }
            Message::Refused { id, reason } => self.remote_passage_refused(id, reason),
        }
        refused
    }

    fn hold_passage(&mut self, f: &PassageForm) -> Result<(), HeldRefusal> {
        let p = passage(f);
        check(&p).map_err(HeldRefusal::Bad)?;
        let [x, z] = p.chunk();
        if !self.world.chunks.contains_key(&(x, z)) {
            return Err(HeldRefusal::ChunkNotHeld);
        }
        let held = &mut self.passages.held;
        let id = PassageId::from_raw(f.id);
        if !held.contains_key(&id) && held.len() >= MAX_PER_LEVEL as usize {
            return Err(HeldRefusal::Full);
        }
        held.insert(id, p);
        Ok(())
    }

    /// The window forgot chunk `(x, z)`: the passages of that chunk go with it.
    pub(super) fn forget_chunk_passages(&mut self, x: i32, z: i32) {
        if !self.passages.held.is_empty() {
            self.passages.held.retain(|_, p| p.chunk() != [x, z]);
        }
    }
}

/// The refusal for a passage that is not a rectangle the engine can carry a player through.
fn check(p: &Passage) -> Result<(), Refused> {
    let parts = |v: Vec3| [v.x, v.y, v.z];
    let finite = [p.origin, p.axis_w, p.axis_h, p.destination]
        .into_iter()
        .flat_map(parts)
        .chain([p.width, p.height])
        .all(f64::is_finite);
    if !finite {
        return Err(Refused::NotFinite);
    }
    let dot = |a: Vec3, b: Vec3| a.x * b.x + a.y * b.y + a.z * b.z;
    let (w, h) = (p.axis_w, p.axis_h);
    if (dot(w, w) - 1.0).abs() > AXIS_TOLERANCE
        || (dot(h, h) - 1.0).abs() > AXIS_TOLERANCE
        || dot(w, h).abs() > AXIS_TOLERANCE
    {
        return Err(Refused::BadAxes);
    }
    let side = |s: f64| s > 0.0 && s <= f64::from(MAX_SIDE);
    if !side(p.width) || !side(p.height) {
        return Err(Refused::BadSize { max: MAX_SIDE });
    }
    let ends = [(p.origin, w, h), (p.destination, p.turned(w), p.turned(h))];
    for (centre, w, h) in ends {
        for (sw, sh) in [(-0.5, -0.5), (0.5, -0.5), (-0.5, 0.5), (0.5, 0.5)] {
            let (a, b) = (sw * p.width, sh * p.height);
            let corner = [
                centre.x + w.x * a + h.x * b,
                centre.y + w.y * a + h.y * b,
                centre.z + w.z * a + h.z * b,
            ];
            if corner[0].abs() > HORIZONTAL_LIMIT
                || corner[2].abs() > HORIZONTAL_LIMIT
                || corner[1].abs() > VERTICAL_LIMIT
            {
                return Err(Refused::OutsideWorld);
            }
        }
    }
    Ok(())
}

fn turn_index(turn: PassageTurn) -> u8 {
    match turn {
        PassageTurn::Clockwise90 => 1,
        PassageTurn::Clockwise180 => 2,
        PassageTurn::Counterclockwise90 => 3,
        _ => 0,
    }
}

fn form(id: PassageId, p: &Passage) -> PassageForm {
    let v = |v: Vec3| [v.x, v.y, v.z];
    PassageForm {
        id: id.raw(),
        origin: v(p.origin),
        axis_w: v(p.axis_w),
        axis_h: v(p.axis_h),
        width: p.width,
        height: p.height,
        destination: v(p.destination),
        turn: turn_index(p.turn),
    }
}

fn passage(f: &PassageForm) -> Passage {
    let v = |[x, y, z]: [f64; 3]| Vec3::new(x, y, z);
    Passage {
        origin: v(f.origin),
        axis_w: v(f.axis_w),
        axis_h: v(f.axis_h),
        width: f.width,
        height: f.height,
        destination: v(f.destination),
        turn: match f.turn {
            1 => PassageTurn::Clockwise90,
            2 => PassageTurn::Clockwise180,
            3 => PassageTurn::Counterclockwise90,
            _ => PassageTurn::None,
        },
        destination_level: None,
    }
}

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::{HeldRefusal, Message, PassageForm, Refused, check};
    use crate::game::Dim;
    use crate::game::native_entity_refs::tests::{with_client_game, with_game};
    use crate::game::native_guest_turn_tests::with_guests;
    use horizon_client::chunk::Chunk;
    use horizon_protocol::packets::play::Clientbound;
    use horizon_protocol::passage_sync::encode;
    use mod_api::{DimensionKey, LevelRef, Passage, PassageId, PassageTurn, Vec3, WorldEpoch};
    use std::sync::{Arc, mpsc};

    /// A 2 by 3 doorway facing south at (0.5, 65.5, 8.5) in chunk (0, 0), coming out 100
    /// blocks east, turned a clockwise quarter.
    pub(in crate::game) fn doorway() -> Passage {
        Passage {
            origin: Vec3::new(0.5, 65.5, 8.5),
            axis_w: Vec3::new(1.0, 0.0, 0.0),
            axis_h: Vec3::new(0.0, 1.0, 0.0),
            width: 2.0,
            height: 3.0,
            destination: Vec3::new(100.5, 65.5, 8.5),
            turn: PassageTurn::Clockwise90,
            destination_level: None,
        }
    }

    /// The doorway as it travels, numbered `id`.
    fn doorway_form(id: u64) -> PassageForm {
        PassageForm {
            id,
            origin: [0.5, 65.5, 8.5],
            axis_w: [1.0, 0.0, 0.0],
            axis_h: [0.0, 1.0, 0.0],
            width: 2.0,
            height: 3.0,
            destination: [100.5, 65.5, 8.5],
            turn: 1,
        }
    }

    /// The play packets a guest was sent, in order; ones this build cannot read are left out.
    pub(in crate::game) fn packets(
        rx: &mpsc::Receiver<horizon_client::lan::Out>,
    ) -> Vec<Clientbound> {
        use horizon_client::lan::Out;
        use horizon_protocol::buf::Reader;
        let mut packets = Vec::new();
        for out in rx.try_iter() {
            let bodies = match out {
                Out::Packet(p) => vec![p],
                Out::Packets(p) => p,
                Out::Disconnect(_) => panic!("unexpected disconnect"),
            };
            for body in bodies {
                let mut r = Reader::new(&body);
                let id = r.varint().unwrap();
                packets.extend(Clientbound::decode(id, r.rest()).ok());
            }
        }
        packets
    }

    /// The passage messages among `packets`, in order.
    pub(in crate::game) fn passage_messages(packets: &[Clientbound]) -> Vec<Message> {
        packets
            .iter()
            .filter_map(|p| match p {
                Clientbound::CustomPayload(p) if p.channel == "horizon:passage_sync" => {
                    Some(horizon_protocol::passage_sync::decode(&p.data).unwrap())
                }
                _ => None,
            })
            .collect()
    }

    /// Guests 12 and 13 with senders to read and the given brands.
    pub(in crate::game) fn listen(
        game: &mut crate::game::Game<'_>,
        brands: [&str; 2],
    ) -> Vec<mpsc::Receiver<horizon_client::lan::Out>> {
        let mut receivers = Vec::new();
        for (conn, brand) in [12, 13].into_iter().zip(brands) {
            let (tx, rx) = mpsc::channel();
            game.lan.as_mut().unwrap().hub.update_guest(conn, |g| {
                g.sender = horizon_client::lan::PlayerSender::new(tx, Arc::default());
                g.brand = brand.into();
            });
            receivers.push(rx);
        }
        receivers
    }

    /// The doorway passes; each way of not being a finite rectangle with unit axes at a
    /// right angle, sides in (0, 32] and corners inside vanilla's clamps is refused by name,
    /// at either end.
    #[test]
    fn a_passage_that_is_not_a_finite_rectangle_inside_the_world_is_refused() {
        assert_eq!(check(&doorway()), Ok(()));
        let with = |change: fn(&mut Passage)| {
            let mut p = doorway();
            change(&mut p);
            check(&p)
        };
        assert_eq!(with(|p| p.origin.y = f64::NAN), Err(Refused::NotFinite));
        assert_eq!(with(|p| p.height = f64::INFINITY), Err(Refused::NotFinite));
        assert_eq!(
            with(|p| p.axis_w = Vec3::new(2.0, 0.0, 0.0)),
            Err(Refused::BadAxes)
        );
        assert_eq!(
            with(|p| p.axis_h = Vec3::new(0.6, 0.8, 0.0)),
            Err(Refused::BadAxes)
        );
        assert_eq!(with(|p| p.width = 0.0), Err(Refused::BadSize { max: 32 }));
        assert_eq!(with(|p| p.height = -1.0), Err(Refused::BadSize { max: 32 }));
        assert_eq!(with(|p| p.width = 32.5), Err(Refused::BadSize { max: 32 }));
        assert_eq!(with(|p| p.width = 32.0), Ok(()));
        // The far side's corner: 29,999,999.5 + 1 east is past 30,000,000.
        assert_eq!(
            with(|p| p.destination.x = 29_999_999.5),
            Ok(()),
            "turned a quarter, the width runs north-south there"
        );
        assert_eq!(
            with(|p| {
                p.destination.x = 29_999_999.5;
                p.turn = PassageTurn::None;
            }),
            Err(Refused::OutsideWorld)
        );
        assert_eq!(
            with(|p| p.origin.z = -30_000_000.5),
            Err(Refused::OutsideWorld)
        );
        assert_eq!(
            with(|p| p.origin.y = 20_000_000.0),
            Err(Refused::OutsideWorld)
        );
    }

    /// Passages are numbered from 1 in order; a chunk takes 8 and a level 256, and the next
    /// is refused with nothing registered; a removal frees its place once. A stale handle
    /// and a client register nothing.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_level_numbers_its_passages_and_refuses_past_its_caps() {
        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let in_chunk = |cx: i32, i: u32| Passage {
                origin: Vec3::new(f64::from(cx * 16) + 0.5, 65.5, f64::from(i) + 0.5),
                ..doorway()
            };
            for i in 0..8 {
                let id = game.level_register_passage(level, in_chunk(0, i)).unwrap();
                assert_eq!(id, PassageId::from_raw(u64::from(i) + 1));
            }
            assert_eq!(
                game.level_register_passage(level, in_chunk(0, 8)),
                Err(Refused::ChunkFull { limit: 8 })
            );
            for cx in 1..32 {
                for i in 0..8 {
                    game.level_register_passage(level, in_chunk(cx, i)).unwrap();
                }
            }
            assert_eq!(
                game.level_register_passage(level, in_chunk(40, 0)),
                Err(Refused::LevelFull { limit: 256 })
            );
            assert!(game.level_remove_passage(level, PassageId::from_raw(3)));
            assert!(!game.level_remove_passage(level, PassageId::from_raw(3)));
            assert!(!game.level_remove_passage(level, PassageId::from_raw(999)));
            assert_eq!(
                game.level_register_passage(level, in_chunk(0, 8)),
                Ok(PassageId::from_raw(257)),
                "the freed place, under a new number"
            );
            assert_eq!(
                game.level_register_passage(level, in_chunk(40, 0)),
                Err(Refused::LevelFull { limit: 256 })
            );
            let stale = LevelRef::new(WorldEpoch::new(0), game.dim.key());
            assert_eq!(
                game.level_register_passage(stale, doorway()),
                Err(Refused::Stale)
            );
            assert!(!game.level_remove_passage(stale, PassageId::from_raw(1)));
            // The last id is given once; then registration is refused, nothing given twice.
            game.passages.last = u64::MAX - 1;
            assert!(game.level_remove_passage(level, PassageId::from_raw(4)));
            assert_eq!(
                game.level_register_passage(level, in_chunk(0, 9)),
                Ok(PassageId::from_raw(u64::MAX))
            );
            assert!(game.level_remove_passage(level, PassageId::from_raw(5)));
            assert_eq!(
                game.level_register_passage(level, in_chunk(0, 10)),
                Err(Refused::NoIdsLeft)
            );
        });
        with_client_game(|game| {
            let epoch = game.native_lifecycle.epoch().unwrap();
            let level = LevelRef::new(epoch, game.dim.key());
            assert_eq!(
                game.level_register_passage(level, doorway()),
                Err(Refused::Unavailable)
            );
            assert!(!game.level_remove_passage(level, PassageId::from_raw(1)));
        });
    }

    /// A level's passages and their untold changes go when it closes: reopened, it starts
    /// with no passage and its whole caps, and numbering goes on (1 is not given again).
    /// A failed open retires everything, the window's copy too.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn passages_go_with_their_level_and_world() {
        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            assert_eq!(
                game.level_register_passage(level, doorway()),
                Ok(PassageId::from_raw(1))
            );
            game.native_levels_closing();
            assert!(game.passages.index.is_empty() && game.passages.dirty.is_empty());
            assert!(!game.level_remove_passage(level, PassageId::from_raw(1)));
            game.native_level_opened(game.dim);
            for i in 0..8 {
                let id = game.level_register_passage(level, doorway()).unwrap();
                assert_eq!(id, PassageId::from_raw(i + 2));
            }
            game.passages.held.insert(PassageId::from_raw(1), doorway());
            game.native_lifecycle_world_failed("test failure");
            assert!(game.passages.levels.is_empty() && game.passages.index.is_empty());
            assert!(game.passages.dirty.is_empty() && game.passages.held.is_empty());
        });
    }

    /// A destination in another level is registered only while that level is open: one that
    /// is not open, and the passage's own level, are refused `UnknownLevel`, with nothing
    /// registered.
    #[test]
    #[ignore = "daily"]
    fn a_destination_level_must_be_open_and_another_level() {
        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let to = |key| Passage {
                destination_level: Some(key),
                ..doorway()
            };
            assert_eq!(
                game.level_register_passage(level, to(DimensionKey::NETHER)),
                Err(Refused::UnknownLevel),
                "not open"
            );
            assert_eq!(
                game.level_register_passage(level, to(level.dimension())),
                Err(Refused::UnknownLevel),
                "its own level"
            );
            assert!(game.passages.index.is_empty());
            game.enter_level(Dim::Nether).unwrap();
            assert_eq!(
                game.level_register_passage(level, to(DimensionKey::NETHER)),
                Ok(PassageId::from_raw(1))
            );
        });
    }

    /// A passage into another level is registered but not sent: neither at the tick's end
    /// nor after its chunk. The client hears only of the passage within the level.
    #[test]
    #[ignore = "daily"]
    fn a_passage_into_another_level_is_not_sent_to_the_clients() {
        with_guests(|game| {
            game.enter_level(Dim::Nether).unwrap();
            game.enter_level(Dim::Overworld).unwrap();
            game.native_lifecycle.world_open_ready().unwrap();
            let receivers = listen(game, ["horizon", "vanilla"]);
            let level = game.level(game.dim.id()).unwrap();
            let far = Passage {
                destination_level: Some(DimensionKey::NETHER),
                ..doorway()
            };
            assert_eq!(
                game.level_register_passage(level, far),
                Ok(PassageId::from_raw(1))
            );
            assert_eq!(
                game.level_register_passage(level, doorway()),
                Ok(PassageId::from_raw(2))
            );
            for cz in -1..=1 {
                for cx in -1..=1 {
                    let mut chunk = Chunk::empty(cx, cz);
                    chunk.lit = true;
                    game.world.chunks.insert((cx, cz), Arc::new(chunk));
                }
            }
            let hub = &mut game.lan.as_mut().unwrap().hub;
            hub.ticks = 10;
            hub.guests.update_all(|g| g.max_unacknowledged = 64);
            let mut sent = Vec::new();
            for _ in 0..5 {
                game.tick_lan(false);
                sent.extend(packets(&receivers[0]));
            }
            assert_eq!(
                passage_messages(&sent),
                [Message::Add(vec![doorway_form(2)])]
            );
        });
    }

    /// In the real order of a server tick: a registration with no client holding the
    /// chunk is told to nobody; the hub then sends the chunk and the passage follows it, to
    /// the Horizon client only. A removal reaches it at the next tick's end.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_passage_follows_its_chunk_to_the_horizon_clients() {
        with_guests(|game| {
            game.native_lifecycle.world_open_ready().unwrap();
            let receivers = listen(game, ["horizon", "vanilla"]);
            let level = game.level(game.dim.id()).unwrap();
            let id = game.level_register_passage(level, doorway()).unwrap();
            game.flush_passages();
            assert!(
                receivers
                    .iter()
                    .all(|rx| passage_messages(&packets(rx)).is_empty())
            );
            for cz in -1..=1 {
                for cx in -1..=1 {
                    let mut chunk = Chunk::empty(cx, cz);
                    chunk.lit = true;
                    game.world.chunks.insert((cx, cz), Arc::new(chunk));
                }
            }
            let hub = &mut game.lan.as_mut().unwrap().hub;
            hub.ticks = 10;
            hub.guests.update_all(|g| g.max_unacknowledged = 64);
            let mut sent: Vec<Vec<Clientbound>> = vec![Vec::new(), Vec::new()];
            for _ in 0..5 {
                game.tick_lan(false);
                for (rx, sent) in receivers.iter().zip(&mut sent) {
                    sent.extend(packets(rx));
                }
            }
            let chunk_at = |sent: &[Clientbound]| {
                sent.iter().position(
                    |p| matches!(p, Clientbound::LevelChunkWithLight(c) if (c.x, c.z) == (0, 0)),
                )
            };
            let horizon = &sent[0];
            let chunk = chunk_at(horizon).expect("the hub sent chunk (0, 0)");
            let told = horizon
                .iter()
                .position(|p| matches!(p, Clientbound::CustomPayload(p) if p.channel == "horizon:passage_sync"))
                .expect("the passage followed");
            assert!(chunk < told, "the chunk comes first");
            assert_eq!(
                passage_messages(horizon),
                [Message::Add(vec![doorway_form(1)])]
            );
            assert!(chunk_at(&sent[1]).is_some());
            assert!(
                passage_messages(&sent[1]).is_empty(),
                "never to a vanilla client"
            );

            assert!(game.level_remove_passage(level, id));
            game.tick_lan(false);
            assert_eq!(
                passage_messages(&packets(&receivers[0])),
                [Message::Remove(vec![1])]
            );
        });
    }

    /// A passage registered and removed before the tick's end is never told and leaves no
    /// trace, however often; the changes kept stay within the live passages and the told
    /// ones removed.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn churn_before_the_tick_end_keeps_and_sends_nothing() {
        with_guests(|game| {
            game.native_lifecycle.world_open_ready().unwrap();
            let receivers = listen(game, ["horizon", "horizon"]);
            let hub = &mut game.lan.as_mut().unwrap().hub;
            hub.guests.update_all(|g| {
                g.center = Some((0, 0));
                g.sent_chunks.insert((0, 0));
            });
            let level = game.level(game.dim.id()).unwrap();
            for _ in 0..1000 {
                let id = game.level_register_passage(level, doorway()).unwrap();
                assert!(game.level_remove_passage(level, id));
            }
            assert!(game.passages.dirty.is_empty() && game.passages.index.is_empty());
            game.flush_passages();
            assert!(
                receivers
                    .iter()
                    .all(|rx| passage_messages(&packets(rx)).is_empty())
            );

            let kept = game.level_register_passage(level, doorway()).unwrap();
            game.flush_passages();
            for _ in 0..1000 {
                let id = game.level_register_passage(level, doorway()).unwrap();
                assert!(game.level_remove_passage(level, id));
            }
            assert!(game.level_remove_passage(level, kept));
            assert_eq!(game.passages.dirty.len(), 1, "only the told one's removal");
            game.flush_passages();
            for rx in &receivers {
                assert_eq!(
                    passage_messages(&packets(rx)),
                    [
                        Message::Add(vec![doorway_form(1001)]),
                        Message::Remove(vec![1001])
                    ]
                );
            }
        });
    }

    /// The LAN closes and opens again with a passage live: the new hub records the chunks
    /// it sends from its start, so a Horizon guest that joins it is sent the passage with
    /// the chunk the first tick sends, before that tick's `flush_passages`.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_reopened_lan_records_chunks_for_the_live_passages() {
        use crate::game::lan::LanSettings;
        use horizon_client::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
        use horizon_protocol::packets::configuration::ClientInformation;
        with_guests(|game| {
            game.native_lifecycle.world_open_ready().unwrap();
            let level = game.level(game.dim.id()).unwrap();
            game.level_register_passage(level, doorway()).unwrap();
            game.flush_passages();
            game.lan_disconnect(false);
            let settings = LanSettings {
                port: 0,
                guest_commands: false,
                force_game_mode: false,
            };
            let net = NetConfig {
                listen: false,
                announce: false,
                lod_distance: 0,
                ..NetConfig::lan(0, String::new(), 2)
            };
            let config = HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: game.dim.id().to_owned(),
                allow_flight: true,
                seed: 0,
                max_players: 2,
                integrated: true,
                levels: game.level_shapes,
            };
            game.lan_host(settings, net, config).unwrap();
            assert!(game.lan.as_ref().unwrap().hub.chunk_watches.recording());
            for cz in -1..=1 {
                for cx in -1..=1 {
                    let mut chunk = Chunk::empty(cx, cz);
                    chunk.lit = true;
                    game.world.chunks.insert((cx, cz), Arc::new(chunk));
                }
            }
            let (tx, rx) = mpsc::channel();
            let mut guest = Guest::new(
                Joined {
                    conn: 14,
                    uuid: 14,
                    name: "late".into(),
                    properties: Vec::new(),
                    info: ClientInformation::new(2),
                    brand: "horizon".into(),
                    sender: PlayerSender::new(tx, Arc::default()),
                    latency: Arc::default(),
                    address: "127.0.0.1:1".parse().unwrap(),
                    expected: false,
                },
                14,
                [3.0, 64.0, 5.0],
                2,
            );
            guest.placed = true;
            guest.max_unacknowledged = 64;
            let hub = &mut game.lan.as_mut().unwrap().hub;
            hub.ticks = 10;
            hub.guests.push(guest);
            // One tick: its level pass sends the chunks, its end flushes the passages.
            game.tick_lan(false);
            let sent = packets(&rx);
            let chunk = sent
                .iter()
                .position(
                    |p| matches!(p, Clientbound::LevelChunkWithLight(c) if (c.x, c.z) == (0, 0)),
                )
                .expect("the first tick sent chunk (0, 0)");
            let told = sent
                .iter()
                .position(|p| matches!(p, Clientbound::CustomPayload(p) if p.channel == "horizon:passage_sync"))
                .expect("the passage went with its chunk");
            assert!(chunk < told);
            assert_eq!(
                passage_messages(&sent),
                [Message::Add(vec![doorway_form(1)])]
            );
        });
    }

    /// A Horizon arrival held for a hand-over is sent its chunks, so it is told their
    /// passages too; a held vanilla client is not.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_held_arrival_is_told_its_chunks_passages() {
        with_guests(|game| {
            game.native_lifecycle.world_open_ready().unwrap();
            let receivers = listen(game, ["horizon", "vanilla"]);
            let hub = &mut game.lan.as_mut().unwrap().hub;
            hub.guests.update_all(|g| {
                g.placed = false;
                g.held = true;
                g.center = Some((0, 0));
                g.sent_chunks.insert((0, 0));
            });
            let level = game.level(game.dim.id()).unwrap();
            game.level_register_passage(level, doorway()).unwrap();
            game.flush_passages();
            assert_eq!(
                passage_messages(&packets(&receivers[0])),
                [Message::Add(vec![doorway_form(1)])]
            );
            assert!(passage_messages(&packets(&receivers[1])).is_empty());
        });
    }

    /// Removes the passage in the second `chunk_sent` it hears.
    struct RemoveOnSecondSent {
        heard: std::sync::Mutex<u32>,
        id: PassageId,
    }
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for RemoveOnSecondSent {
        fn chunk_sent(&self, host: &mut dyn mod_api::ServerHost, e: &mut mod_api::ChunkSentEvent) {
            let mut heard = self.heard.lock().unwrap();
            *heard += 1;
            if *heard == 2 {
                assert!(e.chunk.level().remove_passage(host, self.id));
            }
        }
    }

    /// Two chunk sends in one tick: the first gives a new passage to guest 12 with its
    /// chunk, and the mod removes it at the second, before the tick's end told it to the
    /// rest. The removal still reaches 12, which ends with no passage, and 13 is told
    /// only the removal.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and an isolated process for native mods"]
    fn a_passage_sent_with_its_chunk_and_removed_in_the_same_tick_leaves_no_ghost() {
        use horizon_client::lan::chunk_watch::{
            ChunkWatch, ChunkWatchKind, ChunkWatchLog, LevelWatches,
        };
        let mods = vec![mod_api::ModEntry::new(
            "remover",
            Box::new(RemoveOnSecondSent {
                heard: Default::default(),
                id: PassageId::from_raw(1),
            }),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            with_guests(|game| {
                game.native_lifecycle.world_open_ready().unwrap();
                let receivers = listen(game, ["horizon", "horizon"]);
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let dimension = game.dim.id().to_owned();
                let hub = &mut game.lan.as_mut().unwrap().hub;
                hub.guests.update_all(|g| {
                    g.center = Some((0, 0));
                    g.sent_chunks.insert((0, 0));
                });
                let sent = |uuid: u128| ChunkWatch {
                    kind: ChunkWatchKind::Sent,
                    uuid,
                    entity_id: uuid as i32,
                    chunk: [0, 0],
                };
                hub.chunk_watches = ChunkWatchLog::new(vec![LevelWatches {
                    dimension,
                    watches: vec![sent(12), sent(13)],
                }]);
                let level = game.level(game.dim.id()).unwrap();
                let id = game.level_register_passage(level, doorway()).unwrap();
                assert_eq!(id, PassageId::from_raw(1));
                game.run_chunk_watches();
                game.flush_passages();
                let twelve = packets(&receivers[0]);
                assert_eq!(
                    passage_messages(&twelve),
                    [
                        Message::Add(vec![doorway_form(1)]),
                        Message::Remove(vec![1])
                    ]
                );
                assert_eq!(
                    passage_messages(&packets(&receivers[1])),
                    [Message::Remove(vec![1])]
                );
                // Guest 12's client, given what it was sent, holds nothing.
                for p in &twelve {
                    if let Clientbound::CustomPayload(p) = p
                        && p.channel == "horizon:passage_sync"
                    {
                        assert!(game.remote_passage_sync(&p.data).is_empty());
                    }
                }
                assert!(game.passages.held.is_empty());
            });
        });
    }

    /// The window keeps what its server sent for a chunk it holds, by number, until the
    /// server removes it or the window forgets the chunk. It refuses by name a passage the
    /// server could not have registered, one for a chunk it does not hold, and a 257th,
    /// across packets; a number sent again replaces its passage.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn the_window_keeps_only_passages_it_can_hold() {
        with_client_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let send = |game: &mut crate::game::Game<'_>, message: Message| {
                game.remote_passage_sync(&encode(&message).unwrap())
            };
            let bad = PassageForm {
                width: f64::NAN,
                ..doorway_form(2)
            };
            let skewed = PassageForm {
                axis_h: [1.0, 0.0, 0.0],
                ..doorway_form(3)
            };
            let elsewhere = PassageForm {
                origin: [80.5, 65.5, 8.5],
                ..doorway_form(4)
            };
            let refused = send(
                game,
                Message::Add(vec![doorway_form(1), bad, skewed, elsewhere]),
            );
            assert_eq!(
                refused,
                [
                    (2, HeldRefusal::Bad(Refused::NotFinite)),
                    (3, HeldRefusal::Bad(Refused::BadAxes)),
                    (4, HeldRefusal::ChunkNotHeld),
                ]
            );
            assert_eq!(
                game.passages.held.get(&PassageId::from_raw(1)),
                Some(&doorway())
            );

            let many = |ids: std::ops::Range<u64>| Message::Add(ids.map(doorway_form).collect());
            assert!(send(game, many(10..210)).is_empty());
            let refused = send(game, many(210..270));
            assert_eq!(game.passages.held.len(), 256);
            assert_eq!(refused.len(), 5);
            assert!(refused.iter().all(|(_, why)| *why == HeldRefusal::Full));
            assert_eq!(refused[0].0, 265);
            assert!(send(game, many(10..11)).is_empty(), "a number sent again");

            send(game, Message::Remove(vec![1]));
            assert_eq!(game.passages.held.len(), 255);
            game.client_chunk_unload(1, 0, true);
            assert_eq!(game.passages.held.len(), 255);
            game.client_chunk_unload(0, 0, true);
            assert!(game.passages.held.is_empty());
        });
    }
}
