//! Diagnostics for unavailable original-jar inputs at reload.
/// An accepted root's original-jar inputs could not be checked.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum JarInputsError {
    /// An exact original is unavailable, with every requiring mod retained.
    Missing(Box<MissingJar>),
    /// The accepted inputs or cache metadata could not be read.
    Io {
        /// The input file or cache directory.
        path: String,
        /// The filesystem diagnostic.
        message: String,
    },
    /// The accepted input record failed schema validation.
    Invalid {
        /// The accepted input record.
        path: String,
        /// The decoding or validation diagnostic.
        message: String,
    },
}
/// One missing original file and its owners.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct MissingJar {
    /// Every mod requiring this exact file, in load order.
    pub owners: Vec<String>,
    /// The original jar's id.
    pub id: String,
    /// The original jar's version.
    pub version: String,
    /// The exact file's SHA-512.
    pub sha512: String,
    /// Its expected cache path.
    pub path: String,
}
impl std::fmt::Display for JarInputsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing(m) => write!(
                f,
                "{}: {} {}: {}",
                m.owners.join(", "),
                m.id,
                m.version,
                m.path
            ),
            Self::Io { path, message } | Self::Invalid { path, message } => {
                write!(f, "{path}: {message}")
            }
        }
    }
}
impl std::error::Error for JarInputsError {}

impl MissingJar {
    /// A missing exact file, retaining its requiring mods and original identity.
    pub fn new(
        owners: Vec<String>,
        id: String,
        version: String,
        sha512: String,
        path: String,
    ) -> Self {
        Self {
            owners,
            id,
            version,
            sha512,
            path,
        }
    }
}
