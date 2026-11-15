//! The book's preprocessors publish this tool's stdout as a chapter: it must hold the chapter
//! alone, with no build-script lines from the generator.
use std::process::Command;

fn chapter(name: &str) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["mod-api", name])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
#[ignore = "daily"]
fn the_coverage_chapter_is_the_page_alone() {
    let page = chapter("coverage");
    assert_eq!(page.lines().next(), Some("# Wire boundary coverage"));
    assert!(
        !page.contains("cargo:"),
        "no build-script line in the chapter"
    );
}

#[test]
#[ignore = "daily"]
fn the_hook_notes_chapter_is_the_page_alone() {
    let page = chapter("notes");
    assert!(page.starts_with('#'), "{}", &page[..page.len().min(200)]);
    assert!(
        !page.contains("cargo:"),
        "no build-script line in the chapter"
    );
}
