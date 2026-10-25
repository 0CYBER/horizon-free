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

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "horizon-launcher-registry-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
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
    // Modpacks under Modpacks; a kind this launcher can't install yet (a world) never.
    assert_eq!(found("", Kind::Modpack), ["cozy"]);
    assert!(found("skyblock", Kind::ResourcePack).is_empty());
    let p = hud(&index);
    assert_eq!(p.support(), super::super::Support::Works);
    assert_eq!(p.page, "https://playhorizon.dev/mods/coords_hud");
}

#[test]
fn installs_checked_turns_on_and_removes() {
    let index = sample();
    let home = scratch("install");
    let p = hud(&index);
    let done = install_from(&index, true, &p, &home, &|_, _| {}).unwrap();
    assert_eq!(done[0].file, "horizon-mods/coords_hud.hmod");
    assert_eq!(done[0].verdict.as_deref(), Some("Verified"));
    assert!(done[0].checksum.starts_with("SHA-256:e1135345"));
    assert!(home.join("horizon-mods/coords_hud.hmod").is_file());
    let list = fs::read_to_string(home.join("horizon-mods.txt")).unwrap();
    assert_eq!(list, "mod.coords_hud:true\n");
    assert_eq!(
        fs::read_dir(home.join("mod-browser/downloads"))
            .unwrap()
            .count(),
        0
    );
    // Installed already: nothing to do.
    let again = install_from(&index, true, &p, &home, &|_, _| {}).unwrap();
    assert_eq!(again, done);

    let mut revoked = index.clone();
    revoked.revoked.push(index::Revoked {
        id: "coords_hud".into(),
        sha256: None,
        reason: "Test.".into(),
        date: String::new(),
    });
    let said = turn_off_revoked(&revoked, &home);
    assert_eq!(said, ["Coordinates HUD was taken down and is off: Test."]);
    assert_eq!(
        fs::read_to_string(home.join("horizon-mods.txt")).unwrap(),
        ""
    );

    installed::remove(&home, "coords_hud").unwrap();
    assert!(!home.join("horizon-mods/coords_hud.hmod").exists());
    assert!(installed::Installed::load(&home).entries.is_empty());
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn refuses_a_changed_file_a_taken_id_and_local_files_from_the_real_registry() {
    let index = sample();
    let p = hud(&index);

    let home = scratch("tampered");
    let mut tampered = index.clone();
    let cozy = tampered.projects[1].versions[0].file.url.clone();
    tampered.projects[0].versions[0].file.url = cozy;
    let e = install_from(&tampered, true, &p, &home, &|_, _| {}).unwrap_err();
    assert!(e.contains("larger than its source says"), "{e}");
    assert!(!home.join("horizon-mods").exists());

    let e = install_from(&index, false, &p, &home, &|_, _| {}).unwrap_err();
    assert!(e.contains("file servers"), "{e}");

    fs::create_dir_all(home.join("horizon-mods/coords_hud")).unwrap();
    let e = install_from(&index, true, &p, &home, &|_, _| {}).unwrap_err();
    assert!(e.contains("already in horizon-mods"), "{e}");

    let mut taken_down = index.clone();
    taken_down.revoked.push(index::Revoked {
        id: "coords_hud".into(),
        sha256: Some(index.projects[0].versions[0].file.sha256.clone()),
        reason: "Test.".into(),
        date: String::new(),
    });
    let e = install_from(&taken_down, true, &p, &home, &|_, _| {}).unwrap_err();
    assert_eq!(e, "Coordinates HUD was taken down: Test.");
    fs::remove_dir_all(&home).unwrap();
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

#[test]
fn owner_links_only_take_github_logins() {
    for ok in ["Stink-O", "ludovic111", "a"] {
        assert!(github_login(ok), "{ok}");
    }
    for bad in [
        "",
        "-x",
        "x-",
        "a--b",
        "bad login",
        "a/b",
        "x?y=1",
        &"a".repeat(40),
    ] {
        assert!(!github_login(bad), "{bad}");
    }
}
