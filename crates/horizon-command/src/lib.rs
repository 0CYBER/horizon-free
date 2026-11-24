//! A port of Brigadier (the command library vanilla uses): a string reader,
//! literal/argument node trees with redirects, parsing that tries every
//! relevant child and keeps the best result, completion suggestions, smart
//! usage text and the vanilla error messages. Argument types live in
//! [`args`]; the vanilla command tree in [`vanilla`]. Brigadier is a library,
//! not part of the pinned 26.3 sources, so a comment naming its classes
//! describes the library this module ports.
#![forbid(unsafe_code)]
pub use horizon_client_core::suggestion::{Range, Suggestion};
pub mod args;
pub mod block;
pub mod codec;
pub mod component;
pub mod item;
pub mod network;
pub mod packrat;
pub mod selector;
pub mod snbt;
pub mod tree_data;
pub mod tree_execute;
pub mod tree_extra;
pub mod tree_misc;
pub mod tree_scoreboard;
pub mod tree_world;
pub mod vanilla;
mod vanilla_order;

pub use args::{
    Arg, ArgType, CustomArgument, CustomArgumentKind, CustomCompletionAction, CustomSuggestion,
    Registries, modify_custom_completions,
};
use horizon_client_core::text::Text;

/// The command id of a mod's command (`Dispatcher::with_mod_commands`).
pub const MOD_COMMAND: &str = "mod";
/// The command id of the client's own `/profiler` (`Dispatcher::with_profiler`).
pub const PROFILER_COMMAND: &str = "horizon:profiler";

/// A command error (CommandSyntaxException): the message and, for parse
/// errors, the input and cursor it occurred at.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandError {
    pub message: Text,
    pub input: Option<String>,
    pub cursor: usize,
    /// BuiltInExceptions.literalIncorrect (summarised specially by the chat UI).
    pub literal_incorrect: bool,
    /// The translation key of the exception type, when known (the
    /// exception "type" vanilla code compares against).
    pub key: String,
    /// Not a CommandSyntaxException: `Commands.performCommand`'s catch reports
    /// it as `command.failed` and no result callback runs.
    pub unexpected: bool,
}
impl CommandError {
    pub fn new(message: Text) -> Self {
        Self {
            message,
            input: None,
            cursor: 0,
            literal_incorrect: false,
            key: String::new(),
            unexpected: false,
        }
    }
    /// An exception that is not a CommandSyntaxException, as
    /// `Commands.performCommand` catches it.
    pub fn unexpected() -> Self {
        let mut e = Self::tr("command.failed", &[]);
        e.unexpected = true;
        e
    }
    pub fn tr(key: &str, args: &[&dyn std::fmt::Display]) -> Self {
        let mut e = Self::new(Text::tr(key, args));
        e.key = key.to_owned();
        e
    }
    pub fn at(message: Text, reader: &Reader) -> Self {
        Self {
            message,
            input: Some(reader.string.to_owned()),
            cursor: reader.cursor,
            literal_incorrect: false,
            key: String::new(),
            unexpected: false,
        }
    }
    pub fn tr_at(key: &str, args: &[&dyn std::fmt::Display], reader: &Reader) -> Self {
        let mut e = Self::at(Text::tr(key, args), reader);
        e.key = key.to_owned();
        e
    }
    /// `CommandSyntaxException.getCursor`: the cursor as a Java `String`
    /// index (UTF-16 units), the position vanilla reports.
    pub fn java_cursor(&self) -> usize {
        match &self.input {
            Some(input) => java_index(input, self.cursor),
            None => self.cursor,
        }
    }
    /// Brigadier's command exception context: up to ten characters before the
    /// cursor followed by "<--[HERE]".
    pub fn context(&self) -> Option<String> {
        let input = self.input.as_ref()?;
        let cursor = char_floor(input, self.cursor);
        let mut out = String::new();
        if java_index(input, cursor) > 10 {
            out.push_str("...");
        }
        out.push_str(&input[context_start(input, cursor)..cursor]);
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
                    Text::literal(self.java_cursor().to_string()),
                    Text::literal(context),
                ],
            ),
            None => self.message.clone(),
        }
    }
}
pub type CmdResult<T> = Result<T, CommandError>;

/// `cursor` moved back to the start of the character it falls in, and kept inside `input`.
fn char_floor(input: &str, cursor: usize) -> usize {
    let mut end = cursor.min(input.len());
    while !input.is_char_boundary(end) {
        end -= 1;
    }
    end
}

/// A byte `cursor` into `input` as a Java `String` index: the UTF-16 units before it, which
/// is how Brigadier counts the positions it reports.
pub fn java_index(input: &str, cursor: usize) -> usize {
    input[..char_floor(input, cursor)].encode_utf16().count()
}

/// The byte where the context of an error at `cursor` starts: ten Java chars (UTF-16 units)
/// back, as Brigadier's `substring(Math.max(0, cursor - 10), cursor)`. Where that falls inside
/// a surrogate pair, the half character is left out instead of shown broken.
pub fn context_start(input: &str, cursor: usize) -> usize {
    let target = java_index(input, cursor).saturating_sub(10);
    let mut units = 0;
    for (i, c) in input.char_indices() {
        if units >= target {
            return i;
        }
        units += c.len_utf16();
    }
    input.len()
}

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
        while self.can_read() && Self::is_java_whitespace(self.peek()) {
            self.skip();
        }
    }
    pub fn is_java_whitespace(c: char) -> bool {
        matches!(
            c,
            '\t'..='\r'
                | '\u{1C}'..='\u{20}'
                | '\u{1680}'
                | '\u{2000}'..='\u{2006}'
                | '\u{2008}'..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{205F}'
                | '\u{3000}'
        )
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
    /// Every character up to the next space or the end of the line: a token that may hold `*`,
    /// `:` or `/`. Empty when the reader is at a space or at the end.
    pub fn read_token(&mut self) -> &'a str {
        let start = self.cursor;
        while self.can_read() && self.peek() != ' ' {
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
    /// The level needed to execute this node, independently of walking through it.
    pub execution_permission: u8,
    /// A custom suggestion provider (`.suggests(...)`), used instead of the
    /// argument type's own suggestions.
    pub suggests: Option<Suggests>,
    /// ArgumentNodeStub's RESTRICTED_COMMAND: the server marked this node as
    /// needing elevated permissions.
    pub restricted: bool,
}
/// The custom SuggestionProviders vanilla command nodes use.
#[derive(Clone, Debug, PartialEq)]
pub enum Suggests {
    /// FunctionCommand.SUGGEST_FUNCTION: `#tags` and functions.
    Functions,
    /// A fixed list (SharedSuggestionProvider.suggest).
    Fixed(&'static [&'static str]),
    /// No candidates (providers over state an empty world lacks).
    Nothing,
    /// AdvancementCommands: the criteria of the parsed `advancement`.
    AdvancementCriteria,
    /// SuggestionProviders.SUMMONABLE_ENTITIES (with the type names as tooltips).
    SummonableEntities,
    /// TimeCommand.suggestTimeMarkers for the parsed `clock` (or the overworld clock).
    TimeMarkers,
    /// TriggerCommand.suggestObjectives: the source's enabled trigger objectives.
    Triggers,
    /// ScoreboardCommand.suggestTriggers: trigger objectives some of the
    /// parsed `targets` have no score in, or a locked one.
    EnableTriggers,
    /// SuggestionProviders.ASK_SERVER: the server completes the node
    /// (ClientSuggestionProvider.customSuggestion); nothing locally.
    AskServer,
    /// OpCommand: the online players who are not operators.
    NonOperators,
    /// DeOpCommands: the operators' names (PlayerList.getOpNames).
    Operators,
    /// PardonCommand: the banned players' names (UserBanList.getUserList).
    BannedPlayers,
    /// PardonIpCommand: the banned addresses (IpBanList.getUserList).
    BannedIps,
    /// WhitelistCommand `add`: the online players who are not whitelisted.
    NonWhitelisted,
    /// WhitelistCommand `remove`: the whitelisted names (PlayerList.getWhiteListNames).
    Whitelisted,
}
impl Node {
    pub fn name(&self) -> &str {
        match &self.kind {
            NodeKind::Root => "",
            NodeKind::Literal(l) => l,
            NodeKind::Argument { name, .. } => name,
        }
    }
    /// Brigadier's node usage text.
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
    /// Brigadier's suggestion context: the node whose children
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
    /// ClientPacketListener.isValidCommand: all of it read, no errors, and it
    /// ends on a node that runs something.
    pub fn is_valid(&self) -> bool {
        !self.can_read() && self.errors.is_empty() && self.context.last_child().command.is_some()
    }
    /// SignableCommand.of: the message arguments a signed command signs, by
    /// name, with their text, not past a redirect to the root (`execute ...
    /// run`). `dispatcher` is the one that made this parse.
    pub fn signable_arguments(&self, dispatcher: &Dispatcher) -> Vec<(String, String)> {
        let root = self.context.root;
        let mut out = Vec::new();
        let mut context = Some(&self.context);
        while let Some(c) = context {
            for &(node, (start, end)) in &c.nodes {
                if let NodeKind::Argument {
                    ty: ArgType::Message,
                    name,
                } = &dispatcher.nodes[node].kind
                    && let Some(text) = self.input.get(start..end)
                {
                    out.push((name.clone(), text.to_owned()));
                }
            }
            context = c.child.as_deref().filter(|child| child.root != root);
        }
        out
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
        self.suggest_at(self.start, text);
    }
    /// Brigadier's suggestions-builder `suggest(int)`.
    pub fn suggest_int(&mut self, value: i32) {
        self.out.push(Suggestion {
            range: (self.start, self.input.len()),
            text: value.to_string(),
            tooltip: None,
            int_value: Some(value),
        });
    }
    pub fn suggest_tooltip(&mut self, text: impl Into<String>, tooltip: impl Into<String>) {
        self.suggest_at_tooltip(self.start, text, tooltip);
    }
    /// `SuggestionsBuilder.suggest` for a replacement that starts `start` bytes into the input:
    /// the text from there to the cursor is replaced, so a completion can apply to one token of a
    /// longer text argument (a mod's greedy text being split into words). A `start` past the
    /// input or inside a character suggests nothing, rather than panicking where the range is
    /// used.
    pub fn suggest_at(&mut self, start: usize, text: impl Into<String>) {
        let text = text.into();
        if !self.replaces_something(start) || text == self.input[start..] {
            return;
        }
        self.out.push(Suggestion {
            range: (start, self.input.len()),
            text,
            tooltip: None,
            int_value: None,
        });
    }
    /// [`Self::suggest_at`] with an explanation.
    pub fn suggest_at_tooltip(
        &mut self,
        start: usize,
        text: impl Into<String>,
        tooltip: impl Into<String>,
    ) {
        let text = text.into();
        if !self.replaces_something(start) || text == self.input[start..] {
            return;
        }
        self.out.push(Suggestion {
            range: (start, self.input.len()),
            text,
            tooltip: Some(tooltip.into()),
            int_value: None,
        });
    }
    /// Whether `start` can begin a replacement range of the input.
    fn replaces_something(&self, start: usize) -> bool {
        start <= self.input.len() && self.input.is_char_boundary(start)
    }
    pub fn offset(&self, start: usize) -> SuggestionsBuilder<'a> {
        SuggestionsBuilder::new(self.input, start)
    }
    /// Brigadier's suggestions-builder `add`: keep another builder's suggestions (with their own ranges).
    pub fn add(&mut self, other: SuggestionsBuilder) {
        self.out.extend(other.out);
    }
    /// SharedSuggestionProvider.suggest: candidates matching the remaining
    /// text at the start or after a `.`, `_` or `/`.
    pub fn suggest_matching<'b>(&mut self, candidates: impl IntoIterator<Item = &'b str>) {
        let lower = self.remaining_lower();
        for c in candidates {
            if args::matches_sub_str(&lower, &c.to_lowercase()) {
                self.suggest(c);
            }
        }
    }
}
/// String.compareToIgnoreCase.
pub fn compare_ignore_case(a: &str, b: &str) -> std::cmp::Ordering {
    let fold = |c: char| -> char {
        let u = c.to_uppercase().next().unwrap_or(c);
        u.to_lowercase().next().unwrap_or(u)
    };
    let mut x = a.chars();
    let mut y = b.chars();
    loop {
        match (x.next(), y.next()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(c1), Some(c2)) => {
                if c1 != c2 {
                    let (l1, l2) = (fold(c1), fold(c2));
                    if l1 != l2 {
                        return l1.cmp(&l2);
                    }
                }
            }
        }
    }
}
/// Case-insensitive text order, with IntegerSuggestion comparing numerically.
fn compare_suggestions(a: &Suggestion, b: &Suggestion) -> std::cmp::Ordering {
    match (a.int_value, b.int_value) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => a.text.cmp(&b.text),
        _ => compare_ignore_case(&a.text, &b.text),
    }
}
/// The longest completion text a suggestion packet carries: `ByteBufCodecs.STRING_UTF8` counts
/// 32767 UTF-16 code units, which is what vanilla's reader and its client both enforce.
const MAX_SUGGESTION_TEXT: usize = 32767;
/// Brigadier's suggestions create: merged, expanded to a common range and sorted case-insensitively.
pub fn merge_suggestions(input: &str, list: Vec<Suggestion>) -> (Range, Vec<Suggestion>) {
    if list.is_empty() {
        return ((0, 0), Vec::new());
    }
    let start = list.iter().map(|s| s.range.0).min().unwrap_or(0);
    let end = list.iter().map(|s| s.range.1).max().unwrap_or(0);
    let mut out: Vec<Suggestion> = Vec::new();
    let mut seen: std::collections::HashSet<(String, Option<String>)> =
        std::collections::HashSet::new();
    for s in list {
        // Brigadier's suggestion expand: pad the text with the input around it.
        let text = format!(
            "{}{}{}",
            &input[start..s.range.0],
            s.text,
            &input[s.range.1..end]
        );
        // Padding grows the text past what the suggestion itself holds: a long prefix, and a
        // suggestion starting later, expand to the whole merged range. Each one was within the
        // packet's limit alone, so the expanded text is measured again and the ones that no
        // longer fit are left out, keeping the others.
        if text.encode_utf16().count() > MAX_SUGGESTION_TEXT {
            continue;
        }
        let expanded = Suggestion {
            range: (start, end),
            text,
            tooltip: s.tooltip,
            int_value: if s.range == (start, end) {
                s.int_value
            } else {
                None
            },
        };
        if seen.insert((expanded.text.clone(), expanded.tooltip.clone())) {
            out.push(expanded);
        }
    }
    if out.is_empty() {
        // Every completion was too long to send: nothing to show, and no range to show it in.
        return ((0, 0), out);
    }
    let ints = out.iter().filter(|s| s.int_value.is_some()).count();
    if ints == 0 || ints == out.len() {
        out.sort_by(compare_suggestions);
        return ((start, end), out);
    }
    // A stable insertion sort (the comparator mixes numeric and text order).
    let mut sorted: Vec<Suggestion> = Vec::with_capacity(out.len());
    for s in out {
        let pos =
            sorted.partition_point(|x| compare_suggestions(x, &s) != std::cmp::Ordering::Greater);
        sorted.insert(pos, s);
    }
    ((start, end), sorted)
}

impl Suggests {
    /// The provider's candidates; `context` is the whole parse context
    /// (the built parse context), whose arguments some providers read.
    pub fn suggest(&self, builder: &mut SuggestionsBuilder, reg: &Registries, context: &Context) {
        match self {
            Suggests::AdvancementCriteria => {
                if let Some(Arg::Resource(id)) = context.get("advancement")
                    && let Some(criteria) = reg.advancement_criteria.get(id)
                {
                    builder.suggest_matching(criteria.iter().map(String::as_str));
                }
            }
            Suggests::Functions => {
                args::suggest_resources(builder, &reg.function_tags, "#");
                args::suggest_resources(builder, &reg.functions, "");
            }
            Suggests::Fixed(values) => builder.suggest_matching(values.iter().copied()),
            Suggests::SummonableEntities => {
                let contents = builder.remaining_lower();
                for id in &reg.summonable {
                    if args::resource_matches(&contents, id) {
                        let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
                        let name = horizon_client_core::lang::get(&format!(
                            "entity.{ns}.{}",
                            path.replace('/', ".")
                        ));
                        builder.suggest_tooltip(id.clone(), name);
                    }
                }
            }
            Suggests::TimeMarkers => {
                let clock = match context.get("clock") {
                    Some(Arg::Resource(id)) => id.clone(),
                    _ => "minecraft:overworld".to_owned(),
                };
                match reg.clock_time_markers.get(&clock) {
                    Some(markers) => args::suggest_resources(builder, markers, ""),
                    None if clock == "minecraft:overworld" => {
                        args::suggest_resources(builder, &reg.time_markers, "")
                    }
                    None => {}
                }
            }
            Suggests::EnableTriggers => {
                // getNamesWithDefaultWildcard: `*` means every tracked holder.
                let targets: Vec<String> = match context.get("targets") {
                    Some(Arg::ScoreHolder(args::ScoreHolderArg::Name(n))) => vec![n.clone()],
                    Some(Arg::ScoreHolder(args::ScoreHolderArg::Wildcard)) => reg.tracked.clone(),
                    _ => reg.players.clone(),
                };
                let available: Vec<&str> = reg
                    .trigger_scores
                    .iter()
                    .filter(|(_, scores)| {
                        targets
                            .iter()
                            .any(|t| scores.get(t).is_none_or(|locked| *locked))
                    })
                    .map(|(o, _)| o.as_str())
                    .collect();
                builder.suggest_matching(available);
            }
            Suggests::Triggers => {
                builder.suggest_matching(reg.trigger_objectives.iter().map(String::as_str))
            }
            Suggests::NonOperators => {
                let listed = |name: &&String| !reg.ops.iter().any(|o| o.eq_ignore_ascii_case(name));
                builder.suggest_matching(reg.players.iter().filter(listed).map(String::as_str))
            }
            Suggests::Operators => builder.suggest_matching(reg.ops.iter().map(String::as_str)),
            Suggests::BannedPlayers => {
                builder.suggest_matching(reg.banned_players.iter().map(String::as_str))
            }
            Suggests::BannedIps => {
                builder.suggest_matching(reg.banned_ips.iter().map(String::as_str))
            }
            Suggests::NonWhitelisted => {
                let listed =
                    |name: &&String| !reg.whitelisted.iter().any(|w| w.eq_ignore_ascii_case(name));
                builder.suggest_matching(reg.players.iter().filter(listed).map(String::as_str))
            }
            Suggests::Whitelisted => {
                builder.suggest_matching(reg.whitelisted.iter().map(String::as_str))
            }
            Suggests::Nothing | Suggests::AskServer => {}
        }
    }
}

/// The deepest node/redirect recursion `parse_nodes` accepts. `execute … run`
/// redirects back to the root, so a crafted deeply nested command would recurse
/// once per level and overflow the parsing thread's stack (measured at ~300 on
/// the server thread). Far above any real command chain; vanilla likewise caps
/// command nesting.
const MAX_PARSE_DEPTH: usize = 128;

/// The most children one parse examines, over all its branches: each child a node's selection
/// looks at counts once, and each child it then tries once more. Depth alone does not bound the
/// work: a server's tree whose every level has two arguments leading to the same next level
/// doubles the branches per word, so 40 words were 2^41 tries (Brigadier does the same), and a
/// node with many children reached on many branches is scanned on each. A parse of vanilla's
/// tree looks at the root's hundred-odd children and a few dozen more per level, so an
/// `execute` nested almost to [`MAX_PARSE_DEPTH`] stays far below this; past it the whole
/// command fails to parse.
const MAX_PARSE_VISITS: usize = 100_000;

/// The most nodes one usage line follows down a chain of single children before it ends in
/// "...". Vanilla's longest such chain is 8 nodes; a server's tree may chain hundreds of
/// thousands, which Brigadier follows by recursion and string concatenation per node.
const MAX_USAGE_CHAIN: usize = 64;

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
/// `/profiler start`'s GPU literals: pass timings, and pass timings with their parts and counters.
pub const PROFILER_GPU: [&str; 2] = ["gpu", "gpu_detail"];
impl Dispatcher {
    pub fn new() -> Self {
        Self {
            nodes: vec![Node {
                kind: NodeKind::Root,
                children: Vec::new(),
                command: None,
                redirect: None,
                permission: 0,
                execution_permission: 0,
                suggests: None,
                restricted: false,
            }],
        }
    }
    /// This tree as a player without elevated permissions has it
    /// (ClientPacketListener.restrictedSuggestionsProvider): no restricted node.
    pub fn without_restricted(&self) -> Self {
        let mut tree = self.clone();
        for node in &mut tree.nodes {
            node.children.retain(|&child| !self.nodes[child].restricted);
        }
        tree
    }
    fn add(&mut self, parent: usize, kind: NodeKind) -> usize {
        let name = match &kind {
            NodeKind::Literal(l) => l.clone(),
            NodeKind::Argument { name, .. } => name.clone(),
            NodeKind::Root => String::new(),
        };
        // Same-named children merge.
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
            execution_permission: 0,
            suggests: None,
            restricted: false,
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
    pub fn suggests(&mut self, node: usize, provider: Suggests) -> usize {
        self.nodes[node].suggests = Some(provider);
        node
    }
    pub fn requires(&mut self, node: usize, level: u8) -> usize {
        self.nodes[node].permission = level;
        node
    }
    /// Require a level to run this node without restricting access to its children.
    pub fn requires_execution(&mut self, node: usize, level: u8) -> usize {
        self.nodes[node].execution_permission = level;
        node
    }
    /// This tree plus the commands mods own (`docs/modding/src/how-to/content-mods.md`), so the chat
    /// colours and completes them like the others. Every player may use one,
    /// and the text after its name goes to the mod as it was typed.
    pub fn with_mod_commands<'a>(&self, names: impl IntoIterator<Item = &'a str>) -> Dispatcher {
        let mut d = self.clone();
        for name in names {
            let command = d.literal(0, name);
            d.executes(command, MOD_COMMAND);
            let args = d.argument(command, "args", ArgType::Greedy);
            d.executes(args, MOD_COMMAND);
        }
        d
    }
    /// `/profiler` alone, which the client parses its chat line with.
    pub fn profiler() -> Dispatcher {
        Dispatcher::new().with_profiler()
    }
    /// This tree plus `/profiler`, which the client answers itself, so chat completes its real options.
    pub fn with_profiler(&self) -> Dispatcher {
        let mut d = self.clone();
        if d.find("profiler").is_some() {
            return d;
        }
        let command = d.literal(0, "profiler");
        for action in ["start", "stop"] {
            let action_node = d.literal(command, action);
            d.executes(action_node, PROFILER_COMMAND);
            for side in ["client", "server"] {
                let side = d.literal(action_node, side);
                d.executes(side, PROFILER_COMMAND);
                if action == "start" {
                    for gpu in PROFILER_GPU {
                        let gpu = d.literal(side, gpu);
                        d.executes(gpu, PROFILER_COMMAND);
                    }
                }
            }
            if action == "start" {
                for gpu in PROFILER_GPU {
                    let gpu = d.literal(action_node, gpu);
                    d.executes(gpu, PROFILER_COMMAND);
                }
            }
        }
        let status = d.literal(command, "status");
        d.executes(status, PROFILER_COMMAND);
        for action in ["open", "share"] {
            let action = d.literal(command, action);
            d.executes(action, PROFILER_COMMAND);
            let id = d.argument(action, "id", ArgType::Word);
            d.executes(id, PROFILER_COMMAND);
        }
        let mark = d.literal(command, "mark");
        let name = d.argument(mark, "name", ArgType::Greedy);
        d.executes(name, PROFILER_COMMAND);
        d
    }
    /// Whether this tree's `/profiler` is the one [`Self::with_profiler`] adds, not a server's or a mod's of the same name.
    pub fn has_horizon_profiler(&self) -> bool {
        self.parse("profiler status", &Registries::default())
            .executable()
            .is_ok_and(|(_, command)| command == PROFILER_COMMAND)
    }
    /// A root literal by name.
    pub fn find(&self, name: &str) -> Option<usize> {
        self.nodes[0]
            .children
            .iter()
            .copied()
            .find(|&c| self.nodes[c].name() == name)
    }
    /// The children of `node` worth trying (Brigadier's `getRelevantNodes`). Looking at them is
    /// charged to `visits` first, and past [`MAX_PARSE_VISITS`] none are.
    fn relevant(&self, node: usize, reader: &Reader, visits: &mut usize) -> Vec<usize> {
        let children = &self.nodes[node].children;
        *visits = visits.saturating_add(children.len());
        if *visits > MAX_PARSE_VISITS {
            return Vec::new();
        }
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
        depth: usize,
        visits: &mut usize,
    ) -> ParseResults {
        if depth >= MAX_PARSE_DEPTH || *visits > MAX_PARSE_VISITS {
            // A crafted command nested through `execute … run` (which redirects
            // back to the root) recurses here per level; stop before it
            // overflows the parsing thread's stack. The branch yields no result,
            // so the whole command simply fails to parse. Past the visit budget
            // every branch stops the same way, and `parse_counted` fails the parse.
            return ParseResults {
                context: so_far.clone(),
                input: input.to_owned(),
                cursor,
                errors: Vec::new(),
            };
        }
        let mut errors = Vec::new();
        let mut potentials: Vec<ParseResults> = Vec::new();
        let original = Reader {
            string: input,
            cursor,
        };
        for child in self.relevant(node, &original, visits) {
            if self.nodes[child].permission > level {
                continue;
            }
            *visits += 1;
            if *visits > MAX_PARSE_VISITS {
                break;
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
            context.command = self.nodes[child]
                .command
                .filter(|_| self.nodes[child].execution_permission <= level);
            let redirect = self.nodes[child].redirect;
            if reader.can_read_n(if redirect.is_none() { 2 } else { 1 }) {
                reader.skip();
                if let Some(target) = redirect {
                    let child_context = Context::new(target, reader.cursor);
                    let parse = self.parse_nodes(
                        target,
                        input,
                        reader.cursor,
                        &child_context,
                        reg,
                        level,
                        depth + 1,
                        visits,
                    );
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
                    depth + 1,
                    visits,
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
            // Prefer fully read input, then results without errors. An executable
            // result wins an otherwise complete tie; all other ties stay stable.
            potentials.sort_by_key(|p| (p.can_read(), !p.errors.is_empty(), !p.is_valid()));
            return potentials.swap_remove(0);
        }
        ParseResults {
            context: so_far.clone(),
            input: input.to_owned(),
            cursor,
            errors,
        }
    }
    /// The dispatcher parses from the start of `input` (no leading slash).
    pub fn parse(&self, input: &str, reg: &Registries) -> ParseResults {
        self.parse_at(input, 0, reg)
    }
    /// Parse starting at byte `start` (after a leading '/').
    pub fn parse_at(&self, input: &str, start: usize, reg: &Registries) -> ParseResults {
        self.parse_counted(input, start, reg).0
    }
    /// [`Self::parse_at`], and how many children the parse examined. A parse past
    /// [`MAX_PARSE_VISITS`] fails as a whole, as an unknown command: what it found before the
    /// budget ran out depends on the order it went in.
    fn parse_counted(&self, input: &str, start: usize, reg: &Registries) -> (ParseResults, usize) {
        let context = Context::new(0, start);
        let mut visits = 0;
        let parse = self.parse_nodes(
            0,
            input,
            start,
            &context,
            reg,
            reg.permission,
            0,
            &mut visits,
        );
        if visits > MAX_PARSE_VISITS {
            let failed = ParseResults {
                context,
                input: input.to_owned(),
                cursor: start,
                errors: Vec::new(),
            };
            return (failed, visits);
        }
        (parse, visits)
    }
    /// Brigadier's completion suggestions at `cursor`: it
    /// lists every child, whatever its requirement.
    pub fn suggestions(
        &self,
        parse: &ParseResults,
        cursor: usize,
        reg: &Registries,
    ) -> (Range, Vec<Suggestion>) {
        let (range, list, _) = self.suggestions_within(parse, cursor, reg, u8::MAX);
        (range, list)
    }
    /// The suggestions the player's chat shows: the client's tree only holds
    /// the nodes the player may use (Commands.sendCommands prunes by canUse).
    pub fn client_suggestions(
        &self,
        parse: &ParseResults,
        cursor: usize,
        reg: &Registries,
    ) -> (Range, Vec<Suggestion>) {
        let (range, list, _) = self.suggestions_within(parse, cursor, reg, reg.permission);
        (range, list)
    }
    /// [`Self::client_suggestions`], and whether a node at the cursor asks
    /// the server (`ask_server`): its completions come with the server's
    /// reply, to be merged with these.
    pub fn client_suggestions_asking(
        &self,
        parse: &ParseResults,
        cursor: usize,
        reg: &Registries,
    ) -> (Range, Vec<Suggestion>, bool) {
        self.suggestions_within(parse, cursor, reg, reg.permission)
    }
    fn suggestions_within(
        &self,
        parse: &ParseResults,
        cursor: usize,
        reg: &Registries,
        permission: u8,
    ) -> (Range, Vec<Suggestion>, bool) {
        let (parent, start, context) = parse.context.suggestion_context(cursor);
        let start = start.min(cursor);
        let truncated = &parse.input[..cursor.min(parse.input.len())];
        let mut all = Vec::new();
        let mut ask = false;
        for &child in &self.nodes[parent].children {
            if self.nodes[child].permission > permission {
                continue;
            }
            ask |= matches!(self.nodes[child].suggests, Some(Suggests::AskServer));
            let mut builder = SuggestionsBuilder::new(truncated, start);
            match &self.nodes[child].kind {
                NodeKind::Literal(l) => {
                    if l.to_lowercase().starts_with(&builder.remaining_lower()) {
                        builder.suggest(l.clone());
                    }
                }
                NodeKind::Argument { ty, name } => match &self.nodes[child].suggests {
                    Some(provider) => provider.suggest(&mut builder, reg, &parse.context),
                    None => ty.suggest(&mut builder, reg, context, name),
                },
                NodeKind::Root => {}
            }
            all.extend(builder.out);
        }
        let (range, list) = merge_suggestions(truncated, all);
        // The reply's range is the builder's, covering the token being typed, even when nothing
        // is suggested locally: a node that asks the server (`ask_server`, and the mods' own
        // `tab_complete` answers for the free `id`) brings its completions with the server's
        // reply, which the chat applies to that token. The empty list's `(0, 0)` would paste
        // what the server adds at the start of the line.
        let range = if list.is_empty() {
            (start, truncated.len())
        } else {
            range
        };
        (range, list, ask)
    }
    /// Brigadier's `getSmartUsage(node, source, optional, deep)`. Brigadier recurses once per
    /// node of a chain of single children and prefixes each level's text on the way back; this
    /// walks the chain in a loop and joins the parts once, and stops after
    /// [`MAX_USAGE_CHAIN`] nodes with "...".
    fn smart_usage_of(&self, node: usize, optional: bool, deep: bool, level: u8) -> Option<String> {
        if self.nodes[node].permission > level {
            return None;
        }
        let mut parts: Vec<String> = Vec::new();
        let (mut node, mut optional, mut deep) = (node, optional, deep);
        loop {
            if parts.len() >= MAX_USAGE_CHAIN {
                parts.push("...".to_owned());
                break;
            }
            let n = &self.nodes[node];
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
            if deep {
                parts.push(this);
                break;
            }
            if let Some(r) = n.redirect {
                let target = if r == 0 {
                    "...".to_owned()
                } else {
                    format!("-> {}", self.nodes[r].usage_text())
                };
                parts.push(format!("{this} {target}"));
                break;
            }
            let children: Vec<usize> = n
                .children
                .iter()
                .copied()
                .filter(|&c| self.nodes[c].permission <= level)
                .collect();
            if children.len() == 1 {
                // The child's permission passed the filter, so its usage exists: follow it.
                parts.push(this);
                (node, optional, deep) = (children[0], child_optional, child_optional);
                continue;
            }
            if children.len() > 1 {
                // Each child's usage here is deep: its own text only, no further recursion.
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
                    parts.push(format!(
                        "{this} {}",
                        if child_optional {
                            format!("[{usage}]")
                        } else {
                            usage.clone()
                        }
                    ));
                    break;
                }
                if child_usage.len() > 1 {
                    let alternatives: Vec<String> = children
                        .iter()
                        .map(|&c| self.nodes[c].usage_text())
                        .collect();
                    parts.push(format!("{this} {open}{}{close}", alternatives.join("|")));
                    break;
                }
            }
            parts.push(this);
            break;
        }
        Some(parts.join(" "))
    }
    /// Brigadier's smart usage: one line per child of `node`.
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
mod particle_option_tests;
#[cfg(test)]
mod tests;
