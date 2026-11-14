use super::*;

const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// An index listing `files` (name, bytes), as the writer serializes it.
fn index(files: &[(&str, &[u8])]) -> Index {
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
        schema: SCHEMA,
        build: Build {
            commit: COMMIT.into(),
            game: GAME.into(),
            date: "2026-11-13".into(),
            nightly: None,
        },
        package_format: PACKAGE_FORMAT,
        toolchain: Toolchain {
            rust: "1.99.1".into(),
        },
        content_api: crate::manifest::API.into(),
        surface: Surface {
            hash: "ab".repeat(16),
            previous: None,
            source: None,
            changes: Vec::new(),
            notes: Vec::new(),
        },
        mixins: Mixins {
            rev: crate::mixins_tool::REV.into(),
            patch: crate::mixins_tool::patch_hash(),
        },
        assets,
        inputs: inputs(),
    }
}

fn json(index: &Index) -> serde_json::Value {
    serde_json::to_value(index).unwrap()
}

fn read(value: &serde_json::Value) -> Result<Index, Refusal> {
    Index::read(&serde_json::to_vec(value).unwrap(), COMMIT)
}

const ENGINE: &str = "engine-x86_64-unknown-linux-gnu.tar.xz";

#[test]
fn written_index_reads_back_and_unknown_fields_are_ignored() {
    let written = index(&[("surface.json", b"{}"), (ENGINE, b"engine")]);
    let mut value = json(&written);
    value["later"] = serde_json::json!({"field": 1});
    value["assets"][ENGINE]["later"] = serde_json::json!(true);
    assert_eq!(read(&value).unwrap(), written);
}

#[test]
fn the_comparison_with_the_previous_sdk_is_written_and_read_back() {
    let mut written = index(&[(ENGINE, b"engine")]);
    let hash = written.surface.hash.clone();
    let diff = serde_json::json!({
        "old": "cd".repeat(16), "new": hash, "verdict": "breaking",
        "changes": [
            {"id": "hook.a", "kind": "removed", "class": "breaking", "what": ["removed"],
             "notes": [{"file": "a.toml", "why": "merged into b"}]},
            {"id": "type.b", "kind": "changed", "class": "additive", "what": ["field added"],
             "notes": [{"file": "a.toml", "why": "merged into b"},
                       {"file": "b.toml", "why": "renamed", "migrate": "use c"}]},
        ],
    });
    written
        .compare("f".repeat(40).as_str(), &serde_json::to_vec(&diff).unwrap())
        .unwrap();
    let surface = &written.surface;
    assert_eq!(surface.previous.as_deref(), Some("f".repeat(40).as_str()));
    assert_eq!(surface.source, Some(Verdict::Breaking));
    assert_eq!(surface.changes, ["hook.a", "type.b"]);
    // One per note file, in the notes files' shape, with every id it is attached to.
    let note = |ids: &[&str], why: &str, migrate: Option<&str>| ChangeNote {
        ids: ids.iter().map(|id| id.to_string()).collect(),
        why: why.into(),
        migrate: migrate.map(Into::into),
    };
    assert_eq!(
        surface.notes,
        [
            note(&["hook.a", "type.b"], "merged into b", None),
            note(&["type.b"], "renamed", Some("use c")),
        ]
    );
    assert_eq!(
        json(&written)["surface"]["notes"][1],
        serde_json::json!({"ids": ["type.b"], "why": "renamed", "migrate": "use c"})
    );
    assert_eq!(read(&json(&written)).unwrap(), written);
    // A diff of another surface, or one that is not a diff, is refused.
    let mut other = index(&[(ENGINE, b"engine")]);
    let mut wrong = diff.clone();
    wrong["new"] = "00".repeat(16).into();
    let refused = other.compare("f", &serde_json::to_vec(&wrong).unwrap());
    assert!(matches!(refused, Err(Refusal::Malformed(why)) if why.contains("not this build's")));
    assert!(matches!(
        other.compare("f", b"[]"),
        Err(Refusal::Malformed(_))
    ));
    assert_eq!(other.surface.previous, None);
}

#[test]
fn a_game_releases_copy_names_its_build_before_the_build_is_known() {
    let mut value = json(&index(&[(ENGINE, b"engine")]));
    value["build"]["nightly"] = serde_json::json!("nightly-2026-11-13");
    let bytes = serde_json::to_vec(&value).unwrap();
    let build = Index::build_of(&bytes).unwrap();
    assert_eq!(build.commit, COMMIT);
    assert_eq!(build.nightly.as_deref(), Some("nightly-2026-11-13"));
    value["schema"] = serde_json::json!(SCHEMA + 1);
    let newer = serde_json::to_vec(&value).unwrap();
    assert_eq!(
        Index::build_of(&newer),
        Err(Refusal::Schema(u64::from(SCHEMA) + 1))
    );
    assert!(matches!(
        Index::build_of(b"{\"schema\": 1}"),
        Err(Refusal::Malformed(_))
    ));
    assert!(matches!(
        Index::build_of(b"not json"),
        Err(Refusal::Malformed(_))
    ));
}

#[test]
fn each_bad_index_is_refused_by_name() {
    let good = json(&index(&[(ENGINE, b"engine")]));
    let cases: [(&str, fn(&mut serde_json::Value), Refusal); 8] = [
        (
            "unknown schema",
            |v| v["schema"] = 2.into(),
            Refusal::Schema(2),
        ),
        (
            "wrong build",
            |v| v["build"]["commit"] = "f".repeat(40).into(),
            Refusal::Build {
                expected: COMMIT.into(),
                found: "f".repeat(40),
            },
        ),
        (
            "unknown package format",
            |v| v["package_format"] = 5.into(),
            Refusal::PackageFormat(5),
        ),
        (
            "unknown codec",
            |v| v["assets"][ENGINE]["codec"] = "zstd".into(),
            Refusal::Codec {
                asset: ENGINE.into(),
                codec: "zstd".into(),
            },
        ),
        (
            "unknown kind",
            |v| v["assets"][ENGINE]["kind"] = "slice".into(),
            Refusal::Kind {
                asset: ENGINE.into(),
                kind: "slice".into(),
            },
        ),
        (
            "a path for a name",
            |v| {
                let asset = v["assets"].as_object_mut().unwrap().remove(ENGINE).unwrap();
                v["assets"]["../engine-x86_64-unknown-linux-gnu.tar.xz"] = asset;
            },
            Refusal::AssetName("../engine-x86_64-unknown-linux-gnu.tar.xz".into()),
        ),
        (
            "a path in the target",
            |v| {
                let mut asset = v["assets"].as_object_mut().unwrap().remove(ENGINE).unwrap();
                asset["target"] = "a/../../evil".into();
                v["assets"]["engine-a/../../evil.tar.xz"] = asset;
            },
            Refusal::AssetName("engine-a/../../evil.tar.xz".into()),
        ),
        (
            "a name that says another side",
            |v| v["assets"][ENGINE]["side"] = "server".into(),
            Refusal::AssetName(ENGINE.into()),
        ),
    ];
    for (case, change, refusal) in cases {
        let mut value = good.clone();
        change(&mut value);
        assert_eq!(read(&value).unwrap_err(), refusal, "{case}");
    }
    for broken in [&b"not json"[..], br#"{"schema": 1}"#, b"[]"] {
        assert!(
            matches!(Index::read(broken, COMMIT), Err(Refusal::Malformed(_))),
            "{}",
            String::from_utf8_lossy(broken)
        );
    }
}

#[test]
fn an_asset_is_checked_by_size_then_checksum() {
    let listed = index(&[(ENGINE, b"engine")]);
    listed.verify(ENGINE, &b"engine"[..]).unwrap();
    assert_eq!(
        listed.verify(ENGINE, &b"engine and more"[..]).unwrap_err(),
        Refusal::Size {
            asset: ENGINE.into(),
            expected: 6,
            found: 7,
        }
    );
    assert_eq!(
        listed.verify(ENGINE, &b"ENGINE"[..]).unwrap_err(),
        Refusal::Checksum {
            asset: ENGINE.into()
        }
    );
    assert_eq!(
        listed.verify("surface.json", &b"{}"[..]).unwrap_err(),
        Refusal::Unlisted("surface.json".into())
    );
}

#[test]
fn a_folder_with_a_missing_or_unlisted_asset_is_refused() {
    let listed = index(&[("surface.json", b"{}"), (ENGINE, b"engine")]);
    let folder = tempfile::tempdir().unwrap();
    let write = |name: &str, bytes: &[u8]| std::fs::write(folder.path().join(name), bytes).unwrap();
    write("surface.json", b"{}");
    write(INDEX, b"index");
    assert_eq!(
        listed.verify_folder(folder.path()).unwrap_err(),
        Refusal::Missing(vec![ENGINE.into()])
    );
    write(ENGINE, b"engine");
    listed.verify_folder(folder.path()).unwrap();
    write("api-source.tar.xz", b"source");
    assert_eq!(
        listed.verify_folder(folder.path()).unwrap_err(),
        Refusal::Unlisted("api-source.tar.xz".into())
    );
}

#[test]
fn a_release_folders_index_lists_each_asset_and_reads_back() {
    let folder = tempfile::tempdir().unwrap();
    let write = |name: &str, bytes: &[u8]| std::fs::write(folder.path().join(name), bytes).unwrap();
    let surface = format!(r#"{{"hash":"{}","hooks":[]}}"#, "ab".repeat(16));
    write("surface.json", surface.as_bytes());
    write(ENGINE, b"engine");
    write("SHA256SUMS", b"sums");
    let build = Build {
        commit: COMMIT.into(),
        game: GAME.into(),
        date: "2026-11-13".into(),
        nightly: None,
    };
    let written = Index::of_folder(folder.path(), build.clone(), "1.99.1".into()).unwrap();
    assert_eq!(
        written,
        index(&[("surface.json", surface.as_bytes()), (ENGINE, b"engine")])
    );
    assert_eq!(
        Index::read(written.to_json().as_bytes(), COMMIT).unwrap(),
        written
    );
    write("notes.txt", b"stray");
    assert_eq!(
        Index::of_folder(folder.path(), build, "1.99.1".into()).unwrap_err(),
        Refusal::AssetName("notes.txt".into())
    );
}

#[test]
fn a_route_names_its_assets_for_the_target_and_every_missing_one() {
    let runtime = "runtime-x86_64-unknown-linux-gnu.tar.xz";
    let api = "api-x86_64-unknown-linux-gnu.tar.xz";
    let listed = index(&[(runtime, b"runtime"), (api, b"api"), (ENGINE, b"engine")]);
    let target = "x86_64-unknown-linux-gnu";
    // The API route never needs the engine archive; the engine route needs all three.
    assert_eq!(
        listed.route_assets("api", target, Side::Client).unwrap(),
        [runtime, api]
    );
    assert_eq!(
        listed.route_assets("engine", target, Side::Client).unwrap(),
        [runtime, api, ENGINE]
    );
    assert_eq!(
        listed
            .route_assets("engine", target, Side::Server)
            .unwrap_err(),
        Refusal::Missing(vec![
            "runtime-x86_64-unknown-linux-gnu-server.tar.xz".into(),
            "api-x86_64-unknown-linux-gnu-server.tar.xz".into(),
            "engine-x86_64-unknown-linux-gnu-server.tar.xz".into(),
        ])
    );
}

#[test]
fn asset_names_state_kind_target_and_side() {
    for (name, parts) in [
        (
            "surface.json",
            Some((Kind::Surface, Codec::None, None, None)),
        ),
        (
            "build-data.tar.xz",
            Some((Kind::BuildData, Codec::Xz, None, None)),
        ),
        (
            "runtime-aarch64-apple-darwin.tar.xz",
            Some((
                Kind::Runtime,
                Codec::Xz,
                Some("aarch64-apple-darwin".into()),
                Some(Side::Client),
            )),
        ),
        (
            "engine-x86_64-unknown-linux-gnu-server.tar.xz",
            Some((
                Kind::Engine,
                Codec::Xz,
                Some("x86_64-unknown-linux-gnu".into()),
                Some(Side::Server),
            )),
        ),
        ("engine-.tar.xz", None),
        ("build-data-x.tar.xz", None),
        ("sdk/engine-x.tar.xz", None),
        ("surface.json.xz", None),
        ("engine-a/../../evil.tar.xz", None),
        ("engine-a\\..\\evil.tar.xz", None),
        ("engine-x86.64.tar.xz", None),
        ("engine-X86_64.tar.xz", None),
    ] {
        assert_eq!(Asset::parts_of(name), parts, "{name}");
    }
}

#[test]
fn removing_each_required_input_names_it_before_work() {
    let inputs = inputs();
    for route in ["package", "package-server"] {
        let folder = tempfile::tempdir().unwrap();
        let checkout = folder.path().join("checkout");
        let program = folder.path().join("bin/horizon-client.exe");
        let listed = inputs.route(route).unwrap();
        let files: Vec<(String, PathBuf)> = listed
            .program_files(&program)
            .into_iter()
            .map(|path| (path.file_name().unwrap().to_string_lossy().into(), path))
            .chain(
                listed
                    .checkout
                    .iter()
                    .map(|name| (name.clone(), checkout.join(name))),
            )
            .collect();
        assert!(
            files
                .iter()
                .any(|(name, _)| name == "horizon-wasm-std.wasm")
        );
        assert!(files.iter().any(|(name, _)| name == "horizon-server.exe"));
        for (_, path) in &files {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"").unwrap();
        }
        inputs.check(route, &program, &checkout).unwrap();
        for (name, path) in &files {
            std::fs::remove_file(path).unwrap();
            let Err(Refusal::MissingInputs { missing, how }) =
                inputs.check(route, &program, &checkout)
            else {
                panic!("{route}: {name} missing was not refused");
            };
            assert_eq!(missing, std::slice::from_ref(name), "{route}");
            assert_eq!(how.len(), 1, "{route}: {name} has no way to make it");
            std::fs::write(path, b"").unwrap();
        }
    }
}

#[test]
fn the_package_format_reads_with_any_line_ending() {
    assert_eq!(number("6\n"), 6);
    assert_eq!(number("6\r\n"), 6);
    assert_eq!(number("16"), 16);
    assert_eq!(PACKAGE_FORMAT, 6);
}

#[test]
fn an_unknown_route_is_refused_not_a_panic() {
    let mut downloaded = inputs();
    downloaded.routes.remove("package");
    let folder = tempfile::tempdir().unwrap();
    assert_eq!(
        downloaded
            .check(
                "package",
                &folder.path().join("horizon-client"),
                folder.path()
            )
            .unwrap_err(),
        Refusal::Route("package".into())
    );
    let mut listed = index(&[(ENGINE, b"engine")]);
    listed.inputs = downloaded;
    assert_eq!(
        listed
            .route_assets("package", "x86_64-unknown-linux-gnu", Side::Client)
            .unwrap_err(),
        Refusal::Route("package".into())
    );
}

#[test]
fn how_to_make_an_input_uses_the_game_programs_profile_and_folder() {
    let inputs = inputs();
    for (folder, flag) in [
        ("target/release", "cargo build --release -p horizon-prepare"),
        ("target/debug", "cargo build -p horizon-prepare"),
        (
            "target/fast",
            "cargo build --profile fast -p horizon-prepare",
        ),
        (
            "target/x86_64-pc-windows-gnu/release",
            "cargo build --release -p horizon-prepare",
        ),
        (
            "/opt/horizon",
            "cargo build -p horizon-prepare, with the same profile as the game program",
        ),
    ] {
        let program = Path::new(folder).join("horizon-client");
        assert_eq!(inputs.how("horizon-prepare", &program).unwrap(), flag);
    }
    let program = Path::new("target/release/horizon-client");
    assert_eq!(
        inputs.how("horizon-wasm-std.wasm", program).unwrap(),
        "cargo run --locked -p cargo-horizon-mod -- wasm-std target/release/horizon-wasm-std.wasm"
    );
}

/// The markers around the book's table of required inputs.
const BOOK_START: &str =
    "<!-- required inputs: crates/horizon-mod-package/src/sdk/inputs.json, checked by a test -->";
const BOOK_END: &str = "<!-- end of required inputs -->";

/// The book's table of required inputs, as `inputs.json` says it.
fn book_table(inputs: &Inputs) -> String {
    let mut rows: Vec<(String, &str, Vec<&str>)> = Vec::new();
    for route in ["package", "package-server"] {
        let listed = inputs.route(route).unwrap();
        let files = listed
            .program
            .iter()
            .map(|name| (name, "beside the game program"))
            .chain(listed.checkout.iter().map(|name| (name, "in the checkout")));
        for (name, place) in files {
            match rows.iter_mut().find(|row| &row.0 == name) {
                Some(row) => row.2.push(route),
                None => rows.push((name.clone(), place, vec![route])),
            }
        }
    }
    let mut table = "| File | Where | Needed by | How to make it |\n|---|---|---|---|\n".to_owned();
    for (name, place, routes) in rows {
        let routes: Vec<String> = routes.iter().map(|route| format!("`{route}`")).collect();
        let how = &inputs.how[&name];
        table += &format!("| `{name}` | {place} | {} | {how} |\n", routes.join(", "));
    }
    table
}

#[test]
fn every_named_input_has_a_way_to_make_it_and_the_book_shows_the_list() {
    let inputs = inputs();
    for route in inputs.routes.values() {
        for name in route.program.iter().chain(&route.checkout) {
            assert!(inputs.how.contains_key(name), "{name}");
        }
        for kind in &route.sdk {
            assert!(inputs.how.contains_key(kind.name()), "{}", kind.name());
        }
    }
    let page = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/modding/src/how-to/native-mods/07-local-developer-builds.md"),
    )
    .unwrap()
    .replace("\r\n", "\n");
    let section = page
        .split_once(BOOK_START)
        .and_then(|(_, rest)| rest.split_once(BOOK_END))
        .map(|(section, _)| section.trim())
        .expect("the book page marks its table of required inputs");
    let table = book_table(&inputs);
    assert_eq!(
        section,
        table.trim(),
        "the book's table differs from sdk/inputs.json; it should read:\n{table}"
    );
}

#[cfg(feature = "prebuilt")]
mod unpack {
    use super::*;

    // Fixtures written by Python's `lzma` and `tarfile`, as `tools/prebuilt_game.py` writes the
    // assets: `fixture.tar.xz` is `tar_xz`'s own output (tests/test_prebuilt_game.py checks it);
    // `bomb.tar.xz` is 9.7 KiB holding one 64 MiB file of zeros; `link.tar.xz` holds a file then
    // a symlink; `escape.tar.xz` a file then `../x`.
    const FIXTURE: &[u8] = include_bytes!("fixture.tar.xz");
    const BOMB: &[u8] = include_bytes!("bomb.tar.xz");
    const LINK: &[u8] = include_bytes!("link.tar.xz");
    const ESCAPE: &[u8] = include_bytes!("escape.tar.xz");

    /// Unpack `bytes`, listed as the engine asset, into `<temp>/sdk`; the temp folder too.
    fn unpacked(bytes: &[u8], limit: u64) -> (tempfile::TempDir, Result<(), Refusal>) {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("download");
        std::fs::write(&file, bytes).unwrap();
        let listed = index(&[(ENGINE, bytes)]);
        let result = unpack(&listed, ENGINE, &file, &folder.path().join("sdk"), limit);
        (folder, result)
    }

    /// What is left in the temp folder besides the download.
    fn left(folder: &tempfile::TempDir) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(folder.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            // The lock file stays: removing a lock file another process may hold is racy.
            .filter(|name| name != "download" && !name.ends_with(LOCK_SUFFIX))
            .collect();
        names.sort();
        names
    }

    #[test]
    fn an_archive_the_packager_wrote_unpacks_into_place() {
        let (folder, result) = unpacked(FIXTURE, 1 << 20);
        result.unwrap();
        assert_eq!(
            std::fs::read_to_string(folder.path().join("sdk/deps/a.rmeta")).unwrap(),
            "metadata"
        );
        assert_eq!(left(&folder), ["sdk"]);
    }

    #[test]
    fn nothing_is_decoded_before_the_checksum_matches() {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("download");
        std::fs::write(&file, FIXTURE).unwrap();
        let mut listed = index(&[(ENGINE, FIXTURE)]);
        listed.assets.get_mut(ENGINE).unwrap().sha256 = sha256_hex(b"other");
        assert_eq!(
            unpack(&listed, ENGINE, &file, &folder.path().join("sdk"), 1 << 20).unwrap_err(),
            Refusal::Checksum {
                asset: ENGINE.into()
            }
        );
        assert_eq!(left(&folder), Vec::<String>::new());
    }

    #[test]
    fn a_bomb_is_refused_while_it_streams() {
        let (folder, result) = unpacked(BOMB, 1 << 20);
        assert_eq!(
            result.unwrap_err(),
            Refusal::TooLarge {
                asset: ENGINE.into(),
                limit: 1 << 20,
            }
        );
        assert_eq!(left(&folder), Vec::<String>::new());
        // Fed a byte at a time, the decoder reaches the limit with most of the input unread: it
        // hands decoded bytes on as it goes and never holds the whole output.
        struct Trickle {
            bytes: &'static [u8],
            read: std::rc::Rc<std::cell::Cell<usize>>,
        }
        impl Read for Trickle {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                let at = self.read.get();
                let Some(byte) = self.bytes.get(at) else {
                    return Ok(0);
                };
                buffer[0] = *byte;
                self.read.set(at + 1);
                Ok(1)
            }
        }
        let read = std::rc::Rc::new(std::cell::Cell::new(0));
        let trickle = Trickle {
            bytes: BOMB,
            read: read.clone(),
        };
        let into = tempfile::tempdir().unwrap();
        assert!(matches!(
            extract(ENGINE, trickle, into.path(), 1 << 20),
            Err(Refusal::TooLarge { .. })
        ));
        assert!(
            read.get() < BOMB.len() / 4,
            "{} of {}",
            read.get(),
            BOMB.len()
        );
    }

    #[test]
    fn links_escapes_and_damage_are_refused_and_leave_nothing() {
        for (bytes, why) in [
            (LINK, "link is not a file or folder"),
            (ESCAPE, "../x is not a plain path"),
            (b"not xz".as_slice(), ""),
        ] {
            let (folder, result) = unpacked(bytes, 1 << 20);
            let Err(Refusal::Archive { why: found, .. }) = result else {
                panic!("{why:?}: {result:?}");
            };
            assert!(found.contains(why), "{found}");
            assert_eq!(left(&folder), Vec::<String>::new(), "{why}");
        }
    }

    #[test]
    fn an_existing_destination_is_refused_and_a_crashed_partial_is_replaced() {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("download");
        std::fs::write(&file, FIXTURE).unwrap();
        let listed = index(&[(ENGINE, FIXTURE)]);
        let into = folder.path().join("sdk");
        std::fs::create_dir(&into).unwrap();
        assert_eq!(
            unpack(&listed, ENGINE, &file, &into, 1 << 20).unwrap_err(),
            Refusal::Exists(into.display().to_string())
        );
        std::fs::remove_dir(&into).unwrap();
        let partial = folder.path().join(format!("sdk{PARTIAL_SUFFIX}"));
        std::fs::create_dir_all(partial.join("deps")).unwrap();
        std::fs::write(partial.join("deps/stale"), b"crash").unwrap();
        unpack(&listed, ENGINE, &file, &into, 1 << 20).unwrap();
        assert!(!into.join("deps/stale").exists());
        assert!(into.join("deps/a.rmeta").is_file());
        assert_eq!(left(&folder), ["sdk"]);
    }

    #[test]
    fn a_busy_rename_is_retried_a_bounded_number_of_times() {
        let denied = || std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let tick = std::time::Duration::from_millis(1);
        let held = |error: &std::io::Error| error.kind() == std::io::ErrorKind::PermissionDenied;
        let calls_until = |error: fn() -> std::io::Error, succeed_at: usize| {
            let mut calls = 0;
            let result = rename_retrying(Path::new("a"), Path::new("b"), tick, held, |_, _| {
                calls += 1;
                if calls == succeed_at {
                    Ok(())
                } else {
                    Err(error())
                }
            });
            (calls, result.is_ok())
        };
        assert_eq!(calls_until(denied, 3), (3, true));
        assert_eq!(calls_until(denied, 0), (6, false));
        assert_eq!(
            calls_until(|| std::io::Error::from(std::io::ErrorKind::NotFound), 3),
            (1, false)
        );
    }

    #[test]
    fn only_windows_counts_a_held_file_as_busy() {
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let sharing = std::io::Error::from_raw_os_error(32);
        assert_eq!(busy(&denied), cfg!(windows));
        assert_eq!(busy(&sharing), cfg!(windows));
        assert!(!busy(&std::io::Error::from(std::io::ErrorKind::NotFound)));
    }

    #[test]
    fn a_second_unpack_to_the_same_folder_is_refused_while_the_first_holds_it() {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("download");
        std::fs::write(&file, FIXTURE).unwrap();
        let listed = index(&[(ENGINE, FIXTURE)]);
        let into = folder.path().join("sdk");
        let held = lock(&into, std::fs::File::try_lock).unwrap();
        assert_eq!(
            unpack(&listed, ENGINE, &file, &into, 1 << 20).unwrap_err(),
            Refusal::Busy(into.display().to_string())
        );
        assert!(!into.exists());
        drop(held);
        unpack(&listed, ENGINE, &file, &into, 1 << 20).unwrap();
    }

    #[test]
    fn a_file_system_without_locks_is_refused_naming_the_cause() {
        let folder = tempfile::tempdir().unwrap();
        let into = folder.path().join("sdk");
        let code = if cfg!(windows) {
            50
        } else if cfg!(target_vendor = "apple") {
            45
        } else {
            37
        };
        let refused = lock(&into, |_| {
            Err(std::fs::TryLockError::Error(
                std::io::Error::from_raw_os_error(code),
            ))
        })
        .unwrap_err();
        assert_eq!(refused, Refusal::NoLocks(into.display().to_string()));
        assert!(refused.to_string().contains("does not support file locks"));
        let other = lock(&into, |_| {
            Err(std::fs::TryLockError::Error(std::io::Error::from(
                std::io::ErrorKind::PermissionDenied,
            )))
        })
        .unwrap_err();
        assert!(matches!(other, Refusal::Io(_)), "{other:?}");
    }

    /// A tar of one file entry named `path`, written plain, as a GNU long name, or behind a PAX
    /// header, all by hand (the `tar` crate refuses to write unsafe names).
    fn named(form: &str, path: &str) -> Vec<u8> {
        fn header(name: &[u8], kind: u8, size: usize) -> Vec<u8> {
            let mut h = tar::Header::new_ustar();
            h.as_mut_bytes()[..name.len()].copy_from_slice(name);
            h.set_mode(0o644);
            h.set_size(size as u64);
            h.set_entry_type(tar::EntryType::new(kind));
            h.set_cksum();
            h.as_bytes().to_vec()
        }
        fn padded(out: &mut Vec<u8>, data: &[u8]) {
            out.extend(data);
            out.resize(out.len().div_ceil(512) * 512, 0);
        }
        let mut out = Vec::new();
        let short = match form {
            "plain" => path.as_bytes(),
            "gnu" => {
                let data = format!("{path}\0");
                out.extend(header(b"././@LongLink", b'L', data.len()));
                padded(&mut out, data.as_bytes());
                b"placeholder"
            }
            "pax" => {
                let record = |len: usize| format!("{len} path={path}\n");
                let mut len = record(0).len();
                while record(len).len() != len {
                    len = record(len).len();
                }
                let data = record(len);
                out.extend(header(b"PaxHeaders/x", b'x', data.len()));
                padded(&mut out, data.as_bytes());
                b"placeholder"
            }
            _ => unreachable!(),
        };
        out.extend(header(short, b'0', 1));
        padded(&mut out, b"x");
        out.extend([0u8; 1024]);
        out
    }

    #[test]
    fn every_name_form_writes_inside_and_refuses_each_escape() {
        let into = tempfile::tempdir().unwrap();
        for form in ["plain", "gnu", "pax"] {
            let path = format!("deps/{form}.rmeta");
            untar(ENGINE, &named(form, &path)[..], into.path()).unwrap();
            assert!(into.path().join(&path).is_file(), "{form}");
            for escape in [
                "../x",
                "/etc/x",
                "deps/C:x",
                "deps/a\\..\\..\\x",
                "deps/CON.txt",
                "deps/x.",
            ] {
                let refused = untar(ENGINE, &named(form, escape)[..], into.path());
                assert!(
                    matches!(&refused, Err(Refusal::Archive { why, .. }) if why.contains("not a plain path")),
                    "{form} {escape}: {refused:?}"
                );
            }
        }
        assert!(!into.path().parent().unwrap().join("x").exists());
    }

    #[test]
    fn plain_paths_are_told_from_escapes() {
        let into = Path::new("sdk");
        for path in [
            "deps/a.rmeta",
            "./deps/a",
            "host/lib-x_1.so",
            "com10",
            "com⁴",
            "CONINX",
            "deps/CONSOLE",
        ] {
            assert!(entry_target(into, path.as_bytes()).is_some(), "{path}");
        }
        for path in [
            "..",
            "a/../b",
            "/a",
            "C:x",
            "a\\b",
            "a\0b",
            "CON",
            "con.txt",
            "aux.tar.xz",
            "NUL",
            "COM1",
            "lpt9.dll",
            "CONIN$",
            "conout$.txt",
            "COM¹",
            "lpt³.x",
            "a/b ",
            "a/b.",
        ] {
            assert!(entry_target(into, path.as_bytes()).is_none(), "{path:?}");
        }
    }

    #[test]
    fn unknown_pax_records_and_a_dangling_long_name_are_refused() {
        let into = tempfile::tempdir().unwrap();
        let pax = named("pax", "deps/a").into_iter().collect::<Vec<u8>>();
        let text = String::from_utf8_lossy(&pax[512..1024]).into_owned();
        let record = text.trim_end_matches('\0');
        // The same header with its record turned into a `size` override of the same length.
        let swapped = record.replacen("path=deps/a", "size=999999", 1);
        let mut size = pax.clone();
        size[512..512 + swapped.len()].copy_from_slice(swapped.as_bytes());
        let refused = untar(ENGINE, &size[..], into.path());
        assert!(
            matches!(&refused, Err(Refusal::Archive { why, .. }) if why.contains("PAX record \"size\"")),
            "{refused:?}"
        );
        // A long name and then the end of the archive.
        let mut dangling = named("gnu", "deps/a");
        dangling.truncate(1024);
        dangling.extend([0u8; 1024]);
        let refused = untar(ENGINE, &dangling[..], into.path());
        assert!(
            matches!(&refused, Err(Refusal::Archive { why, .. }) if why.contains("no entry after it")),
            "{refused:?}"
        );
    }

    fn tar_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (path, bytes) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            builder.append_data(&mut header, path, *bytes).unwrap();
        }
        builder.into_inner().unwrap()
    }

    #[test]
    fn long_names_unpack_and_an_oversized_extension_is_refused_before_it_is_read() {
        let long = format!("deps/{}/lib.rmeta", "d".repeat(150));
        let into = tempfile::tempdir().unwrap();
        untar(ENGINE, &tar_bytes(&[(&long, b"long")])[..], into.path()).unwrap();
        assert_eq!(std::fs::read(into.path().join(&long)).unwrap(), b"long");
        // A long-name header declaring 1 GiB, with no data behind it: refused on its size.
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::GNULongName);
        header.set_path("././@LongLink").unwrap();
        header.set_size(1 << 30);
        header.set_cksum();
        let mut bytes = header.as_bytes().to_vec();
        bytes.extend([0u8; 1024]);
        let refused = untar(ENGINE, &bytes[..], into.path());
        assert!(
            matches!(&refused, Err(Refusal::Archive { why, .. }) if why.contains("extension entry")),
            "{refused:?}"
        );
    }

    #[test]
    fn damaged_archives_end_in_a_refusal_never_a_panic() {
        // Truncations at every length, then bit flips at positions from a fixed-seed generator.
        let mut variants: Vec<Vec<u8>> =
            (0..FIXTURE.len()).map(|n| FIXTURE[..n].to_vec()).collect();
        let mut state = 0x2545_f491_4f6c_dd1du64;
        for _ in 0..3000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let mut bytes = FIXTURE.to_vec();
            let at = (state % bytes.len() as u64) as usize;
            bytes[at] ^= 1 << ((state >> 32) % 8);
            variants.push(bytes);
        }
        let folder = tempfile::tempdir().unwrap();
        // Split over the machine's threads: the unit tier keeps each test under 2 s.
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
        let chunk = variants.len().div_ceil(threads);
        let mut variants = variants.into_iter();
        std::thread::scope(|scope| {
            for part in 0..threads {
                // Move the byte buffers: extract owns a 'static reader, but need not clone them.
                let variants: Vec<_> = variants.by_ref().take(chunk).collect();
                if variants.is_empty() {
                    break;
                }
                let folder = folder.path();
                scope.spawn(move || {
                    for (n, bytes) in variants.into_iter().enumerate() {
                        let into = folder.join(format!("{part}-{n}"));
                        std::fs::create_dir(&into).unwrap();
                        // Ok when the flip landed where nothing checks it; otherwise a refusal.
                        let _ = extract(ENGINE, std::io::Cursor::new(bytes), &into, 1 << 20);
                    }
                });
            }
        });
    }

    #[test]
    fn a_plain_asset_is_not_unpacked() {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("surface.json");
        std::fs::write(&file, b"{}").unwrap();
        let listed = index(&[("surface.json", b"{}")]);
        let refused = unpack(
            &listed,
            "surface.json",
            &file,
            &folder.path().join("s"),
            1 << 20,
        );
        assert!(
            matches!(&refused, Err(Refusal::Archive { why, .. }) if why == "not an archive"),
            "{refused:?}"
        );
    }
}
