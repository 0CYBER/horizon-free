//! Loads real mods built with the SDK: the coords-hud example and a guest
//! that misbehaves on command. Needs `rustup target add wasm32-unknown-unknown`.
use horizon_mods::{
    Clock, Corner, Mods, Output, Package, Player, Snapshot, Status, manifest::componentize,
};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
    time::Instant,
};

const GUEST_MANIFEST: &str = r#"
id = "test_guest"
name = "Test Guest"
version = "0.1.0"
api = "0.1"
capabilities = ["chat", "storage"]
"#;

/// Builds both mods once and writes their `.hmod` files; returns the folder.
fn mods_dir() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let target = root.join("target/mods-test");
        let status = Command::new(env!("CARGO"))
            .current_dir(&root)
            .args(["build", "--release", "--target", "wasm32-unknown-unknown"])
            .args([
                "-p",
                "coords-hud",
                "-p",
                "horizon-mods-test-guest",
                "--target-dir",
            ])
            .arg(&target)
            .status()
            .expect("cargo runs");
        assert!(
            status.success(),
            "building the test mods needs `rustup target add wasm32-unknown-unknown`"
        );
        let built = target.join("wasm32-unknown-unknown/release");
        let dir = target.join("mods");
        std::fs::create_dir_all(&dir).unwrap();
        let example = root.join("crates/horizon-mod-sdk/examples/coords-hud/horizon-mod.toml");
        let mods = [
            ("coords_hud", std::fs::read_to_string(example).unwrap()),
            ("horizon_mods_test_guest", GUEST_MANIFEST.to_string()),
        ];
        for (name, manifest) in mods {
            let core = std::fs::read(built.join(format!("{name}.wasm"))).unwrap();
            let hmod = Package::write(&manifest, &componentize(&core).unwrap()).unwrap();
            std::fs::write(dir.join(format!("{name}.hmod")), hmod).unwrap();
        }
        dir
    })
}

fn load(name: &str) -> Mods {
    Mods::load(&[mods_dir().join(name)], |_| true, |c| c == "give")
}

fn player() -> Snapshot {
    Snapshot {
        player: Some(Player {
            x: 12.5,
            y: 64.0,
            z: -3.25,
            yaw: 180.0,
            pitch: 0.0,
            health: 20.0,
            food: 20,
            dimension: "minecraft:overworld".into(),
        }),
        clock: Some(Clock {
            game_time: 100,
            day_time: 24000 + 6000,
        }),
    }
}

#[test]
fn coords_hud_draws_and_answers() {
    let mut mods = load("coords_hud.hmod");
    assert_eq!(mods.entries()[0].status, Status::Running);
    assert_eq!(mods.commands().collect::<Vec<_>>(), ["where", "coords"]);
    mods.set_snapshot(player());
    mods.tick(1);
    let panel = mods.panels().next().expect("a panel after a tick");
    assert_eq!(panel.corner, Corner::TopLeft);
    let lines: Vec<&str> = panel.lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(
        lines,
        [
            "XYZ 12.5 / 64.0 / -3.2",
            "Facing North (-Z)",
            "Day 2, 12:00"
        ]
    );

    assert!(mods.command("where"));
    let chat = Output::Chat {
        name: "Coordinates HUD".into(),
        text: "12, 64, -4 in minecraft:overworld".into(),
    };
    assert_eq!(mods.drain(), [chat]);
    assert!(mods.command("coords"));
    mods.tick(2);
    assert_eq!(mods.panels().count(), 0);
    let hidden = Output::Feedback {
        text: "Coordinates panel hidden".into(),
        error: false,
    };
    assert_eq!(mods.drain(), [hidden]);
    assert!(!mods.command("gamemode creative"));
}

#[test]
fn off_mods_cost_nothing() {
    let mods = Mods::load(&[mods_dir().join("coords_hud.hmod")], |_| false, |_| false);
    assert_eq!(mods.entries()[0].status, Status::Off);
    assert!(!mods.is_running());
    assert_eq!(mods.commands().count(), 0);
}

#[test]
fn undeclared_capabilities_are_refused() {
    let dir = mods_dir().join("undeclared");
    std::fs::create_dir_all(&dir).unwrap();
    let wasm = Package::read(&mods_dir().join("horizon_mods_test_guest.hmod"))
        .unwrap()
        .wasm
        .expect("the guest has code");
    let manifest = GUEST_MANIFEST.replace("\"chat\", ", "");
    std::fs::write(dir.join("horizon-mod.toml"), manifest).unwrap();
    std::fs::write(dir.join("mod.wasm"), wasm).unwrap();
    let mods = Mods::load(&[dir], |_| true, |_| false);
    let refused = Status::Failed("uses capability chat, missing from horizon-mod.toml".into());
    assert_eq!(mods.entries()[0].status, refused);
}

#[test]
fn vanilla_commands_win() {
    let mods = Mods::load(
        &[mods_dir().join("coords_hud.hmod")],
        |_| true,
        |c| c == "where",
    );
    assert_eq!(mods.commands().collect::<Vec<_>>(), ["coords"]);
}

#[test]
fn a_misbehaving_mod_is_stopped_alone() {
    for (command, reason) in [
        ("spin", "it took too long (an endless loop?)"),
        ("panic", "it crashed (a panic)"),
        ("grow", "it crashed (a panic)"),
    ] {
        let paths = [
            mods_dir().join("coords_hud.hmod"),
            mods_dir().join("horizon_mods_test_guest.hmod"),
        ];
        let mut mods = Mods::load(&paths, |_| true, |_| false);
        let started = Instant::now();
        assert!(mods.command(command));
        assert!(started.elapsed().as_secs() < 5, "/{command} was cut short");
        let stopped = Output::Stopped {
            name: "Test Guest".into(),
            reason: reason.into(),
        };
        assert_eq!(mods.drain(), [stopped], "/{command}");
        assert_eq!(mods.entries()[1].status, Status::Failed(reason.into()));
        // The other mod runs on, and the stopped one's command is gone.
        assert!(mods.is_running());
        assert!(!mods.command("hello"));
        mods.set_snapshot(player());
        mods.tick(1);
        assert_eq!(mods.panels().count(), 1);
    }
}

#[test]
fn hot_reload_restarts_a_changed_mod() {
    let dir = mods_dir().join("reload");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("guest.hmod");
    std::fs::copy(mods_dir().join("horizon_mods_test_guest.hmod"), &path).unwrap();
    let mut mods = Mods::load(std::slice::from_ref(&path), |_| true, |_| false);
    mods.tick(1);
    assert!(!mods.reload_changed(|_| false));
    // A rebuild: same bytes, newer file.
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(later)
        .unwrap();
    assert!(mods.reload_changed(|_| false));
    assert!(mods.command("hello world"));
    let chat = Output::Chat {
        name: "Test Guest".into(),
        text: "hello world after 0 ticks".into(),
    };
    assert_eq!(mods.drain(), [chat]);
}

#[test]
fn storage_persists_across_reload() {
    let dir = mods_dir().join("storage_reload");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("guest.hmod");
    std::fs::copy(mods_dir().join("horizon_mods_test_guest.hmod"), &path).unwrap();
    let mut mods = Mods::load(std::slice::from_ref(&path), |_| true, |_| false);
    assert!(mods.command("save remembered"));
    let saved = Output::Feedback {
        text: "saved".into(),
        error: false,
    };
    assert_eq!(mods.drain(), [saved]);

    // A rebuild (hot_reload_restarts_a_changed_mod's pattern): a fresh Store,
    // but the same on-disk data directory.
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(later)
        .unwrap();
    assert!(mods.reload_changed(|_| false));

    assert!(mods.command("load"));
    let chat = Output::Chat {
        name: "Test Guest".into(),
        text: "remembered".into(),
    };
    assert_eq!(mods.drain(), [chat]);
}

#[test]
fn storage_write_budget_limits_one_call() {
    let dir = mods_dir().join("storage_budget");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("guest.hmod");
    std::fs::copy(mods_dir().join("horizon_mods_test_guest.hmod"), &path).unwrap();
    let mut mods = Mods::load(std::slice::from_ref(&path), |_| true, |_| false);
    // Under the 4 MiB on-disk quota, but over the 1 MiB a single call may
    // write: a loop replacing one key can never bypass the quota by
    // writing a lot in one call, only across many.
    let over_budget = "a".repeat((1 << 20) + 1024);
    assert!(mods.command(&format!("save {over_budget}")));
    let refused = Output::Feedback {
        text: "too much storage traffic in one call".into(),
        error: true,
    };
    assert_eq!(mods.drain(), [refused]);
}

#[test]
fn storage_quota_is_enforced() {
    let dir = mods_dir().join("storage_quota");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("guest.hmod");
    std::fs::copy(mods_dir().join("horizon_mods_test_guest.hmod"), &path).unwrap();
    let mut mods = Mods::load(std::slice::from_ref(&path), |_| true, |_| false);
    // Each call is under the per-call write budget; distinct keys so the
    // quota (not the fixed "note" key being overwritten) is what's tested.
    let chunk = "a".repeat(900 << 10);
    for i in 0..4 {
        assert!(mods.command(&format!("save_key k{i} {chunk}")));
        assert_eq!(
            mods.drain(),
            [Output::Feedback {
                text: "saved".into(),
                error: false,
            }]
        );
    }
    assert!(mods.command(&format!("save_key k4 {chunk}")));
    let refused = Output::Feedback {
        text: "would use over 4 MiB of storage".into(),
        error: true,
    };
    assert_eq!(mods.drain(), [refused]);
}

#[test]
fn storage_key_count_is_bounded() {
    let dir = mods_dir().join("storage_keys");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("guest.hmod");
    std::fs::copy(mods_dir().join("horizon_mods_test_guest.hmod"), &path).unwrap();
    let mut mods = Mods::load(std::slice::from_ref(&path), |_| true, |_| false);
    for i in 0..64 {
        assert!(mods.command(&format!("save_key k{i} x")));
        assert_eq!(
            mods.drain(),
            [Output::Feedback {
                text: "saved".into(),
                error: false,
            }],
            "key {i}"
        );
    }
    assert!(mods.command("save_key k64 x"));
    let refused = Output::Feedback {
        text: "would hold over 64 keys".into(),
        error: true,
    };
    assert_eq!(mods.drain(), [refused]);
}

#[test]
fn duplicate_mod_ids_are_refused() {
    let dir = mods_dir().join("duplicate_ids");
    std::fs::create_dir_all(&dir).unwrap();
    let first = dir.join("first.hmod");
    let second = dir.join("second.hmod");
    std::fs::copy(mods_dir().join("horizon_mods_test_guest.hmod"), &first).unwrap();
    std::fs::copy(mods_dir().join("horizon_mods_test_guest.hmod"), &second).unwrap();
    // Both declare id "test_guest" (the guest's own manifest): sharing a
    // `data_dir` would let the second read, overwrite or delete the
    // first's storage, so only the first-loaded one may start.
    let mods = Mods::load(&[first, second], |_| true, |_| false);
    assert_eq!(mods.entries()[0].status, Status::Running);
    let refused = Status::Failed("another running mod already uses id test_guest".into());
    assert_eq!(mods.entries()[1].status, refused);
}

/// `cargo test -p horizon-mods --release -- --ignored --nocapture`: the
/// cost of the per-tick and per-frame hooks (docs/mods.md, Performance).
#[test]
#[ignore]
fn hook_overhead() {
    let time = |mods: &mut Mods, frame: bool| {
        let n = 200_000;
        for i in 0..1000 {
            mods.tick(i);
        }
        let started = Instant::now();
        for i in 0..n {
            if frame {
                mods.frame(0.016);
            } else {
                mods.tick(i);
            }
        }
        started.elapsed().as_nanos() as f64 / n as f64
    };
    let mut guest = load("horizon_mods_test_guest.hmod");
    let mut hud = load("coords_hud.hmod");
    hud.set_snapshot(player());
    let mut none = Mods::default();
    println!("no mods, tick:        {:8.1} ns", time(&mut none, false));
    println!("empty tick hook:      {:8.1} ns", time(&mut guest, false));
    println!("empty frame hook:     {:8.1} ns", time(&mut guest, true));
    println!("coords-hud tick hook: {:8.1} ns", time(&mut hud, false));
    let started = Instant::now();
    let _ = load("coords_hud.hmod");
    println!(
        "load coords-hud:      {:8.1} ms",
        started.elapsed().as_secs_f64() * 1000.0
    );
}

#[test]
fn content_only_mods_load_without_an_engine() {
    let manifest = r#"
id = "ruby"
name = "Ruby"
version = "0.1.0"
api = "0.2"
side = "both"
capabilities = ["registries"]
"#;
    let block: &[u8] = br#"{"template": "minecraft:iron_block"}"#;
    let dir = mods_dir().join("content");
    std::fs::create_dir_all(&dir).unwrap();
    let ok = dir.join("ruby.hmod");
    let files: &[(&str, &[u8])] = &[("data/ruby/horizon/block/ruby_block.json", block)];
    std::fs::write(&ok, Package::write_files(manifest, None, files).unwrap()).unwrap();
    let bad = dir.join("bad.hmod");
    let files: &[(&str, &[u8])] = &[("data/minecraft/recipe/stone.json", b"{}")];
    let bad_manifest = manifest.replace("\"ruby\"", "\"bad\"");
    std::fs::write(
        &bad,
        Package::write_files(&bad_manifest, None, files).unwrap(),
    )
    .unwrap();
    let mods = Mods::load(&[ok, bad], |_| true, |_| false);
    assert_eq!(mods.entries()[0].status, Status::Running);
    assert!(matches!(&mods.entries()[1].status, Status::Failed(e) if e.contains("own namespace")));
    // Content has no hooks: the game's per-tick mod work stays skipped.
    assert!(!mods.is_running());
}
