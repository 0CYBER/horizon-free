//! `sounds.json` sound events as vanilla's `SoundManager.Preparations` builds
//! them: `WeighedSoundEvents` holding files and references to other events
//! (whose weight is the referenced event's total weight, resolved lazily), the
//! `replace` and `subtitle` fields, files that do not exist being dropped, and
//! `WeighedSoundEvents.getSound` drawing `nextInt(totalWeight)` from the sound
//! instance's random.
use horizon_assets::resource_source::ResourceSource;
use horizon_core::random::RandomSource;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// `SoundManager.EMPTY_SOUND_LOCATION`.
pub const EMPTY: &str = "minecraft:empty";
/// `SoundManager.INTENTIONALLY_EMPTY_SOUND_LOCATION`.
pub const INTENTIONALLY_EMPTY: &str = "minecraft:intentionally_empty";

/// `SampledFloat` as sounds use it: constants and `MultipliedFloats`.
#[derive(Clone, Debug, PartialEq)]
pub enum Sampled {
    Constant(f32),
    Product(Vec<Sampled>),
}
impl Sampled {
    /// `SampledFloat.sample`: constants draw nothing (sounds.json only has
    /// constants, but the random stays part of the call as in vanilla).
    #[allow(clippy::only_used_in_recursion)]
    pub fn sample(&self, random: &mut impl RandomSource) -> f32 {
        match self {
            Self::Constant(v) => *v,
            Self::Product(values) => {
                let mut result = 1.0f32;
                for v in values {
                    result *= v.sample(random);
                }
                result
            }
        }
    }
    /// `MultipliedFloats(self, other)`, kept flat: a product of products lists their factors
    /// in the same order instead of nesting, so a long chain of references neither clones,
    /// samples nor drops through one stack frame per level. `sample` multiplies left to right
    /// from 1, so the flat list gives the same result as the nested one, bit for bit.
    fn times(self, other: &Sampled) -> Sampled {
        let mut factors = match self {
            Self::Product(values) => values,
            single => vec![single],
        };
        match other {
            Self::Product(values) => factors.extend(values.iter().cloned()),
            single => factors.push(single.clone()),
        }
        Self::Product(factors)
    }
}
/// `Sound`.
#[derive(Clone, Debug, PartialEq)]
pub struct Sound {
    /// The file identifier (`namespace:path`, under `sounds/` with `.ogg`).
    pub location: String,
    pub volume: Sampled,
    pub pitch: Sampled,
    pub weight: i32,
    /// `Sound.Type.SOUND_EVENT` (a reference to another event).
    pub event: bool,
    pub stream: bool,
    pub preload: bool,
    pub attenuation_distance: i32,
    /// The `EMPTY_SOUND` / `INTENTIONALLY_EMPTY_SOUND` singletons (vanilla
    /// compares them by identity).
    pub sentinel: Sentinel,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sentinel {
    #[default]
    None,
    Empty,
    IntentionallyEmpty,
}
impl Sound {
    fn file(location: &str) -> Self {
        Self {
            location: location.to_owned(),
            volume: Sampled::Constant(1.0),
            pitch: Sampled::Constant(1.0),
            weight: 1,
            event: false,
            stream: false,
            preload: false,
            attenuation_distance: 16,
            sentinel: Sentinel::None,
        }
    }
    /// `SoundManager.EMPTY_SOUND`.
    pub fn empty() -> Self {
        Self {
            sentinel: Sentinel::Empty,
            ..Self::file(EMPTY)
        }
    }
    /// `SoundManager.INTENTIONALLY_EMPTY_SOUND`.
    pub fn intentionally_empty() -> Self {
        Self {
            sentinel: Sentinel::IntentionallyEmpty,
            ..Self::file(INTENTIONALLY_EMPTY)
        }
    }
    /// `Sound.getAttenuationDistance(volume)`.
    pub fn attenuation_distance(&self, volume: f32) -> f32 {
        volume.max(1.0) * self.attenuation_distance as f32
    }
    /// `Sound.getPath`: the resource path (`namespace`, `sounds/<path>.ogg`).
    pub fn path(&self) -> (&str, String) {
        let (namespace, path) = split(&self.location);
        (namespace, format!("sounds/{path}.ogg"))
    }
}
/// `WeighedSoundEvents`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WeighedSoundEvents {
    /// Files, or references to events (`Sound.event`).
    pub sounds: Vec<Sound>,
    /// The subtitle translation key.
    pub subtitle: Option<String>,
}
/// A `sounds.json` entry: event, `replace`, subtitle and sounds.
type Registration = (String, bool, Option<String>, Vec<Sound>);
/// Parse a `sounds.json`; `None` when Gson would reject the whole file (a
/// malformed entry anywhere in it).
fn parse_file(namespace: &str, text: &[u8]) -> Option<Vec<Registration>> {
    let Ok(Value::Object(map)) = serde_json::from_slice::<Value>(text) else {
        return None;
    };
    let mut parsed = Vec::new();
    for (name, def) in &map {
        let Value::Object(object) = def else {
            return None;
        };
        let replace = json_bool(object.get("replace").unwrap_or(&Value::Null), false);
        let subtitle = match object.get("subtitle") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(_) => return None,
        };
        let mut sounds = Vec::new();
        if let Some(list) = object.get("sounds") {
            let Value::Array(list) = list else {
                return None;
            };
            for s in list {
                sounds.push(parse_sound(s)?);
            }
        }
        parsed.push((format!("{namespace}:{name}"), replace, subtitle, sounds));
    }
    Some(parsed)
}
/// Whether vanilla's `SoundManager` reads this `sounds.json` at all.
pub fn is_valid_file(text: &[u8]) -> bool {
    parse_file("minecraft", text).is_some()
}
/// The sound event registry (`SoundManager.registry`).
#[derive(Clone, Debug, Default)]
pub struct SoundRegistry {
    events: HashMap<String, WeighedSoundEvents>,
}
/// `Identifier.parse` with the default namespace.
fn split(id: &str) -> (&str, &str) {
    match id.split_once(':') {
        Some((namespace, path)) => (
            if namespace.is_empty() {
                "minecraft"
            } else {
                namespace
            },
            path,
        ),
        None => ("minecraft", id),
    }
}
pub fn normalize(id: &str) -> String {
    let (namespace, path) = split(id);
    format!("{namespace}:{path}")
}
/// [`normalize`] without allocating when `id` already carries a non-empty namespace.
pub(crate) fn normalize_cow(id: &str) -> std::borrow::Cow<'_, str> {
    match id.split_once(':') {
        Some((namespace, _)) if !namespace.is_empty() => std::borrow::Cow::Borrowed(id),
        _ => std::borrow::Cow::Owned(normalize(id)),
    }
}
/// `GsonHelper.getAsFloat`: Gson parses the number's text as a float.
fn json_float(v: &Value, fallback: f32) -> f32 {
    match v {
        Value::Number(n) => n.to_string().parse().unwrap_or(fallback),
        Value::String(s) => s.parse().unwrap_or(fallback),
        _ => fallback,
    }
}
fn json_int(v: &Value, fallback: i32) -> i32 {
    match v {
        Value::Number(n) => n
            .as_i64()
            .map(|i| i as i32)
            .or_else(|| n.as_f64().map(|f| f as i32))
            .unwrap_or(fallback),
        Value::String(s) => s.parse().unwrap_or(fallback),
        _ => fallback,
    }
}
pub(crate) fn json_bool(v: &Value, fallback: bool) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::String(s) => s == "true",
        _ => fallback,
    }
}
/// `SoundEventRegistrationSerializer.getSound`; None for an invalid entry
/// (vanilla rejects the whole file).
fn parse_sound(v: &Value) -> Option<Sound> {
    if let Value::String(name) = v {
        return Some(Sound::file(&normalize(name)));
    }
    let name = v.get("name")?.as_str()?;
    let event = match v.get("type") {
        None => false,
        Some(t) => match t.as_str()? {
            "file" => false,
            "event" => true,
            _ => return None,
        },
    };
    let volume = json_float(v.get("volume").unwrap_or(&Value::Null), 1.0);
    let pitch = json_float(v.get("pitch").unwrap_or(&Value::Null), 1.0);
    let weight = json_int(v.get("weight").unwrap_or(&Value::Null), 1);
    if !(volume > 0.0 && pitch > 0.0 && weight > 0) {
        return None;
    }
    Some(Sound {
        location: normalize(name),
        volume: Sampled::Constant(volume),
        pitch: Sampled::Constant(pitch),
        weight,
        event,
        stream: json_bool(v.get("stream").unwrap_or(&Value::Null), false),
        preload: json_bool(v.get("preload").unwrap_or(&Value::Null), false),
        attenuation_distance: json_int(v.get("attenuation_distance").unwrap_or(&Value::Null), 16),
        sentinel: Sentinel::None,
    })
}
/// The files under a namespace's `sounds/`, as `sounds/<path>` with `/`.
fn sound_files(root: &ResourceSource, namespace: &str) -> HashSet<String> {
    let prefix = format!("assets/{namespace}/");
    root.list(&format!("{prefix}sounds/"))
        .into_iter()
        .filter_map(|name| name.strip_prefix(&prefix).map(str::to_owned))
        .collect()
}
impl SoundRegistry {
    /// Every namespace's sounds.json, validated against resolved sound resources.
    pub fn load(root: &ResourceSource) -> Self {
        let namespaces = root.namespaces("assets");
        let files: HashMap<&str, HashSet<String>> = namespaces
            .iter()
            .map(|ns| (ns.as_str(), sound_files(root, ns)))
            .collect();
        let exists = |ns: &str, path: &str| files.get(ns).is_some_and(|f| f.contains(path));
        let mut registry = Self::default();
        for namespace in &namespaces {
            let Ok(text) = root.read(&format!("assets/{namespace}/sounds.json")) else {
                continue;
            };
            registry.add_file(namespace, &text, &exists);
        }
        for entry in registry.events.values_mut() {
            entry.sounds.shrink_to_fit();
        }
        registry.events.shrink_to_fit();
        registry
    }
    /// No sound event at all: the resources hold no `sounds.json` (sounds not
    /// imported from the player's launcher when the game was prepared).
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
    /// One `sounds.json` (`Preparations.handleRegistration` for each entry).
    /// `exists(namespace, path)` says whether a sound file is present.
    pub fn add_file(&mut self, namespace: &str, text: &[u8], exists: &dyn Fn(&str, &str) -> bool) {
        let Some(parsed) = parse_file(namespace, text) else {
            return;
        };
        for (location, replace, subtitle, sounds) in parsed {
            let entry = match self.events.get_mut(&location) {
                Some(entry) if !replace => entry,
                _ => {
                    self.events.insert(
                        location.clone(),
                        WeighedSoundEvents {
                            sounds: Vec::new(),
                            subtitle,
                        },
                    );
                    self.events.get_mut(&location).expect("inserted")
                }
            };
            for sound in sounds {
                if !sound.event {
                    let (ns, path) = sound.path();
                    if !exists(ns, &path) {
                        continue;
                    }
                }
                entry.sounds.push(sound);
            }
        }
    }
    pub fn get(&self, event: &str) -> Option<&WeighedSoundEvents> {
        self.events.get(event)
    }
    pub fn contains(&self, event: &str) -> bool {
        self.events.contains_key(event)
    }
    /// Every registered event id.
    pub fn names(&self) -> impl Iterator<Item = &String> {
        self.events.keys()
    }
    /// `Weighted.getWeight` of one entry, with the sounds `refused` names left
    /// out of every pool (the Music page's off tracks). A reference weighs what
    /// its event weighs, and nothing when that event leads into a cycle; `weigh`
    /// has worked the events out before.
    fn entry_weight(
        &self,
        sound: &Sound,
        lookup: &Lookup<'_>,
        refused: &dyn Fn(&str) -> bool,
    ) -> i32 {
        if !sound.event {
            return if refused(&sound.location) {
                0
            } else {
                sound.weight
            };
        }
        match lookup.weights.get(sound.location.as_str()) {
            Some(Weight::Done {
                weight,
                cyclic: false,
            }) => *weight,
            _ => 0,
        }
    }
    /// `WeighedSoundEvents.getWeight` of `root` and of every event it references,
    /// each worked out once, depth first without recursion. An event that can reach
    /// itself again through references (vanilla overflows its stack on one) leads
    /// into a cycle, and so does every event that references one: a reference to such
    /// an event weighs nothing, wherever it is reached from.
    fn weigh<'a>(&'a self, root: &str, lookup: &mut Lookup<'a>, refused: &dyn Fn(&str) -> bool) {
        struct Frame<'a> {
            location: &'a str,
            event: &'a WeighedSoundEvents,
            next: usize,
            weight: i32,
            cyclic: bool,
        }
        let Some((location, event)) = self.events.get_key_value(root) else {
            return;
        };
        if lookup.weights.contains_key(location.as_str()) {
            return;
        }
        lookup.weights.insert(location, Weight::Open);
        let mut stack = vec![Frame {
            location,
            event,
            next: 0,
            weight: 0,
            cyclic: false,
        }];
        while let Some(top) = stack.last_mut() {
            let event: &'a WeighedSoundEvents = top.event;
            let Some(sound) = event.sounds.get(top.next) else {
                let done = stack.pop().expect("a frame on top");
                lookup.weights.insert(
                    done.location,
                    Weight::Done {
                        weight: done.weight,
                        cyclic: done.cyclic,
                    },
                );
                if let Some(parent) = stack.last_mut() {
                    if done.cyclic {
                        parent.cyclic = true;
                    } else {
                        parent.weight = parent.weight.wrapping_add(done.weight);
                    }
                }
                continue;
            };
            top.next += 1;
            if !sound.event {
                if !refused(&sound.location) {
                    top.weight = top.weight.wrapping_add(sound.weight);
                }
                continue;
            }
            match lookup.weights.get(sound.location.as_str()) {
                Some(Weight::Open | Weight::Done { cyclic: true, .. }) => top.cyclic = true,
                Some(Weight::Done { weight, .. }) => top.weight = top.weight.wrapping_add(*weight),
                None => {
                    // An unregistered event weighs nothing.
                    if let Some((location, event)) = self.events.get_key_value(&sound.location) {
                        lookup.weights.insert(location, Weight::Open);
                        stack.push(Frame {
                            location,
                            event,
                            next: 0,
                            weight: 0,
                            cyclic: false,
                        });
                    }
                }
            }
        }
    }
    /// `WeighedSoundEvents.getWeight`.
    pub fn weight(&self, event: &str) -> i32 {
        let mut lookup = Lookup::default();
        self.weigh(event, &mut lookup, &|_| false);
        match lookup.weights.get(event) {
            Some(Weight::Done { weight, .. }) => *weight,
            _ => 0,
        }
    }
    /// `WeighedSoundEvents.getSound`: None when the event is not registered.
    pub fn sound(&self, event: &str, random: &mut impl RandomSource) -> Option<Sound> {
        self.sound_except(event, random, &|_| false)
    }
    /// `WeighedSoundEvents.getSound` with the sounds `refused` names left out
    /// of every pool: `Sound::empty()` when the event has nothing left to
    /// play (all of its tracks are off).
    pub fn sound_except(
        &self,
        event: &str,
        random: &mut impl RandomSource,
        refused: &dyn Fn(&str) -> bool,
    ) -> Option<Sound> {
        let e = self.events.get(event)?;
        let mut lookup = Lookup::default();
        self.weigh(event, &mut lookup, refused);
        Some(self.pick(e, random, &lookup, refused))
    }
    /// The sound files an event names itself, in `sounds.json` order: the
    /// rows of the Music page. A `"type": "event"` reference is not followed
    /// here: the event it names lists its own tracks.
    pub fn tracks(&self, event: &str) -> Vec<String> {
        let Some(e) = self.events.get(event) else {
            return Vec::new();
        };
        let mut tracks: Vec<String> = Vec::new();
        for sound in &e.sounds {
            if !sound.event && !tracks.contains(&sound.location) {
                tracks.push(sound.location.clone());
            }
        }
        tracks
    }
    /// `WeighedSoundEvents.getSound`: a draw in `event`'s pool, and for a reference
    /// a draw in the referenced event's, down to a sound file. A reference that leads
    /// into a cycle weighs nothing, so the walk ends.
    fn pick(
        &self,
        event: &WeighedSoundEvents,
        random: &mut impl RandomSource,
        lookup: &Lookup<'_>,
        refused: &dyn Fn(&str) -> bool,
    ) -> Sound {
        let mut references: Vec<&Sound> = Vec::new();
        let mut event = event;
        let mut picked = Sound::empty();
        'pools: loop {
            let weights: Vec<i32> = event
                .sounds
                .iter()
                .map(|s| self.entry_weight(s, lookup, refused))
                .collect();
            let weight = weights.iter().fold(0i32, |sum, w| sum.wrapping_add(*w));
            if event.sounds.is_empty() || weight == 0 {
                break;
            }
            let Ok(mut index) = random.next_int_bound(weight) else {
                break;
            };
            for (sound, entry_weight) in event.sounds.iter().zip(weights) {
                index -= entry_weight;
                if index < 0 {
                    if !sound.event {
                        picked = sound.clone();
                        break 'pools;
                    }
                    let Some(inner) = self.events.get(&sound.location) else {
                        break 'pools;
                    };
                    references.push(sound);
                    event = inner;
                    continue 'pools;
                }
            }
            break;
        }
        // The anonymous reference Weighted, innermost first: the referenced event's
        // sound with this entry's volume, pitch and stream folded in.
        for sound in references.into_iter().rev() {
            picked = Sound {
                location: picked.location,
                volume: picked.volume.times(&sound.volume),
                pitch: picked.pitch.times(&sound.pitch),
                weight: sound.weight,
                event: false,
                stream: picked.stream || sound.stream,
                preload: picked.preload,
                attenuation_distance: picked.attenuation_distance,
                sentinel: Sentinel::None,
            };
        }
        picked
    }
}

/// One sound lookup's memory: each event's weight, worked out once.
#[derive(Default)]
struct Lookup<'a> {
    weights: HashMap<&'a str, Weight>,
}

enum Weight {
    /// Being worked out: a reference to it closes a cycle.
    Open,
    Done {
        weight: i32,
        cyclic: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::random::LegacyRandom;
    use std::fs;

    #[test]
    fn resolved_sounds_validate_cross_namespace_files_without_directories() {
        use horizon_assets::resource_source::Resource;
        let source = ResourceSource::indexed(std::collections::BTreeMap::from([
            (
                "assets/example/sounds.json".into(),
                Resource::bytes(
                    br#"{"tone":{"sounds":["other:nested/tone","other:missing"]}}"#.to_vec(),
                ),
            ),
            (
                "assets/other/sounds/nested/tone.ogg".into(),
                Resource::bytes(Vec::new()),
            ),
        ]));
        let registry = SoundRegistry::load(&source);
        let sounds = &registry.get("example:tone").unwrap().sounds;
        assert_eq!(sounds.len(), 1);
        assert_eq!(sounds[0].location, "other:nested/tone");
    }

    #[test]
    fn load_keeps_the_sound_files_a_root_holds() {
        let root = std::env::temp_dir().join(format!("horizon-sounds-{}", std::process::id()));
        let ns = root.join("assets/minecraft");
        fs::create_dir_all(ns.join("sounds/block/note")).unwrap();
        fs::write(ns.join("sounds/block/note/harp.ogg"), b"").unwrap();
        fs::write(
            ns.join("sounds.json"),
            br#"{"note": {"sounds": ["block/note/harp", "block/note/gone"]}}"#,
        )
        .unwrap();
        let r = SoundRegistry::load(&ResourceSource::directory(root.clone()));
        let _ = fs::remove_dir_all(&root);
        let sounds = &r.get("minecraft:note").unwrap().sounds;
        let kept: Vec<&str> = sounds.iter().map(|s| s.location.as_str()).collect();
        assert_eq!(kept, ["minecraft:block/note/harp"]);
    }

    #[test]
    fn references_weigh_as_much_as_their_event() {
        let mut r = SoundRegistry::default();
        let json = br#"{
            "a": {"sounds": ["x", "y", "z"]},
            "b": {"sounds": [{"name": "a", "type": "event", "weight": 9, "volume": 0.5}, "w"], "subtitle": "s.b"}
        }"#;
        r.add_file("minecraft", json, &|_, _| true);
        assert_eq!(r.weight("minecraft:b"), 4);
        let mut random = LegacyRandom::new(1);
        let mut seen = std::collections::HashSet::new();
        for _ in 0..64 {
            let s = r.sound("minecraft:b", &mut random).unwrap();
            if s.location != "minecraft:w" {
                assert_eq!(s.volume.sample(&mut random), 0.5);
            }
            seen.insert(s.location);
        }
        assert_eq!(seen.len(), 4);
        assert_eq!(
            r.get("minecraft:b").unwrap().subtitle.as_deref(),
            Some("s.b")
        );
    }

    #[test]
    fn a_nested_event_is_weighed_once_however_often_it_is_named() {
        // Level k names level k + 1 three times; the last level holds one file.
        let lookups = |levels: usize| {
            let mut json = serde_json::Map::new();
            for k in 0..levels {
                let next = serde_json::json!({"name": format!("l{}", k + 1), "type": "event"});
                json.insert(
                    format!("l{k}"),
                    serde_json::json!({"sounds": [next, next, next]}),
                );
            }
            json.insert(format!("l{levels}"), serde_json::json!({"sounds": ["x"]}));
            let mut r = SoundRegistry::default();
            r.add_file(
                "minecraft",
                serde_json::Value::Object(json).to_string().as_bytes(),
                &|_, _| true,
            );
            let asked = std::cell::Cell::new(0);
            let sound = r.sound_except("minecraft:l0", &mut LegacyRandom::new(1), &|_| {
                asked.set(asked.get() + 1);
                false
            });
            (
                r.weight("minecraft:l0"),
                sound.unwrap().location,
                asked.get(),
            )
        };
        let (weight, location, three) = lookups(3);
        assert_eq!((weight, location.as_str()), (27, "minecraft:x"));
        let (weight, _, four) = lookups(4);
        assert_eq!(weight, 81);
        // The file is looked at once per pass over its event, not once per path to it.
        assert_eq!(three, four);
        assert!(four <= 2, "{four}");
    }

    #[test]
    fn references_into_a_cycle_weigh_nothing_wherever_they_are_reached_from() {
        let mut r = SoundRegistry::default();
        // `a` names itself; `b` and `c` name each other; `d` reaches the `b`/`c` cycle; `e` is clean.
        let json = br#"{
            "a": {"sounds": [{"name": "a", "type": "event"}, {"name": "a", "type": "event"}, "x"]},
            "b": {"sounds": [{"name": "c", "type": "event"}]},
            "c": {"sounds": [{"name": "b", "type": "event"}, "y"]},
            "d": {"sounds": [{"name": "c", "type": "event"}, "w"]},
            "e": {"sounds": ["z"]},
            "root": {"sounds": [{"name": "a", "type": "event"}, {"name": "b", "type": "event"}, {"name": "d", "type": "event"}, {"name": "e", "type": "event"}, "v"]}
        }"#;
        r.add_file("minecraft", json, &|_, _| true);
        // An event in or before a cycle weighs its own files, whichever is asked for first.
        assert_eq!(r.weight("minecraft:a"), 1);
        assert_eq!(r.weight("minecraft:c"), 1);
        assert_eq!(r.weight("minecraft:d"), 1);
        // From outside, only the clean reference and the root's own file count.
        assert_eq!(r.weight("minecraft:root"), 2);
        let mut random = LegacyRandom::new(1);
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..64 {
            seen.insert(r.sound("minecraft:root", &mut random).unwrap().location);
        }
        let seen: Vec<&str> = seen.iter().map(String::as_str).collect();
        assert_eq!(seen, ["minecraft:v", "minecraft:z"]);
        // Inside the cycle, a pick never descends into it again.
        for _ in 0..64 {
            assert_eq!(
                r.sound("minecraft:c", &mut random).unwrap().location,
                "minecraft:y"
            );
        }
    }

    #[test]
    fn an_event_reached_at_two_depths_weighs_the_same_at_both() {
        // `root` names `s` directly and through a chain of 80 events; `s` names `t`, which holds `x`.
        let mut json = serde_json::Map::new();
        let reference = |name: &str| serde_json::json!({"name": name, "type": "event"});
        json.insert(
            "root".into(),
            serde_json::json!({"sounds": [reference("c0"), reference("s")]}),
        );
        for k in 0..80 {
            let next = if k == 79 {
                "s".to_string()
            } else {
                format!("c{}", k + 1)
            };
            json.insert(
                format!("c{k}"),
                serde_json::json!({"sounds": [reference(&next)]}),
            );
        }
        json.insert("s".into(), serde_json::json!({"sounds": [reference("t")]}));
        json.insert("t".into(), serde_json::json!({"sounds": ["x"]}));
        let mut r = SoundRegistry::default();
        r.add_file(
            "minecraft",
            serde_json::Value::Object(json).to_string().as_bytes(),
            &|_, _| true,
        );
        assert_eq!(r.weight("minecraft:root"), 2);
        let mut random = LegacyRandom::new(1);
        for _ in 0..16 {
            assert_eq!(
                r.sound("minecraft:root", &mut random).unwrap().location,
                "minecraft:x"
            );
        }
    }

    #[test]
    fn a_sound_reached_through_a_long_chain_clones_samples_and_drops() {
        // 100 000 events, each naming the next with its own volume and pitch. Nested
        // products would recurse once per level to clone, sample and drop the sound.
        let levels = 100_000;
        let mut json = serde_json::Map::new();
        for k in 0..levels {
            let next = serde_json::json!(
                {"name": format!("c{}", k + 1), "type": "event", "volume": 1.0, "pitch": 1.0}
            );
            json.insert(format!("c{k}"), serde_json::json!({"sounds": [next]}));
        }
        json.insert(
            format!("c{levels}"),
            serde_json::json!({"sounds": [{"name": "x", "volume": 0.5, "pitch": 2.0}]}),
        );
        let mut r = SoundRegistry::default();
        r.add_file(
            "minecraft",
            serde_json::Value::Object(json).to_string().as_bytes(),
            &|_, _| true,
        );
        let mut random = LegacyRandom::new(1);
        let sound = r.sound("minecraft:c0", &mut random).unwrap();
        assert_eq!(sound.location, "minecraft:x");
        let copy = sound.clone();
        drop(sound);
        assert_eq!(copy.volume.sample(&mut random), 0.5);
        assert_eq!(copy.pitch.sample(&mut random), 2.0);
    }

    #[test]
    fn missing_files_are_dropped_and_empty_events_give_the_empty_sound() {
        let mut r = SoundRegistry::default();
        r.add_file("minecraft", br#"{"a": {"sounds": ["gone"]}}"#, &|_, _| {
            false
        });
        assert_eq!(r.weight("minecraft:a"), 0);
        let mut random = LegacyRandom::new(1);
        assert_eq!(r.sound("minecraft:a", &mut random).unwrap(), Sound::empty());
        assert!(r.sound("minecraft:b", &mut random).is_none());
    }

    #[test]
    fn tracks_are_the_files_an_event_names_and_no_references() {
        let mut r = SoundRegistry::default();
        r.add_file(
            "minecraft",
            br#"{
                "music.game": {"sounds": ["music/a", "music/b", "music/a"]},
                "music.creative": {"sounds": [{"name": "music.game", "type": "event"}, "music/c"]}
            }"#,
            &|_, _| true,
        );
        assert_eq!(
            r.tracks("minecraft:music.game"),
            ["minecraft:music/a", "minecraft:music/b"]
        );
        // The referenced event lists its own tracks; this one only "music/c".
        assert_eq!(r.tracks("minecraft:music.creative"), ["minecraft:music/c"]);
        assert!(r.tracks("minecraft:music.absent").is_empty());
    }

    #[test]
    fn a_refused_sound_leaves_every_pool_it_is_in() {
        let mut r = SoundRegistry::default();
        r.add_file(
            "minecraft",
            br#"{
                "inner": {"sounds": ["x", "y"]},
                "outer": {"sounds": [{"name": "inner", "type": "event"}, "z"]}
            }"#,
            &|_, _| true,
        );
        let refused = |id: &str| id == "minecraft:x";
        assert_eq!(r.weight("minecraft:outer"), 3);
        let mut random = LegacyRandom::new(1);
        let mut seen = std::collections::HashSet::new();
        for _ in 0..256 {
            let s = r
                .sound_except("minecraft:outer", &mut random, &refused)
                .unwrap();
            assert_ne!(s.location, "minecraft:x");
            seen.insert(s.location);
        }
        // The nested event still contributes its remaining track.
        assert_eq!(
            seen,
            ["minecraft:y".to_owned(), "minecraft:z".to_owned()].into()
        );
        // Nothing left to play: the empty sound, not a missing one.
        let none = |_: &str| true;
        assert_eq!(
            r.sound_except("minecraft:outer", &mut random, &none)
                .unwrap(),
            Sound::empty()
        );
    }
}
