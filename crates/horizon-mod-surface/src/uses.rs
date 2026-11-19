//! What a mod uses of a surface, and whether it fits another build's surface (design §10.3).
//!
//! [`uses_contained`] scans a mod's Rust source ([`crate::scan`]) and lists the surface records it names,
//! then every record their signatures reference, transitively ([`Surface::closure`]). A record
//! counts when its name occurs as an identifier in the source or the manifest names it, with two
//! narrowings:
//! - a member (a trait method, an inherent item, an associated item, a field, a variant, a hook's
//!   argument) counts only when its owner trait, type or hook is reached from what counts,
//!   through the supertraits of called traits too ([`Surface::reach`]): a sub-host trait's method
//!   called on `&mut dyn ServerHost` counts once `ServerHost` does;
//! - a name written after `X::`, where `X` owns a member of that name, is that member only
//!   (`TextHover::Item` is the variant, not a type `Item`).
//!
//! For each struct the source names it also records how the mod builds it, read with a Rust
//! parser ([`crate::syntax`]): [`Built::Full`] where a struct literal or call lists its fields
//! with no `..` (`Name { … }`, `Name::<T> { … }`, `Name(…)`, `Self { … }` in an impl for it, a
//! local alias or renamed import of it), where a pattern matches it exhaustively (`let Name { a }
//! = …`, `Name(a, b) =>`), or where a tuple or unit struct's name stands as a value (`.map(Pos)`,
//! `Marker`); [`Built::Base`] when every literal ends in a base (and for a union, whose literal
//! names one field); [`Built::None`] otherwise. A pattern ending in `..` is a read. Which struct a
//! name means is decided by two rules, with no module resolution:
//! - a name written with a path (`a::b::Name`) is every API struct of that name;
//! - an unqualified name (or `Self` in an impl for one) is the mod's own type only in a file
//!   that defines it at its top level under no `#[cfg]`, binds it with no `use`, has no glob, no
//!   inline module, no `use` with a metavariable, and only standard or API macros and attributes
//!   whose names nothing in the mod rebinds, in a mod with no `include!`. Otherwise it is every
//!   API struct of that name.
//!
//! Neither rule reaches a variant of an enum the mod defines, which builds no API struct, where
//! one file shows that the path leads to it: `Kind::Settings { … }` or `self::Kind::Settings
//! { … }` in a file the second rule could judge that defines `enum Kind` at its top level
//! under no `#[cfg]`, where nothing else binds `Kind` (no other item, module, extern crate,
//! generic parameter or `use` anywhere in the file, no API item of the name); `Self::Settings
//! { … }` in an impl for such an enum; and a name whose only `use` in such a file is the single
//! leaf `use self::Kind::Settings;` (also `as` another name). Any other `use` that names `Kind`
//! or binds the name (braces, a leading `::`, or a bare `use Kind::Settings;`, which edition
//! 2015 reads from the crate's root) leaves the API's. There is no exemption in a file with a
//! macro called through a path (`std::vec!`), nor anywhere in a mod that defines a macro
//! (`macro_rules!`, whose names reach other files), loads macros with `#[macro_use]`, binds a
//! name of a standard or API macro or the name `std`, `core` or `alloc`, or writes a non-ASCII
//! identifier (Rust compares identifiers after normalising them, the scan does not). The scan
//! does not know which file is a crate's root or where a module's file is, so a path through
//! `crate::`, `super::` or another module, and an enum defined in another file or inside a
//! function, still count as the API struct's; so does any literal inside a macro body.
//!
//! The parser does not expand macros, so what it cannot see is counted as doubt, never as
//! fitting, because a refusal is safe and a wrong "fits" is not:
//! - a macro body's tokens are read as they stand: an API struct name in one followed by `{` or
//!   `(` is a build, in full unless it ends in `..`, and so is a tuple or unit struct's name
//!   where a value starts (`.map(Pos)`); a literal of a name the scan cannot place there (a
//!   metavariable's `$t { … }` or `$t(…)`, a struct of an unknown name) builds every buildable
//!   API struct the mod names in full;
//! - so does a `Self { … }` the scan cannot tie to an impl of a plain path, and a literal of a
//!   `type` alias whose target is not a plain path;
//! - a file holding a macro or attribute that is neither standard, the API's own nor the mod's
//!   own `macro_rules!` (a dependency's macro, an unknown derive or attribute macro), or a `use`
//!   with a metavariable, builds in full every buildable API struct it names, since that
//!   expansion may build it.
//!
//! Names assembled inside a macro from outside the mod (`paste!`, `concat_idents!`) never appear
//! in the source and are out of the scan's reach; the compile catches them. A file that does not
//! parse, or nests deeper than [`syntax::MAX_NESTING`], cannot be judged ([`ScanError`]), and
//! neither can any file where the parser cannot have a stack of its own (on wasm). Even a mod
//! judged to fit is compiled on the player's machine against the target build, and that compile
//! stays the final check. The scan also records the public paths the mod writes each item by:
//! imported, written out, under an API glob (`prelude::Biome` after `use mod_api::*`), or
//! through an API module or glob the mod re-exports (`crate::prelude::Biome` after
//! `pub use mod_api::prelude`, `crate::Biome` after `pub use mod_api::prelude::*`).
//!
//! [`fits`] compares such a list with a target build's surface. The mod fits when every entry
//! exists there with an equal hash and under every path the mod writes, every type keeps the
//! trait impls it had, every struct the mod builds can still be built and keeps its field set
//! when built in full, and every hook whose version the manifest names keeps that version. The
//! per-member hashes make this exact about additions: a new `Host` method or sub-host trait, a
//! new hook, a new hook argument, a new field on a struct the mod does not build in full, or a
//! new trait impl changes nothing for the mod.
//!
//! Whether a mod is exact by nature (a `[native.*]` table, internals) is the caller's to decide
//! from its manifest before asking, and such a mod gets no table: this crate does not read
//! manifests.
//!
//! A [`UsesTable`] is the list a package carries: schema, the build it was made for, and its
//! entries. [`UsesTable::read`] refuses an unknown schema, malformed JSON, repeated ids and
//! documents over [`MAX_USES_BYTES`] with a typed [`UsesError`].
use crate::scan::{self, Token};
use crate::syntax::{self, How, Syntax};
use crate::{MacroKind, Role, Surface, Type, TypeKind, leaf};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// What a mod's manifest says that the list depends on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ManifestFacts {
    /// Capability keys and hooks it names.
    pub names: Vec<String>,
    /// Hooks it names a version of, with that version.
    pub hook_versions: BTreeMap<String, u64>,
}

/// Why an entry is in the list.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Origin {
    /// Its name occurs at this line (1-based) of this source file: the first such place, by
    /// file path then line.
    Source { file: String, line: usize },
    /// The manifest names it: a capability, or a hook by name.
    Manifest,
    /// A signature of this entry references it, directly or through others.
    Via(String),
}

/// How a mod builds a struct it names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Built {
    /// Never with a literal or pattern.
    None,
    /// Only with literals and patterns that end in `..`.
    Base,
    /// With at least one literal or pattern that lists its fields with no `..`.
    Full,
}

/// One record a mod uses, as the table carries it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: String,
    /// Its hash at the made-for build.
    pub hash: String,
    /// A type's trait impls at the made-for build: the target must keep each.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub impls: Vec<String>,
    /// For a struct the source names: how the mod builds it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub built: Option<Built>,
    /// With [`Built::Full`]: the struct's `fields_hash` at the made-for build.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields_hash: Option<String>,
    /// With [`Built::Full`]: its public fields' names at the made-for build, to name what the
    /// target gained.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<String>,
    /// With [`Built::Full`] or [`Built::Base`]: `file:line` of the first such literal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub built_at: Option<String>,
    /// The public paths the mod writes it by, other than its id's: the target must still
    /// export it under each.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    /// For a trait's member: every called trait the mod counts whose supertraits lead to the
    /// member's owner (`trait:ServerHost` for a sub-host trait's method the mod may call on
    /// `&mut dyn ServerHost`), sorted. The target must still lead from each to the owner.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reached_from: Vec<String>,
    /// For a hook whose version the manifest names: that version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u64>,
}

/// An entry and why it is there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Used {
    pub entry: Entry,
    pub origin: Origin,
    /// What `check --uses` tells the author about how the mod builds this struct, when it
    /// builds it with a literal or pattern.
    pub note: Option<String>,
}

/// What a mod uses of one surface, sorted by id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Uses {
    pub items: Vec<Used>,
}

impl Uses {
    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.items.iter().map(|used| &used.entry)
    }

    /// The table a package carries for a mod made for `made_for` (a build id).
    pub fn table(&self, made_for: impl Into<String>) -> UsesTable {
        UsesTable {
            schema: USES_SCHEMA,
            made_for: made_for.into(),
            entries: self.entries().cloned().collect(),
        }
    }
}

/// The owner of a member id: a `trait:`, `type:` or `hook:` id (for an inherent item, whichever
/// the surface has); `None` for a record that is not a member.
fn owner(id: &str, ids: &BTreeMap<&str, &str>) -> Option<String> {
    let (kind, rest) = id.split_once(':')?;
    let owner_path = |rest: &str| rest.rsplit_once("::").map(|(owner, _)| owner.to_owned());
    match kind {
        "method" | "assoc" => owner_path(rest).map(|path| format!("trait:{path}")),
        "handle" => owner_path(rest).map(|path| {
            let as_type = format!("type:{path}");
            if ids.contains_key(as_type.as_str()) {
                as_type
            } else {
                format!("trait:{path}")
            }
        }),
        "type" | "hook" => rest
            .rsplit_once('.')
            .map(|(owner, _)| format!("{kind}:{owner}")),
        _ => None,
    }
}

/// A place in the mod's source: a file's path and a line.
type Place<'t> = (&'t str, usize);

/// What the sources say about how a struct is built.
#[derive(Default)]
struct Building<'t> {
    full: Option<Place<'t>>,
    base: Option<Place<'t>>,
    /// Some full build is a brace expression (`S { a: 1 }`), not a pattern or a call.
    brace_literal: bool,
}

impl<'t> Building<'t> {
    fn record(&mut self, place: Place<'t>, full: bool, brace_literal: bool) {
        let slot = if full { &mut self.full } else { &mut self.base };
        if slot.is_none_or(|old| place < old) {
            *slot = Some(place);
        }
        self.brace_literal |= full && brace_literal;
    }
}

type Tokens<'t> = [(Token<'t>, usize)];

fn punct(tokens: &Tokens<'_>, at: usize, byte: u8) -> bool {
    tokens
        .get(at)
        .is_some_and(|(t, _)| *t == Token::Punct(byte))
}

fn ident<'t>(tokens: &Tokens<'t>, at: usize) -> Option<&'t str> {
    match tokens.get(at) {
        Some((Token::Ident(name), _)) => Some(name),
        _ => None,
    }
}

/// For each token, the first token of the path it ends: `a::b::c` from `c`'s index. One pass.
fn path_starts(tokens: &Tokens<'_>) -> Vec<usize> {
    let mut starts: Vec<usize> = Vec::with_capacity(tokens.len());
    for at in 0..tokens.len() {
        let start = if at >= 3
            && ident(tokens, at).is_some()
            && punct(tokens, at - 1, b':')
            && punct(tokens, at - 2, b':')
            && ident(tokens, at - 3).is_some()
        {
            starts[at - 3]
        } else {
            at
        };
        starts.push(start);
    }
    starts
}

/// For each token that opens a bracket, the index of the token that closes it, if it closes;
/// for braces, whether their contents end in `..` at their own level: a base or a rest (in
/// parentheses `..` may as well be a range, `Pos(0, ..)`, so it counts as nothing there); and
/// for each token, the innermost bracket open around it. One pass.
fn brackets(tokens: &Tokens<'_>) -> (Vec<Option<usize>>, Vec<bool>, Vec<Option<usize>>) {
    let mut closers = vec![None; tokens.len()];
    let mut rests = vec![false; tokens.len()];
    let mut enclosing = vec![None; tokens.len()];
    // Each open bracket with the previous token at its own level.
    let mut open: Vec<(usize, Token<'_>)> = Vec::new();
    for at in 0..tokens.len() {
        let token = tokens[at].0;
        enclosing[at] = open.last().map(|(start, _)| *start);
        match token {
            Token::Punct(b'(' | b'[' | b'{') => {
                open.push((at, token));
                continue;
            }
            Token::Punct(b')' | b']' | b'}') => {
                if let Some((start, _)) = open.pop() {
                    closers[start] = Some(at);
                }
            }
            Token::Punct(b'.') => {
                if let Some((start, previous)) = open.last().copied()
                    && punct(tokens, at + 1, b'.')
                    && matches!(previous, Token::Punct(b'{' | b','))
                    && punct(tokens, start, b'{')
                {
                    rests[start] = true;
                }
            }
            _ => {}
        }
        if let Some(last) = open.last_mut() {
            last.1 = token;
        }
    }
    (closers, rests, enclosing)
}

/// Whether the brace at `open` holds what a struct literal or pattern holds: nothing, `..`, or a
/// field name followed by `:`, `,` or `}`.
fn looks_like_fields(tokens: &Tokens<'_>, open: usize) -> bool {
    match tokens.get(open + 1).map(|(t, _)| *t) {
        Some(Token::Punct(b'}')) => true,
        Some(Token::Punct(b'.')) => punct(tokens, open + 2, b'.'),
        Some(Token::Ident(_)) => {
            punct(tokens, open + 2, b',')
                || punct(tokens, open + 2, b'}')
                || (punct(tokens, open + 2, b':') && !punct(tokens, open + 3, b':'))
        }
        _ => false,
    }
}

/// Whether the name at `at` stands where a type does, not a value: after `->`, `impl`, `for`,
/// `dyn`, `<`, a definition keyword or a keyword whose block follows it, path segments before it
/// skipped.
fn in_type_position(tokens: &Tokens<'_>, path_starts: &[usize], at: usize) -> bool {
    let at = path_starts.get(at).copied().unwrap_or(at);
    match at.checked_sub(1).map(|i| tokens[i].0) {
        Some(Token::Ident(
            "impl" | "for" | "dyn" | "struct" | "enum" | "union" | "trait" | "type" | "as" | "mod"
            | "fn" | "where" | "if" | "match" | "while" | "in",
        )) => true,
        Some(Token::Punct(b'>')) => at >= 2 && punct(tokens, at - 2, b'-'),
        Some(Token::Punct(b'<')) => true,
        _ => false,
    }
}

/// One `use` tree from `at`: each path it imports with the name it is imported as. Returns the
/// index after the tree.
fn use_tree<'t>(
    tokens: &Tokens<'t>,
    mut at: usize,
    end: usize,
    prefix: &mut Vec<&'t str>,
    out: &mut Vec<(Vec<&'t str>, &'t str)>,
) -> usize {
    let depth = prefix.len();
    while at < end {
        match tokens[at].0 {
            Token::Ident(name) if name != "as" => {
                prefix.push(name);
                at += 1;
                if punct(tokens, at, b':') && punct(tokens, at + 1, b':') {
                    at += 2;
                    continue;
                }
                let mut alias = name;
                if ident(tokens, at) == Some("as") {
                    if let Some(local) = ident(tokens, at + 1) {
                        alias = local;
                    }
                    at += 2;
                }
                out.push((prefix.clone(), alias));
                break;
            }
            Token::Punct(b'{') => {
                at += 1;
                while at < end && !punct(tokens, at, b'}') {
                    at = use_tree(tokens, at, end, prefix, out);
                    if punct(tokens, at, b',') {
                        at += 1;
                    }
                }
                at += 1;
                break;
            }
            Token::Punct(b'*') => {
                let mut glob = prefix.clone();
                glob.push("*");
                out.push((glob, "*"));
                at += 1;
                break;
            }
            _ => {
                at += 1;
                break;
            }
        }
    }
    prefix.truncate(depth);
    at
}

/// What one source file holds that names resolve through.
struct FileFacts<'t> {
    tokens: Vec<(Token<'t>, usize)>,
    /// Each token's first byte.
    starts: Vec<usize>,
    /// Whether each token is in a `use` statement.
    in_use: Vec<bool>,
    /// Each token's path start ([`path_starts`]).
    path_starts: Vec<usize>,
    /// Each opening bracket's closer and whether it ends in `..` ([`brackets`]).
    closers: Vec<Option<usize>>,
    rests: Vec<bool>,
    /// Each token's innermost open bracket.
    enclosing: Vec<Option<usize>>,
    /// The parser's impls ordered by where their bodies start, each with the innermost impl
    /// whose body holds it.
    impls: Vec<(usize, Option<usize>)>,
    /// The names some `use` in the file binds.
    imported: BTreeSet<&'t str>,
    /// Every name written in a `use` other than a single leaf `use self::A::B;` or
    /// `use self::A::B as C;`: one with braces, a leading `::` (which the imports record
    /// without), or another start.
    odd_uses: BTreeSet<&'t str>,
    /// What the parser reads of the file.
    syntax: Syntax<'t>,
    /// Tokens inside a macro call's delimiters or a `macro_rules!` body, which the parser does
    /// not see into.
    in_macro: Vec<bool>,
    /// Every `use` path with the name it brings in (`*` for a glob), macro bodies included.
    imports: Vec<(Vec<&'t str>, &'t str)>,
    /// Every path written out in macro bodies, `a::b::C`.
    chains: Vec<Vec<&'t str>>,
    /// `type X = …Y<…>;`: (X, Y), and whether it is an item of its own (not an associated type
    /// of an impl), from the parser and from macro bodies.
    type_aliases: Vec<(&'t str, &'t str, bool)>,
    /// Structs, enums, unions and variants written in macro bodies.
    macro_types: BTreeSet<&'t str>,
    /// Some `use` holds a macro metavariable (`use mod_api::$m;`): what it binds is unknown.
    use_metavariable: bool,
    /// Every macro call anywhere in the file, function bodies and macro bodies included: the
    /// macro's path, or `["$"]` for one named by a metavariable.
    macro_calls: Vec<Vec<&'t str>>,
    /// Every attribute anywhere in the file, `#[…]` or `#![…]`: the token range inside its
    /// brackets.
    attributes: Vec<(usize, usize)>,
    /// The name of each `macro_rules!` definition.
    macro_rules: Vec<&'t str>,
}

/// The deepest nesting of `use` groups the scan reads: `use a::{b::{c}}` nests 2.
pub const MAX_USE_NESTING: usize = 64;

/// Why a mod's source cannot be judged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScanError {
    /// A `use` statement nests its groups deeper than [`MAX_USE_NESTING`], at this line of
    /// this file.
    UseTooDeep { file: String, line: usize },
    /// The file does not parse as Rust, at this line.
    Parse { file: String, line: usize },
    /// The file nests brackets, generics or expressions deeper than the scan parses
    /// ([`syntax::MAX_NESTING`]), at this line.
    TooDeep { file: String, line: usize },
    /// The parser's own stack could not be had (always so on wasm), so the file was not parsed.
    NoStack { file: String },
    /// The child process that parses first ([`crate::child`]) died, by a signal, an overflow or
    /// a limit on its memory or processor time, while parsing this file (none: before it
    /// began).
    ParserDied { file: Option<String> },
    /// The child process was still parsing this file (none: had not begun) when
    /// [`crate::child::CHILD_WAIT`] ran out.
    ParserTimedOut { file: Option<String> },
    /// The child process could not be started, or the program started is not a scan's child:
    /// the program cannot run itself here.
    NoChild,
}

impl std::fmt::Display for ScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UseTooDeep { file, line } => write!(
                f,
                "{file}:{line}: a `use` statement nests its groups deeper than {MAX_USE_NESTING}; the mod's source cannot be judged"
            ),
            Self::TooDeep { file, line } => write!(
                f,
                "{file}:{line}: the source nests deeper than {} levels; the mod's source cannot be judged",
                syntax::MAX_NESTING
            ),
            Self::NoStack { file } => write!(
                f,
                "{file}: the scan could not start its parser; the mod's source cannot be judged"
            ),
            Self::Parse { file, line } => write!(
                f,
                "{file}:{line}: the file does not parse as Rust; the mod's source cannot be judged"
            ),
            Self::ParserDied { file } => write!(
                f,
                "{}the scan's parser stopped (out of stack, memory or processor time); the mod's source cannot be judged",
                file.as_ref()
                    .map_or(String::new(), |file| format!("{file}: "))
            ),
            Self::ParserTimedOut { file } => write!(
                f,
                "{}the scan's parser took longer than {} s; the mod's source cannot be judged",
                file.as_ref()
                    .map_or(String::new(), |file| format!("{file}: ")),
                crate::child::CHILD_WAIT.as_secs()
            ),
            Self::NoChild => write!(
                f,
                "the scan could not start its parser process; the mod's source cannot be judged"
            ),
        }
    }
}

impl std::error::Error for ScanError {}

/// The `use` statements a file's tokens hold: each import, whether each token is in one, and
/// whether one holds a macro metavariable.
type UseStatements<'t> = (Vec<(Vec<&'t str>, &'t str)>, Vec<bool>, bool);

/// A file's tokens, each with its line and its first byte, and its `use` statements; or why it
/// cannot be judged before it is parsed.
fn checked_tokens<'t>(
    file: &str,
    text: &'t str,
) -> Result<(Vec<(Token<'t>, usize)>, Vec<usize>, UseStatements<'t>), ScanError> {
    let mut stream = scan::tokens(text);
    let (mut tokens, mut starts) = (Vec::new(), Vec::new());
    while let Some((token, line, start)) = stream.spanned() {
        tokens.push((token, line));
        starts.push(start);
    }
    let statements = use_statements(file, &tokens)?;
    if let Some(line) = syntax::too_deep(&tokens) {
        return Err(ScanError::TooDeep {
            file: file.to_owned(),
            line,
        });
    }
    Ok((tokens, starts, statements))
}

/// A file's facts, or why it cannot be judged.
fn file_facts<'t>(file: &str, text: &'t str) -> Result<FileFacts<'t>, ScanError> {
    let (tokens, starts, (imports, in_use, use_metavariable)) = checked_tokens(file, text)?;
    let syntax = syntax::read(text).map_err(|unread| match unread {
        syntax::Unread::Parse(line) => ScanError::Parse {
            file: file.to_owned(),
            line,
        },
        syntax::Unread::NoStack => ScanError::NoStack {
            file: file.to_owned(),
        },
    })?;
    let in_macro = in_macro_bodies(&starts, &syntax);
    let path_starts = path_starts(&tokens);
    let (closers, rests, enclosing) = brackets(&tokens);
    let impls = impl_order(&syntax);
    let (chains, macro_types, mut type_aliases) =
        macro_bodies(&tokens, &in_macro, &in_use, &path_starts);
    type_aliases.splice(0..0, syntax.type_aliases.iter().copied());
    let (macro_calls, attributes, macro_rules) = calls(&tokens, &path_starts, &closers);
    let imported = imports
        .iter()
        .filter(|(_, local)| *local != "*")
        .map(|(_, local)| *local)
        .collect();
    let mut odd_uses = BTreeSet::new();
    for at in 0..tokens.len() {
        if ident(&tokens, at) != Some("use") {
            continue;
        }
        let end = (at..tokens.len())
            .find(|i| punct(&tokens, *i, b';'))
            .unwrap_or(tokens.len());
        let leaf = |i: usize| ident(&tokens, i).is_some_and(|name| name != "self");
        let colons = |i: usize| punct(&tokens, i, b':') && punct(&tokens, i + 1, b':');
        let single = ident(&tokens, at + 1) == Some("self")
            && colons(at + 2)
            && leaf(at + 4)
            && colons(at + 5)
            && leaf(at + 7)
            && (end == at + 8
                || (end == at + 10 && ident(&tokens, at + 8) == Some("as") && leaf(at + 9)));
        if !single {
            odd_uses.extend((at + 1..end).filter_map(|i| ident(&tokens, i)));
        }
    }
    Ok(FileFacts {
        tokens,
        starts,
        in_use,
        path_starts,
        closers,
        rests,
        enclosing,
        impls,
        imported,
        odd_uses,
        syntax,
        in_macro,
        imports,
        chains,
        type_aliases,
        macro_types,
        use_metavariable,
        macro_calls,
        attributes,
        macro_rules,
    })
}

/// Every `use` statement's imports, which tokens lie in one, and whether one holds a
/// metavariable; a `use` nested deeper than [`MAX_USE_NESTING`] cannot be judged.
#[allow(clippy::type_complexity)]
fn use_statements<'t>(file: &str, tokens: &Tokens<'t>) -> Result<UseStatements<'t>, ScanError> {
    let mut in_use = vec![false; tokens.len()];
    let mut imports = Vec::new();
    let mut use_metavariable = false;
    let mut at = 0;
    while at < tokens.len() {
        if tokens[at].0 == Token::Ident("use") {
            let end = (at..tokens.len())
                .find(|i| punct(tokens, *i, b';'))
                .unwrap_or(tokens.len());
            in_use[at..end].iter_mut().for_each(|t| *t = true);
            use_metavariable |= tokens[at..end]
                .iter()
                .any(|(token, _)| *token == Token::Punct(b'$'));
            // [`use_tree`] recurses once per group: bound it before it reads one.
            let mut nesting = 0usize;
            for (token, _) in &tokens[at..end] {
                match token {
                    Token::Punct(b'{') => nesting += 1,
                    Token::Punct(b'}') => nesting = nesting.saturating_sub(1),
                    _ => {}
                }
                if nesting > MAX_USE_NESTING {
                    return Err(ScanError::UseTooDeep {
                        file: file.to_owned(),
                        line: tokens[at].1,
                    });
                }
            }
            let mut start = at + 1;
            while start < end {
                if punct(tokens, start, b':') {
                    start += 2;
                }
                start = use_tree(tokens, start, end, &mut Vec::new(), &mut imports);
            }
            // `a::b::{self}` binds `b`, and `{self as x}` binds `x`, both to `a::b`.
            for (path, local) in imports.iter_mut() {
                if path.len() >= 2 && path.last() == Some(&"self") {
                    path.pop();
                    if *local == "self" {
                        *local = path[path.len() - 1];
                    }
                }
            }
            at = end;
        }
        at += 1;
    }
    Ok((imports, in_use, use_metavariable))
}

/// Which tokens lie inside a macro body: one sweep over the bodies by where they start.
fn in_macro_bodies(starts: &[usize], syntax: &Syntax<'_>) -> Vec<bool> {
    let mut bodies: Vec<&std::ops::Range<usize>> = syntax.macro_bodies.iter().collect();
    bodies.sort_by_key(|body| body.start);
    let (mut next, mut reach) = (0, 0);
    starts
        .iter()
        .map(|start| {
            while next < bodies.len() && bodies[next].start < *start {
                reach = reach.max(bodies[next].end);
                next += 1;
            }
            *start + 1 < reach
        })
        .collect()
}

/// The parser's impls by where their bodies start, each with its innermost enclosing impl.
fn impl_order(syntax: &Syntax<'_>) -> Vec<(usize, Option<usize>)> {
    let mut order: Vec<usize> = (0..syntax.impls.len()).collect();
    order.sort_by_key(|i| syntax.impls[*i].body.start);
    let mut impls: Vec<(usize, Option<usize>)> = Vec::with_capacity(order.len());
    let mut open: Vec<usize> = Vec::new();
    for (position, index) in order.iter().enumerate() {
        let body = &syntax.impls[*index].body;
        while open
            .last()
            .is_some_and(|outer| syntax.impls[impls[*outer].0].body.end <= body.start)
        {
            open.pop();
        }
        impls.push((*index, open.last().copied()));
        open.push(position);
    }
    impls
}

/// What macro bodies write, which the parser leaves as tokens: paths, the structs, enums and
/// unions they define, and `type` aliases.
#[allow(clippy::type_complexity)]
fn macro_bodies<'t>(
    tokens: &Tokens<'t>,
    in_macro: &[bool],
    in_use: &[bool],
    path_starts: &[usize],
) -> (
    Vec<Vec<&'t str>>,
    BTreeSet<&'t str>,
    Vec<(&'t str, &'t str, bool)>,
) {
    // The next `;` and `=` at or after each token, for reading `type` items in macro bodies.
    let mut next_semi = vec![tokens.len(); tokens.len() + 1];
    let mut next_eq = vec![tokens.len(); tokens.len() + 1];
    for at in (0..tokens.len()).rev() {
        next_semi[at] = if punct(tokens, at, b';') {
            at
        } else {
            next_semi[at + 1]
        };
        next_eq[at] = if punct(tokens, at, b'=') {
            at
        } else {
            next_eq[at + 1]
        };
    }
    // What macro bodies write, which the parser leaves as tokens: paths, type aliases and
    // definitions.
    let mut chains = Vec::new();
    let mut type_aliases = Vec::new();
    let mut macro_types = BTreeSet::new();
    for at in (0..tokens.len()).filter(|at| in_macro[*at]) {
        match tokens[at].0 {
            Token::Ident("type") => {
                // `type X = path::Y<…>;`: X names Y. A target that is not a plain path (a
                // tuple, a macro's output) is not recorded, so X stays a name the scan cannot
                // place.
                let eq = next_eq[at];
                if let Some(alias) = ident(tokens, at + 1)
                    && eq < next_semi[at]
                {
                    let mut last = eq + 1;
                    if punct(tokens, last, b':') && punct(tokens, last + 1, b':') {
                        last += 2;
                    }
                    while ident(tokens, last).is_some()
                        && punct(tokens, last + 1, b':')
                        && punct(tokens, last + 2, b':')
                    {
                        last += 3;
                    }
                    if let Some(target) = ident(tokens, last)
                        && !punct(tokens, last + 1, b'!')
                        && !punct(tokens, last + 1, b'(')
                    {
                        type_aliases.push((alias, target, true));
                    }
                }
            }
            Token::Ident("struct" | "enum" | "union") => {
                if let Some(name) = ident(tokens, at + 1) {
                    macro_types.insert(name);
                }
            }
            Token::Ident(first)
                if !in_use[at]
                    && punct(tokens, at + 1, b':')
                    && punct(tokens, at + 2, b':')
                    && path_starts[at] == at =>
            {
                // A path written out: `a::b::C`.
                let mut path = vec![first];
                let mut i = at + 1;
                while punct(tokens, i, b':') && punct(tokens, i + 1, b':') {
                    let Some(name) = ident(tokens, i + 2) else {
                        break;
                    };
                    path.push(name);
                    i += 3;
                }
                chains.push(path);
            }
            _ => {}
        }
    }
    (chains, macro_types, type_aliases)
}

/// Every macro call (its path, or `["$"]` for one named by a metavariable), every attribute's
/// token range, and every `macro_rules!` name.
#[allow(clippy::type_complexity)]
fn calls<'t>(
    tokens: &Tokens<'t>,
    path_starts: &[usize],
    closers: &[Option<usize>],
) -> (Vec<Vec<&'t str>>, Vec<(usize, usize)>, Vec<&'t str>) {
    let mut macro_calls = Vec::new();
    let mut attributes = Vec::new();
    let mut macro_rules = Vec::new();
    for at in 0..tokens.len() {
        match tokens[at].0 {
            // `name!(…)`, or `macro_rules! name`; not `if !x` or `return !(…)`.
            Token::Punct(b'!')
                if at > 0
                    && ident(tokens, at - 1)
                        .is_some_and(|name| !syntax::KEYWORDS.contains(&name))
                    && match tokens.get(at + 1).map(|(t, _)| *t) {
                        Some(Token::Punct(b'(' | b'[' | b'{')) => true,
                        Some(Token::Ident(_)) => ident(tokens, at - 1) == Some("macro_rules"),
                        _ => false,
                    } =>
            {
                let first = path_starts[at - 1];
                if first >= 1 && punct(tokens, first - 1, b'$') {
                    macro_calls.push(vec!["$"]);
                    continue;
                }
                let path: Vec<&str> = (first..at)
                    .step_by(3)
                    .filter_map(|i| ident(tokens, i))
                    .collect();
                if path == ["macro_rules"]
                    && let Some(name) = ident(tokens, at + 1)
                {
                    macro_rules.push(name);
                }
                macro_calls.push(path);
            }
            Token::Punct(b'#') => {
                let open = if punct(tokens, at + 1, b'!') {
                    at + 2
                } else {
                    at + 1
                };
                if punct(tokens, open, b'[') {
                    attributes.push((open, closers[open].unwrap_or(tokens.len())));
                }
            }
            _ => {}
        }
    }
    (macro_calls, attributes, macro_rules)
}

/// What the whole mod says that each name resolves through.
struct ModFacts<'t> {
    /// The names the API crate is known by: `mod_api`, and any `use mod_api as x` or
    /// `extern crate mod_api as x`.
    roots: BTreeSet<&'t str>,
    /// Every local name some file gives another name, with each name it gives: any
    /// `use … as Y` (`Y` names the path's last segment) and any `type Y = …X<…>;`.
    aliases: BTreeMap<&'t str, BTreeSet<&'t str>>,
    /// API paths some file binds a name to, `mod_api::` left out, with each path: every `use`
    /// of an API path that is not a glob (`use mod_api::prelude as p;` binds `p` to
    /// `prelude`), and those of `use` paths that start at such a name, followed.
    bound: BTreeMap<&'t str, BTreeSet<Vec<&'t str>>>,
    /// The structs, enums, unions and enum variants the mod defines.
    types: BTreeSet<&'t str>,
    /// The type aliases the mod defines as items of their own.
    item_aliases: BTreeSet<&'t str>,
    /// Every other name the mod defines: traits, associated types.
    others: BTreeSet<&'t str>,
    /// Names some `macro_rules!` or some `use` that does not start at an API root binds: a
    /// standard or API macro or attribute of that name may be another.
    rebound: BTreeSet<&'t str>,
    /// Some glob outside the API and the mod (`use dep::*;`) or a `#[macro_use]` may bring in
    /// macros of any name.
    rebound_all: bool,
    /// The mod's own `macro_rules!` names that no `use` binds to something else: the scan reads
    /// their bodies.
    own_macros: BTreeSet<&'t str>,
    /// The modules the mod declares (`mod api;`): a path that starts at one is the mod's own,
    /// like one after `crate::`.
    modules: BTreeSet<&'t str>,
}

fn mod_facts<'t>(files: &[FileFacts<'t>], api_prefixes: &BTreeSet<String>) -> ModFacts<'t> {
    // Only a path the API has is bound, which keeps each name's bindings few.
    let known = |path: &[&str]| api_prefixes.contains(&path.join("::"));
    let mut roots = BTreeSet::from(["mod_api"]);
    for file in files {
        for (path, name) in &file.imports {
            if path[..] == ["mod_api"] {
                roots.insert(*name);
            }
        }
        for at in 0..file.tokens.len() {
            if ident(&file.tokens, at) == Some("extern")
                && ident(&file.tokens, at + 2) == Some("mod_api")
                && ident(&file.tokens, at + 3) == Some("as")
                && let Some(name) = ident(&file.tokens, at + 4)
            {
                roots.insert(name);
            }
        }
    }
    let mut facts = ModFacts {
        roots,
        aliases: BTreeMap::new(),
        bound: BTreeMap::new(),
        types: BTreeSet::new(),
        item_aliases: BTreeSet::new(),
        others: BTreeSet::new(),
        rebound: BTreeSet::new(),
        rebound_all: false,
        own_macros: BTreeSet::new(),
        modules: files
            .iter()
            .flat_map(|file| file.syntax.modules.iter().copied())
            .collect(),
    };
    let mut imported = BTreeSet::new();
    for file in files {
        facts.rebound.extend(file.macro_rules.iter().copied());
        facts.rebound_all |= file
            .attributes
            .iter()
            .any(|(open, _)| ident(&file.tokens, open + 1) == Some("macro_use"));
        for (path, name) in &file.imports {
            let first = path[0];
            let api = facts.roots.contains(first);
            if *name == "*" {
                // A glob of one of the mod's own modules is like `crate::module::*`: what that
                // module binds is read in its own file.
                facts.rebound_all |= !api
                    && !matches!(first, "crate" | "self" | "super")
                    && !facts.modules.contains(first);
                continue;
            }
            if !api {
                facts.rebound.insert(name);
                imported.insert(*name);
            }
            let last = path[path.len() - 1];
            if name != &last {
                facts.aliases.entry(name).or_default().insert(last);
            }
            if api && path.len() >= 2 && known(&path[1..]) {
                facts
                    .bound
                    .entry(name)
                    .or_default()
                    .insert(path[1..].to_vec());
            }
        }
        for (alias, target, item) in &file.type_aliases {
            facts.aliases.entry(alias).or_default().insert(target);
            if *item {
                facts.item_aliases.insert(alias);
            } else {
                facts.others.insert(alias);
            }
        }
        facts.types.extend(file.syntax.types.iter().copied());
        facts.types.extend(file.macro_types.iter().copied());
        facts.others.extend(file.syntax.traits.iter().copied());
    }
    // `use` paths that start at a name bound to an API path: `use p::Biome;` after
    // `use mod_api::prelude as p;`. Followed a bounded number of times.
    for _ in 0..4 {
        let mut more: Vec<(&str, Vec<&str>)> = Vec::new();
        for file in files {
            for (path, name) in &file.imports {
                if *name == "*" || path.len() < 2 {
                    continue;
                }
                for prefix in facts.bound.get(path[0]).into_iter().flatten() {
                    let mut full = prefix.clone();
                    full.extend(&path[1..]);
                    if known(&full)
                        && !facts
                            .bound
                            .get(name)
                            .is_some_and(|bound| bound.contains(&full))
                    {
                        more.push((name, full));
                    }
                }
            }
        }
        if more.is_empty() {
            break;
        }
        for (name, path) in more {
            facts.bound.entry(name).or_default().insert(path);
        }
    }
    facts.own_macros = files
        .iter()
        .flat_map(|file| file.macro_rules.iter().copied())
        .filter(|name| !imported.contains(name))
        .collect();
    facts
}

/// Each API path (`mod_api::` left out) a path written in the mod may be: itself after an API
/// root, or after a name bound to an API path.
fn api_paths_of<'t>(path: &[&'t str], module: &ModFacts<'t>) -> Vec<Vec<&'t str>> {
    let Some((first, rest)) = path.split_first() else {
        return Vec::new();
    };
    if module.roots.contains(first) {
        return vec![rest.to_vec()];
    }
    module
        .bound
        .get(first)
        .into_iter()
        .flatten()
        .map(|prefix| {
            let mut full = prefix.clone();
            full.extend(rest);
            full
        })
        .collect()
}

/// The API modules `file` imports with a glob, `mod_api::` left out.
fn api_globs<'t>(file: &FileFacts<'t>, module: &ModFacts<'t>) -> BTreeSet<Vec<&'t str>> {
    file.imports
        .iter()
        .filter(|(_, local)| *local == "*")
        .flat_map(|(segments, _)| api_paths_of(&segments[..segments.len() - 1], module))
        .collect()
}

/// The API names `name` may stand for: itself when it is one, and every name the mod's aliases
/// lead it to, followed transitively in any file.
fn api_names_of<'t, 'k>(
    name: &'t str,
    module: &ModFacts<'t>,
    by_name: &BTreeMap<&'k str, Vec<&'k str>>,
) -> BTreeSet<&'k str> {
    let mut out = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut pending = vec![name];
    while let Some(next) = pending.pop() {
        if !seen.insert(next) {
            continue;
        }
        if let Some((key, _)) = by_name.get_key_value(next) {
            out.insert(*key);
        }
        pending.extend(module.aliases.get(next).into_iter().flatten().copied());
    }
    out
}

/// Standard macros that cannot bring a name into scope: they expand to an expression, a string
/// or an error, never to a `use` or a `mod`.
const STD_MACROS: &[&str] = &[
    "include_str",
    "include_bytes",
    "compile_error",
    "concat",
    "env",
    "option_env",
    "stringify",
    "line",
    "column",
    "file",
    "module_path",
    "cfg",
    "assert",
    "assert_eq",
    "assert_ne",
    "debug_assert",
    "debug_assert_eq",
    "debug_assert_ne",
    "format",
    "format_args",
    "vec",
    "matches",
    "panic",
    "unreachable",
    "todo",
    "unimplemented",
    "print",
    "println",
    "eprint",
    "eprintln",
    "write",
    "writeln",
    "dbg",
];

/// Attributes the compiler itself reads: none of them expands to items.
const BUILTIN_ATTRIBUTES: &[&str] = &[
    "derive",
    "cfg",
    "cfg_attr",
    "test",
    "allow",
    "warn",
    "deny",
    "forbid",
    "expect",
    "doc",
    "inline",
    "must_use",
    "repr",
    "non_exhaustive",
    "path",
    "deprecated",
    "cold",
    "no_mangle",
    "export_name",
    "link_section",
    "used",
    "track_caller",
    "ignore",
    "should_panic",
    "macro_export",
    "macro_use",
    "automatically_derived",
    "rustfmt",
    "clippy",
    "diagnostic",
    "unsafe",
    "target_feature",
    "link",
];

/// The derives the standard library defines.
const STD_DERIVES: &[&str] = &[
    "Debug",
    "Clone",
    "Copy",
    "PartialEq",
    "Eq",
    "PartialOrd",
    "Ord",
    "Hash",
    "Default",
];

/// The macro, attribute and derive names a file may use and still be read as written: the
/// standard ones and the API's own, less every name something in the mod may rebind.
struct Trusted<'s> {
    roots: &'s BTreeSet<&'s str>,
    /// By kind, the API's own macro names.
    api_macros: BTreeSet<&'s str>,
    api_attributes: BTreeSet<&'s str>,
    api_derives: BTreeSet<&'s str>,
    rebound: &'s BTreeSet<&'s str>,
    rebound_all: bool,
    own_macros: &'s BTreeSet<&'s str>,
}

/// What a macro, attribute or derive path is.
#[derive(Clone, Copy)]
enum Named {
    Macro,
    Attribute,
    Derive,
}

impl Trusted<'_> {
    /// Whether `path`, used as a `kind`, is certainly a standard or API one.
    fn path(&self, path: &[&str], kind: Named) -> bool {
        let (std, api) = match kind {
            Named::Macro => (STD_MACROS, &self.api_macros),
            Named::Attribute => (BUILTIN_ATTRIBUTES, &self.api_attributes),
            Named::Derive => (STD_DERIVES, &self.api_derives),
        };
        match path {
            [name] => {
                !self.rebound_all
                    && !self.rebound.contains(name)
                    && (std.contains(name)
                        || api.contains(name)
                        || (matches!(kind, Named::Macro) && *name == "macro_rules"))
            }
            [root, .., name] if self.roots.contains(root) => api.contains(name),
            [root, .., name] if matches!(*root, "std" | "core" | "alloc") => std.contains(name),
            [root, ..] => {
                matches!(kind, Named::Attribute)
                    && matches!(*root, "rustfmt" | "clippy" | "diagnostic")
            }
            [] => false,
        }
    }

    /// Whether the attribute written in `tokens[start..end]` is certainly a standard or API one,
    /// reading `derive(…)` and `cfg_attr(…)` arguments to `depth` levels.
    fn attribute(&self, file: &FileFacts<'_>, start: usize, end: usize, depth: usize) -> bool {
        let tokens = &file.tokens;
        let (path, next) = path_at(tokens, start, end);
        if path.is_empty() || depth == 0 {
            return false;
        }
        let arguments = || {
            punct(tokens, next, b'(')
                .then(|| file.closers[next])
                .flatten()
                .map(|close| arguments(tokens, next, close))
        };
        match &path[..] {
            ["derive"] if self.path(&path, Named::Attribute) => arguments().is_some_and(|list| {
                list.iter().all(|(from, to)| {
                    let (derive, after) = path_at(tokens, *from, *to);
                    after == *to && self.path(&derive, Named::Derive)
                })
            }),
            ["cfg_attr"] if self.path(&path, Named::Attribute) => arguments().is_some_and(|list| {
                list.iter()
                    .skip(1)
                    .all(|(from, to)| self.attribute(file, *from, *to, depth - 1))
            }),
            _ => self.path(&path, Named::Attribute),
        }
    }

    /// Whether every macro call and attribute in `file` is certainly a standard or API one, so
    /// nothing the scan cannot expand may bring a name into its scope.
    fn file(&self, file: &FileFacts<'_>) -> bool {
        file.macro_calls
            .iter()
            .all(|path| self.path(path, Named::Macro))
            && self.attributes(file)
    }

    fn attributes(&self, file: &FileFacts<'_>) -> bool {
        file.attributes
            .iter()
            .all(|(open, close)| self.attribute(file, open + 1, *close, 16))
    }

    /// Whether the scan reads every expansion in `file`: each macro is a standard or API one
    /// or the mod's own `macro_rules!` (whose body the scan reads as tokens), and each attribute
    /// a standard or API one.
    fn followed(&self, file: &FileFacts<'_>) -> bool {
        let own = |path: &[&str]| match path {
            [name] | ["crate", name] => !self.rebound_all && self.own_macros.contains(name),
            _ => false,
        };
        file.macro_calls
            .iter()
            .all(|path| self.path(path, Named::Macro) || own(path))
            && self.attributes(file)
    }
}

/// The path written from `at` before `end` (a leading `::` allowed), and the index after it;
/// empty when it does not start with a name.
fn path_at<'t>(tokens: &Tokens<'t>, mut at: usize, end: usize) -> (Vec<&'t str>, usize) {
    if punct(tokens, at, b':') && punct(tokens, at + 1, b':') {
        at += 2;
    }
    let mut path = Vec::new();
    while at < end
        && let Some(name) = ident(tokens, at)
    {
        path.push(name);
        at += 1;
        if at < end && punct(tokens, at, b':') && punct(tokens, at + 1, b':') {
            at += 2;
        } else {
            break;
        }
    }
    (path, at)
}

/// The comma-separated arguments inside the bracket `open`..`close`, each as a token range.
fn arguments(tokens: &Tokens<'_>, open: usize, close: usize) -> Vec<(usize, usize)> {
    let mut list = Vec::new();
    let mut depth = 0usize;
    let mut from = open + 1;
    for at in open + 1..close {
        match tokens[at].0 {
            Token::Punct(b'(' | b'[' | b'{') => depth += 1,
            Token::Punct(b')' | b']' | b'}') => depth = depth.saturating_sub(1),
            Token::Punct(b',') if depth == 0 => {
                list.push((from, at));
                from = at + 1;
            }
            _ => {}
        }
    }
    if from < close {
        list.push((from, close));
    }
    list
}

/// What the surface says, indexed once for a scan.
struct Index<'s> {
    surface: &'s Surface,
    /// Every record's hash by id.
    hashes: BTreeMap<&'s str, &'s str>,
    types: BTreeMap<&'s str, &'s Type>,
    roles: BTreeMap<&'s str, Role>,
    /// Each item's public paths, `mod_api::` left out.
    public_paths: BTreeMap<&'s str, &'s [String]>,
    /// The ids each name may stand for.
    by_name: BTreeMap<&'s str, Vec<&'s str>>,
    /// Each member's owner.
    owners: BTreeMap<&'s str, String>,
    /// Every API module or item path and each of its prefixes, `mod_api::` left out: what a
    /// name bound to an API path can stand for.
    api_prefixes: BTreeSet<String>,
}

impl<'s> Index<'s> {
    fn new(surface: &'s Surface) -> Self {
        let hashes: BTreeMap<&str, &str> = surface.entries().collect();
        let types = surface.types.iter().map(|t| (t.id.as_str(), t)).collect();
        let roles = surface
            .traits
            .iter()
            .map(|t| (t.id.as_str(), t.contract.role))
            .collect();
        let mut public_paths: BTreeMap<&str, &[String]> = BTreeMap::new();
        for t in &surface.traits {
            public_paths.insert(&t.id, &t.paths);
        }
        for t in &surface.types {
            public_paths.insert(&t.id, &t.paths);
        }
        for f in &surface.functions {
            public_paths.insert(&f.id, &f.paths);
        }
        for m in &surface.macros {
            public_paths.insert(&m.id, &m.paths);
        }
        let mut by_name: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (name, id) in surface.identifiers() {
            by_name.entry(name).or_default().push(id);
        }
        let owners = hashes
            .keys()
            .filter_map(|id| owner(id, &hashes).map(|owner| (*id, owner)))
            .collect();
        let mut api_prefixes = BTreeSet::new();
        for (id, paths) in &public_paths {
            let own = id.split_once(':').map(|(_, rest)| rest.to_owned());
            for path in paths.iter().cloned().chain(own) {
                let mut prefix = String::new();
                for segment in path.split("::") {
                    if !prefix.is_empty() {
                        prefix.push_str("::");
                    }
                    prefix.push_str(segment);
                    api_prefixes.insert(prefix.clone());
                }
            }
        }
        Index {
            surface,
            hashes,
            types,
            roles,
            public_paths,
            by_name,
            owners,
            api_prefixes,
        }
    }

    fn kind(&self, id: &str) -> Option<TypeKind> {
        self.types.get(id).map(|t| t.contract.kind)
    }

    fn struct_like(&self, id: &str) -> bool {
        matches!(
            self.kind(id),
            Some(TypeKind::Struct | TypeKind::TupleStruct | TypeKind::UnitStruct | TypeKind::Union)
        )
    }

    /// Whether code outside the API may build a struct at all: what the scan cannot see can
    /// only build those.
    fn buildable(&self, id: &str) -> bool {
        self.types.get(id).is_some_and(|t| t.contract.constructible)
    }

    /// The standard and API macro, attribute and derive names the mod may use and still be read
    /// as written.
    fn trusted<'m>(&self, module: &'m ModFacts<'m>) -> Trusted<'m>
    where
        's: 'm,
    {
        let macros = &self.surface.macros;
        Trusted {
            roots: &module.roots,
            api_macros: macros
                .iter()
                .filter(|m| {
                    matches!(
                        m.contract.kind,
                        MacroKind::Function | MacroKind::Declarative
                    )
                })
                .map(|m| leaf(&m.id))
                .collect(),
            api_attributes: macros
                .iter()
                .filter(|m| matches!(m.contract.kind, MacroKind::Attribute | MacroKind::Derive))
                .flat_map(|m| {
                    let own = (m.contract.kind == MacroKind::Attribute).then(|| leaf(&m.id));
                    own.into_iter()
                        .chain(m.contract.attributes.iter().map(String::as_str))
                })
                .collect(),
            api_derives: macros
                .iter()
                .filter(|m| m.contract.kind == MacroKind::Derive)
                .map(|m| leaf(&m.id))
                .collect(),
            rebound: &module.rebound,
            rebound_all: module.rebound_all,
            own_macros: &module.own_macros,
        }
    }
}

/// What a name in the mod may stand for, worked out once per name.
struct Resolved<'s> {
    /// Every API id it may be.
    ids: Vec<&'s str>,
    /// Some alias leads it to an API name other than itself.
    aliased: bool,
}

/// What names mean across the mod: the facts of every file, which files nothing the scan cannot
/// follow may change, and each name's API ids, kept as they are worked out.
struct Resolver<'a, 's, 't> {
    index: &'a Index<'s>,
    module: &'a ModFacts<'t>,
    files: &'a [FileFacts<'t>],
    /// Per file: whether nothing the scan cannot follow may change what names mean there.
    certain: Vec<bool>,
    /// Per file: whether it holds an expansion the scan cannot follow.
    opaque: Vec<bool>,
    /// Per file: whether a variant of its own enum may be told from an API struct
    /// ([`Self::own_enum_in`]).
    variant_files: Vec<bool>,
    /// Per file: the `type` aliases of an API item, which define no type of the mod's own.
    api_aliases: Vec<BTreeSet<&'t str>>,
    /// The API modules some file imports with a glob, `mod_api::` left out.
    mod_globs: BTreeSet<Vec<&'t str>>,
    memo: BTreeMap<&'t str, Resolved<'s>>,
}

impl<'a, 's, 't> Resolver<'a, 's, 't> {
    fn new(index: &'a Index<'s>, module: &'a ModFacts<'t>, files: &'a [FileFacts<'t>]) -> Self {
        let trusted = index.trusted(module);
        // A file brought in by `include!` (under any name: `use std::include as inc;`, which
        // the folder reader refuses) is part of its includer's scope: with one anywhere, no
        // file is judged on its own.
        let included = files.iter().any(|file| {
            file.macro_calls
                .iter()
                .any(|path| path.last() == Some(&"include"))
                || file
                    .imports
                    .iter()
                    .any(|(path, _)| path.last() == Some(&"include"))
        });
        // An expansion the scan cannot follow: a `use` with a metavariable, or a macro or
        // attribute that is neither a standard or API one nor the mod's own `macro_rules!`.
        let opaque: Vec<bool> = files
            .iter()
            .map(|file| file.use_metavariable || !trusted.followed(file))
            .collect();
        let certain: Vec<bool> = files
            .iter()
            .zip(&opaque)
            .map(|(file, opaque)| {
                !included
                    && !opaque
                    && trusted.file(file)
                    && !file.syntax.inline_modules
                    && !file.imports.iter().any(|(_, local)| *local == "*")
            })
            .collect();
        // Anything that may bring a name into a file from outside it or that the scan cannot
        // compare turns the variant exemption off for the whole mod: a macro the mod defines
        // (a `macro_rules!` reaches every file declared after it), `#[macro_use]`, a binding
        // of a standard or API macro's name or of `std`, `core` or `alloc`, and a non-ASCII
        // identifier.
        let known = |name: &str| STD_MACROS.contains(&name) || trusted.api_macros.contains(name);
        let crate_name = |name: &str| matches!(name, "std" | "core" | "alloc");
        let variants_off = module.rebound_all
            || files.iter().any(|file| {
                !file.macro_rules.is_empty()
                    || (0..file.tokens.len()).any(|at| {
                        ident(&file.tokens, at) == Some("macro")
                            && ident(&file.tokens, at + 1).is_some()
                    })
                    || file
                        .tokens
                        .iter()
                        .any(|(token, _)| matches!(token, Token::Ident(name) if !name.is_ascii()))
                    || file
                        .imports
                        .iter()
                        .any(|(_, local)| known(local) || crate_name(local))
                    || file
                        .syntax
                        .bound
                        .iter()
                        .any(|name| known(name) || crate_name(name))
            });
        // A macro called through a path (`std::vec!`, `$crate::m!`) may be another than its
        // name says.
        let variant_files = files
            .iter()
            .zip(&certain)
            .map(|(file, certain)| {
                *certain
                    && !variants_off
                    && file.macro_calls.iter().all(|path| match &path[..] {
                        [name] => *name != "$",
                        _ => false,
                    })
            })
            .collect();
        let mod_globs = files.iter().flat_map(|f| api_globs(f, module)).collect();
        let mut resolver = Resolver {
            index,
            module,
            files,
            certain,
            opaque,
            variant_files,
            api_aliases: Vec::new(),
            mod_globs,
            memo: BTreeMap::new(),
        };
        resolver.api_aliases = files
            .iter()
            .map(|file| {
                file.type_aliases
                    .iter()
                    .filter(|(_, target, item)| *item && !resolver.lookup(target).ids.is_empty())
                    .map(|(alias, _, _)| *alias)
                    .collect()
            })
            .collect();
        resolver
    }

    /// The API ids `name` may stand for, through the mod's aliases.
    fn lookup(&mut self, name: &'t str) -> &Resolved<'s> {
        let (module, by_name) = (self.module, &self.index.by_name);
        self.memo.entry(name).or_insert_with(|| {
            let api_names = api_names_of(name, module, by_name);
            Resolved {
                aliased: api_names.iter().any(|api| *api != name),
                ids: api_names
                    .iter()
                    .flat_map(|api| by_name[api].iter().copied())
                    .collect(),
            }
        })
    }

    /// Whether an unqualified `name` in file `index` certainly means the mod's own type: the
    /// file defines it at its top level under no `#[cfg]` (a `type` alias of an API item is no
    /// definition), no `use` in it binds the name, and the file is certain. Anything else
    /// leaves the API's.
    fn own_in(&self, index: usize, name: &str) -> bool {
        let file = &self.files[index];
        self.certain[index]
            && file.syntax.defines.contains(name)
            && !self.api_aliases[index].contains(name)
            && !file.imported.contains(name)
    }

    /// Whether `owner` in file `index` is certainly that file's own top-level enum with
    /// `variant`: the exemption may apply in the file (`variant_files`), it defines the enum at
    /// its top level under no `#[cfg]`, and binds the name nowhere else (no other item, module,
    /// extern crate or generic parameter, no `use` but a single leaf `use self::…`, nothing in a
    /// macro body), and no API item has the name.
    fn own_enum_in(&self, index: usize, owner: &'t str, variant: &'t str) -> bool {
        let file = &self.files[index];
        self.variant_files[index]
            && !file.odd_uses.contains(owner)
            && file.syntax.top_enums.contains(&(owner, variant))
            && file
                .syntax
                .bound
                .iter()
                .filter(|name| **name == owner)
                .count()
                == 1
            && !file.imports.iter().any(|(_, local)| *local == owner)
            && !file.macro_types.contains(owner)
            && !file
                .type_aliases
                .iter()
                .any(|(alias, _, _)| *alias == owner)
            && api_names_of(owner, self.module, &self.index.by_name).is_empty()
    }

    /// Whether `path`, written in file `index`, is a variant of that file's own enum
    /// ([`Self::own_enum_in`]): `Kind::Settings` or `self::Kind::Settings`, not rooted.
    fn own_variant(&self, index: usize, path: &[&'t str], rooted: bool) -> bool {
        let (owner, variant) = match path {
            [owner, variant] | ["self", owner, variant] => (*owner, *variant),
            _ => return false,
        };
        !rooted && self.own_enum_in(index, owner, variant)
    }

    /// Whether an unqualified `name` in file `index` is a variant of that file's own enum that
    /// the file imports by the single leaf `use self::Kind::Settings;` (or `as` another name):
    /// that `use` is the only one that names it, the variant is the file's own
    /// ([`Self::own_enum_in`]), and nothing else in the file binds or aliases the name.
    fn imported_variant(&self, index: usize, name: &'t str) -> bool {
        let file = &self.files[index];
        let mut bindings = file.imports.iter().filter(|(_, local)| *local == name);
        !file.odd_uses.contains(name)
            && !file.syntax.bound.contains(&name)
            && !file.macro_types.contains(name)
            && !file.type_aliases.iter().any(|(alias, _, _)| *alias == name)
            && matches!(
                (bindings.next(), bindings.next()),
                (Some((path, _)), None) if matches!(path[..], ["self", owner, variant]
                    if self.own_enum_in(index, owner, variant))
            )
    }

    /// Whether a name the scan cannot place may be an API struct by another name.
    fn unknown(&self, name: &str) -> bool {
        name.starts_with(|c: char| c.is_ascii_uppercase())
            && name.chars().any(|c| c.is_ascii_lowercase())
            && name != "Self"
            && !self.module.types.contains(name)
            && !self.module.item_aliases.contains(name)
            && !self.module.others.contains(name)
    }

    /// Each API path (`mod_api::` left out) a path written in a file may be: through an API
    /// root or a name bound to an API path, under one of the file's API globs, or after
    /// `crate`, `self`, `super` or a module the mod declares, through the mod's re-exports and
    /// globs.
    fn api_paths(&self, path: &[&'t str], globs: &BTreeSet<Vec<&'t str>>) -> Vec<Vec<String>> {
        let inside = path
            .iter()
            .take_while(|segment| {
                matches!(**segment, "crate" | "self" | "super")
                    || self.module.modules.contains(*segment)
            })
            .count();
        let (rest, globs) = if inside > 0 {
            (&path[inside..], &self.mod_globs)
        } else {
            (path, globs)
        };
        // A module of the mod's may share its name with an API root's module: both are read.
        let mut out: Vec<Vec<String>> = api_paths_of(rest, self.module)
            .into_iter()
            .chain(
                (inside > 0)
                    .then(|| api_paths_of(path, self.module))
                    .into_iter()
                    .flatten(),
            )
            .map(|p| p.iter().map(|s| (*s).to_owned()).collect())
            .collect();
        // `crate::Biome` through the mod's own `pub use mod_api::prelude::*`.
        if rest.len() >= 2 || (inside > 0 && !rest.is_empty()) {
            for glob in globs {
                out.push(glob.iter().chain(rest).map(|s| (*s).to_owned()).collect());
            }
        }
        out
    }

    /// The innermost impl in file `index` whose body holds `byte`: its target, `None` inside
    /// it when the scan cannot read the target, `None` outside any.
    fn impl_at(&self, index: usize, byte: usize) -> Option<Option<&'a Vec<&'t str>>> {
        let file: &'a FileFacts<'t> = &self.files[index];
        let after = file
            .impls
            .partition_point(|(i, _)| file.syntax.impls[*i].body.start < byte);
        let mut at = after.checked_sub(1);
        while let Some(position) = at {
            let (i, outer) = file.impls[position];
            if byte < file.syntax.impls[i].body.end {
                return Some(file.syntax.impls[i].target.as_ref());
            }
            at = outer;
        }
        None
    }
}

/// What the walk over the mod's files finds: the records named directly, each with the first
/// place that names it; how each named struct is built; the public paths the mod writes each
/// by; and the first place of a literal the scan cannot place.
struct Found<'s, 't> {
    direct: BTreeMap<&'s str, Place<'t>>,
    building: BTreeMap<&'s str, Building<'t>>,
    written: BTreeMap<&'s str, BTreeSet<String>>,
    named_structs: BTreeSet<&'s str>,
    unsure: Option<Place<'t>>,
}

impl<'s, 't> Found<'s, 't> {
    fn mark_unsure(&mut self, here: Place<'t>) {
        if self.unsure.is_none_or(|old| here < old) {
            self.unsure = Some(here);
        }
    }

    fn name(&mut self, id: &'s str, here: Place<'t>) {
        self.direct
            .entry(id)
            .and_modify(|first| *first = here.min(*first))
            .or_insert(here);
    }

    fn build(&mut self, id: &'s str, place: Place<'t>, full: bool, brace_literal: bool) {
        self.building
            .entry(id)
            .or_default()
            .record(place, full, brace_literal);
    }
}

/// The walk over one file.
struct Walk<'r, 'a, 's, 't> {
    resolver: &'r mut Resolver<'a, 's, 't>,
    found: &'r mut Found<'s, 't>,
    index: usize,
    path: &'t str,
}

impl<'s, 't> Walk<'_, '_, 's, 't> {
    /// Every name in the file: the records it names, and what a macro body builds with it.
    fn names(&mut self) {
        let Walk {
            resolver,
            found,
            index,
            path,
        } = self;
        let (index, path) = (*index, *path);
        let files = resolver.files;
        let file = &files[index];
        let tokens = &file.tokens;
        for (at, (token, line)) in tokens.iter().enumerate() {
            let Token::Ident(written_name) = token else {
                continue;
            };
            let place = (path, *line);
            let start = file.path_starts[at];
            let open = at + 1;
            let opens = punct(tokens, open, b'{') || punct(tokens, open, b'(');
            let (name, unqualified) = match *written_name {
                // A `Self` in a macro body may belong to an impl the macro writes, which the
                // parser does not read: `Self { … }` there builds a struct the scan cannot place.
                "Self" if file.in_macro[at] => {
                    if opens || bare_value(file, at) {
                        found.mark_unsure(place);
                    }
                    continue;
                }
                "Self" => match resolver.impl_at(index, file.starts[at]) {
                    Some(Some(target)) => (*target.last().unwrap_or(&"Self"), target.len() == 1),
                    // Outside any impl the parser placed, or in one of a type it cannot read,
                    // the parser reads any build.
                    _ => continue,
                },
                name => (
                    name,
                    start == at
                        && !(at >= 2 && punct(tokens, at - 1, b':') && punct(tokens, at - 2, b':')),
                ),
            };
            resolver.lookup(name);
            let resolver: &Resolver<'_, 's, 't> = resolver;
            let resolved = &resolver.memo[name];
            if resolved.ids.is_empty() {
                // In a macro body, which the parser does not read: a literal of a name the
                // scan cannot place, or a metavariable's `$name { … }` or `$name(…)`, which
                // may be a struct's name.
                let metavariable = at >= 1 && punct(tokens, at - 1, b'$');
                if file.in_macro[at]
                    && ((punct(tokens, open, b'{')
                        && !in_type_position(tokens, &file.path_starts, at)
                        && resolver.unknown(name)
                        && looks_like_fields(tokens, open))
                        || (metavariable
                            && opens
                            && !in_type_position(tokens, &file.path_starts, at - 1)))
                {
                    found.mark_unsure(place);
                }
                continue;
            }
            let own = unqualified && !resolved.aliased && resolver.own_in(index, name);
            let api = resolver.index;
            // After `X::`, where `X` owns a member of the name, the name is that member only.
            let qualifier = (start < at && !resolved.aliased)
                .then(|| ident(tokens, at - 3))
                .flatten();
            let member = |id: &&str| {
                qualifier.is_some_and(|q| api.owners.get(id).is_some_and(|o| leaf(o) == q))
            };
            let members = resolved.ids.iter().any(member);
            for &id in resolved.ids.iter().filter(|id| !members || member(id)) {
                found.name(id, place);
                if !api.struct_like(id) || own {
                    continue;
                }
                found.named_structs.insert(id);
                if resolver.opaque[index] && api.buildable(id) {
                    // An expansion the scan cannot follow may build any API struct the file
                    // names.
                    found.build(id, place, true, false);
                }
                if file.in_macro[at] {
                    macro_body(found, api, file, id, at, place);
                }
            }
        }
    }

    /// Every place the parser reads a struct built or matched.
    fn builds(&mut self) {
        let Walk {
            resolver,
            found,
            index,
            path,
        } = self;
        let (index, path) = (*index, *path);
        let files = resolver.files;
        let file = &files[index];
        for build in &file.syntax.builds {
            let place = (path, build.line);
            let (name, unqualified) = match &build.path[..] {
                ["Self"] => match resolver.impl_at(index, build.byte) {
                    Some(Some(target)) => (*target.last().unwrap_or(&"Self"), target.len() == 1),
                    // Outside any impl the parser placed, or in one of a type it cannot read:
                    // a struct the scan cannot place.
                    _ => {
                        found.mark_unsure(place);
                        continue;
                    }
                },
                [.., last] => (*last, build.path.len() == 1 && !build.rooted),
                [] => continue,
            };
            // A variant of the file's own enum: through the enum, through `Self` in an impl
            // for it, or imported by name.
            let variant =
                match &build.path[..] {
                    ["Self", rest @ ..] => resolver
                        .impl_at(index, build.byte)
                        .flatten()
                        .is_some_and(|target| {
                            let path: Vec<&str> = target.iter().chain(rest).copied().collect();
                            resolver.own_variant(index, &path, false)
                        }),
                    [name] if !build.rooted => resolver.imported_variant(index, name),
                    path => resolver.own_variant(index, path, build.rooted),
                };
            if variant {
                continue;
            }
            resolver.lookup(name);
            let resolver: &Resolver<'_, 's, 't> = resolver;
            let resolved = &resolver.memo[name];
            if resolved.ids.is_empty() {
                // A literal of a name the scan cannot place may be an API struct by another
                // name; one written from the standard library is not.
                let standard = matches!(build.path.first(), Some(&("std" | "core" | "alloc")));
                if build.brace && resolver.unknown(name) && !standard {
                    found.mark_unsure(place);
                }
                continue;
            }
            let own = unqualified && !resolved.aliased && resolver.own_in(index, name);
            let api = resolver.index;
            for &id in &resolved.ids {
                if !api.struct_like(id) || own {
                    continue;
                }
                let full = match build.how {
                    How::Full => api.kind(id) != Some(TypeKind::Union),
                    How::Base => false,
                    How::Value => {
                        if !matches!(
                            api.kind(id),
                            Some(TypeKind::TupleStruct | TypeKind::UnitStruct)
                        ) {
                            continue;
                        }
                        true
                    }
                };
                found.build(id, place, full, build.brace);
            }
        }
    }

    /// The API paths the file writes: imported, written out in code or in macro bodies, or
    /// under an imported glob.
    fn paths(&mut self) {
        let resolver = &*self.resolver;
        let file = &resolver.files[self.index];
        let tokens = &file.tokens;
        // A file with a glob of the mod's own (`use super::*;`, `use api::*;`) sees the mod's
        // API globs.
        let mut globs = api_globs(file, resolver.module);
        if file.imports.iter().any(|(segments, local)| {
            *local == "*"
                && segments.first().is_some_and(|first| {
                    matches!(*first, "crate" | "self" | "super")
                        || resolver.module.modules.contains(first)
                })
        }) {
            globs.extend(resolver.mod_globs.iter().cloned());
        }
        let mut paths: Vec<Vec<String>> = Vec::new();
        for (segments, local) in &file.imports {
            if *local != "*" {
                paths.extend(resolver.api_paths(segments, &globs));
            }
        }
        for chain in file.syntax.paths.iter().chain(&file.chains) {
            paths.extend(resolver.api_paths(chain, &globs));
        }
        // Under a glob, a name written alone is the glob's; one written after a path is
        // that path's, read above.
        let names: BTreeSet<&str> = (0..tokens.len())
            .filter(|at| !(*at >= 2 && punct(tokens, at - 1, b':') && punct(tokens, at - 2, b':')))
            .filter_map(|at| ident(tokens, at))
            .collect();
        for glob in &globs {
            for name in &names {
                paths.push(
                    glob.iter()
                        .map(|s| (*s).to_owned())
                        .chain([(*name).to_owned()])
                        .collect(),
                );
            }
        }
        let index = resolver.index;
        for api in paths {
            let Some(name) = api.last() else {
                continue;
            };
            let path = api.join("::");
            for id in index.by_name.get(name.as_str()).into_iter().flatten() {
                let id_path = id.split_once(':').map_or("", |(_, rest)| rest);
                if path != id_path
                    && index
                        .public_paths
                        .get(id)
                        .is_some_and(|paths| paths.contains(&path))
                {
                    self.found
                        .written
                        .entry(id)
                        .or_default()
                        .insert(path.clone());
                }
            }
        }
    }
}

/// What a macro body, which the parser does not read, builds with API struct `id` at `at`.
/// A name followed by `(` builds or matches it wherever it stands (`a < Pos(1, 2)`,
/// `for Pos(x, y) in v`), and so does one followed by `{`, except where only a type stands and
/// the brace holds no fields (`impl Tr for Marker {}`) or follows `->` (a function's body). Each
/// is in full unless it ends in `..`. A tuple or unit struct's name with no bracket after it is
/// a full build where it stands as a value ([`bare_value`]: `.map(Pos)`, `{ Marker }`,
/// `make: Pos`).
fn macro_body<'s, 't>(
    found: &mut Found<'s, 't>,
    api: &Index<'s>,
    file: &FileFacts<'t>,
    id: &'s str,
    at: usize,
    place: Place<'t>,
) {
    let tokens = &file.tokens;
    if file.in_use[at] {
        return;
    }
    let kind = api.kind(id);
    let start = file.path_starts[at];
    let open = at + 1;
    let brace = punct(tokens, open, b'{');
    let after_arrow =
        start >= 2 && punct(tokens, start - 1, b'>') && punct(tokens, start - 2, b'-');
    let opens = punct(tokens, open, b'(')
        || (brace && (!only_a_type(file, start) || (holds_fields(tokens, open) && !after_arrow)));
    if opens {
        let full = !file.rests[open] && kind != Some(TypeKind::Union);
        found.build(id, place, full, brace);
    } else if !brace
        && matches!(kind, Some(TypeKind::TupleStruct | TypeKind::UnitStruct))
        && bare_value(file, at)
    {
        found.build(id, place, true, false);
    }
}

/// Whether the brace at `open` starts with a field: a name or position followed by `,`, `}` or
/// `:`, or `..`.
fn holds_fields(tokens: &Tokens<'_>, open: usize) -> bool {
    match tokens.get(open + 1).map(|(t, _)| *t) {
        Some(Token::Ident(_) | Token::Literal(_)) => {
            punct(tokens, open + 2, b',')
                || punct(tokens, open + 2, b'}')
                || (punct(tokens, open + 2, b':') && !punct(tokens, open + 3, b':'))
        }
        Some(Token::Punct(b'.')) => punct(tokens, open + 2, b'.'),
        _ => false,
    }
}

/// Whether the path starting at `start` in a macro body stands where only a type can: in a
/// `use`, or after `->`, `impl`, `dyn`, `for`, `as`, `where`, a definition keyword or `<`.
fn only_a_type(file: &FileFacts<'_>, start: usize) -> bool {
    let tokens = &file.tokens;
    file.in_use[start]
        || start
            .checked_sub(1)
            .is_some_and(|before| match tokens[before].0 {
                Token::Ident(word) => matches!(
                    word,
                    "impl"
                        | "dyn"
                        | "struct"
                        | "enum"
                        | "union"
                        | "trait"
                        | "type"
                        | "mod"
                        | "fn"
                        | "where"
                        | "as"
                        | "for"
                ),
                Token::Punct(b'<') => true,
                Token::Punct(b'>') => before >= 1 && punct(tokens, before - 1, b'-'),
                _ => false,
            })
}

/// Whether the parenthesis at `open` opens a function's parameters: it follows `fn name`, with
/// generics or not (looked back over at most 64 tokens). Any other parenthesis, a macro call's
/// (`maker!(make: Pos)`) for one, may hold values.
fn opens_parameters(tokens: &Tokens<'_>, open: usize) -> bool {
    let named =
        |at: usize| at >= 1 && ident(tokens, at).is_some() && ident(tokens, at - 1) == Some("fn");
    if !punct(tokens, open, b'(') {
        return false;
    }
    let Some(before) = open.checked_sub(1) else {
        return false;
    };
    if named(before) {
        return true;
    }
    if !punct(tokens, before, b'>') {
        return false;
    }
    // Back to the `<` that opens the generics; a `->` inside them closes nothing.
    let mut depth = 0usize;
    for at in (before.saturating_sub(64)..=before).rev() {
        if punct(tokens, at, b'>') && !(at >= 1 && punct(tokens, at - 1, b'-')) {
            depth += 1;
        } else if punct(tokens, at, b'<') {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return at >= 1 && named(at - 1);
            }
        }
    }
    false
}

/// Whether the path starting at `start` in a macro body is a type written after `name:` (`&`,
/// `mut` and lifetimes between them): in a function's parameters ([`opens_parameters`]), a
/// closure's first parameter, a `let`, or a struct's or union's field list. Anything else after
/// `:` (a struct literal's field, or a key in a macro call) may be a value.
fn annotation(file: &FileFacts<'_>, start: usize) -> bool {
    let tokens = &file.tokens;
    let mut before = start;
    while let Some(previous) = before.checked_sub(1)
        && matches!(
            tokens[previous].0,
            Token::Punct(b'&') | Token::Lifetime | Token::Ident("mut")
        )
    {
        before = previous;
    }
    let Some(colon) = before.checked_sub(1) else {
        return false;
    };
    if !punct(tokens, colon, b':') || (colon >= 1 && punct(tokens, colon - 1, b':')) {
        return false;
    }
    let Some(name) = colon.checked_sub(1) else {
        return false;
    };
    let word = |at: Option<usize>| at.and_then(|at| ident(tokens, at));
    let bound_by_let = word(name.checked_sub(1)) == Some("let")
        || (word(name.checked_sub(1)) == Some("mut") && word(name.checked_sub(2)) == Some("let"));
    let closure_parameter = name >= 1 && punct(tokens, name - 1, b'|');
    let enclosing = file.enclosing[colon];
    let parameter = enclosing.is_some_and(|open| opens_parameters(tokens, open));
    let field = enclosing.is_some_and(|open| {
        punct(tokens, open, b'{')
            && open >= 2
            && ident(tokens, open - 1).is_some()
            && matches!(ident(tokens, open - 2), Some("struct" | "union"))
    });
    bound_by_let || closure_parameter || parameter || field
}

/// Whether the name at `at` in a macro body, with no bracket after it, stands as a value: not
/// where only a type can, not a type after `name:`, and not followed by `::` or `!`.
fn bare_value(file: &FileFacts<'_>, at: usize) -> bool {
    let tokens = &file.tokens;
    let start = file.path_starts[at];
    !only_a_type(file, start)
        && !annotation(file, start)
        && !(punct(tokens, at + 1, b':') && punct(tokens, at + 2, b':'))
        && !punct(tokens, at + 1, b'!')
}

/// The records `sources` (each a path and its Rust text) and `manifest` bring in, with
/// everything their signatures reference; [`ScanError`] when the source cannot be judged.
///
/// Each file is parsed first in a child process ([`crate::child`]), so a parse that runs out of
/// stack or memory, or does not end, leaves the source "cannot judge" instead of ending the
/// caller. The checks before parsing run first, then one child parses every file, on half the
/// scan's own parser stack; the scan then parses them itself, with megabytes to spare over
/// whatever the child needed. `entry` is what [`crate::child::child_entry`] returns, so a
/// program that has not called it first in `main` does not compile.
pub fn uses_contained<'t>(
    entry: crate::child::ChildEntry,
    sources: impl IntoIterator<Item = (&'t str, &'t str)>,
    manifest: &ManifestFacts,
    surface: &Surface,
) -> Result<Uses, ScanError> {
    let sources: Vec<(&str, &str)> = sources.into_iter().collect();
    for (file, text) in &sources {
        checked_tokens(file, text)?;
    }
    let program = std::env::current_exe().map_err(|_| ScanError::NoChild)?;
    let texts: Vec<&str> = sources.iter().map(|(_, text)| *text).collect();
    let file = |at: Option<usize>| {
        at.and_then(|at| sources.get(at))
            .map(|(f, _)| (*f).to_owned())
    };
    let _ = entry;
    crate::child::parse_in_child(
        &program,
        &texts,
        syntax::PARSER_STACK / 2,
        crate::child::CHILD_WAIT,
    )
    .map_err(|contained| match contained {
        crate::child::Contained::Died(at) => ScanError::ParserDied { file: file(at) },
        crate::child::Contained::TimedOut(at) => ScanError::ParserTimedOut { file: file(at) },
        crate::child::Contained::NoChild => ScanError::NoChild,
    })?;
    uses(sources, manifest, surface)
}

/// [`uses_contained`] without the child process: a parse that overflows aborts the caller, so
/// only the crate's own tests call it.
pub(crate) fn uses<'t>(
    sources: impl IntoIterator<Item = (&'t str, &'t str)>,
    manifest: &ManifestFacts,
    surface: &Surface,
) -> Result<Uses, ScanError> {
    let index = Index::new(surface);
    let sources: Vec<(&str, &str)> = sources.into_iter().collect();
    let files: Vec<FileFacts<'_>> = sources
        .iter()
        .map(|(file, text)| file_facts(file, text))
        .collect::<Result<_, _>>()?;
    let module = mod_facts(&files, &index.api_prefixes);
    let mut resolver = Resolver::new(&index, &module, &files);
    let mut found = Found {
        direct: BTreeMap::new(),
        building: BTreeMap::new(),
        written: BTreeMap::new(),
        named_structs: BTreeSet::new(),
        unsure: None,
    };
    for (file, (path, _)) in sources.iter().copied().enumerate() {
        let mut walk = Walk {
            resolver: &mut resolver,
            found: &mut found,
            index: file,
            path,
        };
        walk.names();
        walk.builds();
        walk.paths();
    }
    let Found {
        direct,
        mut building,
        written,
        named_structs,
        unsure,
    } = found;
    // A literal the scan cannot place, anywhere in the mod, may build any API struct the mod
    // names, anywhere: each counts as built field by field.
    if let Some(place) = unsure {
        for id in named_structs.into_iter().filter(|id| index.buildable(id)) {
            building.entry(id).or_default().record(place, true, false);
        }
    }
    let mut direct: BTreeMap<&str, Origin> = direct
        .into_iter()
        .map(|(id, (file, line))| {
            let file = file.to_owned();
            (id, Origin::Source { file, line })
        })
        .collect();
    for name in &manifest.names {
        for id in [format!("hook:{name}"), format!("capability:{name}")] {
            if let Some((id, _)) = index.hashes.get_key_value(id.as_str()) {
                direct.entry(id).or_insert(Origin::Manifest);
            }
        }
    }
    Ok(list(&index, manifest, direct, &building, &written))
}

/// The list of what counts, from the records named directly: each with how it is built, the
/// paths written, and the hosts that lead to it.
fn list(
    index: &Index<'_>,
    manifest: &ManifestFacts,
    direct: BTreeMap<&str, Origin>,
    building: &BTreeMap<&str, Building>,
    written: &BTreeMap<&str, BTreeSet<String>>,
) -> Uses {
    let Index {
        surface,
        hashes,
        types,
        roles,
        owners,
        ..
    } = index;
    // A member counts once its owner is reached from what counts, through the supertraits of
    // called traits too (a sub-host trait's method called on `&mut dyn ServerHost`).
    let mut counted: BTreeSet<&str> = direct
        .keys()
        .filter(|id| !owners.contains_key(*id))
        .copied()
        .collect();
    loop {
        let reached = surface.reach(counted.iter().copied());
        let more: Vec<&str> = direct
            .keys()
            .filter(|id| {
                !counted.contains(*id) && owners.get(*id).is_some_and(|o| reached.contains(o))
            })
            .copied()
            .collect();
        if more.is_empty() {
            break;
        }
        counted.extend(more);
    }

    // The list: what counts and what its signatures reference, each reached record naming the
    // counted record it was first reached from. A called trait's supertraits are not in it.
    let refs = surface.refs(false);
    let mut origins: BTreeMap<&str, Origin> =
        counted.iter().map(|id| (*id, direct[id].clone())).collect();
    for start in &counted {
        let mut pending = vec![*start];
        while let Some(id) = pending.pop() {
            for next in refs.get(id).into_iter().flatten() {
                if hashes.contains_key(next) && !origins.contains_key(next) {
                    origins.insert(next, Origin::Via((*start).to_owned()));
                    pending.push(next);
                }
            }
        }
    }
    // A trait's member names every called trait in the list that leads to its owner through
    // supertraits, whether or not the mod also names the owner or the called trait: the scan
    // cannot see which one a call goes through (`host.player(id)` on `&mut dyn ServerHost` in a
    // mod that also counts `ClientHost`, `show(host)` upcasting to a named `&dyn PlayersHost`, or
    // a closure's host typed by an API method's signature), so the target must still lead from
    // each.
    let called: Vec<&str> = origins
        .keys()
        .copied()
        .filter(|id| roles.get(id) == Some(&Role::Call))
        .collect();
    let leads: BTreeMap<&str, BTreeSet<String>> = called
        .iter()
        .map(|start| (*start, surface.reach([*start])))
        .collect();
    let reached_from: BTreeMap<&str, Vec<&str>> = counted
        .iter()
        .filter_map(|id| {
            let owner = owners.get(id).filter(|o| o.starts_with("trait:"))?;
            let from: Vec<&str> = called
                .iter()
                .copied()
                .filter(|start| *start != owner.as_str() && leads[start].contains(owner))
                .collect();
            (!from.is_empty()).then_some((*id, from))
        })
        .collect();

    let at = |(file, line): &Place<'_>| format!("{file}:{line}");
    let hook_versions = &manifest.hook_versions;
    Uses {
        items: origins
            .into_iter()
            .map(|(id, origin)| {
                let record = types.get(id);
                let how = building.get(id);
                let built = index.struct_like(id).then_some(match how {
                    Some(Building { full: Some(_), .. }) => Built::Full,
                    Some(Building { base: Some(_), .. }) => Built::Base,
                    _ => Built::None,
                });
                let built_at = how
                    .and_then(|b| b.full.as_ref().or(b.base.as_ref()))
                    .map(at);
                let name = id.strip_prefix("type:").unwrap_or(id);
                let note = match (built, &built_at) {
                    (Some(Built::Base), Some(place)) => Some(format!(
                        "`{name}`: built with `..` ({place}), so a new field in a later game build will not break this mod."
                    )),
                    (Some(Built::Full), Some(place))
                        if how.is_some_and(|b| b.brace_literal)
                            && record.is_some_and(|t| t.impls.iter().any(|i| i == "Default")) =>
                    {
                        Some(format!(
                            "`{name}`: built field by field ({place}). A new field in a later game build will break this line; end the literal with `..Default::default()` to stay compatible."
                        ))
                    }
                    (Some(Built::Full), Some(place)) => Some(format!(
                        "`{name}`: built field by field ({place}). A new field in a later game build will break this line."
                    )),
                    _ => None,
                };
                Used {
                    entry: Entry {
                        id: id.to_owned(),
                        hash: hashes[id].to_owned(),
                        impls: record.map(|t| t.impls.clone()).unwrap_or_default(),
                        fields_hash: (built == Some(Built::Full))
                            .then(|| record.and_then(|t| t.fields_hash.clone()))
                            .flatten(),
                        fields: match (built, record) {
                            (Some(Built::Full), Some(t)) => {
                                t.fields.iter().map(|f| leaf(&f.id).to_owned()).collect()
                            }
                            _ => Vec::new(),
                        },
                        built,
                        built_at,
                        paths: written
                            .get(id)
                            .map(|paths| paths.iter().cloned().collect())
                            .unwrap_or_default(),
                        version: id
                            .strip_prefix("hook:")
                            .and_then(|name| hook_versions.get(name).copied()),
                        reached_from: reached_from
                            .get(id)
                            .into_iter()
                            .flatten()
                            .map(|from| (*from).to_owned())
                            .collect(),
                    },
                    origin,
                    note,
                }
            })
            .collect(),
    }
}

/// How a used item differs in the target build.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Change {
    /// The target has no record with its id.
    Removed,
    /// The target's record has another hash.
    Changed,
    /// The target's type no longer implements this trait.
    LostImpl(String),
    /// The struct's field set changed, and the mod builds it with a full literal at `at`: the
    /// target's fields the mod's list does not name (none when only order or a position
    /// changed).
    Fields { gained: Vec<String>, at: String },
    /// The manifest names version `named` of the hook; the target's is `now`.
    Version { named: u64, now: u64 },
    /// The mod builds the struct with a literal or pattern at `at`, and the target's can no
    /// longer be (a private field, `#[non_exhaustive]`).
    NotBuildable { at: String },
    /// The target no longer exports it under this path (`mod_api::` left out).
    Path(String),
    /// The trait the mod reaches this member through no longer includes the member's owner.
    Route { from: String, owner: String },
}

/// A used item that is not the same in the target build.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Misfit {
    pub id: String,
    pub change: Change,
}

impl std::fmt::Display for Misfit {
    /// `the hook `player_logged_in` (changed)`, `the method `ServerHost::give_item` (removed)`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let item = describe(&self.id);
        match &self.change {
            Change::Removed => write!(f, "{item} (removed)"),
            Change::Changed => write!(f, "{item} (changed)"),
            Change::LostImpl(name) => write!(f, "{item} no longer implements `{name}`"),
            Change::Fields { gained, at } => match &gained[..] {
                [] => write!(
                    f,
                    "the fields of {item} changed, and `{at}` builds it field by field"
                ),
                [one] => write!(
                    f,
                    "{item} gained the field `{one}`, and `{at}` builds it field by field"
                ),
                more => write!(
                    f,
                    "{item} gained the fields {}, and `{at}` builds it field by field",
                    more.iter()
                        .map(|name| format!("`{name}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            },
            Change::Version { named, now } => {
                write!(
                    f,
                    "{item} is now version {now}; this mod names version {named}"
                )
            }
            Change::NotBuildable { at } => write!(
                f,
                "{item} can no longer be built with a literal, and `{at}` builds it"
            ),
            Change::Route { from, owner } => write!(
                f,
                "`{}` no longer includes `{}`, which this mod calls `{}` on",
                leaf(from),
                leaf(owner),
                leaf(&self.id)
            ),
            Change::Path(path) => {
                write!(f, "`mod_api::{path}` is no longer exported under that path")
            }
        }
    }
}

/// What an item is, as people read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Hook,
    /// One argument of a hook.
    Argument,
    Trait,
    Method,
    AssociatedItem,
    Function,
    Macro,
    PartKind,
    Capability,
    Type,
    Variant,
    Field,
}

impl ItemKind {
    /// The English word for it.
    pub fn word(self) -> &'static str {
        match self {
            Self::Hook => "hook",
            Self::Argument => "argument",
            Self::Trait => "trait",
            Self::Method => "method",
            Self::AssociatedItem => "associated item",
            Self::Function => "function",
            Self::Macro => "macro",
            Self::PartKind => "Part kind",
            Self::Capability => "capability",
            Self::Type => "type",
            Self::Variant => "variant",
            Self::Field => "field",
        }
    }
}

/// An id's kind and the name a mod writes for it (`hook:player_logged_in` is the hook
/// `player_logged_in`); `None` for an id of no kind a mod's code can use (a `behaviour:` is never
/// used, so it never shows in a fit report). Programs that show items
/// in other languages build their sentence from this; [`Misfit`]'s text is English.
pub fn item(id: &str) -> Option<(ItemKind, String)> {
    let (kind, rest) = id.split_once(':')?;
    Some(match kind {
        "hook" => match rest.split_once('.') {
            Some((hook, argument)) => (ItemKind::Argument, format!("{hook}.{argument}")),
            None => (ItemKind::Hook, rest.to_owned()),
        },
        "trait" => (ItemKind::Trait, rest.to_owned()),
        "method" | "handle" => (ItemKind::Method, rest.to_owned()),
        "assoc" => (ItemKind::AssociatedItem, rest.to_owned()),
        "fn" => (ItemKind::Function, rest.to_owned()),
        "macro" => (ItemKind::Macro, rest.to_owned()),
        "part" => (ItemKind::PartKind, rest.to_owned()),
        "capability" => (ItemKind::Capability, rest.to_owned()),
        "type" => match rest.rsplit_once('.') {
            // By Rust's naming: a variant is capitalised, a field is not (or is a position).
            Some((owner, member)) if member.starts_with(|c: char| c.is_ascii_uppercase()) => {
                (ItemKind::Variant, format!("{owner}::{member}"))
            }
            Some((owner, member)) => (ItemKind::Field, format!("{owner}.{member}")),
            None => (ItemKind::Type, rest.to_owned()),
        },
        _ => return None,
    })
}

/// An item as people read it: its kind and the name a mod writes for it.
fn describe(id: &str) -> String {
    match item(id) {
        Some((kind, name)) => format!("the {} `{name}`", kind.word()),
        None => format!("`{id}`"),
    }
}

/// Whether a mod's list fits a target build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fit {
    Fits,
    /// What is removed or changed in the target, sorted by id.
    DoesNotFit(Vec<Misfit>),
}

/// Whether every entry exists in `target` with an equal hash and under every path the mod writes,
/// every type keeps its impls, every struct the mod builds can still be built (and keeps its
/// field set when built in full), and every named hook version is unchanged.
pub fn fits<'a>(entries: impl IntoIterator<Item = &'a Entry>, target: &Surface) -> Fit {
    let hashes: BTreeMap<&str, &str> = target.entries().collect();
    let types: BTreeMap<&str, &Type> = target.types.iter().map(|t| (t.id.as_str(), t)).collect();
    let versions: BTreeMap<&str, u64> = target
        .hooks
        .iter()
        .map(|h| (h.id.as_str(), h.contract.version))
        .collect();
    let mut public_paths: BTreeMap<&str, &[String]> = BTreeMap::new();
    for t in &target.traits {
        public_paths.insert(&t.id, &t.paths);
    }
    for t in &target.types {
        public_paths.insert(&t.id, &t.paths);
    }
    for f in &target.functions {
        public_paths.insert(&f.id, &f.paths);
    }
    for m in &target.macros {
        public_paths.insert(&m.id, &m.paths);
    }
    let entries: Vec<&Entry> = entries.into_iter().collect();
    // What each trait a member is reached from leads to in the target, along references built
    // once: a table's `reached_from` strings are many and its own, the surface's size is not.
    let refs = target.refs(true);
    let mut leads: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    let mut misfits = Vec::new();
    for entry in &entries {
        let id = entry.id.as_str();
        let mut misfit = |change| {
            misfits.push(Misfit {
                id: entry.id.clone(),
                change,
            })
        };
        let Some(hash) = hashes.get(id) else {
            misfit(Change::Removed);
            continue;
        };
        if *hash != entry.hash {
            misfit(Change::Changed);
        }
        for path in &entry.paths {
            if !public_paths
                .get(id)
                .is_some_and(|paths| paths.contains(path))
            {
                misfit(Change::Path(path.clone()));
            }
        }
        if let Some(record) = types.get(id) {
            for name in &entry.impls {
                if !record.impls.contains(name) {
                    misfit(Change::LostImpl(name.clone()));
                }
            }
            if matches!(entry.built, Some(Built::Full | Built::Base))
                && !record.contract.constructible
            {
                misfit(Change::NotBuildable {
                    at: entry.built_at.clone().unwrap_or_default(),
                });
            }
            if entry.built == Some(Built::Full) && entry.fields_hash != record.fields_hash {
                let gained = record
                    .fields
                    .iter()
                    .map(|f| leaf(&f.id).to_owned())
                    .filter(|name| !entry.fields.contains(name))
                    .collect();
                misfit(Change::Fields {
                    gained,
                    at: entry.built_at.clone().unwrap_or_default(),
                });
            }
        }
        if let Some(owner) = owner(id, &hashes) {
            for from in &entry.reached_from {
                // A trait the target lacks leads nowhere but to itself.
                let leads_there = if hashes.contains_key(from.as_str()) {
                    leads
                        .entry(from.as_str())
                        .or_insert_with(|| crate::walk_refs(&refs, [from.as_str()]))
                        .contains(&owner)
                } else {
                    *from == owner
                };
                if !leads_there {
                    misfit(Change::Route {
                        from: from.clone(),
                        owner: owner.clone(),
                    });
                }
            }
        }
        if let (Some(named), Some(now)) = (entry.version, versions.get(id))
            && named != *now
        {
            misfit(Change::Version { named, now: *now });
        }
    }
    misfits.sort();
    misfits.dedup();
    if misfits.is_empty() {
        Fit::Fits
    } else {
        Fit::DoesNotFit(misfits)
    }
}

/// The schema [`UsesTable`] writes and reads.
pub const USES_SCHEMA: u32 = 1;

/// The file a mod carries its [`UsesTable`] in, beside its `horizon-mod.toml`.
pub const USES_FILE: &str = "uses.json";

/// The largest table [`UsesTable::read`] reads, in bytes.
pub const MAX_USES_BYTES: usize = 4 << 20;

/// The list a package carries: what the mod uses, with each item's hash at the build it was made
/// for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsesTable {
    pub schema: u32,
    /// The build id (commit) of the surface the hashes come from.
    pub made_for: String,
    pub entries: Vec<Entry>,
}

/// Why a table was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UsesError {
    /// Larger than [`MAX_USES_BYTES`]; the size read.
    TooLarge(usize),
    /// Not JSON, truncated, or a field missing or of the wrong type.
    Json(String),
    /// A schema number this reader does not know.
    Schema(u32),
    /// Two entries with one id.
    DuplicateId(String),
}

impl std::fmt::Display for UsesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge(size) => write!(
                f,
                "the uses table is {size} bytes; the most read is {MAX_USES_BYTES}"
            ),
            Self::Json(error) => write!(f, "the uses table is not valid: {error}"),
            Self::Schema(schema) => write!(
                f,
                "the uses table has schema {schema}; this tool reads schema {USES_SCHEMA}"
            ),
            Self::DuplicateId(id) => write!(f, "the uses table lists {id} twice"),
        }
    }
}

impl std::error::Error for UsesError {}

impl UsesTable {
    /// Reads a table: refuses an oversized document, an unknown schema, malformed JSON and
    /// repeated ids, and unknown fields: the table is this crate's own format, and a field a newer
    /// writer adds is a check this reader would silently skip. Entries come back sorted by id.
    pub fn read(bytes: &[u8]) -> Result<Self, UsesError> {
        #[derive(Deserialize)]
        struct Header {
            schema: u32,
        }
        if bytes.len() > MAX_USES_BYTES {
            return Err(UsesError::TooLarge(bytes.len()));
        }
        let header: Header =
            serde_json::from_slice(bytes).map_err(|e| UsesError::Json(e.to_string()))?;
        if header.schema != USES_SCHEMA {
            return Err(UsesError::Schema(header.schema));
        }
        let mut table: Self =
            serde_json::from_slice(bytes).map_err(|e| UsesError::Json(e.to_string()))?;
        table.entries.sort();
        if let Some(pair) = table
            .entries
            .windows(2)
            .find(|pair| pair[0].id == pair[1].id)
        {
            return Err(UsesError::DuplicateId(pair[0].id.clone()));
        }
        Ok(table)
    }

    /// The table as written: pretty JSON, entries sorted by id, ending in a newline.
    pub fn to_json(&self) -> String {
        let mut sorted = self.clone();
        sorted.entries.sort();
        let mut text = serde_json::to_string_pretty(&sorted).expect("a uses table serializes");
        text.push('\n');
        text
    }
}
