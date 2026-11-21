//! The mod SDK of one game build, as the launcher fetches and keeps it (SDK design §1, §3, §4,
//! §14): its signed index, `sdk.json`, first; then only the slices the instance's route needs,
//! each unpacked into a folder of its own by [`sdk::unpack`] and used again after a cheap stamp
//! check; and the small files the fit test and planning read (`surface.json`, `build-data`).
//!
//! Where it comes from: `<base>/sdk-<build id>/<file>`, the base being this repository's release
//! downloads, or `HORIZON_SDK_BASE` ([`BASE_VAR`]): a URL (a mirror) or a folder (a copy for
//! offline use). Every base is checked the same way: `SHA256SUMS` signed by a release key for the
//! tag `sdk-<build id>`, `sdk.json`'s SHA-256 in it, then each file's size and SHA-256 in
//! `sdk.json`. A game release carries a copy of its SDK's `sdk.json` under the same signature
//! scheme, and that copy is where the game's build id comes from ([`build_of`]).
//!
//! What is kept, under the launcher's native build folder:
//! - `sdk/<build id>/`: `sdk.json` (as verified), `surface.json`, `build-data/`; a few MB, kept,
//!   so a mod's fit is told without a download once a build was seen.
//! - `sdk/versions/<game version>`: the build id that version's release named.
//! - `prebuilt/<build id>-<target>[-server]/`: the compiled slices (`runtime/`, `api/`,
//!   `engine/`), each with `<slice>.stamp` beside it, written while the build lease is held; the
//!   folder is one storage unit, evicted whole with what an unpack left in it (`.partial`,
//!   `.lock`, a download).
use super::prebuilt::{Package, Route};
use super::Target;
use crate::i18n::{tr, trf};
use crate::rust_toolchain;
use horizon_platform::mod_package::mod_surface;
use horizon_platform::mod_package::sdk::{self, Asset, Index, Kind, Side, INDEX};
use horizon_release_sign::{PublicKey, SIGNATURE, SIGNATURE_V2, SUMS};
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Replaces the base SDK releases are read from: a URL, or a folder holding `sdk-<build id>/`
/// folders (and game releases' folders, for their build ids).
pub const BASE_VAR: &str = "HORIZON_SDK_BASE";
/// Where SDK releases are read from by default.
const DEFAULT_BASE: &str = horizon_addresses::SDK_RELEASES;
/// The most an index, checksum list or signature file is read to, in bytes.
const SMALL: u64 = 1 << 20;
/// The most a slice or the build data unpacks to: a corrupt or hostile archive stops there.
const UNPACKED_LIMIT: u64 = 8 << 30;
/// What [`sdk::unpack`]'s destination gets beside it once it is unpacked and listed.
const STAMP_SUFFIX: &str = ".stamp";

/// Why an SDK can't be used. Every reader of downloaded bytes here ends in one of these.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The build has no mod SDK (its SDK build failed or is still running).
    NoSdk { build: String },
    /// The base can't be reached and what is needed isn't kept: `bytes` to download, when known.
    Offline { build: String, bytes: Option<u64> },
    /// The SDK's signature or checksums don't hold: never used.
    Unsigned { build: String, why: String },
    /// The SDK is newer than this launcher reads (`what`: its schema, package format or Rust).
    UpdateLauncher { build: String, what: String },
    /// The SDK was made with an older Rust than this launcher compiles with.
    OlderRust { build: String, rust: String },
    /// A file of the SDK was refused: why, by name.
    Refused { build: String, why: String },
    /// No SDK is published for this computer.
    Host,
    /// Reading or writing the launcher's copy failed.
    Cache(String),
    /// The player stopped the build: said as [`super::CANCELLED`], word for word, so the card
    /// shows no error for it.
    Cancelled,
}

/// What a player can do about a refusal other than waiting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WayOn {
    /// Play another game build (instance settings, Installation).
    OtherBuild,
    /// The problem is Horizon's: report it.
    Report,
    /// Nothing is wrong but the connection: build again once it is back.
    Retry,
}

/// The way on a refusal's `text` offers, found from the sentences it was written with (a build
/// reaches the player as text only).
pub fn way_on(text: &str) -> Option<WayOn> {
    // An offline refusal's size, when known, sits after the build: its opening says it all.
    let offline_opening = |text: &str| {
        let probe = Refusal::Offline {
            build: "\0".to_owned(),
            bytes: None,
        }
        .to_string();
        probe
            .split('\0')
            .next()
            .is_some_and(|opening| text.contains(opening))
    };
    let said = |refusal: Refusal| {
        let probe = refusal.to_string();
        probe.split('\0').all(|piece| text.contains(piece))
    };
    let mark = || "\0".to_owned();
    if said(Refusal::NoSdk { build: mark() })
        || said(Refusal::OlderRust {
            build: mark(),
            rust: mark(),
        })
    {
        Some(WayOn::OtherBuild)
    } else if said(Refusal::Unsigned {
        build: mark(),
        why: mark(),
    }) {
        Some(WayOn::Report)
    } else if offline_opening(text) {
        Some(WayOn::Retry)
    } else {
        None
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::NoSdk { build } => trf(
                "The mod tools for {build} aren't ready yet. Pick another version, or try again later.",
                &[("build", build)],
            ),
            Self::Offline { build, bytes: Some(bytes) } => trf(
                "The launcher is offline and the mod tools for {build} aren't downloaded yet ({size}). Connect to the internet and try again.",
                &[("build", build), ("size", &megabytes(*bytes))],
            ),
            Self::Offline { build, bytes: None } => trf(
                "The launcher is offline and the mod tools for {build} aren't downloaded yet. Connect to the internet and try again.",
                &[("build", build)],
            ),
            Self::Unsigned { build, .. } => trf(
                "The mod tools for {build} didn't pass Horizon's safety check, so nothing was installed and your game is untouched. Try again later; if it keeps happening, please report it.",
                &[("build", build)],
            ),
            Self::UpdateLauncher { build, what } => trf(
                "The mod tools for {build} need a newer launcher. Update the launcher, then build again.\n({what})",
                &[("build", build), ("what", what)],
            ),
            Self::OlderRust { build, rust } => trf(
                "The mod tools for {build} are older than this launcher can build with. Pick a newer version.\n(Rust {rust}; this launcher builds with {ours})",
                &[("build", build), ("rust", rust), ("ours", rust_toolchain::VERSION)],
            ),
            Self::Refused { build, why } => trf(
                "The mod tools for {build} were refused: {why}",
                &[("build", build), ("why", why)],
            ),
            Self::Host => {
                tr("Native mods can't be built on this computer: Horizon has no game package for its system yet.").into()
            }
            Self::Cache(why) => trf(
                "Could not keep the mod tools: {why}",
                &[("why", why)],
            ),
            Self::Cancelled => super::CANCELLED.into(),
        };
        f.write_str(&text)
    }
}

/// `bytes` as people read a download's size: whole megabytes, at least 1.
fn megabytes(bytes: u64) -> String {
    trf(
        "{count} MB",
        &[("count", &bytes.div_ceil(1_000_000).max(1).to_string())],
    )
}

/// A build as people read it: as the Versions page names its Nightly ("the Nov 13 update"),
/// found from an installed version when the index has no name, else "your Horizon version"
/// (an imported build or a snapshot), never a commit. Anything but a build id (a mod's manifest
/// may say anything) is shown as it is, cut where a character ends.
pub fn shown(commit: &str, nightly: Option<&str>) -> String {
    let name = |name: &str| crate::nightly::update_name(name).unwrap_or_else(|| name.to_owned());
    match nightly.map(str::to_owned).or_else(|| nightly_of(commit)) {
        Some(nightly) => name(&nightly),
        None if super::is_revision(commit) => crate::i18n::tr("your Horizon version").to_string(),
        None => commit.get(..7).unwrap_or(commit).to_owned(),
    }
}

/// Build `commit`'s Nightly name, from its kept index or an installed version made from it.
fn nightly_of(commit: &str) -> Option<String> {
    if !super::is_revision(commit) {
        return None;
    }
    fs::read(root().join(commit).join(INDEX))
        .ok()
        .and_then(|bytes| Index::build_of(&bytes).ok())
        .and_then(|build| build.nightly)
        .or_else(|| {
            crate::backend::installed()
                .into_iter()
                .find(|tag| tag.starts_with("nightly-") && tag.ends_with(&commit[..7]))
        })
}

/// Build `commit`'s name as the Versions page gives it, when it is known here (a kept index, or
/// an installed version). `commit` may come from a mod's manifest: anything but a build id is
/// never looked up.
pub fn name_of(commit: &str) -> Option<String> {
    nightly_of(commit).map(|name| shown(commit, Some(&name)))
}

/// Build `commit` as players read it, for a mod's fit: [`name_of`], else "another Horizon
/// version", never a commit.
pub fn known_name(commit: &str) -> String {
    match name_of(commit) {
        Some(name) => name,
        None if super::is_revision(commit) => {
            crate::i18n::tr("another Horizon version").to_string()
        }
        None => commit.get(..7).unwrap_or(commit).to_owned(),
    }
}

/// Where SDK releases are read from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Base {
    /// `<url>/<tag>/<file>`.
    Url(String),
    /// `<folder>/<tag>/<file>`.
    Folder(PathBuf),
}

/// Why a file couldn't be read from a base.
#[derive(Debug, PartialEq, Eq)]
enum Unread {
    /// No connection.
    Offline,
    /// Any other failure, said.
    Failed(String),
}

impl Base {
    /// [`BASE_VAR`]'s base, else this repository's releases.
    pub(super) fn from_env() -> Self {
        Self::parse(std::env::var_os(BASE_VAR))
    }

    /// An `http://` or `https://` value is a URL (without a trailing `/`); any other value a
    /// folder; nothing, or an empty value, the default base.
    fn parse(value: Option<std::ffi::OsString>) -> Self {
        let Some(value) = value.filter(|value| !value.is_empty()) else {
            return Self::Url(DEFAULT_BASE.into());
        };
        match value.to_str() {
            Some(text) if text.starts_with("https://") || text.starts_with("http://") => {
                Self::Url(text.trim_end_matches('/').to_owned())
            }
            _ => Self::Folder(PathBuf::from(value)),
        }
    }

    /// The file `name` of the release `tag`, open, with its length when the base says it;
    /// `Ok(None)` when the base has no such file. `tag` and `name` are plain names: a tag this
    /// module makes, and a name `sdk.json` lists (`Asset::parts_of` accepts only plain ones).
    fn open(&self, tag: &str, name: &str) -> Result<Option<Box<dyn Read>>, Unread> {
        match self {
            Self::Url(url) => {
                match crate::backend::get_unless_missing(&format!("{url}/{tag}/{name}")) {
                    Ok(Some(response)) => Ok(Some(Box::new(response.into_reader()))),
                    Ok(None) => Ok(None),
                    Err(why) if why.starts_with("No connection") => Err(Unread::Offline),
                    Err(why) => Err(Unread::Failed(why)),
                }
            }
            Self::Folder(folder) => {
                let path = folder.join(tag).join(name);
                match fs::File::open(&path) {
                    Ok(file) => Ok(Some(Box::new(file))),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                    Err(error) => Err(Unread::Failed(format!("{}: {error}", path.display()))),
                }
            }
        }
    }

    /// A small file whole (at most [`SMALL`] bytes; a larger one is refused, never cut).
    fn small(&self, tag: &str, name: &str) -> Result<Option<Vec<u8>>, Unread> {
        let Some(body) = self.open(tag, name)? else {
            return Ok(None);
        };
        let mut bytes = Vec::new();
        body.take(SMALL + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| Unread::Failed(format!("{name}: {error}")))?;
        if bytes.len() as u64 > SMALL {
            return Err(Unread::Failed(trf(
                "{name} is larger than {size}",
                &[("name", name), ("size", &megabytes(SMALL))],
            )));
        }
        Ok(Some(bytes))
    }

    /// Where the base keeps the release `tag`'s file `name` on this computer, when it does.
    fn local(&self, tag: &str, name: &str) -> Option<PathBuf> {
        match self {
            Self::Folder(folder) => Some(folder.join(tag).join(name)),
            Self::Url(_) => None,
        }
    }
}

/// The SDK release's tag for `commit`: what its checksums are signed for.
fn tag(commit: &str) -> String {
    format!("sdk-{commit}")
}

/// The launcher's SDK folder: `<build dir>/native/sdk`.
fn root() -> PathBuf {
    super::root().join("sdk")
}

/// A name part no other writer uses: this launcher's route checks run beside its build, which
/// may fetch the same SDK files at once, without the build lease (#19294).
fn unique() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let next = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{}-{next}", std::process::id())
}

/// Write `bytes` to `path` whole or not at all: beside it first, then renamed into place.
fn write_whole(path: &Path, bytes: &[u8]) -> Result<(), Refusal> {
    let cache = |error: std::io::Error| Refusal::Cache(format!("{}: {error}", path.display()));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(cache)?;
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(format!(".{}.new", unique()));
    let temporary = PathBuf::from(temporary);
    let written = fs::write(&temporary, bytes).and_then(|()| fs::rename(&temporary, path));
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written.map_err(cache)
}

fn sha256(bytes: &[u8]) -> String {
    horizon_release_sign::sha256_hex(bytes)
}

/// One build's verified index.
#[derive(Clone, Debug)]
pub(super) struct Sdk {
    pub(super) index: Index,
    /// The SHA-256 of `sdk.json` as signed: what a stamp names.
    sha256: String,
    /// `sdk/<build id>/`.
    dir: PathBuf,
}

impl Sdk {
    pub(super) fn commit(&self) -> &str {
        &self.index.build.commit
    }

    /// The build as people read it.
    pub(super) fn shown(&self) -> String {
        shown(self.commit(), self.index.build.nightly.as_deref())
    }
}

/// The verified index of build `commit`: the one kept when there is one, else fetched from
/// `base` and checked against the release's signed checksums with `keys`, then kept.
pub(super) fn index(base: &Base, commit: &str, keys: &[PublicKey]) -> Result<Sdk, Refusal> {
    index_in(&root(), base, commit, keys)
}

fn index_in(root: &Path, base: &Base, commit: &str, keys: &[PublicKey]) -> Result<Sdk, Refusal> {
    let build = shown(commit, None);
    if !super::is_revision(commit) {
        return Err(Refusal::Refused {
            build,
            why: trf("{commit} is not a build id", &[("commit", commit)]),
        });
    }
    let dir = root.join(commit);
    crate::native_cache::checked_tree(&crate::backend::build_dir(), &dir)
        .map_err(|error| Refusal::Cache(super::cache_error(error)))?;
    let kept = dir.join(INDEX);
    if let Ok(bytes) = fs::read(&kept) {
        // Written only once verified: read as the same checks would, without the signature.
        if let Ok(sdk) = checked(&bytes, commit, dir.clone()) {
            return Ok(sdk);
        }
    }
    let tag = tag(commit);
    let read = |name: &str| {
        base.small(&tag, name).map_err(|unread| match unread {
            Unread::Offline => Refusal::Offline {
                build: build.clone(),
                bytes: None,
            },
            Unread::Failed(why) => Refusal::Refused {
                build: build.clone(),
                why,
            },
        })
    };
    // The index first: a build without one has no SDK, whatever else its release holds.
    let bytes = read(INDEX)?.ok_or_else(|| Refusal::NoSdk {
        build: build.clone(),
    })?;
    let sums = read(SUMS)?;
    let signature = read(SIGNATURE)?.map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
    let unsigned = |why: String| Refusal::Unsigned {
        build: build.clone(),
        why,
    };
    // Signed for exactly `sdk-<build id>`: neither a game release's sums nor another build's SDK
    // pass for it.
    let signed = horizon_release_sign::sdk_digest(
        commit,
        INDEX,
        sums.as_deref(),
        signature.as_deref(),
        keys,
    )
    .map_err(unsigned)?;
    horizon_release_sign::check(INDEX, &signed, &sha256(&bytes)).map_err(unsigned)?;
    let sdk = checked(&bytes, commit, dir)?;
    write_whole(&kept, &bytes)?;
    Ok(sdk)
}

/// `bytes` read as build `commit`'s index, its toolchain this launcher's.
fn checked(bytes: &[u8], commit: &str, dir: PathBuf) -> Result<Sdk, Refusal> {
    let build = shown(commit, None);
    let index = Index::read(bytes, commit).map_err(|refusal| match refusal {
        sdk::Refusal::Schema(schema) => Refusal::UpdateLauncher {
            build: build.clone(),
            what: trf("SDK schema {schema}", &[("schema", &schema.to_string())]),
        },
        sdk::Refusal::PackageFormat(format) => Refusal::UpdateLauncher {
            build: build.clone(),
            what: trf(
                "package format {format}",
                &[("format", &format.to_string())],
            ),
        },
        other => Refusal::Refused {
            build: build.clone(),
            why: other.to_string(),
        },
    })?;
    let build = shown(commit, index.build.nightly.as_deref());
    let rust = &index.toolchain.rust;
    match compare_versions(rust, rust_toolchain::VERSION) {
        Some(std::cmp::Ordering::Equal) => {}
        Some(std::cmp::Ordering::Greater) | None => {
            return Err(Refusal::UpdateLauncher {
                build,
                what: trf("Rust {rust}", &[("rust", rust)]),
            });
        }
        Some(std::cmp::Ordering::Less) => {
            return Err(Refusal::OlderRust {
                build,
                rust: rust.clone(),
            });
        }
    }
    Ok(Sdk {
        index,
        sha256: sha256(bytes),
        dir,
    })
}

/// Two dotted version numbers (`1.95.0`) compared part by part; `None` when either is not one.
fn compare_versions(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    let parts = |text: &str| -> Option<Vec<u64>> {
        text.split('.').map(|part| part.parse().ok()).collect()
    };
    Some(parts(a)?.cmp(&parts(b)?))
}

/// The surface of `sdk`'s build: the kept copy, else fetched from `base` and checked against the
/// index, then kept. Small (under 1.5 MB) and kept, so a fit is told offline once a build was seen.
pub(super) fn surface(sdk: &Sdk, base: &Base) -> Result<mod_surface::Surface, Refusal> {
    const NAME: &str = "surface.json";
    let build = sdk.shown();
    let refused = |why: String| Refusal::Refused {
        build: build.clone(),
        why,
    };
    let kept = sdk.dir.join(NAME);
    let listed = sdk.index.asset(NAME).map_err(|r| refused(r.to_string()))?;
    let bytes = match fs::read(&kept) {
        // Kept only once verified; a size that changed since is fetched again.
        Ok(bytes) if bytes.len() as u64 == listed.size => bytes,
        _ => {
            let tag = tag(sdk.commit());
            let body = base
                .open(&tag, NAME)
                .map_err(|unread| match unread {
                    Unread::Offline => Refusal::Offline {
                        build: build.clone(),
                        bytes: Some(listed.size),
                    },
                    Unread::Failed(why) => refused(why),
                })?
                .ok_or_else(|| refused(sdk::Refusal::Missing(vec![NAME.into()]).to_string()))?;
            let mut bytes = Vec::new();
            body.take(listed.size.saturating_add(1))
                .read_to_end(&mut bytes)
                .map_err(|error| refused(error.to_string()))?;
            sdk.index
                .verify(NAME, bytes.as_slice())
                .map_err(|r| refused(r.to_string()))?;
            write_whole(&kept, &bytes)?;
            bytes
        }
    };
    mod_surface::Surface::read(&bytes).map_err(|error| refused(error.to_string()))
}

/// The folder of `sdk`'s build data (`points.json`, `internals.json`, `loops.json`,
/// `functions.json`): unpacked once, then used again on its stamp. `stop` as [`slices`] asks it
/// (a build's own check; planning outside a build never stops).
pub(super) fn build_data(
    sdk: &Sdk,
    base: &Base,
    stop: &dyn Fn() -> Result<(), String>,
) -> Result<PathBuf, Refusal> {
    const NAME: &str = "build-data.tar.xz";
    let into = sdk.dir.join("build-data");
    let mut fetched = Fetched::default();
    // Route checks fetch it without the build lease, maybe beside a build: their own download.
    fetch_into(sdk, base, NAME, &into, true, &mut fetched, &|_| {}, stop)?;
    Ok(into)
}

/// What a download reports as it goes: bytes done of the whole route's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Fetched {
    pub(super) done: u64,
    pub(super) total: u64,
}

/// The slices `route` needs of `sdk`'s build for this computer and `target`, each kept in its
/// own folder ([`Package`]): one whose stamp holds is used as it is; any other is fetched from
/// `base`, unpacked by [`sdk::unpack`] (which checks it against the index first) and stamped.
/// `lease` is the build lease: the slices and their stamps change only while it is held.
/// `progress` hears the bytes fetched of all that must be fetched, before and while. `stop` is
/// asked before each download, for each chunk and before each unpack: an error from it ends the
/// fetch as [`Refusal::Cancelled`], with no download kept.
pub(super) fn slices(
    _lease: &crate::native_cache::Cache,
    sdk: &Sdk,
    base: &Base,
    route: Route,
    target: Target,
    progress: &dyn Fn(Fetched),
    stop: &dyn Fn() -> Result<(), String>,
) -> Result<Package, Refusal> {
    let host = rust_toolchain::host().ok_or(Refusal::Host)?;
    let side = match target {
        Target::Client => Side::Client,
        Target::Server => Side::Server,
    };
    let prebuilt = super::root().join("prebuilt");
    let package = slices_in(
        &prebuilt,
        sdk,
        base,
        route,
        host.target,
        side,
        progress,
        stop,
    )?;
    crate::storage::mark_used(&prebuilt.join(unit(sdk.commit(), host.target, side)))
        .map_err(|error| Refusal::Cache(super::cache_error(error)))?;
    Ok(package)
}

/// The folder a build's slices for `triple` and `side` are kept in.
pub(super) fn unit(commit: &str, triple: &str, side: Side) -> String {
    match side {
        Side::Client => format!("{commit}-{triple}"),
        Side::Server => format!("{commit}-{triple}-server"),
    }
}

#[allow(clippy::too_many_arguments)]
fn slices_in(
    prebuilt: &Path,
    sdk: &Sdk,
    base: &Base,
    route: Route,
    triple: &str,
    side: Side,
    progress: &dyn Fn(Fetched),
    stop: &dyn Fn() -> Result<(), String>,
) -> Result<Package, Refusal> {
    let build = sdk.shown();
    let refused = |why: String| Refusal::Refused {
        build: build.clone(),
        why,
    };
    let names = sdk
        .index
        .route_assets(route.inputs(), triple, side)
        .map_err(|r| match r {
            sdk::Refusal::Missing(_) => Refusal::Host,
            other => refused(other.to_string()),
        })?;
    let dir = prebuilt.join(unit(sdk.commit(), triple, side));
    let package = Package::at(&dir);
    let folder = |kind: Kind| match kind {
        Kind::Runtime => Ok(package.runtime.clone()),
        Kind::Api => Ok(package.api.clone()),
        Kind::Engine => Ok(package.engine.clone()),
        other => Err(refused(trf(
            "the route lists {kind}, which is not a slice",
            &[("kind", other.name())],
        ))),
    };
    let mut wanted = Vec::new();
    for name in &names {
        let kind = Asset::parts_of(name)
            .map(|(kind, ..)| kind)
            .ok_or_else(|| refused(sdk::Refusal::AssetName(name.clone()).to_string()))?;
        wanted.push((name.as_str(), folder(kind)?));
    }
    let mut fetched = Fetched::default();
    for (name, into) in &wanted {
        if !stamp_holds(sdk, name, into) {
            fetched.total += sdk
                .index
                .asset(name)
                .map_err(|r| refused(r.to_string()))?
                .size;
        }
    }
    progress(fetched);
    for (name, into) in &wanted {
        fetch_into(sdk, base, name, into, false, &mut fetched, progress, stop)?;
    }
    Ok(package)
}

/// What a stamp says: the index that listed the archive, the archive, and every file it
/// unpacked to with its size.
#[derive(serde::Serialize, serde::Deserialize, PartialEq, Eq)]
struct Stamp {
    index: String,
    asset: String,
    files: BTreeMap<String, u64>,
}

fn stamp_path(into: &Path) -> PathBuf {
    let mut path = into.as_os_str().to_owned();
    path.push(STAMP_SUFFIX);
    PathBuf::from(path)
}

/// Every file under `dir`, `/`-separated relative to it, with its size; `None` when it can't
/// be read or holds anything but files and folders.
fn listing(dir: &Path) -> Option<BTreeMap<String, u64>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, u64>) -> Option<()> {
        for entry in fs::read_dir(dir).ok()? {
            let entry = entry.ok()?;
            let kind = entry.file_type().ok()?;
            let path = entry.path();
            if kind.is_dir() {
                walk(root, &path, out)?;
            } else if kind.is_file() {
                let relative = path.strip_prefix(root).ok()?;
                let name = relative
                    .components()
                    .map(|part| part.as_os_str().to_str())
                    .collect::<Option<Vec<_>>>()?
                    .join("/");
                out.insert(name, entry.metadata().ok()?.len());
            } else {
                return None;
            }
        }
        Some(())
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out)?;
    Some(out)
}

/// Whether `into` is `name` as `sdk`'s index lists it: its stamp names this index and this
/// archive, and the folder holds exactly the files the stamp lists, each at its size. No file is
/// hashed: the unpack checked the archive before the stamp was written.
fn stamp_holds(sdk: &Sdk, name: &str, into: &Path) -> bool {
    let Ok(text) = fs::read(stamp_path(into)) else {
        return false;
    };
    let Ok(stamp) = serde_json::from_slice::<Stamp>(&text) else {
        return false;
    };
    stamp.index == sdk.sha256 && stamp.asset == name && listing(into) == Some(stamp.files)
}

/// `name` unpacked at `into`: kept when its stamp holds; otherwise the archive is read from
/// `base` (downloaded beside `into` first when the base is not a folder), and, under
/// `<into>.fetch.lock`, what is there is removed, the archive unpacked and stamped. A route check
/// and a build may fetch the same folder at once (#19294): with `own_download` each downloads its
/// own copy; only one at a time changes the folder, and one stamped meanwhile is kept. `stop`
/// ends the wait for that turn (a build's Stop) with [`super::CANCELLED`].
#[allow(clippy::too_many_arguments)]
fn fetch_into(
    sdk: &Sdk,
    base: &Base,
    name: &str,
    into: &Path,
    own_download: bool,
    fetched: &mut Fetched,
    progress: &dyn Fn(Fetched),
    stop: &dyn Fn() -> Result<(), String>,
) -> Result<(), Refusal> {
    // No link or junction between the native root and the folder, before any I/O in it.
    let parent = into.parent().expect("a slice folder has a parent");
    crate::native_cache::checked_tree(&crate::backend::build_dir(), parent)
        .map_err(|error| Refusal::Cache(super::cache_error(error)))?;
    if stamp_holds(sdk, name, into) {
        return Ok(());
    }
    let stopped = || stop().is_err();
    if stopped() {
        return Err(Refusal::Cancelled);
    }
    let build = sdk.shown();
    let refused = |why: String| Refusal::Refused {
        build: build.clone(),
        why,
    };
    let size = sdk
        .index
        .asset(name)
        .map_err(|r| refused(r.to_string()))?
        .size;
    let stamp = stamp_path(into);
    let tag = tag(sdk.commit());
    let start = fetched.done;
    let (archive, downloaded) = match base.local(&tag, name) {
        Some(path) => (path, None),
        None => {
            let opened = base.open(&tag, name);
            // A request that failed while Stop was pending ends as the stop, not as its error.
            if !matches!(opened, Ok(Some(_))) && stopped() {
                return Err(Refusal::Cancelled);
            }
            let body = opened
                .map_err(|unread| match unread {
                    Unread::Offline => Refusal::Offline {
                        build: build.clone(),
                        bytes: Some(fetched.total.saturating_sub(fetched.done).max(size)),
                    },
                    Unread::Failed(why) => refused(why),
                })?
                .ok_or_else(|| refused(sdk::Refusal::Missing(vec![name.into()]).to_string()))?;
            let file = match own_download {
                true => parent.join(format!("{name}.{}.download", unique())),
                false => parent.join(format!("{name}.download")),
            };
            let listed = sdk.index.asset(name).map_err(|r| refused(r.to_string()))?;
            let total = fetched.total;
            crate::backend::save_checked_until(
                body,
                size,
                name,
                &listed.sha256,
                &file,
                &|fraction: f32| {
                    // `save_checked` reports nine tenths for the bytes, the rest for its check.
                    let part = ((fraction / 0.9).min(1.0) * size as f32) as u64;
                    progress(Fetched {
                        done: start + part,
                        total,
                    });
                },
                stop,
            )
            .map_err(|why| match stopped() {
                true => Refusal::Cancelled,
                false => refused(why),
            })?;
            (file.clone(), Some(file))
        }
    };
    // `sdk::unpack` can't be stopped part way: a stop that came during the download ends here.
    if stopped() {
        if let Some(file) = downloaded {
            let _ = fs::remove_file(file);
        }
        return Err(Refusal::Cancelled);
    }
    let made = in_place(sdk, name, &archive, into, &stamp, &stopped);
    if let Some(file) = downloaded {
        let _ = fs::remove_file(file);
    }
    made.map_err(|refusal| match refusal {
        Made::Cache(error) => Refusal::Cache(format!("{}: {error}", into.display())),
        Made::Unpack(sdk::Refusal::Missing(_) | sdk::Refusal::Io(_)) if !archive.is_file() => {
            refused(sdk::Refusal::Missing(vec![name.into()]).to_string())
        }
        Made::Unpack(
            refusal @ (sdk::Refusal::Busy(_)
            | sdk::Refusal::NoLocks(_)
            | sdk::Refusal::Rename { .. }),
        ) => Refusal::Cache(refusal.to_string()),
        Made::Unpack(other) => refused(other.to_string()),
        Made::Refused(refusal) => refusal,
    })?;
    fetched.done = start + size;
    progress(*fetched);
    Ok(())
}

/// Why [`in_place`] did not make its folder.
enum Made {
    Cache(std::io::Error),
    Unpack(sdk::Refusal),
    Refused(Refusal),
}

/// The local half of [`fetch_into`], under `<into>.fetch.lock`: `archive` unpacked at `into` and
/// stamped, unless another fetch stamped it while this one downloaded.
fn in_place(
    sdk: &Sdk,
    name: &str,
    archive: &Path,
    into: &Path,
    stamp: &Path,
    stop: &dyn Fn() -> bool,
) -> Result<(), Made> {
    let mut path = into.as_os_str().to_owned();
    path.push(".fetch.lock");
    let cache = |error| Made::Refused(Refusal::Cache(super::cache_error(error)));
    let Some(_lock) = crate::native_cache::waited(&PathBuf::from(path), stop).map_err(cache)?
    else {
        return Err(Made::Refused(Refusal::Cancelled));
    };
    if stamp_holds(sdk, name, into) {
        return Ok(());
    }
    match fs::remove_file(stamp) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(Made::Cache(error)),
    }
    match fs::symlink_metadata(into) {
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(into).map_err(Made::Cache)?,
        Ok(_) => fs::remove_file(into).map_err(Made::Cache)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(Made::Cache(error)),
    }
    sdk::unpack(&sdk.index, name, archive, into, UNPACKED_LIMIT).map_err(Made::Unpack)?;
    let files = listing(into).ok_or_else(|| {
        Made::Refused(Refusal::Cache(trf(
            "{folder} holds something other than files and folders",
            &[("folder", &into.display().to_string())],
        )))
    })?;
    let stamp_text = serde_json::to_vec(&Stamp {
        index: sdk.sha256.clone(),
        asset: name.to_owned(),
        files,
    })
    .expect("a stamp serializes");
    write_whole(stamp, &stamp_text).map_err(Made::Refused)
}

/// The build id the game release `version` was made from, as its copy of its SDK's index names
/// it (signed with the release): kept once read, so it is known offline after that. A Nightly
/// (`nightly-<date>-<commit>`) is read from its dated release, signed for its own version, and
/// its index must name a commit its version's starts; a release (`v…`) from its own.
pub(super) fn build_of(version: &str, base: &Base, keys: &[PublicKey]) -> Result<String, Refusal> {
    build_of_in(&root().join("versions"), version, base, keys)
}

fn build_of_in(
    kept: &Path,
    version: &str,
    base: &Base,
    keys: &[PublicKey],
) -> Result<String, Refusal> {
    let refused = |why: String| Refusal::Refused {
        build: version.to_owned(),
        why,
    };
    let plain = !version.is_empty()
        && version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'));
    if !plain {
        return Err(refused(trf(
            "{version} is not a game version",
            &[("version", version)],
        )));
    }
    let file = kept.join(version);
    if let Ok(text) = fs::read_to_string(&file) {
        if super::is_revision(text.trim()) {
            return Ok(text.trim().to_owned());
        }
    }
    // A build of a channel (a Nightly, or a snapshot installed by its tag) is read from its
    // channel only: the version's own prefix, checked whole, names it, never a release's title.
    let channel = horizon_release_sign::Channel::of(version)
        .filter(|channel| horizon_release_sign::is_build(*channel, version));
    // The Nightly being served is published as `nightly` and renamed `nightly-<date>` when the
    // next one goes out: the current tag is tried first. Its v2 signature binds the exact
    // version, so whichever tag serves it, only that version's index is accepted. A snapshot is
    // served under its version alone.
    let tags: Vec<String> = match (channel, horizon_release_sign::nightly_date(version)) {
        (Some(horizon_release_sign::Channel::Nightly), Some(date)) => vec![
            horizon_release_sign::NIGHTLY.to_owned(),
            format!("nightly-{date}"),
        ],
        _ => vec![version.to_owned()],
    };
    let mut bytes = None;
    let mut last = Refusal::NoSdk {
        build: version.to_owned(),
    };
    for tag in &tags {
        match signed_game_index(base, tag, version, channel, keys) {
            Ok(found) => {
                bytes = Some(found);
                break;
            }
            // Offline is offline whichever tag: no use trying the next.
            Err(refusal @ Refusal::Offline { .. }) => return Err(refusal),
            Err(refusal) => last = refusal,
        }
    }
    let Some(bytes) = bytes else {
        return Err(last);
    };
    let build = Index::build_of(&bytes).map_err(|refusal| match refusal {
        sdk::Refusal::Schema(schema) => Refusal::UpdateLauncher {
            build: version.to_owned(),
            what: trf("SDK schema {schema}", &[("schema", &schema.to_string())]),
        },
        other => refused(other.to_string()),
    })?;
    let commit = build.commit;
    let named = version.rsplit('-').next().unwrap_or_default();
    if !super::is_revision(&commit) || (channel.is_some() && !commit.starts_with(named)) {
        return Err(refused(trf(
            "its index names build {commit}, not {version}'s",
            &[("commit", &commit), ("version", version)],
        )));
    }
    write_whole(&file, commit.as_bytes())?;
    Ok(commit)
}

/// The game release `tag`'s copy of its SDK index, checked against the release's signed sums:
/// for a build of `channel` (a Nightly or a snapshot), signed for exactly `version` on that
/// channel (`SHA256SUMS.v2.sig`); for a release, for `tag`.
fn signed_game_index(
    base: &Base,
    tag: &str,
    version: &str,
    channel: Option<horizon_release_sign::Channel>,
    keys: &[PublicKey],
) -> Result<Vec<u8>, Refusal> {
    let read = |name: &str| {
        base.small(tag, name).map_err(|unread| match unread {
            Unread::Offline => Refusal::Offline {
                build: version.to_owned(),
                bytes: None,
            },
            Unread::Failed(why) => Refusal::Refused {
                build: version.to_owned(),
                why,
            },
        })
    };
    let bytes = read(INDEX)?.ok_or_else(|| Refusal::NoSdk {
        build: version.to_owned(),
    })?;
    let sums = read(SUMS)?;
    let unsigned = |why: String| Refusal::Unsigned {
        build: version.to_owned(),
        why,
    };
    let signed = if let Some(channel) = channel {
        let signature = read(SIGNATURE_V2)?.map(|b| String::from_utf8_lossy(&b).into_owned());
        horizon_release_sign::build_digest(
            channel,
            version,
            INDEX,
            sums.as_deref(),
            signature.as_deref(),
            keys,
        )
    } else {
        let signature = read(SIGNATURE)?.map(|b| String::from_utf8_lossy(&b).into_owned());
        horizon_release_sign::signed_digest(tag, INDEX, sums.as_deref(), signature.as_deref(), keys)
    }
    .map_err(unsigned)?;
    horizon_release_sign::check(INDEX, &signed, &sha256(&bytes)).map_err(unsigned)?;
    Ok(bytes)
}

#[cfg(test)]
#[path = "sdk_cache_tests.rs"]
mod tests;
