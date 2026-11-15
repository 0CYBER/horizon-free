//! Behaviour records: one file `crates/horizon-mod-vocab/behaviours/<name>.toml` per promise a mod
//! relies on that is no type or signature. The file name is the id (`behaviour:<name>`).
//!
//! ```toml
//! version = 1
//! doc = "What the engine promises, stated so that a test could check it."
//! tests = ["crates/horizon-client/src/some_tests.rs::the_test_that_pins_it"]
//! ```
//!
//! [`read`] refuses an unreadable folder or file, bad TOML, an unknown key (by name), a missing or
//! mistyped key, a file name that is not an id, no tests, and a test whose file has no `fn` of
//! that name, each with a typed [`BehaviourError`] that names the file.
use mod_surface::{Behaviour, BehaviourContract};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BehaviourError {
    /// The folder or a file could not be read.
    Read { file: String, error: String },
    /// A test's file is not Rust.
    Unparsable { file: String, error: String },
    /// Not TOML.
    Toml { file: String, error: String },
    /// A key a behaviour does not have.
    UnknownKey { file: String, key: String },
    /// A key it must have.
    Missing { file: String, key: &'static str },
    /// A key whose value is not what it must be.
    WrongType {
        file: String,
        key: &'static str,
        expected: &'static str,
    },
    /// The file name is not a lowercase id (`a-z`, `0-9`, `_`, starting with a letter).
    NotAnId { file: String },
    /// `tests` lists nothing.
    NoTests { file: String },
    /// A test that is not `<file path>::<fn name>`.
    BadTest { file: String, test: String },
    /// A test whose file has no `fn` of that name.
    NoSuchTest { file: String, test: String },
}

impl std::fmt::Display for BehaviourError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read { file, error } => write!(f, "behaviour {file}: {error}"),
            Self::Unparsable { file, error } => {
                write!(f, "behaviour test file {file} is not Rust: {error}")
            }
            Self::Toml { file, error } => write!(f, "behaviour {file} is not TOML: {error}"),
            Self::UnknownKey { file, key } => write!(
                f,
                "behaviour {file} has the unknown key `{key}` (a behaviour has `version`, `doc` and `tests`)"
            ),
            Self::Missing { file, key } => write!(f, "behaviour {file} has no `{key}`"),
            Self::WrongType {
                file,
                key,
                expected,
            } => write!(f, "behaviour {file}: `{key}` must be {expected}"),
            Self::NotAnId { file } => write!(
                f,
                "behaviour {file}: the file name must be an id of lowercase letters, digits and `_`, starting with a letter"
            ),
            Self::NoTests { file } => write!(f, "behaviour {file} names no tests"),
            Self::BadTest { file, test } => write!(
                f,
                "behaviour {file}: test `{test}` must be `<file path>::<fn name>`"
            ),
            Self::NoSuchTest { file, test } => {
                write!(f, "behaviour {file}: no `fn` for test `{test}`")
            }
        }
    }
}

impl std::error::Error for BehaviourError {}

/// Reads one behaviour: `file` is its file name (`<name>.toml`) and `text` its contents.
/// `exists` says whether a test's file has a `fn` of its name.
pub fn parse(
    file: &str,
    text: &str,
    exists: impl Fn(&str, &str) -> Result<bool, BehaviourError>,
) -> Result<Behaviour, BehaviourError> {
    let name = file.strip_suffix(".toml").unwrap_or(file);
    if !name.starts_with(|c: char| c.is_ascii_lowercase())
        || !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(BehaviourError::NotAnId { file: file.into() });
    }
    let table: toml::Table = text
        .parse()
        .map_err(|e: toml::de::Error| BehaviourError::Toml {
            file: file.into(),
            error: e.message().to_owned(),
        })?;
    if let Some(key) = table
        .keys()
        .find(|k| !["version", "doc", "tests"].contains(&k.as_str()))
    {
        return Err(BehaviourError::UnknownKey {
            file: file.into(),
            key: key.clone(),
        });
    }
    let wrong = |key, expected| BehaviourError::WrongType {
        file: file.into(),
        key,
        expected,
    };
    let get = |key| {
        table.get(key).ok_or(BehaviourError::Missing {
            file: file.into(),
            key,
        })
    };
    let version = get("version")?
        .as_integer()
        .and_then(|v| u32::try_from(v).ok())
        .filter(|v| *v >= 1)
        .ok_or_else(|| wrong("version", "a whole number from 1"))?;
    let doc = get("doc")?
        .as_str()
        .ok_or_else(|| wrong("doc", "text"))?
        .to_owned();
    let tests: Vec<String> = get("tests")?
        .as_array()
        .ok_or_else(|| wrong("tests", "a list of tests"))?
        .iter()
        .map(|test| test.as_str().map(str::to_owned))
        .collect::<Option<_>>()
        .ok_or_else(|| wrong("tests", "a list of tests"))?;
    if tests.is_empty() {
        return Err(BehaviourError::NoTests { file: file.into() });
    }
    for test in &tests {
        let bad = || BehaviourError::BadTest {
            file: file.into(),
            test: test.clone(),
        };
        let (path, function) = test.rsplit_once("::").ok_or_else(bad)?;
        if path.is_empty() || function.is_empty() {
            return Err(bad());
        }
        if !exists(path, function)? {
            return Err(BehaviourError::NoSuchTest {
                file: file.into(),
                test: test.clone(),
            });
        }
    }
    Ok(Behaviour::new(
        format!("behaviour:{name}"),
        doc,
        tests,
        BehaviourContract { version },
    ))
}

/// Whether `items` hold a function called `name`, in an inline module too.
fn declares(items: &[syn::Item], name: &str) -> bool {
    items.iter().any(|item| match item {
        syn::Item::Fn(function) => function.sig.ident == name,
        syn::Item::Mod(module) => module
            .content
            .as_ref()
            .is_some_and(|(_, items)| declares(items, name)),
        _ => false,
    })
}

/// Every behaviour of `dir` by file name; none when the folder is absent. A test's file is read
/// from the tree at `root`.
pub fn read(root: &Path, dir: &Path) -> Result<Vec<Behaviour>, BehaviourError> {
    let error = |file: &Path, e: std::io::Error| BehaviourError::Read {
        file: file.display().to_string(),
        error: e.to_string(),
    };
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(error(dir, e)),
    };
    let mut paths = Vec::new();
    for entry in entries {
        let path = entry.map_err(|e| error(dir, e))?.path();
        if path.extension().is_some_and(|e| e == "toml") {
            paths.push(path);
        }
    }
    paths.sort();
    let exists = |behaviour: &str, path: &str, name: &str| {
        let file = root.join(path);
        let text = std::fs::read_to_string(&file).map_err(|e| BehaviourError::Read {
            file: behaviour.to_owned(),
            error: format!("its test file {}: {e}", file.display()),
        })?;
        let parsed = syn::parse_file(&text).map_err(|e| BehaviourError::Unparsable {
            file: file.display().to_string(),
            error: e.to_string(),
        })?;
        Ok(declares(&parsed.items, name))
    };
    paths
        .iter()
        .map(|path| {
            let file = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let text = std::fs::read_to_string(path).map_err(|e| error(path, e))?;
            parse(&file, &text, |path, name| exists(&file, path, name))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(_: &str, _: &str) -> Result<bool, BehaviourError> {
        Ok(true)
    }

    const GOOD: &str =
        "version = 2\ndoc = \"It holds.\"\ntests = [\"a/b.rs::holds\", \"c.rs::also\"]\n";

    #[test]
    fn a_behaviour_reads_its_id_version_text_and_tests() {
        let behaviour = parse("pool_merging.toml", GOOD, found).unwrap();
        assert_eq!(behaviour.id, "behaviour:pool_merging");
        assert_eq!(behaviour.contract.version, 2);
        assert_eq!(behaviour.doc, "It holds.");
        assert_eq!(behaviour.tests, ["a/b.rs::holds", "c.rs::also"]);
    }

    #[test]
    fn malformed_behaviours_are_refused_with_a_typed_error_naming_the_file() {
        let file = || "b.toml".to_owned();
        let wrong = |key, expected| BehaviourError::WrongType {
            file: file(),
            key,
            expected,
        };
        for (name, text, expected) in [
            (
                "1b.toml",
                GOOD,
                BehaviourError::NotAnId {
                    file: "1b.toml".into(),
                },
            ),
            (
                "B.toml",
                GOOD,
                BehaviourError::NotAnId {
                    file: "B.toml".into(),
                },
            ),
            (
                "a-b.toml",
                GOOD,
                BehaviourError::NotAnId {
                    file: "a-b.toml".into(),
                },
            ),
            (
                "b.toml",
                "version = 1\ndoc = \"d\"\ntests = [\"a.rs::t\"]\nwhy = \"x\"\n",
                BehaviourError::UnknownKey {
                    file: file(),
                    key: "why".into(),
                },
            ),
            (
                "b.toml",
                "doc = \"d\"\ntests = [\"a.rs::t\"]\n",
                BehaviourError::Missing {
                    file: file(),
                    key: "version",
                },
            ),
            (
                "b.toml",
                "version = 1\ntests = [\"a.rs::t\"]\n",
                BehaviourError::Missing {
                    file: file(),
                    key: "doc",
                },
            ),
            (
                "b.toml",
                "version = 1\ndoc = \"d\"\n",
                BehaviourError::Missing {
                    file: file(),
                    key: "tests",
                },
            ),
            (
                "b.toml",
                "version = 0\ndoc = \"d\"\ntests = [\"a.rs::t\"]\n",
                wrong("version", "a whole number from 1"),
            ),
            (
                "b.toml",
                "version = 1\ndoc = 3\ntests = [\"a.rs::t\"]\n",
                wrong("doc", "text"),
            ),
            (
                "b.toml",
                "version = 1\ndoc = \"d\"\ntests = \"a.rs::t\"\n",
                wrong("tests", "a list of tests"),
            ),
            (
                "b.toml",
                "version = 1\ndoc = \"d\"\ntests = []\n",
                BehaviourError::NoTests { file: file() },
            ),
            (
                "b.toml",
                "version = 1\ndoc = \"d\"\ntests = [\"just_a_name\"]\n",
                BehaviourError::BadTest {
                    file: file(),
                    test: "just_a_name".into(),
                },
            ),
        ] {
            assert_eq!(parse(name, text, found), Err(expected), "{name}: {text}");
        }
        assert!(matches!(
            parse("b.toml", "version = ", found),
            Err(BehaviourError::Toml { .. })
        ));
        assert_eq!(
            parse("b.toml", GOOD, |_, name| Ok(name != "also")),
            Err(BehaviourError::NoSuchTest {
                file: file(),
                test: "c.rs::also".into()
            })
        );
    }

    #[test]
    fn a_test_is_found_by_parsing_for_an_fn_item() {
        let text = "// fn commented() {}\nconst S: &str = \"fn quoted\";\n#[test]\nfn\n spaced() {}\nfn holds_for_all() {}\n#[cfg(test)]\nmod tests { mod deeper { fn nested() {} } }\n";
        let items = syn::parse_file(text).unwrap().items;
        for name in ["spaced", "holds_for_all", "nested"] {
            assert!(declares(&items, name), "{name}");
        }
        for name in ["commented", "quoted", "holds_for", "hold", "tests"] {
            assert!(!declares(&items, name), "{name}");
        }
    }

    #[test]
    fn the_folder_is_read_by_name_and_its_tests_are_looked_up_in_the_tree() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("behaviours");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(dir.path().join("t.rs"), "#[test]\nfn pins_it() {}\n").unwrap();
        let write = |name: &str, test: &str| {
            std::fs::write(
                folder.join(name),
                format!("version = 1\ndoc = \"d\"\ntests = [\"{test}\"]\n"),
            )
            .unwrap();
        };
        write("zeta.toml", "t.rs::pins_it");
        write("alpha.toml", "t.rs::pins_it");
        std::fs::write(folder.join("README.md"), "not a behaviour").unwrap();
        let ids: Vec<String> = read(dir.path(), &folder)
            .unwrap()
            .into_iter()
            .map(|b| b.id)
            .collect();
        assert_eq!(ids, ["behaviour:alpha", "behaviour:zeta"]);
        write("zeta.toml", "t.rs::gone");
        assert_eq!(
            read(dir.path(), &folder),
            Err(BehaviourError::NoSuchTest {
                file: "zeta.toml".into(),
                test: "t.rs::gone".into()
            })
        );
        std::fs::write(dir.path().join("bad.rs"), "fn (").unwrap();
        write("zeta.toml", "bad.rs::pins_it");
        assert!(matches!(
            read(dir.path(), &folder),
            Err(BehaviourError::Unparsable { .. })
        ));
        write("zeta.toml", "missing.rs::pins_it");
        let Err(BehaviourError::Read { file, error }) = read(dir.path(), &folder) else {
            panic!("a missing test file is a read error");
        };
        assert_eq!(file, "zeta.toml");
        assert!(
            error.starts_with("its test file ") && error.contains("missing.rs"),
            "{error}"
        );
        assert!(
            read(dir.path(), &dir.path().join("none"))
                .unwrap()
                .is_empty()
        );
    }
}
