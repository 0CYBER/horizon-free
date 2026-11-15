use super::*;
use horizon_platform::mod_package::sdk::{Build, Mixins, Surface, Toolchain};
use horizon_release_sign::{sha256_hex, sums_file, SigningKey};

const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const TRIPLE: &str = "x86_64-unknown-linux-gnu";
/// An xz tar the packager wrote, holding `deps/a.rmeta`.
const SLICE: &[u8] = include_bytes!("../../../crates/horizon-mod-package/src/sdk/fixture.tar.xz");

fn runtime() -> String {
    Asset::name(Kind::Runtime, TRIPLE, Side::Client)
}

fn api() -> String {
    Asset::name(Kind::Api, TRIPLE, Side::Client)
}

fn engine() -> String {
    Asset::name(Kind::Engine, TRIPLE, Side::Client)
}

/// An index for [`COMMIT`] listing `files`, made with this launcher's Rust.
fn index_of(files: &[(&str, &[u8])], rust: &str) -> Index {
    let assets = files
        .iter()
        .map(|(name, bytes)| {
            let (kind, codec, target, side) = Asset::parts_of(name).unwrap();
            let asset = Asset {
                sha256: sha256_hex(bytes),
                size: bytes.len() as u64,
                kind,
                codec,
                target,
                side,
            };
            (name.to_string(), asset)
        })
        .collect();
    Index {
        schema: sdk::SCHEMA,
        build: Build {
            commit: COMMIT.into(),
            game: sdk::GAME.into(),
            date: "2026-11-13".into(),
            nightly: Some("nightly-2026-11-13".into()),
        },
        package_format: sdk::PACKAGE_FORMAT,
        toolchain: Toolchain { rust: rust.into() },
        content_api: horizon_platform::mod_package::manifest::API.into(),
        surface: Surface {
            hash: "ab".repeat(16),
            previous: None,
            source: None,
            changes: Vec::new(),
            notes: Vec::new(),
        },
        mixins: Mixins {
            rev: horizon_platform::mod_package::mixins_tool::REV.into(),
            patch: horizon_platform::mod_package::mixins_tool::patch_hash(),
        },
        assets,
        inputs: sdk::inputs(),
    }
}

/// A release folder `tag` under `base` holding `files` and an index whose checksums `key` signs
/// for `signed_for`.
fn publish(
    base: &Path,
    tag: &str,
    signed_for: &str,
    key: &SigningKey,
    index: &[u8],
    files: &[(&str, &[u8])],
) {
    let folder = base.join(tag);
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join(INDEX), index).unwrap();
    let mut sums = vec![(INDEX.to_owned(), sha256_hex(index))];
    for (name, bytes) in files {
        fs::write(folder.join(name), bytes).unwrap();
        sums.push((name.to_string(), sha256_hex(bytes)));
    }
    let sums = sums_file(&sums);
    fs::write(folder.join(SUMS), &sums).unwrap();
    let signature = key.sign(signed_for, sums.as_bytes());
    fs::write(folder.join(SIGNATURE), &signature).unwrap();
    fs::write(folder.join(SIGNATURE_V2), &signature).unwrap();
}

struct Fixture {
    scratch: tempfile::TempDir,
    key: SigningKey,
}

impl Fixture {
    fn new() -> Self {
        Self {
            scratch: tempfile::tempdir().unwrap(),
            key: SigningKey::generate().unwrap().0,
        }
    }

    fn base(&self) -> Base {
        Base::Folder(self.scratch.path().join("base"))
    }

    fn kept(&self) -> PathBuf {
        self.scratch.path().join("sdk")
    }

    fn keys(&self) -> [PublicKey; 1] {
        [self.key.public()]
    }

    /// Publishes build [`COMMIT`]'s SDK with a runtime and an api slice and an index listing
    /// `listed` too, then reads its index.
    fn sdk(&self, listed: &[(&str, &[u8])]) -> Result<Sdk, Refusal> {
        let (runtime, api) = (runtime(), api());
        let mut files: Vec<(&str, &[u8])> = vec![(runtime.as_str(), SLICE), (api.as_str(), SLICE)];
        let index = index_of(
            &[files.clone(), listed.to_vec()].concat(),
            rust_toolchain::VERSION,
        )
        .to_json();
        files.retain(|(name, _)| !listed.iter().any(|(other, _)| other == name));
        let tag = tag(COMMIT);
        let base = self.scratch.path().join("base");
        publish(&base, &tag, &tag, &self.key, index.as_bytes(), &files);
        index_in(&self.kept(), &self.base(), COMMIT, &self.keys())
    }
}

#[test]
fn a_base_is_a_url_or_a_folder() {
    assert_eq!(Base::parse(None), Base::Url(DEFAULT_BASE.into()));
    assert_eq!(Base::parse(Some("".into())), Base::Url(DEFAULT_BASE.into()));
    assert_eq!(
        Base::parse(Some("https://mirror.example/sdk/".into())),
        Base::Url("https://mirror.example/sdk".into())
    );
    assert_eq!(
        Base::parse(Some("http://localhost:8000".into())),
        Base::Url("http://localhost:8000".into())
    );
    assert_eq!(
        Base::parse(Some("/media/usb/horizon-sdk".into())),
        Base::Folder("/media/usb/horizon-sdk".into())
    );
}

#[test]
fn a_signed_index_is_read_kept_and_used_offline() {
    let fixture = Fixture::new();
    let sdk = fixture.sdk(&[]).unwrap();
    assert_eq!(sdk.commit(), COMMIT);
    assert_eq!(sdk.shown(), "the Nov 13 update");
    assert!(fixture.kept().join(COMMIT).join(INDEX).is_file());
    // With nothing at the base any more, the kept index serves.
    fs::remove_dir_all(fixture.scratch.path().join("base")).unwrap();
    let again = index_in(&fixture.kept(), &fixture.base(), COMMIT, &fixture.keys()).unwrap();
    assert_eq!(again.index, sdk.index);
    // A build id that is not one is refused before any path is made of it.
    assert!(matches!(
        index_in(&fixture.kept(), &fixture.base(), "../x", &fixture.keys()),
        Err(Refusal::Refused { .. })
    ));
}

#[test]
fn an_index_is_refused_by_name_when_its_signature_or_contents_do_not_hold() {
    let fixture = Fixture::new();
    let base = fixture.scratch.path().join("base");
    let tag = tag(COMMIT);
    let read = || index_in(&fixture.kept(), &fixture.base(), COMMIT, &fixture.keys());
    // No index at all: no SDK for that build.
    assert_eq!(
        read().unwrap_err(),
        Refusal::NoSdk {
            build: "your Horizon version".into()
        }
    );
    let good = index_of(&[], rust_toolchain::VERSION).to_json();
    // Signed for a game release's tag, not this SDK's: refused.
    publish(
        &base,
        &tag,
        "v2099.01.01",
        &fixture.key,
        good.as_bytes(),
        &[],
    );
    assert!(matches!(read(), Err(Refusal::Unsigned { .. })));
    // Signed, but another index than the signed one is served.
    publish(&base, &tag, &tag, &fixture.key, good.as_bytes(), &[]);
    fs::write(
        base.join(&tag).join(INDEX),
        good.replace("2026-11-13", "2026-11-14"),
    )
    .unwrap();
    assert!(matches!(read(), Err(Refusal::Unsigned { .. })));
    // By a key the launcher doesn't trust.
    let stranger = SigningKey::generate().unwrap().0;
    publish(&base, &tag, &tag, &stranger, good.as_bytes(), &[]);
    assert!(matches!(read(), Err(Refusal::Unsigned { .. })));
    // A newer layout, or a newer Rust, needs a newer launcher; an older Rust, a newer game.
    let mut newer = index_of(&[], rust_toolchain::VERSION);
    newer.package_format += 1;
    publish(
        &base,
        &tag,
        &tag,
        &fixture.key,
        newer.to_json().as_bytes(),
        &[],
    );
    assert!(matches!(read(), Err(Refusal::UpdateLauncher { .. })));
    let newer_rust = index_of(&[], "999.0.0").to_json();
    publish(&base, &tag, &tag, &fixture.key, newer_rust.as_bytes(), &[]);
    assert!(matches!(read(), Err(Refusal::UpdateLauncher { .. })));
    let older_rust = index_of(&[], "1.0.0").to_json();
    publish(&base, &tag, &tag, &fixture.key, older_rust.as_bytes(), &[]);
    assert_eq!(
        read().unwrap_err(),
        Refusal::OlderRust {
            build: "the Nov 13 update".into(),
            rust: "1.0.0".into()
        }
    );
    // Nothing refused was kept.
    assert!(!fixture.kept().join(COMMIT).join(INDEX).exists());
}

#[test]
fn the_api_route_fetches_its_two_slices_only_and_reuses_them_on_their_stamps() {
    let fixture = Fixture::new();
    // The engine slice is listed but not published: the API route never asks for it.
    let engine = engine();
    let sdk = fixture
        .sdk(&[(engine.as_str(), &b"never read"[..])])
        .unwrap();
    let prebuilt = fixture.scratch.path().join("prebuilt");
    let heard = std::cell::RefCell::new(Vec::new());
    let package = slices_in(
        &prebuilt,
        &sdk,
        &fixture.base(),
        Route::Api,
        TRIPLE,
        Side::Client,
        &|fetched| heard.borrow_mut().push(fetched),
    )
    .unwrap();
    assert_eq!(
        package,
        Package::at(&prebuilt.join(format!("{COMMIT}-{TRIPLE}")))
    );
    for slice in [&package.runtime, &package.api] {
        assert_eq!(fs::read(slice.join("deps/a.rmeta")).unwrap().len(), 8);
        assert!(stamp_path(slice).is_file());
    }
    assert!(!package.engine.exists());
    let total = 2 * SLICE.len() as u64;
    assert_eq!(heard.borrow().first(), Some(&Fetched { done: 0, total }));
    assert_eq!(heard.borrow().last(), Some(&Fetched { done: total, total }));
    // The engine route needs the slice the base doesn't have: refused by name.
    let refused = slices_in(
        &prebuilt,
        &sdk,
        &fixture.base(),
        Route::Engine,
        TRIPLE,
        Side::Client,
        &|_| {},
    )
    .unwrap_err();
    assert!(refused.to_string().contains(&engine), "{refused}");
    // Kept slices are used again on their stamps alone, with nothing at the base.
    fs::remove_dir_all(fixture.scratch.path().join("base")).unwrap();
    heard.borrow_mut().clear();
    let again = slices_in(
        &prebuilt,
        &sdk,
        &fixture.base(),
        Route::Api,
        TRIPLE,
        Side::Client,
        &|fetched| heard.borrow_mut().push(fetched),
    )
    .unwrap();
    assert_eq!(again, package);
    assert_eq!(*heard.borrow(), [Fetched { done: 0, total: 0 }]);
    // A file whose size changed, or a file that was added, fails the stamp: fetched again
    // (and here refused, the base being gone).
    let changes: [fn(&Path); 2] = [
        |slice| fs::write(slice.join("deps/a.rmeta"), b"longer than eight").unwrap(),
        |slice| fs::write(slice.join("deps/extra.rmeta"), b"").unwrap(),
    ];
    for change in changes {
        let fresh = slices_in(
            &prebuilt,
            &sdk,
            &fixture.base(),
            Route::Api,
            TRIPLE,
            Side::Client,
            &|_| {},
        );
        assert!(fresh.is_ok());
        change(&package.api);
        let stale = slices_in(
            &prebuilt,
            &sdk,
            &fixture.base(),
            Route::Api,
            TRIPLE,
            Side::Client,
            &|_| {},
        );
        assert!(matches!(stale, Err(Refusal::Refused { .. })), "{stale:?}");
        // What failed its stamp is gone, never used half-checked.
        assert!(!package.api.exists());
        // Put it back for the next change.
        fs::create_dir_all(package.api.join("deps")).unwrap();
        fs::copy(
            package.runtime.join("deps/a.rmeta"),
            package.api.join("deps/a.rmeta"),
        )
        .unwrap();
        fs::copy(stamp_path(&package.runtime), stamp_path(&package.api)).unwrap();
        let stamp = fs::read_to_string(stamp_path(&package.api)).unwrap();
        fs::write(stamp_path(&package.api), stamp.replace(&runtime(), &api())).unwrap();
    }
}

#[test]
fn a_stamp_from_another_index_is_not_trusted() {
    let fixture = Fixture::new();
    let sdk = fixture.sdk(&[]).unwrap();
    let prebuilt = fixture.scratch.path().join("prebuilt");
    let package = slices_in(
        &prebuilt,
        &sdk,
        &fixture.base(),
        Route::Api,
        TRIPLE,
        Side::Client,
        &|_| {},
    )
    .unwrap();
    let other = Sdk {
        sha256: "0".repeat(64),
        ..sdk.clone()
    };
    assert!(stamp_holds(&sdk, &runtime(), &package.runtime));
    assert!(!stamp_holds(&other, &runtime(), &package.runtime));
    assert!(!stamp_holds(&sdk, &api(), &package.runtime));
}

#[test]
fn a_game_release_names_its_build_through_its_signed_index() {
    let fixture = Fixture::new();
    let base = fixture.scratch.path().join("base");
    let kept = fixture.scratch.path().join("versions");
    let index = index_of(&[], rust_toolchain::VERSION).to_json();
    let version = "nightly-2026-11-13-0123456";
    // A Nightly: its dated release, signed for its own version.
    publish(
        &base,
        "nightly-2026-11-13",
        version,
        &fixture.key,
        index.as_bytes(),
        &[],
    );
    let read = |version: &str| build_of_in(&kept, version, &fixture.base(), &fixture.keys());
    assert_eq!(read(version).unwrap(), COMMIT);
    assert_eq!(fs::read_to_string(kept.join(version)).unwrap(), COMMIT);
    // Kept: known offline.
    fs::remove_dir_all(&base).unwrap();
    assert_eq!(read(version).unwrap(), COMMIT);
    // The Nightly being served, under `nightly`: read there, signed for its own version.
    let current = "nightly-2026-11-14-0123456";
    publish(
        &base,
        horizon_release_sign::NIGHTLY,
        current,
        &fixture.key,
        index.as_bytes(),
        &[],
    );
    assert_eq!(read(current).unwrap(), COMMIT);
    // `nightly` serving another version doesn't pass for this one; its dated tag is read next.
    let older = "nightly-2026-11-12-0123456";
    assert!(matches!(read(older), Err(Refusal::NoSdk { .. })));
    publish(
        &base,
        "nightly-2026-11-12",
        older,
        &fixture.key,
        index.as_bytes(),
        &[],
    );
    assert_eq!(read(older).unwrap(), COMMIT);
    // A Nightly whose index names another commit than its version: refused.
    let other = "nightly-2026-11-13-abcdef0";
    publish(
        &base,
        "nightly-2026-11-13",
        other,
        &fixture.key,
        index.as_bytes(),
        &[],
    );
    assert!(matches!(read(other), Err(Refusal::Refused { .. })));
    // A snapshot, installed by its tag: read there, signed for its own version on its channel.
    let snapshot = "snapshot-2026-11-13-0123456";
    publish(
        &base,
        snapshot,
        snapshot,
        &fixture.key,
        index.as_bytes(),
        &[],
    );
    assert_eq!(read(snapshot).unwrap(), COMMIT);
    // A snapshot is never read as a Nightly: a snapshot's index and signature served under the
    // Nightly's tags, for a Nightly version of the same date and commit, are refused.
    let as_nightly = "nightly-2026-11-15-0123456";
    for tag in [horizon_release_sign::NIGHTLY, "nightly-2026-11-15"] {
        publish(
            &base,
            tag,
            &snapshot.replace("13", "15"),
            &fixture.key,
            index.as_bytes(),
            &[],
        );
    }
    assert!(matches!(read(as_nightly), Err(Refusal::Unsigned { .. })));
    // A version of neither channel's full form, its own tag holding a validly signed index:
    // no build id (a channel's tag never reads as a release).
    for version in [
        "snapshot-2026-11-16",
        "nightly-2026-11-16",
        "snapshot",
        "snapshot-2026-11-16-XYZ1234",
    ] {
        publish(&base, version, version, &fixture.key, index.as_bytes(), &[]);
        assert!(read(version).is_err(), "{version}");
        assert!(!kept.join(version).exists(), "{version}");
    }
    // A release, signed for its tag; one signed for another tag is refused.
    publish(
        &base,
        "v2099.01.01",
        "v2099.01.01",
        &fixture.key,
        index.as_bytes(),
        &[],
    );
    assert_eq!(read("v2099.01.01").unwrap(), COMMIT);
    publish(
        &base,
        "v2099.01.02",
        "v2099.01.01",
        &fixture.key,
        index.as_bytes(),
        &[],
    );
    assert!(matches!(read("v2099.01.02"), Err(Refusal::Unsigned { .. })));
    // A release with no index has no SDK; a version that is not a plain name is never a path.
    fs::create_dir_all(base.join("v2099.01.03")).unwrap();
    assert!(matches!(read("v2099.01.03"), Err(Refusal::NoSdk { .. })));
    assert!(matches!(
        read("../v2099.01.01"),
        Err(Refusal::Refused { .. })
    ));
}

#[test]
fn rust_versions_compare_by_number() {
    use std::cmp::Ordering::*;
    assert_eq!(compare_versions("1.95.0", "1.95.0"), Some(Equal));
    assert_eq!(compare_versions("1.100.0", "1.95.0"), Some(Greater));
    assert_eq!(compare_versions("1.9.0", "1.95.0"), Some(Less));
    assert_eq!(compare_versions("1.95.0-beta", "1.95.0"), None);
}

#[test]
fn sizes_and_builds_read_plainly() {
    assert_eq!(megabytes(71_000_000), "71 MB");
    assert_eq!(megabytes(1), "1 MB");
    // A build with no name here is never shown as its commit.
    assert_eq!(shown(COMMIT, None), "your Horizon version");
    // A pin from a mod's manifest is cut only where a character ends.
    assert_eq!(shown("ééééé", None), "ééééé");
}

#[test]
fn a_refusal_offers_the_way_on_its_text_promises() {
    let build = || "nightly-2026-11-13".to_owned();
    let way = |refusal: Refusal| way_on(&format!("{refusal}"));
    assert_eq!(
        way(Refusal::NoSdk { build: build() }),
        Some(WayOn::OtherBuild)
    );
    assert_eq!(
        way(Refusal::OlderRust {
            build: build(),
            rust: "1.80.0".into()
        }),
        Some(WayOn::OtherBuild)
    );
    assert_eq!(
        way(Refusal::Unsigned {
            build: build(),
            why: "checksum mismatch".into()
        }),
        Some(WayOn::Report)
    );
    for bytes in [None, Some(212_000_000)] {
        assert_eq!(
            way(Refusal::Offline {
                build: build(),
                bytes
            }),
            Some(WayOn::Retry)
        );
    }
    assert_eq!(way(Refusal::Host), None);
}
