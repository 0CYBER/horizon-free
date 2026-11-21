use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) struct Home(pub(super) PathBuf);
impl Home {
    pub(super) fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "horizon-storage-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        // Canonical, as Storage::at stores it: Windows temp paths are 8.3 short names.
        Self(path.canonicalize().unwrap())
    }
    pub(super) fn seed(&self, path: &str) -> PathBuf {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, vec![7; 8192]).unwrap();
        path
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A directory link as the game makes one: a symlink on Unix, a junction on Windows.
#[cfg(unix)]
pub(super) fn link_dir(from: &Path, to: &Path) {
    std::os::unix::fs::symlink(from, to).unwrap();
}
#[cfg(windows)]
pub(super) fn link_dir(from: &Path, to: &Path) {
    // mklink takes plain paths: no forward slash and no verbatim prefix.
    let plain = |path: &Path| {
        let path = path.to_string_lossy().replace('/', "\\");
        path.trim_start_matches(r"\\?\").to_owned()
    };
    let output = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(plain(to))
        .arg(plain(from))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// #17911: the game links what a resource pack leaves untouched into the pack's mirror in its
/// cache. Such a link counts nothing and is never followed; where none is expected, one still
/// refuses the measurement.
#[test]
fn a_game_made_link_counts_nothing_and_is_not_followed() {
    let home = Home::new();
    let outside = Home::new();
    let vanilla = outside.0.join("version/assets/minecraft/textures");
    fs::create_dir_all(&vanilla).unwrap();
    fs::write(vanilla.join("atlas.png"), vec![7; 4 << 20]).unwrap();
    home.seed(".horizon-cache/resourcepacks/0123456789abcdef/assets/minecraft/pack.json");
    let cache = home.0.join(".horizon-cache");
    let alone = allocated_bytes(&cache).unwrap();
    link_dir(
        &vanilla,
        &cache.join("resourcepacks/0123456789abcdef/assets/minecraft/textures"),
    );

    assert!(matches!(allocated_bytes(&cache), Err(Error::Unsafe(_))));
    let measured = walk(&cache, &mut HashSet::new(), Links::GameMade).unwrap();
    assert!(
        measured >= alone && measured < (4 << 20),
        "{measured} bytes with the link, {alone} without"
    );
    assert_eq!(fs::read(vanilla.join("atlas.png")).unwrap().len(), 4 << 20);
}

#[test]
fn build_tools_remain_clearable_without_claiming_game_absence() {
    let home = Home::new();
    let storage = Storage::at(&home.0).unwrap();
    for (cache, directory) in [
        (BuildCache::Rust, "rust"),
        (BuildCache::Cargo, "cargo"),
        (BuildCache::Scratch, "tmp"),
    ] {
        let output = home.seed(&format!("{directory}/output"));
        assert!(storage.clear(cache).unwrap() > 0);
        assert!(!output.exists());
    }
}

#[test]
fn hard_links_count_once() {
    let home = Home::new();
    let file = home.seed("rust/a");
    let before = allocated_bytes(&home.0.join("rust")).unwrap();
    fs::hard_link(file, home.0.join("rust/b")).unwrap();
    assert_eq!(allocated_bytes(&home.0.join("rust")).unwrap(), before);
}

/// #19481: the folder listings find what one handle per file finds: every file and folder once,
/// a hard link once, and each file's allocation.
#[cfg(windows)]
#[test]
fn listing_walk_matches_one_handle_per_file() {
    fn by_handle(path: &Path, seen: &mut HashSet<(u64, u64)>, out: &mut Vec<((u64, u64), u64)>) {
        let meta = fs::symlink_metadata(path).unwrap();
        let (id, bytes) = allocation(path, &meta).unwrap();
        if !seen.insert(id) {
            return;
        }
        // A folder's own allocation may read differently by handle than in its parent's listing.
        out.push((id, if meta.is_dir() { 0 } else { bytes }));
        if meta.is_dir() {
            for entry in fs::read_dir(path).unwrap() {
                by_handle(&entry.unwrap().path(), seen, out);
            }
        }
    }
    let home = Home::new();
    for index in 0..40 {
        home.seed(&format!("rust/deep/{}/file-{index}", index % 4));
    }
    fs::write(home.0.join("rust/empty"), []).unwrap();
    let file = home.seed("rust/a");
    fs::hard_link(file, home.0.join("rust/deep/b")).unwrap();
    let root = home.0.join("rust");
    let mut expected = Vec::new();
    by_handle(&root, &mut HashSet::new(), &mut expected);
    let folders: HashSet<_> = expected
        .iter()
        .filter(|(_, bytes)| *bytes == 0)
        .map(|(id, _)| *id)
        .collect();
    let mut listed: Vec<_> = survey(&root, Links::Refused)
        .unwrap()
        .into_iter()
        .map(|(id, bytes)| (id, if folders.contains(&id) { 0 } else { bytes }))
        .collect();
    listed.sort();
    expected.sort();
    assert_eq!(listed, expected);
    assert_eq!(
        listed.len(),
        1 + 5 + 40 + 2,
        "the root, 5 folders, 42 files"
    );
}

#[test]
fn clears_only_fixed_cache_and_sweeps_detached_tree() {
    let home = Home::new();
    home.seed("native/source/revision/lib.rs");
    home.seed("settings.json");
    home.seed("worlds/keep/level.dat");
    let storage = Storage::at(&home.0).unwrap();
    assert!(storage.bytes(BuildCache::Source).unwrap() > 0);
    assert!(
        storage
            .trash(
                &storage.path(BuildCache::Source).unwrap(),
                Access::BuildTools
            )
            .unwrap()
            > 0
    );
    assert!(!home.0.join("native/source/revision/lib.rs").exists());
    assert!(home.0.join("settings.json").exists());
    assert!(home.0.join("worlds/keep/level.dat").exists());
    home.seed(".trash-interrupted/file");
    assert!(matches!(storage.sweep(), Err(Error::UnprovenExclusion)));
    assert!(home.0.join(".trash-interrupted/file").exists());
    Storage::clear_entry(
        &storage.directory,
        &storage.directory,
        Path::new(".trash-interrupted"),
    )
    .unwrap();
    assert!(!home.0.join(".trash-interrupted/file").exists());
}

#[test]
fn cleanup_obeys_build_lease() {
    let home = Home::new();
    let _storage = Storage::at(&home.0).unwrap();
    assert!(matches!(Storage::at(&home.0), Err(Error::Busy)));
}

#[cfg(unix)]
#[test]
fn refuses_linked_ancestor_and_descendant_before_deletion() {
    use std::os::unix::fs::symlink;
    let home = Home::new();
    home.seed("worlds/keep");
    let storage = Storage::at(&home.0).unwrap();
    symlink(home.0.join("worlds"), home.0.join("native/source")).unwrap();
    assert!(storage.clear(BuildCache::Source).is_err());
    fs::remove_file(home.0.join("native/source")).unwrap();
    home.seed("native/source/good");
    symlink(home.0.join("worlds"), home.0.join("native/source/bad")).unwrap();
    assert!(storage.clear(BuildCache::Source).is_err());
    assert!(home.0.join("native/source/good").exists());
    assert!(home.0.join("worlds/keep").exists());
    fs::rename(home.0.join("native"), home.0.join("moved")).unwrap();
    symlink(home.0.join("moved"), home.0.join("native")).unwrap();
    assert!(storage.clear(BuildCache::Source).is_err());
}

#[test]
fn read_only_file_can_be_removed() {
    let home = Home::new();
    let file = home.seed("native/tools/tool");
    let mut permissions = fs::metadata(&file).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(file, permissions).unwrap();
    let storage = Storage::at(&home.0).unwrap();
    storage
        .trash(
            &storage.path(BuildCache::Tools).unwrap(),
            Access::BuildTools,
        )
        .unwrap();
    assert!(!home.0.join("native/tools/tool").exists());
}

#[cfg(windows)]
#[test]
fn held_open_file_is_kept_for_next_sweep() {
    use std::os::windows::fs::OpenOptionsExt;
    let home = Home::new();
    let file = home.seed("native/source/file");
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(file)
        .unwrap();
    let storage = Storage::at(&home.0).unwrap();
    assert!(storage.clear(BuildCache::Source).is_err());
    drop(held);
    storage.sweep().unwrap();
    storage
        .trash(
            &storage.path(BuildCache::Source).unwrap(),
            Access::BuildTools,
        )
        .unwrap();
}

#[cfg(unix)]
#[test]
fn root_replacement_cannot_redirect_sweep() {
    let home = Home::new();
    let outside = Home::new();
    outside.seed(".trash-precious/world");
    let storage = Storage::at(&home.0).unwrap();
    let moved = home.0.with_extension("moved");
    fs::rename(&home.0, &moved).unwrap();
    std::os::unix::fs::symlink(&outside.0, &home.0).unwrap();
    assert!(storage.sweep().is_err());
    assert!(storage.clear(BuildCache::Source).is_err());
    assert!(outside.0.join(".trash-precious/world").exists());
    fs::remove_file(&home.0).unwrap();
    fs::rename(moved, &home.0).unwrap();
}

#[test]
fn child_cargo_locks_prevent_clear_and_survive_idle_cleanup() {
    for (cache, expected) in [
        (BuildCache::Source, "native/source"),
        (BuildCache::Tools, "native/tools"),
        (BuildCache::PrebuiltOutput, "native/prebuilt-build"),
        (BuildCache::InternalsSync, "native/internals-sync"),
        (BuildCache::Prebuilt, "native/prebuilt"),
        (BuildCache::ServerBuilds, "native/server-builds"),
        (BuildCache::Rust, "rust"),
        (BuildCache::Cargo, "cargo"),
        (BuildCache::Scratch, "tmp"),
    ] {
        for depth in ["", "target", "rev/target/debug"] {
            let directory = format!("{expected}/{depth}");
            let home = Home::new();
            let lock_path = home.seed(&format!("{directory}/.cargo-lock"));
            let output = home.seed(&format!("{directory}/output"));
            let held = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&lock_path)
                .unwrap();
            held.lock().unwrap();
            let world = home.seed("worlds/precious/level.dat");
            let storage = Storage::at(&home.0).unwrap();
            assert!(matches!(
                storage.trash(&storage.path(cache).unwrap(), Access::BuildTools),
                Err(Error::Busy)
            ));
            assert!(output.exists());
            held.unlock().unwrap();
            if matches!(
                cache.access(&home.0.join(cache.relative())).unwrap(),
                Access::UninstrumentedGame
            ) {
                assert!(matches!(
                    storage.clear(cache),
                    Err(Error::UnprovenExclusion)
                ));
                assert!(output.exists());
            }
            storage
                .trash(&storage.path(cache).unwrap(), Access::BuildTools)
                .unwrap();
            assert!(!output.exists());
            assert!(lock_path.exists());
            assert!(world.exists());
        }
    }
}

#[cfg(windows)]
#[test]
fn held_root_cannot_be_renamed_away_on_windows() {
    let home = Home::new();
    let precious = home.seed("native/source/world");
    let _storage = Storage::at(&home.0).unwrap();
    assert!(fs::rename(&home.0, home.0.with_extension("held")).is_err());
    assert!(precious.exists());
}

#[cfg(unix)]
#[test]
fn same_path_with_different_root_identity_is_refused() {
    let home = Home::new();
    let storage = Storage::at(&home.0).unwrap();
    let moved = home.0.with_extension("held");
    fs::rename(&home.0, &moved).unwrap();
    let precious = home.seed("native/source/world");
    home.seed(".trash-precious/world");
    assert!(storage.clear(BuildCache::Source).is_err());
    assert!(storage.sweep().is_err());
    assert!(precious.exists());
    assert!(home.0.join(".trash-precious/world").exists());
    fs::remove_dir_all(&home.0).unwrap();
    fs::rename(moved, &home.0).unwrap();
}

#[test]
fn freed_bytes_exclude_the_retained_lock_skeleton() {
    let home = Home::new();
    home.seed("native/source/rev/output");
    home.seed("native/source/rev/.cargo-lock");
    let storage = Storage::at(&home.0).unwrap();
    let before = storage.bytes(BuildCache::Source).unwrap();
    let freed = storage
        .trash(
            &storage.path(BuildCache::Source).unwrap(),
            Access::BuildTools,
        )
        .unwrap();
    let after = storage.bytes(BuildCache::Source).unwrap();
    assert!(after > 0);
    assert_eq!(freed, before.saturating_sub(after));
    assert!(freed < before);
}

/// Use checkout storage rather than /tmp (which may report zero directory blocks).
#[cfg(unix)]
#[test]
fn allocated_total_includes_directory_metadata() {
    use std::os::unix::fs::MetadataExt;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/storage-allocation-tests");
    fs::create_dir_all(&root).unwrap();
    let path = root.join(format!("directory-{}", std::process::id()));
    fs::create_dir(&path).unwrap();
    // Home removes the fixture on drop, including when an assertion unwinds.
    let home = Home(path);
    for index in 0..300 {
        fs::write(home.0.join(format!("{index:04}-{}", "x".repeat(180))), []).unwrap();
    }
    let directory_bytes = fs::metadata(&home.0).unwrap().blocks() * 512;
    let entries_bytes: u64 = fs::read_dir(&home.0)
        .unwrap()
        .map(|entry| {
            fs::symlink_metadata(entry.unwrap().path())
                .unwrap()
                .blocks()
                * 512
        })
        .sum();
    assert_eq!(
        allocated_bytes(&home.0).unwrap(),
        directory_bytes + entries_bytes
    );
    eprintln!("independent directory allocation: {directory_bytes} bytes");
}
