//! The horizon-mixins tool for `[[native.mixin]]` builds (`horizon_mod_package::mixins_tool`):
//! once per pin and patch the launcher downloads the prebuilt tool from its release, and where
//! that release has none for this computer it downloads the tool's source at the pin, applies the
//! carried patch only to the exact text it was written against, and builds it.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use horizon_mod_package::mixins_tool::{self, PATCH, PATCH_NAME};
use sha2::{Digest, Sha256};

use super::{root, run, Step, MIXINS_REV};
use crate::i18n::{tr, trf};
use crate::rust_toolchain::{self, Toolchain};

const SUMS: &str = "SHA256SUMS";
/// The archive's largest plausible size, packed or unpacked: a corrupt or hostile one stops there.
const LIMIT: u64 = 512 << 20;
/// A checksum list holds a line per platform.
const SUMS_LIMIT: u64 = 1 << 20;
/// What an unpacking launcher's own folder inside the tool's folder starts with.
const STAGING: &str = ".prebuilt-";
/// Set by `native-build --mixins-from-source`: the prebuilt tool is not looked for.
pub(super) static FROM_SOURCE: AtomicBool = AtomicBool::new(false);

/// One file of the tool's release; `None` when the release has no such file.
type Get<'a> = &'a dyn Fn(&str) -> Result<Option<Box<dyn Read>>, String>;

/// The tool's executable inside its folder.
fn executable(dir: &Path) -> PathBuf {
    dir.join("bin")
        .join(format!("horizon-mixins{}", std::env::consts::EXE_SUFFIX))
}

/// The patched tool at [`MIXINS_REV`], fetched once per pin and patch.
pub(super) fn tool(rust: &Toolchain, progress: &impl Fn(Step)) -> Result<PathBuf, String> {
    // Inside the pin's own tool folder, so storage cleanup counts it with the pin.
    let dir = root()
        .join("tools")
        .join(&MIXINS_REV[..12])
        .join(format!("patched-{}", &mixins_tool::patch_hash()[..12]));
    let bin = executable(&dir);
    if bin.is_file() {
        return Ok(bin);
    }
    progress(Step::Tool);
    let started = std::time::Instant::now();
    let seconds = || format!("{:.1}", started.elapsed().as_secs_f64());
    let target = rust.host.target;
    let release = format!(
        "{}/{}",
        horizon_addresses::MIXINS_TOOL_RELEASES,
        mixins_tool::release_tag()
    );
    let get = |name: &str| {
        Ok(
            crate::backend::get_unless_missing(&format!("{release}/{name}"))?
                .map(|response| Box::new(response.into_reader()) as Box<dyn Read>),
        )
    };
    let from_source = FROM_SOURCE.load(Ordering::Relaxed);
    let get: Get = &get;
    let got = install(&dir, target, (!from_source).then_some(get), &|| {
        if !from_source {
            super::append_log(&trf(
                "horizon-mixins tool: no prebuilt for {target}; building from source (once per pin)",
                &[("target", target)],
            ))?;
        }
        build(rust, &dir)
    })?;
    match got {
        Got::Cached => {}
        Got::Downloaded => super::append_log(&trf(
            "horizon-mixins tool: downloaded in {seconds} s (once per pin)",
            &[("seconds", &seconds())],
        ))?,
        Got::Built => super::append_log(&trf(
            "horizon-mixins tool: fetched and built in {seconds} s (once per pin)",
            &[("seconds", &seconds())],
        ))?,
    }
    Ok(bin)
}

/// How [`install`] left the tool in its folder.
#[derive(Debug, PartialEq)]
enum Got {
    /// Another launcher put it there while this one waited.
    Cached,
    Downloaded,
    Built,
}

/// Makes `dir` hold the tool, one launcher at a time: the folder's lock is held across processes
/// until the tool is in place, and a launcher that waited finds it there. With `release`, the
/// prebuilt tool is looked for first; `build` runs only where there is none.
fn install(
    dir: &Path,
    target: &str,
    release: Option<Get>,
    build: &dyn Fn() -> Result<(), String>,
) -> Result<Got, String> {
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let lock = fs::File::create(dir.join(".lock")).map_err(|e| e.to_string())?;
    lock.lock().map_err(|e| e.to_string())?;
    // What a launcher that was killed while unpacking left: no other one is unpacking now.
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())?.flatten() {
        if entry.file_name().to_string_lossy().starts_with(STAGING) {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
    if executable(dir).is_file() {
        return Ok(Got::Cached);
    }
    if let Some(get) = release {
        if prebuilt(dir, target, get)? {
            return Ok(Got::Downloaded);
        }
    }
    build()?;
    Ok(Got::Built)
}

/// The tool built from its pinned source with the carried patch, into `dir`.
fn build(rust: &Toolchain, dir: &Path) -> Result<(), String> {
    let partial = dir.join(".source");
    let top = super::unpack_archive(
        &format!("{}/{MIXINS_REV}", horizon_addresses::HORIZON_MIXINS_SOURCE),
        &partial,
        256 << 20,
    )?;
    mixins_tool::apply(&top, PATCH).map_err(|why| {
        format!("E_MIXINS_PATCH: {PATCH_NAME} does not apply to horizon-mixins {MIXINS_REV}: {why}")
    })?;
    let mut cmd = rust.command(&rust.cargo());
    rust_toolchain::low_priority(&mut cmd);
    rust.configure_cargo(&mut cmd);
    cmd.args(["install", "--locked", "--path"])
        .arg(top.join("crates/horizon-mixins-cli"))
        .arg("--root")
        .arg(dir);
    let built = run(cmd, "Building horizon-mixins", |_| {});
    let _ = fs::remove_dir_all(&partial);
    built
}

/// Puts the release's prebuilt tool for `target` in `dir/bin`, under [`install`]'s lock.
/// `Ok(false)` only when the release, or its archive for this target, does not exist: the caller
/// builds from source. A download that fails, an archive the release's `SHA256SUMS` does not list
/// or does not match, one built from another source, patch or target, and one without the
/// executable are errors, never a source build.
fn prebuilt(dir: &Path, target: &str, get: Get) -> Result<bool, String> {
    let name = mixins_tool::archive(target);
    let download = |why: String| {
        trf(
            "The horizon-mixins tool couldn't be downloaded ({why}). Try again.",
            &[("why", &why)],
        )
    };
    let invalid = |why: &str| {
        trf(
            "The horizon-mixins tool {name} can't be used: {why}. Its release is invalid; report it on the game.",
            &[("name", &name), ("why", why)],
        )
    };
    let Some(sums) = get(SUMS).map_err(download)? else {
        return Ok(false);
    };
    let Some(body) = get(&name).map_err(download)? else {
        return Ok(false);
    };
    let mut listing = String::new();
    sums.take(SUMS_LIMIT + 1)
        .read_to_string(&mut listing)
        .map_err(|e| download(e.to_string()))?;
    if listing.len() as u64 > SUMS_LIMIT {
        return Err(invalid(tr("its checksum list is too large")));
    }
    let Some(expected) = super::prebuilt::expected_sum(&listing, &name) else {
        return Err(invalid(tr("its release lists no checksum for it")));
    };
    let mut bytes = Vec::new();
    body.take(LIMIT)
        .read_to_end(&mut bytes)
        .map_err(|e| download(e.to_string()))?;
    if format!("{:x}", Sha256::digest(&bytes)) != expected {
        return Err(trf(
            "The horizon-mixins tool {name} doesn't match its release's checksum. Try again.",
            &[("name", &name)],
        ));
    }
    // This launcher's own folder, removed however this returns.
    let staging = tempfile::Builder::new()
        .prefix(STAGING)
        .tempdir_in(dir)
        .map_err(|e| e.to_string())?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(&bytes[..]).take(LIMIT));
    for entry in tar.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.header().entry_type();
        if kind.is_file() || kind.is_dir() {
            entry.unpack_in(staging.path()).map_err(|e| {
                trf(
                    "Could not unpack the horizon-mixins tool: {why}",
                    &[("why", &e.to_string())],
                )
            })?;
        }
    }
    let identity =
        fs::read_to_string(staging.path().join(mixins_tool::IDENTITY)).unwrap_or_default();
    if !identity.starts_with(&mixins_tool::identity(target)) {
        return Err(invalid(tr(
            "it was not built from the source, patch and target this launcher pins",
        )));
    }
    if !executable(staging.path()).is_file() {
        return Err(invalid(tr("it holds no executable")));
    }
    // Under the lock, with no executable in it: what an interrupted source build left, never
    // another launcher's tool.
    let bin = dir.join("bin");
    let _ = fs::remove_dir_all(&bin);
    fs::rename(staging.path().join("bin"), &bin).map_err(|e| e.to_string())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    const TARGET: &str = "x86_64-unknown-linux-gnu";

    /// A release archive holding `identity` and, with `tool`, the executable.
    fn archive(identity: &str, tool: bool) -> Vec<u8> {
        let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        let exe = format!("bin/horizon-mixins{}", std::env::consts::EXE_SUFFIX);
        let name = if tool { exe.as_str() } else { "bin/README" };
        for (path, text) in [(name, "the tool"), (mixins_tool::IDENTITY, identity)] {
            let mut header = tar::Header::new_gnu();
            header.set_size(text.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            tar.append_data(&mut header, path, text.as_bytes()).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap()
    }

    fn good() -> Vec<u8> {
        archive(&mixins_tool::identity(TARGET), true)
    }

    fn sums(archive: &[u8]) -> String {
        format!(
            "{:x}  {}\n",
            Sha256::digest(archive),
            mixins_tool::archive(TARGET)
        )
    }

    /// A release with these two files; `None` is a 404.
    fn release(
        sums: Option<String>,
        archive: Option<Vec<u8>>,
    ) -> impl Fn(&str) -> Result<Option<Box<dyn Read>>, String> {
        move |name| {
            let file = if name == SUMS {
                sums.clone().map(String::into_bytes)
            } else {
                assert_eq!(name, mixins_tool::archive(TARGET));
                archive.clone()
            };
            Ok(file.map(|bytes| Box::new(std::io::Cursor::new(bytes)) as Box<dyn Read>))
        }
    }

    /// No staging folder outlives a call, whatever it returned.
    fn left(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn the_prebuilt_tool_is_checked_then_put_in_place() {
        let dir = tempfile::tempdir().unwrap();
        // The compiler's line after the checked ones is information only.
        let packed = archive(
            &format!("{}rustc = any\n", mixins_tool::identity(TARGET)),
            true,
        );
        let get = release(Some(sums(&packed)), Some(packed));
        assert_eq!(prebuilt(dir.path(), TARGET, &get), Ok(true));
        assert_eq!(
            fs::read_to_string(executable(dir.path())).unwrap(),
            "the tool"
        );
        assert_eq!(left(dir.path()), ["bin"]);
    }

    #[test]
    fn only_a_missing_release_or_archive_means_a_source_build() {
        let dir = tempfile::tempdir().unwrap();
        let packed = good();
        for get in [release(None, None), release(Some(sums(&packed)), None)] {
            assert_eq!(prebuilt(dir.path(), TARGET, &get), Ok(false));
        }
        assert!(left(dir.path()).is_empty());
    }

    #[test]
    fn an_archive_its_release_lists_no_checksum_for_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let packed = good();
        let other = sums(&packed).replace(TARGET, "aarch64-apple-darwin");
        let why = prebuilt(dir.path(), TARGET, &release(Some(other), Some(packed))).unwrap_err();
        assert!(why.contains("lists no checksum for it"), "{why}");
        let long = "0".repeat(SUMS_LIMIT as usize + 1);
        let why = prebuilt(dir.path(), TARGET, &release(Some(long), Some(good()))).unwrap_err();
        assert!(why.contains("checksum list is too large"), "{why}");
        assert!(left(dir.path()).is_empty());
    }

    #[test]
    fn a_failed_download_is_an_error_and_never_a_source_build() {
        let dir = tempfile::tempdir().unwrap();
        let why = prebuilt(dir.path(), TARGET, &|_| Err("connection reset".into())).unwrap_err();
        assert!(
            why.contains("connection reset") && why.contains("Try again"),
            "{why}"
        );
    }

    #[test]
    fn an_archive_that_is_not_the_releases_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let get = release(Some(sums(b"another archive")), Some(good()));
        let why = prebuilt(dir.path(), TARGET, &get).unwrap_err();
        assert!(
            why.contains("doesn't match its release's checksum"),
            "{why}"
        );
        assert!(left(dir.path()).is_empty());
    }

    #[test]
    fn a_tool_built_from_another_pin_or_target_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        for identity in [
            mixins_tool::identity("aarch64-apple-darwin"),
            mixins_tool::identity(TARGET).replace(MIXINS_REV, &"0".repeat(40)),
            String::new(),
        ] {
            let packed = archive(&identity, true);
            let get = release(Some(sums(&packed)), Some(packed));
            let why = prebuilt(dir.path(), TARGET, &get).unwrap_err();
            assert!(
                why.contains("was not built from the source, patch and target"),
                "{why}"
            );
            assert!(left(dir.path()).is_empty());
        }
    }

    #[test]
    fn an_archive_without_the_executable_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let packed = archive(&mixins_tool::identity(TARGET), false);
        let get = release(Some(sums(&packed)), Some(packed));
        let why = prebuilt(dir.path(), TARGET, &get).unwrap_err();
        assert!(why.contains("it holds no executable"), "{why}");
        assert!(left(dir.path()).is_empty());
    }

    /// Two launchers at once: one downloads, the other waits for the lock and finds the tool.
    /// Two threads of one process on this OS: it does not prove the lock between processes, or
    /// on Windows.
    #[test]
    fn two_installers_download_once_and_both_find_the_tool() {
        let dir = tempfile::tempdir().unwrap();
        let tool = dir.path().join("tool");
        let packed = good();
        let listing = sums(&packed);
        let downloads = AtomicUsize::new(0);
        let get = |name: &str| {
            if name != SUMS {
                downloads.fetch_add(1, Ordering::SeqCst);
                // Long enough for the other installer to reach the lock.
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            release(Some(listing.clone()), Some(packed.clone()))(name)
        };
        let never = || -> Result<(), String> { panic!("a prebuilt exists") };
        let start = std::sync::Barrier::new(2);
        // A killed launcher's leftover goes with the first installer.
        fs::create_dir_all(tool.join(".prebuilt-left/bin")).unwrap();
        let got = std::thread::scope(|scope| {
            let run = || {
                start.wait();
                install(&tool, TARGET, Some(&get), &never).unwrap()
            };
            let first = scope.spawn(run);
            let second = scope.spawn(run);
            [first.join().unwrap(), second.join().unwrap()]
        });
        assert!(
            got == [Got::Downloaded, Got::Cached] || got == [Got::Cached, Got::Downloaded],
            "{got:?}"
        );
        assert_eq!(downloads.load(Ordering::SeqCst), 1);
        assert_eq!(fs::read_to_string(executable(&tool)).unwrap(), "the tool");
        assert_eq!(left(&tool), [".lock", "bin"]);
    }

    #[test]
    fn without_a_release_the_tool_is_built() {
        let dir = tempfile::tempdir().unwrap();
        let built = AtomicUsize::new(0);
        let build = || {
            built.fetch_add(1, Ordering::SeqCst);
            Ok(())
        };
        let none = release(None, None);
        assert_eq!(
            install(dir.path(), TARGET, Some(&none), &build),
            Ok(Got::Built)
        );
        // `--mixins-from-source`: no release is asked.
        assert_eq!(install(dir.path(), TARGET, None, &build), Ok(Got::Built));
        assert_eq!(built.load(Ordering::SeqCst), 2);
    }
}
