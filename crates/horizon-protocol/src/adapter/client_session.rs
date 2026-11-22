//! One connection from a client of an older release to a Horizon server: [`super::Session`] run the other
//! way (roadmap `older-clients`). What the client sends is renumbered to the pinned ids before the server
//! reads it; what the server sends is renumbered to the client's, registry ids included, with a stand-in
//! ([`ids::Substitutes`]) for a block, item, entity, particle or sound the client's version lacks.
//!
//! This layer maps ids; the world packets whose layout changed between 26.2 and the pinned release are
//! written in 26.2's layout by [`world`]. Serverbound, 26.2's `accept_teleportation`, `player_action` and
//! `swing` are read as 26.3's (the mirror of `translate`'s writers); other packets whose layout changed
//! still pass in the pinned layout.
//!
//! The states change where vanilla's server-side `Connection` switches its codecs, after the handshake the
//! server read itself: `login_acknowledged` and `login_finished` move each direction to configuration,
//! `finish_configuration` to play, and `configuration_acknowledged` / `start_configuration` back.
use std::{
    cell::Cell,
    collections::{HashMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
};

mod step_1_20_6;
mod step_1_21_1;
mod step_1_21_10;
mod step_1_21_11;
mod step_1_21_3;
mod step_1_21_4;
mod step_1_21_5;
mod step_1_21_6;
mod step_1_21_8;
mod step_26_1;
mod sync_registries;
mod synced;
mod text_down;
mod world;

use super::{
    ServerVersion,
    ids::{self, Ids, Way, down_or_substitute},
    layout_26_2,
    pinned_ids::entity_type::EXPERIENCE_ORB,
    translate::{packet_ids_entity::FALLING_BLOCK, swing_id},
};
use crate::{
    Result, State,
    buf::{Codec, Reader, Writer},
    entity_data::{ParticleData, Value, serializer},
    ids as packet_ids,
    packets::{configuration, play},
};

fn to_u8(state: State) -> u8 {
    match state {
        State::Handshake => 0,
        State::Status => 1,
        State::Login => 2,
        State::Configuration => 3,
        State::Play => 4,
    }
}
fn from_u8(v: u8) -> State {
    match v {
        1 => State::Status,
        2 => State::Login,
        3 => State::Configuration,
        4 => State::Play,
        _ => State::Handshake,
    }
}

/// Shared by a connection's reader and writer.
pub struct ClientSession {
    pub version: ServerVersion,
    /// What the client sends.
    reading: AtomicU8,
    /// What the server sends.
    writing: AtomicU8,
    /// Where the client last said or was told it is, for the place 26.3's `accept_teleportation` states and
    /// 26.2's leaves out.
    place: Mutex<Place>,
    /// Teleports (`player_position`) sent and not yet accepted, oldest first.
    teleports: Mutex<VecDeque<Teleport>>,
    /// The entities the client was sent, by network id: their pinned type and UUID, for the older
    /// releases' entity data and 26.1's spectating by UUID.
    entities: Mutex<HashMap<i32, (i32, u128)>>,
    /// The hotbar slot the client holds (0..=8), for the swap a 1.21.3 client's `pick_item` becomes.
    held: AtomicU8,
    /// Pinned ids -> the client's of the synchronised registries that lost entries its release lacks
    /// (synced.rs).
    synced: Mutex<Arc<synced::IdMaps>>,
    /// Where the client said it is in its last `accept_teleportation`, until its next move: an older
    /// client follows the accept with a `move_player_pos_rot` to the same place, which 26.3's accept
    /// (it carries the place) made redundant, and which counts as the tick's move on the server. Other
    /// packets (a keep-alive answer, a swing) may come between them.
    accepted: Mutex<Option<Place>>,
    /// A client through 1.21.4 is told an experience orb's value by its own packet (`add_experience_orb`): the
    /// orb's add waits here for the entity data that carries the value (the one state a packet leaves for the
    /// next).
    orb: Mutex<Option<play::AddEntity>>,
    /// A packet to pass on after the one just translated ([`Self::next_serverbound`]).
    pending: Mutex<Option<(i32, Vec<u8>)>>,
}

/// A position, yaw and pitch.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Place {
    position: [f64; 3],
    yaw: f32,
    pitch: f32,
}

/// A `player_position` the server sent.
#[derive(Clone, Copy, Debug)]
struct Teleport {
    id: i32,
    to: Place,
    relatives: i32,
    /// A 1.21.2/1.21.3 client's echo of it was dropped ([`ClientSession::echo_before_accept`]).
    echoed: bool,
}

impl Teleport {
    /// Where a client at `from` is once it took this teleport, as `LocalPlayer` applies
    /// `ClientboundPlayerPositionPacket`'s relative flags.
    fn applied(&self, from: Place) -> Place {
        use play::relative as rel;
        let pick = |bit: i32, base: f64, v: f64| {
            if self.relatives & bit != 0 {
                base + v
            } else {
                v
            }
        };
        let angle = |bit: i32, base: f32, v: f32| {
            if self.relatives & bit != 0 {
                base + v
            } else {
                v
            }
        };
        Place {
            position: [
                pick(rel::X, from.position[0], self.to.position[0]),
                pick(rel::Y, from.position[1], self.to.position[1]),
                pick(rel::Z, from.position[2], self.to.position[2]),
            ],
            yaw: angle(rel::Y_ROT, from.yaw, self.to.yaw),
            pitch: angle(rel::X_ROT, from.pitch, self.to.pitch).clamp(-90.0, 90.0),
        }
    }
}

thread_local! {
    /// The client's ids while [`ClientSession::older_layout_down`] runs: its steps re-read payloads already in them.
    static CLIENT_IDS: Cell<Option<&'static Ids>> = const { Cell::new(None) };
}

/// Restores the previous client ids when dropped.
struct ClientIds(Option<&'static Ids>);
impl Drop for ClientIds {
    fn drop(&mut self) {
        CLIENT_IDS.set(self.0);
    }
}

/// Runs `read` on a payload the renumbering already wrote in the client's ids, reading them as the pinned ones
/// (a particle's data, say, is chosen by its pinned id).
pub(super) fn read_in_client_ids<T>(read: impl FnOnce() -> Result<T>) -> Result<T> {
    let _guard = CLIENT_IDS.get().map(|ids| ids::renumbering(ids, Way::Up));
    // Its synchronised registry ids too, which [`write_in_client_ids`] renumbers again.
    let _synced = ids::synced_writing().map(ids::synced_reading);
    read()
}

/// Runs `write`, writing pinned ids as the client's again (the other half of [`read_in_client_ids`]).
pub(super) fn write_in_client_ids(write: impl FnOnce(&mut Writer)) -> Vec<u8> {
    let _guard = CLIENT_IDS.get().map(|ids| ids::renumbering(ids, Way::Down));
    let mut w = Writer::new();
    write(&mut w);
    w.data
}

impl ClientSession {
    /// A client whose login handshake stated `version`'s protocol: both directions are in login.
    pub fn new(version: ServerVersion) -> Self {
        Self {
            version,
            reading: AtomicU8::new(to_u8(State::Login)),
            writing: AtomicU8::new(to_u8(State::Login)),
            place: Mutex::new(Place::default()),
            teleports: Mutex::new(VecDeque::new()),
            entities: Mutex::new(HashMap::new()),
            held: AtomicU8::new(0),
            synced: Mutex::new(Arc::default()),
            accepted: Mutex::new(None),
            orb: Mutex::new(None),
            pending: Mutex::new(None),
        }
    }

    /// A client before 1.21.2 has no `client_tick_end`; it sends at most one movement per tick, so the end of
    /// its tick follows each movement (the server allows one movement per client tick).
    fn tick_ended(&self, pinned: i32) {
        if self.version.at_most(ServerVersion::V1_21_1) {
            let _ = pinned;
            *self.pending.lock().unwrap_or_else(|e| e.into_inner()) =
                Some((packet_ids::play::serverbound::CLIENT_TICK_END, Vec::new()));
        }
    }

    /// A packet the translation of the last one added after it: the server reads it next.
    pub fn next_serverbound(&self) -> Option<(i32, Vec<u8>)> {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }
    fn hold(&self, slot: i32) {
        if let Ok(slot @ 0..=8) = u8::try_from(slot) {
            self.held.store(slot, Ordering::Release);
        }
    }

    /// A 1.21.3 (or older) client's `pick_item`: it found the picked block's item at `slot` of its inventory and
    /// asks for it in its hand. 26.3 has no such packet (its client names the block or entity instead), so it
    /// becomes the swap of that slot with the held hotbar slot, a `container_click` on the inventory. Its state id
    /// is 0: the server applies the click and sends the inventory's full state back.
    fn pick_item(&self, payload: &[u8]) -> Result<Option<(i32, Vec<u8>)>> {
        let slot = Reader::new(payload).varint()?;
        // Inventory index to the inventory menu's slot: the hotbar (0..=8) is 36..=44, the rest 9..=35 alike.
        let slot = match slot {
            0..=8 => slot + 36,
            9..=35 => slot,
            _ => return Ok(None),
        };
        let held = self.held.load(Ordering::Acquire);
        if slot == 36 + i32::from(held) {
            return Ok(None);
        }
        let click = play::Serverbound::ContainerClick(play::ContainerClick {
            container: 0,
            state: 0,
            slot: slot as i16,
            button: held as i8,
            input: 2,
            changed: Vec::new(),
            carried: None,
        });
        Ok(Some((
            packet_ids::play::serverbound::CONTAINER_CLICK,
            strip_id(click.encode()),
        )))
    }

    pub fn reading(&self) -> State {
        from_u8(self.reading.load(Ordering::Acquire))
    }
    pub fn writing(&self) -> State {
        from_u8(self.writing.load(Ordering::Acquire))
    }

    /// A packet the client sent, with the id the pinned version knows it by. `None` drops it: the pinned
    /// version has no such packet.
    pub fn serverbound(&self, id: i32, payload: Vec<u8>) -> Result<Option<(i32, Vec<u8>)>> {
        let state = self.reading();
        // 26.3's punch took over the older swing of the main hand; an off-hand swing has no packet there.
        if state == State::Play && id == swing_id(self.version) {
            let hand = Reader::new(&payload).varint()?;
            return Ok((hand == 0).then_some((packet_ids::play::serverbound::PUNCH, Vec::new())));
        }
        if state == State::Play && Some(id) == old_pick_item_id(self.version) {
            return self.pick_item(&payload);
        }
        let Some(pinned) = self.version.serverbound_id_up(state, id) else {
            return Ok(None);
        };
        use packet_ids::{configuration::serverbound as cfg, login::serverbound as login};
        let next = match (state, pinned) {
            (State::Login, login::LOGIN_ACKNOWLEDGED) => State::Configuration,
            (State::Configuration, cfg::FINISH_CONFIGURATION) => State::Play,
            (State::Play, packet_ids::play::serverbound::CONFIGURATION_ACKNOWLEDGED) => {
                State::Configuration
            }
            _ => state,
        };
        self.reading.store(to_u8(next), Ordering::Release);
        // The stacks it names carry its own synchronised registry ids (enchantments, trims, banner patterns,
        // painting variants): read as the pinned ones.
        let maps = self
            .synced
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let _synced = (!maps.is_empty()).then(|| ids::synced_reading(maps));
        let Some((pinned, payload)) = self.older_layout_up(state, pinned, payload)? else {
            return Ok(None);
        };
        use packet_ids::play::serverbound as sb;
        if (state, pinned) == (State::Play, sb::MOVE_PLAYER_POS_ROT)
            && self.version == ServerVersion::V1_21_3
            && self.echo_before_accept(&payload)
        {
            return Ok(None);
        }
        // The first move after an accept is its echo or shows that none is coming (the client sends the echo
        // before its next tick's move): either way the accepted place is done with.
        if state == State::Play
            && matches!(
                pinned,
                sb::MOVE_PLAYER_POS | sb::MOVE_PLAYER_POS_ROT | sb::MOVE_PLAYER_ROT
            )
        {
            let accepted = self
                .accepted
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take();
            if pinned == sb::MOVE_PLAYER_POS_ROT
                && accepted.is_some_and(|at| same_place(&payload, at))
            {
                return Ok(None);
            }
        }
        let payload = match (state, pinned) {
            (State::Configuration, packet_ids::configuration::serverbound::SELECT_KNOWN_PACKS)
                if synced::own_core_pack(self.version) =>
            {
                synced::core_pack_up(self.version, &payload)?
            }
            (State::Play, sb::ACCEPT_TELEPORTATION) => {
                let accept = self.accept_teleport(&payload)?;
                // 1.21.2/1.21.3 sent the echo before the accept (echo_before_accept).
                if self.version != ServerVersion::V1_21_3 {
                    *self.accepted.lock().unwrap_or_else(|e| e.into_inner()) =
                        Some(*self.place.lock().unwrap_or_else(|e| e.into_inner()));
                }
                accept
            }
            // 26.3 inserted CHANGE_DESTROY_DIRECTION at 1: 26.2's actions past 0 are one up.
            (State::Play, sb::PLAYER_ACTION) => {
                let mut r = Reader::new(&payload);
                let action = r.varint()?;
                let mut w = Writer::new();
                w.varint(if action == 0 {
                    0
                } else {
                    action.wrapping_add(1)
                });
                w.bytes(r.rest());
                w.data
            }
            (State::Play, sb::SET_CARRIED_ITEM) => {
                self.hold(i32::from(Reader::new(&payload).i16()?));
                payload
            }
            (State::Play, sb::MOVE_PLAYER_POS | sb::MOVE_PLAYER_POS_ROT | sb::MOVE_PLAYER_ROT) => {
                self.moved(pinned, &payload)?;
                self.tick_ended(pinned);
                payload
            }
            (State::Play, sb::MOVE_PLAYER_STATUS_ONLY) => {
                self.tick_ended(pinned);
                payload
            }
            (State::Play, sb::SIGN_UPDATE) => sign_update_up(&payload)?,
            _ => payload,
        };
        let Some(ids) = self.version.ids() else {
            return Ok(Some((pinned, payload)));
        };
        let payload = match (state, pinned) {
            // The stacks a client names carry its item and component ids.
            (State::Play, sb::SET_CREATIVE_MODE_SLOT | sb::CONTAINER_CLICK) => {
                let p = {
                    let _up = ids::renumbering(ids, Way::Up);
                    play::Serverbound::decode(pinned, &payload)?
                };
                strip_id(p.encode())
            }
            _ => payload,
        };
        Ok(Some((pinned, payload)))
    }

    /// A packet body (VarInt id + payload) the server sends, as the client numbers it. `None`: the client
    /// has no such packet, or nothing it could show for what the packet names, so nothing is sent. A packet
    /// that becomes several (a wooden boat, 1.21.1 and older) gives only the first here: the connection uses
    /// [`Self::clientbound_all`].
    pub fn clientbound(&self, body: &[u8]) -> Result<Option<Vec<u8>>> {
        Ok(self.translate(body)?.0)
    }

    /// Every body a packet becomes for the client, in the order to send them: empty when [`Self::clientbound`]
    /// would give `None` and nothing came with it, the one body for most packets, and for a packet that brings
    /// another (a boat's wood in entity data) the other after it.
    pub fn clientbound_all(&self, body: &[u8]) -> Result<Vec<Vec<u8>>> {
        let (first, rest) = self.translate(body)?;
        Ok(first.into_iter().chain(rest).collect())
    }

    fn translate(&self, body: &[u8]) -> Result<(Option<Vec<u8>>, Vec<Vec<u8>>)> {
        let state = self.writing();
        self.clientbound_with_extras(state, body).map_err(|e| {
            let id = Reader::new(body).varint().unwrap_or(-1);
            crate::Error::new(format!(
                "{e} (packet {id} in {state:?} for a {} client)",
                self.version.name()
            ))
        })
    }

    /// [`Self::clientbound_in`], for the packets that become several or wait for another: a wooden boat (1.21.1
    /// and older) and an experience orb (1.21.4 and older). The second part is what goes after the first.
    fn clientbound_with_extras(
        &self,
        state: State,
        body: &[u8],
    ) -> Result<(Option<Vec<u8>>, Vec<Vec<u8>>)> {
        use packet_ids::play::clientbound as cb;
        if state != State::Play {
            return Ok((self.clientbound_in(state, body)?, Vec::new()));
        }
        let mut r = Reader::new(body);
        let id = r.varint()?;
        let payload = r.rest();
        if self.version.at_most(ServerVersion::V1_21_4) {
            let waiting = self.orb.lock().unwrap_or_else(|e| e.into_inner()).take();
            let orb = |add: &play::AddEntity, value: i32| {
                self.entities
                    .lock()
                    .unwrap()
                    .insert(add.id, (add.kind, add.uuid));
                let mut w = Writer::new();
                // add_experience_orb has been id 2 in every release through 1.21.4 (the id tables name no such
                // packet)
                w.varint(2);
                w.varint(add.id);
                add.position.iter().for_each(|v| w.f64(*v));
                w.i16(value.clamp(0, i32::from(i16::MAX)) as i16);
                w.data
            };
            if let Some(add) = waiting {
                if id == cb::SET_ENTITY_DATA {
                    let mut d = play::SetEntityData::read(&mut Reader::new(payload))?;
                    if d.id == add.id {
                        let value = d.values.iter().find_map(|(index, _, v)| match v {
                            Value::Int(v) if *index == 8 => Some(*v),
                            _ => None,
                        });
                        d.values.retain(|(index, _, _)| *index != 8);
                        let mut after = Vec::new();
                        if !d.values.is_empty()
                            && let Some(rest) = self.clientbound_in(
                                state,
                                &play::Clientbound::SetEntityData(d).encode(),
                            )?
                        {
                            after.push(rest);
                        }
                        return Ok((Some(orb(&add, value.unwrap_or(0))), after));
                    }
                }
                // Anything else shows the orb with no value first.
                let first = orb(&add, 0);
                let (next, rest) = self.clientbound_with_extras(state, body)?;
                return Ok((Some(first), next.into_iter().chain(rest).collect()));
            }
            if id == cb::ADD_ENTITY {
                let e = play::AddEntity::read(&mut Reader::new(payload))?;
                if e.kind == EXPERIENCE_ORB {
                    *self.orb.lock().unwrap_or_else(|e| e.into_inner()) = Some(e);
                    return Ok((None, Vec::new()));
                }
            }
        }
        // 1.21.2 gave every wood its own boat type; 1.21.1 has the plain one and the wood in entity data.
        if id == cb::ADD_ENTITY && self.version.at_most(ServerVersion::V1_21_1) {
            let e = play::AddEntity::read(&mut Reader::new(payload))?;
            if let Some((plain, wood)) = step_1_21_1::boat(e.kind) {
                let entity = e.id;
                let sent = self.clientbound_in(
                    state,
                    &play::Clientbound::AddEntity(play::AddEntity { kind: plain, ..e }).encode(),
                )?;
                let mut extra = Vec::new();
                if let Some(ids) = self.version.ids()
                    && let Some(mapped) =
                        self.version.clientbound_id_down(state, cb::SET_ENTITY_DATA)
                {
                    let data = play::Clientbound::SetEntityData(play::SetEntityData {
                        id: entity,
                        values: vec![(11, serializer::INT, Value::Int(wood))],
                    });
                    let mut w = Writer::new();
                    w.varint(mapped);
                    w.bytes(&encode_down(ids, &data));
                    extra.push(w.data);
                }
                return Ok((sent, extra));
            }
        }
        Ok((self.clientbound_in(state, body)?, Vec::new()))
    }

    fn clientbound_in(&self, state: State, body: &[u8]) -> Result<Option<Vec<u8>>> {
        let mut r = Reader::new(body);
        let id = r.varint()?;
        let payload = r.rest();
        use packet_ids::{configuration::clientbound as cfg, login::clientbound as login};
        let next = match (state, id) {
            (State::Login, login::LOGIN_FINISHED) => State::Configuration,
            (State::Configuration, cfg::FINISH_CONFIGURATION) => State::Play,
            (State::Play, packet_ids::play::clientbound::START_CONFIGURATION) => {
                State::Configuration
            }
            _ => state,
        };
        self.writing.store(to_u8(next), Ordering::Release);
        let rebiomed;
        // Items and entity data written below take the client's synchronised registry ids.
        let maps: Arc<synced::IdMaps> = self
            .synced
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let _synced = (!maps.is_empty()).then(|| ids::synced(maps));
        let payload = match self.synced_down(state, id, payload)? {
            // A registry the client's release doesn't synchronise is not sent.
            Some(p) if p.is_empty() => return Ok(None),
            Some(p) => {
                rebiomed = p;
                &rebiomed[..]
            }
            None => payload,
        };
        // 1.21.1 has no entity_position_sync: the teleport it stands in for is sent under teleport_entity's id.
        let synced = if self.version.at_most(ServerVersion::V1_21_1) {
            step_1_21_1::entity_position_sync(state, id, payload)?
        } else {
            None
        };
        let (id, payload) = match &synced {
            Some(p) => (packet_ids::play::clientbound::TELEPORT_ENTITY, &p[..]),
            None => (id, payload),
        };
        // 26.2 has no swing_animation: its hands swing through animate (world.rs).
        if state == State::Play
            && id == packet_ids::play::clientbound::SWING_ANIMATION
            && self.version.ids().is_some()
        {
            let animate = packet_ids::play::clientbound::ANIMATE;
            let Some(mapped) = self.version.clientbound_id_down(state, animate) else {
                return Ok(None);
            };
            let mut w = Writer::new();
            w.varint(mapped);
            w.bytes(&world::swing(payload)?);
            return Ok(Some(w.data));
        }
        if (state, id) == (State::Play, packet_ids::play::clientbound::PLAYER_POSITION) {
            self.teleporting(payload)?;
        }
        if (state, id) == (State::Play, packet_ids::play::clientbound::SET_HELD_SLOT) {
            self.hold(Reader::new(payload).varint()?);
        }
        let tracked;
        let payload = match self.track_entities(state, id, payload)? {
            Some(p) => {
                tracked = p;
                &tracked[..]
            }
            None => payload,
        };
        let Some(mapped) = self.version.clientbound_id_down(state, id) else {
            return Ok(None);
        };
        // A packet the renumbering can't read goes as it is, rather than the connection dropping here.
        let rewritten = match self
            .version
            .ids()
            .map(|ids| layout_then_ids(self.version, ids, state, id, payload))
        {
            None | Some(Ok(Sent::AsIs)) => None,
            Some(Ok(Sent::Rewritten(payload))) => Some(payload),
            Some(Ok(Sent::Dropped)) => return Ok(None),
            Some(Err(e)) => {
                log::warn!(
                    "Horizon: could not renumber packet {id} ({state:?}) for a {} client: {e}",
                    self.version.name()
                );
                None
            }
        };
        let out = rewritten.unwrap_or_else(|| payload.to_vec());
        let Some(out) = self.older_layout_down(state, id, out)? else {
            return Ok(None);
        };
        let mut w = Writer::new();
        w.varint(mapped);
        w.bytes(&out);
        Ok(Some(w.data))
    }

    /// Keeps the entities the client is sent (`add_entity`, `remove_entities`), and writes a pinned
    /// `set_entity_data` in 1.21.11's or 1.21.10's entity data (still in pinned ids, which [`down`]
    /// renumbers): `Some` when it changed.
    fn track_entities(&self, state: State, id: i32, payload: &[u8]) -> Result<Option<Vec<u8>>> {
        use packet_ids::play::clientbound as cb;
        if state != State::Play
            || !matches!(
                id,
                cb::ADD_ENTITY | cb::REMOVE_ENTITIES | cb::SET_ENTITY_DATA | cb::LOGIN
            )
        {
            return Ok(None);
        }
        let mut p = play::Clientbound::decode(id, payload)?;
        let mut entities = self.entities.lock().unwrap();
        match &mut p {
            play::Clientbound::AddEntity(e) => {
                entities.insert(e.id, (e.kind, e.uuid));
            }
            play::Clientbound::RemoveEntities(r) => {
                for id in &r.ids {
                    entities.remove(id);
                }
            }
            // The local player is never sent an add_entity: its entity data still needs the player's step.
            play::Clientbound::Login(l) => {
                entities.insert(
                    l.player_id,
                    (crate::adapter::pinned_ids::entity_type::PLAYER, 0),
                );
            }
            play::Clientbound::SetEntityData(d)
                if self.version.at_most(ServerVersion::V1_21_11) =>
            {
                let kind = entities.get(&d.id).map(|&(kind, _)| kind);
                step_1_21_11::entity_data(kind, d);
                if self.version.at_most(ServerVersion::V1_21_10) {
                    step_1_21_10::entity_data(kind, d);
                }
                if self.version.at_most(ServerVersion::V1_21_8) {
                    step_1_21_8::entity_data(kind, d);
                }
                if self.version.at_most(ServerVersion::V1_21_5) {
                    step_1_21_5::entity_data(kind, d);
                }
                // 1.21.5's indexes go back to 1.21.4's last.
                if self.version.at_most(ServerVersion::V1_21_4) {
                    step_1_21_4::entity_data(kind, d);
                }
                if self.version.at_most(ServerVersion::V1_21_1) {
                    step_1_21_1::boat_data(kind, d);
                }
                return Ok(Some(strip_id(p.encode())));
            }
            _ => {}
        }
        Ok(None)
    }

    /// A packet in 26.2's layout (and the client's ids) brought down to the client's own release, a step at a
    /// time: `None` when that release has no such packet.
    fn older_layout_down(
        &self,
        state: State,
        id: i32,
        mut payload: Vec<u8>,
    ) -> Result<Option<Vec<u8>>> {
        let v = self.version;
        let _client_ids = ClientIds(CLIENT_IDS.replace(v.ids()));
        // 1.21.2 reworked the recipe book (recipe displays, no ids a 1.21.1 client knows): a 1.21.1 client gets no
        // recipe book, so its update_recipes and place_ghost_recipe are not sent. Its recipe_book_add, _remove and
        // _settings have no 1.21.1 id at all (the id tables drop them).
        if v.at_most(ServerVersion::V1_21_1)
            && state == State::Play
            && matches!(
                id,
                packet_ids::play::clientbound::UPDATE_RECIPES
                    | packet_ids::play::clientbound::PLACE_GHOST_RECIPE
            )
        {
            return Ok(None);
        }
        // 1.21.11 has world clocks in no registry (26.1 synced them): its client is not sent that one.
        if v.at_most(ServerVersion::V1_21_11)
            && (state, id)
                == (
                    State::Configuration,
                    packet_ids::configuration::clientbound::REGISTRY_DATA,
                )
            && Reader::new(&payload).string(32767)? == "minecraft:world_clock"
        {
            return Ok(None);
        }
        // 26.3's explode carries the radius right after the centre; 1.21.8 dropped it, so it is kept here for
        // 1.21.1's strength (written at the same place, after the centre).
        let explode_radius = (v.at_most(ServerVersion::V1_21_1)
            && (state, id) == (State::Play, packet_ids::play::clientbound::EXPLODE))
            .then(|| payload.get(24..28).map(|b| [b[0], b[1], b[2], b[3]]))
            .flatten();
        type Step = fn(State, i32, &[u8]) -> Result<Option<Vec<u8>>>;
        let steps: [(ServerVersion, Step); 4] = [
            (ServerVersion::V26_1, step_26_1::clientbound),
            (ServerVersion::V1_21_11, step_1_21_11::clientbound),
            (ServerVersion::V1_21_10, step_1_21_10::clientbound),
            (ServerVersion::V1_21_8, step_1_21_8::clientbound),
        ];
        for (to, step) in steps {
            if v.at_most(to)
                && let Some(next) = step(state, id, &payload)?
            {
                payload = next;
            }
        }
        // 1.21.8 is newer than 1.21.6: its packets are brought down after the steps above.
        if v.at_most(ServerVersion::V1_21_6) {
            // 1.21.5 and 1.21.6 have the saddle slot; 1.21.4 doesn't.
            match step_1_21_6::clientbound(state, id, &payload, !v.at_most(ServerVersion::V1_21_4))?
            {
                step_1_21_6::Out::Untouched => {}
                step_1_21_6::Out::Rewritten(p) => payload = p,
                step_1_21_6::Out::Dropped => return Ok(None),
            }
        }
        // 1.21.5 is older than 1.21.6 (and than every step above): its packets are brought down last.
        if v.at_most(ServerVersion::V1_21_5)
            && let Some(next) = step_1_21_5::clientbound(state, id, &payload)?
        {
            payload = next;
        }
        // 1.21.4 is older than 1.21.5 too: after its step, the last one.
        if v.at_most(ServerVersion::V1_21_4)
            && let Some(next) = step_1_21_4::clientbound(state, id, &payload)?
        {
            payload = next;
        }
        // 1.21.2 and 1.21.3 (one protocol): 1.21.4's two additions go.
        if v == ServerVersion::V1_21_3
            && let Some(next) = step_1_21_3::clientbound(state, id, &payload)?
        {
            payload = next;
        }
        // 1.21.1 is older than 1.21.2 and than 1.21.4: its packets are brought down after those, and before 1.20.6's.
        if v.at_most(ServerVersion::V1_21_1)
            && let Some(next) = step_1_21_1::clientbound(state, id, &payload)?
        {
            payload = next;
        }
        if let Some(radius) = explode_radius
            && let Some(strength) = payload.get_mut(24..28)
        {
            strength.copy_from_slice(&radius);
        }
        // 1.20.6 is the oldest release: its packets are brought down after every step above.
        if v.at_most(ServerVersion::V1_20_6)
            && let Some(ids) = v.ids()
            && let Some(next) = step_1_20_6::clientbound(state, id, &payload, ids)?
        {
            payload = next;
        }
        Ok(Some(payload))
    }

    /// A packet an older client sent (already under its pinned id) in 26.2's layout, oldest step first:
    /// `None` drops it.
    fn older_layout_up(
        &self,
        state: State,
        pinned: i32,
        mut payload: Vec<u8>,
    ) -> Result<Option<(i32, Vec<u8>)>> {
        use packet_ids::play::serverbound as sb;
        let v = self.version;
        // 1.21.2 named recipes by a display id; a 1.21.1 client sends the recipe's key (a string) in place_recipe
        // and recipe_book_seen_recipe, which the pinned readers take for a VarInt. It gets no recipe book (see
        // `older_layout_down`), so these are not passed on.
        if v.at_most(ServerVersion::V1_21_1)
            && state == State::Play
            && matches!(pinned, sb::PLACE_RECIPE | sb::RECIPE_BOOK_SEEN_RECIPE)
        {
            return Ok(None);
        }
        // 1.20.6 is the oldest release: its use_item is read first, with the client's last rotation.
        if v.at_most(ServerVersion::V1_20_6) {
            let (yaw, pitch) = {
                let place = self.place.lock().unwrap();
                (place.yaw, place.pitch)
            };
            if let Some(next) = step_1_20_6::serverbound(state, pinned, &payload, (yaw, pitch))? {
                payload = next;
            }
        }
        // 1.21.1 is older than 1.21.2 and 1.21.4: its movement and steering are read before theirs (step_1_21_1).
        if v.at_most(ServerVersion::V1_21_1)
            && let Some(p) = step_1_21_1::serverbound(state, pinned, &payload)?
        {
            payload = p;
        }
        // 1.21.4 is the oldest step: its packets are read before 1.21.5's.
        if v.at_most(ServerVersion::V1_21_4) {
            match step_1_21_4::serverbound(state, pinned, &payload, v.ids())? {
                step_1_21_4::Out::Untouched => {}
                step_1_21_4::Out::Rewritten(p) => payload = p,
            }
        }
        // 1.21.5 is the next oldest: its packets are read before 1.21.6's.
        if v.at_most(ServerVersion::V1_21_5) {
            match step_1_21_5::serverbound(state, pinned, &payload)? {
                step_1_21_5::Out::Untouched => {}
                step_1_21_5::Out::Rewritten(p) => payload = p,
                step_1_21_5::Out::Dropped => return Ok(None),
            }
        }
        // 1.21.6 is the oldest of these: its packets are read first.
        if v.at_most(ServerVersion::V1_21_6) {
            match step_1_21_6::serverbound(state, pinned, &payload)? {
                step_1_21_6::Out::Untouched => {}
                step_1_21_6::Out::Rewritten(p) => payload = p,
                step_1_21_6::Out::Dropped => return Ok(None),
            }
        }
        if v.at_most(ServerVersion::V1_21_11) && (state, pinned) == (State::Play, sb::INTERACT) {
            let mut r = Reader::new(&payload);
            let _entity = r.varint()?;
            return Ok(match r.varint()? {
                // INTERACT follows each INTERACT_AT; 26.3's interact is the latter alone.
                0 => None,
                1 => Some((sb::ATTACK, step_1_21_11::attack(&payload)?)),
                _ => {
                    let p = step_1_21_11::serverbound(state, pinned, &payload)?;
                    Some((pinned, p.unwrap_or(payload)))
                }
            });
        }
        if v.at_most(ServerVersion::V26_1) {
            let entities = self.entities.lock().unwrap();
            let entity_of = |uuid: u128| {
                entities
                    .iter()
                    .find(|(_, (_, u))| *u == uuid)
                    .map(|(&id, _)| id)
            };
            match step_26_1::serverbound(state, pinned, &payload, entity_of)? {
                step_26_1::Out::Untouched => {}
                step_26_1::Out::Rewritten(p) => return Ok(Some((pinned, p))),
                step_26_1::Out::Dropped => return Ok(None),
            }
        }
        Ok(Some((pinned, payload)))
    }
}

/// An older client's serverbound `sign_update` (position, the side as a boolean, the four lines) in
/// 26.3's order (position, the four lines, the side as a VarInt): the mirror of translate's `sign_update`.
fn sign_update_up(payload: &[u8]) -> Result<Vec<u8>> {
    let mut r = Reader::new(payload);
    let pos = r.i64()?;
    let front = r.bool()?;
    let mut w = Writer::new();
    w.i64(pos);
    for _ in 0..4 {
        w.string(&r.string(384)?);
    }
    // 26.3's decoder refuses trailing bytes, so an older client's packet must not reach it with any.
    if r.remaining() != 0 {
        return Err(crate::Error::new(
            "sign_update with bytes after its four lines",
        ));
    }
    w.varint(i32::from(front));
    Ok(w.data)
}

/// A 26.2 client's packets whose layout changed are written in its layout ([`layout_26_2`], when enabled),
/// the others renumbered by [`down`].
fn layout_then_ids(
    version: ServerVersion,
    ids: &'static Ids,
    state: State,
    id: i32,
    payload: &[u8],
) -> Result<Sent> {
    if state == State::Play
        && id == packet_ids::play::clientbound::COMMANDS
        && version.at_most(ServerVersion::V26_2)
    {
        let absent = |registry: &str| synced::absent(version, registry);
        return Ok(Sent::Rewritten(layout_26_2::commands(
            ids, absent, payload,
        )?));
    }
    if version.at_most(ServerVersion::V26_2) && layout_26_2::enabled() {
        match layout_26_2::clientbound(ids, state, id, payload)? {
            layout_26_2::Out::Untouched => {}
            layout_26_2::Out::Rewritten(p) => return Ok(Sent::Rewritten(p)),
            layout_26_2::Out::Dropped => return Ok(Sent::Dropped),
        }
    }
    down(ids, state, id, payload)
}

impl ClientSession {
    /// The client's own core pack offered, the registries its release lacks left out, its registries without
    /// the entries it lacks, and the ids into those renumbered (synced.rs, #15138). `None`: the pinned payload
    /// goes on as it is; empty: nothing is sent.
    fn synced_down(&self, state: State, id: i32, payload: &[u8]) -> Result<Option<Vec<u8>>> {
        use packet_ids::{configuration::clientbound as cfg, play::clientbound as cb};
        if !synced::own_core_pack(self.version) {
            return Ok(None);
        }
        let maps = || {
            self.synced
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
        };
        Ok(match (state, id) {
            (State::Configuration, cfg::REGISTRY_DATA)
                if synced::absent_registry(self.version, payload)? =>
            {
                // Not sent; a registry the client has built in is still renumbered by what the server names.
                if let Some((registry, m)) = synced::built_in(self.version, payload)? {
                    let mut guard = self.synced.lock().unwrap_or_else(|e| e.into_inner());
                    Arc::make_mut(&mut guard).insert(registry, m);
                }
                return Ok(Some(Vec::new()));
            }
            (State::Configuration, cfg::SELECT_KNOWN_PACKS) => {
                Some(synced::core_pack_down(self.version, payload)?)
            }
            (State::Configuration, cfg::REGISTRY_DATA) => {
                let renumbered = synced::registry_data(self.version, payload)?;
                let mut guard = self.synced.lock().unwrap_or_else(|e| e.into_inner());
                match renumbered {
                    Some((p, registry, m)) => {
                        Arc::make_mut(&mut guard).insert(registry, m);
                        Some(p)
                    }
                    // Nothing removed this time (sent again in a later configuration): the client numbers it
                    // as the server does, and a map from before is dropped.
                    None => {
                        if !guard.is_empty() {
                            let registry = Reader::new(payload).string(32767)?;
                            if guard.contains_key(&registry) {
                                Arc::make_mut(&mut guard).remove(&registry);
                            }
                        }
                        None
                    }
                }
            }
            (State::Configuration, cfg::UPDATE_TAGS) => {
                let mut p = configuration::Clientbound::decode(id, payload)?;
                if let configuration::Clientbound::UpdateTags(t) = &mut p {
                    synced::tags(self.version, &maps(), &mut t.registries);
                }
                Some(strip_id(p.encode()))
            }
            (State::Play, cb::UPDATE_TAGS) => {
                let mut p = play::Clientbound::decode(id, payload)?;
                if let play::Clientbound::UpdateTags(t) = &mut p {
                    synced::tags(self.version, &maps(), &mut t.registries);
                }
                Some(strip_id(p.encode()))
            }
            (State::Play, cb::LEVEL_CHUNK_WITH_LIGHT) => {
                match (maps().get(synced::BIOMES), self.version.ids()) {
                    (Some(m), Some(ids)) => {
                        Some(synced::level_chunk(m, ids.state_bits().1, payload)?)
                    }
                    _ => None,
                }
            }
            (State::Play, cb::CHUNKS_BIOMES) => match maps().get(synced::BIOMES) {
                Some(m) => Some(synced::chunks_biomes(m, payload)?),
                None => None,
            },
            (State::Play, cb::DAMAGE_EVENT) => match maps().get(synced::DAMAGE_TYPE) {
                Some(m) => Some(synced::damage_event(m, payload)?),
                None => None,
            },
            _ => None,
        })
    }
    /// A `player_position` on its way: the client accepts it later by its id alone.
    fn teleporting(&self, payload: &[u8]) -> Result<()> {
        let mut r = Reader::new(payload);
        let id = r.varint()?;
        let position = [r.f64()?, r.f64()?, r.f64()?];
        let _delta = [r.f64()?, r.f64()?, r.f64()?];
        let to = Place {
            position,
            yaw: r.f32()?,
            pitch: r.f32()?,
        };
        let relatives = r.i32()?;
        self.teleports.lock().unwrap().push_back(Teleport {
            id,
            to,
            relatives,
            echoed: false,
        });
        Ok(())
    }

    /// Whether a 1.21.2/1.21.3 client's `move_player_pos_rot` is the echo of a teleport: its
    /// `ClientPacketListener.handleMovePlayer` sends the move to the teleport's place before the
    /// `accept_teleportation` (the other releases after it). The echo names where the first teleport not yet
    /// accepted puts the client; one is dropped per teleport, and the accept that follows states the same place.
    fn echo_before_accept(&self, payload: &[u8]) -> bool {
        let place = *self.place.lock().unwrap_or_else(|e| e.into_inner());
        let mut teleports = self.teleports.lock().unwrap_or_else(|e| e.into_inner());
        match teleports.front_mut() {
            Some(t) if !t.echoed && same_place(payload, t.applied(place)) => {
                t.echoed = true;
                true
            }
            _ => false,
        }
    }

    /// A move the client reports (`move_player_pos`, `_pos_rot`, `_rot`): where it now is.
    fn moved(&self, pinned: i32, payload: &[u8]) -> Result<()> {
        use packet_ids::play::serverbound as sb;
        let mut r = Reader::new(payload);
        let mut place = self.place.lock().unwrap();
        if pinned != sb::MOVE_PLAYER_ROT {
            place.position = [r.f64()?, r.f64()?, r.f64()?];
        }
        if pinned != sb::MOVE_PLAYER_POS {
            place.yaw = r.f32()?;
            place.pitch = r.f32()?;
        }
        Ok(())
    }

    /// 26.2's `accept_teleportation` (the teleport's id) as 26.3's, which also states where the client
    /// now is: the teleport it accepts, and any sent before it, applied in order to where it was.
    fn accept_teleport(&self, payload: &[u8]) -> Result<Vec<u8>> {
        let id = Reader::new(payload).varint()?;
        let mut place = self.place.lock().unwrap();
        let mut teleports = self.teleports.lock().unwrap();
        if let Some(i) = teleports.iter().position(|t| t.id == id) {
            for t in teleports.drain(..=i) {
                *place = t.applied(*place);
            }
        }
        let mut w = Writer::new();
        w.varint(id);
        for v in place.position {
            w.f64(v);
        }
        w.f32(place.yaw);
        w.f32(place.pitch);
        Ok(w.data)
    }
}

/// Whether a `move_player_pos_rot` payload names `at` (the place an accepted teleport put the client).
fn same_place(payload: &[u8], at: Place) -> bool {
    let mut r = Reader::new(payload);
    let (Ok(x), Ok(y), Ok(z), Ok(yaw), Ok(pitch)) = (r.f64(), r.f64(), r.f64(), r.f32(), r.f32())
    else {
        return false;
    };
    let close = |a: f64, b: f64| (a - b).abs() < 1e-6;
    close(x, at.position[0])
        && close(y, at.position[1])
        && close(z, at.position[2])
        && (yaw - at.yaw).abs() < 1e-3
        && (pitch - at.pitch).abs() < 1e-3
}

/// What becomes of a clientbound payload on its way to an older client.
enum Sent {
    AsIs,
    Rewritten(Vec<u8>),
    /// It names something the client has nothing to show for.
    Dropped,
}

/// The id of `pick_item` (1.21.3 and older) for a client of `version`, from each release's packets report.
fn old_pick_item_id(version: ServerVersion) -> Option<i32> {
    if version.at_most(ServerVersion::V1_21_1) {
        Some(32)
    } else if version.at_most(ServerVersion::V1_21_3) {
        Some(34)
    } else {
        None
    }
}

fn strip_id(body: Vec<u8>) -> Vec<u8> {
    let mut r = Reader::new(&body);
    match r.varint() {
        Ok(_) => r.rest().to_vec(),
        Err(_) => body,
    }
}

/// A block state as the older client numbers it, or its stand-in's; air when it has neither.
fn state_down(ids: &Ids, id: i32) -> i32 {
    down_or_substitute(ids.block_state, ids.substitutes.block_state, id).unwrap_or(0)
}

/// A pinned play packet encoded for the older client: items and components in its numbering.
fn encode_down(ids: &'static Ids, p: &play::Clientbound) -> Vec<u8> {
    let _down = ids::renumbering(ids, Way::Down);
    strip_id(p.encode())
}

/// The registry ids in a pinned clientbound packet, renumbered for the older client.
fn down(ids: &'static Ids, state: State, id: i32, payload: &[u8]) -> Result<Sent> {
    use packet_ids::play::clientbound as cb;
    let tags = |registries: &mut [configuration::RegistryTags]| {
        for registry in registries {
            let remap = ids.by_registry(&registry.registry);
            for tag in &mut registry.tags {
                tag.ids = tag.ids.iter().filter_map(|&id| remap.down(id)).collect();
            }
        }
    };
    if (state, id)
        == (
            State::Configuration,
            packet_ids::configuration::clientbound::UPDATE_TAGS,
        )
    {
        let mut p = configuration::Clientbound::decode(id, payload)?;
        if let configuration::Clientbound::UpdateTags(t) = &mut p {
            tags(&mut t.registries);
        }
        return Ok(Sent::Rewritten(strip_id(p.encode())));
    }
    if state != State::Play {
        return Ok(Sent::AsIs);
    }
    // Packets whose layout 26.2 has differently (world.rs).
    if let Some(sent) = world::down(ids, id, payload)? {
        return Ok(sent);
    }
    match id {
        cb::UPDATE_TAGS
        | cb::ADD_ENTITY
        | cb::BLOCK_ENTITY_DATA
        | cb::BLOCK_UPDATE
        | cb::SECTION_BLOCKS_UPDATE
        | cb::LEVEL_EVENT
        | cb::BLOCK_EVENT
        | cb::UPDATE_ATTRIBUTES
        | cb::UPDATE_MOB_EFFECT
        | cb::REMOVE_MOB_EFFECT
        | cb::SOUND
        | cb::SOUND_ENTITY
        | cb::SET_ENTITY_DATA
        | cb::CONTAINER_SET_CONTENT
        | cb::CONTAINER_SET_SLOT
        | cb::SET_CURSOR_ITEM
        | cb::SET_PLAYER_INVENTORY
        | cb::SET_EQUIPMENT
        | cb::MERCHANT_OFFERS
        | cb::RECIPE_BOOK_ADD
        | cb::PLACE_GHOST_RECIPE => {}
        _ => return Ok(Sent::AsIs),
    }
    let mut p = play::Clientbound::decode(id, payload)?;
    if let play::Clientbound::UpdateTags(t) = &mut p {
        tags(&mut t.registries);
    }
    Ok(match renumber(ids, &mut p) {
        Some(()) => Sent::Rewritten(encode_down(ids, &p)),
        None => Sent::Dropped,
    })
}

/// The plain registry ids in a decoded play packet, renumbered for the older client: `None` when it names
/// something the client has nothing to show for.
fn renumber(ids: &Ids, p: &mut play::Clientbound) -> Option<()> {
    let sound = |sound: &mut (i32, Option<(String, Option<f32>)>)| match sound.1 {
        // An inline sound is named, not numbered.
        Some(_) => Some(()),
        None => {
            sound.0 = down_or_substitute(ids.sound_event, ids.substitutes.sound_event, sound.0)?;
            Some(())
        }
    };
    match p {
        play::Clientbound::AddEntity(e) => {
            if e.kind == FALLING_BLOCK {
                e.data = state_down(ids, e.data);
            }
            e.kind = down_or_substitute(ids.entity_type, ids.substitutes.entity_type, e.kind)?;
            Some(())
        }
        play::Clientbound::BlockEntityData(b) => {
            world::pot_sherds_down(ids, &mut b.tag);
            b.kind = ids.block_entity_type.down(b.kind)?;
            Some(())
        }
        play::Clientbound::BlockUpdate(b) => {
            b.state = state_down(ids, b.state);
            Some(())
        }
        play::Clientbound::SectionBlocksUpdate(s) => {
            for change in &mut s.changes {
                let state = state_down(ids, (*change >> 12) as i32);
                *change = i64::from(state) << 12 | (*change & 0xfff);
            }
            Some(())
        }
        // LevelEvent.PARTICLES_DESTROY_BLOCK and the brush's BLOCK_FINISHED_BRUSHING carry a state.
        play::Clientbound::LevelEvent(e) if matches!(e.kind, 2001 | 3008) => {
            e.data = state_down(ids, e.data);
            Some(())
        }
        play::Clientbound::BlockEvent(e) => {
            e.block = down_or_substitute(ids.block, ids.substitutes.block, e.block)?;
            Some(())
        }
        // An effect the client lacks (added after its release) is not sent.
        play::Clientbound::UpdateMobEffect(e) => {
            e.effect = ids.mob_effect.down(e.effect)?;
            Some(())
        }
        play::Clientbound::RemoveMobEffect(e) => {
            e.effect = ids.mob_effect.down(e.effect)?;
            Some(())
        }
        play::Clientbound::UpdateAttributes(a) => {
            a.values.retain_mut(|v| {
                ids.attribute
                    .down(v.attribute)
                    .map(|a| v.attribute = a)
                    .is_some()
            });
            Some(())
        }
        play::Clientbound::Sound(s) => sound(&mut s.sound),
        play::Clientbound::SoundEntity(s) => sound(&mut s.sound),
        play::Clientbound::SetEntityData(d) => {
            d.values
                .retain_mut(|(_, kind, value)| entity_value(ids, *kind, value));
            Some(())
        }
        // Item stacks and recipe displays: their writers renumber items and components (encode_down).
        _ => Some(()),
    }
}

/// One entity data value renumbered for the older client: false drops it (a particle it lacks).
fn entity_value(ids: &Ids, kind: i32, value: &mut Value) -> bool {
    let particle = |p: &mut crate::entity_data::Particle| {
        if ids.quirks.simple_spell_particles {
            p.without_plain_options();
        }
        if let Some(ParticleData::State(s)) = &mut p.data {
            *s = state_down(ids, *s);
        }
        match down_or_substitute(ids.particle_type, ids.substitutes.particle_type, p.id) {
            Some(id) => {
                p.id = id;
                true
            }
            None => false,
        }
    };
    match value {
        Value::Id(id)
            if matches!(
                kind,
                serializer::BLOCK_STATE | serializer::OPTIONAL_BLOCK_STATE
            ) =>
        {
            *id = state_down(ids, *id);
            true
        }
        // A painting variant the client lacks is its stand-in (a holder: the id plus one). 1.20.5/1.20.6
        // write the registry id itself (`ByteBufCodecs.holderRegistry`), 1.21 on a holder.
        Value::Id(holder) if kind == serializer::PAINTING_VARIANT && *holder >= 1 => {
            let id = ids::synced_down_or_first(synced::PAINTING_VARIANT, *holder - 1);
            *holder = if ids.quirks.components_before_1_21 {
                id
            } else {
                id + 1
            };
            true
        }
        // 1.21 to 1.21.4 read a wolf variant as a holder (the id plus one, 0 for an inline one); 1.21.5 made it the id.
        Value::Id(id) if kind == serializer::WOLF_VARIANT && ids.quirks.wolf_variant_holder => {
            *id += 1;
            true
        }
        Value::Particle(p) => particle(p),
        Value::Particles(list) => {
            list.retain_mut(particle);
            true
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buf::Codec;

    #[test]
    fn an_older_clients_sign_update_reaches_26_3_with_the_side_after_the_lines() {
        let mut w = Writer::new();
        w.i64(0x1234);
        w.bool(false); // back side
        for line in ["a", "bb", "", "dddd"] {
            w.string(line);
        }
        let up = sign_update_up(&w.data).unwrap();
        let mut r = Reader::new(&up);
        assert_eq!(r.i64().unwrap(), 0x1234);
        for line in ["a", "bb", "", "dddd"] {
            assert_eq!(r.string(384).unwrap(), line);
        }
        assert_eq!(r.varint().unwrap(), 0);
        assert!(r.rest().is_empty());
        let packet = play::Serverbound::decode(packet_ids::play::serverbound::SIGN_UPDATE, &up);
        assert!(packet.is_ok(), "{packet:?}");
    }

    #[test]
    fn a_1_21_4_clients_sign_update_goes_through_the_session_into_26_3s_layout() {
        let s = ClientSession::new(ServerVersion::V1_21_4);
        s.reading.store(to_u8(State::Play), Ordering::Release);
        let old_id = ServerVersion::V1_21_4
            .serverbound_id(State::Play, packet_ids::play::serverbound::SIGN_UPDATE)
            .unwrap();
        let mut w = Writer::new();
        w.i64(0x1234);
        w.bool(false); // back side
        for line in ["a", "bb", "", "dddd"] {
            w.string(line);
        }
        let (id, payload) = s.serverbound(old_id, w.data.clone()).unwrap().unwrap();
        assert_eq!(id, packet_ids::play::serverbound::SIGN_UPDATE);
        let play::Serverbound::SignUpdate(sign) = play::Serverbound::decode(id, &payload).unwrap()
        else {
            panic!("not a sign_update");
        };
        assert_eq!(sign.lines, ["a", "bb", "", "dddd"].map(String::from));
        assert!(!sign.front);
        // 26.3 refuses bytes after the lines, so the session refuses them too.
        w.data.push(0);
        assert!(s.serverbound(old_id, w.data).is_err());
    }

    fn body(id: i32, payload: &[u8]) -> Vec<u8> {
        let mut w = Writer::new();
        w.varint(id);
        w.bytes(payload);
        w.data
    }

    fn in_play() -> ClientSession {
        let s = ClientSession::new(ServerVersion::V26_2);
        s.reading.store(to_u8(State::Play), Ordering::Release);
        s.writing.store(to_u8(State::Play), Ordering::Release);
        s
    }

    // Expected ids are read off the 26.2 and 26.3 packets and registries reports by hand, not from the
    // generated tables.

    /// #15138: a 26.2 client is offered `minecraft:core` 26.2 (so it loads its own registry data), its
    /// answer reaches the server as the pinned pack, and its biome registry loses `dappled_forest`.
    #[test]
    fn a_26_2_client_knows_its_own_core_pack_and_gets_its_own_biomes() {
        use packet_ids::configuration::{clientbound as cfg_cb, serverbound as cfg_sb};
        let s = ClientSession::new(ServerVersion::V26_2);
        s.reading
            .store(to_u8(State::Configuration), Ordering::Release);
        s.writing
            .store(to_u8(State::Configuration), Ordering::Release);
        let pack = |version: &str| configuration::SelectKnownPacks {
            packs: vec![configuration::KnownPack {
                namespace: "minecraft".into(),
                id: "core".into(),
                version: version.into(),
            }],
        };
        let encode = |p: &configuration::SelectKnownPacks| {
            let mut w = Writer::new();
            configuration::SelectKnownPacks::write(p, &mut w);
            w.data
        };
        let sent = s
            .clientbound(&body(
                cfg_cb::SELECT_KNOWN_PACKS,
                &encode(&pack(crate::GAME_VERSION)),
            ))
            .unwrap()
            .unwrap();
        let mut r = Reader::new(&sent);
        r.varint().unwrap();
        let offered = configuration::SelectKnownPacks::read(&mut r).unwrap();
        assert_eq!(offered, pack("26.2"));
        let old_id = ServerVersion::V26_2
            .serverbound_id(State::Configuration, cfg_sb::SELECT_KNOWN_PACKS)
            .unwrap();
        let (pinned, reply) = s
            .serverbound(old_id, encode(&pack("26.2")))
            .unwrap()
            .unwrap();
        assert_eq!(pinned, cfg_sb::SELECT_KNOWN_PACKS);
        assert_eq!(
            configuration::SelectKnownPacks::read(&mut Reader::new(&reply)).unwrap(),
            pack(crate::GAME_VERSION)
        );
        let names = [
            "minecraft:badlands",
            "minecraft:dappled_forest",
            "minecraft:the_void",
        ];
        let mut w = Writer::new();
        configuration::RegistryData::write(
            &configuration::RegistryData {
                registry: "minecraft:worldgen/biome".into(),
                entries: names
                    .iter()
                    .map(|n| configuration::RegistryEntry {
                        id: (*n).into(),
                        data: None,
                    })
                    .collect(),
            },
            &mut w,
        );
        let sent = s
            .clientbound(&body(cfg_cb::REGISTRY_DATA, &w.data))
            .unwrap()
            .unwrap();
        let mut r = Reader::new(&sent);
        r.varint().unwrap();
        let biomes = configuration::RegistryData::read(&mut r).unwrap();
        assert_eq!(
            biomes
                .entries
                .iter()
                .map(|e| e.id.as_str())
                .collect::<Vec<_>>(),
            ["minecraft:badlands", "minecraft:the_void"]
        );
    }

    #[test]
    fn an_effect_an_older_client_lacks_is_not_sent() {
        let s = ClientSession::new(ServerVersion::V1_21_1);
        s.writing.store(to_u8(State::Play), Ordering::Release);
        let effect = |effect| {
            play::Clientbound::UpdateMobEffect(play::UpdateMobEffect {
                id: 1,
                effect,
                amplifier: 0,
                duration: 100,
                flags: 0,
            })
            .encode()
        };
        // Speed (0) is known to 1.21.1; pinned 39 was added after it (1.21.1 has 39 effects).
        assert!(s.clientbound(&effect(0)).unwrap().is_some());
        assert_eq!(s.clientbound(&effect(39)).unwrap(), None);
        let removed =
            play::Clientbound::RemoveMobEffect(play::RemoveMobEffect { id: 1, effect: 39 })
                .encode();
        assert_eq!(s.clientbound(&removed).unwrap(), None);
    }

    #[test]
    fn a_1_21_3_pick_item_swaps_the_slot_into_the_held_hotbar_slot() {
        let s = ClientSession::new(ServerVersion::V1_21_3);
        s.reading.store(to_u8(State::Play), Ordering::Release);
        s.writing.store(to_u8(State::Play), Ordering::Release);
        // The server put the hand on hotbar slot 2.
        let mut w = Writer::new();
        w.varint(2);
        s.clientbound(&body(packet_ids::play::clientbound::SET_HELD_SLOT, &w.data))
            .unwrap();
        // pick_item (34 in 1.21.3) of inventory index 20.
        let mut w = Writer::new();
        w.varint(20);
        let (id, payload) = s.serverbound(34, w.data).unwrap().unwrap();
        assert_eq!(id, packet_ids::play::serverbound::CONTAINER_CLICK);
        let click = play::Serverbound::decode(id, &payload).unwrap();
        let play::Serverbound::ContainerClick(c) = click else {
            panic!("{click:?}");
        };
        assert_eq!((c.container, c.slot, c.button, c.input), (0, 20, 2, 2));
        // A hotbar index is the menu's 36 + index; the held one itself is nothing to swap.
        let mut w = Writer::new();
        w.varint(2);
        assert_eq!(s.serverbound(34, w.data).unwrap(), None);
        // 1.21.4 clients send no pick_item: 34 is not taken over for them.
        assert_eq!(old_pick_item_id(ServerVersion::V1_21_4), None);
        assert_eq!(old_pick_item_id(ServerVersion::V1_20_6), Some(32));
    }

    #[test]
    fn a_1_21_1_client_is_sent_the_real_explosion_radius() {
        let s = ClientSession::new(ServerVersion::V1_21_1);
        s.reading.store(to_u8(State::Play), Ordering::Release);
        s.writing.store(to_u8(State::Play), Ordering::Release);
        let body = play::Clientbound::Explode(play::Explode {
            center: [1.0, 2.0, 3.0],
            radius: 6.5,
            block_count: 12,
            knockback: Some([0.5, 0.1, 0.0]),
            particle: crate::entity_data::Particle { id: 29, data: None },
            sound: (-1, Some(("minecraft:entity.generic.explode".into(), None))),
            block_particles: Vec::new(),
            play_sound: true,
        })
        .encode();
        // Every older client is sent it (1.21.8 and older used to fail on reading it back).
        for v in ServerVersion::ALL {
            let t = ClientSession::new(v);
            t.writing.store(to_u8(State::Play), Ordering::Release);
            assert!(t.clientbound(&body).unwrap().is_some(), "{v:?}");
        }
        let sent = s.clientbound(&body).unwrap().unwrap();
        let mut r = Reader::new(&sent);
        r.varint().unwrap();
        assert_eq!(
            [r.f64().unwrap(), r.f64().unwrap(), r.f64().unwrap()],
            [1.0, 2.0, 3.0]
        );
        // 1.21.1's strength, after the centre: 26.3's radius, not a guess.
        assert_eq!(r.f32().unwrap(), 6.5);
    }

    #[test]
    fn each_direction_follows_its_own_switches() {
        let s = ClientSession::new(ServerVersion::V26_2);
        assert_eq!((s.reading(), s.writing()), (State::Login, State::Login));
        // The server finishes the login: it speaks configuration, the client still login.
        s.clientbound(&body(packet_ids::login::clientbound::LOGIN_FINISHED, &[]))
            .unwrap();
        assert_eq!(
            (s.reading(), s.writing()),
            (State::Login, State::Configuration)
        );
        s.serverbound(
            packet_ids::login::serverbound::LOGIN_ACKNOWLEDGED,
            Vec::new(),
        )
        .unwrap();
        assert_eq!(s.reading(), State::Configuration);
        // finish_configuration is 3 both ways in 26.2 as in 26.3.
        s.clientbound(&body(3, &[])).unwrap();
        s.serverbound(3, Vec::new()).unwrap();
        assert_eq!((s.reading(), s.writing()), (State::Play, State::Play));
    }

    #[test]
    fn packets_are_renumbered_both_ways_and_unknown_ones_dropped() {
        let s = in_play();
        // keep_alive: 26.3's clientbound 45 is 26.2's 44; serverbound 28 in both.
        let sent = s
            .clientbound(&body(packet_ids::play::clientbound::KEEP_ALIVE, &[0; 8]))
            .unwrap()
            .unwrap();
        assert_eq!(sent[0], 44);
        assert_eq!(
            s.serverbound(28, vec![0; 8]).unwrap(),
            Some((packet_ids::play::serverbound::KEEP_ALIVE, vec![0; 8]))
        );
        // add_transient_block (37) is new in 26.3: not sent.
        let transient = packet_ids::play::clientbound::ADD_TRANSIENT_BLOCK;
        assert_eq!(s.clientbound(&body(transient, &[0; 9])).unwrap(), None);
        // 26.2's resource_pack (49) is 26.3's 50.
        assert_eq!(
            s.serverbound(49, vec![7]).unwrap(),
            Some((packet_ids::play::serverbound::RESOURCE_PACK, vec![7]))
        );
    }

    #[test]
    fn a_26_2_main_hand_swing_is_a_punch_and_an_off_hand_one_nothing() {
        let s = in_play();
        // swing: 63 in 26.2, its hand a VarInt (0 main, 1 off); 26.3's punch has no payload.
        assert_eq!(
            s.serverbound(63, vec![0]).unwrap(),
            Some((packet_ids::play::serverbound::PUNCH, Vec::new()))
        );
        assert_eq!(s.serverbound(63, vec![1]).unwrap(), None);
    }

    #[test]
    fn the_recipe_packets_of_a_1_21_1_client_are_not_passed_on() {
        let s = ClientSession::new(ServerVersion::V1_21_1);
        s.reading.store(to_u8(State::Play), Ordering::Release);
        s.writing.store(to_u8(State::Play), Ordering::Release);
        let key = |id: i32| {
            let mut w = Writer::new();
            if id == 34 {
                w.varint(2);
            }
            w.string("minecraft:oak_planks");
            if id == 34 {
                w.bool(false);
            }
            w.data
        };
        // place_recipe is 34 and recipe_book_seen_recipe 41 in 1.21.1 (packets report); both name the recipe by key.
        assert_eq!(s.serverbound(34, key(34)).unwrap(), None);
        assert_eq!(s.serverbound(41, key(41)).unwrap(), None);
        // The settings packet is laid out alike and goes on (40 in 1.21.1, 47 pinned).
        assert_eq!(
            s.serverbound(40, vec![0, 1, 0]).unwrap(),
            Some((
                packet_ids::play::serverbound::RECIPE_BOOK_CHANGE_SETTINGS,
                vec![0, 1, 0]
            ))
        );
    }

    #[test]
    fn a_26_2_digging_action_past_0_is_one_up() {
        let s = in_play();
        let action = |n: i32| {
            let mut w = Writer::new();
            w.varint(n);
            w.i64(crate::buf::pack_block_pos([1, 2, 3]));
            w.u8(1);
            w.varint(9);
            // player_action: 41 in both.
            let (id, payload) = s.serverbound(41, w.data).unwrap().unwrap();
            assert_eq!(id, packet_ids::play::serverbound::PLAYER_ACTION);
            match play::Serverbound::decode(id, &payload).unwrap() {
                play::Serverbound::PlayerAction(p) => {
                    assert_eq!((p.pos, p.direction, p.sequence), ([1, 2, 3], 1, 9));
                    p.action
                }
                other => panic!("{other:?}"),
            }
        };
        assert_eq!(action(0), 0); // start destroying
        assert_eq!(action(1), 2); // abort: 1 in 26.2, 2 in 26.3
        assert_eq!(action(2), 3); // stop destroying
        assert_eq!(action(6), 7); // swap held items
    }

    /// An older client follows each accepted teleport with a move to the same place; 26.3 doesn't (its
    /// accept carries the place), and two moves in one client tick are kicked as invalid movement.
    #[test]
    fn the_move_an_older_client_sends_after_accepting_a_teleport_is_not_passed_on() {
        let s = in_play();
        let p = play::Clientbound::PlayerPosition(play::PlayerPosition {
            teleport: 1,
            position: [100.5, 46.0, 100.5],
            delta: [0.0; 3],
            yaw: 180.0,
            pitch: 0.0,
            relatives: 0,
        });
        s.clientbound(&p.encode()).unwrap().unwrap();
        let mut w = Writer::new();
        w.varint(1);
        s.serverbound(0, w.data).unwrap().unwrap();
        let pos_rot = |x: f64| {
            let mut w = Writer::new();
            for v in [x, 46.0, 100.5] {
                w.f64(v);
            }
            w.f32(180.0);
            w.f32(0.0);
            w.u8(1);
            w.data
        };
        // move_player_pos_rot: 31 in both. The echo of the accepted place is not sent on...
        assert!(s.serverbound(31, pos_rot(100.5)).unwrap().is_none());
        // ...but the client's next real move is.
        assert!(s.serverbound(31, pos_rot(100.5)).unwrap().is_some());
        assert!(s.serverbound(31, pos_rot(101.0)).unwrap().is_some());
    }

    /// A 26.2 session that sent teleport 1 to (100.5, 46, 100.5), yaw 180, and read the client's accept.
    fn accepted_teleport() -> ClientSession {
        let s = in_play();
        let p = play::Clientbound::PlayerPosition(play::PlayerPosition {
            teleport: 1,
            position: [100.5, 46.0, 100.5],
            delta: [0.0; 3],
            yaw: 180.0,
            pitch: 0.0,
            relatives: 0,
        });
        s.clientbound(&p.encode()).unwrap().unwrap();
        // accept_teleportation: 0 in both.
        s.serverbound(0, vec![1]).unwrap().unwrap();
        s
    }

    /// move_player_pos_rot (31 in both) to (x, 46, 100.5), yaw 180, pitch 0, on the ground.
    fn pos_rot(x: f64) -> Vec<u8> {
        let mut w = Writer::new();
        for v in [x, 46.0, 100.5] {
            w.f64(v);
        }
        w.f32(180.0);
        w.f32(0.0);
        w.u8(1);
        w.data
    }

    #[test]
    fn the_echo_move_is_dropped_with_other_packets_between_it_and_the_accept() {
        let s = accepted_teleport();
        // keep_alive (28 in both) and a main-hand swing (63 in 26.2, 26.3's punch 46) come first.
        assert_eq!(
            s.serverbound(28, vec![0; 8]).unwrap(),
            Some((28, vec![0; 8]))
        );
        assert_eq!(s.serverbound(63, vec![0]).unwrap(), Some((46, Vec::new())));
        assert_eq!(s.serverbound(31, pos_rot(100.5)).unwrap(), None);
        // The echo came: the same place again is a move of its own.
        assert!(s.serverbound(31, pos_rot(100.5)).unwrap().is_some());
    }

    #[test]
    fn a_move_other_than_the_echo_goes_on_and_ends_the_wait_for_it() {
        let s = accepted_teleport();
        assert_eq!(
            s.serverbound(31, pos_rot(101.0)).unwrap(),
            Some((31, pos_rot(101.0)))
        );
        // Back at the accepted place a tick later: a real move, passed on.
        assert_eq!(
            s.serverbound(31, pos_rot(100.5)).unwrap(),
            Some((31, pos_rot(100.5)))
        );
        // A turn (move_player_rot: 32 in both) ends the wait too.
        let s = accepted_teleport();
        let mut w = Writer::new();
        w.f32(90.0);
        w.f32(0.0);
        w.u8(1);
        assert!(s.serverbound(32, w.data).unwrap().is_some());
        assert!(s.serverbound(31, pos_rot(100.5)).unwrap().is_some());
    }

    /// A session of `version` in play, its `minecraft:enchantment` registry sent as aqua_affinity, lunge,
    /// mending (pinned 0, 1, 2).
    fn with_enchantments(version: ServerVersion) -> ClientSession {
        let s = ClientSession::new(version);
        s.writing
            .store(to_u8(State::Configuration), Ordering::Release);
        s.clientbound(&body(
            packet_ids::configuration::clientbound::REGISTRY_DATA,
            &enchantments(&[
                "minecraft:aqua_affinity",
                "minecraft:lunge",
                "minecraft:mending",
            ]),
        ))
        .unwrap()
        .unwrap();
        s.reading.store(to_u8(State::Play), Ordering::Release);
        s.writing.store(to_u8(State::Play), Ordering::Release);
        s
    }

    fn enchantments(names: &[&str]) -> Vec<u8> {
        let mut w = Writer::new();
        configuration::RegistryData::write(
            &configuration::RegistryData {
                registry: "minecraft:enchantment".into(),
                entries: names
                    .iter()
                    .map(|n| configuration::RegistryEntry {
                        id: (*n).into(),
                        data: None,
                    })
                    .collect(),
            },
            &mut w,
        );
        w.data
    }

    /// 1.21.1 has no lunge: its mending is 1, 26.3's 2.
    #[test]
    fn a_1_21_1_creative_stack_takes_the_pinned_enchantment_after_one_it_lacks() {
        use crate::item::{Component, component};
        let s = with_enchantments(ServerVersion::V1_21_1);
        // set_creative_mode_slot (50 in 1.21.1): slot 36, one diamond_sword (838 in 1.21.1) with one
        // component, enchantments (9 in 1.21.1): mending (1) at level 3, shown in the tooltip.
        let mut w = Writer::new();
        w.i16(36);
        for v in [1, 838, 1, 0, 9, 1, 1, 3] {
            w.varint(v);
        }
        w.bool(true);
        let (id, payload) = s.serverbound(50, w.data).unwrap().unwrap();
        assert_eq!(id, 57);
        let play::Serverbound::SetCreativeModeSlot(p) =
            play::Serverbound::decode(id, &payload).unwrap()
        else {
            panic!("not a creative slot");
        };
        let stack = p.item.unwrap();
        // diamond_sword: 1050 in 26.3; enchantments: component 13.
        assert_eq!(stack.item, 1050);
        assert_eq!(
            stack.component(component::ENCHANTMENTS),
            Some(&Component::Enchantments(vec![(2, 3)]))
        );
    }

    /// set_equipment with a saddle is read back and written again in the client's ids for clients through
    /// 1.21.6 (they have no saddle slot): its enchantments are renumbered once, not twice.
    #[test]
    fn a_1_21_1_client_is_sent_its_own_enchantment_once() {
        use crate::item::{Component, Stack, component};
        let s = with_enchantments(ServerVersion::V1_21_1);
        let mut sword = Stack::new(1050, 1);
        sword.added.push((
            component::ENCHANTMENTS,
            Component::Enchantments(vec![(2, 3)]),
        ));
        let p = play::Clientbound::SetEquipment(play::SetEquipment {
            id: 4,
            // Main hand, and the saddle slot (7) 1.21.6 lacks.
            slots: vec![(0, Some(sword)), (7, Some(Stack::new(1050, 1)))],
        });
        let sent = s.clientbound(&p.encode()).unwrap().unwrap();
        let mut r = Reader::new(&sent);
        // set_equipment: 91 in 1.21.1; entity 4, main hand (0, the last slot: the saddle is left out).
        assert_eq!(r.varint().unwrap(), 91);
        assert_eq!(r.varint().unwrap(), 4);
        assert_eq!(r.u8().unwrap(), 0);
        let stack = {
            let _up = ids::renumbering(ServerVersion::V1_21_1.ids().unwrap(), Way::Up);
            crate::item::Slot::read(&mut r).unwrap().unwrap()
        };
        // Mending is 1 in the client's numbering (renumbered twice, it was left out).
        assert_eq!(
            stack.component(component::ENCHANTMENTS),
            Some(&Component::Enchantments(vec![(1, 3)]))
        );
    }

    #[test]
    fn a_registry_sent_again_without_removed_entries_is_no_longer_renumbered() {
        use ids::SyncedIds;
        let s = with_enchantments(ServerVersion::V1_21_1);
        let down = |id| {
            let maps = s.synced.lock().unwrap().clone();
            maps.down("minecraft:enchantment", id)
        };
        assert_eq!(down(2), Some(1));
        // A second configuration: the registry comes without lunge, nothing is removed from it.
        s.writing
            .store(to_u8(State::Configuration), Ordering::Release);
        let again = enchantments(&["minecraft:aqua_affinity", "minecraft:mending"]);
        let sent = s
            .clientbound(&body(
                packet_ids::configuration::clientbound::REGISTRY_DATA,
                &again,
            ))
            .unwrap()
            .unwrap();
        // registry_data: 7 in 1.21.1, sent as it is.
        assert_eq!(sent, body(7, &again));
        assert_eq!(down(2), Some(2));
    }

    /// A 1.21.3 session in play that was sent teleport 1 to (100.5, 46, 100.5), yaw 180, pitch 0.
    fn teleported_1_21_3() -> ClientSession {
        let s = ClientSession::new(ServerVersion::V1_21_3);
        s.reading.store(to_u8(State::Play), Ordering::Release);
        s.writing.store(to_u8(State::Play), Ordering::Release);
        let p = play::Clientbound::PlayerPosition(play::PlayerPosition {
            teleport: 1,
            position: [100.5, 46.0, 100.5],
            delta: [0.0; 3],
            yaw: 180.0,
            pitch: 0.0,
            relatives: 0,
        });
        s.clientbound(&p.encode()).unwrap().unwrap();
        s
    }

    /// 1.21.2 and 1.21.3 send the echo before the accept: move_player_pos_rot (29 in 1.21.3, 31 in 26.3),
    /// accept_teleportation (0), then the tick's own move.
    #[test]
    fn a_1_21_3_echo_before_its_accept_is_dropped() {
        let s = teleported_1_21_3();
        assert_eq!(s.serverbound(29, pos_rot(100.5)).unwrap(), None);
        let (id, _) = s.serverbound(0, vec![1]).unwrap().unwrap();
        assert_eq!(id, 0);
        // The tick's move at the same place goes on: nothing waits after the accept.
        assert_eq!(
            s.serverbound(29, pos_rot(100.5)).unwrap(),
            Some((31, pos_rot(100.5)))
        );
    }

    #[test]
    fn a_1_21_3_move_elsewhere_or_a_second_echo_before_the_accept_goes_on() {
        let s = teleported_1_21_3();
        // A move from before the client took the teleport.
        assert_eq!(
            s.serverbound(29, pos_rot(90.0)).unwrap(),
            Some((31, pos_rot(90.0)))
        );
        assert_eq!(s.serverbound(29, pos_rot(100.5)).unwrap(), None);
        // One echo per teleport.
        assert_eq!(
            s.serverbound(29, pos_rot(100.5)).unwrap(),
            Some((31, pos_rot(100.5)))
        );
    }

    /// 1.21.1 has no client_tick_end: one follows each of its moves. 26.3's client_tick_end is 13.
    #[test]
    fn a_1_21_1_move_is_followed_by_one_client_tick_end() {
        let s = ClientSession::new(ServerVersion::V1_21_1);
        s.reading.store(to_u8(State::Play), Ordering::Release);
        // move_player_pos: 26 in 1.21.1, 30 in 26.3; x, y, z and the on-ground boolean.
        let mut w = Writer::new();
        for v in [1.0, 64.0, 2.0] {
            w.f64(v);
        }
        w.bool(true);
        let (id, _) = s.serverbound(26, w.data).unwrap().unwrap();
        assert_eq!(id, 30);
        assert_eq!(s.next_serverbound(), Some((13, Vec::new())));
        assert_eq!(s.next_serverbound(), None);
        // A keep-alive (24 in 1.21.1) is no move: nothing follows it.
        s.serverbound(24, vec![0; 8]).unwrap().unwrap();
        assert_eq!(s.next_serverbound(), None);
    }

    #[test]
    fn a_1_21_4_move_is_followed_by_nothing() {
        let s = ClientSession::new(ServerVersion::V1_21_4);
        s.reading.store(to_u8(State::Play), Ordering::Release);
        // move_player_pos: 28 in 1.21.4; x, y, z and the flags byte (on the ground).
        let mut w = Writer::new();
        for v in [1.0, 64.0, 2.0] {
            w.f64(v);
        }
        w.u8(1);
        let (id, _) = s.serverbound(28, w.data).unwrap().unwrap();
        assert_eq!(id, 30);
        assert_eq!(s.next_serverbound(), None);
    }

    #[test]
    fn a_26_2_teleport_confirmation_states_where_the_teleport_put_the_client() {
        use play::relative as rel;
        let s = in_play();
        let teleport = |id: i32, position: [f64; 3], yaw: f32, relatives: i32| {
            let p = play::Clientbound::PlayerPosition(play::PlayerPosition {
                teleport: id,
                position,
                delta: [0.0; 3],
                yaw,
                pitch: 30.0,
                relatives,
            });
            s.clientbound(&p.encode()).unwrap().unwrap();
        };
        let accept = |id: i32| {
            let mut w = Writer::new();
            w.varint(id);
            // accept_teleportation: 0 in both.
            let (pinned, payload) = s.serverbound(0, w.data).unwrap().unwrap();
            assert_eq!(pinned, packet_ids::play::serverbound::ACCEPT_TELEPORTATION);
            match play::Serverbound::decode(pinned, &payload).unwrap() {
                play::Serverbound::AcceptTeleportation(p) => {
                    assert_eq!(p.id, id);
                    ([p.x, p.y, p.z], p.yaw, p.pitch)
                }
                other => panic!("{other:?}"),
            }
        };
        teleport(1, [8.5, -60.0, 8.5], 90.0, 0);
        assert_eq!(accept(1), ([8.5, -60.0, 8.5], 90.0, 30.0));
        // The client walks (move_player_pos: 30 in both), then is moved by (1, 2, 3) and turned by 10.
        let mut w = Writer::new();
        for v in [10.0, -60.0, 10.0] {
            w.f64(v);
        }
        w.u8(1);
        s.serverbound(30, w.data).unwrap().unwrap();
        teleport(
            2,
            [1.0, 2.0, 3.0],
            10.0,
            rel::X | rel::Y | rel::Z | rel::Y_ROT,
        );
        assert_eq!(accept(2), ([11.0, -58.0, 13.0], 100.0, 30.0));
        // An unknown id: where the client last was.
        assert_eq!(accept(7), ([11.0, -58.0, 13.0], 100.0, 30.0));
    }

    #[test]
    fn block_states_the_client_lacks_get_a_stand_in() {
        let s = in_play();
        let update = |state: i32| {
            let p = play::Clientbound::BlockUpdate(play::BlockUpdate {
                pos: [1, 2, 3],
                state,
            });
            let sent = s.clientbound(&p.encode()).unwrap().unwrap();
            let mut r = Reader::new(&sent);
            // block_update: 8 in both.
            assert_eq!(r.varint().unwrap(), 8);
            match play::Clientbound::decode(8, r.rest()).unwrap() {
                play::Clientbound::BlockUpdate(b) => b.state,
                other => panic!("{other:?}"),
            }
        };
        // chest's first state: 5543 in 26.3, 3987 in 26.2; grass_block 8 in both.
        assert_eq!(update(5543), 3987);
        assert_eq!(update(8), 8);
        // poplar_log axis=y (167, new in 26.3) shows as birch_log axis=y (146 in 26.3, 143 in 26.2).
        assert_eq!(update(167), 143);
        // shelf_mushroom (11227, new in 26.3) is solid: it shows as a brown_mushroom_block (9500 in 26.3, 7766 in
        // 26.2), not as the brown mushroom players would fall through.
        assert_eq!(update(11227), 7766);
    }

    #[test]
    fn items_take_the_clients_ids_and_stand_ins() {
        use crate::item::Stack;
        let s = in_play();
        let slot = |item: i32| {
            let p = play::Clientbound::SetCursorItem(play::SetCursorItem {
                item: Some(Stack::new(item, 3)),
            });
            let sent = s.clientbound(&p.encode()).unwrap().unwrap();
            let mut r = Reader::new(&sent);
            r.varint().unwrap();
            let mut payload = Reader::new(r.rest());
            crate::item::Slot::read(&mut payload).unwrap().unwrap().item
        };
        // diamond: 1012 in 26.3, 926 in 26.2.
        assert_eq!(slot(1012), 926);
        // poplar_planks (72, new in 26.3) shows as birch_planks (65 in both).
        assert_eq!(slot(72), 65);
        // ocean_monument_map (1239, new in 26.3) is a filled map: filled_map (1238 in 26.3, 1133 in 26.2).
        assert_eq!(slot(1239), 1133);
    }

    /// Every client through 1.21.6 was disconnected by the first equipment with a component ("NBT at byte 1"):
    /// 1.21.6's step read the stacks, already in the client's ids, as pinned ones.
    #[test]
    fn equipment_with_components_reaches_every_older_client() {
        use crate::item::{Component, Stack, component};
        use horizon_core::nbt::Tag;
        let mut hat = Stack::new(1012, 1);
        hat.added.push((
            component::CUSTOM_NAME,
            Component::Text(Tag::String("Santa Hat".into())),
        ));
        hat.added
            .push((component::ENCHANTMENT_GLINT_OVERRIDE, Component::Bool(true)));
        let p = play::Clientbound::SetEquipment(play::SetEquipment {
            id: 4,
            slots: vec![(0, Some(hat.clone())), (5, Some(hat))],
        });
        for v in ServerVersion::ALL {
            if v == ServerVersion::Current || v.at_most(ServerVersion::V1_20_3) {
                continue;
            }
            let s = ClientSession::new(v);
            s.reading.store(to_u8(State::Play), Ordering::Release);
            s.writing.store(to_u8(State::Play), Ordering::Release);
            let sent = s.clientbound(&p.encode());
            assert!(matches!(sent, Ok(Some(_))), "{v:?}: {sent:?}");
        }
    }

    #[test]
    fn an_entity_the_client_lacks_is_not_spawned() {
        let s = in_play();
        let add = |kind: i32| {
            play::Clientbound::AddEntity(play::AddEntity {
                id: 5,
                uuid: 1,
                kind,
                position: [0.0; 3],
                velocity: [0.0; 3],
                pitch: 0,
                yaw: 0,
                head_yaw: 0,
                data: 0,
            })
            .encode()
        };
        // cushion (33, new in 26.3) has no stand-in; poplar_boat (106) sails as birch_boat (12).
        assert_eq!(s.clientbound(&add(33)).unwrap(), None);
        let sent = s.clientbound(&add(106)).unwrap().unwrap();
        let mut r = Reader::new(&sent);
        // add_entity: 1 in both.
        assert_eq!(r.varint().unwrap(), 1);
        match play::Clientbound::decode(1, r.rest()).unwrap() {
            play::Clientbound::AddEntity(e) => assert_eq!(e.kind, 12),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_creative_slot_from_the_client_takes_the_pinned_item() {
        let s = in_play();
        // set_creative_mode_slot: 26.2's 56 is 26.3's 57. Slot 36, three diamonds (926 in 26.2), no components.
        let mut w = Writer::new();
        w.i16(36);
        crate::item::DelimitedSlot::write(&Some(crate::item::Stack::new(926, 3)), &mut w);
        let (id, payload) = s.serverbound(56, w.data).unwrap().unwrap();
        assert_eq!(id, packet_ids::play::serverbound::SET_CREATIVE_MODE_SLOT);
        match play::Serverbound::decode(id, &payload).unwrap() {
            play::Serverbound::SetCreativeModeSlot(p) => {
                assert_eq!(p.item.map(|i| (i.item, i.count)), Some((1012, 3)));
            }
            other => panic!("{other:?}"),
        }
    }

    /// A client through 1.21.5 has no synchronised direction on hanging entities (1.21.6 added it at index 8):
    /// an item frame's rotation (26.3's field 10) is its field 9 again and the direction is not sent. A real
    /// 1.21.5 client left with "Invalid entity data item type for field 8" at the first item frame with an item.
    #[test]
    fn a_hanging_entitys_direction_is_not_sent_to_a_client_through_1_21_5() {
        use crate::adapter::pinned_ids::entity_type as entity;
        let add = play::Clientbound::AddEntity(play::AddEntity {
            id: 469,
            uuid: 99,
            kind: entity::ITEM_FRAME,
            position: [85.5, 45.0, 69.5],
            velocity: [0.0; 3],
            pitch: 0,
            yaw: 0,
            head_yaw: 0,
            data: 1,
        });
        let data = play::Clientbound::SetEntityData(play::SetEntityData {
            id: 469,
            values: vec![
                (8, serializer::DIRECTION, Value::Id(1)),
                (10, serializer::INT, Value::Int(3)),
            ],
        });
        for (version, tail) in [
            // 1.21.6 has the direction: the rotation stays at 10.
            (ServerVersion::V1_21_6, &[10, 1, 3, 0xff][..]),
            // Entity id 469 as a VarInt, the rotation at 9 (an int, serializer 1), the end marker: nothing else.
            (ServerVersion::V1_21_5, &[0xd5, 0x03, 9, 1, 3, 0xff][..]),
            (ServerVersion::V1_21_4, &[0xd5, 0x03, 9, 1, 3, 0xff][..]),
            (ServerVersion::V1_21_1, &[0xd5, 0x03, 9, 1, 3, 0xff][..]),
            (ServerVersion::V1_20_6, &[0xd5, 0x03, 9, 1, 3, 0xff][..]),
        ] {
            let s = ClientSession::new(version);
            s.reading.store(to_u8(State::Play), Ordering::Release);
            s.writing.store(to_u8(State::Play), Ordering::Release);
            s.clientbound(&add.encode()).unwrap().unwrap();
            let sent = s.clientbound(&data.encode()).unwrap().unwrap();
            assert!(sent.ends_with(tail), "{version:?}: {sent:?}");
        }
    }

    /// 26.1's `spectate_entity` is its id 62 (the packets report: sign_update 61, spectate_entity 62, swing 63);
    /// 26.3 has it as `spectator_action` at 63 (by network id, sign_update at 62).
    #[test]
    fn a_26_1_client_spectating_an_entity_reaches_the_server_by_network_id() {
        let s = ClientSession::new(ServerVersion::V26_1);
        s.reading.store(to_u8(State::Play), Ordering::Release);
        s.writing.store(to_u8(State::Play), Ordering::Release);
        s.entities.lock().unwrap().insert(7, (0, 0x42));
        let mut w = Writer::new();
        w.uuid(0x42);
        let (id, payload) = s.serverbound(62, w.data).unwrap().unwrap();
        assert_eq!(id, 63);
        // OptVarInt: the network id plus one.
        assert_eq!(payload, [8]);
        // An entity the client was never told of is dropped.
        let mut w = Writer::new();
        w.uuid(0x43);
        assert_eq!(s.serverbound(62, w.data).unwrap(), None);
        // The ids around it keep their numbers: 26.1's sign_update (61) is the pinned 62.
        assert_eq!(
            ServerVersion::V26_1.serverbound_id_up(State::Play, 61),
            Some(62)
        );
    }

    /// A real 1.20.6, 1.21.4 or 1.21.8 client given an absorption effect left with "Invalid entity data item type
    /// for field 17 ... old=0(Byte), new=4.0(Float)": 26.3's Avatar has the main hand (15) and the skin parts (16),
    /// its Player the absorption (17), the score (18) and the shoulders (19, 20); through 1.21.8 the Player has the
    /// absorption (15), the score (16), the skin parts (17), the main hand (18) and the shoulders (19, 20, NBTs).
    /// The local player is never sent an add_entity, so it is told apart by the login's entity id.
    #[test]
    fn a_player_s_own_fields_move_back_for_clients_through_1_21_8() {
        use crate::adapter::pinned_ids::entity_type as entity;
        let login = play::Clientbound::Login(play::Login {
            player_id: 7,
            hardcore: false,
            levels: vec!["minecraft:overworld".into()],
            max_players: 20,
            chunk_radius: 10,
            simulation_distance: 8,
            reduced_debug_info: false,
            show_death_screen: true,
            limited_crafting: false,
            spawn: play::SpawnInfo {
                dimension_type: 0,
                dimension: "minecraft:overworld".into(),
                seed: 42,
                game_type: 1,
                previous_game_type: None,
                debug: false,
                flat: true,
                last_death: None,
                portal_cooldown: 0,
                sea_level: 63,
            },
            online_mode: true,
            enforces_secure_chat: false,
        });
        let add = play::Clientbound::AddEntity(play::AddEntity {
            id: 9,
            uuid: 1,
            kind: entity::PLAYER,
            position: [0.0; 3],
            velocity: [0.0; 3],
            pitch: 0,
            yaw: 0,
            head_yaw: 0,
            data: 0,
        });
        let data = |id| {
            play::Clientbound::SetEntityData(play::SetEntityData {
                id,
                values: vec![
                    (15, serializer::HUMANOID_ARM, Value::Id(1)),
                    (16, serializer::BYTE, Value::Byte(0x7f)),
                    (17, serializer::FLOAT, Value::Float(4.0)),
                    (18, serializer::INT, Value::Int(5)),
                    (19, serializer::OPTIONAL_UNSIGNED_INT, Value::Id(0)),
                    (20, serializer::OPTIONAL_UNSIGNED_INT, Value::Id(0)),
                ],
            })
        };
        for version in [
            ServerVersion::V1_21_8,
            ServerVersion::V1_21_4,
            ServerVersion::V1_20_6,
        ] {
            let s = ClientSession::new(version);
            s.reading.store(to_u8(State::Play), Ordering::Release);
            s.writing.store(to_u8(State::Play), Ordering::Release);
            s.clientbound(&login.encode()).unwrap();
            s.clientbound(&add.encode()).unwrap();
            // main hand 18 (byte), skin parts 17 (byte), absorption 15 (float 4.0), score 16 (VarInt); the
            // shoulders (19, 20) are dropped.
            let tail = [
                18, 0, 1, 17, 0, 0x7f, 15, 3, 0x40, 0x80, 0, 0, 16, 1, 5, 0xff,
            ];
            for id in [7u8, 9] {
                let sent = s
                    .clientbound(&data(i32::from(id)).encode())
                    .unwrap()
                    .unwrap();
                assert!(
                    sent.ends_with(&[&[id][..], &tail[..]].concat()),
                    "{version:?} {id}: {sent:?}"
                );
            }
        }
    }

    /// A wolf variant is a registry-or-inline holder (the id plus one) for 1.21 to 1.21.4 and the id for the
    /// rest: variant 2 ends the entity data as 3 or as 2.
    #[test]
    fn a_wolf_variant_is_a_holder_for_a_client_through_1_21_4() {
        let data = play::Clientbound::SetEntityData(play::SetEntityData {
            id: 9,
            values: vec![(22, serializer::WOLF_VARIANT, Value::Id(2))],
        })
        .encode();
        for (version, tail) in [
            (ServerVersion::V1_21_4, &[3, 0xff][..]),
            (ServerVersion::V1_21_3, &[3, 0xff][..]),
            (ServerVersion::V1_21_1, &[3, 0xff][..]),
            (ServerVersion::V1_21_5, &[2, 0xff][..]),
            (ServerVersion::V1_20_6, &[2, 0xff][..]),
        ] {
            let s = ClientSession::new(version);
            s.reading.store(to_u8(State::Play), Ordering::Release);
            s.writing.store(to_u8(State::Play), Ordering::Release);
            let sent = s.clientbound(&data).unwrap().unwrap();
            assert!(sent.ends_with(tail), "{version:?}: {sent:?}");
        }
    }

    /// An explosion's particle goes through the explode step, which reads it back in the client's layout and writes
    /// it again: for a client through 1.21.8 `dragon_breath`, `effect`, `instant_effect` and `flash` carry no options
    /// there either, so the sound after the particle stays where the client reads it.
    #[test]
    fn an_explosion_particle_keeps_no_options_for_a_client_through_1_21_8() {
        use crate::adapter::pinned_ids::particle_type;
        use crate::entity_data::{Particle, ParticleData as D};
        let send = |version, particle| {
            let packet = play::Clientbound::Explode(play::Explode {
                center: [1.0, 2.0, 3.0],
                radius: 3.0,
                block_count: 0,
                knockback: None,
                particle,
                sound: (-1, Some(("minecraft:entity.generic.explode".into(), None))),
                block_particles: Vec::new(),
                play_sound: true,
            })
            .encode();
            let s = ClientSession::new(version);
            s.reading.store(to_u8(State::Play), Ordering::Release);
            s.writing.store(to_u8(State::Play), Ordering::Release);
            s.clientbound(&packet).unwrap().unwrap()
        };
        for version in [ServerVersion::V1_21_5, ServerVersion::V1_21_8] {
            // A particle without options, as the reference: a one-byte id between the knockback flag and the sound.
            let reference = send(
                version,
                Particle {
                    id: particle_type::EXPLOSION,
                    data: None,
                },
            );
            for (id, data) in [
                (particle_type::DRAGON_BREATH, D::Scalar(1.0)),
                (particle_type::EFFECT, D::ColorPower(0x1122_3344, 1.0)),
                (
                    particle_type::INSTANT_EFFECT,
                    D::ColorPower(0x1122_3344, 1.0),
                ),
                (particle_type::FLASH, D::Color(0x1122_3344)),
            ] {
                let sent = send(
                    version,
                    Particle {
                        id,
                        data: Some(data),
                    },
                );
                assert_eq!(sent.len(), reference.len(), "{version:?} {id}: {sent:?}");
                // everything after the particle id is the sound, which must come out whole
                let sound = &reference[27..];
                assert!(sent.ends_with(sound), "{version:?} {id}: {sent:?}");
            }
        }
    }

    /// Every particle with options, written for the clients before 1.21.2 (1.20.5 to 1.21.1 read the same bytes)
    /// as their `ParticleTypes` read them, checked against the 1.20.6 and 1.21.1 client jars: the particle follows the
    /// count, dust colours are three floats, `entity_effect` is one ARGB int, and `effect`, `instant_effect`,
    /// `dragon_breath` and `flash` carry nothing. A particle they lack (`block_crumble`, `tinted_leaves`, `trail`)
    /// is not sent. Each tail is the count (1), the particle's id and its options.
    #[test]
    fn particles_with_options_are_written_as_1_20_6_and_1_21_1_read_them() {
        use crate::entity_data::{Particle, ParticleData as D};
        let cases: [(i32, D, Option<&str>); 16] = [
            (1, D::State(1), Some(concat!("00000001", "01", "01"))),
            (2, D::State(1), Some(concat!("00000001", "02", "01"))),
            (36, D::State(1), Some(concat!("00000001", "1c", "01"))),
            (121, D::State(1), Some(concat!("00000001", "69", "01"))),
            (125, D::State(1), None),
            (
                28,
                D::Color(0x1122_3344),
                Some(concat!("00000001", "14", "11223344")),
            ),
            (46, D::Color(0x1122_3344), None),
            (52, D::Color(0x1122_3344), Some(concat!("00000001", "27"))),
            (
                21,
                D::ColorScale(0x0011_2233, 1.0),
                Some(concat!(
                    "00000001", "0d", "3d888889", "3e088889", "3e4ccccd", "3f800000"
                )),
            ),
            (
                22,
                D::ColorScale2(0x0011_2233, 0x0044_5566, 1.0),
                Some(concat!(
                    "00000001", "0e", "3d888889", "3e088889", "3e4ccccd", "3e888889", "3eaaaaab",
                    "3ecccccd", "3f800000"
                )),
            ),
            (
                23,
                D::ColorPower(0x1122_3344, 1.0),
                Some(concat!("00000001", "0f")),
            ),
            (
                56,
                D::ColorPower(0x1122_3344, 1.0),
                Some(concat!("00000001", "2b")),
            ),
            (15, D::Scalar(1.0), Some(concat!("00000001", "07"))),
            (
                48,
                D::Scalar(0.5),
                Some(concat!("00000001", "23", "3f000000")),
            ),
            (115, D::Delay(3), Some(concat!("00000001", "63", "03"))),
            (59, D::Trail([1.0, 2.0, 3.0], 0x1122_3344, 20), None),
        ];
        for version in [ServerVersion::V1_20_6, ServerVersion::V1_21_1] {
            for (id, data, tail) in &cases {
                let packet = play::Clientbound::LevelParticles(play::LevelParticles {
                    particle: Particle {
                        id: *id,
                        data: Some(data.clone()),
                    },
                    override_limiter: false,
                    always_show: false,
                    pos: [0.0; 3],
                    dist: [0.0; 3],
                    max_speed: [0.0; 3],
                    count: 1,
                    randomization: 0,
                })
                .encode();
                let s = ClientSession::new(version);
                s.reading.store(to_u8(State::Play), Ordering::Release);
                s.writing.store(to_u8(State::Play), Ordering::Release);
                let sent = s.clientbound(&packet).unwrap();
                let hex = sent.map(|b| b.iter().map(|x| format!("{x:02x}")).collect::<String>());
                match (tail, hex) {
                    (Some(tail), Some(hex)) => {
                        // packet id, long distance flag, 3 doubles, 4 floats: 1 + 1 + 24 + 16 bytes of 2 digits
                        assert!(hex.starts_with("2900"), "{version:?} {id}: {hex}");
                        assert_eq!(&hex[(1 + 1 + 24 + 16) * 2..], *tail, "{version:?} {id}");
                    }
                    (None, None) => {}
                    (tail, hex) => panic!("{version:?} {id}: {hex:?}, expected {tail:?}"),
                }
            }
        }
    }

    /// 1.21.9 gave `effect`, `instant_effect` and `dragon_breath` their options and `flash` its colour; 1.21.8 and
    /// older read none (1.21.5 and 1.21.8 shown, as numbered there).
    #[test]
    fn effect_and_flash_options_start_with_1_21_9() {
        use crate::entity_data::{Particle, ParticleData as D};
        let send = |version, id, data| {
            let packet = play::Clientbound::LevelParticles(play::LevelParticles {
                particle: Particle {
                    id,
                    data: Some(data),
                },
                override_limiter: false,
                always_show: false,
                pos: [0.0; 3],
                dist: [0.0; 3],
                max_speed: [0.0; 3],
                count: 1,
                randomization: 0,
            })
            .encode();
            let s = ClientSession::new(version);
            s.reading.store(to_u8(State::Play), Ordering::Release);
            s.writing.store(to_u8(State::Play), Ordering::Release);
            let sent = s.clientbound(&packet).unwrap().unwrap();
            sent[sent.len() - 4..].to_vec()
        };
        let power = D::ColorPower(0x1122_3344, 1.0);
        let tail = |version, id, data| {
            let t = send(version, id, data);
            t.iter().map(|x| format!("{x:02x}")).collect::<String>()
        };
        // plain: the count and the particle id are the last bytes; with options the colour and power follow
        assert_eq!(tail(ServerVersion::V1_21_5, 23, power.clone()), "0000010f");
        assert_eq!(tail(ServerVersion::V1_21_8, 15, D::Scalar(1.0)), "00000107");
        assert_eq!(tail(ServerVersion::V1_21_8, 52, D::Color(7)), "00000129");
        assert_eq!(tail(ServerVersion::V1_21_10, 23, power), "3f800000");
        assert_eq!(tail(ServerVersion::V1_21_10, 52, D::Color(7)), "00000007");
    }

    /// 26.2 and older read a decorated pot's `sherds` as a list of the four item ids (26.3: a compound of
    /// stack templates).
    #[test]
    fn a_decorated_pot_update_has_its_sherds_as_a_list_for_26_2_and_older() {
        use horizon_core::nbt::{Compound, JavaString, Tag};
        let text = |s: &str| Tag::String(JavaString::from(s));
        let mut front = Compound::new();
        front.insert(
            JavaString::from("id"),
            text("minecraft:angler_pottery_sherd"),
        );
        let mut sides = Compound::new();
        sides.insert(JavaString::from("front"), Tag::Compound(front));
        let mut root = Compound::new();
        root.insert(JavaString::from("sherds"), Tag::Compound(sides));
        let update = play::Clientbound::BlockEntityData(play::BlockEntityData {
            pos: [1, 64, 2],
            kind: 41,
            tag: Tag::Compound(root),
        })
        .encode();
        let list = Tag::List(vec![
            text("minecraft:brick"),
            text("minecraft:brick"),
            text("minecraft:brick"),
            text("minecraft:angler_pottery_sherd"),
        ]);
        for version in [
            ServerVersion::V26_2,
            ServerVersion::V26_1,
            ServerVersion::V1_21_11,
            ServerVersion::V1_21_5,
            ServerVersion::V1_20_6,
        ] {
            let s = ClientSession::new(version);
            s.reading.store(to_u8(State::Play), Ordering::Release);
            s.writing.store(to_u8(State::Play), Ordering::Release);
            let sent = s.clientbound(&update).unwrap().unwrap();
            let mut r = Reader::new(&sent);
            r.varint().unwrap();
            let tag = play::BlockEntityData::read(&mut r).unwrap().tag;
            let Tag::Compound(root) = tag else {
                panic!("{version:?}")
            };
            assert_eq!(
                root.get(&JavaString::from("sherds")),
                Some(&list),
                "{version:?}"
            );
        }
        // a 26.3 client reads the compound
        let s = ClientSession::new(ServerVersion::Current);
        s.reading.store(to_u8(State::Play), Ordering::Release);
        s.writing.store(to_u8(State::Play), Ordering::Release);
        assert_eq!(s.clientbound(&update).unwrap().unwrap(), update);
    }

    /// 1.21.10, 1.21.11 and 26.1 clients lack what 26.3 added: a new item or entity shows as its nearest older
    /// equivalent (the way 26.2's and 1.21.8's tables do), not as nothing.
    #[test]
    fn a_1_21_10_to_26_1_client_is_shown_stand_ins_for_what_26_3_added() {
        use crate::item::Stack;
        for (version, planks, map) in [
            (ServerVersion::V1_21_10, 38, 1104),
            (ServerVersion::V1_21_11, 38, 1104),
            (ServerVersion::V26_1, 38, 1105),
        ] {
            let s = ClientSession::new(version);
            s.reading.store(to_u8(State::Play), Ordering::Release);
            s.writing.store(to_u8(State::Play), Ordering::Release);
            let slot = |item: i32| {
                let p = play::Clientbound::SetCursorItem(play::SetCursorItem {
                    item: Some(Stack::new(item, 1)),
                });
                let sent = s.clientbound(&p.encode()).unwrap().unwrap();
                let mut r = Reader::new(&sent);
                r.varint().unwrap();
                let mut payload = Reader::new(r.rest());
                crate::item::Slot::read(&mut payload).unwrap().unwrap().item
            };
            // poplar_planks (72) is birch_planks, ocean_monument_map (1239) a filled_map
            assert_eq!(slot(72), planks, "{version:?}");
            assert_eq!(slot(1239), map, "{version:?}");
            let add = |kind: i32| {
                play::Clientbound::AddEntity(play::AddEntity {
                    id: 5,
                    uuid: 1,
                    kind,
                    position: [0.0; 3],
                    velocity: [0.0; 3],
                    pitch: 0,
                    yaw: 0,
                    head_yaw: 0,
                    data: 0,
                })
                .encode()
            };
            // poplar_boat (106) sails as birch_boat (12); cushion (33) has no stand-in.
            let sent = s.clientbound(&add(106)).unwrap().unwrap();
            let mut r = Reader::new(&sent);
            r.varint().unwrap();
            assert_eq!(play::AddEntity::read(&mut r).unwrap().kind, 12);
            assert_eq!(s.clientbound(&add(33)).unwrap(), None);
        }
    }

    /// 1.21.4 added the duration to the trail particle's options (TargetColorParticleOption, 1.21.2 and
    /// 1.21.3: a target and a colour only).
    #[test]
    fn a_trail_particle_has_no_duration_for_a_1_21_3_client() {
        use crate::entity_data::{Particle, ParticleData};
        let trail = play::Clientbound::LevelParticles(play::LevelParticles {
            particle: Particle {
                id: 59,
                data: Some(ParticleData::Trail([1.5, 2.0, 3.0], 9, 40)),
            },
            override_limiter: false,
            always_show: false,
            pos: [0.0; 3],
            dist: [0.0; 3],
            max_speed: [0.0; 3],
            count: 1,
            randomization: 0,
        })
        .encode();
        for (version, tail) in [
            (ServerVersion::V1_21_3, &[0, 0, 0, 9][..]),
            (ServerVersion::V1_21_4, &[0, 0, 0, 9, 40][..]),
        ] {
            let s = ClientSession::new(version);
            s.reading.store(to_u8(State::Play), Ordering::Release);
            s.writing.store(to_u8(State::Play), Ordering::Release);
            let sent = s.clientbound(&trail).unwrap().unwrap();
            assert!(sent.ends_with(tail), "{version:?}: {sent:?}");
        }
    }

    /// 1.21.2 split `boat` and `chest_boat` into one type per wood; a client through 1.21.1 knows the two
    /// types (10 and 17) and takes the wood from entity data index 11 (Boat.Type's ordinal), where the
    /// paddles and the bubble time move up by one.
    #[test]
    fn a_boat_of_any_wood_is_the_plain_boat_with_its_wood_for_a_client_through_1_21_1() {
        use crate::adapter::pinned_ids::entity_type as entity;
        use packet_ids::play::clientbound as cb;
        let add = |kind: i32| {
            play::Clientbound::AddEntity(play::AddEntity {
                id: 5,
                uuid: 1,
                kind,
                position: [0.0; 3],
                velocity: [0.0; 3],
                pitch: 0,
                yaw: 0,
                head_yaw: 0,
                data: 0,
            })
            .encode()
        };
        // (pinned type, the client's type, the wood)
        let cases = [
            (entity::ACACIA_BOAT, 10, 4),
            (entity::BAMBOO_RAFT, 10, 8),
            (entity::OAK_BOAT, 10, 0),
            (95, 10, 2),  // pale_oak_boat, a birch to this client
            (96, 17, 2),  // pale_oak_chest_boat
            (106, 10, 2), // poplar_boat
            (107, 17, 2), // poplar_chest_boat
            (entity::ACACIA_CHEST_BOAT, 17, 4),
            (entity::BAMBOO_CHEST_RAFT, 17, 8),
            (entity::OAK_CHEST_BOAT, 17, 0),
        ];
        for version in [ServerVersion::V1_21_1, ServerVersion::V1_20_6] {
            let data_id = version
                .clientbound_id_down(State::Play, cb::SET_ENTITY_DATA)
                .unwrap() as u8;
            for (pinned, old, wood) in cases {
                let s = ClientSession::new(version);
                s.reading.store(to_u8(State::Play), Ordering::Release);
                s.writing.store(to_u8(State::Play), Ordering::Release);
                let all = s.clientbound_all(&add(pinned)).unwrap();
                assert_eq!(all.len(), 2, "{version:?} {pinned}");
                let (main, extra) = (&all[0], &all[1]);
                let mut r = Reader::new(main);
                r.varint().unwrap();
                let spawned = play::AddEntity::read(&mut r).unwrap();
                assert_eq!(spawned.kind, old, "{version:?} {pinned}");
                // entity 5; index 11, serializer 1 (int), the wood, the end marker
                assert_eq!(*extra, [data_id, 5, 11, 1, wood as u8, 0xff]);
                // the paddles (11, 12) and the bubble time (13) take 1.21.1's indexes 12 to 14
                let paddles = play::Clientbound::SetEntityData(play::SetEntityData {
                    id: 5,
                    values: vec![
                        (11, serializer::BOOLEAN, Value::Bool(true)),
                        (13, serializer::INT, Value::Int(7)),
                    ],
                })
                .encode();
                let sent = s.clientbound_all(&paddles).unwrap();
                assert_eq!(sent, [[data_id, 5, 12, 8, 1, 14, 1, 7, 0xff]]);
            }
        }
        // a 1.21.2 client has every wood as it is
        let s = ClientSession::new(ServerVersion::V1_21_3);
        s.reading.store(to_u8(State::Play), Ordering::Release);
        s.writing.store(to_u8(State::Play), Ordering::Release);
        assert_eq!(
            s.clientbound_all(&add(entity::ACACIA_BOAT)).unwrap().len(),
            1
        );
    }

    /// A client through 1.21.4 takes an orb's value from `add_experience_orb` (id 2), not from entity data:
    /// the add waits for the data that follows it.
    #[test]
    fn an_experience_orb_is_added_with_its_value_for_a_client_through_1_21_4() {
        let add = play::Clientbound::AddEntity(play::AddEntity {
            id: 41,
            uuid: 7,
            kind: EXPERIENCE_ORB,
            position: [1.0, 2.0, -3.0],
            velocity: [0.0; 3],
            pitch: 0,
            yaw: 0,
            head_yaw: 0,
            data: 0,
        })
        .encode();
        let data = play::Clientbound::SetEntityData(play::SetEntityData {
            id: 41,
            values: vec![(8, serializer::INT, Value::Int(7))],
        })
        .encode();
        let orb = |value: u8| {
            let mut w = Writer::new();
            w.varint(2);
            w.varint(41);
            for v in [1.0, 2.0, -3.0] {
                w.f64(v);
            }
            w.bytes(&[0, value]);
            w.data
        };
        for version in [
            ServerVersion::V1_21_4,
            ServerVersion::V1_21_1,
            ServerVersion::V1_20_6,
        ] {
            let s = ClientSession::new(version);
            s.reading.store(to_u8(State::Play), Ordering::Release);
            s.writing.store(to_u8(State::Play), Ordering::Release);
            assert!(s.clientbound_all(&add).unwrap().is_empty());
            assert_eq!(s.clientbound_all(&data).unwrap(), [orb(7)], "{version:?}");
            // with no data after it, another packet shows it with the value 0 first
            assert!(s.clientbound_all(&add).unwrap().is_empty());
            let gone = play::Clientbound::RemoveEntities(play::RemoveEntities { ids: vec![41] });
            let sent = s.clientbound_all(&gone.encode()).unwrap();
            assert_eq!(sent.len(), 2, "{version:?}");
            assert_eq!(sent[0], orb(0));
        }
        // 1.21.5 and later have it as entity data: nothing waits
        let s = ClientSession::new(ServerVersion::V1_21_5);
        s.reading.store(to_u8(State::Play), Ordering::Release);
        s.writing.store(to_u8(State::Play), Ordering::Release);
        assert!(s.clientbound(&add).unwrap().is_some());
    }
}
