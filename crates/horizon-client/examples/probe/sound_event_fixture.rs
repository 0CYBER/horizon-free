//! Resolves sound instances through Horizon's sound event registry for
//! `tools/test_sound_event_parity.py`: `sound_event_fixture ROOT INPUT OUTPUT [NAMES]`.
use horizon_client::sound_engine::{Attenuation, MASTER, SoundEngine, SoundInstance};
use horizon_client::sound_events::{Sentinel, SoundRegistry};
use horizon_core::random::{LegacyRandom, RandomSource};
use std::io::Write;

fn bits(v: f32) -> String {
    format!("{:08x}", v.to_bits())
}

pub fn main() {
    let args: Vec<String> = crate::args().collect();
    let registry = SoundRegistry::load(std::path::Path::new(&args[1]));
    let mut engine = SoundEngine::new(registry);
    let input = std::fs::read_to_string(&args[2]).expect("input");
    let mut out = std::io::BufWriter::new(std::fs::File::create(&args[3]).expect("output"));
    let mut names: Vec<String> = engine.registry.names().cloned().collect();
    names.sort();
    for name in names {
        writeln!(out, "registered {name}").expect("write");
    }
    for line in input.lines() {
        let f: Vec<&str> = line.split(' ').collect();
        let seed: i64 = f[1].parse().expect("seed");
        let volume = f32::from_bits(u32::from_str_radix(f[2], 16).expect("volume"));
        let pitch = f32::from_bits(u32::from_str_radix(f[3], 16).expect("pitch"));
        let event = horizon_client::sound_events::normalize(f[0]);
        let instance = SoundInstance::new(
            &event,
            MASTER,
            volume,
            pitch,
            LegacyRandom::new(seed),
            false,
            0,
            Attenuation::Linear,
            [0.0; 3],
            false,
        );
        let id = engine.add(instance, true);
        engine.resolve(id);
        let weight = engine.registry.weight(&event);
        let i = engine.instance_mut(id).expect("instance");
        let mut s = line.to_owned();
        match &i.event {
            None => s.push_str(" event=none"),
            Some(subtitle) => s.push_str(&format!(
                " event weight={weight} subtitle={}",
                subtitle.as_deref().unwrap_or("-")
            )),
        }
        let v = i.get_volume();
        let p = i.get_pitch();
        let sound = i.sound.clone().expect("resolved");
        let (namespace, path) = sound.path();
        let sentinel = match sound.sentinel {
            Sentinel::None => "-",
            Sentinel::Empty => "empty",
            Sentinel::IntentionallyEmpty => "intentionally_empty",
        };
        s.push_str(&format!(
            " sound={} path={namespace}:{path} sentinel={sentinel} weight={} type={} stream={} preload={} attenuation={} volume={} pitch={} range={} next={}",
            sound.location,
            sound.weight,
            if sound.event { "SOUND_EVENT" } else { "FILE" },
            sound.stream,
            sound.preload,
            sound.attenuation_distance,
            bits(v),
            bits(p),
            bits(sound.attenuation_distance(v)),
            i.random.next_long()
        ));
        writeln!(out, "{s}").expect("write");
        engine.release(id);
    }
}
