//! Replays `tools/test_sound_engine_parity.py` scripts through Horizon's
//! `SoundEngine` with a backend recording what an OpenAL source would hold:
//! `sound_engine_fixture ROOT INPUT OUTPUT`.
use horizon_client::sound_engine::{
    Attenuation, Backend, ChannelId, InstanceId, SoundEngine, SoundInstance, TickContext,
};
use horizon_client::sound_events::{Sound, SoundRegistry};
use horizon_core::random::LegacyRandom;
use std::collections::{BTreeMap, HashMap};
use std::io::Write;

const INITIAL: u32 = 0x1011;
const PLAYING: u32 = 0x1012;
const PAUSED: u32 = 0x1013;
const STOPPED: u32 = 0x1014;

/// An OpenAL source's state as `alGetSource*` reports it.
#[derive(Clone)]
pub struct Source {
    pub state: u32,
    pub gain: f32,
    pub pitch: f32,
    pub position: [f32; 3],
    pub relative: bool,
    pub looping: bool,
    pub model: u32,
    pub max: f32,
    pub rolloff: f32,
    pub reference: f32,
}
impl Default for Source {
    fn default() -> Self {
        Self {
            state: INITIAL,
            gain: 1.0,
            pitch: 1.0,
            position: [0.0; 3],
            relative: false,
            looping: false,
            model: 0xD002,
            max: f32::MAX,
            rolloff: 1.0,
            reference: 1.0,
        }
    }
}
/// Buffers and streams are ready at once (silence in the oracle); stopping an
/// initial source leaves it initial, as in OpenAL Soft.
#[derive(Default)]
pub struct Fake {
    pub sources: HashMap<ChannelId, Source>,
    next: ChannelId,
}
impl Backend for Fake {
    fn create(&mut self, _streaming: bool) -> Option<ChannelId> {
        self.next += 1;
        self.sources.insert(self.next, Source::default());
        Some(self.next)
    }
    fn destroy(&mut self, channel: ChannelId) {
        self.sources.remove(&channel);
    }
    fn set_pitch(&mut self, c: ChannelId, pitch: f32) {
        self.sources.get_mut(&c).expect("source").pitch = pitch;
    }
    fn set_volume(&mut self, c: ChannelId, volume: f32) {
        self.sources.get_mut(&c).expect("source").gain = volume;
    }
    fn set_attenuation(&mut self, c: ChannelId, linear: Option<f32>) {
        let s = self.sources.get_mut(&c).expect("source");
        match linear {
            Some(max) => {
                s.model = 0xD003;
                s.max = max;
                s.rolloff = 1.0;
                s.reference = 0.0;
            }
            None => s.model = 0,
        }
    }
    fn set_looping(&mut self, c: ChannelId, looping: bool) {
        self.sources.get_mut(&c).expect("source").looping = looping;
    }
    fn set_position(&mut self, c: ChannelId, p: [f64; 3]) {
        self.sources.get_mut(&c).expect("source").position =
            [p[0] as f32, p[1] as f32, p[2] as f32];
    }
    fn set_relative(&mut self, c: ChannelId, relative: bool) {
        self.sources.get_mut(&c).expect("source").relative = relative;
    }
    fn start(&mut self, c: ChannelId, _sound: &Sound, _looping: bool) {
        self.sources.get_mut(&c).expect("source").state = PLAYING;
    }
    fn stop(&mut self, c: ChannelId) {
        if let Some(s) = self.sources.get_mut(&c)
            && s.state != INITIAL
        {
            s.state = STOPPED;
        }
    }
    fn pause(&mut self, c: ChannelId) {
        let s = self.sources.get_mut(&c).expect("source");
        if s.state == PLAYING {
            s.state = PAUSED;
        }
    }
    fn unpause(&mut self, c: ChannelId) {
        let s = self.sources.get_mut(&c).expect("source");
        if s.state == PAUSED {
            s.state = PLAYING;
        }
    }
    fn stopped(&mut self, c: ChannelId) -> bool {
        self.sources.get(&c).is_none_or(|s| s.state == STOPPED)
    }
}
fn bits(v: f32) -> String {
    format!("{:08x}", v.to_bits())
}
fn f(hex: &str) -> f32 {
    f32::from_bits(u32::from_str_radix(hex, 16).expect("float"))
}
fn d(hex: &str) -> f64 {
    f64::from_bits(u64::from_str_radix(hex, 16).expect("double"))
}
/// The shared per-step dump (instances, notifications, pool counts).
pub fn describe(engine: &SoundEngine, backend: &Fake, name: &str, id: InstanceId) -> String {
    let mut s = format!("  {name} active={}", engine.is_active(id));
    match engine.channel(id).and_then(|c| backend.sources.get(&c)) {
        None => s.push_str(" channel=-"),
        Some(src) => s.push_str(&format!(
            " state={:x} gain={} pitch={} pos={},{},{} rel={} loop={} model={:x} max={} rolloff={} ref={}",
            src.state,
            bits(src.gain),
            bits(src.pitch),
            bits(src.position[0]),
            bits(src.position[1]),
            bits(src.position[2]),
            u8::from(src.relative),
            u8::from(src.looping),
            src.model,
            bits(src.max),
            bits(src.rolloff),
            bits(src.reference)
        )),
    }
    s
}
pub fn flush(engine: &mut SoundEngine, out: &mut impl Write) {
    for n in std::mem::take(&mut engine.notifications) {
        writeln!(
            out,
            "notify {} {} {} {} {:016x} {:016x} {:016x}",
            n.identifier,
            n.subtitle.as_deref().unwrap_or("-"),
            bits(n.range),
            n.looping,
            n.position[0].to_bits(),
            n.position[1].to_bits(),
            n.position[2].to_bits()
        )
        .expect("write");
    }
    let [used, streaming] = engine.used_channels();
    writeln!(
        out,
        "  Sounds: {used}/{} + {streaming}/{}",
        engine.limits[0], engine.limits[1]
    )
    .expect("write");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut engine = SoundEngine::new(SoundRegistry::load(std::path::Path::new(&args[1])));
    engine.listening = true;
    let mut backend = Fake::default();
    let mut ctx = TickContext::default();
    let mut instances: BTreeMap<String, InstanceId> = BTreeMap::new();
    let input = std::fs::read_to_string(&args[2]).expect("input");
    let mut out = std::io::BufWriter::new(std::fs::File::create(&args[3]).expect("output"));
    for line in input.lines() {
        writeln!(out, "{line}").expect("write");
        let a: Vec<&str> = line.split(' ').collect();
        let source = |i: usize| a[i].parse::<usize>().expect("source");
        match a[0] {
            "play" => {
                let event = horizon_client::sound_events::normalize(a[3]);
                let random = LegacyRandom::new(a[14].parse().expect("seed"));
                let instance = match a[2] {
                    "simple" => SoundInstance::new(
                        &event,
                        source(4),
                        f(a[5]),
                        f(a[6]),
                        random,
                        a[10] == "1",
                        a[11].parse().expect("delay"),
                        if a[12] == "linear" {
                            Attenuation::Linear
                        } else {
                            Attenuation::None
                        },
                        [d(a[7]), d(a[8]), d(a[9])],
                        a[13] == "1",
                    ),
                    "loop" => SoundInstance::biome_loop(&event, random),
                    "uloop" => SoundInstance::under_liquid_loop(&event, random),
                    "usub" => SoundInstance::under_liquid_sub(&event, random),
                    _ => panic!("{line}"),
                };
                if let Some(old) = instances.remove(a[1]) {
                    engine.release(old);
                }
                let (id, result) = engine.play_new(instance, true, &mut backend);
                instances.insert(a[1].to_owned(), id);
                writeln!(out, "  result {}", result_name(result)).expect("write");
            }
            "replay" => {
                let result = engine.play(instances[a[1]], &mut backend);
                writeln!(out, "  result {}", result_name(result)).expect("write");
            }
            "queue" => engine.queue_ticking(instances[a[1]]),
            "delayed" => engine.play_delayed(instances[a[1]], a[2].parse().expect("delay")),
            "fade" => {
                let i = engine.instance_mut(instances[a[1]]).expect("instance");
                if a[2] == "in" {
                    i.fade_in();
                } else {
                    i.fade_out();
                }
            }
            "stop" => engine.stop(instances[a[1]], &mut backend),
            "stopmatch" => {
                let sound = (a[1] != "-").then(|| horizon_client::sound_events::normalize(a[1]));
                let src = (a[2] != "-").then(|| source(2));
                engine.stop_matching(sound.as_deref(), src, &mut backend);
            }
            "finish" => {
                if let Some(c) = engine.channel(instances[a[1]]) {
                    backend.stop(c);
                }
            }
            "gain" => engine.update_category_volume(source(1), f(a[2]), &mut backend),
            "volume" => engine.source_volumes[source(1)] = f(a[2]),
            "refresh" => engine.refresh_category_volume(source(1), &mut backend),
            "pause" => engine.pause_all_except(&[1, 10], &mut backend),
            "resume" => engine.resume(&mut backend),
            "player" => {
                ctx.player_underwater = a[1] == "1";
                ctx.player_removed = a[2] == "1";
            }
            "tick" => engine.tick(a[1] == "1", &ctx, &mut backend),
            "forget" => {
                if let Some(id) = instances.remove(a[1]) {
                    engine.release(id);
                }
            }
            _ => panic!("{line}"),
        }
        for (name, id) in &instances {
            writeln!(out, "{}", describe(&engine, &backend, name, *id)).expect("write");
        }
        flush(&mut engine, &mut out);
    }
}
fn result_name(r: horizon_client::sound_engine::PlayResult) -> &'static str {
    match r {
        horizon_client::sound_engine::PlayResult::Started => "STARTED",
        horizon_client::sound_engine::PlayResult::StartedSilently => "STARTED_SILENTLY",
        horizon_client::sound_engine::PlayResult::NotStarted => "NOT_STARTED",
    }
}
