//! Exercise both commands without downloads, using each pin's cached build data.
use horizon_mod_package::{Manifest, manifest::MANIFEST};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};

const OLD: &str = "0123456789abcdef0123456789abcdef01234567";
const NEW: &str = "89abcdef0123456789abcdef0123456789abcdef";
const OLD_CATALOG: &str = include_str!("fixtures/internals-old.json");
const NEW_CATALOG: &str = include_str!("fixtures/internals-new.json");

struct Project {
    dir: PathBuf,
}
impl Project {
    fn new(name: &str, declarations: &str) -> Self {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/i3-d/tests")
            .join(format!("internals-{name}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(MANIFEST), format!("# A modder's description\nid = 'example'\nname = 'Example'\nversion = '0.1.0'\napi = '0.2'\ncapabilities = ['registries']\n{declarations}\n[native] # native settings\ngame = '26.3'\nhorizon = '{OLD}' # pinned game\nmixins = '{}'\n", "a".repeat(40))).unwrap();
        fs::write(dir.join("Cargo.toml"), "# Keep this comment\n[package]\nname = 'example'\nversion = '0.1.0'\n[dependencies]\nhorizon-mod-api = { path = '.horizon/sdk/api/crates/horizon-mod-api' } # API pin\nhorizon-internals = { path = '.horizon/sdk/api/crates/horizon-internals' } # facade pin\n").unwrap();
        let project = Self { dir };
        project.catalog(OLD, OLD_CATALOG);
        project.catalog(NEW, NEW_CATALOG);
        project
    }
    fn catalog(&self, revision: &str, text: &str) {
        // A verified fetch leaves the build's stamp and its unpacked build data.
        let build = self.dir.join(".horizon/cache").join(revision);
        let dir = build.join("build-data");
        fs::create_dir_all(&dir).unwrap();
        fs::write(build.join("build"), revision).unwrap();
        // A bare entry list gets the build-data header of `revision`.
        let text = if text.trim_start().starts_with('[') {
            format!(r#"{{"schema":1,"build":{{"commit":"{revision}"}},"internals":{text}}}"#)
        } else {
            text.to_owned()
        };
        fs::write(dir.join("internals.json"), text).unwrap();
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_cargo-horizon-mod"))
            .arg("horizon-mod")
            .args(args)
            .current_dir(&self.dir)
            // An SDK base with no releases: nothing is downloaded.
            .env("HORIZON_SDK_BASE", self.dir.join("no-sdk"))
            .output()
            .unwrap()
    }
    fn text(&self) -> String {
        fs::read_to_string(self.dir.join(MANIFEST)).unwrap()
    }
    fn manifest(&self) -> Manifest {
        Manifest::parse(&self.text()).unwrap()
    }
}
impl Drop for Project {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.dir).unwrap();
    }
}

fn success(output: &Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

#[test]
#[ignore = "daily"]
fn internals_add_writes_pinned_hashes_and_preserves_the_rest_of_the_manifest() {
    let project = Project::new("add", "");
    let output = project.run(&[
        "internals",
        "add",
        "horizon_client::game::Game",
        "horizon_client::game::debug",
        "horizon_client::game::Game",
    ]);
    success(&output);
    let manifest = project.manifest();
    assert_eq!(manifest.internals.len(), 2);
    assert_eq!(manifest.internals[0].hash.as_deref(), Some("1111"));
    // A guarded alternative before the unconditional entry must not shadow it.
    assert_eq!(manifest.internals[1].hash.as_deref(), Some("7777"));
    assert!(
        project
            .text()
            .starts_with("# A modder's description\nid = 'example'")
    );
    assert!(
        project
            .text()
            .contains("[native] # native settings\ngame = '26.3'")
    );
    let before = project.text();
    success(&project.run(&["internals", "add", "horizon_client::game::Game"]));
    assert_eq!(project.text(), before);
}

#[test]
#[ignore = "daily"]
fn internals_commands_refuse_wasm_mods_without_edits() {
    let project = Project::new("wasm-refused", "");
    let manifest = format!(
        "id='example'\nname='Example'\nversion='0.1.0'\napi='0.2'\ncapabilities=['registries']\n[wasm]\nsha512='{}'\ntoolchain='nightly'\n",
        "a".repeat(128)
    );
    fs::write(project.dir.join(MANIFEST), &manifest).unwrap();
    for args in [
        &["internals", "add", "Example"][..],
        &["internals", "sync"][..],
    ] {
        let output = project.run(args);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("needs a [native] mod"));
        assert_eq!(project.text(), manifest);
    }
}

#[test]
#[ignore = "daily"]
fn native_sync_refuses_repo() {
    let project = Project::new("sync-native-repo", "");
    let output = project.run(&["internals", "sync", "--repo", "."]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("usage:"));
}

#[test]
#[ignore = "daily"]
fn internals_add_refuses_missing_gated_and_shape_only_paths_without_partial_edits() {
    let project = Project::new("refusals", "");
    for (path, code, wording) in [
        (
            "horizon_client::game::no_such_function",
            "E_INTERNAL_MISSING",
            "which this game doesn't have; run cargo horizon-mod internals sync",
        ),
        (
            "horizon_client::pad::hardware::PadId",
            "E_INTERNAL_GATED",
            "which exists only under a cfg; target-specific internals aren't supported.",
        ),
        (
            "horizon_world::World",
            "E_INTERNAL_MISSING",
            "which this game doesn't have; run cargo horizon-mod internals sync",
        ),
    ] {
        let before = project.text();
        let output = project.run(&["internals", "add", "horizon_client::game::Game", path]);
        assert!(!output.status.success());
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(
            error.contains(&format!("{code}: example uses {path}, {wording}")),
            "{error}"
        );
        assert_eq!(project.text(), before);
    }
    for args in [
        vec!["internals"],
        vec!["internals", "add"],
        vec!["internals", "remove", "path"],
        vec!["internals", "add", "path", "--kind"],
        vec![
            "internals",
            "add",
            "path",
            "--kind",
            "fn",
            "--kind",
            "field",
        ],
        vec!["internals", "add", "path", "--unknown"],
    ] {
        assert!(!project.run(&args).status.success());
    }
}

#[test]
#[ignore = "daily"]
fn update_diffs_signatures_refreshes_present_hashes_and_keeps_missing_declarations() {
    let project = Project::new(
        "update",
        "internals = [\n  { path = 'horizon_client::game::Game', hash = '1111' }, # stable\n  { path = \"horizon_client::game::Game<'_>::reach\" }, # reach\n  { path = 'horizon_client::game::Game::creative', hash = '3333' }, # removed\n  { path = 'horizon_client::game::conditional', hash = '8888' }, # gated now\n  { path = 'horizon_client::game::restored', hash = '0000' }, # back again\n]\n",
    );
    let report = success(&project.run(&["update", "--horizon", NEW]));
    assert!(
        report.contains("unchanged: horizon_client::game::Game\n"),
        "{report}"
    );
    assert!(report.contains("changed: horizon_client::game::Game<'_>::reach\n  impl Game<'_> { fn reach(&self) -> f64 } → impl Game<'_> { fn reach(&self, hand: Hand) -> f64 }\n"), "{report}");
    assert!(
        report.contains("missing: horizon_client::game::Game::creative (E_INTERNAL_MISSING:"),
        "{report}"
    );
    assert!(
        report.contains("missing: horizon_client::game::conditional (E_INTERNAL_GATED:"),
        "{report}"
    );
    assert!(
        report.contains("<unavailable in old game> → const restored: bool = true"),
        "{report}"
    );
    let manifest = project.manifest();
    let native = manifest.native.unwrap();
    assert_eq!(native.horizon, NEW);
    assert_eq!(
        manifest
            .internals
            .iter()
            .map(|i| i.hash.as_deref().unwrap_or_default())
            .collect::<Vec<_>>(),
        // `reach` has no hash: it follows the pin, reported but left without one.
        ["1111", "", "3333", "8888", "9999"]
    );
    for comment in [
        "# stable",
        "# reach",
        "# removed",
        "# gated now",
        "# back again",
        "# pinned game",
    ] {
        assert!(project.text().contains(comment), "{comment}");
    }
    // The SDK is named by path: only the manifest's pin moves.
    let cargo = fs::read_to_string(project.dir.join("Cargo.toml")).unwrap();
    assert!(!cargo.contains(NEW), "{cargo}");
    assert!(cargo.starts_with("# Keep this comment"));
    assert!(cargo.contains("# API pin"));
    assert!(cargo.contains("# facade pin"));
}

#[test]
#[ignore = "daily"]
fn table_declarations_keep_their_format_when_adding_and_refreshing() {
    let project = Project::new(
        "tables",
        "\n[[internals]] # the game\npath = 'horizon_client::game::Game'\nhash = 'stale' # fingerprint\n",
    );
    success(&project.run(&[
        "internals",
        "add",
        "horizon_client::game::Game",
        "horizon_client::game::Game::creative",
    ]));
    assert_eq!(
        project.manifest().internals[0].hash.as_deref(),
        Some("1111")
    );
    assert!(project.text().contains("[[internals]] # the game"));
    assert!(project.text().contains("# fingerprint"));
    let report = success(&project.run(&["update", "--horizon", NEW]));
    assert!(report.contains("unchanged: horizon_client::game::Game"));
    assert_eq!(project.manifest().internals.len(), 2);
    assert!(project.text().contains("# fingerprint"));
}

#[test]
#[ignore = "daily"]
fn internals_add_requires_kind_for_ambiguous_paths_and_update_keeps_it() {
    let path = "horizon_client::game::Round::listening";
    for kind in ["fn", "method"] {
        for (form, declarations) in [
            ("inline", String::new()),
            (
                "existing",
                format!("internals = [{{path='{path}',hash='stale'}}]\n"),
            ),
            (
                "tables",
                format!("\n[[internals]]\npath='{path}'\nhash='stale'\n"),
            ),
        ] {
            let project = Project::new(&format!("ambiguous-{kind}-{form}"), &declarations);
            let catalog = serde_json::json!([
                { "path": path, "kind": "field", "signature": "bool", "hash": "field" },
                { "path": path, "kind": kind, "signature": "fn listening(&self) -> bool", "hash": "function" }
            ]);
            project.catalog(OLD, &catalog.to_string());
            let mut new = catalog.clone();
            new[1]["hash"] = "new-function".into();
            new[1]["signature"] = "fn listening(&mut self) -> bool".into();
            project.catalog(NEW, &new.to_string());
            let before = project.text();
            let output = project.run(&["internals", "add", path]);
            assert!(!output.status.success());
            let error = String::from_utf8(output.stderr).unwrap();
            assert!(error.contains("E_INTERNAL_AMBIGUOUS"), "{error}");
            assert!(error.contains(path), "{error}");
            assert!(error.contains(&format!("field, {kind}")), "{error}");
            assert_eq!(project.text(), before);
            success(&project.run(&["internals", "add", path, "--kind", kind]));
            let declaration = &project.manifest().internals[0];
            assert_eq!(declaration.kind.as_deref(), Some(kind));
            assert_eq!(declaration.hash.as_deref(), Some("function"));
            success(&project.run(&["internals", "add", path, "--kind", "field"]));
            let before = project.text();
            success(&project.run(&["internals", "add", "--kind", kind, path]));
            assert_eq!(project.text(), before);
            let report = success(&project.run(&["update", "--horizon", NEW]));
            assert!(
                report.contains("fn listening(&self) -> bool → fn listening(&mut self) -> bool"),
                "{report}"
            );
            let manifest = project.manifest();
            assert_eq!(manifest.internals.len(), 2);
            assert_eq!(manifest.internals[0].kind.as_deref(), Some(kind));
            assert_eq!(manifest.internals[0].hash.as_deref(), Some("new-function"));
            assert_eq!(manifest.internals[1].kind.as_deref(), Some("field"));
            assert_eq!(manifest.internals[1].hash.as_deref(), Some("field"));
        }
    }
}

#[test]
#[ignore = "daily"]
fn unreadable_new_catalog_does_not_move_either_pin_or_hashes() {
    let project = Project::new(
        "bad-catalog",
        "internals = [{ path = 'horizon_client::game::Game', hash = '1111' }]\n",
    );
    project.catalog(NEW, "not JSON");
    let manifest = project.text();
    let cargo = fs::read_to_string(project.dir.join("Cargo.toml")).unwrap();
    let output = project.run(&["update", "--horizon", NEW]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("internals.json"));
    assert_eq!(project.text(), manifest);
    assert_eq!(
        fs::read_to_string(project.dir.join("Cargo.toml")).unwrap(),
        cargo
    );
}

#[cfg(unix)]
#[test]
#[ignore = "fixture writer subprocess only"]
fn write_launcher_actor() {
    use std::os::unix::fs::PermissionsExt;
    if let Some(destination) = std::env::var_os("HORIZON_TEST_LAUNCHER_WRITER") {
        use std::io::Write;
        let mut file = fs::File::create(&destination).unwrap();
        file.write_all(
            std::env::var("HORIZON_TEST_LAUNCHER_SCRIPT")
                .unwrap()
                .as_bytes(),
        )
        .unwrap();
        file.sync_all().unwrap();
        drop(file);
        fs::set_permissions(destination, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn internals_sync_invokes_the_launcher_and_never_edits_on_failure() {
    let project = Project::new(
        "sync-command",
        "internals = [{ path = 'Unused', hash = 'old' }]\n",
    );
    let launcher = project.dir.join("test-launcher");
    let invoke = || {
        Command::new(env!("CARGO_BIN_EXE_cargo-horizon-mod"))
            .args(["horizon-mod", "internals", "sync"])
            .env("HORIZON_LAUNCHER", &launcher)
            .current_dir(&project.dir)
            .output()
            .unwrap()
    };
    // A separate, exact-test writer cannot leak writable fds into other tests'
    // fork children. CLOEXEC would only close those descriptors at exec.
    let write_launcher = |script: &str| {
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "write_launcher_actor", "--ignored"])
            .env("HORIZON_TEST_LAUNCHER_WRITER", &launcher)
            .env("HORIZON_TEST_LAUNCHER_SCRIPT", script)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(
            String::from_utf8(output.stdout)
                .unwrap()
                .lines()
                .any(|line| {
                    line.starts_with("test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; ")
                })
        );
    };
    write_launcher("#!/bin/sh\nexit 1\n");
    let before = project.text();
    assert!(!invoke().status.success());
    assert_eq!(project.text(), before);
    write_launcher(
        r#"#!/bin/sh
[ "$1" = native-build ] && [ "$2" = --sync-internals ] && [ "$3" = --trust-native-code ] && [ "$4" = "$PWD" ] || exit 2
printf '%s\n' '[{"path":"Used","kind":"struct","signature":"struct Used","hash":"compiler"}]'
"#,
    );
    let report = success(&invoke());
    assert!(report.contains("added: Used (struct)"));
    assert!(report.contains("removed: Unused"));
    let manifest = project.manifest();
    assert_eq!(manifest.internals.len(), 1);
    assert_eq!(manifest.internals[0].hash.as_deref(), Some("compiler"));
    assert!(project.text().contains("# native settings"));
    let before = project.text();
    write_launcher("#!/bin/sh\nprintf 'not JSON'\n");
    assert!(!invoke().status.success());
    assert_eq!(project.text(), before);
}
