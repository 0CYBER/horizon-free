//! `getSituationalMusic`/`getMusicVolume` through Horizon's `Situation` for
//! `tools/test_situational_music_parity.py`:
//! `situational_music_fixture ROOT INPUT OUTPUT`.
use horizon_client::music::{BackgroundMusic, Music, MusicTables, Situation};
use horizon_client::sound_events::normalize;
use std::io::Write;

pub fn main() {
    let args: Vec<String> = crate::args().collect();
    let tables = MusicTables::load(std::path::Path::new(&args[1]));
    let input = std::fs::read_to_string(&args[2]).expect("input");
    let mut out = std::io::BufWriter::new(std::fs::File::create(&args[3]).expect("output"));
    for line in input.lines() {
        let f: Vec<&str> = line.split(' ').collect();
        let in_world = f[1] == "1";
        let (background, probe_volume) = if in_world {
            (tables.background(f[2], f[3]), tables.volume(f[2], f[3]))
        } else {
            (BackgroundMusic::default(), 1.0)
        };
        let situation = Situation {
            screen_music: (f[0] == "win").then(Music::credits),
            in_world,
            in_end: f[2] == "minecraft:the_end",
            boss_music: f[4].contains("music"),
            background: &background,
            probe_volume,
            creative: f[5] == "1" && f[6] == "1",
            underwater: f[7] == "1",
        };
        let music = situation.music().map_or("-".to_owned(), |m| {
            format!(
                "{},{},{},{}",
                normalize(&m.sound),
                m.min_delay,
                m.max_delay,
                m.replace_current_music
            )
        });
        writeln!(
            out,
            "{line} music={music} volume={:08x}",
            situation.volume().to_bits()
        )
        .expect("write");
    }
}
