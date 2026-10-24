//! The registry source against the sample index of `horizon_mods::index`
//! (`crates/horizon-mods/tests/index/`), its files served from disk.
use super::*;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../crates/horizon-mods/tests/index")
}

/// The sample index with its files on disk, as a local test registry lists them.
fn sample() -> Index {
    let text = fs::read_to_string(fixtures().join("index-v1.json")).unwrap();
    let mut index = Index::parse(&text).unwrap();
    for p in &mut index.projects {
        for v in &mut p.versions {
            let path = fixtures().join(&v.file.name);
            v.file.url = format!("file://{}", path.canonicalize().unwrap_or(path).display());
        }
    }
    index
}

fn hud(index: &Index) -> Project {
    project_of(index.project("coords_hud").unwrap()).unwrap()
}

#[test]
fn searches_by_kind_and_words() {
    let index = sample();
    let found = |text: &str, kind| -> Vec<String> {
        search(&index, &Query::new(text, kind))
            .into_iter()
            .map(|p| p.id)
            .collect()
    };
    assert_eq!(found("", Kind::Mod), ["coords_hud"]);
    assert_eq!(found("coordinates HUD", Kind::Mod), ["coords_hud"]);
    assert!(found("zoom", Kind::Mod).is_empty());
    // Kinds this launcher can't install yet (modpacks, a world) are never offered.
    assert!(found("", Kind::Modpack).is_empty());
    assert!(found("skyblock", Kind::ResourcePack).is_empty());
    let p = hud(&index);
    assert_eq!(p.support(), super::super::Support::Works);
    assert_eq!(p.page, "https://playhorizon.dev/mods/coords_hud");
}

#[test]
fn registry_versions_run_when_made_for_this_game() {
    use super::super::{pick, support, Support};
    let v = |game: &str| vec![game.to_owned()];
    let horizon = vec![LOADER.to_owned()];
    assert_eq!(support(Kind::Mod, &v(GAME), &horizon), Support::Works);
    assert!(!support(Kind::Mod, &v("26.2"), &horizon).installable());
    let index = sample();
    let p = index.project("coords_hud").unwrap();
    let versions: Vec<Version> = p
        .versions
        .iter()
        .map(|x| version_of(&index, p, x))
        .collect();
    assert_eq!(pick(Kind::Mod, &versions, false).unwrap().number, "0.1.0");
}
