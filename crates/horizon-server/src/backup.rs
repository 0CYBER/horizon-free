//! Backups and restore (docs/server.md, section 7). A backup is one zip in `backups/`: the world folder and
//! the server's settings and lists. On a running server the game first saves everything and stops writing
//! (`save-off`, `save-all`), and writes again once the zip is done (`save-on`), so players stay online.
//! A restore puts the backup's world in place of the current one, which is moved aside, never deleted.

use horizon_server_launch::config::Config;
use std::{
    fs::File,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

pub const DIR: &str = "backups";
/// The settings and lists a backup holds beside the world.
const FILES: [&str; 6] = [
    "server.toml",
    "server.properties",
    "ops.json",
    "whitelist.json",
    "banned-players.json",
    "banned-ips.json",
];
/// Automatic backups are named `<time>-auto.zip`; only those are pruned.
const AUTO: &str = "auto";

/// `2026-10-16_18-30-05`, in UTC: sorts by time, and is valid in a file name everywhere.
fn stamp(time: SystemTime) -> String {
    let secs = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let (y, m, d) = horizon_platform::java_properties::civil_from_days(secs.div_euclid(86_400));
    let s = secs.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02}_{:02}-{:02}-{:02}",
        s / 3600,
        s / 60 % 60,
        s % 60
    )
}

/// A backup's file name: the time, and a name the owner gave (letters, digits, `-` and `_` only). Not
/// `auto`: that is the automatic backups' name, and `prune` deletes the ones that carry it.
pub fn file_name(time: SystemTime, name: Option<&str>) -> Result<String, String> {
    let name = name.unwrap_or("manual");
    if name == AUTO {
        return Err(format!(
            "{AUTO:?} is the name of the automatic backups, which are deleted beyond `keep`: use another name"
        ));
    }
    stamped(time, name)
}

/// `<time>-<name>.zip` for a name made of letters, digits, `-` and `_`.
fn stamped(time: SystemTime, name: &str) -> Result<String, String> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!(
            "backup names use letters, digits, - and _ only (got {name:?})"
        ));
    }
    Ok(format!("{}-{name}.zip", stamp(time)))
}

/// Adds a folder to the zip, under `prefix`, skipping the game's session lock. Only regular files and
/// folders are taken, and links are not followed: vanilla worlds hold no symlinks (vanilla refuses a world
/// with one unless allow-listed), while a link in an extracted archive could pull a file from outside the
/// server folder into the zip, and a FIFO or device would make the copy never finish.
fn add_dir(zip: &mut ZipWriter<BufWriter<File>>, dir: &Path, prefix: &str) -> Result<(), String> {
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .large_file(true);
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .flatten()
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let inner = format!("{prefix}/{name}");
        let kind = entry
            .file_type()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if kind.is_dir() {
            add_dir(zip, &path, &inner)?;
        } else if !kind.is_file() {
            eprintln!(
                "horizon-server: backup skips {}: not a regular file or folder",
                path.display()
            );
        } else if name != "session.lock" {
            zip.start_file(inner.as_str(), options)
                .map_err(|e| e.to_string())?;
            let mut file = File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            std::io::copy(&mut file, zip).map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }
    Ok(())
}

/// Zips the world and the settings into `backups/<file_name>`. The zip is written under a temporary name
/// and renamed when complete, so a backup that looks finished is one. Existing finished or partial
/// backups with this name are left intact. The output directory must resolve outside the world.
pub fn create(dir: &Path, config: &Config, file_name: &str) -> Result<PathBuf, String> {
    let backups = dir.join(DIR);
    std::fs::create_dir_all(&backups).map_err(|e| format!("{}: {e}", backups.display()))?;
    let world = dir.join(&config.world.name);
    if world.is_dir() {
        let source = world
            .canonicalize()
            .map_err(|e| format!("{}: {e}", world.display()))?;
        let destination = backups
            .canonicalize()
            .map_err(|e| format!("{}: {e}", backups.display()))?;
        if destination.starts_with(&source) {
            return Err(format!(
                "{} must be outside the world {}",
                backups.display(),
                world.display()
            ));
        }
    }
    let path = backups.join(file_name);
    let part = backups.join(format!("{file_name}.part"));
    // Owner only: a backup holds server.toml and server.properties, with the API token, the
    // RCON password and the management secret. Reserve a new partial before entering its cleanup path.
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let file = options
        .open(&part)
        .map_err(|e| format!("{}: {e}", part.display()))?;
    let result = (|| {
        // Holding the partial's name also excludes another creator while checking the finished ZIP.
        if path
            .try_exists()
            .map_err(|e| format!("{}: {e}", path.display()))?
        {
            return Err(format!(
                "{} exists already; try another name or try again in a second",
                path.display()
            ));
        }
        let mut zip = ZipWriter::new(BufWriter::new(file));
        if world.is_dir() {
            add_dir(&mut zip, &world, &config.world.name)?;
        }
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for name in FILES {
            if let Ok(bytes) = std::fs::read(dir.join(name)) {
                zip.start_file(name, options).map_err(|e| e.to_string())?;
                zip.write_all(&bytes).map_err(|e| e.to_string())?;
            }
        }
        zip.finish()
            .map_err(|e| e.to_string())?
            .into_inner()
            .map_err(|e| e.to_string())?
            .sync_all()
            .map_err(|e| e.to_string())
    })();
    match result {
        Ok(()) => {
            std::fs::rename(&part, &path).map_err(|e| e.to_string())?;
            Ok(path)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

pub struct Backup {
    pub name: String,
    pub bytes: u64,
}

/// The finished backups, newest first.
pub fn list(dir: &Path) -> Vec<Backup> {
    let mut out: Vec<Backup> = std::fs::read_dir(dir.join(DIR))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.ends_with(".zip").then(|| Backup {
                bytes: e.metadata().map_or(0, |m| m.len()),
                name,
            })
        })
        .collect();
    out.sort_by(|a, b| b.name.cmp(&a.name));
    out
}

/// Deletes the oldest automatic backups beyond `keep`. Backups made by hand are never deleted.
/// The length of a backup name's time stamp, `2026-11-11_13-15-00`.
const STAMP_LEN: usize = "0000-00-00_00-00-00".len();

pub fn prune(dir: &Path, keep: usize) -> Vec<String> {
    let auto_suffix = format!("-{AUTO}.zip");
    list(dir)
        .into_iter()
        // Only `<stamp>-auto.zip`: a backup labelled by hand `nightly-auto` is not automatic.
        .filter(|b| b.name.len() == STAMP_LEN + auto_suffix.len() && b.name.ends_with(&auto_suffix))
        .skip(keep)
        .filter(|b| std::fs::remove_file(dir.join(DIR).join(&b.name)).is_ok())
        .map(|b| b.name)
        .collect()
}

pub fn auto_name(time: SystemTime) -> String {
    stamped(time, AUTO).expect("the automatic name is valid")
}

/// The backup a name points at: a file in `backups/`, by its full name or the start of it.
pub fn find(dir: &Path, name: &str) -> Result<String, String> {
    if name.trim().is_empty() {
        return Err("a backup name is required".into());
    }
    let all = list(dir);
    if let Some(b) = all
        .iter()
        .find(|b| b.name == name || b.name == format!("{name}.zip"))
    {
        return Ok(b.name.clone());
    }
    let matches: Vec<&Backup> = all.iter().filter(|b| b.name.starts_with(name)).collect();
    match matches.as_slice() {
        [one] => return Ok(one.name.clone()),
        [] => {}
        _ => {
            return Err(format!(
                "{name:?} matches {} backups: be more precise",
                matches.len()
            ));
        }
    }
    // The name given to `backup NAME` (or `auto`, `manual`): the newest backup with it.
    let label = format!("-{name}.zip");
    // The whole label after the time stamp: `test` is not the end of `<stamp>-my-test.zip`.
    all.iter()
        .find(|b| b.name.len() == STAMP_LEN + label.len() && b.name.ends_with(&label))
        .map(|b| b.name.clone())
        .ok_or_else(|| {
            format!("no backup {name:?} in {DIR}/ (`horizon-server backups` lists them)")
        })
}

/// Puts the backup's world in place of the current one, which goes to `backups/replaced-<time>/`. The
/// server must be stopped. Returns where the old world went.
pub fn restore(dir: &Path, config: &Config, name: &str) -> Result<Option<PathBuf>, String> {
    let zip_path = dir.join(DIR).join(name);
    let scratch = dir.join(".horizon/restore");
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).map_err(|e| e.to_string())?;
    let file = File::open(&zip_path).map_err(|e| format!("{}: {e}", zip_path.display()))?;
    ZipArchive::new(file)
        .and_then(|mut z| z.extract(&scratch))
        .map_err(|e| format!("{name}: {e}"))?;
    let world = &config.world.name;
    let restored = scratch.join(world);
    if !restored.join("level.dat").is_file() {
        let _ = std::fs::remove_dir_all(&scratch);
        return Err(format!(
            "{name} holds no world named {world:?} (no {world}/level.dat)"
        ));
    }
    let current = dir.join(world);
    let moved = if current.exists() {
        let aside = dir
            .join(DIR)
            .join(format!("replaced-{}", stamp(SystemTime::now())));
        std::fs::create_dir_all(&aside).map_err(|e| e.to_string())?;
        let target = aside.join(world);
        std::fs::rename(&current, &target)
            .map_err(|e| format!("moving the current world aside: {e}"))?;
        Some(target)
    } else {
        None
    };
    if let Err(e) = std::fs::rename(&restored, &current) {
        // The world goes back where it was: the game must not start on an empty folder and
        // make a new world.
        if let Some(old) = &moved {
            let _ = std::fs::rename(old, &current);
        }
        let _ = std::fs::remove_dir_all(&scratch);
        return Err(format!("putting the backup in place: {e}"));
    }
    let _ = std::fs::remove_dir_all(&scratch);
    Ok(moved)
}

/// A dimension id (`minecraft:the_nether`, or `the_nether` for `minecraft:`) as the id and its folder in the
/// world (`dimensions/<namespace>/<path>`, as the game saves it). Only the characters of a resource id.
pub fn dimension_folder(dimension: &str) -> Result<(String, String), String> {
    let (namespace, path) = dimension
        .split_once(':')
        .unwrap_or(("minecraft", dimension));
    let namespace_ok = !namespace.is_empty()
        && namespace
            .chars()
            .all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_' | '-' | '.'));
    let path_ok = !path.is_empty()
        && path
            .split('/')
            .all(|s| !s.is_empty() && s != "." && s != "..")
        && path
            .chars()
            .all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_' | '-' | '.' | '/'));
    if !namespace_ok || !path_ok {
        return Err(format!(
            "{dimension:?} is not a dimension id (like minecraft:the_nether)"
        ));
    }
    Ok((
        format!("{namespace}:{path}"),
        format!("dimensions/{namespace}/{path}"),
    ))
}

/// Puts one dimension of the backup in place of the current one, leaving the rest of the world (the other
/// dimensions, `level.dat`, the players) as it is. The current dimension goes to
/// `backups/replaced-<time>/<world>/dimensions/...`. The server must be stopped. Returns where the old
/// folder went (None when the world had none).
pub fn restore_dimension(
    dir: &Path,
    config: &Config,
    name: &str,
    dimension: &str,
) -> Result<Option<PathBuf>, String> {
    let (id, folder) = dimension_folder(dimension)?;
    let world = &config.world.name;
    let prefix = format!("{world}/{folder}/");
    let zip_path = dir.join(DIR).join(name);
    let scratch = dir.join(".horizon/restore");
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).map_err(|e| e.to_string())?;
    let result = (|| {
        let file = File::open(&zip_path).map_err(|e| format!("{}: {e}", zip_path.display()))?;
        let mut zip = ZipArchive::new(file).map_err(|e| format!("{name}: {e}"))?;
        let mut files = 0;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| format!("{name}: {e}"))?;
            let Some(inner) = entry.enclosed_name() else {
                continue;
            };
            if !inner.starts_with(&prefix) {
                continue;
            }
            let out = scratch.join(&inner);
            if entry.is_dir() {
                std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
                continue;
            }
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut dest = File::create(&out).map_err(|e| format!("{}: {e}", out.display()))?;
            std::io::copy(&mut entry, &mut dest).map_err(|e| format!("{name}: {e}"))?;
            files += 1;
        }
        if files == 0 {
            return Err(format!(
                "{name} holds nothing of the dimension {id} (no {world}/{folder}/)"
            ));
        }
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&scratch);
        return Err(e);
    }
    let restored = scratch.join(world).join(&folder);
    let current = dir.join(world).join(&folder);
    let moved = if current.exists() {
        let target = dir
            .join(DIR)
            .join(format!("replaced-{}", stamp(SystemTime::now())))
            .join(world)
            .join(&folder);
        if target.exists() {
            let _ = std::fs::remove_dir_all(&scratch);
            return Err(format!(
                "{} exists already; try again in a second",
                target.display()
            ));
        }
        std::fs::create_dir_all(target.parent().expect("the folder has a parent"))
            .map_err(|e| e.to_string())?;
        std::fs::rename(&current, &target)
            .map_err(|e| format!("moving the current {id} aside: {e}"))?;
        Some(target)
    } else {
        None
    };
    let put = (|| {
        std::fs::create_dir_all(current.parent().expect("the folder has a parent"))?;
        std::fs::rename(&restored, &current)
    })();
    if let Err(e) = put {
        // Put the old folder back: the world must not be left without the dimension.
        if let Some(old) = &moved {
            let _ = std::fs::rename(old, &current);
        }
        let _ = std::fs::remove_dir_all(&scratch);
        return Err(format!("putting the backup's {id} in place: {e}"));
    }
    let _ = std::fs::remove_dir_all(&scratch);
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_server_launch::config;

    #[test]
    fn names_sort_by_time() {
        let t = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_792_000_000);
        assert_eq!(
            file_name(t, Some("before-update")).unwrap(),
            "2026-10-14_17-46-40-before-update.zip"
        );
        assert!(file_name(t, Some("../x")).is_err());
        assert_eq!(auto_name(t), "2026-10-14_17-46-40-auto.zip");
    }

    #[cfg(unix)]
    #[test]
    fn a_backup_skips_links_and_special_files() {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let dir = std::env::temp_dir().join(format!(
            "horizon-server-backup-links-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).unwrap();
        let outside = dir.join("outside.txt");
        std::fs::write(&outside, b"secret").unwrap();
        let world = dir.join("world");
        std::fs::create_dir(&world).unwrap();
        std::fs::write(world.join("level.dat"), b"level").unwrap();
        std::os::unix::fs::symlink(&outside, world.join("link.txt")).unwrap();
        std::os::unix::fs::symlink(&dir, world.join("link-dir")).unwrap();
        let fifo = CString::new(world.join("pipe").as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        let path = create(&dir, &Config::default(), "x.zip").unwrap();
        let zip = ZipArchive::new(File::open(&path).unwrap()).unwrap();
        let mut names: Vec<_> = zip.file_names().map(|n| n.unwrap().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["world/level.dat"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_backup_collision_preserves_finished_and_partial_files() {
        let dir = std::env::temp_dir().join(format!(
            "horizon-server-backup-collision-{}",
            std::process::id()
        ));
        std::fs::create_dir(&dir).unwrap();
        let config = Config::default();
        let world = dir.join("world");
        std::fs::create_dir(&world).unwrap();
        std::fs::write(world.join("level.dat"), b"before").unwrap();
        let t = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        let name = file_name(t, Some("collision")).unwrap();
        let path = create(&dir, &config, &name).unwrap();
        let original = std::fs::read(&path).unwrap();

        std::fs::write(world.join("level.dat"), b"after").unwrap();
        assert!(create(&dir, &config, &name).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert!(!dir.join(DIR).join(format!("{name}.part")).exists());
        let mut zip = ZipArchive::new(File::open(&path).unwrap()).unwrap();
        let mut restored = Vec::new();
        std::io::Read::read_to_end(&mut zip.by_name("world/level.dat").unwrap(), &mut restored)
            .unwrap();
        assert_eq!(restored, b"before");
        drop(zip);

        let pending = file_name(t, Some("pending")).unwrap();
        let part = dir.join(DIR).join(format!("{pending}.part"));
        std::fs::write(&part, b"another backup in progress").unwrap();
        assert!(create(&dir, &config, &pending).is_err());
        assert_eq!(std::fs::read(&part).unwrap(), b"another backup in progress");
        assert!(!dir.join(DIR).join(&pending).exists());
        assert_eq!(std::fs::read(world.join("level.dat")).unwrap(), b"after");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn backup_output_cannot_alias_the_world() {
        let root = std::env::temp_dir().join(format!(
            "horizon-server-backup-alias-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let config = Config::default();
        let name = "2026-11-13_00-00-00-layout.zip";
        let mut accepted_overlaps = Vec::new();
        for (label, world_target, backup_target, succeeds) in [
            ("world-is-backups", Some("backups"), None, false),
            ("backups-in-world", None, Some("world/archives"), false),
            ("same-target", Some("store"), Some("store"), false),
            ("world-disjoint", Some("world-data"), None, true),
            ("backups-disjoint", None, Some("archive-data"), true),
            (
                "both-disjoint",
                Some("store/world"),
                Some("store/world-archives"),
                true,
            ),
        ] {
            let dir = root.join(label);
            std::fs::create_dir(&dir).unwrap();
            let world = dir.join("world");
            if let Some(target) = world_target {
                std::fs::create_dir_all(dir.join(target)).unwrap();
                std::os::unix::fs::symlink(target, &world).unwrap();
            } else {
                std::fs::create_dir(&world).unwrap();
            }
            let backups = dir.join(DIR);
            if let Some(target) = backup_target {
                std::fs::create_dir_all(dir.join(target)).unwrap();
                std::os::unix::fs::symlink(target, &backups).unwrap();
            }
            std::fs::create_dir(world.join("region")).unwrap();
            std::fs::write(world.join("level.dat"), b"synthetic world").unwrap();
            std::fs::write(world.join("region/r.0.0.mca"), b"synthetic region").unwrap();
            std::fs::write(dir.join("server.toml"), b"synthetic settings").unwrap();

            match create(&dir, &config, name) {
                Ok(path) if succeeds => {
                    let mut zip = ZipArchive::new(File::open(path).unwrap()).unwrap();
                    let mut entries: Vec<_> = zip
                        .file_names()
                        .map(|name| name.unwrap().into_owned())
                        .collect();
                    entries.sort();
                    assert_eq!(
                        entries,
                        ["server.toml", "world/level.dat", "world/region/r.0.0.mca"],
                        "{label}"
                    );
                    let mut bytes = Vec::new();
                    std::io::Read::read_to_end(
                        &mut zip.by_name("world/level.dat").unwrap(),
                        &mut bytes,
                    )
                    .unwrap();
                    assert_eq!(bytes, b"synthetic world", "{label}");
                }
                Ok(_) => accepted_overlaps.push(label),
                Err(error) => {
                    assert!(!succeeds, "{label}: {error}");
                    assert!(!backups.join(name).exists(), "{label}");
                }
            }
            assert!(!backups.join(format!("{name}.part")).exists(), "{label}");
            assert_eq!(
                std::fs::read(world.join("level.dat")).unwrap(),
                b"synthetic world",
                "{label}"
            );
            std::fs::remove_dir_all(&dir).unwrap();
        }
        std::fs::remove_dir_all(&root).unwrap();
        assert!(
            accepted_overlaps.is_empty(),
            "backup accepted overlapping paths: {accepted_overlaps:?}"
        );
    }

    #[test]
    fn back_up_and_restore() {
        let dir =
            std::env::temp_dir().join(format!("horizon-server-backup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let config = Config::default();
        let world = dir.join("world");
        std::fs::create_dir_all(world.join("region")).unwrap();
        std::fs::write(world.join("level.dat"), b"before").unwrap();
        std::fs::write(world.join("region/r.0.0.mca"), vec![7u8; 10_000]).unwrap();
        std::fs::write(world.join("session.lock"), b"x").unwrap();
        std::fs::write(dir.join("server.toml"), config::TEMPLATE).unwrap();
        let t = SystemTime::now();
        let name = file_name(t, None).unwrap();
        create(&dir, &config, &name).unwrap();
        assert_eq!(list(&dir).len(), 1);
        // The world changes, then the backup comes back.
        std::fs::write(world.join("level.dat"), b"after").unwrap();
        let moved = restore(&dir, &config, &find(&dir, &name[..10]).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(std::fs::read(world.join("level.dat")).unwrap(), b"before");
        assert_eq!(
            std::fs::read(world.join("region/r.0.0.mca")).unwrap().len(),
            10_000
        );
        assert!(!world.join("session.lock").exists());
        assert_eq!(std::fs::read(moved.join("level.dat")).unwrap(), b"after");
        // Only automatic backups are pruned.
        for i in 0..3 {
            let later = t + std::time::Duration::from_secs(10 + i);
            create(&dir, &config, &auto_name(later)).unwrap();
        }
        assert_eq!(prune(&dir, 1).len(), 2);
        let left: Vec<String> = list(&dir).into_iter().map(|b| b.name).collect();
        assert_eq!(left.len(), 2);
        assert!(left.iter().any(|n| n.ends_with("-manual.zip")));
        // A backup is found by its name, or the newest with a label.
        assert_eq!(find(&dir, "manual").unwrap(), name);
        assert_eq!(find(&dir, &left[0]).unwrap(), left[0]);
        assert_eq!(
            find(&dir, left[0].trim_end_matches(".zip")).unwrap(),
            left[0]
        );
        assert_eq!(find(&dir, "auto").unwrap(), left[0]);
        assert!(find(&dir, "nothing").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn restore_one_dimension_only() {
        let dir = std::env::temp_dir().join(format!("horizon-server-dim-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let config = Config::default();
        let world = dir.join("world");
        let overworld = world.join("dimensions/minecraft/overworld/region/r.0.0.mca");
        let nether = world.join("dimensions/minecraft/the_nether/region/r.0.0.mca");
        for (file, bytes) in [(&overworld, "ow-before"), (&nether, "nether-before")] {
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, bytes).unwrap();
        }
        std::fs::write(world.join("level.dat"), b"level-before").unwrap();
        let name = file_name(SystemTime::now(), None).unwrap();
        create(&dir, &config, &name).unwrap();
        std::fs::write(&overworld, "ow-after").unwrap();
        std::fs::write(&nether, "nether-after").unwrap();
        std::fs::write(world.join("level.dat"), b"level-after").unwrap();

        // Unknown or malformed dimensions change nothing.
        assert!(restore_dimension(&dir, &config, &name, "minecraft:the_end").is_err());
        assert!(restore_dimension(&dir, &config, &name, "../world").is_err());
        assert!(restore_dimension(&dir, &config, &name, "minecraft:../x").is_err());
        assert_eq!(std::fs::read(&nether).unwrap(), b"nether-after");

        // `the_nether` means `minecraft:the_nether`.
        let moved = restore_dimension(&dir, &config, &name, "the_nether")
            .unwrap()
            .unwrap();
        assert_eq!(std::fs::read(&nether).unwrap(), b"nether-before");
        assert_eq!(std::fs::read(&overworld).unwrap(), b"ow-after");
        assert_eq!(
            std::fs::read(world.join("level.dat")).unwrap(),
            b"level-after"
        );
        assert_eq!(
            std::fs::read(moved.join("region/r.0.0.mca")).unwrap(),
            b"nether-after"
        );
        assert!(!dir.join(".horizon/restore").exists());

        // A dimension the world lost comes back too, with nothing to put aside.
        std::fs::remove_dir_all(world.join("dimensions/minecraft/the_nether")).unwrap();
        assert!(
            restore_dimension(&dir, &config, &name, "minecraft:the_nether")
                .unwrap()
                .is_none()
        );
        assert_eq!(std::fs::read(&nether).unwrap(), b"nether-before");
        std::fs::remove_dir_all(&dir).unwrap();
    }
    #[test]
    fn prune_keeps_backups_labelled_by_hand_whatever_their_label() {
        let dir = std::env::temp_dir().join(format!("horizon-server-prune-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(DIR)).unwrap();
        let t = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        let names = [
            auto_name(t),
            auto_name(t + std::time::Duration::from_secs(60)),
            file_name(t, Some("nightly-auto")).unwrap(),
        ];
        for name in &names {
            std::fs::write(dir.join(DIR).join(name), b"zip").unwrap();
        }
        assert_eq!(prune(&dir, 0).len(), 2);
        assert!(dir.join(DIR).join(&names[2]).exists(), "kept: made by hand");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_backup_is_readable_by_its_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("horizon-server-mode-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("world")).unwrap();
        std::fs::write(dir.join("world/level.dat"), b"x").unwrap();
        std::fs::write(dir.join("server.toml"), crate::config::TEMPLATE).unwrap();
        let path = create(&dir, &Config::default(), "b.zip").unwrap();
        let mode = std::fs::metadata(path).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "{mode:o}");
    }

    /// `restore NAME` takes "the name given to `backup` (the newest with it)": the whole label after the
    /// time stamp, not any name that happens to end the same way.
    #[test]
    fn a_label_matches_only_the_whole_label() {
        let dir = std::env::temp_dir().join(format!(
            "horizon-server-backup-label-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(DIR)).unwrap();
        let test = "2026-11-01_00-00-00-test.zip";
        let my_test = "2026-11-02_00-00-00-my-test.zip";
        let update = "2026-11-03_00-00-00-before-update.zip";
        let nightly = "2026-11-04_00-00-00-nightly-auto.zip";
        for name in [test, my_test, update, nightly] {
            std::fs::write(dir.join(DIR).join(name), b"x").unwrap();
        }
        // The label `test` is the older file, not the newer one whose label ends with it.
        assert_eq!(find(&dir, "test").unwrap(), test);
        assert_eq!(find(&dir, "my-test").unwrap(), my_test);
        assert_eq!(find(&dir, "before-update").unwrap(), update);
        assert_eq!(find(&dir, "nightly-auto").unwrap(), nightly);
        // A label that only ends another one names no backup.
        assert!(find(&dir, "update").is_err());
        assert!(find(&dir, "auto").is_err());
        // The other ways in are as before: the file name, and a unique start of it.
        assert_eq!(find(&dir, my_test).unwrap(), my_test);
        assert_eq!(find(&dir, "2026-11-03").unwrap(), update);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// `auto` is the automatic backups' name: a backup made by hand under it would have exactly an
    /// automatic one's file name, and `prune` would delete it with them.
    #[test]
    fn a_hand_backup_cannot_take_the_automatic_name() {
        let t = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        assert!(file_name(t, Some("auto")).is_err());
        // The automatic name is still made, and a label that only contains `auto` is still a hand one.
        assert_eq!(auto_name(t), "2023-11-14_22-13-20-auto.zip");
        assert_eq!(
            file_name(t, Some("nightly-auto")).unwrap(),
            "2023-11-14_22-13-20-nightly-auto.zip"
        );
        // So `prune` can only meet automatic backups under that name.
        let dir =
            std::env::temp_dir().join(format!("horizon-server-hand-auto-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(DIR)).unwrap();
        let later = t + std::time::Duration::from_secs(60);
        let mut names = vec![auto_name(later)];
        names.extend(file_name(t, Some("auto")));
        for name in &names {
            std::fs::write(dir.join(DIR).join(name), b"zip").unwrap();
        }
        assert_eq!(prune(&dir, 0), vec![auto_name(later)]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
