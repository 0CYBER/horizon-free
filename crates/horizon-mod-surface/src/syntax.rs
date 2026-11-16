//! What the parser reads of one source file for [`crate::uses`]: where structs are built and
//! matched, which paths are written, which impl each `Self` stands in, what the file defines,
//! and the byte ranges of macro bodies, whose tokens the parser does not see into.
use crate::scan::Token;
use std::collections::BTreeSet;
use std::ops::Range;
use syn::visit::{self, Visit};

/// How a struct is built or matched at one place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum How {
    /// A literal or pattern that lists every field: `S { a, b }`, `S(a, b)`.
    Full,
    /// A literal that ends in `..` or a base: `S { a, ..d }`.
    Base,
    /// A name used as a value or bound in a pattern by itself (`.map(Pos)`, `Marker`): a build
    /// only for a tuple or unit struct.
    Value,
}

/// A place a struct may be built: the path written, as names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Build<'t> {
    /// The path's names; `Self` for `Self { … }`.
    pub path: Vec<&'t str>,
    /// Written with a leading `::`.
    pub rooted: bool,
    pub how: How,
    /// A brace literal (`S { … }`), for the `..Default::default()` advice.
    pub brace: bool,
    pub line: usize,
    /// The byte where the path starts.
    pub byte: usize,
}

/// An `impl` block: the bytes of its body, and the path of the type it is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Impl<'t> {
    pub body: Range<usize>,
    /// Its names, an empty first one for a leading `::`; `None` when the type is not a plain
    /// path the scan can read (a tuple, a macro), so what `Self` means there is unknown.
    pub target: Option<Vec<&'t str>>,
}

/// What the parser reads of one file.
#[derive(Debug, Default)]
pub(crate) struct Syntax<'t> {
    pub builds: Vec<Build<'t>>,
    /// Every path written in code, attributes included, `use` trees left out: its names.
    pub paths: Vec<Vec<&'t str>>,
    pub impls: Vec<Impl<'t>>,
    /// The structs, enums, unions and type aliases the file defines at its top level, under no
    /// `#[cfg]` or `#[cfg_attr]`.
    pub defines: BTreeSet<&'t str>,
    /// Every struct, enum and union defined anywhere, and every enum variant.
    pub types: BTreeSet<&'t str>,
    /// Every trait defined anywhere.
    pub traits: BTreeSet<&'t str>,
    /// `type X = …Y<…>;` anywhere: (X, Y), and whether it is an item of its own (not an
    /// associated type of an impl or trait).
    pub type_aliases: Vec<(&'t str, &'t str, bool)>,
    /// The file holds an inline `mod name { … }`, so it is more than one scope.
    pub inline_modules: bool,
    /// Every module the file declares, inline or not (`mod api;`).
    pub modules: BTreeSet<&'t str>,
    /// The bytes inside each macro call's delimiters, and each `macro_rules!` body.
    pub macro_bodies: Vec<Range<usize>>,
}

/// The path a type is, through parentheses and invisible groups; `None` for any other type.
fn plain_path(ty: &syn::Type) -> Option<&syn::Path> {
    match ty {
        syn::Type::Path(t) if t.qself.is_none() => Some(&t.path),
        syn::Type::Paren(t) => plain_path(&t.elem),
        syn::Type::Group(t) => plain_path(&t.elem),
        _ => None,
    }
}

/// Whether an item carries `#[cfg(…)]` or `#[cfg_attr(…)]`.
fn conditional(attrs: &[syn::Attribute]) -> bool {
    attrs
        .iter()
        .any(|a| a.path().is_ident("cfg") || a.path().is_ident("cfg_attr"))
}

struct Reader<'t> {
    text: &'t str,
    out: Syntax<'t>,
}

impl<'t> Reader<'t> {
    /// An identifier's name as written in the text, `r#` left out.
    fn name(&self, ident: &syn::Ident) -> &'t str {
        let written = self.text.get(ident.span().byte_range()).unwrap_or("");
        written.strip_prefix("r#").unwrap_or(written)
    }

    fn names(&self, path: &syn::Path) -> Vec<&'t str> {
        path.segments.iter().map(|s| self.name(&s.ident)).collect()
    }

    fn alias(&mut self, alias: &syn::Ident, ty: &syn::Type, item: bool) {
        // An alias of a type the scan cannot read is not recorded, so a literal of its name is
        // of a name the scan cannot place.
        let alias = self.name(alias);
        if let Some(last) = plain_path(ty).and_then(|path| path.segments.last()) {
            let target = self.name(&last.ident);
            self.out.type_aliases.push((alias, target, item));
        }
    }

    fn build(&mut self, path: &syn::Path, how: How, brace: bool) {
        let span = match (&path.leading_colon, path.segments.first()) {
            (Some(colons), _) => colons.spans[0],
            (None, Some(first)) => first.ident.span(),
            (None, None) => return,
        };
        self.out.builds.push(Build {
            path: self.names(path),
            rooted: path.leading_colon.is_some(),
            how,
            brace,
            line: span.start().line,
            byte: span.byte_range().start,
        });
    }
}

impl<'a, 't> Visit<'a> for Reader<'t> {
    fn visit_expr_struct(&mut self, e: &'a syn::ExprStruct) {
        let how = if e.dot2_token.is_some() || e.rest.is_some() {
            How::Base
        } else {
            How::Full
        };
        self.build(&e.path, how, true);
        visit::visit_expr_struct(self, e);
    }

    fn visit_expr_call(&mut self, e: &'a syn::ExprCall) {
        if let syn::Expr::Path(p) = &*e.func {
            self.build(&p.path, How::Full, false);
        }
        visit::visit_expr_call(self, e);
    }

    fn visit_expr_path(&mut self, e: &'a syn::ExprPath) {
        // A value (and in a pattern, `Pat::Path`): a tuple struct's constructor, a unit struct.
        self.build(&e.path, How::Value, false);
        visit::visit_expr_path(self, e);
    }

    fn visit_pat_struct(&mut self, p: &'a syn::PatStruct) {
        // With `..` a pattern reads the fields it names; without, it lists them all.
        if p.rest.is_none() {
            self.build(&p.path, How::Full, false);
        }
        visit::visit_pat_struct(self, p);
    }

    fn visit_pat_tuple_struct(&mut self, p: &'a syn::PatTupleStruct) {
        if !p.elems.iter().any(|e| matches!(e, syn::Pat::Rest(_))) {
            self.build(&p.path, How::Full, false);
        }
        visit::visit_pat_tuple_struct(self, p);
    }

    fn visit_pat_ident(&mut self, p: &'a syn::PatIdent) {
        // `Marker` alone in a pattern parses as a binding; it may be a unit struct.
        if p.subpat.is_none() && p.by_ref.is_none() && p.mutability.is_none() {
            self.build(&syn::Path::from(p.ident.clone()), How::Value, false);
        }
        visit::visit_pat_ident(self, p);
    }

    fn visit_path(&mut self, p: &'a syn::Path) {
        let names = self.names(p);
        self.out.paths.push(names);
        visit::visit_path(self, p);
    }

    fn visit_item_impl(&mut self, i: &'a syn::ItemImpl) {
        let target = plain_path(&i.self_ty).map(|path| {
            let mut target = self.names(path);
            if path.leading_colon.is_some() {
                target.insert(0, "");
            }
            target
        });
        self.out.impls.push(Impl {
            body: i.brace_token.span.join().byte_range(),
            target,
        });
        visit::visit_item_impl(self, i);
    }

    fn visit_item_mod(&mut self, m: &'a syn::ItemMod) {
        self.out.inline_modules |= m.content.is_some();
        let name = self.name(&m.ident);
        self.out.modules.insert(name);
        visit::visit_item_mod(self, m);
    }

    fn visit_item_struct(&mut self, i: &'a syn::ItemStruct) {
        let name = self.name(&i.ident);
        self.out.types.insert(name);
        visit::visit_item_struct(self, i);
    }

    fn visit_item_enum(&mut self, i: &'a syn::ItemEnum) {
        let name = self.name(&i.ident);
        self.out.types.insert(name);
        for variant in &i.variants {
            let name = self.name(&variant.ident);
            self.out.types.insert(name);
        }
        visit::visit_item_enum(self, i);
    }

    fn visit_item_union(&mut self, i: &'a syn::ItemUnion) {
        let name = self.name(&i.ident);
        self.out.types.insert(name);
        visit::visit_item_union(self, i);
    }

    fn visit_item_trait(&mut self, i: &'a syn::ItemTrait) {
        let name = self.name(&i.ident);
        self.out.traits.insert(name);
        visit::visit_item_trait(self, i);
    }

    fn visit_item_type(&mut self, i: &'a syn::ItemType) {
        self.alias(&i.ident, &i.ty, true);
        visit::visit_item_type(self, i);
    }

    fn visit_impl_item_type(&mut self, i: &'a syn::ImplItemType) {
        self.alias(&i.ident, &i.ty, false);
        visit::visit_impl_item_type(self, i);
    }

    fn visit_macro(&mut self, m: &'a syn::Macro) {
        let span = match &m.delimiter {
            syn::MacroDelimiter::Paren(d) => d.span.join(),
            syn::MacroDelimiter::Brace(d) => d.span.join(),
            syn::MacroDelimiter::Bracket(d) => d.span.join(),
        };
        self.out.macro_bodies.push(span.byte_range());
        visit::visit_macro(self, m);
    }
}

/// The deepest the parser is let recurse, counted by [`too_deep`].
pub(crate) const MAX_NESTING: usize = 256;

/// Keywords after which a token starts an operand rather than continuing one.
pub(crate) const KEYWORDS: &[&str] = &[
    "as", "async", "box", "break", "const", "continue", "dyn", "else", "enum", "extern", "fn",
    "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
    "return", "static", "struct", "trait", "type", "unsafe", "use", "where", "while", "yield",
];

/// Keywords the parser reads what follows by recursion: `return return x`, `fn() -> fn()`,
/// `if … else if …`.
const RECURSIVE: &[&str] = &[
    "async", "box", "break", "dyn", "fn", "if", "impl", "let", "move", "ref", "return", "yield",
];

/// What the nesting count is inside: a bracket, a generic `<`, or a closure's parameters.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Frame {
    Bracket,
    Angle,
    /// A `<` straight after a literal, `)`, `]` or `?`, which compares or shifts, unless a type
    /// follows it: counted like a generic, but ended by a `,` like an expression.
    Compare,
    Params,
}

/// The first line where the parser would recurse deeper than [`MAX_NESTING`].
///
/// The count is taken along the path of nesting. Each open bracket, generic `<` and closure
/// counts until it closes, and so does each assignment, `@` and keyword the parser reads what
/// follows by recursion. A run of prefix operators counts until its operand ends, through
/// method calls and fields (`-x.f(…)`), and no further than the bracket after it. A count is
/// dropped only where the parser has certainly returned: at the bracket that encloses it
/// closing, at a `;`, at a match arm's `=>`, at a `}` that ends a statement, and at a `,`
/// directly inside a bracket (between elements, arms or fields). A `,` inside a generic or a
/// closure's parameters drops nothing, since generic arguments and parameters are listed with
/// commas. A `<` straight after a literal, `)`, `]` or `?` counts like any other, but a `,`
/// ends it, as it ends the comparison or shift it most likely is (`[1 << 0, 1 << 1]`).
///
/// This is a cheap filter, not the guarantee: the scan's parse runs in a child process first
/// ([`crate::child`]), so a parse that still runs out of stack ends the child, not the caller.
pub(crate) fn too_deep(tokens: &[(Token<'_>, usize)]) -> Option<usize> {
    let token = |at: usize| tokens.get(at).map(|(t, _)| *t);
    let punct = |at: usize, byte: u8| token(at) == Some(Token::Punct(byte));
    // Whether the token before `at` ends an operand, so an operator at `at` is binary.
    let operand_end = |at: usize| match at.checked_sub(1).and_then(token) {
        Some(Token::Literal(_) | Token::Punct(b')' | b']' | b'}' | b'?')) => true,
        Some(Token::Ident(name)) => !KEYWORDS.contains(&name),
        _ => false,
    };
    let mut frames: Vec<(usize, Frame)> = Vec::new();
    // The count that stays until a drop, and the prefix operators of the current operand.
    let (mut depth, mut run) = (0usize, 0usize);
    let mut skip = false;
    // Drops the generics and closure parameters above the innermost bracket, and the count of
    // the statement inside it.
    let end_statement = |frames: &mut Vec<(usize, Frame)>| {
        while frames
            .last()
            .is_some_and(|(_, frame)| *frame != Frame::Bracket)
        {
            frames.pop();
        }
        frames.last().map_or(0, |(before, _)| before + 1)
    };
    for (at, (current, line)) in tokens.iter().enumerate() {
        if skip {
            skip = false;
            continue;
        }
        match current {
            // A prefix operator's count ends where the bracket after it closes.
            Token::Punct(b'(' | b'[' | b'{') => {
                frames.push((depth, Frame::Bracket));
                depth += run + 1;
                run = 0;
            }
            Token::Punct(b')' | b']' | b'}') => {
                while let Some((before, frame)) = frames.pop() {
                    depth = before;
                    if frame == Frame::Bracket {
                        break;
                    }
                }
                run = 0;
                // A block that ends its statement: the next one starts afresh.
                if punct(at, b'}')
                    && match token(at + 1) {
                        None | Some(Token::Punct(b'}' | b';' | b',' | b'#' | b')' | b']')) => true,
                        Some(Token::Ident(next)) => !matches!(next, "else" | "as" | "in"),
                        _ => false,
                    }
                {
                    depth = end_statement(&mut frames);
                }
            }
            // A match arm's pattern and guard end at `=>`, generics and comparisons with them.
            Token::Punct(b';') | Token::Punct(b'=') if punct(at, b';') || punct(at + 1, b'>') => {
                depth = end_statement(&mut frames);
                run = 0;
            }
            Token::Punct(b',') => {
                while let Some((before, Frame::Compare)) = frames.last().copied() {
                    frames.pop();
                    depth = before;
                }
                if frames
                    .last()
                    .is_none_or(|(_, frame)| *frame == Frame::Bracket)
                {
                    depth = end_statement(&mut frames);
                }
                run = 0;
            }
            Token::Punct(b'<') if punct(at + 1, b'=') => run = 0,
            Token::Punct(b'<') => {
                // The second `<` of a shift (`1 << 2`) compares too.
                let compare = match at.checked_sub(1).and_then(token) {
                    Some(Token::Literal(_) | Token::Punct(b')' | b']' | b'?')) => true,
                    Some(Token::Punct(b'<')) => {
                        matches!(frames.last(), Some((_, Frame::Compare)))
                    }
                    _ => false,
                };
                let frame = if compare {
                    Frame::Compare
                } else {
                    Frame::Angle
                };
                frames.push((depth + run, frame));
                depth += run + 1;
                run = 0;
            }
            Token::Punct(b'>') => {
                // Not `->`, `=>` or `>=`: the close of the innermost generic, if one is open.
                if !(at >= 1 && (punct(at - 1, b'-') || punct(at - 1, b'=')))
                    && !punct(at + 1, b'=')
                    && let Some((before, Frame::Angle | Frame::Compare)) = frames.last().copied()
                {
                    frames.pop();
                    depth = before;
                }
                run = 0;
            }
            Token::Punct(b'|') => {
                if frames
                    .last()
                    .is_some_and(|(_, frame)| *frame == Frame::Params)
                {
                    // The parameters end; the closure's body follows.
                    if let Some((before, _)) = frames.pop() {
                        depth = before + 1;
                    }
                } else if operand_end(at) {
                    // A binary `|` or `||`.
                    skip = punct(at + 1, b'|');
                } else if punct(at + 1, b'|') {
                    // A closure with no parameters.
                    depth += run + 1;
                    skip = true;
                } else {
                    frames.push((depth + run, Frame::Params));
                    depth += run + 1;
                }
                run = 0;
            }
            Token::Punct(b'=')
                if !punct(at + 1, b'=')
                    && !punct(at + 1, b'>')
                    && !(at >= 1
                        && matches!(
                            token(at - 1),
                            Some(Token::Punct(b'=' | b'!' | b'<' | b'>'))
                        )) =>
            {
                depth += 1;
                run = 0;
            }
            Token::Punct(b'@') => {
                depth += 1;
                run = 0;
            }
            Token::Punct(b'-' | b'!' | b'*' | b'&' | b'.') if !operand_end(at) => run += 1,
            // A path's `::`, and a method call's or field's `.`, continue the operand.
            Token::Punct(b':' | b'.') => {}
            Token::Punct(_) => run = 0,
            Token::Ident(name) if RECURSIVE.contains(name) => depth += 1,
            _ => {}
        }
        if depth + run > MAX_NESTING {
            return Some(*line);
        }
    }
    None
}

/// The stack the parser runs on. Measured on 20 shapes nested as deep as [`too_deep`] accepts
/// (brackets, generics with and without commas, closures with one and two parameters,
/// references, `fn` types, assignments, `return`, `@`, `else if`, qualified paths after a
/// comparison, prefix operators on method calls, mixes of them), the deepest
/// needs 1.6 MiB in a release build and 1.5 MiB in a test build; this leaves a tenfold margin.
pub(crate) const PARSER_STACK: usize = 16 << 20;

/// Why a file was not read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unread {
    /// It does not parse, at this line.
    Parse(usize),
    /// The parser's own stack could not be had, so parsing would risk the caller's: always on
    /// wasm, which has no threads.
    NoStack,
}

/// Parses `text` and reads it, on a thread with a stack of its own.
pub(crate) fn read(text: &str) -> Result<Syntax<'_>, Unread> {
    // The parser reads nesting by recursion. [`too_deep`] bounds it, and a stack of its own
    // leaves room for any file within that bound, wherever the scan is called from. Without one
    // the file is not parsed at all.
    #[cfg(not(target_family = "wasm"))]
    {
        std::thread::scope(|scope| {
            match std::thread::Builder::new()
                .stack_size(PARSER_STACK)
                .spawn_scoped(scope, || parse(text))
            {
                Ok(thread) => thread.join().unwrap_or(Err(Unread::NoStack)),
                Err(_) => Err(Unread::NoStack),
            }
        })
    }
    #[cfg(target_family = "wasm")]
    {
        let _ = text;
        Err(Unread::NoStack)
    }
}

pub(crate) fn parse(text: &str) -> Result<Syntax<'_>, Unread> {
    // The parser skips a byte-order mark and a shebang line itself, which would shift every
    // byte it reports; blank them in a copy of the same length instead.
    let mut copy = text.to_owned();
    if copy.starts_with('\u{feff}') {
        copy.replace_range(..3, "   ");
    }
    if copy.starts_with("#!") && !copy.starts_with("#![") {
        let end = copy.find('\n').unwrap_or(copy.len());
        copy.replace_range(..end, &" ".repeat(end));
    }
    let parsed = syn::parse_str::<syn::File>(&copy);
    let result = match parsed {
        Ok(file) => {
            let mut reader = Reader {
                text,
                out: Syntax::default(),
            };
            for item in &file.items {
                let defined = match item {
                    syn::Item::Struct(i) if !conditional(&i.attrs) => Some(&i.ident),
                    syn::Item::Enum(i) if !conditional(&i.attrs) => Some(&i.ident),
                    syn::Item::Union(i) if !conditional(&i.attrs) => Some(&i.ident),
                    // An alias defines its name only when the scan can read what it stands for.
                    syn::Item::Type(i) if !conditional(&i.attrs) && plain_path(&i.ty).is_some() => {
                        Some(&i.ident)
                    }
                    _ => None,
                };
                if let Some(ident) = defined {
                    let name = reader.name(ident);
                    reader.out.defines.insert(name);
                }
            }
            reader.visit_file(&file);
            Ok(reader.out)
        }
        Err(error) => Err(Unread::Parse(error.span().start().line.max(1))),
    };
    // The parser keeps every text it read for its spans until told to forget them.
    proc_macro2::extra::invalidate_current_thread_spans();
    result
}
