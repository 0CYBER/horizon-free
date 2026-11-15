//! The mod SDK's identity (design: `sdk.json`): one index per game build naming every asset of its
//! release with its checksum, size, kind and codec, the package layout number, and the files each
//! route needs before any download, build or packaging starts. Every reader of these bytes ends a
//! bad input in a named [`Refusal`], never a panic.
//!
//! Schema numbers are integers: a new field does not bump [`SCHEMA`], a removed or redefined one
//! does. Readers ignore fields they do not know and refuse a schema they do not know.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fmt,
    io::Read,
    path::{Path, PathBuf},
};

/// The index's file name, in the release and in an unpacked SDK folder.
pub const INDEX: &str = "sdk.json";
/// The index schema this reader knows.
pub const SCHEMA: u32 = 1;
/// The prebuilt package's layout number. Its one place is the file `sdk/package-format`, which
/// the archive name, the package's identity file, `sdk.json` and the scripts all read.
pub const PACKAGE_FORMAT: u32 = number(include_str!("sdk/package-format"));

/// A decimal number, then any trailing whitespace (a CR included), at compile time: anything else
/// fails the build. The scripts reading the same file trim the same way.
const fn number(text: &str) -> u32 {
    let bytes = text.as_bytes();
    let mut end = bytes.len();
    while end > 0 && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    assert!(end > 0, "sdk/package-format holds one number");
    let mut value = 0u32;
    let mut at = 0;
    while at < end {
        assert!(
            bytes[at].is_ascii_digit(),
            "sdk/package-format holds one number"
        );
        value = value * 10 + (bytes[at] - b'0') as u32;
        at += 1;
    }
    value
}
/// The game version every build of this source implements.
pub const GAME: &str = horizon_core::version::VERSION_ID;
/// Files a release folder holds besides its assets: the index and its signed sums.
pub const BESIDE_ASSETS: [&str; 3] = [INDEX, "SHA256SUMS", "SHA256SUMS.sig"];

/// `sdk.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Index {
    pub schema: u32,
    pub build: Build,
    pub package_format: u32,
    pub toolchain: Toolchain,
    /// The content manifest's version ([`crate::manifest::API`]).
    pub content_api: String,
    pub surface: Surface,
    pub mixins: Mixins,
    /// Every asset of the release but the index itself, by file name.
    pub assets: BTreeMap<String, Asset>,
    /// The files each route needs, named before any work starts ([`inputs`]).
    pub inputs: Inputs,
}

/// Which game build an index describes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Build {
    /// The game's full commit sha: the build id.
    pub commit: String,
    /// The vanilla version ([`GAME`]).
    pub game: String,
    /// The commit's date, `YYYY-MM-DD` (UTC).
    pub date: String,
    /// The Nightly name that ships this build, when one does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nightly: Option<String>,
}

/// The Rust release everything in the SDK was compiled with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Toolchain {
    pub rust: String,
}

/// The stable surface (`surface.json`) this build offers, and how it differs from the SDK
/// published before it ([`Index::compare`]). The first SDK has no `previous`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Surface {
    /// The surface document's own `hash`: equal hashes, equal surface.
    pub hash: String,
    /// The build id of the SDK this one is compared with: the newest published before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<String>,
    /// The comparison in one word: its worst change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Verdict>,
    /// The ids of the records that changed, in the diff's order (breaking first).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<String>,
    /// The change notes attached to them (`crates/horizon-mod-vocab/changes/*.toml`, read by the
    /// surface diff), once each, in the notes files' own shape.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<ChangeNote>,
}

/// One change note, as its file says it: the ids it explains, why they changed, and how a mod
/// moves on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeNote {
    /// The changed records it names (those of them this comparison found changed).
    pub ids: Vec<String>,
    pub why: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migrate: Option<String>,
}

/// A surface comparison's verdict (`horizon-stage surface-diff`'s `verdict`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Identical,
    Additive,
    Conditional,
    Breaking,
}

/// The pinned horizon-mixins tool, which keeps its own release.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mixins {
    pub rev: String,
    /// The SHA-256 of the carried patch ([`crate::mixins_tool::patch_hash`]).
    pub patch: String,
}

/// One release asset.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Asset {
    pub sha256: String,
    pub size: u64,
    pub kind: Kind,
    pub codec: Codec,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub side: Option<Side>,
}

/// What an asset holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// `surface.json`: what an API-only mod can name.
    Surface,
    /// `build-data.tar.xz`: `points.json`, `internals.json`, `loops.json`, `functions.json`.
    BuildData,
    /// The API crates with their generated code written out.
    ApiSource,
    /// The game's libraries, the API route's prebuilt executable and its stub mods library, and
    /// the package identity.
    Runtime,
    /// Metadata of the API closure and its proc macros, the point data and the mods library's
    /// template: what an API-only mod builds against.
    Api,
    /// Every other file of the prebuilt game: what the engine route (a `[native]` table,
    /// internals or a registry dependency) builds against, with the other two.
    Engine,
}

impl Kind {
    const ALL: [Kind; 6] = [
        Kind::Surface,
        Kind::BuildData,
        Kind::ApiSource,
        Kind::Runtime,
        Kind::Api,
        Kind::Engine,
    ];

    /// The name `sdk.json` writes.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Surface => "surface",
            Kind::BuildData => "build-data",
            Kind::ApiSource => "api-source",
            Kind::Runtime => "runtime",
            Kind::Api => "api",
            Kind::Engine => "engine",
        }
    }

    /// Whether its assets exist once per target (and side).
    pub fn per_target(self) -> bool {
        matches!(self, Kind::Runtime | Kind::Api | Kind::Engine)
    }
}

/// How an asset's bytes are packed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    /// The file as it is.
    None,
    /// A tar archive, xz-compressed.
    Xz,
}

/// Which game a per-target asset is built for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Client,
    Server,
}

impl Asset {
    /// The kind, codec, target and side an asset's file name states:
    /// `surface.json`, `build-data.tar.xz`, `api-source.tar.xz` and
    /// `<runtime|api|engine>-<target>[-server].tar.xz`, where a target is made of `a-z`, `0-9`,
    /// `_` and `-` only. `None` for any other name, so a name this accepts is a plain file name.
    pub fn parts_of(name: &str) -> Option<(Kind, Codec, Option<String>, Option<Side>)> {
        if name == "surface.json" {
            return Some((Kind::Surface, Codec::None, None, None));
        }
        let stem = name.strip_suffix(".tar.xz")?;
        let kind = Kind::ALL.into_iter().find(|kind| {
            if kind.per_target() {
                stem.starts_with(&format!("{}-", kind.name()))
            } else {
                stem == kind.name()
            }
        })?;
        if !kind.per_target() {
            return Some((kind, Codec::Xz, None, None));
        }
        let rest = &stem[kind.name().len() + 1..];
        let (target, side) = match rest.strip_suffix("-server") {
            Some(target) => (target, Side::Server),
            None => (rest, Side::Client),
        };
        let plain = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-';
        (!target.is_empty() && target.chars().all(plain))
            .then(|| (kind, Codec::Xz, Some(target.to_owned()), Some(side)))
    }

    /// A per-target asset's file name, as [`Asset::parts_of`] reads it.
    pub fn name(kind: Kind, target: &str, side: Side) -> String {
        let suffix = if side == Side::Server { "-server" } else { "" };
        format!("{}-{target}{suffix}.tar.xz", kind.name())
    }
}

/// The files each route needs: the one list the packager, `developer-install`, the launcher and
/// the mod tool read (`sdk/inputs.json`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inputs {
    pub routes: BTreeMap<String, Route>,
    /// How to make each listed file, by its name.
    pub how: BTreeMap<String, String>,
    /// The files each compiled slice must hold, by its kind's name, as paths in the slice:
    /// `<folder>/@program:<name>` is that program (`.exe` on Windows), `<folder>/@dylib:<crate>`
    /// that crate's dynamic library and `<folder>/@rmeta:<crate>` its metadata, built for the
    /// slice's target. The packager refuses a package whose slices miss one.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub members: BTreeMap<String, Vec<String>>,
}

/// What one route needs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route {
    /// Files beside the game program the route starts from. A name with no extension is a
    /// program and takes the game program's executable suffix.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub program: Vec<String>,
    /// Files in the game checkout, relative to its root.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checkout: Vec<String>,
    /// SDK asset kinds, one asset each for the host's target and side.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sdk: Vec<Kind>,
}

/// The required inputs of every route.
pub fn inputs() -> Inputs {
    serde_json::from_str(include_str!("sdk/inputs.json")).expect("sdk/inputs.json is an input list")
}

impl Inputs {
    /// The route named `name`. The list may come from a downloaded `sdk.json`, so an unknown
    /// name is a refusal.
    pub fn route(&self, name: &str) -> Result<&Route, Refusal> {
        self.routes
            .get(name)
            .ok_or_else(|| Refusal::Route(name.to_owned()))
    }

    /// How to make `name` (as listed, without `.exe`) for the game program `program`: `PROFILE`
    /// becomes the cargo flag of the profile `program` was built with (its folder's name) and
    /// `PROGRAM_FOLDER` its folder, so the command makes the file where the check looks.
    /// A program outside `target/<profile>/` (or `target/<triple>/<profile>/`) gets the command
    /// with no flag and the words "with the same profile as the game program".
    pub fn how(&self, name: &str, program: &Path) -> Option<String> {
        let folder = program.parent().unwrap_or(Path::new(""));
        fn named(path: Option<&Path>) -> Option<&str> {
            path.and_then(Path::file_name).and_then(|n| n.to_str())
        }
        let in_target = named(folder.parent()) == Some("target")
            || named(folder.parent().and_then(Path::parent)) == Some("target");
        let how = self.how.get(name)?;
        let how = match named(Some(folder)).filter(|_| in_target) {
            _ if !how.contains("PROFILE") => how.clone(),
            Some("debug") => how.replace("PROFILE ", ""),
            Some("release") => how.replace("PROFILE", "--release"),
            Some(other) => how.replace("PROFILE", &format!("--profile {other}")),
            None => how.replace("PROFILE ", "") + ", with the same profile as the game program",
        };
        Some(how.replace("PROGRAM_FOLDER", &folder.display().to_string()))
    }

    /// Every file of `route` that is missing, beside `program` or in `checkout`, together with
    /// how to make each, in one message; `Ok` when nothing is missing.
    pub fn check(&self, route: &str, program: &Path, checkout: &Path) -> Result<(), Refusal> {
        let route = self.route(route)?;
        let missing: Vec<String> = route
            .program_files(program)
            .into_iter()
            .map(|path| {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                (name.into_owned(), path)
            })
            .chain(
                route
                    .checkout
                    .iter()
                    .map(|name| (name.clone(), checkout.join(name))),
            )
            .filter(|(_, path)| !path.is_file())
            .map(|(name, _)| name)
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        let how = missing
            .iter()
            .filter_map(|name| {
                let listed = name.strip_suffix(".exe").unwrap_or(name);
                self.how(listed, program)
                    .map(|how| format!("{name}: {how}"))
            })
            .collect();
        Err(Refusal::MissingInputs { missing, how })
    }
}

impl Route {
    /// The route's files beside `program`, with its executable suffix on programs.
    pub fn program_files(&self, program: &Path) -> Vec<PathBuf> {
        let exe = program.extension().is_some_and(|e| e == "exe");
        self.program
            .iter()
            .map(|name| {
                let name = if exe && !name.contains('.') {
                    format!("{name}.exe")
                } else {
                    name.clone()
                };
                program.with_file_name(name)
            })
            .collect()
    }
}

/// Why an SDK index, asset or input set was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Refusal {
    /// The index is not JSON of the expected form.
    Malformed(String),
    /// The index's schema is one this reader does not know.
    Schema(u64),
    /// The index describes another build than the one asked for.
    Build { expected: String, found: String },
    /// The package layout is one this reader does not know.
    PackageFormat(u64),
    /// An asset names a kind this reader does not know.
    Kind { asset: String, kind: String },
    /// An asset names a codec this reader does not know.
    Codec { asset: String, codec: String },
    /// An asset's name is not a plain file name, or does not say what the entry claims.
    AssetName(String),
    /// Listed assets that are not there.
    Missing(Vec<String>),
    /// A file that the index does not list.
    Unlisted(String),
    /// An asset's size differs from the index's.
    Size {
        asset: String,
        expected: u64,
        found: u64,
    },
    /// An asset's SHA-256 differs from the index's.
    Checksum { asset: String },
    /// An asset unpacks to more than its limit.
    TooLarge { asset: String, limit: u64 },
    /// An asset's archive is damaged or holds something other than files and folders.
    Archive { asset: String, why: String },
    /// Reading or writing failed.
    Io(String),
    /// The input list has no such route.
    Route(String),
    /// Another unpack to the same folder holds its lock.
    Busy(String),
    /// The file system holding this folder does not support file locks (some network shares).
    NoLocks(String),
    /// The folder an asset would unpack into already exists.
    Exists(String),
    /// The unpacked folder could not be moved into place.
    Rename { to: String, why: String },
    /// Files a route needs are not there; how to make each.
    MissingInputs {
        missing: Vec<String>,
        how: Vec<String>,
    },
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(why) => write!(f, "{INDEX} is not a valid SDK index: {why}"),
            Self::Schema(schema) => write!(
                f,
                "{INDEX} has schema {schema}; this program reads schema {SCHEMA}. Update it to use this SDK"
            ),
            Self::Build { expected, found } => {
                write!(f, "{INDEX} is for build {found}, not {expected}")
            }
            Self::PackageFormat(format) => write!(
                f,
                "{INDEX} has package format {format}; this program reads format {PACKAGE_FORMAT}. Update it to use this SDK"
            ),
            Self::Kind { asset, kind } => write!(f, "{asset}: unknown asset kind {kind:?}"),
            Self::Codec { asset, codec } => write!(f, "{asset}: unknown codec {codec:?}"),
            Self::AssetName(asset) => write!(f, "{asset:?} is not a valid SDK asset name"),
            Self::Missing(assets) => write!(f, "missing SDK assets: {}", assets.join(", ")),
            Self::Unlisted(asset) => write!(f, "{asset} is not listed in {INDEX}"),
            Self::Size {
                asset,
                expected,
                found,
            } => write!(
                f,
                "{asset} is {found} bytes; {INDEX} lists {expected}. Download it again"
            ),
            Self::Checksum { asset } => write!(
                f,
                "{asset} does not match its checksum in {INDEX}. Download it again"
            ),
            Self::TooLarge { asset, limit } => {
                write!(f, "{asset} unpacks to more than {limit} bytes")
            }
            Self::Archive { asset, why } => write!(f, "{asset}: damaged archive: {why}"),
            Self::Io(why) => f.write_str(why),
            Self::Route(route) => write!(f, "the SDK's input list has no route {route}"),
            Self::Exists(path) => write!(f, "{path} already exists"),
            Self::Busy(path) => write!(f, "another program is unpacking into {path}; try again"),
            Self::NoLocks(path) => write!(
                f,
                "{path}: this folder's file system does not support file locks: move the \
                 launcher's cache to a local disk"
            ),
            Self::Rename { to, why } => write!(f, "could not move the unpacked SDK to {to}: {why}"),
            Self::MissingInputs { missing, how } => {
                write!(f, "missing required inputs: {}", missing.join(", "))?;
                for line in how {
                    write!(f, "\n  {line}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for Refusal {}

/// The index as written, before its kinds and codecs are checked.
#[derive(Deserialize)]
struct Written {
    build: Build,
    package_format: u64,
    toolchain: Toolchain,
    content_api: String,
    surface: Surface,
    mixins: Mixins,
    assets: BTreeMap<String, WrittenAsset>,
    inputs: Inputs,
}

#[derive(Deserialize)]
struct WrittenAsset {
    sha256: String,
    size: u64,
    kind: String,
    codec: String,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    side: Option<Side>,
}

impl Index {
    /// The index of the release folder `folder` for `build`, compiled with Rust `rust`: every file
    /// there but [`BESIDE_ASSETS`] becomes an asset, named as [`Asset::parts_of`] reads it, and the
    /// surface hash comes from its `surface.json`. The publish step writes it ([`Index::to_json`]).
    pub fn of_folder(folder: &Path, build: Build, rust: String) -> Result<Index, Refusal> {
        let io = |error: std::io::Error| Refusal::Io(format!("{}: {error}", folder.display()));
        let mut assets = BTreeMap::new();
        for entry in std::fs::read_dir(folder).map_err(io)? {
            let name = entry
                .map_err(io)?
                .file_name()
                .to_string_lossy()
                .into_owned();
            if BESIDE_ASSETS.contains(&name.as_str()) {
                continue;
            }
            let (kind, codec, target, side) =
                Asset::parts_of(&name).ok_or_else(|| Refusal::AssetName(name.clone()))?;
            let mut file = std::fs::File::open(folder.join(&name)).map_err(io)?;
            let mut hash = Sha256::new();
            let size = std::io::copy(&mut file, &mut hash).map_err(io)?;
            let asset = Asset {
                sha256: format!("{:x}", hash.finalize()),
                size,
                kind,
                codec,
                target,
                side,
            };
            assets.insert(name, asset);
        }
        #[derive(Deserialize)]
        struct Document {
            hash: String,
        }
        let surface = std::fs::read(folder.join("surface.json"))
            .map_err(|_| Refusal::Missing(vec!["surface.json".into()]))?;
        let Document { hash } = serde_json::from_slice(&surface)
            .map_err(|error| Refusal::Malformed(format!("surface.json: {error}")))?;
        Ok(Index {
            schema: SCHEMA,
            build,
            package_format: PACKAGE_FORMAT,
            toolchain: Toolchain { rust },
            content_api: crate::manifest::API.to_owned(),
            surface: Surface {
                hash,
                previous: None,
                source: None,
                changes: Vec::new(),
                notes: Vec::new(),
            },
            mixins: Mixins {
                rev: crate::mixins_tool::REV.to_owned(),
                patch: crate::mixins_tool::patch_hash(),
            },
            assets,
            inputs: inputs(),
        })
    }

    /// Fills the surface's comparison with `previous` (a build id) from `diff`, the JSON that
    /// `horizon-stage surface-diff <previous surface.json> <this surface.json> --json` wrote. A
    /// diff whose new side is not this surface is refused: it compares another build.
    pub fn compare(&mut self, previous: &str, diff: &[u8]) -> Result<(), Refusal> {
        #[derive(Deserialize)]
        struct Diff {
            new: String,
            verdict: Verdict,
            changes: Vec<Change>,
        }
        #[derive(Deserialize)]
        struct Change {
            id: String,
            #[serde(default)]
            notes: Vec<Note>,
        }
        #[derive(Deserialize)]
        struct Note {
            file: String,
            why: String,
            #[serde(default)]
            migrate: Option<String>,
        }
        let diff: Diff = serde_json::from_slice(diff)
            .map_err(|error| Refusal::Malformed(format!("the surface diff: {error}")))?;
        if diff.new != self.surface.hash {
            return Err(Refusal::Malformed(format!(
                "the surface diff is of surface {}, not this build's {}",
                diff.new, self.surface.hash
            )));
        }
        // One entry per note file, with every change it is attached to.
        let mut files: Vec<&str> = Vec::new();
        let mut notes: Vec<ChangeNote> = Vec::new();
        for change in &diff.changes {
            for note in &change.notes {
                match files.iter().position(|file| *file == note.file) {
                    Some(at) => notes[at].ids.push(change.id.clone()),
                    None => {
                        files.push(&note.file);
                        notes.push(ChangeNote {
                            ids: vec![change.id.clone()],
                            why: note.why.clone(),
                            migrate: note.migrate.clone(),
                        });
                    }
                }
            }
        }
        self.surface.previous = Some(previous.to_owned());
        self.surface.source = Some(diff.verdict);
        self.surface.changes = diff.changes.into_iter().map(|change| change.id).collect();
        self.surface.notes = notes;
        Ok(())
    }

    /// `sdk.json`'s text: pretty, sorted, ending in a newline.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("an index serializes") + "\n"
    }

    /// The build an index names, read before that build's id is known: a game release carries
    /// a copy of its SDK's index, and that copy is where its build id comes from. Only the schema
    /// is checked here; [`Index::read`] checks the rest once the build is fetched by its id.
    pub fn build_of(bytes: &[u8]) -> Result<Build, Refusal> {
        #[derive(Deserialize)]
        struct Named {
            build: Build,
        }
        Self::schema(bytes)?;
        serde_json::from_slice::<Named>(bytes)
            .map(|named| named.build)
            .map_err(|error| Refusal::Malformed(error.to_string()))
    }

    /// Refuses a document that is not JSON with a `schema`, or whose schema is not [`SCHEMA`].
    fn schema(bytes: &[u8]) -> Result<(), Refusal> {
        #[derive(Deserialize)]
        struct Schema {
            schema: u64,
        }
        let Schema { schema } =
            serde_json::from_slice(bytes).map_err(|error| Refusal::Malformed(error.to_string()))?;
        if schema != u64::from(SCHEMA) {
            return Err(Refusal::Schema(schema));
        }
        Ok(())
    }

    /// Read `sdk.json` for build `commit`: its schema, build, package format and every asset's
    /// kind, codec and name are checked here, once.
    pub fn read(bytes: &[u8], commit: &str) -> Result<Index, Refusal> {
        let malformed = |error: serde_json::Error| Refusal::Malformed(error.to_string());
        Self::schema(bytes)?;
        let written: Written = serde_json::from_slice(bytes).map_err(malformed)?;
        if written.build.commit != commit {
            return Err(Refusal::Build {
                expected: commit.to_owned(),
                found: written.build.commit,
            });
        }
        if written.package_format != u64::from(PACKAGE_FORMAT) {
            return Err(Refusal::PackageFormat(written.package_format));
        }
        let mut assets = BTreeMap::new();
        for (name, asset) in written.assets {
            let kind = Kind::ALL
                .into_iter()
                .find(|kind| kind.name() == asset.kind)
                .ok_or_else(|| Refusal::Kind {
                    asset: name.clone(),
                    kind: asset.kind.clone(),
                })?;
            let codec = match asset.codec.as_str() {
                "none" => Codec::None,
                "xz" => Codec::Xz,
                _ => {
                    return Err(Refusal::Codec {
                        asset: name,
                        codec: asset.codec,
                    });
                }
            };
            let entry = Asset {
                sha256: asset.sha256,
                size: asset.size,
                kind,
                codec,
                target: asset.target,
                side: asset.side,
            };
            // The name is a plain file name that says what the entry claims, so it is safe to
            // join to a folder and to build from a kind, target and side.
            if Asset::parts_of(&name) != Some((kind, codec, entry.target.clone(), entry.side)) {
                return Err(Refusal::AssetName(name));
            }
            assets.insert(name, entry);
        }
        Ok(Index {
            schema: SCHEMA,
            build: written.build,
            package_format: PACKAGE_FORMAT,
            toolchain: written.toolchain,
            content_api: written.content_api,
            surface: written.surface,
            mixins: written.mixins,
            assets,
            inputs: written.inputs,
        })
    }

    /// The listed asset `name`, or why there is none.
    pub fn asset(&self, name: &str) -> Result<&Asset, Refusal> {
        self.assets
            .get(name)
            .ok_or_else(|| Refusal::Unlisted(name.to_owned()))
    }

    /// The asset names `route` needs on `target` and `side`, refusing with every missing one.
    pub fn route_assets(
        &self,
        route: &str,
        target: &str,
        side: Side,
    ) -> Result<Vec<String>, Refusal> {
        let route = self.inputs.route(route)?;
        let names: Vec<String> = route
            .sdk
            .iter()
            .map(|&kind| Asset::name(kind, target, side))
            .collect();
        let missing: Vec<String> = names
            .iter()
            .filter(|name| !self.assets.contains_key(*name))
            .cloned()
            .collect();
        if missing.is_empty() {
            Ok(names)
        } else {
            Err(Refusal::Missing(missing))
        }
    }

    /// Check `body` against the listed asset `name`: its size, then its SHA-256. Reads at most
    /// one byte past the listed size.
    pub fn verify(&self, name: &str, body: impl Read) -> Result<(), Refusal> {
        let asset = self.asset(name)?;
        let mut body = body.take(asset.size.saturating_add(1));
        let mut hash = Sha256::new();
        let mut size = 0u64;
        let mut buffer = [0u8; 1 << 16];
        loop {
            let read = body
                .read(&mut buffer)
                .map_err(|error| Refusal::Io(format!("{name}: {error}")))?;
            if read == 0 {
                break;
            }
            size += read as u64;
            hash.update(&buffer[..read]);
        }
        if size != asset.size {
            return Err(Refusal::Size {
                asset: name.to_owned(),
                expected: asset.size,
                found: size,
            });
        }
        if format!("{:x}", hash.finalize()) != asset.sha256 {
            return Err(Refusal::Checksum {
                asset: name.to_owned(),
            });
        }
        Ok(())
    }

    /// Check a release folder: every listed asset is there and matches, and nothing else is
    /// there but the index and its sums ([`BESIDE_ASSETS`]).
    pub fn verify_folder(&self, folder: &Path) -> Result<(), Refusal> {
        let io = |error: std::io::Error| Refusal::Io(format!("{}: {error}", folder.display()));
        let mut present = Vec::new();
        for entry in std::fs::read_dir(folder).map_err(io)? {
            let name = entry.map_err(io)?.file_name();
            let name = name.to_string_lossy().into_owned();
            if !BESIDE_ASSETS.contains(&name.as_str()) {
                self.asset(&name)?;
                present.push(name);
            }
        }
        let missing: Vec<String> = self
            .assets
            .keys()
            .filter(|name| !present.contains(name))
            .cloned()
            .collect();
        if !missing.is_empty() {
            return Err(Refusal::Missing(missing));
        }
        for name in self.assets.keys() {
            let file = std::fs::File::open(folder.join(name)).map_err(io)?;
            self.verify(name, file)?;
        }
        Ok(())
    }
}

/// The largest xz dictionary [`unpack`] decodes with: `xz -9`'s. The decoder's memory is this plus
/// fixed buffers, whatever the archive holds; an archive asking for more is refused as damaged.
#[cfg(feature = "prebuilt")]
pub const DICTIONARY_LIMIT: usize = 64 << 20;

/// The largest tar extension entry (a GNU long name or link, a PAX header) [`unpack`] reads.
#[cfg(feature = "prebuilt")]
pub const EXTENSION_LIMIT: u64 = 64 << 10;

/// What [`unpack`] appends to the destination's name for the file it holds an exclusive lock on
/// while it unpacks.
#[cfg(feature = "prebuilt")]
pub const LOCK_SUFFIX: &str = ".lock";

/// What [`unpack`] appends to the destination's name for the folder it unpacks into first: a
/// leftover one is a crash's, removed before the next unpack (and by cache eviction).
#[cfg(feature = "prebuilt")]
pub const PARTIAL_SUFFIX: &str = ".partial";

/// Unpack the listed archive `name`, downloaded to the file `file`, into the folder `into`, which
/// must not exist yet: whether to reuse a folder is the caller's decision. From one open handle
/// the file is checked against the index ([`Index::verify`]), then decoded, so nothing unverified
/// is decoded. The archive streams through a bounded dictionary into `<into>.partial`
/// ([`PARTIAL_SUFFIX`]), renamed into place only when every entry unpacked: a refusal partway
/// leaves nothing. Refused: unpacked bytes past `limit` (a decompression bomb), extension entries
/// past [`EXTENSION_LIMIT`], and any entry that is not a plain file or folder inside `into`.
#[cfg(feature = "prebuilt")]
pub fn unpack(
    index: &Index,
    name: &str,
    file: &Path,
    into: &Path,
    limit: u64,
) -> Result<(), Refusal> {
    use std::io::{Seek, SeekFrom};
    let io = |path: &Path| {
        let path = path.display().to_string();
        move |error: std::io::Error| Refusal::Io(format!("{path}: {error}"))
    };
    let _lock = lock(into, std::fs::File::try_lock)?;
    if into.symlink_metadata().is_ok() {
        return Err(Refusal::Exists(into.display().to_string()));
    }
    let mut body = std::fs::File::open(file).map_err(io(file))?;
    index.verify(name, &mut body)?;
    if index.asset(name)?.codec != Codec::Xz {
        return Err(Refusal::Archive {
            asset: name.to_owned(),
            why: "not an archive".into(),
        });
    }
    body.seek(SeekFrom::Start(0)).map_err(io(file))?;
    let mut partial = into.as_os_str().to_owned();
    partial.push(PARTIAL_SUFFIX);
    let partial = Partial(PathBuf::from(partial));
    partial.clear().map_err(io(&partial.0))?;
    std::fs::create_dir(&partial.0).map_err(io(&partial.0))?;
    extract(name, body, &partial.0, limit)?;
    rename_retrying(
        &partial.0,
        into,
        std::time::Duration::from_millis(100),
        busy,
        |from, to| std::fs::rename(from, to),
    )
}

/// The exclusive lock on `<into>.lock` ([`LOCK_SUFFIX`]), held by the returned file until it
/// drops: a second unpack to the same destination waits up to a second, then is refused as
/// [`Refusal::Busy`]. It is an OS file lock, so a crashed unpack never leaves it held. It guards
/// only this unpack's folders; PR 6's cache lock wraps it for the whole download and stamp. The
/// lock file is left in place: delete it only while holding that outer cache lock, since another
/// unpack may hold it. A file system without file locks is refused as [`Refusal::NoLocks`].
/// `try_lock` is `File::try_lock`, passed in so tests can inject its errors.
#[cfg(feature = "prebuilt")]
fn lock(
    into: &Path,
    try_lock: impl Fn(&std::fs::File) -> Result<(), std::fs::TryLockError>,
) -> Result<std::fs::File, Refusal> {
    let mut path = into.as_os_str().to_owned();
    path.push(LOCK_SUFFIX);
    let path = PathBuf::from(path);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|error| Refusal::Io(format!("{}: {error}", path.display())))?;
    for _ in 0..20 {
        match try_lock(&file) {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) => {
                std::thread::sleep(std::time::Duration::from_millis(50))
            }
            Err(std::fs::TryLockError::Error(error)) if no_locks(&error) => {
                return Err(Refusal::NoLocks(into.display().to_string()));
            }
            Err(std::fs::TryLockError::Error(error)) => {
                return Err(Refusal::Io(format!("{}: {error}", path.display())));
            }
        }
    }
    Err(Refusal::Busy(into.display().to_string()))
}

/// Whether a failed lock means the file system has no file locks: `ENOTSUP`/`EOPNOTSUPP` or
/// `ENOLCK` on Unix, `ERROR_NOT_SUPPORTED` or `ERROR_INVALID_FUNCTION` on Windows, or the
/// standard library's own "unsupported".
#[cfg(feature = "prebuilt")]
fn no_locks(error: &std::io::Error) -> bool {
    let codes: &[i32] = if cfg!(windows) {
        &[1, 50]
    } else if cfg!(target_vendor = "apple") {
        &[45, 102, 77]
    } else {
        &[95, 37]
    };
    error.kind() == std::io::ErrorKind::Unsupported
        || error
            .raw_os_error()
            .is_some_and(|code| codes.contains(&code))
}

/// Rename `from` to `to`, retrying while [`busy`] (Windows, while a scanner or indexer holds a file
/// just written) five times, waiting `delay`, then twice as long each time (about
/// 3 s at 100 ms); then [`Refusal::Rename`].
#[cfg(feature = "prebuilt")]
fn rename_retrying(
    from: &Path,
    to: &Path,
    mut delay: std::time::Duration,
    busy: impl Fn(&std::io::Error) -> bool,
    mut rename: impl FnMut(&Path, &Path) -> std::io::Result<()>,
) -> Result<(), Refusal> {
    let mut tries = 0;
    loop {
        match rename(from, to) {
            Ok(()) => return Ok(()),
            Err(error) if busy(&error) && tries < 5 => {
                tries += 1;
                std::thread::sleep(delay);
                delay *= 2;
            }
            Err(error) => {
                return Err(Refusal::Rename {
                    to: to.display().to_string(),
                    why: error.to_string(),
                });
            }
        }
    }
}

/// Whether a failed rename is another process briefly holding a file: on Windows access denied or
/// a sharing violation (OS error 32); elsewhere never.
#[cfg(feature = "prebuilt")]
fn busy(error: &std::io::Error) -> bool {
    cfg!(windows)
        && (error.kind() == std::io::ErrorKind::PermissionDenied
            || error.raw_os_error() == Some(32))
}

/// Decode the xz tar `body` into the existing folder `into`, counting the decoded bytes as they
/// are read.
#[cfg(feature = "prebuilt")]
fn extract(name: &str, body: impl Read + 'static, into: &Path, limit: u64) -> Result<(), Refusal> {
    use std::{cell::Cell, rc::Rc};
    /// A reader that fails once more than `limit` bytes came through it.
    struct Capped<R> {
        inner: R,
        read: Rc<Cell<u64>>,
        limit: u64,
    }
    impl<R: Read> Read for Capped<R> {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let read = self.inner.read(buffer)?;
            self.read.set(self.read.get() + read as u64);
            if self.read.get() > self.limit {
                return Err(std::io::Error::other("over the limit"));
            }
            Ok(read)
        }
    }
    let decoder = xz4rust::XzDecoder::in_heap_with_alloc_dict_size(1 << 20, DICTIONARY_LIMIT);
    let buffer = std::num::NonZeroUsize::new(1 << 16).expect("non-zero");
    let read = Rc::new(Cell::new(0));
    let capped = Capped {
        inner: xz4rust::XzReader::new_with_buffer_size_and_decoder(body, buffer, decoder),
        read: read.clone(),
        limit,
    };
    untar(name, capped, into).map_err(|refusal| match refusal {
        Refusal::Archive { .. } if read.get() > limit => Refusal::TooLarge {
            asset: name.to_owned(),
            limit,
        },
        other => other,
    })
}

/// The PAX records [`untar`] accepts: `path`, applied to the next entry, and metadata it does not
/// use. Any other record (a `size` override, say) is refused.
#[cfg(feature = "prebuilt")]
const PAX_KEYS: [&str; 10] = [
    "path", "linkpath", "mtime", "atime", "ctime", "uid", "gid", "uname", "gname", "comment",
];

/// Unpack the tar stream `body` into the existing folder `into`. Entries are read raw, so the tar
/// reader never buffers an extension entry itself: a GNU long name or a PAX `path` is read here,
/// capped at [`EXTENSION_LIMIT`], and applied to the entry after it; one with no entry after it is
/// refused. Only files and folders are written, each at a path [`entry_target`] accepts.
#[cfg(feature = "prebuilt")]
fn untar(name: &str, body: impl Read, into: &Path) -> Result<(), Refusal> {
    let damaged = |why: String| Refusal::Archive {
        asset: name.to_owned(),
        why,
    };
    let error = |error: std::io::Error| damaged(error.to_string());
    let mut archive = tar::Archive::new(body);
    let mut long_name: Option<Vec<u8>> = None;
    for entry in archive.entries().map_err(error)?.raw(true) {
        let mut entry = entry.map_err(error)?;
        let kind = entry.header().entry_type();
        let size = entry.header().entry_size().map_err(error)?;
        if kind.is_gnu_longname()
            || kind.is_gnu_longlink()
            || kind.is_pax_local_extensions()
            || kind.is_pax_global_extensions()
        {
            if size > EXTENSION_LIMIT {
                return Err(damaged(format!(
                    "a tar extension entry of {size} bytes, over {EXTENSION_LIMIT}"
                )));
            }
            let mut data = Vec::with_capacity(size as usize);
            entry.read_to_end(&mut data).map_err(error)?;
            if kind.is_gnu_longname() {
                long_name = Some(data.split(|b| *b == 0).next().unwrap_or_default().to_vec());
            } else if kind.is_pax_local_extensions() || kind.is_pax_global_extensions() {
                for extension in tar::PaxExtensions::new(&data) {
                    let extension = extension.map_err(error)?;
                    let key = extension.key().unwrap_or("");
                    if !PAX_KEYS.contains(&key) {
                        return Err(damaged(format!("a tar PAX record {key:?}")));
                    }
                    if key == "path" {
                        if kind.is_pax_global_extensions() {
                            return Err(damaged("a global tar PAX path".into()));
                        }
                        long_name = Some(extension.value_bytes().to_vec());
                    }
                }
            }
            continue;
        }
        let path = match long_name.take() {
            Some(bytes) => bytes,
            None => entry.path_bytes().into_owned(),
        };
        let shown = String::from_utf8_lossy(&path).into_owned();
        if !kind.is_file() && !kind.is_dir() {
            return Err(damaged(format!("{shown} is not a file or folder")));
        }
        let target = entry_target(into, &path)
            .ok_or_else(|| damaged(format!("{shown} is not a plain path inside the folder")))?;
        if kind.is_dir() {
            std::fs::create_dir_all(&target).map_err(error)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(error)?;
        }
        let mut file = std::fs::File::create(&target).map_err(error)?;
        std::io::copy(&mut entry, &mut file).map_err(error)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = entry.header().mode().map_err(error)? & 0o755;
            file.set_permissions(std::fs::Permissions::from_mode(mode | 0o600))
                .map_err(error)?;
        }
    }
    if long_name.is_some() {
        return Err(damaged("a long name with no entry after it".into()));
    }
    Ok(())
}

/// Where the archive path `path` (`/`-separated bytes) goes under `into`, or `None` unless it is
/// plain on every OS: relative, UTF-8, and each component neither `..` nor holding `:`, `\` or a
/// NUL, nor a Windows device name (`CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9`, with
/// any extension), nor ending in a dot or a space. `.` (the archive's root folder) is `into`
/// itself. The result is checked to stay under `into`.
#[cfg(feature = "prebuilt")]
fn entry_target(into: &Path, path: &[u8]) -> Option<PathBuf> {
    let path = std::str::from_utf8(path).ok()?;
    if path.starts_with('/') {
        return None;
    }
    let mut target = into.to_path_buf();
    for part in path
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
    {
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        let device = matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        ) || stem.split_at_checked(3).is_some_and(|(port, number)| {
            matches!(port, "COM" | "LPT")
                && matches!(
                    number,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
        });
        if part == ".." || part.contains([':', '\\', '\0']) || device || part.ends_with(['.', ' '])
        {
            return None;
        }
        target.push(part);
    }
    // The backstop: every component was checked above, but this check alone would have caught
    // round 3's bug (`deps/C:x` passing as plain on Linux and escaping on Windows), had the
    // component rules missed it. It keeps any future gap in them from writing outside `into`.
    let inside = target.strip_prefix(into).ok()?;
    inside
        .components()
        .all(|part| matches!(part, std::path::Component::Normal(_)))
        .then_some(target)
}

/// The folder an unpack writes before its rename, removed when dropped.
#[cfg(feature = "prebuilt")]
struct Partial(PathBuf);

#[cfg(feature = "prebuilt")]
impl Partial {
    /// Remove what a crashed unpack left here.
    fn clear(&self) -> std::io::Result<()> {
        match std::fs::symlink_metadata(&self.0) {
            Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(&self.0),
            Ok(_) => std::fs::remove_file(&self.0),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

#[cfg(feature = "prebuilt")]
impl Drop for Partial {
    fn drop(&mut self) {
        // After the rename there is nothing here to remove.
        let _ = self.clear();
    }
}

#[cfg(test)]
#[path = "sdk/tests.rs"]
mod tests;
