//! Every `horizon.` id in the game's `en_us.json` is one the code asks for (#13802).
//!
//! A key no code names is text the game no longer shows; its translations stay in every
//! language and new languages translate it too. `horizon-client-core`'s
//! `every_sentence_key_in_en_us_is_one_the_code_names` checks the sentence keys; this checks
//! the ids, which are also built at run time: from a `format!` template
//! (`"horizon.pvp.colour.{}"`) or from the PvP page's option table (`pvp::Opt`).

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Whether `key` fits `template`, whose `{…}` holes each stand for one or more key characters.
fn fits(template: &str, key: &str) -> bool {
    let Some(open) = template.find('{') else {
        return template == key;
    };
    let Some(close) = template[open..].find('}') else {
        return false;
    };
    let (head, tail) = (&template[..open], &template[open + close + 1..]);
    let Some(rest) = key.strip_prefix(head) else {
        return false;
    };
    let is_key_char = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'.';
    (1..=rest.len())
        .take_while(|&n| is_key_char(rest.as_bytes()[n - 1]))
        .any(|n| fits(tail, &rest[n..]))
}

/// The `horizon.` string literals in `source`. A comment is not code: a key only a comment
/// still names is one nothing shows, so it does not count as asked for.
fn named_keys(source: &str) -> Vec<&str> {
    let bytes = source.as_bytes();
    let mut keys = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            // `//` to the end of the line, `///` and `//!` included.
            b'/' if bytes.get(at + 1) == Some(&b'/') => {
                at = source[at..].find('\n').map_or(bytes.len(), |n| at + n + 1);
            }
            // `/* … */`, which nests.
            b'/' if bytes.get(at + 1) == Some(&b'*') => {
                let mut depth = 0;
                while at < bytes.len() {
                    if bytes[at..].starts_with(b"/*") {
                        depth += 1;
                        at += 2;
                    } else if bytes[at..].starts_with(b"*/") {
                        depth -= 1;
                        at += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        at += 1;
                    }
                }
            }
            b'"' => match string_at(source, at) {
                Some((literal, next)) => {
                    if literal.starts_with("horizon.") {
                        keys.push(literal);
                    }
                    at = next;
                }
                None => at = bytes.len(),
            },
            // A char literal, whose `"` is no string start, or a lifetime.
            b'\'' => at = quote_end(source, at),
            b'r' | b'b' => match raw_string_at(source, at) {
                Some((literal, next)) => {
                    if literal.starts_with("horizon.") {
                        keys.push(literal);
                    }
                    at = next;
                }
                None => at += 1,
            },
            _ => at += 1,
        }
    }
    keys
}

/// The string literal whose opening quote is at `open`: its content and the byte after its close.
fn string_at(source: &str, open: usize) -> Option<(&str, usize)> {
    let bytes = source.as_bytes();
    let mut at = open + 1;
    while at < bytes.len() {
        match bytes[at] {
            b'\\' => at += 1 + source[at + 1..].chars().next().map_or(0, char::len_utf8),
            b'"' => return Some((&source[open + 1..at], at + 1)),
            _ => at += 1,
        }
    }
    None
}

/// The raw string starting at `at` (`r"…"`, `r#"…"#`, `br"…"`): its content and the byte after it.
fn raw_string_at(source: &str, at: usize) -> Option<(&str, usize)> {
    let bytes = source.as_bytes();
    if at > 0 && (bytes[at - 1].is_ascii_alphanumeric() || bytes[at - 1] == b'_') {
        return None; // The end of an identifier, not a prefix.
    }
    let mut open = at;
    if bytes[open] == b'b' {
        open += 1;
    }
    if bytes.get(open) != Some(&b'r') {
        return None;
    }
    open += 1;
    let mut hashes = 0;
    while bytes.get(open) == Some(&b'#') {
        hashes += 1;
        open += 1;
    }
    if bytes.get(open) != Some(&b'"') {
        return None;
    }
    let close = format!("\"{}", "#".repeat(hashes));
    let start = open + 1;
    let end = source[start..].find(&close)? + start;
    Some((&source[start..end], end + close.len()))
}

/// The byte after the char literal or lifetime whose opening quote is at `quote`.
fn quote_end(source: &str, quote: usize) -> usize {
    let rest = &source[quote + 1..];
    let mut chars = rest.char_indices();
    match chars.next() {
        // `'\n'`, `'\''`, `'\u{1F600}'`: past the escaped char, to the closing quote.
        Some((_, '\\')) => {
            let escaped = rest[1..].chars().next().map_or(1, char::len_utf8);
            rest[1 + escaped..]
                .find('\'')
                .map_or(source.len(), |n| quote + 2 + escaped + n + 1)
        }
        // `'x'` closes it; a lifetime or label (`'a`) does not.
        Some((start, c)) => {
            let after = quote + 1 + start + c.len_utf8();
            if source[after..].starts_with('\'') {
                after + 1
            } else {
                after
            }
        }
        None => source.len(),
    }
}

#[test]
fn every_horizon_id_in_en_us_is_one_the_code_names() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let en: BTreeMap<String, String> = serde_json::from_str(
        &std::fs::read_to_string(crates.join("horizon-client/assets/horizon/lang/en_us.json"))
            .unwrap(),
    )
    .unwrap();
    // Every crate: a feature moved out of the client crates keeps its strings.
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&crates).unwrap().flatten() {
        rust_files(&entry.path().join("src"), &mut files);
    }
    assert!(files.len() > 100, "found only {} sources", files.len());
    let mut literals: HashSet<String> = HashSet::new();
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        literals.extend(named_keys(&text).into_iter().map(str::to_owned));
    }
    for opt in crate::pvp::Opt::ALL {
        literals.insert(opt.caption_key().to_owned());
        literals.insert(opt.description_key().to_owned());
    }
    let templates: Vec<&String> = literals.iter().filter(|l| l.contains('{')).collect();
    let ids: Vec<&String> = en.keys().filter(|k| k.starts_with("horizon.")).collect();
    assert!(ids.len() > 500, "found only {} ids", ids.len());
    let dead: Vec<&&String> = ids
        .iter()
        .filter(|key| !literals.contains(key.as_str()) && !templates.iter().any(|t| fits(t, key)))
        .collect();
    assert!(
        dead.is_empty(),
        "en_us.json ids no code asks for: {dead:#?}"
    );
}

#[test]
fn a_key_only_a_comment_names_is_not_one_the_code_asks_for() {
    let source = r##"
        let a = "horizon.used";
        let b = format!("horizon.pvp.colour.{}", c);
        // "horizon.line_comment"
        /// "horizon.doc_comment"
        /* "horizon.block" /* "horizon.nested" */ */
        let c = '"';
        let d = b"horizon.bytes";
        let e = r#"horizon.raw"#;
        let f = 'x';
        let g = '\'';
        let h = "horizon.after_char_literals";
    "##;
    assert_eq!(
        named_keys(source),
        [
            "horizon.used",
            "horizon.pvp.colour.{}",
            "horizon.bytes",
            "horizon.raw",
            "horizon.after_char_literals",
        ]
    );
}
