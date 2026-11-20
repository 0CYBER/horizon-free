//! Player statistics (`ServerStatsCounter`): the custom counters, blocks
//! mined, items used/crafted/broken/picked up/dropped and mobs killed or
//! killed by, saved to `players/stats/<uuid>.json`.
use horizon_core::random::RandomSource;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

/// `stats[type][key]`, with `type` and `key` namespaced ids.
#[derive(Default)]
pub struct Stats {
    pub values: BTreeMap<String, BTreeMap<String, i32>>,
    pub loaded: bool,
    /// Ticks until the phantom spawner runs again.
    phantom_tick: i32,
    /// Statistic changes (type, key, amount; `None` = reset) for the
    /// scoreboard criteria to follow.
    pub events: Vec<(String, String, Option<i32>)>,
    /// Changed since the client was last sent them (`StatsCounter`'s dirty
    /// set), by type.
    dirty: BTreeMap<String, BTreeSet<String>>,
}

/// `minecraft:` and `name`, given to `f` from the stack when short: some
/// counters go up every tick for every player, and they are found without
/// a string made for each.
pub fn namespaced<T>(name: &str, f: impl FnOnce(&str) -> T) -> T {
    const NAMESPACE: &str = "minecraft:";
    let mut buf = [0u8; 96];
    let len = NAMESPACE.len() + name.len();
    if len > buf.len() {
        return f(&format!("{NAMESPACE}{name}"));
    }
    buf[..NAMESPACE.len()].copy_from_slice(NAMESPACE.as_bytes());
    buf[NAMESPACE.len()..len].copy_from_slice(name.as_bytes());
    // Two strings one after the other are one.
    f(std::str::from_utf8(&buf[..len]).unwrap_or_default())
}

/// The map under `key`, made when missing, found without an owned key
/// when there.
fn entry<'a, V: Default>(map: &'a mut BTreeMap<String, V>, key: &str) -> &'a mut V {
    if !map.contains_key(key) {
        map.insert(key.to_owned(), V::default());
    }
    map.get_mut(key).expect("just inserted")
}

impl Stats {
    /// `PhantomSpawner.tick`'s countdown: one tick down, true when the spawner runs.
    pub fn phantom_due(&mut self) -> bool {
        self.phantom_tick -= 1;
        self.phantom_tick <= 0
    }
    /// Sets the ticks until the phantom spawner runs.
    pub fn set_phantom_tick(&mut self, ticks: i32) {
        self.phantom_tick = ticks;
    }
    /// Ticks until the phantom spawner runs again, added to the countdown.
    pub fn phantom_delay(&mut self, ticks: i32) {
        self.phantom_tick += ticks;
    }
    pub fn add(&mut self, ty: &str, key: &str, n: i32) {
        namespaced(ty, |ty| {
            let v = entry(entry(&mut self.values, ty), key);
            *v = v.saturating_add(n);
            self.events.push((ty.to_owned(), key.to_owned(), Some(n)));
            self.mark_dirty(ty, key);
        });
    }
    fn mark_dirty(&mut self, ty: &str, key: &str) {
        let keys = entry(&mut self.dirty, ty);
        if !keys.contains(key) {
            keys.insert(key.to_owned());
        }
    }
    pub fn get(&self, ty: &str, key: &str) -> i32 {
        self.values
            .get(&format!("minecraft:{ty}"))
            .and_then(|m| m.get(key))
            .copied()
            .unwrap_or(0)
    }
    pub fn reset(&mut self, ty: &str, key: &str) {
        if let Some(m) = self.values.get_mut(&format!("minecraft:{ty}")) {
            m.insert(key.to_owned(), 0);
        }
        self.mark_dirty(&format!("minecraft:{ty}"), key);
        self.events
            .push((format!("minecraft:{ty}"), key.to_owned(), None));
    }
    /// `ServerPlayer.die`'s `resetStat` of `TIME_SINCE_DEATH` and `TIME_SINCE_REST`.
    pub fn reset_death_timers(&mut self) {
        self.reset("custom", "minecraft:time_since_death");
        self.reset("custom", "minecraft:time_since_rest");
    }
    pub fn custom(&mut self, key: &str, n: i32) {
        namespaced(key, |key| self.add("custom", key, n));
    }
    /// `StatsCounter.setValue` for every statistic: `values` in place of the counters, those
    /// it lacks back to 0. Each changed one is reset then raised to its value for the
    /// scoreboard criteria to follow, and is dirty for the client's next request.
    pub fn replace(&mut self, values: BTreeMap<String, BTreeMap<String, i32>>) {
        let old = std::mem::take(&mut self.values);
        let value = |all: &BTreeMap<String, BTreeMap<String, i32>>, ty: &str, key: &str| {
            all.get(ty).and_then(|m| m.get(key)).copied().unwrap_or(0)
        };
        let keys: BTreeSet<(String, String)> = old
            .iter()
            .chain(&values)
            .flat_map(|(ty, m)| m.keys().map(move |key| (ty.clone(), key.clone())))
            .collect();
        for (ty, key) in keys {
            let new = value(&values, &ty, &key);
            if value(&old, &ty, &key) == new {
                continue;
            }
            self.events.push((ty.clone(), key.clone(), None));
            if new != 0 {
                self.events.push((ty.clone(), key.clone(), Some(new)));
            }
            self.mark_dirty(&ty, &key);
        }
        self.values = values;
    }
    /// `ServerStatsCounter.markAllDirty`: everything goes out on the next request.
    pub fn mark_all_dirty(&mut self) {
        self.dirty = self
            .values
            .iter()
            .map(|(ty, m)| (ty.clone(), m.keys().cloned().collect()))
            .collect();
    }
    /// `ServerStatsCounter.sendStats`' entries: the dirty statistics and
    /// their values, the set cleared.
    pub fn take_dirty(&mut self) -> Vec<(String, String, i32)> {
        std::mem::take(&mut self.dirty)
            .into_iter()
            .flat_map(|(ty, keys)| keys.into_iter().map(move |key| (ty.clone(), key)))
            .map(|(ty, key)| {
                let v = self.values.get(&ty).and_then(|m| m.get(&key)).copied();
                (ty, key, v.unwrap_or(0))
            })
            .collect()
    }
    /// `ServerStatsCounter.parseLocal`: the counters of a stats file.
    pub fn read(path: &Path) -> Self {
        let json = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .unwrap_or_default();
        Self::from_json(&json)
    }
    /// The counters of a stats file's JSON, or of a hand-over's record (`player-import`).
    pub fn from_json(json: &serde_json::Value) -> Self {
        let mut stats = Self {
            loaded: true,
            ..Self::default()
        };
        if let Some(all) = json.get("stats").and_then(|s| s.as_object()) {
            for (ty, entries) in all {
                let Some(entries) = entries.as_object() else {
                    continue;
                };
                let map = stats.values.entry(ty.clone()).or_default();
                for (key, v) in entries {
                    if let Some(n) = v.as_i64() {
                        map.insert(key.clone(), n as i32);
                    }
                }
            }
        }
        stats
    }
    /// `ServerStatsCounter.toJson`: what `write` puts in the file, and what a
    /// hand-over's record carries (`player-export`).
    pub fn to_json(&self) -> serde_json::Value {
        let stats: serde_json::Map<String, serde_json::Value> = self
            .values
            .iter()
            .filter(|(_, m)| !m.is_empty())
            .map(|(ty, m)| {
                let entries: serde_json::Map<String, serde_json::Value> =
                    m.iter().map(|(k, v)| (k.clone(), (*v).into())).collect();
                (ty.clone(), serde_json::Value::Object(entries))
            })
            .collect();
        serde_json::json!({
            "stats": stats,
            "DataVersion": horizon_world::DATA_VERSION,
        })
    }
    /// `ServerStatsCounter.save`: written whole (`save::write_durable`), so a crash or a full disk
    /// leaves the old file; a failure is logged.
    pub fn write(&self, path: &Path) {
        if let Err(e) = self.try_write(path) {
            log::warn!("could not save the statistics file {}: {e}", path.display());
        }
    }
    /// [`Self::write`], reporting why it failed. The old file is left as it was then.
    pub fn try_write(&self, path: &Path) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(&self.to_json()).map_err(std::io::Error::other)?;
        horizon_client::save::write_durable_io(path, text.as_bytes())
    }
    /// The counters of a snapshot a mod hands over (`player_set_statistics`), read strictly: a JSON
    /// object with a `stats` object of type objects of whole numbers that fit an `i32`. Anything else is
    /// None, so a half-valid snapshot never replaces live counters; `{"stats":{}}` is the explicit reset.
    pub fn from_snapshot(json: &serde_json::Value) -> Option<Self> {
        let all = json.get("stats")?.as_object()?;
        let mut stats = Self {
            loaded: true,
            ..Self::default()
        };
        for (ty, entries) in all {
            let map = stats.values.entry(ty.clone()).or_default();
            for (key, v) in entries.as_object()? {
                map.insert(key.clone(), i32::try_from(v.as_i64()?).ok()?);
            }
        }
        Some(stats)
    }
}
/// A player's advancements or statistics file written whole (`save::write_durable`): a crash
/// during the write leaves the old file, and a failure names the file.
pub fn write_player_json(path: &Path, json: &serde_json::Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(json).map_err(|e| e.to_string())?;
    horizon_client::save::write_durable(path, text.as_bytes())
        .map_err(|e| format!("{} is not saved: {e}", path.display()))
}
/// `players/<dir>/<uuid>.json` of a world (advancements, stats).
pub fn player_json(world: &Path, dir: &str, uuid: [i32; 4]) -> PathBuf {
    let hex: String = uuid.iter().map(|v| format!("{:08x}", *v as u32)).collect();
    let text = format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    );
    world.join("players").join(dir).join(format!("{text}.json"))
}
/// `FileUtil.isValidPathSegment`: not `.` or `..`, and only the characters
/// `[-._a-z0-9]`. A player name with a capital letter in it is refused like any
/// other segment vanilla would not write to disk.
fn valid_path_segment(segment: &str) -> bool {
    segment != "."
        && segment != ".."
        && segment.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'.' | b'_')
        })
}

/// `PlayerList.locateStatsFile`: the player's stats file, with the one an older
/// version named after the player (`stats/<name>.json`) moved to the uuid file the
/// first time it is asked for. A uuid file that is already there wins, and a move
/// that fails leaves the old one in place.
pub fn locate_player_json(world: &Path, dir: &str, uuid: [i32; 4], name: &str) -> PathBuf {
    let uuid_file = player_json(world, dir, uuid);
    if uuid_file.exists() {
        return uuid_file;
    }
    let file = format!("{name}.json");
    if !valid_path_segment(&file) {
        return uuid_file;
    }
    let named = uuid_file.with_file_name(&file);
    if !named.is_file() {
        return uuid_file;
    }
    match std::fs::rename(&named, &uuid_file) {
        Ok(()) => uuid_file,
        Err(e) => {
            log::warn!(
                "Horizon: failed to move the {} file {} to {}: {e}",
                dir,
                named.display(),
                uuid_file.display()
            );
            named
        }
    }
}

/// `1 + random.nextInt(difficulty.getId() + 1)` of `PhantomSpawner.tick`: Peaceful still draws.
pub fn phantom_group_size(random: &mut impl RandomSource, difficulty_id: i32) -> i32 {
    1 + random.next_int_bound(difficulty_id + 1).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `PlayerList.locateStatsFile`: a stats file an older version named after the player is moved
    /// to the uuid file the first time it is asked for; a uuid file that is already there wins, and
    /// a name vanilla would not write as a path segment is not looked for at all.
    #[test]
    fn an_old_stats_file_named_after_the_player_moves_to_its_uuid() {
        let world = std::env::temp_dir().join(format!(
            "horizon-stats-file-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let stats = world.join("players").join("stats");
        std::fs::create_dir_all(&stats).unwrap();
        let uuid = [1, 2, 3, 4];
        let located = player_json(&world, "stats", uuid);
        // Nothing to move: the uuid name is the answer, and looking writes nothing.
        assert_eq!(locate_player_json(&world, "stats", uuid, "notch"), located);
        assert!(!located.exists(), "looking for the file writes none");
        // The old name-based file is moved onto the uuid name, and not left behind.
        let old = stats.join("notch.json");
        std::fs::write(&old, "{}").unwrap();
        assert_eq!(locate_player_json(&world, "stats", uuid, "notch"), located);
        assert!(located.is_file(), "the old file landed on the uuid name");
        assert!(!old.exists(), "and is not left behind");
        // A uuid file that exists is the answer, whether or not an old one does.
        std::fs::write(&old, "{}").unwrap();
        assert_eq!(locate_player_json(&world, "stats", uuid, "notch"), located);
        assert!(old.is_file(), "the old file is left alone");
        // FileUtil.isValidPathSegment on the file name: capitals and separators are not a segment,
        // while a lowercase name with dashes, dots or digits is.
        for name in ["Notch", "no/tch", "no tch"] {
            assert!(
                !valid_path_segment(&format!("{name}.json")),
                "{name:?} is not a path segment"
            );
        }
        for name in ["notch", "a-b_c.1", "jeb_"] {
            assert!(
                valid_path_segment(&format!("{name}.json")),
                "{name:?} is a path segment"
            );
        }
        let _ = std::fs::remove_dir_all(&world);
    }

    /// An advancements file is replaced whole: when the new one cannot be written, the old one is
    /// still there as it was, and the failure names the file.
    #[test]
    fn a_failed_player_file_write_keeps_the_old_file_and_is_reported() {
        let world = std::env::temp_dir().join(format!(
            "horizon-player-json-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&world);
        let path = player_json(&world, "advancements", [1, 2, 3, 4]);
        let good = serde_json::json!({"minecraft:story/root": {"done": true}});
        write_player_json(&path, &good).unwrap();
        let before = std::fs::read_to_string(&path).unwrap();
        // A folder where the new file goes: it cannot be created.
        let file = path.file_name().unwrap().to_str().unwrap();
        std::fs::create_dir(path.with_file_name(format!("{file}_new"))).unwrap();
        let error = write_player_json(&path, &serde_json::json!({})).unwrap_err();
        assert!(error.contains(file), "{error}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        let _ = std::fs::remove_dir_all(&world);
    }

    /// ServerPlayer.die zeroes the two timers (and only them) and sends the new zero.
    #[test]
    fn death_resets_the_timers_since_death_and_rest() {
        let mut s = Stats::default();
        s.custom("time_since_death", 700);
        s.custom("time_since_rest", 900);
        s.custom("deaths", 2);
        s.events.clear();
        s.reset_death_timers();
        assert_eq!(s.get("custom", "minecraft:time_since_death"), 0);
        assert_eq!(s.get("custom", "minecraft:time_since_rest"), 0);
        assert_eq!(s.get("custom", "minecraft:deaths"), 2);
        assert_eq!(s.events.len(), 2);
    }

    #[test]
    fn phantom_group_size_follows_difficulty() {
        // Expected from java.util.Random(12345): 1 + nextInt(id + 1), six draws each.
        let expected: [(i32, [i32; 6]); 4] = [
            (0, [1, 1, 1, 1, 1, 1]),
            (1, [1, 2, 2, 2, 2, 1]),
            (2, [2, 2, 1, 1, 2, 2]),
            (3, [2, 3, 4, 4, 4, 1]),
        ];
        for (id, sizes) in expected {
            let mut r = horizon_core::random::LegacyRandom::new(12345);
            let got: Vec<i32> = (0..6).map(|_| phantom_group_size(&mut r, id)).collect();
            assert_eq!(got, sizes, "difficulty {id}");
        }
    }

    #[test]
    fn counters_events_and_dirty_entries() {
        let mut s = Stats::default();
        s.custom("play_time", 1);
        s.custom("play_time", 1);
        s.add("mined", "minecraft:stone", 2);
        s.custom(&"x".repeat(100), 3);
        assert_eq!(s.get("custom", "minecraft:play_time"), 2);
        assert_eq!(s.get("mined", "minecraft:stone"), 2);
        assert_eq!(
            s.get("custom", &format!("minecraft:{}", "x".repeat(100))),
            3
        );
        assert_eq!(s.events.len(), 4);
        assert_eq!(
            s.events[1],
            (
                "minecraft:custom".into(),
                "minecraft:play_time".into(),
                Some(1)
            )
        );
        let long = format!("minecraft:{}", "x".repeat(100));
        let sent = |v: &[(&str, &str, i32)]| -> Vec<(String, String, i32)> {
            v.iter()
                .map(|&(t, k, n)| (t.to_owned(), k.to_owned(), n))
                .collect()
        };
        assert_eq!(
            s.take_dirty(),
            sent(&[
                ("minecraft:custom", "minecraft:play_time", 2),
                ("minecraft:custom", &long, 3),
                ("minecraft:mined", "minecraft:stone", 2),
            ])
        );
        assert!(s.take_dirty().is_empty());
        s.reset("mined", "minecraft:stone");
        assert_eq!(
            s.take_dirty(),
            sent(&[("minecraft:mined", "minecraft:stone", 0)])
        );
        s.mark_all_dirty();
        assert_eq!(s.take_dirty().len(), 3);
    }

    /// A snapshot is read whole or not at all; the file the mod's counters are saved to is replaced
    /// atomically and a failed save leaves the old one.
    #[test]
    fn a_snapshot_is_validated_whole_and_a_save_is_atomic() {
        let ok = serde_json::json!({"stats": {"minecraft:mined": {"minecraft:stone": 3}}});
        let read = Stats::from_snapshot(&ok).unwrap();
        assert_eq!(read.values["minecraft:mined"]["minecraft:stone"], 3);
        assert!(
            Stats::from_snapshot(&serde_json::json!({"stats": {}}))
                .unwrap()
                .values
                .is_empty()
        );
        for bad in [
            serde_json::json!({}),
            serde_json::json!({"DataVersion": 1}),
            serde_json::json!({"stats": []}),
            serde_json::json!({"stats": {"minecraft:mined": 5}}),
            serde_json::json!({"stats": {"minecraft:mined": {"minecraft:stone": "3"}}}),
            serde_json::json!({"stats": {"minecraft:mined": {"minecraft:stone": 2_147_483_648_i64}}}),
            serde_json::json!({"stats": {"minecraft:mined": {"minecraft:stone": 1.5}}}),
        ] {
            assert!(Stats::from_snapshot(&bad).is_none(), "{bad}");
        }
        let dir = std::env::temp_dir().join(format!("stats-atomic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("p.json");
        read.try_write(&path).unwrap();
        assert!(!dir.join("p.json_new").exists());
        let before = std::fs::read_to_string(&path).unwrap();
        // the temporary file cannot be made (its name is a directory): the old file stays whole
        std::fs::create_dir(dir.join("p.json_new")).unwrap();
        let error = Stats::default().try_write(&path).unwrap_err();
        // The operating system's own error, not one rebuilt from its text.
        assert_ne!(error.kind(), std::io::ErrorKind::Other, "{error}");
        assert!(error.raw_os_error().is_some(), "{error}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
