//! The in-place resource view Horizon resolves for a pack selection (`packs::resources`)
//! and what its loaders read from it, for VanillaPackResourcesOracle.
use horizon_client::{lang::Language, packs, sound_events::SoundRegistry};
use std::{fmt::Write, fs, path::PathBuf};
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100_0000_01b3)
    })
}
/// `namespace:path` to a file under the root's `assets/`.
fn asset(id: &str, prefix: &str, suffix: &str) -> String {
    let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
    format!("assets/{namespace}/{prefix}{path}{suffix}")
}
fn types(
    root: &horizon_assets::resource_source::ResourceSource,
    file: String,
    key: &str,
) -> String {
    let value: serde_json::Value = root
        .read(&file)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    value[key]
        .as_array()
        .into_iter()
        .flatten()
        .map(|v| {
            let t = v["type"].as_str().unwrap_or("?");
            format!(" {}", t.strip_prefix("minecraft:").unwrap_or(t))
        })
        .collect()
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = crate::args().collect();
    let input = fs::read_to_string(&args[1])?;
    let mut lines = input.lines();
    let root: Vec<String> = serde_json::from_str(lines.next().ok_or("empty input")?)?;
    if root.len() < 3 || root[0] != "root" {
        return Err("invalid resource-pack header".into());
    }
    let vanilla = PathBuf::from(&root[1]);
    let ids = root[3..].to_vec();
    let all = packs::discover(&vanilla, &PathBuf::from(&root[2]));
    let enabled = packs::enabled(&all, &ids);
    let resources = packs::resources(
        horizon_assets::resource_source::ResourceSource::directory(vanilla),
        &enabled,
    )?;
    let language = Language::load(&[&resources], "en_us");
    let sounds = SoundRegistry::load(&resources);
    let mut out = String::new();
    for line in lines {
        let (kind, arg) = line.split_once(' ').unwrap_or((line, ""));
        write!(out, "{line} =>")?;
        match kind {
            "file" => match resources.read(&asset(arg, "", "")) {
                Ok(bytes) => write!(out, " {:x}", fnv(&bytes))?,
                Err(_) => out.push_str(" missing"),
            },
            "lang" => write!(out, " {}", language.get(arg).unwrap_or(arg))?,
            "sound" => {
                let event = if arg.contains(':') {
                    arg.to_owned()
                } else {
                    format!("minecraft:{arg}")
                };
                match sounds.get(&event) {
                    None => out.push_str(" missing"),
                    Some(e) => {
                        write!(out, " subtitle={}", e.subtitle.as_deref().unwrap_or("none"))?;
                        for s in &e.sounds {
                            if s.event {
                                out.push_str(" event");
                            } else {
                                write!(out, " {}*{}", s.location, s.weight)?;
                            }
                        }
                    }
                }
            }
            "font" => out.push_str(&types(
                &resources,
                asset(arg, "font/", ".json"),
                "providers",
            )),
            "atlas" => out.push_str(&types(
                &resources,
                asset(arg, "atlases/", ".json"),
                "sources",
            )),
            _ => return Err(format!("unknown query {line}").into()),
        }
        out.push('\n');
    }
    fs::write(&args[2], out)?;
    Ok(())
}
