use std::{fs, path::Path};

fn compare(root: &Path, plain: &Path, cached: &Path) {
    crate::build::generate_points_inner(root, plain).unwrap();
    crate::generate_points(root, cached).unwrap();
    assert_eq!(
        fs::read(plain.join("points_generated.rs")).unwrap(),
        fs::read(cached.join("points_generated.rs")).unwrap()
    );
}

#[test]
#[ignore = "full source-tree generation equivalence; run explicitly when changing the cache"]
fn cached_points_match_full_generation_across_source_changes() {
    let original = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let scratch = original
        .join("target")
        .join(format!("interface-cache-test-{}", std::process::id()));
    fs::create_dir_all(&scratch).unwrap();
    let root = scratch.join("source");
    let tracked = std::process::Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(&original)
        .output()
        .unwrap();
    assert!(tracked.status.success());
    for relative in tracked
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
    {
        let relative = std::str::from_utf8(relative).unwrap();
        let source = original.join(relative);
        if source.is_file() {
            let destination = root.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(source, destination).unwrap();
        }
    }
    let plain = scratch.join("plain");
    let cached = scratch.join("cached");
    fs::create_dir_all(&plain).unwrap();
    fs::create_dir_all(&cached).unwrap();
    compare(&root, &plain, &cached);
    compare(&root, &plain, &cached);
    let probe = root.join("crates/horizon-client/src/game/mobs/build_cache_probe.rs");
    fs::write(
        &probe,
        "pub fn build_cache_probe(value: i32) -> i32 { value }\n",
    )
    .unwrap();
    compare(&root, &plain, &cached);
    let before = fs::read(cached.join("points_generated.rs")).unwrap();
    fs::write(
        &probe,
        "pub fn build_cache_probe(value: i32) -> i32 { value + 1 }\n",
    )
    .unwrap();
    compare(&root, &plain, &cached);
    assert_eq!(
        before,
        fs::read(cached.join("points_generated.rs")).unwrap()
    );
    fs::write(
        &probe,
        "pub fn build_cache_probe(value: i64) -> i64 { value }\n",
    )
    .unwrap();
    compare(&root, &plain, &cached);
    assert_ne!(
        before,
        fs::read(cached.join("points_generated.rs")).unwrap()
    );
    let renamed = probe.with_file_name("build_cache_renamed.rs");
    fs::rename(&probe, &renamed).unwrap();
    compare(&root, &plain, &cached);
    fs::remove_file(renamed).unwrap();
    compare(&root, &plain, &cached);
    fs::remove_dir_all(scratch).unwrap();
}

#[test]
fn unchanged_outputs_keep_their_timestamp_but_changed_outputs_replace_bytes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    fs::create_dir_all(&root).unwrap();
    let output = root.join(format!("write-if-changed-test-{}", std::process::id()));
    crate::write_if_changed(&output, b"first").unwrap();
    let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000);
    fs::File::options()
        .write(true)
        .open(&output)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(old))
        .unwrap();
    crate::write_if_changed(&output, b"first").unwrap();
    assert_eq!(fs::metadata(&output).unwrap().modified().unwrap(), old);
    crate::write_if_changed(&output, b"second").unwrap();
    assert_eq!(fs::read(&output).unwrap(), b"second");
    fs::remove_file(output).unwrap();
}

mod per_file {
    use crate::{
        incremental,
        points::{self, GamePoint, WireTypes},
        source,
    };
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    fn put(root: &Path, path: &str, text: &str) {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    /// A game module with nested, inline, conditional and `#[path]` files, and a lower crate with
    /// marked functions.
    fn fixture(name: &str) -> tempfile::TempDir {
        let guard = tempfile::Builder::new()
            .prefix(&format!("per-file-{name}-"))
            .tempdir()
            .unwrap();
        let root = guard.path().to_path_buf();
        let src = "crates/horizon-client/src";
        put(&root, "crates/horizon-mod-api-gen/internals.txt", "");
        put(
            &root,
            "crates/horizon-mod-api-gen/Cargo.toml",
            "[package]\nname = 'horizon-mod-api-gen'\n",
        );
        put(&root, &format!("{src}/lib.rs"), "mod game;\n");
        put(
            &root,
            &format!("{src}/game.rs"),
            "mod remote;\nmod nested { mod leaf; }\n#[path = \"elsewhere\"]\nmod moved { mod deep; }\n#[cfg(test)]\nmod probes;\n#[cfg(unix)]\nmod unixy;\n#[cfg_attr(windows, path = \"win.rs\")]\nmod winy;\nmod inline { include!(\"inc.rs\"); }\nimpl Game<'_> {\n    fn tick(&mut self) {}\n    fn native_tick(&mut self) {}\n    fn __fnr_tick(&mut self) {}\n}\n",
        );
        put(
            &root,
            &format!("{src}/game/remote.rs"),
            "mod lan;\nimpl Game<'_> { fn send(&self, id: u32) { let _ = id; } }\nimpl Other { fn send(&self) {} }\nfn decode<T>(bytes: &[u8]) -> T { todo!() }\n",
        );
        put(
            &root,
            &format!("{src}/game/remote/lan.rs"),
            "impl Game<'_> { fn join(&mut self) {} }",
        );
        put(
            &root,
            &format!("{src}/game/elsewhere/deep.rs"),
            "impl Game<'_> { fn dive(&mut self) {} }",
        );
        put(
            &root,
            &format!("{src}/game/nested/leaf.rs"),
            "impl Game<'_> { fn fall(&mut self) {} }",
        );
        put(
            &root,
            &format!("{src}/game/probes.rs"),
            "impl Game<'_> { fn probe(&self) {} }",
        );
        put(
            &root,
            &format!("{src}/game/unixy.rs"),
            "impl Game<'_> { fn unix_only(&self) {} }",
        );
        put(
            &root,
            &format!("{src}/game/win.rs"),
            "impl Game<'_> { fn windows_only(&self) {} }",
        );
        put(
            &root,
            &format!("{src}/game/inc.rs"),
            "impl Game<'_> { fn included(&self) {} }",
        );
        put(
            &root,
            "crates/horizon-engine-slots/Cargo.toml",
            "[package]\nname = 'horizon-engine-slots'\n[dependencies]\nhorizon-world = { path = '../horizon-world' }\n",
        );
        put(
            &root,
            "crates/horizon-world/Cargo.toml",
            "[package]\nname = 'horizon-world'\n",
        );
        put(
            &root,
            "crates/horizon-client-render/Cargo.toml",
            "[package]\nname = 'horizon-client-render'\n[dependencies]\nhorizon-engine-slots = { path = '../horizon-engine-slots' }\n",
        );
        put(
            &root,
            "crates/horizon-client-render/src/lib.rs",
            "pub mod mesher;\n",
        );
        put(
            &root,
            "crates/horizon-client-render/src/mesher.rs",
            "pub struct Mesher<'a>(&'a u8);\nimpl Mesher<'_> { #[horizon_mod_api_macros::replaceable] pub fn mesh(&mut self, chunk: &Chunk) -> usize { 0 } fn face(&self) {} }\n",
        );
        guard
    }

    /// Everything the generator emits from the points, and what the catalog lists.
    fn rendered(points: &[GamePoint]) -> String {
        format!(
            "{}\n{}\n{}",
            points::records(points),
            points::engine_records(points, None),
            serde_json::to_string(points).unwrap()
        )
    }

    fn plain(root: &Path) -> String {
        rendered(&points::points_with(root, &WireTypes::new()).unwrap())
    }

    /// The points of one cached generation, and its cache report.
    fn cached(root: &Path, out: &Path) -> (String, String) {
        incremental::with_cache(out, || {
            source::with_walk_cache(|| {
                let points = points::points_with(root, &WireTypes::new())?;
                let marked = crate::replace::marked(root)?;
                Ok((
                    format!("{}\n{marked:?}", rendered(&points)),
                    incremental::report(),
                ))
            })
        })
        .unwrap()
    }

    fn with_marked(root: &Path, points: String) -> String {
        format!("{points}\n{:?}", crate::replace::marked(root).unwrap())
    }

    /// `(found, computed)` of one kind of entry in a report.
    fn counts(report: &str, kind: &str) -> (u32, u32) {
        let line = report
            .lines()
            .find_map(|line| line.trim().strip_prefix(&format!("cache {kind}: ")))
            .unwrap_or_else(|| panic!("no {kind} in {report}"));
        let (found, computed) = line.split_once(" found, ").unwrap();
        (
            found.parse().unwrap(),
            computed.strip_suffix(" computed").unwrap().parse().unwrap(),
        )
    }

    fn walk(root: &Path, every_build: bool) -> Vec<(String, PathBuf, Vec<String>)> {
        let mut files: Vec<_> =
            source::game_files(&root.join("crates/horizon-client/src"), every_build)
                .unwrap()
                .into_iter()
                .chain(source::crate_files(root, "horizon_client_render").unwrap())
                .map(|file| (file.module, file.path, file.cfg))
                .collect();
        files.sort();
        files
    }

    #[test]
    fn module_walks_are_the_same_with_and_without_the_cache() {
        let guard = fixture("walks");
        let root = guard.path().to_path_buf();
        let out = root.join("out");
        fs::create_dir_all(&out).unwrap();
        let expected = [walk(&root, true), walk(&root, false)];
        // The conditional files are walked only when the builds are not required to compile them.
        assert!(expected[1].iter().any(|(m, ..)| m.ends_with("unixy")));
        assert!(!expected[0].iter().any(|(m, ..)| m.ends_with("unixy")));
        for run in 0..2 {
            let (found, report) = incremental::with_cache(&out, || {
                Ok((
                    [walk(&root, true), walk(&root, false)],
                    incremental::report(),
                ))
            })
            .unwrap();
            assert_eq!(found, expected, "run {run}");
            if run == 1 {
                assert_eq!(counts(&report, "layout").1, 0, "{report}");
            }
        }
    }

    #[test]
    fn a_cached_generation_equals_a_plain_one_and_reparses_only_what_changed() {
        let guard = fixture("equal");
        let root = guard.path().to_path_buf();
        let out = root.join("out");
        fs::create_dir_all(&out).unwrap();
        let src = "crates/horizon-client/src";
        let expected = with_marked(&root, plain(&root));

        let (cold, report) = cached(&root, &out);
        assert_eq!(cold, expected);
        let (_, files) = counts(&report, "points");
        assert!(files >= 7, "{report}");

        let (warm, report) = cached(&root, &out);
        assert_eq!(warm, expected);
        assert_eq!(counts(&report, "points"), (files, 0), "{report}");
        assert_eq!(counts(&report, "layout").1, 0, "{report}");
        assert_eq!(counts(&report, "marked").1, 0, "{report}");

        // A body edit: that one file is read again, the points do not change.
        put(
            &root,
            &format!("{src}/game/remote.rs"),
            "mod lan;\nimpl Game<'_> { fn send(&self, id: u32) { let _ = id + 1; } }\nimpl Other { fn send(&self) {} }\nfn decode<T>(bytes: &[u8]) -> T { todo!() }\n",
        );
        let (edited, report) = cached(&root, &out);
        assert_eq!(edited, expected);
        assert_eq!(edited, with_marked(&root, plain(&root)));
        assert_eq!(counts(&report, "points"), (files - 1, 1), "{report}");
        assert_eq!(counts(&report, "layout").1, 1, "{report}");

        // A signature edit changes that point, and only that file is computed again.
        put(
            &root,
            &format!("{src}/game/remote.rs"),
            "mod lan;\nimpl Game<'_> { fn send(&self, id: u64) { let _ = id; } }\nimpl Other { fn send(&self) {} }\nfn decode<T>(bytes: &[u8]) -> T { todo!() }\n",
        );
        let (changed, report) = cached(&root, &out);
        assert_ne!(changed, expected);
        assert_eq!(changed, with_marked(&root, plain(&root)));
        assert_eq!(counts(&report, "points"), (files - 1, 1), "{report}");

        // A new file and a new module declaration.
        put(
            &root,
            &format!("{src}/game/fresh.rs"),
            "impl Game<'_> { fn fresh(&mut self, value: i32) {} }",
        );
        let game = fs::read_to_string(root.join(format!("{src}/game.rs"))).unwrap();
        put(
            &root,
            &format!("{src}/game.rs"),
            &format!("mod fresh;\n{game}"),
        );
        let (grown, report) = cached(&root, &out);
        assert_eq!(grown, with_marked(&root, plain(&root)));
        assert!(grown.contains("fresh"));
        assert_eq!(counts(&report, "points").1, 2, "{report}");

        // A function becomes replaceable in a lower crate: its list changes, so the lower
        // crate's files are computed again and the game's are not.
        put(
            &root,
            "crates/horizon-client-render/src/mesher.rs",
            "pub struct Mesher<'a>(&'a u8);\nimpl Mesher<'_> { #[horizon_mod_api_macros::replaceable] pub fn mesh(&mut self, chunk: &Chunk) -> usize { 0 } #[horizon_mod_api_macros::replaceable] fn face(&self) {} }\n",
        );
        let (marked, report) = cached(&root, &out);
        assert_eq!(marked, with_marked(&root, plain(&root)));
        assert_ne!(marked, grown);
        assert_eq!(counts(&report, "points").1, 2, "{report}");
    }

    #[test]
    fn corrupt_or_missing_entries_fall_back_to_parsing() {
        let guard = fixture("corrupt");
        let root = guard.path().to_path_buf();
        let out = root.join("out");
        fs::create_dir_all(&out).unwrap();
        let expected = with_marked(&root, plain(&root));
        let (first, report) = cached(&root, &out);
        assert_eq!(first, expected);
        let computed = counts(&report, "points").1;
        let folder = fs::read_dir(&out).unwrap().next().unwrap().unwrap().path();
        let mut entries: Vec<_> = fs::read_dir(&folder)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        entries.sort();
        for (index, entry) in entries.iter().enumerate() {
            match index % 4 {
                0 => fs::remove_file(entry).unwrap(),
                1 => fs::write(entry, "").unwrap(),
                2 => fs::write(entry, "not-a-digest\n[]").unwrap(),
                _ => {
                    let text = fs::read_to_string(entry).unwrap();
                    fs::write(entry, &text[..text.len() / 2]).unwrap();
                }
            }
        }
        let (again, report) = cached(&root, &out);
        assert_eq!(again, expected);
        assert!(counts(&report, "points").1 > 0, "{report} {computed}");
        // The damaged entries were written again: the next run finds them all.
        let (last, report) = cached(&root, &out);
        assert_eq!(last, expected);
        assert_eq!(counts(&report, "points").1, 0, "{report}");
    }

    #[test]
    fn the_tracked_paths_are_the_same_with_the_cache() {
        let guard = fixture("tracked");
        let root = guard.path().to_path_buf();
        let out = root.join("out");
        fs::create_dir_all(&out).unwrap();
        let paths = |root: &Path| {
            let mut paths = points::tracked(root).unwrap();
            paths.sort();
            paths
        };
        let expected = paths(&root);
        assert!(expected.iter().any(|path| path.ends_with("mesher.rs")));
        for _ in 0..2 {
            let found =
                incremental::with_cache(&out, || source::with_walk_cache(|| Ok(paths(&root))))
                    .unwrap();
            assert_eq!(found, expected);
        }
    }
}
