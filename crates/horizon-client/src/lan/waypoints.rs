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
    team: Option<(String, Option<i32>)>,
}

/// One receiver's connections, by source, in the order they were made: a remade
/// connection keeps its place, a broken one leaves it. Found by source in one
/// lookup: a crowd has a few hundred in each row, and every move visits them.
#[derive(Default)]
struct Row {
    slots: Vec<Option<(u128, Connection)>>,
    at: HashMap<u128, usize>,
}

impl Row {
    fn get_mut(&mut self, source: u128) -> Option<&mut Connection> {
        let i = *self.at.get(&source)?;
        self.slots[i].as_mut().map(|(_, c)| c)
    }
    fn put(&mut self, source: u128, c: Connection) {
        if let Some(slot) = self.get_mut(source) {
            *slot = c;
            return;
        }
        self.at.insert(source, self.slots.len());
        self.slots.push(Some((source, c)));
    }
    fn take(&mut self, source: u128) -> Option<Connection> {
        let i = self.at.remove(&source)?;
        let c = self.slots[i].take().map(|(_, c)| c);
        // The holes are skipped in order, and packed once they are most of the row.
        if self.slots.len() > 16 && self.at.len() * 2 < self.slots.len() {
            self.slots.retain(Option::is_some);
            for (i, slot) in self.slots.iter().enumerate() {
                if let Some((s, _)) = slot {
                    self.at.insert(*s, i);
                }
            }
        }
        c
    }
    /// The sources, in the row's order.
    fn sources(self) -> impl Iterator<Item = u128> {
        self.slots.into_iter().flatten().map(|(s, _)| s)
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
    /// The `locator_bar` game rule as last seen.
    enabled: Option<bool>,
}

/// What every row reads this tick: the players, read only.
struct Shared<'a> {
    players: HashMap<u128, &'a Player>,
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
    out: &'r mut Vec<(u32, C)>,
}

impl Shared<'_> {
    fn send(
        &self,
        out: &mut Vec<(u32, C)>,
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
        out.push((conn, waypoint(source, operation, icon, target)));
    }
    fn connect(&self, out: &mut Vec<(u32, C)>, receiver: u128, source: u128, c: &Connection) {
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
    fn disconnect(&self, out: &mut Vec<(u32, C)>, receiver: u128, source: u128) {
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
    /// `LivingEntity.makeWaypointConnectionWith`.
    fn make(&self, view: View, (s, r): (&Player, &Player)) -> Option<Connection> {
        if s.first_tick || self.fresh.contains(&s.uuid) || s.uuid == r.uuid || self.ignores(s, r) {
            return None;
        }
        // Waypoint.Icon.cloneAndAssignStyle: the icon's colour, else the team's.
        let mut icon = s.icon.clone();
        if icon.color.is_none() {
            icon.color = s.team.as_ref().and_then(|t| t.1);
        }
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
    fn update(out: &mut Vec<(u32, C)>, c: &mut Connection, (s, r): (&Player, &Player)) {
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
            out.push((
                conn,
                waypoint(s.uuid, waypoint_operation::UPDATE, &c.icon, target),
            ));
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
                to.row.put(source, c);
            }
            None => {
                if to.row.take(source).is_some() {
                    self.disconnect(to.out, receiver, source);
                }
            }
        }
    }
    /// The receiver's connection to the source updated in its place
    /// (`updateConnection`), or made if it has none (`createConnection`).
    fn update_or_create(&self, to: &mut Receiving, source: u128) {
        let receiver = to.receiver;
        if receiver == source || !self.enabled {
            return;
        }
        let pair = self.pair(source, receiver);
        let Some(c) = to.row.get_mut(source) else {
            return self.create_connection(to, source);
        };
        // A connection to a player who isn't in the level is broken.
        if let Some(pair) = pair
            && !self.broken(to.view, c, pair)
        {
            Self::update(to.out, c, pair);
            return;
        }
        match pair.and_then(|pair| self.make(to.view, pair)) {
            Some(new) => {
                self.connect(to.out, receiver, source, &new);
                *c = new;
            }
            None => {
                self.disconnect(to.out, receiver, source);
                to.row.take(source);
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
    out: Vec<(u32, C)>,
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
    /// It ticked since joining: its waypoint and its own connections are updated.
    ticked: bool,
    /// Its waypoint is tracked: every receiver's connection to it is updated.
    transmits: bool,
    receives: bool,
    /// Its view's new centre, when it changed chunk.
    new_view: View,
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
        self.connections.get_mut(&receiver)?.take(source)
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
        for receiver in self.players.clone() {
            if self.take(receiver, source).is_some() {
                t.disconnect(receiver, source);
            }
        }
        self.waypoints.retain(|w| *w != source);
    }
    /// `addPlayer`.
    fn add_player(&mut self, t: &mut Tick, player: u128, transmitting: bool) {
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
        for source in self
            .connections
            .remove(&player)
            .unwrap_or_default()
            .sources()
        {
            t.disconnect(player, source);
        }
        self.untrack(t, player);
        self.players.retain(|p| *p != player);
    }
    /// `breakAllConnections`.
    fn break_all(&mut self, t: &mut Tick) {
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
    ) -> Vec<(u32, C)> {
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
    ) -> Vec<(u32, C)> {
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
    ) -> Vec<(u32, C)> {
        let mut t = Tick {
            s: Shared {
                players: players.iter().map(|p| (p.uuid, p)).collect(),
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
        // Players who left, and the ServerPlayer a respawn replaced.
        let gone: Vec<u128> = self
            .players
            .iter()
            .copied()
            .filter(|uuid| !t.s.players.contains_key(uuid) || respawned.contains(uuid))
            .collect();
        for uuid in gone {
            self.remove_player(&mut t, uuid);
            self.memo.remove(&uuid);
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
                    team: p.team.clone(),
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
                (p.transmit_range > 0.0) == memo.transmitting && p.team == memo.team
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
            if transmitting != self.memo[&p.uuid].transmitting {
                self.memo.get_mut(&p.uuid).unwrap().transmitting = transmitting;
                if transmitting {
                    self.track(&mut t, p.uuid);
                } else {
                    self.untrack(&mut t, p.uuid);
                }
            }
            // ServerScoreboard: a team joined, left or recoloured remakes the connections.
            if p.team != self.memo[&p.uuid].team {
                self.memo.get_mut(&p.uuid).unwrap().team = p.team.clone();
                for receiver in self.players.clone() {
                    self.create_connection(&mut t, receiver, p.uuid);
                }
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
        let mut moves = Vec::new();
        for player in players {
            let memo = self
                .memo
                .get_mut(&player.uuid)
                .expect("every player has a memo");
            if memo.position == player.position {
                continue;
            }
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
            out: Vec<(u32, C)>,
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
                out: Vec::new(),
            })
            .collect();
        let (s, waypoints, moves) = (&t.s, &self.waypoints, &moves);
        horizon_threads::par_chunks_mut(&mut jobs, 8, |jobs| {
            for job in jobs {
                let mut to = Receiving {
                    receiver: job.receiver,
                    view: job.view,
                    row: &mut job.row,
                    out: &mut job.out,
                };
                for m in moves {
                    if m.uuid != job.receiver {
                        // `updateWaypoint`: this receiver's connection to the mover.
                        if job.listed && m.ticked && m.transmits {
                            s.update_or_create(&mut to, m.uuid);
                        }
                        continue;
                    }
                    // `updatePlayer` before and after the view follows the move.
                    if m.ticked && m.receives {
                        for &source in waypoints {
                            s.update_or_create(&mut to, source);
                        }
                    }
                    if m.new_view.is_some() {
                        to.view = m.new_view;
                    }
                    if m.receives {
                        for &source in waypoints {
                            s.update_or_create(&mut to, source);
                        }
                    }
                }
            }
        });
        for job in jobs {
            t.out.extend(job.out);
            self.connections.insert(job.receiver, job.row);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(uuid: u128, conn: u32, position: [f64; 3]) -> Player {
        Player {
            uuid,
            conn: Some(conn),
            position,
            spectator: false,
            transmit_range: PLAYER_RANGE,
            receive_range: PLAYER_RANGE,
            first_tick: false,
            view_distance: 4,
            team: None,
            icon: Icon::default(),
        }
    }

    /// The packets to `conn`, as (operation, target).
    fn to(out: &[(u32, C)], conn: u32) -> Vec<(i32, WaypointTarget)> {
        out.iter()
            .filter(|(c, _)| *c == conn)
            .map(|(_, p)| match p {
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

    /// Each connection's packets as bytes, in the order it is sent them.
    fn by_conn(out: &[(u32, C)]) -> std::collections::BTreeMap<u32, Vec<Vec<u8>>> {
        let mut by = std::collections::BTreeMap::<u32, Vec<Vec<u8>>>::new();
        for (conn, packet) in out {
            by.entry(*conn).or_default().push(packet.encode());
        }
        by
    }

    /// The moves worked out by receiver give every connection the bytes the one loop gives,
    /// in its order: crowds walking, crossing chunks and going far, with joins, leaves,
    /// respawns, hiding, team changes, spectators and the rule switched (which take the loop).
    #[test]
    fn moves_by_receiver_match_the_loop() {
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = move |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        let mut sent = 0;
        for level in 0..40 {
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
                    if next(4) != 0 {
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
                            3 => p.spectator = !p.spectator,
                            4 => respawned.push(p.uuid),
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
                assert_eq!(by_conn(&a), by_conn(&b), "level {level}, tick {tick}");
            }
        }
        assert!(
            sent > 10_000,
            "the crowds were sent their waypoints ({sent})"
        );
    }

    #[test]
    fn team_colours() {
        assert_eq!(team_rgb("red"), Some(0xFF5555));
        assert_eq!(team_rgb("black"), Some(0xFF30_3030_u32 as i32));
        assert_eq!(team_rgb("reset"), None);
    }
}
