//! Step 1b, the jars: for the mods the player asked to port, their own
//! download of each jar into `<work>/jars/`. Only from where the pack itself
//! points (a Modrinth pack's download URLs) or the same file on Modrinth's
//! public CDN (found by its SHA-1), and only kept when the hash matches; or
//! copied from a folder the player already has (an installed instance, a
//! downloads folder). CurseForge's own downloads need an API key, so a file
//! only CurseForge has is listed as needing one. `<work>/fetch.json` says
//! what happened to each file.
use crate::{
    import::{Pack, PackFile, sha1_hex},
    inventory::Record,
    modrinth::{self, Cache},
    pipeline::{Log, read_json, write_json},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha1::Sha1;
use sha2::{Digest, digest::Output};
use std::{
    collections::HashMap,
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

/// Hosts a Modrinth pack may download from (the `.mrpack` format's list).
const HOSTS: [&str; 4] = [
    "https://cdn.modrinth.com/",
    "https://github.com/",
    "https://raw.githubusercontent.com/",
    "https://gitlab.com/",
];
/// No mod jar is this large; a bigger answer is not one.
pub(crate) const MAX_JAR: u64 = 512 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum How {
    /// Already in `jars/` (or the imported instance).
    Present,
    /// Downloaded from the pack's URL or Modrinth's CDN, hash checked.
    Downloaded,
    /// Copied from the player's folder.
    Local,
    /// Only on CurseForge, whose downloads are coming soon (its API needs a paid key).
    NeedsKey,
    NotFound,
    Failed,
}

impl How {
    pub fn label(self) -> &'static str {
        match self {
            How::Present => "already here",
            How::Downloaded => "downloaded",
            How::Local => "copied from your folder",
            How::NeedsKey => "CurseForge downloads coming soon",
            How::NotFound => "not found",
            How::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Outcome {
    pub key: String,
    pub file: String,
    pub how: How,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Fetched {
    pub files: Vec<Outcome>,
}

impl Fetched {
    pub fn count(&self, how: How) -> usize {
        self.files.iter().filter(|o| o.how == how).count()
    }
    /// `<work>/fetch.json`, empty when the step has not run.
    pub fn load(work: &Path) -> Self {
        read_json(&work.join("fetch.json")).unwrap_or_default()
    }
}

/// A file name from a pack's index that stays inside `jars/`: one plain part, with no separator
/// of any platform, no drive and not `.` or `..`.
fn plain_file_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', ':', '\0'])
}

/// The jars of the records `want` picks that the work folder lacks.
pub fn fetch(
    work: &Path,
    records: &[Record],
    from: Option<&Path>,
    want: &dyn Fn(&Record) -> bool,
    log: Log,
) -> Result<Fetched, String> {
    let pack: Pack = read_json(&work.join("pack.json"))?;
    let cache = Cache::load(&work.join("modrinth.json"));
    let files: HashMap<&str, &PackFile> = pack.files.iter().map(|f| (f.path.as_str(), f)).collect();
    let jars = work.join("jars");
    fs::create_dir_all(&jars).map_err(|e| format!("{}: {e}", jars.display()))?;
    let local = from.map(Local::index);
    let wanted: Vec<&Record> = records.iter().filter(|r| want(r)).collect();
    let mut fetched = Fetched::default();
    for (i, r) in wanted.iter().enumerate() {
        let Some(file) = files.get(r.path.as_str()) else {
            continue;
        };
        let name = file.file_name();
        let outcome = |how, detail: String| Outcome {
            key: r.key.clone(),
            file: name.to_owned(),
            how,
            detail,
        };
        if !plain_file_name(name) {
            // A name like `..\..\x.dll` or `C:\x` would write outside jars/ on Windows.
            let o = outcome(How::Failed, format!("{name:?} is not a plain file name"));
            log(format!("{}: {} ({})", o.key, o.how.label(), o.detail));
            fetched.files.push(o);
            continue;
        }
        let dest = jars.join(name);
        let sha1 = file.sha1.as_deref();
        let o = if r.jar.is_some() || matches(&dest, sha1) {
            outcome(How::Present, String::new())
        } else if let Some(path) = local.as_ref().and_then(|l| l.find(file)) {
            match fs::copy(&path, &dest) {
                Ok(_) => outcome(How::Local, path.display().to_string()),
                Err(e) => outcome(How::Failed, format!("{}: {e}", path.display())),
            }
        } else if let Some(url) = url_of(file, &cache) {
            log(format!("{}/{} {name}", i + 1, wanted.len()));
            match download(&url, sha1, &dest, true) {
                Ok(()) => outcome(How::Downloaded, url),
                Err(e) => outcome(How::Failed, e),
            }
        } else if pack.format == "curseforge" || file.curseforge.is_some() {
            outcome(
                How::NeedsKey,
                "on CurseForge only: choose your installed instance, or put the jar in jars/"
                    .into(),
            )
        } else {
            outcome(How::NotFound, "no download link in the pack".into())
        };
        if matches!(o.how, How::Failed | How::NeedsKey | How::NotFound) {
            log(format!("{}: {} ({})", o.key, o.how.label(), o.detail));
        }
        fetched.files.push(o);
    }
    // The loader's own data (its convention tags), from its own repository.
    if let Some((dest, url)) = crate::worldgen::loader_jar(work, &pack.loader, &pack.loader_version)
        .filter(|(dest, _)| !dest.exists())
    {
        let checksum = format!("{url}.sha1");
        let sha1 = crate::net::agent(&checksum)
            .get(&checksum)
            .set("User-Agent", modrinth::AGENT)
            .call()
            .ok()
            .and_then(|r| r.into_string().ok())
            .map(|s| s.trim().to_owned());
        let made = dest.parent().map(fs::create_dir_all);
        match made
            .unwrap_or(Ok(()))
            .map_err(|e| e.to_string())
            .and_then(|()| {
                // Maven's `.sha1` or nothing: a jar whose checksum couldn't be read isn't taken.
                let sha1 = sha1
                    .as_deref()
                    .filter(|s| s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit()))
                    .ok_or_else(|| format!("{checksum}: no SHA-1"))?;
                download(&url, Some(sha1), &dest, true)
            }) {
            Ok(()) => log(format!("loader: {}", dest.display())),
            Err(e) => log(format!(
                "loader: {e} (structures keep only vanilla biome tags)"
            )),
        }
    }
    write_json(&work.join("fetch.json"), &fetched)?;
    log(format!(
        "jars: {} downloaded, {} copied, {} already here, {} on CurseForge only (CurseForge downloads coming soon), {} not found, {} failed",
        fetched.count(How::Downloaded),
        fetched.count(How::Local),
        fetched.count(How::Present),
        fetched.count(How::NeedsKey),
        fetched.count(How::NotFound),
        fetched.count(How::Failed)
    ));
    Ok(fetched)
}

/// Whether `path` holds the file with this SHA-1 (any file when none is known).
fn matches(path: &Path, sha1: Option<&str>) -> bool {
    match (fs::read(path), sha1) {
        (Ok(bytes), Some(h)) => sha1_hex(&bytes).eq_ignore_ascii_case(h),
        (Ok(_), None) => true,
        (Err(_), _) => false,
    }
}

/// The pack's own download link, else the same file on Modrinth.
pub fn url_of(file: &PackFile, cache: &Cache) -> Option<String> {
    let allowed = |u: &&str| HOSTS.iter().any(|h| u.starts_with(h));
    if let Some(u) = file.downloads.iter().map(String::as_str).find(allowed) {
        return Some(u.to_owned());
    }
    let sha1 = file.sha1.as_deref()?;
    cache.versions.get(sha1)?["files"]
        .as_array()?
        .iter()
        .find(|f| f["hashes"]["sha1"].as_str() == Some(sha1))?["url"]
        .as_str()
        .filter(allowed)
        .map(str::to_owned)
}

/// Downloads `url` to `dest`, kept only if its SHA-1 is `sha1`.
pub(crate) fn download(
    url: &str,
    sha1: Option<&str>,
    dest: &Path,
    overwrite: bool,
) -> Result<(), String> {
    let response = crate::net::get_from_hosts(
        url,
        &[
            "cdn.modrinth.com",
            "github.com",
            "raw.githubusercontent.com",
            "objects.githubusercontent.com",
            "release-assets.githubusercontent.com",
            "gitlab.com",
            "maven.neoforged.net",
        ],
    )?
    .ok_or_else(|| format!("{url}: not found"))?;
    store_download(response.into_reader(), sha1, dest, overwrite).map_err(|e| format!("{url}: {e}"))
}

/// Download exactly the manifest jar through the Modrinth CDN.
pub fn download_jar(
    url: &str,
    jar: &horizon_mod_package::manifest::JarRef,
    dest: &Path,
) -> Result<(), String> {
    let response =
        crate::net::get_from_hosts(url, &["cdn.modrinth.com"])?.ok_or("Modrinth jar not found")?;
    store::<sha2::Sha512>(
        response.into_reader(),
        Some(jar.sha512()),
        dest,
        true,
        "SHA-512",
    )
}

/// Hash and copy a jar stream, refusing inputs larger than 512 MiB.
/// Pass [`io::sink()`] as the writer when only the digest is needed.
pub fn hash_reader<D: Digest>(
    mut reader: impl Read,
    mut writer: impl Write,
) -> io::Result<Output<D>> {
    let mut hash = D::new();
    let mut size = 0u64;
    let mut buf = [0u8; 65536];
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        size += n as u64;
        if size > MAX_JAR {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "jar exceeds 512 MiB",
            ));
        }
        hash.update(&buf[..n]);
        writer.write_all(&buf[..n])?;
    }
    Ok(hash.finalize())
}

/// Stream into a private temporary file, publishing only after verification.
pub(crate) fn store_download(
    reader: impl Read,
    sha1: Option<&str>,
    dest: &Path,
    overwrite: bool,
) -> Result<(), String> {
    store::<Sha1>(reader, sha1, dest, overwrite, "SHA-1")
}

fn store<D: Digest>(
    reader: impl Read,
    expected: Option<&str>,
    dest: &Path,
    overwrite: bool,
    algorithm: &str,
) -> Result<(), String>
where
    Output<D>: std::fmt::LowerHex,
{
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let suffix = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let part = dest.with_file_name(format!(
        ".horizon-download-{}-{suffix}.part",
        std::process::id()
    ));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&part)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        let hash = hash_reader::<D>(reader, &mut file).map_err(|e| e.to_string())?;
        let got = format!("{hash:x}");
        if let Some(want) = expected.filter(|want| !want.eq_ignore_ascii_case(&got)) {
            return Err(format!("{algorithm} {got}, the pack says {want}"));
        }
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        if overwrite {
            fs::rename(&part, dest).map_err(|e| e.to_string())
        } else {
            match fs::hard_link(&part, dest) {
                Ok(()) => fs::remove_file(&part).map_err(|e| e.to_string()),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(e.to_string()),
                Err(_) => {
                    let mut out = fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(dest)
                        .map_err(|e| e.to_string())?;
                    let copied = std::io::copy(
                        &mut fs::File::open(&part).map_err(|e| e.to_string())?,
                        &mut out,
                    )
                    .map_err(|e| e.to_string());
                    drop(out);
                    if copied.is_err() {
                        let _ = fs::remove_file(dest);
                    }
                    copied.map(|_| ())
                }
            }
        }
    })();
    let _ = fs::remove_file(&part);
    result
}

/// The jars in a folder of the player's, by SHA-1 and by name, and what a
/// CurseForge app instance (`minecraftinstance.json`) says each
/// project's file is called.
#[derive(Default)]
struct Local {
    by_sha1: HashMap<String, PathBuf>,
    by_name: HashMap<String, PathBuf>,
    curseforge: HashMap<(u64, u64), String>,
}

impl Local {
    fn index(dir: &Path) -> Local {
        let mut local = Local::default();
        local.walk(dir, 3);
        if let Ok(text) = fs::read_to_string(dir.join("minecraftinstance.json"))
            && let Ok(v) = serde_json::from_str::<Value>(&text)
        {
            local.curseforge = curseforge_names(&v);
        }
        local
    }

    fn walk(&mut self, dir: &Path, depth: usize) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for path in entries.flatten().map(|e| e.path()) {
            if path.is_dir() {
                if depth > 0 {
                    self.walk(&path, depth - 1);
                }
            } else if path.extension().is_some_and(|x| x == "jar") {
                if let Ok(bytes) = fs::read(&path) {
                    self.by_sha1.insert(sha1_hex(&bytes), path.clone());
                }
                if let Some(n) = path.file_name() {
                    self.by_name
                        .insert(n.to_string_lossy().into_owned(), path.clone());
                }
            }
        }
    }

    fn find(&self, file: &PackFile) -> Option<PathBuf> {
        if let Some(h) = &file.sha1 {
            return self.by_sha1.get(&h.to_ascii_lowercase()).cloned();
        }
        let name = file
            .curseforge
            .and_then(|ids| self.curseforge.get(&ids))
            .map_or(file.file_name(), String::as_str);
        self.by_name.get(name).cloned()
    }
}

/// `installedAddons[]`: (project, file) → the jar's file name.
pub fn curseforge_names(instance: &Value) -> HashMap<(u64, u64), String> {
    instance["installedAddons"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| {
            let file = &a["installedFile"];
            Some((
                (a["addonID"].as_u64()?, file["id"].as_u64()?),
                file["fileName"].as_str()?.to_owned(),
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    #[ignore = "daily"]
    fn jar_hashing_accepts_512_mib_and_refuses_one_extra_byte() {
        let limit = 512 * 1024 * 1024;
        assert!(hash_reader::<sha2::Sha512>(io::repeat(0).take(limit), io::sink()).is_ok());
        let error =
            hash_reader::<sha2::Sha512>(io::repeat(0).take(limit + 1), io::sink()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn hashing_and_copying_share_the_digest_for_each_algorithm() {
        let mut copied = Vec::new();
        let hash = hash_reader::<Sha1>(&b"abc"[..], &mut copied).unwrap();
        assert_eq!(
            format!("{hash:x}"),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(copied, b"abc");
        let hash = hash_reader::<sha2::Sha512>(&b"abc"[..], io::sink()).unwrap();
        assert_eq!(
            format!("{hash:x}"),
            concat!(
                "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a",
                "2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
            )
        );
    }

    #[test]
    fn sha1_stream_is_verified_before_atomic_publication() {
        let root = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("port-pack-stream-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let dest = root.join("file.jar");
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for name in [
            "assets/a/test",
            "data/a/test",
            "META-INF/neoforge.mods.toml",
            "Dummy.class",
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"test").unwrap();
        }
        let bytes = zip.finish().unwrap().into_inner();
        let hash = sha1_hex(&bytes);
        fs::write(&dest, b"existing").unwrap();
        assert!(store_download(&b"wrong"[..], Some(&hash), &dest, true).is_err());
        assert_eq!(fs::read(&dest).unwrap(), b"existing");
        struct Interrupted;
        impl Read for Interrupted {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("interrupted"))
            }
        }
        assert!(store_download(Interrupted, Some(&hash), &dest, true).is_err());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_file(&dest).unwrap();
        store_download(&bytes[..], Some(&hash), &dest, true).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), bytes);
        let sha1 = sha1_hex(b"legacy");
        fs::remove_file(&dest).unwrap();
        store_download(&b"legacy"[..], Some(&sha1), &dest, true).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"legacy");
        assert!(store_download(&b"bad"[..], Some(&sha1), &dest, true).is_err());
        assert_eq!(fs::read(&dest).unwrap(), b"legacy");
        assert!(store_download(&b"new"[..], None, &dest, false).is_err());
        assert_eq!(fs::read(&dest).unwrap(), b"legacy");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sha512_publication_is_verified_and_atomic_between_downloads() {
        let root = std::env::temp_dir().join(format!("sha512-store-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let dest = root.join("source.jar");
        let bytes = b"a synthetic jar download";
        let hash = format!("{:x}", sha2::Sha512::digest(bytes));
        assert!(store::<sha2::Sha512>(&b"wrong"[..], Some(&hash), &dest, true, "SHA-512").is_err());
        assert!(!dest.exists());
        std::thread::scope(|scope| {
            let save = || store::<sha2::Sha512>(&bytes[..], Some(&hash), &dest, true, "SHA-512");
            let first = scope.spawn(save);
            let second = scope.spawn(save);
            first.join().unwrap().unwrap();
            second.join().unwrap().unwrap();
        });
        assert_eq!(fs::read(&dest).unwrap(), bytes);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_pack_index_name_must_stay_inside_jars() {
        assert!(plain_file_name("sodium-0.6.jar"));
        for name in ["", ".", "..", "..\\..\\evil.dll", "C:evil.jar", "a/b.jar"] {
            assert!(!plain_file_name(name), "{name:?}");
        }
    }

    fn file(sha1: Option<&str>, downloads: &[&str]) -> PackFile {
        PackFile {
            path: "mods/a.jar".into(),
            sha1: sha1.map(str::to_owned),
            downloads: downloads.iter().map(|d| (*d).to_owned()).collect(),
            ..PackFile::default()
        }
    }

    #[test]
    fn links_come_from_the_pack_or_modrinth_and_only_allowed_hosts() {
        let mut cache = Cache::default();
        cache.versions.insert(
            "ab".into(),
            json!({"files": [
                {"hashes": {"sha1": "cd"}, "url": "https://cdn.modrinth.com/other.jar"},
                {"hashes": {"sha1": "ab"}, "url": "https://cdn.modrinth.com/a.jar"}]}),
        );
        let pack_link = file(
            Some("ab"),
            &["https://evil.example/a.jar", "https://github.com/a.jar"],
        );
        assert_eq!(
            url_of(&pack_link, &cache).as_deref(),
            Some("https://github.com/a.jar")
        );
        let modrinth = file(Some("ab"), &["https://evil.example/a.jar"]);
        assert_eq!(
            url_of(&modrinth, &cache).as_deref(),
            Some("https://cdn.modrinth.com/a.jar")
        );
        assert_eq!(url_of(&file(Some("ef"), &[]), &cache), None);
        assert_eq!(url_of(&file(None, &[]), &cache), None);
    }

    #[test]
    fn reads_a_curseforge_instance() {
        let instance = json!({"installedAddons": [
            {"addonID": 238222, "installedFile": {"id": 5101366, "fileName": "jei-1.21.1.jar"}},
            {"addonID": 1, "installedFile": {"fileName": "no-id.jar"}}]});
        let names = curseforge_names(&instance);
        assert_eq!(names.len(), 1);
        assert_eq!(names[&(238222, 5101366)], "jei-1.21.1.jar");
    }

    #[test]
    fn copies_the_players_jars_and_lists_what_needs_a_key() {
        let root = std::env::temp_dir().join(format!("horizon-fetch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (work, mine) = (root.join("work"), root.join("instance/mods"));
        fs::create_dir_all(&mine).unwrap();
        fs::create_dir_all(&work).unwrap();
        fs::write(mine.join("renamed.jar"), b"jar a").unwrap();
        fs::write(mine.join("cf.jar"), b"jar b").unwrap();
        fs::write(
            root.join("instance/minecraftinstance.json"),
            json!({"installedAddons": [{"addonID": 7, "installedFile": {"id": 8, "fileName": "cf.jar"}}]})
                .to_string(),
        )
        .unwrap();
        let pack = Pack {
            format: "curseforge".into(),
            files: vec![
                PackFile {
                    path: "mods/a.jar".into(),
                    sha1: Some(sha1_hex(b"jar a")),
                    ..PackFile::default()
                },
                PackFile {
                    path: "mods/curseforge-7-8".into(),
                    curseforge: Some((7, 8)),
                    ..PackFile::default()
                },
                PackFile {
                    path: "mods/curseforge-9-9".into(),
                    curseforge: Some((9, 9)),
                    ..PackFile::default()
                },
            ],
            ..Pack::default()
        };
        write_json(&work.join("pack.json"), &pack).unwrap();
        let records: Vec<Record> = pack
            .files
            .iter()
            .map(|f| Record {
                key: f.file_name().into(),
                path: f.path.clone(),
                ..Record::default()
            })
            .collect();
        let fetched = fetch(
            &work,
            &records,
            Some(&root.join("instance")),
            &|_| true,
            &|_| {},
        )
        .unwrap();
        let hows: Vec<How> = fetched.files.iter().map(|o| o.how).collect();
        assert_eq!(hows, [How::Local, How::Local, How::NeedsKey]);
        assert_eq!(fs::read(work.join("jars/a.jar")).unwrap(), b"jar a");
        assert_eq!(
            fs::read(work.join("jars/curseforge-7-8")).unwrap(),
            b"jar b"
        );
        // A second run finds them in place.
        let again = fetch(&work, &records, None, &|_| true, &|_| {}).unwrap();
        assert_eq!(again.count(How::Present), 2);
        assert_eq!(Fetched::load(&work).count(How::Present), 2);
        fs::remove_dir_all(&root).unwrap();
    }
}
