//! `horizon-stage surface-diff` end to end: exit codes, notes and the gate, on two small
//! surfaces written here.
use mod_surface::{Behaviour, BehaviourContract, Function, FunctionContract, Surface};
use std::path::Path;
use std::process::Output;

fn function(path: &str) -> Function {
    Function::new(
        format!("fn:{path}"),
        "",
        FunctionContract {
            signature: format!("fn {path}()"),
            impl_generics: String::new(),
            wasm: true,
            refs: vec![],
        },
    )
}

fn surface(functions: &[&str]) -> Surface {
    let functions = functions.iter().map(|f| function(f)).collect();
    Surface::new(
        vec![],
        vec![],
        vec![],
        functions,
        vec![],
        vec![],
        vec![],
        vec![],
    )
}

/// A folder with `old.json` (fn:spawn, fn:keep) and `new.json` (fn:keep, fn:summon): one
/// breaking removal and one addition.
fn folder() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("old.json"),
        surface(&["spawn", "keep"]).to_json(),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("new.json"),
        surface(&["keep", "summon"]).to_json(),
    )
    .unwrap();
    dir
}

fn note(dir: &Path, folder: &str, name: &str, text: &str) {
    let notes = dir.join(folder);
    std::fs::create_dir_all(&notes).unwrap();
    std::fs::write(notes.join(name), text).unwrap();
}

fn run(dir: &Path, extra: &[&str]) -> (i32, String, String) {
    let Output {
        status,
        stdout,
        stderr,
    } = std::process::Command::new(env!("CARGO_BIN_EXE_horizon-stage"))
        .current_dir(dir)
        .args(["surface-diff", "old.json", "new.json"])
        .args(extra)
        .output()
        .unwrap();
    (
        status.code().expect("exited, not killed"),
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

#[test]
#[ignore = "daily"]
fn without_the_gate_a_break_reports_and_passes() {
    let dir = folder();
    let (code, out, _) = run(dir.path(), &["--notes", "changes", "--base-notes", "base"]);
    assert_eq!(code, 0);
    assert!(out.starts_with(
        "Surface: breaking (1 breaking, 0 conditional, 1 additive; 1 breaking without a note)"
    ));
    assert!(out.find("- fn:spawn (removed)") < out.find("- fn:summon (added)"));
}

#[test]
#[ignore = "daily"]
fn the_gate_fails_an_undeclared_break_and_passes_a_declared_one() {
    let dir = folder();
    let (code, _, err) = run(dir.path(), &["--gate"]);
    assert_eq!(code, 1);
    assert!(
        err.contains("breaking change without a change note: fn:spawn"),
        "{err}"
    );
    note(
        dir.path(),
        "changes",
        "rename.toml",
        "ids = [\"fn:spawn\", \"fn:summon\"]\nwhy = \"Spawn reads as eggs.\"\nmigrate = \"Call `summon`.\"\n",
    );
    let (code, out, err) = run(
        dir.path(),
        &["--gate", "--notes", "changes", "--base-notes", "base"],
    );
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains("    why (rename.toml): Spawn reads as eggs.\n    migrate: Call `summon`.\n")
    );
}

#[test]
#[ignore = "daily"]
fn a_removed_behaviour_fails_the_gate_without_a_note() {
    let dir = tempfile::tempdir().unwrap();
    let behaviour = Behaviour::new(
        "behaviour:pool_merging",
        "",
        vec!["a.rs::t".into()],
        BehaviourContract { version: 1 },
    );
    std::fs::write(
        dir.path().join("old.json"),
        surface(&[]).with_behaviours(vec![behaviour]).to_json(),
    )
    .unwrap();
    std::fs::write(dir.path().join("new.json"), surface(&[]).to_json()).unwrap();
    let (code, _, err) = run(dir.path(), &["--gate"]);
    assert_eq!(code, 1);
    assert!(
        err.contains("breaking change without a change note: behaviour:pool_merging"),
        "{err}"
    );
    note(
        dir.path(),
        "changes",
        "pool-merging.toml",
        "ids = [\"behaviour:pool_merging\"]\nwhy = \"Additions are no longer merged.\"\n",
    );
    let (code, _, err) = run(
        dir.path(),
        &["--gate", "--notes", "changes", "--base-notes", "base"],
    );
    assert_eq!(code, 0, "{err}");
}

#[test]
#[ignore = "daily"]
fn a_new_note_naming_an_unchanged_id_fails_and_an_old_or_edited_one_is_ignored() {
    let dir = folder();
    let stale = "ids = [\"fn:keep\"]\nwhy = \"Nothing.\"\n";
    note(dir.path(), "changes", "stale.toml", stale);
    let (code, _, err) = run(dir.path(), &["--notes", "changes", "--base-notes", "base"]);
    assert_eq!(code, 1);
    assert!(
        err.contains("change note stale.toml names fn:keep, which did not change"),
        "{err}"
    );
    // The same file in the base tree: an earlier change's note, neither checked nor attached.
    note(dir.path(), "base", "stale.toml", stale);
    let (code, out, err) = run(dir.path(), &["--notes", "changes", "--base-notes", "base"]);
    assert_eq!(code, 0, "{err}");
    assert!(!out.contains("Nothing."));
    // Edited since the base: still an earlier change's note, so editing it never fails.
    note(
        dir.path(),
        "changes",
        "stale.toml",
        "ids = [\"fn:keep\"]\nwhy = \"Edited.\"\n",
    );
    let (code, ..) = run(dir.path(), &["--notes", "changes", "--base-notes", "base"]);
    assert_eq!(code, 0);
    // Notes without the base's notes cannot tell new from old.
    let (code, _, err) = run(dir.path(), &["--notes", "changes"]);
    assert_eq!(code, 2);
    assert!(err.contains("--notes needs --base-notes"), "{err}");
}

#[test]
#[ignore = "daily"]
fn a_malformed_note_or_surface_is_refused_by_name() {
    let dir = folder();
    note(
        dir.path(),
        "changes",
        "bad.toml",
        "ids = [\"fn:spawn\"]\nwhy = \"x\"\nreason = \"y\"\n",
    );
    let (code, _, err) = run(dir.path(), &["--notes", "changes", "--base-notes", "base"]);
    assert_eq!(code, 2);
    assert!(
        err.contains("change note bad.toml has the unknown key `reason`"),
        "{err}"
    );
    std::fs::write(dir.path().join("new.json"), "{\"schema\": 2, \"hooks\": [").unwrap();
    let (code, _, err) = run(dir.path(), &[]);
    assert_eq!(code, 2);
    assert!(
        err.contains("new.json: surface.json is not a valid surface"),
        "{err}"
    );
}

#[test]
#[ignore = "daily"]
fn the_json_report_is_written() {
    let dir = folder();
    let (code, ..) = run(dir.path(), &["--json", "diff.json"]);
    assert_eq!(code, 0);
    let diff: mod_surface::Diff =
        serde_json::from_slice(&std::fs::read(dir.path().join("diff.json")).unwrap()).unwrap();
    assert_eq!(diff.verdict, mod_surface::Verdict::Breaking);
    assert_eq!(diff.changes.len(), 2);
}
