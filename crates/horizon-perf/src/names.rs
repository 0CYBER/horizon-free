//! Function-name normalization (docs/perf-platform.md §5.2): one name for every monomorphization, closure index and build hash.

/// The name two builds of the same source agree on: no hash, no generic arguments, no closure numbers, `<X as Trait>::f` as `X::f`.
pub fn normalize(name: &str) -> String {
    if (name.starts_with('[') && !name.contains("::")) || !name.contains(['<', '{', ':']) {
        return name.to_string();
    }
    let name = strip_hash(name);
    let name = numbered_braces(&pdb_segments(&name));
    let out = path(&name).unwrap_or(name);
    out.replace("::::", "::").trim_end_matches("::").to_string()
}

/// The last two path segments: `Game::tick_blocks`. A slice keeps its brackets around its type's last segment:
/// `[Team]::clone_into`.
pub fn short(normalized: &str) -> String {
    let parts: Vec<&str> = segments(normalized)
        .into_iter()
        .filter(|p| !p.is_empty())
        .collect();
    match parts.len() {
        0 | 1 => normalized.to_string(),
        n => parts[n - 2..]
            .iter()
            .map(
                |part| match part.strip_prefix('[').and_then(|p| p.strip_suffix(']')) {
                    Some(inner) => format!("[{}]", segments(inner).last().unwrap_or(&inner)),
                    None => (*part).to_string(),
                },
            )
            .collect::<Vec<_>>()
            .join("::"),
    }
}

/// `text` split at each `::` outside brackets, so `[a::T]::f` is two segments.
fn segments(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut start, mut previous) = (0usize, 0, ' ');
    for (i, c) in text.char_indices() {
        match c {
            '[' | '<' => depth += 1,
            ']' => depth = depth.saturating_sub(1),
            '>' if previous != '-' => depth = depth.saturating_sub(1),
            ':' if depth == 0 && i > start && previous == ':' => {
                parts.push(&text[start..i - 1]);
                start = i + 1;
            }
            _ => {}
        }
        previous = c;
    }
    parts.push(&text[start..]);
    parts
}

/// The PDB's `impl$3` and `closure$0` (Windows' inlined and line-tables-only names) as Linux's `{impl#3}` and
/// `{closure#0}`, which [`numbered_braces`] then drops the numbers of.
fn pdb_segments(name: &str) -> String {
    if !name.contains('$') {
        return name.to_string();
    }
    name.split("::")
        .map(|part| match part.split_once('$') {
            Some((word @ ("impl" | "closure"), digits))
                if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) =>
            {
                format!("{{{word}#{digits}}}")
            }
            _ => part.to_string(),
        })
        .collect::<Vec<_>>()
        .join("::")
}

/// `name::h0123456789abcdef`, the legacy symbol hash, without the hash.
fn strip_hash(name: &str) -> String {
    if let Some((head, tail)) = name.rsplit_once("::h")
        && tail.len() == 16
        && tail.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return head.to_string();
    }
    name.to_string()
}

/// `{closure#3}`, `{impl#0}` and `{{closure}}` lose their numbering or doubled braces.
fn numbered_braces(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut rest = name;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        rest = &rest[open..];
        if let Some(inner) = rest.strip_prefix("{{")
            && let Some(end) = inner.find("}}")
            && is_word(&inner[..end])
        {
            out.push('{');
            out.push_str(&inner[..end]);
            out.push('}');
            rest = &inner[end + 2..];
            continue;
        }
        if let Some(end) = rest.find('}')
            && let Some((word, digits)) = rest[1..end].split_once('#')
            && is_word(word)
            && !digits.is_empty()
            && digits.bytes().all(|b| b.is_ascii_digit())
        {
            out.push('{');
            out.push_str(word);
            out.push('}');
            rest = &rest[end + 1..];
            continue;
        }
        out.push('{');
        rest = &rest[1..];
    }
    out.push_str(rest);
    out
}

fn is_word(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// The index of the `>` closing the `<` at the start of `text`, if it closes.
fn close_of(text: &str) -> Option<usize> {
    let mut depth = 0usize;
    let mut previous = ' ';
    for (i, c) in text.char_indices() {
        match c {
            '<' => depth += 1,
            '>' if previous != '-' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        previous = c;
    }
    None
}

/// `<X as Trait>::f` and `<X>::f` as `X::f`, then generic arguments dropped; None when the brackets do not balance.
fn path(name: &str) -> Option<String> {
    if let Some(rest) = name.strip_prefix('<') {
        let end = close_of(name)?;
        let inner = &rest[..end - 1];
        let ty = outer_type(inner);
        let base = path(ty.trim_start_matches(['&', '*']).trim_start_matches("mut "))?;
        let base = base
            .strip_prefix("const ")
            .or_else(|| base.strip_prefix("dyn "))
            .unwrap_or(&base);
        return Some(format!("{base}{}", strip_generics(&name[end + 1..])?));
    }
    strip_generics(name)
}

/// The text before the first top-level ` as `.
fn outer_type(inner: &str) -> &str {
    let mut depth = 0usize;
    let mut previous = ' ';
    for (i, c) in inner.char_indices() {
        match c {
            '<' => depth += 1,
            '>' if previous != '-' => depth = depth.saturating_sub(1),
            ' ' if depth == 0 && inner[i..].starts_with(" as ") => return &inner[..i],
            _ => {}
        }
        previous = c;
    }
    inner
}

fn strip_generics(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0usize;
    let mut previous = ' ';
    for c in text.chars() {
        match c {
            '<' => depth += 1,
            '>' if previous != '-' => depth = depth.checked_sub(1)?,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
        previous = c;
    }
    (depth == 0).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (as symbolized, full normalized name, short name)
    const TABLE: [(&str, &str, &str); 22] = [
        (
            "<horizon_world::level::Sim<horizon_world::level::NoHooks>>::set_block_limit",
            "horizon_world::level::Sim::set_block_limit",
            "Sim::set_block_limit",
        ),
        (
            "<horizon_world::level::Sim<…>>::set_block_limit",
            "horizon_world::level::Sim::set_block_limit",
            "Sim::set_block_limit",
        ),
        (
            "<alloc::vec::Vec<T,A> as core::ops::drop::Drop>::drop",
            "alloc::vec::Vec::drop",
            "Vec::drop",
        ),
        (
            "horizon_client_lib::lan::waypoints::Waypoints::tick_level",
            "horizon_client_lib::lan::waypoints::Waypoints::tick_level",
            "Waypoints::tick_level",
        ),
        (
            "core::ptr::drop_in_place<alloc::vec::Vec<u8>>",
            "core::ptr::drop_in_place",
            "ptr::drop_in_place",
        ),
        (
            "std::collections::hash::map::HashMap<horizon::TickTypeId,u32,S>::get<horizon::TickTypeId>",
            "std::collections::hash::map::HashMap::get",
            "HashMap::get",
        ),
        (
            "horizon_client::game::Game::tick_blocks::{{closure}}",
            "horizon_client::game::Game::tick_blocks::{closure}",
            "tick_blocks::{closure}",
        ),
        (
            "horizon_client::game::Game::tick::{closure#12}",
            "horizon_client::game::Game::tick::{closure}",
            "tick::{closure}",
        ),
        (
            "horizon_world::level::{impl#3}::get",
            "horizon_world::level::{impl}::get",
            "{impl}::get",
        ),
        (
            "horizon_world::foo::bar::h0123456789abcdef",
            "horizon_world::foo::bar",
            "foo::bar",
        ),
        (
            "<&horizon_world::chunk::Chunk as horizon_world::Palette>::get",
            "horizon_world::chunk::Chunk::get",
            "Chunk::get",
        ),
        (
            "<<alloc::sync::Arc<T> as core::ops::deref::Deref>::Target as core::fmt::Debug>::fmt",
            "alloc::sync::Arc::Target::fmt",
            "Target::fmt",
        ),
        (
            "core::iter::adapters::map::map_fold::<T,B,Acc>::{{closure}}",
            "core::iter::adapters::map::map_fold::{closure}",
            "map_fold::{closure}",
        ),
        (
            "<[horizon_client_core::scoreboard::Team]>::clone_into",
            "[horizon_client_core::scoreboard::Team]::clone_into",
            "[Team]::clone_into",
        ),
        (
            "[horizon_client_core::scoreboard::Team]::clone_into",
            "[horizon_client_core::scoreboard::Team]::clone_into",
            "[Team]::clone_into",
        ),
        (
            "[horizon_world::Pos]::sort_by::closure$1",
            "[horizon_world::Pos]::sort_by::{closure}",
            "sort_by::{closure}",
        ),
        (
            "horizon_client_core::who::impl$5::eq",
            "horizon_client_core::who::{impl}::eq",
            "{impl}::eq",
        ),
        (
            "horizon_world::block_collision_scan::impl$2::find_next::closure$0",
            "horizon_world::block_collision_scan::{impl}::find_next::{closure}",
            "find_next::{closure}",
        ),
        ("mi_free", "mi_free", "mi_free"),
        (
            "[unmapped libgallium.so]",
            "[unmapped libgallium.so]",
            "[unmapped libgallium.so]",
        ),
        (
            "horizon_client+0x1a2b3c",
            "horizon_client+0x1a2b3c",
            "horizon_client+0x1a2b3c",
        ),
        (
            "core::ops::function::FnOnce::call_once{{vtable.shim}}",
            "core::ops::function::FnOnce::call_once{{vtable.shim}}",
            "FnOnce::call_once{{vtable.shim}}",
        ),
    ];

    #[test]
    fn real_names_map_to_their_expected_normalized_and_short_forms() {
        for (raw, full, shortened) in TABLE {
            let got = normalize(raw);
            assert_eq!(got, full, "{raw}");
            assert_eq!(short(&got), shortened, "{raw}");
        }
    }

    #[test]
    fn unbalanced_brackets_are_left_as_they_were() {
        assert_eq!(normalize("operator<(a, b)"), "operator<(a, b)");
        assert_eq!(normalize("a::b<c"), "a::b<c");
    }

    #[test]
    fn an_arrow_inside_generics_does_not_close_them() {
        assert_eq!(
            normalize("alloc::boxed::Box<dyn core::ops::function::Fn() -> u8>::call"),
            "alloc::boxed::Box::call"
        );
    }

    #[test]
    fn normalizing_twice_changes_nothing() {
        for (raw, full, _) in TABLE {
            assert_eq!(normalize(full), full, "{raw}");
        }
    }
}
