//! Resource identifiers that can safely name a world's level folders.
/// Whether `namespace` is a resource location namespace (`[a-z0-9_.-]`) that is also one safe
/// folder name: not empty, not `.` or `..`.
pub fn valid_level_namespace(namespace: &str) -> bool {
    !matches!(namespace, "" | "." | "..")
        && namespace
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-'))
}

/// Whether `path` is a resource location path (`[a-z0-9_.-/]`) a level can be stored under:
/// every `/`-separated segment is a safe folder name, so no empty segment (a leading, trailing
/// or doubled `/`) and no `.` or `..`.
pub fn valid_level_path(path: &str) -> bool {
    !path.is_empty() && path.split('/').all(valid_level_namespace)
}

/// Whether `id` (`namespace:path`) names a level whose folder stays inside the world's
/// `dimensions` folder. Vanilla keeps a level's folder under it through
/// `Identifier.resolveAgainst`.
pub fn valid_level_id(id: &str) -> bool {
    id.split_once(':')
        .is_some_and(|(namespace, path)| valid_level_namespace(namespace) && valid_level_path(path))
}
