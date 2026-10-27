//! Resource identifiers (`Identifier`).

/// Identifier.parse: `ns:path` or `path`, validated.
pub fn parse_identifier(raw: &str) -> Option<String> {
    let (ns, path) = match raw.split_once(':') {
        Some(("", path)) => ("minecraft", path),
        Some((ns, path)) => (ns, path),
        None => ("minecraft", raw),
    };
    let ns_ok = ns != ".."
        && ns
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.' | '-'));
    let path_ok = path.chars().all(|c| {
        c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.' | '-' | '/')
    });
    (ns_ok && path_ok).then(|| format!("{ns}:{path}"))
}
