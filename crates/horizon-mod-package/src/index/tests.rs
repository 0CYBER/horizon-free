//! The sample index (`tests/index/index-v1.json`, the one docs/modding/src/reference/registry-index.md
//! shows) and its files: `coords_hud-0.1.0.hmod`, signed with the throwaway
//! key `test.pub` (Ed25519/Blake2b via OpenSSL; its private key was deleted),
//! `cozy-1.0.0.hpack`, unsigned, and `glow-1.0.0.hshader`, unsigned
//! (`horizon-shader new --template post`, then `horizon-shader pack`).
use super::*;
use std::path::PathBuf;

const OTHER_KEY: &str = "RWSe8ZD+cTOAqNd/jnXTmDtoKp0c0HxkGMrBBWcWQeEkPmcHB+XkaE9S";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/index")
}

fn sample() -> Index {
    Index::parse(&std::fs::read_to_string(fixtures().join("index-v1.json")).unwrap()).unwrap()
}

fn check(index: &Index, id: &str, file: &Path) -> Result<Trust, String> {
    let p = index.project(id).unwrap();
    check_file(index, p, &p.versions[0], file)
}

#[test]
fn reads_the_sample_and_keeps_unknown_kinds() {
    let index = sample();
    assert_eq!(index.schema, SCHEMA);
    let kinds: Vec<&str> = index.projects.iter().map(|p| p.kind.name()).collect();
    assert_eq!(kinds, ["mod", "modpack", "shader", "world"]);
    assert_eq!(index.projects[3].kind, Kind::Other("world".into()));
    let hud = index.project("coords_hud").unwrap();
    assert_eq!(
        hud.versions[0].capabilities.known(),
        [Capability::Registries]
    );
    // What CI writes reads back the same.
    let text = serde_json::to_string(&index).unwrap();
    let again = Index::parse(&text).unwrap();
    assert_eq!(again, index);
    // Download counts: the publish job adds them; none is left out.
    assert_eq!(hud.downloads, 0);
    assert!(!text.contains("downloads"));
    let counted = text.replacen("\"versions\"", "\"downloads\":12345,\"versions\"", 1);
    assert_eq!(Index::parse(&counted).unwrap().projects[0].downloads, 12345);
    // GitHub stars: the same, the source repository's.
    assert_eq!(hud.stars, 0);
    assert!(!text.contains("\"stars\""));
    let starred = text.replacen("\"versions\"", "\"stars\":42,\"versions\"", 1);
    assert_eq!(Index::parse(&starred).unwrap().projects[0].stars, 42);
}

#[test]
fn a_capability_this_build_does_not_know_blocks_only_its_version() {
    // LF whatever the checkout made of it (Git for Windows' core.autocrlf), for the edit below.
    let text = std::fs::read_to_string(fixtures().join("index-v1.json"))
        .unwrap()
        .replace("\r\n", "\n");
    let old = text.replacen(
        "\"registries\"\n          ],",
        "\"registries\", \"game-read\"\n          ],",
        1,
    );
    assert_ne!(old, text, "the fixture's capability list moved");
    let index = Index::parse(&old).unwrap();
    assert_eq!(index.projects.len(), sample().projects.len());
    let hud = index.project("coords_hud").unwrap();
    let listed = &hud.versions[0].capabilities;
    assert_eq!(listed.known(), [Capability::Registries]);
    assert_eq!(listed.unknown(), ["game-read"]);
    assert!(listed.unsupported().unwrap().contains("game-read"));
    assert!(index.installable(hud, "26.3").is_none());
    let pack = index.project("cozy").unwrap();
    assert!(index.installable(pack, &pack.versions[0].game).is_some());
    // Written back, the unknown name stays.
    let again = Index::parse(&serde_json::to_string(&index).unwrap()).unwrap();
    assert_eq!(again, index);
}

#[test]
fn files_pass_against_their_entries() {
    let index = sample();
    let hmod = fixtures().join("coords_hud-0.1.0.hmod");
    assert_eq!(check(&index, "coords_hud", &hmod), Ok(Trust::Verified));
    let hpack = fixtures().join("cozy-1.0.0.hpack");
    assert_eq!(check(&index, "cozy", &hpack), Ok(Trust::Unsigned));
    let hshader = fixtures().join("glow-1.0.0.hshader");
    assert_eq!(check(&index, "glow", &hshader), Ok(Trust::Unsigned));
    let world = index.project("skyblock_start").unwrap();
    let e = check_file(&index, world, &world.versions[0], &hpack).unwrap_err();
    assert!(e.contains("bytes"), "{e}");
}

#[test]
fn a_changed_file_key_or_manifest_is_refused() {
    let index = sample();
    let dir = std::env::temp_dir().join(format!("horizon-index-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let hmod = fixtures().join("coords_hud-0.1.0.hmod");
    let mut bytes = std::fs::read(&hmod).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    let tampered = dir.join("coords_hud-0.1.0.hmod");
    std::fs::write(&tampered, &bytes).unwrap();
    let e = check(&index, "coords_hud", &tampered).unwrap_err();
    assert!(e.contains("SHA-256"), "{e}");

    let mut other_key = index.clone();
    other_key.projects[0].key = Some(OTHER_KEY.into());
    let e = check(&other_key, "coords_hud", &hmod).unwrap_err();
    assert!(e.contains("key"), "{e}");

    let mut more_caps = index.clone();
    more_caps.projects[0].versions[0].capabilities = Default::default();
    let e = check(&more_caps, "coords_hud", &hmod).unwrap_err();
    assert!(e.contains("isn't what the index lists"), "{e}");

    let mut revoked = index.clone();
    revoked.revoked.push(Revoked {
        id: "coords_hud".into(),
        sha256: None,
        reason: "Test.".into(),
        date: String::new(),
    });
    let e = check(&revoked, "coords_hud", &hmod).unwrap_err();
    assert!(e.contains("taken down: Test."), "{e}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn revocations_match_by_hash_or_by_id() {
    let index = sample();
    let ones = "1".repeat(64);
    assert!(index.revoked("bad_mod", &ones).is_some());
    assert!(index.revoked("bad_mod", &"2".repeat(64)).is_none());
    assert!(index.revoked("coords_hud", &ones).is_none());
}

#[test]
fn refuses_broken_indexes() {
    let text = std::fs::read_to_string(fixtures().join("index-v1.json")).unwrap();
    let newer = text.replacen("\"schema\": 1", "\"schema\": 2", 1);
    assert!(Index::parse(&newer).unwrap_err().contains("schema 2"));
    let broken = |edit: &dyn Fn(&mut Index)| {
        let mut index = sample();
        edit(&mut index);
        index.check().unwrap_err()
    };
    let e = broken(&|i| {
        let twin = i.projects[0].clone();
        i.projects.push(twin)
    });
    assert!(e.contains("listed twice"), "{e}");
    let e = broken(&|i| i.projects[0].versions[0].file.url = "http://example.org/x.hmod".into());
    assert!(e.contains("https"), "{e}");
    // A local test registry may list files on disk; the published one may not.
    let local = text.replace(
        "https://github.com/horizon-rust/horizon-mods/releases/download/coords_hud-0.1.0/",
        "file:///tmp/",
    );
    let published = Index::parse(&local).unwrap();
    assert!(published.project("coords_hud").is_none());
    assert!(
        published
            .left_out
            .iter()
            .any(|l| l.reason.contains("https"))
    );
    assert!(Index::parse_local(&local).unwrap().left_out.is_empty());
    let e = broken(&|i| i.projects[0].versions[0].file.name = "../coords_hud.hmod".into());
    assert!(e.contains("plain file name"), "{e}");
    let e = broken(&|i| i.projects[0].versions[0].file.name = "coords_hud.zip".into());
    assert!(e.contains("plain file name"), "{e}");
    let e = broken(&|i| i.projects[0].versions[0].file.signature = None);
    assert!(e.contains("unsigned"), "{e}");
    let e = broken(&|i| i.projects[0].key = Some("not a key".into()));
    assert!(e.contains("key"), "{e}");
    let e = broken(&|i| i.projects[1].versions[0].requires[0].version = "9.9".into());
    assert!(e.contains("not in the index"), "{e}");
    let e = broken(&|i| i.projects[1].versions[0].requires[0].sha256 = "2".repeat(64));
    assert!(e.contains("another file"), "{e}");
    let e = broken(&|i| i.projects[0].id = "Coords".into());
    assert!(e.contains("is not"), "{e}");
}

/// `index` as the registry would publish it, read back.
fn reread(index: &Index) -> Index {
    Index::parse(&serde_json::to_string(index).unwrap()).unwrap()
}

/// The sample with a native mod published under the retired `.hnmod` extension, as the live
/// registry still lists some.
fn with_hnmod() -> Index {
    let mut index = sample();
    let mut native = index.projects[0].clone();
    native.id = "accurate_block_placement".into();
    native.kind = Kind::NativeMod;
    native.versions[0].file.name = "accurate_block_placement-1.0.0.hnmod".into();
    index.projects.push(native);
    index
}

#[test]
fn a_project_with_no_usable_version_is_left_out_and_the_rest_loads() {
    let index = reread(&with_hnmod());
    let ids: Vec<&str> = index.projects.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["coords_hud", "cozy", "glow", "skyblock_start"]);
    let reasons: Vec<(&str, Option<&str>)> = index
        .left_out
        .iter()
        .map(|l| (l.id.as_str(), l.version.as_deref()))
        .collect();
    assert_eq!(
        reasons,
        [
            ("accurate_block_placement", Some("0.1.0")),
            ("accurate_block_placement", None)
        ]
    );
    assert!(index.left_out[0].reason.contains("plain file name"));
}

#[test]
fn a_bad_version_is_left_out_and_its_project_keeps_the_others() {
    let mut index = with_hnmod();
    let native = index.projects.last_mut().unwrap();
    let mut good = native.versions[0].clone();
    good.version = "0.0.9".into();
    good.file.name = "accurate_block_placement-0.0.9.hmod".into();
    native.versions.push(good);
    let index = reread(&index);
    let native = index.project("accurate_block_placement").unwrap();
    let kept: Vec<&str> = native.versions.iter().map(|v| v.version.as_str()).collect();
    assert_eq!(kept, ["0.0.9"]);
    assert_eq!(index.left_out.len(), 1);
    assert_eq!(index.left_out[0].version.as_deref(), Some("0.1.0"));
}

#[test]
fn a_modpack_whose_mod_was_left_out_is_left_out_too() {
    let mut index = sample();
    index.projects[0].versions[0].file.url = "http://example.org/coords_hud.hmod".into();
    let index = reread(&index);
    assert!(index.project("coords_hud").is_none());
    assert!(index.project("cozy").is_none());
    assert!(index.project("glow").is_some());
    let cozy = index.left_out.iter().find(|l| l.id == "cozy").unwrap();
    assert!(cozy.reason.contains("not in the index"), "{}", cozy.reason);
}

#[test]
fn an_id_listed_twice_leaves_out_both() {
    let mut index = sample();
    let twin = index.projects[2].clone();
    index.projects.push(twin);
    let index = reread(&index);
    assert!(index.project("glow").is_none());
    let twice = index
        .left_out
        .iter()
        .filter(|l| l.reason.contains("listed twice"));
    assert_eq!(twice.count(), 2);
}

#[test]
fn only_text_that_is_not_an_index_fails_to_load() {
    assert!(Index::parse("{\"schema\": 1, \"projects\": [{}]}").is_err());
    assert!(Index::parse("not json").is_err());
    // Odd values a mod's author controls never panic the reader.
    let mut index = with_hnmod();
    index.projects[0].versions[0].version = String::new();
    index.projects[0].id = "Ünïcode ✓".into();
    index.projects[2].versions[0].file.name = String::new();
    let index = reread(&index);
    assert!(index.left_out.len() >= 3);
}

#[test]
fn resource_packs_need_a_pack_mcmeta() {
    use std::io::Write;
    let dir = std::env::temp_dir().join(format!("horizon-index-rp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pack = |files: &[(&str, &str)]| {
        let path = dir.join("look.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        for (name, text) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        path
    };
    let meta = r#"{"pack": {"description": "Look", "min_format": 75, "max_format": 75}}"#;
    assert_eq!(
        check_resource_pack(&pack(&[("pack.mcmeta", meta)])),
        Ok("75".into())
    );
    let e = check_resource_pack(&pack(&[("assets/x.png", "")])).unwrap_err();
    assert!(e.contains("missing"), "{e}");
    let e = check_resource_pack(&pack(&[("pack.mcmeta", "{}")])).unwrap_err();
    assert!(e.contains("\"pack\""), "{e}");
    let e = check_resource_pack(&pack(&[("pack.mcmeta", meta), ("../x", "")])).unwrap_err();
    assert!(e.contains("plain path"), "{e}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn data_packs_and_shader_folder_packs_are_read_by_their_content() {
    use std::io::Write;
    let dir = std::env::temp_dir().join(format!("horizon-index-dp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut n = 0;
    let mut pack = |files: &[&str]| {
        n += 1;
        let path = dir.join(format!("{n}.zip"));
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        for name in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            let text = if *name == "pack.mcmeta" {
                r#"{"pack": {"description": "D", "pack_format": 94}}"#
            } else {
                ""
            };
            zip.write_all(text.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        path
    };
    let data = pack(&["pack.mcmeta", "data/x/function/a.mcfunction"]);
    assert_eq!(inspect(&Kind::DataPack, &data).unwrap().api, "94");
    assert_eq!(zip_kind(&data), Kind::DataPack);
    let looks = pack(&["pack.mcmeta", "assets/x/textures/a.png"]);
    let e = inspect(&Kind::DataPack, &looks).unwrap_err();
    assert!(e.contains("data/"), "{e}");
    assert_eq!(zip_kind(&looks), Kind::ResourcePack);
    let no_meta = pack(&["data/x/function/a.mcfunction"]);
    assert!(
        inspect(&Kind::DataPack, &no_meta)
            .unwrap_err()
            .contains("missing")
    );
    let shaders = pack(&["shaders/composite.fsh", "shaders/lang/en_us.lang"]);
    assert_eq!(inspect(&Kind::ShaderPack, &shaders).unwrap().api, "");
    assert_eq!(zip_kind(&shaders), Kind::ShaderPack);
    let e = inspect(&Kind::ShaderPack, &looks).unwrap_err();
    assert!(e.contains("shaders/"), "{e}");
    let sneaky = pack(&["shaders/a.fsh", "../../x"]);
    assert!(
        inspect(&Kind::ShaderPack, &sneaky)
            .unwrap_err()
            .contains("plain path")
    );
    // Both are zips, and a reader from before them keeps them as unknown kinds.
    for kind in [Kind::DataPack, Kind::ShaderPack] {
        assert_eq!(kind.extension(), Some("zip"));
        assert!(open(&kind).is_ok());
        assert_eq!(Kind::from(kind.name().to_owned()), kind);
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_newest_unrevoked_version_for_this_game_is_installed() {
    let mut index = sample();
    let mut newer = index.projects[0].versions[0].clone();
    newer.version = "0.2.0".into();
    newer.file.sha256 = "2".repeat(64);
    index.projects[0].versions.insert(0, newer);
    let p = index.project("coords_hud").unwrap();
    assert_eq!(index.installable(p, "26.3").unwrap().version, "0.2.0");
    assert_eq!(index.installable(p, "26.4"), None);
    index.revoked.push(Revoked {
        id: "coords_hud".into(),
        sha256: Some("2".repeat(64)),
        reason: "Test.".into(),
        date: String::new(),
    });
    let p = index.project("coords_hud").unwrap();
    assert_eq!(index.installable(p, "26.3").unwrap().version, "0.1.0");
    index.revoked.last_mut().unwrap().sha256 = None;
    let p = index.project("coords_hud").unwrap();
    assert_eq!(index.installable(p, "26.3"), None);
}

/// The shader pack `glow` with one version per API line in `apis`, newest first, each named
/// after its line.
fn glow_for(index: &Index, apis: &[&str]) -> Project {
    let mut glow = index.project("glow").unwrap().clone();
    let base = glow.versions[0].clone();
    glow.versions = apis
        .iter()
        .map(|api| Version {
            version: format!("for-{api}"),
            api: (*api).to_owned(),
            ..base.clone()
        })
        .collect();
    glow
}

/// A game gets the newest shader pack version made for a shader API line it serves: one that
/// serves 0.1 and 0.2, one that serves only 0.2 (main since #15886), and the Nightly of
/// 2026-11-13, which says 0.1.
#[test]
fn a_shader_pack_version_is_for_a_line_the_game_serves() {
    let index = sample();
    let picked = |apis: &[&str], served: &[&str]| {
        let glow = glow_for(&index, apis);
        index
            .installable_for(&glow, "26.3", served)
            .map(|v| v.api.clone())
    };
    for (served, only_01, only_02, mixed) in [
        (&["0.1", "0.2"][..], Some("0.1"), Some("0.2"), Some("0.1")),
        (&["0.2"][..], None, Some("0.2"), Some("0.2")),
        (&["0.1"][..], Some("0.1"), None, Some("0.1")),
    ] {
        assert_eq!(picked(&["0.1"], served).as_deref(), only_01, "{served:?}");
        assert_eq!(picked(&["0.2"], served).as_deref(), only_02, "{served:?}");
        assert_eq!(
            picked(&["0.1", "0.2"], served).as_deref(),
            mixed,
            "{served:?}"
        );
    }
    // What this build itself serves is the answer where nothing names a game (the game's own
    // mod list), and only shader packs are asked.
    let old = glow_for(&index, &["0.1"]);
    assert_eq!(index.installable(&old, "26.3"), None);
    let hud = index.project("coords_hud").unwrap();
    assert!(index.installable_for(hud, "26.3", &["0.1"]).is_some());
    assert!(!needs_other_shader_api(hud, &hud.versions[0], &["0.1"]));
}

/// A `.hshader` of `files`, stored as they are: a pack this build's own packer wouldn't take.
fn hshader(path: &Path, files: &[(&str, &str)]) {
    use std::io::Write;
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    for (name, text) in files {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}

/// A shader pack for a line the game serves and this build doesn't: this build's checker can't
/// read it, so the file's size, hash and declared id, version and line are what is held.
#[test]
fn a_shader_pack_for_a_line_only_the_game_serves_keeps_the_index_s_checks() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = |id: &str| {
        format!(
            "schema = 1\nid = \"{id}\"\nname = \"Glow\"\nversion = \"for-0.1\"\napi = \"0.1\"\n\
             [[post]]\nfile = \"post.wgsl\"\n"
        )
    };
    let listed = |index: &Index, path: &Path| {
        let mut glow = glow_for(index, &["0.1"]);
        glow.versions[0].file.size = std::fs::metadata(path).unwrap().len();
        glow.versions[0].file.sha256 = sha256_file(path).unwrap();
        glow
    };
    let index = sample();
    let good = dir.path().join("glow.hshader");
    let post = "fn post(input: PostInput) -> vec4<f32> { return input.color; }\n";
    hshader(
        &good,
        &[
            (horizon_shader_api::MANIFEST, &manifest("glow")),
            ("post.wgsl", post),
        ],
    );
    let glow = listed(&index, &good);
    let v = &glow.versions[0];
    assert_eq!(
        check_file_for(&index, &glow, v, &good, &["0.1"]),
        Ok(Trust::Unsigned)
    );
    assert_eq!(
        check_file_for(&index, &glow, v, &good, &["0.1", "0.2"]),
        Ok(Trust::Unsigned)
    );
    // A game that doesn't serve the line, and this build: the file goes to this build's check.
    for refused in [
        check_file_for(&index, &glow, v, &good, &["0.2"]),
        check_file(&index, &glow, v, &good),
    ] {
        let e = refused.unwrap_err();
        assert!(
            e.contains("needs shader API 0.1; this Horizon serves 0.2"),
            "{e}"
        );
    }
    let mut changed = glow.clone();
    changed.versions[0].file.sha256 = "0".repeat(64);
    let e = check_file_for(&index, &changed, &changed.versions[0], &good, &["0.1"]).unwrap_err();
    assert!(e.contains("SHA-256"), "{e}");

    let other = dir.path().join("other.hshader");
    hshader(
        &other,
        &[
            (horizon_shader_api::MANIFEST, &manifest("other")),
            ("post.wgsl", post),
        ],
    );
    let glow = listed(&index, &other);
    let e = check_file_for(&index, &glow, &glow.versions[0], &other, &["0.1"]).unwrap_err();
    assert!(e.contains("isn't what the index lists"), "{e}");

    let escaping = dir.path().join("escaping.hshader");
    hshader(
        &escaping,
        &[
            (horizon_shader_api::MANIFEST, &manifest("glow")),
            ("../post.wgsl", post),
        ],
    );
    let glow = listed(&index, &escaping);
    let e = check_file_for(&index, &glow, &glow.versions[0], &escaping, &["0.1"]).unwrap_err();
    assert!(e.contains("is not a plain path"), "{e}");
}

#[test]
fn shader_packs_pass_the_game_s_own_check() {
    let manifest = "schema = 1\nid = \"glow\"\nname = \"Glow\"\nversion = \"1.0.0\"\napi = \"0.2\"\n\
                    [[post]]\nfile = \"post.wgsl\"\n";
    let pack = |post: &str| {
        let files = [
            (horizon_shader_api::MANIFEST, manifest),
            ("post.wgsl", post),
        ]
        .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
        .into();
        horizon_shader_api::Pack::from_files(files)
            .unwrap()
            .package()
            .unwrap()
    };
    let dir = std::env::temp_dir().join(format!("horizon-mods-shader-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let good = dir.join("glow-1.0.0.hshader");
    std::fs::write(
        &good,
        pack("fn post(input: PostInput) -> vec4<f32> { return input.color; }\n"),
    )
    .unwrap();
    let bad = dir.join("bad.hshader");
    std::fs::write(
        &bad,
        pack("fn post(input: PostInput) -> vec4<f32> { return input.color }\n"),
    )
    .unwrap();
    let facts = inspect(&Kind::Shader, &good);
    let refused = inspect(&Kind::Shader, &bad).unwrap_err();
    let not_a_pack = inspect(&Kind::Shader, &fixtures().join("cozy-1.0.0.hpack")).unwrap_err();
    std::fs::remove_dir_all(&dir).unwrap();
    let facts = facts.unwrap();
    assert_eq!(
        (
            facts.id.as_deref(),
            facts.version.as_deref(),
            facts.api.as_str()
        ),
        (Some("glow"), Some("1.0.0"), "0.2")
    );
    assert!(refused.contains("post.wgsl:1:"), "{refused}");
    assert!(
        not_a_pack.contains(horizon_shader_api::MANIFEST),
        "{not_a_pack}"
    );
    assert_eq!(Kind::Shader.extension(), Some("hshader"));
    assert!(open(&Kind::Shader).is_ok());
}
