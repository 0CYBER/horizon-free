//! The registry source against the sample index of `horizon_platform::mod_package::index`
//! (`crates/horizon-mod-package/tests/index/`), its files served from disk.
use super::*;
use crate::game_shader_api::tests::with_game;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../crates/horizon-mod-package/tests/index")
}

/// The sample index with its files on disk, as a local test registry lists them.
pub(super) fn sample() -> Index {
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

pub(super) fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "horizon-launcher-registry-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn hud(index: &Index) -> Project {
    project_of(index, index.project("coords_hud").unwrap()).unwrap()
}

/// Installs the sample's checked content mod, as a modpack does.
fn put(index: &Index, local: bool, home: &Path) -> Result<Vec<installed::Entry>, String> {
    let p = index.project("coords_hud").unwrap();
    install_version(index, local, p, &p.versions[0], home, &|_, _| {}).map(|e| vec![e])
}

#[test]
fn native_registry_entries_are_mods_that_say_they_run_native_code() {
    assert_eq!(kind_of(&index::Kind::NativeMod), Some(Kind::Mod));
    let mut index = sample();
    index.projects[0].kind = index::Kind::NativeMod;
    index.projects[0].versions[0].file.name = "coords_hud-0.1.0.hmod".into();
    let found = search(&index, &Query::new("", Kind::Mod));
    assert!(found.iter().any(|p| p.id == index.projects[0].id));
    let entry = |file: &str| {
        serde_json::from_value::<installed::Entry>(serde_json::json!({
            "source": "horizon", "project": "p", "slug": "p", "title": "P", "kind": "mod",
            "version": "1", "number": "1", "file": file, "checksum": "", "page": ""
        }))
        .unwrap()
    };
    assert_eq!(
        mod_id(&entry("horizon-native-mods/veinminer.hmod")),
        Some("veinminer")
    );
    assert_eq!(mod_id(&entry("horizon-mods/zoom.hmod")), Some("zoom"));
}

#[test]
fn only_horizons_own_projects_go_without_the_not_endorsed_line() {
    let mut p = sample().projects[0].clone();
    p.authors = vec!["Horizon".into()];
    p.source = "https://github.com/horizon-rust/horizon-mod-zoom".into();
    assert!(official(&p));
    p.source = "https://github.com/someone/horizon-mod-zoom".into();
    assert!(!official(&p), "the name alone doesn't make it official");
    p.authors = vec!["levzzz5154".into()];
    assert!(!official(&p));
}

#[test]
fn only_horizons_own_projects_say_they_come_from_horizon() {
    let mut index = sample();
    let id = index.projects[0].id.clone();
    index.projects[0].authors = vec!["levzzz5154".into()];
    index.projects[0].source = "https://github.com/levzzz5154/horizon-mod-zoom".into();
    assert!(!official_in(Some(&index), &id), "a contributor's project");
    index.projects[0].authors = vec!["Horizon".into()];
    index.projects[0].source = "https://github.com/horizon-rust/horizon-mod-zoom".into();
    assert!(official_in(Some(&index), &id));
    assert!(!official_in(Some(&index), "not_listed"));
    assert!(!official_in(None, &id), "no index read yet");
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
    assert_eq!(found("", Kind::Shader), ["glow"]);
    // The index's download counts, most downloaded first (of native mods: the browser lists them).
    let mut counted = index.clone();
    counted.projects[0].kind = index::Kind::NativeMod;
    counted.projects[0].versions[0].file.name = "coords_hud-0.1.0.hmod".into();
    counted.projects.push(index::Project {
        id: "zoom".into(),
        name: "Zoom".into(),
        downloads: 12_345,
        ..counted.projects[0].clone()
    });
    counted.projects[0].downloads = 80;
    let first = search(&counted, &Query::new("", Kind::Mod));
    let order: Vec<(&str, u64)> = first.iter().map(|p| (p.id.as_str(), p.downloads)).collect();
    assert_eq!(order, [("zoom", 12_345), ("coords_hud", 80)]);
    // Newest orders by the day a project's first version came out, Updated by its latest.
    let mut dated = counted.clone();
    let base = dated.projects[0].versions[0].clone();
    let version_on = |day: &str| index::Version {
        native_binaries: Vec::new(),
        published: day.into(),
        ..base.clone()
    };
    dated.projects[0].versions = vec![version_on("2026-09-01"), version_on("2026-01-01")];
    let zoom = dated.projects.iter().position(|p| p.id == "zoom").unwrap();
    dated.projects[zoom].versions = vec![version_on("2026-05-01")];
    let sorted = |sort| -> Vec<String> {
        let mut q = Query::new("", Kind::Mod);
        q.filters.sort = Some(sort);
        search(&dated, &q).into_iter().map(|p| p.id).collect()
    };
    assert_eq!(
        sorted(super::super::details::Sort::Newest),
        ["zoom", "coords_hud"]
    );
    assert_eq!(
        sorted(super::super::details::Sort::Updated),
        ["coords_hud", "zoom"]
    );
    assert_eq!(
        sorted(super::super::details::Sort::Downloads),
        ["zoom", "coords_hud"]
    );
    let p = hud(&index);
    assert_eq!(p.support(), super::super::Support::Works);
    assert_eq!(p.page, "https://playhorizon.dev/mods/coords_hud");
}

#[test]
fn update_rejects_a_changed_kind_without_removing_the_installed_file() {
    let mut index = sample();
    let home = scratch("changed-kind");
    put(&index, true, &home).unwrap();
    let project = index
        .projects
        .iter_mut()
        .find(|p| p.id == "coords_hud")
        .unwrap();
    project.kind = index::Kind::NativeMod;
    let path = home.join("horizon-mods/coords_hud.hmod");
    let before = fs::read(&path).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let p = index.project("coords_hud").unwrap();
    let error = install_version(&index, true, p, &p.versions[0], &home, &|_, _| {}).unwrap_err();
    assert!(error.contains("different installed kind"), "{error}");
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn installed_kind_comes_from_the_archive_not_its_folder() {
    for native in [false, true] {
        let mut index = sample();
        let home = scratch(if native {
            "native-in-content"
        } else {
            "content-in-native"
        });
        put(&index, true, &home).unwrap();
        let mut list = installed::Installed::load(&home);
        let old = home.join(&list.entries[0].file);
        let folder = if native {
            "horizon-mods"
        } else {
            "horizon-native-mods"
        };
        fs::create_dir_all(home.join(folder)).unwrap();
        let file = format!("{folder}/misplaced.hmod");
        let path = home.join(&file);
        if native {
            let manifest = format!(
                "id='coords_hud'\nname='HUD'\nversion='0.1.0'\napi='0.2'\ncapabilities=['registries']\n[native]\ngame='26.3'\nhorizon='{}'\nmixins='{}'\n",
                "a".repeat(40),
                "b".repeat(40)
            );
            let files: &[(&str, &[u8])] = &[
                ("Cargo.toml", b"[package]\nname='hud'\nversion='1'\n"),
                ("src/lib.rs", b"pub fn hooks() {}"),
            ];
            fs::write(
                &path,
                horizon_platform::mod_package::Package::write_files(&manifest, files).unwrap(),
            )
            .unwrap();
            index::native::Bundle::read(&path).unwrap();
            index
                .projects
                .iter_mut()
                .find(|p| p.id == "coords_hud")
                .unwrap()
                .kind = index::Kind::NativeMod;
        } else {
            fs::copy(&old, &path).unwrap();
        }
        list.entries[0].file = file;
        list.save(&home).unwrap();
        let p = index.project("coords_hud").unwrap();
        let result = install_version(&index, true, p, &p.versions[0], &home, &|_, _| {});
        assert!(result.is_ok(), "{result:?}");
        assert!(path.is_file());
        fs::remove_dir_all(home).unwrap();
    }
}

#[test]
fn corrupt_installed_archive_is_not_treated_as_content() {
    let index = sample();
    let home = scratch("corrupt-installed");
    put(&index, true, &home).unwrap();
    let path = home.join("horizon-mods/coords_hud.hmod");
    fs::write(&path, b"not a zip").unwrap();
    let before = fs::read(&path).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let p = index.project("coords_hud").unwrap();
    let error = install_version(&index, true, p, &p.versions[0], &home, &|_, _| {}).unwrap_err();
    assert!(error.contains("Cannot inspect installed"), "{error}");
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn removing_native_mod_does_not_remove_a_separate_content_file() {
    let index = sample();
    let home = scratch("native-remove");
    put(&index, true, &home).unwrap();
    let mut list = installed::Installed::load(&home);
    list.entries[0].file = "horizon-native-mods/coords_hud.hmod".into();
    list.save(&home).unwrap();
    fs::create_dir_all(home.join("horizon-native-mods")).unwrap();
    fs::write(home.join("horizon-native-mods/coords_hud.hmod"), "").unwrap();
    installed::remove(&home, "coords_hud").unwrap();
    assert!(home.join("horizon-mods/coords_hud.hmod").is_file());
    assert!(!home.join("horizon-native-mods/coords_hud.hmod").exists());
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn searches_by_category_and_tag() {
    let mut index = sample();
    let glow = index.projects.iter().position(|p| p.id == "glow").unwrap();
    index.projects[glow].categories = vec!["fantasy".into(), "bloom".into()];
    index.projects[glow].tags = vec!["sunset".into()];
    let found = |text: &str, categories: &[&str]| -> Vec<String> {
        let mut q = Query::new(text, Kind::Shader);
        q.filters.categories = categories.iter().map(|c| c.to_string()).collect();
        search(&index, &q).into_iter().map(|p| p.id).collect()
    };
    assert_eq!(found("", &["bloom"]), ["glow"]);
    assert_eq!(found("", &["bloom", "fantasy"]), ["glow"]);
    assert!(
        found("", &["bloom", "pbr"]).is_empty(),
        "every category asked"
    );
    assert_eq!(found("sunset", &[]), ["glow"], "tags are searched");
    assert!(found("dawn", &[]).is_empty());
    let tagged = |tags: &[&str]| -> Vec<String> {
        let mut q = Query::new("", Kind::Shader);
        q.filters.tags = tags.iter().map(|t| t.to_string()).collect();
        search(&index, &q).into_iter().map(|p| p.id).collect()
    };
    assert_eq!(tagged(&["sunset"]), ["glow"]);
    assert!(tagged(&["sunset", "dawn"]).is_empty(), "every tag asked");
    // The filter offers the kind's own list: a resolution for resource packs, not for shaders.
    let shader = Registry.tags(Kind::Shader).unwrap().categories;
    assert!(shader.iter().any(|c| c.name == "path-tracing"));
    let packs = Registry.tags(Kind::ResourcePack).unwrap().categories;
    assert!(packs.iter().any(|c| c.name == "16x"));
    assert!(!shader.iter().any(|c| c.name == "16x"));
}

#[test]
fn the_tag_filter_offers_the_kinds_tags_most_used_first() {
    let mut index = sample();
    let at = |index: &Index, id: &str| index.projects.iter().position(|p| p.id == id).unwrap();
    let glow = at(&index, "glow");
    index.projects[glow].tags = vec![
        "sunset".into(),
        "cozy".into(),
        "sunset".into(),
        "Not Plain".into(),
    ];
    assert_eq!(
        tags_of(&index, Kind::Shader),
        ["cozy", "sunset"],
        "ties by name"
    );
    let mut dusk = index.projects[glow].clone();
    dusk.id = "dusk".into();
    dusk.tags = vec!["sunset".into()];
    index.projects.push(dusk);
    let hud = at(&index, "coords_hud");
    index.projects[hud].tags = vec!["pvp".into()];
    assert_eq!(tags_of(&index, Kind::Shader), ["sunset", "cozy"]);
    assert_eq!(tags_of(&index, Kind::Mod), ["pvp"], "each kind its own");
    assert!(tags_of(&index, Kind::DataPack).is_empty());
}

/// A data pack zip with one function, and the registry project that lists it.
fn data_pack(index: &mut Index, dir: &Path) -> Project {
    use std::io::Write;
    let path = dir.join("lanterns-1.0.0.zip");
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
    for (name, text) in [
        (
            "pack.mcmeta",
            r#"{"pack": {"description": "Lanterns", "pack_format": 94}}"#,
        ),
        ("data/lanterns/function/light.mcfunction", "say lit"),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
    let base = index.project("glow").unwrap().clone();
    let mut version = base.versions[0].clone();
    version.version = "1.0.0".into();
    version.api = "94".into();
    version.side = None;
    version.capabilities = Default::default();
    version.requires.clear();
    version.file = index::File {
        name: "lanterns-1.0.0.zip".into(),
        url: format!("file://{}", path.display()),
        size: fs::metadata(&path).unwrap().len(),
        sha256: index::sha256_file(&path).unwrap(),
        signature: None,
    };
    index.projects.push(index::Project {
        id: "lanterns".into(),
        kind: index::Kind::DataPack,
        name: "Lanterns".into(),
        key: None,
        versions: vec![version],
        ..base
    });
    project_of(index, index.project("lanterns").unwrap()).unwrap()
}

#[test]
fn a_registry_data_pack_is_checked_then_goes_into_a_world() {
    let dir = scratch("datapack");
    let mut index = sample();
    let p = data_pack(&mut index, &dir);
    assert_eq!(p.kind, Kind::DataPack);
    assert_eq!(p.support(), super::super::Support::Works);
    let found: Vec<String> = search(&index, &Query::new("", Kind::DataPack))
        .into_iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(found, ["lanterns"]);
    // Not into the game folder: into a world.
    assert!(install_from(&index, true, &p, &dir.join("home"), &|_, _| {}).is_err());
    let (zip, number) = fetch_datapack_from(&index, true, &p, &dir.join("dl"), &|_| {}).unwrap();
    assert_eq!(number, "1.0.0");
    let world = dir.join("world");
    let pack = crate::datapacks::add(&world, &zip).unwrap();
    assert!(world.join("datapacks").join(&pack.file).is_file());

    // A file other than the one the index lists never gets there.
    let mut changed = index.clone();
    let at = changed.projects.len() - 1;
    changed.projects[at].versions[0].file.sha256 = "0".repeat(64);
    let e = fetch_datapack_from(&changed, true, &p, &dir.join("dl2"), &|_| {}).unwrap_err();
    assert!(e.contains("checksum") || e.contains("SHA-256"), "{e}");
    // Nor one taken down.
    index.revoked.push(index::Revoked {
        id: "lanterns".into(),
        sha256: None,
        reason: "Test.".into(),
        date: String::new(),
    });
    let e = fetch_datapack_from(&index, true, &p, &dir.join("dl3"), &|_| {}).unwrap_err();
    assert_eq!(e, "Lanterns was taken down: Test.");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn screenshots_are_the_gallery_from_the_registry_only() {
    let mut p = sample().projects[0].clone();
    assert!(gallery_of(&p).is_empty());
    let pages = "https://horizon-rust.github.io/horizon-mods/screenshots/coords_hud";
    p.screenshots = vec![
        format!("{pages}/b.png"),
        "https://example.com/tracker.png".into(),
        format!("{pages}/a.jpg"),
    ];
    let urls: Vec<String> = gallery_of(&p).into_iter().map(|i| i.url).collect();
    assert_eq!(urls, [format!("{pages}/b.png"), format!("{pages}/a.jpg")]);
}

#[test]
fn installs_combined_native_content_as_one_hmod() {
    use std::io::Write;
    let home = scratch("native-content-install");
    let archive = home.join("source.hmod");
    let a = "a".repeat(40);
    let b = "b".repeat(40);
    let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
    for (name, text) in [
        ("Cargo.toml", "[package]\nname='coords_hud'\nversion='0.1.0'\n".into()),
        ("src/lib.rs", "fn hook() {}".into()),
        ("horizon-mod.toml", format!("id='coords_hud'\nname='Sound proof'\nversion='0.1.0'\napi='0.2'\ncapabilities=['registries']\n[native]\ngame='{GAME}'\nhorizon='{a}'\nmixins='{b}'\n")),
        ("assets/coords_hud/sounds.json", "{}".into()),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
    let bytes = fs::read(&archive).unwrap();
    let mut index = sample();
    let p = &mut index.projects[0];
    p.kind = index::Kind::NativeMod;
    p.key = None;
    let v = &mut p.versions[0];
    v.api = format!("native-1/{a}/{b}");
    v.version = "0.1.0".into();
    v.game = GAME.into();
    v.side = None;
    v.requires.clear();
    v.capabilities = Default::default();
    v.file.name = "coords_hud.hmod".into();
    v.file.url = format!("file://{}", archive.display());
    v.file.size = bytes.len() as u64;
    v.file.sha256 = index::sha256(&bytes);
    v.file.signature = None;
    let p = &index.projects[0];
    let entry = install_version(&index, true, p, &p.versions[0], &home, &|_, _| {}).unwrap();
    assert_eq!(entry.file, "horizon-native-mods/coords_hud.hmod");
    assert_eq!(mod_id(&entry), Some("coords_hud"));
    let bundle = index::native::Bundle::read(&home.join(&entry.file)).unwrap();
    assert_eq!(bundle.files["assets/coords_hud/sounds.json"], b"{}");
    assert_eq!(crate::native_status::enabled(&home).len(), 1);
    installed::remove(&home, "coords_hud").unwrap();
    assert!(!home.join(entry.file).exists());
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn installs_checked_turns_on_and_removes() {
    let index = sample();
    let home = scratch("install");
    let p = hud(&index);
    let done = install_from(&index, true, &p, &home, &|_, _| {}).unwrap();
    assert_eq!(done[0].file, "horizon-mods/coords_hud.hmod");
    assert!(done[0].skipped.is_empty());
    assert_eq!(done[0].verdict.as_deref(), Some("Verified"));
    assert_eq!(
        done[0].checksum,
        "SHA-256:f891bde88f4b841fd35c4f3305bc8904b671ebe9d5ad28d5aecc57e8b23fc3ce"
    );
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
    let again = put(&index, true, &home).unwrap();
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
fn executable_content_is_refused_without_installing_or_skipping_code() {
    use std::io::Write;
    let home = scratch("executable");
    let path = home.join("executable.hmod");
    fs::copy(fixtures().join("coords_hud-0.1.0.hmod"), &path).unwrap();
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    let mut zip = zip::ZipWriter::new_append(file).unwrap();
    zip.start_file("mod.wasm", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"\0asm").unwrap();
    zip.finish().unwrap();
    let mut index = sample();
    let p = &mut index.projects[0];
    p.key = None;
    let v = &mut p.versions[0];
    v.file.url = format!("file://{}", path.display());
    v.file.size = fs::metadata(&path).unwrap().len();
    v.file.sha256 = index::sha256_file(&path).unwrap();
    v.file.signature = None;
    let e = put(&index, true, &home).unwrap_err();
    assert_eq!(e, "mod.wasm needs a [wasm] section");
    assert!(!home.join("horizon-mods/coords_hud.hmod").exists());
    assert!(installed::Installed::load(&home).entries.is_empty());
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn installs_a_shader_pack_into_shaderpacks() {
    let index = sample();
    let home = scratch("shader");
    let p = project_of(&index, index.project("glow").unwrap()).unwrap();
    assert_eq!(p.support(), super::super::Support::Works);
    let done = install_from(&index, true, &p, &home, &|_, _| {}).unwrap();
    assert_eq!(done[0].file, "shaderpacks/glow-1.0.0.hshader");
    assert_eq!(done[0].kind, Kind::Shader);
    assert!(home.join("shaderpacks/glow-1.0.0.hshader").is_file());
    // Not a mod: nothing turns on in horizon-mods.txt; the game's Shader Packs page picks it.
    assert!(!home.join("horizon-mods.txt").exists());

    let mut revoked = index.clone();
    revoked.revoked.push(index::Revoked {
        id: "glow".into(),
        sha256: None,
        reason: "Test.".into(),
        date: String::new(),
    });
    let said = turn_off_revoked(&revoked, &home);
    assert_eq!(said, ["Soft Glow was taken down: Test. Remove it."]);

    installed::remove(&home, "glow").unwrap();
    assert!(!home.join("shaderpacks/glow-1.0.0.hshader").exists());
    // A file of the same name the registry didn't put there is left alone.
    fs::write(home.join("shaderpacks/glow-1.0.0.hshader"), b"mine").unwrap();
    let e = install_from(&index, true, &p, &home, &|_, _| {}).unwrap_err();
    assert_eq!(
        e,
        "shaderpacks/glow-1.0.0.hshader is already there: move it away first."
    );
    fs::remove_dir_all(&home).unwrap();
}

/// The browser's versions of `id`, as `Registry::versions` gives them.
fn versions_of(index: &Index, id: &str) -> Vec<Version> {
    let p = index.project(id).unwrap();
    p.versions.iter().map(|v| version_of(index, p, v)).collect()
}

/// A shader pack whose every version is for a shader API the installed game doesn't serve is
/// marked, and neither picked nor downloaded (#17245).
#[test]
fn shader_packs_for_another_shader_api_are_marked_and_not_installed() {
    with_game(&["0.2"], marked_and_not_installed);
}

fn marked_and_not_installed() {
    let mut index = sample();
    let glow = index.projects.iter_mut().find(|p| p.id == "glow").unwrap();
    glow.versions[0].api = "0.1".into();
    let versions = versions_of(&index, "glow");
    assert_eq!(
        versions[0].blocked.as_deref(),
        Some("needs shader API 0.1; this Horizon serves 0.2")
    );
    assert!(super::super::pick(Kind::Shader, &versions, true).is_none());
    let p = index.project("glow").unwrap();
    assert_eq!(
        newest(&index, p).unwrap_err(),
        "Soft Glow needs shader API 0.1; this Horizon serves 0.2."
    );
    assert!(installable(&index, p).is_none());
    let home = scratch("shader-api");
    let project = project_of(&index, p).unwrap();
    // The card, the CLI and MCP give this answer. As everything that can't be installed here,
    // it is left out of the list, and listed last with `everything`.
    let marked =
        super::super::Support::No("Made for shader API 0.1: this Horizon serves 0.2.".into());
    assert_eq!(project.support(), marked);
    assert!(search(&index, &Query::new("", Kind::Shader)).is_empty());
    let everything = Query {
        everything: true,
        ..Query::new("", Kind::Shader)
    };
    let listed = search(&index, &everything);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].support(), marked);
    // Not counted as a mod for the earlier mod system by the note above the list.
    assert!(matches!(
        blocked(&index, p),
        Some(Blocked::OtherShaderApi(_))
    ));
    let e = install_from(&index, true, &project, &home, &|_, _| {}).unwrap_err();
    assert_eq!(
        e,
        "Soft Glow needs shader API 0.1; this Horizon serves 0.2."
    );
    assert!(!home.join("mod-browser").exists(), "nothing was downloaded");
    assert!(!home.join("shaderpacks").exists());
    fs::remove_dir_all(&home).unwrap();
}

/// A newer version for a shader API the installed game doesn't serve above one for a line it
/// does: the one that runs is picked, installed and updated to.
#[test]
fn a_shader_pack_offers_its_newest_version_for_this_shader_api() {
    with_game(&["0.2"], offers_its_newest_version_for_the_games_shader_api);
}

fn offers_its_newest_version_for_the_games_shader_api() {
    let mut index = sample();
    let glow = index.projects.iter_mut().find(|p| p.id == "glow").unwrap();
    let mut old_api = glow.versions[0].clone();
    old_api.version = "1.1.0".into();
    old_api.api = "0.1".into();
    old_api.file.sha256 = "1".repeat(64);
    glow.versions.insert(0, old_api);
    let versions = versions_of(&index, "glow");
    let picked = super::super::pick(Kind::Shader, &versions, true).unwrap();
    assert_eq!(picked.number, "1.0.0");
    let p = index.project("glow").unwrap();
    assert_eq!(newest(&index, p).unwrap().version, "1.0.0");
    assert_eq!(installable(&index, p).unwrap().version, "1.0.0");
    let home = scratch("shader-api-mixed");
    let project = project_of(&index, p).unwrap();
    assert_eq!(project.support(), super::super::Support::Works);
    let done = install_from(&index, true, &project, &home, &|_, _| {}).unwrap();
    assert_eq!(done[0].version, "1.0.0");
    fs::remove_dir_all(&home).unwrap();
}

/// A registry shader pack made for shader API 0.1 on disk, as the registry's 13 packs are:
/// `glow`'s only version, or its newest above the sample's 0.2 one.
fn glow_for_0_1(index: &mut Index, dir: &Path, only: bool) {
    use std::io::Write;
    let path = dir.join("glow-0.9.0.hshader");
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
    for (name, text) in [
        (
            horizon_shader_api::MANIFEST,
            "schema = 1\nid = \"glow\"\nname = \"Soft Glow\"\nversion = \"0.9.0\"\napi = \"0.1\"\n\
             [[post]]\nfile = \"post.wgsl\"\n",
        ),
        (
            "post.wgsl",
            "fn post(input: PostInput) -> vec4<f32> { return input.color; }\n",
        ),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
    let glow = index.projects.iter_mut().find(|p| p.id == "glow").unwrap();
    let mut version = glow.versions[0].clone();
    version.version = "0.9.0".into();
    version.api = "0.1".into();
    version.file = index::File {
        name: "glow-0.9.0.hshader".into(),
        url: format!("file://{}", path.display()),
        size: fs::metadata(&path).unwrap().len(),
        sha256: index::sha256_file(&path).unwrap(),
        signature: None,
    };
    if only {
        glow.versions.clear();
    }
    glow.versions.insert(0, version);
}

/// What the browser, an install and an automatic update make of `glow` in `index`: the version
/// each takes (an install really downloads and checks it), or the reason the card gives.
fn glow_taken(index: &Index, name: &str) -> Result<String, String> {
    let p = index.project("glow").unwrap();
    let project = project_of(index, p).unwrap();
    let home = scratch(name);
    let installed = install_from(index, true, &project, &home, &|_, _| {});
    let versions = versions_of(index, "glow");
    let picked = super::super::pick(Kind::Shader, &versions, true);
    let listed = search(index, &Query::new("", Kind::Shader)).len() == 1;
    let taken = match (installed, project.support()) {
        (Ok(done), super::super::Support::Works) => {
            let version = done[0].version.clone();
            assert!(home.join(&done[0].file).is_file());
            // The three places agree: the card, the picked version, the install, the update.
            assert_eq!(picked.unwrap().number, version);
            assert_eq!(newest(index, p).unwrap().version, version);
            assert_eq!(installable(index, p).unwrap().version, version);
            assert!(listed);
            Ok(version)
        }
        (Err(refused), super::super::Support::No(why)) => {
            assert!(picked.is_none() && installable(index, p).is_none() && !listed);
            assert!(newest(index, p).is_err());
            assert!(!home.join("mod-browser").exists(), "nothing was downloaded");
            assert!(
                refused.starts_with("Soft Glow needs shader API "),
                "{refused}"
            );
            Err(why)
        }
        other => panic!("the card and the install disagree: {other:?}"),
    };
    fs::remove_dir_all(&home).unwrap();
    taken
}

/// The launcher offers and installs a shader pack for the game it installed, not for the build
/// it is itself (#17527): a game that serves 0.1 and 0.2, one that serves only 0.2 (main since
/// #15886), and the Nightly of 2026-11-13, whose `--launch-api` says 0.1. The 0.1 file goes
/// through the index's checks although this launcher's own checker can't read it.
#[test]
fn shader_packs_are_offered_for_the_shader_api_the_installed_game_serves() {
    let dir = scratch("shader-api-game");
    let only_0_2 = sample();
    let (mut only_0_1, mut mixed) = (sample(), sample());
    glow_for_0_1(&mut only_0_1, &dir, true);
    glow_for_0_1(&mut mixed, &dir, false);
    let taken = |game: &[&str], index: &Index| with_game(game, || glow_taken(index, "taken"));
    let ok = |version: &str| Ok(version.to_owned());
    let made_for = |api: &str, served: &str| {
        Err(format!(
            "Made for shader API {api}: this Horizon serves {served}."
        ))
    };

    let both = ["0.1", "0.2"];
    assert_eq!(taken(&both, &only_0_1), ok("0.9.0"));
    assert_eq!(taken(&both, &only_0_2), ok("1.0.0"));
    assert_eq!(taken(&both, &mixed), ok("0.9.0"));

    assert_eq!(taken(&["0.2"], &only_0_1), made_for("0.1", "0.2"));
    assert_eq!(taken(&["0.2"], &only_0_2), ok("1.0.0"));
    assert_eq!(taken(&["0.2"], &mixed), ok("1.0.0"));

    assert_eq!(taken(&["0.1"], &only_0_1), ok("0.9.0"));
    assert_eq!(taken(&["0.1"], &only_0_2), made_for("0.2", "0.1"));
    assert_eq!(taken(&["0.1"], &mixed), ok("0.9.0"));

    // A game that gave no answer: the launcher's own line, as before.
    assert_eq!(glow_taken(&only_0_1, "taken"), made_for("0.1", "0.2"));
    fs::remove_dir_all(&dir).unwrap();
}

/// Two instances playing different games: what is installed into one, updated in it and marked
/// in its pack list goes by the game that instance plays, not by the selected instance's
/// (#17527). In its own launcher folders, since it installs versions and makes an instance.
#[test]
fn a_shader_pack_goes_by_the_game_of_the_instance_it_is_for() {
    const CHILD: &str = "HORIZON_TEST_TWO_GAMES";
    let Some(root) = std::env::var_os(CHILD).map(PathBuf::from) else {
        let root = scratch("two-games");
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "browser::horizon::tests::a_shader_pack_goes_by_the_game_of_the_instance_it_is_for",
                "--nocapture",
            ])
            .env(CHILD, &root)
            .env("HORIZON_LAUNCHER_HOME", root.join("launcher"))
            .env("HORIZON_HOME", root.join("game"))
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("XDG_DATA_HOME", root.join("data"))
            .output()
            .unwrap();
        let said = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "{said}");
        fs::remove_dir_all(&root).unwrap();
        return;
    };
    use crate::game_shader_api::tests::answers;
    // The selected instance plays the launcher's version, the newest: a game from main. The
    // other one keeps the Nightly of 2026-11-13.
    answers("nightly-2026-11-13-9f42064", &["0.1"]);
    answers("nightly-2026-11-16-aaaaaaa", &["0.2"]);
    let selected = crate::instances::selected();
    crate::instances::create("Kept", None).unwrap();
    let kept = crate::instances::edit("Kept", |meta| {
        meta.version = Some("nightly-2026-11-13-9f42064".into());
    })
    .unwrap();
    assert_ne!(kept.home, selected.home);
    assert_eq!(crate::game_shader_api::served(), ["0.2"]);

    let for_0_2 = sample();
    let mut for_0_1 = sample();
    glow_for_0_1(&mut for_0_1, &root, true);
    let install = |index: &Index, home: &Path| {
        let project = project_of(index, index.project("glow").unwrap()).unwrap();
        let done = install_from(index, true, &project, home, &|_, _| {});
        done.map(|done| done[0].version.clone())
    };
    assert_eq!(
        install(&for_0_2, &kept.home).unwrap_err(),
        "Soft Glow needs shader API 0.2; this Horizon serves 0.1."
    );
    {
        // An operation on one folder that reaches into another (an instance's command that
        // installs into another profile): each folder goes by its own game, and the outer
        // one's comes back.
        use crate::game_shader_api::{for_home, served};
        let _outer = for_home(&selected.home);
        assert_eq!(served(), ["0.2"]);
        assert_eq!(install(&for_0_1, &kept.home).unwrap(), "0.9.0");
        assert_eq!(served(), ["0.2"]);
        let inner = for_home(&kept.home);
        let same = for_home(&kept.home);
        assert_eq!(served(), ["0.1"]);
        drop(same);
        assert_eq!(served(), ["0.1"]);
        drop(inner);
        assert_eq!(served(), ["0.2"]);
    }
    assert_eq!(
        install(&for_0_1, &selected.home).unwrap_err(),
        "Soft Glow needs shader API 0.1; this Horizon serves 0.2."
    );
    assert_eq!(install(&for_0_2, &selected.home).unwrap(), "1.0.0");

    // A newer version for 0.2 comes out: the instance on the Nightly stays on its 0.1 version.
    let mut newer = sample();
    glow_for_0_1(&mut newer, &root, false);
    newer
        .projects
        .iter_mut()
        .find(|p| p.id == "glow")
        .unwrap()
        .versions
        .swap(0, 1);
    let play = auto::Context { in_pack: false };
    let report = auto::run_with(&newer, true, &kept.home, &play, &|_, _| {});
    assert_eq!(report, auto::Report::default());
    assert!(kept.home.join("shaderpacks/glow-0.9.0.hshader").is_file());
    // Its pack list doesn't mark the pack its game runs.
    let listed = crate::packs::list(&kept.home, crate::packs::Kind::Shader);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].unsupported, None);
}

/// A shader pack for this build's shader API, and a shaders/-folder pack (which names none), are
/// left as they were.
#[test]
fn shader_packs_for_this_shader_api_are_offered_as_before() {
    let mut index = sample();
    let versions = versions_of(&index, "glow");
    assert_eq!(versions[0].blocked, None);
    let support = |index: &Index| {
        project_of(index, index.project("glow").unwrap())
            .unwrap()
            .support()
    };
    assert_eq!(support(&index), super::super::Support::Works);
    let glow = index.projects.iter_mut().find(|p| p.id == "glow").unwrap();
    glow.kind = index::Kind::ShaderPack;
    glow.versions[0].api = String::new();
    assert_eq!(versions_of(&index, "glow")[0].blocked, None);
    assert_eq!(support(&index), super::super::Support::Works);
}

#[test]
fn refuses_a_changed_file_a_taken_id_and_local_files_from_the_real_registry() {
    let index = sample();

    let home = scratch("tampered");
    let mut tampered = index.clone();
    let cozy = tampered.projects[1].versions[0].file.url.clone();
    tampered.projects[0].versions[0].file.url = cozy;
    let e = put(&tampered, true, &home).unwrap_err();
    assert!(e.contains("larger than its source says"), "{e}");
    assert!(!home.join("horizon-mods").exists());

    let e = put(&index, false, &home).unwrap_err();
    assert!(e.contains("file servers"), "{e}");

    fs::create_dir_all(home.join("horizon-mods/coords_hud")).unwrap();
    let e = put(&index, true, &home).unwrap_err();
    assert!(e.contains("already in horizon-mods"), "{e}");

    let mut taken_down = index.clone();
    taken_down.revoked.push(index::Revoked {
        id: "coords_hud".into(),
        sha256: Some(index.projects[0].versions[0].file.sha256.clone()),
        reason: "Test.".into(),
        date: String::new(),
    });
    let e = put(&taken_down, true, &home).unwrap_err();
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
    let versions = |id: &str| -> Vec<Version> {
        let p = index.project(id).unwrap();
        p.versions
            .iter()
            .map(|x| version_of(&index, p, x))
            .collect()
    };
    assert_eq!(
        pick(Kind::Modpack, &versions("cozy"), false)
            .unwrap()
            .number,
        "1.0.0"
    );
    let hud = versions("coords_hud");
    assert!(hud[0].blocked.is_none());
    assert_eq!(pick(Kind::Mod, &hud, false).unwrap().number, "0.1.0");
}

#[test]
fn a_project_made_for_the_earlier_mod_system_says_it_cant_be_installed() {
    let mut index = sample();
    let hud = index
        .projects
        .iter_mut()
        .find(|p| p.id == "coords_hud")
        .unwrap();
    hud.versions[0].game = GAME.into();
    hud.versions[0].capabilities = vec!["game-read".to_string()].into();
    let p = project_of(&index, index.project("coords_hud").unwrap()).unwrap();
    assert!(!p.support().installable());
    assert!(p.support().reason().unwrap().contains("earlier mod system"));
    // Beside a mod that installs, with fewer downloads:
    let mut other = index.project("coords_hud").unwrap().clone();
    other.id = "lanterns_plus".into();
    other.versions[0].capabilities = Default::default();
    other.downloads = 0;
    index.projects.push(other);
    index
        .projects
        .iter_mut()
        .find(|p| p.id == "coords_hud")
        .unwrap()
        .downloads = 1_000_000;
    let ids = |everything: bool| -> Vec<String> {
        let mut q = Query::new("", Kind::Mod);
        q.everything = everything;
        search(&index, &q).into_iter().map(|p| p.id).collect()
    };
    // The list leaves it out, as the note above it says.
    assert!(!ids(false).iter().any(|id| id == "coords_hud"));
    // Asked for everything, it comes after what installs, despite its downloads.
    let order = ids(true);
    let at = |id: &str| order.iter().position(|o| o == id).unwrap();
    assert!(at("lanterns_plus") < at("coords_hud"), "{order:?}");
    // The same project with a version this Horizon can use installs as before.
    let mut fine = sample();
    fine.projects
        .iter_mut()
        .find(|p| p.id == "coords_hud")
        .unwrap()
        .versions[0]
        .game = GAME.into();
    let p = project_of(&fine, fine.project("coords_hud").unwrap()).unwrap();
    assert!(p.blocked.is_none());
}

#[test]
fn a_summary_the_registry_cut_mid_word_ends_on_a_whole_word() {
    let cut = format!("{} and tools at the crafti", "x".repeat(SUMMARY_CUT - 24));
    assert_eq!(cut.chars().count(), SUMMARY_CUT);
    assert!(summary(&cut).ends_with(" at the…"), "{}", summary(&cut));
    // Whole sentences, and short ones, stay as written.
    let whole = format!("{}.", "x ".repeat(SUMMARY_CUT / 2));
    assert_eq!(summary(&whole), whole);
    assert_eq!(summary("A short one"), "A short one");
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

#[test]
fn ratings_keep_only_what_is_well_formed() {
    let text = r#"{"projects": {
        "zoom": {"average": 4.25, "count": 12},
        "minimap": {"average": 9, "count": 3},
        "keystrokes": {"average": 3, "count": 0},
        "Bad Id": {"average": 4, "count": 1},
        "fps_overlay": {"average": "5", "count": 1}
    }}"#;
    let ratings = parse_ratings(text);
    assert_eq!(
        ratings.into_iter().collect::<Vec<_>>(),
        [(
            "zoom".to_owned(),
            Rating {
                average: 4.25,
                count: 12
            }
        )]
    );
    assert!(parse_ratings("not json").is_empty());
    assert!(parse_ratings(r#"{"projects": []}"#).is_empty());
    assert_eq!(
        rate_url("zoom"),
        "https://playhorizon.dev/api/mods/auth/login?back=/mods/zoom"
    );
}

#[test]
fn a_project_is_starred_on_the_repository_its_source_is_in() {
    let repo = |s: &str| github_repo(s);
    let daybreak = Some("https://github.com/hildlabs/horizon-shaders".to_owned());
    // A folder of it, a trailing slash, `.git`, a query or an anchor: the same repository.
    for source in [
        "https://github.com/hildlabs/horizon-shaders",
        "https://github.com/hildlabs/horizon-shaders/tree/main/pack",
        "https://github.com/hildlabs/horizon-shaders/",
        "https://github.com/hildlabs/horizon-shaders.git",
        "https://github.com/hildlabs/horizon-shaders?tab=readme",
        "https://github.com/hildlabs/horizon-shaders#readme",
    ] {
        assert_eq!(repo(source), daybreak, "{source:?}");
    }
    assert_eq!(
        repo("https://github.com/horizon-rust/horizon.v2_mods"),
        Some("https://github.com/horizon-rust/horizon.v2_mods".to_owned())
    );
    // Not GitHub, no repository, or a name GitHub doesn't allow: no star button.
    for source in [
        "https://gitlab.com/hildlabs/horizon-shaders",
        "http://github.com/hildlabs/horizon-shaders",
        "https://github.com.evil.example/hildlabs/horizon-shaders",
        "https://github.com/hildlabs",
        "https://github.com/hildlabs/",
        "https://github.com/-hildlabs/horizon-shaders",
        "https://github.com/hildlabs/..",
        "https://github.com/hildlabs/horizon shaders",
        "",
    ] {
        assert_eq!(repo(source), None, "{source:?}");
    }
}
