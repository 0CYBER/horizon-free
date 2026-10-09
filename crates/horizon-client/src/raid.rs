//! Raids (`Raid` and `Raids`): the raid omen that starts one, the waves of
//! raiders each difficulty sends, the boss bar, victory and loss, and
//! `data/minecraft/raids.dat`. The world is reached through [`RaidLevel`] and
//! the results come back as [`Action`]s, so the rules can be tested alone.
use horizon_core::random::{LegacyRandom, RandomSource};
use horizon_world::nbt::Tag;
use std::collections::BTreeMap;

pub const MAX_OMEN_LEVEL: i32 = 5;
/// `Raid.RAID_TIMEOUT_TICKS`.
const TIMEOUT: i64 = 48_000;
const PRE_RAID_TICKS: i32 = 300;
const POST_RAID_LIMIT: i32 = 40;
const CELEBRATION_TICKS: i32 = 600;
/// `Raids.getNearbyRaid(pos, 9216)`: within 96 blocks.
pub const VALID_RAID_RADIUS_SQR: f64 = 9216.0;
/// `RAID_REMOVAL_THRESHOLD_SQR`: a raider this far from the centre leaves.
const REMOVAL_THRESHOLD_SQR: f64 = 12544.0;
/// `HERO_OF_THE_VILLAGE_DURATION`.
pub const HERO_DURATION: i32 = 48_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ongoing,
    Victory,
    Loss,
    Stopped,
}
impl Status {
    pub fn name(self) -> &'static str {
        match self {
            Self::Ongoing => "ongoing",
            Self::Victory => "victory",
            Self::Loss => "loss",
            Self::Stopped => "stopped",
        }
    }
    fn from_name(s: &str) -> Self {
        match s {
            "victory" => Self::Victory,
            "loss" => Self::Loss,
            "stopped" => Self::Stopped,
            _ => Self::Ongoing,
        }
    }
}

/// `Raid.RaiderType`, in spawn order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RaiderType {
    Vindicator,
    Evoker,
    Pillager,
    Witch,
    Ravager,
}
impl RaiderType {
    pub const ALL: [Self; 5] = [
        Self::Vindicator,
        Self::Evoker,
        Self::Pillager,
        Self::Witch,
        Self::Ravager,
    ];
    pub fn entity(self) -> &'static str {
        match self {
            Self::Vindicator => "minecraft:vindicator",
            Self::Evoker => "minecraft:evoker",
            Self::Pillager => "minecraft:pillager",
            Self::Witch => "minecraft:witch",
            Self::Ravager => "minecraft:ravager",
        }
    }
    /// The health `addWaveMob` counts toward the bar.
    pub fn health(self) -> f32 {
        match self {
            Self::Witch => 26.0,
            Self::Ravager => 100.0,
            _ => 24.0,
        }
    }
    pub fn can_be_leader(self) -> bool {
        !matches!(self, Self::Witch | Self::Ravager)
    }
    /// `spawnsPerWaveBeforeBonus`, by wave number.
    fn spawns(self) -> [i32; 8] {
        match self {
            Self::Vindicator => [0, 0, 2, 0, 1, 4, 2, 5],
            Self::Evoker => [0, 0, 0, 0, 0, 1, 1, 2],
            Self::Pillager => [0, 4, 3, 3, 4, 4, 4, 2],
            Self::Witch => [0, 0, 0, 0, 3, 0, 0, 1],
            Self::Ravager => [0, 0, 0, 1, 0, 1, 0, 2],
        }
    }
}
/// `Raid.getNumGroups`: waves per difficulty (0 peaceful to 3 hard).
pub fn num_groups(difficulty: i32) -> i32 {
    match difficulty {
        1 => 3,
        2 => 5,
        3 => 7,
        _ => 0,
    }
}

/// One raider of a wave, with the raider riding it (ravagers only).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spawn {
    pub kind: RaiderType,
    pub leader: bool,
    pub rider: Option<RaiderType>,
}

/// A raider that belongs to a raid, as the level reports it.
#[derive(Clone, Debug)]
pub struct RaiderInfo {
    pub id: u32,
    pub wave: i32,
    pub health: f32,
    pub position: [i32; 3],
    /// `tickCount`.
    pub age: u32,
    pub no_action_time: i32,
    pub ticks_outside: i32,
    pub in_village: bool,
}

/// What a raid asks of the world.
pub trait RaidLevel {
    /// The world difficulty, 0 (peaceful) to 3 (hard).
    fn difficulty(&self) -> i32;
    fn has_chunk_at(&self, pos: [i32; 3]) -> bool;
    /// `ServerLevel.isVillage`: within one section of a village centre.
    fn is_village(&self, pos: [i32; 3]) -> bool;
    /// Centres of the village sections within two sections of `pos`.
    fn village_sections_near(&self, pos: [i32; 3]) -> Vec<[i32; 3]>;
    /// The living raiders of a raid.
    fn raiders(&self, raid: i32) -> Vec<RaiderInfo>;
    /// `Heightmap.WORLD_SURFACE` height at a column.
    fn surface_height(&self, x: i32, z: i32) -> i32;
    /// The chunks around the position are loaded and ticking.
    fn is_ticking(&self, pos: [i32; 3]) -> bool;
    /// A ravager may spawn there (or on snow), in loaded chunks.
    fn can_spawn_at(&self, pos: [i32; 3]) -> bool;
}

/// What the game must do after a raid tick.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Spawn the raiders of a wave around `pos`.
    Spawn {
        pos: [i32; 3],
        wave: i32,
        spawns: Vec<Spawn>,
    },
    /// The raid horn, heard by players near or in the raid.
    Horn {
        pos: [i32; 3],
    },
    /// A raider leaves the raid (far away, despawned or idle outside).
    Remove {
        id: u32,
    },
    SetOutside {
        id: u32,
        ticks: i32,
    },
    /// The raid was won: heroes get the effect at `level` (amplifier).
    Victory {
        heroes: Vec<[i32; 4]>,
        amplifier: i32,
    },
}

/// The raid boss bar's title.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarName {
    Raid,
    Remaining(i32),
    Victory,
    Defeat,
}
#[derive(Clone, Debug)]
pub struct Bar {
    pub name: BarName,
    pub progress: f32,
    pub visible: bool,
    /// The local player is in the raid (`ServerBossEvent.getPlayers`).
    pub player: bool,
}

#[derive(Clone, Debug)]
pub struct Raid {
    pub started: bool,
    pub active: bool,
    pub ticks_active: i64,
    pub omen_level: i32,
    pub groups_spawned: i32,
    pub cooldown_ticks: i32,
    pub post_raid_ticks: i32,
    pub total_health: f32,
    pub num_groups: i32,
    pub status: Status,
    pub center: [i32; 3],
    pub heroes: Vec<[i32; 4]>,
    pub bar: Bar,
    celebration_ticks: i32,
    wave_spawn_pos: Option<[i32; 3]>,
    random: LegacyRandom,
}
impl Raid {
    /// `new Raid(center, difficulty)`.
    pub fn new(center: [i32; 3], difficulty: i32, seed: i64) -> Self {
        Self {
            started: false,
            active: true,
            ticks_active: 0,
            omen_level: 0,
            groups_spawned: 0,
            cooldown_ticks: PRE_RAID_TICKS,
            post_raid_ticks: 0,
            total_health: 0.0,
            num_groups: num_groups(difficulty),
            status: Status::Ongoing,
            center,
            heroes: Vec::new(),
            bar: Bar {
                name: BarName::Raid,
                progress: 0.0,
                visible: true,
                player: false,
            },
            celebration_ticks: 0,
            wave_spawn_pos: None,
            random: LegacyRandom::new(seed),
        }
    }
    pub fn is_over(&self) -> bool {
        self.is_victory() || self.is_loss()
    }
    pub fn is_victory(&self) -> bool {
        self.status == Status::Victory
    }
    pub fn is_loss(&self) -> bool {
        self.status == Status::Loss
    }
    pub fn is_stopped(&self) -> bool {
        self.status == Status::Stopped
    }
    pub fn has_first_wave_spawned(&self) -> bool {
        self.groups_spawned > 0
    }
    /// `isBetweenWaves`: the next wave is counting down.
    pub fn is_between_waves(&self, raiders_alive: usize) -> bool {
        self.has_first_wave_spawned() && raiders_alive == 0 && self.cooldown_ticks > 0
    }
    pub fn stop(&mut self) {
        self.active = false;
        self.bar.player = false;
        self.status = Status::Stopped;
    }
    fn has_bonus_wave(&self) -> bool {
        self.omen_level > 1
    }
    fn is_final_wave(&self) -> bool {
        self.groups_spawned == self.num_groups
    }
    fn has_spawned_bonus_wave(&self) -> bool {
        self.groups_spawned > self.num_groups
    }
    fn has_more_waves(&self) -> bool {
        if self.has_bonus_wave() {
            !self.has_spawned_bonus_wave()
        } else {
            !self.is_final_wave()
        }
    }
    fn should_spawn_bonus_group(&self, alive: usize) -> bool {
        self.is_final_wave() && alive == 0 && self.has_bonus_wave()
    }
    fn should_spawn_group(&self, alive: usize) -> bool {
        self.cooldown_ticks == 0
            && (self.groups_spawned < self.num_groups || self.should_spawn_bonus_group(alive))
            && alive == 0
    }
    /// `Raid.absorbRaidOmen`: the omen level rises by the effect's level.
    /// True when this is the omen that first triggers the raid.
    pub fn absorb_omen(&mut self, amplifier: i32) -> bool {
        self.omen_level = (self.omen_level + amplifier + 1).clamp(0, MAX_OMEN_LEVEL);
        !self.has_first_wave_spawned()
    }
    /// `Raid.getEnchantOdds`.
    pub fn enchant_odds(&self) -> f32 {
        match self.omen_level {
            2 => 0.1,
            3 => 0.25,
            4 => 0.5,
            5 => 0.75,
            _ => 0.0,
        }
    }
    pub fn add_hero(&mut self, uuid: [i32; 4]) {
        if !self.heroes.contains(&uuid) {
            self.heroes.push(uuid);
        }
    }
    /// `Raid.getDefaultNumSpawns` and `getPotentialBonusSpawns`.
    fn wave_count(&mut self, kind: RaiderType, wave: i32, difficulty: i32, bonus: bool) -> i32 {
        let index = if bonus { self.num_groups } else { wave } as usize;
        let base = kind.spawns()[index.min(7)];
        let easy = difficulty == 1;
        let normal = difficulty == 2;
        let extra = match kind {
            RaiderType::Vindicator | RaiderType::Pillager => {
                if easy {
                    self.random.next_int_bound(2).unwrap_or(0)
                } else if normal {
                    1
                } else {
                    2
                }
            }
            RaiderType::Evoker => return base,
            RaiderType::Witch => {
                if easy || wave <= 2 || wave == 4 {
                    return base;
                }
                1
            }
            RaiderType::Ravager => i32::from(!easy && bonus),
        };
        base + if extra > 0 {
            self.random.next_int_bound(extra + 1).unwrap_or(0)
        } else {
            0
        }
    }
    /// The raiders of wave `wave` (`spawnGroup`): per type the counted
    /// spawns, the first able raider leading, and ravagers carrying a
    /// pillager (the last normal wave) or an evoker and vindicators (hard).
    pub fn compose_wave(&mut self, wave: i32, difficulty: i32, bonus: bool) -> Vec<Spawn> {
        let mut out = Vec::new();
        let mut leader_set = false;
        for kind in RaiderType::ALL {
            let count = self.wave_count(kind, wave, difficulty, bonus);
            let mut ravagers = 0;
            for _ in 0..count {
                let leader = !leader_set && kind.can_be_leader();
                leader_set |= leader;
                let mut rider = None;
                if kind == RaiderType::Ravager {
                    if wave == num_groups(2) {
                        rider = Some(RaiderType::Pillager);
                    } else if wave >= num_groups(3) {
                        rider = Some(if ravagers == 0 {
                            RaiderType::Evoker
                        } else {
                            RaiderType::Vindicator
                        });
                    }
                    ravagers += 1;
                }
                out.push(Spawn {
                    kind,
                    leader,
                    rider,
                });
            }
        }
        out
    }
    /// The health `joinRaid` adds for a wave.
    pub fn wave_health(spawns: &[Spawn]) -> f32 {
        spawns
            .iter()
            .map(|s| s.kind.health() + s.rider.map_or(0.0, RaiderType::health))
            .sum()
    }
    /// `Raid.findRandomSpawnPos`: a ring around the centre that closes in as
    /// the countdown runs down.
    fn find_spawn_pos(&mut self, level: &dyn RaidLevel, max_tries: i32) -> Option<[i32; 3]> {
        let seconds = self.cooldown_ticks / 20;
        let how_far = 0.22 * seconds as f32 - 0.24;
        let start = self.random.next_float() * (std::f32::consts::PI * 2.0);
        for i in 0..max_tries {
            let angle = start + std::f32::consts::PI * i as f32 / 8.0;
            let x = self.center[0]
                + (angle.cos() * 32.0 * how_far).floor() as i32
                + self.random.next_int_bound(3).unwrap_or(0) * how_far.floor() as i32;
            let z = self.center[2]
                + (angle.sin() * 32.0 * how_far).floor() as i32
                + self.random.next_int_bound(3).unwrap_or(0) * how_far.floor() as i32;
            let y = level.surface_height(x, z);
            if (y - self.center[1]).abs() > 96 {
                continue;
            }
            let pos = [x, y, z];
            if (!level.is_village(pos) || seconds <= 7) && level.can_spawn_at(pos) {
                return Some(pos);
            }
        }
        None
    }
    fn move_center_to_nearby_village(&mut self, level: &dyn RaidLevel) {
        let c = self.center;
        let d = |p: &[i32; 3]| (0..3).map(|i| f64::from(p[i] - c[i]).powi(2)).sum::<f64>();
        if let Some(best) = level
            .village_sections_near(c)
            .into_iter()
            .min_by(|a, b| d(a).total_cmp(&d(b)))
        {
            self.center = best;
        }
    }
    /// Boss bar progress while raiders are alive (`updateBossbar`).
    pub fn update_bar(&mut self, living_health: f32) {
        self.bar.progress = if self.total_health > 0.0 {
            (living_health / self.total_health).clamp(0.0, 1.0)
        } else {
            0.0
        };
    }
    /// `Raid.tick`. `raiders` are the living members; `player` is whether
    /// the local player's position falls in this raid.
    pub fn tick(&mut self, id: i32, level: &dyn RaidLevel, player: bool) -> Vec<Action> {
        let mut actions = Vec::new();
        if self.is_stopped() {
            return actions;
        }
        let raiders = level.raiders(id);
        let alive = raiders.len();
        if self.status == Status::Ongoing {
            let was_active = self.active;
            self.active = level.has_chunk_at(self.center);
            if level.difficulty() == 0 {
                self.stop();
                return actions;
            }
            if was_active != self.active {
                self.bar.visible = self.active;
            }
            if !self.active {
                return actions;
            }
            if !level.is_village(self.center) {
                self.move_center_to_nearby_village(level);
            }
            if !level.is_village(self.center) {
                if self.groups_spawned > 0 {
                    self.status = Status::Loss;
                } else {
                    self.stop();
                }
            }
            self.ticks_active += 1;
            if self.ticks_active >= TIMEOUT {
                self.stop();
                return actions;
            }
            // Raid.updateBossbar, as raiders are hurt, die or leave.
            if alive > 0 {
                self.update_bar(raiders.iter().map(|r| r.health).sum());
            } else if self.started && !self.has_more_waves() {
                self.bar.progress = 0.0;
            }
            if alive == 0 && self.has_more_waves() {
                if self.cooldown_ticks <= 0 {
                    if self.cooldown_ticks == 0 && self.groups_spawned > 0 {
                        self.cooldown_ticks = PRE_RAID_TICKS;
                        self.bar.name = BarName::Raid;
                        return actions;
                    }
                } else {
                    let cached = self.wave_spawn_pos.is_some();
                    let mut try_find = !cached && self.cooldown_ticks % 5 == 0;
                    if cached && !self.wave_spawn_pos.is_some_and(|p| level.is_ticking(p)) {
                        try_find = true;
                    }
                    if try_find {
                        self.wave_spawn_pos = self.find_spawn_pos(level, 8);
                    }
                    if self.cooldown_ticks == PRE_RAID_TICKS || self.cooldown_ticks % 20 == 0 {
                        self.bar.player = player;
                    }
                    self.cooldown_ticks -= 1;
                    self.bar.progress =
                        ((PRE_RAID_TICKS - self.cooldown_ticks) as f32 / 300.0).clamp(0.0, 1.0);
                }
            }
            if self.ticks_active % 20 == 0 {
                self.bar.player = player;
                self.update_raiders(&raiders, level, &mut actions);
                self.bar.name = if alive > 0 && alive <= 2 {
                    BarName::Remaining(alive as i32)
                } else {
                    BarName::Raid
                };
            }
            let mut sound = false;
            let mut attempt = 0;
            let mut spawned_alive = alive;
            while self.should_spawn_group(spawned_alive) {
                let pos = self
                    .wave_spawn_pos
                    .or_else(|| self.find_spawn_pos(level, 20));
                if let Some(pos) = pos {
                    self.started = true;
                    let wave = self.groups_spawned + 1;
                    let bonus = self.should_spawn_bonus_group(spawned_alive);
                    let difficulty = level.difficulty();
                    let spawns = self.compose_wave(wave, difficulty, bonus);
                    self.total_health = Self::wave_health(&spawns);
                    spawned_alive = spawns
                        .iter()
                        .map(|s| 1 + usize::from(s.rider.is_some()))
                        .sum();
                    self.wave_spawn_pos = None;
                    self.groups_spawned += 1;
                    self.bar.progress = 1.0;
                    actions.push(Action::Spawn { pos, wave, spawns });
                    if !sound {
                        actions.push(Action::Horn { pos });
                        sound = true;
                    }
                } else {
                    attempt += 1;
                }
                if attempt > 5 {
                    self.stop();
                    break;
                }
            }
            if self.started && !self.has_more_waves() && alive == 0 {
                if self.post_raid_ticks < POST_RAID_LIMIT {
                    self.post_raid_ticks += 1;
                } else {
                    self.status = Status::Victory;
                    actions.push(Action::Victory {
                        heroes: self.heroes.clone(),
                        amplifier: self.omen_level - 1,
                    });
                }
            }
        } else if self.is_over() {
            self.celebration_ticks += 1;
            if self.celebration_ticks >= CELEBRATION_TICKS {
                self.stop();
                return actions;
            }
            if self.celebration_ticks % 20 == 0 {
                self.bar.player = player;
                self.bar.visible = true;
                if self.is_victory() {
                    self.bar.progress = 0.0;
                    self.bar.name = BarName::Victory;
                } else {
                    self.bar.name = BarName::Defeat;
                }
            }
        }
        actions
    }
    /// `Raid.updateRaiders`.
    fn update_raiders(
        &mut self,
        raiders: &[RaiderInfo],
        level: &dyn RaidLevel,
        actions: &mut Vec<Action>,
    ) {
        let _ = level;
        for r in raiders {
            let d = (0..3)
                .map(|i| f64::from(self.center[i] - r.position[i]).powi(2))
                .sum::<f64>();
            let mut remove = d >= REMOVAL_THRESHOLD_SQR;
            if !remove && r.age > 600 {
                let mut outside = r.ticks_outside;
                if !r.in_village && r.no_action_time > 2400 {
                    outside += 1;
                    actions.push(Action::SetOutside {
                        id: r.id,
                        ticks: outside,
                    });
                }
                remove = outside >= 30;
            }
            if remove {
                self.total_health -= r.health;
                actions.push(Action::Remove { id: r.id });
            }
        }
    }

    pub fn to_nbt(&self) -> Tag {
        Tag::compound([
            ("started", Tag::Byte(i8::from(self.started))),
            ("active", Tag::Byte(i8::from(self.active))),
            ("ticks_active", Tag::Long(self.ticks_active)),
            ("raid_omen_level", Tag::Int(self.omen_level)),
            ("groups_spawned", Tag::Int(self.groups_spawned)),
            ("cooldown_ticks", Tag::Int(self.cooldown_ticks)),
            ("post_raid_ticks", Tag::Int(self.post_raid_ticks)),
            ("total_health", Tag::Float(self.total_health.to_bits())),
            ("group_count", Tag::Int(self.num_groups)),
            ("status", Tag::String(self.status.name().into())),
            ("center", Tag::IntArray(self.center.to_vec())),
            (
                "heroes_of_the_village",
                Tag::List(
                    self.heroes
                        .iter()
                        .map(|u| Tag::IntArray(u.to_vec()))
                        .collect(),
                ),
            ),
        ])
    }
    pub fn from_nbt(tag: &Tag, seed: i64) -> Option<Self> {
        let int = |k: &str| match tag.get(k) {
            Some(Tag::Int(v)) => Some(*v),
            Some(Tag::Byte(v)) => Some(i32::from(*v)),
            Some(Tag::Short(v)) => Some(i32::from(*v)),
            _ => None,
        };
        let center = match tag.get("center") {
            Some(Tag::IntArray(v)) if v.len() == 3 => [v[0], v[1], v[2]],
            _ => return None,
        };
        let mut raid = Self::new(center, 0, seed);
        raid.started = int("started")? != 0;
        raid.active = int("active")? != 0;
        raid.ticks_active = match tag.get("ticks_active") {
            Some(Tag::Long(v)) => *v,
            _ => 0,
        };
        raid.omen_level = int("raid_omen_level").unwrap_or(0);
        raid.groups_spawned = int("groups_spawned").unwrap_or(0);
        raid.cooldown_ticks = int("cooldown_ticks").unwrap_or(PRE_RAID_TICKS);
        raid.post_raid_ticks = int("post_raid_ticks").unwrap_or(0);
        raid.total_health = match tag.get("total_health") {
            Some(Tag::Float(bits)) => f32::from_bits(*bits),
            _ => 0.0,
        };
        raid.num_groups = int("group_count").unwrap_or(0);
        raid.status = match tag.get("status") {
            Some(Tag::String(s)) => Status::from_name(&s.to_string_lossy()),
            _ => Status::Ongoing,
        };
        if let Some(Tag::List(heroes)) = tag.get("heroes_of_the_village") {
            for h in heroes {
                if let Tag::IntArray(v) = h
                    && v.len() == 4
                {
                    raid.heroes.push([v[0], v[1], v[2], v[3]]);
                }
            }
        }
        raid.bar.visible = raid.active;
        Some(raid)
    }
}

/// `Raids`: the raids of a dimension by id.
#[derive(Clone, Debug)]
pub struct Raids {
    pub map: BTreeMap<i32, Raid>,
    next_id: i32,
    tick: i32,
    /// Changed since the last save.
    pub dirty: bool,
}
impl Default for Raids {
    fn default() -> Self {
        Self {
            map: BTreeMap::new(),
            next_id: 1,
            tick: 0,
            dirty: false,
        }
    }
}
impl Raids {
    pub fn get(&self, id: i32) -> Option<&Raid> {
        self.map.get(&id)
    }
    pub fn id_of(&self, raid: &Raid) -> Option<i32> {
        self.map
            .iter()
            .find(|(_, r)| std::ptr::eq(*r, raid))
            .map(|(id, _)| *id)
    }
    /// `Raids.getNearbyRaid`: the closest active raid within 96 blocks.
    pub fn nearby(&self, pos: [i32; 3]) -> Option<(i32, &Raid)> {
        let mut best: Option<(i32, &Raid, f64)> = None;
        for (id, r) in &self.map {
            let d = (0..3)
                .map(|i| f64::from(r.center[i] - pos[i]).powi(2))
                .sum::<f64>();
            if r.active && d < best.map_or(VALID_RAID_RADIUS_SQR, |b| b.2) {
                best = Some((*id, r, d));
            }
        }
        best.map(|(id, r, _)| (id, r))
    }
    /// `Raids.createOrExtendRaid`, after the caller found the village centre:
    /// the raid at that position (a new one if none), with the omen's level
    /// absorbed. Returns the raid id and whether the omen triggered it.
    pub fn create_or_extend(
        &mut self,
        center: [i32; 3],
        difficulty: i32,
        omen_amplifier: Option<i32>,
        seed: i64,
    ) -> (i32, bool) {
        let id = match self.nearby(center) {
            Some((id, _)) => id,
            None => {
                self.next_id += 1;
                self.map
                    .insert(self.next_id, Raid::new(center, difficulty, seed));
                self.next_id
            }
        };
        let mut triggered = false;
        if let Some(raid) = self.map.get_mut(&id)
            && (!raid.started || raid.omen_level < MAX_OMEN_LEVEL)
            && let Some(amp) = omen_amplifier
        {
            triggered = raid.absorb_omen(amp);
        }
        self.dirty = true;
        (id, triggered)
    }
    /// `Raids.tick`: every raid, stopped ones removed. `rules_raids` is the
    /// `raids` game rule; `player` the local player's block position.
    pub fn tick(
        &mut self,
        level: &dyn RaidLevel,
        rules_raids: bool,
        player: Option<[i32; 3]>,
    ) -> Vec<(i32, Action)> {
        self.tick += 1;
        let inside = player.and_then(|p| self.nearby(p).map(|(id, _)| id));
        let mut out = Vec::new();
        let mut gone = Vec::new();
        for (id, raid) in &mut self.map {
            if !rules_raids {
                raid.stop();
            }
            if raid.is_stopped() {
                gone.push(*id);
                continue;
            }
            for a in raid.tick(*id, level, inside == Some(*id)) {
                out.push((*id, a));
            }
        }
        for id in gone {
            self.map.remove(&id);
            self.dirty = true;
        }
        if self.tick % 200 == 0 || !out.is_empty() {
            self.dirty = true;
        }
        out
    }
    pub fn to_nbt(&self) -> Tag {
        let raids = self
            .map
            .iter()
            .map(|(id, r)| {
                let mut tag = r.to_nbt();
                if let Tag::Compound(map) = &mut tag {
                    map.insert("id".into(), Tag::Int(*id));
                }
                tag
            })
            .collect();
        Tag::compound([
            ("raids", Tag::List(raids)),
            ("next_id", Tag::Int(self.next_id)),
            ("tick", Tag::Int(self.tick)),
        ])
    }
    pub fn from_nbt(tag: &Tag, seed: i64) -> Self {
        let mut out = Self::default();
        if let Some(Tag::Int(v)) = tag.get("next_id") {
            out.next_id = *v;
        }
        if let Some(Tag::Int(v)) = tag.get("tick") {
            out.tick = *v;
        }
        if let Some(Tag::List(list)) = tag.get("raids") {
            for (i, r) in list.iter().enumerate() {
                if let Some(Tag::Int(id)) = r.get("id")
                    && let Some(raid) = Raid::from_nbt(r, seed.wrapping_add(i as i64))
                {
                    out.map.insert(*id, raid);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Level {
        village: bool,
        raiders: Vec<RaiderInfo>,
        difficulty: i32,
    }
    impl RaidLevel for Level {
        fn difficulty(&self) -> i32 {
            self.difficulty
        }
        fn has_chunk_at(&self, _: [i32; 3]) -> bool {
            true
        }
        fn is_village(&self, _: [i32; 3]) -> bool {
            self.village
        }
        fn village_sections_near(&self, _: [i32; 3]) -> Vec<[i32; 3]> {
            Vec::new()
        }
        fn raiders(&self, _: i32) -> Vec<RaiderInfo> {
            self.raiders.clone()
        }
        fn surface_height(&self, _: i32, _: i32) -> i32 {
            64
        }
        fn is_ticking(&self, _: [i32; 3]) -> bool {
            true
        }
        fn can_spawn_at(&self, _: [i32; 3]) -> bool {
            true
        }
    }
    fn count(spawns: &[Spawn], kind: RaiderType) -> usize {
        spawns.iter().filter(|s| s.kind == kind).count()
    }

    #[test]
    fn waves_per_difficulty() {
        assert_eq!(
            [0, 1, 2, 3].map(num_groups),
            [0, 3, 5, 7],
            "peaceful, easy, normal, hard"
        );
    }
    #[test]
    fn first_wave_is_four_pillagers_plus_bonus() {
        // Normal difficulty always adds one bonus roll (0 or 1) per
        // pillager and vindicator slot: wave 1 has 4 pillagers and 0-1 more.
        for seed in 0..50 {
            let mut raid = Raid::new([0, 64, 0], 2, seed);
            let w = raid.compose_wave(1, 2, false);
            let pillagers = count(&w, RaiderType::Pillager);
            assert!((4..=5).contains(&pillagers), "{pillagers}");
            assert!(count(&w, RaiderType::Vindicator) <= 1);
            assert_eq!(count(&w, RaiderType::Ravager), 0);
            assert_eq!(w.iter().filter(|s| s.leader).count(), 1);
        }
    }
    #[test]
    fn ravagers_carry_riders_by_wave() {
        // Wave 5 on normal: the ravager carries a pillager.
        let mut raid = Raid::new([0, 64, 0], 2, 1);
        let w = raid.compose_wave(5, 2, false);
        let ravagers: Vec<_> = w.iter().filter(|s| s.kind == RaiderType::Ravager).collect();
        assert_eq!(ravagers.len(), 1);
        assert_eq!(ravagers[0].rider, Some(RaiderType::Pillager));
        // Wave 7 on hard: an evoker rides the first ravager, vindicators the rest.
        let mut raid = Raid::new([0, 64, 0], 3, 1);
        let w = raid.compose_wave(7, 3, false);
        let riders: Vec<_> = w
            .iter()
            .filter(|s| s.kind == RaiderType::Ravager)
            .map(|s| s.rider)
            .collect();
        assert!(riders.len() >= 2);
        assert_eq!(riders[0], Some(RaiderType::Evoker));
        assert!(
            riders[1..]
                .iter()
                .all(|r| *r == Some(RaiderType::Vindicator))
        );
        // Wave 4 has no ravager.
        let mut raid = Raid::new([0, 64, 0], 3, 1);
        assert_eq!(
            count(&raid.compose_wave(4, 3, false), RaiderType::Ravager),
            0
        );
    }
    #[test]
    fn witches_arrive_from_wave_five_and_bonus_waves_reuse_the_last_row() {
        let mut raid = Raid::new([0, 64, 0], 2, 3);
        assert_eq!(count(&raid.compose_wave(3, 2, false), RaiderType::Witch), 0);
        assert_eq!(count(&raid.compose_wave(4, 2, false), RaiderType::Witch), 3);
        // Bonus wave of a normal raid: the wave-5 row (and a possible
        // extra ravager).
        let bonus = raid.compose_wave(6, 2, true);
        assert_eq!(count(&bonus, RaiderType::Evoker), 1);
        assert!((1..=2).contains(&count(&bonus, RaiderType::Ravager)));
        assert!(count(&bonus, RaiderType::Witch) <= 1);
    }
    #[test]
    fn only_able_raiders_lead() {
        let mut raid = Raid::new([0, 64, 0], 3, 9);
        let w = raid.compose_wave(4, 3, false);
        assert!(
            w.iter()
                .filter(|s| s.leader)
                .all(|s| s.kind.can_be_leader())
        );
        assert_eq!(w.iter().filter(|s| s.leader).count(), 1);
    }
    #[test]
    fn omen_levels_stack_and_clamp() {
        let mut raid = Raid::new([0, 64, 0], 2, 1);
        assert!(raid.absorb_omen(1));
        assert_eq!(raid.omen_level, 2);
        raid.absorb_omen(4);
        assert_eq!(raid.omen_level, MAX_OMEN_LEVEL);
        assert_eq!(raid.enchant_odds(), 0.75);
    }
    #[test]
    fn a_raid_counts_down_spawns_and_is_won() {
        let level = Level {
            village: true,
            raiders: Vec::new(),
            difficulty: 1,
        };
        let mut raid = Raid::new([0, 64, 0], 1, 5);
        raid.absorb_omen(0);
        let mut spawned = 0;
        // 300 ticks of countdown, then the first wave.
        for _ in 0..400 {
            for a in raid.tick(1, &level, true) {
                if let Action::Spawn { wave, .. } = a {
                    spawned += 1;
                    assert_eq!(wave, 1);
                }
            }
            if spawned > 0 {
                break;
            }
        }
        assert_eq!(spawned, 1);
        assert!(raid.started && raid.groups_spawned == 1);
        assert_eq!(raid.bar.progress, 1.0);
        assert!(raid.total_health > 0.0);
    }
    #[test]
    fn the_last_wave_cleared_wins_after_a_pause() {
        let level = Level {
            village: true,
            raiders: Vec::new(),
            difficulty: 1,
        };
        let mut raid = Raid::new([0, 64, 0], 1, 5);
        raid.started = true;
        raid.groups_spawned = raid.num_groups;
        raid.cooldown_ticks = 0;
        let mut won = false;
        for _ in 0..60 {
            for a in raid.tick(1, &level, true) {
                if let Action::Victory { amplifier, .. } = a {
                    won = true;
                    assert_eq!(amplifier, -1);
                }
            }
        }
        assert!(won && raid.is_victory());
        // The celebration lasts 600 ticks, then the raid stops.
        for _ in 0..600 {
            raid.tick(1, &level, true);
        }
        assert!(raid.is_stopped());
    }
    #[test]
    fn losing_the_village_loses_the_raid() {
        let level = Level {
            village: false,
            raiders: Vec::new(),
            difficulty: 2,
        };
        let mut raid = Raid::new([0, 64, 0], 2, 5);
        raid.started = true;
        raid.groups_spawned = 2;
        raid.tick(1, &level, true);
        assert!(raid.is_loss());
        // Before any wave, losing the village just cancels the raid.
        let mut fresh = Raid::new([0, 64, 0], 2, 5);
        fresh.tick(1, &level, true);
        assert!(fresh.is_stopped());
    }
    #[test]
    fn peaceful_stops_raids() {
        let level = Level {
            village: true,
            raiders: Vec::new(),
            difficulty: 0,
        };
        let mut raid = Raid::new([0, 64, 0], 2, 5);
        raid.tick(1, &level, true);
        assert!(raid.is_stopped());
    }
    #[test]
    fn far_or_idle_raiders_leave() {
        let raider = |id, x, no_action, outside| RaiderInfo {
            id,
            wave: 1,
            health: 24.0,
            position: [x, 64, 0],
            age: 1000,
            no_action_time: no_action,
            ticks_outside: outside,
            in_village: false,
        };
        let level = Level {
            village: true,
            raiders: vec![
                raider(1, 500, 0, 0),
                raider(2, 10, 3000, 29),
                raider(3, 10, 0, 0),
            ],
            difficulty: 2,
        };
        let mut raid = Raid::new([0, 64, 0], 2, 1);
        raid.started = true;
        raid.groups_spawned = 1;
        raid.total_health = 72.0;
        raid.ticks_active = 19;
        let actions = raid.tick(1, &level, true);
        assert!(actions.contains(&Action::Remove { id: 1 }));
        assert!(actions.contains(&Action::Remove { id: 2 }));
        assert!(!actions.contains(&Action::Remove { id: 3 }));
        assert_eq!(raid.total_health, 24.0);
    }
    #[test]
    fn raids_round_trip_through_nbt() {
        let mut raids = Raids::default();
        let (id, triggered) = raids.create_or_extend([10, 64, 20], 2, Some(2), 7);
        assert_eq!(id, 2);
        assert!(triggered);
        raids.map.get_mut(&id).unwrap().add_hero([1, 2, 3, 4]);
        let back = Raids::from_nbt(&raids.to_nbt(), 1);
        let raid = back.get(2).unwrap();
        assert_eq!(raid.center, [10, 64, 20]);
        assert_eq!(raid.omen_level, 3);
        assert_eq!(raid.num_groups, 5);
        assert_eq!(raid.heroes, vec![[1, 2, 3, 4]]);
        assert_eq!(raid.status, Status::Ongoing);
    }
    #[test]
    fn a_second_omen_extends_the_nearby_raid() {
        let mut raids = Raids::default();
        let (a, _) = raids.create_or_extend([0, 64, 0], 2, Some(0), 1);
        let (b, again) = raids.create_or_extend([20, 64, 20], 2, Some(1), 1);
        assert_eq!(a, b);
        assert!(again);
        assert_eq!(raids.get(a).unwrap().omen_level, 3);
        let (c, _) = raids.create_or_extend([300, 64, 0], 2, Some(0), 1);
        assert_ne!(a, c);
    }
}
