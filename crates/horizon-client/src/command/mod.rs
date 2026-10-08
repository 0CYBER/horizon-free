//! A port of Brigadier (the command library vanilla uses): a string reader,
//! literal/argument node trees with redirects, parsing that tries every
//! relevant child and keeps the best result, completion suggestions, smart
//! usage text and the vanilla error messages. Argument types live in
//! [`args`]; the vanilla command tree in [`vanilla`].
pub mod args;
pub mod vanilla;

use crate::text::Text;
pub use args::{Arg, ArgType, Registries};

/// A command error (CommandSyntaxException): the message and, for parse
/// errors, the input and cursor it occurred at.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandError {
    pub message: Text,
    pub input: Option<String>,
    pub cursor: usize,
    /// BuiltInExceptions.literalIncorrect (summarised specially by the chat UI).
    pub literal_incorrect: bool,
}
impl CommandError {
    pub fn new(message: Text) -> Self {
        Self {
            message,
            input: None,
            cursor: 0,
            literal_incorrect: false,
        }
    }
    pub fn tr(key: &str, args: &[&dyn std::fmt::Display]) -> Self {
        Self::new(Text::tr(key, args))
    }
    pub fn at(message: Text, reader: &Reader) -> Self {
        Self {
            message,
            input: Some(reader.string.to_owned()),
            cursor: reader.cursor,
            literal_incorrect: false,
        }
    }
    pub fn tr_at(key: &str, args: &[&dyn std::fmt::Display], reader: &Reader) -> Self {
        Self::at(Text::tr(key, args), reader)
    }
    /// CommandSyntaxException.getContext: up to ten characters before the
    /// cursor followed by "<--[HERE]".
    pub fn context(&self) -> Option<String> {
        let input = self.input.as_ref()?;
        let cursor = self.cursor.min(input.len());
        let start = (0..=cursor.saturating_sub(10))
            .rev()
            .find(|&i| input.is_char_boundary(i))
            .unwrap_or(0);
        let mut out = String::new();
        if cursor > 10 {
            out.push_str("...");
        }
        out.push_str(&input[start..cursor]);
        out.push_str("<--[HERE]");
        Some(out)
    }
    /// The message as shown by command suggestions ("... at position N: ...").
    pub fn with_context(&self) -> Text {
        match self.context() {
            Some(context) => Text::translate(
                "command.context.parse_error",
                &[
                    self.message.clone(),
                    Text::literal(self.cursor.to_string()),
                    Text::literal(context),
                ],
            ),
            None => self.message.clone(),
        }
    }
}
pub type CmdResult<T> = Result<T, CommandError>;

/// Brigadier StringReader over a command line (byte cursor on char boundaries).
#[derive(Clone, Debug)]
pub struct Reader<'a> {
    pub string: &'a str,
    pub cursor: usize,
}
impl<'a> Reader<'a> {
    pub fn new(string: &'a str) -> Self {
        Self { string, cursor: 0 }
    }
    pub fn can_read(&self) -> bool {
        self.cursor < self.string.len()
    }
    pub fn can_read_n(&self, n: usize) -> bool {
        self.string[self.cursor..].chars().take(n).count() == n
    }
    pub fn peek(&self) -> char {
        self.string[self.cursor..].chars().next().unwrap_or('\0')
    }
    pub fn peek_at(&self, offset: usize) -> char {
        self.string[self.cursor..]
            .chars()
            .nth(offset)
            .unwrap_or('\0')
    }
    pub fn skip(&mut self) {
        self.cursor += self.peek().len_utf8();
    }
    pub fn read(&mut self) -> char {
        let c = self.peek();
        self.skip();
        c
    }
    pub fn remaining(&self) -> &'a str {
        &self.string[self.cursor..]
    }
    pub fn skip_whitespace(&mut self) {
        while self.can_read() && self.peek().is_whitespace() {
            self.skip();
        }
    }
    pub fn is_allowed_number(c: char) -> bool {
        c.is_ascii_digit() || c == '.' || c == '-'
    }
    pub fn is_allowed_in_unquoted(c: char) -> bool {
        c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+')
    }
    fn read_number_text(&mut self) -> &'a str {
        let start = self.cursor;
        while self.can_read() && Self::is_allowed_number(self.peek()) {
            self.skip();
        }
        &self.string[start..self.cursor]
    }
    fn number<T: std::str::FromStr>(&mut self, kind: &str) -> CmdResult<T> {
        let start = self.cursor;
        let text = self.read_number_text();
        if text.is_empty() {
            return Err(CommandError::tr_at(
                &format!("parsing.{kind}.expected"),
                &[],
                self,
            ));
        }
        // Java's parsers reject a trailing '.'-less exponent etc. the same way.
        match text.parse::<T>() {
            Ok(v) if !(kind == "int" || kind == "long") || !text.contains('.') => Ok(v),
            _ => {
                self.cursor = start;
                Err(CommandError::tr_at(
                    &format!("parsing.{kind}.invalid"),
                    &[&text],
                    self,
                ))
            }
        }
    }
    pub fn read_int(&mut self) -> CmdResult<i32> {
        self.number("int")
    }
    pub fn read_long(&mut self) -> CmdResult<i64> {
        self.number("long")
    }
    pub fn read_double(&mut self) -> CmdResult<f64> {
        self.number("double")
    }
    pub fn read_float(&mut self) -> CmdResult<f32> {
        self.number("float")
    }
    pub fn read_unquoted_string(&mut self) -> &'a str {
        let start = self.cursor;
        while self.can_read() && Self::is_allowed_in_unquoted(self.peek()) {
            self.skip();
        }
        &self.string[start..self.cursor]
    }
    pub fn read_string_until(&mut self, terminator: char) -> CmdResult<String> {
        let mut out = String::new();
        let mut escaped = false;
        while self.can_read() {
            let c = self.read();
            if escaped {
                if c == terminator || c == '\\' {
                    out.push(c);
                    escaped = false;
                } else {
                    self.cursor -= c.len_utf8();
                    return Err(CommandError::tr_at("parsing.quote.escape", &[&c], self));
                }
            } else if c == '\\' {
                escaped = true;
            } else if c == terminator {
                return Ok(out);
            } else {
                out.push(c);
            }
        }
        Err(CommandError::tr_at("parsing.quote.expected.end", &[], self))
    }
    pub fn read_quoted_string(&mut self) -> CmdResult<String> {
        if !self.can_read() {
            return Ok(String::new());
        }
        let next = self.peek();
        if next != '"' && next != '\'' {
            return Err(CommandError::tr_at(
                "parsing.quote.expected.start",
                &[],
                self,
            ));
        }
        self.skip();
        self.read_string_until(next)
    }
    pub fn read_string(&mut self) -> CmdResult<String> {
        if !self.can_read() {
            return Ok(String::new());
        }
        let next = self.peek();
        if next == '"' || next == '\'' {
            self.skip();
            return self.read_string_until(next);
        }
        Ok(self.read_unquoted_string().to_owned())
    }
    pub fn read_boolean(&mut self) -> CmdResult<bool> {
        let start = self.cursor;
        let value = self.read_string()?;
        if value.is_empty() {
            return Err(CommandError::tr_at("parsing.bool.expected", &[], self));
        }
        match value.as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => {
                self.cursor = start;
                Err(CommandError::tr_at("parsing.bool.invalid", &[&value], self))
            }
        }
    }
    pub fn expect(&mut self, c: char) -> CmdResult<()> {
        if !self.can_read() || self.peek() != c {
            return Err(CommandError::tr_at("parsing.expected", &[&c], self));
        }
        self.skip();
        Ok(())
    }
}

pub type Range = (usize, usize);
#[derive(Clone, Debug)]
pub enum NodeKind {
    Root,
    Literal(String),
    Argument { name: String, ty: ArgType },
}
#[derive(Clone, Debug)]
pub struct Node {
    pub kind: NodeKind,
    pub children: Vec<usize>,
    /// The command id run when parsing ends here.
    pub command: Option<&'static str>,
    pub redirect: Option<usize>,
    /// Required permission level (0 everyone, 2 game masters, 4 owners).
    pub permission: u8,
}
impl Node {
    pub fn name(&self) -> &str {
        match &self.kind {
            NodeKind::Root => "",
            NodeKind::Literal(l) => l,
            NodeKind::Argument { name, .. } => name,
        }
    }
    /// CommandNode.getUsageText.
    pub fn usage_text(&self) -> String {
        match &self.kind {
            NodeKind::Root => String::new(),
            NodeKind::Literal(l) => l.clone(),
            NodeKind::Argument { name, .. } => format!("<{name}>"),
        }
    }
}
#[derive(Clone, Debug)]
pub struct ParsedArg {
    pub name: String,
    pub value: Arg,
    pub range: Range,
}
/// CommandContextBuilder: the nodes and arguments parsed so far.
#[derive(Clone, Debug)]
pub struct Context {
    pub root: usize,
    pub nodes: Vec<(usize, Range)>,
    pub args: Vec<ParsedArg>,
    pub command: Option<&'static str>,
    pub range: Range,
    pub child: Option<Box<Context>>,
    /// Literal words parsed so far (mode keywords such as `masked`).
    pub literals: Vec<String>,
}
impl Context {
    fn new(root: usize, start: usize) -> Self {
        Self {
            root,
            nodes: Vec::new(),
            args: Vec::new(),
            command: None,
            range: (start, start),
            child: None,
            literals: Vec::new(),
        }
    }
    /// Whether the literal `word` was parsed (in this context).
    pub fn has(&self, word: &str) -> bool {
        self.literals.iter().any(|l| l == word)
    }
    fn with_node(&mut self, node: usize, range: Range) {
        self.nodes.push((node, range));
        self.range = (self.range.0.min(range.0), self.range.1.max(range.1));
    }
    pub fn last_child(&self) -> &Context {
        let mut c = self;
        while let Some(child) = &c.child {
            c = child;
        }
        c
    }
    pub fn get(&self, name: &str) -> Option<&Arg> {
        self.args
            .iter()
            .rev()
            .find(|a| a.name == name)
            .map(|a| &a.value)
    }
    /// CommandContextBuilder.findSuggestionContext: the node whose children
    /// complete the text at `cursor`, and where that text starts.
    pub fn suggestion_context(&self, cursor: usize) -> (usize, usize, &Context) {
        if self.range.1 < cursor {
            if let Some(child) = &self.child {
                return child.suggestion_context(cursor);
            }
            if let Some((node, range)) = self.nodes.last() {
                return (*node, range.1 + 1, self);
            }
            return (self.root, self.range.0, self);
        }
        let mut prev = self.root;
        for (node, range) in &self.nodes {
            if range.0 <= cursor && cursor <= range.1 {
                return (prev, range.0, self);
            }
            prev = *node;
        }
        (prev, self.range.0, self)
    }
}
#[derive(Clone, Debug)]
pub struct ParseResults {
    pub context: Context,
    pub input: String,
    pub cursor: usize,
    pub errors: Vec<(usize, CommandError)>,
}
impl ParseResults {
    pub fn can_read(&self) -> bool {
        self.cursor < self.input.len()
    }
    /// Commands.getParseException.
    pub fn error(&self) -> Option<CommandError> {
        if !self.can_read() {
            return None;
        }
        if self.errors.len() == 1 {
            return Some(self.errors[0].1.clone());
        }
        let reader = Reader {
            string: &self.input,
            cursor: self.cursor,
        };
        Some(if self.context.range.0 == self.context.range.1 {
            CommandError::tr_at("command.unknown.command", &[], &reader)
        } else {
            CommandError::tr_at("command.unknown.argument", &[], &reader)
        })
    }
    /// The executable context and command id, or the vanilla error.
    pub fn executable(&self) -> CmdResult<(&Context, &'static str)> {
        if let Some(e) = self.error() {
            return Err(e);
        }
        let last = self.context.last_child();
        match last.command {
            Some(id) => Ok((last, id)),
            None => Err(CommandError::tr_at(
                "command.unknown.command",
                &[],
                &Reader {
                    string: &self.input,
                    cursor: self.cursor,
                },
            )),
        }
    }
}
/// A completion: replace `range` of the input with `text`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub range: Range,
    pub text: String,
    pub tooltip: Option<String>,
}
impl Suggestion {
    /// Suggestion.apply.
    pub fn apply(&self, input: &str) -> String {
        let (s, e) = (self.range.0.min(input.len()), self.range.1.min(input.len()));
        format!("{}{}{}", &input[..s], self.text, &input[e..])
    }
}
/// SuggestionsBuilder: suggestions for the text from `start` to the cursor.
pub struct SuggestionsBuilder<'a> {
    pub input: &'a str,
    pub start: usize,
    pub out: Vec<Suggestion>,
}
impl<'a> SuggestionsBuilder<'a> {
    pub fn new(input: &'a str, start: usize) -> Self {
        Self {
            input,
            start: start.min(input.len()),
            out: Vec::new(),
        }
    }
    pub fn remaining(&self) -> &'a str {
        &self.input[self.start..]
    }
    pub fn remaining_lower(&self) -> String {
        self.remaining().to_lowercase()
    }
    pub fn suggest(&mut self, text: impl Into<String>) {
        let text = text.into();
        if text != self.remaining() {
            self.out.push(Suggestion {
                range: (self.start, self.input.len()),
                text,
                tooltip: None,
            });
        }
    }
    pub fn suggest_tooltip(&mut self, text: impl Into<String>, tooltip: impl Into<String>) {
        let text = text.into();
        if text != self.remaining() {
            self.out.push(Suggestion {
                range: (self.start, self.input.len()),
                text,
                tooltip: Some(tooltip.into()),
            });
        }
    }
    pub fn offset(&self, start: usize) -> SuggestionsBuilder<'a> {
        SuggestionsBuilder::new(self.input, start)
    }
    /// SharedSuggestionProvider.suggest: candidates starting with the remaining text.
    pub fn suggest_matching<'b>(&mut self, candidates: impl IntoIterator<Item = &'b str>) {
        let lower = self.remaining_lower();
        for c in candidates {
            if c.to_lowercase().starts_with(&lower) {
                self.suggest(c);
            }
        }
    }
}
/// Suggestions.create: merged, expanded to a common range and sorted case-insensitively.
pub fn merge_suggestions(input: &str, list: Vec<Suggestion>) -> (Range, Vec<Suggestion>) {
    if list.is_empty() {
        return ((0, 0), Vec::new());
    }
    let start = list.iter().map(|s| s.range.0).min().unwrap_or(0);
    let end = list.iter().map(|s| s.range.1).max().unwrap_or(0);
    let mut out: Vec<Suggestion> = Vec::new();
    for s in list {
        // Suggestion.expand: pad the text with the input around it.
        let text = format!(
            "{}{}{}",
            &input[start..s.range.0],
            s.text,
            &input[s.range.1..end]
        );
        let expanded = Suggestion {
            range: (start, end),
            text,
            tooltip: s.tooltip,
        };
        if !out.iter().any(|o| o.text == expanded.text) {
            out.push(expanded);
        }
    }
    out.sort_by_key(|a| a.text.to_lowercase());
    ((start, end), out)
}

/// The command tree (CommandDispatcher). Node 0 is the root.
#[derive(Clone, Debug)]
pub struct Dispatcher {
    pub nodes: Vec<Node>,
}
impl Default for Dispatcher {
    fn default() -> Self {
        Self::new()
    }
}
impl Dispatcher {
    pub fn new() -> Self {
        Self {
            nodes: vec![Node {
                kind: NodeKind::Root,
                children: Vec::new(),
                command: None,
                redirect: None,
                permission: 0,
            }],
        }
    }
    fn add(&mut self, parent: usize, kind: NodeKind) -> usize {
        let name = match &kind {
            NodeKind::Literal(l) => l.clone(),
            NodeKind::Argument { name, .. } => name.clone(),
            NodeKind::Root => String::new(),
        };
        // CommandNode.addChild merges same-named children.
        if let Some(&existing) = self.nodes[parent]
            .children
            .iter()
            .find(|&&c| self.nodes[c].name() == name)
        {
            return existing;
        }
        let permission = self.nodes[parent].permission;
        self.nodes.push(Node {
            kind,
            children: Vec::new(),
            command: None,
            redirect: None,
            permission,
        });
        let id = self.nodes.len() - 1;
        self.nodes[parent].children.push(id);
        id
    }
    pub fn literal(&mut self, parent: usize, name: &str) -> usize {
        self.add(parent, NodeKind::Literal(name.to_owned()))
    }
    pub fn argument(&mut self, parent: usize, name: &str, ty: ArgType) -> usize {
        self.add(
            parent,
            NodeKind::Argument {
                name: name.to_owned(),
                ty,
            },
        )
    }
    pub fn executes(&mut self, node: usize, command: &'static str) -> usize {
        self.nodes[node].command = Some(command);
        node
    }
    pub fn redirect(&mut self, node: usize, target: usize) -> usize {
        self.nodes[node].redirect = Some(target);
        node
    }
    pub fn requires(&mut self, node: usize, level: u8) -> usize {
        self.nodes[node].permission = level;
        node
    }
    /// A root literal by name.
    pub fn find(&self, name: &str) -> Option<usize> {
        self.nodes[0]
            .children
            .iter()
            .copied()
            .find(|&c| self.nodes[c].name() == name)
    }
    fn relevant(&self, node: usize, reader: &Reader) -> Vec<usize> {
        let children = &self.nodes[node].children;
        let literals: Vec<usize> = children
            .iter()
            .copied()
            .filter(|&c| matches!(self.nodes[c].kind, NodeKind::Literal(_)))
            .collect();
        let arguments: Vec<usize> = children
            .iter()
            .copied()
            .filter(|&c| matches!(self.nodes[c].kind, NodeKind::Argument { .. }))
            .collect();
        if literals.is_empty() {
            return arguments;
        }
        let rest = reader.remaining();
        let word = rest.split(' ').next().unwrap_or("");
        match literals.iter().find(|&&l| self.nodes[l].name() == word) {
            Some(&l) => vec![l],
            None => arguments,
        }
    }
    fn parse_node(
        &self,
        node: usize,
        reader: &mut Reader,
        context: &mut Context,
        reg: &Registries,
    ) -> CmdResult<()> {
        let start = reader.cursor;
        match &self.nodes[node].kind {
            NodeKind::Root => Ok(()),
            NodeKind::Literal(literal) => {
                if reader.remaining().starts_with(literal.as_str()) {
                    let end = start + literal.len();
                    reader.cursor = end;
                    if !reader.can_read() || reader.peek() == ' ' {
                        context.with_node(node, (start, end));
                        context.literals.push(literal.clone());
                        return Ok(());
                    }
                    reader.cursor = start;
                }
                let mut e = CommandError::tr_at("argument.literal.incorrect", &[literal], reader);
                e.literal_incorrect = true;
                Err(e)
            }
            NodeKind::Argument { name, ty } => {
                let value = ty.parse(reader, reg)?;
                let range = (start, reader.cursor);
                context.args.push(ParsedArg {
                    name: name.clone(),
                    value,
                    range,
                });
                context.with_node(node, range);
                Ok(())
            }
        }
    }
    fn parse_nodes(
        &self,
        node: usize,
        input: &str,
        cursor: usize,
        so_far: &Context,
        reg: &Registries,
        level: u8,
    ) -> ParseResults {
        let mut errors = Vec::new();
        let mut potentials: Vec<ParseResults> = Vec::new();
        let original = Reader {
            string: input,
            cursor,
        };
        for child in self.relevant(node, &original) {
            if self.nodes[child].permission > level {
                continue;
            }
            let mut context = so_far.clone();
            let mut reader = original.clone();
            let parsed = self
                .parse_node(child, &mut reader, &mut context, reg)
                .and_then(|()| {
                    if reader.can_read() && reader.peek() != ' ' {
                        Err(CommandError::tr_at(
                            "command.expected.separator",
                            &[],
                            &reader,
                        ))
                    } else {
                        Ok(())
                    }
                });
            if let Err(e) = parsed {
                errors.push((child, e));
                continue;
            }
            context.command = self.nodes[child].command;
            let redirect = self.nodes[child].redirect;
            if reader.can_read_n(if redirect.is_none() { 2 } else { 1 }) {
                reader.skip();
                if let Some(target) = redirect {
                    let child_context = Context::new(target, reader.cursor);
                    let parse =
                        self.parse_nodes(target, input, reader.cursor, &child_context, reg, level);
                    context.child = Some(Box::new(parse.context));
                    return ParseResults {
                        context,
                        input: input.to_owned(),
                        cursor: parse.cursor,
                        errors: parse.errors,
                    };
                }
                potentials.push(self.parse_nodes(
                    child,
                    input,
                    reader.cursor,
                    &context,
                    reg,
                    level,
                ));
            } else {
                potentials.push(ParseResults {
                    context,
                    input: input.to_owned(),
                    cursor: reader.cursor,
                    errors: Vec::new(),
                });
            }
        }
        if !potentials.is_empty() {
            // Prefer fully read input, then results without errors (stable sort).
            potentials.sort_by_key(|p| (p.can_read(), !p.errors.is_empty()));
            return potentials.swap_remove(0);
        }
        ParseResults {
            context: so_far.clone(),
            input: input.to_owned(),
            cursor,
            errors,
        }
    }
    /// CommandDispatcher.parse from the start of `input` (no leading slash).
    pub fn parse(&self, input: &str, reg: &Registries) -> ParseResults {
        self.parse_at(input, 0, reg)
    }
    /// Parse starting at byte `start` (after a leading '/').
    pub fn parse_at(&self, input: &str, start: usize, reg: &Registries) -> ParseResults {
        let context = Context::new(0, start);
        self.parse_nodes(0, input, start, &context, reg, reg.permission)
    }
    /// CommandDispatcher.getCompletionSuggestions at `cursor`.
    pub fn suggestions(
        &self,
        parse: &ParseResults,
        cursor: usize,
        reg: &Registries,
    ) -> (Range, Vec<Suggestion>) {
        let (parent, start, context) = parse.context.suggestion_context(cursor);
        let start = start.min(cursor);
        let truncated = &parse.input[..cursor.min(parse.input.len())];
        let mut all = Vec::new();
        for &child in &self.nodes[parent].children {
            if self.nodes[child].permission > reg.permission {
                continue;
            }
            let mut builder = SuggestionsBuilder::new(truncated, start);
            match &self.nodes[child].kind {
                NodeKind::Literal(l) => {
                    if l.to_lowercase().starts_with(&builder.remaining_lower()) {
                        builder.suggest(l.clone());
                    }
                }
                NodeKind::Argument { ty, name } => ty.suggest(&mut builder, reg, context, name),
                NodeKind::Root => {}
            }
            all.extend(builder.out);
        }
        merge_suggestions(truncated, all)
    }
    fn smart_usage_of(&self, node: usize, optional: bool, deep: bool, level: u8) -> Option<String> {
        let n = &self.nodes[node];
        if n.permission > level {
            return None;
        }
        let this = if optional {
            format!("[{}]", n.usage_text())
        } else {
            n.usage_text()
        };
        let child_optional = n.command.is_some();
        let (open, close) = if child_optional {
            ("[", "]")
        } else {
            ("(", ")")
        };
        if !deep {
            if let Some(r) = n.redirect {
                let target = if r == 0 {
                    "...".to_owned()
                } else {
                    format!("-> {}", self.nodes[r].usage_text())
                };
                return Some(format!("{this} {target}"));
            }
            let children: Vec<usize> = n
                .children
                .iter()
                .copied()
                .filter(|&c| self.nodes[c].permission <= level)
                .collect();
            if children.len() == 1 {
                if let Some(usage) =
                    self.smart_usage_of(children[0], child_optional, child_optional, level)
                {
                    return Some(format!("{this} {usage}"));
                }
            } else if children.len() > 1 {
                let mut child_usage: Vec<String> = Vec::new();
                for &c in &children {
                    if let Some(u) = self.smart_usage_of(c, child_optional, true, level)
                        && !child_usage.contains(&u)
                    {
                        child_usage.push(u);
                    }
                }
                if child_usage.len() == 1 {
                    let usage = &child_usage[0];
                    return Some(format!(
                        "{this} {}",
                        if child_optional {
                            format!("[{usage}]")
                        } else {
                            usage.clone()
                        }
                    ));
                }
                if child_usage.len() > 1 {
                    let alternatives: Vec<String> = children
                        .iter()
                        .map(|&c| self.nodes[c].usage_text())
                        .collect();
                    return Some(format!("{this} {open}{}{close}", alternatives.join("|")));
                }
            }
        }
        Some(this)
    }
    /// CommandDispatcher.getSmartUsage: one line per child of `node`.
    pub fn smart_usage(&self, node: usize, level: u8) -> Vec<(usize, String)> {
        let optional = self.nodes[node].command.is_some();
        self.nodes[node]
            .children
            .iter()
            .filter_map(|&c| {
                self.smart_usage_of(c, optional, false, level)
                    .map(|u| (c, u))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
