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

/// ServerWaypointManager for one level.
#[derive(Default)]
pub struct Waypoints {
    /// The transmitters, in the order they were tracked.
    waypoints: Vec<u128>,
    /// The receivers, in the order they joined.
    players: Vec<u128>,
    /// Connections by receiver, then source.
    connections: HashMap<u128, Vec<(u128, Connection)>>,
    memo: HashMap<u128, Memo>,
    /// The `locator_bar` game rule as last seen.
    enabled: Option<bool>,
}

/// One tick's players, and the packets for their connections.
struct Tick<'a> {
    players: HashMap<u128, &'a Player>,
    /// The connections of the players in other levels: one who just left this level is
    /// still told of the waypoints it loses (`removePlayer` disconnects them).
    away: &'a [(u128, u32)],
    view: HashMap<u128, ([i32; 2], i32)>,
    /// Players a respawn replaced this tick: a new ServerPlayer, on its first tick.
    fresh: &'a [u128],
    enabled: bool,
    out: Vec<(u32, C)>,
}

impl<'a> Tick<'a> {
    fn send(
        &mut self,
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
        self.out
            .push((conn, waypoint(source, operation, icon, target)));
    }
    fn connect(&mut self, receiver: u128, source: u128, c: &Connection) {
        let target = match c.kind {
            Kind::Block(p) => WaypointTarget::Block(p),
            Kind::Chunk(p) => WaypointTarget::Chunk(p),
            Kind::Azimuth(a) => WaypointTarget::Azimuth(a),
        };
        self.send(receiver, source, waypoint_operation::TRACK, &c.icon, target);
    }
    /// `ClientboundTrackedWaypointPacket.removeWaypoint`: an empty waypoint with the null icon.
    fn disconnect(&mut self, receiver: u128, source: u128) {
        self.send(
            receiver,
            source,
            waypoint_operation::UNTRACK,
            &Icon::default(),
            WaypointTarget::Empty,
        );
    }
    /// `ChunkTrackingView.isInViewDistance` of the receiver's view.
    fn chunk_visible(&self, [x, z]: [i32; 2], receiver: u128) -> bool {
        let Some(&([cx, cz], r)) = self.view.get(&receiver) else {
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
    fn pair(&self, source: u128, receiver: u128) -> Option<(&'a Player, &'a Player)> {
        Some((*self.players.get(&source)?, *self.players.get(&receiver)?))
    }
    /// `LivingEntity.makeWaypointConnectionWith`.
    fn make(&self, (s, r): (&Player, &Player)) -> Option<Connection> {
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
        } else if !self.chunk_visible(s.chunk(), r.uuid) {
            Kind::Chunk(s.chunk())
        } else {
            Kind::Block(s.block())
        };
        Some(Connection { kind, icon })
    }
    /// `Connection.isBroken` for each kind.
    fn broken(&self, c: &Connection, (s, r): (&Player, &Player)) -> bool {
        let receiver = r.uuid;
        match c.kind {
            Kind::Block(last) => {
                let now = s.block();
                (0..3).map(|k| (now[k] - last[k]).abs()).sum::<i32>() > 1 || self.ignores(s, r)
            }
            Kind::Chunk(last) => {
                let now = s.chunk();
                let moved = (now[0] - last[0]).abs().max((now[1] - last[1]).abs());
                moved > 1 || self.ignores(s, r) || self.chunk_visible(last, receiver)
            }
            Kind::Azimuth(_) => {
                self.ignores(s, r)
                    || self.chunk_visible(s.chunk(), receiver)
                    || !Self::really_far(s, r)
            }
        }
    }
    /// `Connection.update`: the packet when the waypoint moved.
    fn update(&mut self, c: &mut Connection, (s, r): (&Player, &Player)) {
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
            self.out.push((
                conn,
                waypoint(s.uuid, waypoint_operation::UPDATE, &c.icon, target),
            ));
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

impl Waypoints {
    fn put(&mut self, receiver: u128, source: u128, c: Connection) {
        let row = self.connections.entry(receiver).or_default();
        match row.iter_mut().find(|(s, _)| *s == source) {
            Some(slot) => slot.1 = c,
            None => row.push((source, c)),
        }
    }
    fn take(&mut self, receiver: u128, source: u128) -> Option<Connection> {
        let row = self.connections.get_mut(&receiver)?;
        let i = row.iter().position(|(s, _)| *s == source)?;
        Some(row.remove(i).1)
    }

    /// `createConnection`.
    fn create_connection(&mut self, t: &mut Tick, receiver: u128, source: u128) {
        if receiver == source || !t.enabled {
            return;
        }
        match t.pair(source, receiver).and_then(|pair| t.make(pair)) {
            Some(c) => {
                t.connect(receiver, source, &c);
                self.put(receiver, source, c);
            }
            None => {
                if self.take(receiver, source).is_some() {
                    t.disconnect(receiver, source);
                }
            }
        }
    }
    /// The receiver's connection to the source updated in its place
    /// (`updateConnection`), or made if it has none (`createConnection`).
    fn update_or_create(&mut self, t: &mut Tick, receiver: u128, source: u128) {
        if receiver == source || !t.enabled {
            return;
        }
        let Some(row) = self.connections.get_mut(&receiver) else {
            return self.create_connection(t, receiver, source);
        };
        let Some(i) = row.iter().position(|(s, _)| *s == source) else {
            return self.create_connection(t, receiver, source);
        };
        let c = &mut row[i].1;
        // A connection to a player who isn't in the level is broken.
        let pair = t.pair(source, receiver);
        if let Some(pair) = pair
            && !t.broken(c, pair)
        {
            t.update(c, pair);
            return;
        }
        match pair.and_then(|pair| t.make(pair)) {
            Some(new) => {
                t.connect(receiver, source, &new);
                *c = new;
            }
            None => {
                t.disconnect(receiver, source);
                row.remove(i);
            }
        }
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
        for (source, _) in self.connections.remove(&player).unwrap_or_default() {
            t.disconnect(player, source);
        }
        self.untrack(t, player);
        self.players.retain(|p| *p != player);
    }
    /// `breakAllConnections`.
    fn break_all(&mut self, t: &mut Tick) {
        for receiver in self.players.clone() {
            for (source, _) in self.connections.remove(&receiver).unwrap_or_default() {
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
    pub fn tick_level(
        &mut self,
        players: &[Player],
        away: &[(u128, u32)],
        respawned: &[u128],
        locator_bar: bool,
    ) -> Vec<(u32, C)> {
        let mut t = Tick {
            players: players.iter().map(|p| (p.uuid, p)).collect(),
            away,
            view: HashMap::default(),
            fresh: respawned,
            enabled: locator_bar,
            out: Vec::new(),
        };
        // Vanilla reads the rule when it connects; changing it breaks or remakes
        // every connection (MinecraftServer.onGameRuleChanged).
        let was = self.enabled.replace(locator_bar);
        for (uuid, memo) in &self.memo {
            if let Some(p) = t.players.get(uuid) {
                t.view.insert(*uuid, (memo.view_center, p.view_distance));
            }
        }
        // Players who left, and the ServerPlayer a respawn replaced.
        let gone: Vec<u128> = self
            .players
            .iter()
            .copied()
            .filter(|uuid| !t.players.contains_key(uuid) || respawned.contains(uuid))
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

    #[test]
    fn team_colours() {
        assert_eq!(team_rgb("red"), Some(0xFF5555));
        assert_eq!(team_rgb("black"), Some(0xFF30_3030_u32 as i32));
        assert_eq!(team_rgb("reset"), None);
    }
}
