//! Content-validated overlay reuse. Live symlink files/subtrees need no content
//! scan; merged, extracted and copied bytes do. Topology and link targets are
//! always checked, including vanilla directories materialized by mirror.
use super::{Pack, Source, build_overlay_uncached};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, io, io::Read, path::Path, path::PathBuf};

// Bump when overlay/merge semantics change. Markers are disposable cache data.
const VERSION: u32 = 1;

#[derive(Default, Serialize, Deserialize)]
pub(super) struct Dependencies {
    entries: Vec<(PathBuf, bool, String)>,
    #[serde(skip)]
    failed: bool,
    #[serde(skip)]
    tracking: bool,
}
impl Dependencies {
    fn record(&mut self, path: &Path, directory: bool) {
        if !self.tracking {
            return;
        }
        match dependency_signature(path, directory) {
            Ok(signature) => self.entries.push((path.into(), directory, signature)),
            Err(_) => self.failed = true,
        }
    }
    pub(super) fn file(&mut self, path: &Path) {
        self.record(path, false);
    }
    pub(super) fn directory(&mut self, path: &Path) {
        self.record(path, true);
    }
    fn valid(&self) -> bool {
        !self.failed
            && self.entries.iter().all(|(path, directory, signature)| {
                dependency_signature(path, *directory).is_ok_and(|s| s == *signature)
            })
    }
}

#[derive(Serialize, Deserialize)]
struct Marker {
    version: u32,
    vanilla: PathBuf,
    packs: String,
    dependencies: Dependencies,
    output: String,
}

fn finish(hash: Sha256) -> String {
    format!("{:x}", hash.finalize())
}
// Length framing keeps distinct paths/configurations unambiguous.
fn field(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}
fn content(hash: &mut Sha256, path: &Path) -> io::Result<()> {
    let mut file = fs::File::open(path)?;
    hash.update(file.metadata()?.len().to_le_bytes());
    let mut buffer = [0; 64 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(())
}
fn entries(path: &Path) -> io::Result<Vec<PathBuf>> {
    let mut entries = fs::read_dir(path)?
        .map(|e| e.map(|e| e.path()))
        .collect::<io::Result<Vec<_>>>()?;
    entries.sort();
    Ok(entries)
}
fn dependency_signature(path: &Path, directory: bool) -> io::Result<String> {
    let mut hash = Sha256::new();
    match fs::metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => field(&mut hash, b"missing"),
        Err(e) => return Err(e),
        Ok(meta) if directory && meta.is_dir() => {
            field(&mut hash, b"directory");
            for entry in entries(path)? {
                field(&mut hash, entry.file_name().unwrap().as_encoded_bytes());
                // mirror's decisions use the followed file type, including symlinks.
                field(&mut hash, &[u8::from(entry.is_dir())]);
            }
        }
        Ok(_) if directory => field(&mut hash, b"not-directory"),
        Ok(_) => content(&mut hash, path)?,
    }
    Ok(finish(hash))
}
fn tree(hash: &mut Sha256, path: &Path, pack: Option<&Pack>) -> io::Result<()> {
    tree_inner(hash, path, pack, &mut BTreeSet::new())
}

fn needs_content(path: &Path, pack: Option<&Pack>) -> bool {
    let Some(pack) = pack.filter(|_| cfg!(unix)) else {
        return true;
    };
    let Source::Dir(root) = &pack.source else {
        return true;
    };
    let relative = path
        .strip_prefix(root)
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/");
    // Ordinary folder resources are live Unix file symlinks. Rewrites and
    // replacements at that path are visible without rebuilding. Merged JSON
    // and configuration bytes are materialized and need content validation.
    relative == "pack.mcmeta"
        || super::merge_kind(&relative).is_some()
        || pack.overlays.iter().any(|overlay| {
            path.strip_prefix(root.join(overlay.trim_end_matches('/')))
                .ok()
                .is_some_and(|name| {
                    super::merge_kind(&name.to_string_lossy().replace('\\', "/")).is_some()
                })
        })
}
fn tree_inner(
    hash: &mut Sha256,
    path: &Path,
    pack: Option<&Pack>,
    active: &mut BTreeSet<PathBuf>,
) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.is_symlink() {
        field(hash, b"link");
        field(hash, fs::read_link(path)?.as_os_str().as_encoded_bytes());
        if pack.is_none() {
            return Ok(());
        }
    }
    if path.is_dir() {
        let canonical = if pack.is_some() {
            let canonical = fs::canonicalize(path)?;
            if !active.insert(canonical.clone()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "pack symlink cycle",
                ));
            }
            Some(canonical)
        } else {
            None
        };
        field(hash, b"directory");
        for entry in entries(path)? {
            field(hash, entry.file_name().unwrap().as_encoded_bytes());
            tree_inner(hash, &entry, pack, active)?;
        }
        field(hash, b"end-directory");
        if let Some(canonical) = canonical {
            active.remove(&canonical);
        }
    } else {
        field(hash, b"file");
        if needs_content(path, pack) {
            content(hash, path)?;
        }
    }
    Ok(())
}
fn output_signature(out: &Path) -> io::Result<String> {
    let mut hash = Sha256::new();
    tree(&mut hash, out, None)?;
    Ok(finish(hash))
}
fn pack_signature(packs: &[Pack]) -> io::Result<String> {
    let mut hash = Sha256::new();
    field(&mut hash, &[u8::from(cfg!(unix))]);
    for pack in packs {
        if matches!(pack.source, Source::Dir(_))
            && pack.overlays.iter().any(|overlay| {
                Path::new(overlay).components().any(|component| {
                    matches!(
                        component,
                        std::path::Component::ParentDir
                            | std::path::Component::RootDir
                            | std::path::Component::Prefix(_)
                    )
                })
            })
        {
            // The builder historically accepts external overlay roots. Preserve
            // that behavior, but don't cache dependencies outside this pack tree.
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "external pack overlay",
            ));
        }
        field(&mut hash, pack.id.as_bytes());
        // Includes source kind/path, overlay order and compiled filter settings.
        field(&mut hash, format!("{:?}", pack.source).as_bytes());
        field(&mut hash, format!("{:?}", pack.overlays).as_bytes());
        field(&mut hash, format!("{:?}", pack.filter).as_bytes());
        match &pack.source {
            Source::Dir(path) => tree(&mut hash, path, Some(pack))?,
            Source::Zip(path) => content(&mut hash, path)?,
        }
    }
    Ok(finish(hash))
}

pub(super) fn marker_path(out: &Path) -> PathBuf {
    // Outside the resource tree: no resource-name collisions or mirror changes.
    let mut name = out.as_os_str().to_os_string();
    name.push(".overlay-cache.json");
    PathBuf::from(name)
}

fn absolute(path: &Path) -> io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.into())
    } else {
        std::env::current_dir().map(|cwd| cwd.join(path))
    }
}

fn lock_path(out: &Path) -> PathBuf {
    marker_path(out).with_extension("lock")
}

pub(super) fn build(vanilla: &Path, packs: &[Pack], out: &Path) -> io::Result<()> {
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    // Keep this inode persistent: deleting it would let waiters and new callers
    // acquire different locks. Closing the file releases the advisory OS lock.
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path(out))?;
    lock.lock()?;
    // Absolute paths also prevent cwd changes from reusing relative symlinks.
    // Keep root symlink aliases live, as mirror does for individual resources.
    let vanilla = absolute(vanilla)?;
    let packs: Vec<Pack> = packs
        .iter()
        .map(|pack| {
            let mut pack = pack.clone();
            match &mut pack.source {
                Source::Dir(path) | Source::Zip(path) => {
                    *path = absolute(path)?;
                }
            }
            Ok(pack)
        })
        .collect::<io::Result<_>>()?;
    let marker_path = marker_path(out);
    let signature = pack_signature(&packs);
    if let Ok(signature) = &signature
        && let Ok(bytes) = fs::read(&marker_path)
        && let Ok(marker) = serde_json::from_slice::<Marker>(&bytes)
        && marker.version == VERSION
        && marker.vanilla == vanilla
        && marker.packs == *signature
        && marker.dependencies.valid()
        && output_signature(out).is_ok_and(|s| s == marker.output)
    {
        return Ok(());
    }
    // An interrupted/failed rebuild must never leave a reusable marker.
    match fs::remove_file(&marker_path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let mut dependencies = Dependencies {
        tracking: signature.is_ok(),
        ..Dependencies::default()
    };
    build_overlay_uncached(&vanilla, &packs, out, &mut dependencies)?;
    if let Ok(signature) = signature
        && dependencies.valid()
        && pack_signature(&packs).is_ok_and(|s| s == signature)
    {
        let marker = Marker {
            version: VERSION,
            vanilla,
            packs: signature,
            dependencies,
            output: output_signature(out)?,
        };
        // PathBuf serialization requires UTF-8; unusual resource root names
        // can still build normally without a reusable marker.
        let Ok(bytes) = serde_json::to_vec(&marker) else {
            return Ok(());
        };
        // Publish only a complete marker, after all output writes succeeded.
        let temporary = marker_path.with_extension("tmp");
        fs::write(&temporary, bytes)?;
        fs::rename(&temporary, &marker_path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{packs, text::Text};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "horizon-overlay-cache-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn write(&self, name: &str, bytes: &[u8]) {
            let path = self.0.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        fn dir_pack(&self, name: &str) -> Pack {
            self.write(
                &format!("{name}/pack.mcmeta"),
                br#"{"pack":{"description":"test","pack_format":97}}"#,
            );
            packs::pack(name.into(), Text::empty(), Source::Dir(self.0.join(name))).unwrap()
        }
        fn rebuild_matches(&self, selected: &[Pack]) {
            let vanilla = self.0.join("vanilla");
            let cached = self.0.join("cached");
            packs::build_overlay(&vanilla, selected, &cached).unwrap();
            build_overlay_uncached(
                &vanilla,
                selected,
                &self.0.join("reference"),
                &mut Dependencies::default(),
            )
            .unwrap();
            assert_eq!(
                output_signature(&cached).unwrap(),
                output_signature(&self.0.join("reference")).unwrap()
            );
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    // Stored/deflated ZIP fixture using the production reader, no zip dependency.
    fn archive(files: &[(&str, Vec<u8>)], deflate: bool) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, body) in files {
            let data = if deflate {
                miniz_oxide::deflate::compress_to_vec(body, 6)
            } else {
                body.clone()
            };
            let offset = out.len() as u32;
            out.extend(0x0403_4b50u32.to_le_bytes());
            out.extend([20, 0, 0, 0]);
            out.extend((if deflate { 8u16 } else { 0 }).to_le_bytes());
            out.extend([0; 8]);
            out.extend((data.len() as u32).to_le_bytes());
            out.extend((body.len() as u32).to_le_bytes());
            out.extend((name.len() as u16).to_le_bytes());
            out.extend(0u16.to_le_bytes());
            out.extend(name.as_bytes());
            out.extend(&data);
            central.extend(0x0201_4b50u32.to_le_bytes());
            central.extend([20, 0, 20, 0, 0, 0]);
            central.extend((if deflate { 8u16 } else { 0 }).to_le_bytes());
            central.extend([0; 8]);
            central.extend((data.len() as u32).to_le_bytes());
            central.extend((body.len() as u32).to_le_bytes());
            central.extend((name.len() as u16).to_le_bytes());
            central.extend([0; 12]);
            central.extend(offset.to_le_bytes());
            central.extend(name.as_bytes());
        }
        let start = out.len() as u32;
        out.extend(&central);
        out.extend(0x0605_4b50u32.to_le_bytes());
        out.extend([0; 4]);
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((central.len() as u32).to_le_bytes());
        out.extend(start.to_le_bytes());
        out.extend([0; 2]);
        out
    }

    #[test]
    fn directory_inputs_config_and_output_invalidate() {
        let f = Fixture::new();
        let resource = "assets/minecraft/lang/en_us.json";
        f.write(&format!("vanilla/{resource}"), br#"{"v":"a"}"#);
        f.write("vanilla/assets/minecraft/textures/untouched/a", b"live");
        let mut low = f.dir_pack("low");
        let high = f.dir_pack("high");
        f.write(&format!("low/{resource}"), br#"{"p":"a"}"#);
        f.write(&format!("high/{resource}"), br#"{"p":"b"}"#);
        f.rebuild_matches(&[low.clone(), high.clone()]);
        let out = f.0.join("cached");
        let marker = marker_path(&out);
        // A hit keeps the actual merged file (and marker) in place.
        let time = fs::metadata(out.join(resource))
            .unwrap()
            .modified()
            .unwrap();
        let marker_time = fs::metadata(&marker).unwrap().modified().unwrap();
        f.rebuild_matches(&[low.clone(), high.clone()]);
        assert_eq!(
            fs::metadata(out.join(resource))
                .unwrap()
                .modified()
                .unwrap(),
            time
        );
        assert_eq!(
            fs::metadata(&marker).unwrap().modified().unwrap(),
            marker_time
        );
        // Same-length rewrites cannot hide behind file size or preserved mtime.
        let path = f.0.join(format!("high/{resource}"));
        let original_time = fs::metadata(&path).unwrap().modified().unwrap();
        fs::write(&path, br#"{"p":"c"}"#).unwrap();
        fs::File::open(path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(original_time))
            .unwrap();
        f.rebuild_matches(&[low.clone(), high.clone()]);
        f.write(&format!("vanilla/{resource}"), br#"{"v":"b"}"#);
        f.rebuild_matches(&[low.clone(), high.clone()]);
        f.write("vanilla/assets/minecraft/lang/new.json", b"new");
        f.rebuild_matches(&[low.clone(), high.clone()]);
        fs::remove_file(f.0.join("vanilla/assets/minecraft/lang/new.json")).unwrap();
        f.rebuild_matches(&[low.clone(), high.clone()]);
        f.write("high/assets/minecraft/lang/added.json", br#"{"a":1}"#);
        f.rebuild_matches(&[low.clone(), high.clone()]);
        fs::remove_file(f.0.join("high/assets/minecraft/lang/added.json")).unwrap();
        f.rebuild_matches(&[low.clone(), high.clone()]);
        f.rebuild_matches(&[high.clone(), low.clone()]);
        f.rebuild_matches(std::slice::from_ref(&low));
        low.overlays.push("extra".into());
        f.write(&format!("low/extra/{resource}"), br#"{"p":"overlay"}"#);
        f.rebuild_matches(std::slice::from_ref(&low));
        low.filter = packs::filter_of(&serde_json::json!({"filter":{"block":[{"path":"lang"}]}}));
        f.rebuild_matches(std::slice::from_ref(&low));
        fs::write(out.join(resource), b"corrupt").unwrap();
        f.rebuild_matches(std::slice::from_ref(&low));
        fs::remove_file(out.join(resource)).unwrap();
        f.rebuild_matches(std::slice::from_ref(&low));
        fs::write(&marker, b"corrupt").unwrap();
        f.rebuild_matches(std::slice::from_ref(&low));
        fs::remove_file(&marker).unwrap();
        f.rebuild_matches(std::slice::from_ref(&low));
        fs::remove_dir_all(&out).unwrap();
        f.rebuild_matches(std::slice::from_ref(&low));
        #[cfg(unix)]
        {
            low.filter.clear();
            f.rebuild_matches(std::slice::from_ref(&low));
            // Unrelated vanilla content remains live through the directory symlink.
            let before = fs::read(&marker).unwrap();
            f.write("vanilla/assets/minecraft/textures/untouched/a", b"new live");
            f.write(
                "vanilla/assets/minecraft/textures/untouched/b",
                b"added live",
            );
            f.rebuild_matches(std::slice::from_ref(&low));
            assert_eq!(fs::read(&marker).unwrap(), before);
            assert_eq!(
                fs::read(out.join("assets/minecraft/textures/untouched/b")).unwrap(),
                b"added live"
            );
        }
        // Missing selected source disables reuse; the original directory builder
        // still tolerates it, and does not publish a marker for an incomplete input.
        fs::remove_dir_all(f.0.join("low")).unwrap();
        f.rebuild_matches(&[low]);
        assert!(!marker.exists());
    }

    #[test]
    fn zip_content_and_failed_build_invalidate() {
        let f = Fixture::new();
        f.write("vanilla/facts.json", b"[]");
        let name = "assets/minecraft/textures/a";
        let files = |data: &[u8]| {
            vec![
                (
                    "pack.mcmeta",
                    br#"{"pack":{"description":"test","pack_format":97}}"#.to_vec(),
                ),
                (name, data.to_vec()),
            ]
        };
        f.write("pack.zip", &archive(&files(b"aaa"), false));
        let pack = packs::pack(
            "zip".into(),
            Text::empty(),
            Source::Zip(f.0.join("pack.zip")),
        )
        .unwrap();
        f.rebuild_matches(std::slice::from_ref(&pack));
        f.rebuild_matches(std::slice::from_ref(&pack));
        f.write("pack.zip", &archive(&files(b"bbb"), false));
        f.rebuild_matches(std::slice::from_ref(&pack));
        assert_eq!(fs::read(f.0.join("cached").join(name)).unwrap(), b"bbb");
        // The ZIP opens and enumerates, then its unsupported method fails after
        // mirror has created directories. No reusable marker survives.
        let mut broken = archive(&files(b"ccc"), false);
        let central = broken
            .windows(4)
            .position(|s| s == [0x50, 0x4b, 0x01, 0x02])
            .unwrap();
        let second = central + 46 + "pack.mcmeta".len();
        broken[second + 10..second + 12].copy_from_slice(&99u16.to_le_bytes());
        f.write("pack.zip", &broken);
        let out = f.0.join("cached");
        assert!(
            packs::build_overlay(&f.0.join("vanilla"), std::slice::from_ref(&pack), &out).is_err()
        );
        assert!(!marker_path(&out).exists());
        f.write("pack.zip", &archive(&files(b"ddd"), true));
        f.rebuild_matches(std::slice::from_ref(&pack));
        fs::remove_file(f.0.join("pack.zip")).unwrap();
        assert!(packs::build_overlay(&f.0.join("vanilla"), &[pack], &out).is_err());
        assert!(!marker_path(&out).exists());
    }

    #[cfg(unix)]
    #[test]
    fn unrelated_directory_pack_symlink_cycle_disables_reuse() {
        let f = Fixture::new();
        f.write("vanilla/facts.json", b"[]");
        let pack = f.dir_pack("pack");
        f.write("pack/assets/minecraft/textures/a", b"pack");
        fs::create_dir_all(f.0.join("pack/docs")).unwrap();
        std::os::unix::fs::symlink(f.0.join("pack"), f.0.join("pack/docs/back")).unwrap();
        f.rebuild_matches(std::slice::from_ref(&pack));
        assert!(!marker_path(&f.0.join("cached")).exists());
    }

    #[cfg(unix)]
    #[test]
    fn linked_pack_bytes_stay_live_and_topology_invalidates() {
        let f = Fixture::new();
        f.write("vanilla/facts.json", b"[]");
        let pack = f.dir_pack("pack");
        let name = "assets/minecraft/textures/a";
        f.write(&format!("pack/{name}"), b"aaa");
        f.rebuild_matches(std::slice::from_ref(&pack));
        let out = f.0.join("cached");
        let marker = marker_path(&out);
        let before = fs::metadata(&marker).unwrap().modified().unwrap();
        let path = f.0.join("pack").join(name);
        let original_time = fs::metadata(&path).unwrap().modified().unwrap();
        fs::write(&path, b"bbb").unwrap();
        fs::File::open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(original_time))
            .unwrap();
        f.rebuild_matches(std::slice::from_ref(&pack));
        assert_eq!(fs::read(out.join(name)).unwrap(), b"bbb");
        assert_eq!(fs::metadata(&marker).unwrap().modified().unwrap(), before);
        // Replacing a regular file at the same linked path is also already live.
        f.write("replacement", b"ccc");
        fs::rename(f.0.join("replacement"), &path).unwrap();
        f.rebuild_matches(std::slice::from_ref(&pack));
        assert_eq!(fs::read(out.join(name)).unwrap(), b"ccc");
        assert_eq!(fs::metadata(&marker).unwrap().modified().unwrap(), before);
        // A changed source symlink identity is fingerprinted, and a corrupt
        // output symlink is repaired even when its target bytes happen to match.
        fs::remove_file(&path).unwrap();
        f.write("other", b"ddd");
        std::os::unix::fs::symlink(f.0.join("other"), &path).unwrap();
        f.rebuild_matches(std::slice::from_ref(&pack));
        assert_eq!(fs::read(out.join(name)).unwrap(), b"ddd");
        fs::remove_file(out.join(name)).unwrap();
        std::os::unix::fs::symlink(f.0.join("other"), out.join(name)).unwrap();
        f.rebuild_matches(std::slice::from_ref(&pack));
        assert_eq!(fs::read_link(out.join(name)).unwrap(), path);
        fs::remove_file(&path).unwrap();
        f.rebuild_matches(std::slice::from_ref(&pack));
        assert!(fs::symlink_metadata(out.join(name)).is_err());
    }

    #[test]
    fn normalized_and_external_overlay_roots_keep_merged_bytes_current() {
        let f = Fixture::new();
        let name = "assets/minecraft/lang/en_us.json";
        f.write(&format!("vanilla/{name}"), br#"{"v":1}"#);
        let mut pack = f.dir_pack("pack");
        f.write(&format!("pack/extra/{name}"), br#"{"p":1}"#);
        for overlay in ["extra/", "./extra", "extra/./", "extra//"] {
            pack.overlays = vec![overlay.into()];
            f.rebuild_matches(std::slice::from_ref(&pack));
            let marker = marker_path(&f.0.join("cached"));
            assert!(marker.exists());
            f.write(&format!("pack/extra/{name}"), br#"{"p":2}"#);
            f.rebuild_matches(std::slice::from_ref(&pack));
            assert_eq!(
                fs::read(f.0.join("cached").join(name)).unwrap(),
                br#"{"p":2,"v":1}"#
            );
            f.write(&format!("pack/extra/{name}"), br#"{"p":1}"#);
        }
        f.write(&format!("outside/{name}"), br#"{"p":3}"#);
        for overlay in [
            "../outside".into(),
            f.0.join("outside").to_string_lossy().into_owned(),
        ] {
            pack.overlays = vec![overlay];
            f.rebuild_matches(std::slice::from_ref(&pack));
            assert!(!marker_path(&f.0.join("cached")).exists());
            f.write(&format!("outside/{name}"), br#"{"p":4}"#);
            f.rebuild_matches(std::slice::from_ref(&pack));
            assert_eq!(
                fs::read(f.0.join("cached").join(name)).unwrap(),
                br#"{"p":4,"v":1}"#
            );
            f.write(&format!("outside/{name}"), br#"{"p":3}"#);
        }
    }

    #[cfg(unix)]
    #[test]
    fn root_symlink_aliases_stay_live_and_retargeted_merges_invalidate() {
        let f = Fixture::new();
        let name = "assets/minecraft/lang/en_us.json";
        f.write("vanilla-a/facts.json", b"a");
        f.write("vanilla-b/facts.json", b"b");
        f.write(&format!("vanilla-a/{name}"), br#"{"v":"a"}"#);
        f.write(&format!("vanilla-b/{name}"), br#"{"v":"b"}"#);
        let mut pack = f.dir_pack("pack-a");
        f.dir_pack("pack-b");
        f.write("pack-a/assets/minecraft/textures/a", b"a");
        f.write("pack-b/assets/minecraft/textures/a", b"b");
        f.write(&format!("pack-a/{name}"), br#"{"p":"a"}"#);
        f.write(&format!("pack-b/{name}"), br#"{"p":"b"}"#);
        let vanilla = f.0.join("vanilla");
        let source = f.0.join("pack");
        std::os::unix::fs::symlink(f.0.join("vanilla-a"), &vanilla).unwrap();
        std::os::unix::fs::symlink(f.0.join("pack-a"), &source).unwrap();
        pack.source = Source::Dir(source.clone());
        f.rebuild_matches(std::slice::from_ref(&pack));
        fs::remove_file(&vanilla).unwrap();
        fs::remove_file(&source).unwrap();
        std::os::unix::fs::symlink(f.0.join("vanilla-b"), &vanilla).unwrap();
        std::os::unix::fs::symlink(f.0.join("pack-b"), &source).unwrap();
        let out = f.0.join("cached");
        assert_eq!(fs::read(out.join("facts.json")).unwrap(), b"b");
        assert_eq!(
            fs::read(out.join("assets/minecraft/textures/a")).unwrap(),
            b"b"
        );
        f.rebuild_matches(std::slice::from_ref(&pack));
        assert_eq!(fs::read(out.join(name)).unwrap(), br#"{"p":"b","v":"b"}"#);
    }

    #[test]
    fn concurrent_selections_wait_and_never_publish_mismatched_markers() {
        use std::{sync::mpsc, time::Duration};
        let f = Fixture::new();
        let name = "assets/minecraft/lang/en_us.json";
        f.write(&format!("vanilla/{name}"), br#"{"v":1}"#);
        let a = f.dir_pack("a");
        let b = f.dir_pack("b");
        f.write(&format!("a/{name}"), br#"{"p":"a"}"#);
        f.write(&format!("b/{name}"), br#"{"p":"b"}"#);
        let out = f.0.join("cached");
        let held = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path(&out))
            .unwrap();
        held.lock().unwrap();
        let (started, starts) = mpsc::channel();
        let (done, finishes) = mpsc::channel();
        std::thread::scope(|scope| {
            for pack in [&a, &b] {
                let started = started.clone();
                let done = done.clone();
                let vanilla = f.0.join("vanilla");
                let out = &out;
                scope.spawn(move || {
                    started.send(()).unwrap();
                    for _ in 0..20 {
                        packs::build_overlay(&vanilla, std::slice::from_ref(pack), out).unwrap();
                    }
                    done.send(()).unwrap();
                });
            }
            starts.recv().unwrap();
            starts.recv().unwrap();
            assert!(matches!(
                finishes.recv_timeout(Duration::from_millis(50)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ));
            drop(held);
        });
        finishes.recv().unwrap();
        finishes.recv().unwrap();
        for pack in [&a, &b, &a, &b] {
            f.rebuild_matches(std::slice::from_ref(pack));
            let expected = if pack.id == "a" {
                br#"{"p":"a","v":1}"#
            } else {
                br#"{"p":"b","v":1}"#
            };
            assert_eq!(fs::read(out.join(name)).unwrap(), expected);
        }
        assert!(lock_path(&out).exists());
    }

    #[test]
    #[ignore = "reproducible overlay benchmark; run with --ignored --nocapture"]
    fn benchmark_overlay_reuse() {
        use std::{hint::black_box, time::Instant};
        let f = Fixture::new();
        let pack = f.dir_pack("pack");
        let mut files = vec![(
            "pack.mcmeta".to_owned(),
            fs::read(f.0.join("pack/pack.mcmeta")).unwrap(),
        )];
        // 2,048 overrides (32 MiB), 32 merged language files, 8,192 live
        // vanilla textures (128 MiB). Synthetic incompressible payloads approximate
        // already-compressed PNG bytes; texture decoding is outside this benchmark.
        let mut random = 0x1234_5678u32;
        let texture: Vec<u8> = (0..16384)
            .map(|_| {
                random ^= random << 13;
                random ^= random >> 17;
                random ^= random << 5;
                (random >> 24) as u8
            })
            .collect();
        for i in 0..8192 {
            let name = format!("assets/minecraft/textures/block/{i}.png");
            f.write(&format!("vanilla/{name}"), &texture);
            if i < 2048 {
                f.write(&format!("pack/{name}"), &texture);
                files.push((name, texture.clone()));
            }
        }
        for i in 0..32 {
            let name = format!("assets/minecraft/lang/{i}.json");
            let vanilla = serde_json::to_vec(&serde_json::Value::Object(
                (0..2000)
                    .map(|j| {
                        (
                            format!("key{j}"),
                            serde_json::json!(format!("Vanilla translation {j}")),
                        )
                    })
                    .collect(),
            ))
            .unwrap();
            let body = serde_json::to_vec(&serde_json::Value::Object(
                (0..1000)
                    .map(|j| {
                        (
                            format!("key{j}"),
                            serde_json::json!(format!("Pack translation {j}")),
                        )
                    })
                    .collect(),
            ))
            .unwrap();
            f.write(&format!("vanilla/{name}"), &vanilla);
            f.write(&format!("pack/{name}"), &body);
            files.push((name, body));
        }
        let borrowed: Vec<_> = files
            .iter()
            .map(|(name, body)| (name.as_str(), body.clone()))
            .collect();
        f.write("pack.zip", &archive(&borrowed, true));
        println!(
            "fixture ZIP bytes={}",
            fs::metadata(f.0.join("pack.zip")).unwrap().len()
        );
        let zip = packs::pack(
            "zip".into(),
            Text::empty(),
            Source::Zip(f.0.join("pack.zip")),
        )
        .unwrap();
        let median = |mut times: Vec<f64>| {
            times.sort_by(f64::total_cmp);
            times[times.len() / 2]
        };
        for (kind, pack) in [("directory", pack), ("deflated zip", zip)] {
            f.rebuild_matches(std::slice::from_ref(&pack));
            let vanilla = f.0.join("vanilla");
            let out = f.0.join("cached");
            let mut baseline = Vec::new();
            let mut cold = Vec::new();
            let mut hit = Vec::new();
            for _ in 0..11 {
                let start = Instant::now();
                build_overlay_uncached(
                    black_box(&vanilla),
                    black_box(std::slice::from_ref(&pack)),
                    black_box(&f.0.join("reference")),
                    &mut Dependencies::default(),
                )
                .unwrap();
                baseline.push(start.elapsed().as_secs_f64() * 1000.0);
                fs::remove_file(marker_path(&out)).unwrap();
                let start = Instant::now();
                packs::build_overlay(
                    black_box(&vanilla),
                    black_box(std::slice::from_ref(&pack)),
                    black_box(&out),
                )
                .unwrap();
                cold.push(start.elapsed().as_secs_f64() * 1000.0);
                let start = Instant::now();
                packs::build_overlay(
                    black_box(&vanilla),
                    black_box(std::slice::from_ref(&pack)),
                    black_box(&out),
                )
                .unwrap();
                hit.push(start.elapsed().as_secs_f64() * 1000.0);
                assert_eq!(
                    output_signature(&out).unwrap(),
                    output_signature(&f.0.join("reference")).unwrap()
                );
            }
            println!(
                "{kind}: baseline={:.3} ms cold={:.3} ms unchanged={:.3} ms (11 medians, full validation)",
                median(baseline),
                median(cold),
                median(hit)
            );
        }
    }

    #[test]
    #[ignore = "content-heavy few-file directory benchmark"]
    fn benchmark_content_heavy_directory() {
        use std::{hint::black_box, time::Instant};
        let f = Fixture::new();
        f.write("vanilla/facts.json", b"[]");
        let pack = f.dir_pack("pack");
        let bytes = vec![42; 8 * 1024 * 1024];
        for i in 0..4 {
            f.write(&format!("pack/assets/minecraft/textures/{i}.png"), &bytes);
        }
        let vanilla = f.0.join("vanilla");
        let out = f.0.join("cached");
        f.rebuild_matches(std::slice::from_ref(&pack));
        let mut timings = [Vec::new(), Vec::new(), Vec::new()];
        for _ in 0..11 {
            let start = Instant::now();
            build_overlay_uncached(
                black_box(&vanilla),
                black_box(std::slice::from_ref(&pack)),
                &f.0.join("reference"),
                &mut Dependencies::default(),
            )
            .unwrap();
            timings[0].push(start.elapsed().as_secs_f64() * 1000.0);
            fs::remove_file(marker_path(&out)).unwrap();
            for times in &mut timings[1..] {
                let start = Instant::now();
                packs::build_overlay(
                    black_box(&vanilla),
                    black_box(std::slice::from_ref(&pack)),
                    black_box(&out),
                )
                .unwrap();
                times.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            assert_eq!(
                output_signature(&out).unwrap(),
                output_signature(&f.0.join("reference")).unwrap()
            );
        }
        let medians: Vec<_> = timings
            .into_iter()
            .map(|mut times| {
                times.sort_by(f64::total_cmp);
                times[5]
            })
            .collect();
        println!(
            "4 x 8 MiB directory textures: baseline={:.3} ms cold={:.3} ms unchanged={:.3} ms (11 medians, full validation)",
            medians[0], medians[1], medians[2]
        );
    }
}
