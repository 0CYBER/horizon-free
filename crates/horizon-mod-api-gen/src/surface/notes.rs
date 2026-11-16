//! Change notes: optional files `crates/horizon-mod-vocab/changes/<name>.toml`, one per change,
//! that say why surface items changed and how a mod moves on.
//!
//! ```toml
//! ids = ["hook:player_logged_in", "type:PlayerLoggedIn.player"]
//! why = "The hook now fires on the server only."
//! migrate = "Take `ServerPlayerRef` instead of `PlayerRef`."   # optional
//! ```
//!
//! [`read`] refuses an unreadable file, bad TOML, an unknown key (by name), a missing or
//! mistyped key and an empty `ids` with a typed [`NoteError`]. A note is *new* when the base
//! tree has no file of that name or a different one; only new notes are attached to a diff, and
//! a new note that names an id the diff does not list is stale.
use mod_surface::diff::Note;
use std::path::Path;

/// A note as read: its text and the ids it names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteFile {
    pub note: Note,
    pub ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoteError {
    /// The folder or a file could not be read.
    Read { file: String, error: String },
    /// Not TOML.
    Toml { file: String, error: String },
    /// A key a note does not have.
    UnknownKey { file: String, key: String },
    /// A key it must have.
    Missing { file: String, key: &'static str },
    /// A key whose value is not what it must be.
    WrongType {
        file: String,
        key: &'static str,
        expected: &'static str,
    },
    /// `ids` lists nothing.
    NoIds { file: String },
}

impl std::fmt::Display for NoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read { file, error } => write!(f, "change note {file}: {error}"),
            Self::Toml { file, error } => write!(f, "change note {file} is not TOML: {error}"),
            Self::UnknownKey { file, key } => write!(
                f,
                "change note {file} has the unknown key `{key}` (a note has `ids`, `why` and `migrate`)"
            ),
            Self::Missing { file, key } => write!(f, "change note {file} has no `{key}`"),
            Self::WrongType {
                file,
                key,
                expected,
            } => write!(f, "change note {file}: `{key}` must be {expected}"),
            Self::NoIds { file } => write!(f, "change note {file} names no ids"),
        }
    }
}

impl std::error::Error for NoteError {}

/// Reads one note's text; `file` names it in errors and in the report.
pub fn parse(file: &str, text: &str) -> Result<NoteFile, NoteError> {
    let table: toml::Table = text.parse().map_err(|e: toml::de::Error| NoteError::Toml {
        file: file.into(),
        error: e.message().to_owned(),
    })?;
    if let Some(key) = table
        .keys()
        .find(|k| !["ids", "why", "migrate"].contains(&k.as_str()))
    {
        return Err(NoteError::UnknownKey {
            file: file.into(),
            key: key.clone(),
        });
    }
    let wrong = |key, expected| NoteError::WrongType {
        file: file.into(),
        key,
        expected,
    };
    let ids = table.get("ids").ok_or(NoteError::Missing {
        file: file.into(),
        key: "ids",
    })?;
    let ids: Vec<String> = ids
        .as_array()
        .ok_or_else(|| wrong("ids", "a list of ids"))?
        .iter()
        .map(|id| id.as_str().map(str::to_owned))
        .collect::<Option<_>>()
        .ok_or_else(|| wrong("ids", "a list of ids"))?;
    if ids.is_empty() {
        return Err(NoteError::NoIds { file: file.into() });
    }
    let why = table
        .get("why")
        .ok_or(NoteError::Missing {
            file: file.into(),
            key: "why",
        })?
        .as_str()
        .ok_or_else(|| wrong("why", "text"))?
        .to_owned();
    let migrate = match table.get("migrate") {
        None => None,
        Some(value) => Some(
            value
                .as_str()
                .ok_or_else(|| wrong("migrate", "text"))?
                .to_owned(),
        ),
    };
    Ok(NoteFile {
        note: Note {
            file: file.into(),
            why,
            migrate,
        },
        ids,
    })
}

/// The `*.toml` files of `dir` by name, with their bytes; none when the folder is absent.
fn files(dir: &Path) -> Result<Vec<(String, Vec<u8>)>, NoteError> {
    let error = |file: &Path, e: std::io::Error| NoteError::Read {
        file: file.display().to_string(),
        error: e.to_string(),
    };
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(error(dir, e)),
    };
    let mut out = Vec::new();
    for entry in entries {
        let path = entry.map_err(|e| error(dir, e))?.path();
        if path.extension().is_none_or(|e| e != "toml") {
            continue;
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let bytes = std::fs::read(&path).map_err(|e| error(&path, e))?;
        out.push((name, bytes));
    }
    out.sort();
    Ok(out)
}

/// The new notes of `dir`: those whose file name is not in `base`, and whose bytes are not
/// either. A note already merged is an earlier change's, so editing or renaming it is neither
/// read nor checked.
pub fn read(dir: &Path, base: &Path) -> Result<Vec<NoteFile>, NoteError> {
    let old = files(base)?;
    files(dir)?
        .into_iter()
        .filter(|(name, bytes)| !old.iter().any(|(n, b)| n == name || b == bytes))
        .map(|(name, bytes)| {
            let text = String::from_utf8(bytes).map_err(|_| NoteError::Toml {
                file: name.clone(),
                error: "not UTF-8".into(),
            })?;
            parse(&name, &text)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_reads_its_ids_and_text() {
        let note = parse(
            "a.toml",
            "ids = [\"hook:tick\"]\nwhy = \"Faster.\"\nmigrate = \"Nothing to do.\"\n",
        )
        .unwrap();
        assert_eq!(note.ids, ["hook:tick"]);
        assert_eq!(note.note.why, "Faster.");
        assert_eq!(note.note.migrate.as_deref(), Some("Nothing to do."));
        let bare = parse("b.toml", "ids = [\"hook:tick\"]\nwhy = \"Faster.\"\n").unwrap();
        assert_eq!(bare.note.migrate, None);
    }

    #[test]
    fn malformed_notes_are_refused_with_a_typed_error() {
        let file = || "n.toml".to_owned();
        for (text, expected) in [
            (
                "ids = [\"x\"]\nwhy = \"y\"\nmigration = \"z\"\n",
                NoteError::UnknownKey {
                    file: file(),
                    key: "migration".into(),
                },
            ),
            (
                "why = \"y\"\n",
                NoteError::Missing {
                    file: file(),
                    key: "ids",
                },
            ),
            (
                "ids = [\"x\"]\n",
                NoteError::Missing {
                    file: file(),
                    key: "why",
                },
            ),
            (
                "ids = \"x\"\nwhy = \"y\"\n",
                NoteError::WrongType {
                    file: file(),
                    key: "ids",
                    expected: "a list of ids",
                },
            ),
            (
                "ids = [1]\nwhy = \"y\"\n",
                NoteError::WrongType {
                    file: file(),
                    key: "ids",
                    expected: "a list of ids",
                },
            ),
            (
                "ids = [\"x\"]\nwhy = 3\n",
                NoteError::WrongType {
                    file: file(),
                    key: "why",
                    expected: "text",
                },
            ),
            (
                "ids = [\"x\"]\nwhy = \"y\"\nmigrate = []\n",
                NoteError::WrongType {
                    file: file(),
                    key: "migrate",
                    expected: "text",
                },
            ),
            ("ids = []\nwhy = \"y\"\n", NoteError::NoIds { file: file() }),
        ] {
            assert_eq!(parse("n.toml", text), Err(expected), "{text}");
        }
        assert!(matches!(
            parse("n.toml", "ids = [\"x\"\nwhy"),
            Err(NoteError::Toml { .. })
        ));
    }

    #[test]
    fn only_notes_new_since_the_base_are_read() {
        let dir = tempfile::tempdir().unwrap();
        let (head, base) = (dir.path().join("head"), dir.path().join("base"));
        std::fs::create_dir_all(&head).unwrap();
        std::fs::create_dir_all(&base).unwrap();
        let note = |name: &str, why: &str| format!("ids = [\"{name}\"]\nwhy = \"{why}\"\n");
        std::fs::write(head.join("old.toml"), note("a", "same")).unwrap();
        std::fs::write(base.join("old.toml"), note("a", "same")).unwrap();
        std::fs::write(head.join("edited.toml"), note("b", "now")).unwrap();
        std::fs::write(base.join("edited.toml"), note("b", "before")).unwrap();
        std::fs::write(head.join("added.toml"), note("c", "new")).unwrap();
        // Renamed with its text unchanged: still the earlier change's.
        std::fs::write(head.join("renamed.toml"), note("e", "moved")).unwrap();
        std::fs::write(base.join("before-rename.toml"), note("e", "moved")).unwrap();
        std::fs::write(head.join("README.md"), "not a note").unwrap();
        let names = |found: Vec<NoteFile>| -> Vec<String> {
            found.into_iter().map(|n| n.note.file).collect()
        };
        // An edited note keeps its name, and stays an earlier change's, even when it no longer
        // parses.
        std::fs::write(head.join("broken.toml"), "ids = [").unwrap();
        std::fs::write(base.join("broken.toml"), note("d", "fine")).unwrap();
        assert_eq!(names(read(&head, &base).unwrap()), ["added.toml"]);
        std::fs::remove_file(head.join("broken.toml")).unwrap();
        assert_eq!(
            names(read(&head, &dir.path().join("none")).unwrap()),
            ["added.toml", "edited.toml", "old.toml", "renamed.toml"]
        );
        // An absent folder holds no notes, on either side.
        assert!(
            read(&dir.path().join("none"), &dir.path().join("none"))
                .unwrap()
                .is_empty()
        );
    }
}
