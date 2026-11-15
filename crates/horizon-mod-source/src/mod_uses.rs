//! A mod folder as `horizon-stage uses` scans it: its Rust sources and what its manifest names.
use mod_surface::scan::{Token, tokens};
use std::path::{Path, PathBuf};

/// The largest Rust source file read, in bytes; a larger one is refused, never truncated.
pub const MAX_SOURCE_BYTES: u64 = 8 << 20;
/// The most Rust source read from one mod, in bytes.
pub const MAX_TOTAL_BYTES: u64 = 64 << 20;
/// The most Rust source files read from one mod.
pub const MAX_FILES: usize = 4096;
/// The largest `horizon-mod.toml` read, in bytes.
pub const MAX_MANIFEST_BYTES: u64 = 64 << 10;

/// What [`read`] found in a mod folder.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ModInput {
    /// Each `.rs` file's path, relative and `/`-separated, with its text, sorted by path.
    pub sources: Vec<(String, String)>,
    /// The capability keys `horizon-mod.toml` lists.
    pub manifest_names: Vec<String>,
}

/// Why a mod folder cannot be judged.
#[derive(Debug, PartialEq, Eq)]
pub enum ReadError {
    /// A file or folder could not be read.
    Io { path: PathBuf, error: String },
    /// A symbolic link in the folder: what it points to is not the mod's own.
    Symlink(PathBuf),
    /// A `#[path]` or `include!` in `file` brings in source from outside the folder, or from a
    /// place the scan cannot know.
    Escapes { file: String, target: String },
    /// A `#[path]` or `include!` in `file` brings in a file of the folder the scan does not read:
    /// not a `.rs` file, under the root `target/` or a hidden folder, or missing.
    NotScanned { file: String, target: String },
    /// The mod is made for exactly one build (a `[native.*]` table or internals): why.
    Exact(String),
    /// More source than is read: a file, or the whole mod.
    TooLarge { what: String, size: u64, most: u64 },
    /// `horizon-mod.toml` is not valid TOML.
    Manifest(String),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, error } => write!(f, "{}: {error}", path.display()),
            Self::Symlink(path) => write!(
                f,
                "{} is a symbolic link; the mod's source cannot be judged",
                path.display()
            ),
            Self::Escapes { file, target } => write!(
                f,
                "{file} brings in `{target}`, outside the mod's folder; the mod's source cannot be judged"
            ),
            Self::NotScanned { file, target } => write!(
                f,
                "{file} brings in `{target}`, which the scan does not read; the mod's source cannot be judged"
            ),
            Self::Exact(why) => write!(
                f,
                "this mod is tied to one build ({why}); fit checks do not apply"
            ),
            Self::TooLarge { what, size, most } => {
                write!(f, "{what} is {size} bytes; the most read is {most}")
            }
            Self::Manifest(error) => write!(f, "horizon-mod.toml: {error}"),
        }
    }
}

impl std::error::Error for ReadError {}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> ReadError + '_ {
    move |error| ReadError::Io {
        path: path.to_path_buf(),
        error: error.to_string(),
    }
}

/// `target` joined to `base` (both relative to the mod's folder), `/`-separated, if it stays
/// inside the folder.
fn inside(base: &[String], target: &str) -> Option<String> {
    if target.starts_with('/') || target.starts_with('\\') || target.contains(':') {
        return None;
    }
    let mut parts: Vec<&str> = base.iter().map(String::as_str).collect();
    for component in target.split(['/', '\\']) {
        match component {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            name => parts.push(name),
        }
    }
    Some(parts.join("/"))
}

/// The text of a string literal token as Rust reads it: `"…"` with its escapes, or a raw string
/// `r"…"` / `r#"…"#`. `None` for any other literal or an escape this does not know.
fn decode(literal: &str) -> Option<String> {
    if let Some(raw) = literal.strip_prefix('r') {
        let hashes = raw.len() - raw.trim_start_matches('#').len();
        let body = raw.get(hashes..raw.len().checked_sub(hashes)?)?;
        return body.strip_prefix('"')?.strip_suffix('"').map(str::to_owned);
    }
    let body = literal.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::new();
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next()? {
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            '0' => out.push('\0'),
            '\\' => out.push('\\'),
            '\'' => out.push('\''),
            '"' => out.push('"'),
            'x' => {
                let code =
                    u8::from_str_radix(&chars.by_ref().take(2).collect::<String>(), 16).ok()?;
                out.push(char::from(code).is_ascii().then_some(char::from(code))?);
            }
            'u' => {
                if chars.next()? != '{' {
                    return None;
                }
                let digits: String = chars.by_ref().take_while(|c| *c != '}').collect();
                out.push(char::from_u32(
                    u32::from_str_radix(&digits.replace('_', ""), 16).ok()?,
                )?);
            }
            '\n' => {
                // A line continuation: the newline and the whitespace after it are skipped.
                let rest: String = chars.collect();
                out.push_str(&decode(&format!("\"{}\"", rest.trim_start()))?);
                return Some(out);
            }
            _ => return None,
        }
    }
    Some(out)
}

/// For each token, whether it sits inside an attribute's brackets, `#[…]` or `#![…]`, at any
/// depth. One pass.
fn in_attributes(list: &[Token<'_>]) -> Vec<bool> {
    let mut inside = Vec::with_capacity(list.len());
    // Each open bracket, and whether it is or lies inside an attribute's.
    let mut open: Vec<bool> = Vec::new();
    for (at, token) in list.iter().enumerate() {
        let outer = open.last().copied().unwrap_or(false);
        match token {
            Token::Punct(b'(' | b'[' | b'{') => {
                let attribute = *token == Token::Punct(b'[')
                    && at >= 1
                    && (list[at - 1] == Token::Punct(b'#')
                        || (at >= 2
                            && list[at - 1] == Token::Punct(b'!')
                            && list[at - 2] == Token::Punct(b'#')));
                inside.push(outer);
                open.push(outer || attribute);
            }
            Token::Punct(b')' | b']' | b'}') => {
                open.pop();
                inside.push(outer);
            }
            _ => inside.push(outer),
        }
    }
    inside
}

/// Every file `text` brings in with `#[path = "…"]` or `include!(…)` (any delimiter), each as a
/// path relative to the mod's folder, given the file's folder. `Err` names the first one that
/// leaves the folder or that the scan cannot read: a computed or undecodable path, an `include!`
/// renamed by a `use` (`use std::include as inc;`), or a
/// `#[path]` inside braces (an inline module, whose files nest by its name).
fn references(text: &str, folder: &[String]) -> Result<Vec<String>, String> {
    let list: Vec<Token<'_>> = tokens(text).map(|(t, _)| t).collect();
    let in_attribute = in_attributes(&list);
    let mut found = Vec::new();
    let mut depth = 0usize;
    let resolve = |at: usize, found: &mut Vec<String>| match list.get(at) {
        Some(Token::Literal(literal)) => match decode(literal) {
            Some(target) => match inside(folder, &target) {
                Some(path) => {
                    found.push(path);
                    Ok(())
                }
                None => Err(target),
            },
            None => Err(format!("{literal}, a path the scan cannot read")),
        },
        _ => Err("a path the scan cannot read".to_owned()),
    };
    for at in 0..list.len() {
        match list[at] {
            Token::Punct(b'{') => depth += 1,
            Token::Punct(b'}') => depth = depth.saturating_sub(1),
            // `path = "…"` anywhere inside an attribute, `cfg_attr(…, path = …)` included.
            Token::Ident("path")
                if list.get(at + 1) == Some(&Token::Punct(b'='))
                    && list.get(at + 2) != Some(&Token::Punct(b'='))
                    && in_attribute[at] =>
            {
                if depth > 0 {
                    return Err("a #[path] inside braces".to_owned());
                }
                resolve(at + 2, &mut found)?;
            }
            // `use std::include as inc;`: an `include!` the scan would not see by its name.
            Token::Ident("include") if list.get(at + 1) == Some(&Token::Ident("as")) => {
                return Err("`include!` under another name".to_owned());
            }
            Token::Ident("include") if list.get(at + 1) == Some(&Token::Punct(b'!')) => {
                let close = match list.get(at + 2) {
                    Some(Token::Punct(b'(')) => b')',
                    Some(Token::Punct(b'[')) => b']',
                    Some(Token::Punct(b'{')) => b'}',
                    _ => continue,
                };
                if list.get(at + 4) != Some(&Token::Punct(close)) {
                    return Err("a computed path".to_owned());
                }
                resolve(at + 3, &mut found)?;
            }
            _ => {}
        }
    }
    Ok(found)
}

/// Reads at most `most` bytes of `path`, refusing a larger file by its size.
fn read_bounded(path: &Path, shown: &str, most: u64) -> Result<String, ReadError> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(most + 1).read_to_end(&mut bytes))
        .map_err(io(path))?;
    if bytes.len() as u64 > most {
        return Err(ReadError::TooLarge {
            what: shown.to_owned(),
            size: bytes.len() as u64,
            most,
        });
    }
    String::from_utf8(bytes).map_err(|e| ReadError::Io {
        path: path.to_path_buf(),
        error: e.to_string(),
    })
}

/// Why a mod whose manifest is `manifest` is made for exactly one build: a `[native.*]` table, or
/// game internals it names; `None` for a mod a fit check applies to.
pub fn exact(manifest: &toml::Table) -> Option<String> {
    let native_table = manifest
        .get("native")
        .and_then(toml::Value::as_table)
        .and_then(|native| {
            native
                .iter()
                .find(|(_, v)| v.is_table() || v.is_array())
                .map(|(k, _)| k)
        });
    if let Some(table) = native_table {
        return Some(format!("it has a [native.{table}] table"));
    }
    manifest
        .get("internals")
        .and_then(toml::Value::as_array)
        .is_some_and(|rows| !rows.is_empty())
        .then(|| "it names game internals".to_owned())
}

/// Reads `dir`: every `.rs` file outside the mod's `target/` and hidden folders, and
/// `horizon-mod.toml` when there is one. A mod made for exactly one build is refused
/// ([`ReadError::Exact`]), and so is a `#[path]` or `include!` of a file the scan does not read.
pub fn read(dir: &Path) -> Result<ModInput, ReadError> {
    let mut input = ModInput::default();
    let mut total = 0u64;
    let mut brought: Vec<(String, String)> = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(folder) = pending.pop() {
        for entry in std::fs::read_dir(&folder).map_err(io(&folder))? {
            let entry = entry.map_err(io(&folder))?;
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let kind = entry.file_type().map_err(io(&path))?;
            if kind.is_symlink() {
                return Err(ReadError::Symlink(path));
            }
            if kind.is_dir() {
                let build_output = folder == dir && name == "target";
                if !name.starts_with('.') && !build_output {
                    pending.push(path);
                }
            } else if kind.is_file() && name.ends_with(".rs") {
                let relative: Vec<String> = path
                    .strip_prefix(dir)
                    .unwrap_or(&path)
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect();
                let shown = relative.join("/");
                let text = read_bounded(&path, &shown, MAX_SOURCE_BYTES)?;
                total += text.len() as u64;
                if total > MAX_TOTAL_BYTES || input.sources.len() >= MAX_FILES {
                    return Err(ReadError::TooLarge {
                        what: format!("the mod's source ({} files)", input.sources.len() + 1),
                        size: total,
                        most: MAX_TOTAL_BYTES,
                    });
                }
                match references(&text, &relative[..relative.len() - 1]) {
                    Ok(paths) => brought.extend(paths.into_iter().map(|p| (shown.clone(), p))),
                    Err(target) => {
                        return Err(ReadError::Escapes {
                            file: shown,
                            target,
                        });
                    }
                }
                input.sources.push((shown, text));
            }
        }
    }
    input.sources.sort();
    for (file, target) in brought {
        if input
            .sources
            .binary_search_by(|(path, _)| path.as_str().cmp(&target))
            .is_err()
        {
            return Err(ReadError::NotScanned { file, target });
        }
    }
    let manifest = dir.join("horizon-mod.toml");
    if manifest.is_file() {
        let text = read_bounded(&manifest, "horizon-mod.toml", MAX_MANIFEST_BYTES)?;
        let value: toml::Table =
            toml::from_str(&text).map_err(|e| ReadError::Manifest(e.to_string()))?;
        if let Some(capabilities) = value.get("capabilities").and_then(toml::Value::as_array) {
            input.manifest_names = capabilities
                .iter()
                .filter_map(toml::Value::as_str)
                .map(str::to_owned)
                .collect();
        }
        if let Some(why) = exact(&value) {
            return Err(ReadError::Exact(why));
        }
    }
    Ok(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, path: &str, text: &str) {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn reads_sources_outside_target_and_hidden_folders_and_the_manifest_names() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "src/lib.rs", "a");
        write(root, "src/deep/b.rs", "b");
        write(root, "build.rs", "c");
        write(root, "target/debug/build/x.rs", "no");
        write(root, "src/target/aim.rs", "d");
        write(root, ".horizon/sdk/api/lib.rs", "no");
        write(root, "src/notes.txt", "no");
        write(
            root,
            "horizon-mod.toml",
            "id = \"m\"\ncapabilities = [\"registries\"]\n[native]\ngame = \"26.3\"\n",
        );
        let input = read(root).unwrap();
        let sources: Vec<(&str, &str)> = input
            .sources
            .iter()
            .map(|(p, t)| (p.as_str(), t.as_str()))
            .collect();
        assert_eq!(
            sources,
            [
                ("build.rs", "c"),
                ("src/deep/b.rs", "b"),
                ("src/lib.rs", "a"),
                ("src/target/aim.rs", "d")
            ]
        );
        assert_eq!(input.manifest_names, ["registries"]);
    }

    #[test]
    fn a_mod_tied_to_one_build_gets_no_table() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "horizon-mod.toml",
            "capabilities = []\n[native]\ngame = \"26.3\"\n[[native.mixin]]\nid = \"x\"\n",
        );
        assert_eq!(
            read(dir.path()).unwrap_err().to_string(),
            "this mod is tied to one build (it has a [native.mixin] table); fit checks do not apply"
        );
        write(
            dir.path(),
            "horizon-mod.toml",
            "capabilities = []\n[[internals]]\npath = \"x\"\n",
        );
        assert_eq!(
            read(dir.path()).unwrap_err(),
            ReadError::Exact("it names game internals".into())
        );
    }

    #[test]
    fn the_manifest_and_each_source_are_read_with_a_cap() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let big = format!(
            "capabilities = []\n# {}\n",
            "x".repeat(MAX_MANIFEST_BYTES as usize)
        );
        write(root, "horizon-mod.toml", &big);
        assert!(matches!(
            read(root).unwrap_err(),
            ReadError::TooLarge { what, most: MAX_MANIFEST_BYTES, .. } if what == "horizon-mod.toml"
        ));
        write(root, "horizon-mod.toml", "capabilities = []\n");
        write(
            root,
            "src/lib.rs",
            &"/".repeat(MAX_SOURCE_BYTES as usize + 1),
        );
        assert!(matches!(
            read(root).unwrap_err(),
            ReadError::TooLarge { what, most: MAX_SOURCE_BYTES, .. } if what == "src/lib.rs"
        ));
    }

    #[test]
    fn an_unreadable_manifest_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "horizon-mod.toml", "capabilities = [");
        assert!(matches!(
            read(dir.path()).unwrap_err(),
            ReadError::Manifest(_)
        ));
    }

    #[test]
    fn source_from_outside_the_folder_cannot_be_judged() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // Inside the folder and scanned: fine, with any delimiter and a decoded or raw literal.
        write(root, "src/gen/x.rs", "");
        write(root, "src/y.rs", "");
        write(
            root,
            "src/lib.rs",
            "#[path = \"gen/x.rs\"] mod x;\ninclude!(\"../src/y.rs\");\ninclude! { r#\"y.rs\"# }\ninclude![\"\\x79.rs\"];\ninclude!(\"\\u{79}.rs\");\n// #[path = \"/etc\"]",
        );
        assert!(read(root).is_ok());
        let escapes = |text: &str| {
            write(root, "src/lib.rs", text);
            read(root).unwrap_err()
        };
        // Escapes decoded before the check, with every delimiter.
        for text in [
            "include!(\"./\\x2e\\x2e/\\x2e\\x2e/shared.rs\");",
            "include!(\"\\u{2e}\\u{2e}/../shared.rs\");",
            "include! { \"../../shared.rs\" }",
            "include![\"../../shared.rs\"];",
            "#[path = r\"../../shared.rs\"] mod x;",
        ] {
            assert!(matches!(escapes(text), ReadError::Escapes { .. }), "{text}");
        }
        // An escape the scan cannot decode cannot be judged.
        assert!(matches!(
            escapes("include!(\"\\q.rs\");"),
            ReadError::Escapes { .. }
        ));
        // A file of the folder the scan does not read cannot be judged.
        write(root, "src/part.inc", "");
        write(root, "target/gen.rs", "");
        write(root, ".hidden/gen.rs", "");
        for (text, target) in [
            ("include!(\"part.inc\");", "src/part.inc"),
            ("include!(\"../target/gen.rs\");", "target/gen.rs"),
            ("#[path = \"../.hidden/gen.rs\"] mod gen;", ".hidden/gen.rs"),
            ("include!(\"missing.rs\");", "src/missing.rs"),
        ] {
            assert_eq!(
                escapes(text),
                ReadError::NotScanned {
                    file: "src/lib.rs".into(),
                    target: target.into()
                }
            );
        }
        // `include!` under another name is not seen by its name: not judged.
        for text in [
            "use std::include as inc;\ninc!(\"y.rs\");",
            "use core::{include as inc};\ninc!(\"../../shared/gen.rs\");",
        ] {
            assert!(matches!(escapes(text), ReadError::Escapes { .. }), "{text}");
        }
        // Many `path = …` statements outside attributes are read in one pass.
        let many = format!("fn f() {{ {} }}", "path = x; ".repeat(400_000));
        let started = std::time::Instant::now();
        write(root, "src/lib.rs", &many);
        assert!(read(root).is_ok());
        assert!(started.elapsed() < std::time::Duration::from_millis(1500));
        // A `#[path]` inside an inline module nests by its name: not judged.
        assert!(matches!(
            escapes("mod outer { #[path = \"y.rs\"] mod inner; }"),
            ReadError::Escapes { .. }
        ));
        assert_eq!(
            escapes("#[path = \"../../other/x.rs\"] mod x;"),
            ReadError::Escapes {
                file: "src/lib.rs".into(),
                target: "../../other/x.rs".into()
            }
        );
        assert!(matches!(
            escapes("#[path = \"/abs/x.rs\"] mod x;"),
            ReadError::Escapes { .. }
        ));
        assert!(matches!(
            escapes("#[path = \"C:\\\\x.rs\"] mod x;"),
            ReadError::Escapes { .. }
        ));
        assert_eq!(
            escapes("#[cfg_attr(all(), path = \"../../elsewhere/x.rs\")] mod x;"),
            ReadError::Escapes {
                file: "src/lib.rs".into(),
                target: "../../elsewhere/x.rs".into()
            }
        );
        assert_eq!(
            escapes("include!(concat!(env!(\"OUT_DIR\"), \"/gen.rs\"));"),
            ReadError::Escapes {
                file: "src/lib.rs".into(),
                target: "a computed path".into()
            }
        );
    }

    #[test]
    fn a_deep_folder_tree_is_read_without_recursion() {
        let dir = tempfile::tempdir().unwrap();
        let deep = "m/".repeat(200);
        write(dir.path(), &format!("src/{deep}x.rs"), "x");
        let input = read(dir.path()).unwrap();
        assert_eq!(input.sources, [(format!("src/{deep}x.rs"), "x".to_owned())]);
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_cannot_be_judged() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "src/lib.rs", "a");
        std::os::unix::fs::symlink(root.join("src"), root.join("linked")).unwrap();
        assert_eq!(
            read(root).unwrap_err(),
            ReadError::Symlink(root.join("linked"))
        );
    }
}
