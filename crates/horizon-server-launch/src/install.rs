//! Read-only paths in an installer-managed server installation.
use std::path::{Path, PathBuf};

/// The installer's folder: `releases/`, `current`, `previous` and `game-files/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Install {
    pub root: PathBuf,
}

impl Install {
    /// The install this program runs from, if it runs from one.
    pub fn find() -> Option<Self> {
        Self::around(&std::env::current_exe().ok()?)
    }

    /// The install holding `program` (`<root>/releases/<tag>/horizon-server`, reached directly or through
    /// `current`).
    pub fn around(program: &Path) -> Option<Self> {
        let program = dunce::canonicalize(program).ok()?;
        let releases = program.parent()?.parent()?;
        if releases.file_name()? != "releases" {
            return None;
        }
        let root = releases.parent()?.to_path_buf();
        root.join("current").is_dir().then_some(Self { root })
    }

    /// The install, or why there is none to update.
    pub fn required() -> Result<Self, String> {
        Self::find().ok_or_else(|| {
            "this horizon-server wasn't installed by install.sh or install.ps1 (no releases/ and current \
             around it): update it the way you installed it"
                .into()
        })
    }

    pub fn release(&self, tag: &str) -> PathBuf {
        self.root.join("releases").join(tag)
    }

    /// Where the game programs in use are: `current`, so a switch applies at the next start.
    pub fn programs(&self) -> PathBuf {
        self.root.join("current")
    }

    /// The release in use.
    pub fn current(&self) -> Option<String> {
        let target = std::fs::canonicalize(self.root.join("current")).ok()?;
        Some(target.file_name()?.to_string_lossy().into_owned())
    }

    /// The release used before the current one, if it is still there.
    pub fn previous(&self) -> Option<String> {
        let tag = std::fs::read_to_string(self.root.join("previous")).ok()?;
        let tag = tag.trim();
        (!tag.is_empty() && self.release(tag).is_dir()).then(|| tag.to_owned())
    }
}
