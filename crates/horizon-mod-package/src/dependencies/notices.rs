//! Structured startup notices, retained for both affected Mods-page rows.
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeKind {
    Discouraged,
    OverrideDropped,
    OverrideDiscouraged,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub owner: String,
    pub target: String,
    pub version: Option<String>,
    pub reason: Option<String>,
    pub url: Option<String>,
    pub kind: NoticeKind,
}
impl Notice {
    /// A translation key and literal arguments, before the Mods-page note layout.
    pub fn message(&self) -> (String, Vec<String>) {
        let mut args = vec![self.owner.clone(), self.target.clone()];
        let key = match self.kind {
            NoticeKind::Discouraged => {
                if let Some(version) = &self.version {
                    args.push(version.clone());
                    "horizon.mods.warning.discouraged"
                } else {
                    "horizon.mods.warning.discouraged_missing"
                }
            }
            NoticeKind::OverrideDropped => "horizon.mods.warning.override_dropped",
            NoticeKind::OverrideDiscouraged => "horizon.mods.warning.override_discouraged",
        };
        (key.to_owned(), args)
    }
}
impl fmt::Display for Notice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            NoticeKind::OverrideDropped => write!(
                f,
                "dependency override: {} -> {} dropped",
                self.owner, self.target
            ),
            NoticeKind::OverrideDiscouraged => write!(
                f,
                "dependency override: {} -> {} only discouraged",
                self.owner, self.target
            ),
            NoticeKind::Discouraged => {
                write!(f, "{} works badly with {}", self.owner, self.target)?;
                if let Some(version) = &self.version {
                    write!(f, " {version}")?;
                }
                write!(
                    f,
                    "{}",
                    super::diagnostic_note(self.reason.as_deref(), self.url.as_deref())
                )
            }
        }
    }
}
