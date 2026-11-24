use super::*;
use ring::signature::{Ed25519KeyPair, KeyPair};

const BUILD: &str = "0123456789abcdef0123456789abcdef01234567";

/// A release folder `sdk-<BUILD>` under a base folder, signed with a fresh key: the base and the
/// key.
fn release(nightly: Option<&str>) -> (tempfile::TempDir, PublicKey) {
    let base = tempfile::tempdir().unwrap();
    let folder = base.path().join(format!("sdk-{BUILD}"));
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join(SURFACE), r#"{"hash":"ab"}"#).unwrap();
    // Any archive the unpack reads stands in for the API source and the build data.
    for archive in [API_SOURCE, build_data::ARCHIVE] {
        fs::write(
            folder.join(archive),
            include_bytes!("../../../horizon-mod-package/src/sdk/fixture.tar.xz"),
        )
        .unwrap();
    }
    let build = sdk::Build {
        commit: BUILD.into(),
        game: sdk::GAME.into(),
        date: "2026-11-13".into(),
        nightly: nightly.map(str::to_owned),
    };
    let index = Index::of_folder(&folder, build, "1.99.1".into()).unwrap();
    fs::write(folder.join(sdk::INDEX), index.to_json()).unwrap();
    let digest = |name: &str| format!("{:x}", Sha256::digest(fs::read(folder.join(name)).unwrap()));
    let sums = horizon_release_sign::sums_file(
        &[sdk::INDEX, SURFACE, API_SOURCE].map(|name| (name.to_owned(), digest(name))),
    );
    fs::write(folder.join(horizon_release_sign::SUMS), &sums).unwrap();
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
    let message = horizon_release_sign::message(&format!("sdk-{BUILD}"), sums.as_bytes());
    use base64::Engine;
    let signature = base64::engine::general_purpose::STANDARD.encode(pair.sign(&message).as_ref());
    fs::write(
        folder.join(horizon_release_sign::SIGNATURE),
        signature + "\n",
    )
    .unwrap();
    let key: PublicKey = pair.public_key().as_ref().try_into().unwrap();
    (base, key)
}

#[test]
fn a_signed_release_is_unpacked_once_and_reused() {
    let (base, key) = release(Some("nightly-2026-11-13"));
    let project = tempfile::tempdir().unwrap();
    let dir = project.path().join(DIR);
    let base = Base::Folder(base.path().to_owned());
    let pinned = fetch(&base, &[key], BUILD, &dir, true).unwrap();
    assert_eq!(pinned.build, BUILD);
    assert_eq!(pinned.name, "nightly-2026-11-13 (0123456)");
    assert!(dir.join(SURFACE).is_file());
    assert!(dir.join("api/deps/a.rmeta").is_file());
    assert!(!dir.join(API_SOURCE).exists());
    assert_eq!(fs::read_to_string(dir.join(STAMP)).unwrap(), BUILD);
    // The stamp names the build: the folder is used as it is, with nothing read from the base.
    let gone = Base::Folder(project.path().join("nowhere"));
    assert_eq!(
        fetch(&gone, &[key], BUILD, &dir, true).unwrap().name,
        pinned.name
    );
}

#[test]
fn a_release_not_signed_by_a_trusted_key_or_not_matching_its_sums_is_refused() {
    let (base, key) = release(None);
    let project = tempfile::tempdir().unwrap();
    let dir = project.path().join(DIR);
    let folder = base.path().join(format!("sdk-{BUILD}"));
    let base = Base::Folder(base.path().to_owned());
    let (_, other) = release(None);
    let error = fetch(&base, &[other], BUILD, &dir, true).unwrap_err();
    assert!(error.contains("signature doesn't match"), "{error}");
    assert!(!dir.join(STAMP).exists());
    // A changed sdk.json is not the one the signed sums list.
    let index = fs::read_to_string(folder.join(sdk::INDEX)).unwrap();
    fs::write(
        folder.join(sdk::INDEX),
        index.replacen("\"ab\"", "\"cd\"", 1),
    )
    .unwrap();
    let error = fetch(&base, &[key], BUILD, &dir, true).unwrap_err();
    assert!(
        error.contains("is not the one its signed SHA256SUMS list"),
        "{error}"
    );
    fs::write(folder.join(sdk::INDEX), index).unwrap();
    // An archive that is not the one sdk.json lists.
    fs::write(folder.join(API_SOURCE), b"not the archive").unwrap();
    let error = fetch(&base, &[key], BUILD, &dir, true).unwrap_err();
    assert!(error.starts_with(&format!("sdk-{BUILD}: ")), "{error}");
    assert!(!dir.join(STAMP).exists());
    // Another build's release under this build's name.
    let other = "89abcdef0123456789abcdef0123456789abcdef";
    fs::rename(&folder, base_folder(&base).join(format!("sdk-{other}"))).unwrap();
    assert!(fetch(&base, &[key], other, &dir, false).is_err());
}

#[test]
fn build_data_comes_from_the_signed_sdk_once_and_only_from_it() {
    let (base, key) = release(None);
    let folder = base.path().join(format!("sdk-{BUILD}"));
    let base = Base::Folder(base.path().to_owned());
    let project = tempfile::tempdir().unwrap();
    // A build with no SDK release is refused by name.
    let other = "89abcdef0123456789abcdef0123456789abcdef";
    let error = build_data_from(&base, &[key], project.path(), other).unwrap_err();
    assert!(
        error.starts_with(&format!(
            "game build {other} has no signed SDK (release sdk-{other}): "
        )),
        "{error}"
    );
    // An archive that is not the one sdk.json lists leaves no build data.
    let archive = fs::read(folder.join(build_data::ARCHIVE)).unwrap();
    fs::write(folder.join(build_data::ARCHIVE), b"not the archive").unwrap();
    let error = build_data_from(&base, &[key], project.path(), BUILD).unwrap_err();
    assert!(error.starts_with(&format!("sdk-{BUILD}: ")), "{error}");
    let into = project.path().join(CACHE).join(BUILD).join(BUILD_DATA);
    assert!(!into.exists());
    fs::write(folder.join(build_data::ARCHIVE), archive).unwrap();
    assert_eq!(
        build_data_from(&base, &[key], project.path(), BUILD).unwrap(),
        into
    );
    assert!(into.join("deps/a.rmeta").is_file());
    assert!(!into.with_file_name(build_data::ARCHIVE).exists());
    // Unpacked once: later reads need no release.
    let gone = Base::Folder(project.path().join("nowhere"));
    assert_eq!(
        build_data_from(&gone, &[key], project.path(), BUILD).unwrap(),
        into
    );
}

fn base_folder(base: &Base) -> &Path {
    match base {
        Base::Folder(folder) => folder,
        Base::Url(_) => unreachable!("a folder"),
    }
}

#[test]
fn a_build_is_named_by_sha_short_sha_or_nightly() {
    let (base, _) = release(None);
    let nightly = base.path().join("nightly-2026-11-13");
    fs::create_dir_all(&nightly).unwrap();
    fs::write(
        nightly.join(sdk::INDEX),
        format!(r#"{{"schema":1,"build":{{"commit":"{BUILD}"}}}}"#),
    )
    .unwrap();
    let base = Base::Folder(base.path().to_owned());
    assert_eq!(resolve(&base, BUILD).unwrap(), BUILD);
    assert_eq!(resolve(&base, "0123456").unwrap(), BUILD);
    assert_eq!(resolve(&base, "nightly-2026-11-13").unwrap(), BUILD);
    let error = resolve(&base, "nightly").unwrap_err();
    assert!(error.starts_with("Nightly nightly has no SDK"), "{error}");
    assert!(
        resolve(&base, "89abcde")
            .unwrap_err()
            .contains("no SDK release")
    );
    assert!(
        resolve(&base, "main")
            .unwrap_err()
            .contains("is not a build")
    );
}

#[test]
fn a_nightly_name_is_only_nightly_or_a_dated_one() {
    let root = tempfile::tempdir().unwrap();
    let base = root.path().join("releases");
    // A name that read through the base would reach this decoy outside it.
    fs::create_dir_all(base.join("nightly-")).unwrap();
    let decoy = root.path().join("decoy");
    fs::create_dir_all(&decoy).unwrap();
    fs::write(
        decoy.join(sdk::INDEX),
        format!(r#"{{"schema":1,"build":{{"commit":"{BUILD}"}}}}"#),
    )
    .unwrap();
    let base = Base::Folder(base);
    for text in [
        "nightly-/../../decoy",
        "nightly-2026-11-1x",
        "nightly-2026/11/13",
        "nightly-2026-11-130",
        "nightly-",
    ] {
        let error = resolve(&base, text).unwrap_err();
        assert!(error.contains("is not a build"), "{text}: {error}");
    }
}

#[test]
fn the_pin_is_the_native_tables_or_the_cargo_metadata() {
    let project = tempfile::tempdir().unwrap();
    let write = |name: &str, text: &str| fs::write(project.path().join(name), text).unwrap();
    let header = "id='m'\nname='m'\nversion='1'\napi='0.2'\ncapabilities=['registries']\n";
    write(MANIFEST, header);
    write("Cargo.toml", "[package]\nname='m'\nversion='1.0.0'\n");
    assert!(
        pin(project.path())
            .unwrap_err()
            .contains("[package.metadata.horizon] sdk")
    );
    repin(project.path(), BUILD).unwrap();
    assert_eq!(pin(project.path()).unwrap(), BUILD);
    let other = "89abcdef0123456789abcdef0123456789abcdef";
    write(
        MANIFEST,
        &format!("{header}[native]\ngame='26.3'\nhorizon='{BUILD}'\nmixins='{BUILD}'\n"),
    );
    fs::create_dir_all(project.path().join(DIR)).unwrap();
    repin(project.path(), other).unwrap();
    assert_eq!(pin(project.path()).unwrap(), other);
    assert!(!project.path().join(DIR).exists());
}

#[test]
fn a_project_inside_a_game_checkout_builds_against_it_and_a_pinned_one_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let write = |path: &Path, text: &str| {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    let workspace = "[workspace]\nmembers = ['crates/horizon-mod-api']\n";
    let dependency = |path: &str| {
        format!(
            "[package]\nname='m'\nversion='1.0.0'\n[dependencies]\nhorizon-mod-api = {{ path = '{path}' }}\n"
        )
    };
    // The game's own fixture: a project inside the checkout, its dependency the checkout's crate.
    let game = dir.path().join("game");
    write(&game.join("Cargo.toml"), workspace);
    write(&game.join("crates/horizon-mod-api/Cargo.toml"), "");
    let fixture = game.join("mods/fixture");
    write(
        &fixture.join("Cargo.toml"),
        &dependency("../../crates/horizon-mod-api"),
    );
    assert_eq!(
        game_tree(&fixture).unwrap(),
        Some(game.canonicalize().unwrap())
    );
    // A mod made from the template: its dependency is the fetched SDK's own workspace.
    let project = dir.path().join("mod");
    write(&project.join(".horizon/sdk/api/Cargo.toml"), workspace);
    write(
        &project.join(".horizon/sdk/api/crates/horizon-mod-api/Cargo.toml"),
        "",
    );
    write(
        &project.join("Cargo.toml"),
        &dependency(".horizon/sdk/api/crates/horizon-mod-api"),
    );
    assert_eq!(game_tree(&project).unwrap(), None);
    // A project outside the checkout that names its crate is still pinned, not built against it.
    let outside = dir.path().join("outside");
    write(
        &outside.join("Cargo.toml"),
        &dependency("../game/crates/horizon-mod-api"),
    );
    assert_eq!(game_tree(&outside).unwrap(), None);
}

/// URL downloads in `sdk_fetch` go through `horizon_port_pack::net::agent`, so a configured proxy
/// receives requests for non-local hosts (`.invalid` never resolves without the proxy).
#[test]
fn url_fetch_routes_through_configured_proxy() {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    struct ResetProxy;
    impl Drop for ResetProxy {
        fn drop(&mut self) {
            horizon_port_pack::net::set_proxy(None);
        }
    }

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request_line = String::new();
        reader.read_line(&mut request_line).unwrap();
        let mut header = String::new();
        while reader.read_line(&mut header).unwrap() > 2 {
            header.clear();
        }
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\nproxied-sdk",
            )
            .unwrap();
        request_line
    });

    let _reset = ResetProxy;
    horizon_port_pack::net::set_proxy(Some(format!("http://127.0.0.1:{port}")));
    let base = Base::Url("http://horizon.invalid/releases".to_owned());
    let body = base.read("sdk-test", "sdk.json", 64).unwrap();
    assert_eq!(body, b"proxied-sdk");
    let request_line = server.join().unwrap();
    assert!(
        request_line.starts_with("GET http://horizon.invalid/releases/sdk-test/sdk.json "),
        "unexpected proxy request line: {request_line:?}"
    );
}
