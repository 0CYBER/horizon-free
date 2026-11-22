//! The locator bar for remote players: ServerWaypointManager and
//! WaypointTransmitter's connections, for players (the transmitters vanilla has
//! by default: `waypoint_transmit_range` is 0 for every other entity).
//!
//! Vanilla updates a player's connections each time the player's position
//! changes (`Entity.setPosRaw`, then `ServerChunkCache.move`). The hub sees the
//! position once a tick, after the moves it handled, and updates from there:
//! the same packets, since a client sends one move a tick. Hiding (crouching,
//! invisibility, a carved pumpkin or a mob head worn) changes
//! `waypoint_transmit_range` and tracks or untracks the waypoint, as
//! `LivingEntity.onAttributeUpdated` does; joining or leaving a team remakes the
//! connections with the team's colour (`ServerScoreboard.updatePlayerWaypoint`).
use crate::fast_hash::FxHashMap as HashMap;
use horizon_protocol::packets::play::{
    Clientbound as C, TrackedWaypoint, WaypointId, WaypointTarget, waypoint_operation,
};
use std::sync::{Arc, OnceLock};

/// A waypoint packet encoded once, for every receiver it goes to.
pub struct Encoded {
    pub packet: C,
    pub body: super::Body,
}

/// What a receiver is sent: a packet of its own, or one shared by every receiver of the same
/// waypoint this tick (encoded once).
pub enum Sent {
    /// Boxed: a `Clientbound` is as large as its largest packet, and a crowd's locator bar holds
    /// tens of thousands of these a tick.
    Packet(Box<C>),
    Shared(Arc<Encoded>),
}

impl Sent {
    pub fn packet(&self) -> &C {
        match self {
            Self::Packet(p) => p,
            Self::Shared(e) => &e.packet,
        }
    }
    /// The body to send: a shared one stays shared up to the wire.
    pub fn body(&self) -> Option<super::Body> {
        match self {
            Self::Packet(p) => match p.try_encode() {
                Ok(body) => Some(body.into()),
                Err(error) => {
                    eprintln!("[lan] send refused: {error}");
                    None
                }
            },
            Self::Shared(e) => Some(e.body.clone()),
        }
    }
    /// Send to `g`: a shared body stays shared up to the wire.
    pub fn deliver(&self, g: &super::guest::Guest) {
        match self {
            Self::Packet(p) => g.send(p),
            Self::Shared(e) => g.send_shared(&e.packet, &e.body),
        }
    }
}

impl From<C> for Sent {
    fn from(p: C) -> Self {
        Self::Packet(Box::new(p))
    }
}

const NONE: u32 = u32::MAX;

/// `Player.createAttributes`: both waypoint ranges.
pub const PLAYER_RANGE: f64 = 6.0e7;
/// `WaypointTransmitter.REALLY_FAR_DISTANCE`.
const REALLY_FAR: f32 = 332.0;
/// `EntityAzimuthConnection.update`: half a degree.
const AZIMUTH_STEP: f32 = 0.008_726_646;

/// Items that hide their wearer (`Waypoint.addHideAttribute` on the head slot).
pub const HIDING_HEADS: [&str; 8] = [
    "minecraft:carved_pumpkin",
    "minecraft:skeleton_skull",
    "minecraft:wither_skeleton_skull",
    "minecraft:player_head",
    "minecraft:zombie_head",
    "minecraft:creeper_head",
    "minecraft:dragon_head",
    "minecraft:piglin_head",
];

/// A team colour as the locator bar draws it (`Waypoint.Icon.cloneAndAssignStyle`:
/// `TeamColor.rgb()`, black lightened so it shows).
pub fn team_rgb(color: &str) -> Option<i32> {
    let rgb = match color {
        "black" => return Some(-13_619_152),
        "dark_blue" => 0x0000AA,
        "dark_green" => 0x00AA00,
        "dark_aqua" => 0x00AAAA,
        "dark_red" => 0xAA0000,
        "dark_purple" => 0xAA00AA,
        "gold" => 0xFFAA00,
        "gray" => 0xAAAAAA,
        "dark_gray" => 0x555555,
        "blue" => 0x5555FF,
        "green" => 0x55FF55,
        "aqua" => 0x55FFFF,
        "red" => 0xFF5555,
        "light_purple" => 0xFF55FF,
        "yellow" => 0xFFFF55,
        "white" => 0xFFFFFF,
        _ => return None,
    };
    Some(rgb)
}

/// Waypoint.Icon.
#[derive(Clone, Debug, PartialEq)]
pub struct Icon {
    pub style: String,
    pub color: Option<i32>,
}

impl Default for Icon {
    fn default() -> Self {
        Self {
            style: "minecraft:default".into(),
            color: None,
        }
    }
}

/// A player as the waypoint manager sees them this tick.
#[derive(Clone, Debug)]
pub struct Player {
    pub uuid: u128,
    /// The connection that receives the player's locator bar (None: the host,
    /// whose own game draws it).
    pub conn: Option<u32>,
    pub position: [f64; 3],
    pub spectator: bool,
    /// `waypoint_transmit_range`: 0 while hidden.
    pub transmit_range: f64,
    pub receive_range: f64,
    /// Both ranges were read from the player's body. A guest whose body is not made yet has the
    /// default ranges until it is: its first real read is how vanilla's `addPlayer` saw it on
    /// joining, not an attribute change.
    pub ranges_known: bool,
    /// `Entity.firstTick`: the player has not ticked since joining.
    pub first_tick: bool,
    /// The chunk view radius (`ChunkMap.getPlayerViewDistance`).
    pub view_distance: i32,
    /// The player's team and its colour (`TeamColor.rgb`).
    pub team: Option<(String, Option<i32>)>,
    /// The icon `/waypoint modify` set.
    pub icon: Icon,
}

impl Player {
    fn block(&self) -> [i32; 3] {
        self.position.map(|v| v.floor() as i32)
    }
    fn chunk(&self) -> [i32; 2] {
        let [x, _, z] = self.block();
        [x >> 4, z >> 4]
    }
    /// `Entity.distanceTo`.
    fn distance_to(&self, other: &Player) -> f32 {
        let d2: f64 = (0..3)
            .map(|k| (self.position[k] - other.position[k]).powi(2))
            .sum();
        d2.sqrt() as f32
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Block([i32; 3]),
    Chunk([i32; 2]),
    Azimuth(f32),
}

#[derive(Clone, Debug)]
struct Connection {
    kind: Kind,
    icon: Icon,
}

/// What the manager remembers of a player between ticks.
struct Memo {
    position: [f64; 3],
    /// The centre of the player's ChunkTrackingView (`ChunkMap.move`).
    view_center: [i32; 2],
    transmitting: bool,
    /// The computed `waypoint_transmit_range` and `waypoint_receive_range` the manager last saw:
    /// vanilla's `onAttributeUpdated` runs on every change of either, not only across zero.
    transmit_range: f64,
    receive_range: f64,
    ranges_known: bool,
    team: Option<(String, Option<i32>)>,
    icon: Icon,
}

/// One receiver's connections, by source, in the order they were made: a remade
/// connection keeps its place, a broken one leaves it. Found by source in one
/// lookup: a crowd has a few hundred in each row, and every move visits them.
#[derive(Default)]
struct Row {
    slots: Vec<Option<(u32, u128, Connection)>>,
    /// The slot of each source, by its id (`Waypoints::ids`); NONE for a source with none.
    at: Vec<u32>,
    /// What the pair check reads of each connection, by source number, in a line of its own:
    /// `[x, y, z, 1]` of a Block connection's last block, zeros for any other or none.
    last: Vec<[i32; 4]>,
    live: usize,
    /// The transmitters this row has no block connection to (none, or a chunk or an azimuth one), by
    /// number in the order of the waypoints, as of `watch_epoch`: the pairs a calm tick still looks
    /// at when the source moves or the receiver does (see `Waypoints::moves_by_receiver`).
    watch: Vec<u32>,
    watch_epoch: u64,
}

/// A Block connection's last block, as [`Row::last`] keeps it.
fn block_record(kind: &Kind) -> [i32; 4] {
    match *kind {
        Kind::Block([x, y, z]) => [x, y, z, 1],
        _ => [0; 4],
    }
}

impl Row {
    /// The connection, its block brought to the one in `last` (the flat step moves that one on
    /// its own).
    fn get_mut(&mut self, id: u32) -> Option<&mut Connection> {
        let last = *self.last.get(id as usize)?;
        let c = self.raw_mut(id)?;
        if let (1, Kind::Block(b)) = (last[3], &mut c.kind) {
            *b = [last[0], last[1], last[2]];
        }
        Some(c)
    }
    fn raw_mut(&mut self, id: u32) -> Option<&mut Connection> {
        let i = *self.at.get(id as usize)?;
        if i == NONE {
            return None;
        }
        self.slots[i as usize].as_mut().map(|(_, _, c)| c)
    }
    /// Brings [`Self::last`] to what the connection to `id` is now.
    fn sync(&mut self, id: u32) {
        let now = match self.raw_mut(id) {
            Some(c) => block_record(&c.kind),
            None => [0; 4],
        };
        if let Some(last) = self.last.get_mut(id as usize) {
            // A connection that stops or starts being a block one is no longer in, or now is in, `watch`.
            if (last[3] == 1) != (now[3] == 1) {
                self.watch_epoch = 0;
            }
            *last = now;
        }
    }
    /// The row's `watch`, made again when the waypoints or the row's connections changed.
    fn watch_for(&mut self, epoch: u64, own: u32, sources: &[Option<Src>]) -> &[u32] {
        if self.watch_epoch != epoch {
            self.watch.clear();
            for src in sources.iter().flatten() {
                let id = src.source.id;
                if id != own && self.last.get(id as usize).is_none_or(|l| l[3] != 1) {
                    self.watch.push(id);
                }
            }
            self.watch_epoch = epoch;
        }
        &self.watch
    }
    fn put(&mut self, id: u32, source: u128, c: Connection) {
        assert!(id != NONE, "a source has a number");
        self.watch_epoch = 0;
        if let Some(slot) = self.get_mut(id) {
            *slot = c;
            self.sync(id);
            return;
        }
        if self.at.len() <= id as usize {
            self.at.resize(id as usize + 1, NONE);
            self.last.resize(id as usize + 1, [0; 4]);
        }
        self.last[id as usize] = block_record(&c.kind);
        self.at[id as usize] = self.slots.len() as u32;
        self.live += 1;
        self.slots.push(Some((id, source, c)));
    }
    fn take(&mut self, id: u32) -> Option<Connection> {
        let i = std::mem::replace(self.at.get_mut(id as usize)?, NONE);
        if i == NONE {
            return None;
        }
        self.watch_epoch = 0;
        self.live -= 1;
        self.last[id as usize] = [0; 4];
        let c = self.slots[i as usize].take().map(|(_, _, c)| c);
        // The holes are skipped in order, and packed once they are most of the row.
        if self.slots.len() > 16 && self.live * 2 < self.slots.len() {
            self.slots.retain(Option::is_some);
            for (i, slot) in self.slots.iter().enumerate() {
                if let Some((id, _, _)) = slot {
                    self.at[*id as usize] = i as u32;
                }
            }
        }
        c
    }
    /// The sources, in the row's order.
    fn sources(self) -> impl Iterator<Item = u128> {
        self.slots.into_iter().flatten().map(|(_, s, _)| s)
    }
}

/// A receiver's ChunkTrackingView: its centre and radius.
type View = Option<([i32; 2], i32)>;

/// ServerWaypointManager for one level.
#[derive(Default)]
pub struct Waypoints {
    /// The transmitters, in the order they were tracked.
    waypoints: Vec<u128>,
    /// The receivers, in the order they joined.
    players: Vec<u128>,
    /// Connections by receiver, then source.
    connections: HashMap<u128, Row>,
    memo: HashMap<u128, Memo>,
    /// A small number for each player the manager knows, which its rows index their
    /// connections by (a lookup by number is a load, by UUID a hash and a probe).
    ids: Arc<HashMap<u128, u32>>,
    free: Vec<u32>,
    next_id: u32,
    /// The `locator_bar` game rule as last seen.
    enabled: Option<bool>,
    /// Which generation of the waypoints the rows' `watch` lists belong to: the transmitters, their
    /// numbers and the receivers change it.
    epoch: u64,
}

/// What every row reads this tick: the players, read only.
struct Shared<'a> {
    players: HashMap<u128, &'a Player>,
    ids: Arc<HashMap<u128, u32>>,
    /// One waypoint update per mover and tick, encoded once for the receivers it goes to.
    updates: HashMap<u128, OnceLock<Option<Arc<Encoded>>>>,
    /// The connections of the players in other levels: one who just left this level is
    /// still told of the waypoints it loses (`removePlayer` disconnects them).
    away: &'a [(u128, u32)],
    /// Players a respawn replaced this tick: a new ServerPlayer, on its first tick.
    fresh: &'a [u128],
    enabled: bool,
}

/// One receiver's row as a connection to it changes: its view, and where its packets go.
struct Receiving<'r> {
    receiver: u128,
    view: View,
    row: &'r mut Row,
    out: &'r mut Vec<(u32, Sent)>,
}

impl Shared<'_> {
    fn id(&self, uuid: u128) -> u32 {
        self.ids.get(&uuid).copied().unwrap_or(NONE)
    }
    /// What `update_or_create` does for a Block connection whose source has not left its
    /// block and is not ignored: nothing. True only when that is all it would do (a pair that
    /// is not quiet may be too: it then takes the long way), read from flat records alone.
    #[inline]
    fn quiet(row: &Row, src: &Source, me: &Receiver) -> bool {
        if row.last.get(src.id as usize) != Some(&src.record) {
            return false;
        }
        // `doesSourceIgnoreReceiver`, from `Player::distance_to`'s own arithmetic.
        me.spectator
            || (!src.spectator && {
                let d = [0, 1, 2].map(|k| (src.position[k] - me.position[k]).powi(2));
                let range = src.range.min(me.range);
                f64::from((d[0] + d[1] + d[2]).sqrt() as f32) < range
            })
    }
    /// An update packet of `s`'s waypoint, shared with every receiver given the same.
    fn update_packet(&self, s: &Player, icon: &Icon, target: WaypointTarget) -> Sent {
        let own = || Sent::from(waypoint(s.uuid, waypoint_operation::UPDATE, icon, target));
        let Some(cell) = self.updates.get(&s.uuid) else {
            return own();
        };
        let shared = cell.get_or_init(|| {
            let packet = waypoint(s.uuid, waypoint_operation::UPDATE, icon, target);
            let body = packet.try_encode().ok()?;
            Some(Arc::new(Encoded {
                packet,
                body: super::Body::shared(body),
            }))
        });
        match shared {
            Some(e)
                if matches!(&e.packet, C::TrackedWaypoint(w)
                    if w.style == icon.style && w.color == icon.color && w.target == target) =>
            {
                Sent::Shared(Arc::clone(e))
            }
            _ => own(),
        }
    }
    fn send(
        &self,
        out: &mut Vec<(u32, Sent)>,
        receiver: u128,
        source: u128,
        operation: i32,
        icon: &Icon,
        target: WaypointTarget,
    ) {
        let Some(conn) = self
            .players
            .get(&receiver)
            .and_then(|p| p.conn)
            .or_else(|| {
                self.away
                    .iter()
                    .find(|(uuid, _)| *uuid == receiver)
                    .map(|&(_, conn)| conn)
            })
        else {
            return;
        };
        out.push((conn, waypoint(source, operation, icon, target).into()));
    }
    fn connect(&self, out: &mut Vec<(u32, Sent)>, receiver: u128, source: u128, c: &Connection) {
        let target = match c.kind {
            Kind::Block(p) => WaypointTarget::Block(p),
            Kind::Chunk(p) => WaypointTarget::Chunk(p),
            Kind::Azimuth(a) => WaypointTarget::Azimuth(a),
        };
        self.send(
            out,
            receiver,
            source,
            waypoint_operation::TRACK,
            &c.icon,
            target,
        );
    }
    /// `ClientboundTrackedWaypointPacket.removeWaypoint`: an empty waypoint with the null icon.
    fn disconnect(&self, out: &mut Vec<(u32, Sent)>, receiver: u128, source: u128) {
        self.send(
            out,
            receiver,
            source,
            waypoint_operation::UNTRACK,
            &Icon::default(),
            WaypointTarget::Empty,
        );
    }
    /// `ChunkTrackingView.isInViewDistance` of the receiver's view.
    fn chunk_visible(view: View, [x, z]: [i32; 2]) -> bool {
        let Some(([cx, cz], r)) = view else {
            return false;
        };
        let dx = i64::from(((x - cx).abs() - 1).max(0));
        let dz = i64::from(((z - cz).abs() - 1).max(0));
        dx * dx + dz * dz < i64::from(r) * i64::from(r)
    }
    /// `WaypointTransmitter.doesSourceIgnoreReceiver`.
    fn ignores(&self, source: &Player, receiver: &Player) -> bool {
        if receiver.spectator {
            return false;
        }
        if source.spectator {
            return true;
        }
        let range = source.transmit_range.min(receiver.receive_range);
        f64::from(source.distance_to(receiver)) >= range
    }
    fn really_far(source: &Player, receiver: &Player) -> bool {
        source.distance_to(receiver) > REALLY_FAR
    }
    /// `EntityAzimuthConnection`: the angle of the receiver seen from the source,
    /// `Mth.atan2` of the offset turned clockwise.
    fn azimuth(source: &Player, receiver: &Player) -> f32 {
        let d = [0, 1, 2].map(|k| receiver.position[k] - source.position[k]);
        horizon_core::geometry::trig::atan2(d[0], -d[2]) as f32
    }
    /// The source and the receiver, if both are in the level this tick.
    fn pair(&self, source: u128, receiver: u128) -> Option<(&Player, &Player)> {
        Some((*self.players.get(&source)?, *self.players.get(&receiver)?))
    }
    /// The icon of a connection to `s`: the player's own, with the team's colour when it has none
    /// (`Waypoint.Icon.cloneAndAssignStyle`).
    fn effective_icon(s: &Player) -> Icon {
        let mut icon = s.icon.clone();
        if icon.color.is_none() {
            icon.color = s.team.as_ref().and_then(|t| t.1);
        }
        icon
    }
    /// The pair check of a mover meeting a connection that moves by one block, from the flat
    /// records alone: what [`Self::update_or_create`] would do for a block connection one block off
    /// that its source has not left the range of (`update`), done here. False: nothing was done.
    #[inline]
    fn step(row: &mut Row, src: &Src, me: &Receiver, out: &mut Vec<(u32, Sent)>) -> bool {
        #[cfg(test)]
        if tests::NO_STEP.load(std::sync::atomic::Ordering::Relaxed) {
            return false;
        }
        let s = &src.source;
        let Some(last) = row.last.get_mut(s.id as usize) else {
            return false;
        };
        let moved: i32 = (0..3).map(|k| (s.record[k] - last[k]).abs()).sum();
        if last[3] != 1 || moved != 1 {
            return false;
        }
        let in_range = me.spectator
            || (!s.spectator && {
                let d = [0, 1, 2].map(|k| (s.position[k] - me.position[k]).powi(2));
                f64::from((d[0] + d[1] + d[2]).sqrt() as f32) < s.range.min(me.range)
            });
        if !in_range {
            return false;
        }
        let Some(e) = src.update() else {
            return false;
        };
        *last = s.record;
        if let Some(conn) = me.conn {
            out.push((conn, Sent::Shared(Arc::clone(e))));
        }
        true
    }
    /// `LivingEntity.makeWaypointConnectionWith`.
    fn make(&self, view: View, (s, r): (&Player, &Player)) -> Option<Connection> {
        if s.first_tick || self.fresh.contains(&s.uuid) || s.uuid == r.uuid || self.ignores(s, r) {
            return None;
        }
        let icon = Self::effective_icon(s);
        let kind = if Self::really_far(s, r) {
            Kind::Azimuth(Self::azimuth(s, r))
        } else if !Self::chunk_visible(view, s.chunk()) {
            Kind::Chunk(s.chunk())
        } else {
            Kind::Block(s.block())
        };
        Some(Connection { kind, icon })
    }
    /// `Connection.isBroken` for each kind.
    fn broken(&self, view: View, c: &Connection, (s, r): (&Player, &Player)) -> bool {
        match c.kind {
            Kind::Block(last) => {
                let now = s.block();
                (0..3).map(|k| (now[k] - last[k]).abs()).sum::<i32>() > 1 || self.ignores(s, r)
            }
            Kind::Chunk(last) => {
                let now = s.chunk();
                let moved = (now[0] - last[0]).abs().max((now[1] - last[1]).abs());
                moved > 1 || self.ignores(s, r) || Self::chunk_visible(view, last)
            }
            Kind::Azimuth(_) => {
                self.ignores(s, r)
                    || Self::chunk_visible(view, s.chunk())
                    || !Self::really_far(s, r)
            }
        }
    }
    /// `Connection.update`: the packet when the waypoint moved.
    fn update(&self, out: &mut Vec<(u32, Sent)>, c: &mut Connection, (s, r): (&Player, &Player)) {
        let target = match &mut c.kind {
            Kind::Block(last) => {
                let now = s.block();
                if now == *last {
                    return;
                }
                *last = now;
                WaypointTarget::Block(now)
            }
            Kind::Chunk(last) => {
                let now = s.chunk();
                if now == *last {
                    return;
                }
                *last = now;
                WaypointTarget::Chunk(now)
            }
            Kind::Azimuth(last) => {
                let now = Self::azimuth(s, r);
                if (now - *last).abs() <= AZIMUTH_STEP {
                    return;
                }
                *last = now;
                WaypointTarget::Azimuth(now)
            }
        };
        if let Some(conn) = r.conn {
            out.push((conn, self.update_packet(s, &c.icon, target)));
        }
    }

    /// `createConnection` on the receiver's row.
    fn create_connection(&self, to: &mut Receiving, source: u128) {
        let receiver = to.receiver;
        if receiver == source || !self.enabled {
            return;
        }
        match self
            .pair(source, receiver)
            .and_then(|pair| self.make(to.view, pair))
        {
            Some(c) => {
                self.connect(to.out, receiver, source, &c);
                to.row.put(self.id(source), source, c);
            }
            None => {
                if to.row.take(self.id(source)).is_some() {
                    self.disconnect(to.out, receiver, source);
                }
            }
        }
    }
    /// The receiver's connection to the source updated in its place
    /// (`updateConnection`), or made if it has none (`createConnection`).
    fn update_or_create(&self, to: &mut Receiving, source: u128) {
        let id = self.id(source);
        // The connection's block is `last`'s (the flat step moves that one alone) before anything
        // changes it, so that `sync` reads back what the connection is.
        let _ = to.row.get_mut(id);
        self.update_or_create_unsynced(to, source);
        to.row.sync(id);
    }
    fn update_or_create_unsynced(&self, to: &mut Receiving, source: u128) {
        let receiver = to.receiver;
        if receiver == source || !self.enabled {
            return;
        }
        let pair = self.pair(source, receiver);
        let Some(c) = to.row.get_mut(self.id(source)) else {
            return self.create_connection(to, source);
        };
        // A connection to a player who isn't in the level is broken.
        if let Some(pair) = pair
            && !self.broken(to.view, c, pair)
        {
            self.update(to.out, c, pair);
            return;
        }
        match pair.and_then(|pair| self.make(to.view, pair)) {
            Some(new) => {
                self.connect(to.out, receiver, source, &new);
                *c = new;
            }
            None => {
                self.disconnect(to.out, receiver, source);
                to.row.take(self.id(source));
            }
        }
    }
}

fn waypoint(source: u128, operation: i32, icon: &Icon, target: WaypointTarget) -> C {
    C::TrackedWaypoint(TrackedWaypoint {
        operation,
        id: WaypointId::Uuid(source),
        style: icon.style.clone(),
        color: icon.color,
        target,
    })
}

/// One tick's players, the receivers' views, and the packets for their connections.
struct Tick<'a> {
    s: Shared<'a>,
    view: HashMap<u128, ([i32; 2], i32)>,
    out: Vec<(u32, Sent)>,
}

impl Tick<'_> {
    fn disconnect(&mut self, receiver: u128, source: u128) {
        self.s.disconnect(&mut self.out, receiver, source);
    }
}

/// A player who moved this tick, as each receiver's row meets the move
/// ([`Waypoints::moves_by_receiver`]).
struct Move {
    uuid: u128,
    source: Source,
    /// It ticked since joining: its waypoint and its own connections are updated.
    ticked: bool,
    /// Its waypoint is tracked: every receiver's connection to it is updated.
    transmits: bool,
    receives: bool,
    /// Its view's new centre, when it changed chunk.
    new_view: View,
    /// It left the block it was in.
    block_changed: bool,
}

/// A transmitter as the pair check reads it: its number, last-block record, position, range
/// and spectator mode, apart from the `Player`.
#[derive(Clone, Copy)]
struct Source {
    id: u32,
    record: [i32; 4],
    position: [f64; 3],
    range: f64,
    spectator: bool,
}

impl Source {
    fn of(id: u32, p: &Player) -> Self {
        let [x, y, z] = p.block();
        Self {
            id,
            record: [x, y, z, 1],
            position: p.position,
            range: p.transmit_range,
            spectator: p.spectator,
        }
    }
}

/// A receiver as the pair check reads it.
struct Receiver {
    conn: Option<u32>,
    position: [f64; 3],
    range: f64,
    spectator: bool,
}

/// A transmitter for the rows to read this tick: the pair check's record of it, and its update packet,
/// encoded once for the receivers that meet it.
struct Src<'a> {
    source: Source,
    player: &'a Player,
    shared: OnceLock<Option<Arc<Encoded>>>,
}

impl Src<'_> {
    /// The update of its waypoint to the block it is at: what every connection to it that moves by a
    /// block is sent (their icon is the player's own, as `Waypoints::tick_with` sees to).
    fn update(&self) -> Option<&Arc<Encoded>> {
        self.shared
            .get_or_init(|| {
                let p = self.player;
                let packet = waypoint(
                    p.uuid,
                    waypoint_operation::UPDATE,
                    &Shared::effective_icon(p),
                    WaypointTarget::Block(p.block()),
                );
                let body = packet.try_encode().ok()?;
                Some(Arc::new(Encoded {
                    packet,
                    body: super::Body::shared(body),
                }))
            })
            .as_ref()
    }
}

/// Players at which the moves are worked out on several threads, a share of the receivers each.
const MOVES_BY_RECEIVER: usize = 32;

impl Waypoints {
    /// The receiver's row, its view and the tick's packets.
    fn with_row(&mut self, t: &mut Tick, receiver: u128, f: impl FnOnce(&Shared, &mut Receiving)) {
        let mut to = Receiving {
            receiver,
            view: t.view.get(&receiver).copied(),
            row: self.connections.entry(receiver).or_default(),
            out: &mut t.out,
        };
        f(&t.s, &mut to);
    }
    fn take(&mut self, receiver: u128, source: u128) -> Option<Connection> {
        let id = *self.ids.get(&source)?;
        self.connections.get_mut(&receiver)?.take(id)
    }
    /// Numbers for the players this tick has (a number is freed when its player is gone).
    fn number(&mut self, players: &[Player]) {
        for p in players {
            if !self.ids.contains_key(&p.uuid) {
                let id = self.free.pop().unwrap_or_else(|| {
                    self.next_id += 1;
                    self.next_id - 1
                });
                Arc::make_mut(&mut self.ids).insert(p.uuid, id);
            }
        }
    }
    /// A gone player's number goes back, after no row holds a connection to it.
    fn release(&mut self, uuid: u128) {
        self.epoch += 1;
        if let Some(id) = Arc::make_mut(&mut self.ids).remove(&uuid) {
            for row in self.connections.values_mut() {
                row.take(id);
            }
            self.free.push(id);
        }
    }
    /// `createConnection`.
    fn create_connection(&mut self, t: &mut Tick, receiver: u128, source: u128) {
        self.with_row(t, receiver, |s, to| s.create_connection(to, source));
    }
    fn update_or_create(&mut self, t: &mut Tick, receiver: u128, source: u128) {
        self.with_row(t, receiver, |s, to| s.update_or_create(to, source));
    }
    /// `trackWaypoint`.
    fn track(&mut self, t: &mut Tick, source: u128) {
        self.epoch += 1;
        if !self.waypoints.contains(&source) {
            self.waypoints.push(source);
        }
        for receiver in self.players.clone() {
            self.create_connection(t, receiver, source);
        }
    }
    /// `updateWaypoint`.
    fn update_waypoint(&mut self, t: &mut Tick, source: u128) {
        if !self.waypoints.contains(&source) {
            return;
        }
        // Connections change; the players don't.
        for k in 0..self.players.len() {
            self.update_or_create(t, self.players[k], source);
        }
    }
    /// `untrackWaypoint`.
    fn untrack(&mut self, t: &mut Tick, source: u128) {
        self.epoch += 1;
        for receiver in self.players.clone() {
            if self.take(receiver, source).is_some() {
                t.disconnect(receiver, source);
            }
        }
        self.waypoints.retain(|w| *w != source);
    }
    /// `addPlayer`.
    fn add_player(&mut self, t: &mut Tick, player: u128, transmitting: bool) {
        self.epoch += 1;
        if !self.players.contains(&player) {
            self.players.push(player);
        }
        for source in self.waypoints.clone() {
            self.create_connection(t, player, source);
        }
        if transmitting {
            self.track(t, player);
        }
    }
    /// `updatePlayer`.
    fn update_player(&mut self, t: &mut Tick, player: u128) {
        // Connections change; the waypoints don't.
        for k in 0..self.waypoints.len() {
            self.update_or_create(t, player, self.waypoints[k]);
        }
    }
    /// `removePlayer`.
    fn remove_player(&mut self, t: &mut Tick, player: u128) {
        self.remove_player_keeping(t, player, false);
    }
    /// `remove_player`, leaving the player's own waypoint tracked when `keep_waypoint`.
    fn remove_player_keeping(&mut self, t: &mut Tick, player: u128, keep_waypoint: bool) {
        self.epoch += 1;
        for source in self
            .connections
            .remove(&player)
            .unwrap_or_default()
            .sources()
        {
            t.disconnect(player, source);
        }
        if !keep_waypoint {
            self.untrack(t, player);
        }
        self.players.retain(|p| *p != player);
    }
    /// `breakAllConnections`.
    fn break_all(&mut self, t: &mut Tick) {
        self.epoch += 1;
        for receiver in self.players.clone() {
            for source in self
                .connections
                .remove(&receiver)
                .unwrap_or_default()
                .sources()
            {
                t.disconnect(receiver, source);
            }
        }
        self.connections.clear();
    }

    /// One tick: players who left, joined, moved, hid or changed team, and the
    /// `locator_bar` game rule. `respawned` players are removed and added again,
    /// as `PlayerList.respawn` replaces the ServerPlayer. Returns the packets for
    /// each connection.
    pub fn tick(
        &mut self,
        players: &[Player],
        respawned: &[u128],
        locator_bar: bool,
    ) -> Vec<(u32, Sent)> {
        self.tick_level(players, &[], respawned, locator_bar)
    }

    /// [`Self::tick`] for one level of several: `away` are the connections of the players
    /// in the other levels, for the untracks owed to one who has just left this one.
    ///
    /// Each connection's packets go to its receiver, so only their order for each receiver
    /// matters: the packets of different receivers may come out interleaved differently
    /// (the moves worked out by receiver), each receiver's never.
    pub fn tick_level(
        &mut self,
        players: &[Player],
        away: &[(u128, u32)],
        respawned: &[u128],
        locator_bar: bool,
    ) -> Vec<(u32, Sent)> {
        self.tick_with(players, away, respawned, locator_bar, MOVES_BY_RECEIVER)
    }

    /// [`Self::tick_level`], the moves worked out by receiver from `by_receiver` players on.
    fn tick_with(
        &mut self,
        players: &[Player],
        away: &[(u128, u32)],
        respawned: &[u128],
        locator_bar: bool,
        by_receiver: usize,
    ) -> Vec<(u32, Sent)> {
        self.number(players);
        let mut t = Tick {
            s: Shared {
                players: players.iter().map(|p| (p.uuid, p)).collect(),
                ids: Arc::clone(&self.ids),
                updates: players.iter().map(|p| (p.uuid, OnceLock::new())).collect(),
                away,
                fresh: respawned,
                enabled: locator_bar,
            },
            view: HashMap::default(),
            out: Vec::new(),
        };
        // Vanilla reads the rule when it connects; changing it breaks or remakes
        // every connection (MinecraftServer.onGameRuleChanged).
        let was = self.enabled.replace(locator_bar);
        for (uuid, memo) in &self.memo {
            if let Some(p) = t.s.players.get(uuid) {
                t.view.insert(*uuid, (memo.view_center, p.view_distance));
            }
        }
        // Players who left, and the ServerPlayer a respawn replaced: every known entity, receiver or not
        // (`ServerLevel.EntityCallbacks.onTrackingEnd` calls `removePlayer` for each), receivers first
        // in their order, then the rest by id.
        let departed = |uuid: &u128| !t.s.players.contains_key(uuid) || respawned.contains(uuid);
        let mut gone: Vec<u128> = self.players.iter().copied().filter(departed).collect();
        let mut silent: Vec<u128> = self
            .memo
            .keys()
            .copied()
            .filter(|uuid| departed(uuid) && !self.players.contains(uuid))
            .collect();
        silent.sort_unstable();
        gone.extend(silent);
        for uuid in gone {
            self.remove_player(&mut t, uuid);
            self.memo.remove(&uuid);
            if !t.s.players.contains_key(&uuid) {
                self.release(uuid);
            }
        }
        if was == Some(true) && !locator_bar {
            self.break_all(&mut t);
        }
        // Players who joined (PlayerList.placeNewPlayer: addPlayer).
        for p in players {
            if self.memo.contains_key(&p.uuid) {
                continue;
            }
            let center = p.chunk();
            t.view.insert(p.uuid, (center, p.view_distance));
            let transmitting = p.transmit_range > 0.0;
            self.memo.insert(
                p.uuid,
                Memo {
                    position: p.position,
                    view_center: center,
                    transmitting,
                    transmit_range: p.transmit_range,
                    receive_range: p.receive_range,
                    ranges_known: p.ranges_known,
                    team: p.team.clone(),
                    icon: p.icon.clone(),
                },
            );
            if p.receive_range > 0.0 {
                self.add_player(&mut t, p.uuid, transmitting);
            } else if transmitting {
                self.track(&mut t, p.uuid);
            }
        }
        if was == Some(false) && locator_bar {
            for player in self.players.clone() {
                self.update_player(&mut t, player);
            }
        }
        // Nobody hid, showed or changed team: the moves alone, worked out by receiver.
        if players.len() >= by_receiver
            && players.iter().all(|p| {
                let memo = &self.memo[&p.uuid];
                p.transmit_range == memo.transmit_range
                    && p.receive_range == memo.receive_range
                    && (p.transmit_range > 0.0) == memo.transmitting
                    && p.team == memo.team
                    && p.icon == memo.icon
            })
        {
            self.moves_by_receiver(&mut t, players);
            return t.out;
        }
        for p in players {
            let memo = &self.memo[&p.uuid];
            // Entity.setPosRaw, then ServerChunkCache.move.
            if memo.position != p.position {
                let moved_view = memo.view_center != p.chunk();
                let transmitting = memo.transmitting;
                self.memo.get_mut(&p.uuid).unwrap().position = p.position;
                if !p.first_tick {
                    if transmitting {
                        self.update_waypoint(&mut t, p.uuid);
                    }
                    if p.receive_range > 0.0 {
                        self.update_player(&mut t, p.uuid);
                    }
                }
                if moved_view {
                    self.memo.get_mut(&p.uuid).unwrap().view_center = p.chunk();
                    t.view.insert(p.uuid, (p.chunk(), p.view_distance));
                }
                if p.receive_range > 0.0 {
                    self.update_player(&mut t, p.uuid);
                }
            }
            // LivingEntity.onAttributeUpdated(WAYPOINT_TRANSMIT_RANGE).
            let transmitting = p.transmit_range > 0.0;
            let memo = self.memo.get_mut(&p.uuid).unwrap();
            let changed = p.transmit_range != memo.transmit_range;
            memo.transmit_range = p.transmit_range;
            if transmitting != memo.transmitting {
                memo.transmitting = transmitting;
                if transmitting {
                    self.track(&mut t, p.uuid);
                } else {
                    self.untrack(&mut t, p.uuid);
                }
            } else if changed && transmitting {
                // A new value while still transmitting is `trackWaypoint` again.
                self.track(&mut t, p.uuid);
            }
            // ServerPlayer.onAttributeUpdated(WAYPOINT_RECEIVE_RANGE): a receiver
            // is added (its connections remade, and `addPlayer` tracks it again if it transmits,
            // even in the tick the transmit callback tracked it) or removed.
            let memo = self.memo.get_mut(&p.uuid).unwrap();
            // The first read of a body's ranges is the player as it joined, not a change: one that
            // receives nothing just never was a receiver, and its waypoint stays.
            let initial = p.ranges_known && !memo.ranges_known;
            memo.ranges_known = p.ranges_known;
            if p.receive_range != memo.receive_range {
                memo.receive_range = p.receive_range;
                if p.receive_range > 0.0 {
                    self.add_player(&mut t, p.uuid, transmitting);
                } else {
                    self.remove_player_keeping(&mut t, p.uuid, initial && transmitting);
                }
            }
            // ServerScoreboard: a team joined, left or recoloured remakes the connections.
            if p.team != self.memo[&p.uuid].team {
                self.memo.get_mut(&p.uuid).unwrap().team = p.team.clone();
                for receiver in self.players.clone() {
                    self.create_connection(&mut t, receiver, p.uuid);
                }
            }
            // `WaypointCommand.mutateIcon`: a changed icon is untracked, then
            // tracked again, whatever the transmit range.
            if p.icon != self.memo[&p.uuid].icon {
                self.memo.get_mut(&p.uuid).unwrap().icon = p.icon.clone();
                self.untrack(&mut t, p.uuid);
                self.track(&mut t, p.uuid);
            }
        }
        t.out
    }

    /// The moves loop of [`Self::tick_with`] when nobody hid, showed or changed team, row by
    /// row: a move changes the connections to the player who moved (in every receiver's row)
    /// and that player's own row, and a row reads only itself, its receiver's view and the
    /// players. So each receiver meets the moves in the loop's order on its own, on several
    /// threads, and its packets come out as the loop sends them.
    fn moves_by_receiver(&mut self, t: &mut Tick, players: &[Player]) {
        let sparse = Self::calm(&t.s, players);
        let mut moves = Vec::new();
        for player in players {
            let memo = self
                .memo
                .get_mut(&player.uuid)
                .expect("every player has a memo");
            if memo.position == player.position {
                continue;
            }
            let block_changed = memo.position.map(|v| v.floor() as i32) != player.block();
            memo.position = player.position;
            let moved_view = memo.view_center != player.chunk();
            if moved_view {
                memo.view_center = player.chunk();
            }
            moves.push(Move {
                uuid: player.uuid,
                ticked: !player.first_tick,
                transmits: memo.transmitting && self.waypoints.contains(&player.uuid),
                receives: player.receive_range > 0.0,
                new_view: moved_view.then(|| (player.chunk(), player.view_distance)),
                source: Source::of(t.s.id(player.uuid), player),
                block_changed,
            });
        }
        if moves.is_empty() {
            return;
        }
        // Every row a move reaches: the receivers', and those of movers who receive but aren't
        // listed as receivers (`updatePlayer` runs on their own row all the same).
        struct Job {
            receiver: u128,
            listed: bool,
            view: View,
            row: Row,
            out: Vec<(u32, Sent)>,
        }
        let mut jobs: Vec<Job> = self
            .players
            .iter()
            .map(|&receiver| (receiver, true))
            .chain(
                moves
                    .iter()
                    .filter(|m| m.receives && !self.players.contains(&m.uuid))
                    .map(|m| (m.uuid, false)),
            )
            .map(|(receiver, listed)| Job {
                receiver,
                listed,
                view: t.view.get(&receiver).copied(),
                row: self.connections.remove(&receiver).unwrap_or_default(),
                // A receiver is sent a few dozen in a crowd: no growing by doubling.
                out: Vec::with_capacity(64),
            })
            .collect();
        // The transmitters as the rows meet them: player, number, block, and the update packet.
        let sources: Vec<Option<Src>> = self
            .waypoints
            .iter()
            .map(|u| {
                t.s.players.get(u).map(|&p| Src {
                    source: Source::of(t.s.id(*u), p),
                    player: p,
                    shared: OnceLock::new(),
                })
            })
            .collect();
        let mut slot = vec![NONE; self.next_id as usize];
        for (i, src) in sources.iter().enumerate() {
            if let Some(src) = src {
                slot[src.source.id as usize] = i as u32;
            }
        }
        // What a calm tick looks at (see `calm`): the movers that changed block, in the order of the
        // moves, and the waypoints that changed block, in the order of the waypoints; the moves by
        // number; where each receiver's own move is.
        let sparse = sparse && sources.iter().all(Option::is_some);
        #[cfg(test)]
        let sparse = sparse && !tests::NO_SPARSE.load(std::sync::atomic::Ordering::Relaxed);
        #[cfg(test)]
        if sparse {
            tests::SPARSE_TICKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        let mut changed_moves: Vec<u32> = Vec::new();
        let mut changed_slots: Vec<u32> = Vec::new();
        let mut move_of = vec![NONE; self.next_id as usize];
        let mut own: HashMap<u128, u32> = HashMap::default();
        if sparse {
            for (i, m) in moves.iter().enumerate() {
                own.insert(m.uuid, i as u32);
                if m.source.id != NONE {
                    move_of[m.source.id as usize] = i as u32;
                }
                if m.block_changed {
                    if m.ticked && m.transmits {
                        changed_moves.push(i as u32);
                    }
                    if let Some(&at) = slot.get(m.source.id as usize)
                        && at != NONE
                    {
                        changed_slots.push(at);
                    }
                }
            }
            changed_slots.sort_unstable();
        }
        let all_moves: Vec<u32> = if sparse {
            Vec::new()
        } else {
            (0..moves.len() as u32).collect()
        };
        let all_slots: Vec<u32> = if sparse {
            Vec::new()
        } else {
            (0..sources.len() as u32).collect()
        };
        let epoch = self.epoch + 1;
        let (s, waypoints, moves) = (&t.s, &self.waypoints, &moves);
        let (sources, slot, move_of, own) = (&sources, &slot, &move_of, &own);
        let (changed_moves, changed_slots) = (&changed_moves, &changed_slots);
        let (all_moves, all_slots) = (&all_moves, &all_slots);
        horizon_threads::par_chunks_mut(&mut jobs, 8, |jobs| {
            for job in jobs {
                let me = s.players.get(&job.receiver).map(|&p| Receiver {
                    conn: p.conn,
                    position: p.position,
                    range: p.receive_range,
                    spectator: p.spectator,
                });
                // The moves this row meets, in order, and the waypoints it sweeps: everything, or in a
                // calm tick what changed, the pairs this row has no block connection for, and its own.
                let mut own_moves: Vec<u32> = Vec::new();
                let mut own_slots: Vec<u32> = Vec::new();
                if sparse {
                    let own_id = s.id(job.receiver);
                    let watch = job.row.watch_for(epoch, own_id, sources);
                    for &id in watch {
                        if let Some(&mi) = move_of.get(id as usize)
                            && mi != NONE
                        {
                            let m = &moves[mi as usize];
                            if m.ticked && m.transmits && !m.block_changed {
                                own_moves.push(mi);
                            }
                        }
                        own_slots.push(slot[id as usize]);
                    }
                    if let Some(&mi) = own.get(&job.receiver)
                        && changed_moves.binary_search(&mi).is_err()
                    {
                        own_moves.push(mi);
                    }
                    own_slots.retain(|k| changed_slots.binary_search(k).is_err());
                    if !own_moves.is_empty() {
                        own_moves.extend_from_slice(changed_moves);
                        own_moves.sort_unstable();
                    }
                    if !own_slots.is_empty() {
                        own_slots.extend_from_slice(changed_slots);
                        own_slots.sort_unstable();
                    }
                }
                let (meet, swept): (&[u32], &[u32]) = if !sparse {
                    (all_moves, all_slots)
                } else {
                    (
                        if own_moves.is_empty() {
                            changed_moves
                        } else {
                            &own_moves
                        },
                        if own_slots.is_empty() {
                            changed_slots
                        } else {
                            &own_slots
                        },
                    )
                };
                let mut to = Receiving {
                    receiver: job.receiver,
                    view: job.view,
                    row: &mut job.row,
                    out: &mut job.out,
                };
                for &mi in meet {
                    let m = &moves[mi as usize];
                    if m.uuid != job.receiver {
                        // `updateWaypoint`: this receiver's connection to the mover.
                        if job.listed && m.ticked && m.transmits {
                            // Most movers stay in their block: nothing to send or remake.
                            let done = me.as_ref().is_some_and(|me| {
                                Shared::quiet(to.row, &m.source, me)
                                    || slot
                                        .get(m.source.id as usize)
                                        .and_then(|&i| sources.get(i as usize)?.as_ref())
                                        .is_some_and(|src| Shared::step(to.row, src, me, to.out))
                            });
                            if !done {
                                s.update_or_create(&mut to, m.uuid);
                            }
                        }
                        continue;
                    }
                    // `updatePlayer` before and after the view follows the move.
                    let sweep = |to: &mut Receiving| {
                        for &k in swept {
                            let (source, known) = (waypoints[k as usize], &sources[k as usize]);
                            if let (Some(src), Some(me)) = (known, &me)
                                && (Shared::quiet(to.row, &src.source, me)
                                    || Shared::step(to.row, src, me, to.out))
                            {
                                continue;
                            }
                            s.update_or_create(to, source);
                        }
                    };
                    if m.ticked && m.receives {
                        sweep(&mut to);
                    }
                    if m.new_view.is_some() {
                        to.view = m.new_view;
                    }
                    if m.receives {
                        sweep(&mut to);
                    }
                }
            }
        });
        t.out.reserve(jobs.iter().map(|j| j.out.len()).sum());
        for job in jobs {
            t.out.extend(job.out);
            self.connections.insert(job.receiver, job.row);
        }
    }
    /// Whether this tick's moves can be met by what changed alone. A pair neither of whose players
    /// changed block, and whose connection is a block one in range, has nothing to send: the check
    /// of the one loop (`Shared::quiet`) comes out the same for it. That holds while nobody can be
    /// out of range or ignored (no spectator, every range the player's own, every player within
    /// 17 000 000 blocks of the origin, so no two are as far as the range), and nobody is a respawned
    /// player or the rule is off.
    fn calm(s: &Shared, players: &[Player]) -> bool {
        s.enabled
            && s.fresh.is_empty()
            && players.iter().all(|p| {
                !p.spectator
                    && p.receive_range == PLAYER_RANGE
                    && (p.transmit_range == PLAYER_RANGE || p.transmit_range == 0.0)
                    && p.position.iter().all(|c| c.abs() <= 1.7e7)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A measurement's switches: the flat step off, the sparse moves off (the long way for every pair).
    pub(super) static NO_STEP: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);
    pub(super) static NO_SPARSE: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);
    /// Ticks the moves were met by what changed alone.
    pub(super) static SPARSE_TICKS: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);

    fn player(uuid: u128, conn: u32, position: [f64; 3]) -> Player {
        Player {
            uuid,
            conn: Some(conn),
            position,
            spectator: false,
            transmit_range: PLAYER_RANGE,
            receive_range: PLAYER_RANGE,
            ranges_known: true,
            first_tick: false,
            view_distance: 4,
            team: None,
            icon: Icon::default(),
        }
    }

    /// The packets to `conn`, as (operation, target).
    fn to(out: &[(u32, Sent)], conn: u32) -> Vec<(i32, WaypointTarget)> {
        out.iter()
            .filter(|(c, _)| *c == conn)
            .map(|(_, p)| match p.packet() {
                C::TrackedWaypoint(w) => (w.operation, w.target),
                other => panic!("{other:?}"),
            })
            .collect()
    }

    #[test]
    fn a_new_player_shows_once_it_moves() {
        let mut w = Waypoints::default();
        let a = player(1, 10, [0.5, 100.0, 0.5]);
        let mut b = player(2, 20, [5.5, 100.0, 0.5]);
        b.first_tick = true;
        w.tick(std::slice::from_ref(&a), &[], true);
        let out = w.tick(&[a.clone(), b.clone()], &[], true);
        // B gets A at once; A sees nothing of B while B hasn't ticked.
        assert_eq!(to(&out, 20), [(0, WaypointTarget::Block([0, 100, 0]))]);
        assert!(to(&out, 10).is_empty());
        b.first_tick = false;
        assert!(w.tick(&[a.clone(), b.clone()], &[], true).is_empty());
        b.position[0] += 1.0;
        let out = w.tick(&[a.clone(), b.clone()], &[], true);
        assert_eq!(to(&out, 10), [(0, WaypointTarget::Block([6, 100, 0]))]);
        // One block: an update. Three: a new connection. Out of view: a chunk.
        b.position[0] += 1.0;
        let out = w.tick(&[a.clone(), b.clone()], &[], true);
        assert_eq!(to(&out, 10), [(2, WaypointTarget::Block([7, 100, 0]))]);
        b.position[0] += 3.0;
        let out = w.tick(&[a.clone(), b.clone()], &[], true);
        assert_eq!(to(&out, 10), [(0, WaypointTarget::Block([10, 100, 0]))]);
        b.position[0] = 100.5;
        let out = w.tick(&[a.clone(), b.clone()], &[], true);
        assert_eq!(to(&out, 10), [(0, WaypointTarget::Chunk([6, 0]))]);
        // Really far: an angle.
        b.position[0] = 1000.5;
        let out = w.tick(&[a.clone(), b.clone()], &[], true);
        assert!(matches!(
            to(&out, 10)[..],
            [(0, WaypointTarget::Azimuth(_))]
        ));
        // Hidden: untracked; the rule off: every connection broken.
        b.transmit_range = 0.0;
        let out = w.tick(&[a.clone(), b.clone()], &[], true);
        assert_eq!(to(&out, 10), [(1, WaypointTarget::Empty)]);
        let out = w.tick(&[a.clone(), b.clone()], &[], false);
        assert_eq!(to(&out, 20), [(1, WaypointTarget::Empty)]);
        assert!(to(&out, 10).is_empty());
        // Leaving untracks what was left.
        let out = w.tick(&[a.clone(), b.clone()], &[], true);
        assert!(matches!(
            to(&out, 20)[..],
            [(0, WaypointTarget::Azimuth(_))]
        ));
        let out = w.tick(&[b.clone()], &[], true);
        assert_eq!(to(&out, 20), [(1, WaypointTarget::Empty)]);
    }

    /// What a connection to `source` shows, as (operation, style, colour).
    fn icons(out: &[(u32, Sent)], conn: u32) -> Vec<(i32, String, Option<i32>)> {
        out.iter()
            .filter(|(c, _)| *c == conn)
            .map(|(_, p)| match p.packet() {
                C::TrackedWaypoint(w) => (w.operation, w.style.clone(), w.color),
                other => panic!("{other:?}"),
            })
            .collect()
    }

    #[test]
    fn a_changed_icon_is_untracked_and_tracked_again_for_whoever_sees_it() {
        let (a, mut b) = (
            player(1, 10, [0.5, 64.0, 0.5]),
            player(2, 20, [4.5, 64.0, 0.5]),
        );
        let mut w = Waypoints::default();
        let out = w.tick(&[a.clone(), b.clone()], &[], true);
        assert_eq!(
            icons(&out, 10),
            [(0, "minecraft:default".into(), None)],
            "A sees B's default icon"
        );
        assert!(w.tick(&[a.clone(), b.clone()], &[], true).is_empty());

        b.icon = Icon {
            style: "example:flag".into(),
            color: Some(0x12_34AB),
        };
        let out = w.tick(&[a.clone(), b.clone()], &[], true);
        assert_eq!(
            icons(&out, 10),
            [
                (
                    waypoint_operation::UNTRACK,
                    "minecraft:default".into(),
                    None
                ),
                (
                    waypoint_operation::TRACK,
                    "example:flag".into(),
                    Some(0x12_34AB)
                ),
            ]
        );
        // B's own bar shows A as before: B's icon is not A's.
        assert!(icons(&out, 20).is_empty());
        assert!(w.tick(&[a.clone(), b.clone()], &[], true).is_empty());

        // The icon is the player's own, not their team's colour, once it has one.
        b.team = Some(("red".into(), Some(0xFF5555)));
        let out = w.tick(&[a.clone(), b.clone()], &[], true);
        assert_eq!(
            icons(&out, 10).last(),
            Some(&(
                waypoint_operation::TRACK,
                "example:flag".into(),
                Some(0x12_34AB)
            ))
        );
    }

    #[test]
    fn a_hidden_players_changed_icon_shows_nobody_anything() {
        let (a, mut b) = (
            player(1, 10, [0.5, 64.0, 0.5]),
            player(2, 20, [4.5, 64.0, 0.5]),
        );
        b.transmit_range = 0.0;
        let mut w = Waypoints::default();
        w.tick(&[a.clone(), b.clone()], &[], true);
        b.icon.style = "example:flag".into();
        assert!(w.tick(&[a.clone(), b.clone()], &[], true).is_empty());
        // Showing again carries the new style.
        b.transmit_range = PLAYER_RANGE;
        let out = w.tick(&[a, b], &[], true);
        assert_eq!(
            icons(&out, 10),
            [(waypoint_operation::TRACK, "example:flag".into(), None)]
        );
    }

    #[test]
    fn a_player_leaving_the_level_loses_its_waypoints() {
        let (a, b) = (
            player(1, 10, [0.5, 64.0, 0.5]),
            player(2, 20, [4.5, 64.0, 0.5]),
        );
        let mut w = Waypoints::default();
        w.tick(&[a.clone(), b.clone()], &[], true);
        let mut moved = b.clone();
        moved.position = [5.5, 64.0, 0.5];
        w.tick(&[a.clone(), moved], &[], true);
        // B goes to another level: A loses B, and B (reached through `away`) loses A.
        let out = w.tick_level(std::slice::from_ref(&a), &[(2, 20)], &[], true);
        assert_eq!(
            to(&out, 10),
            vec![(waypoint_operation::UNTRACK, WaypointTarget::Empty)]
        );
        assert_eq!(
            to(&out, 20),
            vec![(waypoint_operation::UNTRACK, WaypointTarget::Empty)]
        );
    }

    /// A player who receives nothing is still a source (`onTrackingEnd` calls `removePlayer` for
    /// every ServerPlayer): leaving the level, or being replaced, untracks it, and it joins afresh.
    #[test]
    fn a_player_receiving_nothing_leaves_and_returns_like_any_other() {
        let a = player(1, 10, [0.5, 64.0, 0.5]);
        let mut b = player(2, 20, [4.5, 64.0, 0.5]);
        let untrack = vec![(waypoint_operation::UNTRACK, WaypointTarget::Empty)];
        let track = vec![(waypoint_operation::TRACK, WaypointTarget::Block([4, 64, 0]))];
        for respawn in [false, true] {
            let mut w = Waypoints::default();
            w.tick(&[a.clone(), b.clone()], &[], true);
            // Receiving nothing removes B's reception and (vanilla) its waypoint; a new transmit
            // value tracks it again, and it is a source still.
            b.receive_range = 0.0;
            let out = w.tick(&[a.clone(), b.clone()], &[], true);
            assert_eq!(to(&out, 10), untrack);
            b.transmit_range = 10.0;
            let out = w.tick(&[a.clone(), b.clone()], &[], true);
            assert_eq!(to(&out, 10), track);
            // B leaves (or respawns): A loses it, though B never was a receiver.
            let out = if respawn {
                w.tick_level(&[a.clone(), b.clone()], &[], &[2], true)
            } else {
                w.tick_level(std::slice::from_ref(&a), &[(2, 20)], &[], true)
            };
            assert_eq!(to(&out, 10), untrack);
            if respawn {
                // The new ServerPlayer shows once it has ticked and moved.
                b.position[0] += 1.0;
                let out = w.tick(&[a.clone(), b.clone()], &[], true);
                assert_eq!(
                    to(&out, 10),
                    [(waypoint_operation::TRACK, WaypointTarget::Block([5, 64, 0]))]
                );
                b.position[0] -= 1.0;
            } else {
                assert!(!w.memo.contains_key(&2));
                // It comes back with nothing changed: joined afresh, tracked, receiving nothing.
                let out = w.tick(&[a.clone(), b.clone()], &[], true);
                assert_eq!(to(&out, 10), track);
                assert!(to(&out, 20).is_empty());
            }
            b.receive_range = 60_000_000.0;
            b.transmit_range = 60_000_000.0;
        }
    }

    /// Both ranges change in one tick: the transmit callback tracks B and the receive callback's
    /// `addPlayer` tracks it again (`trackWaypoint` always sends), so A is told twice.
    #[test]
    fn both_ranges_changing_in_one_tick_track_twice() {
        let a = player(1, 10, [0.5, 64.0, 0.5]);
        let mut b = player(2, 20, [4.5, 64.0, 0.5]);
        let mut w = Waypoints::default();
        w.tick(&[a.clone(), b.clone()], &[], true);
        b.transmit_range = 5.0e7;
        b.receive_range = 5.0e7;
        let out = w.tick(&[a, b], &[], true);
        let track = (waypoint_operation::TRACK, WaypointTarget::Block([4, 64, 0]));
        assert_eq!(to(&out, 10), vec![track; 2]);
    }

    /// A guest first read without a body has the default ranges; when its saved receive range of
    /// zero arrives, that is how it joined, not a change: it stays a source for the others.
    #[test]
    fn a_guests_first_real_ranges_do_not_untrack_it() {
        let a = player(1, 10, [0.5, 64.0, 0.5]);
        let mut b = player(2, 20, [4.5, 64.0, 0.5]);
        b.ranges_known = false;
        let mut w = Waypoints::default();
        w.tick(&[a.clone(), b.clone()], &[], true);
        b.ranges_known = true;
        b.receive_range = 0.0;
        let out = w.tick(&[a.clone(), b.clone()], &[], true);
        assert!(to(&out, 10).is_empty());
        // It receives nothing now: it only drops A's marker.
        assert_eq!(
            to(&out, 20),
            [(waypoint_operation::UNTRACK, WaypointTarget::Empty)]
        );
        // Moving on, A still hears of it.
        b.position[0] += 1.0;
        let out = w.tick(&[a, b], &[], true);
        assert_eq!(
            to(&out, 10),
            [(
                waypoint_operation::UPDATE,
                WaypointTarget::Block([5, 64, 0])
            )]
        );
    }

    /// Each connection's packets as bytes, in the order it is sent them.
    fn by_conn(out: &[(u32, Sent)]) -> std::collections::BTreeMap<u32, Vec<Vec<u8>>> {
        let mut by = std::collections::BTreeMap::<u32, Vec<Vec<u8>>>::new();
        for (conn, packet) in out {
            let bytes = packet.packet().encode();
            if let Sent::Shared(e) = packet {
                assert_eq!(&*e.body, &bytes[..], "a shared body is its packet's bytes");
            }
            by.entry(*conn).or_default().push(bytes);
        }
        by
    }

    /// The moves worked out by receiver give every connection the bytes the one loop gives,
    /// in its order: crowds walking, crossing chunks and going far, with joins, leaves,
    /// respawns, hiding, team changes, spectators and the rule switched (which take the loop).
    #[test]
    #[ignore = "daily"]
    fn moves_by_receiver_match_the_loop() {
        // Most players move, as a crowd walking.
        crowds_match_the_loop((3, 4), true);
    }

    /// A hub: most players stand still (1 in 20, then 3 in 10 move a tick), with the events that
    /// do not take the tick off the sparse path (joins, leaves, respawns, hiding, teams, icons), and
    /// with every event, spectators and other ranges too: the moves are met by what changed in the
    /// first, by the whole loop in the second, and either way the bytes of every connection are
    /// the one loop's.
    #[test]
    #[ignore = "daily"]
    fn moves_by_receiver_match_the_loop_when_most_players_stand_still() {
        let before = SPARSE_TICKS.load(std::sync::atomic::Ordering::Relaxed);
        crowds_match_the_loop((1, 20), false);
        crowds_match_the_loop((3, 10), false);
        crowds_match_the_loop((1, 2), false);
        let met = SPARSE_TICKS.load(std::sync::atomic::Ordering::Relaxed) - before;
        assert!(met > 150, "the calm ticks took the sparse path ({met})");
        crowds_match_the_loop((1, 20), true);
        crowds_match_the_loop((3, 10), true);
    }

    fn crowds_match_the_loop(move_odds: (u64, u64), all_events: bool) {
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = move |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        let (mut sent, mut shared) = (0, 0);
        for level in 0..16 {
            let (mut serial, mut by_receiver) = (Waypoints::default(), Waypoints::default());
            let count = 2 + next(60) as u32;
            let spread = [4.0, 40.0, 400.0, 4000.0][level % 4];
            let mut players: Vec<Player> = (0..count)
                .map(|i| {
                    let mut p = player(
                        u128::from(i) * 7 + 1,
                        100 + i,
                        [
                            (next(1000) as f64 / 1000.0 - 0.5) * spread,
                            64.0,
                            (next(1000) as f64 / 1000.0 - 0.5) * spread,
                        ],
                    );
                    p.view_distance = 2 + next(10) as i32;
                    p.first_tick = next(4) == 0;
                    p
                })
                .collect();
            let mut joined = count;
            for tick in 0..30 {
                let mut respawned = Vec::new();
                let calm = tick % 3 != 0;
                for p in &mut players {
                    p.first_tick = p.first_tick && next(3) == 0;
                    if next(move_odds.1) < move_odds.0 {
                        let step = if next(20) == 0 { spread } else { 1.5 };
                        p.position[0] += (next(1000) as f64 / 1000.0 - 0.5) * step;
                        p.position[2] += (next(1000) as f64 / 1000.0 - 0.5) * step;
                    }
                    if !calm {
                        match next(40) {
                            0 => {
                                p.transmit_range = if p.transmit_range > 0.0 {
                                    0.0
                                } else {
                                    PLAYER_RANGE
                                }
                            }
                            1 => p.team = Some(("red".into(), team_rgb("red"))),
                            2 => p.team = None,
                            3 if all_events => p.spectator = !p.spectator,
                            4 => respawned.push(p.uuid),
                            5 => {
                                p.icon = if p.icon == Icon::default() {
                                    Icon {
                                        style: "example:flag".into(),
                                        color: Some(0x12_34AB),
                                    }
                                } else {
                                    Icon::default()
                                }
                            }
                            6 if all_events => {
                                p.receive_range = if p.receive_range == PLAYER_RANGE {
                                    100.0
                                } else {
                                    PLAYER_RANGE
                                }
                            }
                            7 if all_events => {
                                p.transmit_range = if p.transmit_range == PLAYER_RANGE {
                                    30.0
                                } else {
                                    PLAYER_RANGE
                                }
                            }
                            _ => {}
                        }
                    }
                }
                if !calm && next(3) == 0 && players.len() > 2 {
                    players.remove(next(players.len() as u64) as usize);
                }
                if !calm && next(3) == 0 {
                    joined += 1;
                    let mut p = player(u128::from(joined) * 7 + 1, 100 + joined, [0.5, 64.0, 0.5]);
                    p.first_tick = true;
                    players.push(p);
                }
                let rule = calm || next(10) != 0;
                let a = serial.tick_with(&players, &[], &respawned, rule, usize::MAX);
                let b = by_receiver.tick_with(&players, &[], &respawned, rule, 0);
                sent += a.len();
                shared += b
                    .iter()
                    .filter(|(_, p)| matches!(p, Sent::Shared(_)))
                    .count();
                let (ba, bb) = (by_conn(&a), by_conn(&b));
                if ba != bb {
                    let show = |out: &[(u32, Sent)], conn: u32| -> Vec<String> {
                        out.iter()
                            .filter(|(c, _)| *c == conn)
                            .map(|(_, p)| match p.packet() {
                                C::TrackedWaypoint(w) => {
                                    format!("{}:{:?}:{:?}", w.operation, w.id, w.target)
                                }
                                _ => String::new(),
                            })
                            .collect()
                    };
                    let conn = ba
                        .keys()
                        .chain(bb.keys())
                        .copied()
                        .find(|c| ba.get(c) != bb.get(c))
                        .unwrap();
                    panic!(
                        "odds {move_odds:?} events {all_events} level {level}, tick {tick}, conn {conn}\n serial {:#?}\n rows {:#?}",
                        show(&a, conn),
                        show(&b, conn)
                    );
                }
            }
        }
        let (least, least_shared) = match move_odds.0 * 100 / move_odds.1 {
            0..=10 => (300, 20),
            11..=40 => (1_000, 100),
            _ => (4_000, 1_000),
        };
        assert!(
            sent > least,
            "the crowds were sent their waypoints ({sent}, odds {move_odds:?})"
        );
        assert!(
            shared > least_shared,
            "most updates are shared bodies ({shared}, odds {move_odds:?})"
        );
        assert!(shared > 1_000, "most updates are shared bodies ({shared})");
    }

    /// A crowd of `n` bots in a few chunks, `movers` percent of them walking, the moves worked out by
    /// receiver: (updates sent, ticks). Run under `perf stat -e instructions:u,cache-misses:u,
    /// L1-dcache-load-misses:u` with `--ignored --nocapture`: the cost of the manager alone.
    fn crowd(n: u32, ticks: u32, movers: u32) -> (usize, usize) {
        let mut seed = 0x1234_5678_9abc_def1_u64;
        let mut next = move |m: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % m
        };
        let side = (f64::from(n) * 2.0).sqrt().max(8.0);
        let mut players: Vec<Player> = (0..n)
            .map(|i| {
                player(
                    u128::from(i) * 7 + 1,
                    100 + i,
                    [
                        next(1000) as f64 / 1000.0 * side,
                        -36.0,
                        next(1000) as f64 / 1000.0 * side,
                    ],
                )
            })
            .collect();
        let walks: Vec<bool> = (0..n).map(|_| next(100) < u64::from(movers)).collect();
        let mut heading: Vec<f64> = (0..n).map(|_| next(6283) as f64 / 1000.0).collect();
        let mut w = Waypoints::default();
        w.tick_with(&players, &[], &[], true, 0);
        let (mut sent, mut counted) = (0, 0);
        for t in 0..ticks {
            for ((p, h), walk) in players.iter_mut().zip(&mut heading).zip(&walks) {
                if !walk {
                    continue;
                }
                if next(40) == 0 {
                    *h = next(6283) as f64 / 1000.0;
                }
                p.position[0] = (p.position[0] + 0.28 * h.cos()).clamp(0.0, side);
                p.position[2] = (p.position[2] + 0.28 * h.sin()).clamp(0.0, side);
                p.first_tick = false;
            }
            let out = w.tick_with(&players, &[], &[], true, 0);
            if t >= 50 {
                sent += out.len();
                counted += 1;
            }
        }
        (sent, counted)
    }

    #[test]
    #[ignore = "daily"]
    fn crowd_cost() {
        let var = |name: &str, default: u32| {
            std::env::var(name)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        };
        NO_STEP.store(
            std::env::var_os("NO_STEP").is_some(),
            std::sync::atomic::Ordering::Relaxed,
        );
        NO_SPARSE.store(
            std::env::var_os("NO_SPARSE").is_some(),
            std::sync::atomic::Ordering::Relaxed,
        );
        let (n, ticks, movers) = (var("CROWD", 200), var("TICKS", 600), var("MOVERS", 100));
        let (sent, counted) = crowd(n, ticks, movers);
        println!(
            "crowd {n}, {movers} % walking: {sent} packets in {counted} ticks = {} a tick",
            sent / counted
        );
    }

    /// The floor of the receiver x mover check, on flat arrays: the source's block compared with
    /// the one its connections last showed, and the receiver's distance to it against the range
    /// (`ignores`). Run with `cargo test --release ... flat_pair_floor -- --ignored --nocapture`
    /// under `perf stat -e instructions:u`: the prototype's cost per ordered pair.
    #[test]
    #[ignore = "daily"]
    fn flat_pair_floor() {
        let n = 200usize;
        let mut seed = 12345u64;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % 1000) as f64 / 1000.0
        };
        let (mut x, mut y, mut z) = (vec![0.0f64; n], vec![0.0f64; n], vec![0.0f64; n]);
        let mut last = vec![[0i32; 3]; n];
        for i in 0..n {
            x[i] = rnd() * 6.0;
            y[i] = -36.0;
            z[i] = rnd() * 6.0;
            last[i] = [
                x[i].floor() as i32,
                y[i].floor() as i32,
                z[i].floor() as i32,
            ];
        }
        let range2 = PLAYER_RANGE * PLAYER_RANGE;
        let ticks = 2_000;
        let mut changed = 0u64;
        for t in 0..ticks {
            // Each source steps a fraction of a block; its block changes now and then.
            for i in 0..n {
                x[i] += 0.2 * (((t + i) % 7) as f64 / 7.0);
            }
            for m in 0..n {
                let b = [
                    x[m].floor() as i32,
                    y[m].floor() as i32,
                    z[m].floor() as i32,
                ];
                for r in 0..n {
                    if r == m {
                        continue;
                    }
                    let (dx, dy, dz) = (x[r] - x[m], y[r] - y[m], z[r] - z[m]);
                    if b == last[m] && dx * dx + dy * dy + dz * dz < range2 {
                        continue;
                    }
                    changed += 1;
                }
                last[m] = b;
            }
        }
        let pairs = (ticks * n * (n - 1)) as u64;
        println!("flat floor: {pairs} ordered pairs, {changed} needing a packet");
    }

    #[test]
    fn team_colours() {
        assert_eq!(team_rgb("red"), Some(0xFF5555));
        assert_eq!(team_rgb("black"), Some(0xFF30_3030_u32 as i32));
        assert_eq!(team_rgb("reset"), None);
    }
}
