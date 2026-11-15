use super::*;
use serde_json::{json, Value};

const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
const SHA2: &str = "fedcba9876543210fedcba9876543210fedcba98";

fn link(text: &str) -> Link {
    parse(text).unwrap()
}

#[test]
fn links_name_a_repository_release_ref_or_commit() {
    let plain = link("https://github.com/acme/zoom");
    assert_eq!(
        (plain.owner.as_str(), plain.repo.as_str(), plain.target),
        ("acme", "zoom", Target::Latest)
    );
    assert_eq!(link("https://github.com/acme/zoom.git/").repo, "zoom");
    assert_eq!(
        link("https://github.com/acme/zoom/releases").target,
        Target::Latest
    );
    assert_eq!(
        link("https://github.com/acme/zoom/releases/tag/v1.2.0").target,
        Target::Tag("v1.2.0".into())
    );
    // A branch name keeps its slashes.
    assert_eq!(
        link("https://github.com/acme/zoom/tree/feature/x?tab=1").target,
        Target::Ref("feature/x".into())
    );
    assert_eq!(
        link("https://github.com/acme/zoom/commit/ABCDEF1").target,
        Target::Commit("abcdef1".into())
    );
}

#[test]
fn links_that_are_not_github_repositories_are_refused() {
    for bad in [
        "",
        "acme/zoom",
        "http://github.com/acme/zoom",
        "https://gitlab.com/acme/zoom",
        "https://github.com/acme",
        "https://github.com/acme/zoom/issues/3",
        "https://github.com/acme/zoom/commit/not-hex",
        "https://github.com/acme/zoom/tree/a..b",
        "https://github.com/../zoom",
        "https://github.com/acme/zo om",
    ] {
        assert!(parse(bad).is_err(), "{bad:?}");
    }
}

/// A recorded API: the JSON each path answers, 404 for the rest.
fn recorded(answers: Vec<(&'static str, Value)>) -> impl Fn(&str) -> Result<Option<Value>, String> {
    move |path| {
        Ok(answers
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(_, v)| v.clone()))
    }
}

fn release(tag: &str, assets: Value) -> Value {
    json!({ "tag_name": tag, "assets": assets })
}

fn asset(name: &str, digest: Option<&str>) -> Value {
    json!({
        "name": name,
        "browser_download_url": format!("https://github.com/acme/zoom/releases/download/v1/{name}"),
        "digest": digest,
    })
}

#[test]
fn a_repository_resolves_to_its_latest_releases_hmod_and_commit() {
    let digest = format!("sha256:{}", "AB".repeat(32));
    let api = recorded(vec![
        (
            "repos/acme/zoom/releases/latest",
            release(
                "v1",
                json!([asset("zoom.zip", None), asset("zoom-1.hmod", Some(&digest))]),
            ),
        ),
        ("repos/acme/zoom/commits/v1", json!({ "sha": SHA })),
    ]);
    let found = resolve(&link("https://github.com/acme/zoom"), &api).unwrap();
    assert_eq!(
        (found.tag.as_deref(), found.commit.as_str()),
        (Some("v1"), SHA)
    );
    let file = found.asset.unwrap();
    assert_eq!(file.name, "zoom-1.hmod");
    assert_eq!(file.sha256, Some("ab".repeat(32)));
}

#[test]
fn a_release_with_no_hmod_falls_back_to_its_commit() {
    let api = recorded(vec![
        (
            "repos/acme/zoom/releases/latest",
            release("v1", json!([asset("zoom.zip", None)])),
        ),
        ("repos/acme/zoom/commits/v1", json!({ "sha": SHA })),
    ]);
    let found = resolve(&link("https://github.com/acme/zoom"), &api).unwrap();
    assert_eq!(found.asset, None);
    assert_eq!(found.commit, SHA);
}

#[test]
fn an_asset_from_another_host_is_ignored() {
    let evil = json!({ "name": "x.hmod", "browser_download_url": "https://evil.example/x.hmod" });
    let api = recorded(vec![
        (
            "repos/acme/zoom/releases/latest",
            release("v1", json!([evil])),
        ),
        ("repos/acme/zoom/commits/v1", json!({ "sha": SHA })),
    ]);
    assert_eq!(
        resolve(&link("https://github.com/acme/zoom"), &api)
            .unwrap()
            .asset,
        None
    );
}

#[test]
fn no_release_resolves_the_default_branch_to_a_commit() {
    let api = recorded(vec![
        ("repos/acme/zoom", json!({ "default_branch": "dev/main" })),
        ("repos/acme/zoom/commits/dev/main", json!({ "sha": SHA })),
    ]);
    let found = resolve(&link("https://github.com/acme/zoom"), &api).unwrap();
    assert_eq!(
        (&found.tag, &found.asset, found.commit.as_str()),
        (&None, &None, SHA)
    );
    assert_eq!(found.label(), "acme/zoom @ 0123456");
    assert_eq!(
        found.source_url(),
        format!("https://github.com/acme/zoom/tree/{SHA}")
    );
}

#[test]
fn refs_and_short_commits_become_full_commits_and_unknown_ones_say_so() {
    let api = recorded(vec![
        ("repos/acme/zoom/commits/feature/x", json!({ "sha": SHA })),
        ("repos/acme/zoom/commits/abcdef1", json!({ "sha": SHA2 })),
        ("repos/acme/zoom/releases/tags/v9", Value::Null),
    ]);
    let commit = |url: &str| resolve(&link(url), &api).map(|r| r.commit);
    assert_eq!(
        commit("https://github.com/acme/zoom/tree/feature/x"),
        Ok(SHA.into())
    );
    assert_eq!(
        commit("https://github.com/acme/zoom/commit/abcdef1"),
        Ok(SHA2.into())
    );
    let err = commit("https://github.com/acme/zoom/tree/gone").unwrap_err();
    assert!(err.contains("no commit, tag or branch gone"), "{err}");
    // A tag with no release is a commit.
    assert!(commit("https://github.com/acme/zoom/releases/tag/v9").is_err());
}

#[test]
fn a_rate_limit_is_said_plainly() {
    let limited = |_: &str| Err(RATE_LIMIT.to_string());
    let err = resolve(&link("https://github.com/acme/zoom"), &limited).unwrap_err();
    assert!(
        err.contains("rate limit") && err.contains("Try again"),
        "{err}"
    );
}

fn tree(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gh-mods-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn project(root: &Path, id: &str, horizon: &str) {
    write(
        &root.join("horizon-mod.toml"),
        &format!(
            "id='{id}'\nname='{id}'\nversion='0.1.0'\napi='0.2'\ncapabilities=['registries']\n[native]\ngame='26.3'\nhorizon='{horizon}'\nmixins='{SHA2}'\n"
        ),
    );
    write(
        &root.join("Cargo.toml"),
        "[package]\nname='cargo-name'\nversion='9.0.0'\n",
    );
    write(&root.join("src/lib.rs"), "pub fn hooks() {}");
}

#[test]
fn mod_folders_are_found_wherever_they_are_and_not_searched_inside() {
    let root = tree("find");
    project(&root.join("zoom"), "zoom", SHA);
    project(&root.join("keys"), "keys", SHA);
    project(&root.join("target/debug"), "built", SHA);
    project(&root.join(".git/x"), "hidden", SHA);
    // Stop at the mod root, not arbitrary nested crates inside its source.
    write(
        &root.join("zoom/src/nested/horizon-mod.toml"),
        "not a package",
    );
    let ids: Vec<String> = find_mods(&root)
        .unwrap()
        .into_iter()
        .map(|b| b.manifest.id)
        .collect();
    assert_eq!(ids, ["keys", "zoom"]);
    assert!(find_mods(&tree("empty")).unwrap().is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_mod_folder_packs_into_a_bundle_the_registry_checks_accept() {
    let root = tree("pack");
    project(&root, "zoom", SHA);
    write(&root.join("src/zoom.rs"), "pub fn f() {}");
    write(&root.join("data/zoom/a.json"), "{}");
    write(&root.join("README.md"), "# Zoom");
    write(&root.join("data/zoom/table.bin"), "t");
    write(&root.join("LICENSE"), "MIT");
    write(&root.join("logo.png"), "not shipped");
    let bundle = &find_mods(&root).unwrap()[0];
    let bytes = pack(bundle).unwrap();
    assert_eq!(bytes, pack(bundle).unwrap(), "the same files pack the same");
    let packed = checked(&bytes, &tree("pack-output")).unwrap();
    assert_eq!(
        (packed.id.as_str(), packed.version.as_str()),
        ("zoom", "0.1.0")
    );
    assert_eq!(packed.sha256, sha256(&bytes));
    let read = horizon_mod_package::index::native::Bundle::read(&packed.file).unwrap();
    for shipped in [
        "src/zoom.rs",
        "data/zoom/a.json",
        "README.md",
        "data/zoom/table.bin",
        "LICENSE",
    ] {
        assert!(read.files.contains_key(shipped), "{shipped}");
    }
    assert!(!read.files.keys().any(|n| n.contains("logo")));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_mod_with_no_exact_horizon_commit_is_refused_at_read_boundary() {
    let root = tree("unpinned");
    project(&root, "zoom", "main");
    let err = find_mods(&root).unwrap_err();
    assert!(err.contains("exact revisions"), "{err}");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_flat_hmod_keeps_its_pins_and_ships_only_selected_files() {
    let root = tree("unpacked");
    project(&root, "zoom", SHA);
    write(&root.join("assets/zoom/lang/en_us.json"), "{}");
    let bundle = &find_mods(&root).unwrap()[0];
    assert_eq!(bundle.manifest.native.as_ref().unwrap().mixins, SHA2);
    let packed = checked(&pack(bundle).unwrap(), &tree("unpacked-out")).unwrap();
    let read = horizon_mod_package::index::native::Bundle::read(&packed.file).unwrap();
    assert_eq!(read.manifest.id, "zoom");
    assert!(read.files.contains_key("assets/zoom/lang/en_us.json"));
    let native = read.manifest.native.unwrap();
    assert_eq!(
        (native.horizon.as_str(), native.mixins.as_str()),
        (SHA, SHA2)
    );
    write(&root.join("assets/minecraft/lang/en_us.json"), "{}");
    assert!(pack(bundle).is_err());
    fs::remove_dir_all(root.join("assets/minecraft")).unwrap();
    write(&root.join("evil.exe"), "MZ");
    let again = checked(&pack(bundle).unwrap(), &tree("unpacked-out2")).unwrap();
    assert_eq!(again.sha256, packed.sha256);
    write(&root.join("horizon-mixin.toml"), "[mod]\nid='zoom'\n");
    assert!(find_mods(&root).is_err());
    fs::remove_dir_all(root).unwrap();
}

fn packed(id: &str) -> Packed {
    Packed {
        id: id.into(),
        version: "0.1.0".into(),
        file: PathBuf::new(),
        sha256: String::new(),
    }
}

#[test]
fn several_mods_list_and_ask_which() {
    let two = [packed("keys"), packed("zoom")];
    let err = choose(&two, None, false).unwrap_err();
    assert!(err.contains("keys, zoom") && err.contains("--all"), "{err}");
    assert_eq!(choose(&two, Some("zoom"), false).unwrap()[0].id, "zoom");
    assert_eq!(choose(&two, None, true).unwrap().len(), 2);
    assert!(choose(&two, Some("map"), false).is_err());
    assert_eq!(choose(&two[..1], None, false).unwrap()[0].id, "keys");
}

#[test]
fn installing_records_the_origin_and_updates_follow_releases_or_the_head() {
    let home = tree("install");
    let source = tree("install-src");
    project(&source, "zoom", SHA);
    let bytes = pack(&find_mods(&source).unwrap()[0]).unwrap();
    let ready = checked(&bytes, &source.join("out")).unwrap();
    let found = Resolved {
        owner: "acme".into(),
        repo: "zoom".into(),
        tag: None,
        commit: SHA.into(),
        asset: None,
    };
    let kept = ready.file.clone();
    let origin = install(&home, &found, &ready).unwrap();
    assert!(!kept.exists());
    assert_eq!(origin.label(), "acme/zoom @ 0123456");
    assert_eq!(origin.sha256, sha256(&bytes));
    assert_eq!(self::origin(&home, "zoom"), Some(origin.clone()));
    assert_eq!(origins(&home), vec![origin.clone()]);
    assert!(java::is_enabled(&home, "zoom"));
    // A commit's mod follows the default branch's head.
    assert_eq!(origin.follows().target, Target::Head);
    let newer = Resolved {
        commit: SHA2.into(),
        ..found.clone()
    };
    assert!(is_update(&origin, &newer));
    assert!(!is_update(&origin, &found));
    // A release's mod follows the latest release.
    let release = Origin {
        tag: Some("v1".into()),
        asset: Some("zoom.hmod".into()),
        ..origin.clone()
    };
    assert_eq!(release.label(), "acme/zoom @ v1");
    assert_eq!(release.follows().target, Target::Latest);
    let v2 = Resolved {
        tag: Some("v2".into()),
        ..found.clone()
    };
    assert!(is_update(&release, &v2));
    assert!(!is_update(
        &release,
        &Resolved {
            tag: Some("v1".into()),
            ..found
        }
    ));
    // A file that isn't from GitHub is never replaced.
    write(&home.join(INSTALLED).join("keys.hmod"), "x");
    let other = Packed {
        id: "keys".into(),
        ..packed("keys")
    };
    assert!(install(&home, &newer, &other)
        .unwrap_err()
        .contains("not from GitHub"));
    assert!(remove(&home, "zoom").unwrap());
    assert!(!remove(&home, "zoom").unwrap());
    assert!(!home.join(INSTALLED).join("zoom.hmod").exists());
    assert!(!java::is_enabled(&home, "zoom"));
    let _ = fs::remove_dir_all(home);
    let _ = fs::remove_dir_all(source);
}
