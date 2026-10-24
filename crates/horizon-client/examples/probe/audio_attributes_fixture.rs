//! Audio environment attributes from Horizon's `MusicTables` and
//! `AmbientTables` for `tools/test_audio_attributes_parity.py`:
//! `audio_attributes_fixture ROOT INPUT OUTPUT`.
use horizon_client::ambience::AmbientTables;
use horizon_client::music::{Music, MusicTables};
use horizon_client::sound_events::normalize;
use std::io::Write;

fn music(m: Option<&Music>) -> String {
    m.map_or("-".to_owned(), |m| {
        format!(
            "{},{},{},{}",
            normalize(&m.sound),
            m.min_delay,
            m.max_delay,
            m.replace_current_music
        )
    })
}

pub fn main() {
    let args: Vec<String> = crate::args().collect();
    let root = std::path::Path::new(&args[1]);
    let tables = MusicTables::load(root);
    let ambient = AmbientTables::load(root);
    let input = std::fs::read_to_string(&args[2]).expect("input");
    let mut out = std::io::BufWriter::new(std::fs::File::create(&args[3]).expect("output"));
    for (name, m) in [
        ("MENU", Music::menu()),
        ("CREDITS", Music::credits()),
        ("END_BOSS", Music::end_boss()),
        ("CREATIVE", Music::creative()),
        ("END", Music::end()),
        ("UNDER_WATER", Music::under_water()),
        ("GAME", Music::game()),
    ] {
        writeln!(out, "{name} {}", music(Some(&m))).expect("write");
    }
    for line in input.lines() {
        let f: Vec<&str> = line.split(' ').collect();
        let (dimension, biome) = (f[0], f[1]);
        let bg = tables.background(dimension, biome);
        let mut s = format!(
            "{line} default={} creative={} underwater={}",
            music(bg.default.as_ref()),
            music(bg.creative.as_ref()),
            music(bg.underwater.as_ref())
        );
        for creative in [false, true] {
            for underwater in [false, true] {
                s.push_str(&format!(
                    " select={}",
                    music(bg.select(creative, underwater))
                ));
            }
        }
        s.push_str(&format!(
            " volume={:08x}",
            tables.volume(dimension, biome).to_bits()
        ));
        let a = ambient.get(dimension, biome);
        s.push_str(&format!(
            " loop={}",
            a.r#loop.as_deref().map_or("-".to_owned(), normalize)
        ));
        s.push_str(&format!(
            " mood={}",
            a.mood.as_ref().map_or("-".to_owned(), |m| format!(
                "{},{},{},{:016x}",
                normalize(&m.sound),
                m.tick_delay,
                m.block_search_extent,
                m.offset.to_bits()
            ))
        ));
        for (sound, chance) in &a.additions {
            s.push_str(&format!(
                " addition={},{:016x}",
                normalize(sound),
                chance.to_bits()
            ));
        }
        writeln!(out, "{s}").expect("write");
    }
}
