//! The API crates' own `macro_rules!` calls at module and impl level, expanded so the items they
//! write are seen like any other. Only the plain forms are transcribed: fragments matched up to
//! the next literal token, and repetitions `$( … ) sep rep` split at their separator. A call
//! that is not expanded is listed in the export's notes, naming the macro and the module.
use crate::index::Index;
use proc_macro2::{Delimiter, Group, Spacing, TokenStream, TokenTree};
use std::collections::{BTreeMap, HashMap};
use syn::Item;

/// Macros whose module-level calls write no item a mod can name, or that the export reads by
/// itself (`include!` through the index, `layout_fields!` through the wire forms).
const KNOWN: &[&str] = &["include", "layout_fields", "thread_local"];

/// One element of a matcher.
#[derive(Clone, Debug)]
enum Matcher {
    /// `$name:fragment`.
    Fragment(String, String),
    /// A token that must appear as written.
    Literal(String),
    /// `$( … ) sep? rep`.
    Repeat(Vec<Matcher>, Option<String>),
}

type Bindings = HashMap<String, Binding>;

#[derive(Clone, Debug)]
enum Binding {
    One(TokenStream),
    Many(Vec<Bindings>),
}

/// A `macro_rules!` definition: its arms.
struct Rules {
    arms: Vec<(Vec<Matcher>, TokenStream)>,
}

fn matcher(tokens: TokenStream) -> Option<Vec<Matcher>> {
    let trees: Vec<TokenTree> = tokens.into_iter().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < trees.len() {
        match &trees[i] {
            TokenTree::Punct(p) if p.as_char() == '$' => match trees.get(i + 1)? {
                TokenTree::Ident(name) => {
                    // `$name : fragment`
                    let TokenTree::Punct(colon) = trees.get(i + 2)? else {
                        return None;
                    };
                    if colon.as_char() != ':' {
                        return None;
                    }
                    let TokenTree::Ident(fragment) = trees.get(i + 3)? else {
                        return None;
                    };
                    out.push(Matcher::Fragment(name.to_string(), fragment.to_string()));
                    i += 4;
                }
                TokenTree::Group(group) if group.delimiter() == Delimiter::Parenthesis => {
                    let inner = matcher(group.stream())?;
                    let mut separator = None;
                    let mut next = i + 2;
                    match trees.get(next)? {
                        TokenTree::Punct(p) if matches!(p.as_char(), '*' | '+' | '?') => {}
                        other => {
                            separator = Some(other.to_string());
                            next += 1;
                        }
                    }
                    match trees.get(next)? {
                        TokenTree::Punct(p) if matches!(p.as_char(), '*' | '+' | '?') => {}
                        _ => return None,
                    }
                    out.push(Matcher::Repeat(inner, separator));
                    i = next + 1;
                }
                _ => return None,
            },
            // A group in a matcher would need nested matching: not a plain form.
            TokenTree::Group(_) => return None,
            other => {
                out.push(Matcher::Literal(other.to_string()));
                i += 1;
            }
        }
    }
    Some(out)
}

fn rules(tokens: TokenStream) -> Option<Rules> {
    let trees: Vec<TokenTree> = tokens.into_iter().collect();
    let mut arms = Vec::new();
    let mut i = 0;
    while i < trees.len() {
        let TokenTree::Group(pattern) = &trees[i] else {
            return None;
        };
        // `=>`
        let (TokenTree::Punct(eq), TokenTree::Punct(gt)) = (trees.get(i + 1)?, trees.get(i + 2)?)
        else {
            return None;
        };
        if eq.as_char() != '=' || gt.as_char() != '>' {
            return None;
        }
        let TokenTree::Group(body) = trees.get(i + 3)? else {
            return None;
        };
        arms.push((matcher(pattern.stream())?, body.stream()));
        i += 4;
        if matches!(trees.get(i), Some(TokenTree::Punct(p)) if p.as_char() == ';') {
            i += 1;
        }
    }
    Some(Rules { arms })
}

/// Splits `trees` at top-level tokens equal to `separator`, dropping an empty last part.
fn split(trees: &[TokenTree], separator: &str) -> Vec<Vec<TokenTree>> {
    let mut parts = vec![Vec::new()];
    for tree in trees {
        if tree.to_string() == separator {
            parts.push(Vec::new());
        } else {
            parts.last_mut().expect("one part").push(tree.clone());
        }
    }
    if parts.last().is_some_and(Vec::is_empty) {
        parts.pop();
    }
    parts
}

/// Matches `input` against `pattern`, binding its fragments.
fn bind(pattern: &[Matcher], input: &[TokenTree]) -> Option<Bindings> {
    let mut bindings = Bindings::new();
    let mut at = 0;
    for (index, element) in pattern.iter().enumerate() {
        // Where this element's input ends: at the next literal of the pattern, or the end.
        let next_literal = pattern[index + 1..].iter().find_map(|m| match m {
            Matcher::Literal(text) => Some(text.clone()),
            _ => None,
        });
        let end = match &next_literal {
            Some(text) => input[at..]
                .iter()
                .position(|t| t.to_string() == *text)
                .map_or(input.len(), |p| at + p),
            None => input.len(),
        };
        match element {
            Matcher::Literal(text) => {
                if input.get(at)?.to_string() != *text {
                    return None;
                }
                at += 1;
            }
            Matcher::Fragment(name, fragment) => {
                let take = if fragment == "tt" { at + 1 } else { end };
                if take <= at || take > input.len() {
                    return None;
                }
                bindings.insert(
                    name.clone(),
                    Binding::One(input[at..take].iter().cloned().collect()),
                );
                at = take;
            }
            Matcher::Repeat(inner, separator) => {
                let has_fragments = inner.iter().any(|m| !matches!(m, Matcher::Literal(_)));
                if !has_fragments {
                    // `$(,)?`: an optional trailing token.
                    if let [Matcher::Literal(text)] = &inner[..]
                        && input.get(at).is_some_and(|t| t.to_string() == *text)
                    {
                        at += 1;
                    }
                    continue;
                }
                let parts = match separator {
                    Some(separator) => split(&input[at..end], separator),
                    None => vec![input[at..end].to_vec()],
                };
                let mut each = Vec::new();
                for part in parts {
                    each.push(bind(inner, &part)?);
                }
                bindings.insert(format!("${index}"), Binding::Many(each));
                at = end;
            }
        }
    }
    (at == input.len()).then_some(bindings)
}

/// The bindings of a repetition that uses `names`.
fn repetitions(bindings: &Bindings, names: &[String]) -> Option<Vec<Bindings>> {
    bindings.values().find_map(|binding| match binding {
        Binding::Many(each)
            if each
                .first()
                .is_some_and(|first| names.iter().any(|n| first.contains_key(n))) =>
        {
            Some(each.clone())
        }
        _ => None,
    })
}

fn names_in(tokens: &TokenStream, out: &mut Vec<String>) {
    let trees: Vec<TokenTree> = tokens.clone().into_iter().collect();
    for (i, tree) in trees.iter().enumerate() {
        match tree {
            TokenTree::Punct(p) if p.as_char() == '$' => {
                if let Some(TokenTree::Ident(name)) = trees.get(i + 1) {
                    out.push(name.to_string());
                }
            }
            TokenTree::Group(group) => names_in(&group.stream(), out),
            _ => {}
        }
    }
}

/// Writes `body` with `bindings`.
fn transcribe(body: TokenStream, bindings: &Bindings) -> Option<TokenStream> {
    let trees: Vec<TokenTree> = body.into_iter().collect();
    let mut out = TokenStream::new();
    let mut i = 0;
    while i < trees.len() {
        match &trees[i] {
            TokenTree::Punct(p) if p.as_char() == '$' => match trees.get(i + 1)? {
                TokenTree::Ident(name) => {
                    match bindings.get(&name.to_string())? {
                        // One token as itself (an identifier stays an identifier); more as one
                        // invisible group, as the compiler substitutes a fragment.
                        Binding::One(tokens) if tokens.clone().into_iter().count() == 1 => {
                            out.extend(tokens.clone());
                        }
                        Binding::One(tokens) => {
                            out.extend([TokenTree::Group(Group::new(
                                Delimiter::None,
                                tokens.clone(),
                            ))]);
                        }
                        Binding::Many(_) => return None,
                    }
                    i += 2;
                }
                TokenTree::Group(group) if group.delimiter() == Delimiter::Parenthesis => {
                    let mut names = Vec::new();
                    names_in(&group.stream(), &mut names);
                    let each = repetitions(bindings, &names)?;
                    let mut next = i + 2;
                    let mut separator = None;
                    match trees.get(next)? {
                        TokenTree::Punct(p) if matches!(p.as_char(), '*' | '+' | '?') => {}
                        other => {
                            separator = Some(other.clone());
                            next += 1;
                        }
                    }
                    for (n, inner) in each.iter().enumerate() {
                        if n > 0
                            && let Some(separator) = &separator
                        {
                            out.extend([separator.clone()]);
                        }
                        let mut scope = bindings.clone();
                        scope.extend(inner.clone());
                        out.extend(transcribe(group.stream(), &scope)?);
                    }
                    i = next + 1;
                }
                _ => return None,
            },
            TokenTree::Group(group) => {
                let inner = transcribe(group.stream(), bindings)?;
                let mut written = Group::new(group.delimiter(), inner);
                written.set_span(group.span());
                out.extend([TokenTree::Group(written)]);
                i += 1;
            }
            TokenTree::Punct(p) if p.spacing() == Spacing::Joint => {
                out.extend([trees[i].clone()]);
                i += 1;
            }
            other => {
                out.extend([other.clone()]);
                i += 1;
            }
        }
    }
    Some(out)
}

fn expand(rules: &Rules, input: TokenStream) -> Option<TokenStream> {
    let input: Vec<TokenTree> = input.into_iter().collect();
    rules.arms.iter().find_map(|(pattern, body)| {
        let bindings = bind(pattern, &input)?;
        transcribe(body.clone(), &bindings)
    })
}

fn last_name(path: &syn::Path) -> String {
    path.segments
        .last()
        .map(|s| s.ident.to_string())
        .unwrap_or_default()
}

/// Expands the module- and impl-level calls of the `macro_rules!` macros the given modules
/// define, in place, and returns one note per call left as it is.
pub(super) fn expand_modules(index: &mut Index<'_>, modules: &[usize]) -> Vec<String> {
    // The definitions, by name; a name defined twice differently is left unexpanded.
    let mut defined: BTreeMap<String, Option<TokenStream>> = BTreeMap::new();
    for &module in modules {
        for item in &index.modules[module].items {
            if let Item::Macro(mac) = item
                && mac.mac.path.is_ident("macro_rules")
                && let Some(name) = &mac.ident
            {
                let tokens = mac.mac.tokens.clone();
                defined
                    .entry(name.to_string())
                    .and_modify(|old| {
                        if old.as_ref().map(ToString::to_string) != Some(tokens.to_string()) {
                            *old = None;
                        }
                    })
                    .or_insert(Some(tokens));
            }
        }
    }
    let rules: HashMap<String, Rules> = defined
        .into_iter()
        .filter_map(|(name, tokens)| Some((name, rules(tokens?)?)))
        .collect();
    let mut notes = Vec::new();
    for &module in modules {
        let place = {
            let m = &index.modules[module];
            std::iter::once(m.krate)
                .chain(m.path.iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join("::")
        };
        let items = std::mem::take(&mut index.modules[module].items);
        let mut out = Vec::with_capacity(items.len());
        expand_items(&rules, &place, items, 0, &mut out, &mut notes);
        index.modules[module].items = out;
    }
    notes
}

/// How deep a macro's expansion may call macros again before the call is noted instead.
const MAX_NESTING: usize = 8;

/// The note for a call left as it is.
fn unexpanded(name: &str, where_: &str, place: &str, depth: usize) -> String {
    let nested = if depth > 0 {
        " (inside another macro's expansion)"
    } else {
        ""
    };
    format!(
        "{name}! called in {where_}{place}{nested}: not expanded, the items it writes are not recorded"
    )
}

/// `items` with their macro calls expanded, and the items those write expanded again, at most
/// [`MAX_NESTING`] deep.
fn expand_items(
    rules: &HashMap<String, Rules>,
    place: &str,
    items: Vec<Item>,
    depth: usize,
    out: &mut Vec<Item>,
    notes: &mut Vec<String>,
) {
    for item in items {
        match item {
            Item::Macro(mac) if mac.ident.is_none() => {
                let name = last_name(&mac.mac.path);
                if KNOWN.contains(&name.as_str()) {
                    out.push(Item::Macro(mac));
                    continue;
                }
                let expanded = (depth < MAX_NESTING)
                    .then(|| rules.get(&name))
                    .flatten()
                    .and_then(|rules| expand(rules, mac.mac.tokens.clone()))
                    .and_then(|tokens| syn::parse2::<syn::File>(tokens).ok());
                match expanded {
                    Some(file) => expand_items(rules, place, file.items, depth + 1, out, notes),
                    None => {
                        notes.push(unexpanded(&name, "", place, depth));
                        out.push(Item::Macro(mac));
                    }
                }
            }
            Item::Impl(mut block) => {
                let members = std::mem::take(&mut block.items);
                expand_members(rules, place, members, depth, &mut block.items, notes);
                out.push(Item::Impl(block));
            }
            item => out.push(item),
        }
    }
}

/// An impl block's `members` with their macro calls expanded, as [`expand_items`] does.
fn expand_members(
    rules: &HashMap<String, Rules>,
    place: &str,
    members: Vec<syn::ImplItem>,
    depth: usize,
    out: &mut Vec<syn::ImplItem>,
    notes: &mut Vec<String>,
) {
    for member in members {
        let syn::ImplItem::Macro(mac) = member else {
            out.push(member);
            continue;
        };
        let name = last_name(&mac.mac.path);
        if KNOWN.contains(&name.as_str()) {
            out.push(syn::ImplItem::Macro(mac));
            continue;
        }
        let expanded = (depth < MAX_NESTING)
            .then(|| rules.get(&name))
            .flatten()
            .and_then(|rules| expand(rules, mac.mac.tokens.clone()))
            .and_then(|tokens| syn::parse2::<ImplItems>(tokens).ok().map(|items| items.0));
        match expanded {
            Some(items) => expand_members(rules, place, items, depth + 1, out, notes),
            None => {
                notes.push(unexpanded(&name, "an impl block in ", place, depth));
                out.push(syn::ImplItem::Macro(mac));
            }
        }
    }
}

/// The items of an impl block's body.
struct ImplItems(Vec<syn::ImplItem>);

impl syn::parse::Parse for ImplItems {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut items = Vec::new();
        while !input.is_empty() {
            items.push(input.parse()?);
        }
        Ok(Self(items))
    }
}
