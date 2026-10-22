use super::*;
use serde_json::json;
use sha2::Digest;

fn v(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn version(id: &str, games: &[&str], loaders: &[&str]) -> Version {
    Version {
        id: id.into(),
        number: id.into(),
        published: String::new(),
        game_versions: v(games),
        loaders: v(loaders),
        files: vec![FileRef {
            url: format!("https://cdn.modrinth.com/data/x/versions/{id}/{id}.zip"),
            name: format!("{id}.zip"),
            size: 3,
            sha512: Some("ab".into()),
            sha1: None,
            primary: true,
        }],
        required: Vec::new(),
        blocked: None,
    }
}

#[test]
fn support_by_kind() {
    assert_eq!(
        support(
            Kind::ResourcePack,
            &v(&["1.21.4", GAME]),
            &v(&["minecraft"])
        ),
        Support::Works
    );
    let old = support(
        Kind::ResourcePack,
        &v(&["1.20.1", "1.21.10", "1.21.9", "26.4-snapshot-1"]),
        &[],
    );
    assert_eq!(old.label(), "Partly");
    assert!(
        old.reason().unwrap().starts_with("Made for 1.21.10:"),
        "{old:?}"
    );
    assert!(support(Kind::ShaderPack, &[], &v(&["iris"])).installable());
    assert!(support(Kind::ShaderPack, &[], &v(&["optifine"])).installable());
    let canvas = support(Kind::ShaderPack, &[], &v(&["canvas"]));
    assert!(!canvas.installable());
    assert!(canvas.reason().unwrap().contains("canvas"));
    assert!(!support(Kind::ShaderPack, &[], &[]).installable());
    assert!(support(Kind::Modpack, &v(&["1.21.1"]), &v(&["neoforge"])).installable());
}

#[test]
fn picks_the_newest_version_that_runs() {
    let versions = [
        version("snapshot", &["26.4-snapshot-5"], &["minecraft"]),
        version("now", &["26.2", GAME], &["minecraft"]),
        version("old", &["1.21.1"], &["minecraft"]),
    ];
    assert_eq!(
        pick(Kind::ResourcePack, &versions, false).unwrap().id,
        "now"
    );
    let old = [version("old", &["1.21.1"], &["minecraft"])];
    assert!(pick(Kind::ResourcePack, &old, false).is_none());
    assert_eq!(pick(Kind::ResourcePack, &old, true).unwrap().id, "old");

    let shaders = [
        version("canvas", &[GAME], &["canvas"]),
        version("iris", &["1.21.1"], &["iris"]),
    ];
    assert_eq!(pick(Kind::ShaderPack, &shaders, false).unwrap().id, "iris");
    let only_canvas = [version("canvas", &[GAME], &["canvas"])];
    assert!(pick(Kind::ShaderPack, &only_canvas, true).is_none());

    let mut blocked = version("blocked", &[GAME], &["minecraft"]);
    blocked.blocked = Some("no".into());
    let mut empty = version("empty", &[GAME], &["minecraft"]);
    empty.files.clear();
    assert!(pick(Kind::ResourcePack, &[blocked, empty], true).is_none());
}

#[test]
fn file_names_must_be_plain() {
    for good in [
        "Faithful 32x.zip",
        "FreshAnimations_v1.11.1_26.1+.zip",
        "BSL_v10.zip",
    ] {
        assert_eq!(safe_name(good), Some(good));
    }
    for bad in [
        "",
        "../x.zip",
        "a/b.zip",
        "a\\b.zip",
        ".hidden.zip",
        "c:x.zip",
        "x.zip.",
        "x\n.zip",
    ] {
        assert_eq!(safe_name(bad), None, "{bad:?}");
    }
}

#[test]
fn modrinth_search_asks_for_what_runs() {
    let q = Query::new("  faithful ", Kind::ResourcePack);
    assert_eq!(q.text, "faithful");
    assert_eq!(
        modrinth::facets(&q),
        r#"[["project_type:resourcepack"],["versions:26.3"]]"#
    );
    let url = modrinth::search_url(&q);
    assert!(url.starts_with("https://api.modrinth.com/v2/search?facets=%5B%5B%22project_type"));
    assert!(
        url.contains("&index=relevance&") && url.ends_with("&query=faithful"),
        "{url}"
    );
    let shaders = Query::new("", Kind::ShaderPack);
    assert_eq!(
        modrinth::facets(&shaders),
        r#"[["project_type:shader"],["categories:iris","categories:optifine"]]"#
    );
    assert!(modrinth::search_url(&shaders).contains("&index=downloads&"));
    let everything = Query {
        everything: true,
        ..Query::new("", Kind::ResourcePack)
    };
    assert_eq!(
        modrinth::facets(&everything),
        r#"[["project_type:resourcepack"]]"#
    );
    assert_eq!(
        modrinth::facets(&Query::new("", Kind::Modpack)),
        r#"[["project_type:modpack"]]"#
    );
}

#[test]
fn modrinth_answers() {
    let hit = json!({
        "project_id": "50dA9Sha", "project_type": "resourcepack", "slug": "fresh-animations",
        "title": "Fresh Animations", "description": "Animated entities.", "author": "FreshLX",
        "downloads": 51404735, "icon_url": "https://cdn.modrinth.com/data/50dA9Sha/icon.webp",
        "license": "LicenseRef-See-Terms", "versions": ["1.21.11", "26.3"],
        "categories": ["16x", "minecraft"]
    });
    let p = modrinth::parse_hit(&hit).unwrap();
    assert_eq!(p.id, "50dA9Sha");
    assert_eq!(p.kind, Kind::ResourcePack);
    assert_eq!(p.page, "https://modrinth.com/resourcepack/fresh-animations");
    assert_eq!(p.support(), Support::Works);
    assert_eq!(p.downloads, 51404735);
    // A Java mod is never a result.
    let jar = json!({"project_id": "AANobbMI", "project_type": "mod", "slug": "sodium"});
    assert!(modrinth::parse_hit(&jar).is_none());

    let project = json!({
        "id": "HVnmMxH1", "project_type": "shader", "slug": "complementary-reimagined",
        "title": "Complementary Shaders - Reimagined", "description": "d",
        "license": {"id": "LicenseRef-Complementary", "name": ""},
        "game_versions": ["1.21.1"], "loaders": ["iris", "optifine"]
    });
    let p = modrinth::parse_project(&project).unwrap();
    assert_eq!(p.kind, Kind::ShaderPack);
    assert_eq!(p.license, "LicenseRef-Complementary");
    assert_eq!(
        p.page,
        "https://modrinth.com/shader/complementary-reimagined"
    );
    assert!(p.support().installable());

    let version = json!({
        "id": "tvGVYMD4", "version_number": "1.11.1", "date_published": "2026-10-20T02:23:13Z",
        "game_versions": ["26.2", "26.3"], "loaders": ["minecraft"],
        "dependencies": [
            {"project_id": "need", "dependency_type": "required"},
            {"project_id": "maybe", "dependency_type": "optional"},
            {"version_id": "x", "project_id": null, "dependency_type": "required"}
        ],
        "files": [
            {"hashes": {"sha512": "AB12", "sha1": "cd34"}, "url": "https://cdn.modrinth.com/data/a/b.zip",
             "filename": "b.zip", "primary": false, "size": 10},
            {"hashes": {"sha512": "ef56", "sha1": "0789"}, "url": "https://cdn.modrinth.com/data/a/c.zip",
             "filename": "c.zip", "primary": true, "size": 20}
        ]
    });
    let ver = modrinth::parse_version(&version).unwrap();
    assert_eq!(ver.required, ["need"]);
    assert_eq!(ver.files[0].sha512.as_deref(), Some("ab12"));
    let file = ver.file().unwrap();
    assert_eq!(file.name, "c.zip");
    assert_eq!(file.checksum(), Some(("SHA-512", "ef56")));
}

#[test]
fn curseforge_stays_off_without_a_key() {
    assert_eq!(curseforge::key(None, None), None);
    assert_eq!(curseforge::key(Some("  ".into()), None), None);
    assert_eq!(curseforge::key(None, Some("")), None);
    assert_eq!(
        curseforge::key(None, Some("built")).as_deref(),
        Some("built")
    );
    assert_eq!(
        curseforge::key(Some("env ".into()), Some("built")).as_deref(),
        Some("env")
    );
}

#[test]
fn curseforge_answers() {
    let q = Query::new("bsl", Kind::ShaderPack);
    let url = curseforge::search_url(&q);
    assert!(url.contains("gameId=432&classId=6552&"), "{url}");
    assert!(
        !url.contains("gameVersion=") && url.ends_with("&searchFilter=bsl"),
        "{url}"
    );
    assert!(
        curseforge::search_url(&Query::new("", Kind::ResourcePack)).contains("&gameVersion=26.3")
    );

    let m = json!({
        "id": 228756, "classId": 6552, "slug": "bsl-shaders", "name": "BSL Shaders",
        "summary": "s", "authors": [{"name": "CaptTatsu"}], "downloadCount": 1.5e7,
        "links": {"websiteUrl": "https://www.curseforge.com/minecraft/shaders/bsl-shaders"},
        "logo": {"thumbnailUrl": "https://media.forgecdn.net/avatars/thumbnails/x.png"},
        "latestFilesIndexes": [{"gameVersion": "1.21.1"}],
        "latestFiles": [{"gameVersions": ["1.21.1", "Iris", "OptiFine"]}]
    });
    let p = curseforge::parse_mod(&m).unwrap();
    assert_eq!(
        (p.source, p.id.as_str(), p.kind),
        (SourceId::CurseForge, "228756", Kind::ShaderPack)
    );
    assert_eq!(p.loaders, ["iris", "optifine"]);
    assert_eq!(p.author, "CaptTatsu");
    assert!(p.support().installable());
    assert!(curseforge::parse_mod(&json!({"id": 1, "classId": 6})).is_none());

    let file = json!({
        "id": 5000, "displayName": "BSL v10", "fileName": "BSL_v10.zip", "fileLength": 300,
        "fileDate": "2026-01-01T00:00:00Z", "downloadUrl": "https://edge.forgecdn.net/files/5/0/BSL_v10.zip",
        "gameVersions": ["1.21.1", "Iris"],
        "hashes": [{"value": "D41D8CD9", "algo": 2}, {"value": "DA39A3EE", "algo": 1}],
        "dependencies": [{"modId": 7, "relationType": 3}, {"modId": 8, "relationType": 2}]
    });
    let ver = curseforge::parse_file(&file).unwrap();
    assert_eq!(ver.required, ["7"]);
    assert_eq!(ver.loaders, ["iris"]);
    assert_eq!(ver.file().unwrap().checksum(), Some(("SHA-1", "da39a3ee")));
    assert!(ver.blocked.is_none());
    let mut closed = file.clone();
    closed["downloadUrl"] = serde_json::Value::Null;
    let ver = curseforge::parse_file(&closed).unwrap();
    assert!(ver.files.is_empty() && ver.blocked.is_some());
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("horizon-browser-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn file_for(body: &[u8], sha512: bool) -> FileRef {
    FileRef {
        url: "https://cdn.modrinth.com/data/a/versions/b/pack.zip".into(),
        name: "pack.zip".into(),
        size: body.len() as u64,
        sha512: sha512.then(|| hex(&sha2::Sha512::digest(body))),
        sha1: (!sha512).then(|| hex(&sha1::Sha1::digest(body))),
        primary: true,
    }
}

#[test]
fn a_download_is_kept_only_when_its_checksum_matches() {
    let dir = scratch("save");
    let body = b"a resource pack";
    for strong in [true, false] {
        let file = file_for(body, strong);
        let path = download::save(&file, &body[..], &dir, &|_| {}).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), body);
        std::fs::remove_file(path).unwrap();
    }
    // Another file than the one listed: refused, and nothing is left behind.
    let file = file_for(body, true);
    let other = b"a resource paCk";
    let e = download::save(&file, &other[..], &dir, &|_| {}).unwrap_err();
    assert!(e.contains("SHA-512 checksum doesn't match"), "{e}");
    let short = download::save(&file, &body[..4], &dir, &|_| {}).unwrap_err();
    assert!(short.starts_with("Got 4 bytes"), "{short}");
    let long = download::save(&file, &b"a resource pack and more"[..], &dir, &|_| {}).unwrap_err();
    assert!(long.contains("larger"), "{long}");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn downloads_refused_before_any_byte() {
    let body = b"x";
    let mut file = file_for(body, true);
    file.url = "https://example.com/pack.zip".into();
    assert!(download::refuse(&file).unwrap().contains("file servers"));
    let mut file = file_for(body, true);
    file.url = "http://cdn.modrinth.com/data/a/pack.zip".into();
    assert!(download::refuse(&file).is_some());
    let mut file = file_for(body, true);
    file.name = "../../options.txt".into();
    assert!(download::refuse(&file).unwrap().contains("plain file name"));
    let mut file = file_for(body, true);
    file.sha512 = None;
    assert!(download::refuse(&file).unwrap().contains("no checksum"));
    let mut file = file_for(body, false);
    file.url = "https://edge.forgecdn.net/files/1/2/pack.zip".into();
    assert_eq!(download::refuse(&file), None);
}

/// Against the real Modrinth: `cargo test --lib browser::tests::live -- --ignored`.
#[test]
#[ignore = "needs the network"]
fn live_modrinth_search_pick_and_download() {
    let source = modrinth::Modrinth;
    for kind in Kind::ALL {
        let found = source.search(&Query::new("", kind)).unwrap();
        assert!(!found.is_empty(), "{kind:?}");
        assert!(found
            .iter()
            .all(|p| p.kind == kind && p.support().installable()));
    }
    let packs = source
        .search(&Query::new("fresh animations", Kind::ResourcePack))
        .unwrap();
    let p = packs.iter().find(|p| p.slug == "fresh-animations").unwrap();
    assert_eq!(p.support(), Support::Works);
    let versions = source.versions(p).unwrap();
    let chosen = pick(p.kind, &versions, false).unwrap();
    assert!(chosen.game_versions.iter().any(|g| g == GAME));
    let dir = scratch("live");
    let path = download::fetch(chosen.file().unwrap(), &dir, &|_| {}).unwrap();
    assert!(path.is_file());
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(source.project(&p.id).unwrap().slug, p.slug);
}
