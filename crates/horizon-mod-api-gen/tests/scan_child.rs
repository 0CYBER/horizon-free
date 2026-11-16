//! The uses scan's child process, run as `horizon-stage` starts it: a parse that overflows its
//! stack ends the child, which the caller reads as "cannot judge", and the caller lives on.
use mod_surface::Surface;
use mod_surface::child::{CHILD_VAR, Contained, parse_in_child};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

fn program() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_horizon-stage"))
}

/// A chain of qualified paths after a comparison, which the parser reads by recursion.
fn qualified_paths(depth: usize) -> String {
    format!(
        "fn f() {{ let x = 1 < {}T{}; }}",
        "<".repeat(depth),
        " as A>::B".repeat(depth)
    )
}

#[test]
#[ignore = "daily"]
fn a_parse_that_overflows_ends_the_child_and_names_the_file() {
    let fine = "fn f() { let x = (1, [2]); }";
    let deep = qualified_paths(50_000);
    let wait = Duration::from_secs(60);
    assert_eq!(
        parse_in_child(program(), &[fine, &deep], 256 << 10, wait),
        Err(Contained::Died(Some(1)))
    );
    assert_eq!(
        parse_in_child(program(), &[fine, fine], 256 << 10, wait),
        Ok(())
    );
    // A child given no time is stopped, not waited for.
    assert!(matches!(
        parse_in_child(program(), &[&deep], 64 << 20, Duration::ZERO),
        Err(Contained::TimedOut(_))
    ));
    // A program that is not there starts no child.
    assert_eq!(
        parse_in_child(
            Path::new("/nonexistent/horizon-stage"),
            &[fine],
            256 << 10,
            wait
        ),
        Err(Contained::NoChild)
    );
}

/// The exit status of a child given `input` on stdin.
fn child_status(input: &[u8]) -> Option<i32> {
    let mut child = Command::new(program())
        .env(CHILD_VAR, "1048576")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // The child may exit before it reads everything.
    let _ = child.stdin.take().unwrap().write_all(input);
    child.wait().unwrap().code()
}

#[test]
#[ignore = "daily"]
fn the_child_refuses_input_that_is_not_a_scans() {
    assert_eq!(child_status(b"uses src\n"), Some(3));
    // One source, framed as the parent frames it, is parsed; a byte after it is refused.
    let source = b"fn f() {}";
    let mut input = b"horizon-mod-scan 1\n".to_vec();
    input.extend(1u64.to_le_bytes());
    input.extend((source.len() as u64).to_le_bytes());
    input.extend(source);
    assert_eq!(child_status(&input), Some(0));
    input.push(b'x');
    assert_eq!(child_status(&input), Some(3));
}

#[test]
#[ignore = "daily"]
fn a_program_that_is_not_a_scans_child_is_stopped() {
    // This test binary never calls the child entry: started as a child, it prints its own
    // output first, and the parent stops it.
    let this = std::env::current_exe().unwrap();
    assert_eq!(
        parse_in_child(&this, &["fn f() {}"], 1 << 20, Duration::from_secs(60)),
        Err(Contained::NoChild)
    );
}

#[test]
#[ignore = "daily"]
fn horizon_stage_scans_through_its_child() {
    let dir = tempfile::tempdir().unwrap();
    let surface = dir.path().join("surface.json");
    let empty = Surface::new(
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
    );
    std::fs::write(&surface, empty.to_json()).unwrap();
    let scan = |source: &str| {
        let folder = tempfile::tempdir().unwrap();
        std::fs::create_dir(folder.path().join("src")).unwrap();
        std::fs::write(folder.path().join("src/lib.rs"), source).unwrap();
        let output = Command::new(program())
            .arg("uses")
            .arg(&surface)
            .arg(folder.path())
            .output()
            .unwrap();
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };
    let (status, said) = scan("fn f() { let x = (1, [2]); }");
    assert_eq!(status, Some(0), "{said}");
    // Past the nesting limit, the scan cannot judge, and says where.
    let (status, said) = scan(&qualified_paths(50_000));
    assert_eq!(status, Some(2));
    assert!(said.contains("src/lib.rs:1"), "{said}");
}
