//! Viewed areas and their caps (#3716): squares of chunks a mod asks for one player in any level,
//! each `Loaded` (kept loaded and sent, nothing ticks for it) or `Ticking` (as a player's
//! simulation ticket). A call that breaks a cap changes nothing and names the cap it hit.
use crate::fast_hash::FxHashSet;
use std::collections::BTreeMap;

/// The largest radius: vanilla's largest view distance.
pub const MAX_RADIUS: u32 = 32;
/// The farthest an area's centre may be from (0, 0) on each axis, in chunks: the largest world
/// border's edge (`WorldBorder.MAX_SIZE`: 29,999,984 blocks out, inside chunk 1,875,000). Every
/// square, ring and ticket of an area then stays far inside `i32`.
pub const MAX_CENTER: i32 = 1_875_000;
/// The most areas a player may have, every mod together.
pub const MAX_AREAS: usize = 32;
/// The most `Loaded` chunks one player may hold in one level: the ceiling a client of revision 4
/// of `horizon:levels` keeps of a viewed level (`viewed_levels::MAX_COLUMNS`). A higher
/// `max_viewed_loaded_chunks` is clamped to it. A client of an older revision keeps at most
/// `viewed_levels::LEGACY_COLUMNS`, so its own cap is that ([`ViewedCaps::for_client`]).
pub const MAX_LOADED_PER_PLAYER: u32 = horizon_protocol::viewed_levels::MAX_COLUMNS;
/// The default per-player `Loaded` cap.
const DEFAULT_LOADED_PER_PLAYER: u32 = 2048;

/// What an area's ticket does. No default: each area names one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChunkAreaMode {
    /// Loaded to full and sent; nothing ticks there because of the area (`PLAYER_LOADING`).
    Loaded,
    /// Entity and block ticking, as a player's simulation ticket (`PLAYER_SIMULATION`).
    Ticking,
}

/// The chunks within `radius` (Chebyshev) of `center` in `level`, a dimension id. A level the
/// server does not have holds nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkArea {
    pub level: String,
    pub center: (i32, i32),
    pub radius: u32,
    pub mode: ChunkAreaMode,
}

impl ChunkArea {
    /// The square `Streaming.extra` keeps for it, which loads one ring past its radius: a
    /// `Loaded` area's own, a `Ticking` area's out to its block-ticking ring, so its `FULL` ring
    /// loads too.
    pub fn extra(&self) -> ((i32, i32), i32) {
        let r = self.radius as i32;
        match self.mode {
            ChunkAreaMode::Loaded => (self.center, r),
            ChunkAreaMode::Ticking => (self.center, r + 1),
        }
    }
}

/// Which chunk cap a refusal hit; each names its server setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the game maps each to the mod API's `ChunkCap`
pub enum ChunkCap {
    /// `max_viewed_loaded_chunks`, per (player, level).
    LoadedPerPlayer,
    /// `max_viewed_ticking_chunks`, per (player, level).
    TickingPerPlayer,
    /// `max_viewed_loaded_chunks_per_level`, all players together.
    LoadedPerLevel,
    /// `max_viewed_ticking_chunks_per_level`, all players together.
    TickingPerLevel,
}

impl ChunkCap {
    /// The `server.toml` setting.
    pub fn setting(self) -> &'static str {
        match self {
            Self::LoadedPerPlayer => "max_viewed_loaded_chunks",
            Self::TickingPerPlayer => "max_viewed_ticking_chunks",
            Self::LoadedPerLevel => "max_viewed_loaded_chunks_per_level",
            Self::TickingPerLevel => "max_viewed_ticking_chunks_per_level",
        }
    }
    /// Its `server.properties` key.
    pub fn property(self) -> &'static str {
        match self {
            Self::LoadedPerPlayer => "horizon-max-viewed-loaded-chunks",
            Self::TickingPerPlayer => "horizon-max-viewed-ticking-chunks",
            Self::LoadedPerLevel => "horizon-max-viewed-loaded-chunks-per-level",
            Self::TickingPerLevel => "horizon-max-viewed-ticking-chunks-per-level",
        }
    }
    const ALL: [Self; 4] = [
        Self::LoadedPerPlayer,
        Self::TickingPerPlayer,
        Self::LoadedPerLevel,
        Self::TickingPerLevel,
    ];
}

/// Why a call to set a player's areas changed nothing. Each names the cap it hit and its value.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: the game maps each to the mod API's `ChunkAreaRefusal`
pub enum ChunkAreaRefusal {
    /// The player is not online on this server.
    NotOnline,
    /// A radius over the engine's 32.
    RadiusTooLarge { radius: u32, max: u32 },
    /// A centre more than `max` chunks (1,875,000, the world border's limit) from (0, 0) on an
    /// axis.
    CenterOutOfRange { center: (i32, i32), max: i32 },
    /// More than the engine's 32 areas for this player, all mods together.
    TooManyAreas { asked: u32, max: u32 },
    /// Areas in more levels than the client can view (`cap` is its capability answer; 0 for a
    /// client that views none).
    TooManyLevels { asked: u32, cap: u32 },
    /// More chunks in `level` than `cap` allows; `limit` is the setting's value now.
    TooManyChunks {
        cap: ChunkCap,
        level: String,
        asked: u32,
        limit: u32,
    },
}

/// The four chunk caps, server settings (`server.toml` `[server]`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewedCaps {
    pub loaded_per_player: u32,
    pub ticking_per_player: u32,
    pub loaded_per_level: u32,
    pub ticking_per_level: u32,
}

impl Default for ViewedCaps {
    /// 2048 `Loaded`; 441 `Ticking`, one player's simulation area at distance 10 (21 × 21); the
    /// per-level totals 8 times those.
    fn default() -> Self {
        Self {
            loaded_per_player: DEFAULT_LOADED_PER_PLAYER,
            ticking_per_player: 441,
            loaded_per_level: DEFAULT_LOADED_PER_PLAYER * 8,
            ticking_per_level: 441 * 8,
        }
    }
}

impl ViewedCaps {
    pub fn get(&self, cap: ChunkCap) -> u32 {
        match cap {
            ChunkCap::LoadedPerPlayer => self.loaded_per_player,
            ChunkCap::TickingPerPlayer => self.ticking_per_player,
            ChunkCap::LoadedPerLevel => self.loaded_per_level,
            ChunkCap::TickingPerLevel => self.ticking_per_level,
        }
    }
    fn slot(&mut self, cap: ChunkCap) -> &mut u32 {
        match cap {
            ChunkCap::LoadedPerPlayer => &mut self.loaded_per_player,
            ChunkCap::TickingPerPlayer => &mut self.ticking_per_player,
            ChunkCap::LoadedPerLevel => &mut self.loaded_per_level,
            ChunkCap::TickingPerLevel => &mut self.ticking_per_level,
        }
    }
    /// The caps for a player whose client answered `answer`: from revision 4 these, sent in each
    /// `open`; before it, a per-player `Loaded` cap of at most `viewed_levels::LEGACY_COLUMNS`,
    /// which such a client keeps at most.
    pub fn for_client(self, answer: horizon_protocol::viewed_levels::Answer) -> Self {
        if answer.bindings() {
            return self;
        }
        Self {
            loaded_per_player: self
                .loaded_per_player
                .min(horizon_protocol::viewed_levels::LEGACY_COLUMNS),
            ..self
        }
    }
    /// The caps `server.properties` sets (horizon-server writes them from `server.toml`), the
    /// defaults for the others: `property` reads a key. A per-player `Loaded` cap over
    /// [`MAX_LOADED_PER_PLAYER`] is clamped to it; the value asked comes back with the caps.
    pub fn from_properties<'a>(property: impl Fn(&str) -> Option<&'a str>) -> (Self, Option<u32>) {
        let mut caps = Self::default();
        for cap in ChunkCap::ALL {
            if let Some(n) = property(cap.property()).and_then(|v| v.trim().parse::<u32>().ok()) {
                *caps.slot(cap) = n;
            }
        }
        let clamped = (caps.loaded_per_player > MAX_LOADED_PER_PLAYER)
            .then(|| std::mem::replace(&mut caps.loaded_per_player, MAX_LOADED_PER_PLAYER));
        (caps, clamped)
    }
}

/// Each mod's areas for one player, by the mod's id.
pub type Areas = BTreeMap<String, Vec<ChunkArea>>;

/// How many chunks the areas in one level hold: `(loaded, ticking)`. Counted as the union of the
/// areas, by mode: the `Ticking` squares count as ticking; every chunk `Streaming.extra` loads for
/// them that is not one of those counts as loaded: a `Loaded` area's square and the ring past it
/// ((2r + 3)²), and a `Ticking` area's block-ticking and `FULL` rings.
pub fn chunks<'a>(areas: impl Iterator<Item = &'a ChunkArea> + Clone) -> (u32, u32) {
    let mut ticking = FxHashSet::default();
    for a in areas.clone().filter(|a| a.mode == ChunkAreaMode::Ticking) {
        square(a.center, a.radius as i32, |c| {
            ticking.insert(c);
        });
    }
    let mut loaded = FxHashSet::default();
    for a in areas {
        let (center, r) = a.extra();
        square(center, r + 1, |c| {
            if !ticking.contains(&c) {
                loaded.insert(c);
            }
        });
    }
    (loaded.len() as u32, ticking.len() as u32)
}

fn square((cx, cz): (i32, i32), r: i32, mut visit: impl FnMut((i32, i32))) {
    for z in cz - r..=cz + r {
        for x in cx - r..=cx + r {
            visit((x, z));
        }
    }
}

/// The checks of a call that makes `owner`'s areas `new` for a player in `own` whose client views
/// up to `max_levels` other levels: the engine's limits, then the four chunk caps, each level's
/// totals over `others` (`(loaded, ticking)` of every other player there). On success, the
/// player's areas by level after the call, with their counts.
pub fn check(
    areas: &Areas,
    owner: &str,
    new: &[ChunkArea],
    own: &str,
    max_levels: u8,
    caps: &ViewedCaps,
    others: impl Fn(&str) -> (u32, u32),
) -> Result<BTreeMap<String, (u32, u32)>, ChunkAreaRefusal> {
    if let Some(a) = new.iter().find(|a| a.radius > MAX_RADIUS) {
        return Err(ChunkAreaRefusal::RadiusTooLarge {
            radius: a.radius,
            max: MAX_RADIUS,
        });
    }
    let far = |c: i32| !(-MAX_CENTER..=MAX_CENTER).contains(&c);
    if let Some(a) = new.iter().find(|a| far(a.center.0) || far(a.center.1)) {
        return Err(ChunkAreaRefusal::CenterOutOfRange {
            center: a.center,
            max: MAX_CENTER,
        });
    }
    let all: Vec<&ChunkArea> = areas
        .iter()
        .filter(|(o, _)| o.as_str() != owner)
        .flat_map(|(_, a)| a)
        .chain(new)
        .collect();
    if all.len() > MAX_AREAS {
        return Err(ChunkAreaRefusal::TooManyAreas {
            asked: all.len() as u32,
            max: MAX_AREAS as u32,
        });
    }
    let mut levels: BTreeMap<String, (u32, u32)> = BTreeMap::new();
    for a in &all {
        levels.entry(a.level.clone()).or_default();
    }
    let viewed = levels.keys().filter(|l| l.as_str() != own).count() as u32;
    if viewed > u32::from(max_levels) {
        return Err(ChunkAreaRefusal::TooManyLevels {
            asked: viewed,
            cap: u32::from(max_levels),
        });
    }
    for (level, counts) in &mut levels {
        *counts = chunks(all.iter().copied().filter(|a| a.level == *level));
        let (loaded, ticking) = *counts;
        let (other_loaded, other_ticking) = others(level);
        for (cap, asked) in [
            (ChunkCap::LoadedPerPlayer, loaded),
            (ChunkCap::TickingPerPlayer, ticking),
            (ChunkCap::LoadedPerLevel, loaded + other_loaded),
            (ChunkCap::TickingPerLevel, ticking + other_ticking),
        ] {
            let limit = caps.get(cap);
            if asked > limit {
                return Err(ChunkAreaRefusal::TooManyChunks {
                    cap,
                    level: level.clone(),
                    asked,
                    limit,
                });
            }
        }
    }
    Ok(levels)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(center: (i32, i32), radius: u32, mode: ChunkAreaMode) -> ChunkArea {
        ChunkArea {
            level: "minecraft:the_nether".into(),
            center,
            radius,
            mode,
        }
    }

    #[test]
    fn a_loaded_area_counts_its_extra_ring_and_a_ticking_one_its_rings_as_loaded() {
        use ChunkAreaMode::{Loaded, Ticking};
        // (2r + 3)²: 5 × 5 for radius 1.
        assert_eq!(chunks([area((0, 0), 1, Loaded)].iter()), (25, 0));
        // A ticking square of 21 × 21, its rings out to 25 × 25 loaded.
        assert_eq!(chunks([area((0, 0), 10, Ticking)].iter()), (625 - 441, 441));
        // The union: a loaded area inside a ticking square counts no chunk twice.
        assert_eq!(
            chunks([area((0, 0), 10, Ticking), area((0, 0), 2, Loaded)].iter()),
            (184, 441)
        );
    }

    #[test]
    fn the_caps_read_their_properties_and_default_to_the_ruling() {
        let caps = ViewedCaps::default();
        assert_eq!(
            (
                caps.loaded_per_player,
                caps.ticking_per_player,
                caps.loaded_per_level,
                caps.ticking_per_level
            ),
            (2048, 441, 16384, 3528)
        );
        let (read, clamped) = ViewedCaps::from_properties(|k| {
            (k == "horizon-max-viewed-ticking-chunks").then_some(" 9 ")
        });
        assert_eq!((read.ticking_per_player, read.loaded_per_player), (9, 2048));
        assert_eq!(clamped, None);
        // At most the client's ceiling, 4489: a higher value is clamped, and said.
        let (read, clamped) = ViewedCaps::from_properties(|k| {
            (k == "horizon-max-viewed-loaded-chunks").then_some("5000")
        });
        assert_eq!((read.loaded_per_player, clamped), (4489, Some(5000)));
        // A client of revision 4 gets it; an older one at most 2048, all it keeps.
        let answer = |revision| horizon_protocol::viewed_levels::Answer {
            revision,
            max_levels: 2,
        };
        assert_eq!(read.for_client(answer(4)).loaded_per_player, 4489);
        assert_eq!(read.for_client(answer(3)).loaded_per_player, 2048);
        assert_eq!(read.for_client(answer(3)).ticking_per_player, 441);
        let (read, _) = ViewedCaps::from_properties(|k| {
            (k == "horizon-max-viewed-loaded-chunks").then_some("100")
        });
        assert_eq!(read.loaded_per_player, 100);
    }
}
