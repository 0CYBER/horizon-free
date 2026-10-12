//! Pillager patrols (`PatrolSpawner`), the raid omen, and raids (`Raids`):
//! their waves, boss bar, victory and the raiders' bookkeeping.
use super::{Dim, Game};
use horizon_client::mobs::villager as mobs_villager;
use horizon_client::{
    effects::EffectInstance,
    hud_overlays,
    mobs::{Mob, Species, data, raider},
    raid::{self, Action, BarName, RaidLevel, RaiderInfo, RaiderType, Raids, Spawn},
    text::Text,
};
use horizon_core::{geometry::Vec3, random::RandomSource};
use std::collections::{HashMap, HashSet};

/// Patrol and raid state owned by the game.
#[derive(Default)]
pub(super) struct Raiding {
    /// `PatrolSpawner.nextTick`.
    patrol_next_tick: i32,
    raids: Raids,
    loaded: bool,
    /// `ServerPlayer.raidOmenPosition`.
    omen_position: Option<[i32; 3]>,
    /// Beds near each raid's centre, rescanned now and then (`HOME` points
    /// of interest for `RaiderMoveThroughVillageGoal`).
    homes: HashMap<i32, (i64, std::sync::Arc<Vec<[i32; 3]>>)>,
}

/// `SectionPos` of a block position.
pub(super) fn section(p: [i32; 3]) -> (i32, i32, i32) {
    (p[0] >> 4, p[1] >> 4, p[2] >> 4)
}
/// `PoiManager.DistanceTracker`: sections from the nearest village centre,
/// 7 when farther than 6.
pub(super) fn sections_to_village(centers: &HashSet<(i32, i32, i32)>, at: (i32, i32, i32)) -> i32 {
    centers
        .iter()
        .map(|c| {
            (c.0 - at.0)
                .abs()
                .max((c.1 - at.1).abs())
                .max((c.2 - at.2).abs())
        })
        .min()
        .map_or(7, |d| d.min(7))
}

/// The level as a raid sees it, with this tick's village and raider data.
struct Lvl<'a, 'g> {
    game: &'a Game<'g>,
    difficulty: i32,
    centers: &'a HashSet<(i32, i32, i32)>,
    raiders: &'a HashMap<i32, Vec<RaiderInfo>>,
}
impl RaidLevel for Lvl<'_, '_> {
    fn difficulty(&self) -> i32 {
        self.difficulty
    }
    fn has_chunk_at(&self, p: [i32; 3]) -> bool {
        self.game.world.chunks.contains_key(&(p[0] >> 4, p[2] >> 4))
    }
    fn is_village(&self, p: [i32; 3]) -> bool {
        sections_to_village(self.centers, section(p)) <= 1
    }
    fn village_sections_near(&self, p: [i32; 3]) -> Vec<[i32; 3]> {
        let (sx, sy, sz) = section(p);
        let mut out = Vec::new();
        for dx in -2..=2 {
            for dy in -2..=2 {
                for dz in -2..=2 {
                    let s = (sx + dx, sy + dy, sz + dz);
                    let center = [s.0 * 16 + 8, s.1 * 16 + 8, s.2 * 16 + 8];
                    if self.is_village(center) {
                        out.push(center);
                    }
                }
            }
        }
        out
    }
    fn raiders(&self, raid: i32) -> Vec<RaiderInfo> {
        self.raiders.get(&raid).cloned().unwrap_or_default()
    }
    fn surface_height(&self, x: i32, z: i32) -> i32 {
        self.game.surface_height(x, z)
    }
    fn is_ticking(&self, p: [i32; 3]) -> bool {
        [(-10, -10), (10, -10), (-10, 10), (10, 10)]
            .iter()
            .all(|(dx, dz)| self.has_chunk_at([p[0] + dx, p[1], p[2] + dz]))
    }
    fn can_spawn_at(&self, p: [i32; 3]) -> bool {
        if !self.is_ticking(p) {
            return false;
        }
        let g = self.game;
        let w = g.mob_world_base(&[]);
        horizon_client::mobs::spawning::placement_ok(Species::Ravager, &w, p[0], p[1], p[2])
            || w.block_name(p[0], p[1] - 1, p[2]) == "minecraft:snow"
                && g.res.facts.is_air(g.world.block(p[0], p[1], p[2]))
    }
}

/// `DyeColor.getFireworkColor`.
const FIREWORK_COLORS: [u32; 16] = [
    0xF0F0F0, 0xEB8844, 0xC354CD, 0x6689D3, 0xDECF2A, 0x41CD34, 0xD88198, 0x434343, 0xABABAB,
    0x287697, 0x7B2FBE, 0x253192, 0x51301A, 0x3B511A, 0xB3312C, 0x1E1B1B,
];

fn block_of(p: Vec3) -> [i32; 3] {
    [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32]
}

impl Game<'_> {
    /// `Heightmap.Types.WORLD_SURFACE`: the first free block above the
    /// highest non-air one.
    pub(super) fn surface_height(&self, x: i32, z: i32) -> i32 {
        let top = if self.dim == Dim::Nether { 127 } else { 319 };
        (horizon_client::chunk::MIN_Y..=top)
            .rev()
            .find(|&y| !self.res.facts.is_air(self.world.block(x, y, z)))
            .map_or(horizon_client::chunk::MIN_Y, |y| y + 1)
    }
    /// Every claimed village point of interest (job sites, beds, bells).
    fn village_pois(&self) -> Vec<[i32; 3]> {
        let mut out: Vec<[i32; 3]> = Vec::new();
        for m in &self.mobs {
            if m.species() != Species::Villager || !m.alive() {
                continue;
            }
            for p in [m.extra.home, m.extra.bed, m.extra.meeting]
                .into_iter()
                .flatten()
            {
                if !out.contains(&p) {
                    out.push(p);
                }
            }
        }
        out
    }
    fn difficulty_id(&self) -> i32 {
        use horizon_world::food::Difficulty;
        match self.world.health_rules.difficulty {
            Difficulty::Peaceful => 0,
            Difficulty::Easy => 1,
            Difficulty::Normal => 2,
            Difficulty::Hard => 3,
        }
    }
    fn player_block(&self) -> Option<[i32; 3]> {
        self.player.as_ref().map(|p| block_of(p.position))
    }
    fn raids_path(&self) -> std::path::PathBuf {
        self.world_root.join("data/minecraft/raids.dat")
    }
    fn load_raids(&mut self) {
        if self.raiding.loaded {
            return;
        }
        self.raiding.loaded = true;
        let seed = self.world.random.next_long();
        if let Some(tag) = horizon_client::save::read_dat(&self.raids_path())
            && let Some(data) = tag.get("data")
        {
            self.raiding.raids = Raids::from_nbt(data, seed);
        }
    }
    /// `Raids` as `data/minecraft/raids.dat`, when it changed.
    pub(super) fn save_raids(&mut self) {
        if !self.raiding.loaded || !self.raiding.raids.dirty || self.remote.is_some() {
            return;
        }
        let tag = horizon_world::nbt::Tag::compound([
            ("data", self.raiding.raids.to_nbt()),
            (
                "DataVersion",
                horizon_world::nbt::Tag::Int(horizon_world::DATA_VERSION),
            ),
        ]);
        if horizon_client::save::write_dat(&self.raids_path(), tag).is_ok() {
            self.raiding.raids.dirty = false;
        }
    }

    /// `BadOmenMobEffect` and `RaidOmenMobEffect`, before the effects tick:
    /// Bad Omen inside a village becomes Raid Omen, which starts or extends a
    /// raid when it runs out.
    pub(super) fn tick_omens(&mut self) {
        if self.dim != Dim::Overworld || self.spectator() {
            return;
        }
        let Some(pos) = self.player_block() else {
            return;
        };
        self.load_raids();
        if let Some(amp) = self.potions.effects.amplifier("bad_omen")
            && self.difficulty_id() != 0
            && sections_to_village(&self.village_centers(), section(pos)) <= 1
            && self
                .raiding
                .raids
                .nearby(pos)
                .is_none_or(|(_, r)| r.omen_level < raid::MAX_OMEN_LEVEL)
        {
            self.potions.effects.list.retain(|e| e.id != "bad_omen");
            self.potions
                .effects
                .add(EffectInstance::new("raid_omen", 600, amp));
            self.raiding.omen_position = Some(pos);
            let eye = self.eye_position().unwrap_or_default();
            self.audio
                .play("minecraft:event.mob_effect.raid_omen", Some(eye), 1.0, 1.0);
        }
        let due = self
            .potions
            .effects
            .get("raid_omen")
            .filter(|e| e.duration == 1)
            .map(|e| e.amplifier);
        if let Some(amp) = due
            && let Some(at) = self.raiding.omen_position.take()
        {
            self.create_or_extend_raid(at, Some(amp));
        }
    }
    /// `Raids.createOrExtendRaid`: the raid centres on the claimed village
    /// points within 64 blocks.
    fn create_or_extend_raid(&mut self, at: [i32; 3], amplifier: Option<i32>) -> Option<i32> {
        if !self.ui.rules.bool("raids") || self.dim != Dim::Overworld {
            return None;
        }
        let near: Vec<[i32; 3]> = self
            .village_pois()
            .into_iter()
            .filter(|p| (0..3).map(|i| f64::from(p[i] - at[i]).powi(2)).sum::<f64>() <= 4096.0)
            .collect();
        let center = if near.is_empty() {
            at
        } else {
            let n = near.len() as f64;
            let mean = |i: usize| near.iter().map(|p| f64::from(p[i])).sum::<f64>() / n;
            [
                mean(0).floor() as i32,
                mean(1).floor() as i32,
                mean(2).floor() as i32,
            ]
        };
        let seed = self.world.random.next_long();
        let difficulty = self.difficulty_id();
        let (id, triggered) = self
            .raiding
            .raids
            .create_or_extend(center, difficulty, amplifier, seed);
        if triggered {
            self.stats.custom("raid_trigger", 1);
            self.trigger_player("minecraft:voluntary_exile");
        }
        Some(id)
    }
    /// `ServerLevel.getRaidAt`.
    pub(super) fn raiding_nearby(&self, at: [i32; 3]) -> Option<i32> {
        self.raiding.raids.nearby(at).map(|(id, _)| id)
    }
    pub(super) fn raid_started(&self, id: i32) -> bool {
        self.raiding.raids.get(id).is_some_and(|r| r.started)
    }
    /// `/raid start`: a raid in the local village at the given omen level.
    pub(super) fn raid_command_create(&mut self, at: [i32; 3], omen_level: i32) -> bool {
        self.load_raids();
        let Some(id) = self.create_or_extend_raid(at, None) else {
            return false;
        };
        if let Some(r) = self.raiding.raids.map.get_mut(&id) {
            r.omen_level = omen_level;
        }
        self.raiding.raids.dirty = true;
        true
    }
    pub(super) fn raid_command_stop(&mut self, id: i32) {
        if let Some(r) = self.raiding.raids.map.get_mut(&id) {
            r.stop();
        }
    }
    pub(super) fn raid_set_omen(&mut self, id: i32, level: i32) -> i32 {
        let Some(r) = self.raiding.raids.map.get_mut(&id) else {
            return 0;
        };
        std::mem::replace(&mut r.omen_level, level)
    }
    pub(super) fn raid_glow(&mut self, id: i32) {
        for m in &mut self.mobs {
            if raider::state(m).is_some_and(|s| s.raid_id == Some(id)) {
                m.add_effect("glowing", 1000, 1);
            }
        }
    }
    /// The `/raid check` summary line.
    pub(super) fn raid_status_text(&self, id: i32) -> String {
        let Some(r) = self.raiding.raids.get(id) else {
            return String::new();
        };
        let members: Vec<&Mob> = self
            .mobs
            .iter()
            .filter(|m| m.alive() && raider::state(m).is_some_and(|s| s.raid_id == Some(id)))
            .collect();
        let health: f32 = members.iter().map(|m| m.health).sum();
        format!(
            "Num groups spawned: {} Raid omen level: {} Num mobs: {} Raid health: {} / {}",
            r.groups_spawned,
            r.omen_level,
            members.len(),
            health,
            r.total_health
        )
    }

    /// The living raiders by raid.
    fn raider_infos(&self, centers: &HashSet<(i32, i32, i32)>) -> HashMap<i32, Vec<RaiderInfo>> {
        let mut out: HashMap<i32, Vec<RaiderInfo>> = HashMap::new();
        for m in &self.mobs {
            if !m.alive() || m.removed {
                continue;
            }
            let Some(s) = raider::state(m) else { continue };
            let Some(id) = s.raid_id else { continue };
            let position = block_of(m.position);
            out.entry(id).or_default().push(RaiderInfo {
                id: m.id,
                wave: s.wave,
                health: m.health,
                position,
                age: m.age,
                no_action_time: m.ai.no_action_time,
                ticks_outside: s.ticks_outside_raid,
                in_village: sections_to_village(centers, section(position)) <= 1,
            });
        }
        out
    }
    /// `ServerLevel.tick` for the raids and their raiders.
    pub(super) fn tick_raids(&mut self) {
        if self.remote.is_some() || self.dim != Dim::Overworld {
            return;
        }
        self.load_raids();
        let centers = self.village_centers();
        let infos = self.raider_infos(&centers);
        let mut raids = std::mem::take(&mut self.raiding.raids);
        let actions = {
            let lvl = Lvl {
                game: self,
                difficulty: self.difficulty_id(),
                centers: &centers,
                raiders: &infos,
            };
            raids.tick(&lvl, self.ui.rules.bool("raids"), self.player_block())
        };
        self.raiding.raids = raids;
        for (id, action) in actions {
            self.apply_raid_action(id, action);
        }
        self.sync_raiders(&centers);
        self.sync_villager_raids(&infos);
    }
    fn apply_raid_action(&mut self, id: i32, action: Action) {
        match action {
            Action::Spawn { pos, wave, spawns } => self.spawn_wave(id, pos, wave, &spawns),
            Action::Horn { pos } => self.raid_horn(id, pos),
            Action::Remove { id: mob } => {
                if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                    let s = raider::state_mut(m);
                    s.raid_id = None;
                    s.ticks_outside_raid = 0;
                    s.active_raid = false;
                }
            }
            Action::SetOutside { id: mob, ticks } => {
                if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                    raider::state_mut(m).ticks_outside_raid = ticks;
                }
            }
            Action::Victory { heroes, amplifier } => {
                let me = self.level.player_uuid;
                if me.is_some_and(|u| heroes.contains(&u)) && !self.spectator() {
                    let mut effect =
                        EffectInstance::new("hero_of_the_village", raid::HERO_DURATION, amplifier);
                    effect.visible = false;
                    self.potions.effects.add(effect);
                    self.stats.custom("raid_win", 1);
                    self.trigger_player("minecraft:hero_of_the_village");
                }
            }
        }
    }
    /// `Raid.playSound`: the horn, 13 blocks away from the player toward the
    /// wave, for players within 64 blocks or in the raid.
    fn raid_horn(&mut self, id: i32, pos: [i32; 3]) {
        let Some(p) = self.player.as_ref().map(|p| p.position) else {
            return;
        };
        let in_raid = self.raiding.raids.get(id).is_some_and(|r| r.bar.player);
        let (rx, rz) = (f64::from(pos[0]) + 0.5, f64::from(pos[2]) + 0.5);
        let distance = ((rx - p.x).powi(2) + (rz - p.z).powi(2)).sqrt();
        if distance <= 64.0 || in_raid {
            let k = 13.0 / distance.max(1e-4);
            let at = [p.x + k * (rx - p.x), p.y, p.z + k * (rz - p.z)];
            self.audio
                .play("minecraft:event.raid.horn", Some(at), 1.0, 1.0);
        }
    }
    /// `Raid.spawnGroup`: a wave of raiders around `pos`, ravagers carrying
    /// their riders.
    fn spawn_wave(&mut self, raid_id: i32, pos: [i32; 3], wave: i32, spawns: &[Spawn]) {
        let odds = self
            .raiding
            .raids
            .get(raid_id)
            .map_or(0.0, raid::Raid::enchant_odds);
        for sp in spawns {
            let Some(id) = self.spawn_raider(sp.kind, pos, raid_id, wave, sp.leader, odds) else {
                continue;
            };
            if let Some(kind) = sp.rider
                && let Some(rider) = self.spawn_raider(kind, pos, raid_id, wave, false, odds)
            {
                self.mount_mob(rider, id);
            }
        }
    }
    /// `Raid.joinRaid` for a new raider, with `applyRaidBuffs`.
    fn spawn_raider(
        &mut self,
        kind: RaiderType,
        pos: [i32; 3],
        raid_id: i32,
        wave: i32,
        leader: bool,
        odds: f32,
    ) -> Option<u32> {
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 1.0,
            f64::from(pos[2]) + 0.5,
        ];
        let id = self.spawn_mob_for(kind.entity(), at, false, true)?;
        let m = self.mobs.iter_mut().find(|m| m.id == id)?;
        if leader {
            raider::set_patrol_leader(m, true);
        }
        raider::finalize(m, raider::Reason::Event);
        let s = raider::state_mut(m);
        s.raid_id = Some(raid_id);
        s.wave = wave;
        s.can_join_raid = true;
        s.ticks_outside_raid = 0;
        s.active_raid = true;
        m.on_ground = true;
        self.apply_raid_buffs(id, kind, wave, odds);
        Some(id)
    }

    /// Per-tick raid bookkeeping of every raider (and the ravagers they
    /// ride): the raid's state for the goals, joining and recruiting,
    /// the banner of a fallen captain, idle time.
    fn sync_raiders(&mut self, centers: &HashSet<(i32, i32, i32)>) {
        let now = self.world.game_time;
        let golems: HashSet<u32> = self
            .mobs
            .iter()
            .filter(|m| m.species() == Species::IronGolem)
            .map(|m| m.id)
            .collect();
        // Living leaders and carried riders.
        let mut leaders: HashSet<(i32, i32)> = HashSet::new();
        let mut ridden: HashSet<u32> = HashSet::new();
        let mut positions: Vec<(u32, Vec3, bool)> = Vec::new();
        for m in &self.mobs {
            let Some(s) = raider::state(m) else { continue };
            if !m.alive() {
                continue;
            }
            if let (Some(id), true) = (s.raid_id, s.patrol_leader) {
                leaders.insert((id, s.wave));
            }
            if let Some(v) = m.extra.mount {
                ridden.insert(v);
            }
            positions.push((m.id, m.position, s.active_raid));
        }
        let banners: Vec<(usize, Vec3)> = self
            .dropped
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                &*e.stack.item == raider::BANNER
                    && e.stack.data().is_some_and(|d| d.ominous_banner)
                    && e.pickup_delay == 0
            })
            .map(|(i, e)| (i, e.position))
            .collect();
        let mut taken: Vec<usize> = Vec::new();
        let mut recruit: Vec<(i32, i32, Vec3)> = Vec::new();
        let homes = self.raid_homes();
        for m in &mut self.mobs {
            let species = m.species();
            if !raider::is_raider(species) {
                continue;
            }
            let alive = m.alive();
            let position = m.position;
            let block = block_of(position);
            let in_village = sections_to_village(centers, section(block)) <= 1;
            let has_rider = ridden.contains(&m.id);
            let target_golem = matches!(m.ai.target, Some(horizon_client::mobs::Who::Mob(t)) if golems.contains(&t));
            let target_player = m.ai.target == Some(horizon_client::mobs::Who::Player);
            let (health, age, range) = (m.health, m.age, m.attrs.follow_range);
            let raids = &mut self.raiding.raids;
            let s = m.extra.raider.get_or_insert_with(Box::default);
            s.rider = has_rider;
            s.in_village = in_village;
            if s.raid_id.is_some_and(|id| raids.get(id).is_none()) {
                s.raid_id = None;
                s.ticks_outside_raid = 0;
            }
            match s.raid_id.and_then(|id| raids.get(id)) {
                Some(r) => {
                    s.active_raid = r.active;
                    s.raid_over = r.is_over();
                    s.raid_loss = r.is_loss();
                    s.raid_center = r.center;
                    s.homes = s.raid_id.and_then(|id| homes.get(&id).cloned());
                }
                None => {
                    s.active_raid = false;
                    s.raid_over = false;
                    s.raid_loss = false;
                    s.homes = None;
                    s.banner_at = None;
                }
            }
            if !alive {
                continue;
            }
            // Raider.updateNoActionTime adds two per tick, and a raider
            // fighting a player or golem in a raid is never idle.
            if s.raid_id.is_some() {
                m.ai.no_action_time = if target_player || target_golem {
                    0
                } else {
                    m.ai.no_action_time + 1
                };
            }
            // Raider.aiStep: join the raid the raider stands in.
            if s.raid_id.is_none()
                && s.can_join_raid
                && now % 20 == 0
                && m.ai.no_action_time <= 2400
                && let Some((id, r)) = raids.nearby(block)
            {
                let wave = r.groups_spawned;
                s.raid_id = Some(id);
                s.wave = wave;
                s.ticks_outside_raid = 0;
                if let Some(r) = raids.map.get_mut(&id) {
                    r.total_health += health;
                }
            }
            // PathfindToRaidGoal.recruitNearby.
            if s.active_raid
                && !s.raid_over
                && !s.in_village
                && age % 20 == 0
                && let Some(id) = s.raid_id
            {
                recruit.push((id, s.wave, position));
            }
            // ObtainRaidLeaderBannerGoal.
            s.banner_at = None;
            if let Some(id) = s.raid_id
                && s.active_raid
                && !s.raid_over
                && raider::can_be_leader(species)
                && m.extra.head.as_deref() != Some(raider::BANNER)
                && !leaders.contains(&(id, s.wave))
            {
                let near = banners
                    .iter()
                    .filter(|(i, p)| {
                        !taken.contains(i)
                            && (p.x - position.x).abs() <= range
                            && (p.z - position.z).abs() <= range
                            && (p.y - position.y).abs() <= 8.0
                    })
                    .min_by(|a, b| {
                        a.1.distance_squared(position)
                            .total_cmp(&b.1.distance_squared(position))
                    });
                if let Some((i, p)) = near {
                    s.banner_at = Some([p.x, p.y, p.z]);
                    if p.distance_squared(position) < 1.414 * 1.414 {
                        // pickUpItem: the banner becomes the wave's leader.
                        taken.push(*i);
                        leaders.insert((id, s.wave));
                        s.banner_at = None;
                        s.patrol_leader = true;
                        s.patrolling = true;
                        m.extra.head = Some(raider::BANNER.into());
                    }
                }
            }
        }
        taken.sort_unstable();
        for i in taken.into_iter().rev() {
            self.dropped.remove(i);
        }
        // Recruiting: raiders nearby that are in no raid join this one.
        for (id, wave, at) in recruit {
            let now_nearby: Vec<u32> = positions
                .iter()
                .filter(|(_, p, active)| {
                    !active
                        && (p.x - at.x).abs() <= 16.0
                        && (p.y - at.y).abs() <= 16.0
                        && (p.z - at.z).abs() <= 16.0
                })
                .map(|(i, _, _)| *i)
                .collect();
            for mid in now_nearby {
                let Some(m) = self.mobs.iter_mut().find(|m| m.id == mid) else {
                    continue;
                };
                let health = m.health;
                let idle = m.ai.no_action_time;
                let s = raider::state_mut(m);
                if s.raid_id.is_none() && s.can_join_raid && idle <= 2400 {
                    s.raid_id = Some(id);
                    s.wave = wave;
                    s.ticks_outside_raid = 0;
                    s.active_raid = true;
                    if let Some(r) = self.raiding.raids.map.get_mut(&id) {
                        r.total_health += health;
                    }
                }
            }
        }
    }
    /// Beds within 64 blocks of each raid's centre (the `HOME` points of
    /// interest), rescanned every five seconds.
    fn raid_homes(&mut self) -> HashMap<i32, std::sync::Arc<Vec<[i32; 3]>>> {
        let now = self.world.game_time;
        let ids: Vec<(i32, [i32; 3])> = self
            .raiding
            .raids
            .map
            .iter()
            .filter(|(_, r)| r.active && !r.is_over() && r.started)
            .map(|(id, r)| (*id, r.center))
            .collect();
        self.raiding
            .homes
            .retain(|id, _| ids.iter().any(|(i, _)| i == id));
        for (id, c) in ids {
            if self
                .raiding
                .homes
                .get(&id)
                .is_some_and(|(at, _)| now - at < 100)
            {
                continue;
            }
            let mut beds = Vec::new();
            let mut known: HashMap<u32, bool> = HashMap::new();
            for y in c[1] - 32..=c[1] + 32 {
                for z in c[2] - 64..=c[2] + 64 {
                    for x in c[0] - 64..=c[0] + 64 {
                        let state = self.world.block(x, y, z);
                        if self.res.facts.is_air(state) {
                            continue;
                        }
                        let bed = *known.entry(state.0).or_insert_with(|| {
                            self.res.facts.get(state).name.ends_with("_bed")
                                && self.res.assets.registry.state(state).is_some_and(|s| {
                                    s.properties.get("part").map(String::as_str) == Some("head")
                                })
                        });
                        if bed {
                            beds.push([x, y, z]);
                        }
                    }
                }
            }
            self.raiding
                .homes
                .insert(id, (now, std::sync::Arc::new(beds)));
        }
        self.raiding
            .homes
            .iter()
            .map(|(id, (_, v))| (*id, v.clone()))
            .collect()
    }
    /// `Raider.die`: the killing player becomes a hero of the village.
    pub(super) fn raider_died(&mut self, m: &Mob, by_player: bool) {
        let Some(id) = raider::state(m).and_then(|s| s.raid_id) else {
            return;
        };
        if by_player
            && let Some(uuid) = self.level.player_uuid
            && let Some(r) = self.raiding.raids.map.get_mut(&id)
        {
            r.add_hero(uuid);
        }
    }

    /// `SetRaidStatus` / `ResetRaidStatus` for villagers: which raid package
    /// they follow, by the raid nearest to them.
    fn sync_villager_raids(&mut self, infos: &HashMap<i32, Vec<RaiderInfo>>) {
        let raids = &self.raiding.raids;
        let random = &mut self.world.random;
        for m in &mut self.mobs {
            if m.species() != Species::Villager {
                continue;
            }
            let at = block_of(m.position);
            let status = match raids.nearby(at) {
                Some((id, r)) if !r.is_stopped() && !r.is_loss() => {
                    let alive = infos.get(&id).map_or(0, Vec::len);
                    if r.is_victory() {
                        mobs_villager::RAID_VICTORY
                    } else if r.has_first_wave_spawned() && !r.is_between_waves(alive) {
                        mobs_villager::RAID
                    } else {
                        mobs_villager::PRE_RAID
                    }
                }
                _ => mobs_villager::RAID_NONE,
            };
            let ext = mobs_villager::ext_mut(m);
            // ResetRaidStatus clears at once; SetRaidStatus one tick in twenty.
            if status == mobs_villager::RAID_NONE
                || ext.raid != status && random.next_int_bound(20).unwrap_or(1) == 0
            {
                ext.raid = status;
            }
        }
    }

    /// The raid boss bars: red and notched in tens, with the raid's title.
    pub(super) fn raid_bars(&self, bars: &mut Vec<hud_overlays::BossBar>) {
        for raid in self.raiding.raids.map.values() {
            if !raid.bar.visible || !raid.bar.player || raid.is_stopped() {
                continue;
            }
            let title = self.translate("event.minecraft.raid", "Raid");
            let name = match raid.bar.name {
                BarName::Raid => title,
                BarName::Remaining(n) => format!(
                    "{title} - {}",
                    self.translate(
                        "event.minecraft.raid.raiders_remaining",
                        "Raiders Remaining: %s"
                    )
                    .replace("%s", &n.to_string())
                ),
                BarName::Victory => {
                    self.translate("event.minecraft.raid.victory.full", "Raid - Victory")
                }
                BarName::Defeat => {
                    self.translate("event.minecraft.raid.defeat.full", "Raid - Defeat")
                }
            };
            bars.push(hud_overlays::BossBar {
                name: Text::literal(name),
                progress: raid.bar.progress,
                color: 2,
                overlay: 2,
            });
        }
    }
    /// A firework a villager launches to celebrate (`CelebrateVillagersSurvivedRaid`).
    pub(super) fn villager_firework(&mut self, at: [f64; 3], color: usize, flight: u8) {
        let mut stack = horizon_client::items::ItemStack::new("minecraft:firework_rocket", 1);
        stack.data_mut().fireworks = Some((
            flight,
            vec![horizon_client::particles::FireworkExplosion {
                shape: horizon_client::particles::FireworkShape::Burst,
                colors: vec![FIREWORK_COLORS[color & 15]],
                fade: Vec::new(),
                trail: false,
                twinkle: false,
            }],
        ));
        let rocket = self.new_rocket(Vec3::new(at[0], at[1], at[2]), &stack, false);
        self.rockets.push(rocket);
    }
    /// `BehaviorUtils.throwItem`: a hero's gift tossed toward the player.
    pub(super) fn throw_gift(&mut self, table: &str, from: [f64; 3], to: [f64; 3]) {
        let drops = self
            .res
            .items
            .loot_table(table, false, None, &mut self.world.random);
        let d = Vec3::new(to[0] - from[0], to[1] - from[1], to[2] - from[2]).normalize();
        for stack in drops {
            self.spawn_item(stack, from, [d.x * 0.3, d.y * 0.3, d.z * 0.3], 10);
        }
    }
}

impl Game<'_> {
    /// Sections holding a claimed village point of interest (a job site,
    /// bed or bell a villager has taken): `PoiManager.isVillageCenter`.
    pub(super) fn village_centers(&self) -> HashSet<(i32, i32, i32)> {
        let mut out = HashSet::new();
        for m in &self.mobs {
            if m.species() != Species::Villager || !m.alive() {
                continue;
            }
            for p in [m.extra.home, m.extra.bed, m.extra.meeting]
                .into_iter()
                .flatten()
            {
                out.insert(section(p));
            }
        }
        out
    }
    /// `ServerLevel.isCloseToVillage`.
    pub(super) fn close_to_village(&self, at: [i32; 3], distance: i32) -> bool {
        distance <= 6 && sections_to_village(&self.village_centers(), section(at)) <= distance
    }
    /// `PatrolSpawner.tick`: every 10 minutes, a fifth of the time in
    /// daylight, a patrol of pillagers spawns 24 to 48 blocks from a player
    /// who is not near a village.
    pub(super) fn tick_patrol_spawner(&mut self) {
        if self.dim != Dim::Overworld
            || self.world.health_rules.difficulty == horizon_world::food::Difficulty::Peaceful
            || !self.ui.rules.bool("spawn_monsters")
            || !self.ui.rules.bool("spawn_patrols")
        {
            return;
        }
        self.raiding.patrol_next_tick -= 1;
        if self.raiding.patrol_next_tick > 0 {
            return;
        }
        let delay = self.world.random.next_int_bound(1200).unwrap_or(0);
        self.raiding.patrol_next_tick += 12000 + delay;
        let bright = self.sky_darken() < 4;
        let r = &mut self.world.random;
        if !bright || r.next_int_bound(5).unwrap_or(1) != 0 {
            return;
        }
        // One player: `players.get(random.nextInt(1))`.
        let _ = r.next_int_bound(1);
        let Some(p) = self.player.as_ref().map(|p| p.position) else {
            return;
        };
        if self.spectator() {
            return;
        }
        let block = [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32];
        if self.close_to_village(block, 2) {
            return;
        }
        let r = &mut self.world.random;
        let x = (24 + r.next_int_bound(24).unwrap_or(0)) * if r.next_boolean() { -1 } else { 1 };
        let z = (24 + r.next_int_bound(24).unwrap_or(0)) * if r.next_boolean() { -1 } else { 1 };
        let mut at = [block[0] + x, block[1], block[2] + z];
        // hasChunksAt(±10).
        for (dx, dz) in [(-10, -10), (10, -10), (-10, 10), (10, 10)] {
            if !self
                .world
                .chunks
                .contains_key(&((at[0] + dx) >> 4, (at[2] + dz) >> 4))
            {
                return;
            }
        }
        // The `can_pillager_patrol_spawn` attribute: the early game timeline
        // (first five days) and the mushroom fields forbid it.
        let biome = self.biome_facts(at[0], at[1], at[2]);
        if self.world.day_time < 120_000 || biome.mushroom {
            return;
        }
        let f = self.local_difficulty([f64::from(at[0]), f64::from(at[1]), f64::from(at[2])]);
        let group = f.effective.ceil() as i32 + 1;
        for i in 0..group {
            at[1] = self.motion_top(at[0], at[2]);
            if !self.spawn_patrol_member(at, i == 0) && i == 0 {
                break;
            }
            let r = &mut self.world.random;
            at[0] += r.next_int_bound(5).unwrap_or(0) - r.next_int_bound(5).unwrap_or(0);
            at[2] += r.next_int_bound(5).unwrap_or(0) - r.next_int_bound(5).unwrap_or(0);
        }
    }
    /// A raider's ominous banner (drop chance 2.0: always dropped) and a
    /// patrol captain's ominous bottle (`entities/pillager`).
    /// `OminousBottleAmplifier.onConsume`: Bad Omen for 100 minutes at the
    /// bottle's amplifier, then the dispose sound; the bottle is used up.
    pub(super) fn drink_ominous_bottle(
        &mut self,
        slot: usize,
        stack: &horizon_client::items::ItemStack,
    ) {
        let amplifier = stack.data().and_then(|d| d.ominous_amplifier).unwrap_or(0);
        let mut effect =
            horizon_client::effects::EffectInstance::new("bad_omen", 120_000, i32::from(amplifier));
        effect.visible = false;
        self.potions.effects.add(effect);
        let eye = self.eye_position().unwrap_or_default();
        self.audio
            .play("minecraft:item.ominous_bottle.dispose", Some(eye), 1.0, 1.0);
        if !self.creative
            && let Some(s) = self.inventory[slot].as_mut()
        {
            s.count -= 1;
            if s.count == 0 {
                self.inventory[slot] = None;
            }
        }
    }
    /// `PatrolSpawner.spawnPatrolMember`: a pillager on a valid empty block
    /// that may spawn there; the leader picks a target and wears the banner.
    fn spawn_patrol_member(&mut self, at: [i32; 3], leader: bool) -> bool {
        let state = self.world.block(at[0], at[1], at[2]);
        let below = self.world.block(at[0], at[1] - 1, at[2]);
        let empty = self.res.mob_data.state(state).flags & data::VALID_EMPTY_SPAWN != 0;
        let valid = self.res.mob_data.state(below).flags & data::VALID_SPAWN != 0;
        // PatrollingMonster.checkPatrollingMonsterSpawnRules.
        if !empty || !valid || self.world.light(at[0], at[1], at[2]) & 15 > 8 {
            return false;
        }
        let pos = [f64::from(at[0]), f64::from(at[1]), f64::from(at[2])];
        let Some(id) = self.spawn_mob_for("minecraft:pillager", pos, false, true) else {
            return false;
        };
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) else {
            return false;
        };
        if leader {
            raider::set_patrol_leader(m, true);
            // findPatrolTarget runs before the mob is placed, so the target
            // is around the world origin.
            raider::find_patrol_target_from(m, [0, 0, 0]);
        }
        raider::finalize(m, raider::Reason::Patrol);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_to_village_is_a_chebyshev_distance_capped_at_seven() {
        let mut centers = HashSet::new();
        assert_eq!(sections_to_village(&centers, (0, 0, 0)), 7);
        centers.insert(section([40, 64, 40]));
        assert_eq!(sections_to_village(&centers, (2, 4, 2)), 0);
        assert_eq!(sections_to_village(&centers, (4, 4, 2)), 2);
        assert_eq!(sections_to_village(&centers, (-10, 4, 2)), 7);
        assert_eq!(section([-1, -17, 15]), (-1, -2, 0));
    }
}
