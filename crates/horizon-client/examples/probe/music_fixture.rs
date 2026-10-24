//! Replays `tools/test_music_parity.py` scripts through Horizon's
//! `MusicManager` over its `SoundEngine`: `music_fixture ROOT INPUT OUTPUT`.
use crate::audio_common as common;
use common::{Fake, describe, f, flush};
use horizon_client::music::{Music, MusicFrequency, MusicManager};
use horizon_client::sound_engine::{Bound, MUSIC, SoundEngine, TickContext};
use horizon_client::sound_events::SoundRegistry;
use std::io::Write;

fn music(spec: &str) -> Option<Music> {
    if spec == "-" {
        return None;
    }
    let p: Vec<&str> = spec.split(',').collect();
    Some(Music::new(
        &horizon_client::sound_events::normalize(p[0]),
        p[1].parse().expect("min"),
        p[2].parse().expect("max"),
        p[3] == "1",
    ))
}

pub fn main() {
    let args: Vec<String> = crate::args().collect();
    let mut engine = SoundEngine::new(SoundRegistry::load(std::path::Path::new(&args[1])));
    engine.listening = true;
    let mut backend = Fake::default();
    let mut manager = MusicManager::new(0);
    let (mut situation, mut volume) = (None::<Music>, 1.0f32);
    let input = std::fs::read_to_string(&args[2]).expect("input");
    let mut out = std::io::BufWriter::new(std::fs::File::create(&args[3]).expect("output"));
    for line in input.lines() {
        writeln!(out, "{line}").expect("write");
        let a: Vec<&str> = line.split(' ').collect();
        match a[0] {
            "start" => {
                if let Some(id) = manager.current_instance() {
                    engine.release(id);
                }
                manager = MusicManager::new(a[1].parse().expect("seed"));
                manager.frequency = MusicFrequency::parse(a[2]);
                engine.stop_all(&mut backend);
            }
            "situation" => {
                situation = music(a[1]);
                volume = f(a[2]);
            }
            "tick" => {
                let mut sounds = Bound {
                    engine: &mut engine,
                    backend: &mut backend,
                };
                manager.tick(situation.as_ref(), volume, a[1] == "1", &mut sounds);
                engine.tick(a[2] == "1", &TickContext::default(), &mut backend);
            }
            "finish" => {
                if let Some(c) = manager.current_instance().and_then(|id| engine.channel(id)) {
                    horizon_client::sound_engine::Backend::stop(&mut backend, c);
                }
            }
            "stop" => manager.stop_playing(
                situation.as_ref(),
                &mut Bound {
                    engine: &mut engine,
                    backend: &mut backend,
                },
            ),
            "stopmusic" => {
                let m = music(a[1]).expect("music");
                manager.stop_playing_music(
                    &m,
                    situation.as_ref(),
                    &mut Bound {
                        engine: &mut engine,
                        backend: &mut backend,
                    },
                );
            }
            "freq" => manager.set_frequency(MusicFrequency::parse(a[1]), situation.as_ref()),
            "toast" => manager.show_now_playing_toast_if_needed(),
            "volume" => {
                let source: usize = a[1].parse().expect("source");
                engine.source_volumes[source] = f(a[2]);
                engine.refresh_category_volume(source, &mut backend);
            }
            "stopall" => engine.stop_all(&mut backend),
            _ => panic!("{line}"),
        }
        writeln!(
            out,
            "  current={} delay={} gain={:08x} toastShown={} musicGain={:08x}",
            manager.current(&engine).unwrap_or("-"),
            manager.next_song_delay(),
            manager.current_gain().to_bits(),
            manager.toast_shown(),
            engine.category_gain(MUSIC).to_bits()
        )
        .expect("write");
        for show in std::mem::take(&mut manager.toast_requests) {
            writeln!(out, "  toast {}", if show { "show" } else { "hide" }).expect("write");
        }
        if let Some(id) = manager.current_instance() {
            writeln!(out, "{}", describe(&engine, &backend, "music", id)).expect("write");
        }
        flush(&mut engine, &mut out);
    }
}
